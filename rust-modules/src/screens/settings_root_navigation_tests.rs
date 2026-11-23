//! Root-page mounting, Jellyfin server navigation, BACK/focus restore, and the surface's
//! own logical-state hash across its inner stack.

use super::*;
use super::test_support::*;
use super::super::registry::LoopReq;
use crate::ui::machine::{Edge, InputEvent, InputKind, Source};
use crate::ui::present::Present;
use crate::ui::screen::By;

/// The drawn row index of the row carrying `label` — the page's own row order is its business,
/// so a test names a row by what it says rather than by a position that a redesign moves.
fn row_of(page: &RootPage, label: &str) -> i32 {
    page.table
        .sections
        .iter()
        .flat_map(|s| s.rows.iter())
        .position(|r| r.label == label)
        .unwrap_or_else(|| panic!("no row labelled {label}")) as i32
}

#[test]
fn mounting_the_surface_names_its_root_page() {
    let _g = crate::testlock::serial();
    let _sess = scratch_session("surface-mount");
    let mut s = RouteSurface::new(
        EntryId(0),
        InstanceId(0),
        Family::Settings,
        SettingsPage::Root,
        crate::catalog::HubsSnapshot::empty_for_test().view(),
    );
    step(&mut s, ScreenEvent::Mount, None);
    assert_eq!(name(&s), word::SETTINGS);
    assert_eq!(s.inner.depth(), 1);
    assert_eq!(s.kind, Family::Settings);
}

/// The root is the mockup's one page: four groups, every row visible. It used to be four
/// door-cards with the playback knobs one level in, which is exactly what the owner reported as
/// "no están los ajustes como el mockup".
#[test]
fn the_root_is_the_mockups_one_page_of_four_groups() {
    let _g = crate::testlock::serial();
    let _sess = scratch_session("jellyfin-root-rows");
    let root = RootPage::new(EntryId(0));
    let headers: Vec<&str> = root.table.sections.iter().map(|s| s.header.as_str()).collect();
    assert_eq!(headers.len(), 4, "servidor, reproducción, audio y subtítulos, interfaz");
    assert_eq!(root.table.sections[0].rows.len(), 3, "server, add another, disconnect");
    assert_eq!(root.table.sections[2].rows.len(), 3, "the three subtitle rows");
    assert_eq!(root.table.sections[3].rows.len(), 3, "language, legal, about");
    assert!(root.table.sections[1].rows.len() >= 7,
        "quality, direct play, audio sync, next episode, skip, recently added, screensaver");
    assert_eq!(root.table.cols(), 2, "the mockup's two-column grid");
}

#[test]
fn back_at_the_surface_s_own_root_is_not_handled() {
    let _g = crate::testlock::serial();
    let _sess = scratch_session("surface-back-root");
    let mut s = RouteSurface::new(
        EntryId(0),
        InstanceId(0),
        Family::Settings,
        SettingsPage::Root,
        crate::catalog::HubsSnapshot::empty_for_test().view(),
    );
    step(&mut s, ScreenEvent::Mount, None);
    let back: ScreenEvent<InnerHost> = ScreenEvent::Input(InputEvent {
        at: Tick::default(),
        source: Source::Sdl,
        kind: InputKind::Key {
            key: Key::Back,
            sym: 0,
            wcode: 0,
            edge: Edge::Down,
            at_edge: false,
        },
    });
    let mut out = Vec::new();
    let mut present = Present::new();
    let mut fx = Effects::new(&mut out, MachineId::Instance(InstanceId(0)), &mut present);
    let handled = <RouteSurface as Machine<InnerHost>>::step(&mut s, &back, &cx(None), &mut fx);
    assert_eq!(handled, Handled::No);
    assert_eq!(s.inner.depth(), 1);
}

#[test]
fn a_pop_from_servers_restores_focus_to_the_row_that_opened_it() {
    let _g = crate::testlock::serial();
    let _sess = scratch_session("surface-pop-focus");
    let mut s = RouteSurface::new(
        EntryId(0),
        InstanceId(0),
        Family::Settings,
        SettingsPage::Root,
        crate::catalog::HubsSnapshot::empty_for_test().view(),
    );
    step(&mut s, ScreenEvent::Mount, None);

    let server_row = FocusKey { entry: EntryId(0), elem: 0 };
    step(
        &mut s,
        ScreenEvent::FocusMoved { from: None, to: server_row, by: By::Dir },
        Some(server_row),
    );
    step(&mut s, ScreenEvent::Activate(server_row.elem), Some(server_row));
    assert_eq!(name(&s), "servers");
    assert_eq!(s.inner.depth(), 2);

    let back: ScreenEvent<InnerHost> = ScreenEvent::Input(InputEvent {
        at: Tick::default(),
        source: Source::Sdl,
        kind: InputKind::Key {
            key: Key::Back,
            sym: 0,
            wcode: 0,
            edge: Edge::Down,
            at_edge: false,
        },
    });
    let out = step(&mut s, back, None);
    assert_eq!(name(&s), word::SETTINGS);
    assert_eq!(s.inner.depth(), 1);

    let reseat = out.iter().find_map(|st| match &st.fx {
        Fx::Deliver(
            _,
            Delivery::Screen(ScreenEvent::Enter(Enter::Fresh {
                focus: FocusTarget::Elem(k),
            })),
        ) => Some(*k),
        _ => None,
    });
    assert_eq!(reseat, Some(server_row));
}

