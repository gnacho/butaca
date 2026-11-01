//! The three-state fetch machine (section, source, and the (source, section) table):
//! Loading/Ready/Failed derivation and multi-source table identity.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;

/// THE bug, one layer up: `ensure_sections` folded every failure into an empty table, so the
/// screen saw exactly what it sees before the first request — no section, no state, and
/// `fetch_state()` answering `Loading` out of its `unwrap_or` — and spun forever with no way
/// out. A source that did not answer must be a state the screen can SEE.
#[test]
fn a_source_that_did_not_answer_is_observable_rather_than_an_eternal_spinner() {
    let _g = crate::testlock::serial();
    let mut browse = TestBrowse::default();
    seed_one_source(&mut browse, true, false);
    assert_eq!(
        browse.state.cur_source_state(),
        SecFetch::Loading,
        "nobody has asked it anything yet"
    );
    browse.state.source_mut(0).unwrap().set_reachable(false);
    assert_eq!(
        browse.state.cur_source_state(),
        SecFetch::Failed,
        "the screen must be able to see this"
    );
}
/// An account with nothing we browse ANSWERED. `Ready` with no sections, never `Failed` — the
/// same reason an empty listing is (`StatusKind::Empty`), and the case that lands in the very
/// same two `unwrap_or` defaults as a failure and so used to spin identically.
#[test]
fn a_source_with_no_browsable_library_answered_and_did_not_fail() {
    let _g = crate::testlock::serial();
    let mut browse = TestBrowse::default();
    seed_one_source(&mut browse, true, true);
    assert_eq!(
        browse.state.cur_source_state(),
        SecFetch::Ready,
        "an empty answer is an answer"
    );
    assert_eq!(browse.section_count(), 0);
}
/// A served table clears a previous failure and seeds one state per section, and from then on
/// the state is read off the SECTION's source rather than off the current server.
#[test]
fn a_served_table_clears_the_failure_and_seeds_its_states() {
    let _g = crate::testlock::serial();
    let mut browse = TestBrowse::default();
    seed_one_source(&mut browse, false, false);
    assert_eq!(browse.state.cur_source_state(), SecFetch::Failed);
    {
        let s = browse.state.source_mut(0).unwrap();
        s.set_reachable(true);
        s.sections_done = true;
    }
    browse.append_sections(
        0,
        vec![
            (1, "Movies".into(), SecKind::Movie),
            (2, "Film Club".into(), SecKind::Movie),
        ],
    );
    assert_eq!(browse.state.cur_source_state(), SecFetch::Ready);
    assert_eq!(browse.section_count(), 2);
    assert!(browse.loading_initial(), "a fresh section has not answered yet");
}
/// THE reason the table gained a source dimension. Measured against the real share on
/// 2026-08-11: both servers have a section `1`, and they are different libraries. A bare key
/// names two things, so every row carries its source and the two rows coexist.
#[test]
fn two_servers_both_have_a_section_one_and_the_table_tells_them_apart() {
    let _g = crate::testlock::serial();
    let mut browse = TestBrowse::default();
    browse.seed_sources(vec![
        a_source("mac-mini", "", true),
        a_source("nas-home", "friend", true),
    ]);
    browse.append_sections(
        0,
        vec![
            (1, "Movies".into(), SecKind::Movie),
            (2, "TV Shows".into(), SecKind::Show),
        ],
    );
    browse.append_sections(1, vec![(1, "Film Club".into(), SecKind::Movie)]);

    assert_eq!(browse.section_count(), 3);
    let ours = &browse.state.sections()[0];
    let theirs = &browse.state.sections()[2];
    assert_eq!((ours.key, ours.src), (1, 0), "our section 1, on source 0");
    assert_eq!(
        (theirs.key, theirs.src),
        (1, 1),
        "THEIR section 1 — same key, different source"
    );
    assert_eq!(
        (browse.section_title(0), browse.section_title(2)),
        ("Movies", "Film Club")
    );
    // and the pill label's handle run follows the section being browsed, not the account
    assert_eq!(browse.state.sources()[theirs.src].handle, "friend");
    assert_eq!(browse.state.sources()[ours.src].handle, "",
        "your own libraries carry no owner at all");
}
/// A source discovered LATE must never move an existing index. `PageResult.sec` is a section
/// index, so a table that reshuffled under an in-flight fetch would splice one library's items
/// into another's store — the soundness the old `ensure_sections` early-return provided and
/// APPEND-ONLY now provides for every source rather than only for the second call.
#[test]
fn a_source_arriving_late_appends_and_moves_no_existing_index() {
    let _g = crate::testlock::serial();
    // A landing re-derives every row's favourite and, since 2026-09-05, REPOINTS `cur`
    // when that takes the current section's away — so this test's own subject (an index
    // holding still) is only well-defined against a known favourite set. Without a
    // scratch session it grades whatever `auth.json` a neighbouring test left behind.
    let _t = TempPins::new("late-append");
    _t.watching("u-late-append");
    let mut browse = TestBrowse::default();
    browse.seed_sources(vec![
        a_source("mac-mini", "", true),
        a_source("nas-home", "friend", true),
    ]);
    browse.append_sections(
        0,
        vec![
            (1, "Movies".into(), SecKind::Movie),
            (2, "TV Shows".into(), SecKind::Show),
        ],
    );
    browse.state.set_cur(1);
    let before = (
        browse.state.cur(),
        browse.section_title(1).to_string(),
        browse.state.states().len(),
    );

    browse.append_sections(1, vec![(1, "Film Club".into(), SecKind::Movie)]);
    assert_eq!(
        browse.state.cur(),
        before.0,
        "the library being browsed is still the one at that index"
    );
    assert_eq!(browse.section_title(1), before.1);
    assert_eq!(
        browse.state.states().len(),
        browse.section_count(),
        "states stay in lockstep with the table"
    );
    assert_eq!(browse.state.states().len(), before.2 + 1);

    // A RE-discovery ("Check for new shares", or a server that came back) re-offers the same
    // list: every row is already there, so nothing is duplicated and nothing moves…
    browse.append_sections(1, vec![(1, "Film Club".into(), SecKind::Movie)]);
    assert_eq!(browse.section_count(), 3);
    assert_eq!(browse.state.cur(), before.0);
    // …while a library the owner has CREATED since is appended, at the end, where it cannot
    // disturb an index anything is already holding.
    browse.append_sections(
        1,
        vec![
            (1, "Film Club".into(), SecKind::Movie),
            (4, "Club Shows".into(), SecKind::Show),
        ],
    );
    assert_eq!(browse.section_count(), 4);
    assert_eq!(browse.section_title(3), "Club Shows");
    assert_eq!(
        browse.section_title(1),
        before.1,
        "and the row we were browsing is untouched"
    );
}

/// `SecKind` is the closed, exhaustively-matched gate the "Plays" sort is added behind — and
/// it has no `Artist`/`Photo` variant at all: `SecKind::from_wire` already drops those
/// directory types before they can become a `BrowseSection`, so there is no path into the
/// augmentation for them, ever, regardless of this change. This is the invariant the gate
/// relies on rather than a guessed string comparison.
#[test]
fn artist_and_photo_sections_have_no_seckind_and_so_cannot_reach_the_plays_sort_gate() {
    assert_eq!(SecKind::from_wire("artist"), None);
    assert_eq!(SecKind::from_wire("photo"), None);
}
