---
type: planning
entity: implementation-plan
plan: "butaca-mockup-fidelity"
phase: 3
status: draft
created: "2026-11-23"
updated: "2026-11-23"
---

# Implementation Plan: Phase 3 - Detail and Player

> Implements [Phase 3](../phases/phase-3.md) of [butaca-mockup-fidelity](../plan.md)

## Approach

Recompose the existing `DetailScreen` and the immediate-mode Player HUD rather than adding routes, screens, server requests or playback policy. Detail keeps its metadata, stable element identities, scroll flow, native actions and popover track picker while gaining the mockup's bounded backdrop, overlapped poster, editorial copy column, lower track selectors and vertical episodes. The Player keeps its current engine, remote transport, overlays and native-only controls while moving its title and playbar to the mockup's composition and adding real chapter ticks from the playing item.

Consume Phase 1's Display-role text API and shared chrome, and preserve Phase 2's decision that its browsing card metrics do not retune Detail. Do not add Phase 4 preference state, default-track policy, decoder behavior or playback-engine work.

The owner explicitly authorized revising the older HUD transport presentation rule on 2026-11-23. The Player implements the mockup's left-aligned Play/Pause, Audio, Subtitles and Settings controls, retains Stop and Next as native secondary controls, and moves the existing popover anchors with their controls. No playback-engine behavior changes.

## Affected Modules

| Module | Change Type | Description |
|--------|-------------|-------------|
| `rust-modules/src/ui/detail_layout.rs` | modify | Replace the incumbent hero geometry with the 52vh backdrop, 280x420 overlapped poster and measured editorial flow. |
| `rust-modules/src/screens/detail/{mod,hero}.rs` | modify | Recompose backdrop, kicker, display title, metadata, actions and lower track selectors without changing their real commands. |
| `rust-modules/src/screens/detail/{episodes,season,cast,related,extras}.rs` | modify | Build vertical episode rows and place retained shelves in the same right-hand document flow. |
| `rust-modules/src/screens/detail/{tests,geometry_tests}.rs` | modify | Pin the new geometry, navigation, pointer and preserved-action contracts. |
| `rust-modules/src/ui/widgets.rs` | modify | Add an opt-in two-line eyebrow/value presentation to the existing Button for real track selectors. |
| `rust-modules/src/ui/player_hud.rs` | modify | Move title/metadata, derive shared playbar and pointer geometry, draw real chapter ticks and place stand-ins safely. |
| `rust-modules/src/screens/player/mod.rs` | modify | Consume the shared playbar geometry in focus, click and drag paths while preserving HUD state and activation. |
| `rust-modules/src/screens/detail/trailer.rs` | modify | Adapt the existing trailer transport to the shared title/playbar signatures without forking its behavior. |
| `rust-modules/src/ui/up_next.rs` | modify | Derive Up Next geometry from the stand-in slot while retaining its real item, two actions and countdown. |
| `locales/{en,es,be}/browse.json` | modify | Add only missing localized kicker, track-selector and episode/shelf copy. |
| `tests/screenshots/scenes.json` and `tests/fixtures/replay/ALPHABET.json` | modify | Add deterministic Detail, episode, track-picker and Player comparison evidence and any new closed text vocabulary. |
| `docs/DESIGN.md` and `rust-modules/src/ui/CLAUDE.md` | modify | Keep the shared Detail and Player geometry inventory truthful and revise the superseded HUD transport presentation rule. |

## Required Context

