"""Offline executable specification for the *proposed* evidence packet v1.

Test support only: no capture, persistence, network, or production CLI API.
"""
import base64
import binascii
import hashlib
import json
import math
import re
from datetime import datetime
from urllib.parse import parse_qs, urlsplit

FORMAT = "tranche.evidence-packet/v1"
PROFILE = "pr-review/v1"
COMPONENTS = ("metadata", "diff", "files", "discussion", "review_comments",
              "reviews", "checks", "closing_issues")
MAX_PACKET_BYTES = 1024 * 1024  # Small conformance profile, not a CLI default.


def require(condition, message):
    if not condition:
        raise ValueError(message)


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False, allow_nan=False).encode("utf-8")


def digest(value):
    return hashlib.sha256(encode(value)).hexdigest()


def byte_digest(value):
    return hashlib.sha256(value).hexdigest()


def fields(value, names):
    require(type(value) is dict and set(value) == set(names.split()), "fields")


def integer(value, minimum=0):
    require(type(value) is int and value >= minimum, "integer")


def sha(value, length=64):
    require(type(value) is str and re.fullmatch(f"[0-9a-f]{{{length}}}", value), "digest")


def timestamp(value):
    require(type(value) is str and re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", value),
            "UTC timestamp")
    datetime.strptime(value, "%Y-%m-%dT%H:%M:%S%z")


def seal(packet):
    packet["packet_digest"] = digest({k: v for k, v in packet.items() if k != "packet_digest"})
    return packet


def generation(selection, capture_id):
    return digest({"format": FORMAT, "profile": PROFILE, "selection": selection,
                   "capture_id": capture_id})


def source_id(generation_id, source):
    return digest({"generation": generation_id, "source":
                   {k: v for k, v in source.items() if k != "id"}})


def parse(data, max_bytes=MAX_PACKET_BYTES):
    """Reject ambiguous JSON and bound the complete serialized input first."""
    require(type(data) is bytes and len(data) <= max_bytes, "packet byte limit")

    def unique_pairs(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, "duplicate JSON key")
            result[key] = value
        return result

    def invalid_constant(value):
        raise ValueError("non-finite JSON number")

    def finite_float(value):
        number = float(value)
        require(math.isfinite(number), "non-finite JSON number")
        return number

    return json.loads(data.decode("utf-8"), object_pairs_hook=unique_pairs,
                      parse_constant=invalid_constant, parse_float=finite_float)


def validate(packet, expected_selection=None, max_bytes=MAX_PACKET_BYTES):
    """Validate bytes/relationships; expected_selection comes from bound reports.

    Without it this proves internal consistency only, never current report state.
    """
    try:
        return _validate(packet, expected_selection, max_bytes)
    except (KeyError, TypeError, AttributeError, OverflowError, RecursionError) as exc:
        raise ValueError("malformed packet") from exc


