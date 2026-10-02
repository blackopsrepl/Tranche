#!/usr/bin/env python3
"""Native evidence capture, inspection and export for a selected Tranche batch.

Selecting a batch and capturing its public GitHub evidence is a Tranche operation,
not a handout to another application. This module owns the native state, the
inspection path and the export format; the workbench and the MCP server are
intended to read the same service rather than reimplementing acquisition.

Read-only and model-free: the transport is `ghread`, which keeps the credential
inside `gh`, issues only GETs and GraphQL queries, and never mutates GitHub. No
command here calls a model, opens a browser, or executes captured content.

The identities this module keeps apart are documented in
`docs/decisions/native-evidence-cli.md`: report association, code identity,
mutable observation, and capture generation.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

import ghread
import report_loader
import tranche

FORMAT = "tranche.evidence-packet/v1"
PROFILE = "pr-review/v1"

# The public operations a caller may rely on. `checks` keeps check-runs and
# commit statuses as two separately paginated groups inside one component: an
# empty check-run list and an empty status list are different observations and
# neither is proof that no CI is configured.
REST_COMPONENTS = ("metadata", "diff", "files", "discussion", "review_comments", "reviews")
COMPONENTS = (*REST_COMPONENTS, "checks", "closing_issues")
# component -> the endpoint groups it is made of. REST components are one group
# each. `checks` is four independently paginated collections, because a PR's
# checks are created where the suite was created and that has two possible homes:
# the base repository (the PR's own checks, which is what a reviewer means by
# "the CI") and, only when the head is a fork, the linked repository, whose CI is
# the contributor's own branch state. Both are recorded and each response names
# the repository it came from, so nothing here claims an absence that one source
# simply could not see.
COMPONENT_GROUPS = {
    "metadata": ("metadata",), "diff": ("diff",), "files": ("files",),
    "discussion": ("discussion",), "review_comments": ("review_comments",),
    "reviews": ("reviews",),
    "checks": ("check_runs", "statuses", "fork_check_runs", "fork_statuses"),
    "closing_issues": ("closing_issues",),
}
GROUPS = tuple(group for groups in COMPONENT_GROUPS.values() for group in groups)
BASE_CI_GROUPS = ("check_runs", "statuses")
FORK_CI_GROUPS = ("fork_check_runs", "fork_statuses")
CI_GROUPS = (*BASE_CI_GROUPS, *FORK_CI_GROUPS)
# A check-runs response is capped at the 1000 most recent suites and a file list
# at 3000 entries; reaching either is a named gap, never silent completeness.
CHECK_RUN_CAP = 1000
FILE_LIST_CAP = 3000


def ci_targets(member: dict) -> list[tuple[str, str, int]]:
    """Every repository whose CI is worth reading for this member, and why.

    The base repository holds the PR's own checks; the fork, when there is one,
    holds whatever CI the contributor's branch has. Reading only the base would
    quietly miss the second, and reading only the fork would present the wrong
    repository's CI as the PR's.
    """
    targets = [(group, member["base_repo_name"], member["base_repo_id"])
               for group in BASE_CI_GROUPS]
    if member["head_repo_name"] != member["base_repo_name"]:
        targets += [(group, member["head_repo_name"], member["head_repo_id"])
                    for group in FORK_CI_GROUPS]
    return targets


def groups_for(member: dict, component: str) -> tuple[str, ...]:
    if component == "checks":
        return tuple(group for group, _repo, _id in ci_targets(member))
    return COMPONENT_GROUPS[component]
# Components whose raw bytes are a document a person reads, so citations from
# them resolve against the decoded text of that same record.
CONTENT_COMPONENTS = ("files", "diff", "discussion", "review_comments", "reviews")

EXIT_OK = 0
EXIT_USABLE = 1
EXIT_USAGE = 2
EXIT_REFUSED = 3
EXIT_INCOMPLETE = 4

DEFAULT_REQUEST_BUDGET = 200
DEFAULT_MAX_BYTES = 64 * 1024 * 1024
DEFAULT_EXPORT_BYTES = 64 * 1024 * 1024
DEFAULT_WINDOW_BYTES = 16 * 1024
MAX_PAGES = 20
PER_PAGE = 100
MAX_DIFF_BYTES = 3 * 1024 * 1024
MAX_DIFF_LINES = 50_000
MAX_CITATIONS = 5_000
MAX_CITATIONS_PER_PAGE = 32
RESERVED_BUDGET = 3
LOCK_TTL = 900

CAPTURE_ID = re.compile(r"[0-9a-f]{32}")
BATCH_ID = re.compile(r"B[0-9]{3,6}")
LOCATOR = re.compile(r"(https?://\S+|@[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})|[A-Za-z0-9_-]+/[A-Za-z0-9_.-]+#\d+)")
TRAILING = ".,;:!?)]}'\""
CLOSING_QUERY = (
    "query trancheClosingIssues($owner:String!,$name:String!,$number:Int!,$cursor:String){"
    " repository(owner:$owner,name:$name){ pullRequest(number:$number){ number url"
    " closingIssuesReferences(first:100,after:$cursor){ totalCount"
    " pageInfo{hasNextPage endCursor} nodes{number url state title} } } } }"
)


class EvidenceError(RuntimeError):
    """The evidence request cannot be satisfied as asked."""


class SelectionError(EvidenceError):
    """The current native selection refuses this request."""


class IncompleteCapture(EvidenceError):
    """The capture is usable but not complete; the message says how to resume."""


def now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def root() -> Path:
    """The evidence root, resolved at call time so tests can move OUT_DIR."""
    return tranche.OUT_DIR / "evidence"


def evidence_digest(value) -> str:
    """Tranche's own object digest, so evidence identity matches the repo's."""
    return tranche.digest(value)


# ---------------------------------------------------------------------------
# Paths, confinement and the writer lock
# ---------------------------------------------------------------------------

def capture_dir(capture_id: str) -> Path:
    if not isinstance(capture_id, str) or CAPTURE_ID.fullmatch(capture_id) is None:
        raise EvidenceError("capture id must be 32 lowercase hex characters")
    return root() / capture_id


def confined_file(path, base: Path) -> Path:
    """A regular single-link file beneath `base`, with no symlink anywhere.

    Every level beneath the root is checked with `lstat`, so a symlinked
    directory or a hard-linked body is refused rather than followed, and a
    traversal that escapes the root is refused even if the final path exists.
    """
    if not isinstance(path, str) or not path:
        raise EvidenceError("empty evidence path")
    if "\x00" in path:
        raise EvidenceError("evidence path contains a NUL byte")
    parts = Path(path).parts
    if any(part in ("..", "") for part in parts) or Path(path).is_absolute():
        raise EvidenceError(f"refusing unconfined path {path!r}")
    if not any(part not in (".",) for part in parts):
        raise EvidenceError(f"refusing empty path {path!r}")
    base = base.resolve()
    current = base
    for part in parts:
        current = current / part
        if current.is_symlink():
            raise EvidenceError(f"refusing symlink in evidence path: {path!r}")
        if current.exists() and current.is_dir() and current != base:
            continue
    resolved = (base / Path(*parts)).resolve()
    if resolved != base and base not in resolved.parents:
        raise EvidenceError(f"refusing path outside the evidence root: {path!r}")
    if resolved.is_symlink():
        raise EvidenceError(f"refusing symlinked evidence file: {path!r}")
    if resolved.exists():
        info = resolved.lstat()
        if not resolved.is_file():
            raise EvidenceError(f"refusing non-regular evidence file: {path!r}")
        if info.st_nlink != 1:
            raise EvidenceError(f"refusing multiply-linked evidence file: {path!r}")
    return resolved


def atomic_bytes(path: Path, data: bytes) -> None:
    """Publish bytes atomically, replacing any previous record only on success."""
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.parent / f".pending-{os.getpid()}-{int(time.time() * 1000)}"
    try:
        with temporary.open("wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.chmod(temporary, 0o600)
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


def atomic_manifest(path: Path, value: dict) -> None:
    atomic_bytes(path, json.dumps(value, ensure_ascii=False, allow_nan=False,
                                  sort_keys=True).encode("utf-8"))


class Lock:
    """One writer per capture, with a bounded TTL so a crash cannot strand work.

    The lock records the owning pid and when it started. It is released by the
    owner, reclaimed when the recorded process no longer exists, and refused
    while a live writer holds it. `break_lock` is the explicit operator override
    for the remaining case: a live-looking pid that no longer owns the capture.
    """

    def __init__(self, capture_id: str, *, ttl: int = LOCK_TTL):
        self.capture_id = capture_id
        self.ttl = ttl
        self.path = root() / f"{capture_id}.lock"
        self.held = False

    def _owner(self) -> dict | None:
        try:
            value = json.loads(self.path.read_text())
        except (OSError, json.JSONDecodeError):
            return None
        return value if isinstance(value, dict) else None

    @staticmethod
    def _alive(pid) -> bool:
        if type(pid) is not int or pid <= 0:
            return False
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return False
        except PermissionError:
            return True
        return True

    def acquire(self, *, break_lock: bool = False) -> None:
        root().mkdir(parents=True, exist_ok=True)
        for attempt in (1, 2):
            try:
                handle = os.open(self.path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
            except FileExistsError:
                owner = self._owner() or {}
                stale = not self._alive(owner.get("pid"))
                if self.path.exists():
                    try:
                        age = time.time() - self.path.stat().st_mtime
                    except OSError:
                        age = 0
                    stale = stale or age > self.ttl
                if break_lock or stale:
                    self._release_file()
                    if attempt == 1:
                        continue
                    raise EvidenceError(
                        f"could not replace the writer lock at {self.path}") from None
                raise EvidenceError(
                    f"another writer holds {self.capture_id} "
                    f"(pid {owner.get('pid', 'unknown')}); use --break-lock if it is gone")
            with os.fdopen(handle, "w") as stream:
                json.dump({"pid": os.getpid(), "started": now(),
                           "started_at": time.time()}, stream)
                stream.flush()
                os.fsync(stream.fileno())
            self.held = True
            return

    def _release_file(self) -> None:
        try:
            self.path.unlink()
        except OSError:
            pass

    def release(self) -> None:
        if self.held:
            self._release_file()
            self.held = False

    def __enter__(self) -> "Lock":
        self.acquire()
        return self

    def __exit__(self, *exc) -> None:
        self.release()


# ---------------------------------------------------------------------------
# Selection: the exact batch, revisions and provenance this evidence belongs to
# ---------------------------------------------------------------------------

def captured_items() -> dict:
    """Raw captured PR items keyed by number, for base/head identity.

    `load_prs` projects the fields a judgment depends on and deliberately drops
    the base revision and the repository identities, which code identity needs.
    The membership itself is read through the same source selection and the same
    checksum, never a parallel snapshot.
    """
    snapshot = tranche.PAGES_DIR / "snapshot.json"
    if snapshot.exists():
        value = json.loads(snapshot.read_text())
        if (not isinstance(value, dict) or value.get("repo") != tranche.REPO
                or value.get("digest") != tranche.digest(value.get("items"))):
            raise SelectionError("captured snapshot identity or checksum differs; fetch again")
        pages = [value["items"]]
    else:
        pages = [json.loads(path.read_text())
                 for path in sorted(tranche.PAGES_DIR.glob("page_*.json"))]
    items: dict[int, dict] = {}
    for page in pages:
        if not isinstance(page, list):
            raise SelectionError("captured PR membership must be a list; fetch again")
        for item in page:
            if not isinstance(item, dict) or type(item.get("number")) is not int:
                raise SelectionError("invalid captured PR shape; fetch again")
            items[item["number"]] = item
    return items


SHA1 = re.compile(r"[0-9a-f]{40}")


def _revision(number: int, items: dict) -> dict:
    """Full base/head identity for one member, or refuse the selection.

    A short or missing revision is never substituted: the whole point of the code
    observation is that it names an exact pair of commits.
    """
    item = items.get(number)
    if item is None:
        raise SelectionError(f"#{number} is not in the captured membership; fetch again")
    head = item.get("head") or {}
    base = item.get("base") or {}
    head_repo = head.get("repo")
    base_repo = base.get("repo")
    if not isinstance(head_repo, dict) or not isinstance(base_repo, dict):
        raise SelectionError(
            f"#{number} no longer exposes its base/head repository (a deleted fork cannot be "
            "captured); the selection cannot bind this member")
    for name, value in (("base_sha", base.get("sha")), ("head_sha", head.get("sha"))):
        if not isinstance(value, str) or SHA1.fullmatch(value) is None:
            raise SelectionError(f"#{number} has no full 40-character {name}; fetch again")
    identity = {
        "number": number,
        "base_sha": base["sha"], "head_sha": head["sha"],
        "base_repo_id": base_repo.get("id"), "base_repo_name": base_repo.get("full_name"),
        "head_repo_id": head_repo.get("id"), "head_repo_name": head_repo.get("full_name"),
        "head_fork": bool(head_repo.get("fork")),
        "updated_at": item.get("updated_at"),
    }
    for name in ("base_repo_id", "head_repo_id"):
        if type(identity[name]) is not int or identity[name] <= 0:
            raise SelectionError(f"#{number} has no usable {name}; fetch again")
    for name in ("base_repo_name", "head_repo_name"):
        if not isinstance(identity[name], str) or "/" not in identity[name]:
            raise SelectionError(f"#{number} has no usable {name}; fetch again")
    if identity["updated_at"] is not None and not isinstance(identity["updated_at"], str):
        raise SelectionError(f"#{number} has an unreadable updated_at; fetch again")
    return identity


def revision_key(identity: dict) -> dict:
    """What a revision move actually means: commits and the repositories they sit in.

    `updated_at` is deliberately excluded. It is observed upstream data and a PR
    update is thread activity, not a code revision; treating it as a universal
    revision marker would force re-downloading unchanged code.
    """
    return {name: identity[name] for name in
            ("base_sha", "head_sha", "base_repo_id", "base_repo_name",
             "head_repo_id", "head_repo_name")}


# Code components name a revision's content; mutable components are timestamped
# observations of a moving conversation. A thread update refreshes the second
# group without touching the first, and only a revision move touches the first.
CODE_COMPONENTS = ("metadata", "diff", "files")
MUTABLE_COMPONENTS = ("discussion", "review_comments", "reviews", "checks",
                      "closing_issues")


class Selection:
    """The immutable native selection one capture generation is bound to."""

    __slots__ = ("repository", "report", "batch", "members", "_prompt", "_membership")

    def __init__(self, repository: dict, report: dict, batch: dict, members: list):
        self.repository = repository
        self.report = report
        self.batch = batch
        self.members = members
        self._prompt = batch["review_prompt"]
        self._membership = [m["number"] for m in members]

    @property
    def numbers(self) -> list[int]:
        return list(self._membership)

    def as_json(self) -> dict:
        """The frozen projection written into every manifest and export.

        The native batch is copied whole - ordinal, ordered membership and the
        exact reviewer prompt - because the prompt is provenance a reader can
        check, not a cache key. Freezing only these fields keeps a later native
        presentation change from invalidating recorded evidence.
        """
        return {"repository": self.repository,
                "report": self.report,
                "batch": {"id": self.batch["id"], "ordinal": self.batch["ordinal"],
                          "members": list(self._membership), "count": self.batch["count"],
                          "review_prompt": self._prompt},
                "members": [self._member_json(m) for m in self.members]}

    @staticmethod
    def _member_json(member: dict) -> dict:
        keys = ("number", "source_digest", "evidence_digest", "base_sha", "head_sha",
                "base_repo_id", "base_repo_name", "head_repo_id", "head_repo_name",
                "head_fork", "updated_at")
        return {key: member[key] for key in keys}

    @property
    def membership_digest(self) -> str:
        return evidence_digest(self._membership)

    def revision(self) -> dict:
        """The code identity of the whole selection, stable across thread churn."""
        return {str(m["number"]): revision_key(m) for m in self.members}


def select(batch_id: str, report: report_loader.BoundReport) -> Selection:
    """Resolve one batch through the authoritative bound report, or refuse.

    Every refusal here is a refusal to attach evidence to the wrong thing: an
    unknown ordinal, a batch file the loader rejected, a parked member, a member
    whose revisions or repository identities are unavailable.
    """
    if not isinstance(batch_id, str) or BATCH_ID.fullmatch(batch_id) is None:
        raise SelectionError("expected an exact batch id such as B001")
    if report.batches is None:
        raise SelectionError("batches.json is unavailable; run tranche.py batches")
    batch = next((b for b in report.batches["batches"] if b.get("id") == batch_id), None)
    if batch is None:
        raise SelectionError(f"unknown batch id {batch_id}; the current report has "
                             f"{len(report.batches['batches'])} batches")
    parked = {m["number"] for m in (report.parked or {}).get("members", [])}
    inconsistent = sorted(set(batch["members"]) & parked)
    if inconsistent:
        raise SelectionError(
            "batch " + batch_id + " lists parked PRs " +
            ", ".join(f"#{n}" for n in inconsistent) +
            "; the batch plan and the park record disagree - rerun tranche.py batches")
    items = captured_items()
    members = []
    for number in batch["members"]:
        identity = _revision(number, items)
        pr = report.prs.get(number)
        if pr is None:
            raise SelectionError(f"#{number} is missing from the report projection")
        identity["source_digest"] = pr["source_digest"]
        identity["evidence_digest"] = pr["evidence_digest"]
        members.append(identity)
    base_repo = members[0]
    repository = {"id": base_repo["base_repo_id"], "full_name": base_repo["base_repo_name"],
                  "visibility": base_repo.get("base_repo_visibility", "unknown")}
    if any(m["base_repo_id"] != repository["id"] or m["base_repo_name"] != repository["full_name"]
           for m in members):
        raise SelectionError("batch members do not share one base repository")
    report_block = {"report_binding": report.identity["report_binding"],
                    "output_digests": {name: report.identity[name] for name in
                                       ("clusters.json", "dupes.json",
                                        "batches.json", "parked.json")}}
    return Selection(repository, report_block, batch, members)


def selection_of(capture_id: str) -> Selection:
    """Rebuild the frozen selection of a stored capture (works offline, any report)."""
    manifest = read_manifest(capture_id)
    stored = manifest["selection"]
    batch = stored["batch"]
    members = [dict(m) for m in stored["members"]]
    return Selection(dict(stored["repository"]), dict(stored["report"]), batch, members)


# ---------------------------------------------------------------------------
# Manifest, generation and stored bytes
# ---------------------------------------------------------------------------

def strict_json_loads(data: bytes):
    """Load JSON refusing the shapes that make a record ambiguous.

    Duplicate keys, NaN/Infinity and invalid UTF-8 are all refusal cases: a
    checkpoint that can be read two ways cannot be trusted to resume from.
    """
    def unique(pairs):
        seen = {}
        for key, value in pairs:
            if key in seen:
                raise EvidenceError(f"duplicate JSON key {key!r}")
            seen[key] = value
        return seen

    def reject(value):
        raise EvidenceError(f"non-finite JSON number {value!r}")

    try:
        return json.loads(data.decode("utf-8"), object_pairs_hook=unique,
                          parse_constant=reject)
    except UnicodeDecodeError as exc:
        raise EvidenceError(f"evidence record is not valid UTF-8 ({exc})") from exc


def manifest_path(capture_id: str) -> Path:
    return capture_dir(capture_id) / "manifest.json"


def body_path(body_sha256: str) -> Path:
    """Bodies live in one content-addressed store at the evidence root.

    Keying by digest rather than by capture is what lets a new association reuse
    unchanged bytes without copying them, and it keeps a body a single immutable
    artifact instead of one copy per capture that recorded it. The manifest
    carries only the digest; the location is derived, never recorded, so there is
    no second identity for the same bytes.
    """
    if not isinstance(body_sha256, str) or re.fullmatch(r"[0-9a-f]{64}", body_sha256) is None:
        raise EvidenceError("source body digest must be 64 lowercase hex characters")
    return root() / "bodies" / f"{body_sha256}.bin"


def store_body(data: bytes) -> str:
    """Persist immutable source bytes and return their sha256.

    Bytes are written and fsynced before any manifest may cite them, so a crash
    between the two leaves an unreferenced file - swept later - and never a
    checkpoint pointing at bytes that are not there.
    """
    sha = hashlib.sha256(data).hexdigest()
    path = body_path(sha)
    if path.exists():
        read_body(sha)  # identical name is not proof: verify before trusting it
        return sha
    atomic_bytes(path, data)
    return sha


def read_body(body_sha256: str) -> bytes:
    """Read stored bytes, verifying them against their digest before returning."""
    path = body_path(body_sha256)
    if not path.exists():
        raise EvidenceError(f"stored source {body_sha256[:12]}… is missing")
    data = path.read_bytes()
    actual = hashlib.sha256(data).hexdigest()
    if actual != body_sha256:
        raise EvidenceError(
            f"stored source {body_sha256[:12]}… is corrupted (computed {actual[:12]}…)")
    return data


def referenced_bodies() -> set[str]:
    """Every body digest any stored capture cites, for the orphan sweep."""
    referenced: set[str] = set()
    directory = root()
    if not directory.exists():
        return referenced
    for path in directory.glob("*/manifest.json"):
        try:
            manifest = strict_json_loads(path.read_bytes())
        except (OSError, EvidenceError):
            continue
        if isinstance(manifest, dict) and isinstance(manifest.get("sources"), list):
            for source in manifest["sources"]:
                if isinstance(source, dict) and isinstance(source.get("body_sha256"), str):
                    referenced.add(source["body_sha256"])
    return referenced


def sweep_orphans() -> int:
    """Remove stored bodies no capture cites.

    A crash can leave bytes written but not yet cited. They are harmless, but they
    are also unreferenced state, so a completed run tidies them rather than
    leaving unbounded growth behind.
    """
    directory = root() / "bodies"
    if not directory.exists():
        return 0
    keep = referenced_bodies()
    removed = 0
    for path in directory.glob("*.bin"):
        if path.stem not in keep:
            try:
                path.unlink()
                removed += 1
            except OSError:
                pass
    return removed


def new_capture_id() -> str:
    return os.urandom(16).hex()


def generation_id(selection: Selection, capture_id: str) -> str:
    """The capture generation: what all its sources belong to and cannot escape.

    Built from the repository identity, the members' code identity and the report
    association. It deliberately excludes `updated_at`: a thread update must not
    mint a new generation or force re-downloading unchanged code, where a moved
    base or head must.
    """
    return evidence_digest({
        "format": FORMAT, "profile": PROFILE, "capture_id": capture_id,
        "repository": selection.repository, "revision": selection.revision(),
        "membership": selection.membership_digest, "batch": selection.batch["id"],
        "report": selection.report,
    })


def component_slots(selection: Selection) -> list[tuple[int, str]]:
    return [(number, component) for number in selection.numbers for component in COMPONENTS]


def fresh_components(selection: Selection) -> list[dict]:
    entries = []
    for number, component in component_slots(selection):
        entry = {"number": number, "component": component, "status": "missing",
                 "reason": "not acquired", "groups": []}
        for group in groups_for(member_repo(number, selection), component):
            entry["groups"].append({"group": group, "status": "missing", "pages": 0,
                                    "next_url": None, "source_ids": [],
                                    "reason": "not acquired"})
        entries.append(entry)
    return entries


def roll_up(entry: dict) -> dict:
    """Aggregate a component's groups into one honest status.

    A terminal cursor is not completeness: a group only counts as complete when
    it produced at least one recorded page (or an explicitly captured empty
    response) and had no continuation left and no blocking reason.
    """
    statuses = [group["status"] for group in entry["groups"]]
    if any(status == "blocked" for status in statuses):
        entry["status"] = "blocked"
    elif any(status == "missing" for status in statuses):
        entry["status"] = "missing"
    elif all(status == "complete" for status in statuses):
        entry["status"] = "complete"
    else:
        entry["status"] = "partial"
    problems = [f"{group['group']}: {group['reason']}" for group in entry["groups"]
                if group["status"] != "complete" and group.get("reason")]
    entry["reason"] = None if entry["status"] == "complete" else (
        "; ".join(problems) or "incomplete")
    return entry


def build_manifest(selection: Selection, capture_id: str, *, request_limit: int,
                   observed_at: str | None = None) -> dict:
    return {
        "format": FORMAT, "profile": PROFILE, "capture_id": capture_id,
        "generation": generation_id(selection, capture_id),
        "created_at": observed_at or now(), "updated_at": observed_at or now(),
        "selection": selection.as_json(),
        "code_observation": {"observed_at": observed_at or now(),
                             "thread_updated_at": {str(m["number"]): m["updated_at"]
                                                   for m in selection.members}},
        "capture": {"observed_at": observed_at or now(), "request_limit": request_limit,
                    "requests_used": 0, "reserved": RESERVED_BUDGET, "failures": 0,
                    "retries": 0, "identity_checks": 0, "stop_reason": None,
                    "bytes_stored": 0},
        "sources": [], "components": fresh_components(selection), "citations": [],
    }


def write_manifest(capture_id: str, manifest: dict) -> None:
    manifest["updated_at"] = now()
    atomic_manifest(manifest_path(capture_id), manifest)


def read_manifest(capture_id: str) -> dict:
    """Load and structurally verify one capture checkpoint.

    Verifies the manifest is internally consistent - the generation recomputes
    from its own selection, source ids recompute from their own records, ids are
    unique, bodies the manifest cites are accounted for. It does not read body
    bytes; those are verified when they are exposed. A historical capture stays
    readable even if the whole report changes or disappears: nothing here consults
    the current report.
    """
    path = manifest_path(capture_id)
    if not path.exists():
        raise EvidenceError(f"no capture {capture_id} under {root()}")
    manifest = strict_json_loads(path.read_bytes())
    if not isinstance(manifest, dict):
        raise EvidenceError("capture manifest is not a JSON object")
    for field in ("format", "profile", "capture_id", "generation", "created_at", "updated_at",
                  "selection", "code_observation", "capture", "sources", "components",
                  "citations"):
        if field not in manifest:
            raise EvidenceError(f"capture manifest is missing {field!r}")
    if manifest["format"] != FORMAT or manifest["profile"] != PROFILE:
        raise EvidenceError("capture manifest declares a different format or profile")
    if manifest["capture_id"] != capture_id:
        raise EvidenceError("capture manifest does not match its directory")
    if not isinstance(manifest["sources"], list) or not isinstance(manifest["components"], list):
        raise EvidenceError("capture manifest records are malformed")
    if not isinstance(manifest["citations"], list):
        raise EvidenceError("capture manifest citations are malformed")
    stored = manifest["selection"]
    if not isinstance(stored, dict) or set(stored) != {"repository", "report", "batch", "members"}:
        raise EvidenceError("capture manifest selection is malformed")
    batch = stored["batch"]
    if (not isinstance(batch, dict) or set(batch) != {"id", "ordinal", "members", "count",
                                                      "review_prompt"}
            or not isinstance(batch.get("review_prompt"), str)
            or not batch["review_prompt"].strip()
            or not isinstance(batch.get("members"), list)
            or any(type(n) is not int or n <= 0 for n in batch["members"])
            or batch.get("count") != len(batch["members"])
            or batch.get("id") != f"B{batch.get('ordinal'):03d}"):
        raise EvidenceError("capture manifest batch provenance is malformed")
    members = stored["members"]
    if (not isinstance(members, list) or [m.get("number") for m in members] != batch["members"]):
        raise EvidenceError("capture manifest membership does not match its batch")
    for member in members:
        if not isinstance(member, dict) or not isinstance(member.get("base_sha"), str) \
                or SHA1.fullmatch(member["base_sha"]) is None \
                or not isinstance(member.get("head_sha"), str) \
                or SHA1.fullmatch(member["head_sha"]) is None:
            raise EvidenceError("capture manifest member revision is malformed")
    selection = Selection(dict(stored["repository"]), dict(stored["report"]),
                          batch, [dict(m) for m in members])
    if manifest["generation"] != generation_id(selection, capture_id):
        raise EvidenceError("capture manifest generation does not match its own selection")
    seen_ids, seen_slots = set(), set()
    for source in manifest["sources"]:
        if not isinstance(source, dict):
            raise EvidenceError("capture manifest source record is malformed")
        for field in ("id", "number", "component", "group", "page", "cursor", "next_cursor",
                      "url", "accept", "media_type", "captured_at", "generation",
                      "body_sha256"):
            if field not in source:
                raise EvidenceError(f"source record is missing {field!r}")
        if type(source["number"]) is not int or source["number"] not in batch["members"]:
            raise EvidenceError("source record names a member outside the selection")
        if source["component"] not in COMPONENTS or source["group"] not in GROUPS:
            raise EvidenceError("source record names an unknown component or group")
        if source["group"] not in COMPONENT_GROUPS[source["component"]]:
            raise EvidenceError("source record group does not belong to its component")
        if type(source["page"]) is not int or source["page"] < 1:
            raise EvidenceError("source record page is malformed")
        slot = (source["number"], source["component"], source["group"], source["page"])
        if slot in seen_slots or source["id"] in seen_ids:
            raise EvidenceError("capture manifest repeats a source slot or id")
        seen_slots.add(slot)
        seen_ids.add(source["id"])
        if source["generation"] != manifest["generation"]:
            raise EvidenceError("source record belongs to a different generation")
        if source["id"] != evidence_digest(
                {"generation": manifest["generation"],
                 "source": {k: v for k, v in source.items() if k != "id"}}):
            raise EvidenceError("source record id does not match its own contents")
        if body_path(source["body_sha256"]) is None:
            raise EvidenceError("source record has no usable body digest")
    for citation in manifest["citations"]:
        if not isinstance(citation, dict) or set(citation) != {
                "id", "number", "component", "source_id", "source_sha256",
                "start_byte", "end_byte", "excerpt_sha256", "kind"}:
            raise EvidenceError("capture manifest citation is malformed")
        if citation["source_id"] not in seen_ids:
            raise EvidenceError("citation references a source outside the capture")
        if (type(citation["start_byte"]) is not int or type(citation["end_byte"]) is not int
                or citation["start_byte"] < 0
                or citation["end_byte"] <= citation["start_byte"]):
            raise EvidenceError("citation byte range is not a nonempty half-open interval")
    return manifest


# ---------------------------------------------------------------------------
# Endpoint mapping. Real GitHub endpoints; the #13 fixtures' synthetic
# /pulls/N/metadata, /discussion and /checks URLs are not endpoints.
# ---------------------------------------------------------------------------

DIFF_ACCEPT = "application/vnd.github.diff"
JSON_MEDIA = "application/vnd.github+json"


def member_repo(number: int, selection: Selection) -> dict:
    for member in selection.members:
        if member["number"] == number:
            return member
    raise EvidenceError(f"#{number} is not a member of this selection")


def source_url(number: int, component: str, group: str, member: dict, *,
               page_url: str | None = None) -> str:
    """The exact URL for one endpoint group, or the recorded continuation URL."""
    if page_url:
        return page_url
    base = member["base_repo_name"]
    if component == "metadata":
        return f"{ghread.ORIGIN}/repos/{base}/pulls/{number}"
    if component == "diff":
        return f"{ghread.ORIGIN}/repos/{base}/pulls/{number}"
    if component == "files":
        return f"{ghread.ORIGIN}/repos/{base}/pulls/{number}/files"
    if component == "discussion":
        return f"{ghread.ORIGIN}/repos/{base}/issues/{number}/comments"
    if component == "review_comments":
        return f"{ghread.ORIGIN}/repos/{base}/pulls/{number}/comments"
    if component == "reviews":
        return f"{ghread.ORIGIN}/repos/{base}/pulls/{number}/reviews"
    if component == "checks" and group in CI_GROUPS:
        suffix = "check-runs" if group.endswith("check_runs") else "status"
        return f"{ghread.ORIGIN}/repos/{ci_repo(member, group)}/commits/{member['head_sha']}/{suffix}"
    if component == "closing_issues":
        return ghread.GRAPHQL_URL
    raise EvidenceError(f"no endpoint is mapped for {component}/{group}")


def ci_repo(member: dict, group: str) -> str:
    """The repository a CI group is read from: the PR's own, or the linked fork."""
    return (member["head_repo_name"] if group in FORK_CI_GROUPS
            else member["base_repo_name"])


