---
type: planning
entity: todo
plan: "NetGrip visual redesign"
updated: "2026-11-22"
---

# Todo: NetGrip visual redesign

> Tracking [NetGrip visual redesign](plan.md)

## Active Phase: 3 - Local finish and publication

### Phase Context

- **Scope**: [Phase 3](phases/phase-3.md)
- **Implementation**: Phase 2 plans and the final gates in `plan.md`
- **Latest Handover**: None
- **Relevant Docs**: `PRODUCT.md`, `docs/DESIGN.md`, `rust-modules/src/ui/CLAUDE.md`, `rust-modules/src/screens/CLAUDE.md`, `rust-modules/src/player/CLAUDE.md`

### Pending

- [ ] Commit, push, open the PR and run CI only after explicit user authorization <!-- added: 2026-11-22 -->

### In Progress

- [ ] Present the local finish and request a publication decision <!-- started: 2026-11-22 -->

### Completed

- [x] Confirm product truth, scope, NetGrip direction, three phases and local PC validation <!-- completed: 2026-11-22 -->
- [x] Implement and review Home, Detail, Library, Settings and Player at 1920x1080 <!-- completed: 2026-11-22 -->
- [x] Add and pass focused visual-state and compact-geometry regressions <!-- completed: 2026-11-22 -->
- [x] Run Phase 1 full suites and Phase 2 targeted suites plus release-feature checks <!-- completed: 2026-11-22 -->
- [x] Capture and review Library opening/grid/sort, Settings root/picker and paused Player <!-- completed: 2026-11-22 -->
- [x] Obtain an independent `ship` visual verdict and finish `docs/DESIGN.md` <!-- completed: 2026-11-22 -->
- [x] Pass complete host, hostsim, storage, lint, release-feature and replay verification on the local tree <!-- completed: 2026-11-22 -->

### Blocked

- [ ] Final clean-tree gate, CI and publication require the user's explicit authorization, not needed for local implementation

## Changelog

### 2026-11-22

- Plan and Phase 1 tracking created.
- Phase 1 completed with local visual and host evidence; Phase 2 started.
- The full clean-tree `make check` remains a publication gate because the current work is intentionally uncommitted.
- Phase 2 completed with reviewed local captures and targeted tests. Phase 3 started for independent review, docs, full verification and authorized publication.
- #446 and #447 were filed for the stale shelf capture and the modal Back/wordmark overlap respectively.
- Independent visual review returned `ship`. Full local suites, lint, release check and six SAME replays are green. Publication now awaits the user's explicit decision.
