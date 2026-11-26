---
type: planning
entity: phase
plan: "butaca-mockup-fidelity"
phase: 4
status: pending
created: "2026-11-23"
updated: "2026-11-23"
---

# Phase 4: Real audio and subtitle preferences

> Part of [butaca-mockup-fidelity](../plan.md)

## Objective

Implement the three preferences shown by the mockup as real product behavior: preferred audio language, subtitle mode and preferred subtitle language.

## Scope

### Includes

- Persisted fields and a forward-compatible session migration.
- Settings rows and native pickers connected to those fields.
- Track-selection precedence: explicit item choice, then installed preference, then server default, then Off where applicable.
- A distinction between explicit Off and no user selection so existing retry and transcode behavior remains intact.
- Migration, Settings, route-plan and track-selection regressions, followed by full gates and final validation.

### Excludes

- Changes unrelated to these three preferences or their required track-selection behavior.
- Using decorative Settings rows without persistence and playback effect.

## Deliverables

- [ ] Preference schema, migration, Settings UI and route-plan integration.
- [ ] Tests for languages, forced subtitles, explicit Off, retries and transcodes.
- [ ] Full host/replay/CI evidence and a final device validation report.

## Acceptance Criteria

- [ ] All three preferences survive restart and are editable through the real Settings flow.
- [ ] Explicit item selections outrank preferences, and explicit Off is not mistaken for an unset choice.
- [ ] Preferences influence direct-play and transcode track selection only through the documented precedence.
- [ ] Full gates pass on the committed tree and ARM plus Simulator CI are green.
- [ ] Final validation uses a guest or mock identity and leaves no real-account writes.

## Dependencies on Other Phases

| Phase | Relationship | Notes |
|-------|-------------|-------|
| 1 | blocked-by | Adds rows to the Settings composition and native picker behavior established in Phase 1. |
