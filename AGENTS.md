# Working rules for this repository

These are the conventions every change here follows. They are short on purpose:
each one exists because breaking it caused a real defect, and the reason is stated
so a later reader can tell whether the rule still applies.

## 1. No source file reaches 500 lines

Split by responsibility into sibling modules. A module that needs three concepts is
three modules. This applies to every crate's `src/`, not to tests. Preserve the
existing module layout; use the 500-line limit for future additions and changes.

The limit is not cosmetic. A file that has grown past it has almost always taken on
a second job, and the two jobs then have to be read together to understand either.

## 2. `mod.rs` and `lib.rs` hold no code

They declare modules and re-export names, and that is all. No functions, no
structs, no `impl` blocks, no `const` definitions, no helper types — not even a
small one.

A module root is a table of contents. Code placed there is code nobody looks for:
it is invisible to a reader scanning for where a behaviour lives, and it accretes
because the file always looks empty. Behaviour belongs in a named sibling module;
`mod.rs` points at it.

```
src/
  evidence/
    mod.rs          declarations and re-exports only
    vocabulary.rs   the constants
    errors.rs       the error types
    coverage.rs     the behaviour
```

## 3. Tests live only in `tests/`

Never a `#[cfg(test)]` module inside a source file. Integration tests under
`tests/` are named for the outcome they protect rather than the module they
exercise: a test named after a module drifts into restating the implementation,
while one named after an outcome fails when the behaviour breaks.

## 4. `gh` stays a subprocess

The GitHub credential stays inside `gh`. Do not reimplement GitHub auth, and do not
read the token in this process or place it in an argv.

## 5. The on-disk format is the contract

Everything committed under `out/` and `docs/` is an interface every reader depends
on. A change to the pipeline is judged by whether it reproduces those bytes, not by
whether it looks correct. Four properties look like accidents and are not:

- **Insertion order, not sorted.** Category order and `tranches.md` sections follow
  first appearance. The digests cannot catch a mistake here, because they sort keys
  explicitly.
- **Floats keep the shortest round-tripping form.** `0.6` stays `0.6`.
- **Stored JSON separators carry a space.** A compact serializer restyles every
  artifact.
- **The workbench payload is flattened to ASCII** and escapes `&`, `<` and `>` — it
  is fetched and injected, and a PR title is untrusted text from GitHub.

## 6. Never fabricate a result

A claim about live state comes from a fresh read of that state. A test assertion is
backed by a real run. When a step cannot be completed, name the blocker precisely
rather than producing output that looks like success.
