#!/usr/bin/env python3
"""Read-only, model-free access to Tranche's bound local observation."""
import argparse
import json
import math
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

import report_loader
import tranche

MAX_RESULT_BYTES = 1024 * 1024

# The bounded-input fingerprints and the validation predicates live in
# `report_loader`, which the evidence CLI consumes too. One authority for the
# bound report, two consumers with different response contracts.
MAX_FILE_BYTES = report_loader.DEFAULT_LIMITS.max_file_bytes
MAX_TOTAL_BYTES = report_loader.DEFAULT_LIMITS.max_total_bytes
MAX_INPUT_FILES = report_loader.DEFAULT_LIMITS.max_input_files


def input_digests():
    """Fingerprint the bound inputs under this module's advertised bounds."""
    return report_loader.input_digests(
        report_loader.Limits(MAX_FILE_BYTES, MAX_TOTAL_BYTES, MAX_INPUT_FILES))

DISCLAIMER = ("Model suggestions from titles and shortened descriptions, not merge/close "
              "approval. Patches, CI, reproductions and security have not been verified.")


ReportError = report_loader.ReportError


def result_text(result):
    """The exact JSON text sent over MCP; the SDK must not reserialize it."""
    text = json.dumps(result, ensure_ascii=False, allow_nan=False)
    if len(text.encode()) > MAX_RESULT_BYTES:
        raise ReportError("Response exceeds byte limit; narrow filters or lower limit")
    return text


