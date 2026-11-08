//! **#367: a press on a poster ends with the pre-playback window on screen.**
//!
//! The window is built from a loaded `metadata::Detail` and a card press carries only an item id,
//! so the chain the owner asked for is the one under test here: the press asks for the item's page,
//! the page fetches the item, and the window opens over the page the moment that item lands,
//! without the viewer finding Play a second time and without anything playing. The screen-level
//! halves live in `screens/detail/tests.rs`; this file pins the seam between them, which is what a
//! press actually travels.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;
use super::test_support::{detail_arg, frame, tick};

fn movie(sid: crate::registry::ServerId, rk: &str) -> crate::metadata::Detail {
    crate::metadata::Detail {
        sid,
        rk: rk.into(),
        kind: "movie".into(),
        title: "Movie".into(),
        part: format!("/library/parts/{rk}"),
        ..Default::default()
    }
}

/// Land the item's fetch the way the worker would, then tell the mounted pages about it.
///
/// The request is minted here rather than borrowed from the page's own: this helper describes the
/// LANDING, and the press under test is not required to have issued a fetch of its own.
fn land(
    d: &mut Dispatcher<AppHost>, rig: &mut Bridge,
    sid: crate::registry::ServerId, rk: &str, detail: crate::metadata::Detail,
) {
    let gen = crate::metadata::begin_detail_for_test(rig.metadata_mut().adapter_ref(), sid, rk);
    let installed = {
        let (state, adapter) = rig.metadata_mut().split_for_test();
        crate::metadata::land_detail_for_test(state, adapter, sid, rk, gen, Some(detail))
    };
    assert!(installed, "the fetch this page asked for is the one that lands");
    d.store_changed(StoreId::Metadata.ord(), rig.metadata_mut().gen());
}

/// Everything the mounted pages have asked the loop for since the last drain.
fn requests(rig: &mut Bridge) -> Vec<crate::screens::registry::ContentReq> {
    rig.take_content_reqs().into_iter().map(|(_, request, _)| request).collect()
}

/// Did the page ask the loop for the pre-playback window?
fn window_requested(asked: &[crate::screens::registry::ContentReq]) -> bool {
    asked.iter().any(|request| matches!(
        request,
        crate::screens::registry::ContentReq::Panel(
            crate::screens::registry::ContentPanel::PrePlayback { restart: false })
    ))
}

/// Did anything ask the loop to start a film?
fn play_requested(asked: &[crate::screens::registry::ContentReq]) -> bool {
    asked.iter().any(|request| matches!(request, crate::screens::registry::ContentReq::Play { .. }))
}

/// Present the window the way the loop's own `ContentReq::Panel` arm does: over the top page, with
/// that page as the subject.
fn present_window(d: &mut Dispatcher<AppHost>, sid: crate::registry::ServerId, rk: &str) {
    let host = d.nav.top_page().and_then(|e| e.inst.as_ref()).map(|i| i.id)
        .expect("the page the window opens over is mounted");
    open_content_panel(d, host, Some((sid, rk)),
        crate::screens::registry::ContentPanel::PrePlayback { restart: false });
}