def ci_repo_id(member: dict, group: str) -> int:
    return member["head_repo_id"] if group in FORK_CI_GROUPS else member["base_repo_id"]


def accept_for(component: str, group: str) -> str:
    return DIFF_ACCEPT if component == "diff" else JSON_MEDIA


def list_params(component: str, group: str, page: int = 1) -> dict:
    """Pagination parameters for a group; single-object groups take none.

    `per_page` is always sent rather than assumed: GitHub's default is 30, and a
    page whose size we do not control cannot be reasoned about.
    """
    if component in ("metadata", "diff"):
        return {}
    if component == "checks" and group == "statuses":
        return {}  # the combined status is one object, not a list
    return {"per_page": str(PER_PAGE), "page": str(page)}


def closing_query() -> str:
    return ("query trancheClosingIssues($owner:String!,$name:String!,$number:Int!,$cursor:String){"
            " repository(owner:$owner,name:$name){ nameWithOwner id"
            " pullRequest(number:$number){ number url"
            " closingIssuesReferences(first:100,after:$cursor){ totalCount"
            " pageInfo{hasNextPage endCursor} nodes{number url state title} } } } }")


def closing_variables(member: dict, cursor: str | None) -> dict:
    owner, name = member["base_repo_name"].split("/", 1)
    return {"owner": owner, "name": name, "number": member["number"], "cursor": cursor}


