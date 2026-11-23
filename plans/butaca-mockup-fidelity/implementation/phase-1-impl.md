---
type: planning
entity: implementation-plan
plan: "butaca-mockup-fidelity"
phase: 1
status: completed
created: "2026-11-23"
updated: "2026-11-23"
---

# Implementation Plan: Phase 1 - Shared chrome and Settings

> Implements [Phase 1](../phases/phase-1.md) of [butaca-mockup-fidelity](../plan.md)

## Approach

Reuse the existing chrome projection, `TableView` card grid, account/server loop requests and Settings picker state. Replace only the presentation and integration points that prevent the supplied mockup from being visible and operable: textual destinations, a real avatar and wordmark, chrome on Detail and the Settings surface, a real server dialog, scoped font roles, a fact-based Settings header and anchored pickers.

Keep the Jellyfin palette, native additions and all current destinations and actions. Do not add preferences, server data fetching, a new navigation route or a new picker screen. Keep Settings pickers inside `RootPage`, retain Inter as a packaged fallback and leave Home, Library and Detail bodies unchanged.

## Affected Modules

| Module | Change Type | Description |
|--------|-------------|-------------|
| `rust-modules/src/app/{chrome,bridge,run}.rs` | modify | Publish and paint the complete shared chrome, including surfaces and chrome loop actions. |
| `rust-modules/src/screens/{registry,account_menu}.rs` | modify | Extend the bar-wearing alphabet and build the real profile/server dialog from configured servers. |
| `rust-modules/src/screens/detail/mod.rs` | modify | Activate shared chrome without disturbing Detail actions, trailer behavior or Back. |
| `rust-modules/src/screens/settings.rs` and `screens/settings/page.rs` | modify | Activate chrome from the Settings surface, build the real header and host anchored native pickers. |
| `rust-modules/src/ui/{widgets,dispatch,table,route_screen,table_screen,popover,decision_alert,machine,rec,label,text_view}.rs` | modify | Share geometry, surface focus/stop composition, card metrics, display text and role-aware replay metrics. |
| `rust-modules/src/{text,fontcov}.rs` | modify | Add interface/display font roles, fallback chains, cache identity and coverage gates. |
| `locales/{en,es,be}/{browse,settings}.json` | modify | Add textual Search, server-dialog and real Settings subtitle strings. |
| `pkg/` and `THIRD-PARTY-NOTICES.md` | create/modify | Add Manrope and Fraunces static faces while retaining Inter/CJK assets and updating attribution. |
| `Makefile`, `ci/{check-package,source_bundle}.py`, `tools/font-hint-audit.py` | modify | Package and audit every shipped face without changing ASS or website font assets. |
| `rust-modules/src/dev/scenarios.rs` and `tests/screenshots/scenes.json` | modify | Reach Settings root and representative pickers through real input for 1920x1080 captures. |
| `docs/DESIGN.md` | modify | Replace the stale no-font-stack statement with the actual scoped roles and fallback pointers. |

## Required Context