| File | Why |
|------|-----|
| `plans/butaca-mockup-fidelity/{plan,phases/phase-3,reviews/plan-review}.md` | Gated scope, exclusions and acceptance criteria. |
| `plans/butaca-mockup-fidelity/implementation/{phase-1-impl,phase-2-impl}.md` | Sequential Display-role, chrome, scoped-card, capture and replay contracts to consume rather than recreate. |
| Supplied `butaca-rediseno-mockup.zip` | Sole visual reference. `index.html`'s `.detail-*`, `.ep-*`, `.track`, `.osd-*`, `.seek`, `.skip-intro` and `.upnext` rules plus `renderDetail`/`renderPlayer` define the target composition. |
| `rust-modules/src/ui/CLAUDE.md` and `rust-modules/src/screens/CLAUDE.md` | Shared tokens, text roles, focus ownership, popover, card-mark and layer invariants, including the transport presentation verdict revised by the owner. |
| `rust-modules/src/player/CLAUDE.md` | Playback-engine boundaries, client-rendered subtitle identity and device-verification limits. |
| `docs/DESIGN.md` | Fixed canvas, safe margins, couch-legibility floor and shared component contracts. |
| `rust-modules/src/ui/detail_layout.rs` | Current 540x810 poster, copy-column constants, measured `HeroChain` and scrim tests. |
| `rust-modules/src/screens/detail/mod.rs` | Backdrop/hero painting, `ScrollColumn` flow, section order, focus/place/seat logic, `StartTracks` and all real action requests. |
| `rust-modules/src/screens/detail/hero.rs` | Stable `HeroCtl` ids, conditional action set, order, widths and real track/trailer helpers. |
| `rust-modules/src/screens/detail/episodes.rs` | Existing Still-play/Text-detail identities, episode facts, progress/watched painting and filmstrip geometry. |
| `rust-modules/src/screens/detail/{season,cast,related,extras}.rs` | Existing section identities, shelf geometry, headings and retained destinations. |
| `rust-modules/src/screens/track_pick.rs` | Compact `TableView` popover, `TrackPickArg.host`, one-sided commit messages and Off behavior. |
| `rust-modules/src/ui/track_facts.rs` | Real language, codec, channel and subtitle labels to reuse without placeholder values. |
| `rust-modules/src/route/plan.rs` | `StartTracks`' three subtitle states and explicit-item-choice contract. |
| `rust-modules/src/ui/player_hud.rs` | Current title, disc/tab/playbar geometry, transport state glyph, pointer helpers, subtitle lift and no-chapter-tick implementation. |
| `rust-modules/src/screens/player/mod.rs` | `PlayerScreen` HUD state, `Focusable` groups, stop z-order, click/drag seek behavior and subtitle composition. |
| `rust-modules/src/screens/player/skip_pill.rs` and `rust-modules/src/ui/up_next.rs` | `ControlSlot` stand-ins, real marker/next-episode data, focus precedence and countdown behavior. |
| `rust-modules/src/ui/{track_menu,more_menu,chapters_panel}.rs` and `screens/player/overlay.rs` | Real audio, subtitle, quality and chapter overlays whose behavior and playing-item identity must survive. |
| `rust-modules/src/screens/detail/trailer.rs` | The second consumer of shared `draw_title` and `draw_playbar`, including its no-chapter/no-pointer case. |
| `tests/screenshots/scenes.json` and `rust-modules/src/dev/scenarios.rs` | Existing deterministic Detail/Player scenes and real-input `ContentBoot` paths. |
| `tests/replay_fixtures.py` and `tests/fixtures/replay/README.md` | Supported shape-rebase and measurement-fill migration, with no behavior re-recording by default. |

## Implementation Steps

### Step 1: Pin the new composition and preserved behavior with failing regressions

