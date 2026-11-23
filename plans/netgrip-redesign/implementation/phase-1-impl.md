---
type: planning
entity: implementation-plan
plan: "NetGrip visual redesign"
phase: 1
status: completed
created: "2026-11-22"
updated: "2026-11-22"
---

# Implementation Plan: Phase 1 - Shared language, Home and Detail

> Implements [Phase 1](../phases/phase-1.md) of [NetGrip visual redesign](../plan.md)

## Approach

Retune the existing two-layer theme and component state derivations, then compose Home and Detail from those same roles. Use the supplied NetGrip graphite stops `#0A0B0D`, `#121316`, `#17191D` and signal amber `#FF9E40`, while retaining Butaca's warm-neutral reading hierarchy and existing type scale. Reuse `AmbientWash`, `ControlPalette`, `Button`, `CircleButton`, `TabStrip`, `PageDots`, `keyline_chip` and `CardRow`. Add no screen, font, shader, navigation model or parallel visual primitive.

Keep every laid-out rect, focus identity, route, content string, animation spring and playback action intact. Visual changes may reach other surfaces through shared roles, but Phase 1 makes no direct Library, Settings or Player edits. Their existing behavior and compilation remain protected by the host suite. The local simulator and committed mock Jellyfin library are the only visual deployment and content source.

## Affected Modules

| Module | Change Type | Description |
|--------|-------------|-------------|
| `rust-modules/src/ui/theme.rs` | modify | Add the missing graphite primitives, retune existing surface/accent/state roles and reduce the shared card radius. |
| `rust-modules/src/ui/widgets.rs` | modify | Make lit controls amber on graphite, keep dark ink on amber, translate tab and carousel states, and give focused linked chips a non-color fill change. |
| `rust-modules/src/screens/home/mod.rs` | modify | Use the fixed lit palette for hero actions instead of artwork-derived control hues. Existing hero, grid and wash geometry stays put. |
| `rust-modules/src/screens/detail/mod.rs` | modify | Feed actual focus into linked keyline chips while reusing the shared Detail backdrop, poster, metadata and action composition. |
| `rust-modules/src/screens/detail/episodes.rs` | modify | Adapt the noninteractive episode-rating chip to the explicit focus parameter without changing its face. |
| `rust-modules/src/ui/widgets_control_face_tests.rs` | modify | Pin fixed control colors, dark-on-amber focus, neutral unkeyed idle faces and linked-chip focus. |
| `rust-modules/src/ui/widgets_tab_capsule_motion_tests.rs` | modify | Preserve capsule/ink and selection-composite contracts under the amber roles. |
| `rust-modules/src/screens/home/tests.rs` | modify | Regrade the no-artwork fallback as graphite without changing wash behavior. |
| `rust-modules/src/ui/table.rs` | modify | Update only the existing selection-tone test that hard-codes the retired violet. |

## Required Context

| File | Why |
|------|-----|
| `plans/netgrip-redesign/plan.md` | Global visual contract, product invariants and exclusion of TV work. |
| `plans/netgrip-redesign/phases/phase-1.md` | Gated Phase 1 scope, deliverables and acceptance criteria. |
| `PRODUCT.md` | Fixed canvas, remote-first input, artwork priority and preserved product behavior. |
| `docs/DESIGN.md` | Role/primitive split, text floor, component ownership and scrim contracts. |
| `rust-modules/src/ui/CLAUDE.md` | Binding color, text, component, focus, motion and rendering rules. |
| `rust-modules/src/screens/CLAUDE.md` | Screen ownership, focus and landing invariants. |
| `rust-modules/src/ui/theme.rs` | Existing private primitives and all public roles being retuned. |
| `rust-modules/src/ui/widgets.rs` | `ControlPalette`, `ControlStyle::face`, `Button`, `CircleButton`, `TabPill`, `TabStrip`, `StripRender`, `PageDots`, `keyline_chip` and `draw_back_control_at`. |
| `rust-modules/src/ui/card_row.rs` | `RowStyle`, shared tile radius, headings, captions, focus pop and shelf geometry that must not drift. |
| `rust-modules/src/ui/landing_hero.rs` | Home's measured text stack and atmospheric curve. |
| `rust-modules/src/ui/detail_layout.rs` | Detail's poster/copy geometry, `HeroChain` and contrast inputs. |
| `rust-modules/src/screens/home/mod.rs` and `rust-modules/src/screens/home/tests.rs` | Home composition and existing geometry, navigation, hit and wash regressions. |
| `rust-modules/src/screens/detail/mod.rs` and `rust-modules/src/screens/detail/hero.rs` | Detail composition, fixed action identities and measured action geometry. |
| `rust-modules/src/screens/detail/geometry_tests.rs`, `tests.rs` and `identity_tests.rs` | Existing layout, focus, activation and content-preservation evidence. |
| `rust-modules/src/ui/widgets_hero_scrim_tests.rs` and `widgets_ambient_ground_tests.rs` | Existing legibility floors that the new roles must continue to satisfy. |
| `tests/screenshots/scenes.json` | Existing `home`, `home-shelves` and `detail` mock-backed capture states. |
| `.agents/skills/ui-sim/SKILL.md` | Full-resolution capture and FIFO driving rules for this PC-only iteration. |

