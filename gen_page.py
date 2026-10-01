#!/usr/bin/env python3
"""Render the PR workbench from mutually consistent, bound report inputs."""

import html
import json
import os
import tempfile
from pathlib import Path

import tranche


def atomic_text(path: Path, text: str) -> None:
    """Replace path with text atomically; a failed render never truncates it."""
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".pending-", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            stream.write(text)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


ROOT = Path(__file__).resolve().parent
OUT = ROOT / "out"
DOCS = ROOT / "docs"
TEMPLATE = ROOT / "page" / "template.html"

summary = json.loads((OUT / "summary.json").read_text())
dupes = json.loads((OUT / "dupes.json").read_text())
clusters = json.loads((OUT / "clusters.json").read_text())
prs = tranche.load_prs()
allow_unbound = summary.get("allow_unbound", False)
judgments = tranche.current_judgments(prs, allow_unbound=allow_unbound)
verdicts = tranche.current_pairs(prs, judgments, allow_unbound=allow_unbound)
if (summary.get("format_version") != 2
        or summary.get("repo") != tranche.REPO
        or summary.get("report_binding") != tranche.report_binding(prs, judgments, verdicts)
        or summary.get("output_digests") != {"clusters.json": tranche.digest(clusters),
                                              "dupes.json": tranche.digest(dupes)}):
    raise tranche.TrancheFatal("Report inputs changed or are legacy/mixed; rerun cluster before rendering")

# Issue #4: merge batches ship with the workbench when present; a stale or
# foreign batches file must never be shown against a newer dupe run.
batches_path = OUT / "batches.json"
batches = None
if batches_path.exists():
    batches = json.loads(batches_path.read_text())
    if (batches.get("format_version") != 3 or batches.get("repo") != tranche.REPO
            or batches.get("dupes_digest") != summary["output_digests"]["dupes.json"]):
        raise tranche.TrancheFatal("batches.json is stale or foreign; rerun 'tranche.py batches' before rendering")
# Issue #8: the park record renders beside the batches it gated. A batches
# run that parked PRs without its park record - or against a mutated one -
# must never render, same discipline as gen_page's other bound inputs.
parked = None
parked_path = OUT / "parked.json"
if parked_path.exists():
    parked = json.loads(parked_path.read_text())
    expected_parked = tranche.parked_payload(
        tranche.park_state(dupes, judgments, prs), prs, judgments, summary["output_digests"]["dupes.json"])
    if parked != expected_parked:
        raise tranche.TrancheFatal("parked.json is stale, foreign or modified; rerun 'tranche.py batches' before rendering")
if (batches or {}).get("parked_prs") and parked is None:
    raise tranche.TrancheFatal("batches.json parked PRs but parked.json is missing; rerun 'tranche.py batches'")
batch_of = {}
for batch in (batches or {}).get("batches", []):
    tag = {"id": batch["id"], "count": batch["count"]}
    for number in batch["members"]:
        batch_of.setdefault(number, []).append(tag)
merge_batches = [
    {"id": batch["id"], "count": batch["count"], "members": batch["members"],
     "security_members": batch["security_members"],
     "average_risk": batch["average_risk"], "created": batch["created"],
     "review_prompt": batch["review_prompt"]}
    for batch in (batches or {}).get("batches", [])
]

CAT_LABELS = {
    "security-review": "Security (meta)",
    "install-setup": "Install & Setup", "desktop-config": "Desktop Config",
    "user-experience": "User Experience",
    "shell-cli": "Shell & CLI", "apps-integrations": "Apps & Integrations",
    "hardware-drivers": "Hardware & Drivers", "update-release": "Update & Release",
    "agents-ai": "Agents & AI", "docs": "Docs", "fix-misc": "Fixes & Misc",
    "unclear": "Unclear", "unknown": "Unknown",
}

# Eligibility stays in the CLI policy; this renderer never reclassifies a judgment.
in_dupe = {n for g in dupes["confirmed_groups"] for n in g} | {n for g in dupes["review_groups"] for n in g["members"]}
related = in_dupe | {n for pair in dupes["uncertain_pairs"] for n in (pair["a"], pair["b"])}
parked_by_number = {m["number"]: m for m in (parked or {}).get("members", [])}
rows = []
latest_judgments = tranche.load_done()
for n, pr in sorted(prs.items()):
    judgment = judgments.get(n, {})
    finished = tranche.metric(judgment, "finished_form")
    rows.append({
        "number": n, "title": pr["title"], "body": pr["body"],
        "body_truncated": pr["body_truncated"], "author": pr["author"],
        "created": pr["created"], "draft": pr["draft"],
        "activity": tranche.pr_activity(pr, latest_judgments.get(n, {})),
        "category": tranche.category(judgment) if judgment else "unknown",
        "categories": ([ "security-review" ]
                       if judgment and tranche.security_priority(judgment) else []),
        "freshness": judgment.get("freshness", "unjudged or stale"),
        "risk": tranche.metric(judgment, "risk"),
        "security": tranche.metric(judgment, "security_flag", "noul"),
        "security_priority": bool(judgment) and tranche.security_priority(judgment),
        "finished": finished, "effort": tranche.metric(judgment, "review_effort"),
        "is_fix": tranche.metric(judgment, "is_fix", "noul"),
        "diffstat": tranche.pr_state(pr)["pr"]["diffstat"],
        "candidate": bool(judgment) and tranche.review_candidate(pr, judgment, in_dupe),
        "senior": tranche.escalated(judgment),
        "followup": finished is not None and finished <= 1 and n not in in_dupe,
        "related": n in related,
        "parked": parked_by_number.get(n, {}).get("reasons", []),
        "batches": batch_of.get(n, []),
    })
# The workbench data ships as docs/data/workbench.json and is fetched at boot:
# the HTML shell parses immediately and the payload is cacheable across
# refreshes. Same strict escaping discipline as the former inline payload;
# the JSON bytes are unchanged, only the transport moved.
payload = json.dumps({"prs": rows, "categories": CAT_LABELS, "groups": dupes,
                      "batches": merge_batches,
                      "batches_available": batches is not None,
                      "parked": parked},
                     ensure_ascii=True, allow_nan=False, separators=(",", ":"))
payload = payload.replace("&", "\\u0026").replace("<", "\\u003c").replace(">", "\\u003e")
# The shell is HTML in page/template.html: {{options}}, {{count}} and
# {{count_commas}} are filled by literal replacement - the same cheap,
# deterministic fill the f-string gave, with HTML out of the Python source.
options = ''.join(f'<option value="{cat}">{html.escape(label)}</option>' for cat, label in CAT_LABELS.items())
page = TEMPLATE.read_text()
page = page.replace("{{options}}", options)
page = page.replace("{{count_commas}}", f"{len(prs):,}").replace("{{count}}", str(len(prs)))
DOCS.mkdir(exist_ok=True)
atomic_text(DOCS / "data" / "workbench.json", payload)
(DOCS / "index.html").write_text(page)
print(f"wrote docs/index.html ({len(page)//1024} KB) and docs/data/workbench.json "
      f"({len(payload)//1024} KB); {len(rows)} captured PRs")
print(f"candidates: {sum(r['candidate'] for r in rows)}, senior: {sum(r['senior'] for r in rows)}, "
      f"follow-up: {sum(r['followup'] for r in rows)}, parked: {len(parked_by_number)}, "
      f"related: {sum(r['related'] for r in rows)}")
