//! Session/browse dispatch mechanics: chrome tab generation ownership, per-bridge browse
//! isolation, and the auth owner's queued commit-reply/carry machinery.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;

#[test]
fn browse_tab_generation_is_owned_and_chrome_never_replays_a_stale_shape() {
    let _guard = crate::testlock::serial();
    let session = crate::server::session::TempSession::new("bridge-browse-tabs");
    session.watching("u-bridge-browse-tabs");
    crate::registry::reset_for_test();
    let own = crate::registry::ServerId::from_raw(0);
    let shared = crate::registry::ServerId::from_raw(1);
    let mut rig = Bridge::for_test(|| 0);
    rig.stores.browse.borrow_mut().seed_registered_table_for_test([own, shared]);
    let mut pages = Dispatcher::<AppHost>::new();
    rig.capture_views(&mut pages);
    rig.capture_chrome(&mut pages);
    let before = rig.chrome.labels();
    assert!(before.labels.iter().any(|label| label == "TV Shows"));
    let before_gen = before.generation;

    rig.stores.browse.borrow_mut().run(
        crate::stores::browse::BrowseCmd::ApplyPins(vec![(1, false)]));
    rig.capture_views(&mut pages);
    rig.capture_chrome(&mut pages);
    let changed = rig.chrome.labels();
    assert!(changed.generation > before_gen);
    assert!(!changed.labels.iter().any(|label| label == "TV Shows"));
    let changed_gen = changed.generation;

    rig.capture_views(&mut pages);
    rig.capture_chrome(&mut pages);
    assert_eq!(rig.chrome.labels().generation, changed_gen,
        "capturing again must not republish the owner's old tab generation");
    assert!(!rig.chrome.labels().labels.iter().any(|label| label == "TV Shows"));
    crate::registry::reset_for_test();
}