def component_sources(manifest: dict, number: int, component: str, group: str) -> list[dict]:
    return [s for s in manifest["sources"]
            if s["number"] == number and s["component"] == component and s["group"] == group]


def component_entry(manifest: dict, number: int, component: str) -> dict:
    for entry in manifest["components"]:
        if entry["number"] == number and entry["component"] == component:
            return entry
    entry = {"number": number, "component": component, "status": "missing",
             "reason": "not acquired", "groups": []}
    manifest["components"].append(entry)
    return entry


def group_entry(manifest: dict, number: int, component: str, group: str) -> dict:
    entry = component_entry(manifest, number, component)
    for state in entry["groups"]:
        if state["group"] == group:
            return state
    state = {"group": group, "status": "missing", "pages": 0, "next_url": None,
             "source_ids": [], "reason": "not acquired"}
    entry["groups"].append(state)
    return state


# ---------------------------------------------------------------------------
# Live identity: what a response must prove before its bytes are accepted
# ---------------------------------------------------------------------------

class LiveCheck:
    """Counts and bounds the live identity reads a capture performs."""

    def __init__(self, budget: ghread.Budget):
        self.budget = budget

    def verify_pr(self, member: dict) -> dict:
        """Re-read the PR's own revisions and prove the recorded ones still hold.

        This is the check that makes reuse honest: checksums prove bytes were not
        corrupted, only a live read shows they still describe the revision they
        claim. It also notices a fork that has been deleted or renamed since the
        capture was made.
        """
        url = source_url(member["number"], "metadata", "metadata", member)
        self.budget.identity_checks += 1
        response = ghread.read(url, accept=JSON_MEDIA, budget=self.budget,
                               repo_prefix=member["base_repo_name"])
        data = response.json()
        if not isinstance(data, dict) or data.get("number") != member["number"]:
            raise EvidenceError(
                f"live metadata for #{member['number']} reports a different pull request")
        live_base = (data.get("base") or {}).get("repo") or {}
        live_head = (data.get("head") or {}).get("repo") or {}
        live = {"base_sha": (data.get("base") or {}).get("sha"),
                "head_sha": (data.get("head") or {}).get("sha"),
                "base_repo_id": live_base.get("id"), "base_repo_name": live_base.get("full_name"),
                "head_repo_id": live_head.get("id"), "head_repo_name": live_head.get("full_name")}
        if live["head_sha"] is None or live["base_sha"] is None:
            raise EvidenceError(f"#{member['number']} no longer exposes its revisions")
        recorded = revision_key(member)
        if {k: live[k] for k in recorded} != recorded:
            raise SelectionError(
                f"#{member['number']} moved: recorded "
                f"{recorded['head_sha'][:12]}… is now {str(live['head_sha'])[:12]}…; "
                "start a fresh capture for the new revision")
        return data

    def verify_ci_scope(self, member: dict, group: str, payload: dict) -> None:
        """A CI response must name the repository and revision we asked about.

        A repository-prefix check on the URL is not proof that a response belongs
        to the resource we asked for, so the returned repository identity is
        compared against the repository this group was read from.
        """
        expected_repo = ci_repo(member, group)
        expected_id = ci_repo_id(member, group)
        repo = payload.get("repository")
        if isinstance(repo, dict):
            if repo.get("full_name") != expected_repo:
                raise EvidenceError(
                    f"CI for #{member['number']} came from {repo.get('full_name')!r}, "
                    f"not {expected_repo!r}")
            if type(repo.get("id")) is int and repo["id"] != expected_id:
                raise EvidenceError(
                    f"CI for #{member['number']} came from repository id {repo['id']}, "
                    f"not {expected_id}")
        sha = payload.get("sha") if "sha" in payload else None
        if sha is not None and sha != member["head_sha"]:
            raise EvidenceError(
                f"CI for #{member['number']} names revision {sha}, not {member['head_sha']}")
        for run in payload.get("check_runs") or []:
            if not isinstance(run, dict):
                raise EvidenceError(f"CI for #{member['number']} returned a malformed check run")
            if run.get("head_sha") is not None and run["head_sha"] != member["head_sha"]:
                raise EvidenceError(
                    f"a check run for #{member['number']} names revision {run['head_sha']}")