class Reports:
    def _load(self):
        """Validate the bound report, then apply what is specific to MCP.

        `report_loader` owns the report gates - current snapshot, bindings,
        output digests, recomputed batches and park record. This method adds the
        before/after input fingerprint that only a long-lived server needs, so a
        file changing under the read is refused rather than half-served.
        """
        before = input_digests()
        try:
            report = report_loader.load()
        except ReportError:
            raise
        except Exception as exc:
            raise ReportError("Invalid or missing bound reports; rerun cluster and batches") from exc
        self._adopt(report)
        if input_digests() != before:
            raise ReportError("Report files changed during read; retry")
        self.identity["input_bytes"] = before

    def _adopt(self, report):
        self.summary, self.clusters, self.dupes = report.summary, report.clusters, report.dupes
        self.batches, self.parked = report.batches, report.parked
        self.prs, self.judgments, self.pairs = report.prs, report.judgments, report.pairs
        self.latest_judgments = report.latest_judgments
        self.identity = dict(report.identity)

    def _envelope(self, **data):
        result = {"repo": tranche.REPO, "disclaimer": DISCLAIMER,
                  "digests": self.identity, **data}
        result_text(result)
        return result

    def _activity(self, members):
        return {str(n): tranche.pr_activity(self.prs[n], self.latest_judgments.get(n, {}))
                for n in members}

    def _rows(self):
        grouped = {n for group in self.dupes["confirmed_groups"] for n in group}
        grouped |= {n for group in self.dupes["review_groups"] for n in group["members"]}
        related = grouped | {n for pair in self.dupes["uncertain_pairs"] for n in (pair["a"], pair["b"])}
        parked = {m["number"]: m["reasons"] for m in (self.parked or {}).get("members", [])}
        rows = []
        for number, pr in self.prs.items():
            judgment = self.judgments.get(number, {})
            risk = tranche.metric(judgment, "risk")
            finished = tranche.metric(judgment, "finished_form")
            row = dict(pr, answers=judgment.get("answers", {}),
                       category=tranche.category(judgment) if judgment else "unknown",
                       risk=risk, finished_form=finished,
                       security=tranche.metric(judgment, "security_flag", "noul"),
                       security_priority=tranche.security_priority(judgment),
                       freshness=judgment.get("freshness", "unjudged"),
                       judgment_binding=judgment.get("binding"),
                       risk_band="unknown" if risk is None else
                       "low" if risk <= 1.5 else "core" if risk <= 2.5 else "danger",
                       candidate=bool(judgment) and tranche.review_candidate(pr, judgment, grouped),
                       senior=tranche.escalated(judgment),
                       followup=finished is not None and finished <= 1 and number not in grouped,
                       related=number in related,
                       parked=list(parked.get(number, [])),
                       activity=self._activity([number])[str(number)])
            rows.append(row)
        rows.sort(key=lambda row: (not row["security_priority"],
                                  row["risk"] if row["risk"] is not None else 99,
                                  row["created"], row["number"]))
        return rows

    def query(self, text: str = "", category: str | None = None,
              risk_band: str | None = None, security: bool | None = None,
              finished_form: float | None = None, batch: str | None = None,
              queue: str = "all", offset: int = 0, limit: int = 25):
        self._load()
        if (type(offset) is not int or not 0 <= offset <= 100000
                or type(limit) is not int or not 1 <= limit <= 100
                or not isinstance(text, str) or len(text) > 512
                or category is not None and (not isinstance(category, str) or category not in
                    [*tranche.judge_questions()["category"]["criteria"], "security-review", "unknown"])
                or risk_band is not None and risk_band not in ("low", "core", "danger", "unknown")
                or security is not None and type(security) is not bool
                or finished_form is not None and (type(finished_form) not in (int, float)
                    or not 0 <= finished_form <= 3 or not math.isfinite(finished_form))
                or queue not in ("all", "security", "candidates", "senior", "followup", "parked", "related")
                or batch is not None and (not isinstance(batch, str) or
                    re.fullmatch(r"B[0-9]{3,6}", batch) is None)):
            raise ReportError("Invalid query arguments; limit 1..100, offset 0..100000")
        members = None
        if batch is not None:
            members = self._batch(batch)["members"]
        terms = text.casefold().split()
        rows = []
        for row in self._rows():
            haystack = f"#{row['number']} @{row['author']} {row['title']} {row['body']}".casefold()
            if (any(term not in haystack for term in terms)
                    or category is not None and not (row["category"] == category or
                        category == "security-review" and row["security_priority"])
                    or risk_band is not None and row["risk_band"] != risk_band
                    or security is not None and (row["security"] is None or
                        row["security_priority"] != security)
                    or finished_form is not None and row["finished_form"] != finished_form
                    or members is not None and row["number"] not in members
                    or queue != "all" and not row[{
                        "security": "security_priority", "candidates": "candidate",
                        "senior": "senior", "followup": "followup", "related": "related",
                        "parked": "parked"}[queue]]):
                continue
            rows.append(row)
        if queue == "parked":  # issue #8: an empty reason array never parks.
            rows = [row for row in rows if row["parked"]]
        end = offset + limit
        return self._envelope(items=rows[offset:end], total=len(rows), offset=offset,
                              next_offset=end if end < len(rows) else None)

    def _batch(self, batch_id):
        if not isinstance(batch_id, str) or re.fullmatch(r"B[0-9]{3,6}", batch_id) is None:
            raise ReportError("Expected exact batch id such as B001")
        if self.batches is None:
            raise ReportError("batches.json is unavailable; run tranche.py batches")
        for batch in self.batches["batches"]:
            if batch["id"] == batch_id:
                return batch
        raise ReportError("Unknown batch id")

    def pick(self, batch_id: str):
        self._load()
        batch = self._batch(batch_id)
        rows = {row["number"]: row for row in self._rows()}
        return self._envelope(batch=batch, prs=[rows[n] for n in batch["members"]],
                              activity=self._activity(batch["members"]))

    def next_prompt(self, after: int | str | None = None):
        self._load()
        if self.batches is None:
            raise ReportError("batches.json is unavailable; run tranche.py batches")
        if isinstance(after, str):
            after = self._batch(after)["ordinal"]
        if after is None:
            after = 0
        if type(after) is not int or not 0 <= after <= len(self.batches["batches"]):
            raise ReportError("after must be an existing batch id or ordinal (0 starts)")
        batch = next((b for b in self.batches["batches"] if b["ordinal"] > after), None)
        return self._envelope(batch=batch,
                              activity=self._activity(batch["members"]) if batch else {})

    def related(self, number: int, offset: int = 0, limit: int = 25):
        self._load()
        if (type(number) is not int or number not in self.prs
                or type(offset) is not int or not 0 <= offset <= 100000
                or type(limit) is not int or not 1 <= limit <= 100):
            raise ReportError("Expected captured PR number and bounded pagination")
        items = []
        for group in self.dupes["confirmed_groups"]:
            if number in group:
                items.append({"kind": "confirmed_group", "members": group})
        for group in self.dupes["review_groups"]:
            if number in group["members"]:
                items.append(dict(group, kind="review_group"))
        for pair in self.pairs:
            if number in (pair["a"], pair["b"]):
                items.append(dict(pair, kind="pair", classification=tranche.pair_classification(pair)))
        page = items[offset:offset + limit]
        for item in page:
            members = item.get("members", [item.get("a"), item.get("b")])
            item["sources"] = {str(n): {key: self.prs[n][key] for key in
                                       ("number", "source_digest", "evidence_digest",
                                        "head_sha", "updated", "url")}
                               for n in members}
        end = offset + limit
        return self._envelope(items=page, total=len(items), offset=offset,
                              next_offset=end if end < len(items) else None)

    def digests(self):
        self._load()
        return self._envelope()

    def surface(self):
        self._load()
        overview = [{key: batch[key] for key in
                     ("id", "ordinal", "count", "security_members", "average_risk", "created")}
                    for batch in (self.batches or {}).get("batches", [])]
        rows = self._rows()
        category_counts = {}
        for row in rows:
            category_counts[row["category"]] = category_counts.get(row["category"], 0) + 1
        queues = {}
        for queue, field in {"all": None, "security": "security_priority",
                             "candidates": "candidate", "related": "related",
                             "senior": "senior", "followup": "followup",
                             "parked": "parked"}.items():
            if queue == "parked":
                members = [row["number"] for row in rows if row["parked"]]
            else:
                members = [row["number"] for row in rows if field is None or row[field]]
            queues[queue] = {"count": len(members), "members": members}
        parked_members = queues["parked"]["members"]
        parked_block = {"count": len(parked_members), "members": parked_members}
        if self.parked is not None:
            parked_block["unblock"] = {m["number"]: m["unblock"]
                                       for m in self.parked.get("members", [])}
        # summary.json coverage is outside output_digests; derive it from inputs.
        coverage = {"prs_in_corpus": len(self.prs), "judged": len(self.judgments),
                    "unjudged": len(self.prs) - len(self.judgments)}
        activity = self._activity(self.prs)
        now = datetime.now(timezone.utc)
        durations = []
        for item in activity.values():
            if item["idle_basis"] != "judgment":
                continue  # creation age is not evidence of a still head
            try:
                elapsed = (now - datetime.fromisoformat(item["idle_since"].replace("Z", "+00:00"))).days
                durations.append(max(0, elapsed))
            except (ValueError, TypeError, OverflowError):
                pass
        activity_counts = {"head_moved": sum(item["head_moved"] for item in activity.values()),
                           "idle_since_known": sum(bool(item["idle_since"]) for item in activity.values()),
                           "idle_7d": sum(days >= 7 for days in durations),
                           "idle_30d": sum(days >= 30 for days in durations),
                           "idle_60d": sum(days >= 60 for days in durations),
                           "as_of": now.isoformat()}
        return self._envelope(summary=coverage, activity=activity_counts,
                              batches_available=self.batches is not None,
                              category_counts=category_counts, queues=queues,
                              parked=parked_block,
                              batches=overview, filters={
                                  "categories": ["security-review", *tranche.judge_questions()["category"]["criteria"], "unknown"],
                                  "risk_bands": ["low", "core", "danger", "unknown"],
                                  "queues": ["security", "all", "candidates", "senior", "followup", "parked", "related"],
                              })