| File | Why |
|------|-----|
| `plans/butaca-mockup-fidelity/{plan,phases/phase-1,reviews/plan-review}.md` | Gated scope, preserved behavior and acceptance criteria. |
| Supplied `butaca-rediseno-mockup.zip` | Sole binding visual source. Its `index.html` defines the avatar/wordmark order, textual navigation, gear/clock order, Settings header/cards and server dialog. |
| `rust-modules/src/ui/CLAUDE.md` and `rust-modules/src/screens/CLAUDE.md` | Shared-component, layer, focus, popover and capture invariants. |
| `docs/DESIGN.md` | Current canvas, palette, legibility and component contracts. |
| `rust-modules/src/app/chrome.rs` | Current labels, stable element identities and `ChromeSnapshot` publication. |
| `rust-modules/src/app/bridge.rs` | Current chrome capture, selection, PNG brand attempt, gear/clock drawing and `nav_tab` implementation. |
| `rust-modules/src/app/run.rs` | Existing `LoopReq` performers for server switching/adding and Settings. |
| `rust-modules/src/app/content.rs` | Existing HomeTab/account request behavior and focus-return conventions. |
| `rust-modules/src/screens/registry.rs` | `AppArg::chrome`, `LoopReq`, `HomeTab` and surface identities. |
| `rust-modules/src/ui/widgets.rs` | Chip, tab, Back, gear and `anchored_menu` geometry plus `profile_chip_with` and `StripRender`. |
| `rust-modules/src/ui/dispatch.rs` | `PageWithStrip`, top-page chrome drawing and ModalStack surface drawing/stops. |
| `rust-modules/src/screens/detail/mod.rs` | Detail activation and the native Back disc's draw/focus/pointer rectangles. |
| `rust-modules/src/screens/account_menu.rs` and `screens/servers.rs` | Current account actions and the proven server-to-`LoopReq` mapping. |
| `rust-modules/src/screens/settings.rs` and `screens/settings/page.rs` | `RouteSurface` input delegation, `RootPage` state/actions, card grid, full-level picker and Force confirmation. |
| `rust-modules/src/screens/{settings_root_navigation_tests,settings_draw_and_lifecycle_tests,settings_text_fit_tests}.rs` | Existing Settings regression conventions and translation-fit coverage. |
| `rust-modules/src/ui/{table,route_screen,table_screen}.rs` | Two-column/card geometry, centered narrative flow and Back control measurement. |
| `rust-modules/src/ui/{popover,decision_alert}.rs` | Embedded popover lifecycle and the correct page-local underlay pattern for a panel over Settings. |
| `rust-modules/src/text.rs` and `ui/{machine,rec,label,text_view}.rs` | Font loading, run fallback, measurement capabilities, cache keys and display call sites. |
| `rust-modules/src/fontcov.rs` and `tools/font-hint-audit.py` | Real shipped-font coverage/advance and rasterization gates. |
| `Makefile`, `ci/check-package.py`, `ci/source_bundle.py`, `THIRD-PARTY-NOTICES.md`, `pkg/OFL.txt` | Font payload, source-bundle and licensing integration. |
| `rust-modules/src/dev/scenarios.rs` and `tests/screenshots/scenes.json` | Existing deterministic capture mechanics. |
| `tests/replay_fixtures.py` and `tests/fixtures/replay/README.md` | Supported shape-rebase and measurement-fill migration. |

## Implementation Steps

### Step 1: Pin the visible and behavioral mismatches with failing regressions

- **What**: Add tests for textual Home/Favorites/Search, the avatar's stable `STRIP_BASE + 4` identity and left-to-right member order, Detail/Settings bar ownership, dynamic server rows and their loop requests, the real Settings subtitle, anchored picker geometry and BACK restoration, role-specific fallback and role-specific replay metric identity. Keep the existing tests for row/action pairing, Force confirmation and focus restoration as required regression anchors.
- **Where**: `app/chrome.rs`, `app/surface_navigation_tests.rs`, `screens/registry.rs`, `screens/account_menu.rs`, `screens/settings_*tests.rs`, `ui/dispatch.rs`, `ui/table.rs`, `text.rs`, `fontcov.rs` and `ui/rec.rs`.
- **Authorized By**: Phase 1 Deliverables and Acceptance Criteria 1-5, plus the repository invariant that a reported mismatch starts with a failing regression.
- **Why**: The incumbent tests explicitly expect icon destinations, no Detail/Settings chrome and a full-level picker. They must be changed from evidence of the old layout into proof of the gated one before implementation.
- **Considerations**: Assert real fixture data and stable identities, not mockup placeholder profiles, addresses, versions, warm colors or Phase 4 preference rows.

### Step 2: Add scoped font roles without forking the text or measurement systems

