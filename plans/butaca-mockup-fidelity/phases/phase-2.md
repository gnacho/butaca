---
type: planning
entity: phase
plan: "butaca-mockup-fidelity"
phase: 2
status: pending
created: "2026-11-23"
updated: "2026-11-23"
---

# Phase 2: Home, Library and Search

> Part of [butaca-mockup-fidelity](../plan.md)

## Objective

Implement the mockup's cinematic browsing experience across Home, Library and Search while retaining real Jellyfin content and existing navigation.

## Scope

### Includes

- Home kicker, bounded synopsis, textual primary/secondary actions, right-aligned page indicators and panoramic Continue Watching cards with real progress.
- Real favorite/rating/resolution metadata needed by the mockup, using Jellyfin DTO fields without changing server behavior.
- Library grid-first composition, safe-area-compatible poster columns, library-specific titles/counts and retained shelves/A-Z access.
- Separate unwatched and genre controls plus native sort/type menus.
- Search field and result composition derived from the mockup while preserving editing, recents and server search.
- Regression tests and reviewed Home, Library, Sort and Search captures.

### Excludes

- Detail, episodes and Player geometry.
- New audio/subtitle preference behavior.

## Deliverables

- [ ] Home hero and panoramic Continue Watching implementation.
- [ ] Library and Search layouts with preserved navigation and real filtering/sorting.
- [ ] Metadata plumbing required by the new visual elements.
- [ ] Passing targeted regressions and reviewed comparison captures.

## Acceptance Criteria

- [ ] Home displays the mockup's missing hero elements and panoramic progress cards without changing activation behavior.
- [ ] Library opens with the mockup's title/toolbar/grid composition and keeps shelves, A-Z, type, sort and filters reachable.
- [ ] Search matches the reference composition without clipping the keyboard or losing existing behavior.
- [ ] Favorite and other metadata are real and absent gracefully when the server does not supply them.
- [ ] Captures show material improvement against Home, Movies and Search references at 1920x1080.

## Dependencies on Other Phases

| Phase | Relationship | Notes |
|-------|-------------|-------|
| 1 | blocked-by | Uses the shared chrome and typography delivered in Phase 1. |