#[test]
fn quick_connect_requests_the_jellyfin_flow_without_pushing_a_settings_page() {
    let _g = crate::testlock::serial();
    let _sess = scratch_session("root-quick-connect");
    let mut root = RootPage::new(EntryId(0));
    let mut out = Vec::new();
    let mut present = Present::new();
    let mut fx = Effects::new(&mut out, MachineId::Instance(InstanceId(0)), &mut present);
    root.activate(row_of(&root, crate::i18n::msg::settings_servers_add()) as usize, &mut fx);
    let requests: Vec<_> = out
        .iter()
        .filter_map(|st| match &st.fx {
            Fx::App(AppFx::Loop(req)) => Some(*req),
            _ => None,
        })
        .collect();
    assert_eq!(requests, vec![LoopReq::JfQuickConnect]);
    assert!(!out.iter().any(|st| matches!(st.fx, Fx::Nav(_))));
}

/// The navigation rows of the one page push the pages the family already had, and the two
/// account rows still go through the loop's own requests.
#[test]
fn navigation_rows_push_their_existing_pages_and_sign_out_disconnects() {
    let _g = crate::testlock::serial();
    let _sess = scratch_session("root-preference-actions");
    for (label, wanted) in [
        (crate::i18n::msg::settings_servers_current(), SettingsPage::Servers),
        (crate::i18n::msg::settings_language_title(), SettingsPage::Language),
        (crate::i18n::msg::settings_legal_title(), SettingsPage::Legal),
        (crate::i18n::msg::settings_about_title(), SettingsPage::About),
    ] {
        let mut root = RootPage::new(EntryId(0));
        let mut out = Vec::new();
        let mut present = Present::new();
        let mut fx = Effects::new(&mut out, MachineId::Instance(InstanceId(0)), &mut present);
        let row = row_of(&root, label);
        root.activate(row as usize, &mut fx);
        assert!(out.iter().any(|st| matches!(st.fx, Fx::Nav(NavOp::Push(page)) if page == wanted)),
            "row {row:?} opens {wanted:?}");
    }

    let mut root = RootPage::new(EntryId(0));
    let mut out = Vec::new();
    let mut present = Present::new();
    let mut fx = Effects::new(&mut out, MachineId::Instance(InstanceId(0)), &mut present);
    let sign_out = row_of(&root, crate::i18n::msg::settings_servers_disconnect());
    root.activate(sign_out as usize, &mut fx);
    assert!(out.iter().any(|st| matches!(st.fx, Fx::App(AppFx::Loop(LoopReq::JfDisconnectServer)))),
        "the disconnect row uses the existing disconnect flow");
    assert!(!out.iter().any(|st| matches!(st.fx, Fx::Nav(_))));
}

#[test]
fn the_logical_state_follows_the_inner_stack() {
    let _g = crate::testlock::serial();
    let _sess = scratch_session("surface-state-hash");
    let mut s = RouteSurface::new(
        EntryId(0),
        InstanceId(0),
        Family::Settings,
        SettingsPage::Root,
        crate::catalog::HubsSnapshot::empty_for_test().view(),
    );
    step(&mut s, ScreenEvent::Mount, None);
    let at_root = <RouteSurface as Screen<InnerHost>>::state(&s).hash();

    let server_row = FocusKey { entry: EntryId(0), elem: 0 };
    step(
        &mut s,
        ScreenEvent::FocusMoved { from: None, to: server_row, by: By::Dir },
        Some(server_row),
    );
    step(&mut s, ScreenEvent::Activate(server_row.elem), Some(server_row));
    let at_servers = <RouteSurface as Screen<InnerHost>>::state(&s).hash();
    assert_ne!(at_root, at_servers);

    let mut probe = String::new();
    <RouteSurface as Screen<InnerHost>>::state(&s).probe(&mut probe);
    assert!(probe.starts_with("settings/root:") && probe.contains("/servers:"));
}
