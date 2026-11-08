//! convert — Jellyfin `BaseItemDto` → the app's catalog row (`catalog::CatalogMovie`).
//!
//! This is the whole "impedance match" between the two backends, and it is small precisely
//! because the internal row was never one backend's: it is a title, a year, a duration, a
//! resume point, image paths and watch state. Everything here is one such field's mapping, and
//! anything cleverer belongs to the layer that asked for the row.
//!
//! ## Image paths are PATHS, not URLs
//!
//! `CatalogMovie.thumb`/`still`/`art` hold server-relative paths that the poster store later boxes
//! with a size ([`JfClient::image_path`]'s `fillWidth`/`fillHeight` suffix). What is stored here is
//! therefore the UNSIZED form, `/Items/{id}/Images/Primary?tag=…`, and the tag is embedded now
//! because it is the server's cache-buster: leave it out and a re-scraped poster would be served
//! stale from the app's LRU under a key that no longer names the current image.

use super::dto::BaseItemDto;
use crate::registry::ServerId;
use crate::catalog::CatalogMovie;
use std::os::raw::c_int;

/// `None` for kinds the app has no shelf for (music, photos, box sets) — the caller skips the
/// row, exactly as it skips any row whose type is not in its filter.
///
/// `sec` is Jellyfin's containing-library id collapsed to the row's `i64`: Jellyfin library ids
/// are GUIDs, which do not fit, and the browse adapter already scopes every listing by
/// `ParentId` at QUERY time, so the per-row pin the retired layer needed (`/hubs` answers mixed
/// every library into one response) has no counterpart here. `0` is "not pinned", not a bug.
pub(crate) fn movie_from_dto(it: &BaseItemDto, sid: ServerId, sec: i64) -> Option<CatalogMovie> {
    let kind: c_int = match it.kind.as_str() {
        "Movie" => 0,
        "Series" => 1,
        "Season" => 2,
        "Episode" => 3,
        _ => return None,
    };

    // The units rule from the module doc, applied at the one border where ticks exist.
    let dur_ns = it.run_time_ticks.unwrap_or(0).saturating_mul(100);
    let (resume_ms, unwatched, watched) = match &it.user_data {
        Some(ud) => (
            ud.playback_position_ticks / 10_000,
            ud.play_count == 0 && !ud.played,
            ud.played,
        ),
        None => (0, true, false),
    };

    // Images. An episode's own Primary is its 16:9 still and the poster shown for it is the
    // SERIES': the retired row modelled that as `thumb` (substitute) vs the episode's own art, and
    // the two fields on the row keep that distinction. A movie's Primary is both.
    let own_primary = image_path(it, "Primary", it.image_tags.get("Primary"));
    let series_poster = it.series_id.as_deref().map(|sid_| {
        format!("/Items/{sid_}/Images/Primary")
    });
    let (thumb, still) = if kind == 3 {
        (
            series_poster.clone().unwrap_or_else(|| own_primary.clone()),
            own_primary,
        )
    } else {
        (own_primary, String::new())
    };
    let art_owner = it.series_id.as_deref().unwrap_or(&it.id);
    let art = if it.backdrop_tags.is_empty() {
        String::new()
    } else {
        format!("/Items/{art_owner}/Images/Backdrop/0")
    };

    let (vcodec, acodec) = codecs(it);
    let blur = blur_corners(it);

    Some(CatalogMovie {
        sid,
        sec,
        title: it.name.clone(),
        year: it.year.unwrap_or(0) as c_int,
        rating: it.official_rating.clone().unwrap_or_default(),
        aired: String::new(),
        dur_ns,
        // The MediaSource id is what phase 3's PlaybackInfo is addressed by; with no sources in
        // the payload the item id itself is the right fallback — Jellyfin addresses both.
        part: it
            .media_sources
            .as_ref()
            .and_then(|s| s.first())
            .map(|s| s.id.clone())
            .unwrap_or_else(|| it.id.clone()),
        thumb,
        still,
        art,
        summary: it.overview.clone().unwrap_or_default(),
        rk: it.id.clone(),
        vcodec,
        acodec,
        blur: blur.unwrap_or([[0.0; 3]; 4]),
        has_blur: blur.is_some(),
        kind,
        resume_ms,
        show_rk: it.series_id.clone().unwrap_or_default(),
        season_index: (if kind == 2 { it.index_number } else { it.parent_index })
            .unwrap_or(0) as c_int,
        show_title: it.series_name.clone().unwrap_or_default(),
        ep_index: if kind == 3 {
            it.index_number.unwrap_or(0) as c_int
        } else {
            0
        },
        child_count: 0,
        unwatched,
        watched,
    })
}

