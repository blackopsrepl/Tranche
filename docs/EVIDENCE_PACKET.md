# Proposed portable evidence packet v1

Draft for [issue #9](https://github.com/blackopsrepl/Tranche/issues/9), following
[Vittorio's requested first contribution](https://github.com/blackopsrepl/Tranche/issues/9#issuecomment-5929341489).
This proposes the packet format for the native Tranche evidence workflow being
built in this repository. Tranche will let users select a batch, capture its
source material, inspect citations, resume a partial capture and export the
packet. The capture implementation, native state and packet format will be
maintained here and shipped as Tranche, with the CLI, workbench and read-only MCP
sharing the same evidence.

Users will be able to complete that workflow in Tranche without another
application. External tools may optionally consume exported
packets.

This PR covers the proposed contract and offline conformance cases, and - since a
contract with no implementation behind it cannot be accepted - the native
implementation it describes is implemented in this same PR. The CLI, its native
state and the packet format are maintained in this repository and shipped as
Tranche; the workbench and the read-only MCP server are intended to read the same
evidence rather than acquiring or reinterpreting it.

## Envelope and identity

The packet is one UTF-8 JSON object with the following required fields. Inspect
the [partial example](../tests/fixtures/evidence/partial.json) and
[resumed example](../tests/fixtures/evidence/resumed.json) for complete records.
Unexpected fields are refused in v1, except inside the copied native `batch`.

| Field | Meaning |
| --- | --- |
| `format` | `tranche.evidence-packet/v1` |
| `profile` | `pr-review/v1`, requiring all eight components below for every member |
| `selection` | Immutable repository, native report, selected batch and ordered member revisions |
| `membership_digest` | SHA-256 of the ordered list of PR numbers |
| `capture_id` | Random 128-bit lowercase hex identity assigned once when starting a capture |
| `generation` | SHA-256 of `{format, profile, selection, capture_id}` |
| `complete` | True exactly when every member/component has complete pagination |
| `capture` | Observation time, per-run request limit/usage, explicit stop reason |
| `coverage` | Exactly one record per member/component; missing evidence remains visible |
| `sources` | Captured source bytes with identity, URL, timestamp and pagination |
| `citations` | Exact source and byte-range bindings |
| `packet_digest` | SHA-256 of the whole packet with this field omitted |

`selection.repository` contains a positive numeric GitHub repository `id`, exact
`full_name` and `visibility: public`. Both ID and name participate in identity;
a rename, transfer or visibility change is a gate, not a silent URL rewrite.

`selection.report` contains the native `report_binding` and `output_digests`
for `clusters.json`, `dupes.json`, `batches.json`, and `parked.json` (null only
when the native park file is absent and permitted by the existing loader).
These are the **existing parsed-object digests**, not file-byte checksums.
`selection.batch` is the whole selected native batch, unchanged: ID, ordinal,
ordered membership, count, diagnostics and exact `review_prompt`. A batch ID
such as B001 is a display ordinal, never a cache identity on its own.

Resume looks up the existing active capture for that selection and retains its
`capture_id`; it does not mint one per run. An explicitly fresh observation can
start a new capture even if revisions are unchanged (for example, CI results
changed). It gets a new ID/generation and does not overwrite frozen old sources.

`selection.members` is in the batch's exact order. Each entry contains `number`,
the native `source_digest` and `evidence_digest`, full 40-character `base_sha`
and `head_sha`, and `updated_at`. The native source/evidence digests retain
their existing distinct meanings; source bytes below have their own digest.
The report's projection does not currently supply a full base revision: future
capture must resolve it from PR metadata, verify head/update identity against
the selected report, and bind the resolved base here. Unknown or short revisions
cannot be substituted. Changed base, head or update time requires a new selection
and generation, even if the ordinal and membership are unchanged.

An exporter must first pass the existing report gates: current snapshot,
judgment/pair bindings, summary output digests, exact recomputed batches and
park state. `mcp_server.Reports.pick()` already applies those gates and returns
the same batch/prompt as the workbench. Copy its selected batch and digests;
resolve repository ID and revisions through the explicitly selected local read
transport. Validate inputs before and after export to refuse concurrent changes.
The executable spec accepts `expected_selection` from this validated context;
without it, successful validation proves **internal consistency only**.

The packet need not contain the whole discovery corpus or model cache. Opaque
native report digests are anchors to an independently validated report, not
enough to reconstruct or authenticate it offline. Source evidence for the
selected batch is carried in full. Any future provider/report schema work is
separate from this proposal.

## Digest and source-byte rules

All named digests are lowercase SHA-256 hex. Object digests use the existing
Tranche encoding: JSON sorted keys, separators `(',', ':')`, `ensure_ascii=False`,
`allow_nan=False`, then UTF-8 without BOM or trailing newline. Arrays retain
order; strings retain their Unicode form. Duplicate keys, NaN/Infinity and
invalid UTF-8 are rejected. Structural integer fields refuse JSON booleans.
This is Tranche's encoding, not a claim of general RFC 8785 canonicalization;
other languages must reproduce the provided vectors, including native batch
numbers. Git revisions are lowercase 40-character hex, not SHA-256 digests.

Every `sources` record contains:

| Field | Meaning |
| --- | --- |
| `number`, `component`, `page` | Member PR, one required component, contiguous 1-based page |
| `cursor`, `next_cursor` | Opaque continuation labels; first cursor and terminal next cursor are null |
| `url` | Public source URL, useful for provenance; inspection does not require fetching it |
| `media_type` | `application/json`, `text/plain`, or `text/x-diff` |
| `captured_at` | UTC acquisition timestamp |
| `body_base64` | Canonical base64 of exact response-body bytes after HTTP content decoding |
| `body_sha256` | SHA-256 of those decoded bytes, not JSON reserialization |
| `id` | SHA-256 of `{generation, source}` with the source's own `id` omitted |

The conformance profile uses `https://api.github.com/repos/OWNER/REPO/` REST
source URLs or `https://api.github.com/graphql` for closing-issue queries.
GraphQL URLs alone cannot identify a PR; the recorded number/component and
actual returned repository/PR identity must agree at acquisition time. Other
source namespaces need an explicit profile extension with checked scope.
The synthetic bodies are deliberately opaque text, including Unicode and hostile
instructions. Source/page/revision association is a recorded acquisition claim;
hashes do not prove GitHub served it. Future capture must check actual returned
metadata/revisions and response pagination, rather than trusting declarations.

There are no external artifact paths, executable commands, authorization headers,
credentials or transport configuration fields. URLs must not carry credentials.
Do not normalize line endings, trim text, reserialize JSON or silently redact
captured evidence while retaining its old digest. If source content cannot be
exported safely, leave that component explicitly blocked and explain why.
Never execute source text or patches. Browser rendering must escape it.

Each citation has a unique `id`, `source_id`, `source_sha256`, `start_byte`,
`end_byte` and `excerpt_sha256`. Ranges are nonempty, zero-based, half-open
**byte** offsets into decoded source bytes. The excerpt digest covers exactly
that slice; there is no codepoint/line-number ambiguity. Rebind neither the
source nor the citation to a newer revision. An offline reader can decode the
source and inspect the slice without Tranche installed or GitHub access.

## Coverage, budgets, interruption and resume

`pr-review/v1` requires `metadata`, `diff`, `files`, `discussion`,
`review_comments`, `reviews`, `checks`, and `closing_issues` for each member.
Coverage records contain `number`, `component`, `status`, ordered `source_ids`,
`next_cursor` and `reason`. Status is one of:

- `missing`: no captured page, null cursor, nonempty reason.
- `partial`: captured prefix with a non-null continuation and nonempty reason.
- `complete`: at least one page, terminal null cursor, null reason. An empty
  collection needs a captured empty response, not an invented absence.
- `blocked`: nonempty reason; previously captured pages may be retained.

Pages must belong to that member/component/generation, start at page 1, have
contiguous page numbers and linked nonrepeating cursors. No orphan or duplicate
sources are allowed. The continuation on the last source must agree with the
coverage cursor. Declared `complete` does not mean full review, test execution,
source authenticity, current CI, duplicate equivalence or merge approval.

`capture` contains UTC `observed_at`, `request_limit`, `requests_used` and
`stop_reason`: null, `request_budget`, `storage_budget`, `interrupted`,
`transport_error`, `visibility_revoked`, or `revision_drift`. Every attempted
network request counts, including failures and identity/pagination checks;
budget exhaustion has used == limit. Counts refer to the last run, not all
runs or the number of retained sources. Partial observations may have a null
stop reason when acquisition simply has not run yet. Complete packets cannot
have a stop reason. Timestamps use `YYYY-MM-DDTHH:MM:SSZ` in this profile.

Persist progress atomically before stopping. Resume against the same immutable
selection/profile preserves the generation and all previous source/citation
records, adds pages, never regresses complete coverage, and produces a different
packet digest when progress changes. Repeating completed work may reuse the
same bytes without requests; it does not certify live freshness. If GitHub
revisions drift or visibility is revoked during acquisition, the stopped
generation cannot resume. Retain it for inspection
and start a fresh one after regenerating/validating the report as needed. Never
mix old source IDs into the new generation. The
[revision-drift fixture](../tests/fixtures/evidence/revision-drift.json) shows
the same B001 with a changed head/update time and a distinct generation.

Consumers bound the **whole serialized packet**, including base64 expansion,
citations and metadata, before JSON parsing, and bound any resulting record
before persistence. The conformance harness uses a deliberately small 1 MiB
ceiling and tests exact-size acceptance and one-byte overflow; it is not a CLI
storage-default decision. Native acquisition will additionally need bounded
streaming, response/page counts and storage accounting. The wire format avoids
artifact paths entirely. Future filesystem persistence must separately confine
paths beneath its selected root, reject unsafe links and support crash recovery.

Sharing additionally requires a fresh local read confirming the same public
repository ID/name. Unknown visibility, revocation or drift stops sharing; do
not infer permission from a previously public packet. A synthetic sharing check
is supplied, not live policy enforcement. CLI capture remains read-only and
model-free. The workbench only displays deliberately published packets and never
acquires evidence with browser credentials; MCP should use bounded read/retrieve
operations over these same bytes. Existing MCP response limits still apply:
large packets need progressive retrieval, not an unbounded inline tool result.

## Executable cases and remaining decisions

Run `python3 -m unittest tests.test_evidence_packet -v` or the existing
`make check`. [evidence_contract.py](../tests/evidence_contract.py) is test support
for the proposed contract, not the native implementation. Its checks cover:

- Partial capture with a paginated files prefix, preserved resume, revision
  drift, nonregressing pages and immutable prior citations.
- Exact decoded bytes, multibyte citation slices, corrupt/dangling/duplicate
  sources and citations, incomplete pagination and misleading completeness.
- Native synthetic report/batch generation and MCP selection, exact prompts,
  stale output/report bindings, reordered members and foreign repository identity.
- Whole-record/request limits, malformed JSON, unexpected artifact fields,
  public scope and a synthetic visibility/identity sharing gate.

All three JSON fixtures use **invented repository/PR identities and source
bytes**; no captured project data, private datasets, provider patch or external
package is included. Their report/batch fields were generated by today's native
producer with synthetic inputs. Tests also generate fresh native bindings rather
than treating the fixture hashes as production trust anchors.

Settled by the implementation now in this PR, and recorded here rather than asked
as an open question: `pr-review/v1` is the profile this repository builds against;
CI is captured as **two separated collections** (check-runs and commit statuses,
read from both the base repository and a linked fork, since a PR's own checks live
in the base repository and a check-runs response cannot show the fork's); the
packet's byte offsets are byte-based and its timestamps are UTC seconds, so no
subsecond precision is added. Native persistence, the finite budgets and the
bounded retrieval path are implemented, not deferred: see
[native-evidence-cli.md](decisions/native-evidence-cli.md) for the design and
[EVIDENCE_CLI.md](EVIDENCE_CLI.md) for the implemented behaviour.

Still open, and explicitly not claimed here: publication of captured evidence to
the workbench or through MCP retrieval, which remains follow-up work on the same
native service. The acceptance work this proposal listed as future - live capture,
concurrent-file safety, full path-confinement tests, transport/pagination
verification, revision rechecks and a real read-only GitHub trial - is covered by
the implementation's own tests and its live trial; the offline conformance cases
here still claim nothing about live behaviour themselves.
