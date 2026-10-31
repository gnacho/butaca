//! Root-page mounting, Jellyfin server navigation, BACK/focus restore, and the surface's
//! own logical-state hash across its inner stack.

use super::*;
use super::test_support::*;
use super::super::registry::LoopReq;
use crate::ui::machine::{Edge, InputEvent, InputKind, Source};
use crate::ui::present::Present;
use crate::ui::screen::By;

#[test]
fn mounting_the_surface_names_its_root_page() {
    let _g = crate::testlock::serial();
    let _sess = scratch_session("surface-mount");
    let mut s = RouteSurface::new(
        EntryId(0),
        InstanceId(0),
        Family::Settings,
        SettingsPage::Root,
        crate::pms::HubsSnapshot::empty_for_test().view(),
    );
    step(&mut s, ScreenEvent::Mount, None);
    assert_eq!(name(&s), word::SETTINGS);
    assert_eq!(s.inner.depth(), 1);
    assert_eq!(s.kind, Family::Settings);
}

#[test]
fn the_root_offers_only_the_two_jellyfin_server_doors() {
    let _g = crate::testlock::serial();
    let _sess = scratch_session("jellyfin-root-rows");
    let root = RootPage::new(EntryId(0), cx(None).views);
    assert_eq!(root.rows, vec![Action::SelectServer, Action::QuickConnect]);
    assert_eq!(root.table.sections.len(), 1, "a single Servers section");
    assert_eq!(root.table.sections[0].rows.len(), 2, "select and quick connect");
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
        crate::pms::HubsSnapshot::empty_for_test().view(),
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
        crate::pms::HubsSnapshot::empty_for_test().view(),
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
    let mut root = RootPage::new(EntryId(0), cx(None).views);
    let mut out = Vec::new();
    let mut present = Present::new();
    let mut fx = Effects::new(&mut out, MachineId::Instance(InstanceId(0)), &mut present);
    root.activate(1, cx(None).views, &mut fx);
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

#[test]
fn the_logical_state_follows_the_inner_stack() {
    let _g = crate::testlock::serial();
    let _sess = scratch_session("surface-state-hash");
    let mut s = RouteSurface::new(
        EntryId(0),
        InstanceId(0),
        Family::Settings,
        SettingsPage::Root,
        crate::pms::HubsSnapshot::empty_for_test().view(),
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
