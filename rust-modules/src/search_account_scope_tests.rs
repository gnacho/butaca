//! Account/profile slot scoping and favourite-edit re-arming of a resident query.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;

/// **A favourite edit under a resident query supersedes it and re-arms the fetch.** Search
/// watched the ROSTER generation alone and the favourite answer moves without it — discovery
/// appending a library, and `apply_pins` recording an edit, both bump `SECTIONS_GEN`, and this
/// screen deliberately runs discovery immediately before its own pump.
///
/// It re-ARMS rather than re-sorting, and that is not a preference: after the fold a `TagHit`
/// no longer carries the section its bit came from (`TagHit::fav`), so the bit cannot be
/// re-derived from what is on screen. Re-arming is cheaper than retaining full
/// contributing-section provenance through both folds.
#[test]
fn a_favourite_edit_supersedes_a_resident_query_and_re_arms_it() {
    let _g = fresh();
    let mut owner = Owner::default();
    let _t = crate::server::session::TempSession::new("search-favgen");
    _t.watching("u-search-favgen");
    register(&mut owner, 1);
    let stores = crate::stores::Stores::default();
    let mut directory = crate::stores::browse::DirectorySnapshot::default();
    stores.capture_browse(&mut directory);
    owner.set_query_from_directory("wallace", directory.view());
    hold_off(&mut owner);
    owner.pump_with_directory(SETTLE_S + 0.1, directory.view()); // settles and takes the snapshot's generation
    let gen0 = owner.state.gen;
    owner.state.armed = false;

    // …a library lands, which is what `apply_pins` and discovery both look like from here
    stores.browse.borrow_mut().seed_two_source_table_for_test();
    stores.capture_browse(&mut directory);
    hold_off(&mut owner);
    owner.pump_with_directory(0.016, directory.view());
    assert_ne!(
        owner.state.gen,
        gen0,
        "the resident answer was superseded, so a landing under the old table is discarded"
    );
    assert!(
        owner.state.armed,
        "…and the query is owed a fresh fetch rather than left on stale shelves"
    );
    assert_eq!(
        owner.state.fav_gen,
        directory.view().sections_gen(),
        "the snapshot moved with it"
    );

    // …and a SECOND pump with nothing further changed must not re-arm again, or every frame
    // after any edit would supersede the query it just started
    hold_off(&mut owner);
    let gen1 = owner.state.gen;
    owner.pump_with_directory(0.016, directory.view());
    assert_eq!(owner.state.gen, gen1, "it settles");
}

/// The other half, and the one that costs nothing to get wrong until a user types: with NO
/// query resident there is nothing to invalidate, so the snapshot is merely brought up to date.
/// Superseding here would mean the first query after any library landing opened by discarding
/// itself.
#[test]
fn a_favourite_edit_with_no_query_resident_only_refreshes_the_snapshot() {
    let _g = fresh();
    let mut owner = Owner::default();
    let _t = crate::server::session::TempSession::new("search-favgen-idle");
    _t.watching("u-search-favgen-idle");
    register(&mut owner, 1);
    let stores = crate::stores::Stores::default();
    let mut directory = crate::stores::browse::DirectorySnapshot::default();
    stores.capture_browse(&mut directory);
    hold_off(&mut owner);
    owner.pump_with_directory(0.016, directory.view());
    let gen0 = owner.state.gen;

    stores.browse.borrow_mut().seed_two_source_table_for_test();
    stores.capture_browse(&mut directory);
    hold_off(&mut owner);
    owner.pump_with_directory(0.016, directory.view());
    assert_eq!(
        owner.state.gen,
        gen0,
        "nothing was resident, so nothing was superseded"
    );
    assert_eq!(
        owner.state.fav_gen,
        directory.view().sections_gen(),
        "…but the snapshot is current, so the next query does not open by re-arming"
    );
}