- **What**: Introduce `text::FontRole::{Interface, Display}`. Keep existing `Painter::text` and `Measure` entry points as Interface wrappers and add role-aware variants used by `Label` and `TextView` display builders. Thread the role through `TtfMeasure`, `TableMeasure`, `Measurements`, fitting/elision, prewarm, backdrop text declarations and glyph/wrap/cache keys, including `MetricKey`. Define Interface as Manrope -> Inter -> bundled CJK -> system fallback and Display as Fraunces -> Inter -> bundled CJK -> system fallback in `text::{app_font,link_file,link_font,split_runs_with}`.
- **Where**: `text.rs`, `ui/machine.rs`, `ui/rec.rs`, `ui/mod.rs`, `ui/label.rs` and `ui/text_view.rs`.
- **Authorized By**: Phase 1 Includes "Manrope interface typography and a scoped Fraunces display role" and Acceptance Criterion 3.
- **Why**: Size plus bold cannot select a typeface. Omitting the role from any measurement or texture key can reuse Inter metrics or pixels for Fraunces, which breaks layout and deterministic replay.
- **Considerations**: Update `RESOLUTION_SHAPE` and rebase unchanged recordings before enabling display call sites. Later steps fill the new role-keyed measurements. Existing callers remain on Interface by default, and no display role reaches ordinary body copy.

### Step 3: Vendor and package the new faces while retaining Inter and CJK

- **What**: Add statically instanced Manrope 400/700 and Fraunces 600/700 as new `pkg/appfont-manrope*.ttf` and `pkg/appfont-fraunces*.ttf` assets with pinned upstream provenance. Keep `appfont.ttf`, `appfont-bold.ttf` and `appfont-cjk.ttf` unchanged as the Inter and CJK fallback assets. Extend `fontcov` shipped-face/coverage/advance tests, `Makefile::APP_FILES`, `ci/source_bundle.py::PKG_FILES`, `ci/check-package.py` payload/attribution checks and `tools/font-hint-audit.py`. Update `THIRD-PARTY-NOTICES.md`, its OFL summary and the stale typography statement in `docs/DESIGN.md`.
- **Where**: `pkg/`, `rust-modules/src/fontcov.rs`, `Makefile`, `ci/`, `tools/font-hint-audit.py`, `THIRD-PARTY-NOTICES.md` and `docs/DESIGN.md`.
- **Authorized By**: Phase 1 Includes packaged fonts, Inter/CJK fallback and licensing updates, plus plan Non-Functional requirements.
- **Why**: New files are required by the two roles. Reusing the existing Inter files for Manrope would remove the mandated fallback and unnecessarily alter ASS fixtures and website typography.
- **Considerations**: No runtime Google Fonts dependency, no CJK re-cut, no changes to `tools/cut-inter.py`, ASS attachment fixtures or `tools/subset-site-fonts.py`. Record each source version, modification and license notice.

### Step 4: Implement the mockup's shared visual chrome and real avatar entry

- **What**: Replace Home/Favorites/Search icons with localized text in `ChromeSnapshot`, retaining every stable `Pill` and `STRIP_BASE` destination. Publish the avatar as `STRIP_BASE + 4`, map it to `TopFocus::Chip` and order chip, optional instance, pills and gear by their drawn geometry. In `ui::widgets`, use one shared chrome layout for the avatar, Fraunces wordmark, native Back disc, optional instance badge, centered navigation, gear and clock so the native Back cannot overlap the avatar or brand. In `Bridge::draw_chrome`, replace the PNG brand attempt with display-role text, draw `profile_chip_with` after `StripRender::draw` and keep the existing local clock and gear-to-clock ordering.
- **Where**: `app/chrome.rs`, `app/bridge.rs`, `ui/widgets.rs` and `locales/{en,es,be}/browse.json`.
- **Authorized By**: Phase 1 Includes textual destinations, real profile/avatar entry and mockup ordering, and Acceptance Criterion 1.
- **Why**: `profile_chip_with` currently has no production caller and the PNG wordmark is not in `APP_FILES`, so the current left chrome cannot become faithful by relabeling pills alone. The existing Back disc also occupies the avatar's region on Detail.
- **Considerations**: Preserve `STRIP_BASE + 7` Settings and `STRIP_BASE + 8` instance identities, Jellyfin colors, safe margins and current couch-legible sizes. Retain native Back as an addition beside the mockup controls and update overscan/focus geometry tests rather than hiding it.