def _validate(packet, expected_selection, max_bytes):
    require(len(encode(packet)) <= max_bytes, "packet byte limit")
    fields(packet, "format profile selection membership_digest capture_id generation complete capture "
           "coverage sources citations packet_digest")
    require(packet["format"] == FORMAT and packet["profile"] == PROFILE, "version/profile")
    require(packet["packet_digest"] == digest(
        {k: v for k, v in packet.items() if k != "packet_digest"}), "packet digest")
    selection = packet["selection"]
    fields(selection, "repository report batch members")
    repository = selection["repository"]
    fields(repository, "id full_name visibility")
    integer(repository["id"], 1)
    require(type(repository["full_name"]) is str and re.fullmatch(
        r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository["full_name"]), "repository name")
    require(repository["visibility"] == "public", "public repository required")
    report = selection["report"]
    fields(report, "report_binding output_digests")
    sha(report["report_binding"])
    fields(report["output_digests"], "clusters.json dupes.json batches.json parked.json")
    for name, value in report["output_digests"].items():
        if name != "parked.json" or value is not None:
            sha(value)
    # Copy the selected native batch unchanged, including the exact prompt.
    batch = selection["batch"]
    require(type(batch) is dict and {"id", "ordinal", "members", "count", "review_prompt"}
            <= set(batch), "batch fields")
    integer(batch["ordinal"], 1)
    require(batch["id"] == f"B{batch['ordinal']:03d}", "batch ordinal")
    require(type(batch["review_prompt"]) is str and bool(batch["review_prompt"]), "prompt")
    require(type(selection["members"]) is list and bool(selection["members"]), "members")
    numbers = []
    for member in selection["members"]:
        fields(member, "number source_digest evidence_digest base_sha head_sha updated_at")
        integer(member["number"], 1)
        numbers.append(member["number"])
        for name in ("source_digest", "evidence_digest"):
            sha(member[name])
        for name in ("base_sha", "head_sha"):
            sha(member[name], 40)
        timestamp(member["updated_at"])
    require(len(set(numbers)) == len(numbers), "duplicate member")
    require(type(batch["members"]) is list and all(type(n) is int for n in batch["members"])
            and batch["members"] == numbers, "ordered batch membership")
    integer(batch["count"], 1)
    require(batch["count"] == len(numbers), "batch count")
    require(packet["membership_digest"] == digest(numbers), "membership digest")
    sha(packet["capture_id"], 32)
    require(packet["generation"] == generation(selection, packet["capture_id"]), "generation binding")
    if expected_selection is not None:
        require(encode(selection) == encode(expected_selection), "stale/foreign report selection")
    capture = packet["capture"]
    fields(capture, "observed_at request_limit requests_used stop_reason")
    timestamp(capture["observed_at"])
    integer(capture["request_limit"])
    integer(capture["requests_used"])
    require(capture["requests_used"] <= capture["request_limit"], "request budget")
    require(capture["stop_reason"] in (None, "request_budget", "storage_budget", "interrupted",
                                       "transport_error", "visibility_revoked", "revision_drift"),
            "stop reason")
    if capture["stop_reason"] == "request_budget":
        require(capture["requests_used"] == capture["request_limit"], "unexhausted request budget")
    require(type(packet["sources"]) is list, "sources")
    sources, slots, source_bytes = {}, set(), {}
    for source in packet["sources"]:
        fields(source, "id number component page cursor next_cursor url media_type captured_at "
               "body_base64 body_sha256")
        integer(source["number"], 1)
        require(source["number"] in numbers and source["component"] in COMPONENTS, "source scope")
        integer(source["page"], 1)
        slot = (source["number"], source["component"], source["page"])
        require(slot not in slots and source["id"] not in sources, "duplicate source")
        slots.add(slot)
        for name in ("cursor", "next_cursor"):
            require(source[name] is None or (type(source[name]) is str and bool(source[name])),
                    "cursor")
        # Reference URLs are labels only, never fetched by this specification.
        require(type(source["url"]) is str and not any(c in source["url"] for c in "\r\n"), "source URL")
        url = urlsplit(source["url"])
        prefix = f"/repos/{repository['full_name']}/"
        require(url.scheme == "https" and url.netloc == "api.github.com" and not url.fragment
                and (url.path.startswith(prefix) or url.path == "/graphql"), "source URL")
        require(not {key.lower() for key in parse_qs(url.query)} &
                {"token", "access_token", "authorization", "api_key", "client_secret"}, "credential URL")
        require(source["media_type"] in ("application/json", "text/plain", "text/x-diff"),
                "media type")
        timestamp(source["captured_at"])
        require(source["captured_at"] <= capture["observed_at"], "source observed in future")
        require(type(source["body_base64"]) is str, "source body")
        try:
            body = base64.b64decode(source["body_base64"], validate=True)
        except (ValueError, binascii.Error) as exc:
            raise ValueError("source base64") from exc
        require(base64.b64encode(body).decode("ascii") == source["body_base64"], "canonical base64")
        require(byte_digest(body) == source["body_sha256"], "source bytes digest")
        require(source["id"] == source_id(packet["generation"], source), "source generation")
        sources[source["id"]], source_bytes[source["id"]] = source, body
    require(type(packet["coverage"]) is list, "coverage")
    covered, referenced, completed = set(), set(), []
    for component in packet["coverage"]:
        fields(component, "number component status source_ids next_cursor reason")
        integer(component["number"], 1)
        key = (component["number"], component["component"])
        require(key[0] in numbers and key[1] in COMPONENTS and key not in covered, "coverage scope")
        covered.add(key)
        require(component["status"] in ("missing", "partial", "complete", "blocked"), "coverage status")
        require(type(component["source_ids"]) is list, "source ids")
        pages = []
        for index, source_key in enumerate(component["source_ids"], 1):
            require(type(source_key) is str and source_key in sources, "missing source")
            source = sources[source_key]
            require((source["number"], source["component"], source["page"]) == (*key, index),
                    "page binding/order")
            require(source["cursor"] == (pages[-1]["next_cursor"] if pages else None), "cursor chain")
            if pages:
                require(pages[-1]["next_cursor"] is not None, "page after terminal")
            pages.append(source)
            referenced.add(source_key)
        cursors = [p["cursor"] for p in pages]
        require(len(set(cursors)) == len(cursors), "pagination cursor cycle")
        end = pages[-1]["next_cursor"] if pages else None
        require(end is None or end not in cursors, "pagination cursor cycle")
        require(component["next_cursor"] == end, "resume cursor")
        if component["status"] == "complete":
            require(bool(pages) and end is None and component["reason"] is None, "incomplete pagination")
        else:
            require(type(component["reason"]) is str and bool(component["reason"]), "missing reason")
            if component["status"] == "missing":
                require(not pages, "missing has sources")
            if component["status"] == "partial":
                require(bool(pages) and end is not None, "partial needs continuation")
        completed.append(component["status"] == "complete")
    require(covered == {(n, c) for n in numbers for c in COMPONENTS}, "missing coverage component")
    require(referenced == set(sources), "orphan source")
    require(type(packet["complete"]) is bool and packet["complete"] == all(completed), "completeness")
    require(not packet["complete"] or capture["stop_reason"] is None, "complete but stopped")
    require(type(packet["citations"]) is list, "citations")
    citation_ids = set()
    for citation in packet["citations"]:
        fields(citation, "id source_id source_sha256 start_byte end_byte excerpt_sha256")
        require(type(citation["id"]) is str and bool(citation["id"])
                and citation["id"] not in citation_ids, "citation id")
        citation_ids.add(citation["id"])
        require(type(citation["source_id"]) is str and citation["source_id"] in sources, "citation source")
        body = source_bytes[citation["source_id"]]
        integer(citation["start_byte"])
        integer(citation["end_byte"], 1)
        require(citation["start_byte"] < citation["end_byte"] <= len(body), "citation range")
        require(citation["source_sha256"] == byte_digest(body), "citation source digest")
        require(citation["excerpt_sha256"] == byte_digest(
            body[citation["start_byte"]:citation["end_byte"]]), "citation excerpt digest")
    return packet