PROTOCOL_VERSIONS = ("2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05")
PROTOCOL_VERSION = PROTOCOL_VERSIONS[0]


def _version():
    """Repository VERSION when present, so serverInfo tracks the checkout."""
    try:
        return (Path(__file__).resolve().parent / "VERSION").read_text().strip() or "0"
    except OSError:
        return "0"


def _tool_definitions():
    """Standard MCP tool definitions: JSON Schema in, `readOnlyHint` annotations out."""
    descriptions = {
        "surface": "Inspect bound report coverage; model suggestions, never merge approval.",
        "query": "Security-first PR search. Exact filters, finished_form score; offset/limit pagination.",
        "pick": "Inspect one exact batch id with source-bound PRs and the unchanged review prompt.",
        "next_prompt": "Next batch in report order; after is an existing ordinal/id, 0 starts.",
        "related": "Inspect model relationship evidence, conflicts and missing pairs; no survivor selected.",
        "digests": "Read current report binding, output checksums and before/after checked input byte digests.",
    }
    pagination = {
        "offset": {"type": "integer", "minimum": 0, "maximum": 100000, "default": 0},
        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 25},
    }
    batch_id = {"type": "string", "pattern": r"^B[0-9]{3,6}$"}
    properties = {
        "surface": {},
        "query": {
            "text": {"type": "string", "maxLength": 512, "default": ""},
            "category": {"type": ["string", "null"], "default": None,
                         "enum": [*tranche.judge_questions()["category"]["criteria"],
                                  "security-review", "unknown", None]},
            "risk_band": {"type": ["string", "null"], "default": None,
                          "enum": ["low", "core", "danger", "unknown", None]},
            "security": {"type": ["boolean", "null"], "default": None},
            "finished_form": {"type": ["number", "null"], "minimum": 0,
                              "maximum": 3, "default": None},
            "batch": {"type": ["string", "null"], "pattern": batch_id["pattern"],
                      "default": None},
            "queue": {"type": "string", "default": "all", "enum":
                      ["all", "security", "candidates", "senior", "followup", "parked", "related"]},
            **pagination,
        },
        "pick": {"batch_id": batch_id},
        "next_prompt": {"after": {"anyOf": [
            {"type": "integer", "minimum": 0}, batch_id, {"type": "null"}],
            "default": None}},
        "related": {"number": {"type": "integer", "minimum": 1}, **pagination},
        "digests": {},
    }
    required = {"pick": ["batch_id"], "related": ["number"]}
    return [{
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties[name],
                        "additionalProperties": False, "required": required.get(name, [])},
        "annotations": {"readOnlyHint": True, "destructiveHint": False,
                        "idempotentHint": True, "openWorldHint": False},
    } for name, description in descriptions.items()]


