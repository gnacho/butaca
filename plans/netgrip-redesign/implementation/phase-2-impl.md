---
type: planning
entity: implementation-plan
plan: "NetGrip visual redesign"
phase: 2
status: completed
created: "2026-11-22"
updated: "2026-11-22"
---

# Implementation Plan: Phase 2 - Library, Settings and Player

> Implements [Phase 2](../phases/phase-2.md) of [NetGrip visual redesign](../plan.md)

## Approach

Extend the accepted Phase 1 language through four direct changes to existing code: make Library's `ValueChip` compact and keylined, put Settings on the authored graphite route ground, align the shared table corner radii with the compact card radius, and recolor the existing Player playbar through shared roles. Reuse `RouteGround::draw_default`, `TableView`, `Painter::rring`, `RAIL_TRACK` and `ACCENT`. Add no theme primitive, widget, font, screen, navigation rule or playback behavior.

Library's shelves, grids and popovers already inherit the shared Phase 1 roles, so their screen composition and menu actions stay unchanged. Settings keeps its modal scrim, entrance, inner stack, pickers and focus model. Player changes only `draw_playbar` colors, not its rects, clocks, state glyph, scrub state, timing or transport. The existing screenshot harness supplies the visual evidence on the committed mock Jellyfin library. Phase 3 still owns replay, release-feature, publication and other final gates.

## Affected Modules

| Module | Change Type | Description |
|--------|-------------|-------------|
| `rust-modules/src/ui/value_chip.rs` | modify | Replace the semicircular Library toolbar capsule with a compact graphite plate and idle keyline, retaining the amber focused face and existing measurements. Add focused draw regressions in its existing test module. |
| `rust-modules/src/screens/settings.rs` | modify | Draw ordinary Settings through the authored default route ground instead of sampling the rendered Home artwork. Update the affected local comments. |
| `rust-modules/src/ui/route_screen.rs` | modify | Correct only the route-ground comments whose ordinary-Settings host-sampling claim becomes false. |
| `rust-modules/src/ui/table.rs` | modify | Resolve the existing row-pill and card radii to the compact shared card radius without changing table geometry or behavior. Add a focused radius regression. |
| `rust-modules/src/ui/theme.rs` | modify | Correct the existing radius cross-reference comment only; do not retune any accepted Phase 1 role. |
| `rust-modules/src/ui/player_hud.rs` | modify | Use the shared accent role for playbar progress and playhead, and derive its focused glow from that role. |
| `tests/screenshots/scenes.json` | modify | Add full-resolution Library and Player outputs and one mock-backed Settings root scene, reusing the existing capture driver and triggers. |

## Required Context

| File | Why |
|------|-----|
| `plans/netgrip-redesign/plan.md` | Global visual contract, preserved product behavior and exclusion of TV and unrelated work. |
| `plans/netgrip-redesign/phases/phase-2.md` | Gated surface scope, deliverables and acceptance criteria for this phase. |
| `plans/netgrip-redesign/implementation/phase-1-impl.md` | Accepted shared-language continuity and the explicit boundary with Phase 3 verification. |
| `PRODUCT.md` | Fixed canvas, remote-first focus, artwork priority and preserved workflows. |
| `docs/DESIGN.md` | Role-based color, shared-component ownership and fixed-canvas legibility rules. |
| `rust-modules/src/ui/CLAUDE.md` | Binding role, component, focus, popover, modal-scrim and rendering invariants. |
| `rust-modules/src/screens/CLAUDE.md` | Screen composition, focus ownership and capture requirements. |
| `rust-modules/src/player/CLAUDE.md` | Playback boundaries and the evidence limits of host and simulator verification. Only HUD constraints apply here. |
| `rust-modules/src/ui/theme.rs` | Current graphite, accent, keyline, rail and card-radius roles. Current source is authoritative for their values. |
| `rust-modules/src/ui/value_chip.rs` | Existing chip draw branches, measured width and inline host tests. |
| `rust-modules/src/screens/library/draw.rs` | Real `ValueChip` paint call, focus input and layer order, all preserved. |
| `rust-modules/src/screens/library/toolbar.rs` | Existing chip measurement, rect placement, element identities and menu anchors, all preserved. |
| `rust-modules/src/screens/library/menu.rs` | Existing shared `TableView` popover composition and actions, preserved without a screen-local treatment. |
| `rust-modules/src/screens/settings.rs` | `RouteSurface::draw`, `ground_ready`, modal alpha composition and `SessionGround::refresh` call site. |
| `rust-modules/src/screens/family.rs` | `SessionGround::new` and `refresh`, which show why an ordinary `RouteSurface` does not follow a session seed. |
| `rust-modules/src/ui/route_screen.rs` | Existing `draw_host`, `draw_default`, fallback latching and ground-readiness-related contracts. |
| `rust-modules/src/screens/settings_draw_and_lifecycle_tests.rs` | Existing modal alpha, page-draw and mount-time behavior regressions that must remain intact. |
| `rust-modules/src/ui/table.rs` | Existing amber/plain-ink rules, card and keyline draws, radii, geometry and inline tests. |
| `rust-modules/src/ui/player_hud.rs` | `draw_playbar`, `Playbar`, `Knob`, existing HUD geometry and behavior tests. |
| `tests/screenshots/scenes.json` | Existing Library, sort-menu and paused-Player target states and output declarations. |
| `.agents/skills/ui-sim/SKILL.md` | Mock-backed full-resolution capture, output selection, visual review and simulator evidence limits. |