## Implementation Steps

### Step 1: Add failing visual-state regressions before retuning production code

- **What**: Extend `widgets_control_face_tests.rs` so a lit keyed control has flat `ACCENT` focus, `CONTROL_IDLE_FILL` rest and `ACCENT_INK` on focus, an unkeyed control keeps the same focus meaning and returns to `CONTROL_IDLE_FILL_UNKEYED` at rest. Add a pure `keyline_chip` face test proving that focus adds an amber fill and dark ink rather than only changing ink hue. In `widgets_tab_capsule_motion_tests.rs`, pin quiet amber selection, full amber focus, dark focused ink and the existing mixed-coverage/composite behavior. Rename Home's cool-fallback test to a graphite-fallback test and make it read the new shared graphite roles. Change `ui/table.rs`'s violet-specific test to the confirmed amber tone instead of deleting its coverage.
- **Where**: The listed existing test modules, with no new test file or harness.
- **Authorized By**: Phase 1 Includes "Required theme roles, component treatments and visual-state tests" and Deliverables "Focused host regressions for new derivations", plan Non-Functional requirements on readable text and state distinctions.
- **Why**: Current tests defend violet controls, while no host test covers linked-chip focus. The new and updated assertions must fail against the old behavior before the visual translation can be accepted.
- **Considerations**: Keep existing motion, contrast and geometry assertions. Do not weaken `widgets_hero_scrim_tests.rs`, `widgets_ambient_ground_tests.rs`, Home geometry tests or Detail geometry tests to obtain a pass.

### Step 2: Establish graphite grounds, amber signal and compact corners in `theme.rs`

- **What**: Add private exact `rgb8` graphite primitives for `#0A0B0D`, `#121316` and `#17191D`, and retune the existing amber signal stop to `#FF9E40`. Resolve `SURFACE_APP` to the darkest graphite, `SURFACE_PANEL`/`PANEL_BOT` to the middle stop, and `PANEL_TOP`, `CONTROL_IDLE_FILL`, `CARD_PLACEHOLDER` and `SKELETON_TOP` to the raised stop. Keep `SKELETON_BOT`, `SCRIM_INK`, `ACCENT_INK` and `INK_ON_RESUME` on the darkest stop. Resolve `ACCENT`, `CHROME_LIT` and `KICKER_INK` to the amber signal, derive `CONTROL_SPENT_FILL` from that amber and the app ground, and keep `INK_ON_ACCENT` as the existing alias. Move `TEXT_READING` to the existing warm-white `WARM_0` primitive without merging the role with `TEXT_PRIMARY`, retain the existing warm secondary and tertiary roles and every `theme::size` rung. Replace the blue/violet fallback atmosphere with a four-corner graphite wash using the new stops, and rename the touched `WASH_*`, `AMBIENT_*` and `AmbientWash` constructors from `blue*` to `graphite*` at their existing call sites. Reduce `CARD_RING_RAD` from 14 to 8 while leaving circular styles and all card rectangles unchanged. Give `TAB_PLATE_IDLE`, `TAB_PLATE_SELECTED` and `TAB_PLATE_SELECTED_OVER` the same amber hue at their current alpha weights.
- **Where**: `rust-modules/src/ui/theme.rs`, plus mechanical constructor renames in the existing `AmbientWash` callers and tests.
- **Authorized By**: Phase 1 Objective and Includes "Required theme roles", plan Direction contract "graphite grounds, amber signal and focus, warm-neutral reading ink, compact keylines, smaller radii", plan Guiding Decisions requiring existing theme and component systems.
- **Why**: These existing roles already reach Home, Detail and the shared components. Retargeting them creates one visual world without screen-local literals or duplicate primitives. The tab plates keep their current alpha arithmetic, so selection remains quieter than focus.
- **Considerations**: Add no raw color outside `theme.rs`. Reuse existing alpha values, scrim curves, `space` rungs, text rungs and logo geometry. Update comments touched by the rename so they no longer claim a blue or violet world.

