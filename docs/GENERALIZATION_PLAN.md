# Generalized production readiness plan

Tranche is the generic public-GitHub triage application. Omarchy is the demo
policy, captured-data fixture and published example. This plan closes the gaps
found at `239f639`; it does not add arbitrary model providers or other forges.

## Delivery rules

- Implement each numbered unit as a coherent conventional commit with tests.
- Write and run a regression test before changing behavior.
- Preserve historical judgment/pair bindings and committed report bytes.
- Keep source files below 500 lines; module roots only declare/re-export.
- Use local transport/model fixtures, never paid calls or credentials in tests.
- Run `make check`, full-corpus compatibility tests and packaged-binary smoke
  tests before pushing this branch to `origin`.

## 1. Explain the generic app and demo

- `README.md`: distinguish the application from the Omarchy deployment; explain
  why the backlog is a useful demo; document separate-root initialization and
  current runtime resource requirements.
- `docs/GENERALIZATION_PLAN.md`: record this file-by-file delivery contract.
- Acceptance: documented init and resource setup run against scratch roots.

## 2. Validate deployment contracts

- `crates/tranche-core/src/policy/contract.rs`: delegate boundary validation;
  reject unsupported versions, empty model aliases and malformed policies.
- New `policy/validation.rs` and `policy/repository.rs`: validate the reserved
  v1 question names/types, instructions, criteria and score lengths; share a
  strict GitHub owner/repository parser.
- `crates/tranche-core/src/policy/mod.rs`: export the shared parser only.
- `crates/tranche-cli/src/init.rs`: validate identifiers before writes; preserve
  existing contracts unless force was requested.
- `crates/tranche-core/tests/`: add contract-boundary outcome tests.
- `crates/tranche-cli/tests/`: add init refusal/no-mutation tests.
- Acceptance: unsupported contracts fail before external work; historical v1
  contracts and synthetic fixtures remain valid without digest reshaping.

## 3. Preserve repository identity and empty captures

- `crates/tranche-core/src/domain/pr/error.rs`: validate the repository identity
  of legacy membership shards before projecting them under a contract.
- `crates/tranche-cli/src/reports.rs`: distinguish an observed empty snapshot
  from missing membership and permit bound zero-count reports.
- Core/CLI integration tests: cover foreign legacy shards, overlapping PR
  numbers, valid empty captures and absent captures.
- Acceptance: foreign membership cannot be judged; an empty backlog replaces
  stale reports with a valid zero-count observation.

## 4. Drain subprocess output while fetching

- `crates/tranche-core/src/gh.rs`: drain stdout/stderr concurrently, enforce
  output bounds and preserve timeout cleanup; extract a sibling module if needed.
- `crates/tranche-core/tests/reading_github.rs`: cover output beyond pipe
  capacity, large stderr, timeout cleanup and retained snapshots.
- Acceptance: a large valid page finishes without pipe deadlock.

## 5. Make model passes durable and failure-aware

- `crates/tranche-cli/src/judgment.rs`, `dupes.rs`: checkpoint completed model
  responses during execution instead of waiting for the entire pass.
- New CLI checkpoint helper: repair/quarantine unterminated tails, serialize
  append operations and preserve the previous usable fresh-pass generation.
- `crates/tranche-cli/src/refresh.rs`, `commands.rs`: propagate per-request
  failure counts; stop downstream publication on failed passes.
- Judge/dupes/refresh integration tests: interruption/resume, partial failures,
  fresh-pass preservation and one-pass truncated-tail recovery.
- Acceptance: completed work survives interruption; failed composite passes
  return nonzero and do not claim completion.

## 6. Unify validated report reads

- `crates/tranche-cli/src/workbench/page.rs`: use the shared bound-report gate
  rather than independently checking a subset of batch invariants.
- `crates/tranche-cli/src/commands.rs`: make verified info use the same gate.
- `crates/tranche-core/src/policy/mod.rs`, `README.md`: explain that subject
  edits require offline batch regeneration, not new model judgments.
- `crates/tranche-core/tests/the_report_gate.rs`: copy contracts to the root,
  not out/, and retain negative-gate coverage.
- Page/MCP tests: subject changes, modified batches and matching acceptance.
- Acceptance: page/export/MCP/info agree on whether an observation is current.

## 7. Ship deployment resources and generic navigation

- New CLI workbench resource module: embed default template, CSS/JS and required
  generic images; provision resources without a source checkout.
- `crates/tranche-cli/src/init.rs`: provision default resources safely.
- `crates/tranche-cli/src/workbench/writing.rs`: support embedded defaults and
  explicit local overrides; ensure referenced assets are installed.
- `crates/tranche-cli/src/workbench/page.rs`: allow standalone exports without
  a deployment-local HTML template.
- `page/template.html`, `docs/assets/workbench.js`: generic default branding and
  PR links derived from validated deployment repository identity.
- Workbench payload/export code: carry the repository identity to the browser.
- Browser/render/bootstrap integration tests: second-repository links, missing
  local templates, all resource references, and packaged-binary execution.
- `README.md`: replace manual resource-copy instructions with the final behavior.
- Acceptance: an unpacked binary initializes and renders a non-Omarchy deployment
  with working assets; JSON/XLSX exports do not require HTML setup.

## Final acceptance

Run the full standard gate, the local full-corpus binding/reproduction/gate
suite, and a release-binary smoke test outside the checkout. Audit the atomic
history and working tree, push `explore/generalize-tranche` to `origin`, and
read the remote branch ref back to confirm it equals the local commit.
