//! Cross-source shelf merge, favourite ranking, and tag shelf building.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;

/// The same person on two SERVERS is one person. `project` folds per response, so it
/// cannot see across sources — [`merge`] is the only place both are in hand, and until it did
/// this a shared actor drew twice with a split count while `same_tag`'s own doc claimed the
/// round-robin brought them together.
#[test]
fn one_person_on_two_servers_is_one_row_after_the_merge() {
    let mk = |sid: u16, id: &str, thumb: &str, count: i64| {
        let mut p: Projection = Default::default();
        p[3].push(Item::Tag(TagHit {
            sid: ServerId::from_raw(sid),
            name: "Wallace Shawn".into(),
            tag_key: "gid-1".into(),
            id: id.into(),
            thumb: thumb.into(),
            count,
            ..Default::default()
        }));
        Source {
            status: Status::Answered,
            items: p,
            ..Source::EMPTY
        }
    };
    // Same guid, DIFFERENT local ids (they are server-local) and only one carries a face.
    let sh = merge_favs(&[mk(0, "921", "", 5), mk(1, "4471", "/t.jpg", 3)]);
    let people = &sh
        .iter()
        .find(|s| s.kind == Kind::Person)
        .expect("a Person shelf")
        .items;
    assert_eq!(people.len(), 1, "one person, not one per server");
    let Item::Tag(t) = &people[0] else {
        panic!("a person is a Tag row")
    };
    assert_eq!(t.count, 8, "their credits across both servers");
    assert_eq!(t.thumb, "/t.jpg", "the server with a face supplies it");
}

// ---- favourites RANK, and never filter (§6) --------------------------------------------
//
// The owner's direction was that the Favorite libraries switch affects the whole app, and the
// first reading of that was to REMOVE a non-favourite library's hits from Search. These grade
// the reading that shipped instead, and the four assertions are the argument for it: a
// non-favourite result is still here, it is merely later; its COUNT is untouched; a tag that
// straddles the two ranks favourite; and each server's own ranking survives the pass.

/// A media hit in a favourite library outranks one in a library switched off — **and the
/// switched-off one is still on the shelf.** Removing it would turn "I don't browse this
/// often" into "this does not exist" for a film the user owns and can play.
#[test]
fn a_non_favourite_library_is_ranked_later_and_never_removed() {
    let sid = ServerId::from_raw(0);
    let mk = |title: &str, sec: i64| {
        Item::Media(PmsMovie {
            sid,
            sec,
            title: title.into(),
            ..Default::default()
        })
    };
    let mut p: Projection = Default::default();
    // the SERVER's own order puts the non-favourite first, which is what makes this a test of
    // ranking rather than of the round robin
    p[0].push(mk("Off Shelf", 9));
    p[0].push(mk("Fav Shelf", 1));
    let sources = [Source {
        status: Status::Answered,
        items: p,
        ..Source::EMPTY
    }];
    let favs = [(sid, 1i64, true), (sid, 9i64, false)];
    let sh = merge(&sources, &favs);
    let titles: Vec<&str> = sh[0].items.iter().map(|i| i.title()).collect();
    assert_eq!(
        titles,
        vec!["Fav Shelf", "Off Shelf"],
        "the favourite is lifted and the other is still there"
    );
}

/// **Within a pass the server's own ranking survives**, because the only ranking any server
/// hands over is the order of its own list. Two favourites from one server keep their order,
/// and the round robin between servers keeps its interleave — the sort is STABLE and must not
/// become a comparison that reorders inside a group.
#[test]
fn ranking_is_stable_so_each_servers_own_order_survives_the_pass() {
    let mk = |sid: u16, title: &str, sec: i64| {
        Item::Media(PmsMovie {
            sid: ServerId::from_raw(sid),
            sec,
            title: title.into(),
            ..Default::default()
        })
    };
    // the source's OWN sid rides on each item; the store keys a slot by array position, which
    // is what the two calls below stand in for
    let src = |rows: Vec<Item>| {
        let mut p: Projection = Default::default();
        p[0] = rows;
        Source {
            status: Status::Answered,
            items: p,
            ..Source::EMPTY
        }
    };
    let a = src(vec![mk(0, "A-off", 9), mk(0, "A-fav1", 1), mk(0, "A-fav2", 1)]);
    let b = src(vec![mk(1, "B-fav", 1), mk(1, "B-off", 9)]);
    let favs = [
        (ServerId::from_raw(0), 1i64, true),
        (ServerId::from_raw(0), 9i64, false),
        (ServerId::from_raw(1), 1i64, true),
        (ServerId::from_raw(1), 9i64, false),
    ];
    let sh = merge(&[a, b], &favs);
    let titles: Vec<&str> = sh[0].items.iter().map(|i| i.title()).collect();
    // round robin gives A-off, B-fav, A-fav1, B-off, A-fav2; the stable lift keeps that order
    // inside each group
    assert_eq!(
        titles,
        vec!["B-fav", "A-fav1", "A-fav2", "A-off", "B-off"],
        "favourites first, each in the order the round robin produced them"
    );
}

