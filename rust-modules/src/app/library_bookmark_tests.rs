//! Library bookmark commands under the Bridge dispatcher.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;
use super::test_support::{frame, frame_with_tap};

#[test]
fn bookmark_commands_are_addressed_and_identical_snapshots_are_quiet() {
    use crate::stores::browse::{BrowseCmd, LibraryWork, SectionAddress};
    let _guard = crate::testlock::serial();
    let stores = crate::stores::Stores::default();
    stores.browse.borrow_mut().seed_two_source_table_for_test();
    let mut directory = crate::stores::browse::DirectorySnapshot::default();
    stores.capture_browse(&mut directory);
    let section = &directory.view().sections()[0];
    let target = SectionAddress {
        epoch: directory.view().epoch().unwrap(),
        sid: section.sid.unwrap(),
        section: section.key,
    };
    stores.browse_run(BrowseCmd::SetCur(0));
    let mut retained = None;
    for at in [
        crate::stores::browse::CursorAt::SlotIndex(52),
        crate::stores::browse::CursorAt::ItemKey {
            sid: target.sid,
            rk: "synthetic-52".into(),
            slot: 52,
        },
    ] {
        let cursor = crate::stores::browse::Cursor { at, scroll: 1200.0 };
        let query = stores.browse.borrow_mut().listing_snapshot().view().id().unwrap().query;
        let command = BrowseCmd::Addressed {
            target,
            work: LibraryWork::SaveCursor {
                query,
                cursor: cursor.clone(),
            },
        };
        assert!(stores.browse_run(command.clone()));
        let snapshot = stores.browse.borrow_mut().listing_snapshot();
        assert_eq!(snapshot.view().cursor(), Some(&cursor));
        if let Some((old, value)) = &retained {
            let old: &crate::stores::browse::ListingSnapshot = old;
            assert_eq!(
                old.view().cursor(),
                Some(value),
                "a later save cannot mutate a retained frame"
            );
        }
        retained = Some((snapshot, cursor.clone()));
        stores.take_notices();
        let generation = stores.gen(StoreId::Browse);
        assert!(!stores.browse_run(command));
        assert_eq!(stores.gen(StoreId::Browse), generation);
        assert!(
            stores.take_notices().is_empty(),
            "identical snapshots owe no store notice"
        );
        assert!(!stores.browse_run(BrowseCmd::Addressed {
            target: SectionAddress {
                epoch: target.epoch.wrapping_add(1),
                ..target
            },
            work: LibraryWork::SaveCursor {
                query,
                cursor: cursor.clone()
            },
        }));
        assert_eq!(stores.gen(StoreId::Browse), generation);
        assert!(
            stores.take_notices().is_empty(),
            "rejected snapshots owe no store notice"
        );
        assert!(!stores.browse_run(BrowseCmd::Addressed {
            target,
            work: LibraryWork::SaveCursor {
                query: query.wrapping_add(1),
                cursor
            },
        }));
        assert_eq!(stores.gen(StoreId::Browse), generation);
        assert!(
            stores.take_notices().is_empty(),
            "an obsolete query snapshot is quiet too"
        );
    }
    stores.browse_run(BrowseCmd::Reset);
    assert!(stores.browse.borrow_mut().listing_snapshot()
        .view()
        .cursor()
        .is_none());
    let (snapshot, value) = retained.unwrap();
    assert_eq!(
        snapshot.view().cursor(),
        Some(&value),
        "profile reset drops live bookmarks, not retained frames"
    );
}

