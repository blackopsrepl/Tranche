# Native evidence CLI

Capture the public GitHub evidence of a Tranche batch, inspect it offline, resume
an interrupted capture and export a self-contained packet — all with Tranche.

```bash
python3 tranche.py evidence capture --batch B001 --request-budget 100
python3 tranche.py evidence show    --batch B001
python3 tranche.py evidence export  --batch B001 --output review-packet.json
```

Evidence capture is **optional, read-only toward GitHub and model-free**. It never
runs a model, never writes to GitHub, never executes a patch or source file, and
never opens a browser. It reads through `gh`, which keeps the credential inside
itself: this code never sees a token and never puts one in an argv.

Design and the reasons behind it: [decisions/native-evidence-cli.md](decisions/native-evidence-cli.md).

## Commands

| Command | What it does |
| --- | --- |
| `evidence capture --batch B00x` | Validate the current native batch, acquire or resume its evidence, print the capture identity and per-component coverage |
| `evidence show --batch B00x` | Inspect the evidence associated with the **current** batch, offline |
| `evidence show --capture ID` | Inspect one capture **historically**, with no report consulted at all |
| `evidence show --capture ID --source ID [--start-byte N] [--length N]` | Read a bounded, terminal-safe byte window from stored bytes |
| `evidence show --capture ID --citation ID` | Resolve one citation against its stored bytes |
| `evidence export --batch B00x --output FILE` | Validate the current association, pass the sharing gate, write the packet atomically |

Options worth knowing:

- `--request-budget N` — requests this run may attempt (default 200). The last
  three are held back for the completion checks, so a shortfall stops the capture
  explicitly instead of certifying an unverified batch.
- `--max-bytes N` — bytes this capture may store (default 64 MiB).
- `--fresh` — start a new capture generation even if one is stored.
- `--reuse-capture ID` — resume or extend one explicit capture.
- `--break-lock` — replace a writer lock whose process is gone.
- `--json` — machine-readable output instead of the human report.
- `export --allow-historical` — confirm an export of a capture that is not the
  current one. Without it a historical export is refused.
- `--output -` — write the packet to stdout.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success (a capture that used its whole budget is exit `4`, not this) |
| `2` | Usage: contradictory flags or an impossible budget |
| `3` | Refused: the current report or the batch selection cannot be used |
| `4` | Incomplete: usable evidence, but the capture is not complete; resume it |
| `1` | Unusable local state (a missing, corrupt or unreadable capture) |

## Coverage

Eight components per member: `metadata`, `diff`, `files`, `discussion`,
`review_comments`, `reviews`, `checks`, `closing_issues`. `checks` is four
separately recorded groups, because a pull request's own checks live in its base
repository while a fork's head can carry its own:

```
[check_runs=complete, statuses=complete, fork_check_runs=complete, fork_statuses=complete]
```

A group that is not `complete` always carries a reason, and the report never
presents an unknown as an empty answer:

| Situation | Recorded as |
| --- | --- |
| No statuses configured | a captured empty response, `observed 0 / reported 0` |
| More items reported than observed | `partial` — "GitHub reports N items but M were observed" |
| Check runs at GitHub's 1000-suite ceiling | `partial` — "caps check runs at the 1000 most recent suites" |
| Diff over the review bound | `blocked` — the size, and that it is not presented as complete |
| Response over the transport bound | `blocked` — the bound |
| Transport failure | `blocked` — the failure |

A terminal cursor is not completeness: reaching the page bound, the byte bound or
the request budget leaves the capture incomplete with a reason and a resume line.

## What reuse means

Two different changes, two different outcomes:

- **Base or head moved** — a new code observation. The old capture cannot resume;
  a new generation starts. Old bytes stay readable and are never rebound.
- **Thread updated, report re-clustered, batch renumbered or prompt edited** — the
  association and the mutable observations change; the code bytes are reused
  without a single request. Generation, sources and citations are re-bound to the
  new association, so its identity is self-consistent while the previous capture
  keeps its own.

The reviewer prompt is recorded as provenance and is **not** a cache key: editing
the prompt does not invalidate source evidence.

## What an export contains

One UTF-8 JSON object, `tranche.evidence-packet/v1` / `pr-review/v1`:

- `selection` — the repository identity, the native report association (report
  binding plus the four output digests) and the batch copied unchanged, including
  the exact reviewer prompt;
- `members` — every member's `source_digest`, `evidence_digest`, full 40-character
  `base_sha`/`head_sha` and the repository identities they sit in;
- `components` — per member and component, the status, the page count, the reason
  and the source ids, per endpoint group;
- `sources` — each acquisition with its real URL, media type, timestamps and the
  digest of its exact response bytes;
- `bodies` — the decoded response bytes, base64, once per distinct digest;
- `citations` — nonempty half-open byte ranges with their excerpt digests;
- `capture` — the observation time, the limits as used, and the stop reason;
- `packet_digest` — the digest of the whole packet.

`complete` is true only when every group reported a terminal page within the
recorded budgets and nothing stopped the run.

An external reader needs none of this code, no network and no state store: decode
`bodies`, check each against its digest, and slice `citations` out of them. Every
citation carries the digest of its source body and of its own excerpt.

### What a packet does not claim

- Digests prove **integrity, not authenticity** — that the bytes are the bytes
  recorded, not that GitHub served them.
- A captured CI run is an **observation**, not a test performed by Tranche.
- `complete` is not a review, a test run, a finding, or approval to merge.
- A hash of the selection is not a signature; it is a reproducible identity.
- Offline reading of a historical capture is explicitly historical, not current.

## Storage

```
out/evidence/
  <capture_id>/manifest.json   the atomic checkpoint: selection, coverage, sources, citations
  bodies/<sha256>.bin          immutable response bytes, content-addressed
  <capture_id>.lock            one writer per capture, stale-recoverable
```

`out/evidence/` is ignored by Git: captured project data is runtime state, never
committed. Manifests are written atomically and bodies are fsynced before any
manifest cites them, so a crash between the two leaves an unreferenced file
(swept on the next completed run) and never a checkpoint pointing at missing
bytes. Stored bytes are verified against their digest on every read. The lock
records its owner and is reclaimed once that process is gone; `--break-lock` is
the explicit override for the remaining case.

## Not in this slice

Publication of captured evidence to the workbench, and bounded MCP read/retrieve
adapters, are follow-up work on this same service. No acquisition tool has been
added to the MCP server, and no evidence is copied into `docs/`.
