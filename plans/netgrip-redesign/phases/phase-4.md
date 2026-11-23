---
type: planning
entity: phase
plan: "NetGrip visual redesign"
phase: 4
status: completed
created: "2026-11-23"
updated: "2026-11-23"
---

# Phase 4: Structural menu follow-up

> Part of [NetGrip visual redesign](../plan.md)

## Objective

Make the menus visibly NetGrip-derived instead of preserving the old tvOS structure under a new palette. Keep every destination, action, focus identity and playback behavior intact.

## Scope

### Includes

- Flat top navigation with icons above short uppercase labels, underline selection and rectangular focus keylines.
- Numbered Library eyebrow, compact outlined filter chips and 520 px sort/filter/type/genre popovers with hairlines.
- Settings' numbered compact header and one technical card per section, with rows inside.
- Host regressions and full-resolution simulator comparisons against the committed mock library.

### Excludes

- New destinations, navigation models, playback changes, television work, Play On and unrelated bug fixes.

## Prerequisites

- [x] The first visual PR #448 is green, and the user identified that its menus still look unchanged.

## Deliverables

- [x] Flat top navigation with complete icon and label vocabulary.
- [x] Technical Library chips and compact popovers.
- [x] Settings' compact numbered header and section cards.
- [x] Passing regressions, full suites, replay and reviewed simulator captures.

## Acceptance Criteria

- [x] The menu structure is visibly different from the previous tvOS capsules, large popovers and per-setting buttons.
- [x] Destination identities, focus order, actions, strings, remote behavior and 1920x1080 geometry contracts remain intact.
- [x] Host, hostsim, release-feature and replay gates pass on the final committed tree.
- [x] The follow-up PR head passes ARM and Simulator CI.

## Dependencies on Other Phases

| Phase | Relationship | Notes |
|-------|-------------|-------|
| 1-3 | follows | The first palette-only implementation passed CI but did not satisfy the user's structural feedback. |
