# Tranche — PR review candidates

Corpus: 12 observed open PRs; 12 matching judgments; 0 unjudged/stale; 0 unbound legacy judgments.
Review candidates: 0. Model-consistent groups: 4. Groups needing relationship review: 0. Security-priority items: 0 (meta-category, reviewed first).

Evidence: titles and shortened descriptions (1200 characters per PR; 400 per pair). Diffstat is unknown unless captured input supplies it. Patches, CI, reproductions, fix coverage and security have not been verified. Model scores are suggestions, not calibrated guarantees or approval to merge/close. Pagination records an observation, not a point-in-time GitHub snapshot.

## Model-consistent candidate groups — verify fix coverage; no survivor selected

- #12074, #12090, #12680
- #12649, #12755, #13009
- #13165, #13477, #13645
- #13459, #13463, #13613

## Escalate for risk/security review

- #12090 Install matching kernel headers before broadcom-wl-dkms

# Suggested pre-release batches (issue #4)

A batch is **5 PRs merged together as one tranche**. Jev determines
the composition: model-consistent same_change groups are atomic — their PRs combine into
ONE pull request inside the batch. Batches are disjoint (every PR is in at most one
batch); review groups are excluded on purpose. Ordered security-first, then
risk band and evidenced head idle time. Model-suggested, never verified safe to merge.

Batches: 3 · security-first batches: 0 · same-change groups: 3 · review-group PRs excluded: 0.

| Batch | Size | Security | Avg risk | Groups | Members |
|---|---|---|---|---|---|
| B001 | 3 | — | 0.9 | 1 | #13459 #13463 #13613 |
| B002 | 3 | — | 1.0 | 1 | #13165 #13477 #13645 |
| B003 | 3 | — | 1.0 | 1 | #12649 #12755 #13009 |

## Reviewer agent prompts

Copy-paste a prompt into an agent to start a thorough, methodical review that
produces one unified proposal + merge plan for the batch.

### B001

```
You are reviewing Omarchy pre-release batch B001 (3 PRs to be merged together as one tranche).

Pull requests in this batch:
- #13459: Scale clock panel hero date with fontScale — https://github.com/omacom/omarchy/pull/13459
- #13463: Scale the clock panel hero text with fontScale — https://github.com/omacom/omarchy/pull/13463
- #13613: Scale clock panel hero date with fontScale and fit long months — https://github.com/omacom/omarchy/pull/13613

Work through the batch methodically:
1. Read every PR fully — description, diff, and review comments. For PRs Jev flagged as the same change, verify they truly overlap and identify the strongest implementation of each.
2. Map dependencies between the PRs (shared files, ordering constraints, conflicts) and check each PR's CI status.
3. Produce ONE unified proposal for the batch: what merges, in which order, what gets squashed or dropped, and why — as a single coherent plan, not per-PR verdicts.
4. Verify the plan: does the combined result still build and pass tests? Any PR that cannot be verified stays out — say so explicitly.
5. Deliver: (a) the unified proposal, (b) a step-by-step merge plan with exact commands, (c) risks with mitigations, (d) an explicit list of anything excluded and why.

Facts over plausibility: base every claim on the actual diffs and CI state, never on titles alone. You are proposing — the human decides.
```

### B002

```
You are reviewing Omarchy pre-release batch B002 (3 PRs to be merged together as one tranche).

Pull requests in this batch:
- #13165: Skip placeholder screens when building bars — https://github.com/omacom/omarchy/pull/13165
- #13477: Skip placeholder and FALLBACK screens when creating bars — https://github.com/omacom/omarchy/pull/13477
- #13645: Skip placeholder and FALLBACK screens when building bars — https://github.com/omacom/omarchy/pull/13645

Work through the batch methodically:
1. Read every PR fully — description, diff, and review comments. For PRs Jev flagged as the same change, verify they truly overlap and identify the strongest implementation of each.
2. Map dependencies between the PRs (shared files, ordering constraints, conflicts) and check each PR's CI status.
3. Produce ONE unified proposal for the batch: what merges, in which order, what gets squashed or dropped, and why — as a single coherent plan, not per-PR verdicts.
4. Verify the plan: does the combined result still build and pass tests? Any PR that cannot be verified stays out — say so explicitly.
5. Deliver: (a) the unified proposal, (b) a step-by-step merge plan with exact commands, (c) risks with mitigations, (d) an explicit list of anything excluded and why.

Facts over plausibility: base every claim on the actual diffs and CI state, never on titles alone. You are proposing — the human decides.
```

### B003

```
You are reviewing Omarchy pre-release batch B003 (3 PRs to be merged together as one tranche).

Pull requests in this batch:
- #12649: Keep UPower rate when sysfs power read fails with ENODEV — https://github.com/omacom/omarchy/pull/12649
- #12755: Keep UPower rate when sysfs power reads fail or are non-numeric — https://github.com/omacom/omarchy/pull/12755
- #13009: Guard battery sysfs rate against bogus EC readings — https://github.com/omacom/omarchy/pull/13009

Work through the batch methodically:
1. Read every PR fully — description, diff, and review comments. For PRs Jev flagged as the same change, verify they truly overlap and identify the strongest implementation of each.
2. Map dependencies between the PRs (shared files, ordering constraints, conflicts) and check each PR's CI status.
3. Produce ONE unified proposal for the batch: what merges, in which order, what gets squashed or dropped, and why — as a single coherent plan, not per-PR verdicts.
4. Verify the plan: does the combined result still build and pass tests? Any PR that cannot be verified stays out — say so explicitly.
5. Deliver: (a) the unified proposal, (b) a step-by-step merge plan with exact commands, (c) risks with mitigations, (d) an explicit list of anything excluded and why.

Facts over plausibility: base every claim on the actual diffs and CI state, never on titles alone. You are proposing — the human decides.
```

# Parked before batching (issue #8)

3 PRs are parked: drafts, PRs without finished form, PRs without a
current judgment, and same_change groups holding for a parked member. Park is a
**hold with a named unblock path, never a close** — re-entry is automatic when the
reason clears and the next refresh re-packs. No batch lists a parked PR.

Reasons: draft 1 · finished_form 1 · unjudged_or_stale 0 · same_change_hold 3.

| PR | Reasons | Unblocked by |
|---|---|---|
| #12074 | same_change_hold | Held with its same_change group: atomic units are never split, so the group re-enters together when every member clears its own park reason. |
| #12090 | same_change_hold | Held with its same_change group: atomic units are never split, so the group re-enters together when every member clears its own park reason. |
| #12680 | draft, finished_form, same_change_hold | Author marks the pull request ready for review; the next fetch recaptures it and the next batches run re-packs it. Author adds the missing description or QA evidence; judge --resume re-binds the judgment and the PR re-enters on the next refresh. Held with its same_change group: atomic units are never split, so the group re-enters together when every member clears its own park reason. |

Parked does not remove a security-flagged PR from the security meta-category;
it only removes it from merge batches. Full record: out/parked.json.
