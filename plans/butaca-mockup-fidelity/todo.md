---
type: planning
entity: todo
plan: "butaca-mockup-fidelity"
updated: "2026-11-23"
---

# Todo: butaca-mockup-fidelity

> Tracking [butaca-mockup-fidelity](plan.md)

## Active Phase: 1 - Shared chrome and Settings

### Phase Context

- **Scope**: [Phase 1](phases/phase-1.md)
- **Implementation**: [Phase 1 implementation plan](implementation/phase-1-impl.md)
- **Latest Handover**: None
- **Relevant Docs**: `rust-modules/src/ui/CLAUDE.md`, `rust-modules/src/screens/CLAUDE.md`, `docs/DESIGN.md`

### Pending

- [x] Add failing regressions for textual navigation, real profile access, shared chrome and Settings geometry <!-- completed: 2026-11-23 -->
- [x] Implement shared chrome, typography, real profile/server dialog and anchored Settings pickers <!-- completed: 2026-11-23 -->
- [x] Review all six 1920x1080 comparison captures <!-- completed: 2026-11-23 -->
- [x] Run the final committed-tree gates and publish the Phase 1 PR <!-- completed: 2026-11-23 -->


### In Progress

- [ ] Add failing regressions for textual navigation, real profile access, shared chrome and Settings geometry <!-- started: 2026-11-23 -->

### Completed

- [x] Audit all mockup surfaces and map material mismatches to current source <!-- completed: 2026-11-23 -->
- [x] Confirm the four-phase scope including real audio/subtitle preferences <!-- completed: 2026-11-23 -->
- [x] Author and verify the Phase 1 implementation plan against current source <!-- completed: 2026-11-23 -->
- [x] Commit Phase 1 implementation, replay migration and deterministic capture fixes <!-- completed: 2026-11-23 -->

### Blocked

- None

## Changelog

### 2026-11-23

- Plan created with Phase 1 active. Baseline captures collected for Home, Library, Detail and Settings. Player baseline capture is blocked by the unrelated host simvideo failure and will not be treated as redesign work.
- Phase 1 implementation plan completed and regression authoring started.
- Phase 1 code landed and PR #450 opened. Final code commit `efe7afed2` passed 3972 default, 4013 hostsim, 102 storage and 352 harness tests, release and hint checks, six deterministic captures, and 6/6 strict replays. CI on main plus Simulator remains to confirm.