- **What**: Add Detail tests for the 52vh backdrop, 280x420 overlapped poster, kicker/display-title flow, left action order, lower track-selector geometry and real values, vertical episode rows and their two independent actions, Cast/Related order and the complete right-hand scroll flow. Add movie and show-play cases proving explicit audio, subtitle and explicit Off ids reach `ContentReq::Play`, plus popover anchor and one-sided commit cases. Add Player tests for top title/metadata, the left PlayPause/Audio/Subtitles/More order, retained Stop/Next secondary actions, popover anchors, fixed time-label boxes, chapter positions, focused knob geometry, pointer-to-fraction inversion, subtitle clearance and Skip/Up Next safe frames. Keep `DiscAction` semantics, remote transport, overlay precedence, failure read-outs and existing activation tests as anchors.
- **Where**: `ui/detail_layout.rs` tests, `screens/detail/{tests,geometry_tests}.rs`, `screens/track_pick.rs` tests, `ui/player_hud.rs` tests, `ui/up_next.rs` tests and `screens/player/mod.rs` tests.
- **Authorized By**: Phase 3 Includes, Deliverables and Acceptance Criteria 1-4, plus the repository invariant that a reported mismatch starts with a failing regression.
- **Why**: Current tests pin a full-height atmospheric hero, a 540x810 poster, horizontal episode filmstrips, moving elapsed time and an empty chapter rail. They must prove the gated replacement before implementation while existing action and playback tests prevent a visual rewrite from changing product behavior.
- **Considerations**: Assert real `Detail`, `Episode`, `Stream` and `Chapter` values, including missing metadata, never mockup placeholder media, scores, chapters or warm colors. Test the authorized transport composition without changing what any action does.

### Step 2: Rebuild Detail's editorial hero on the existing measured flow

- **What**: Replace the hero constants with a 52vh backdrop band, a body origin 190px above its foot, a 280x420 poster and a 48px column gap inside the existing safe margins. Set the copy origin at `MARGIN_X + 280 + 48` and derive its width to `SCR_W - MARGIN_X`. Start the information column 150px below the body origin. Add the real item-kind/first-genre kicker, switch only the title's `Label` to Phase 1's Display-role builder at `size::DISPLAY`, retain the real metadata badges and place genre/studio chips in their own measured row before the synopsis. Crop backdrop art and its top/left/bottom scrims to the bounded band and re-grade the existing contrast anchors. Make the action row left-aligned at the copy origin, ordered Play, Favorite, watched toggle, then native Restart, Trailer, alternate versions and Artwork. Offer Trailer only when `hero::trailer_play` returns a playable extra.
- **Where**: `ui/detail_layout.rs`, `screens/detail/mod.rs::{draw_backdrop,draw_hero,draw_identity_line,draw_buttons,hero_set}` and `screens/detail/hero.rs::{hero_ctls,hero_btn_rect_at}`.
- **Authorized By**: Phase 3 Includes "Detail's 52vh backdrop, overlapped 280x420 poster, scoped display title, kicker, action order" and Acceptance Criterion 1.
- **Why**: The current full-screen art, oversized poster and right-justified action row cannot express the reference by color or spacing tweaks. `HeroChain`, `hero_ctls` and the existing art identity already supply one layout and real actions, so no parallel hero is needed.
- **Considerations**: Preserve the Jellyfin palette, artwork-derived atmosphere, poster fallback, Restart/Trailer/alternate-version/artwork commands, trailer-preview fades and native Back/chrome from Phase 1. Keep the measured cap-band flow rather than absolute per-row text baselines.

### Step 3: Move audio and subtitles into real lower selectors without changing their choice semantics

- **What**: Remove Audio and Subtitles from the ordinary action list and add a two-button track row below it, using the existing `ELEM_AUDIO`/`ELEM_SUBTITLES` identities in a separate `TRACK_GROUP`. Extend `widgets::Button` with an opt-in eyebrow/value layout for the mockup's two-line selector, then draw real values through `ui::track_facts::{row_label,sub_row_label}`: explicit page choices first, otherwise the server-selected or file-default audio fact, the selected subtitle or the existing Off label when no subtitle is selected. Move the retained people/credit line below the track row. Anchor `TrackPickArg` to the new selector rect and leave its compact `TableView`, host delivery and one-sided commit unchanged. Carry the page's `audio_sid`/`sub_sid` into `play_episode_value` through the same `StartTracks` construction already used by movie Play, preserving `-1` as no pick and `0` as explicit Off.
- **Where**: `screens/detail/{mod,hero}.rs`, `ui/widgets.rs`, `screens/track_pick.rs` and `screens/detail/tests.rs`.
- **Authorized By**: Phase 3 Includes "lower track selectors", Acceptance Criterion 2, and the exclusion of Phase 4 preference persistence/default policy.
- **Why**: The existing discs reach the real picker, but they occupy the wrong composition and `play_episode_value` currently replaces page choices with `StartTracks::NONE`. The existing ids, track facts and host-targeted popover already provide every required behavior. A two-line Button variant is necessary because the current shared Button cannot represent the selector's eyebrow plus value.
- **Considerations**: Update `groups`, `neighbour`, `place`, `seat`, `reconcile` and pointer stops together so actions move into tracks and tracks into the next section without focus loss. Do not add persisted preferences, alter `TrackPickArg.chosen`, change `StartTracks::NONE` or infer tracks from mockup strings.

