#!/usr/bin/env python3
"""Tranche — model-assisted PR discovery via TypeSafe Jev.

Groups observed open PRs of omacom/omarchy into review candidates, proposes
related groups, and flags items that may need follow-up — supporting the roll-up work DHH
asked the triage team to do (x.com/dhh/status/2098755120540393908).

Judgments come from TypeSafe's System One API (Jev). Code owns the workflow:
fetch -> judge (one batched call per PR) -> compare candidate pairs -> cluster.

Usage:
  python3 tranche.py refresh            # the whole pipeline, incremental, one command
  python3 tranche.py fetch              # refresh data/pages/snapshot.json
  python3 tranche.py judge [--limit N] [--resume]
  python3 tranche.py dupes [--max-pairs N]
  python3 tranche.py cluster            # writes out/clusters.json, out/dupes.json,
                                       #            out/tranches.md, out/summary.json
  python3 tranche.py batches            # writes out/batches.json (cumulative pre-release batches)
  python3 tranche.py all [--limit N]
"""

from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import math
import os
import random
import re
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from itertools import combinations
from pathlib import Path

ROOT = Path(__file__).resolve().parent
PAGES_DIR = ROOT / "data" / "pages"
OUT_DIR = ROOT / "out"
JUDGMENTS_PATH = OUT_DIR / "judgments.jsonl"
PAIRS_PATH = OUT_DIR / "pair_verdicts.jsonl"
KEY_FILE = Path.home() / "Documents" / "jevapi.txt"

API_URL = "https://api.typesafe.ai/v1/systemone"
MODEL = "jev-latest"
REPO = "omacom/omarchy"
BODY_CHARS = 1200
WORKERS = 6
BINDING_VERSION = 1
# Issue #3: security is a meta-category with top priority. A judgment at or above
# this probability enters the security-first queue, which outranks every category.
SECURITY_PRIORITY = 0.5


def digest(value) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                     ensure_ascii=False, allow_nan=False).encode()).hexdigest()


def atomic_json(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".pending-", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(value, stream, ensure_ascii=False, allow_nan=False)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)

# ---------------------------------------------------------------------------
# Jev questions — one batched call per PR (independent questions over one state
# run in parallel and are far cheaper than one call per question).
# ---------------------------------------------------------------------------

def judge_questions() -> dict:
    return {
        "category": {
            "type": "choice",
            "instructions": {
                "question": (
                    "Which area of Omarchy does this pull request mainly touch? "
                    "Read `pr.title` and `pr.body`; `pr.diffstat` shows the size. "
                    "Pick exactly one; use `unclear` only when the text is too thin to tell."
                )
            },
            "criteria": {
                "install-setup": "omarchy-setup menu, installer, first boot, ISO, dotfiles bootstrap",
                "desktop-config": "Hyprland, Walker, waybar, wlogout, mako, keybinds, wallpapers, theming",
                "user-experience": "a default/taste or look-and-feel proposal (themes, wallpapers, icons, fonts, bar or menu aesthetics); judge by the change's effect, not the subsystem it edits",
                "shell-cli": "zsh config, aliases, starship, CLI tool defaults, terminal usage",
                "apps-integrations": "default apps, mime handling, new application integrations (e.g. dropbox, spotify, 1password)",
                "hardware-drivers": "NVIDIA, wifi, bluetooth, audio, power, HiDPI, laptops, ARM/Snapdragon, firmware",
                "update-release": "omarchy-update, version bumps, release machinery, migration between versions, boot entries",
                "agents-ai": "AI coding agents, agent hooks, MCP, integrations for Claude/Codex/Gemini-like tools",
                "docs": "README, documentation, wiki, help text only",
                "fix-misc": "a real change that fits none of the above",
                "unclear": "cannot be placed from title and body alone",
            },
        },
        "risk": {
            "type": "score",
            "instructions": {
                "question": (
                    "How risky is merging this pull request for existing Omarchy installations? "
                    "Judge from `pr.title`, `pr.body` and `pr.diffstat`."
                )
            },
            "criteria": [
                "Text-only: docs, themes, wallpapers, menu definitions; nothing executes",
                "Config or script change that is scoped and revertible; no root-side effects at install/update time",
                "Runs as root during install or update, edits boot entries or mounts, or flips system-wide defaults",
                "Touches disk layout, networking, sudo/permissions, security posture, or kernel drivers/firmware",
                "Could break existing installs outright: data loss, boot failure, or user lockout",
            ],
        },
        "is_fix": {
            "type": "noul",
            "instructions": (
                "Is this pull request primarily a fix for a bug or regression, rather than "
                "a new feature, a default/taste change, or a refactor? Judge from `pr.title` and `pr.body`."
            ),
        },
        "dupe_signal": {
            "type": "noul",
            "instructions": (
                "Do `pr.title` or `pr.body` indicate this pull request duplicates another change, "
                "or is superseded by / supersedes one (another open PR, or already-merged upstream work)?"
            ),
        },
        "finished_form": {
            "type": "score",
            "instructions": {
                "question": (
                    "Is this pull request in finished, reviewable form as described by `pr.body` "
                    "(with `pr.title`)? DHH asked the triage team to ensure everything is ready "
                    "for consideration in a finished form."
                )
            },
            "criteria": [
                "Empty or near-empty body; no description of what or why",
                "Says what it does but not why, or shows no evidence it was tried",
                "Clear what and why; states that it was tested on a real system",
                "Clear what and why plus concrete QA evidence (before/after, screenshots, test steps); small and focused",
            ],
        },
        "review_effort": {
            "type": "score",
            "instructions": {
                "question": "How much reviewer effort does this pull request need, judging by `pr.diffstat` and the change described?"
            },
            "criteria": [
                "Trivial and mechanical: a typo, version number, or one-line constant",
                "Small: one focused change a reviewer can hold in their head",
                "Moderate: several related edits that must be checked together",
                "Substantial: architectural or many-part change needing deep review",
            ],
        },
        "security_flag": {
            "type": "noul",
            "instructions": (
                "Does this change touch credentials or secrets, download-and-execute remote code, "
                "sudo/permission changes, network exposure, or crypto material? Judge from `pr.title` and `pr.body`."
            ),
        },
    }


def pair_questions() -> dict:
    return {
        "sameness": {
            "type": "choice",
            "instructions": {
                "question": (
                    "Do `pr_a` and `pr_b` propose the same underlying change to Omarchy? "
                    "Judge by what they modify and the outcome, not by wording."
                )
            },
            "criteria": {
                "same_change": "two attempts at the same change; merging one makes the other redundant",
                "related_but_different": "same area or theme but distinct outcomes; both could merge",
                "unrelated": "different changes that merely share words",
            },
        },
    }


# ---------------------------------------------------------------------------
# TypeSafe HTTP
# ---------------------------------------------------------------------------

class TrancheFatal(RuntimeError):
    pass


def read_key() -> str:
    key = os.environ.get("TYPESAFE_API_KEY")
    if key:
        return key.strip()
    if KEY_FILE.exists():
        return KEY_FILE.read_text().strip()
    raise TrancheFatal(f"No API key: set TYPESAFE_API_KEY or create {KEY_FILE}")


