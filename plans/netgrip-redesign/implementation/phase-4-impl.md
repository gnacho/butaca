---
type: planning
entity: implementation-plan
plan: "NetGrip visual redesign"
phase: 4
status: completed
created: "2026-11-23"
updated: "2026-11-23"
---

# Implementation Plan: Phase 4 - Structural menu follow-up

> Implements [Phase 4](../phases/phase-4.md) of [NetGrip visual redesign](../plan.md)

## Approach

Add scoped visual variants to the existing `TabStrip`, `ValueChip`, `TableView`, `Header` and `RootPage` instead of creating parallel widgets. Preserve every focus identity, action, route and fixed-canvas contract. Drive each change with a failing host regression, then inspect the actual simulator frames.

## Affected Modules

| Module | Change Type | Description |
|--------|-------------|-------------|
| `rust-modules/src/app/chrome.rs` | modify | Publish complete uppercase labels and existing Lucide icons for every destination without changing keys. |
| `rust-modules/src/ui/widgets.rs` | modify | Add the flat top track, flat ink and `TabGround::Flat`, with underline selection and rectangular focus. |
| `rust-modules/src/ui/value_chip.rs` | modify | Draw 44 px outlined chips with a light focus wash instead of a solid amber block. |
| `rust-modules/src/screens/library/toolbar.rs` | modify | Add the numbered technical eyebrow and preserve the chips' measured center. |
| `rust-modules/src/screens/library/menu.rs` | modify | Use 520 px compact menus and the technical table face; Sources stays 650 px. |
| `rust-modules/src/ui/table.rs` | modify | Add reusable technical rows, hairlines and section cards without changing row indices. |
| `rust-modules/src/ui/route_screen.rs` | modify | Add title-size and leading variants while keeping the ordinary HERO wrapper. |
| `rust-modules/src/ui/table_screen.rs` | modify | Add `Header::with_title_size` and route it through the sized narrative. |
| `rust-modules/src/screens/settings/page.rs` | modify | Add the numbered compact title and one-column section cards. |
| `locales/{en,es,be}/browse.json` | modify | Add the short Search destination label used by the flat navigation. |

## Required Context

| File | Why |
|------|-----|
| `plans/netgrip-redesign/plan.md` | Product and publication constraints. |
| `plans/netgrip-redesign/phases/phase-4.md` | Gated structural scope and acceptance. |
| `rust-modules/src/ui/CLAUDE.md` | Shared component, token, text and geometry rules. |
| `rust-modules/src/screens/CLAUDE.md` | Focus and screen ownership invariants. |
| `docs/DESIGN.md` | Current visual-world documentation to update after verification. |

## Implementation Steps

### Step 1: Flatten the shared top navigation

- **What**: Publish complete labels/icons, measure labels even with icons, draw a safe-width graphite track, use underline selection and rectangular focus.
- **Where**: `app/chrome.rs` and `widgets.rs`, with regressions in `widgets_tab_strip_geometry_tests.rs` and `app::chrome` tests.
- **Authorized By**: Phase 4 Includes "Flat top navigation" and Acceptance Criteria requiring a visibly different structure with stable destinations.
- **Why**: The centered tvOS capsule was the most visible holdout of the old interface.
- **Considerations**: Keep `TOP_BAR_BOTTOM`, focus identities, tab springs and every other strip unchanged.

### Step 2: Translate Library chips and popovers

- **What**: Add the numbered eyebrow, 44 px outlined chips, compact panels and technical table rows with hairlines.
- **Where**: `screens/library/{toolbar,menu,draw}.rs` and `ui/{value_chip,table}.rs`.
- **Authorized By**: Phase 4 Includes "Numbered Library eyebrow" and compact popovers.
- **Why**: The old large buttons and 650 px panels looked unchanged after the palette pass.
- **Considerations**: Keep chip measurement, anchors, menu actions, selected rows and focus order.

### Step 3: Group Settings by section

- **What**: Add the numbered compact header and one technical card per section, with rows and hairlines inside.
- **Where**: `ui/{route_screen,table_screen,table}.rs` and `screens/settings/page.rs`.
- **Authorized By**: Phase 4 Includes "Settings' numbered compact header and one technical card per section".
- **Why**: Per-setting buttons preserved the previous product's structure too closely.
- **Considerations**: Keep the Back disc, explanatory text, pickers, actions, values, saved preferences and table motion state.

### Step 4: Verify on this PC

- **What**: Run targeted and full host suites, release check, replay and official mock-backed captures of Library and Settings, then publish the follow-up after CI.
- **Where**: Local simulator and the existing CI workflows.
- **Authorized By**: Phase 4 Deliverables and Acceptance Criteria.
- **Why**: Tests cannot judge whether the user will recognize the redesign; the frames and CI together cover structure and integration.
- **Considerations**: No television claim and no merge or release.

## Testing Plan

**Primary Verify Command**: `set -e; CARGO_INCREMENTAL=0 cargo +nightly-2026-07-02 test --manifest-path rust-modules/Cargo.toml --lib && CARGO_INCREMENTAL=0 cargo +nightly-2026-07-02 test --manifest-path rust-modules/Cargo.toml --lib --features hostsim && CARGO_INCREMENTAL=0 cargo +nightly-2026-07-02 check --manifest-path rust-modules/Cargo.toml --lib --no-default-features && RUSTUP_TOOLCHAIN=nightly-2026-07-02 make check-replay && python3 tools/screenshots.py --bin rust-modules/target-sim/release/butaca-sim --out /tmp/opencode/netgrip-structural --only library-grid,library-sort,settings`

The full clean-tree `make check` and CI run after the structural commit. Capture files are opened and reviewed before publication.

## Reality Check

- The previous plan was marked complete before the user saw the result. Their feedback reopened the structural scope; palette and CI evidence alone were not sufficient.
