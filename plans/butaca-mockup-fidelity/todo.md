---
type: planning
entity: todo
plan: "butaca-mockup-fidelity"
updated: "2026-11-25"
---

# Todo: butaca-mockup-fidelity

> Tracking [butaca-mockup-fidelity](plan.md)

## Active Phase: 1 - Shared chrome and Settings

### Phase Context

- **Scope**: [Phase 1](phases/phase-1.md)
- **Implementation**: [Phase 1 implementation plan](implementation/phase-1-impl.md)
- **Latest Handover**: None
- **Relevant Docs**: `rust-modules/src/ui/CLAUDE.md`, `rust-modules/src/screens/CLAUDE.md`, `docs/DESIGN.md`

### Pending

- [x] Add failing regressions for textual navigation, real profile access, shared chrome and Settings geometry <!-- completed: 2026-11-23 -->
- [x] Implement shared chrome, typography, real profile/server dialog and anchored Settings pickers <!-- completed: 2026-11-23 -->
- [x] Review all six 1920x1080 comparison captures <!-- completed: 2026-11-23 -->
- [x] Run the final committed-tree gates and publish the Phase 1 PR <!-- completed: 2026-11-23 -->


### In Progress

- [ ] Pixel-fidelity pass on the shared top bar against the reference (geometry, faces, inks) <!-- started: 2026-11-25 -->

### Completed

- [x] Audit all mockup surfaces and map material mismatches to current source <!-- completed: 2026-11-23 -->
- [x] Confirm the four-phase scope including real audio/subtitle preferences <!-- completed: 2026-11-23 -->
- [x] Author and verify the Phase 1 implementation plan against current source <!-- completed: 2026-11-23 -->
- [x] Commit Phase 1 implementation, replay migration and deterministic capture fixes <!-- completed: 2026-11-23 -->

### Blocked

- None

## Changelog

### 2026-11-25

- The shared top bar now names the supplied mockup's own numbers instead of approximating them.
  Measured off the reference's DOM at 1920x1080: `#topbar { padding: 28px 56px 22px }`, avatar 46px
  at (56, 34.5) and FIRST in the left group, the wordmark's two runs at 122 / 192.33, `#mainnav`'s
  content box at 707.84 inside a 59px track on 28, its pills 45 tall at 35 with `padding: 11px 24px`
  around a 17px label, the gear at 1752.02 and the clock right-aligned on 1864.
- Two faces the mockup is set in are now shipped as their own roles, because the ones the app had
  are a weight and an optical size away: Manrope 600 (`#mainnav button`) and Fraunces 700 instanced
  at optical size 30 (the wordmark; the 72pt cut the app shipped is ~10% narrower at 30px). Both are
  drawn UNHINTED, which is what the browser does - light hinting snapped every nav advance to a
  whole pixel and cost +8px across the five labels.
- The bar's material is the reference's warm brown at its .55 with the peach rim, not the solved
  light-lift face; the solve still decides the alpha, with .55 as its floor, so the labels keep the
  app's contrast floor over bright artwork while a dark ground draws the reference's exact weight.
- `the_top_bar_is_the_supplied_mockups_own_geometry` asserts every one of those numbers, so a later
  "small tweak" cannot walk the bar off the reference again.
- Kerning and tabular figures closed the last two measured residuals. FreeType 2.15 answers
  `FT_Get_Kerning` with 0 even for a face carrying valid pairs (measured against DejaVu Sans Bold on
  this host), so the fonts' GPOS pairs are baked into the shipped cuts as legacy `kern` tables and
  `src/kern.rs` applies them to LAYOUT: the five navigation advances land on the reference's DOM
  numbers to 0.03px (`Favoritos` was 1.61px wide before), and `kern`'s test asserts the reference's
  own values. The clock swaps to a Manrope 700 cut with the `tnum` substitutions baked into the
  cmap, so any `HH:MM` measures the reference's 47.98 and the box never breathes.
- Measured against the reference at 1920x1080, the bar's boxes are now exact: avatar
  (56, 34.5, 46, 46), track (700.84, 28, 576.31, 59), the five pills (707.84/805.56/932.66/
  1038.06/1166.16 at y 35, 45 tall) and the gear (1752.02, 34.5, 46, 46) all land within 0.01px.
  What remains is the rasterizer's own floor: SDL_ttf paints the unkerned advance inside the kerned
  box and snaps each glyph to a whole pixel, so a label's INK sits within 3px of the reference
  (all five share its baseline, y 51..64), and the track's alpha is the reference's .55 only over a
  dark ground — over a bright hero the solve raises it to keep the app's contrast floor.

### 2026-11-23

- Plan created with Phase 1 active. Baseline captures collected for Home, Library, Detail and Settings. Player baseline capture is blocked by the unrelated host simvideo failure and will not be treated as redesign work.
- Phase 1 implementation plan completed and regression authoring started.
- Phase 1 code landed and PR #450 opened. Final code commit `efe7afed2` passed 3972 default, 4013 hostsim, 102 storage and 352 harness tests, release and hint checks, six deterministic captures, and 6/6 strict replays. CI on main plus Simulator remains to confirm.
