# Tranche

<img src="docs/assets/tranche-mascot.png" alt="Tranche, a watchful geometric owl holding a bundle of three pull-request cards" width="200">

**Tranche, the backlog keeper.** Spots duplicates and gathers fixes into reviewable
batches. Jev supplies the judgments; humans make the merge call.

MIT licensed (see [LICENSE](LICENSE)); Tranche reads public repositories only and
never includes credentials in its reports.

**Live report:** <https://vdistefano.studio/Tranche/>

Tranche exists because the Omarchy PR backlog hit 2,800 open PRs growing by
~100 a day. DHH stood up a human triage team on 2026-09-12
([x.com/dhh/status/2098755120540393908](https://x.com/dhh/status/2098755120540393908))
and asked for exactly three things: roll fixes into clusters that can be
trusted, consolidate duplicates, and make sure everything reaches him in a
finished form. Tranche is the tool side of that job. It uses
[TypeSafe](https://docs.typesafe.ai)'s System One model **Jev** to judge PR
descriptions, and it is built for the team DHH described — including his
boundary from the follow-up reply: *"I'm not delegating the user experience."*
Taste-level PRs get their own `user-experience` lane that the pipeline prepares
but never pre-judges.

## What Tranche actually does

- **Fetches** the open PRs of `omacom/omarchy` into a local snapshot (via `gh`, already authenticated).
- **Judges** each PR with one batched Jev call: what area it touches, how risky
  it is, whether it's a fix, how finished it looks, how much review effort it
  needs, whether it smells security-relevant, and whether it admits to
  duplicating something.
- **Finds likely duplicate groups** by comparing candidate pairs (title/body
  similarity plus cross-references between PRs).
- **Batches** confirmed groups into merge units of up to five PRs, security
  batches first — each batch ships a ready-to-paste reviewer prompt.
- **Publishes** everything: a browsable HTML workbench and a read-only MCP
  server that AI agents can question directly.

It is a discovery and prioritization pass. It never merges, never closes, and
its "duplicates" are model suggestions, not verified facts. Maintainers decide
what happens to a PR — always.

## Quick start

You need Python 3.10+ and the [`gh` CLI](https://cli.github.com) signed in.
A Jev API key is only needed for the model stages (`judge`, `dupes`);
fetching, clustering, rendering and tests run offline.

```bash
git clone https://github.com/blackopsrepl/Tranche && cd Tranche

# one-time: give the model stages a key
echo "apikey_..." > ~/Documents/jevapi.txt      # or: export TYPESAFE_API_KEY=...

python3 tranche.py refresh                       # the whole pipeline, incremental
```

That single command runs `fetch → judge → dupes → cluster → batches → page`
in a fixed order and reuses everything its caches can still support. The
first run judges the whole backlog (thousands of model calls); every run
after that pays only for what actually changed. Open `docs/index.html` when
it finishes — or just browse the live report linked above.

Check what a refresh *would* do before spending anything:

```bash
python3 tranche.py refresh --dry-run
```

## Everyday commands

```bash
python3 tranche.py refresh           # normal day: whole pipeline, incremental, deterministic
python3 tranche.py fetch             # re-capture open-PR membership (authenticated via gh)
python3 tranche.py judge --resume    # model pass; resume reuses matching judgments only
python3 tranche.py dupes             # re-run the candidate-pair comparisons
python3 tranche.py cluster           # rebuild out/{clusters.json,dupes.json,tranches.md,summary.json}
python3 tranche.py batches           # rebuild out/batches.json and parked.json
python3 gen_page.py                  # render docs/index.html + workbench payload
```

Cost intuition: a refresh with nothing new spends **zero** model calls. A full
corpus re-judge is ~2,800 calls (~5M input tokens) and only happens when the
question policy or the model changes — which is rare and deliberate. A dupe
pass over all candidate pairs is a few hundred thousand tokens. The dry-run
tells you which situation you are in before you commit to it.

## The MCP server

Tranche can serve your local reports to AI agents over the
[Model Context Protocol](https://modelcontextprotocol.io) — the same data the
workbench shows, plus the per-batch reviewer prompts. The server is a single
self-contained Python file. No SDK, no package install, no daemon, no
database: your client launches it, it answers over stdio, done.

### Starting it

```bash
python3 mcp_server.py --root /absolute/path/to/Tranche
```

The `--root` must be a Tranche checkout that already has reports — run
`refresh` at least once. Every MCP client config expresses the same two
fields (command + args) under its own names; the generic shape is:

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

The server speaks standard JSON-RPC 2.0 over newline-delimited stdio
(`initialize`, `tools/list`, `tools/call`, `ping`), negotiating protocol
versions 2024-11-05 through 2025-11-25. Any conforming MCP client can drive
it; no client is named or required. Tools advertise JSON Schema input schemas
and standard `readOnlyHint` annotations. Invalid arguments and stale reports
come back as tool errors with `isError: true` — so an agent can read the
message and correct itself — and unknown tools get protocol error -32602.

### The six tools

| Tool | Answers the question |
| --- | --- |
| `surface()` | "What's in the corpus right now?" Coverage, category counts, queue sizes, batch inventory, parked state, activity counts, available filters. |
| `query(...)` | "Which PRs match this?" Filter by text, category, risk band, security, exact finished-form score, batch or queue; security-first ordering, paginated. |
| `pick(batch_id)` | "What exactly is batch B042?" Members with source bindings and the exact reviewer prompt the workbench would copy. |
| `next_prompt(after)` | "What's the next batch to review?" Walks batches in report order; stateless — no reservations, nothing is marked done. |
| `related(number)` | "What is this PR related to?" Model relationship evidence with explicit uncertain/contradictory diagnostics. |
| `digests()` | "Am I looking at the right report?" Report binding and SHA-256 digests of every input and output read. |

Every result carries the repository, provenance digests and a disclaimer:
model suggestions from titles and shortened descriptions — never merge
approval, never verified duplication. Unknown values stay `null`, never zero.

### Updating and maintaining it

The server has **no state of its own**. It reads the report files under
`--root` on every call and revalidates their binding digests each time, so
"updating" means updating the reports, not the server process:

1. **New data:** run `python3 tranche.py refresh` (or the individual stages).
   The next `surface()` call reflects it. No server restart needed for data.
2. **New code:** `git pull`. This is the one case where you *must* restart the
   server process — a long-running process keeps its old code in memory and
   will refuse the new reports with a staleness error. Kill it; the client
   respawns a fresh one on its next call.
3. **New categories or tools:** clients cache the tool list per session, so
   reconnect the client (or start a new session) after a schema-changing
   update even if the server itself is fresh.

Health checks, cheapest first: call `digests()` and compare
`report_binding` against `out/summary.json`; discover the tools with
`hermes mcp test tranche` (on Hermes) or your client's equivalent; or run the
full integration check against a disposable copy of the corpus:

```bash
make mcp-check MCP_PYTHON=/path/to/python-with-mcp-sdk
```

When something goes wrong:

| Symptom | What it means | Fix |
| --- | --- | --- |
| every tool says "Reports are stale, unbound, foreign or modified; rerun cluster" | the report files and the code that validates them disagree | restart the server process; if the reports really are old, run `refresh` |
| a new category or tool is missing in your client | the client cached its session's tool list | reconnect the client / start a new session |
| `pick` / `next_prompt` fail, other tools work | no batch file in this report root | `python3 tranche.py batches` |
| refusal naming an argument or enum value | the call didn't match the advertised schema | fix the arguments; the message lists what is accepted |
| everything fails on a copied directory | the copy is missing snapshot, caches or matching `out/` reports | serve a complete report root, or run the pipeline there |

Hard limits, by design: results paginate at 25 items (max 100), query text
caps at 512 characters, input files at 128 MiB (256 MiB total, 512 paths),
responses at 1 MiB. `--allow-unbound` inspection reports are refused outright.

## FAQ

**Does Tranche merge or close anything?**
No. The pipeline and the MCP server are read-only toward GitHub. Nothing
claims, reserves or approves work.

**What does "duplicate" mean here?**
A Jev judgment that two PRs propose the same underlying change, with the
model-consistency checks described below. It is a merge-review lead, not a
verified duplicate — always compare sources before closing anything, and note
that PR age never selects a survivor.

**Why is a risk score wrong?**
Because it judges `title + body + diffstat` only. It never reads patches,
test results or CI. Treat scores as sorting hints for human review, and treat
`unknown` (null) as unknown — the pipeline never fakes a zero.

**What does a refresh cost?**
Nothing when nothing changed. Rough numbers for this backlog: full re-judge
~2,800 calls / ~5M input tokens (only after a question-policy or model
change), a full dupe pass a few hundred thousand tokens. `refresh --dry-run`
reports the situation without spending anything.

**Why did all judgments suddenly invalidate?**
The question policy or the model changed, so every cached answer no longer
answers *today's question*. That invalidation is the point — it is how a new
category like `user-experience` becomes real instead of cosmetic. The next
`refresh` re-judges the corpus once; expect a full-corpus pass, and run the
dupe stage with a raised `--max-pairs` (e.g. `refresh --max-pairs 900`) so
the pair comparisons aren't cut off mid-pass.

**The MCP server says "reports are stale" right after I pulled new code.**
A running server process holds its old code in memory. Restart it (see the
maintenance table above); your client will relaunch it automatically.

**Do I need a Jev key to try this?**
Not for `fetch`, `cluster`, `page`, the MCP server, or the tests. Only
`judge` and `dupes` call the model.

**Where do the reviewer prompts come from?**
Each batch carries one, stored in `out/batches.json` and byte-identical to
the workbench's Copy button. It is review *instruction* for an agent, not
executable authorization.

**Can several agents share one server?**
Each client launches its own server process; the processes are stateless and
read-only, so they can't get in each other's way. The shared state is the
report directory itself.

**How do I cut a release?**
See [Versioned releases](#versioned-releases) — one command plus a push, and
the GitHub release publishes itself.

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

## How the evidence binds

Judgments bind the fields they were based on — title, body, head SHA, labels,
draft state, diffstat, timestamps, the exact question set and the model — not
the whole GitHub response envelope, whose repository-wide counters change on
unrelated events. A re-fetch that changed nothing reuses everything; a changed
description re-asks. The same discipline applies to pair verdicts. Resume
reuses only matching records, and matching-but-malformed responses are
reported as unknowns, never silently dropped. A floating alias like
`jev-latest` can drift under its fixed name; pin a concrete model or re-run
`judge` when that matters.

`fetch` commits exact observed membership atomically — a failed capture
leaves the previous snapshot untouched — and runs through `gh api` so the
credential never touches a command line. Unauthenticated GitHub allows 60
requests/hour and one capture costs ~29. Pagination is not a point-in-time
snapshot: a captured PR may have changed by the time you read it.

Published historical judgments have no bindings and cannot be retrofitted;
`judge --resume` re-evaluates them at API cost. `--limit N` (judge) and
`--max-pairs N` (dupes) bound work per pass. For offline inspection only,
`cluster --allow-unbound` includes legacy judgments with warnings; they never
enter review candidates or batches.

## Outputs

- `out/tranches.md` — review candidates, relationship diagnostics, risk/security
  escalation leads, author follow-up leads, and the batch plan.
- `out/clusters.json` — judgments grouped by category × risk band
  (`low`, `core`, `danger`, `unknown`), with source digests and URLs.
- `out/dupes.json` — `confirmed_groups` (model-consistent candidates, **not
  verified duplicates**), `review_groups` and `uncertain_pairs`.
- `out/batches.json` — merge batches bound to the `dupes.json` digest;
  `out/parked.json` — PRs held out of batches (drafts, unfinished forms,
  unjudged/stale, same-change holds) with their unblock paths.
- `out/summary.json` — counts, token usage, report binding and output digests.
- `docs/index.html` + `docs/data/workbench.json` — the rendered workbench.

Review-candidate thresholds: risk ≤ 1.5, finished_form ≥ 1.8, is_fix ≥ 0.6,
security_flag < 0.5, current judgment, not a draft, outside a candidate
group. A missing field can never qualify a PR.

## Security meta-category (top priority)

Security is a cross-cutting classification over **all** PRs, not a category
slot: every PR whose `security_flag` reaches 0.5 joins a `security-review`
set that outranks every category — first section of `tranches.md`, first
queue of the workbench, top of the sidebar. Review these before any batch.
Membership never replaces a PR's own category, and an unknown probability is
never flagged.

## Batches

A **batch is a Jev-determined group of PRs to merge into ONE pull request**:
exactly the model-consistent `same_change` groups, disjoint by construction.
Groups with contradictory or untested internal evidence and uncertain pairs
are excluded on purpose — being unbatched is the honest state for them, and
`out/parked.json` records why each held-out PR is there. Batches are ordered
security-first, then risk band, then activity: `updated_at` on this repo is
continuous bot churn, so the activity signal is the head SHA bound at
judgment time ("head revised" = the author pushed since). Batches are a
plan, never a verified-safe merge.

## Development

```bash
make test          # unit tests, synthetic inputs, no network, no credentials
make check         # tests + lint + optional real-browser workbench probe
make mcp-check MCP_PYTHON=/path/to/venv-python   # opt-in real-client MCP integration
```

## Versioned releases

One coherent iteration produces one release tag. `VERSION` and
`CHANGELOG.md` are owned by [commit-and-tag-version](https://github.com/absolute-version/commit-and-tag-version)
(`.versionrc.js`); never edit them by hand. Developer tooling: Node,
`commit-and-tag-version` and Ruff — no runtime Python dependencies.

```bash
git pull --ff-only origin main
make release-dry-run        # offline checks, clean-main gate, preview; no writes
npx commit-and-tag-version --release-as minor   # or plain for the computed bump
git push --follow-tags origin main
```

The pushed `v*` tag triggers the `release` workflow
(`.github/workflows/release.yml`), which publishes the GitHub release from
that tag's own `CHANGELOG.md` section — notes can never drift from the
changelog. A tag that predates the workflow (or any existing tag) can be
published retroactively through the same path:

```bash
gh workflow run release --ref main -f tag=v0.8.1
```

Verify afterwards with `gh release list` — the releases page, not the tag
list, is what people see. Releases version the code, not the freshness or
correctness of historical model judgments.