## Implementation Steps

### Step 1: Pin the compact chip and table treatments with the existing host harness

- **What**: Extend `value_chip.rs`'s existing tests with a recording-painter draw regression: an idle chip emits its fill and one keyline on the same unchanged rect, while the focused chip retains its fill without the idle keyline. Keep the existing width and focus-layout tests unchanged. In `table.rs`, add a small regression asserting that `PILL_RAD` and `CARD_RAD` both resolve to `theme::CARD_RING_RAD`, rather than deleting any existing card, selection, motion or text-fit coverage.
- **Where**: The existing inline test modules in `rust-modules/src/ui/value_chip.rs` and `rust-modules/src/ui/table.rs`; no new test file or harness.
- **Authorized By**: Phase 2 Deliverables "Reusable state treatments" and "Relevant host regressions", and Acceptance Criteria requiring existing shared component contracts to remain intact.
- **Why**: The current chip tests protect measurement but not the missing idle keyline, and the table's 18/20 radii are not pinned to the accepted compact corner language. These focused assertions fail against those old treatments while preserving the behavioral suites around them.
- **Considerations**: The draw census proves primitive presence and rect preservation, not pixel color. The simulator captures in Step 6 remain the visual evidence for graphite, amber and contrast.

### Step 2: Translate Library's toolbar chip in place

- **What**: In `ValueChip::draw`, replace both `r.h * 0.5` corner arguments with `theme::CARD_RING_RAD`. Give the idle face the existing graphite `theme::SURFACE_PANEL` fill and a 1 px `theme::HAIRLINE` ring through `Painter::rring`. Keep the focused `ACCENT` fill, `ACCENT_INK` value/name/icon treatment, note alpha and all text positions unchanged. Do not edit `ValueChip::width`, `PAD`, `ICON_SIZE`, `ICON_GAP`, the builder inputs or the 52 px caller rect.
- **Where**: `rust-modules/src/ui/value_chip.rs` only. Read, but do not change, `screens/library/draw.rs`, `toolbar.rs` and `menu.rs`.
- **Authorized By**: Phase 2 Includes "Library shelves, grids, sort/filter menus and focus states", plan Direction contract requiring compact keylines and smaller radii, and the plan requirement to reuse existing widgets.
- **Why**: This shared chip is the remaining screen-specific visual holdout. Its current full capsule and absent idle border are corrected at the one existing draw site, while every Library shelf, grid, focus identity and menu action continues through its current code.
- **Considerations**: The keyline belongs to the idle state; the focused state remains a solid amber fill with dark ink. No new chip variant, parallel Library widget, size change, menu-trigger change or content rewrite.

### Step 3: Put Settings on graphite and align its existing table corners

