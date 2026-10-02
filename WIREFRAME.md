# WIREFRAME — Tranche's surfaces

The structural description of everything this project renders or serves, and the
contract each piece must keep. It is deliberately about layout, identity and
provenance rather than pixels: styling lives in `docs/assets/workbench.css`, and
the tests in `tests/workbench.test.cjs` assert the behaviour named here.

Tranche has three surfaces. They share one bound observation and must never
disagree with each other:

| Surface | File | Audience |
| --- | --- | --- |
| Workbench | `docs/index.html` + `docs/data/workbench.json` | a human triaging the backlog |
| Evidence CLI | `tranche.py evidence …` (`evidence.py`) | a reviewer, and the workbench/MCP later |
| MCP server | `mcp_server.py` | an AI agent asking bounded questions |

## 1. Workbench

Static, inert HTML. `gen_page.py` renders one shell and one data payload; the
page fetches the payload at boot and does all filtering client-side. Nothing on
the page acquires evidence, holds a credential or executes captured text.

```
┌────────────────────────────────────────────────────────────────────┐
│ masthead                                                           │
│   OMARCHY — TRIAGE with Tranche (powered by Jev)      [keeper owl] │
│   PULL REQUEST WORKBENCH · omacom/omarchy ↗ · 2,817 captured PRs    │
├────────────┬───────────────────────────────────────────────────────┤
│ sidebar    │ Pull requests              FIND · INSPECT · REVIEW     │
│ CATEGORIES │ AI-assisted priorities; review code and tests before   │
│  PRs       │ merging.                                               │
│  ────────  │ ┌ queues ────────────────────────────────────────────┐ │
│  Security  │ │ Security first · All · Review candidates ·         │ │
│  Install   │ │ Senior review · Author follow-up · Parked ·        │ │
│  …         │ │ Related PRs · Head revised · Batches               │ │
│  Unknown   │ └────────────────────────────────────────────────────┘ │
│            │ (Batches view) Suggested merge batches                 │
│            │   one card per batch: id, size, security, avg risk,    │
│            │   members, [Copy agent prompt]                         │
│  ────────  │ ┌ toolbar ───────────────────────────────────────────┐ │
│ Review     │ │ [search title · #number · @author · description /] │ │
│ deliberately│ │ Sort [newest ▾]                                   │ │
│ Built for  │ └────────────────────────────────────────────────────┘ │
│ the team.  │ N results · [Reset filters]                            │
│ Tranche ↗  │ PR / TITLE                            MODEL RISK       │
│            │ ▸ #NNNN  title · @author · category · batch chip       │
│            │ ▸ #NNNN  …                                             │
│            │ [← Previous]  page 1 of 57  [Next →]                   │
│            │ Tranche · 2,817 captured PRs · Open GitHub ↗            │
└────────────┴───────────────────────────────────────────────────────┘

PR detail (modal <dialog>, Esc / Close restores focus to the row)
┌────────────────────────────────────────────────────────────────────┐
│ #NNNN                                            [Close  Esc]      │
│ title · author · created                                            │
│ model: risk · finished · effort · fix · security · freshness        │
│ activity: head revised? · idle since (basis: judgment/creation)     │
│ diffstat · labels · batch membership · parked reasons + unblock     │
│ related PRs (discovery suggestions, never verified duplicates)      │
└────────────────────────────────────────────────────────────────────┘
```

### Required behaviour

- **Filters compose.** Queue, category, risk band, search terms, sort and page are
  independent; the total in `#result-count` counts every match, not the page.
- **Search is typo-tolerant** over title, body, `#number` and `@author`.
- **URL state round-trips** search, queue, category, sort, page and the selected PR.
- **Queues share one predicate with their counter.** A row is in a queue exactly
  when the nav count says so — in particular an empty `parked` array is not a
  parked PR.
- **Untrusted text is rendered through `textContent`**, never `innerHTML`; the
  embedded JSON escapes `<`, `>`, `&` and the Unicode line separators.
- **Unknowns stay unknown.** A missing diffstat, risk or judgment is displayed as
  unknown, never as zero.
- **The payload is data, not prose.** `docs/data/workbench.json` carries
  `prs`, `categories`, `groups`, `batches`, `batches_available` and `parked`.
  Assertions about corpus numbers read the payload; assertions about behaviour
  read the live DOM. Never mix the two.
- **Empty and failure states exist**: no matches, JavaScript disabled, payload
  load failure.
- **Reduced motion** swaps the animated masthead for the static PNGs.

## 2. Evidence CLI

A capture is **not** a workbench view. Its output is a coverage report plus, on
request, a byte window or a resolved citation. The shape:

```
$ python3 tranche.py evidence capture --batch B001 --request-budget 100
capture <32 hex>  batch B001  generation <sha>…  INCOMPLETE | COMPLETE
  observed <Z>  stop request_budget|none  requests 37/100 (failures 0, …)
  281655 bytes stored, 40 citations
  #11720  head <sha12>…  base <sha12>…
      metadata         complete    1p    0c  [metadata=complete]
      diff             complete    1p    0c  [diff=complete]
      files            complete    1p    4c  [files=complete]
      discussion       missing     0p    0c  [discussion=missing] — not acquired
      checks           complete    4p    0c  [check_runs=complete, statuses=complete,
                                              fork_check_runs=complete, fork_statuses=complete]
      …
  resume: python3 tranche.py evidence capture --batch B001 --reuse-capture <id>
```

### Required behaviour

- **One line per capture**, then per member, then per component, then per endpoint
  group. A group that is not `complete` always carries a reason.
- **Partial is a first-class, usable outcome.** The resume line is printed for any
  incomplete capture, and the exit code says which kind of outcome it was
  (`4` incomplete, `3` refused, `0` success).
- **`--json` is bounded** and carries the same facts; it never prints a whole
  packet into a terminal.
- **`show --batch` is the current association only** and explains an absent or
  stale one instead of showing an older capture of the same ordinal.
- **`show --capture` is historical and offline**: it works with no current report.
- **`show --source` prints a bounded window** with `start..end`, a `next` hint and
  control characters escaped; `--citation` resolves one citation against stored
  bytes. Neither prints the packet by default.
- **`export` writes a self-contained packet**, atomically, after a fresh public
  gate; a historical export is explicit and claims no current compatibility.
- **No command** calls a model, mutates GitHub, opens a browser or executes
  captured content.

## 3. MCP server

Read-only, model-free, stdio JSON-RPC. Every response is bounded by
`MAX_RESULT_BYTES` (1 MiB) and carries the report identity digests it was served
from. Tools today: `surface`, `query`, `pick`, `next_prompt`, `related`,
`digests`.

### Required behaviour

- **Refuse stale, foreign, unbound or modified reports** before answering, using
  the same loader the CLI uses.
- **Bounded schemas**: advertised `inputSchema` with enums, bounds and patterns;
  arguments are validated against it, not by Python signature errors.
- **Never serve historical stale cache rows** as current; the producer's
  projection is the only one published.
- **Prompt picking stays stateless and read-only**.
- Evidence read/retrieve tools are **not** implemented yet; when they are, they
  must reuse this evidence service and use progressive retrieval rather than
  inlining a packet.

## 4. Publication

`make page` renders the workbench into `docs/`. Publication is the only step that
touches the served site, and only these paths are served:

```
docs/index.html            shell
docs/data/workbench.json   inert payload
docs/assets/*              css, js, images
docs/decisions/*.md        decision records (source, not served as a page)
```

Captured evidence never enters `docs/`. It lives in the ignored runtime root
`out/evidence/` and leaves the machine only through an explicit export. No
harness credential, session or scratch file is committed at any point.