TOOLS = _tool_definitions()
TOOL_NAMES = {tool["name"] for tool in TOOLS}


def error_response(code, message, request_id):
    return {"jsonrpc": "2.0", "id": request_id, "error": {"code": code, "message": message}}


def _type_matches(value, expected):
    """JSON types, not Python's: ``bool`` is not a number, and ``None`` is null."""
    if expected == "null":
        return value is None
    if expected == "boolean":
        return isinstance(value, bool)
    if expected == "integer":
        return isinstance(value, int) and not isinstance(value, bool)
    if expected == "number":
        return isinstance(value, (int, float)) and not isinstance(value, bool)
    if expected == "string":
        return isinstance(value, str)
    if expected == "array":
        return isinstance(value, list)
    if expected == "object":
        return isinstance(value, dict)
    return True


def _check_schema(value, schema, where):
    """Validate against the subset of JSON Schema these tools advertise.
    Returns an error string, or None. ``where`` names the property for the message."""
    if "anyOf" in schema:
        if not any(_check_schema(value, option, where) is None for option in schema["anyOf"]):
            return f"'{where}' does not match any accepted form"
        return None
    for expected in schema.get("type", []) if isinstance(schema.get("type"), list) else [schema.get("type")]:
        if expected is not None and _type_matches(value, expected):
            break
    else:
        kinds = schema.get("type")
        kinds = "/".join(kinds) if isinstance(kinds, list) else kinds
        return f"'{where}' must be {kinds}"
    if value is None:
        return None
    if "enum" in schema and value not in schema["enum"]:
        return f"'{where}' must be one of: {', '.join(str(item) for item in schema['enum'] if item is not None)}"
    if "minimum" in schema and value < schema["minimum"]:
        return f"'{where}' must be >= {schema['minimum']}"
    if "maximum" in schema and value > schema["maximum"]:
        return f"'{where}' must be <= {schema['maximum']}"
    if "maxLength" in schema and len(value) > schema["maxLength"]:
        return f"'{where}' must be at most {schema['maxLength']} characters"
    if "pattern" in schema and re.fullmatch(schema["pattern"], value) is None:
        return f"'{where}' does not match {schema['pattern']}"
    return None