def parse_closing(payload: dict, member: dict) -> tuple[str | None, list[dict]]:
    """Read one closing-issues GraphQL page, or refuse a malformed/foreign answer."""
    if not isinstance(payload, dict):
        raise EvidenceError("closing-issues query returned a non-object")
    if payload.get("errors"):
        raise EvidenceError(f"closing-issues query reported errors: {payload['errors']}")
    data = payload.get("data")
    repository = data.get("repository") if isinstance(data, dict) else None
    if not isinstance(repository, dict):
        raise EvidenceError("closing-issues query returned no repository")
    if repository.get("nameWithOwner") != member["base_repo_name"]:
        raise EvidenceError(
            f"closing-issues query answered for {repository.get('nameWithOwner')!r}, "
            f"not {member['base_repo_name']!r}")
    pull = repository.get("pullRequest")
    if not isinstance(pull, dict) or pull.get("number") != member["number"]:
        raise EvidenceError("closing-issues query answered about a different pull request")
    connection = pull.get("closingIssuesReferences")
    if not isinstance(connection, dict):
        raise EvidenceError("closing-issues query returned no connection")
    page_info = connection.get("pageInfo") or {}
    nodes = connection.get("nodes")
    if not isinstance(nodes, list):
        raise EvidenceError("closing-issues query returned no node list")
    has_next = bool(page_info.get("hasNextPage"))
    cursor = page_info.get("endCursor")
    if has_next and (not isinstance(cursor, str) or not cursor):
        raise EvidenceError("closing-issues query claims more pages without a cursor")
    return (cursor if has_next else None), [n for n in nodes if isinstance(n, dict)]


# ---------------------------------------------------------------------------
# Acquisition
# ---------------------------------------------------------------------------