def ask(state, questions: dict, key: str, timeout: int = 90) -> dict:
    payload = json.dumps({"state": state, "model": MODEL, "questions": questions}).encode()
    backoff = 2.0
    last = ""
    for _attempt in range(6):
        req = urllib.request.Request(
            API_URL,
            data=payload,
            headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return json.load(resp)
        except urllib.error.HTTPError as e:
            last = f"HTTP {e.code}: {e.read().decode(errors='replace')[:300]}"
            if e.code == 401:
                raise TrancheFatal(f"Auth rejected (401). Check the key. {last}") from e
            if e.code == 422:
                raise TrancheFatal(f"Request rejected (422) — question shape bug. {last}") from e
            if e.code in (429, 529) or e.code >= 500:
                time.sleep(backoff + random.random())
                backoff = min(backoff * 2, 60)
                continue
            raise TrancheFatal(last) from e
        except urllib.error.URLError as e:
            last = f"network: {e}"
            time.sleep(backoff + random.random())
            backoff = min(backoff * 2, 60)
    raise TrancheFatal(f"Retries exhausted. Last error: {last}")


# ---------------------------------------------------------------------------
# Data loading
# ---------------------------------------------------------------------------

def validate_pr(item):
    if (not isinstance(item, dict) or type(item.get("number")) is not int
            or item["number"] <= 0 or not isinstance(item.get("title"), str)
            or item.get("body") is not None and not isinstance(item["body"], str)):
        raise TrancheFatal("Invalid captured PR shape; fetch again")
    for field in ("head", "user", "author"):
        if item.get(field) is not None and not isinstance(item[field], dict):
            raise TrancheFatal(f"Invalid captured PR {field}; fetch again")
    labels = item.get("labels", [])
    if not isinstance(labels, list) or any(not isinstance(label, dict) or not isinstance(label.get("name"), str) for label in labels):
        raise TrancheFatal("Invalid captured PR labels; fetch again")


# Issue #10 — literal body references, repository-qualified (adapted from
# Reposition's `relations.py`). A mention selects a comparison candidate only
# when it names a pull request of the repository under review: a bare `#N` can
# only mean this repository, while a GitHub link or `owner/repo#N` names its own
# repository and must not be read as ours. Scanning order makes the reading
# explicit and non-overlapping — links first, then qualified tokens, then bare
# numbers that no earlier token already covers — so the `#123` inside
# `other/repo#123` or inside a pasted URL is never re-read on its own.
LINK_REFERENCE = re.compile(
    r"https?://(?:www\.)?github\.com/(?P<repo>[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)"
    r"/(?:issues|pull)/(?P<number>\d+)"
)
QUALIFIED_REFERENCE = re.compile(r"(?P<repo>[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)#(?P<number>\d+)")
BARE_REFERENCE = re.compile(r"(?<![\w#])#(?P<number>\d+)\b")


# Captured fields that carry this PR's own reviewable evidence. Everything else
# in a GitHub pull-list envelope churns without the PR changing: `base.repo`
# embeds repository-wide counters (stargazers, forks, open_issues, pushed_at,
# updated_at) that move on every unrelated event, `_links`/`statuses_url` are
# URL scaffolding, and `user`/`head` embed whole nested objects whose remote
# halves drift independently. Digesting the whole envelope made every PR look
# changed after every fetch.
PR_EVIDENCE_FIELDS = (
    "number", "title", "body", "created_at", "updated_at", "state", "draft",
    "labels", "milestone", "requested_reviewers", "requested_teams",
    "changed_files", "additions", "deletions", "commits", "comments",
    "review_comments", "merged_at", "merge_commit_sha", "user", "head",
)
# Nested objects inside the evidence set, reduced to the PR-owned half.
PR_EVIDENCE_PROJECTION = {
    "user": ("login",),
    "head": ("sha", "ref", "label"),
}


def pr_evidence_digest(item: dict) -> str:
    """Digest of the captured fields a judgment actually depends on."""
    evidence = {}
    for field in PR_EVIDENCE_FIELDS:
        if field not in item:
            continue
        value = item[field]
        keep = PR_EVIDENCE_PROJECTION.get(field)
        if keep and isinstance(value, dict):
            value = {key: value[key] for key in keep if key in value}
        elif keep and isinstance(value, list):
            value = [{key: part[key] for key in keep if key in part}
                     if isinstance(part, dict) else part for part in value]
        elif field == "labels":
            value = [label.get("name") if isinstance(label, dict) else label for label in value]
        evidence[field] = value
    return digest(evidence)


def reference_digest(item: dict) -> str:
    """Digest of the body references a PR contributes as comparison candidates.

    Kept out of `pr_evidence_digest`: refs come from the raw captured body, they
    are not part of any judgment or pair binding, and only `dupes` reads them.
    """
    raw = item.get("body") or ""
    number = item.get("number")
    return digest({"references": reference_numbers(raw, REPO, number if type(number) is int else None)})


def reference_numbers(raw_body: str, repository: str = REPO, own_number: int | None = None) -> list[int]:
    """PR numbers of `repository` literally mentioned in a description.

    Cross-repository mentions are dropped, never rewritten as bare numbers, and
    the PR's own number is discarded so a description can never pair a PR with
    itself.
    """
    spans: list[tuple[int, int]] = []
    numbers: set[int] = set()
    for pattern in (LINK_REFERENCE, QUALIFIED_REFERENCE):
        for match in pattern.finditer(raw_body):
            spans.append(match.span())
            if match["repo"].lower() == repository.lower():
                numbers.add(int(match["number"]))
    for match in BARE_REFERENCE.finditer(raw_body):
        if any(start <= match.start() < end for start, end in spans):
            continue
        numbers.add(int(match["number"]))
    if own_number is not None:
        numbers.discard(own_number)
    return sorted(numbers)


def load_prs() -> dict[int, dict]:
    prs: dict[int, dict] = {}
    snapshot = PAGES_DIR / "snapshot.json"
    if snapshot.exists():
        value = json.loads(snapshot.read_text())
        if (not isinstance(value, dict) or type(value.get("version")) is not int
                or value.get("version") != 1 or value.get("repo") != REPO
                or value.get("digest") != digest(value.get("items"))):
            raise TrancheFatal("Fetched snapshot identity or checksum differs; fetch again")
        pages = [value["items"]]
    else:
        # Existing/enriched page caches remain readable until the next fetch.
        pages = [json.loads(path.read_text()) for path in sorted(PAGES_DIR.glob("page_*.json"))]
    for page in pages:
        if not isinstance(page, list):
            raise TrancheFatal("Captured PR membership must be a list; fetch again")
        for p in page:
            validate_pr(p)
            if p["number"] in prs:
                raise TrancheFatal("Repeated PR in captured membership; fetch again")
            raw_body = p.get("body") or ""
            refs = reference_numbers(raw_body, REPO, p["number"])
            body = re.sub(r"<!--.*?-->", "", raw_body, flags=re.S)
            body = re.sub(r"!\[[^\]]*\]\([^)]*\)", "", body)  # images
            body = re.sub(r"https?://\S+", "", body)          # bare links
            body = re.sub(r"\s+", " ", body).strip()
            prs[p["number"]] = {
                "number": p["number"],
                "title": p["title"].strip(),
                "body": body[:BODY_CHARS],
                "author": ((p.get("user") or p.get("author") or {}).get("login", "unknown")),
                "created": p.get("created_at", ""),
                "updated": p.get("updated_at", ""),
                "draft": bool(p.get("draft")),
                "files": p.get("changed_files"),
                "additions": p.get("additions"),
                "deletions": p.get("deletions"),
                "labels": [label["name"] for label in p.get("labels", [])],
                "refs": refs,
                "head_sha": (p.get("head") or {}).get("sha"),
                "url": p.get("html_url") or f"https://github.com/{REPO}/pull/{p['number']}",
                "source_digest": digest(p),
                "evidence_digest": pr_evidence_digest(p),
                "ref_digest": digest({"references": refs}),
                "body_truncated": len(body) > BODY_CHARS,
            }
    return prs


def ref_index(prs) -> dict[int, list[int]]:
    """PR number → the references it contributes as comparison candidates."""
    return {n: pr.get("refs", []) for n, pr in prs.items()}


def pr_state(pr: dict) -> dict:
    sizes = [pr[key] for key in ("files", "additions", "deletions")]
    known = all(type(size) is int and size >= 0 for size in sizes)
    return {
        "pr": {
            "title": pr["title"],
            "body": pr["body"] or "(empty body)",
            "author": pr["author"],
            "diffstat": f"{sizes[0]} files changed, +{sizes[1]}/-{sizes[2]}" if known else "unknown (not supplied by the captured PR list)",
            "draft": pr["draft"],
            "evidence_basis": "title and shortened description; patches, CI and reproduction results not verified",
            "diffstat_available": known,
            "body_truncated": pr["body_truncated"],
        }
    }


# ---------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------

GITHUB_API = "https://api.github.com"


def fetch_page(url: str, transport: str) -> list:
    """One page of the open-PR list, over `gh` (preferred), curl or urllib.

    `gh api` is preferred because it is already authenticated: unauthenticated
    GitHub allows 60 requests an hour and one capture of this backlog costs
    about 29, so an unauthenticated refresh fails on the third pass. The token
    stays inside `gh` — it is never read by this process and never placed in an
    argv, where it would be visible to every user on the host via `ps`.
    """
    if transport == "gh":
        result = subprocess.run(["gh", "api", url], capture_output=True, text=True, timeout=120)
        if result.returncode:
            detail = result.stderr.strip().splitlines()[-1] if result.stderr.strip() else "no detail"
            raise TrancheFatal(f"gh api fetch failed ({detail}); previous snapshot retained")
        return json.loads(result.stdout)
    if transport == "curl":
        # The original host's urllib/IPv6 workaround. Deliberately unauthenticated.
        command = ["curl", "--fail", "--silent", "--show-error", "--max-time", "60", url]
        result = subprocess.run(command, capture_output=True, text=True)
        if result.returncode:
            detail = result.stderr.strip().splitlines()[-1] if result.stderr.strip() else "no detail"
            raise TrancheFatal(f"curl fetch failed ({detail}); previous snapshot retained")
        return json.loads(result.stdout)
    with urllib.request.urlopen(url, timeout=60) as response:
        return json.load(response)


def cmd_fetch(args) -> None:
    PAGES_DIR.mkdir(parents=True, exist_ok=True)
    page, total = 1, 0
    captured = []
    seen = set()
    transport = getattr(args, "transport", "gh")
    while True:
        url = f"{GITHUB_API}/repos/{REPO}/pulls?state=open&per_page=100&page={page}"
        arr = fetch_page(url, transport)
        if not isinstance(arr, list):
            raise TrancheFatal("GitHub did not return a PR list; previous snapshot retained")
        if not arr:
            break
        for item in arr:
            validate_pr(item)
            if item["number"] in seen:
                raise TrancheFatal("Invalid or repeated PR during pagination; previous snapshot retained")
            seen.add(item["number"])
        captured.extend(arr)
        total += len(arr)
        print(f"page {page}: {len(arr)} PRs (total {total})")
        if len(arr) < 100:
            break
        page += 1
        time.sleep(0.4)
    # One atomic membership commit, including an empty or exact-page result.
    # Failed/partial fetches leave the last committed observation untouched.
    atomic_json(PAGES_DIR / "snapshot.json", {
        "version": 1, "repo": REPO, "items": captured, "digest": digest(captured),
        "observed_at": datetime.now(timezone.utc).isoformat(),
    })
    print(f"fetched {total} observed open PRs into {PAGES_DIR}")


def finite_json(value):
    """Keep evidence JSON-shaped while removing non-standard numeric constants."""
    if isinstance(value, dict):
        return {key: finite_json(item) for key, item in value.items()}
    if isinstance(value, list):
        return [finite_json(item) for item in value]
    if isinstance(value, float) and not math.isfinite(value):
        return None
    return value


def normalized_record(record):
    record = {key: finite_json(value) for key, value in record.items()}
    usage = record.get("usage")
    usage = usage if isinstance(usage, dict) else {}
    record["usage"] = {field: usage.get(field) if type(usage.get(field)) is int
                       and usage[field] >= 0 else 0
                       for field in ("input_tokens", "output_tokens")}
    return record


def normalize_pair(record):
    record = normalized_record(record)
    policy = pair_questions().get("sameness")
    choices = policy.get("criteria", []) if isinstance(policy, dict) else []
    verdict = record.get("verdict")
    if not isinstance(verdict, str) or verdict not in choices:
        record["verdict"] = None
    probabilities = record.get("probabilities")
    probabilities = dict(probabilities) if isinstance(probabilities, dict) else {}
    for key, value in probabilities.items():
        if type(value) not in (int, float) or not 0 <= value <= 1 or not math.isfinite(value):
            probabilities[key] = None
    record["probabilities"] = probabilities
    probabilities["same_change"] = p_same(record)
    record["normalization_errors"] = [field + ": invalid or missing" for field, value in
        (("verdict", record.get("verdict")), ("probabilities.same_change", p_same(record)))
        if value is None]
    # The judgment block is what the report re-reads; the per-request token usage
    # is not. Keep it out of the cached record so cluster can read the multi-megabyte
    # log without holding every usage block in memory.
    if type(record.get("input_tokens")) is not int or type(record.get("output_tokens")) is not int:
        usage = record.get("usage") if isinstance(record.get("usage"), dict) else {}
        record.pop("input_tokens", None)
        record.pop("output_tokens", None)
        for field in ("input_tokens", "output_tokens"):
            value = usage.get(field)
            record[field] = value if type(value) is int and value >= 0 else 0
    record.pop("usage", None)
    return record


def pair_usage(record: dict) -> tuple[int, int]:
    """Tokens spent on one pair verdict, for a cached record or a raw API reply."""
    usage = record.get("usage")
    usage = usage if isinstance(usage, dict) else {}
    values = []
    for field in ("input_tokens", "output_tokens"):
        value = record.get(field)
        if type(value) is not int or value < 0:
            value = usage.get(field)
        values.append(value if type(value) is int and value >= 0 else 0)
    return values[0], values[1]


def reusable_pair(record):
    return record.get("verdict") is not None and p_same(record) is not None


def normalize_judgment(record):
    record = normalized_record(record)
    data = record.get("answers")
    data = data if isinstance(data, dict) else {}
    record["answers"] = data
    errors = []
    for name, question in judge_questions().items():
        answer = data.get(name)
        answer = dict(answer) if isinstance(answer, dict) else {}
        field = {"choice": "choice", "score": "score", "noul": "noul"}[question["type"]]
        if field == "choice":
            value = answer.get(field)
            valid = isinstance(value, str) and value in question["criteria"]
        else:
            value = metric(record, name, field)
            valid = value is not None
        if not valid:
            errors.append(f"answers.{name}.{field}: invalid or missing")
            value = None
        answer[field] = value
        data[name] = answer
    record["normalization_errors"] = errors
    return record


def judgment_is_current(pr, record) -> bool:
    """True when a stored judgment still answers today's question for today's evidence.

    The PR's own captured evidence and the model/question policy both bind a
    judgment, so a changed description or a newer model re-asks the question,
    while a re-fetch of unchanged evidence does not. The binding is the single
    currency test: it digests the full question policy, so a judgment made
    under an older question set is never presented as current. `input` and
    `requested_model` stay on records as provenance only. Completeness is not
    tested here: an answer with an unknown field is still the answer that was
    given, and dropping it would turn a known category into an unknown one.
    """
    return record.get("binding") == judgment_binding(pr)


def reusable_judgment(record):
    return (record.get("answers", {}).get("category", {}).get("choice") is not None
            and all(metric(record, name, field) is not None for name, field in (
                ("risk", "score"), ("finished_form", "score"), ("review_effort", "score"),
                ("is_fix", "noul"), ("security_flag", "noul"))))


def load_done() -> dict[int, dict]:
    """Latest usable judgment per PR, reading the cache without loading it twice.

    The log is append-only, so one PR can hold several lines; only the last
    valid record per number wins and a bounded tail of it is returned, because
    the full log has multi-megabyte `input` and `usage` blocks that no caller
    needs. A malformed tail (a truncated write) never costs the earlier records.
    """
    done: dict[int, dict] = {}
    if not JUDGMENTS_PATH.exists():
        return done
    with JUDGMENTS_PATH.open(encoding="utf-8") as stream:
        for line in stream:
            if "\"number\"" not in line:
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            if (isinstance(rec, dict) and "error" not in rec
                    and type(rec.get("number")) is int and rec["number"] > 0):
                done[rec["number"]] = rec
    return {number: normalize_judgment(rec) for number, rec in done.items()}


def judgment_binding(pr):
    return digest({"version": BINDING_VERSION, "repo": REPO,
                   "source": pr["evidence_digest"], "state": pr_state(pr),
                   "questions": judge_questions(), "model": MODEL})


def current_judgments(prs, *, allow_unbound=False):
    current = {}
    for number, record in load_done().items():
        if number not in prs:
            continue
        matches = judgment_is_current(prs[number], record)
        legacy = "binding" not in record and allow_unbound
        if matches or legacy:
            current[number] = dict(record, freshness="current" if matches else "unbound")
    return current


def brief(pr):
    return {"number": pr["number"], "title": pr["title"], "body": pr["body"][:400],
            "evidence_basis": "shortened descriptions only; source equivalence not verified"}


def pair_binding(prs, a, b):
    a, b = sorted((a, b))
    return digest({"version": BINDING_VERSION, "repo": REPO, "model": MODEL,
                   "sources": [prs[a]["evidence_digest"], prs[b]["evidence_digest"]],
                   "state": [brief(prs[a]), brief(prs[b])], "questions": pair_questions()})


def current_pairs(prs, judgments, *, allow_unbound=False):
    """Pair verdicts a report may publish.

    A verdict is published when the pair's current descriptions still verify
    against the record (binding, or the byte-identical inputs it was shown) —
    that is the same strict rule as before, and it never invents a relationship.
    A pair whose similarity has since fallen below the discovery threshold is
    still published when the model actually judged it: the threshold finds
    candidates, it does not get to hide a verified dupe. Legacy records without a
    binding are inspection-only behind `allow_unbound`.
    """
    pairs = {}
    for pair, record in current_verdicts(prs).items():
        a, b = pair
        if a not in judgments or b not in judgments:
            continue
        matches = pair_is_current(prs, pair, record)
        legacy = "binding" not in record and allow_unbound
        if matches or legacy:
            pairs[pair] = dict(record, a=a, b=b,
                               freshness="current" if matches else "unbound")
    return list(pairs.values())


def pair_cache(prs, judgments) -> dict[tuple[int, int], dict]:
    """Every captured pair with a stored verdict, newest first.

    The reuse predicate for `dupes` is `reusable_pair` + `pair_is_current`, so a
    verdict survives a refetch that does not move the evidence it was based on
    and is replaced the moment evidence, model or policy changes.
    """
    return {pair: dict(record, a=pair[0], b=pair[1],
                       freshness="current" if pair_is_current(prs, pair, record) else "stale")
            for pair, record in current_verdicts(prs).items()}


def cmd_judge(args) -> None:
    prs = load_prs()
    done = {n: rec for n, rec in current_judgments(prs).items()
            if reusable_judgment(rec)} if args.resume else {}
    if not args.resume:
        JUDGMENTS_PATH.parent.mkdir(parents=True, exist_ok=True)
        JUDGMENTS_PATH.write_text("")
    JUDGMENTS_PATH.parent.mkdir(parents=True, exist_ok=True)
    todo = [n for n in sorted(prs, reverse=True) if n not in done]
    if args.limit:
        todo = todo[: args.limit]
    print(f"{len(prs)} PRs, {len(done)} already judged, {len(todo)} to go")
    if not todo:
        return
    key = read_key()
    lock = threading.Lock()
    errors: list[str] = []
    tokens_in = tokens_out = 0

    def work(number: int) -> None:
        nonlocal tokens_in, tokens_out
        pr = prs[number]
        try:
            resp = ask(pr_state(pr), judge_questions(), key)
            if not isinstance(resp, dict):
                resp = {}
            rec = {"number": number, "title": pr["title"], "answers": resp.get("answers"), "usage": resp.get("usage", {}),
                   "binding": judgment_binding(pr), "input": pr_state(pr),
                   "source_digest": pr["evidence_digest"], "head_sha": pr["head_sha"],
                   "updated_at": pr["updated"], "requested_model": MODEL,
                   "judged_at": datetime.now(timezone.utc).isoformat(),
                   "resolved_model": resp.get("model"), "request_id": resp.get("request_id")}
        except TrancheFatal as e:
            with lock:
                errors.append(f"#{number}: {e}")
            if "401" in str(e):
                raise
            return
        rec = normalize_judgment(rec)
        with lock:
            with JUDGMENTS_PATH.open("a") as f:
                f.write(json.dumps(rec, allow_nan=False) + "\n")
            tokens_in += rec["usage"]["input_tokens"]
            tokens_out += rec["usage"]["output_tokens"]
            n = len(done) + 1
            done[number] = rec
            if n % 25 == 0:
                print(f"judged {n}/{len(todo)}  (in {tokens_in} tok / out {tokens_out} tok)")

    try:
        with ThreadPoolExecutor(max_workers=WORKERS) as ex:
            futures = [ex.submit(work, n) for n in todo]
            for f in as_completed(futures):
                f.result()
    except TrancheFatal as e:
        print(f"FATAL: {e}", file=sys.stderr)
        print("partial progress is saved; re-run with --resume", file=sys.stderr)
        sys.exit(2)

    print(f"done: {len(done)} matching judgments; errors: {len(errors)}")
    for e in errors[:10]:
        print("  " + e)
    print(f"tokens: in={tokens_in} out={tokens_out}")


def load_judgments() -> dict[int, dict]:
    return load_done()


def current_verdicts(prs) -> dict[tuple[int, int], dict]:
    """Newest stored verdict per pair, for pairs whose members are both captured.

    The log is append-only, so the same pair can appear many times; only the last
    valid record wins. Records written by older code without a `binding` are kept
    as unbound evidence, exactly as before.
    """
    verdicts: dict[tuple[int, int], dict] = {}
    if not PAIRS_PATH.exists():
        return verdicts
    with PAIRS_PATH.open(encoding="utf-8") as stream:
        for line in stream:
            if "\"a\"" not in line:
                continue
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue
            if not isinstance(record, dict) or "error" in record:
                continue
            if any(type(record.get(key)) is not int or record[key] <= 0 for key in ("a", "b")):
                continue
            pair = (min(record["a"], record["b"]), max(record["a"], record["b"]))
            if pair[0] == pair[1] or pair[0] not in prs or pair[1] not in prs:
                continue
            verdicts[pair] = normalize_pair(record)
    return verdicts


def pair_is_current(prs, pair, record) -> bool:
    """True when a stored verdict still answers today's question for today's evidence.

    Same rule as `judgment_is_current`: the binding is the single currency test
    and it digests the full question policy, so a verdict made under an older
    question set is never presented as current. A moved branch that leaves the
    two descriptions alone still matches the binding. `input` and
    `requested_model` are provenance.
    """
    a, b = pair
    return record.get("binding") == pair_binding(prs, a, b)


def lexical_pairs(prs, judgments, refs=None, threshold=0.72,
                  jaccard_threshold=0.62) -> list[tuple[float, int, int]]:
    """Candidate duplicate pairs: high title similarity within the same judged category."""

    def toks(s):
        return set(re.findall(r"[a-z0-9]+", s.lower())) - {
            "the", "a", "an", "and", "or", "to", "of", "for", "in", "on", "with", "fix", "add",
        }

    by_cat: dict[str, list[int]] = defaultdict(list)
    for n, j in judgments.items():
        cat = category(j)
        by_cat[cat].append(n)
    pairs = []
    for nums in by_cat.values():
        nums = sorted(nums)
        for i, a in enumerate(nums):
            ta = prs[a]["title"].lower()
            for b in nums[i + 1:]:
                tb = prs[b]["title"].lower()
                ratio = difflib.SequenceMatcher(None, ta, tb).ratio()
                if ratio >= threshold:
                    pairs.append((ratio, a, b))
                    continue
                A, B = toks(prs[a]["title"]), toks(prs[b]["title"])
                if A and B:
                    jac = len(A & B) / len(A | B)
                    if jac >= jaccard_threshold:
                        pairs.append((jac, a, b))
    pairs.sort(reverse=True)
    seen = {(a, b) for _, a, b in pairs}

    def pair_key(a, b):
        return (min(a, b), max(a, b))

    # Cross-referenced PRs (body cites each other) are candidate duplicates even
    # when titles differ — e.g. fixes to the same bug split across files.
    for n, pr in prs.items():
        if n not in judgments or (refs and n not in refs):
            continue
        for r in (refs or {}).get(n, pr.get("refs", [])):
            a, b = pair_key(n, r)
            if a != b and a in prs and b in prs and a in judgments and b in judgments and (a, b) not in seen:
                seen.add((a, b))
                pairs.append((1.0, a, b))
    pairs.sort(key=lambda t: -t[0])
    return pairs


def candidate_pairs(prs, judgments=None, refs=None) -> dict[tuple[int, int], float]:
    """The pairs worth a model verdict, as {(a, b): similarity}.

    Similarity alone cannot be trusted as a proxy: the candidate set also
    carries the title-independent body-reference edges, and it moves whenever
    one side's title, category or refs change. So the pair cache is keyed by
    membership in this exact set, not by a single pair's own score.
    """
    if judgments is not None:
        scored = lexical_pairs(prs, judgments, refs=refs)
    else:
        scored = lexical_pairs(prs, current_judgments(prs), refs=refs)
    return {(a, b): score for score, a, b in scored}


def outstanding_pairs(prs, judgments) -> list[tuple[float, int, int]]:
    """The exact work a `dupes` pass would do, best candidate first.

    Defined once and consumed by both `dupes` and `refresh --dry-run`, so the
    reported count can never disagree with the work performed. A stored verdict
    that is still current is not work; neither is a pair that has stopped being
    a candidate (its similarity fell below the threshold or its body reference
    was removed) even though its verdict is now stale — re-running it would be
    work that never converges, because nothing asks for that verdict.
    """
    cache = pair_cache(prs, judgments)
    done = {pair for pair, rec in cache.items()
            if reusable_pair(rec) and rec["freshness"] == "current"}
    candidates = candidate_pairs(prs, judgments, ref_index(prs))
    work = [(score, a, b) for (a, b), score in candidates.items() if (a, b) not in done]
    work.sort(key=lambda item: -item[0])
    return work


def cmd_dupes(args) -> None:
    prs = load_prs()
    judgments = current_judgments(prs)
    missing = len(prs) - len(judgments)
    if missing > 100:
        print(f"warning: {missing} PRs not judged yet; dupe pass runs on judged subset", file=sys.stderr)
    candidates = candidate_pairs(prs, judgments, ref_index(prs))
    pending = outstanding_pairs(prs, judgments)
    print(f"{len(pending[:args.max_pairs])} candidate pairs to compare "
          f"(skipping {len(candidates) - len(pending)} matching records)")
    pairs = pending[: args.max_pairs]
    if not pairs:
        return
    PAIRS_PATH.parent.mkdir(parents=True, exist_ok=True)
    key = read_key()
    lock = threading.Lock()
    tokens_in = tokens_out = 0

    def work(item):
        nonlocal tokens_in, tokens_out
        s, a, b = item
        if a not in prs or b not in prs:
            return  # candidate became stale (PR closed and refetched mid-run)
        resp = ask({"pr_a": brief(prs[a]), "pr_b": brief(prs[b])}, pair_questions(), key)
        resp = resp if isinstance(resp, dict) else {}
        data = resp.get("answers")
        data = data if isinstance(data, dict) else {}
        answer = data.get("sameness")
        answer = answer if isinstance(answer, dict) else {}
        rec = {
            "a": a, "b": b, "similarity": round(s, 3),
            "verdict": answer.get("choice"),
            "probabilities": answer.get("probabilities"),
            "usage": resp.get("usage", {}),
            "binding": pair_binding(prs, a, b),
            "requested_model": MODEL, "resolved_model": resp.get("model"),
            "request_id": resp.get("request_id"),
            "judged_at": datetime.now(timezone.utc).isoformat(),
            "input": {"pr_a": brief(prs[a]), "pr_b": brief(prs[b])},
        }
        rec = normalize_pair(rec)
        with lock:
            with PAIRS_PATH.open("a") as f:
                f.write(json.dumps(rec, allow_nan=False) + "\n")
            tokens_in += pair_usage(rec)[0]
            tokens_out += pair_usage(rec)[1]

    with ThreadPoolExecutor(max_workers=WORKERS) as ex:
        futures = [ex.submit(work, it) for it in pairs]
        for i, f in enumerate(as_completed(futures)):
            f.result()
            if (i + 1) % 25 == 0:
                print(f"compared {i + 1}/{len(pairs)}")
    print("candidate pair comparison complete")
    print(f"tokens: in={tokens_in} out={tokens_out}")


class DSU:
    def __init__(self):
        self.parent = {}

    def find(self, x):
        self.parent.setdefault(x, x)
        while self.parent[x] != x:
            self.parent[x] = self.parent[self.parent[x]]
            x = self.parent[x]
        return x

    def union(self, a, b):
        ra, rb = self.find(a), self.find(b)
        if ra != rb:
            self.parent[rb] = ra


def metric(judgment, name, field="score"):
    """Absent, non-finite or out-of-range model values are unknown, never zero."""
    answer = (judgment.get("answers") or {}).get(name)
    value = answer.get(field) if isinstance(answer, dict) else None
    ceiling = 1 if field == "noul" else (4 if name == "risk" else 3)
    if type(value) not in (int, float) or not 0 <= value <= ceiling or not math.isfinite(value):
        return None
    return value


def category(judgment):
    answer = (judgment.get("answers") or {}).get("category")
    value = answer.get("choice") if isinstance(answer, dict) else None
    return value if isinstance(value, str) and value in judge_questions()["category"]["criteria"] else "unclear"


def p_same(pair):
    probabilities = pair.get("probabilities")
    value = probabilities.get("same_change") if isinstance(probabilities, dict) else None
    return value if type(value) in (int, float) and 0 <= value <= 1 and math.isfinite(value) else None


def pair_classification(pair):
    """Separate genuine contradictions from the human-review threshold band."""
    probability = p_same(pair)
    verdict = pair.get("verdict")
    if probability is None or verdict not in ("same_change", "related_but_different", "unrelated"):
        return "malformed"
    if verdict == "same_change":
        return "same" if probability >= 0.65 else ("contradictory" if probability < 0.35 else "uncertain")
    return "different" if probability < 0.35 else ("contradictory" if probability >= 0.65 else "uncertain")


def accepted_pair(pair):
    return pair_classification(pair) == "same"


def duplicate_groups(verdicts):
    """Connectivity proposes groups; every internal relationship remains visible."""
    dsu = DSU()
    indexed = {(v["a"], v["b"]): v for v in verdicts}
    for v in verdicts:
        if accepted_pair(v):
            dsu.union(v["a"], v["b"])
    connected = defaultdict(list)
    for number in sorted(dsu.parent):
        connected[dsu.find(number)].append(number)
    consistent, review = [], []
    for members in sorted(connected.values(), key=lambda g: (-len(g), g)):
        if len(members) < 2:
            continue
        conflicts, uncertain, missing = [], [], []
        unbound = False
        for a, b in combinations(members, 2):
            pair = indexed.get((a, b))
            if pair is None:
                missing.append([a, b])
                continue
            unbound |= pair.get("freshness") != "current"
            if accepted_pair(pair):
                continue
            classification = pair_classification(pair)
            diagnostic = {"a": a, "b": b, "verdict": pair.get("verdict"),
                          "p_same": p_same(pair), "classification": classification}
            # Strong difference evidence conflicts with the proposed group;
            # a self-contradictory model response also requires relationship review.
            if classification in ("different", "contradictory"):
                conflicts.append(diagnostic)
            else:
                uncertain.append(diagnostic)
        if conflicts or uncertain or missing or unbound:
            review.append({"members": members, "conflicting_pairs": conflicts,
                           "uncertain_pairs": uncertain, "missing_pairs": missing,
                           "unbound_evidence": unbound})
        else:
            consistent.append(members)
    return consistent, review


def review_candidate(pr, judgment, grouped):
    required = [metric(judgment, "risk"), metric(judgment, "finished_form"),
                metric(judgment, "is_fix", "noul"), metric(judgment, "security_flag", "noul"),
                metric(judgment, "review_effort")]
    if (judgment.get("freshness") != "current" or not reusable_judgment(judgment) or pr["draft"]
            or pr["number"] in grouped or any(value is None for value in required)):
        return False
    risk, finished, fix, security, _ = required
    return risk <= 1.5 and finished >= 1.8 and fix >= 0.6 and security < 0.5


def escalated(judgment):
    risk, security = metric(judgment, "risk"), metric(judgment, "security_flag", "noul")
    return (risk is not None and risk >= 3) or (security is not None and security >= 0.5)


def security_priority(judgment):
    """Issue #3: a cross-cutting meta-category ranked above every other category.

    Membership uses the same 0.5 probability bar as senior escalation; unlike a
    category choice it never replaces the PR's own area. Unknown stays unknown.
    """
    security = metric(judgment, "security_flag", "noul")
    return security is not None and security >= SECURITY_PRIORITY


# ---------------------------------------------------------------------------
# Issue #4 — pre-release batches. A batch is 5 PRs merged together as one
# tranche. Jev determines the composition: model-consistent same_change groups
# are atomic units (their PRs combine into ONE pull request inside the batch),
# remaining PRs fill the batch up to five. Units are ordered security-first,
# then average model risk, then age. Batches are strictly disjoint — every PR
# belongs to at most one batch — and review groups (contradictory or untested
# internal evidence) are excluded entirely so no PR is claimed twice.
# ---------------------------------------------------------------------------

BATCH_SIZE = 5

# Issue #8: park is a hold with a named unblock path, never a close. A parked
# PR re-enters automatically: the author pushes, the judgment re-binds, the
# next refresh re-packs. Closing remains a maintainer decision.
PARK_UNBLOCK = {
    "draft": "Author marks the pull request ready for review; the next fetch "
             "recaptures it and the next batches run re-packs it.",
    "finished_form": "Author adds the missing description or QA evidence; "
                     "judge --resume re-binds the judgment and the PR re-enters "
                     "on the next refresh.",
    "unjudged_or_stale": "Run judge --resume (or refresh) to restore a current "
                         "judgment; the PR re-enters on the next batches run.",
    "same_change_hold": "Held with its same_change group: atomic units are never "
                        "split, so the group re-enters together when every member "
                        "clears its own park reason.",
}


def park_set(prs, judgments):
    """Issue #8: pipeline-legible park decisions, keyed by PR number.

    Reasons come only from facts the pipeline already holds — the draft bit,
    the finished-form score, the judgment's freshness. One predicate serves
    every consumer (batches, parked.json, workbench, MCP). A finished form
    just above 1 is not parked; a missing judgment parks under its own reason.
    """
    parks = {}
    for n, item in sorted(prs.items()):
        reasons = []
        if item.get("draft"):
            reasons.append("draft")
        judgment = judgments.get(n)
        if judgment is None:
            reasons.append("unjudged_or_stale")
        else:
            finished = metric(judgment, "finished_form")
            if finished is not None and finished <= 1:
                reasons.append("finished_form")
        if reasons:
            parks[n] = reasons
    return parks


def parked_payload(parks, prs, judgments, dupes_digest):
    """First-class park record: the security meta-category's discipline, second
    instance. Membership never replaces the PR's own category."""
    members = []
    for n in sorted(parks):
        item = prs.get(n, {})
        members.append({
            "number": n, "title": item.get("title", ""), "author": item.get("author", ""),
            "reasons": list(parks[n]),
            "unblock": " ".join(PARK_UNBLOCK[reason] for reason in parks[n]),
            "head_sha": item.get("head_sha"), "url": item.get("url"),
            "created": item.get("created", ""),
            "security_flag": metric(judgments.get(n, {}), "security_flag", "noul"),
        })
    return {
        "format_version": 1, "repo": REPO, "dupes_digest": dupes_digest,
        "parked": len(members),
        "meaning": "Parked before batching (issue #8): drafts, PRs without finished "
                   "form, PRs without a current judgment, and same_change groups "
                   "holding for a parked member. A hold with a named unblock path, "
                   "never a close; re-entry is automatic when the reason clears.",
        "members": members,
    }


def batch_review_prompt(members, batch_id, prs):
    """Reviewer agent prompt: point it at every PR of the batch and demand a
    methodical unified proposal + plan. Deterministic, self-contained text."""
    listed = "\n".join(f"- #{n}: {prs[n]['title']} — {prs[n]['url']}" for n in members
                       if n in prs)
    return (
        f"You are reviewing Omarchy pre-release batch {batch_id} ({len(members)} PRs to be "
        f"merged together as one tranche).\n\n"
        f"Pull requests in this batch:\n{listed}\n\n"
        "Work through the batch methodically:\n"
        "1. Read every PR fully — description, diff, and review comments. For PRs Jev flagged "
        "as the same change, verify they truly overlap and identify the strongest implementation "
        "of each.\n"
        "2. Map dependencies between the PRs (shared files, ordering constraints, conflicts) and "
        "check each PR's CI status.\n"
        "3. Produce ONE unified proposal for the batch: what merges, in which order, what gets "
        "squashed or dropped, and why — as a single coherent plan, not per-PR verdicts.\n"
        "4. Verify the plan: does the combined result still build and pass tests? Any PR that "
        "cannot be verified stays out — say so explicitly.\n"
        "5. Deliver: (a) the unified proposal, (b) a step-by-step merge plan with exact commands, "
        "(c) risks with mitigations, (d) an explicit list of anything excluded and why.\n\n"
        "Facts over plausibility: base every claim on the actual diffs and CI state, never on "
        "titles alone. You are proposing — the human decides."
    )


def park_state(dupes, judgments, prs):
    """One park predicate for every consumer: own reasons plus same_change_hold
    for every member of a confirmed group that waits for a parked member.
    Atomic units are never split — pulling one member out would leave the rest
    of the group claimed twice or held entirely."""
    parks = park_set(prs, judgments)
    for members in dupes["confirmed_groups"]:
        if any(m in parks for m in members):
            for m in members:
                entry = parks.setdefault(m, [])
                if "same_change_hold" not in entry:
                    entry.append("same_change_hold")
    return parks


def pr_activity(pr, record):
    """Head evidence is distinct from GitHub thread churn (including bots).

    A matching recorded head establishes an idle lower bound at judgment time.
    An unjudged PR has only its creation date as an age proxy. GitHub's
    updated_at is displayed as thread metadata, never used for priority.
    """
    bound_head = record.get("head_sha")
    head_moved = bool(bound_head and pr.get("head_sha") and bound_head != pr["head_sha"])
    idle_since = None if head_moved else (
        record.get("judged_at") if bound_head and bound_head == pr.get("head_sha")
        else pr.get("created"))
    return {"head_moved": head_moved, "idle_since": idle_since,
            "idle_basis": "unknown" if head_moved or not idle_since else
                          "judgment" if bound_head and bound_head == pr.get("head_sha")
                          and record.get("judged_at") else "creation",
            "thread_updated": pr.get("updated")}


def merge_batches(dupes, judgments, prs, dupes_digest):
    """Pack PRs into security-first batches of five, Jev-determined.

    Issue #8: parked PRs never enter a batch. A batch prompt claims exactly one
    thing — these PRs merge together — so a draft, a PR without finished form,
    or a PR without a current judgment must not appear in it."""
    parks = park_state(dupes, judgments, prs)

    def security_count(members):
        return sum(1 for n in members
                   if (metric(judgments.get(n, {}), "security_flag", "noul") or 0) >= SECURITY_PRIORITY)

    def unit_stats(members):
        risks = [value for n in members if (value := metric(judgments.get(n, {}), "risk")) is not None]
        return (security_count(members),
                round(sum(risks) / len(risks), 2) if risks else None,
                min((prs[n]["created"] for n in members if n in prs), default=""))

    grouped, units = set(), []
    for members in dupes["confirmed_groups"]:
        if any(m in parks for m in members):
            grouped.update(members)
            continue  # the whole atomic unit waits; it re-enters together
        grouped.update(members)
        security, average_risk, created = unit_stats(members)
        units.append({"members": list(members), "same_change": True, "security": security,
                      "risk": average_risk, "created": created})
    excluded = set()
    for group in dupes["review_groups"]:
        excluded.update(group["members"])
    for n in sorted(prs):
        if n not in grouped and n not in excluded and n not in parks:
            security, average_risk, created = unit_stats([n])
            units.append({"members": [n], "same_change": False, "security": security,
                          "risk": average_risk, "created": created})
    # Security outranks all risk bands; within a band, the longest evidenced
    # quiet head goes first. The newest bound in an atomic unit is its floor.
    def unit_priority(unit):
        activity = [pr_activity(prs[n], judgments.get(n, {})) for n in unit["members"]]
        moving = any(item["head_moved"] for item in activity)
        idle = max((item["idle_since"] or "9999" for item in activity), default="9999")
        risk = unit["risk"]
        band = 3 if risk is None else 0 if risk <= 1.5 else 1 if risk <= 2.5 else 2
        return (-unit["security"], band, moving, idle, unit["created"], unit["members"][0])

    units.sort(key=unit_priority)
    packed, current = [], []
    for unit in units:
        if current and len(current) + len(unit["members"]) > BATCH_SIZE:
            packed.append(current)
            current = []
        current.extend(unit["members"])
    if current:
        packed.append(current)
    if {n for members in packed for n in members} & set(parks):
        raise TrancheFatal("internal error: a parked PR entered a batch (issue #8 gate failed)")
    batches = []
    for ordinal, members in enumerate(packed, 1):
        security, average_risk, created = unit_stats(members)
        groups = sum(1 for unit in units if unit["same_change"] and set(unit["members"]) <= set(members))
        batches.append({
            "ordinal": ordinal, "id": f"B{ordinal:03d}",
            "members": members, "count": len(members),
            "target": "merged together as one tranche",
            "same_change_groups": groups,
            "security_members": security, "average_risk": average_risk, "created": created,
            "review_prompt": batch_review_prompt(members, f"B{ordinal:03d}", prs),
        })
    return {
        "format_version": 3, "repo": REPO, "dupes_digest": dupes_digest,
        "batch_size": BATCH_SIZE,
        "meaning": "A batch is 5 PRs merged together as one tranche (issue #4). Jev determines the "
                   "composition: same_change groups are atomic and combine into ONE pull request "
                   "inside their batch. Batches are disjoint: every PR belongs to at most one batch. "
                   "Ordered security-first, then risk band and evidenced idle lower bound. "
                   "Parked PRs are excluded before packing (issue #8). "
                   "Model-suggested, not verified safe to merge.",
        "batches": batches,
        "security_batches": sum(1 for b in batches if b["security_members"] > 0),
        "same_change_groups": sum(b["same_change_groups"] for b in batches),
        "excluded_review_prs": len(excluded),
        "parked_prs": len(parks),
    }


def cmd_batches(args) -> None:
    """Write out/batches.json and the tranches.md batch plan (issue #4)."""
    summary = json.loads((OUT_DIR / "summary.json").read_text())
    dupes = json.loads((OUT_DIR / "dupes.json").read_text())
    clusters = json.loads((OUT_DIR / "clusters.json").read_text())
    if summary.get("format_version") != 2 or summary.get("repo") != REPO:
        raise TrancheFatal("Unrecognized cluster observation; run cluster first")
    digests = summary.get("output_digests", {})
    if digests.get("clusters.json") != digest(clusters):
        raise TrancheFatal("clusters.json does not match the recorded digest; run cluster first")
    if digests.get("dupes.json") != digest(dupes):
        raise TrancheFatal("dupes.json does not match the recorded digest; run cluster first")
    prs = load_prs()
    judgments = current_judgments(prs)
    batches = merge_batches(dupes, judgments, prs, digests["dupes.json"])
    parks = park_state(dupes, judgments, prs)
    atomic_json(OUT_DIR / "batches.json", batches)
    atomic_json(OUT_DIR / "parked.json", parked_payload(parks, prs, judgments, digests["dupes.json"]))
    append_batch_plan(batches)
    if parks:
        append_park_section(parks, prs)
    print(f"{len(batches['batches'])} batches of ≤ {batches['batch_size']} PRs "
          f"({batches['security_batches']} security-first, {batches['same_change_groups']} "
          f"same-change groups, {batches['excluded_review_prs']} review-group PRs excluded, "
          f"{batches['parked_prs']} parked)")
    print(f"wrote {OUT_DIR}/batches.json, {OUT_DIR}/parked.json")


def append_batch_plan(batches) -> None:
    """Append the suggested batch plan to the published tranches.md."""
    path = OUT_DIR / "tranches.md"
    if not path.exists():
        return  # cluster owns the report; batches only append its plan section.
    lines = ["", "# Suggested pre-release batches (issue #4)", "",
             f"A batch is **{batches['batch_size']} PRs merged together as one tranche**. Jev determines",
             "the composition: model-consistent same_change groups are atomic — their PRs combine into",
             "ONE pull request inside the batch. Batches are disjoint (every PR is in at most one",
             "batch); review groups are excluded on purpose. Ordered security-first, then",
             "risk band and evidenced head idle time. Model-suggested, never verified safe to merge.", "",
             f"Batches: {len(batches['batches'])} · security-first batches: "
             f"{batches['security_batches']} · same-change groups: "
             f"{batches['same_change_groups']} · review-group PRs excluded: "
             f"{batches['excluded_review_prs']}.", ""]
    if batches["batches"]:
        lines += ["| Batch | Size | Security | Avg risk | Groups | Members |",
                  "|---|---|---|---|---|---|"]
        for batch in batches["batches"]:
            members = " ".join(f"#{n}" for n in batch["members"])
            risk = "unknown" if batch["average_risk"] is None else f"{batch['average_risk']:.1f}"
            lines.append(f"| {batch['id']} | {batch['count']} | "
                         f"{batch['security_members'] or '—'} | {risk} | "
                         f"{batch['same_change_groups'] or '—'} | {members} |")
        lines += ["", "## Reviewer agent prompts", "",
                  "Copy-paste a prompt into an agent to start a thorough, methodical review that",
                  "produces one unified proposal + merge plan for the batch.", ""]
        for batch in batches["batches"]:
            lines += [f"### {batch['id']}", "", "```", batch["review_prompt"], "```", ""]
    text = "\n".join(lines).rstrip("\n") + "\n"
    with path.open("a", encoding="utf-8") as stream:
        stream.write(text)


def append_park_section(parks, prs) -> None:
    """Append the park record (issue #8) to the published tranches.md.
    Batches append; cluster owns the file."""
    path = OUT_DIR / "tranches.md"
    if not path.exists():
        return
    lines = ["", "# Parked before batching (issue #8)", "",
             f"{len(parks)} PRs are parked: drafts, PRs without finished form, PRs without a",
             "current judgment, and same_change groups holding for a parked member. Park is a",
             "**hold with a named unblock path, never a close** — re-entry is automatic when the",
             "reason clears and the next refresh re-packs. No batch lists a parked PR.",
             "",
             f"Reasons: draft {sum('draft' in r for r in parks.values())} · "
             f"finished_form {sum('finished_form' in r for r in parks.values())} · "
             f"unjudged_or_stale {sum('unjudged_or_stale' in r for r in parks.values())} · "
             f"same_change_hold {sum('same_change_hold' in r for r in parks.values())}.", "",
             "| PR | Reasons | Unblocked by |", "|---|---|---|"]
    for n, reasons in sorted(parks.items()):
        unblock = " ".join(PARK_UNBLOCK[reason] for reason in reasons)
        lines.append(f"| #{n} | {', '.join(reasons)} | {unblock} |")
    lines += ["", "Parked does not remove a security-flagged PR from the security meta-category;",
              "it only removes it from merge batches. Full record: out/parked.json.", ""]
    text = "\n".join(lines).rstrip("\n") + "\n"
    with path.open("a", encoding="utf-8") as stream:
        stream.write(text)


def report_binding(prs, judgments, verdicts):
    return digest({"version": BINDING_VERSION, "repo": REPO,
                   "sources": {n: {"source": pr["source_digest"],
                                   "evidence": pr["evidence_digest"],
                                   "references": pr["ref_digest"]}
                               for n, pr in prs.items()},
                   "judgments": judgments, "pairs": verdicts,
                   "questions": [judge_questions(), pair_questions()], "model": MODEL})


def cmd_cluster(args) -> None:
    prs = load_prs()
    allow_unbound = getattr(args, "allow_unbound", False)
    judgments = current_judgments(prs, allow_unbound=allow_unbound)
    verdicts = current_pairs(prs, judgments, allow_unbound=allow_unbound)
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    dupe_groups, review_groups = duplicate_groups(verdicts)
    in_group = {n for g in dupe_groups for n in g} | {n for g in review_groups for n in g["members"]}
    uncertain_pairs = [{"a": v["a"], "b": v["b"], "p_same": p_same(v),
                        "similarity": v.get("similarity"), "verdict": v.get("verdict"),
                        "classification": pair_classification(v)}
                       for v in verdicts if pair_classification(v) in
                       ("uncertain", "contradictory", "malformed")]
    uncertain_pairs.sort(key=lambda v: -(v["p_same"] if v["p_same"] is not None else -1))

    clusters = {}
    tranches = defaultdict(list)
    security_review = []
    follow_up, escalate = [], []
    for n, j in sorted(judgments.items()):
        risk = metric(j, "risk")
        band = "unknown" if risk is None else ("low" if risk <= 1.5 else ("core" if risk <= 2.5 else "danger"))
        cat = category(j)
        item = {"number": n, "title": prs[n]["title"], "author": prs[n]["author"],
                "risk": risk, "finished_form": metric(j, "finished_form"),
                "is_fix": metric(j, "is_fix", "noul"), "review_effort": metric(j, "review_effort"),
                "security_flag": metric(j, "security_flag", "noul"), "freshness": j["freshness"],
                "superseded_by": None, "source_digest": prs[n]["source_digest"],
                "evidence_digest": prs[n]["evidence_digest"],
                "head_sha": prs[n]["head_sha"], "url": prs[n]["url"]}
        clusters.setdefault(cat, {}).setdefault(band, []).append(item)
        if review_candidate(prs[n], j, in_group):
            tranches[cat].append(item)
        if security_priority(j):
            security_review.append(item)
        finished = metric(j, "finished_form")
        if finished is not None and finished <= 1 and n not in in_group:
            follow_up.append(n)
        if escalated(j):
            escalate.append(n)
    # Issue #3: the security meta-category is a first-class output key, ordered
    # before every category when consumers read clusters.json.
    security_review.sort(key=lambda it: (-(it["security_flag"]
                         if it["security_flag"] is not None else -1), it["number"]))
    clusters_payload = {"security-review": security_review, **clusters}
    tokens = Counter()
    for record in [*judgments.values(), *verdicts]:
        for field in ("input_tokens", "output_tokens"):
            value = record.get("usage", {}).get(field)
            if type(value) is int and value >= 0:
                tokens[field] += value

    dupes = {"confirmed_groups": dupe_groups, "review_groups": review_groups,
             "uncertain_pairs": uncertain_pairs,
             "meaning": "Model-consistent candidate groups, not verified duplicates. No survivor selected."}
    summary = {
        "format_version": 2, "repo": REPO, "prs_in_corpus": len(prs), "judged": len(judgments),
        "unjudged_or_stale": len(prs) - len(judgments),
        "unbound_judgments": sum(j["freshness"] == "unbound" for j in judgments.values()),
        "allow_unbound": allow_unbound, "report_binding": report_binding(prs, judgments, verdicts),
        "dupe_groups": len(dupe_groups), "review_groups": len(review_groups),
        "uncertain_pairs": len(uncertain_pairs), "prs_in_dupe_groups": len(in_group),
        "superseded": 0, "ready_tranches": len(tranches),
        "ready_prs": sum(map(len, tranches.values())),
        "recommendation_kind": "review-candidates", "needs_author_followup": len(follow_up),
        "escalate_review": len(escalate), "security_priority": len(security_review),
        "unknown_risk_or_security": sum(metric(j, "risk") is None or metric(j, "security_flag", "noul") is None
                                        for j in judgments.values()),
        "tokens": {"input": tokens["input_tokens"], "output": tokens["output_tokens"]},
        "output_digests": {"clusters.json": digest(clusters_payload), "dupes.json": digest(dupes)},
    }
    # Issue #3: the security meta-category ships with top priority — its own key
    # inside clusters.json, sorted probability-first, before any category output.
    atomic_json(OUT_DIR / "clusters.json", clusters_payload)
    atomic_json(OUT_DIR / "dupes.json", dupes)
    lines = [
        "# Tranche — PR review candidates", "",
        f"Corpus: {len(prs)} observed open PRs; {len(judgments)} matching judgments; "
        f"{summary['unjudged_or_stale']} unjudged/stale; {summary['unbound_judgments']} unbound legacy judgments.",
        f"Review candidates: {summary['ready_prs']}. Model-consistent groups: {len(dupe_groups)}. "
        f"Groups needing relationship review: {len(review_groups)}. "
        f"Security-priority items: {summary['security_priority']} (meta-category, reviewed first).", "",
        "Evidence: titles and shortened descriptions (1200 characters per PR; 400 per pair). "
        "Diffstat is unknown unless captured input supplies it. Patches, CI, reproductions, "
        "fix coverage and security have not been verified. Model scores are suggestions, "
        "not calibrated guarantees or approval to merge/close. Pagination records an observation, not a point-in-time GitHub snapshot.", "",
    ]
    if allow_unbound:
        lines += ["LEGACY INSPECTION: unbound judgments cannot establish freshness or enter review-candidate tranches.", ""]
    def cell(value):
        return str(value).replace("|", "\\|").replace("\n", " ")
    def candidate_row(it):
        return (f"| [#{it['number']}]({it['url']}) | {cell(it['title'][:80])} | {cell(it['author'])} | "
                f"{it['finished_form']:.1f} | {it['review_effort']:.1f} | {it['is_fix']:.2f} |")
    CANDIDATE_TABLE = ("| PR | Title | Author | Model finished | Model effort | Model fix |\n"
                       "|---|---|---|---|---|---|")
    if security_review:
        # Issue #3: security ranks above every category in the report.
        lines += ["## Security review — top priority (meta-category)", "",
                  "These PRs touch credentials, remote code execution, sudo/permissions, network",
                  "exposure or crypto material (model probability ≥ "
                  f"{SECURITY_PRIORITY}). Review before any category batch.", "",
                  CANDIDATE_TABLE, *[candidate_row(it) for it in security_review], ""]
    for cat, candidates in sorted(tranches.items(), key=lambda t: -len(t[1])):
        lines += [f"## Review candidates: {cat} — {len(candidates)} PRs", "", CANDIDATE_TABLE,
                  *[candidate_row(it) for it in candidates]]
        lines.append("")
    for label, groups in (("Model-consistent candidate groups — verify fix coverage; no survivor selected", dupe_groups),
                          ("Candidate groups needing relationship review", review_groups)):
        if groups:
            lines += [f"## {label}", ""]
            for group in groups:
                members = group if isinstance(group, list) else group["members"]
                lines.append("- " + ", ".join(f"#{n}" for n in members))
                if isinstance(group, dict):
                    for field in ("conflicting_pairs", "uncertain_pairs", "missing_pairs"):
                        if group[field]:
                            lines.append(f"  - {field}: {json.dumps(group[field])}")
                    if group["unbound_evidence"]:
                        lines.append("  - Unbound legacy evidence; revisions cannot be checked.")
            lines.append("")
    if uncertain_pairs:
        lines += ["## Uncertain pairs — human comparison needed", ""]
        for pair in uncertain_pairs:
            lines.append(f"- #{pair['a']} ↔ #{pair['b']}: P(same)={pair['p_same']}; "
                         f"verdict={pair['verdict']}; {pair['classification']}")
        lines.append("")
    for label, numbers in (("Escalate for risk/security review", escalate),
                           ("Possible author follow-up — verify before requesting changes", follow_up)):
        if numbers:
            lines += [f"## {label}", "", *[f"- #{n} {cell(prs[n]['title'])}" for n in numbers], ""]
    (OUT_DIR / "tranches.md").write_text("\n".join(lines), encoding="utf-8")
    # Commit the report manifest last. The renderer rejects mixed generations.
    atomic_json(OUT_DIR / "summary.json", summary)
    print(json.dumps(summary, indent=1))
    print(f"\nwrote {OUT_DIR}/clusters.json dupes.json tranches.md summary.json")


def cmd_refresh(args) -> None:
    """Deterministic incremental refresh: the documented one-command pipeline.

    Runs the same steps in the same order every time and reuses everything the
    caches can still support, so the work is proportional to what actually
    changed since the last pass rather than to the corpus size.
    """
    steps = [step for step in ("fetch", "judge", "dupes", "cluster", "batches", "page")
             if not (step == "page" and args.no_page)]
    before = None
    summary_path = OUT_DIR / "summary.json"
    if summary_path.exists():
        try:
            before = json.loads(summary_path.read_text())
        except json.JSONDecodeError:
            before = None
    batches_path = OUT_DIR / "batches.json"
    before_batches = None
    if batches_path.exists():
        try:
            before_batches = json.loads(batches_path.read_text())
        except json.JSONDecodeError:
            before_batches = None

    if args.dry_run:
        prs = load_prs()
        judgments = current_judgments(prs)
        pending = outstanding_pairs(prs, judgments)
        print(f"refresh plan ({' -> '.join(steps)}), no changes written:")
        print(f"  judged now          {len(judgments)}/{len(prs)} captured PRs")
        print(f"  pair verdicts to re-run  {len(pending)} of "
              f"{len(candidate_pairs(prs, judgments, ref_index(prs)))} candidates")
        if "fetch" in steps:
            print("  fetch               would re-read open-PR membership from GitHub")
        return

    print(f"refresh: {' -> '.join(steps)}")
    for step in steps:
        if step == "fetch":
            cmd_fetch(argparse.Namespace(transport="gh"))
        elif step == "judge":
            cmd_judge(argparse.Namespace(resume=True, limit=None))
        elif step == "dupes":
            cmd_dupes(argparse.Namespace(max_pairs=args.max_pairs))
        elif step == "cluster":
            cmd_cluster(argparse.Namespace(allow_unbound=False))
        elif step == "batches":
            cmd_batches(argparse.Namespace())
        elif step == "page":
            render_page()

    after = json.loads(summary_path.read_text()) if summary_path.exists() else {}
    after_batches = json.loads(batches_path.read_text()) if batches_path.exists() else {}
    print("\nrefresh complete")
    for key in ("prs_in_corpus", "judged", "dupe_groups", "review_groups",
                "uncertain_pairs", "ready_prs", "escalate_review", "security_priority"):
        old, new = (before or {}).get(key), after.get(key)
        marker = "" if old == new else f"   (was {old})"
        print(f"  {key:<22} {new}{marker}")
    old, new = (before_batches or {}).get("parked_prs"), after_batches.get("parked_prs")
    marker = "" if old == new else f"   (was {old})"
    print(f"  {'parked_prs':<22} {new}{marker}")


def render_page() -> None:
    """Render docs/index.html through gen_page.py, refusing inconsistent inputs."""
    result = subprocess.run([sys.executable, str(ROOT / "gen_page.py")],
                            capture_output=True, text=True)
    sys.stdout.write(result.stdout)
    if result.returncode:
        sys.stderr.write(result.stderr)
        raise TrancheFatal("page rendering failed; out/ inputs were not rewritten")


def main() -> None:
    # Imported here, not at module scope: `evidence` imports tranche, and the
    # pipeline must stay importable without pulling the evidence service in.
    # Under __main__ the same module object is `__main__`, so importing `tranche`
    # here would create a second copy of this module's state; importing from
    # `tranche` when it is already `__main__` avoids that by construction.
    import evidence

    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    fetch = sub.add_parser("fetch", help="refresh observed open-PR membership from GitHub")
    fetch.add_argument("--transport", choices=("gh", "urllib", "curl"), default="gh",
                       help="gh (authenticated via the gh CLI, default), curl or urllib")
    j = sub.add_parser("judge", help="Jev judgment pass over PRs")
    j.add_argument("--limit", type=int, default=None, help="judge only the N newest unjudged PRs")
    j.add_argument("--resume", action="store_true", help="reuse judgments bound to unchanged input/questions/model")
    d = sub.add_parser("dupes", help="compare candidate pairs with Jev")
    d.add_argument("--max-pairs", type=int, default=300)
    cluster = sub.add_parser("cluster", help="build candidate groups and review reports offline")
    cluster.add_argument("--allow-unbound", action="store_true", help="inspect legacy judgments with freshness warnings; no legacy review tranches")
    sub.add_parser("batches", help="classify review candidates into cumulative pre-release batches (issue #4)")
    refresh = sub.add_parser("refresh", help="deterministic incremental refresh: fetch → judge → dupes → cluster → batches → page")
    refresh.add_argument("--max-pairs", type=int, default=400,
                         help="cap model pair comparisons this pass (default 400)")
    refresh.add_argument("--no-page", action="store_true", help="skip rendering docs/index.html")
    refresh.add_argument("--dry-run", action="store_true", help="report what would change without writing")
    a = sub.add_parser("all", help="judge --resume, dupes, cluster, batches")
    a.add_argument("--limit", type=int, default=None)
    a.add_argument("--resume", action="store_true", help="accepted for compatibility; all always resumes")
    a.add_argument("--max-pairs", type=int, default=300)
    evidence.add_parser(sub)
    args = ap.parse_args()
    if getattr(args, "limit", None) is not None and args.limit <= 0:
        ap.error("--limit must be positive")
    if getattr(args, "max_pairs", 0) < 0:
        ap.error("--max-pairs must be nonnegative")
    if args.cmd == "evidence":
        # Evidence is a separate, read-only service; its exit codes are its own.
        raise SystemExit(evidence.dispatch(args))
    if args.cmd == "fetch":
        cmd_fetch(args)
    elif args.cmd == "judge":
        cmd_judge(args)
    elif args.cmd == "dupes":
        cmd_dupes(args)
    elif args.cmd == "cluster":
        cmd_cluster(args)
    elif args.cmd == "batches":
        cmd_batches(args)
    elif args.cmd == "refresh":
        cmd_refresh(args)
    elif args.cmd == "all":
        args.resume = True
        cmd_judge(args)
        cmd_dupes(args)
        cmd_cluster(args)
        cmd_batches(args)


if __name__ == "__main__":
    main()
