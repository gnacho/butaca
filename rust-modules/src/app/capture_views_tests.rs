//! `capture_views`'s change attribution: the section-shelf publication is compared once final.
//!
//! On a route with no `home_io` the browse-owned shelves land first and the catalog projection
//! overwrites them in the same pass, so a comparison taken between those two writes saw a change
//! on every frame and re-announced `StoreChanged(Browse, …)` for data that never moved (#257).

use super::*;
#[allow(unused_imports)]
use super::test_support::*;

#[test]
fn a_route_without_home_io_does_not_reannounce_browse_on_an_unchanged_frame() {
    let _guard = crate::testlock::serial();
    let session = crate::server::session::TempSession::new("capture-views-hubs");
    session.watching("u-capture-views-hubs");
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            crate::registry::reset_for_test();
        }
    }
    let _cleanup = Cleanup;
    crate::registry::reset_for_test();
    let mut d = Dispatcher::<AppHost>::new();
    let mut rig = Bridge::for_test(|| 0);
    rig.stores
        .browse
        .borrow_mut()
        .seed_registered_table_for_test([crate::registry::ServerId::from_raw(0); 2]);
    rig.refresh_browse_directory();
    rig.seed_hubs_for_test(6, crate::catalog::HubState::Ready);
    assert!(rig.home_io.is_none(), "the fixture is the detail route's shape");

    let _ = rig.capture_views(&mut d);
    let generation = rig.hubs.view().generation;
    assert!(rig.hubs.view().hub_count() > 0, "the catalog projection must run");
    assert_eq!(
        rig.section_hubs.view().revision(),
        Some(u64::from(generation)),
        "the published shelves are the catalog's, not the browse owner's stand-in"
    );

    let (browse_changed, _, _) = rig.capture_views(&mut d);
    assert!(
        !browse_changed,
        "an identical frame announces nothing: the shelf pair is compared once it is final"
    );
}
