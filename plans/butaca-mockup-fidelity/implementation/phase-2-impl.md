---
type: planning
entity: implementation-plan
plan: "butaca-mockup-fidelity"
phase: 2
status: draft
created: "2026-11-23"
updated: "2026-11-23"
---

# Implementation Plan: Phase 2 - Home, Library and Search

> Implements [Phase 2](../phases/phase-2.md) of [butaca-mockup-fidelity](../plan.md)

## Approach

Reuse the owned screens, `CardRow`/`poster_grid` geometry, Jellyfin's existing listing payload and the current Play, Detail, query-edit and menu actions. Add the mockup's missing composition where the current structures cannot express it: a measured Home hero, panoramic Continue Watching cards, a grid-first Library header and a centered Search column.

Scope all new poster geometry to Home, Library and Search. Do not retune `RowStyle::HOME`, `RowStyle::EPISODE` or the existing `poster_grid::COLS` path shared with Detail, Person and Collection. Consume Phase 1's display-role text API and chrome without changing either. Do not add routes, requests, fonts, preferences, Detail or Player work.

## Affected Modules

| Module | Change Type | Description |
|--------|-------------|-------------|
| `rust-modules/src/catalog.rs` and `catalog/{initial,record}.rs` | modify | Carry optional real favorite, community-rating and video-size facts through the complete catalog-row codec. |
| `rust-modules/src/jellyfin/convert.rs` | modify | Convert fields already present in `BaseItemDto` without changing server requests. |
| `rust-modules/src/ui/{poster_grid,card_row,widgets,hero_logo,landing_hero,value_chip}.rs` | modify | Add scoped browsing geometry, collision-safe card badges, a display-capable Home logo fallback and kicker-aware hero flow. |
| `rust-modules/src/screens/home/{mod,tests}.rs` | modify | Implement the complete hero and panoramic Continue Watching composition while preserving activation. |
| `rust-modules/src/screens/library/{mod,layout,draw,toolbar,parts}.rs` and tests | modify | Open on title/toolbar/grid, move shelves after it and split unwatched from genre. |
| `rust-modules/src/screens/library/menu.rs` | modify | Reuse the existing Genre menu directly while leaving Sort and Type behavior intact. |
| `rust-modules/src/screens/search/{layout,render}.rs` and tests | modify | Center the field and result document, apply browsing cards and retain native result kinds and recents. |
| `locales/{en,es,be}/browse.json` | modify | Add localized hero kicker, New badge and Search discovery copy. |
| `tests/screenshots/scenes.json` and `tests/fixtures/replay/ALPHABET.json` | modify | Update the existing comparison-scene descriptions and closed text vocabulary. |
| `docs/DESIGN.md` | modify | Keep the shared card/grid inventory truthful about the scoped styles and grid-first Library. |

## Required Context

| File | Why |
|------|-----|
| `plans/butaca-mockup-fidelity/{plan,phases/phase-2,reviews/plan-review}.md` | Gated scope, preserved behavior and acceptance criteria. |
| `plans/butaca-mockup-fidelity/implementation/phase-1-impl.md` | Cross-phase display-role, chrome, capture and replay contracts to consume rather than recreate. |
| Supplied `butaca-rediseno-mockup.zip` | Sole binding visual source. `renderHome`, `renderGridScreen`, `renderSearch`, `cardHTML`, `metaHTML` and their CSS define the target composition. |
| `rust-modules/src/ui/CLAUDE.md`, `screens/CLAUDE.md` and `server/CLAUDE.md` | Shared-widget, layer, focus, capture and canonical-state invariants. |
| `docs/DESIGN.md` | Fixed canvas, palette, legibility, scrim and current card contracts. |
| `rust-modules/src/catalog.rs` and `catalog/{initial,record}.rs` | `CatalogMovie`, its additive serde compatibility and exhaustive canonical projection. |
| `rust-modules/src/jellyfin/{dto,convert,client}.rs` | Existing DTO facts, conversion boundary and proof that listing requests already carry every needed field. |
| `rust-modules/src/ui/{poster_grid,card_row,widgets,hero_logo,landing_hero,value_chip}.rs` | Existing geometry, watched/resume marks, badge primitives, clearLogo fallback and hero stack. |
| `rust-modules/src/ui/fmt.rs` | Existing honest resolution formatter, including absent-video behavior. |
| `rust-modules/src/screens/home/{mod,tests}.rs` | Current hero flow, stable action IDs, row styles, activation and geometry tests. |
| `rust-modules/src/screens/library/{mod,layout,draw,toolbar,parts,menu}.rs` and tests | Current document order, real section titles/counts, query edits, popovers and focus restoration. |
| `rust-modules/src/screens/search/{mod,layout,render,tests}.rs` | Current editing, recents, server-result projection, centered-field drawing and keyboard geometry. |
| `rust-modules/src/dev/scenarios/screenshot.rs` and `tests/screenshots/scenes.json` | Existing deterministic Home, Library, Sort and Search capture paths. |
| `tests/replay_fixtures.py` and `tests/fixtures/replay/README.md` | Supported shape-rebase and measurement-fill migration, with no behavior re-recording by default. |