/// `/Items/{id}/Images/{kind}` with the server's cache tag when the payload named one. An image
/// the server says does not exist becomes an EMPTY path — the poster store already treats "" as
/// "no art", and requesting a nonexistent image would burn a worker on a 404 per tile.
fn image_path(it: &BaseItemDto, kind: &str, tag: Option<&String>) -> String {
    match tag {
        Some(t) => format!("/Items/{}/Images/{kind}?tag={t}", it.id),
        None => String::new(),
    }
}

/// A BoxSet row → the browse grid's collection card (`kind == KIND_COLLECTION`). The card carries
/// the box set's own id as its `rk`, so opening it routes to the collection screen — exactly the
/// split `movie_from_dto` refuses to make, where a BoxSet would otherwise be skipped as an
/// unshelfable kind. `ChildCount` rides the row's `child_count`, which the collection tile reads
/// as its "N items" caption.
pub(crate) fn boxset_from_dto(it: &BaseItemDto, sid: ServerId) -> Option<CatalogMovie> {
    if it.kind != "BoxSet" {
        return None;
    }
    let thumb = image_path(it, "Primary", it.image_tags.get("Primary"));
    Some(CatalogMovie {
        sid,
        sec: 0,
        title: it.name.clone(),
        year: it.year.unwrap_or(0) as c_int,
        thumb,
        summary: it.overview.clone().unwrap_or_default(),
        rk: it.id.clone(),
        kind: crate::catalog::KIND_COLLECTION,
        child_count: it.child_count.unwrap_or(0).max(0),
        ..Default::default()
    })
}

/// First video and first audio codec ids, from the first MediaSource that carries streams.
/// `(unknown, unknown)` is honest — a listing fetched without `Fields=MediaSources` is not a
/// file without codecs, and the direct-play gate re-reads them from the detail fetch anyway.
fn codecs(it: &BaseItemDto) -> (String, String) {
    let mut v = String::new();
    let mut a = String::new();
    if let Some(sources) = &it.media_sources {
        for s in sources {
            for st in &s.streams {
                let Some(c) = &st.codec else { continue };
                if st.kind == "Video" && v.is_empty() {
                    v = c.clone();
                } else if st.kind == "Audio" && a.is_empty() {
                    a = c.clone();
                }
            }
            if !v.is_empty() && !a.is_empty() {
                break;
            }
        }
    }
    (v, a)
}

