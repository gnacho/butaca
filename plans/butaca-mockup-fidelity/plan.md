---
type: planning
entity: plan
plan: "butaca-mockup-fidelity"
status: active
created: "2026-11-23"
updated: "2026-11-23"
---

# Plan: butaca-mockup-fidelity

## Problem / Context

Issue #449. The current interface preserves parts of the supplied `butaca-rediseno-mockup.zip`, but its shared menus and several screen compositions are not a faithful implementation. Restoring the incumbent baseline did not reduce those gaps. The audit mapped material differences across shared chrome, Home, Library, Search, Detail, Settings and Player.

## Target Outcome

A visibly faithful native implementation of the supplied Butaca mockup, demonstrated with before-and-after 1920x1080 captures on every surface, while preserving the Jellyfin palette adaptation and existing product behavior except for the explicitly requested audio and subtitle preferences.

## Guiding Decisions & Constraints

- The supplied Butaca mockup is the only visual reference.
- Preserve the current Jellyfin violet/blue palette and artwork-derived atmosphere rather than copying the mockup's warm colors.
- Preserve the fixed 1920x1080 canvas, LG safe margins, couch-legibility floor, remote-first focus, destinations and existing native capabilities.
- Use and extend the shared Rust theme, text, layout, widget and motion systems. No parallel screen-local design system.
- Mockup placeholder interactions become real native flows using existing routes and actions. No hard-coded profile, version, media, timestamps or server data.
- Audio language, subtitle mode and subtitle language are included as real persisted preferences with explicit track-selection tests. They are not decorative rows.
- No merge or release is included.

## Requirements

### Functional

- [ ] Align shared chrome, profile/server access, typography and Settings with the mockup.
- [ ] Align Home, Library, Search, Detail and Player, including the mockup structures missing from the current implementation.
- [ ] Preserve existing controls and capabilities as deliberate native additions where the mockup has no equivalent.
- [ ] Add persisted audio-language, subtitle-mode and subtitle-language preferences and use them in track selection with explicit precedence.

### Non-Functional

- [ ] Keep keyboard, pointer and focus identities coherent across the changed geometry.
- [ ] Keep ordinary content at or above the existing couch-legibility floor and within LG safe margins.
- [ ] Package any new font roles with fallback coverage and verify their rendering on the television.

## Scope

### In Scope

- Shared chrome, typography, Home, Library, Search, Detail, Settings and Player presentation.
- Data already present in Jellyfin DTOs that is needed for mockup metadata and badges.
- Real preference persistence, Settings rows and track-selection precedence for the three requested preferences.
- Regression tests, replay measurements, simulator captures, CI and final device validation.

### Out of Scope

- A new navigation model, playback-engine rewrite or unrelated bug fixes.
- Copying mockup placeholder toasts, fictitious data or warm palette.
- The unrelated host `simvideo` first-frame failure observed while collecting baseline captures.
- Merge, release or stable installation.

## Definition of Done

- [ ] Every named surface has reviewed before-and-after captures showing material alignment with the mockup.
- [ ] Shared menus and Settings no longer contain the confirmed structural mismatches.
- [ ] The three new preferences persist and drive track selection without breaking explicit item choices or Off.
- [ ] Existing destinations, actions, focus behavior and native-only controls remain covered by regressions.
- [ ] Host, hostsim, release-feature and replay gates pass on the committed tree, followed by green ARM and Simulator CI.
- [ ] Television validation covers display typography, chrome focus and Player composition without real-account writes.

## Testing Strategy

- [ ] Unit and geometry regressions for shared widgets, chrome, screen layouts and preference precedence.
- [ ] Navigation and activation tests for every changed focus path and popover.
- [ ] Supported replay measurement fills where text metrics change, with no behavior re-recording unless the fixture shape requires it.
- [ ] Reviewed 1920x1080 mock-library captures before and after each phase.
- [ ] Final television checks for font rasterization, focus and Player composition.

## Phases

| Phase | Title | Contribution | Why Separate | Detail | Status |
|-------|-------|--------------|--------------|--------|--------|
| 1 | Shared chrome and Settings | Delivers the first visibly faithful menu structure, display typography and real profile/server access | Establishes shared components consumed by later surfaces and provides an early visible milestone | [Phase](phases/phase-1.md) | in_progress |
| 2 | Home, Library and Search | Implements the cinematic hero, poster grids, filters and search composition | Builds on the shared chrome and data contracts without mixing in Detail or Player geometry | [Phase](phases/phase-2.md) | pending |
| 3 | Detail and Player | Aligns media presentation, track controls, episodes and playback OSD | Requires the shared typography and metadata work from earlier phases and has device-specific validation needs | [Phase](phases/phase-3.md) | pending |
| 4 | Real audio and subtitle preferences | Adds the three persisted preferences and applies them to track selection | Changes server/session and route behavior beyond visual fidelity and needs isolated migration and playback tests | [Phase](phases/phase-4.md) | pending |

## Changelog

### 2026-11-23

- Plan created after the full mockup audit and the user's decision to include real audio/subtitle preferences.
- Phase 1 implementation plan authored and verified. Phase 1 started.
