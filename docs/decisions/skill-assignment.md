# Skill assignment: synthetic, evidence-bound proposals

Assignment is an optional experimental preparation step, not a GitHub claim,
reservation, approval or automatic mention. All member IDs start with `synthetic-`;
the checked-in resumes describe invented people and invented experience. There is
no integration with real contributor identities.

## Workflow and spending boundary

From the repository root, use the Rust CLI:

```sh
cargo run -p tranche-cli -- qualify --dry-run --limit 1
cargo run -p tranche-cli -- requirements --dry-run --limit 1
cargo run -p tranche-cli -- qualify --limit 1
cargo run -p tranche-cli -- requirements --limit 1
cargo run -p tranche-cli -- assign
cargo run -p tranche-cli -- page --export-json --export-xlsx
```

`qualify` explicitly sends each selected resume to Jev. `requirements` explicitly
sends each selected PR projection to Jev. These are potentially paid calls; review
the dry-run first. A limit caps sources, not individual questions. Both commands
resume current cached records; `--force` deliberately reprocesses them. Provider
failures remain unknown and are reported without echoing private source text.

`assign` uses only local evidence and SolverForge 0.19.7. Page generation, MCP and
the solver never invoke a model. Refresh without a team retains its existing
pipeline. Refresh with team inputs adds only offline assignment; it does not
implicitly qualify resumes or classify required skills.

## Preserve the seven-question judgment contract

The original PR judgment questions are unchanged. Required skills live in a
separate `out/requirements.jsonl` cache, avoiding a full paid corpus rejudge solely
for assignment. `skills.toml` is the shared taxonomy: descriptions feed both
preprocessing question sets. Each skill is an independent supported `noul` in one
batched question set per source, not an invented `multi_label` primitive.

Authoritative primitive documentation:
<https://docs.typesafe.ai/primitives/noul.md> and
<https://docs.typesafe.ai/primitives.md>.

Qualifications live in `out/qualifications.jsonl`. Records bind source state,
complete taxonomy descriptions, questions, model alias and thresholds. PR
requirements additionally bind repository and full PR evidence digest. A change
to resume text, capacity, PR evidence or taxonomy makes affected evidence stale.
The alias `jev-latest` is not a claimed resolved model version. Freshness here is
content/policy binding, not a guarantee against upstream alias drift or elapsed
calendar time; use `--force` when deliberately refreshing that evidence.

## Unknowns and hard feasibility

A probability at least 0.8 qualifies a member or requires a skill. A PR skill
probability at most 0.2 establishes nonrequirement. Intermediate, missing or
invalid PR probabilities hold the unit unknown. Missing/stale qualifications
never fall back to a declared `Skills:` line. A member with incomplete numeric
qualification answers cannot receive work. Valid lower qualification probabilities
do not qualify that skill.

Members are SolverForge problem facts, tasks are nullable planning entities.
Confirmed same-change groups become indivisible units. Capacity counts every PR
in a unit, per member, as a hard bound; overflow stays unassigned. Skills, unknown
holds and security-review qualification are hard constraints. Parked units stay
unassigned. Balancing, affinity and preference for covered work are soft goals,
not a promise of globally optimal ownership. Solver phases come from the pinned
crate's supported public API and TOML format.

## Read gates and privacy

`out/assignments.json` binds the private input plan, report, caches, resume digests,
taxonomy and solver policy/configuration. Consumers rebuild the inputs, validate
complete row coverage, atomic groups, known owners, skills, security, hard capacity
and exact output digest/projection. Modified or stale proposals refuse page/MCP
consumption. Regeneration skips only the old proposal being replaced, not the
ordinary report gates.

Public Markdown, workbench JSON, standalone JSON and XLSX carry only synthetic IDs,
loads, capacities, skills, reasons and proposal metadata: never resume text, paths
or provider responses. Treat local preprocessing caches as private: provider
responses could contain echoed source text. Do not publish these caches. The
workbench Proposed owners queue and MCP `assigned` queue use the same owned-row
membership, excluding missing and explicitly unassigned evidence.

## Verification

`make check` runs Rust integration tests, formatting, clippy and Node tests,
including a real Chromium workbench probe when Chromium is installed. Assignment
tests use synthetic fixture inputs and injected preprocessing transports, not paid
calls. They exercise actual SolverForge execution, asymmetric/zero capacities,
atomic unit size, unknown/security holds, incremental preprocessing, dry-run
non-writes, private exports, tamper/stale refusal and page/MCP queue parity.

This is a synthetic assignment experiment, not production contributor dispatch.
No real resume or corpus-wide paid preprocessing run is required to verify it.