- **What**: In `RouteSurface::draw`, replace only `self.ground.draw_host(root.alpha(a))` with `self.ground.draw_default(root.alpha(a))`, so `ROUTE_GROUND_FALLBACK` supplies the authored graphite field without sampling Home. Leave the preceding `settings_scrim_alpha` rectangle, entrance alpha, page drawing and `ground_ready = a >= 0.995` exactly as they are. Update the local `Family::Settings` and `RouteGround` comments that would otherwise still describe host sampling. In `TableView`, resolve `PILL_RAD` and `CARD_RAD` to `theme::CARD_RING_RAD` and correct the adjacent radius comments, including the existing cross-reference in `theme.rs`.
- **Where**: `rust-modules/src/screens/settings.rs`, `rust-modules/src/ui/route_screen.rs`, `rust-modules/src/ui/table.rs` and the comment-only radius reference in `rust-modules/src/ui/theme.rs`.
- **Authorized By**: Phase 2 Includes "Settings routes, cards, rows, pickers and modal surfaces", Acceptance Criteria requiring preserved modal hierarchy and behavior, and plan Direction contract requiring graphite grounds and smaller radii.
- **Why**: The current simulator capture shows Home's brown/teal artwork light surviving through `draw_host`. `draw_default` is the existing direct path to the authored fallback and latches it once through `latch_default`. `RouteSurface` constructs `SessionGround::new`, whose `from_session` is false, so its existing `refresh` remains a no-op and cannot replace this ground from a later session seed. The existing table already supplies amber focus, dark/plain ink, cards and keylines, so only its corner constants need to move.
- **Considerations**: Do not alter `draw_host` for other callers, `RouteGround::for_home`, `pre_home_ground`, onboarding, session persistence, route transitions, row heights, column counts, table springs, focus stops or picker behavior. No theme role or color value changes.

### Step 4: Recolor the existing Player playbar through shared roles

- **What**: In `draw_playbar`, keep the unfilled track on `theme::RAIL_TRACK`, but use `theme::ACCENT` for the played rail and the playhead knob/tick in every `Knob` variant. Replace the raw focused glow with `theme::with_a(theme::ACCENT, 0.22)`, retaining its existing 34 px rect and the existing 22/18/3 px playhead geometry. Keep elapsed and remaining clocks on `TEXT_PRIMARY` and `TEXT_SECONDARY`, and leave the state glyph and every other HUD component alone.
- **Where**: `rust-modules/src/ui/player_hud.rs`, inside `draw_playbar` only.
- **Authorized By**: Phase 2 Includes "Player HUD, progress, state glyph, chapters and overlays", plan Direction contract requiring amber signal and focus, and Acceptance Criteria requiring player state and behavior to remain legible and intact.
- **Why**: Progress and the focused playhead currently read as local white rather than the accepted amber signal. `ACCENT` and `RAIL_TRACK` already express the required jobs. Retargeting `RAIL_FILL` would instead recolor the shared paged/continuous scroll rails and `tracks_panel`, so the least broad correct change is at the Player playbar call site.
- **Considerations**: Do not create a playbar widget or theme token, and do not touch `Playbar::live`, `TransportMark`, scrub fractions, clock measurement, focus identities, control discs, chapters, overlays, subtitles, playback timing, seeking or the playback engine.

### Step 5: Add only the missing full-resolution capture declarations

- **What**: Add a 1920x1080 `ux-library.jpg` output to the existing `library` opening-state scene while retaining its 1600x900 documentation output, and add a 1920x1080 `ux-player.jpg` output to the existing paused `player` scene while retaining `player.jpg`. Add one `settings` scene with trigger `{"settings": "root"}`, expectations `coldopen screen=settings` and `route=home overlay=settings`, and one 1920x1080 `ux-settings.jpg` output. Reuse the existing `ux-library-grid.jpg` and `ux-library-sort.jpg` scenes unchanged for the grid and sort-popover states.
- **Where**: `tests/screenshots/scenes.json` only; use the existing `tools/screenshots.py` driver and mock catalog without new infrastructure.
- **Authorized By**: Phase 2 Includes "Full-resolution simulator captures of ordinary, focused and modal states", Deliverables requiring complete simulator captures, and plan Testing Strategy requiring committed mock Jellyfin content.
- **Why**: The Library opening and Player target states already exist but lack full-resolution outputs, and no scene currently reaches Settings. One output addition per existing scene and one Settings target supply the missing evidence without changing capture behavior or creating a parallel harness.
- **Considerations**: Keep the existing Player `clocksink`, `simvideo` and `autopause` triggers and expectations byte-for-byte. Do not add real-account access, TV capture, new playback scenarios or final-phase release/replay work.

