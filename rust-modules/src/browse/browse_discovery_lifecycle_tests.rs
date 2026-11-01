//! Discovery landing lifecycle: retry addressing, page work and single-flight reset. The
//! repoint/retoken/profile staleness guards belonged to the Plex registry lifecycle and went
//! with the Plex discovery arm (#166); the roster's one source has no lifecycle beyond the
//! client install itself.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;

#[test]
fn addressed_discovery_retry_rejects_retired_tables_and_other_sources() {
    let _guard = crate::testlock::serial();
    let mut browse = TestBrowse::default();
    seed_sources_for_owner_test(&mut browse.state, 2, false);
    browse.state.source_mut(0).unwrap().retry_cd = 9;
    browse.state.source_mut(1).unwrap().retry_cd = 13;
    let sid = browse.state.sources()[1].sid;
    let epoch = browse.state.table_epoch();
    assert!(browse.state.retry_source(epoch, sid));
    assert_eq!(browse.state.sources()[0].retry_cd, 9);
    assert_eq!(browse.state.sources()[1].retry_cd, 0);
    browse.state.source_mut(1).unwrap().retry_cd = 17;
    assert!(!browse.state.retry_source(epoch.wrapping_add(1), sid));
    assert!(!browse.state.retry_source(epoch, ServerId::from_raw(99)));
    assert_eq!(browse.state.sources()[1].retry_cd, 17);
}
#[test]
fn a_settled_query_change_with_unknown_total_is_still_page_work() {
    let _g = crate::testlock::serial();
    let mut state = BrowseState::default();
    let adapter = BrowseAdapter::default();
    crate::browse::seed_source_for_owner_test(&mut state, ServerId::from_raw(1));
    crate::browse::append_section_for_owner_test(&mut state, 0, 1, "Movies", SecKind::Movie);
    for source in &mut state.sources {
        source.sections_done = true;
        source.counts_done = true;
    }
    let resident = state.state_mut(0).unwrap();
    resident.fetch = SecFetch::Ready;
    resident.total = 1;
    adapter.src_fetching.store(true, Ordering::SeqCst);
    assert!(!state.pump_needs_work(&adapter), "the resident query starts settled");

    state.set_unwatched(true);

    assert_eq!(state.states[state.cur()].total, -1);
    assert!(state.pump_needs_work(&adapter),
        "unknown total means page zero is owed even before a wanted window is published");
}
#[test]
fn reset_clears_the_single_flight_flags_with_the_mailboxes() {
    let _g = crate::testlock::serial();
    let mut browse = TestBrowse::default();
    browse.adapter.fetching.store(true, Ordering::SeqCst);
    browse.adapter.genre_fetching.store(true, Ordering::SeqCst);
    browse.adapter.letters_fetching.store(true, Ordering::SeqCst);
    *browse.adapter.page_result.lock().unwrap_or_else(|e| e.into_inner()) = None;

    browse.reset();

    assert!(
        !browse.adapter.fetching.load(Ordering::SeqCst),
        "page fetch stayed latched — Library wedges"
    );
    assert!(
        !browse.adapter.genre_fetching.load(Ordering::SeqCst),
        "genre fetch stayed latched"
    );
    assert!(
        !browse.adapter.letters_fetching.load(Ordering::SeqCst),
        "letters fetch stayed latched"
    );
}
/// `reset()` must also drop the retry backoff, or a profile switch inherits the previous
/// user's cooldown and stalls the first page fetch for up to ~2s.
#[test]
fn reset_clears_the_retry_backoff() {
    // Takes the crate lock for the same reason the fetch-machine tests below do — see the note
    // there. `reset()` is the most destructive call in this module, and a test that makes it
    // without the lock is not testing concurrently, it is CORRUPTING whoever is.
    let _g = crate::testlock::serial();
    let mut browse = TestBrowse::default();
    browse.state.retry_cd = 120;
    browse.reset();
    assert_eq!(browse.state.retry_cd, 0);
}
