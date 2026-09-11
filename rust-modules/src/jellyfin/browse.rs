//! browse — the Library screen's wire side: views as sections, one paged query, the menus.
//!
//! Everything here is the Jellyfin half of a `browse.rs` idiom, shaped so the store side stays
//! backend-blind: sections arrive as the same `(key, title, SecKind)` rows `project_sections`
//! builds off a PMS, a page lands as the same `(Vec<PmsMovie>, total)` the Plex page worker
//! lands, and the sort menu is a `Vec<SortEntry>` like any other. What could not be shared is
//! the CLIENT (Plex's lifecycle validation reads registry state a Jellyfin client has no
//! counterpart of), so `browse.rs` carries a narrow jellyfin lane beside each mailbox — the
//! same split `posters.rs` and `pms.rs` already made.
//!
//! ## The key side-table
//!
//! `BrowseSection.key` is an `i64`, because PMS section keys are integers. A Jellyfin view id is
//! a GUID, so this module mints a synthetic key per view and remembers the pairing
//! ([`key_for_view`] / [`view_for_key`]): the table, the toolbar and every page landing speak the
//! integer, and only the two fetch boundaries here ever see the GUID again. Keys are 1-based and
//! append-only for the life of the process — a view's key must never move under an in-flight
//! page landing, which is `browse.rs`'s own append-never-rebuild rule read one level down. (A
//! re-discovery that finds the same GUIDs hands back the SAME keys, so `append_sections`'s
//! new-only filter keeps working.)

use super::client::{BrowseQuery, JfClient};
use super::SERVER_ID;
use crate::browse::{GenreEntry, SecKind, SortEntry};
use crate::pms::PmsMovie;
use std::sync::Mutex;

/// The synthetic section keys handed out so far, 1-based: `VIEW_KEYS[i]` is the GUID of the view
/// whose key is `i + 1`. A `Mutex` rather than a main-thread static because both sides of the
/// boundary touch it — discovery lands keys on the main thread, page workers read them back.
static VIEW_KEYS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The integer key for a view GUID, minting one on first sight. MAIN THREAD (discovery landing)
/// in practice; the lock makes the claim unnecessary to keep.
pub(crate) fn key_for_view(guid: &str) -> i64 {
    let mut keys = VIEW_KEYS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = keys.iter().position(|g| g == guid) {
        return i as i64 + 1;
    }
    keys.push(guid.to_string());
    keys.len() as i64
}

/// The view GUID behind a synthetic key — the page/genre/count workers' half of the pairing.
pub(crate) fn view_for_key(key: i64) -> Option<String> {
    if key < 1 {
        return None;
    }
    VIEW_KEYS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key as usize - 1)
        .cloned()
}

/// The sort menu of a Jellyfin section. Plex's is server-driven (`Meta.Type[].Sort` rides the
/// first page); Jellyfin's `SortBy` vocabulary is FIXED, so the menu is a constant — handed to
/// the section state with the first landed page, which is the moment Plex's menus arrive too.
/// Each `key` IS the `SortBy` token, so `browse.rs`'s menu state feeds the query unchanged.
pub(crate) fn sorts() -> Vec<SortEntry> {
    vec![
        SortEntry {
            key: "SortName".into(),
            title: "Title".into(),
            default_desc: false,
        },
        SortEntry {
            key: "PremiereDate".into(),
            title: "Release date".into(),
            default_desc: true,
        },
        SortEntry {
            key: "DateCreated".into(),
            title: "Date added".into(),
            default_desc: true,
        },
        SortEntry {
            key: "CommunityRating".into(),
            title: "Rating".into(),
            default_desc: true,
        },
    ]
}

/// `GET /Users/{uid}/Views` → the `(key, title, kind)` rows `browse::append_sections` takes.
/// `movies`/`tvshows` map to the app's two section kinds; every other collection type (music,
/// books, mixed — the `None` included) is dropped here, which is `SecKind::from_wire`'s rule in
/// Jellyfin spelling: a type the app draws no level for never reaches the table.
pub(crate) fn fetch_sections(c: &JfClient) -> Option<Vec<(i64, String, SecKind)>> {
    let views = c.views()?;
    let out = views
        .items
        .iter()
        .filter_map(|v| {
            let kind = match v.collection_type.as_deref() {
                Some("movies") => SecKind::Movie,
                Some("tvshows") => SecKind::Show,
                _ => return None,
            };
            Some((key_for_view(&v.id), v.name.clone(), kind))
        })
        .collect();
    Some(out)
}