## Implementation Steps

### Step 1: Pin the new composition and preserved behavior with failing regressions

- **What**: Add DTO conversion cases for present and absent favorite/rating/video dimensions, exhaustive catalog-codec coverage, eight-column band arithmetic, panoramic 16:9 geometry, badge collision cases, kicker-aware hero flow, grid-first Library order, direct unwatched versus Genre activation, and centered Search keyboard/recents cases. Keep Play/Detail requests, resume values, Sort/Type edits, result activation and Back behavior as regression anchors.
- **Where**: `jellyfin/convert.rs`, `catalog/{initial,record}.rs`, `ui/{poster_grid,card_row}.rs`, `screens/home/tests.rs`, `screens/library/*tests.rs`, `screens/search/{layout,tests}.rs` and `screens/search/render.rs` tests.
- **Authorized By**: Phase 2 Includes, Deliverables and Acceptance Criteria 1-5, plus the repository invariant that a reported mismatch starts with a failing regression.
- **Why**: Current tests pin a poster-only Home deck, shelves before the Library grid, a combined Filter menu and a 1200px Search field. They must prove the gated replacement before implementation, while existing activation and query tests prevent a visual rewrite from changing product behavior.
- **Considerations**: Assert real values and absence, not mockup placeholder scores, resolutions, titles or profiles. Include focused and lifted-card badge paths, restored Library scroll/focus and the raised Search keyboard.

### Step 2: Map the already-delivered Jellyfin metadata through the complete catalog row

- **What**: Add `is_favorite: bool`, `community_rating: Option<f32>`, `video_width: i64` and `video_height: i64` to `CatalogMovie`, each with additive serde defaults. In `movie_from_dto`, map `UserData.IsFavorite`, `CommunityRating` and the first video stream's `Width`/`Height`. Preserve `None`/zero for missing data and for box sets. Append the fields to `catalog::initial::movie`'s canonical census without reordering existing terms and extend the exhaustive `catalog::record` fixture.
- **Where**: `catalog.rs`, `catalog/{initial,record}.rs` and `jellyfin/convert.rs`.
- **Authorized By**: Phase 2 Includes "Real favorite/rating/resolution metadata needed by the mockup, using Jellyfin DTO fields without changing server behavior" and Acceptance Criterion 4.
- **Why**: `BaseItemDto` already declares all four sources, but conversion currently drops them. The row is serialized into controlled initial state, so adding only the UI fields would leave the codec incomplete and older fixtures unable to prove compatibility.
- **Considerations**: Do not edit `dto.rs` or `client.rs`: `LISTING_FIELDS` already includes `MediaSources` and `CommunityRating`, and `UserData` is already converted. Do not infer a score or resolution when absent. `ui::fmt::resolution` will return no badge for two zero dimensions.

### Step 3: Add scoped browsing card styles and shared, collision-safe badges