/// **A press on a film's poster opens the window over the item's own page, and BACK gives the page
/// back.**
///
/// Driven through the press seam (`open_detail_from_card`, what `activate_card` calls for a film
/// card on Home or the Library), the real mount, a real landing and the real container: the window
/// is a surface over the page, so it never replaces it and BACK dismisses it. Nothing in the path
/// starts the film; the page's own Play is still where a play is decided.
#[test]
fn a_poster_press_opens_the_window_once_its_item_lands() {
    let _g = crate::testlock::serial();
    let sid = crate::registry::ServerId::UNSET;
    let mut d = Dispatcher::<AppHost>::new();
    let mut rig = Bridge::for_test(|| 0);
    let mut t = 0u32;

    // The press: a card, an id, and nothing else. The page mounts while the store holds no item
    // for it, which is the shape the window cannot be built from.
    open_detail_from_card(&mut d, &mut rig, sid, "m1", None, None);
    for _ in 0..6 {
        t += 1;
        frame(&mut d, &mut rig, detail_arg("m1"), tick(t), vec![]);
    }
    assert!(d.nav.top_page().is_some_and(|e| e.arg == detail_arg("m1")), "the press lands on the page");
    assert!(!window_requested(&requests(&mut rig)), "nothing is loaded yet, so there is no window to open");

    // The item lands.
    land(&mut d, &mut rig, sid, "m1", movie(sid, "m1"));
    for _ in 0..3 {
        t += 1;
        frame(&mut d, &mut rig, detail_arg("m1"), tick(t), vec![]);
    }
    let asked = requests(&mut rig);
    assert!(window_requested(&asked), "the landing asks for the window");
    assert!(!play_requested(&asked), "the landing never starts playback");

    present_window(&mut d, sid, "m1");
    for _ in 0..6 {
        t += 1;
        frame(&mut d, &mut rig, detail_arg("m1"), tick(t), vec![]);
    }
    assert_eq!(overlay_word(&d), Some("preplay"), "the window is on screen");
    assert!(
        d.nav.top_page().is_some_and(|e| e.arg == detail_arg("m1")),
        "the window is presented OVER the page, it does not replace it"
    );

    // BACK dismisses the window and gives the page back, still not playing anything. The surface
    // starts its closing transition on the press (as every surface does) and the loop's own
    // `ContentReq::Back` arm completes the dismissal; what matters here is that the press asked
    // for exactly that and for nothing else.
    t += 1;
    frame(&mut d, &mut rig, detail_arg("m1"), tick(t), script_key(Key::Back, tick(t)));
    for _ in 0..6 {
        t += 1;
        frame(&mut d, &mut rig, detail_arg("m1"), tick(t), vec![]);
    }
    assert_eq!(
        d.nav.modals.top().map(|s| s.phase),
        Some(crate::ui::containers::modal::Phase::Closing),
        "BACK dismisses the window"
    );
    assert!(d.nav.top_page().is_some_and(|e| e.arg == detail_arg("m1")), "and leaves the viewer on the page");
    assert!(!play_requested(&requests(&mut rig)), "dismissing the window plays nothing");
}

/// **The arm belongs to the press that named the item**, so a page the press did not ask for never
/// asks for a window, and the page the press DID ask for still asks once the viewer is back on it.
///
/// This is what keeps every other way onto a detail page (a menu row, a deep link, a boot trigger,
/// a return from playback) exactly as it was: they never arm, and an arm for another item is never
/// handed to them.
#[test]
fn only_the_press_that_named_the_item_opens_its_window() {
    let _g = crate::testlock::serial();
    let sid = crate::registry::ServerId::UNSET;
    let mut d = Dispatcher::<AppHost>::new();
    let mut rig = Bridge::for_test(|| 0);
    let mut t = 0u32;

    open_detail_from_card(&mut d, &mut rig, sid, "m1", None, None);
    for _ in 0..4 {
        t += 1;
        frame(&mut d, &mut rig, detail_arg("m1"), tick(t), vec![]);
    }
    // Something else pushes a different page over it before the item lands.
    nav_push(&mut d, detail_arg("m2"));
    for _ in 0..6 {
        t += 1;
        frame(&mut d, &mut rig, detail_arg("m2"), tick(t), vec![]);
    }
    land(&mut d, &mut rig, sid, "m2", movie(sid, "m2"));
    for _ in 0..3 {
        t += 1;
        frame(&mut d, &mut rig, detail_arg("m2"), tick(t), vec![]);
    }
    assert!(!window_requested(&requests(&mut rig)), "the page the press did not name never asks for the window");

    // And the page the press named still asks once the viewer is back on it.
    nav_pop(&mut d);
    for _ in 0..6 {
        t += 1;
        frame(&mut d, &mut rig, detail_arg("m1"), tick(t), vec![]);
    }
    land(&mut d, &mut rig, sid, "m1", movie(sid, "m1"));
    for _ in 0..3 {
        t += 1;
        frame(&mut d, &mut rig, detail_arg("m1"), tick(t), vec![]);
    }
    assert!(window_requested(&requests(&mut rig)), "the press's own page still asks for it");
}