### Step 5: Extend drawing, focus and activation to Detail and the Settings surface

- **What**: Mark `ContentArg::Detail` and `AppArg::Settings` as `Chrome::TabBar`. Include Detail in `Bridge::capture_chrome` and read focus from the topmost visible bar-wearing surface, falling back to the top page. Generalize `dispatch::PageWithStrip` ownership and the ModalStack draw/stop path so an input-owning `Chrome::TabBar` surface gets the same captured strip as a top page. Add one registry mapping from stable chrome elements to existing `HomeTab`, `AccountSettings` and account actions, with `LoopReq::{ChromeTab(HomeTab), ChromeAccount}` as the small boundary for the two new consumers. Call it from `DetailScreen::activate` and `RouteSurface::step`, then perform the requests in `app::run` through `nav_tab`, `open_account_menu` and the existing surface dismissal path.
- **Where**: `screens/registry.rs`, `app/{bridge,run}.rs`, `ui/dispatch.rs`, `screens/detail/mod.rs` and `screens/settings.rs`.
- **Authorized By**: Phase 1 Includes shared chrome on Detail and Settings with working D-pad and activation paths, and Acceptance Criteria 1 and 4.
- **Why**: Changing `AppArg::chrome` alone only covers top pages. Settings is an opaque ModalStack surface, so its drawing, focus owner, pointer stops and activation would otherwise remain absent or attached to the host page.
- **Considerations**: Do not turn Settings into a NavStack page or add another route/screen id. Keep current Home/Library/Search/Favorites request handlers, Detail's `HEADER_CRUMB` and trailer behavior, and the existing destination and focus identities.

### Step 6: Turn the account sheet into the real server dialog without losing native actions

- **What**: Replace the static account row slice with a row vector built from `jellyfin::boot::load_servers`, using the same `ServerEntry { user, url }` facts and active index as `ServersPage`. Map each server to `LoopReq::JfSwitchServer(index)` and the add row to `LoopReq::JfAddServer`. Center the panel as the mockup's server dialog, add its localized display title and explanatory subtitle and retain `TableView` scrolling and `panel_ground`. Keep SignIn/SignOut, Settings and lab diagnostics as deliberate native additions below the server choices, with stable explicit element ids and non-destructive opening focus.
- **Where**: `screens/account_menu.rs` and `locales/{en,es,be}/settings.json`.
- **Authorized By**: Phase 1 Includes real server switching and adding through the profile menu using existing account/server actions, and Acceptance Criteria 1 and 4.
- **Why**: The current sheet only offers account and Settings rows. `ServersPage` and `app::run` already implement the required real actions, so no new account behavior or server fetch is needed.
- **Considerations**: Do not invent server names, Jellyfin versions or placeholder toasts. Preserve Quick Connect in Settings and password add in this dialog, and leave `ServersPage` behavior unchanged.

### Step 7: Align Settings composition and replace full-level pickers with anchored popovers