- **What**: In `poster_grid`, derive `BROWSE_COLS = 8`, `BROWSE_GAP = 22`, `BROWSE_CARD_W`, `BROWSE_CARD_H`, `BROWSE_STYLE` and `BROWSE_ROW_PITCH` from the existing `BAND_W`, keeping the 2:3 poster aspect and exact band closure. In `card_row`, add a 330x186 `CONTINUE` style with 22px gaps and the mockup's 1.07 focus scale. Add `TileBadges` with a `NONE` default and an item-based constructor for favorite plus optional New. Thread it through `draw_tile` and `draw_focused`, passing `NONE` at unchanged callers. Draw New through the existing `widgets::badge`/`BadgeStyle::OverArt`, and a small `widgets::favorite_mark` using `Icon::HeartFill` and existing theme roles.
- **Where**: `ui/{poster_grid,card_row,widgets}.rs` and their geometry/mark tests.
- **Authorized By**: Phase 2 Includes panoramic Continue Watching, safe-area-compatible poster columns and real favorite metadata, plus plan constraints to use shared widgets and preserve native additions.
- **Why**: The current five 300px columns cannot express the mockup's browsing density, and changing the shared constants would also alter excluded surfaces. A scoped style is the smallest way to deliver the new composition without that regression. The badge model is needed because favorite, watched, New and resume are independent facts that share corners on both ordinary and lifted cards.
- **Considerations**: Keep `COLS`, `STYLE`, `ROW_PITCH`, `HOME` and `EPISODE` unchanged for Collection, Detail, Person and episode grids. Place favorite at the top-right, below the watched tick when both exist, and New at the top-left. Never suppress the watched tick or the real resume bar.

### Step 4: Complete Home's measured hero and panoramic Continue Watching row

- **What**: Add a localized kicker above `HeroLogo`, selecting Continue when `resume_frac()` is present and Featured otherwise. Add `HeroLogo::display()` so only Home's textual fallback uses Phase 1's Display role. Include kicker height in `hero_text_metrics` and `landing_hero::stack_bottom`, bound `landing_hero::SYN_W` to the mockup's 540px synopsis column and retain the shared three-line `hero_synopsis` cap. Replace `hero_meta_text` with a measured `HeroMeta` flow composed of `Label`, `Icon::Star` and `widgets::badge`: year/runtime, `OfficialRating`, one-decimal `CommunityRating`, `ui::fmt::resolution` and the existing source annotation, all absent gracefully. Make secondary Info a textual `Button` beside the existing Play/Resume action and right-align `PageDots` at `SCR_W - MARGIN_X - width`. For Continue Watching, select `CONTINUE`, `Art::Still` and `widgets::still_line` with the real title, `focused_caption` time/episode facts, `press_plays = true` and `resume_frac`. Use `BROWSE_STYLE` and `BROWSE_ROW_PITCH` for other shelves and apply New only to recent hubs.
- **Where**: `screens/home/mod.rs`, `ui/{hero_logo,landing_hero,poster_grid,card_row,widgets}.rs` and `locales/{en,es,be}/browse.json`.
- **Authorized By**: Phase 2 Includes Home kicker, bounded synopsis, textual actions, right-aligned indicators, panoramic progress cards and real metadata, plus Acceptance Criterion 1.
- **Why**: `hero_content` currently has no kicker, a single joined meta string, an icon-only Info disc and centered dots. `hero_row_y`, drawing, focus groups and pointer stops already share one flow, so extending that flow preserves remote behavior. `Art::Still` plus `still_line` supplies the existing artwork scrim and play/progress vocabulary needed by the panoramic deck without a new widget.
- **Considerations**: Keep `HERO_PLAY_ELEM`, `HERO_INFO_ELEM`, `HomeReq::{Play,Detail}`, resume computation, hero flipping, native clearLogo art and the existing pager chevron. Do not use mockup color literals, fabricate missing metadata or alter Detail's logo role. Update all Home pitch/placement reads that currently assume `CARD_H` so the smaller posters and wide deck share one geometry.