### Step 3: Translate shared control, tab, chip and carousel states in `widgets.rs`

- **What**: Make `ControlPalette::lit()` return fixed `ACCENT` focus stops, `CONTROL_IDLE_FILL` idle and `CONTROL_SPENT_FILL` spent values rather than sampling a violet hue. In `ControlStyle::Accent`, use `ACCENT_INK` for keyed focus and restore the neutral `CONTROL_IDLE_FILL_UNKEYED` film for unkeyed idle. Keep `ControlGround`, focus pop, cast, rim and button sizing intact. Make `draw_back_control_at` use dark ink on its focused amber custom face. Let `TabStrip` draw quiet amber selection from `TAB_PLATE_SELECTED*` and full amber focus from `CHROME_LIT`, with `TabPill::mixed_ink` retaining dark ink under focus. Extend `keyline_chip` with a `focused` input and a pure face derivation: idle keeps its hollow ring and caller ink, focused fills the same measured rect with `ACCENT` and uses `ACCENT_INK`. Use `CHROME_LIT` for the active `PageDots` mark and retain the dim inactive marks.
- **Where**: Existing symbols in `rust-modules/src/ui/widgets.rs`, update the four current `keyline_chip` call sites in Detail and `screens/detail/episodes.rs` to pass the new state explicitly.
- **Authorized By**: Phase 1 Includes "component treatments" and Acceptance Criteria on readable focus and primary actions, plan Non-Functional rule that color cannot be the only state distinction.
- **Why**: The current keyed focus face is violet with white ink, Home can derive a different hue from artwork, and linked genre/studio chips currently change only ink color. These direct component changes provide one legible amber focus contract without moving any control.
- **Considerations**: Do not fork `Button`, `TabPill`, `TabStrip` or `keyline_chip`. Do not alter the shader-baked card glow, capsule outlines, disc diameters, tab springs or `SelMark::Lands`/`Travels` behavior. Noninteractive episode and accessibility chips remain hollow and unfilled.

### Step 4: Integrate Home through its existing composition

- **What**: In `HomeScreen::draw_hero_actions`, replace `sample_control_ground(...).map(ControlPalette::ambient)` with `ControlPalette::lit()` and remove the now-unused read gate and `page_alpha` parameter from that helper and its calls. Leave `hero_content`, `hero_text_metrics`, `hero_row_y`, `HeroLogo`, `PageDots` placement, `Grid`, `CardRow`, backdrop layer alphas and carousel timing unchanged. The Step 2 wash and Step 3 component changes supply the new ground, navigation, action and next-row treatment.
- **Where**: `rust-modules/src/screens/home/mod.rs`, using existing `ui/landing_hero.rs` and `ui/card_row.rs` unchanged.
- **Authorized By**: Phase 1 Includes "Home navigation, hero, action row, carousel and next-content row", Acceptance Criteria requiring preserved navigation/content behavior, plan Direction contract requiring a full-width backdrop, flat navigation with amber state and the next content row visible.
- **Why**: Fixed lit controls are required for a consistent amber focus instead of a poster-dependent hue. Removing this one newly unnecessary sample also avoids paying for a readback whose answer no longer affects the controls. No layout edit is needed because Home already has the gated composition.
- **Considerations**: Preserve stable item identities, remote/pointer geometry, focused-last card drawing, hero auto-rotation and present-gate behavior. Do not uppercase or rewrite server/app content and do not add a Home kicker or new control.