def validate_arguments(name, arguments):
    """Enforce the tool's advertised inputSchema; returns a message or None.

    Clients are not required to validate — the spec puts the obligation on the
    server — so an argument that reaches here unchecked must still be refused in
    the tool's own terms rather than by a Python call-signature error.
    """
    schema = next(tool["inputSchema"] for tool in TOOLS if tool["name"] == name)
    if not isinstance(arguments, dict):
        return "Arguments must be a JSON object"
    unknown = [key for key in arguments if key not in schema["properties"]]
    if unknown and schema.get("additionalProperties") is False:
        return f"Unknown argument(s): {', '.join(sorted(unknown))}"
    missing = [key for key in schema["required"] if key not in arguments]
    if missing:
        return f"Missing required argument(s): {', '.join(missing)}"
    for key, value in arguments.items():
        spec = schema["properties"].get(key)
        if spec is None:
            continue
        problem = _check_schema(value, spec, key)
        if problem:
            return problem
    return None


def call_tool(name, arguments):
    """Invoke one tool after enforcing its advertised schema."""
    if name not in TOOL_NAMES:
        raise ReportError(f"Unknown tool: {name}")
    problem = validate_arguments(name, arguments)
    if problem:
        raise ReportError(problem)
    return result_text(getattr(Reports(), name)(**arguments))


def dispatch(message):
    """Handle one JSON-RPC message. Returns the response, or None for a notification."""
    if not isinstance(message, dict) or message.get("jsonrpc") != "2.0":
        return error_response(-32600, "Invalid Request", message.get("id") if isinstance(message, dict) else None)
    method = message.get("method")
    request_id = message.get("id")
    if not isinstance(method, str):
        return error_response(-32600, "Invalid Request", request_id)
    if request_id is None:  # notification
        return None
    if method == "initialize":
        requested = (message.get("params") or {}).get("protocolVersion")
        version = requested if requested in PROTOCOL_VERSIONS else PROTOCOL_VERSION
        return {"jsonrpc": "2.0", "id": request_id, "result": {
            "protocolVersion": version,
            "capabilities": {"tools": {"listChanged": False}},
            "serverInfo": {"name": "tranche", "version": _version()},
            "instructions": DISCLAIMER}}
    if method == "ping":
        return {"jsonrpc": "2.0", "id": request_id, "result": {}}
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": request_id, "result": {"tools": TOOLS}}
    if method == "tools/call":
        params = message.get("params") or {}
        if params.get("name") not in TOOL_NAMES:
            return error_response(-32602, f"Unknown tool: {params.get('name')}", request_id)
        try:
            text = call_tool(params.get("name"), params.get("arguments") or {})
        except Exception as exc:  # execution errors are results with isError
            return {"jsonrpc": "2.0", "id": request_id, "result": {
                "content": [{"type": "text", "text": str(exc)}], "isError": True}}
        return {"jsonrpc": "2.0", "id": request_id, "result": {
            "content": [{"type": "text", "text": text}], "isError": False}}
    return error_response(-32601, f"Method not found: {method}", request_id)


def serve(stdin=None, stdout=None):
    """Newline-delimited JSON-RPC over stdio; the standard MCP stdio transport."""
    stdin = stdin or sys.stdin
    stdout = stdout or sys.stdout
    for line in stdin:
        if not line.strip():
            continue
        try:
            message = json.loads(line)
        except ValueError:
            response = error_response(-32700, "Parse error", None)
        else:
            response = dispatch(message)
        if response is not None:
            stdout.write(json.dumps(response, ensure_ascii=False, allow_nan=False) + "\n")
            stdout.flush()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=tranche.ROOT,
                        help="Local Tranche report root (data/pages and out); no acquisition")
    args = parser.parse_args(argv)
    root = args.root.resolve()
    tranche.ROOT = root
    tranche.PAGES_DIR = root / "data" / "pages"
    tranche.OUT_DIR = root / "out"
    tranche.JUDGMENTS_PATH = tranche.OUT_DIR / "judgments.jsonl"
    tranche.PAIRS_PATH = tranche.OUT_DIR / "pair_verdicts.jsonl"
    serve()


if __name__ == "__main__":
    main()