### Step 4: Replace the episode filmstrip with vertical rows and retain the native shelves

- **What**: In `episodes.rs`, derive an 860px-maximum vertical list with 12px row gaps, 14px padding and a 230x129 16:9 still. Put episode number, title, two-line summary and real duration in the row, with existing watched/progress treatment. Keep `Row::Still` as Play and `Row::Text` as OpenDetail at their stable ids, but calculate `still_rect`, `meta_rect`, `row_rect` and `block_h` vertically and cull off-screen rows with `on_axis`. Change episode navigation so UP/DOWN walks episodes and LEFT/RIGHT chooses the play versus detail target, with matching seat/restore/pointer geometry. Add the real season/episode-count heading. Start every below-hero section at the editorial copy origin, order Season, Episodes, Cast, Related, then native Extras, and give Cast and Related their localized shared-pitch headings. Retire the now-unused horizontal episode scroll only with its call sites.
- **Where**: `screens/detail/{episodes,season,cast,related,extras,mod}.rs`, `ui/detail_layout.rs`, `screens/detail/{tests,geometry_tests}.rs` and `locales/{en,es,be}/browse.json`.
- **Authorized By**: Phase 3 Includes "Vertical episode rows with independent play/detail actions, plus retained cast and related shelves" and Acceptance Criteria 1 and 3.
- **Why**: The current two-target identity model already supplies the required independent commands, but its horizontal strip, full-width sections and headingless Cast cannot form the mockup's document. Reusing `SectionId`, `ScrollColumn` and existing shelf widgets keeps restoration and activation coherent.
- **Considerations**: Preserve season loading, stable `SectionId`/`Spot` restoration, Related/Person/Extras destinations, resume bars and watched marks. Keep Detail on its existing card styles, since Phase 2 deliberately scoped `BROWSE_STYLE` away from Detail. Compute `content_top` after the poster, track row and retained people/credit line rather than introducing a second document.

### Step 5: Align the shared Player title and playbar while preserving transport input

- **What**: Move `draw_title` to the top safe row, with the real show/movie title leading and its episode/context metadata following on the same cap band at `size::CAPTION`. Use the named HUD title role at the unobtrusive `size::HEADLINE` scale and add the mockup's top scrim through existing theme roles. Pass `Measure` through both Player and trailer call sites. Replace playbar offsets with one measured geometry derived from duration: fixed elapsed at `MARGIN_X`, fixed negative remaining at `SCR_W - MARGIN_X`, and a bottom track between their measured boxes with an 18px gap. Keep the transport state glyph beside elapsed and the existing Focused/Held/Tick knob states. Add small chapter ticks from `metadata::playing_chapters()`, using `start_ms` against the same duration snapshot and passing an empty slice for a trailer. Use that same geometry in `draw_playbar`, `scrub_hit_rect`, `scrub_frac_x` and `PlayerScreen::pointed_ns`, and recompute the subtitle ceiling from the resulting bottom-HUD top.
- **Where**: `ui/player_hud.rs`, `screens/player/mod.rs` and `screens/detail/trailer.rs`.
- **Authorized By**: Phase 3 Includes "Player title/metadata placement" and "Bottom playbar with fixed time labels, focused knob and real chapter markers", plus Acceptance Criterion 3.
- **Why**: Current title ink sits in the bottom control band and elapsed time follows the knob, while the rail deliberately draws no chapters. One shared geometry is required because drawing and pointer mapping currently export separate helpers, and using `playing_chapters()` avoids reading a show container instead of the playing episode.
- **Considerations**: Preserve remote LEFT/RIGHT scrub acceleration, tap commit, pointer drag/release, seek preview, one transport snapshot and all playback-engine behavior. Do not create chapter buttons or marker-band stops. Keep text and bitmap subtitles clear of the new controls and validate long durations and the no-chapter trailer case.