/// The unfiltered item count of each library, by section key — the Sources row's "185 films".
/// One `Limit=1` page per view: `TotalRecordCount` rides the envelope, and a view that does not
/// answer is simply absent from the result, which is `browse.rs`'s "empty is a failure" signal.
/// The caller (the discovery arm) pairs each key with its kind, which the store side already
/// holds — a count by bare key would have to ask the table what type to filter by.
pub(crate) fn fetch_counts(c: &JfClient, keys: &[(i64, SecKind)]) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    for &(k, kind) in keys {
        let Some(view) = view_for_key(k) else { continue };
        let q = BrowseQuery {
            view_id: &view,
            start: 0,
            limit: 1,
            include_types: include_types(kind),
            sort_by: "SortName",
            sort_desc: false,
            unwatched: false,
            genre_ids: "",
        };
        if let Some(res) = c.browse_page(&q) {
            out.push((k, res.total));
        }
    }
    out
}

/// One page of one section's current query → the rows + the listing's total, in exactly the
/// shape the Plex page worker's landing carries. `None` is the FAILURE sentinel — the store
/// side leaves a populated grid untouched on it, the same rule the Plex lane keeps.
///
/// `sort_key` is a [`sorts`] key or "" (menus not landed yet); "" means the Title default, sent
/// EXPLICITLY — Jellyfin's server default is an implementation detail, and the toolbar's first
/// chip already says "Title". Rows are converted by [`super::convert::movie_from_dto`] and
/// stamped with [`SERVER_ID`]; the query's `IncludeItemTypes` is what guarantees every row is of
/// a convertible kind, so the `filter_map` never actually drops one (a dropped row would shift
/// the sparse store's indexing against `total`).
pub(crate) fn fetch_page(
    c: &JfClient,
    key: i64,
    kind: SecKind,
    start: i64,
    limit: i64,
    sort_key: &str,
    sort_desc: bool,
    unwatched: bool,
    genre_ids: &str,
) -> Option<(Vec<PmsMovie>, i64)> {
    let view = view_for_key(key)?;
    let sort_by = if sort_key.is_empty() {
        "SortName"
    } else {
        sort_key
    };
    let res = c.browse_page(&BrowseQuery {
        view_id: &view,
        start,
        limit,
        include_types: include_types(kind),
        sort_by,
        sort_desc,
        unwatched,
        genre_ids,
    })?;
    let items: Vec<PmsMovie> = res
        .items
        .iter()
        .filter_map(|it| super::convert::movie_from_dto(it, SERVER_ID, 0))
        .collect();
    Some((items, res.total))
}

/// The genre value list of one section → the filter menu's rows. `GenreEntry.id` is the genre's
/// GUID, because the listing filter (`GenreIds`) takes ids back — a name never round-trips.
pub(crate) fn fetch_genres(c: &JfClient, key: i64) -> Option<Vec<GenreEntry>> {
    let view = view_for_key(key)?;
    let res = c.genres(&view)?;
    let out = res
        .items
        .iter()
        .filter(|g| !g.id.is_empty() && !g.name.is_empty())
        .map(|g| GenreEntry {
            id: g.id.clone(),
            title: g.name.clone(),
        })
        .collect();
    Some(out)
}

/// The `IncludeItemTypes` spelling of a section kind. Both kinds list their LEAF-playable
/// parent type: a movies view lists Movies, a tvshows view lists Series (episodes are reached
/// through the detail page, as on Plex).
fn include_types(kind: SecKind) -> &'static str {
    match kind {
        SecKind::Movie => "Movie",
        SecKind::Show => "Series",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_keys_round_trip_and_never_move() {
        let a = key_for_view("guid-alpha");
        let b = key_for_view("guid-beta");
        assert_ne!(a, b);
        assert!(a >= 1 && b >= 1);
        // a re-discovery of the same GUID reuses its key — append_sections' new-only filter
        // depends on it
        assert_eq!(key_for_view("guid-alpha"), a);
        assert_eq!(view_for_key(a).as_deref(), Some("guid-alpha"));
        assert_eq!(view_for_key(b).as_deref(), Some("guid-beta"));
        assert!(view_for_key(0).is_none());
        assert!(view_for_key(i64::MAX).is_none());
    }

    #[test]
    fn the_sort_menu_is_jellyfins_fixed_vocabulary() {
        let s = sorts();
        assert_eq!(s[0].key, "SortName");
        assert!(!s[0].default_desc); // Title opens ascending
        assert!(s.iter().all(|e| !e.title.is_empty()));
        // every key is a token Jellyfin's SortBy actually takes
        for e in &s {
            assert!(matches!(
                e.key.as_str(),
                "SortName" | "PremiereDate" | "DateCreated" | "CommunityRating"
            ));
        }
    }

    #[test]
    fn section_kinds_map_to_leaf_types() {
        assert_eq!(include_types(SecKind::Movie), "Movie");
        assert_eq!(include_types(SecKind::Show), "Series");
    }
}
