---
type: planning
entity: phase
plan: "NetGrip visual redesign"
phase: 1
status: completed
created: "2026-11-22"
updated: "2026-11-22"
---

# Phase 1: Shared language, Home and Detail

> Part of [NetGrip visual redesign](../plan.md)

## Objective

Establish the confirmed graphite-and-amber language in shared roles and components, then prove it on Home and Detail at full 1920x1080 simulator resolution.

## Scope

### Includes

- Required theme roles, component treatments and visual-state tests.
- Home navigation, hero, action row, carousel and next-content row.
- Detail backdrop, poster, metadata, actions and below-hero continuity.
- Simulator captures of ordinary and focused states on this PC.

### Excludes

- Library, Settings and Player beyond preserving their compilation and existing behavior.
- Navigation changes, new features, playback internals and television validation.

## Prerequisites

- [x] `PRODUCT.md`, the confirmed direction and the plan exist.
- [x] Current visual evidence and the supplied NetGrip reference have been read.

## Deliverables

- [x] Shared visual roles and reusable treatments needed by Home and Detail.
- [x] Home and Detail implementing the direction without behavior changes.
- [x] Focused host regressions for new derivations.
- [x] Full-resolution simulator captures reviewed by the agent.

## Acceptance Criteria

- [x] Both surfaces clearly carry the confirmed visual world and retain readable text, focus and primary actions.
- [x] No screen-local raw color, text-size or hand-placed-text violations are introduced.
- [x] Existing navigation and content behavior remain intact.
- [x] Targeted host tests and simulator capture commands pass.

## Dependencies on Other Phases

| Phase | Relationship | Notes |
|-------|-------------|-------|
| 2 | blocks | The shared language and reference captures must exist before extending it. |
