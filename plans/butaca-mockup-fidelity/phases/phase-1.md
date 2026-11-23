---
type: planning
entity: phase
plan: "butaca-mockup-fidelity"
phase: 1
status: in_progress
created: "2026-11-23"
updated: "2026-11-23"
---

# Phase 1: Shared chrome and Settings

> Part of [butaca-mockup-fidelity](../plan.md)

## Objective

Deliver a visible first milestone: mockup-derived shared navigation, profile/server access, display typography and a Settings page whose composition and native pickers match the reference instead of the incumbent layout.

## Scope

### Includes

- Textual Home, Favorites and Search destinations, the real profile/avatar entry, and mockup ordering of navigation, settings and clock.
- Shared chrome on Detail and Settings with working D-pad and activation paths, not drawing alone.
- Real server switching and adding through the profile menu using existing account/server actions.
- Manrope interface typography and a scoped Fraunces display role, packaged with Inter/CJK fallback and licensing updates.
- Settings title, real profile/version/privacy subtitle, two-column card geometry and anchored native pickers with BACK/focus restoration.
- Regression tests and 1920x1080 captures for Home chrome, Detail chrome, Settings root and representative pickers.

### Excludes

- Home, Library, Detail and Player body redesign beyond the shared chrome they consume.
- The three new playback preferences, which are isolated in Phase 4.
- Changing server/account behavior beyond wiring existing real actions into the mockup profile entry.

## Deliverables

- [ ] Shared chrome and account-menu changes with stable focus identities and working destinations.
- [ ] Scoped text-family support and packaged font assets with fallback coverage.
- [ ] Settings layout and anchored picker changes using existing state and actions.
- [ ] Passing targeted regressions and reviewed comparison captures.

## Acceptance Criteria

- [ ] Home, Detail and Settings expose the mockup's textual navigation and real profile/server access through drawing and D-pad activation.
- [ ] Settings is recognizably aligned with the mockup's title, subtitle, two-column cards and native anchored selectors.
- [ ] Display typography does not replace ordinary body copy or CJK fallback.
- [ ] Existing actions, server switching, force-Direct-Play confirmation and focus restoration remain covered and green.
- [ ] Captures are opened and reviewed against the mockup at 1920x1080.

## Dependencies on Other Phases

| Phase | Relationship | Notes |
|-------|-------------|-------|
| 2 and 3 | blocks | Later surfaces consume the shared chrome and typography roles established here. |
