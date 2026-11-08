//! The Identify store (#369): the two requests, their landings, and the phases a surface reads.
//!
//! The worker is never started here, `request_identify_with_spawn`'s spawn argument is the seam
//! (`request_detail_with_spawn`'s twin), so each test posts the answer the worker would have
//! produced and grades the landing. A real thread would race every assertion below, and a fetch for
//! an unknown server returns in microseconds.

use super::*;
use crate::registry::ServerId;

fn hit(name: &str, year: i32, provider: &str) -> IdentifyHit {
    crate::jellyfin::identify::hit_for_test(name, year, provider)
}

/// A store with one movie loaded, and the adapter its workers would capture.
fn loaded(sid: ServerId, rk: &str) -> (MetadataState, std::sync::Arc<MetadataAdapter>) {
    let mut state = MetadataState::default();
    set_current_for_test(
        &mut state,
        Some(Detail {
            sid,
            rk: rk.to_string(),
            kind: "movie".into(),
            title: "Alien".into(),
            year: 1979,
            ..Default::default()
        }),
    );
    (state, std::sync::Arc::new(MetadataAdapter::default()))
}

/// The lookup's answer lands as the list, and the surface is READY, which is a different answer
/// from an empty one, and both are different from the lookup failing.
#[test]
fn a_lookup_lands_its_candidates_and_the_surface_becomes_ready() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    let mut gen = 0;
    request_identify_with_spawn(&mut state, &adapter, sid, "mv-1", |g| {
        gen = g;
        true
    });
    assert_eq!(state.identify.status(), IdentifyStatus::Searching, "the lookup is out");
    assert!(state.identify.hits().is_empty());

    adapter.identify.post(
        (gen, IdentifyAnswer::Hits(Some(vec![hit("Alien", 1979, "TheMovieDb")]))),
        |old| old.0 < gen,
    );
    assert!(pump_identify(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert_eq!(state.identify.status(), IdentifyStatus::Ready);
    assert_eq!(state.identify.hits().len(), 1);
    assert_eq!(state.identify.hits()[0].provider, "TheMovieDb");
    assert!(state.identify.is_about(sid, "mv-1"), "the answer names the item it is about");
    assert!(!state.identify.is_about(sid, "mv-2"), "and never another item's");
}

/// An EMPTY list is READY with no rows; a FAILED lookup is FAILED. The surface words those two
/// differently ("no match found" versus "the server did not answer"), so the store must not fold
/// them into one.
#[test]
fn an_empty_list_and_a_failed_lookup_are_different_answers() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    let mut gen = 0;
    request_identify_with_spawn(&mut state, &adapter, sid, "mv-1", |g| {
        gen = g;
        true
    });
    adapter.identify.post((gen, IdentifyAnswer::Hits(Some(Vec::new()))), |old| old.0 < gen);
    assert!(pump_identify(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert_eq!(state.identify.status(), IdentifyStatus::Ready);
    assert!(state.identify.hits().is_empty());

    let (mut state, adapter) = loaded(sid, "mv-1");
    let mut gen = 0;
    request_identify_with_spawn(&mut state, &adapter, sid, "mv-1", |g| {
        gen = g;
        true
    });
    adapter.identify.post((gen, IdentifyAnswer::Hits(None)), |old| old.0 < gen);
    assert!(pump_identify(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert_eq!(state.identify.status(), IdentifyStatus::Failed);
}

/// A REFUSED spawn settles the surface at once: nothing will ever land, so a spinner left running
/// would be a lie about a request that was never made (the one-event rule `record::refused` keeps
/// for the detail fetch).
#[test]
fn a_refused_spawn_settles_instead_of_spinning() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    request_identify_with_spawn(&mut state, &adapter, sid, "mv-1", |_| false);
    assert_eq!(state.identify.status(), IdentifyStatus::Failed);
    assert!(!adapter.identify.busy(), "the claim is released, so a retry can spawn");
}

/// The apply's verdict is the SERVER's: `Applied` only from a success, and a refusal is its own
/// phase rather than looking like a network wobble.
#[test]
fn the_apply_verdict_is_the_servers_and_a_refusal_is_its_own_phase() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    let mut gen = 0;
    request_identify_with_spawn(&mut state, &adapter, sid, "mv-1", |g| {
        gen = g;
        true
    });
    adapter.identify.post(
        (gen, IdentifyAnswer::Hits(Some(vec![hit("Alien", 1979, "TheMovieDb")]))),
        |old| old.0 < gen,
    );
    assert!(pump_identify(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));

    // A refusal.
    state.identify.applying();
    assert_eq!(state.identify.status(), IdentifyStatus::Applying);
    let mut gen = 0;
    request_identify_apply_with_spawn(&mut state, &adapter, |g| {
        gen = g;
        true
    });
    adapter.identify.post((gen, IdentifyAnswer::Applied(false)), |old| old.0 < gen);
    assert!(pump_identify(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert_eq!(state.identify.status(), IdentifyStatus::ApplyFailed);

    // …and a success. The pump re-requests the item; under `cfg(test)` that fetch is HELD, never
    // started, so no thread races this assertion.
    state.identify.applying();
    let mut gen = 0;
    request_identify_apply_with_spawn(&mut state, &adapter, |g| {
        gen = g;
        true
    });
    adapter.identify.post((gen, IdentifyAnswer::Applied(true)), |old| old.0 < gen);
    assert!(pump_identify(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert_eq!(state.identify.status(), IdentifyStatus::Applied);
}

/// An answer from a SUPERSEDED request is discarded rather than installed: the generation is the
/// fence between a lookup the viewer already replaced and the list they are waiting for.
#[test]
fn a_superseded_answer_is_discarded() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    request_identify_with_spawn(&mut state, &adapter, sid, "mv-1", |_| true);
    // A second request supersedes the first, then the FIRST one's answer arrives late.
    request_identify_with_spawn(&mut state, &adapter, sid, "mv-1", |_| true);
    adapter.identify.post(
        (0, IdentifyAnswer::Hits(Some(vec![hit("Stale", 1999, "TheMovieDb")]))),
        |_| true,
    );
    assert!(!pump_identify(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert_eq!(state.identify.status(), IdentifyStatus::Searching, "still waiting");
    assert!(state.identify.hits().is_empty());
}

/// Closing the page drops the candidates: they are the departing item's, and the write they would
/// perform names its id.
#[test]
fn closing_the_page_drops_the_candidates() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    let mut gen = 0;
    request_identify_with_spawn(&mut state, &adapter, sid, "mv-1", |g| {
        gen = g;
        true
    });
    adapter.identify.post(
        (gen, IdentifyAnswer::Hits(Some(vec![hit("Alien", 1979, "TheMovieDb")]))),
        |old| old.0 < gen,
    );
    assert!(pump_identify(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert!(!state.identify.hits().is_empty());

    clear(&mut state, &adapter);
    assert_eq!(state.identify.status(), IdentifyStatus::Idle);
    assert!(state.identify.hits().is_empty());
    assert_eq!(state.identify.key(), None);
}