### Step 6: Place Skip Intro, Up Next and retained native tabs in non-colliding native additions

- **What**: Decouple the stand-in slot from the ordinary disc floor. Right-align Skip Intro at the safe right edge with its bottom 170px above the screen foot and a 400px minimum slot, preserving its real marker action and focus precedence. Derive Up Next's existing still, caption and Watch Credits/Next Episode buttons from that same slot without changing its queue, countdown or cancellation rules. Move the native Stop/Next and Info/Chapters controls to a measured top-right secondary row beside the title block so they remain available without occupying the mockup's bottom control/playbar band, and elide title/context before either can cross those controls. Keep their existing transport and overlay requests and panel behavior. Update all related focus extents, pointer stops, overscan rects and subtitle-clearance tests.
- **Where**: `ui/player_hud.rs`, `ui/up_next.rs`, `screens/player/skip_pill.rs` and `screens/player/mod.rs`.
- **Authorized By**: Phase 3 Includes "retained secondary native controls" and "safe Skip Intro and Up Next placement", plus Acceptance Criterion 3.
- **Why**: `ctrl_slot` currently inherits its width and right edge from the ordinary discs, and the bottom tabs compete for the same band. The mockup gives the marker offers a distinct right-side location, while the native tabs need a deliberate addition rather than disappearance.
- **Considerations**: Preserve `ControlSlot` precedence, stable `ELEM_*` identities, `ControlSlot::item_rect`, real chapter/track/quality overlays and countdown focus semantics. Preserve every `DiscAction` and overlay behavior while the primary and secondary anchors change under the authorized composition.

### Step 7: Implement the authorized mockup transport row and revise the superseded prose rule

- **What**: Place four primary left-aligned controls in the mockup order PlayPause, Audio, Subtitles and More/Settings. Keep Stop and Next in the secondary row from Step 6. Preserve every existing `DiscAction` and activation, point the audio, subtitle and More/quality popovers at the new control rectangles, and update `BTN_N`, `disc_action`, `btn_x`, focus order, pointer stops and overscan geometry together. Revise `ui/CLAUDE.md` Product idiom 3 to permit this mockup-derived transport presentation while retaining the existing remote-key requirement and the rule against additional transport glyphs.
- **Where**: `ui/player_hud.rs`, `screens/player/mod.rs`, `ui/{track_menu,more_menu}.rs`, `rust-modules/src/ui/CLAUDE.md` and Player regressions.
- **Authorized By**: The owner's explicit 2026-11-23 decision to include the mockup's Play/Pause control and revise the older HUD restriction, plus Phase 3 Acceptance Criterion 3.
- **Why**: The four primary controls are a visible part of the supplied mockup. Reusing the existing actions provides that composition without changing playback behavior or losing native Stop/Next/quality capabilities.
- **Considerations**: More/Settings continues to open the real quality/options menu. No new transport command, decoder behavior, placeholder toast or real-account playback is introduced.

### Step 8: Add deterministic comparison evidence and complete supported replay migration

