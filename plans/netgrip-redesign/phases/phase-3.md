---
type: planning
entity: phase
plan: "NetGrip visual redesign"
phase: 3
status: in_progress
created: "2026-11-22"
updated: "2026-11-22"
---

# Phase 3: Local finish and publication

> Part of [NetGrip visual redesign](../plan.md)

## Objective

Finish the redesign on this PC through visual review, complete gates, accurate documentation and a verified PR after publication is authorized.

## Scope

### Includes

- Final simulator capture round and independent visual review.
- Host, hostsim, release-feature and replay verification on the committed tree.
- Documentation, CI evidence and PR preparation.
- Explicit reporting that no television validation was performed.

### Excludes

- Television deployment, release cutting and merge without user authorization.

## Prerequisites

- [x] Phases 1 and 2 are complete and all surfaces are integrated.

## Deliverables

- [x] Reviewed final captures and resolution of material findings.
- [ ] Passing local gates and clean committed tree.
- [ ] Updated `docs/DESIGN.md`, plan evidence and PR verification report.
- [ ] Green ARM and Simulator CI on the authorized PR.

## Acceptance Criteria

- [ ] Every Definition of Done item has direct evidence, with any unmet item named rather than hidden.
- [x] The final artifact runs on this PC and all reviewed captures match the implemented surfaces.
- [ ] No commit, push, merge or release occurs without the required user authorization.
- [ ] The PR states the exact verification scope and excludes any television claim.
- [ ] The PR does not close #444. Final TV validation remains tracked by the issue for a separately authorized iteration.

## Dependencies on Other Phases

| Phase | Relationship | Notes |
|-------|-------------|-------|
| 1 and 2 | blocked-by | Final verification requires the complete integrated redesign. |