### Step 5: Recompose Library as title, toolbar and grid before retained shelves

- **What**: Change `Layout` so `grid_block_top()` starts after the optional library selector, the title/toolbar/grid occupies the first content block and shelves begin after the complete grid. Keep `doc_h`, `max_scroll`, `visible_rows`, `row_reveal`, `shelf_reveal`, `seat_for_scroll`, `first()` and `Screen::links` on that same order. In `draw_grid_header`, render the selected `SectionView.row.title` through Phase 1's Display role and its real `row.count_line` as the subtitle, fitted to the existing grid band. Resize `GRID_HEAD_H` for those two one-line bands plus the 52px controls using cap-band/theme spacing, and use the same values in `toolbar_chip_rect`. Apply `BROWSE_STYLE` to the All grid and non-landscape shelves while leaving episode grids alone.
- **Where**: `screens/library/{mod,layout,draw,toolbar,parts}.rs`, layout/navigation/deferred/window tests and `docs/DESIGN.md`.
- **Authorized By**: Phase 2 Includes Library grid-first composition, safe-area-compatible columns, library-specific titles/counts and retained shelves/A-Z access, plus Acceptance Criterion 2.
- **Why**: `Layout` currently places every shelf before the grid and the header says only All. The section directory already owns the real library title and unfiltered count, so no fetch or new model is needed. Reordering the existing pure geometry, rather than drawing a second page, keeps paging, restoration and focus coherent.
- **Considerations**: Preserve the optional library selector as a native addition above the title, shelf publication/staging, the fixed A-Z rail, type-specific empty states and `LibraryWork::Want` grid-local paging. A restore must seat the block visible at its saved scroll, not assume the old shelf-first order.

### Step 6: Split Library unwatched from Genre without changing query behavior

- **What**: Replace the visible combined Filter chip with `UNWATCHED` and `GENRE`, reusing the existing element id 2 for Unwatched and allocating id 8 for Genre. Unwatched toggles `AppMsg::LibraryEdit(QueryEdit::Unwatched(!current))` directly through the existing pending transaction. Genre opens the existing `LibraryMenuKind::Genre` surface at its anchor, retaining `genre_draft`, `LibraryWork::Genres` and `QueryEdit::Genre`. Keep Type and Sort chips and their menu requests unchanged. Give `ValueChip` a `show_icon` toggle so Unwatched has no dropdown affordance and draws `Icon::Check` only while active, without changing its measured width.
- **Where**: `screens/library/{mod,toolbar,menu}.rs`, `ui/value_chip.rs` and selector/menu/navigation tests.
- **Authorized By**: Phase 2 Includes "Separate unwatched and genre controls plus native sort/type menus" and Acceptance Criterion 2.
- **Why**: `QueryEdit` and the Genre surface already implement both operations. Only the visible control-to-action mapping and presentation need to change. `show_icon` avoids treating a direct toggle as a menu while keeping all chips on one measured toolbar.
- **Considerations**: Collections still omit both filter controls. Preserve pending/queued values, focus anchors, Back restoration and filter-aware A-Z behavior. Keep `LibraryMenuKind::Filter` and its canonical discriminant for existing dev/canonical callers, but give it no production toolbar caller. Keep `LibraryCmd::OpenMenu(Filter)` and `libmenu=filter` on the combined surface through the existing menu-request path, independently of the visible Unwatched element.

### Step 7: Center Search and complete its real empty/result composition