class Capture:
    """One bounded acquisition run over a capture's missing work."""

    def __init__(self, selection: Selection, capture_id: str, manifest: dict, *,
                 budget: ghread.Budget, max_bytes: int = DEFAULT_MAX_BYTES,
                 log=None, max_pages: int = MAX_PAGES):
        self.selection = selection
        self.capture_id = capture_id
        self.manifest = manifest
        self.budget = budget
        self.live = LiveCheck(budget)
        self.max_bytes = max_bytes
        self.max_pages = max_pages
        self.log = log or (lambda message: None)
        self.dirty = False
        self.stop_reason: str | None = None
        self.reused = 0
        self.fetched = 0

    # -- bookkeeping ------------------------------------------------------

    @property
    def stored_bytes(self) -> int:
        return self.manifest["capture"]["bytes_stored"]

    def _spend_bytes(self, count: int) -> None:
        self.manifest["capture"]["bytes_stored"] = self.stored_bytes + count
        if self.stored_bytes > self.max_bytes:
            self.stop_reason = "storage_budget"
            raise ghread.BudgetExhausted(
                f"stored bytes reached {self.stored_bytes} of {self.max_bytes}")

    def _record(self, number: int, component: str, group: str, data: bytes, *, url: str,
                accept: str, media_type: str, cursor, next_cursor) -> None:
        sha = store_body(data)
        state = group_entry(self.manifest, number, component, group)
        source = {
            "id": "", "number": number, "component": component, "group": group,
            "page": state["pages"] + 1, "cursor": cursor, "next_cursor": next_cursor,
            "url": url, "accept": accept, "media_type": media_type,
            "captured_at": now(), "generation": self.manifest["generation"],
            "body_sha256": sha,
        }
        source["id"] = evidence_digest(
            {"generation": self.manifest["generation"],
             "source": {k: v for k, v in source.items() if k != "id"}})
        self.manifest["sources"].append(source)
        state["pages"] += 1
        state["source_ids"].append(source["id"])
        if "items_seen" in state:
            state["items_observed"] = state.pop("items_seen")
        if "items_total" in state:
            state["items_reported"] = state.pop("items_total")
        state["next_url"] = next_cursor
        self._spend_bytes(len(data))
        self.dirty = True

    def _finish_group(self, number: int, component: str, group: str, *, complete: bool,
                      reason: str | None) -> None:
        state = group_entry(self.manifest, number, component, group)
        state["status"] = "complete" if complete else "partial"
        state["reason"] = reason
        if complete:
            state["next_url"] = None
        roll_up(component_entry(self.manifest, number, component))
        self.dirty = True

    def _block_group(self, number: int, component: str, group: str, reason: str) -> None:
        state = group_entry(self.manifest, number, component, group)
        state["status"] = "blocked"
        state["reason"] = reason
        roll_up(component_entry(self.manifest, number, component))
        self.dirty = True

    def _checkpoint(self) -> None:
        capture = self.manifest["capture"]
        capture.update(requests_used=self.budget.used, failures=self.budget.failures,
                       retries=self.budget.retries, identity_checks=self.budget.identity_checks,
                       reserved=self.budget.reserved)
        if self.stop_reason:
            capture["stop_reason"] = self.stop_reason
        elif self.manifest["capture"].get("stop_reason") == "request_budget":
            # A resumed capture that got past the point where it ran out of budget
            # is no longer stopped by it; a stop reason describes the last run.
            capture["stop_reason"] = None
        capture["observed_at"] = now()
        write_manifest(self.capture_id, self.manifest)

    def _reserve(self) -> None:
        """Stop cleanly when the completion checks can no longer be afforded."""
        if not self.budget.can(self.budget.reserved):
            self.stop_reason = "request_budget"
            raise ghread.BudgetExhausted(
                "request budget cannot cover acquisition and the completion checks")

    # -- group acquisition ------------------------------------------------

    def acquire_group(self, number: int, component: str, group: str) -> None:
        member = member_repo(number, self.selection)
        state = group_entry(self.manifest, number, component, group)
        if state["status"] == "complete":
            self.reused += 1
            return
        if component == "closing_issues":
            self._acquire_graphql(member, state)
            return
        self._acquire_rest(member, component, group, state)

    def _acquire_rest(self, member: dict, component: str, group: str, state: dict) -> None:
        page = state["pages"] + 1
        url = state["next_url"] or source_url(member["number"], component, group, member)
        accept = accept_for(component, group)
        while True:
            self._reserve()
            params = list_params(component, group, page)
            try:
                response = ghread.read(url, accept=accept, params=params, budget=self.budget,
                                       reserve=self.budget.reserved,
                                       repo_prefix=(ci_repo(member, group) if component == "checks"
                                                    else member["base_repo_name"]))
            except ghread.ResponseTooLarge as exc:
                self._block_group(member["number"], component, group,
                                  f"response exceeds the transport bound: {exc}")
                return
            except ghread.BudgetExhausted:
                self.stop_reason = "request_budget"
                raise
            except ghread.ReadError as exc:
                self._block_group(member["number"], component, group, f"acquisition failed: {exc}")
                return
            if component == "checks":
                try:
                    payload = response.json()
                except ghread.ReadError as exc:
                    self._block_group(member["number"], component, group,
                                      f"CI response is not JSON: {exc}")
                    return
                try:
                    self.live.verify_ci_scope(member, group, payload)
                except EvidenceError as exc:
                    self._block_group(member["number"], component, group, str(exc))
                    return
            try:
                if component == "diff":
                    next_cursor, complete, reason = self._accept_diff(response)
                else:
                    next_cursor, complete, reason = self._accept_list(
                        member, component, group, response, page, state)
            except _Blocked as exc:
                self._block_group(member["number"], component, group, str(exc))
                return
            self._record(member["number"], component, group, response.body, url=response.url,
                         accept=accept, media_type=response.media_type,
                         cursor=None if page == 1 else str(page), next_cursor=next_cursor)
            self.fetched += 1
            if complete:
                shortfall = self._shortfall(state)
                if shortfall:
                    self._finish_group(member["number"], component, group, complete=False,
                                       reason=shortfall)
                    return
                self._finish_group(member["number"], component, group, complete=True, reason=None)
                return
            if next_cursor is None:
                self._finish_group(member["number"], component, group, complete=False,
                                   reason=reason or "incomplete: no continuation")
                return
            if page >= self.max_pages:
                self._finish_group(member["number"], component, group, complete=False,
                                   reason=f"incomplete: stopped at the {self.max_pages}-page bound")
                return
            url = next_cursor
            page += 1

    def _accept_diff(self, response: ghread.Response):
        """Bound the diff and report honestly what was and was not captured.

        An oversized diff is blocked, not truncated: half a patch presented as a
        diff would be worse than a named gap, and GitHub itself omits patches for
        large or unsupported files, which the file list shows separately.
        """
        body = response.body
        lines = body.count(b"\n")
        if len(body) > MAX_DIFF_BYTES or lines > MAX_DIFF_LINES:
            raise _Blocked(
                f"diff exceeds the review bound ({len(body)} bytes, {lines} lines); "
                "not captured in full and not presented as complete")
        return None, True, None

    def _accept_list(self, member: dict, component: str, group: str,
                     response: ghread.Response, page: int, state: dict):
        """Validate one list page and decide what it says about completeness.

        Each collection has its own envelope, so each is read in its own terms:
        check-runs answer with `{total_count, check_runs}` while the other lists
        are bare arrays. The reported total is recorded separately and compared
        against what was actually observed, because a terminal cursor on a
        truncated collection is not completeness.
        """
        try:
            payload = response.json()
        except ghread.ReadError as exc:
            raise _Blocked(f"{component} response is not JSON: {exc}") from exc
        if component == "metadata":
            if not isinstance(payload, dict) or payload.get("number") != member["number"]:
                raise _Blocked("metadata response is not this pull request")
            if (payload.get("head") or {}).get("sha") != member["head_sha"]:
                raise _Blocked("metadata response names a different head revision")
            return None, True, None
        if component == "checks" and group in ("check_runs", "fork_check_runs"):
            if not isinstance(payload, dict):
                raise _Blocked("check-runs response is not an object")
            runs = payload.get("check_runs")
            if not isinstance(runs, list):
                raise _Blocked("check-runs response carries no check-run list")
            state["items_seen"] = state.get("items_seen", 0) + len(runs)
            if type(payload.get("total_count")) is int:
                state["items_total"] = payload["total_count"]
        elif component == "checks" and group in ("statuses", "fork_statuses"):
            if not isinstance(payload, dict):
                raise _Blocked("combined status response is not an object")
            statuses = payload.get("statuses")
            if not isinstance(statuses, list):
                raise _Blocked("combined status response carries no status list")
            state["items_seen"] = len(statuses)
            if type(payload.get("total_count")) is int:
                state["items_total"] = payload["total_count"]
        elif not isinstance(payload, list):
            raise _Blocked(f"{component} response is not a list")
        next_url = response.next_url
        if next_url is not None:
            if ghread.page_param(next_url) <= page:
                raise _Blocked("continuation URL does not advance the page number")
            return next_url, False, None
        return None, True, None

    @staticmethod
    def _shortfall(state: dict) -> str | None:
        """A group that reported more items than it delivered is not complete.

        A terminal cursor says the pagination ended; it does not say the
        collection was fully observed. Both the reported-vs-observed count and
        GitHub's own documented ceilings are checked here, and either gap is
        named rather than smoothed over.
        """
        seen, total = state.get("items_observed"), state.get("items_reported")
        if type(seen) is int and type(total) is int:
            if total < 0:
                return "incomplete: GitHub reported an unusable item count"
            if seen < total:
                return (f"incomplete: GitHub reports {total} items but {seen} were observed; "
                        "the collection changed or is capped")
            if state["group"] in ("check_runs", "fork_check_runs") and total >= CHECK_RUN_CAP:
                return (f"incomplete: GitHub caps check runs at the {CHECK_RUN_CAP} most "
                        "recent suites; iterate the suites to see them all")
            if state["group"] == "files" and total >= FILE_LIST_CAP:
                return (f"incomplete: GitHub caps the file list at {FILE_LIST_CAP} entries")
        return None

    def _acquire_graphql(self, member: dict, state: dict) -> None:
        cursor = None
        page = state["pages"] + 1
        while True:
            self._reserve()
            variables = closing_variables(member, cursor)
            try:
                response = ghread.graphql(closing_query(), variables, budget=self.budget,
                                          reserve=self.budget.reserved)
                payload = response.json()
            except ghread.BudgetExhausted:
                self.stop_reason = "request_budget"
                raise
            except ghread.ReadError as exc:
                self._block_group(member["number"], "closing_issues", "closing_issues",
                                  f"acquisition failed: {exc}")
                return
            try:
                next_cursor, _nodes = parse_closing(payload, member)
            except EvidenceError as exc:
                self._block_group(member["number"], "closing_issues", "closing_issues", str(exc))
                return
            self._record(member["number"], "closing_issues", "closing_issues", response.body,
                         url=response.url, accept=JSON_MEDIA, media_type=response.media_type,
                         cursor=cursor, next_cursor=next_cursor)
            self.fetched += 1
            if next_cursor is None:
                self._finish_group(member["number"], "closing_issues", "closing_issues",
                                   complete=True, reason=None)
                return
            if page >= self.max_pages:
                self._finish_group(member["number"], "closing_issues", "closing_issues",
                                   complete=False,
                                   reason=f"incomplete: stopped at the {self.max_pages}-page bound")
                return
            cursor = next_cursor
            page += 1

    def _refresh_mutable(self, member: dict, thread_updated: str) -> None:
        """Mark mutable components for re-acquisition without touching code bytes.

        The previously captured immutable sources stay in place and stay
        readable; only the mutable groups go back to `missing` so this run or a
        later one refreshes them. The generation does not change, so historical
        citations keep resolving.
        """
        number = member["number"]
        for component in MUTABLE_COMPONENTS:
            entry = component_entry(self.manifest, number, component)
            for state in entry["groups"]:
                state["status"] = "missing"
                state["reason"] = f"thread updated upstream ({thread_updated}); refresh pending"
            roll_up(entry)
        self.manifest["code_observation"]["thread_updated_at"][str(number)] = thread_updated
        self.dirty = True

    # -- the run ----------------------------------------------------------

    def run(self) -> str | None:
        """Acquire everything missing, reusing what the live revision allows."""
        try:
            # Live revision check first: it decides whether any recorded code
            # evidence may be reused at all, and it must happen before the
            # checks that reserve capacity is meant to protect.
            for member in self.selection.members:
                self._reserve()
                metadata = self.live.verify_pr(member)
                # A thread-only update refreshes the mutable observations and
                # leaves the code observation alone: the diffs and file lists
                # still describe the same two commits.
                live_updated = metadata.get("updated_at")
                recorded = self.manifest["code_observation"]["thread_updated_at"].get(
                    str(member["number"]))
                if isinstance(live_updated, str) and live_updated != recorded:
                    self._refresh_mutable(member, live_updated)
            self._checkpoint()
            for number in self.selection.numbers:
                member = member_repo(number, self.selection)
                for component in COMPONENTS:
                    for group in groups_for(member, component):
                        state = group_entry(self.manifest, number, component, group)
                        if state["status"] == "complete":
                            self.reused += 1
                            continue
                        try:
                            self.acquire_group(number, component, group)
                        except _Blocked:
                            pass
                        self._checkpoint()
        except ghread.BudgetExhausted as exc:
            self.log(str(exc))
            self.stop_reason = self.stop_reason or "request_budget"
        except ghread.ReadError as exc:
            self.log(f"acquisition stopped: {exc}")
            self.stop_reason = self.stop_reason or "transport_error"
        except SelectionError:
            raise
        finally:
            self._checkpoint()
        return self.stop_reason