### Step 5: Integrate Detail without reopening its settled layout or actions

- **What**: Keep `draw_backdrop`, the 540x810 poster, plain bold title, identity/rating/synopsis/people chain, action identities/order, fixed disc verb slot and below-hero `ScrollColumn` flow intact. Reuse the existing `ControlPalette::lit()` action row and shared Back control after Step 3. In `draw_identity_line`, pass actual genre/studio focus to `keyline_chip` and stop using `CHROME_LIT` as a color-only focus substitute. Leave noninteractive resolution, CC, SDH and AD badges on their existing components and leave season-tab motion on `SelMark::Lands`.
- **Where**: `rust-modules/src/screens/detail/mod.rs`, read but do not redesign `screens/detail/hero.rs`, `ui/detail_layout.rs`, `screens/detail/season.rs` or the below-hero section modules.
- **Authorized By**: Phase 1 Includes "Detail backdrop, poster, metadata, actions and below-hero continuity", Acceptance Criteria on readable focus, primary actions and unchanged behavior, `ui/CLAUDE.md`'s settled Detail flow and owner constraints.
- **Why**: Detail already composes the required shared pieces. Only the linked-chip focus input needs screen data, every other visual change arrives through the established roles and widgets.
- **Considerations**: Do not change the poster's size/alignment, plain-text title, five-line synopsis/MORE behavior, trailer-preview fades, track choices, watch/favorite/artwork actions or any playback/server path. Existing Detail geometry and identity suites remain the behavioral guard.

### Step 6: Capture and review the two reference surfaces on this PC

- **What**: Run the primary verify command below, then open the generated `ux-home-hero.jpg`, `ux-home-shelves.jpg` and `ux-detail.jpg` at their native 1920x1080 size. Review graphite grounds, warm-neutral text, amber selection/focus, dark focused ink, artwork priority, ordinary and focused actions, the Home carousel and next-content row, and Detail's poster/meta/actions/below-hero continuity. Use the existing simulator FIFO against `tests/mock_jf.py --catalog` to add one Home top-strip focus capture and one Detail non-primary-action or linked-genre focus capture, then open those shots as well. Iterate only on the tokens/components above if the captures expose a defect.
- **Where**: Local `make screenshots` output under `/tmp/opencode/netgrip-phase-1` and the existing `ui-sim` workflow. No production server, real account or television.
- **Authorized By**: Phase 1 Includes and Deliverables requiring full-resolution ordinary/focused simulator captures reviewed by the agent, plan Testing Strategy requiring captures against the committed mock library.
- **Why**: Host tests cannot see pixels. The visual phase is incomplete until the actual Home and Detail frames have been opened and judged at the authored canvas size.
- **Considerations**: Use only the mock library. Do not claim LG text rasterization, video-plane compositing, native performance or TV validation from these captures. Phase 3 owns final documentation, replay/release gates and publication.

## Testing Plan

**Primary Verify Command**: `RUSTUP_TOOLCHAIN=nightly-2026-07-02 make RUST_NIGHTLY=nightly-2026-07-02 check && RUSTUP_TOOLCHAIN=nightly-2026-07-02 make screenshots SHOT_SCENES=ux-home-hero.jpg,ux-home-shelves.jpg,ux-detail.jpg SHOT_OUT=/tmp/opencode/netgrip-phase-1`

The first leg runs lint, localization, the default and hostsim suites and the repository's other host gates, including the new state tests and existing navigation/geometry/contrast regressions. The second builds the capture simulator, uses the committed mock library and produces the three full-resolution reference states for the required visual review. Record actual results and opened-capture findings rather than treating command success alone as visual acceptance.

## Reality Check

- **Blocking only for literal mono-numeral work**: the direction contract mentions mono numerals, but `rust-modules/src/text.rs` currently has Inter regular/bold plus fallback faces and no font-family axis. This plan deliberately adds no font asset, text cache key, measurement axis or packaging work. If monospaced numerals are mandatory for #444 rather than a later device-validated typography item, the Maintainer must explicitly gate that expansion before any dependent implementation. The graphite/amber and existing-geometry work above does not depend on it.
