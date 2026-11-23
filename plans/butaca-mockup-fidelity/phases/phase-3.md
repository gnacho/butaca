---
type: planning
entity: phase
plan: "butaca-mockup-fidelity"
phase: 3
status: pending
created: "2026-11-23"
updated: "2026-11-23"
---

# Phase 3: Detail and Player

> Part of [butaca-mockup-fidelity](../plan.md)

## Objective

Align media detail and playback presentation with the mockup's editorial detail composition and unobtrusive playback OSD while preserving native playback controls and actions.

## Scope

### Includes

- Detail's 52vh backdrop, overlapped 280x420 poster, scoped display title, kicker, action order and lower track selectors.
- Vertical episode rows with independent play/detail actions, plus retained cast and related shelves.
- Player title/metadata placement, left-aligned primary controls and retained secondary native controls.
- Bottom playbar with fixed time labels, focused knob and real chapter markers, plus safe Skip Intro and Up Next placement.
- Geometry, pointer, activation and overlay regressions, followed by simulator and television captures.

### Excludes

- Playback-engine or decoder changes.
- Preference persistence and default track-selection policy, handled in Phase 4.

## Deliverables

- [ ] Detail and episode layouts using existing data and actions.
- [ ] Player HUD/playbar geometry with retained track, quality, chapter and marker behavior.
- [ ] Passing targeted regressions and reviewed comparison captures.

## Acceptance Criteria

- [ ] Detail visibly matches the mockup's backdrop, poster, copy and action composition without losing Restart, Trailer, alternate versions or artwork.
- [ ] Track selectors continue to reach the real playback choices and selected tracks survive the flow.
- [ ] Player controls, chapter markers and pointer targets match the new geometry without cross-activation or subtitle collisions.
- [ ] Final television captures verify native text and video-plane composition without playback-history writes.

## Dependencies on Other Phases

| Phase | Relationship | Notes |
|-------|-------------|-------|
| 1 and 2 | blocked-by | Uses shared typography, chrome and metadata established earlier. |