/// **A tag that straddles the two is favourite-ranked, and its count stays grant-wide.** The
/// bit is OR'd at the fold; the number is summed. Those are different operations on one row
/// and conflating them is how "12 films" would silently become 5.
#[test]
fn a_tag_in_a_favourite_and_a_non_favourite_library_ranks_favourite_and_keeps_its_whole_count()
{
    let sid = ServerId::from_raw(0);
    // The same person as two rows, one per library section: the favourite first, as the
    // round robin would deliver the sections.
    let row = |count: i64, fav: bool| Item::Tag(TagHit {
        sid,
        name: "Wallace Shawn".into(),
        tag_key: "gid-1".into(),
        id: "921".into(),
        count,
        fav,
        ..Default::default()
    });
    let mut p: Projection = Default::default();
    p[3].push(row(5, false));
    p[3].push(row(3, true));
    let sources = [Source {
        status: Status::Answered,
        items: p,
        ..Source::EMPTY
    }];
    let favs = [(sid, 1i64, true), (sid, 9i64, false)];
    let sh = merge(&sources, &favs);
    let people = &sh.iter().find(|s| s.kind == Kind::Person)
        .expect("a Person shelf").items;
    assert_eq!(people.len(), 1, "one person, folded across the two rows");
    let Item::Tag(t) = &people[0] else {
        panic!("a person is a Tag row")
    };
    assert!(t.fav, "one favourite section is enough to rank the row");
    assert_eq!(t.count, 8, "…and the count is every credit, both sections");
}

/// **The cap bounds what is DRAWN, not what is COUNTED.** The fill used to `break` out the
/// moment the shelf was full, so once the favourite pass had filled it a later duplicate could
/// no longer augment an already displayed tag — a person's total silently became however many
/// the servers reported before the cap. The fold now runs to completion and the truncation is
/// last.
#[test]
fn a_duplicate_past_the_shelf_cap_still_augments_a_displayed_tags_count() {
    let sid = ServerId::from_raw(0);
    let person = |name: &str, key: &str, count: i64| {
        Item::Tag(TagHit {
            sid,
            name: name.into(),
            tag_key: key.into(),
            count,
            fav: true,
            ..Default::default()
        })
    };
    // Server A leads with our subject, so it is certainly drawn, and fills out behind it.
    let mut a: Projection = Default::default();
    a[3].push(person("Wallace Shawn", "gid-1", 5));
    for i in 1..SHELF_MAX {
        a[3].push(person(&format!("A filler {i}"), &format!("gid-a{i}"), 1));
    }
    // Server B repeats the subject as its LAST row. The merge is round robin by DEPTH, so the
    // shelf is full around depth `SHELF_MAX / 2` and this duplicate is not even LOOKED at until
    // depth `SHELF_MAX - 1` — which is the whole point: under the old `break` the fill had long
    // since stopped and this row was never folded at all.
    let mut b: Projection = Default::default();
    for i in 1..SHELF_MAX {
        b[3].push(person(&format!("B filler {i}"), &format!("gid-b{i}"), 1));
    }
    b[3].push(person("Wallace Shawn", "gid-1", 3));
    let src = |items: Projection| Source {
        status: Status::Answered,
        items,
        ..Source::EMPTY
    };
    let sh = merge(&[src(a), src(b)], NO_FAVS);
    let people = &sh
        .iter()
        .find(|s| s.kind == Kind::Person)
        .expect("a Person shelf")
        .items;
    assert_eq!(people.len(), SHELF_MAX, "the shelf is capped as it always was");
    let Item::Tag(t) = &people[0] else {
        panic!("a person is a Tag row")
    };
    assert_eq!(
        t.count, 8,
        "the fold outlived the cap: 5 + 3, not the 5 a break would have left"
    );
}

/// An unclassified row ranks as a FAVOURITE, in both directions — the server said nothing
/// about its library, or the section table has not enumerated it yet. Demoting what cannot be
/// classified would push a user's own results down the shelf on the frame the app boots, which
/// is the same trap `pms::item_pinned` documents at the same join.
#[test]
fn an_unclassified_row_ranks_as_a_favourite_rather_than_being_demoted() {
    let sid = ServerId::from_raw(0);
    let favs = [(sid, 1i64, true), (sid, 9i64, false)];
    assert!(
        section_is_fav(&favs, sid, 0),
        "the server sent no librarySectionID"
    );
    assert!(
        section_is_fav(&favs, sid, 77),
        "a library nobody has enumerated yet"
    );
    assert!(!section_is_fav(&favs, sid, 9), "…and a known Off one is not");
    assert!(
        section_is_fav(&favs, ServerId::from_raw(1), 9),
        "a section key is server-local: another server's 9 is not this one's"
    );
}

