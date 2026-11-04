//! A library's sort survives a restart (GitHub #278): *"sort order should persist between
//! restarts. I have to change it from the default (title) every time I fire it up."*
//!
//! Graded end to end against a loopback server: the choice goes through the Library's own commit
//! (`QueryEdit::Sort`), reaches the session file, and a FRESH store — a cold session cache read
//! back from disk, a new `BrowseState`, a section whose menu nobody has seen yet this run — must
//! open that library in the remembered order.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;

/// The sort is the PROFILE's: another person on the same television opens the same library in
/// the default order, and a profile switch (a store reset) drops the previous person's memory.
#[test]
fn a_remembered_sort_belongs_to_the_profile_that_chose_it() {
    let _g = crate::testlock::serial();
    let session = TempPins::new("sort-memory-profile");
    session.watching("u-sorter");
    crate::server::session::update(|current| {
        let mut next = current.clone();
        next.set_sort_for("u-sorter", "mac-mini", 1, Some(("viewCount", true)));
        Some(next)
    });
    let mut browse = TestBrowse::default();
    browse.seed_sources(vec![a_source("mac-mini", "", true)]);
    browse.append_sections(0, vec![(1, "Movies".into(), SecKind::Movie)]);
    assert_eq!(
        browse.state.sort_memory.get("mac-mini", 1).map(|(sort, desc)| (sort.to_string(), desc)),
        Some(("viewCount".to_string(), true)));

    browse.reset();
    assert!(browse.state.sort_memory.get("mac-mini", 1).is_none(),
        "a reset forgets the previous profile's sorts");
    session.watching("u-other");
    browse.seed_sources(vec![a_source("mac-mini", "", true)]);
    browse.append_sections(0, vec![(1, "Movies".into(), SecKind::Movie)]);
    assert!(browse.state.sort_memory.get("mac-mini", 1).is_none(),
        "another profile opens it in the default order");
}

/// Choosing the default order back FORGETS the entry rather than recording it, and a non-primary
/// view (Collections of a movie library) is never remembered. The menu is seeded directly — the
/// landing that used to deliver it was the server page worker, retired with the server browse arm
/// (#166 pass 3); what this grades is the recording rule, which is backend-blind.
#[test]
fn choosing_the_default_order_forgets_the_entry() {
    let _g = crate::testlock::serial();
    let session = TempPins::new("sort-memory-default");
    session.watching("u-sorter");
    let mut browse = TestBrowse::default();
    browse.seed_sources(vec![a_source("mac-mini", "", true)]);
    browse.append_sections(0, vec![(1, "Movies".into(), SecKind::Movie)]);
    let menu = |browse: &mut TestBrowse, entries: &[(&str, bool)]| {
        browse.state.states[0].sorts = std::sync::Arc::new(entries.iter()
            .map(|(key, desc)| SortEntry {
                key: (*key).into(), title: (*key).into(), default_desc: *desc,
            })
            .collect());
    };
    let commit = |browse: &mut TestBrowse, key: &str, desc: bool| {
        let target = crate::stores::browse::SectionAddress {
            epoch: browse.state.table_epoch(),
            sid: browse.state.section_sid(0).unwrap(),
            section: 1,
        };
        assert!(browse.state.addressed_with_adapter(&browse.adapter, target,
            crate::stores::browse::LibraryWork::Commit {
                select: false, choice: false,
                query: Some(crate::stores::browse::QueryEdit::Sort { key: key.into(), desc }),
            }));
        crate::storage_worker::drain_for_test();
    };
    let held = |browse: &TestBrowse| {
        let machine = browse.state.sources()[0].machine_id.clone();
        crate::server::session::peek().sorts_for("u-sorter")
            .and_then(|sorts| sorts.get(&machine, 1).map(|(sort, desc)| (sort.to_string(), desc)))
    };
    menu(&mut browse, &[("SortName", false), ("addedAt", true)]);
    commit(&mut browse, "addedAt", true);
    assert_eq!(held(&browse), Some(("addedAt".to_string(), true)));
    // The menu re-landed, as a real page would deliver it; choosing the default order back
    // FORGETS the entry rather than recording it.
    menu(&mut browse, &[("SortName", false)]);
    commit(&mut browse, "SortName", false);
    assert_eq!(held(&browse), None, "the default order needs no record");
    assert!(crate::server::session::peek().library_sorts.is_empty(),
        "and a profile with nothing remembered leaves no empty record behind");
    // A Collections view's sort is that view's menu, not the library's: never remembered.
    assert!(browse.state.set_library_type(LibraryType::Collections));
    menu(&mut browse, &[("SortName", false)]);
    commit(&mut browse, "SortName", true);
    assert_eq!(held(&browse), None);
}

/// The per-profile list is bounded: the most recently sorted libraries are kept.
#[test]
fn the_remembered_sorts_are_capped_most_recent_first() {
    use crate::server::session::LibrarySorts;
    let mut sorts = LibrarySorts::default();
    for key in 0..(LibrarySorts::CAP as i64 + 5) {
        sorts.set("m", key, Some(("addedAt", true)));
    }
    assert_eq!(sorts.libs.len(), LibrarySorts::CAP);
    assert!(sorts.get("m", 0).is_none(), "the oldest entries are evicted");
    assert!(sorts.get("m", LibrarySorts::CAP as i64 + 4).is_some());
    // re-sorting an old library refreshes it rather than duplicating it
    sorts.set("m", 5, Some(("titleSort", true)));
    assert_eq!(sorts.libs.len(), LibrarySorts::CAP);
    assert_eq!(sorts.libs.last().map(|lib| lib.key), Some(5));
    // nameless libraries and oversized keys are never recorded
    sorts.set("", 1, Some(("addedAt", true)));
    assert!(sorts.get("", 1).is_none());
    sorts.set("m", 99, Some((&"k".repeat(200), true)));
    assert!(sorts.get("m", 99).is_none());
}
