//! The *Edit images* store (#369): the two requests, their landings, and the phases a surface
//! reads.
//!
//! The worker is never started here, `request_images_with_spawn`'s spawn argument is the seam
//! (`request_identify_with_spawn`'s twin), so each test posts the answer the worker would have
//! produced and grades the landing.

use super::*;
use crate::registry::ServerId;

fn picture(url: &str) -> RemoteImage {
    crate::jellyfin::images::image_for_test(url, "TheMovieDb", 1000, 1500)
}

fn page(images: Vec<RemoteImage>, total: i32) -> crate::jellyfin::images::RemoteImages {
    crate::jellyfin::images::RemoteImages { images, total }
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
            ..Default::default()
        }),
    );
    (state, std::sync::Arc::new(MetadataAdapter::default()))
}

/// The list lands as the grid, with the server's total, and the surface is READY.
#[test]
fn a_list_lands_the_providers_pictures_and_the_total() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    let mut gen = 0;
    request_images_with_spawn(&mut state, &adapter, sid, "mv-1", |g| {
        gen = g;
        true
    });
    assert_eq!(state.images.status(), ImagesStatus::Loading, "the list is out");

    adapter.images.post(
        (
            gen,
            ImagesAnswer::List(Some(page(vec![picture("https://img.example/a.jpg")], 31))),
        ),
        |old| old.0 < gen,
    );
    assert!(pump_images(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert_eq!(state.images.status(), ImagesStatus::Ready);
    assert_eq!(state.images.images().len(), 1);
    assert_eq!(state.images.total(), 31, "the server holds more than this page");
    assert!(state.images.is_about(sid, "mv-1"), "the answer names the item it is about");
    assert!(!state.images.is_about(sid, "mv-2"), "and never another item's");
}

/// An EMPTY page is READY with no tiles; a FAILED list is FAILED. The surface words those two
/// differently ("no pictures found" versus "the server did not answer").
#[test]
fn an_empty_page_and_a_failed_list_are_different_answers() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    let mut gen = 0;
    request_images_with_spawn(&mut state, &adapter, sid, "mv-1", |g| {
        gen = g;
        true
    });
    adapter.images.post((gen, ImagesAnswer::List(Some(page(Vec::new(), 0)))), |old| old.0 < gen);
    assert!(pump_images(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert_eq!(state.images.status(), ImagesStatus::Ready);
    assert!(state.images.images().is_empty());

    let (mut state, adapter) = loaded(sid, "mv-1");
    let mut gen = 0;
    request_images_with_spawn(&mut state, &adapter, sid, "mv-1", |g| {
        gen = g;
        true
    });
    adapter.images.post((gen, ImagesAnswer::List(None)), |old| old.0 < gen);
    assert!(pump_images(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert_eq!(state.images.status(), ImagesStatus::Failed);
}

/// A REFUSED spawn settles the surface at once: nothing will ever land.
#[test]
fn a_refused_spawn_settles_instead_of_spinning() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    request_images_with_spawn(&mut state, &adapter, sid, "mv-1", |_| false);
    assert_eq!(state.images.status(), ImagesStatus::Failed);
    assert!(!adapter.images.busy(), "the claim is released, so a retry can spawn");
}

/// The download's verdict is the SERVER's: `Applied` only from a success, and a refusal — most
/// often an account without elevation — is its own phase.
#[test]
fn the_download_verdict_is_the_servers_and_a_refusal_is_its_own_phase() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    let mut gen = 0;
    request_images_with_spawn(&mut state, &adapter, sid, "mv-1", |g| {
        gen = g;
        true
    });
    adapter.images.post(
        (
            gen,
            ImagesAnswer::List(Some(page(vec![picture("https://img.example/a.jpg")], 1))),
        ),
        |old| old.0 < gen,
    );
    assert!(pump_images(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));

    // A refusal.
    state.images.applying();
    assert_eq!(state.images.status(), ImagesStatus::Applying);
    let mut gen = 0;
    request_images_apply_with_spawn(&mut state, &adapter, |g| {
        gen = g;
        true
    });
    adapter.images.post((gen, ImagesAnswer::Applied(false)), |old| old.0 < gen);
    assert!(pump_images(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert_eq!(state.images.status(), ImagesStatus::ApplyFailed);

    // …and a success. The pump re-requests the item; under `cfg(test)` that fetch is HELD, never
    // started, so no thread races this assertion.
    state.images.applying();
    let mut gen = 0;
    request_images_apply_with_spawn(&mut state, &adapter, |g| {
        gen = g;
        true
    });
    adapter.images.post((gen, ImagesAnswer::Applied(true)), |old| old.0 < gen);
    assert!(pump_images(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert_eq!(state.images.status(), ImagesStatus::Applied);
}

/// An answer from a SUPERSEDED request is discarded rather than installed.
#[test]
fn a_superseded_answer_is_discarded() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    request_images_with_spawn(&mut state, &adapter, sid, "mv-1", |_| true);
    request_images_with_spawn(&mut state, &adapter, sid, "mv-1", |_| true);
    adapter.images.post(
        (0, ImagesAnswer::List(Some(page(vec![picture("https://img.example/stale.jpg")], 1)))),
        |_| true,
    );
    assert!(!pump_images(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert_eq!(state.images.status(), ImagesStatus::Loading, "still waiting");
    assert!(state.images.images().is_empty());
}

/// Closing the page drops the pictures: they are the departing item's, and the write they would
/// perform names its id.
#[test]
fn closing_the_page_drops_the_pictures() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    let mut gen = 0;
    request_images_with_spawn(&mut state, &adapter, sid, "mv-1", |g| {
        gen = g;
        true
    });
    adapter.images.post(
        (
            gen,
            ImagesAnswer::List(Some(page(vec![picture("https://img.example/a.jpg")], 1))),
        ),
        |old| old.0 < gen,
    );
    assert!(pump_images(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert!(!state.images.images().is_empty());

    clear(&mut state, &adapter);
    assert_eq!(state.images.status(), ImagesStatus::Idle);
    assert!(state.images.images().is_empty());
    assert_eq!(state.images.key(), None);
}

// ---- the local intros (#370) -------------------------------------------------------------------

/// The intros land against the item they are about, and the first one is the one a press plays.
#[test]
fn intros_land_against_the_item_and_the_first_is_the_one_offered() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    let mut gen = 0;
    request_intros_with_spawn(&mut state, &adapter, sid, "mv-1", |g| {
        gen = g;
        true
    });
    assert!(state.intros.first().is_none(), "nothing is offered while the request is out");
    assert!(state.intros.is_about(sid, "mv-1"), "the request names the item it is about");

    adapter.intros.post(
        (
            gen,
            vec![
                crate::jellyfin::intros::intro_for_test("intro-1", "Studio logo"),
                crate::jellyfin::intros::intro_for_test("intro-2", "Second clip"),
            ],
        ),
        |old| old.0 < gen,
    );
    assert!(pump_intros(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert_eq!(state.intros.first().map(|i| i.title.as_str()), Some("Studio logo"));
    assert_eq!(state.intros.first().map(|i| i.rk.as_str()), Some("intro-1"));
    assert!(!state.intros.is_about(sid, "mv-2"), "and never another item's");
}

/// An EMPTY answer is the ordinary one on a server without the plugin, and it is not an error: the
/// window draws nothing for it, which is what "no intros" means.
#[test]
fn an_empty_answer_offers_nothing() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    let mut gen = 0;
    request_intros_with_spawn(&mut state, &adapter, sid, "mv-1", |g| {
        gen = g;
        true
    });
    adapter.intros.post((gen, Vec::new()), |old| old.0 < gen);
    assert!(pump_intros(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert!(state.intros.first().is_none());
    assert!(state.intros.is_about(sid, "mv-1"), "an empty answer still names its item");
}

/// A REFUSED spawn leaves the empty list `begin` installed, which already means "no intro": there
/// is nothing to settle and no spinner to strand.
#[test]
fn a_refused_spawn_offers_nothing() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    request_intros_with_spawn(&mut state, &adapter, sid, "mv-1", |_| false);
    assert!(state.intros.first().is_none());
    assert!(!adapter.intros.busy(), "the claim is released, so a retry can spawn");
}

/// Closing the page drops the intros: they are the departing item's.
#[test]
fn closing_the_page_drops_the_intros() {
    let _g = crate::testlock::serial();
    let sid = ServerId::from_raw(1);
    let (mut state, adapter) = loaded(sid, "mv-1");
    let mut gen = 0;
    request_intros_with_spawn(&mut state, &adapter, sid, "mv-1", |g| {
        gen = g;
        true
    });
    adapter.intros.post(
        (gen, vec![crate::jellyfin::intros::intro_for_test("intro-1", "Studio logo")]),
        |old| old.0 < gen,
    );
    assert!(pump_intros(&mut state, &adapter, &crate::ui::landgate::fixture_gate()));
    assert!(state.intros.first().is_some());

    clear(&mut state, &adapter);
    assert!(state.intros.first().is_none());
    assert!(!state.intros.is_about(sid, "mv-1"));
}