- **What**: Build and retain a root subtitle from `server::session::current` plus `Session::account` (the same account path `ChromeSnapshot` captures), `server::identity::VERSION` and a localized no-telemetry clause. Use the display role for the Settings title at the shared `DISPLAY` rung and measure the actual title/subtitle flow before placing content. Keep the existing four groups, `grid(2)`, `cards(true)` and every native row/action, retuning only card-specific metrics toward the mockup's 86px rows, 12px gaps and 18px radius. Replace `RoutePush` with an embedded `Popover`: compute the picker frame from `TableView::row_rect` plus `widgets::anchored_menu`, draw through `Popover::panel` with a Settings-page-local `UnderlayField`, and keep the existing options, commits, optimistic values and `DecisionPrompt` Force flow. BACK dismisses the popover and restores `parent_row`.
- **Where**: `screens/settings/page.rs`, `ui/{table,route_screen,table_screen,popover,decision_alert}.rs` and `locales/{en,es,be}/settings.json`.
- **Authorized By**: Phase 1 Includes Settings title, real profile/version/privacy subtitle, two-column card geometry and anchored native pickers with BACK/focus restoration, and Acceptance Criteria 2 and 4.
- **Why**: The root already has the correct section and card primitives, but its static subtitle and HERO title do not match the reference and its picker replaces the whole level. `RootPage` already owns every needed action and confirmation, so a new registered picker screen would be unnecessary.
- **Considerations**: Scope metric changes to `cards(true)` so ordinary lists do not move. Do not globally convert `RouteLayout::narrative_title` or body text to Fraunces, and do not add the Phase 4 audio/subtitle preferences.

### Step 8: Add the required captures and complete the replay metric migration

- **What**: Add a dev-only scenario that opens named Settings pickers through the real D-pad/OK path. Add 1920x1080 `settings`, `settings-quality` and `settings-subtitle-size` scenes beside the existing `home`, `account-menu` and `detail` scenes, with deterministic output names and no real account writes. After the role descriptor rebase in Step 2, use `tests/replay_fixtures.py --fill` on the completed UI to learn Manrope/Fraunces metrics, then run strict replay. Open the six captures and review them against the supplied mockup.
- **Where**: `rust-modules/src/dev/scenarios.rs`, `tests/screenshots/scenes.json`, `tests/replay_fixtures.py` fixtures and `tests/fixtures/replay/ALPHABET.json`.
- **Authorized By**: Phase 1 Includes regression tests and 1920x1080 captures, Acceptance Criterion 5, and the plan's supported replay measurement policy.
- **Why**: Current screenshot scenes have no Settings root or picker target, and old Inter metrics cannot validate new role-keyed rendering. Existing dev triggers and the fill tool provide the required evidence without new capture infrastructure or behavior re-recording.
- **Considerations**: Rebase only the descriptor/shape, fill measurements and never re-record behavior for this visual phase. Preserve unrelated fixture content and existing tolerance regions.

## Testing Plan

Run this once on the committed, clean tree after the replay fill, then restore any `make check` self-test debris and open the generated `ux-home-hero.jpg`, `ux-account-menu.jpg`, `ux-detail.jpg`, `settings.jpg`, `settings-quality.jpg` and `settings-subtitle-size.jpg` for the required mockup review.

**Primary Verify Command**:
```sh
make RUST_NIGHTLY=nightly-2026-07-02 check && \
CARGO_INCREMENTAL=0 cargo +nightly-2026-07-02 check --manifest-path rust-modules/Cargo.toml --lib --no-default-features && \
./tools/font-hint-audit.py && \
RUSTUP_TOOLCHAIN=nightly-2026-07-02 make screenshots SHOT_SCENES=home,account-menu,detail,settings,settings-quality,settings-subtitle-size SHOT_OUT=/tmp/opencode/butaca-449-phase1 SHOT_CHECK=1 && \
RUSTUP_TOOLCHAIN=nightly-2026-07-02 make check-replay
```

## Reality Check

- **Non-blocking**: Settings already implements the four groups, two-column grid and one card per row in `RootPage::rebuild` and `TableView::{grid,cards}`. Rebuild only the real header and picker presentation, preserving the row/action model and native additions.
- **Non-blocking**: `profile_chip_with` has no production call site and `pkg/butaca-name.png` is absent from `APP_FILES`. The avatar and wordmark therefore need explicit layout and draw integration, not a simple asset swap.
- **Non-blocking**: Settings is a `Style::Opaque` ModalStack surface, not a top page. Its shared chrome requires the dispatcher surface path in Step 5 rather than only an `AppArg::chrome` edit.
- **Non-blocking**: Role-aware metrics change the replay measurement identity. The supported descriptor rebase and fill are required, but behavior re-recording is neither needed nor authorized.
