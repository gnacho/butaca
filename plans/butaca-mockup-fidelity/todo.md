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
- [ ] Run the final committed-tree gates and publish the Phase 1 PR <!-- added: 2026-11-23 -->


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
- Phase 1 code landed in ba1be9e4a, d92adebf4, 10773d8b4, b0dbde7cf and 1a8639953. Replay re-recorded; six captures reviewed and deterministic. Final committed-tree gate running.