- **What**: Keep `detail` at 1920x1080 and add `detail-episodes` for `@caminandes`, `detail-track-picker` using the real `ContentBoot` DOWN/OK path, and `player-hud` using the existing clock-sink/autopause mechanics without `simvideo`. Add a full-frame 1920x1080 output to the existing `site-up-next` scene while retaining its website crop. Update scene expectations to the actual new states, add any new literals to `ALPHABET.json`, rebase replay shape only after an actual shape refusal and fill new Display-role/layout measurements. Update `docs/DESIGN.md` only where the implemented Detail and Player contracts changed.
- **Where**: `tests/screenshots/scenes.json`, `tests/fixtures/replay/ALPHABET.json`, replay fixtures and `docs/DESIGN.md`.
- **Authorized By**: Phase 3 Includes regression/capture evidence, Deliverable 3, Acceptance Criterion 4 and the plan's supported replay policy.
- **Why**: Existing scenes do not expose vertical episodes or the lower picker, and the ordinary Player scene's `simvideo` dependency has a known excluded host failure. Existing boot walks and clock-sink scenes provide deterministic visual evidence without new capture infrastructure or real-account writes.
- **Considerations**: Never re-record behavior for this visual phase, preserve unrelated fixture bytes/tolerance regions and do not investigate the excluded `simvideo` first-frame failure. Include the authorized four-control transport composition in the deterministic HUD and final television comparisons.

## Testing Plan

Run this once on the committed, clean tree after any required shape/measurement migration, then restore `make check` self-test debris and open `ux-detail.jpg`, `ux-detail-episodes.jpg`, `ux-detail-track-picker.jpg`, `ux-player-hud.jpg` and `ux-player-up-next.jpg` against the corresponding mockup references.

**Primary Verify Command**:
```sh
make RUST_NIGHTLY=nightly-2026-07-02 check && \
CARGO_INCREMENTAL=0 cargo +nightly-2026-07-02 check --manifest-path rust-modules/Cargo.toml --lib --no-default-features && \
RUSTUP_TOOLCHAIN=nightly-2026-07-02 make screenshots SHOT_SCENES=detail,detail-episodes,detail-track-picker,player-hud,site-up-next SHOT_OUT=/tmp/opencode/butaca-449-phase3 SHOT_CHECK=1 && \
RUSTUP_TOOLCHAIN=nightly-2026-07-02 make check-replay
```

### Additional Checks

- Under the repository TV lock, use the established guest/synthetic playback path to capture Detail and Player on the television at 1920x1080. Verify native Display/Interface text, top and bottom HUD composition, real chapter markers, pointer targets, subtitle lift and Skip/Up Next over the video plane. Do not press OK on real catalog items, scrobble, mark watched or change tracks on a real account. Verify the authorized primary and secondary transport controls without changing playback state on a real account.

## Reality Check

- **Resolved on 2026-11-23**: The owner explicitly authorized revising `ui/CLAUDE.md` Product idiom 3 for the supplied mockup. Step 7 implements PlayPause, Audio, Subtitles and More/Settings on the left, retains Stop and Next as secondary native controls and moves every popover anchor with its control. Remote transport and all existing action semantics remain intact.
- **Non-blocking**: `ui/player_hud.rs` currently says chapter ticks were removed and must not be re-added. Phase 3 explicitly authorizes real chapter markers, so Step 5 supersedes that local presentation rule only for small ticks from `playing_chapters()`. It does not authorize intro/credits bands or new chapter buttons.
- **Non-blocking**: `play_episode_value` currently discards the Detail page's explicit track choices with `StartTracks::NONE`, although movie Play preserves them. Step 3 fixes only that existing per-item flow. It does not add Phase 4 persistence or default preference precedence.
- **Non-blocking**: The existing `player` screenshot depends on `simvideo: first picture decoded`, whose host failure is expressly out of scope. The new `player-hud` scene is for deterministic HUD layout only and is not presented as video-plane proof. Real video composition remains a television check.
- **Non-blocking**: Phase 1 owns Display-role text and shared chrome, and Phase 2 owns browsing-only card metrics. Phase 3 starts only after those interfaces land and must not backport or redesign them here.
