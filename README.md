# Tranche

<img src="docs/assets/tranche-mascot.png" alt="Tranche, a watchful geometric owl holding a bundle of three pull-request cards" width="200">

**Tranche, the backlog keeper.** Spots duplicates and gathers fixes into reviewable
batches. Jev supplies the judgments; humans make the merge call.

MIT licensed (see [LICENSE](LICENSE)); Tranche reads public repositories only and
never includes credentials in its reports.

Model-assisted discovery and review prioritization for the Omarchy PR backlog, built for the triage team DHH stood up
on 2026-09-12 ([x.com/dhh/status/2098755120540393908](https://x.com/dhh/status/2098755120540393908)):

> "We're 2,200 PRs deep on GH now and getting nearly a hundred new ones every day. I'll never
> be able to catch up. Agents will help, but we need humans too. If you have DEEP Linux
> experience, is agent-forward, and want to join the new Omarchy triage team, write
> triage@omarchy.org."
>
> "What we need in particular is people who can roll-up batches of fixes into clusters that I
> can trust are fully reviewed … so I can merge whole tranches of fixes into core."
> "Omarchy Triage can help consolidate PRs, remove dupes, and ensure that everything is ready
> for consideration in a finished form."

**Live report:** <https://vdistefano.studio/Tranche/>

This tool uses [TypeSafe](https://docs.typesafe.ai)'s System One model **Jev**
to suggest review candidates and related PR groups. Code owns the workflow;
Jev supplies judgments about the descriptions it receives. This is a discovery
and prioritization pass, not completed QA, verified duplicate detection or a
security review. Maintainers decide what to merge or close.

## Setup

```bash
# Needed only for judge/dupes; fetch, cluster and tests do not use a model key
echo "apikey_..." > ~/Documents/jevapi.txt     # or: export TYPESAFE_API_KEY=...
```

## Usage

```bash
python3 tranche.py refresh          # whole pipeline, incremental, deterministic — the normal way
python3 tranche.py refresh --dry-run # report what a refresh would re-run; writes nothing
python3 tranche.py fetch            # atomically replace data/pages/snapshot.json (authenticated via gh)
python3 tranche.py judge            # Jev pass over all PRs  (~7 questions, one call per PR)
python3 tranche.py judge --resume   # reuse only matching input/question/model bindings
python3 tranche.py dupes            # compare candidate pairs using shortened descriptions
python3 tranche.py cluster          # build out/{clusters.json,dupes.json,tranches.md,summary.json}
python3 tranche.py batches          # batch candidates into out/batches.json, park the rest (issue #8)
python3 tranche.py all --resume     # judge --resume + dupes + cluster + batches
```

`refresh` is the whole pipeline in the fixed order
`fetch → judge --resume → dupes → cluster → batches → page`, and it is
**incremental**: every stage reuses what its cache can still support. A PR is
re-judged only when the evidence a judgment was based on actually changed, and a
pair verdict is re-run only when one of its PRs did. A refresh with nothing new
spends no model calls. `--max-pairs N` caps the pair comparisons per pass
(default 400).

The evidence binding is what makes that safe: judgments and pair verdicts bind
the captured fields a judgment depends on — title, description, head SHA, labels,
draft state, diffstat, timestamps — not the whole GitHub pull-list envelope.
Repository-wide counters that ride along in every response (`stargazers`,
`forks`, `open_issues`, `pushed_at`) change on unrelated events and would
otherwise invalidate the entire corpus on every fetch.

## Read-only MCP access

Agents can inspect the same local reports and retrieve batch reviewer prompts over
stdio without a browser or copy/paste. The server implements the standard MCP
stdio transport directly with the standard library: no SDK, no third-party runtime
dependency, nothing to install or pin. It is harness-agnostic — any MCP client that
speaks the protocol can drive it, and none is named or required here.

```bash
python3 mcp_server.py --root /absolute/path/to/Tranche
```

Point your client at that command. Every client expresses the same two fields —
the executable to launch and its arguments — under whatever name its own config
file uses, so the shape below is a reference, not a supported-client list:

```json
{
  "mcpServers": {
    "tranche": {
      "command": "python3",
      "args": ["/absolute/path/to/Tranche/mcp_server.py", "--root", "/absolute/path/to/Tranche"]
    }
  }
}
```

Protocol: JSON-RPC 2.0 over newline-delimited stdio (`initialize`, `tools/list`,
`tools/call`, `ping`), negotiating protocol versions 2024-11-05 through 2025-11-25.
Tools advertise JSON Schema input schemas and the standard `readOnlyHint`,
`destructiveHint`, `idempotentHint` and `openWorldHint` annotations. Unknown tools
return protocol error -32602; invalid arguments and stale reports return tool results
with `isError: true`, so a model can correct itself.

The server is a single self-contained script: no packaging, no build step, no
service. Its `--root` may point at any Tranche report directory. Everything else
runs through one interpreter — `make print-interpreter` shows which, and setting
`PYTHON` overrides it everywhere (Makefile targets, tests, and your client
config). If your client cannot launch the interpreter by name, use its absolute
path.

The root defaults to the directory containing `tranche.py`, not the client's
working directory. It must contain the captured PR snapshot (or legacy pages),
judgment/pair JSONL caches and matching `out/` reports. Run `cluster` and `batches`
with the pipeline before serving them. The server never acquires missing evidence,
regenerates reports, calls Jev, claims work, or writes to GitHub.

| Tool | Contract |
| --- | --- |
| `surface()` | Computed corpus coverage, category counts, priority queue membership, batch ordinals, parked state with unblock paths, head-activity idle counts and available filters. |
| `query(...)` | Text, category, risk band, security, exact finished-form score, batch and queue filters; security-first ordering; bounded pagination. Rows carry a `head_moved`/`idle_since` activity label. |
| `pick(batch_id)` | The batch, member source/head bindings, per-member head-activity labels and the exact workbench `review_prompt`. |
| `next_prompt(after)` | The next ordinal after an integer or batch ID; omitted cursor starts at the first batch. Exhaustion returns `batch: null`. Annotates the batch's members with head-activity labels. Stateless: no reservation or completed-work tracking. |
| `related(number)` | Proposed group edges and explicit uncertain/contradictory/malformed diagnostics, with verdict and P(same) where available. |
| `digests()` | Report binding, output digests and SHA-256 hashes of the input/report bytes read. |

Every result carries the repository, provenance, digests and metadata-only evidence
warning. Unknown values remain null/unknown. Related does not mean verified duplicate;
prompts are review instructions, not executable authorization. Digests establish local
consistency, not source authenticity or current GitHub state. Recheck revisions before
acting on a PR.

Every call revalidates report bindings, output digests and exact batch recomputation;
changed/mixed reports and files changed during reading fail with MCP tool errors.
Historical stale/unbound cache rows are excluded exactly as in the producer, not
served as claims. Missing `batches.json` permits inventory/query/related tools but
prompt tools fail; a present invalid batch file fails all tools. `--allow-unbound`
reports are refused. Repair inputs with the pipeline rather than bypassing this gate.

Query and related results default to 25 items (maximum 100), with `next_offset` for
pagination. Queries accept at most 512 text characters. Inputs are limited to
128 MiB per file, 256 MiB total and at most 512 paths; response JSON text to 1 MiB.
Arguments reject coercion, unexpected fields and invalid ranges. The client launches
the server process; protocol stdout stays free of banners.

For an explicit real-client subprocess check against the unchanged local corpus
(the MCP package is blocked inside the server process, proving it is not borrowed):

```bash
make mcp-check MCP_PYTHON=/path/to/python-with-mcp-sdk
```

This opt-in check discovers all tools, reads a real batch/prompt, rejects malformed
arguments and refuses altered batches in a disposable copy. Normal `make check`
exercises the SDK-independent core with synthetic inputs and skips this integration.

## What Jev is asked (one batched call per PR)

| Question      | Type   | Meaning                                             |
|---------------|--------|-----------------------------------------------------|
| `category`    | Choice | install-setup / desktop-config / user-experience / shell-cli / apps-integrations / hardware-drivers / update-release / agents-ai / docs / fix-misc / unclear |
| `risk`        | Score  | 0 text-only → 4 could break existing installs        |
| `is_fix`      | Noul   | P(bug fix, not feature/taste change)                 |
| `dupe_signal` | Noul   | P(title/body admits duplication or supersedence)     |
| `finished_form` | Score | 0 no description → 3 what+why+QA evidence           |
| `review_effort` | Score | 0 trivial → 3 substantial                           |
| `security_flag` | Noul  | P(touches secrets/sudo/remote-code/network exposure) |

## Evidence and candidate groups

The per-PR projection includes title, author, draft status and at most 1200
cleaned body characters. Pair comparisons receive titles and at most 400 body
characters each. These calls do **not** inspect patches, test/CI results,
reproductions, merged history or full fix coverage. `finished_form` reflects
described testing, not testing performed by this pipeline. Risk/security scores
are model suggestions; their probabilities have not been independently calibrated.

The GitHub PR-list response generally omits diffstat. Missing or invalid counts
are explicitly **unknown**, never zero. Enriched legacy pages can provide real
counts, but neither input path verifies patch contents.

Title similarity (SequenceMatcher ≥ 0.72 or Jaccard ≥ 0.62) within a model category,
plus body references, proposes pairs. Body references are read literally and
repository-qualified (issue #10): `#123`, `omacom/omarchy#123` and a pasted
`github.com/omacom/omarchy/pull/123` link all name the same PR of the reviewed
repository, while `omacom/omarchy-pkgs#123` or a link to any other repository is
dropped instead of being re-read as a bare number — it is never mistaken for
Omarchy's own #123. A PR's own number is never a reference, so a description
cannot pair a PR with itself. A reference only selects a PR for comparison; it is
not evidence of duplication. `same_change` with P(same) ≥ 0.65 proposes a
connection. A connected group is only model-consistent when **all** its internal
pairs were tested and agree. Contradictory, uncertain, untested or unbound internal
relationships go to `review_groups` with their diagnostics. The shared pair classifier
also exposes standalone contradictory or malformed responses in `uncertain_pairs`,
Markdown and HTML: a different-change verdict with P(same) ≥ 0.65, or a same-change
verdict with P(same) < 0.35, contradicts its probability. The middle band remains
uncertain, not contradictory. A valid different-change verdict with P(same) < 0.35
remains strong difference evidence (and a conflict inside a connected group), not
an undecided standalone pair. Invalid verdicts or probabilities are malformed.
Even a consistent
model group still needs source comparison. PR age does not select a survivor;
no member is automatically marked superseded.

## Outputs

- `out/tranches.md` — model-suggested review candidates, relationship diagnostics,
  risk/security escalation leads and possible author follow-up.
- `out/clusters.json` — matching judgments by category × model risk band
  (`low`, `core`, `danger`, `unknown`); includes source digest, head SHA and URL.
- `out/dupes.json` — `confirmed_groups` (model-consistent candidates, **not verified
  duplicates**), `review_groups` and `uncertain_pairs`.
- `out/batches.json` — suggested merge batches per category of work
  (issue #4), bound to the `dupes.json` digest recorded in `summary.json`.
- `out/summary.json` — counts, token usage for selected records, input binding and
  output digests. Historical `ready_*` keys now count **review candidates**;
  `superseded` is zero and `superseded_by` is null. `security_priority` counts
  the security meta-category.
- `python3 gen_page.py` — render the matching report to `docs/index.html` plus the
  workbench payload `docs/data/workbench.json` (fetched by the page at boot). Refuses
  legacy, changed or mixed report inputs until `cluster` is rerun.

Review-candidate thresholds remain risk ≤ 1.5, finished_form ≥ 1.8, is_fix ≥ 0.6,
security_flag < 0.5, outside a candidate group. All required numeric fields
(including review effort) must be valid; the judgment must be current and the PR
must not be a draft. Missing judgment fields cannot qualify an item.

## Security meta-category (top priority)

Security is a meta-classification over **all** captured PRs, not a category
slot: every PR whose `security_flag` probability reaches 0.5 joins a
cross-cutting `security-review` set that outranks every category. It is the
first section of `tranches.md` (probability-first order), the `security-review`
key of `clusters.json`, the leading **Security first** queue of the workbench,
and a **Security (meta)** entry at the top of the category sidebar — the
answer to "which of all PRs are security related?". Flagged rows carry a red
tag and the inspector shows the probability. Membership never replaces a PR's
own category; an unknown security probability is never flagged. Review these
before any batch.

## Suggested pre-release batches

A **batch is a Jev-determined group of PRs to merge into ONE pull request**
(`tranche.py batches`, issue #4): exactly the model-consistent `same_change`
groups from the dupe pipeline. Batches are **disjoint** — every PR belongs to
at most one batch — and groups with contradictory or untested internal
evidence (review groups) plus uncertain pairs are excluded on purpose.
Batches are ordered security-first (batches containing security-related PRs
merge first), then by risk band, then by the newest evidenced idle bound in the
group: the head revision bound at judgment time is the activity signal, because
`updated_at` on this repository is continuous bot churn (issue #11). A PR whose
head changed since its bound judgment is "head revised" — it sinks in the order
and is flagged — without any extra GitHub or model calls; each batch becomes one
cumulative PR of the final deliverable. Batches are a model-suggested
plan, never verified safe to merge. Output: `out/batches.json` (bound to the
dupes digest; `gen_page.py` refuses a stale file), the batch plan appended to
`out/tranches.md`, and the **Batches** view with per-batch member browsing,
reviewer agent prompts and per-PR batch detail in the workbench. PRs outside a
confirmed group are intentionally
unbatched.

## Freshness and migration

`fetch` commits exact observed membership in one atomic local snapshot, including
empty results and page-boundary endings. Failed pagination leaves the previous
snapshot untouched. Captures run through `gh api`, which is already
authenticated — unauthenticated GitHub allows 60 requests an hour and one
capture of this backlog costs about 29 — and the credential stays inside `gh`
rather than being passed on a command line where `ps` would expose it.
`--transport curl|urllib` remain for hosts that need the original unauthenticated
paths. `make refresh` runs the incremental pipeline; `make all` still runs the
stages in order. GitHub pagination is not a point-in-time snapshot: PRs can
change during acquisition, and this tool does not certify that a captured item is
still open when read later.

Each new judgment binds the repository, the captured PR evidence digest, actual
projected model input, questions, requested model and binding version — where the
evidence digest covers the fields a judgment depends on and deliberately excludes
the repository-wide counters that change on unrelated events. Pair records bind
both PRs' evidence and the pair questions. A record whose binding no longer
matches is still reusable when its own stored projection is byte-identical today
and the model alias is unchanged, which is how judgments taken before evidence
digests existed keep verifying exactly as they were judged. Resume reuses only
matching records; closed, changed or differently
configured inputs are excluded from reports. Matching but malformed responses remain
reportable as unknown values with `normalization_errors`; they are not resume hits.
Required category, risk, finished-form, effort, fix and security answers must be valid
for judgment reuse (the advisory dupe signal is optional). Pair reuse requires a valid
verdict and finite P(same). New responses and existing JSONL caches share normalization:
invalid metrics become null, valid evidence is retained, non-finite numbers are removed,
and usage counters become nonnegative integers (invalid/missing counters become zero).
Malformed JSONL records without valid positive integer identities are ignored. Recovery
is in memory and never adds bindings to legacy records. New JSONL writes are strict JSON.
New records retain the input,
requested model, returned model/request ID when supplied, and judgment time.
The local digest detects changed captured bytes, not authenticity or live freshness.
A floating model alias such as `jev-latest` can move without changing the requested
name; use a fixed supported model name or rerun `judge` to force reevaluation.

**Published historical judgments have no bindings and cannot safely be retrofitted.**
`judge --resume` will reevaluate them, which incurs API cost. Run `judge --resume
--limit N` and `dupes --max-pairs N` to bound work per pass (`all --resume --limit N
--max-pairs N` also works). Reports explicitly count excluded unjudged/stale items.
For offline inspection only, `cluster --allow-unbound` includes legacy judgments
with warnings; they never enter review-candidate tranches. Bound-but-stale records
remain excluded even in this mode. Existing published output files are retained
as historical artifacts rather than regenerated without their original inputs.

## Offline regression checks

```bash
python3 -m unittest discover -s tests -v  # or make test
```

Tests use synthetic inputs and mocked transports/model responses. They make no
network calls, require no credentials and do not modify the published reports.

## Versioned releases

Release tooling requires Node and `commit-and-tag-version` (install with
`npm install --global commit-and-tag-version@12.5.0`), plus Ruff for `make check`.
These are developer tools, not runtime Python dependencies. `VERSION` is the only
version surface; `.versionrc.js` owns its bumps, generated `CHANGELOG.md`, the
`chore(release)` commit and `v` tag. Never update those by hand.

After merging conventional fix/feature commits, release from a clean, up-to-date
`main`:

```bash
git pull --ff-only origin main
make release-dry-run  # offline checks, clean-main gate, preview; no writes/tags
make release         # same gate, then tool-generated version/changelog/commit/tag
git push --atomic origin main "$(git describe --exact-match --tags HEAD)"
```

Without `--release-as`, the tool chooses the next version from conventional
commits. One coherent iteration produces one release tag. Inspect the generated
changelog and verify the remote branch/tag after pushing. Releases version the
code, not the freshness or correctness of historical model judgments; neither
release target calls GitHub or Jev or regenerates captured evidence.
