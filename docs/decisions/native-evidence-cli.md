# Native evidence CLI — architecture note

Design input for the native evidence CLI (issue #9). Written before implementation;
the acceptance section at the end is the matrix the implementation is held to.

## Reuse seams (traced in the current tree, not assumed)

| Seam | Where | What is reused |
| --- | --- | --- |
| `load_prs()` | `tranche.py` | Native membership + PR identity: `number`, `head_sha`, `source_digest`, `evidence_digest`, `created`/`updated`, `url` |
| Raw captured item | `data/pages/*.json` | `base.sha`, `base.repo.{id,full_name}`, `head.repo.{id,full_name}` — present in the capture, not projected by `load_prs`, so evidence reads the item |
| `Reports._read()` | `mcp_server.py` | The authoritative bound-report gate: summary/clusters/dupes/batches/parked, `report_binding`, output digests, recomputed batches/park, changed-during-read check |
| `tranche.atomic_json()` | `tranche.py` | Atomic JSON publication (`mkstemp` + `fsync` + `os.replace`) |
| `batch["review_prompt"]`, `batch["id"/"ordinal"/"members"/"count"]` | `out/batches.json` | Copied verbatim as provenance; the prompt is recorded, never used to key evidence |
| `fetch_page()` | `tranche.py` | **Not reused.** It discards status, headers and raw bytes, which evidence requires; its behaviour stays untouched |

`gen_page.py` executes at import and is not importable as a service; evidence never imports it.

## New files

- `ghread.py` — bounded, GET-only authenticated read transport over the existing `gh`
  credential, returning status/headers/bytes, with budgets and origin checks.
- `evidence.py` — native evidence service: paths, locks, checkpoints, coverage,
  identity, component acquisition, export, and the `evidence` CLI commands.
- `tests/test_ghread.py`, `tests/test_evidence.py` — offline synthetic suites.
- `docs/decisions/native-evidence-cli.md` (this note), `docs/EVIDENCE_CLI.md`
  (user documentation, written from implemented behaviour at the end).
- `evidence` subparser wired into `tranche.py`'s existing `argparse` entry point.

## Four identities that must not be collapsed

1. **Report association** — repository `full_name` + `report_binding` + the four output
   digests + the selected `batch` copied unchanged + each member's `source_digest` and
   `evidence_digest`. Proves *which native report* this evidence belongs to. A batch id
   (`B001`) is a display ordinal and never an identity on its own.
2. **Code identity** — base and head repository `id` + `full_name`, `base_sha`, `head_sha`
   (full 40-char, both resolved from fresh PR metadata and verified against the captured
   projection) plus the observed `updated_at`. This — not the report digest — is what code
   evidence is keyed to and reused by.
3. **Mutable observation** — `updated_at`, discussion, reviews, review comments, CI. Timestamped,
   refreshed independently, never able to invalidate code bytes.
4. **Capture generation** — `capture_id` (128-bit lowercase hex, minted only when a new
   generation starts) and `generation` = digest over `{format, profile, repository,
   revision, selection}`. Every source binds to its generation, so historical bytes stay
   readable and are never rebound.

### Reuse and refresh rules (explicit, and each tested)

| Change | Required effect |
| --- | --- |
| `head_sha` or `base_sha` moves | New code observation: the affected generation cannot resume; a new generation starts. Old sources stay readable and frozen |
| `updated_at` moves only | Mutable components refresh; code bytes are reused; the code observation is untouched |
| Report, prompt, question policy or batch ordinal changes with the same base/head | Code bytes reused; the association is re-bound and the new prompt recorded as provenance |
| Unrelated corpus change (other PRs, other batches) | Nothing invalidated; the report digest is stored as provenance only |
| Batch renumbering with identical members | A bare `--batch B00x` capture starts a new generation; reusing the old one requires the explicit `--reuse-capture <id>` |
| Stale, foreign, unbound, modified, mixed or park-inconsistent report | Refused (exit 3) before any acquisition; reuse is refused for a park-inconsistent batch |
| Same batch, `--fresh` | New capture id and generation; identical code bytes may be re-fetched but never rebound |

Freshness on reuse is checked, not assumed: before a stored source is reused the PR's
live base/head identity is re-read within the current budget and must equal the recorded
revision. Checksums prove bytes were not corrupted; only a live identity read shows the
bytes still describe the revision they claim.

## Components and endpoint mapping

Profile `pr-review/v1` requires, per member, exactly these eight components:

| Component | REST endpoint (real; the #13 fixtures' `/pulls/N/metadata`, `/discussion`, `/checks` do not exist) |
| --- | --- |
| `metadata` | `GET /repos/{base}/pulls/{n}` |
| `diff` | `GET /repos/{base}/pulls/{n}` with `Accept: application/vnd.github.diff` |
| `files` | `GET /repos/{base}/pulls/{n}/files` (`application/vnd.github+json`) |
| `discussion` | `GET /repos/{base}/issues/{n}/comments` |
| `review_comments` | `GET /repos/{base}/pulls/{n}/comments` |
| `reviews` | `GET /repos/{base}/pulls/{n}/reviews` |
| `checks` | `GET /repos/{base}/commits/{head_sha}/check-runs` and `.../status`, recorded under one component |
| `closing_issues` | GraphQL `closingIssuesReferences` (query only, never a mutation) |

`checks` keeps check-runs and commit statuses as two separately paginated endpoint
groups inside one coverage entry and does not assume a missing group means "no CI".

## Command semantics

```
tranche.py evidence capture --batch B001 [--request-budget N] [--fresh]
                                  [--reuse-capture ID] [--json]
tranche.py evidence show    --batch B001 [--json]
tranche.py evidence show    --capture ID [--json]
tranche.py evidence show    --capture ID --source SOURCE_ID
                            [--start-byte N] [--length N] [--raw] [--json]
tranche.py evidence export  --batch B001 --output FILE [--capture ID] [--json]
```

- `capture` validates the current native selection, acquires or resumes, prints the capture
  identity and per-member/component coverage. Interruption or budget exhaustion leaves a
  usable checkpoint and prints the exact resume line.
- `show --batch` resolves evidence for the **current** validated batch offline; absent or
  stale associations are explained, and a historical capture is never presented as the
  current one.
- `show --capture` is historical inspection and works with any report state or none.
- `show --capture --source` resolves stored bytes, returns half-open offsets and a
  continuation, and escapes terminal control characters. It never prints a whole packet by default.
- `export` validates the current association, applies the fresh public-scope gate, checks
  the input digests for concurrent change, and writes atomically. A historical export
  (`--capture`) is explicit and never claims current-batch compatibility.

Exit codes: `0` success, `2` invalid arguments, `3` refused selection (stale/foreign/
unbound/park-inconsistent/unreadable evidence), `4` incomplete (budget/interruption/
acquisition failure) with a usable checkpoint, `1` unusable local state. No command calls
a model, mutates GitHub, opens a browser or executes captured content.

## Storage

```
out/evidence/                     ← gitignored runtime root (OUT_DIR / "evidence")
  <capture_id>/manifest.json      ← the one atomic checkpoint per capture
  <capture_id>/sources/<id>.bin   ← immutable raw response bytes, sha256-named
  <capture_id>.lock               ← advisory writer lock, stale-recoverable
```

The manifest carries `format`, `profile`, `capture_id`, `generation`, repository/revision
identity, the copied native selection, `capture` (limits/usage/stop reason), `coverage`,
`sources` and `citations`. Bodies are stored once and referenced by id, so saving one page
never rewrites a packed export. `--output -` writes the packet to stdout; a file path may
be outside the repository, but packet-supplied paths are never honoured: the destination
comes only from the operator.

Write order is body → fsync → manifest (atomic replace), so a crash between the two leaves
an unreferenced body (harmless, swept on the next run) and never a manifest citing missing
bytes. Reads verify the stored bytes' sha256 before exposing them. The lock is a file
holding pid/start time with a bounded TTL; `--break-lock` recovers an interrupted writer,
and no lock survives its process's death unnoticed.

## Limits (finite, configurable defaults)

| Bound | Default |
| --- | --- |
| Requests per capture | `--request-budget`, 200 |
| Concurrent writers per capture | 1 |
| Response bytes per request | 4 MiB |
| Stored bytes per capture | 64 MiB |
| Pages per endpoint group | 20 (`per_page=100`) |
| Diff bytes / diff lines | 3 MiB / 50 000 |
| Citations per capture | 5 000 |
| Export bytes | 64 MiB (separate from the 1 MiB MCP inline limit) |
| Export byte-window | 16 KiB |

Every attempted HTTP request counts against the budget, including failures and the final
identity checks; capacity for the completion checks is reserved, and when it cannot be
reserved the capture stops explicitly with `request_budget` rather than certifying.
Pagination completion is tracked separately from content coverage: a terminal cursor is
not completeness evidence. Truncated diffs, capped file lists and unsupported/omitted
patches are recorded as unknowns, never as empty responses.

## Acceptance matrix

**Selection and refusal.** One synthetic native batch through the bound gate; association
and provenance recorded; refusal of stale, foreign, unbound, modified, mixed and
park-inconsistent reports; no model/key/network read during `show`.

**One real source path.** Bounded `metadata` acquisition through the production transport
adapter into storage, then offline byte and citation inspection, asserting the real request
line (method, URL, headers, bounds) rather than a replacement returning idealised dicts.

**Full component coverage.** REST pagination per component, GraphQL closing references,
separated CI collections, valid fork evidence, honest truncation and completeness, captured
empty responses (no fabricated citations).

**Interruption, resume, reuse.** Request/storage stops, crash windows (body before
manifest, manifest before completion), concurrent writers, abandoned locks; changed
base/head invalidates the affected code observation; report/prompt/thread-only changes
retain still-valid bytes; mutable components refresh without touching code.

**Portable export.** Independent parsing and citation resolution, exact serialization
limits, corruption refusal, fresh visibility gate, atomic destinations, concurrent-input
refusal, no credentials anywhere in artifacts or diagnostics.

**Citation and hostile-input handling.** Multibyte offsets, binary bodies, empty bodies,
terminal/HTML control content, duplicate JSON keys, non-finite values, integer booleans,
path escape, symlink attacks, hostile `next` links, redirects off the allowed origins,
foreign resource identity, repeated cursors/ids, bad counts, mid-pagination edits.

**Operational.** Human and JSON output, help/README/Makefile integration, `make test` and
`make check` green, existing commands/reports/MCP/workbench unchanged when evidence is
absent, discovery and model caches untouched, no GitHub mutations from evidence commands.

**Remaining work not in this PR.** Workbench publication of evidence and MCP read/retrieve
adapters (bounded schemas, progressive retrieval) are follow-ups; #9 is not closed by this
slice. Ordinary page generation must never copy evidence into `docs/` or the workbench payload.