#[test]
fn retired_library_entry_seeds_its_previous_section_card_and_viewport() {
    #[derive(Default)]
    struct Saves(Vec<(crate::stores::browse::SectionAddress, crate::stores::browse::Cursor)>);
    impl crate::ui::dispatch::Tap<AppHost> for Saves {
        fn effect(&mut self, _: u64, effect: &crate::ui::machine::Stamped<AppHost>) {
            if let Fx::App(AppFx::Store(
                _,
                crate::stores::StoreCmd::Browse(crate::stores::browse::BrowseCmd::Addressed {
                    target,
                    work: crate::stores::browse::LibraryWork::SaveCursor { cursor, .. },
                }),
            )) = &effect.fx
            {
                self.0.push((*target, cursor.clone()));
            }
        }
    }
    let _guard = crate::testlock::serial();
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            crate::jellyfin::uninstall();
            crate::registry::reset_for_test();
        }
    }
    let _cleanup = Cleanup;
    let session = crate::server::session::TempSession::new("library-bookmark-return");
    session.watching("u-library-bookmark-return");
    crate::registry::reset_for_test();
    // The flavor's world: one server, the installed client. A seeded source naming any other
    // sid is retired by the first roster sync (browse_roster_jellyfin_tests), so the fixture
    // table rides the jellyfin SERVER_ID, and the section switch below (A -> B) stays within
    // that one server's own sections.
    crate::jellyfin::install(crate::jellyfin::JfClient::new(
        crate::origin::Origin::http("127.0.0.1", 1), "dev-1".into()));
    let mut d = Dispatcher::<AppHost>::new();
    let mut rig = Bridge::for_test(|| 0);
    rig.stores.browse.borrow_mut().seed_registered_table_for_test([
        crate::jellyfin::SERVER_ID; 2]);
    rig.refresh_browse_directory();
    rig.browse_run(crate::stores::browse::BrowseCmd::SetCur(0));
    rig.stores.browse.borrow_mut().seed_items_for_test(120);
    rig.stores.browse.borrow_mut().seed_shelves_for_test(0, &[], 4);
    frame(&mut d, &mut rig, AppArg::Home, tick(0), vec![]);
    d.nav.tabs.stack.transition = Box::new(crate::ui::containers::transition::Immediate);
    frame(&mut d, &mut rig, AppArg::Library, tick(1), vec![]);
    Bridge::library_command(
        &mut d,
        crate::screens::registry::LibraryCmd::FocusGrid { row: 8, col: 4 },
    );
    for i in 2..80 {
        frame(&mut d, &mut rig, AppArg::Library, tick(i), vec![]);
    }
    let old_entry = d.nav.top_page().unwrap().id;
    let (item, opener) = rig.library_selection(&d, old_entry, d.focus()).unwrap();
    let before = opener.rect.unwrap();
    let select = |d: &mut Dispatcher<AppHost>, rig: &Bridge, index: usize| {
        let view = rig.directory.view();
        let section = &view.sections()[index];
        let target = crate::stores::browse::SectionAddress {
            epoch: view.epoch().unwrap(),
            sid: section.sid.unwrap(),
            section: section.key,
        };
        let instance = d.nav.top_page().unwrap().inst.as_ref().unwrap().id;
        d.emit(
            MachineId::Nav,
            Fx::Deliver(
                MachineId::Instance(instance),
                Delivery::Screen(ScreenEvent::App(AppMsg::LibrarySelect(target))),
            ),
        );
    };
    // Menu selection addresses a covered host: its Cx must read that host entry's
    // engine memory, never the menu's current row or its unrelated remembered groups.
    let listing_id = rig.listing.view().id().unwrap();
    d.nav.next_style = crate::ui::containers::modal::Style::Compact;
    d.request(
        MachineId::Nav,
        NavOp::Present(AppArg::LibraryMenu(
            crate::screens::registry::LibraryMenuArg {
                host: d.nav.top_page().unwrap().inst.as_ref().unwrap().id,
                kind: crate::screens::registry::LibraryMenuKind::Filter,
                anchor: [0; 4],
                target: crate::stores::browse::SectionAddress {
                    epoch: listing_id.epoch,
                    sid: listing_id.sid,
                    section: listing_id.section,
                },
            },
        )),
    );
    frame(&mut d, &mut rig, AppArg::Library, tick(80), vec![]);
    let InputOwner::Entry(menu_entry) = d.nav.input_owner().unwrap() else {
        unreachable!()
    };
    assert_ne!(menu_entry, old_entry);
    select(&mut d, &rig, 1);
    let mut saves = Saves::default();
    for i in 81..120 {
        frame_with_tap(
            &mut d,
            &mut rig,
            AppArg::Library,
            tick(i),
            vec![],
            &mut saves,
        );
    }
    assert!(saves.0.iter().any(|(target, cursor)| target.sid == item.sid
        && matches!(&cursor.at, crate::stores::browse::CursorAt::ItemKey { sid, rk, slot: 52 } if *sid == item.sid && rk == &item.rk)),
        "section switching must emit the outgoing A snapshot through the StoreCmd drain");
    assert_eq!(
        rig.directory.view().current(),
        Some(1),
        "the actual deferred transaction enters section B"
    );
    d.request(MachineId::Nav, NavOp::Dismiss(menu_entry));
    rig.stores.browse.borrow_mut().seed_items_for_test(120);
    // B's shelves answer too (none): a section reveal, and a bookmark's re-entry seat, wait on
    // them rather than on the listing alone.
    rig.stores.browse.borrow_mut().seed_shelves_for_test(1, &[], 4);
    frame(&mut d, &mut rig, AppArg::Library, tick(120), vec![]);
    Bridge::library_command(
        &mut d,
        crate::screens::registry::LibraryCmd::FocusGrid { row: 3, col: 1 },
    );
    for i in 121..200 {
        frame(&mut d, &mut rig, AppArg::Library, tick(i), vec![]);
    }
    let (b_item, _) = rig.library_selection(&d, old_entry, d.focus()).unwrap();
    for i in 200..210 {
        let (_, report) = frame_with_tap(
            &mut d,
            &mut rig,
            AppArg::Home,
            tick(i),
            vec![],
            &mut saves,
        );
        d.prune(&report.unmounted);
    }
    assert!(saves.0.iter().any(|(target, cursor)| target.sid == b_item.sid
        && matches!(&cursor.at, crate::stores::browse::CursorAt::ItemKey { sid, rk, slot: 19 } if *sid == b_item.sid && rk == &b_item.rk)),
        "outgoing WillLeave must emit B's snapshot before its engine memory is retired");
    // One server now: the old cross-server rating-key-namespace assert has no subject (the
    // multi-source namespace contract lives on in catalog_multi_source_merge_tests). What must
    // still hold is that A and B each saved under their OWN section address.
    let saved_sections: std::collections::BTreeSet<i64> = saves.0.iter().map(|(t, _)| t.section).collect();
    assert!(saved_sections.len() >= 2, "A and B each saved under their own section");
    assert!(
        d.nav.entry(old_entry).is_none(),
        "the test must retire, not merely evict, the old entry"
    );
    // B is the one server's Show section now, so the re-entry opens the Show library.
    rig.enter_library(crate::stores::browse::SecKind::Show);
    for i in 210..290 {
        frame(&mut d, &mut rig, AppArg::Library, tick(i), vec![]);
    }
    let entry = d.nav.top_page().unwrap().id;
    assert_ne!(entry, old_entry);
    let (returned_b, _) = rig
        .library_selection(&d, entry, d.focus())
        .expect("new entry restores B's saved card");
    assert_eq!(
        (returned_b.sid, returned_b.rk.as_str()),
        (b_item.sid, b_item.rk.as_str())
    );
    select(&mut d, &rig, 0);
    for i in 290..370 {
        frame(&mut d, &mut rig, AppArg::Library, tick(i), vec![]);
    }
    assert_eq!(rig.directory.view().current(), Some(0));
    let (returned, opener) = rig
        .library_selection(&d, entry, d.focus())
        .expect("new entry restores the saved card");
    assert_eq!(
        (returned.sid, returned.rk.as_str()),
        (item.sid, item.rk.as_str())
    );
    let after = opener.rect.unwrap();
    assert!((before.y - after.y).abs() < 0.01, "{before:?} -> {after:?}");
}
