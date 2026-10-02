# Tranche is a Rust project

`crates/tranche-core` holds the logic and `crates/tranche-cli` is the `tranche`
binary. Dependency direction is one-way: the binary depends on the library, and
the library never depends on a transport.

## Layout

```
Cargo.toml                      workspace root
crates/
  tranche-core/                 lib — logic, no I/O transport
    src/report/                 path vocabulary, input digests, binding, the gate
    src/domain/pr/              membership, identity, evidence digests
    src/domain/judge/           question policy, normalization, resume binding
    src/domain/dupe/            candidate pairs, similarity, stored verdicts
    src/domain/cluster/         grouping, predicates, the report and its binding
    src/domain/batch/           packing, park decisions, report sections
    src/domain/questions.rs     the frozen question policy
    src/evidence/               selection, coverage, manifest, lock, store
  tranche-cli/                  bin `tranche`
    src/cli.rs                  the command tree
    src/commands.rs             dispatch
```

Dependency direction is one-way: `tranche-cli` → `tranche-core`. Core never
depends on a transport.

## The on-disk artifacts are the contract

`out/` is the interface between the pipeline and everything that reads it, so
the code is judged by whether it reproduces those bytes — not by whether it
looks correct. `crates/tranche-core/tests/the_published_report.rs` is that
judgement: it rebuilds the report from the stored corpus and asserts equality
against the committed files.

Three properties of the format look like accidents and are not, because each one
silently changes a tracked artifact when it is got wrong:

1. **Key order is insertion order, not sorted.** Categories in `clusters.json`
   and the sections of `tranches.md` follow first appearance. The digests cannot
   catch a mistake here, because they sort keys explicitly.
2. **Floats keep the shortest round-tripping form.** `0.6` stays `0.6` and
   `0.5700000000000001` keeps all seventeen digits; rounding to a fixed number
   of places is wrong.
3. **JSON separators carry a space** after each comma and colon, so a write that
   uses a compact serializer restyles every artifact.

`serde_json` needs `arbitrary_precision` (or a float parses to a different
double than the stored one) and `preserve_order` (or an object is written
sorted). Both are load-bearing, and the crate documents them as such.

## Rules

1. **No source file reaches 300 lines.** Split by responsibility into sibling
   modules; a module that needs three concepts is three modules. This applies to
   every crate's `src/`, not to tests.
2. **Tests live only in `tests/`**, never in a `#[cfg(test)]` module inside a
   source file, and they are named for the outcome they protect rather than the
   module they exercise. A test named after a module drifts into restating the
   implementation; a test named after an outcome fails when the behaviour breaks.
3. **`gh` stays a subprocess.** The GitHub credential stays inside `gh`; do not
   reimplement GitHub auth.

## Not wired yet

Dispatch exists for `cluster`, `batches` and `info`. `fetch`, `judge`, `dupes`,
`refresh`, `all`, `page` and the `evidence` verbs refuse until their transports
land. Jev is a plain HTTPS POST, so `judge` and `dupes` need an HTTP client and
nothing else.

Two pieces of the codebase are still to be ported, and `mcp_server.py` and
`tests/test_mcp_server.py` are kept as the reference they will be verified
against rather than deleted:

1. **The read projection.** The workbench and the MCP server each reconstruct
   the report today. One `view` module should serve both, so the two surfaces
   cannot disagree.
2. **`tranche-mcp`.** A second binary, not a subcommand: clients spawn one
   process and speak JSON-RPC over stdio. The six tools are `surface`, `query`,
   `pick`, `next_prompt`, `related` and `digests`. Its exit criterion is that a
   real MCP client completes `initialize` → `tools/list` → `tools/call`, with the
   1 MiB response cap, 25/100 pagination and error `-32602` for unknown tools
   preserved.

The masthead GIF generator was not ported; the GIF stays as a committed asset.
