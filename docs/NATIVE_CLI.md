# Native Tranche CLI

The `tranche` binary exposes the Tranche command tree and is the pipeline
implementation. The commands that run today are marked below.

## Install

From a Tranche checkout, with a current Rust toolchain:

```sh
cargo install --path crates/tranche-cli --locked
tranche --help
tranche --root /absolute/path/to/Tranche cluster
```

The binary is self-contained: it needs nothing but its checkout's data and
outputs at runtime. `--root` selects the checkout containing the captured corpus
and the derived reports; the default is the current working directory, not a
baked-in build path.

## Commands

Every command below is implemented. The MCP server is a read-only front-end over
the same bound report and validation gates, not a second pipeline.

```sh
tranche fetch [--transport gh|urllib|curl]
tranche judge [--limit N] [--resume]
tranche dupes [--max-pairs N]
tranche cluster [--allow-unbound]
tranche batches
tranche refresh [--max-pairs N] [--no-page] [--dry-run]
tranche all [--limit N] [--resume] [--max-pairs N]
tranche page
tranche --root /absolute/path/to/Tranche mcp
tranche evidence capture --batch B001 [--request-budget N] [--max-bytes N]
                                 [--fresh] [--reuse-capture ID] [--break-lock]
tranche evidence show [--batch B001 | --capture ID] [--source ID] [--citation ID]
                      [--start-byte N] [--length N]
tranche evidence export [--batch B001 | --capture ID] --output FILE
                        [--allow-historical] [--request-budget N] [--max-bytes N]
```

Use `tranche COMMAND --help` for its flags. The evidence service retains its
native capture, resume, selection, sharing gate and exit-code contracts; see
[the evidence guide](EVIDENCE_CLI.md). Repeat capture to resume, or explicitly
select a generation with `--reuse-capture ID`. Stored evidence stays local under
the checkout's ignored `out/evidence/` directory.

`judge`, `dupes`, `refresh`, and `all` can make paid model calls; only explicit
commands start them. Captured CI remains an observation, not tests executed by
Tranche.

## Automation

`--json` emits machine-readable output in the stored format. Diagnostics go to
stderr, so stdout stays parseable.

```sh
tranche --root /absolute/path/to/Tranche cluster --json
tranche evidence show --capture ID --json
tranche evidence export --capture ID --output - > packet.json
```

Export to `--output -` writes the packet to stdout. The native public-sharing
gate still applies.

## Verify

```sh
make check         # format, strict clippy, tests and whitespace
make cli-install   # build and install the binary from this checkout
```

Rust integration fixtures use disposable relocated checkouts and synthetic
transport responses. They verify service calls and packet parity without
acquiring contributor source trees or calling paid models.

## Attribution

Author: Vittorio Distefano ([blackopsrepl](https://github.com/blackopsrepl)).
Builds on Christopher's initial CLI POC
([@GreyforgeLabs](https://github.com/GreyforgeLabs)), with the Rust frontend and
native integration developed and maintained in Tranche.
