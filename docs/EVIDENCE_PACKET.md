# Native evidence packet v1

This documents the export produced by Tranche's native
[`evidence.py`](../evidence.py), specifically `build_packet()` and
`packet_bytes()`. The implementation on master is authoritative for the native
wire format; the architecture is recorded in
[native-evidence-cli.md](decisions/native-evidence-cli.md) and commands in
[EVIDENCE_CLI.md](EVIDENCE_CLI.md).

The original portable-packet proposal for
[issue #9](https://github.com/blackopsrepl/Tranche/issues/9), following
[Vittorio's requested first contribution](https://github.com/blackopsrepl/Tranche/issues/9#issuecomment-5929341489),
contributed the integrity, exact-byte citation, explicit incompleteness and
resume conformance work. That work is preserved as historical test support,
not the native wire specification. In particular, its `coverage` and inline
`body_base64` records are **not** the native export's `components` and
content-addressed `bodies` map. Both use the same format/profile labels; those
labels alone do not establish compatibility between the two layouts.

Capture, inspection, resume and export are native Tranche operations. No
external reader or application is required. An optional reader can inspect an
export offline because it includes the response bytes needed to resolve its
citations. Workbench publication and MCP evidence retrieval remain follow-ups
on the same native service, not dependencies of the CLI workflow.

## Export envelope

The export is one UTF-8 JSON object. `build_packet()` emits these fields:

| Field | Native meaning |
| --- | --- |
| `format` | `tranche.evidence-packet/v1` |
| `profile` | `pr-review/v1` |
| `capture_id` | Random 128-bit lowercase hex identity assigned to a capture |
| `generation` | Digest computed by `generation_id()` as described below |
| `selection` | The frozen `Selection.as_json()` projection |
| `membership_digest` | SHA-256 object digest of the ordered PR-number list |
| `complete` | `is_complete()`: no truthy capture stop reason and every component status is `complete` |
| `capture` | Observation time and last-run request accounting, including the stop reason |
| `components` | Native per-member/component state, including endpoint groups |
| `sources` | Acquisition records referencing response bodies by digest |
| `bodies` | Map from body digest to `{sha256, bytes, base64}` |
| `citations` | Source bindings and exact byte ranges |
| `notes` | Native explanatory text about integrity, observation and completeness limits |
| `packet_digest` | Object digest of the packet before this field is added |

The packet is a projection, not a copy of the persistence manifest. Manifest-only
fields such as `created_at`, `updated_at`, `code_observation` and capture
`bytes_stored` are not exported by `build_packet()`. There is no top-level
`coverage` or `members`, and sources do not contain inline `body_base64`.
This describes producer output, not a separate imported-packet validator or a
promise that every unknown field is refused.

## Selection and identity

`Selection.as_json()` returns exactly `repository`, `report`, `batch` and
`members`:

- `repository` is the native repository identity object passed to `Selection`.
  Native selection records the base repository's numeric `id` and exact
  `full_name` from captured PR metadata. Its visibility is initially `unknown`;
  export checks live public scope separately, without rewriting this projection.
- `report` carries `report_binding` and `output_digests` for `clusters.json`,
  `dupes.json`, `batches.json` and `parked.json`. These are the existing native
  parsed-object digests, not file-byte checksums; an absent permitted park file
  is represented by a null digest.
- `batch` is a **projection**, not the entire native batch object. Its keys are
  exactly `id`, `ordinal`, `members`, `count` and `review_prompt`. Ordered
  membership and the exact reviewer prompt are retained; diagnostics and other
  native batch fields are not copied into this projection.
- `members` is ordered with the batch. Each entry contains exactly `number`,
  `source_digest`, `evidence_digest`, `base_sha`, `head_sha`, `base_repo_id`,
  `base_repo_name`, `head_repo_id`, `head_repo_name`, `head_fork` and `updated_at`.
  Base/head revisions are full lowercase 40-character Git SHA-1 identifiers.
  Native selection reads the raw captured PR item for repository and base
  identity, rather than inventing missing projection fields.

Selection uses the authoritative bound-report loader in
[`report_loader.py`](../report_loader.py), shared with the report consumers.
Opaque report digests associate evidence with a validated report; they neither
reconstruct that report offline nor authenticate the source bytes. A batch ID
such as `B001` is a display ordinal, never a sufficient evidence identity.

`generation_id(selection, capture_id)` computes the native object digest of:

```python
{
    "format": FORMAT,
    "profile": PROFILE,
    "capture_id": capture_id,
    "repository": selection.repository,
    "revision": selection.revision(),
    "membership": selection.membership_digest,
    "batch": selection.batch["id"],
    "report": selection.report,
}
```

`selection.revision()` maps each PR number **as a string** to exactly
`base_sha`, `head_sha`, `base_repo_id`, `base_repo_name`, `head_repo_id` and
`head_repo_name`. The formula is not a digest of the entire selection.
`updated_at`, the reviewer prompt and `head_fork` are not direct inputs to this
revision map or the generation formula; report association remains an input,
so report-digest changes can still change the generation.

A base/head or repository-identity move prevents reuse of that recorded code
observation. Thread-only updates refresh mutable observations without making
unchanged code bytes invalid. Report/prompt/batch changes can establish a new
association while reusing content-addressed code bodies; copied acquisition
records and citations are rebound to the new generation, leaving the donor
capture intact. Within a resumed generation, mutable refresh can make component
state incomplete again. Thus neither a universal nonregression promise nor
"every `updated_at` change requires a new generation" describes native state.
See `Capture._refresh_mutable()` and `carry_over_code()` in `evidence.py`.

## Sources, bodies and citations

Each exported source has exactly these keys:

| Fields | Meaning |
| --- | --- |
| `id` | Digest of `{generation, source}` using the acquisition source record with its own `id` omitted |
| `number`, `component`, `group`, `page` | Member, component, endpoint group and 1-based page within that group |
| `cursor`, `next_cursor` | Recorded continuation values; REST uses URLs and GraphQL uses cursors |
| `url`, `accept`, `media_type` | Acquisition URL, requested representation and recorded response media type |
| `captured_at`, `generation` | Acquisition timestamp and capture-generation binding |
| `body_sha256` | SHA-256 of the exact decoded response bytes |

Source IDs bind the acquisition record, not a body-bearing packet record. The
record includes its own `generation` as well as the outer generation used in
its ID calculation. Different sources can reference the same body digest.

`bodies[source.body_sha256]` contains:

- `sha256`: the same lowercase body digest as the map key;
- `bytes`: decoded byte length;
- `base64`: standard base64 encoding of the exact response bytes.

`build_packet()` reads and verifies each distinct stored body once. It does not
normalize line endings, trim text or reserialize captured JSON. Native runtime
storage is `out/evidence/bodies/<sha256>.bin`, shared across capture manifests;
these local paths are not exported. The packet needs no state store or network
for byte inspection.

Every citation has `id`, `number`, `component`, `source_id`, `source_sha256`,
`start_byte`, `end_byte`, `excerpt_sha256` and `kind`. The range is a nonempty,
zero-based, half-open **byte** interval in the decoded source body, not a
codepoint or line interval. Resolve `source_id` in `sources`, verify its body
binding, decode the corresponding `bodies` entry, and check the SHA-256 of the
slice against `excerpt_sha256`. Empty captured bodies do not require fabricated
citations. Native inspection also provides bounded source/citation retrieval.

URLs record provenance, not permission to execute or fetch source text. Treat
all captured content as untrusted: never execute patches or embedded
instructions, and escape it for display. Digests show integrity of recorded
bytes, not proof that GitHub served them or that evidence is still current.

## Component state and completeness

`pr-review/v1` has eight components per member: `metadata`, `diff`, `files`,
`discussion`, `review_comments`, `reviews`, `checks` and `closing_issues`.
Each component entry starts with `number`, `component`, `status`, `reason` and
`groups`. Each group starts with `group`, `status`, `pages`, `next_url`, ordered
`source_ids` and `reason`; acquisition can add count fields such as
`items_observed` and `items_reported`. `build_packet()` exports component state
as held in the manifest, not a fixed flattened coverage schema.

The REST components each have one same-named group. `closing_issues` has one
GraphQL group. `checks` has independently paginated `check_runs` and `statuses`
for the base repository, plus `fork_check_runs` and `fork_statuses` only when
the head repository differs from the base. Fork CI is a separate observation,
not a substitute for the PR's base-repository CI. Real endpoints are documented
in the [architecture note](decisions/native-evidence-cli.md); the proposal's
synthetic `/pulls/N/metadata`, `/discussion` and `/checks` URLs are not GitHub
REST endpoints.

Statuses are `missing`, `partial`, `complete` or `blocked`. Fresh groups have
zero pages, no continuation, no sources, and reason `not acquired`.
`roll_up()` uses this precedence: any blocked group makes the component blocked;
otherwise any missing group makes it missing; otherwise all complete groups
make it complete; otherwise it is partial. Complete components have null
reason; other reasons aggregate group problems.

A terminal continuation alone does not establish content coverage. Acquisition
records captured empty responses, reported/observed count gaps, page caps,
truncation and failures rather than silently treating unknowns as empty.
`is_complete()` checks component statuses and the capture stop reason; it does
not independently revalidate all pagination/content claims when serializing.
Completeness is not a review, a test performed by Tranche, current CI, source
authenticity, duplicate equivalence or approval to merge.

## Accounting, serialization and sharing

The exported `capture` keys are exactly `observed_at`, `request_limit`,
`requests_used`, `reserved`, `failures`, `retries`, `identity_checks` and
`stop_reason`. Accounting describes the acquisition run, not the number of
retained sources or a lifetime request total. Reserved completion-check capacity
can cause a request-budget stop before the full limit has been spent. Timestamps
produced by native `now()` are UTC seconds (`YYYY-MM-DDTHH:MM:SSZ`).

Object digests use `tranche.digest()`: sorted JSON keys, separators `(',', ':')`,
`ensure_ascii=False`, `allow_nan=False`, then UTF-8. Arrays retain order. This is
Tranche's encoding, not a claim of RFC 8785 canonicalization. Body and excerpt
digests hash raw bytes instead of object serialization.

`packet_bytes()` uses that JSON encoding without a trailing newline, and checks
the exact serialized size against its export limit (default 64 MiB), including
base64 expansion and metadata. The proposal harness's 1 MiB ceiling is not this
native export default or the MCP delivery contract. Native strict JSON loading
and storage/transport protections are implementation concerns, not evidence
that the historical harness validates native packets.

The CLI export path validates association, runs a fresh public-scope sharing
check for the base and linked repositories, and checks report inputs for
concurrent changes before publishing. Historical export is explicit and never
claims current-batch compatibility. `build_packet()` and `packet_bytes()` alone
do not perform that live gate. CLI capture is read-only toward GitHub and
model-free; a prior public packet is not ongoing sharing permission.

## Verification and preserved proposal cases

Native implementation tests are in
[`tests/test_evidence.py`](../tests/test_evidence.py) and transport tests in
[`tests/test_ghread.py`](../tests/test_ghread.py):

```bash
python3 -m unittest tests.test_evidence tests.test_ghread -v
```

The original proposal's
[`tests/evidence_contract.py`](../tests/evidence_contract.py) and
[`tests/test_evidence_packet.py`](../tests/test_evidence_packet.py) remain
valuable **historical synthetic conformance support**:

- [partial.json](../tests/fixtures/evidence/partial.json),
  [resumed.json](../tests/fixtures/evidence/resumed.json) and
  [revision-drift.json](../tests/fixtures/evidence/revision-drift.json) preserve
  the proposed flattened coverage/inline-body layout.
- They use invented repository/PR identities and source bytes, with synthetic
  native report/batch inputs. They are not captured GitHub evidence or examples
  emitted by `build_packet()`.
- They exercise exact-byte integrity, multibyte slices, corrupt/dangling records,
  partial/resumed pagination, budgets and synthetic identity gates under that
  proposal. Passing them does not demonstrate native wire compatibility or live
  acquisition behaviour.

Run those preserved cases with
`python3 -m unittest tests.test_evidence_packet -v`. Keeping them credits and
retains the proposal work without requiring native state or exports to conform
to its historical layout. Any native wire-format claim must instead be checked
against the native producer and its tests.