- **What**: Set `layout::FIELD` to x=550, y=188, w=820, retaining its 96px height. Add one results-column origin/width pair of 550/820 and use it for headings, recents, Clear, tile styles and empty-state layout. Apply `BROWSE_STYLE` with that origin to Movie, Show and Collection results, retaining native Episode and Person shapes in the same column. Keep `CONTENT_TOP = 300` and `KEYBOARD_H = 324`, and update `render::draw` to use the shared origin rather than `MARGIN_X`. Add localized discovery text explaining the real all-library search, retaining the current live-query no-results text. Apply favorite badges to Search tiles.
- **Where**: `screens/search/{layout,render}.rs` and tests, `ui/card_row.rs` call sites and `locales/{en,es,be}/browse.json`.
- **Authorized By**: Phase 2 Includes Search field and result composition while preserving editing, recents and server search, plus Acceptance Criterion 3.
- **Why**: The current 1200px field and full-width results do not follow the mockup's centered 900px wrapper including padding. Existing `Draft`, `SearchCmd`, `CardRow`, `Resources` and heading counts already provide every behavior and real result fact needed, so geometry and copy are the only necessary changes.
- **Considerations**: Do not change query debounce/minimum, keyboard opening/closing, caret editing, recent selection/clear, result activation or the mock server. Preserve native Episode, Person and Collection results and prove the first result heading and recents remain clear of the raised keyboard. Do not copy the mockup's example searches.

### Step 8: Refresh existing comparison evidence and complete supported replay migration

- **What**: Update the existing `home`, `home-shelves`, `library`, `library-grid`, `library-sort` and `search` scene descriptions to their new target states, keeping their real triggers and deterministic outputs. Add any new localized literals to `ALPHABET.json`. If the canonical additions move the replay census, use the supported shape rebase with reason `browse`, then fill newly referenced display-role and layout metrics. Run strict replay and open all six 1920x1080 captures against the corresponding mockup Home, Movies and Search references. Update `docs/DESIGN.md` only where the scoped card/grid contracts changed.
- **Where**: `tests/screenshots/scenes.json`, `tests/fixtures/replay/ALPHABET.json`, replay fixtures and `docs/DESIGN.md`.
- **Authorized By**: Phase 2 Includes regression tests and reviewed Home, Library, Sort and Search captures, Acceptance Criterion 5, and the plan's supported replay policy.
- **Why**: These scenes already reach every required surface without new capture infrastructure. New text and canonical fields need the repository's explicit replay migration, not edited hashes or an unjustified behavior recording.
- **Considerations**: Rebase only after an actual shape refusal, fill measurements rather than re-recording behavior, and preserve unrelated fixture bytes and tolerance regions. The Sort capture must show the same native sort menu over the new grid, not a replacement menu.

## Testing Plan

Run this once on the committed, clean tree after any required shape/measurement migration, then restore `make check` self-test debris and open `ux-home-hero.jpg`, `ux-home-shelves.jpg`, `library.jpg`, `ux-library-grid.jpg`, `ux-library-sort.jpg` and `ux-search.jpg` for the required before-and-after mockup review.

**Primary Verify Command**:
```sh
make RUST_NIGHTLY=nightly-2026-07-02 check && \
CARGO_INCREMENTAL=0 cargo +nightly-2026-07-02 check --manifest-path rust-modules/Cargo.toml --lib --no-default-features && \
RUSTUP_TOOLCHAIN=nightly-2026-07-02 make screenshots SHOT_SCENES=home,home-shelves,library,library-grid,library-sort,search SHOT_OUT=/tmp/opencode/butaca-449-phase2 SHOT_CHECK=1 && \
RUSTUP_TOOLCHAIN=nightly-2026-07-02 make check-replay
```

## Reality Check

- **Non-blocking**: `jellyfin::dto::BaseItemDto` and `JfClient::LISTING_FIELDS` already supply favorite, community rating and video dimensions. No server query or DTO request-field change is authorized or needed.
- **Non-blocking**: `RowStyle::HOME`, `RowStyle::EPISODE` and `poster_grid::COLS` have consumers outside this phase. Step 3 deliberately adds scoped styles instead of changing those shared values and silently altering Detail, Person or Collection.
- **Non-blocking**: `LibraryMenuKind::Filter` remains for existing dev/canonical compatibility. The visible unwatched control uses the existing query edit directly, and the visible genre control opens `LibraryMenuKind::Genre`, so no canonical-menu migration is needed.
- **Non-blocking**: Phase 1 owns the Display font role and shared chrome. Phase 2 starts only after those interfaces land and must not backport or redesign them here.