class _Blocked(Exception):
    """A group failed its own identity or content check; recorded, not raised."""


# ---------------------------------------------------------------------------
# Citations: which reviewable evidence this capture actually points at
# ---------------------------------------------------------------------------

def extract_citations(manifest: dict) -> list[dict]:
    """Derive citations inside the native capture, from the recorded bytes.

    Citations are a projection of what was captured, computed here once and
    stored, so every reader - export, a windowed source read, a future MCP
    adapter - resolves the same offsets without reparsing anything or inventing
    its own eligibility rules. Discussion and review bodies are JSON, so their
    text is quoted exactly and the offsets are computed against the stored bytes
    of the response that carried them.

    Deliberately absent: the pull request's own description, which is the
    report's evidence and not source evidence, and any citation into a `patch`
    field GitHub omitted for a large or unsupported file, which the file list
    reports separately as a gap.
    """
    citations: list[dict] = []

    def add(number: int, component: str, source: dict, body: bytes, text: str) -> None:
        if not text or len(citations) >= MAX_CITATIONS:
            return
        encoded = json.dumps(text, ensure_ascii=False).encode("utf-8")[1:-1]
        start = body.find(encoded)
        if start < 0:
            return
        citations.append({
            "id": "", "number": number, "component": component, "source_id": source["id"],
            "source_sha256": hashlib.sha256(body).hexdigest(),
            "start_byte": start, "end_byte": start + len(encoded),
            "excerpt_sha256": hashlib.sha256(encoded).hexdigest(), "kind": "body",
        })

    for source in manifest["sources"]:
        if source["component"] not in CONTENT_COMPONENTS or len(citations) >= MAX_CITATIONS:
            continue
        body = read_body(source["body_sha256"])
        try:
            payload = json.loads(body.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            continue
        if source["component"] == "files":
            for entry in (payload if isinstance(payload, list) else [])[:MAX_CITATIONS_PER_PAGE]:
                if not isinstance(entry, dict):
                    continue
                for key in ("patch", "filename"):
                    value = entry.get(key)
                    if isinstance(value, str) and value:
                        add(source["number"], "files", source, body, value)
        elif source["component"] in ("discussion", "review_comments", "reviews"):
            for entry in payload if isinstance(payload, list) else []:
                if isinstance(entry, dict) and isinstance(entry.get("body"), str):
                    add(source["number"], source["component"], source, body, entry["body"])
    for citation in citations:
        citation["id"] = evidence_digest({key: value for key, value in citation.items()
                                          if key != "id"})
    return citations


def index_sources(manifest: dict) -> dict:
    return {source["id"]: source for source in manifest["sources"]}


def source_for_window(manifest: dict, source_id: str) -> dict:
    source = index_sources(manifest).get(source_id)
    if source is None:
        raise EvidenceError(f"no source {source_id!r} in capture {manifest['capture_id']}")
    return source


PRINTABLE = re.compile(r"[^\x20-\x7e\n]")


def printable(data: bytes) -> str:
    """A terminal-safe rendering of stored bytes.

    Control characters, escape sequences and non-ASCII are shown as escapes
    rather than emitted, so a captured body - including a hostile patch or a
    description aimed at the operator's terminal - cannot drive the terminal that
    reads it. The stored and hashed bytes are never changed; only this rendering
    is.
    """
    text = data.decode("utf-8", errors="replace")
    return PRINTABLE.sub(
        lambda match: text[match.start():match.end()].encode("unicode_escape").decode("ascii"),
        text)


def window(manifest: dict, source_id: str, *, start: int = 0,
           length: int = DEFAULT_WINDOW_BYTES) -> dict:
    """A bounded, escape-safe window into one stored source.

    Returns the half-open byte range actually delivered and where the next window
    begins, so a reader walks a large body in bounded steps instead of asking for
    a whole packet.
    """
    if type(start) is not int or start < 0:
        raise EvidenceError("start byte must be a nonnegative integer")
    if type(length) is not int or not 1 <= length <= DEFAULT_WINDOW_BYTES:
        raise EvidenceError(f"length must be 1..{DEFAULT_WINDOW_BYTES} bytes")
    source = source_for_window(manifest, source_id)
    body = read_body(source["body_sha256"])
    if start > len(body):
        raise EvidenceError(f"start byte {start} is past the end of a {len(body)} byte source")
    end = min(len(body), start + length)
    chunk = body[start:end]
    return {
        "capture_id": manifest["capture_id"], "source_id": source_id,
        "number": source["number"], "component": source["component"],
        "group": source["group"], "page": source["page"], "url": source["url"],
        "media_type": source["media_type"], "body_sha256": source["body_sha256"],
        "body_bytes": len(body), "start_byte": start, "end_byte": end,
        "next_start": end if end < len(body) else None,
        "window_sha256": hashlib.sha256(chunk).hexdigest(),
        "content": printable(chunk),
    }


def resolve_citation(manifest: dict, citation_id: str) -> dict:
    """Resolve one citation against the stored bytes it names, offline.

    This is the operation a reader must be able to perform without the state
    store, the network or another application: look the source up, verify its
    bytes, and re-derive the excerpt digest that was recorded.
    """
    for citation in manifest["citations"]:
        if citation["id"] != citation_id:
            continue
        body = read_body(citation["source_sha256"])
        excerpt = body[citation["start_byte"]:citation["end_byte"]]
        digest = hashlib.sha256(excerpt).hexdigest()
        if digest != citation["excerpt_sha256"]:
            raise EvidenceError(
                f"citation {citation_id[:12]}… no longer matches its stored bytes")
        return {"citation": citation, "source": index_sources(manifest)[citation["source_id"]],
                "excerpt_sha256": digest, "excerpt": printable(excerpt)}
    raise EvidenceError(f"no citation {citation_id!r} in capture {manifest['capture_id']}")


# ---------------------------------------------------------------------------
# Export: the self-contained, versioned packet
# ---------------------------------------------------------------------------

def is_complete(manifest: dict) -> bool:
    """True when every component of every member completed and nothing stopped us.

    A terminal cursor on every group is necessary but not sufficient: a capture
    that ran out of budget, storage or the page bound is not complete even if the
    pages it did fetch look terminal.
    """
    if manifest["capture"].get("stop_reason"):
        return False
    return all(entry["status"] == "complete" for entry in manifest["components"])


def build_packet(manifest: dict) -> dict:
    """Freeze a small, versioned projection of the capture for an external reader.

    The packet is deliberately not the manifest. It carries what a reader needs to
    resolve citations with no state store, no network and no second application:
    the copied native selection (including the exact reviewer prompt), the ordered
    members with full revisions, the component coverage, the sources with their
    URLs and media types, and the exact bytes of every body it cites, base64 of
    the decoded response. Bodies are stored once per digest and referenced by it,
    so identical bytes are not repeated.
    """
    bodies: dict[str, dict] = {}
    for source in manifest["sources"]:
        sha = source["body_sha256"]
        if sha in bodies:
            continue
        raw = read_body(sha)
        bodies[sha] = {"sha256": sha, "bytes": len(raw),
                       "base64": base64.b64encode(raw).decode("ascii")}
    selection = stored_selection(manifest)
    packet = {
        "format": FORMAT, "profile": PROFILE,
        "capture_id": manifest["capture_id"], "generation": manifest["generation"],
        "selection": manifest["selection"], "membership_digest": selection.membership_digest,
        "complete": is_complete(manifest),
        "capture": {key: manifest["capture"][key] for key in
                    ("observed_at", "request_limit", "requests_used", "reserved", "failures",
                     "retries", "identity_checks", "stop_reason")},
        "components": manifest["components"],
        "sources": [{key: source[key] for key in
                     ("id", "number", "component", "group", "page", "cursor", "next_cursor",
                      "url", "accept", "media_type", "captured_at", "generation",
                      "body_sha256")}
                    for source in manifest["sources"]],
        "bodies": bodies,
        "citations": manifest["citations"],
        "notes": (
            "Captured from public GitHub by Tranche; read-only and model-free. Digests prove "
            "integrity, not authenticity: they show the bytes are the bytes recorded, not that "
            "GitHub served them. A captured CI run is an observation, not a test performed by "
            "Tranche. Complete means every endpoint group reported a terminal page within the "
            "recorded budgets; it is not a review, a test run or approval to merge."),
    }
    packet["packet_digest"] = evidence_digest(packet)
    return packet


def packet_bytes(manifest: dict, *, limit: int = DEFAULT_EXPORT_BYTES) -> bytes:
    """Serialize the packet under an explicit bound, checked on the exact bytes."""
    packet = build_packet(manifest)
    try:
        data = json.dumps(packet, ensure_ascii=False, allow_nan=False, sort_keys=True,
                          separators=(",", ":")).encode("utf-8")
    except (TypeError, ValueError) as exc:
        raise EvidenceError(f"packet cannot be serialized ({exc})") from exc
    if len(data) > limit:
        raise EvidenceError(
            f"packet serializes to {len(data)} bytes, over the {limit} byte export bound; "
            "narrow the batch or raise the bound deliberately")
    return data


def repository_scope(full_name: str, budget: ghread.Budget, expected_id=None) -> dict:
    """Read one repository's live identity and visibility."""
    response = ghread.read(f"{ghread.ORIGIN}/repos/{full_name}", budget=budget,
                           reserve=budget.reserved, repo_prefix=full_name)
    data = response.json()
    if not isinstance(data, dict) or data.get("full_name") != full_name:
        raise SelectionError(f"repository {full_name!r} answered with a different identity")
    if expected_id is not None and data.get("id") != expected_id:
        raise SelectionError(f"repository {full_name!r} is no longer id {expected_id}")
    return {"id": data.get("id"), "full_name": data.get("full_name"),
            "visibility": data.get("visibility"), "private": bool(data.get("private")),
            "archived": bool(data.get("archived")), "url": data.get("html_url")}


def require_shareable(selection: Selection, budget: ghread.Budget) -> dict:
    """The fresh public-scope gate export must pass, for every repository that enters.

    A previously public capture is not permission: visibility revocation blocks
    new sharing. A network failure is not permission either, so a failed check
    raises rather than falling through to writing the packet.
    """
    base = repository_scope(selection.repository["full_name"], budget,
                            selection.repository.get("id"))
    if base["private"] or base["visibility"] != "public":
        raise SelectionError(
            f"repository {base['full_name']} is {base['visibility'] or 'private'}; "
            "refusing to export a packet that would share it")
    linked = []
    for name in sorted({m["head_repo_name"] for m in selection.members
                        if m["head_repo_name"] != base["full_name"]}):
        head = repository_scope(name, budget)
        if head["private"] or head["visibility"] != "public":
            raise SelectionError(
                f"linked repository {name} is not public; refusing to export its evidence")
        linked.append(head)
    return {"repository": base, "linked": linked}


def export_bytes(manifest: dict, selection: Selection, *,
                 limit: int = DEFAULT_EXPORT_BYTES) -> bytes:
    return packet_bytes(manifest, limit=limit)


def stored_selection(manifest: dict) -> Selection:
    stored = manifest["selection"]
    return Selection(dict(stored["repository"]), dict(stored["report"]),
                     stored["batch"], [dict(m) for m in stored["members"]])


def find_captures(selection: Selection) -> dict[str, dict]:
    """Stored captures whose frozen selection matches this one exactly."""
    matches: dict[str, dict] = {}
    directory = root()
    if not directory.exists():
        return matches
    for path in sorted(directory.glob("*/manifest.json")):
        capture_id = path.parent.name
        try:
            manifest = read_manifest(capture_id)
        except EvidenceError:
            continue
        if manifest["selection"] == selection.as_json():
            matches[capture_id] = manifest
    return matches


# ---------------------------------------------------------------------------
# Reuse across associations: unchanged code is never downloaded twice
# ---------------------------------------------------------------------------

def carry_over_code(selection: Selection, manifest: dict) -> int:
    """Adopt complete code evidence from any capture of the same revision.

    Reuse is keyed to what a code observation actually claims - the repository
    identities and the full base/head revisions - never to the report digest, the
    batch ordinal, the reviewer prompt or the thread's `updated_at`. So a
    re-cluster, a renumbered batch, an edited prompt or ordinary thread churn
    changes the association and the mutable observations while leaving the diffs
    and file lists exactly where they are, with no network request at all.

    The adopted records keep their original acquisition time, URL and media type;
    what changes is the generation they are bound to, so the new association owns
    them and the old capture remains readable on its own terms.
    """
    adopted = 0
    donors: dict[int, tuple[str, dict, dict]] = {}
    directory = root()
    if not directory.exists():
        return 0
    for path in sorted(directory.glob("*/manifest.json")):
        donor_id = path.parent.name
        if donor_id == manifest["capture_id"]:
            continue
        try:
            donor = read_manifest(donor_id)
        except EvidenceError:
            continue
        index = index_sources(donor)
        for member in donor["selection"]["members"]:
            number = member["number"]
            if number in donors:
                continue
            if number not in selection.numbers:
                continue
            if revision_key(member) != revision_key(member_repo(number, selection)):
                continue
            donors[number] = (donor_id, donor, index)
    for number, (_donor_id, donor, index) in donors.items():
        for component in CODE_COMPONENTS:
            for group in COMPONENT_GROUPS[component]:
                source_state = group_entry(donor, number, component, group)
                if source_state["status"] != "complete" or not source_state["source_ids"]:
                    continue
                target = group_entry(manifest, number, component, group)
                if target["status"] == "complete":
                    continue
                for source_id in source_state["source_ids"]:
                    source = index.get(source_id)
                    if source is None:
                        break
                    clone = {key: value for key, value in source.items() if key != "id"}
                    clone["generation"] = manifest["generation"]
                    clone["id"] = evidence_digest({"generation": manifest["generation"],
                                                   "source": clone})
                    manifest["sources"].append(clone)
                    target["source_ids"].append(clone["id"])
                    adopted += 1
                target["pages"] = len(target["source_ids"])
                target["status"] = "complete"
                target["reason"] = None
                target["next_url"] = None
            roll_up(component_entry(manifest, number, component))
    return adopted



# ---------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------

def human(status: str) -> str:
    return {"complete": "complete", "partial": "partial", "missing": "missing",
            "blocked": "blocked"}.get(status, status)


def coverage_report(manifest: dict) -> dict:
    """Per-member and per-component coverage, from the recorded state alone."""
    members = []
    for member in manifest["selection"]["members"]:
        number = member["number"]
        components = []
        for entry in manifest["components"]:
            if entry["number"] != number:
                continue
            components.append({
                "component": entry["component"], "status": entry["status"],
                "pages": sum(state["pages"] for state in entry["groups"]),
                "reason": entry["reason"],
                "groups": [{"group": state["group"], "status": state["status"],
                            "pages": state["pages"], "reason": state["reason"]}
                           for state in entry["groups"]],
                "citations": sum(1 for c in manifest["citations"]
                                 if c["number"] == number and c["component"] == entry["component"]),
            })
        members.append({"number": number, "base_sha": member["base_sha"],
                        "head_sha": member["head_sha"], "components": components})
    return {"capture_id": manifest["capture_id"], "generation": manifest["generation"],
            "batch": manifest["selection"]["batch"]["id"],
            "observed_at": manifest["capture"]["observed_at"],
            "stop_reason": manifest["capture"].get("stop_reason"),
            "complete": is_complete(manifest),
            "requests": {key: manifest["capture"][key] for key in
                         ("request_limit", "requests_used", "reserved", "failures",
                          "retries", "identity_checks")},
            "bytes_stored": manifest["capture"].get("bytes_stored", 0),
            "citations": len(manifest["citations"]), "members": members}


def print_coverage(report: dict, stream=sys.stdout) -> None:
    state = "COMPLETE" if report["complete"] else "INCOMPLETE"
    print(f"capture {report['capture_id']}  batch {report['batch']}  "
          f"generation {report['generation'][:12]}…  {state}", file=stream)
    requests = report["requests"]
    print(f"  observed {report['observed_at']}  stop {report['stop_reason'] or 'none'}  "
          f"requests {requests['requests_used']}/{requests['request_limit']} "
          f"(failures {requests['failures']}, identity checks {requests['identity_checks']})",
          file=stream)
    print(f"  {report['bytes_stored']} bytes stored, {report['citations']} citations", file=stream)
    for member in report["members"]:
        print(f"  #{member['number']}  head {member['head_sha'][:12]}…  "
              f"base {member['base_sha'][:12]}…", file=stream)
        for component in member["components"]:
            detail = ""
            if component["status"] != "complete" and component["reason"]:
                detail = f" — {component['reason']}"
            groups = ", ".join(f"{g['group']}={g['status']}" for g in component["groups"])
            print(f"      {component['component']:<16} {human(component['status']):<9} "
                  f"{component['pages']:>3}p {component['citations']:>4}c  [{groups}]{detail}",
                  file=stream)


def resume_hint(capture_id: str, manifest: dict) -> str:
    batch = manifest["selection"]["batch"]["id"]
    return (f"resume: python3 tranche.py evidence capture --batch {batch} "
            f"--reuse-capture {capture_id}")


def resolve_current(batch_id: str, *, request_limit: int | None = None,
                    need_batch: bool = True):
    """Validate the current report and resolve the batch, or refuse with a reason."""
    report = report_loader.load()
    if not need_batch:
        return report, None
    return report, select(batch_id, report)


def cmd_capture(args) -> int:
    report, selection = resolve_current(args.batch)
    existing: dict[str, dict] = {}
    if not args.fresh:
        existing = find_captures(selection)
    capture_id = None
    if args.reuse_capture:
        manifest = read_manifest(args.reuse_capture)
        if manifest["selection"] != selection.as_json():
            raise SelectionError(
                f"capture {args.reuse_capture} belongs to a different selection; "
                "omit --reuse-capture to start a fresh one")
        capture_id = args.reuse_capture
    elif existing and not args.fresh:
        capture_id = sorted(existing)[0]
    if capture_id is None:
        capture_id = new_capture_id()
    lock = Lock(capture_id)
    lock.acquire(break_lock=args.break_lock)
    try:
        if capture_id in existing:
            manifest = existing[capture_id]
        elif manifest_path(capture_id).exists():
            manifest = read_manifest(capture_id)
        else:
            manifest = build_manifest(selection, capture_id, request_limit=args.request_budget)
            # First pass: adopt every unchanged, complete code observation from
            # any stored capture of the same revision, so nothing is downloaded
            # twice because a report, a batch or a prompt changed.
            adopted = carry_over_code(selection, manifest)
            if adopted:
                print(f"  reused {adopted} stored code source(s) for the unchanged revision(s)")
            write_manifest(capture_id, manifest)
        manifest["selection"] = selection.as_json()  # re-bind the association, keep bodies
        manifest["capture"]["request_limit"] = args.request_budget
        budget = ghread.Budget(args.request_budget, reserved=RESERVED_BUDGET)
        capture = Capture(selection, capture_id, manifest, budget=budget,
                          max_bytes=args.max_bytes, log=lambda message: print(f"  {message}"))
        print(f"capture {capture_id} for batch {args.batch} "
              f"({len(selection.members)} members, {len(COMPONENTS)} components each, "
              f"budget {args.request_budget})")
        if existing and not args.fresh:
            print(f"  resuming stored progress ({len(manifest['sources'])} sources already recorded)")
        stop = capture.run()
        manifest["citations"] = extract_citations(manifest)
        write_manifest(capture_id, manifest)
        if stop is None:
            removed = sweep_orphans()
            if removed:
                print(f"  swept {removed} unreferenced body file(s)")
        report_text = coverage_report(manifest)
    finally:
        lock.release()
    if args.json:
        print(json.dumps({"coverage": report_text,
                          "next": None if report_text["complete"] else
                          resume_hint(capture_id, manifest)},
                         ensure_ascii=False, allow_nan=False, indent=1))
    else:
        print_coverage(report_text)
        if not report_text["complete"]:
            print(f"  {resume_hint(capture_id, manifest)}")
    return EXIT_OK if report_text["complete"] else EXIT_INCOMPLETE


def current_association(batch_id: str) -> tuple[dict | None, str]:
    """The capture that currently matches this batch, or why there is none.

    Kept separate from historical inspection on purpose: `show --batch` must never
    present an older capture of the same ordinal as if it were the current one.
    """
    try:
        report, selection = resolve_current(batch_id)
    except report_loader.ReportError as exc:
        return None, f"the current report cannot be read ({exc})"
    except SelectionError as exc:
        return None, f"the current report refuses this batch ({exc})"
    matches = find_captures(selection)
    if not matches:
        return None, (f"no capture is associated with the current {batch_id}; run "
                      f"evidence capture --batch {batch_id}")
    if len(matches) > 1:
        newest = max(matches, key=lambda cid: matches[cid]["updated_at"])
        others = ", ".join(sorted(set(matches) - {newest}))
        return (matches[newest],
                f"several captures match the current {batch_id}; showing the newest "
                f"({newest}); also present: {others}")
    capture_id = next(iter(matches))
    return matches[capture_id], ""


def cmd_show(args) -> int:
    note = ""
    if args.capture:
        manifest = read_manifest(args.capture)  # historical: no report consulted at all
        if args.batch:
            stored = manifest["selection"]["batch"]["id"]
            if stored != args.batch:
                note = (f"capture {args.capture} belongs to batch {stored}, not {args.batch}; "
                        "showing the recorded capture")
    else:
        manifest, note = current_association(args.batch)
        if manifest is None:
            print(note, file=sys.stderr)
            return EXIT_REFUSED
    if args.source:
        windowed = window(manifest, args.source, start=args.start_byte, length=args.length)
        if args.json:
            print(json.dumps(windowed, ensure_ascii=False, allow_nan=False, indent=1))
        else:
            print(f"source {windowed['source_id']}  #{windowed['number']} "
                  f"{windowed['component']}/{windowed['group']} page {windowed['page']}")
            print(f"  {windowed['url']}  {windowed['media_type']}  "
                  f"{windowed['body_bytes']} bytes  sha256 {windowed['body_sha256']}")
            print(f"  bytes {windowed['start_byte']}..{windowed['end_byte']}"
                  + (f"  next --start-byte {windowed['next_start']}"
                     if windowed["next_start"] is not None else "  (end of source)"))
            sys.stdout.write(windowed["content"])
            if not windowed["content"].endswith("\n"):
                sys.stdout.write("\n")
        return EXIT_OK
    if args.citation:
        resolved = resolve_citation(manifest, args.citation)
        if args.json:
            print(json.dumps(resolved, ensure_ascii=False, allow_nan=False, indent=1))
        else:
            citation = resolved["citation"]
            print(f"citation {citation['id']}  #{citation['number']} {citation['component']}")
            print(f"  source {citation['source_id']}  bytes "
                  f"{citation['start_byte']}..{citation['end_byte']}  "
                  f"excerpt sha256 {resolved['excerpt_sha256']}")
            sys.stdout.write(resolved["excerpt"])
            if not resolved["excerpt"].endswith("\n"):
                sys.stdout.write("\n")
        return EXIT_OK
    report_text = coverage_report(manifest)
    if args.json:
        payload = {"coverage": report_text, "historical": bool(args.capture),
                   "note": note or None,
                   "sources": [{"id": s["id"], "number": s["number"],
                                "component": s["component"], "group": s["group"],
                                "page": s["page"], "bytes": body_path(s["body_sha256"]).exists()}
                               for s in manifest["sources"]],
                   "citations": manifest["citations"]}
        text = json.dumps(payload, ensure_ascii=False, allow_nan=False, indent=1)
        sys.stdout.write(text if len(text) <= DEFAULT_WINDOW_BYTES else
                         text[:DEFAULT_WINDOW_BYTES] + "\n… (truncated; use --source to read bytes)")
        print()
    else:
        if note:
            print(note)
        if args.capture:
            print("historical capture — nothing here is checked against the current report")
        print_coverage(report_text)
        if not report_text["complete"]:
            print(f"  {resume_hint(manifest['capture_id'], manifest)}")
    return EXIT_OK


def cmd_export(args) -> int:
    historical = bool(args.capture)
    if historical:
        manifest = read_manifest(args.capture)
        selection = stored_selection(manifest)
    else:
        manifest, note = current_association(args.batch)
        if manifest is None:
            print(note, file=sys.stderr)
            return EXIT_REFUSED
        selection = stored_selection(manifest)
    before = report_loader.input_digests() if not historical else None
    budget = ghread.Budget(max(args.request_budget, 4), reserved=2)
    gate = require_shareable(selection, budget)
    if historical and not args.allow_historical:
        print(f"capture {args.capture} is being exported as history: it claims no "
              f"current-batch compatibility and the report association it records is "
              f"the one from its own capture. Pass --allow-historical to confirm.",
              file=sys.stderr)
        return EXIT_USAGE
    if not historical and report_loader.input_digests() != before:
        raise SelectionError("report inputs changed during export; retry")
    data = export_bytes(manifest, selection, limit=args.max_bytes)
    if args.output == "-":
        sys.stdout.buffer.write(data)
        sys.stdout.buffer.write(b"\n")
        destination = "stdout"
    else:
        destination_path = Path(args.output)
        if destination_path.exists() and destination_path.is_dir():
            raise EvidenceError(f"{args.output} is a directory")
        atomic_bytes(destination_path, data)
        written = destination_path.read_bytes()
        if written != data:
            raise EvidenceError(f"exported bytes at {args.output} differ from what was written")
        destination = str(destination_path)
    summary = {"capture_id": manifest["capture_id"], "batch": selection.batch["id"],
               "historical": historical, "complete": is_complete(manifest),
               "bytes": len(data), "citations": len(manifest["citations"]),
               "sources": len(manifest["sources"]), "output": destination,
               "repository": gate["repository"]["full_name"],
               "visibility": gate["repository"]["visibility"],
               "linked": [repo["full_name"] for repo in gate["linked"]],
               "packet_digest": build_packet_digest(data)}
    if args.json:
        print(json.dumps(summary, ensure_ascii=False, allow_nan=False, indent=1))
    else:
        print(f"wrote {summary['bytes']} bytes to {destination} "
              f"({summary['sources']} sources, {summary['citations']} citations, "
              f"{'complete' if summary['complete'] else 'INCOMPLETE'})")
        print(f"  packet digest {summary['packet_digest']}")
        print(f"  sharing gate: {summary['repository']} is {summary['visibility']}"
              + (f"; linked public: {', '.join(summary['linked'])}" if summary["linked"] else ""))
    return EXIT_OK


def build_packet_digest(data: bytes) -> str:
    """The packet's own digest, read back from the serialized bytes."""
    try:
        packet = json.loads(data.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise EvidenceError(f"exported packet is not readable JSON ({exc})") from exc
    return packet.get("packet_digest", "")


def attach_commands(commands) -> None:
    """The three public operations, attached to whichever parser owns them."""
    capture = commands.add_parser("capture", help="acquire or resume a batch's public evidence")
    capture.add_argument("--batch", required=True, help="native batch id, e.g. B001")
    capture.add_argument("--request-budget", type=int, default=DEFAULT_REQUEST_BUDGET,
                         help=f"requests this run may attempt (default {DEFAULT_REQUEST_BUDGET})")
    capture.add_argument("--max-bytes", type=int, default=DEFAULT_MAX_BYTES,
                         help=f"bytes this capture may store (default {DEFAULT_MAX_BYTES})")
    capture.add_argument("--fresh", action="store_true",
                         help="start a new capture generation even if one is stored")
    capture.add_argument("--reuse-capture", default=None,
                         help="resume or extend one explicit capture id")
    capture.add_argument("--break-lock", action="store_true",
                         help="replace a writer lock whose process is gone")
    capture.add_argument("--json", action="store_true", help="machine-readable summary")

    show = commands.add_parser("show", help="inspect coverage, sources or citations offline")
    show.add_argument("--batch", default=None, help="native batch id: the current association")
    show.add_argument("--capture", default=None, help="explicit capture id: historical, offline")
    show.add_argument("--source", default=None, help="source id to read a byte window from")
    show.add_argument("--citation", default=None, help="citation id to resolve offline")
    show.add_argument("--start-byte", type=int, default=0, help="first byte of the window")
    show.add_argument("--length", type=int, default=DEFAULT_WINDOW_BYTES,
                      help=f"window size (max {DEFAULT_WINDOW_BYTES})")
    show.add_argument("--json", action="store_true", help="machine-readable output")

    export = commands.add_parser("export", help="write a self-contained packet")
    export.add_argument("--batch", default=None, help="native batch id: the current association")
    export.add_argument("--capture", default=None, help="explicit capture id: historical export")
    export.add_argument("--output", required=True, help="destination file, or - for stdout")
    export.add_argument("--allow-historical", action="store_true",
                        help="confirm an export of a capture that is not the current one")
    export.add_argument("--request-budget", type=int, default=8,
                        help="requests the sharing gate may attempt")
    export.add_argument("--max-bytes", type=int, default=DEFAULT_EXPORT_BYTES,
                        help=f"packet byte bound (default {DEFAULT_EXPORT_BYTES})")
    export.add_argument("--json", action="store_true", help="machine-readable summary")


def add_parser(sub) -> None:
    """Attach the `evidence` command tree to Tranche's existing entry point."""
    parser = sub.add_parser(
        "evidence", help="capture, inspect and export the evidence of a native batch (issue #9)")
    attach_commands(parser.add_subparsers(dest="evidence_cmd", required=True))


def validate(args) -> str | None:
    """Reject a combination this command cannot honour, with a usable message."""
    if args.evidence_cmd == "capture":
        if args.max_bytes <= 0:
            return "--max-bytes must be positive"
        if args.request_budget <= RESERVED_BUDGET:
            return (f"--request-budget must exceed the {RESERVED_BUDGET} held back for the "
                    "completion checks")
        return None
    if args.evidence_cmd in ("show", "export"):
        if bool(args.batch) == bool(args.capture):
            return ("give exactly one of --batch (the current association) or "
                    "--capture (historical, offline)")
    if args.evidence_cmd == "show":
        if args.source and args.citation:
            return "give at most one of --source or --citation"
        if args.length < 1 or args.length > DEFAULT_WINDOW_BYTES:
            return f"--length must be 1..{DEFAULT_WINDOW_BYTES}"
        if args.start_byte < 0:
            return "--start-byte must be nonnegative"
    if args.evidence_cmd == "export":
        if args.max_bytes <= 0:
            return "--max-bytes must be positive"
        if args.request_budget < 2:
            return ("--request-budget must be at least 2 so the sharing gate can check "
                    "the base repository and every linked one")
    return None


def dispatch(args) -> int:
    """Run one evidence command. Exit codes are stable for scripts:
    0 success, 1 unusable local state, 2 usage, 3 refused selection, 4 incomplete.
    """
    try:
        problem = validate(args)
        if problem:
            print(f"usage: {problem}", file=sys.stderr)
            return EXIT_USAGE
        if args.evidence_cmd == "capture":
            return cmd_capture(args)
        if args.evidence_cmd == "show":
            return cmd_show(args)
        if args.evidence_cmd == "export":
            return cmd_export(args)
        return EXIT_USAGE
    except (report_loader.ReportError, SelectionError) as exc:
        print(f"refused: {exc}", file=sys.stderr)
        return EXIT_REFUSED
    except IncompleteCapture as exc:
        print(f"incomplete: {exc}", file=sys.stderr)
        return EXIT_INCOMPLETE
    except (EvidenceError, ghread.ReadError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return EXIT_USABLE


def main(argv=None) -> int:
    """Standalone entry point: `python3 evidence.py <command>`."""
    parser = argparse.ArgumentParser(prog="evidence.py", description=__doc__)
    attach_commands(parser.add_subparsers(dest="evidence_cmd", required=True))
    return dispatch(parser.parse_args(argv))


if __name__ == "__main__":
    raise SystemExit(main())
