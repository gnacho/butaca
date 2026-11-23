---
type: planning
entity: plan
plan: "NetGrip visual redesign"
status: active
created: "2026-11-22"
updated: "2026-11-23"
---

# Plan: NetGrip visual redesign

## Problem / Context

Butaca already has a mature, remote-first native interface. Issue #444 replaces its visual world using the NetGrip guides supplied by the user, while keeping the product behavior and native architecture intact. `PRODUCT.md` records the confirmed product truth. `docs/DESIGN.md` and `rust-modules/src/ui/CLAUDE.md` describe the incumbent implementation and its binding rules.

## Target Outcome

Home, Detail, Library, Settings and Player share a deliberate graphite-and-amber language derived from NetGrip: technical hierarchy, keylines, smaller radii, disciplined spacing and unmistakable amber focus. Artwork remains the protagonist. The result is deployed and reviewed on this PC through the native simulator, with captures of ordinary, focused and modal states.

## Guiding Decisions & Constraints

- Preserve product truth, content, navigation, remote-first input and existing features.
- Use the existing theme, layout, widget and motion systems. No parallel visual primitives or web stack.
- Do not import NetGrip's radar, network motifs, mobile density or domain vocabulary.
- Keep the fixed 1920x1080 canvas, text-legibility rules and performance budget.
- Validate on this PC. No television session is part of this iteration.
- Do not include Play On or unrelated bug fixes.

## Direction contract

- THESIS: a private cinema presented with the discipline of a technical control surface, not a generic wall of posters and not a router-console costume.
- OWN-WORLD: graphite grounds, amber signal and focus, warm-neutral reading ink, compact keylines, smaller radii and technical uppercase labels using the existing native type family.
- STORY: the viewer recognizes the library, understands the selected work and sees the next action immediately. Artwork leads while the interface provides orientation and control.
- FIRST VIEWPORT: a full-width cinematic backdrop, a flat navigation strip with amber selection, a clearly structured title/meta/synopsis/action block and the next content row visible. Detail uses the same hierarchy around its poster and work information.
- FORM: the confirmed NetGrip translation, pinned by the user. `seed: none (unavailable)`, because the supplied direction replaces a concept roll.
- FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance.

## Requirements

### Functional

- [x] Apply the new language to Home, Detail, Library, Settings and Player, including their focused and modal states.
- [x] Preserve navigation, content, playback controls, settings and all existing user workflows.
- [x] Run and capture the redesigned app on this PC with the native simulator and mock Jellyfin content.

### Non-Functional

- [x] Preserve fixed-canvas geometry contracts or update them through the existing tested layout machinery.
- [x] Keep text readable at the existing size floor and never use color alone as the only state distinction.
- [x] Keep motion and rendering within existing native performance and invalidation contracts.

## Scope

### In Scope

- Shared visual tokens and reusable component treatments required by the redesign.
- Home, Detail, Library, Settings and Player rendering.
- Associated visual and host regressions, simulator captures and documentation.
- Local validation, PR preparation and CI after the user authorizes publication.

### Out of Scope

- Play On and any playback-engine or server-protocol change.
- New screens, navigation models, product features or copy changes not required by the visual translation.
- Television deployment or validation in this iteration.
- NetGrip network-specific motifs, radar graphics and mobile layouts.

## Definition of Done

- [x] The five surfaces visibly implement the confirmed direction with coherent focused and modal states.
- [x] The simulator runs on this PC and the final captures have been opened and reviewed.
- [x] Relevant host, hostsim, release-feature and replay gates pass on the final committed tree.
- [x] ARM and Simulator CI pass after the authorized PR is opened.
- [x] `docs/DESIGN.md` and the plan accurately describe the finished world and verification evidence.
- [x] The local PR does not close #444. The issue retains final TV validation for a separately authorized iteration.

## Testing Strategy

- [x] Unit tests for new token, geometry, focus and state derivations that can fail.
- [x] Existing host and hostsim suites, release feature check and replay gate.
- [x] Simulator screenshots against the committed mock Jellyfin library, opened by the agent and checked across all five surfaces.
- [x] ARM and Simulator CI on the final PR, without any claim of television verification.

## Local visual review

- Independent verdict: `ship`, with no material findings and no required action.
- Coverage: Home ordinary/shelf/strip focus, Detail primary action, Library opening/grid/sort, Settings root/quality picker and paused Player.
- Captures were opened at 1920x1080. The review is simulator evidence only: it does not prove LG rasterization, video-plane composition, native playback, transitions or performance.
- Known unrelated defects remain tracked by #445, #446 and #447 rather than being absorbed into this branch.

## Publication

- PR #448, `feat(ui): redesign the interface from NetGrip`, targets `main` with `Refs #444` and no automatic issue closure.
- Verification commit: `d3f10debac40a0cc7aec1b6e9abb1bdb950310f1`.
- CI run 45121893410: host checks and ARM cross-build with artifact assertions passed.
- Simulator CI run 45121893334: Linux build, 1080p launch capture, replay and Windows launcher passed.
- No merge, release or television validation was performed. Final TV validation remains with #444 for a separately authorized iteration.

## Phases

| Phase | Title | Contribution | Why Separate | Detail | Status |
|-------|-------|--------------|--------------|--------|--------|
| 1 | Shared language, Home and Detail | Establishes the new world on the two cinematic reference surfaces | Creates a reviewable end-to-end visual baseline before extending it | [Phase](phases/phase-1.md) | completed |
| 2 | Library, Settings and Player | Extends the proven language to every remaining surface and state | Depends on the Phase 1 shared components and visual review | [Phase](phases/phase-2.md) | completed |
| 3 | Local finish and publication | Completes capture review, gates, documentation and PR evidence | Verification is meaningful only after all surfaces are integrated | [Phase](phases/phase-3.md) | completed |
| 4 | Structural menu follow-up | Replaces the old tvOS menu structure with visibly NetGrip-derived navigation, chips and section cards | The first palette-only PR passed CI but the user still saw the old menus | [Phase](phases/phase-4.md) | in_progress |

## Changelog

### 2026-11-22

- Plan created after the user confirmed the complete redesign, the NetGrip guides and local PC validation.
- Applied review note F-1: the local PR will not close #444, whose final TV validation requires a separately authorized iteration.
- Phase 1 completed after reviewed 1920x1080 Home/Detail captures, 3965 default and 4006 hostsim tests, 164 Detail tests and a green no-default-features check. The repository's clean-tree self-test remains deferred until an authorized commit.
- Phase 2 started for Library, Settings and Player.
- Phase 2 completed with five reviewed 1920x1080 scenes plus the Settings picker, targeted suites 3/3 ValueChip, 13/13 Table, 43/43 Settings and 36/36 HUD, and a green no-default-features check. Phase 3 started.
- #446 tracks the stale Library shelf capture and #447 tracks the shared Back/wordmark overlap. Neither is included in this visual branch.
- Local finish: independent visual verdict `ship`; 3966 default and 4007 hostsim tests, 102 storage tests, no-default-features, lint, dependency/static/flavor/Lab checks and demo manifests green. Replay: 3 fixtures, 6 replays, all SAME with zero focus/input divergences. The clean-tree gate and CI still require authorized publication.

### 2026-11-23

- PR #448 opened with `Refs #444`. Host checks, ARM cross-build, Linux simulator, 1080p capture, replay and Windows launcher all passed. Plan completed; no merge, release or TV validation.
- User feedback showed that palette-only changes still read as the old menus. Phase 4 started for flat navigation, technical chips/popovers and section cards, with regressions before production edits.
