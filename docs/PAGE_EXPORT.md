# Page exports

`tranche page` always renders `docs/index.html` and its private client payload at
`docs/data/workbench.json`. Standalone exports are opt-in:

```sh
tranche page --export-json
tranche page --export-xlsx
tranche page --export-json --export-xlsx
```

The options can be used independently or together. They add
`docs/data/report.json` and/or `docs/data/report.xlsx`; they do not change the
workbench payload. Default `page` and `refresh` runs leave existing standalone
exports untouched, so those files remain snapshots until you rerun the matching
export option. The renderer validates the corpus, judgments, duplicate
report, batches and park record before writing any output. A stale or mixed
report is refused for every format.

## JSON contract

`report.json` is UTF-8 JSON with one-space indentation and a final newline. Its
versioned schema is [`page-export.schema.json`](page-export.schema.json). The
`format` is `tranche.page-export`; `schema_version` is `1`. Consumers should
check both before reading the payload and ignore fields they do not use.

Top-level fields:

| Field | Meaning |
|---|---|
| `repository` | Repository the validated report describes. |
| `report_binding` | The 64-character binding checked by the renderer. |
| `pull_requests` | One row per captured PR, with source details, judgment values, review flags, activity, park reasons and batch memberships. |
| `categories` | Category keys and their display labels. |
| `groups` | The bound duplicate report: confirmed groups, review groups, uncertain pairs and its meaning. |
| `batches_available` | Whether validated batch data was present. |
| `batches` | Available batch summaries, including member PRs and review prompts; empty when unavailable. |
| `parked` | The validated park record, or `null` when unavailable. |

The JSON export is the stable standalone contract. `workbench.json` remains a
page implementation detail and is not covered by this schema.

## Excel workbook

`report.xlsx` contains filterable, frozen-header sheets:

- **Report** — format version, repository, report binding and availability.
- **PRs** — one row per PR, with scalar columns suitable for sorting and filtering; activity fields are separate columns.
- **Groups** — confirmed, review and uncertain grouping rows.
- **Batches** — one row per available batch, including membership and review prompt.
- **Parked** — one row per parked PR, including reasons and unblock path.

Nested lists in Excel are rendered as readable comma-separated values. The JSON
export retains their structured representation.