/// The ambient ground's four corners, decoded from the item's BlurHash strings. The backdrop's
/// hash is the page's own picture; the poster's is the fallback for a library with no backdrops.
pub(crate) fn blur_corners(it: &BaseItemDto) -> Option<[[f32; 3]; 4]> {
    let hash = |kind: &str, tag: Option<&String>| {
        it.image_blur_hashes
            .get(kind)
            .and_then(|tags| tag.and_then(|t| tags.get(t)))
    };
    hash("Backdrop", it.backdrop_tags.first())
        .or_else(|| hash("Primary", it.image_tags.get("Primary")))
        .and_then(|h| super::blurhash::corners(h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jellyfin::dto::ItemsResult;

    fn one(json: &str) -> BaseItemDto {
        let r: ItemsResult = serde_json::from_str(json).unwrap();
        r.items.into_iter().next().unwrap()
    }

    #[test]
    fn movie_maps_fields_and_units() {
        let it = one(
            r#"{"Items": [{
                "Id": "edb39341c5039551a5157e51fe4a3364", "Name": "Bubble", "Type": "Movie",
                "ProductionYear": 1976, "OfficialRating": "PG", "RunTimeTicks": 55842000000,
                "Overview": "line one\nline two",
                "ImageTags": {"Primary": "tag1"}, "BackdropImageTags": ["bd1"],
                "UserData": {"Played": false, "PlayCount": 20, "PlaybackPositionTicks": 12119991500},
                "MediaSources": [{"Id": "ms1", "MediaStreams": [
                    {"Codec": "h264", "Type": "Video"}, {"Codec": "aac", "Type": "Audio"}]}]
            }], "TotalRecordCount": 1}"#,
        );
        let m = movie_from_dto(&it, ServerId::from_raw(0), 0).unwrap();
        assert_eq!(m.kind, 0);
        assert_eq!(m.rk, "edb39341c5039551a5157e51fe4a3364");
        assert_eq!(m.year, 1976);
        assert_eq!(m.rating, "PG");
        // ticks → ns is ×100, ticks → ms is ÷10_000, and the two must not cross.
        assert_eq!(m.dur_ns, 5_584_200_000_000);
        assert_eq!(m.resume_ms, 1_211_999);
        assert_eq!(m.thumb, "/Items/edb39341c5039551a5157e51fe4a3364/Images/Primary?tag=tag1");
        assert_eq!(m.art, "/Items/edb39341c5039551a5157e51fe4a3364/Images/Backdrop/0");
        assert_eq!(m.part, "ms1");
        assert_eq!(m.vcodec, "h264");
        assert_eq!(m.acodec, "aac");
        // In progress: not unwatched (it has a play count), not watched.
        assert!(!m.unwatched);
        assert!(!m.watched);
        assert!(m.resume_frac().is_some());
    }

    #[test]
    fn blur_corners_come_from_the_backdrop_then_the_poster() {
        let with_both = one(
            r#"{"Items": [{
                "Id": "m1", "Name": "A Film", "Type": "Movie",
                "ImageTags": {"Primary": "p1"}, "BackdropImageTags": ["b1"],
                "ImageBlurHashes": {
                    "Primary": {"p1": "LEHV6nWB2yk8pyo0adR*.7kCMdnj"},
                    "Backdrop": {"b1": "LEHV6nWB2yk8pyo0adR*.7kCMdnj"}
                }
            }], "TotalRecordCount": 1}"#,
        );
        let m = movie_from_dto(&with_both, ServerId::from_raw(0), 0).unwrap();
        assert!(m.has_blur);
        assert!(m.blur.iter().flatten().all(|c| (0.0..=1.0).contains(c)));

        let poster_only = one(
            r#"{"Items": [{
                "Id": "m2", "Name": "Another", "Type": "Movie",
                "ImageTags": {"Primary": "p1"}, "BackdropImageTags": [],
                "ImageBlurHashes": {"Primary": {"p1": "LEHV6nWB2yk8pyo0adR*.7kCMdnj"}}
            }], "TotalRecordCount": 1}"#,
        );
        let m = movie_from_dto(&poster_only, ServerId::from_raw(0), 0).unwrap();
        assert!(m.has_blur, "the poster's hash is the fallback");

        let no_hash =
            one(r#"{"Items": [{"Id": "m3", "Name": "Third", "Type": "Movie"}], "TotalRecordCount": 1}"#);
        let m = movie_from_dto(&no_hash, ServerId::from_raw(0), 0).unwrap();
        assert!(!m.has_blur, "no hash is no ground, not a black one");

        let broken = one(
            r#"{"Items": [{
                "Id": "m4", "Name": "Broken", "Type": "Movie",
                "ImageTags": {"Primary": "p1"}, "ImageBlurHashes": {"Primary": {"p1": "nope"}}
            }], "TotalRecordCount": 1}"#,
        );
        let m = movie_from_dto(&broken, ServerId::from_raw(0), 0).unwrap();
        assert!(!m.has_blur, "a hash that will not decode is no ground either");
    }

    #[test]
    fn episode_takes_the_series_poster_and_keeps_its_own_still() {
        let it = one(
            r#"{"Items": [{
                "Id": "ep1", "Name": "Pilot", "Type": "Episode",
                "SeriesId": "series9", "SeriesName": "The Show",
                "ParentIndexNumber": 1, "IndexNumber": 4,
                "ImageTags": {"Primary": "etag"}, "BackdropImageTags": [],
                "UserData": {"Played": true, "PlayCount": 1, "PlaybackPositionTicks": 0}
            }], "TotalRecordCount": 1}"#,
        );
        let m = movie_from_dto(&it, ServerId::from_raw(0), 0).unwrap();
        assert_eq!(m.kind, 3);
        assert_eq!(m.thumb, "/Items/series9/Images/Primary");
        assert_eq!(m.still, "/Items/ep1/Images/Primary?tag=etag");
        assert_eq!(m.art, ""); // no backdrop tags on the payload → no art request
        assert_eq!(m.show_rk, "series9");
        assert_eq!(m.show_title, "The Show");
        assert_eq!(m.season_index, 1);
        assert_eq!(m.ep_index, 4);
        assert!(m.watched);
        assert!(!m.unwatched);
    }

    #[test]
    fn unknown_kind_is_skipped_not_misfilled() {
        let it = one(r#"{"Items": [{"Id": "b1", "Name": "Set", "Type": "BoxSet"}], "TotalRecordCount": 1}"#);
        assert!(movie_from_dto(&it, ServerId::from_raw(0), 0).is_none());
    }

    #[test]
    fn a_boxset_row_converts_to_a_collection_card() {
        let it = one(
            r#"{"Items": [{
                "Id": "box1", "Name": "The Saga", "Type": "BoxSet",
                "ImageTags": {"Primary": "btag"}, "ChildCount": 7
            }], "TotalRecordCount": 1}"#,
        );
        let m = boxset_from_dto(&it, ServerId::from_raw(0)).unwrap();
        assert_eq!(m.kind, crate::catalog::KIND_COLLECTION);
        assert_eq!(m.rk, "box1");
        assert_eq!(m.title, "The Saga");
        assert_eq!(m.thumb, "/Items/box1/Images/Primary?tag=btag");
        assert_eq!(m.child_count, 7);
        assert_eq!(m.sec, 0);
        // a non-BoxSet row is refused, never misfilled
        assert!(boxset_from_dto(&one(r#"{"Items": [{"Id": "m", "Name": "Film", "Type": "Movie"}], "TotalRecordCount": 1}"#), ServerId::from_raw(0)).is_none());
    }
}