### Step 6: Run the phase gate and review the actual frames

- **What**: Run the primary verify command below, then open `ux-library.jpg`, `ux-library-grid.jpg`, `ux-library-sort.jpg`, `ux-settings.jpg` and `ux-player.jpg` at their native 1920x1080 size. Review compact chip corners and idle keylines, unchanged Library opening/grid/menu composition, amber focus with readable ink, Settings' graphite ground without the baseline brown/teal host light, compact table cards, and Player's amber played rail/playhead/focus glow with unchanged clocks and paused state. Iterate only on the direct treatments above if a capture exposes a defect in this phase.
- **Where**: Local output under `/tmp/opencode/netgrip-phase-2`, produced against the committed mock library. No production server, real account or television.
- **Authorized By**: Phase 2 Acceptance Criteria requiring coherent operable surfaces, legible focus/modal/player states and passing targeted host and capture commands, and Deliverables requiring complete captures.
- **Why**: Host tests cannot judge pixels, while these five existing-harness states cover the changed chip, table, route ground and playbar plus Library's focused and modal states. Opening the files is required for visual acceptance; command success alone is insufficient.
- **Considerations**: Report simulator evidence only. Do not claim LG text rasterization, native video-plane composition, playback correctness, television performance or TV validation. Leave replay, release-feature, final documentation and PR gates to Phase 3.

## Testing Plan

**Primary Verify Command**: `set -e; CARGO_INCREMENTAL=0 cargo +nightly-2026-07-02 test --manifest-path rust-modules/Cargo.toml --lib ui::value_chip::tests && CARGO_INCREMENTAL=0 cargo +nightly-2026-07-02 test --manifest-path rust-modules/Cargo.toml --lib ui::table:: && CARGO_INCREMENTAL=0 cargo +nightly-2026-07-02 test --manifest-path rust-modules/Cargo.toml --lib screens::settings && CARGO_INCREMENTAL=0 cargo +nightly-2026-07-02 test --manifest-path rust-modules/Cargo.toml --lib ui::player_hud:: && RUSTUP_TOOLCHAIN=nightly-2026-07-02 make screenshots SHOT_SCENES=ux-library.jpg,ux-library-grid.jpg,ux-library-sort.jpg,ux-settings.jpg,ux-player.jpg SHOT_OUT=/tmp/opencode/netgrip-phase-2`

The test legs cover the new chip/radius regressions and existing Library, Settings, route-ground and Player behavior. The capture leg builds the simulator and produces the five mock-backed full-resolution states for the required opened-image review. The full clean-tree `make check` remains a Phase 3 publication gate because it intentionally rejects this uncommitted working tree. Record the actual test result lines and visual findings, without treating the simulator as television evidence.

## Reality Check

- Phase 1's implementation prose and current `theme.rs` do not agree on every role-to-stop mapping: current `SURFACE_PANEL` resolves to `#17191D`, while `CONTROL_IDLE_FILL` still resolves to `#252527`. This plan deliberately uses the current `SURFACE_PANEL` role for ValueChip and does not reopen or broadly retune the locally accepted Phase 1 controls.
- The legacy `site-tiles` scene currently fails its `libshelf=2,2` expectation because Home-only shelves are excluded and Recently Added is opt-in and off by default. It is not a valid Library shelf capture and is outside this phase's capture gate; no preference or product behavior is changed to manufacture that state. Tracked by #446.
- The sort capture exposed a shared Back/wordmark overlap owned by dispatcher chrome rather than these visual treatments. Tracked separately by #447.