/// **A person in two libraries is one person.** A source can carry the same actor as two rows
/// with the same identity and each section's own `count` — which drew the same face twice, and
/// would have reported 5 credits for someone with 8 had the duplicate simply been dropped.
/// Device-observed before it was fixed.
#[test]
fn a_tag_that_arrives_once_per_library_section_folds_into_one_row_with_the_counts_summed() {
    let rows = vec![
        // the Movies section: no artwork on this row
        Item::Tag(TagHit {
            sid: ServerId::UNSET,
            name: "Wallace Shawn".into(),
            id: "921".into(),
            tag_key: "gid-1".into(),
            count: 5,
            ..Default::default()
        }),
        // …and the TV Shows section, same person, and this is the row with a headshot
        Item::Tag(TagHit {
            sid: ServerId::UNSET,
            name: "Wallace Shawn".into(),
            id: "4471".into(),
            tag_key: "gid-1".into(),
            count: 3,
            thumb: "/t.jpg".into(),
            ..Default::default()
        }),
        // a different person who happens to share the FIRST one's local id in another section
        Item::Tag(TagHit {
            sid: ServerId::UNSET,
            name: "Dee Wallace".into(),
            id: "921".into(),
            tag_key: "gid-2".into(),
            count: 2,
            ..Default::default()
        }),
    ];
    let sh = merge_favs(&[answered(3, rows)]);

    assert_eq!(
        sh.iter().find(|s| s.kind == Kind::Person).expect("a Person shelf")
            .items.iter().map(|i| i.title()).collect::<Vec<_>>(),
        ["Wallace Shawn", "Dee Wallace"],
        "one row per PERSON, and a shared local id does not merge two of them"
    );
    let Item::Tag(first) = &sh.iter().find(|s| s.kind == Kind::Person).unwrap().items[0] else {
        panic!("a person is a Tag row")
    };
    assert_eq!(
        first.count, 8,
        "the section counts are summed, not taken from whichever came first"
    );
    assert_eq!(
        first.thumb, "/t.jpg",
        "a section with no artwork must not blank a face another supplied"
    );
}

/// Sources are taken ROUND ROBIN, so a second server's best match sits second rather than
/// twenty-fifth — and a source that has not answered (pending) or cannot (failed) contributes
/// nothing at all instead of a gap.
#[test]
fn the_merge_round_robins_every_answered_source_and_skips_the_rest() {
    let sources = [
        answered(0, vec![media("ours-1"), media("ours-2"), media("ours-3")]),
        failed(),
        answered(0, vec![media("theirs-1"), media("theirs-2")]),
        Source::EMPTY, // still pending
    ];
    let sh = merge_favs(&sources);
    assert_eq!(sh.len(), 1, "an empty type draws nothing at all");
    assert_eq!(
        titles(&sh[0]),
        ["ours-1", "theirs-1", "ours-2", "theirs-2", "ours-3"]
    );
}

/// Neither a source's own list nor the merged shelf may exceed a `CardRow`'s spring count: past
/// it `scale(i)` clamps to the last cell, so an over-cap tile would wear its neighbour's pop and
/// never animate its own.
#[test]
fn a_merged_shelf_is_capped_at_the_card_rows_spring_count() {
    let many = |p: &str| {
        (0..SHELF_MAX)
            .map(|i| media(&format!("{p}{i}")))
            .collect::<Vec<_>>()
    };
    let sh = merge_favs(&[answered(0, many("a")), answered(0, many("b"))]);
    assert_eq!(sh[0].items.len(), SHELF_MAX);
    // …and the cap falls on a ROUND boundary rather than on one source: both are represented
    assert_eq!(titles(&sh[0])[..4], ["a0", "b0", "a1", "b1"]);
}

/// The count read-out and the heading, which are the only strings this store hands the screen.
/// The two counts are different questions and a Collections shelf answers both at once: three
/// collections found ("3 results"), one of which holds twelve films ("12 items").
#[test]
fn a_person_shelf_counts_people_and_everything_else_counts_results() {
    assert_eq!(
        (Kind::Person.title(), Kind::Person.count_label(1)),
        ("Cast & Crew", "1 person".to_owned())
    );
    assert_eq!(Kind::Person.count_label(2), "2 people");
    assert_eq!(Kind::Movie.count_label(1), "1 result");
    assert_eq!(Kind::Collection.count_label(0), "0 results");

    assert_eq!(
        (Kind::Collection.count_label(3), crate::ui::fmt::item_count(12)),
        ("3 results".to_owned(), "12 items".to_owned())
    );
    assert_eq!(crate::ui::fmt::item_count(1), "1 item");
    // Cardinal rules apply to the absolute value, including negative wire counts.
    assert_eq!((crate::ui::fmt::item_count(0), crate::ui::fmt::item_count(-1)),
        ("0 items".to_owned(), "-1 item".to_owned()));
}