def validate_resume(previous, resumed):
    """A resumed observation can add pages; it cannot rewrite prior evidence."""
    validate(previous)
    require(previous["capture"]["stop_reason"] not in ("revision_drift", "visibility_revoked"),
            "capture invalidated; fresh capture required")
    validate(resumed, previous["selection"])
    require(resumed["generation"] == previous["generation"], "resume generation")
    require(resumed["capture"]["observed_at"] >= previous["capture"]["observed_at"], "resume time")
    for name in ("sources", "citations"):
        indexed = {entry["id"]: entry for entry in resumed[name]}
        require(all(indexed.get(entry["id"]) == entry for entry in previous[name]), "resume rewrites evidence")
    indexed = {(c["number"], c["component"]): c for c in resumed["coverage"]}
    for component in previous["coverage"]:
        new = indexed[(component["number"], component["component"])]
        require(new["source_ids"][:len(component["source_ids"])] == component["source_ids"], "resume pages")
        if component["status"] == "complete":
            require(new == component, "resume regresses complete coverage")


def validate_sharing(packet, current_repository):
    """Synthetic live-policy gate; offline bytes alone cannot grant sharing."""
    validate(packet)
    require(current_repository == packet["selection"]["repository"], "visibility/identity changed")
    require(packet["capture"]["stop_reason"] not in ("visibility_revoked", "revision_drift"),
            "capture invalidated")
