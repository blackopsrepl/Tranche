#!/usr/bin/env python3
"""The one authority for reading Tranche's bound report outputs.

Every consumer of a report - the MCP server and the evidence CLI - must apply
exactly the same gates: current snapshot, judgment and pair bindings, the
recorded output digests, the recomputed batches and park record, and no input
file changing under the read. Those gates used to live inside the MCP server's
response formatting; they live here now so a second consumer cannot end up with
a second, weaker set of predicates.

This module deliberately knows nothing about MCP or about evidence storage. It
returns the validated projection and raises `ReportError` when the local
observation cannot safely be used.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import NamedTuple

import tranche


class ReportError(ValueError):
    """The local observation cannot safely be used."""


class Limits(NamedTuple):
    """Byte and file-count bounds for fingerprinting the bound inputs."""

    max_file_bytes: int = 128 * 1024 * 1024
    max_total_bytes: int = 256 * 1024 * 1024
    max_input_files: int = 512


DEFAULT_LIMITS = Limits()


class BoundReport(NamedTuple):
    """The validated report projection: inputs, derived records and identity."""

    summary: dict
    clusters: dict
    dupes: dict
    batches: dict | None
    parked: dict | None
    prs: dict
    judgments: dict
    pairs: list
    latest_judgments: dict
    identity: dict


def source_paths() -> list[Path]:
    """The captured membership, then the report outputs, in a fixed order."""
    snapshot = tranche.PAGES_DIR / "snapshot.json"
    sources = [snapshot] if snapshot.exists() else sorted(tranche.PAGES_DIR.glob("page_*.json"))
    return [snapshot, *sources, tranche.JUDGMENTS_PATH, tranche.PAIRS_PATH,
            *(tranche.OUT_DIR / name for name in
              ("summary.json", "clusters.json", "dupes.json", "batches.json", "parked.json"))]


def input_digests(limits: Limits = DEFAULT_LIMITS) -> dict:
    """Bounded byte fingerprints include optional files and corpus membership."""
    paths = source_paths()
    if len(set(paths)) > limits.max_input_files:
        raise ReportError("Input file count exceeds limit")
    result, total = {}, 0
    for path in dict.fromkeys(paths):
        if not path.exists():
            result[str(path)] = None
            continue
        with path.open("rb") as stream:
            data = stream.read(limits.max_file_bytes + 1)
        total += len(data)
        if len(data) > limits.max_file_bytes or total > limits.max_total_bytes:
            raise ReportError("Input bytes exceed limit")
        result[str(path)] = hashlib.sha256(data).hexdigest()
    return result


def load(limits: Limits = DEFAULT_LIMITS) -> BoundReport:
    """Read and validate the bound report, or raise `ReportError`.

    Raises before returning anything the caller could mistake for a usable
    observation: a stale, unbound, foreign or modified input is refused here, not
    downstream.
    """
    try:
        return _read(limits)
    except ReportError:
        raise
    except (OSError, ValueError, TypeError, KeyError, AttributeError, tranche.TrancheFatal) as exc:
        raise ReportError("Invalid or missing bound reports; rerun cluster and batches") from exc


def _read(limits: Limits) -> BoundReport:
    out = tranche.OUT_DIR
    summary = json.loads((out / "summary.json").read_text())
    clusters = json.loads((out / "clusters.json").read_text())
    dupes = json.loads((out / "dupes.json").read_text())
    prs = tranche.load_prs()
    if tranche.JUDGMENTS_PATH.exists():
        for line in tranche.JUDGMENTS_PATH.read_text().splitlines():
            if line.strip() and not isinstance(json.loads(line), dict):
                raise ReportError("Malformed judgment record")
    judgments = tranche.current_judgments(prs)
    # Only the producer's current projection, which the report must bind.
    pairs = tranche.current_pairs(prs, judgments)
    if tranche.PAIRS_PATH.exists():
        for line in tranche.PAIRS_PATH.read_text().splitlines():
            if line.strip() and not isinstance(json.loads(line), dict):
                raise ReportError("Malformed pair record")
    expected = {"clusters.json": tranche.digest(clusters), "dupes.json": tranche.digest(dupes)}
    if (summary.get("format_version") != 2 or summary.get("repo") != tranche.REPO
            or summary.get("allow_unbound") is not False
            or summary.get("report_binding") != tranche.report_binding(prs, judgments, pairs)
            or summary.get("output_digests") != expected):
        raise ReportError("Reports are stale, unbound, foreign or modified; rerun cluster")
    batches_path = out / "batches.json"
    batches = json.loads(batches_path.read_text()) if batches_path.exists() else None
    if batches is not None and tranche.digest(batches) != tranche.digest(tranche.merge_batches(
            dupes, judgments, prs, expected["dupes.json"])):
        raise ReportError("batches.json is stale or modified; rerun batches")
    # Issue #8: the park record is part of the same observation as the batches it
    # gated. It must match the producer predicate byte for byte whenever batches
    # parked PRs, and is otherwise refused as modified.
    parked_path = out / "parked.json"
    parked = json.loads(parked_path.read_text()) if parked_path.exists() else None
    if parked is not None and tranche.digest(parked) != tranche.digest(tranche.parked_payload(
            tranche.park_state(dupes, judgments, prs), prs, judgments, expected["dupes.json"])):
        raise ReportError("parked.json is stale or modified; rerun batches")
    if (batches or {}).get("parked_prs") and parked is None:
        raise ReportError("batches.json parked PRs but parked.json is missing; rerun batches")
    identity = {"report_binding": summary["report_binding"],
                "batches.json": tranche.digest(batches) if batches is not None else None,
                "parked.json": tranche.digest(parked) if parked is not None else None,
                **summary["output_digests"]}
    return BoundReport(summary=summary, clusters=clusters, dupes=dupes, batches=batches,
                       parked=parked, prs=prs, judgments=judgments, pairs=pairs,
                       latest_judgments=tranche.load_done(), identity=identity)
