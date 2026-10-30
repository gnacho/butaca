//! detail — the Jellyfin fetch behind the detail page: one item's full record, its
//! seasons/episodes, its OnDeck, its Related shelf, its intro/credits segments — projected into
//! the ONE `metadata::Detail` the page has always drawn.
//!
//! This is the detail twin of `pms::fetch_source_jellyfin`: the page, its mailbox, its
//! season-switch worker and every draw site are untouched; what differs is which server answered
//! and how its payload parsed. The orchestration deliberately mirrors `metadata::fetch_full`'s
//! shape — the same sequence (record → show extras → related), the same failure tolerance
//! (a failed extra is an empty shelf, never a failed page) — so the two backends can be read
//! side by side.
//!
//! ## Kinds, spelled Plex's way
//!
//! `Detail.kind` is read by the UI in PLEX's vocabulary ("movie"/"show"/"season"/"episode"),
//! and so are the fields around it (`is_show`, `show_title`, …). The converter therefore
//! translates Jellyfin's "Series" to "show" etc. at the border — one small table in
//! [`detail_from_dto`] — rather than teaching forty draw sites a second spelling.

use super::client::JfClient;
use super::dto::{BaseItemDto, MediaSourceDto, PersonDto};
use crate::metadata::{Cast, Chapter, Detail, Dovi, Episode, Season, Stream};
use crate::plex::ServerId;

/// The whole detail fetch for one item — `metadata::fetch_full`'s Jellyfin arm. `None` is the
/// same contract as there: the page keeps what it had (or, mounted fresh, the catalog row's
/// hero) and the log line below is the only voice the failure gets.
pub(crate) fn fetch_full(c: &JfClient, sid: ServerId, rk: &str) -> Option<Detail> {
    let t0 = std::time::Instant::now();
    let Some(it) = c.item_detail(rk) else {
        crate::log(&format!(
            "detail: rk={rk} sid={sid:?} — no metadata (jellyfin: server unresolved, or it refused)"
        ));
        return None;
    };
    let mut d = detail_from_dto(&it, sid);
    if d.is_show {
        d.on_deck = c
            .next_up(rk)
            .and_then(|r| r.items.into_iter().next())
            .map(|e| episode_from_dto(&e));
        d.seasons = c
            .seasons(rk)
            .map(|r| r.items.iter().map(season_from_dto).collect())
            .unwrap_or_default();
        if let Some(s0) = d.seasons.first() {
            d.episodes = fetch_episodes(c, rk, &s0.rk).unwrap_or_else(|| {
                crate::log(&format!(
                    "detail: rk={rk} season rk={} episodes did not answer — the eps= below is that refusal",
                    s0.rk
                ));
                Vec::new()
            });
        }
        // A show carries no streams of its own: borrow the hero episode's — the one Play starts
        // (OnDeck, else the first) — exactly as `fetch_full` does, and with its same caveat:
        // `part`/`vcodec`/`acodec` stay the SHOW's own (empty), because "a show has no playable
        // part" is load-bearing on the play path.
        let hero_ep = d
            .on_deck
            .as_ref()
            .map(|e| e.rk.clone())
            .or_else(|| d.episodes.first().map(|e| e.rk.clone()));
        if let Some(ep_rk) = hero_ep {
            if let Some(ep) = c.item_detail(&ep_rk) {
                fill_streams(&ep, &mut d);
            }
        }
        // A leaf's segments belong to the file; a show's Skip pill is armed per episode at PLAY
        // time, not here.
        d.markers = Vec::new();
    } else {
        d.markers = c
            .media_segments(rk)
            .map(|s| convert_segments(&s))
            .unwrap_or_default();
    }
    d.related = c
        .similar(rk, 12)
        .map(|r| {
            r.items
                .iter()
                .filter_map(|i| super::convert::movie_from_dto(i, sid, 0))
                .collect()
        })
        .unwrap_or_default();
    crate::log(&format!(
        "detail: rk={rk} loaded (jellyfin) — {} seasons, {} eps, {} cast, {} related, {} markers | on_deck={} | ms={}",
        d.seasons.len(),
        d.episodes.len(),
        d.credits_len(),
        d.related.len(),
        d.markers.len(),
        i32::from(d.on_deck.is_some()),
        t0.elapsed().as_millis()
    ));
    Some(d)
}

/// One season's episode list — called from the show fetch above AND from `metadata`'s
/// season-switch worker (the tab change refetches one season), which is why it is a function of
/// this module and not a closure in [`fetch_full`].
pub(crate) fn fetch_episodes(c: &JfClient, series_rk: &str, season_rk: &str) -> Option<Vec<Episode>> {
    c.episodes(series_rk, season_rk)
        .map(|r| r.items.iter().map(episode_from_dto).collect())
}

/// The playing-item store's Jellyfin fetch — `metadata::fetch_playing_item`'s other-backend arm.
/// The store feeds PLAYBACK (the track menu, the direct-play gate's frame size, the in-player
/// markers), so unlike the detail page this one is per-LEAF and fetched fresh at every play;
/// markers ride `/MediaSegments` here just as they do on the leaf detail page.
pub(crate) fn playing_item(
    c: &JfClient,
    sid: ServerId,
    rk: &str,
) -> Option<crate::metadata::PlayingItem> {
    let it = c.item_detail(rk)?;
    let src = it
        .media_sources
        .as_ref()
        .and_then(|ss| ss.iter().find(|s| !s.streams.is_empty()).or(ss.first()));
    let mut audio = Vec::new();
    let mut subs = Vec::new();
    let mut video_fps = 0.0;
    let (mut width, mut height) = (0, 0);
    let mut dovi = Dovi::NONE;
    for st in src.map(|s| s.streams.as_slice()).unwrap_or(&[]) {
        match st.kind.as_str() {
            "Video" if width == 0 => {
                width = st.width.unwrap_or(0) as i64;
                height = st.height.unwrap_or(0) as i64;
                video_fps = st.real_frame_rate.unwrap_or(0.0) as f64;
                dovi = dovi_from_range(st.video_range_type.as_deref());
            }
            "Audio" => audio.push(convert_stream(st)),
            "Subtitle" => subs.push(convert_stream(st)),
            _ => {}
        }
    }
    Some(crate::metadata::PlayingItem {
        sid,
        rk: rk.to_string(),
        audio,
        subs,
        video_fps,
        width,
        blur: None,
        show_rk: String::new(),
        // the MediaSource's own container word — a GUID id has no extension, so this field IS
        // the demuxer test on this backend
        container: src.and_then(|s| s.container.clone()).unwrap_or_default(),
        height,
        bitrate: src.and_then(|s| s.bitrate).unwrap_or(0) / 1_000,
        // Derived from `VideoRangeType` (see `dovi_from_range`): DV present, base layer named
        // when the string names one, and bare `DOVI` read as Profile 5 (single-layer IPT-PQ).
        dovi,
        markers: c
            .media_segments(rk)
            .map(|s| convert_segments(&s))
            .unwrap_or_default(),
        chapters: chapters_from_dto(&it.chapters),
    })
}

/// `ChapterDto[]` → the strip's `Chapter[]` — 1-based index, ticks to ms. Jellyfin HAS chapter
/// images, but the panel's thumbs are a later nicety and an empty path already draws nothing.
fn chapters_from_dto(chs: &[super::dto::ChapterDto]) -> Vec<Chapter> {
    chs.iter()
        .enumerate()
        .map(|(i, ch)| Chapter {
            index: (i + 1) as i64,
            start_ms: ch.start_position_ticks / 10_000,
            title: ch.name.clone().unwrap_or_default(),
            thumb: String::new(),
        })
        .collect()
}

/// Jellyfin kind → the Plex spelling the UI reads. `None` (a kind the app has no page for) is
/// filtered by the caller; [`detail_from_dto`] is only ever asked with a playable kind.
fn plex_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "Movie" => Some("movie"),
        "Series" => Some("show"),
        "Season" => Some("season"),
        "Episode" => Some("episode"),
        _ => None,
    }
}

/// The date part of Jellyfin's ISO timestamp — the only part the UI draws ("1976-11-12").
fn date_part(iso: &str) -> String {
    iso.split('T').next().unwrap_or(iso).to_string()
}

/// The detail record itself. Field by field the same meaning as `metadata::fetch_detail`'s
/// mapping; where Jellyfin simply does not have the datum the field gets its honest zero and a
/// comment says so.
fn detail_from_dto(it: &BaseItemDto, sid: ServerId) -> Detail {
    let kind = plex_kind(&it.kind).unwrap_or("movie");
    let is_show = kind == "show";
    let (resume_ms, watched) = match &it.user_data {
        Some(ud) => {
            let watched = if is_show || kind == "season" {
                // Plex's rule (`leaf_count > 0 && viewed >= leaf_count`), with Jellyfin's
                // complement arithmetic: viewed = ChildCount − UnplayedItemCount.
                let leaf = it.child_count.unwrap_or(0);
                let viewed = leaf - ud.unplayed_item_count.unwrap_or(leaf);
                leaf > 0 && viewed >= leaf
            } else {
                ud.played
            };
            (ud.playback_position_ticks / 10_000, watched)
        }
        None => (0, false),
    };

    let is_favorite = it.user_data.as_ref().map(|ud| ud.is_favorite).unwrap_or(false);
    let mut d = Detail {
        sid,
        rk: it.id.clone(),
        is_favorite,
        collection: None,
        extras: Vec::new(),
        trailer_rk: String::new(),
        // One server on this backend: no attribution to draw, and no portable cross-server
        // identity exists on Jellyfin; the item id fills the field so the "Also available"
        // machinery has a well-formed non-answer and no other source to ask.
        guid: it.id.clone(),
        is_show,
        kind: kind.to_string(),
        show_title: it.series_name.clone().unwrap_or_default(),
        show_rk: it.series_id.clone().unwrap_or_default(),
        season: it.parent_index.unwrap_or(0) as i64,
        index: it.index_number.unwrap_or(0) as i64,
        title: it.name.clone(),
        year: it.year.unwrap_or(0) as i64,
        rating: it.official_rating.clone().unwrap_or_default(),
        summary: it.overview.clone().unwrap_or_default(),
        tagline: it.taglines.first().cloned().unwrap_or_default(),
        aired: it
            .premiere_date
            .as_deref()
            .map(date_part)
            .unwrap_or_default(),
        dur_ms: it.run_time_ticks.unwrap_or(0) / 10_000,
        resume_ms,
        watched,
        part: String::new(),     // fill_streams owns the playable fields
        vcodec: String::new(),  //
        acodec: String::new(),  //
        video_fps: 0.0,
        video_resolution: String::new(),
        width: 0,
        height: 0,
        bitrate: 0,
        container: String::new(),
        file: String::new(),
        size: 0,
        aspect_ratio: 0.0,
        video: None,
        hdr: false,
        // `fill_streams` derives the DV layering from `VideoRangeType` (like `part`/`vcodec`).
        dovi: Default::default(),
        art: if it.backdrop_tags.is_empty() {
            String::new()
        } else {
            format!("/Items/{}/Images/Backdrop/0", it.id)
        },
        thumb: match it.image_tags.get("Primary") {
            Some(t) => format!("/Items/{}/Images/Primary?tag={t}", it.id),
            None => String::new(),
        },
        blur: [[0.0; 3]; 4],
        has_blur: false, // BlurHash, not corner colours — see convert::movie_from_dto
        genres: it.genres.clone(),
        countries: it.production_locations.clone(),
        cast: cast_from_people(&it.people),
        directors: crew_names(&it.people, "Director"),
        crew: crew_credits(&it.people),
        audio: Vec::new(),
        subs: Vec::new(),
        seasons: Vec::new(),
        episodes: Vec::new(),
        on_deck: None,
        cur_season: 0,
        related: Vec::new(),
        chapters: chapters_from_dto(&it.chapters),
        markers: Vec::new(), // the caller arms them per kind (see fetch_full)
        ratings: Vec::new(), // Jellyfin does not name score sources; an empty row draws nothing
    };
    if !is_show {
        fill_streams(it, &mut d);
    }
    d
}

/// The `Media[0].Part[0].Stream[]` block of [`Detail`], filled from the first MediaSource that
/// carries streams. `part`/`vcodec`/`acodec` move ONLY here — for a leaf this is the item's own
/// record, and a show deliberately never gets them (see [`fetch_full`]).
fn fill_streams(it: &BaseItemDto, d: &mut Detail) {
    let Some(src) = it
        .media_sources
        .as_ref()
        .and_then(|ss| ss.iter().find(|s| !s.streams.is_empty()).or(ss.first()))
    else {
        return;
    };
    d.part = src.id.clone();
    d.container = src.container.clone().unwrap_or_default();
    d.file = src.path.clone().unwrap_or_default();
    d.size = src.size.unwrap_or(0);
    d.bitrate = src.bitrate.unwrap_or(0) / 1_000; // wire is bps, Detail is kbps
    for st in &src.streams {
        let s = convert_stream(st);
        match st.kind.as_str() {
            "Video" => {
                if d.video.is_none() {
                    d.vcodec = st.codec.clone().unwrap_or_default();
                    d.width = st.width.unwrap_or(0) as i64;
                    d.height = st.height.unwrap_or(0) as i64;
                    d.video_resolution = resolution_class(st.height.unwrap_or(0));
                    d.video_fps = st.real_frame_rate.unwrap_or(0.0) as f64;
                    d.aspect_ratio = st
                        .aspect_ratio
                        .as_deref()
                        .and_then(parse_aspect)
                        .unwrap_or(0.0);
                    d.hdr = st
                        .video_range_type
                        .as_deref()
                        .is_some_and(|t| !t.eq_ignore_ascii_case("sdr"));
                    d.dovi = dovi_from_range(st.video_range_type.as_deref());
                    d.video = Some(s);
                }
            }
            "Audio" => {
                if d.acodec.is_empty() {
                    d.acodec = st.codec.clone().unwrap_or_default();
                }
                d.audio.push(s);
            }
            "Subtitle" => d.subs.push(s),
            _ => {}
        }
    }
}

/// `VideoRangeType` → the Dolby Vision layering the play gate can act on.
///
/// Jellyfin names the base layer's cross-compatibility in the string (`DOVIWithHDR10`,
/// `DOVIWithHLG`, `DOVIWithSDR` → `bl_compat` 1 / 4 / 2) but only ever reports DV presence,
/// never a profile number. The ONE spelling that does name the profile is bare `DOVI`: it says
/// the file declares NO HDR10, HLG or SDR base layer, which is exactly the shape of a
/// **Profile 5** (a single IPT-PQ layer with nothing an ordinary decoder can fall back on).
/// Reading it as profile 5 is not inventing a profile — it is the only reading consistent with
/// what the string says — and it makes the gate treat the file exactly as a Plex P5 item:
/// declared to the panel when the set supports DV, refused otherwise, never played as a bare
/// base layer.
///
/// **What the assumption costs.** If the file were actually a dual-layer Profile 7 whose base
/// layer Jellyfin failed to name (so it too shows as bare `DOVI`), we would declare it as
/// single-layer P5: the panel reads the base layer as IPT-PQ and shows a wrong picture, where
/// the honest `NotDv` fallback would have degraded to the (HDR10) base layer instead. The
/// `WithHDR10`/`WithHLG`/`WithSDR` spellings do NOT make this assumption: their base layer is
/// named and displayable, so `profile` stays 0 (unknown) and they play through that base layer
/// — the string cannot tell a single-layer P8 from a dual-layer P7, and the base layer is a
/// correct picture either way.
fn dovi_from_range(video_range_type: Option<&str>) -> Dovi {
    let Some(t) = video_range_type else {
        return Dovi::NONE;
    };
    let t = t.trim().to_ascii_uppercase();
    if !t.starts_with("DOVI") {
        return Dovi::NONE;
    }
    let (profile, bl_compat) = match t.as_str() {
        "DOVI" => (5, 0),         // no base layer at all = single-layer IPT-PQ (Profile 5)
        "DOVIWITHHDR10" => (0, 1), // HDR10 base layer
        "DOVIWITHSDR" => (0, 2),   // SDR base layer
        "DOVIWITHHLG" => (0, 4),   // HLG base layer
        _ => (0, 0),               // an unrecognised DOVI spelling: no profile the gate can name
    };
    Dovi {
        present: true,
        profile,
        bl_compat,
        el_present: false,
        ..Dovi::NONE
    }
}

/// `4k` / `1080` / `720` / `sd` — the hero badge's vocabulary, classified by frame height
/// exactly as Plex's `Media.videoResolution` is (a 1918-pixel-wide scope film is 1080p).
fn resolution_class(height: i32) -> String {
    match height {
        h if h >= 2000 => "4k",
        h if h >= 1000 => "1080",
        h if h >= 700 => "720",
        _ => "sd",
    }
    .to_string()
}

/// "16:9" or "2.35:1" → 1.78 / 2.35; a bare float string is accepted too. `None` = not said.
fn parse_aspect(s: &str) -> Option<f64> {
    if let Some((w, h)) = s.split_once(':') {
        let (w, h) = (w.parse::<f64>().ok()?, h.parse::<f64>().ok()?);
        (h > 0.0).then_some(w / h)
    } else {
        s.parse::<f64>().ok()
    }
}

/// One `MediaStream` → the app's `Stream`. `id` mirrors `index`: Jellyfin addresses streams by
/// source-relative index alone, so there is no second numbering to preserve.
fn convert_stream(st: &super::dto::MediaStreamDto) -> Stream {
    Stream {
        id: st.index.unwrap_or(0) as i64,
        index: st.index.unwrap_or(0) as i64,
        key: String::new(),
        can_normalize_loudness: false,
        language_tag: st.language.clone().unwrap_or_default(),
        // The Language column is the ISO code on both backends; the human line ("English") is
        // derived from it by the track menu's own naming, as it is for Plex's "eng".
        lang: st.language.clone().unwrap_or_default(),
        lang_code: st.language.clone().unwrap_or_default(),
        codec: st.codec.clone().unwrap_or_default(),
        channels: st.channels.unwrap_or(0) as i64,
        layout: st.channel_layout.clone().unwrap_or_default(),
        bitrate: st.bit_rate.unwrap_or(0) / 1_000,
        profile: st.profile.clone().unwrap_or_default(),
        bit_depth: st.bit_depth.unwrap_or(0) as i64,
        chroma: String::new(), // Jellyfin does not report chroma subsampling per stream
        title: st.display_title.clone().unwrap_or_default(),
        sdh: st.is_sdh.unwrap_or(false),
        ad: false, // no standard field; a descriptive track reads as an ordinary one
        forced: st.is_forced,
        default: st.is_default,
        external: st.is_external,
        selected: false, // the play path arms the server's default, not the listing's
    }
}

/// Actor rows, in server order. The headshot is the person's own Primary image — a
/// server-relative path, so the poster pipeline treats it like any other art (Plex headshots
/// arrive as absolute URLs and take a special path; this backend's never do).
fn cast_from_people(people: &[PersonDto]) -> Vec<Cast> {
    people
        .iter()
        .filter(|p| p.kind == "Actor" && !p.name.is_empty())
        .map(|p| Cast {
            tag: p.name.clone(),
            role: p.role.clone().unwrap_or_default(),
            thumb: person_thumb(p),
            id: 0, // Plex's numeric person id has no counterpart; `tag_key` is the address
            tag_key: p.id.clone(),
        })
        .collect()
}

fn person_thumb(p: &PersonDto) -> String {
    match &p.primary_image_tag {
        Some(t) => format!("/Items/{}/Images/Primary?tag={t}", p.id),
        None => String::new(),
    }
}

/// The "Directed by …" line: one job's names, in order, deduped.
fn crew_names(people: &[PersonDto], job: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in people.iter().filter(|p| p.kind == job && !p.name.is_empty()) {
        if !out.contains(&p.name) {
            out.push(p.name.clone());
        }
    }
    out
}

/// Director + Writer folded into credits for the shelf's tail — `metadata::crew_credits`' exact
/// rule (one tile per person, jobs joined) over Jellyfin's one-array `People[]`.
fn crew_credits(people: &[PersonDto]) -> Vec<Cast> {
    let mut out: Vec<Cast> = Vec::new();
    for job in ["Director", "Writer"] {
        for p in people.iter().filter(|p| p.kind == job && !p.name.is_empty()) {
            match out.iter_mut().find(|c| c.tag == p.name) {
                Some(c) if !c.role.ends_with(job) => {
                    c.role.push_str(", ");
                    c.role.push_str(job);
                }
                Some(_) => {}
                None => out.push(Cast {
                    tag: p.name.clone(),
                    role: job.to_string(),
                    thumb: person_thumb(p),
                    id: 0,
                    tag_key: p.id.clone(),
                }),
            }
        }
    }
    out
}

fn episode_from_dto(it: &BaseItemDto) -> Episode {
    let (resume_ms, watched) = match &it.user_data {
        Some(ud) => (ud.playback_position_ticks / 10_000, ud.played),
        None => (0, false),
    };
    let (mut vcodec, mut acodec) = (String::new(), String::new());
    let mut part = String::new();
    if let Some(src) = it.media_sources.as_ref().and_then(|ss| ss.first()) {
        part = src.id.clone();
        for st in &src.streams {
            match st.kind.as_str() {
                "Video" if vcodec.is_empty() => vcodec = st.codec.clone().unwrap_or_default(),
                "Audio" if acodec.is_empty() => acodec = st.codec.clone().unwrap_or_default(),
                _ => {}
            }
        }
    }
    Episode {
        rk: it.id.clone(),
        index: it.index_number.unwrap_or(0) as i64,
        season: it.parent_index.unwrap_or(0) as i64,
        title: it.name.clone(),
        summary: it.overview.clone().unwrap_or_default(),
        aired: it
            .premiere_date
            .as_deref()
            .map(date_part)
            .unwrap_or_default(),
        dur_ms: it.run_time_ticks.unwrap_or(0) / 10_000,
        thumb: match it.image_tags.get("Primary") {
            Some(t) => format!("/Items/{}/Images/Primary?tag={t}", it.id),
            None => String::new(),
        },
        resume_ms,
        watched,
        part,
        rating: it.official_rating.clone().unwrap_or_default(),
        vcodec,
        acodec,
    }
}

pub(crate) fn season_from_dto(it: &BaseItemDto) -> Season {
    let leaf = it.child_count.unwrap_or(0);
    let unplayed = it
        .user_data
        .as_ref()
        .and_then(|ud| ud.unplayed_item_count)
        .unwrap_or(leaf);
    Season {
        rk: it.id.clone(),
        index: it.index_number.unwrap_or(0) as i64,
        title: it.name.clone(),
        leaf_count: leaf,
        viewed_leaf_count: leaf - unplayed,
    }
}

/// `MediaSegments` → the player's intro/credits markers. Only Intro and Outro survive — the
/// Skip pill's vocabulary — and a degenerate range is dropped rather than offered, the rule
/// `metadata::convert_markers` states and its tests pin.
fn convert_segments(s: &super::dto::SegmentsResult) -> Vec<crate::metadata::Marker> {
    s.items
        .iter()
        .filter_map(|seg| {
            let kind = match seg.kind.as_str() {
                "Intro" => crate::metadata::MarkerKind::Intro,
                "Outro" => crate::metadata::MarkerKind::Credits,
                _ => return None,
            };
            let (start_ms, end_ms) = (seg.start_ticks / 10_000, seg.end_ticks / 10_000);
            (end_ms > start_ms && start_ms >= 0).then_some(crate::metadata::Marker {
                kind,
                start_ms,
                end_ms,
                final_seg: false, // Jellyfin does not mark the final segment
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A leaf detail record as a 10.11 server sends it (shape verified live; values trimmed).
    const DETAIL_JSON: &str = r#"{
        "Id": "m1", "Name": "Big Buck Bunny", "Type": "Movie", "ProductionYear": 2008,
        "Overview": "A giant rabbit takes revenge.", "Taglines": ["Watch out."],
        "OfficialRating": "PG", "RunTimeTicks": 596000000,
        "PremiereDate": "2008-05-30T00:00:00.0000000Z",
        "Genres": ["Animation", "Comedy"], "ProductionLocations": ["Netherlands"],
        "ImageTags": {"Primary": "ptag"}, "BackdropImageTags": ["btag"],
        "UserData": {"Played": false, "PlayCount": 0, "PlaybackPositionTicks": 150000000},
        "People": [
            {"Name": "Sacha Goedegebure", "Id": "p1", "Type": "Director", "PrimaryImageTag": "dt"},
            {"Name": "Bunny", "Id": "p2", "Role": "Himself", "Type": "Actor", "PrimaryImageTag": "at"}
        ],
        "Chapters": [{"Name": "Opening", "StartPositionTicks": 0}],
        "MediaSources": [{
            "Id": "ms1", "Container": "mkv", "Path": "/media/bbb.mkv",
            "Size": 732042240, "Bitrate": 98280000,
            "MediaStreams": [
                {"Type": "Video", "Codec": "hevc", "Index": 0, "Width": 3840, "Height": 1608,
                 "RealFrameRate": 24.0, "AspectRatio": "2.40:1", "VideoRangeType": "HDR10",
                 "BitRate": 95000000, "Profile": "Main 10", "BitDepth": 10},
                {"Type": "Audio", "Codec": "eac3", "Index": 1, "Language": "eng",
                 "DisplayTitle": "English - EAC3 - 5.1 - Default", "Channels": 6,
                 "ChannelLayout": "5.1", "IsDefault": true, "IsForced": false, "IsExternal": false},
                {"Type": "Subtitle", "Codec": "subrip", "Index": 2, "Language": "spa",
                 "DisplayTitle": "Spanish - SUBRIP", "IsDefault": false, "IsForced": false,
                 "IsExternal": true, "IsHearingImpaired": false}
            ]
        }]
    }"#;

    #[test]
    fn a_leaf_record_maps_the_whole_detail() {
        let it: BaseItemDto = serde_json::from_str(DETAIL_JSON).unwrap();
        let d = detail_from_dto(&it, ServerId::from_raw(0));
        assert_eq!(d.kind, "movie");
        assert!(!d.is_show);
        assert_eq!(d.title, "Big Buck Bunny");
        assert_eq!(d.year, 2008);
        assert_eq!(d.tagline, "Watch out.");
        assert_eq!(d.aired, "2008-05-30");
        assert_eq!(d.dur_ms, 59_600);
        assert_eq!(d.resume_ms, 15_000);
        assert!(!d.watched);
        assert_eq!(d.genres, vec!["Animation", "Comedy"]);
        assert_eq!(d.countries, vec!["Netherlands"]);
        assert_eq!(d.directors, vec!["Sacha Goedegebure"]);
        assert_eq!(d.cast.len(), 1);
        assert_eq!(d.cast[0].role, "Himself");
        assert_eq!(d.cast[0].thumb, "/Items/p2/Images/Primary?tag=at");
        assert_eq!(d.crew.len(), 1);
        assert_eq!(d.crew[0].role, "Director");
        assert_eq!(d.chapters.len(), 1);
        assert_eq!(d.thumb, "/Items/m1/Images/Primary?tag=ptag");
        assert_eq!(d.art, "/Items/m1/Images/Backdrop/0");

        // the playable block
        assert_eq!(d.part, "ms1");
        assert_eq!(d.vcodec, "hevc");
        assert_eq!(d.acodec, "eac3");
        assert_eq!(d.container, "mkv");
        assert_eq!(d.file, "/media/bbb.mkv");
        assert_eq!(d.size, 732042240);
        assert_eq!(d.bitrate, 98_280); // bps → kbps
        assert_eq!(d.width, 3840);
        assert_eq!(d.height, 1608);
        assert_eq!(d.video_resolution, "1080"); // 1608 lines of scope IS 1080p
        assert!((d.video_fps - 24.0).abs() < 0.01);
        assert!((d.aspect_ratio - 2.4).abs() < 0.01);
        assert!(d.hdr);
        assert!(!d.dovi.present); // "HDR10" is not a DOVI range
        assert_eq!(d.audio.len(), 1);
        assert_eq!(d.audio[0].index, 1);
        assert!(d.audio[0].default);
        assert_eq!(d.subs.len(), 1);
        assert!(d.subs[0].external);
    }

    #[test]
    fn a_dovi_range_derives_the_dv_layering() {
        let it: BaseItemDto = serde_json::from_str(
            r#"{"Id":"d1","Name":"DV","Type":"Movie",
                "MediaSources":[{"Id":"ms2","Container":"mkv","MediaStreams":[
                    {"Type":"Video","Codec":"hevc","Index":0,"Width":3840,"Height":2160,
                     "VideoRangeType":"DOVIWithHDR10"}]}]}"#,
        )
        .unwrap();
        let d = detail_from_dto(&it, ServerId::from_raw(0));
        assert!(d.dovi.present);
        assert_eq!(d.dovi.bl_compat, 1); // HDR10 base layer
        assert_eq!(d.dovi.profile, 0); // a named base layer names no profile
        assert!(!d.dovi.el_present);
        assert!(d.hdr);

        // bare DOVI declares no compatible base layer at all — read as Profile 5 (see the note)
        let it: BaseItemDto = serde_json::from_str(
            r#"{"Id":"d2","Name":"DV5","Type":"Movie",
                "MediaSources":[{"Id":"ms3","MediaStreams":[
                    {"Type":"Video","Codec":"hevc","Index":0,"VideoRangeType":"DOVI"}]}]}"#,
        )
        .unwrap();
        let d = detail_from_dto(&it, ServerId::from_raw(0));
        assert!(d.dovi.present);
        assert_eq!(d.dovi.bl_compat, 0);
        assert_eq!(d.dovi.profile, 5);

        // the two other named base layers map to their compat ids, and non-DOVI stays silent
        for (range, compat) in [("DOVIWithHLG", 4), ("DOVIWithSDR", 2)] {
            let it: BaseItemDto = serde_json::from_str(&format!(
                r#"{{"Id":"d3","Name":"DV","Type":"Movie",
                    "MediaSources":[{{"Id":"ms4","MediaStreams":[
                        {{"Type":"Video","Codec":"hevc","Index":0,"VideoRangeType":"{range}"}}]}}]}}"#
            ))
            .unwrap();
            let d = detail_from_dto(&it, ServerId::from_raw(0));
            assert!(d.dovi.present, "range {range}");
            assert_eq!(d.dovi.bl_compat, compat, "range {range}");
        }

        let it: BaseItemDto = serde_json::from_str(
            r#"{"Id":"d4","Name":"SDR","Type":"Movie",
                "MediaSources":[{"Id":"ms5","MediaStreams":[
                    {"Type":"Video","Codec":"hevc","Index":0,"VideoRangeType":"SDR"}]}]}"#,
        )
        .unwrap();
        let d = detail_from_dto(&it, ServerId::from_raw(0));
        assert!(!d.dovi.present);
    }

    /// Bare `DOVI` is the whole point of issue #77: it names no base layer, so it must never
    /// fall through to `NotDv` (feeding IPT-PQ to an ordinary HEVC decoder shows wrong colours).
    /// The converter reads it as Profile 5, so the plan carries it to the same decision a Plex P5
    /// item gets: declared to the panel when we say so, refused otherwise.
    #[test]
    fn a_bare_dovi_range_is_declared_as_profile_5_not_played_as_a_base_layer() {
        use crate::metadata::DvPresentation;
        use crate::webos::caps::DvCapability::Supported;

        let dovi = dovi_from_range(Some("DOVI"));
        assert_eq!((dovi.profile, dovi.bl_compat, dovi.el_present), (5, 0, false));
        assert!(dovi.base_layer_unusable(), "IPT-PQ has no base layer to fall back on");

        // withheld signal (or unconfirmed support): refuse, never NotDv
        assert_eq!(
            dovi.presentation(false, Supported, true),
            DvPresentation::Refuse("no cross-compatible base layer")
        );
        assert_eq!(dovi.presentation(false, Supported, true).declared(), None);

        // signal + confirmed support + hevc: declare Profile 5 to the panel
        let declared = dovi.presentation(true, Supported, true).declared();
        assert_eq!(declared.map(|n| n.profile_id), Some(5));
    }

    #[test]
    fn dovi_from_range_pins_the_honest_partial_record() {
        // the detail path above covers the mapping end-to-end; this pins the shared helper the
        // `playing_item` path uses: a named base layer keeps profile 0 (unknown), bare `DOVI`
        // reads as Profile 5, and a non-DOVI or absent range reads as no DV at all.
        assert_eq!(dovi_from_range(Some("DOVIWithHDR10")).bl_compat, 1);
        assert_eq!(dovi_from_range(Some("doviwithhlg")).bl_compat, 4); // case-insensitive
        assert_eq!(dovi_from_range(Some("DOVI")).bl_compat, 0);
        assert_eq!(dovi_from_range(Some("DOVI")).profile, 5);
        assert!(dovi_from_range(Some("DOVIWithHDR10")).present);
        assert_eq!(dovi_from_range(Some("DOVIWithHDR10")).profile, 0);
        assert!(!dovi_from_range(Some("DOVIWithHDR10")).el_present);
        assert_eq!(dovi_from_range(Some("HDR10")), Dovi::NONE);
        assert_eq!(dovi_from_range(Some("HDR10Plus")), Dovi::NONE);
        assert_eq!(dovi_from_range(None), Dovi::NONE);
    }

    #[test]
    fn a_jellyfin_leaf_owns_a_file_so_the_languages_column_is_available() {
        // Issue #33: the Languages MORE panel is gated on `Detail::has_own_file()` (a non-empty
        // `part`). On Jellyfin a leaf's `part` is the MediaSource id, not a path, so a movie with
        // media sources MUST own a file and enable Languages; a show container has no file of its
        // own and hides the column BY DESIGN.
        let it: BaseItemDto = serde_json::from_str(
            r#"{"Id":"m1","Name":"Movie","Type":"Movie",
                "MediaSources":[{"Id":"ms1","MediaStreams":[
                    {"Type":"Video","Codec":"hevc","Index":0}]}]}"#,
        )
        .unwrap();
        let d = detail_from_dto(&it, ServerId::from_raw(0));
        assert!(d.has_own_file(), "a Jellyfin movie with a media source owns a file");

        let show: BaseItemDto = serde_json::from_str(
            r#"{"Id":"s1","Name":"Show","Type":"Series","ChildCount":3}"#,
        )
        .unwrap();
        let d = detail_from_dto(&show, ServerId::from_raw(0));
        assert!(!d.has_own_file(), "a Jellyfin show has no file of its own");
    }

    #[test]
    fn a_show_counts_watched_through_the_complement() {
        let it: BaseItemDto = serde_json::from_str(
            r#"{"Id":"s1","Name":"Show","Type":"Series","ChildCount":10,
                "UserData":{"Played":false,"PlayCount":0,"PlaybackPositionTicks":0,
                            "UnplayedItemCount":3}}"#,
        )
        .unwrap();
        let d = detail_from_dto(&it, ServerId::from_raw(0));
        assert_eq!(d.kind, "show");
        assert!(!d.watched); // 7 of 10 viewed
        // A show's own playable fields stay EMPTY — the play path reads "show" off them.
        assert!(d.part.is_empty());
    }

    #[test]
    fn seasons_and_episodes_map_their_counts_and_units() {
        let season: BaseItemDto = serde_json::from_str(
            r#"{"Id":"se1","Name":"Season 1","Type":"Season","IndexNumber":1,"ChildCount":8,
                "UserData":{"Played":false,"PlayCount":0,"PlaybackPositionTicks":0,
                            "UnplayedItemCount":8}}"#,
        )
        .unwrap();
        let s = season_from_dto(&season);
        assert_eq!((s.index, s.leaf_count, s.viewed_leaf_count), (1, 8, 0));

        let ep: BaseItemDto = serde_json::from_str(
            r#"{"Id":"e1","Name":"Pilot","Type":"Episode","IndexNumber":1,"ParentIndexNumber":1,
                "RunTimeTicks":26400000000,"PremiereDate":"2020-01-01T00:00:00Z",
                "ImageTags":{"Primary":"et"},
                "UserData":{"Played":true,"PlayCount":1,"PlaybackPositionTicks":0},
                "MediaSources":[{"Id":"ms9","MediaStreams":[
                    {"Type":"Video","Codec":"h264"},{"Type":"Audio","Codec":"aac"}]}]}"#,
        )
        .unwrap();
        let e = episode_from_dto(&ep);
        assert_eq!(e.rk, "e1");
        assert_eq!(e.dur_ms, 2_640_000);
        assert!(e.watched);
        assert_eq!(e.part, "ms9");
        assert_eq!(e.vcodec, "h264");
        assert_eq!(e.acodec, "aac");
        assert_eq!(e.thumb, "/Items/e1/Images/Primary?tag=et");
    }

    #[test]
    fn only_intro_and_outro_segments_become_markers() {
        let s: super::super::dto::SegmentsResult = serde_json::from_str(
            r#"{"Items":[
                {"Type":"Intro","StartTicks":50000000,"EndTicks":900000000},
                {"Type":"Commercial","StartTicks":1000000000,"EndTicks":2000000000},
                {"Type":"Outro","StartTicks":50000000000,"EndTicks":54000000000},
                {"Type":"Intro","StartTicks":900,"EndTicks":100}
            ]}"#,
        )
        .unwrap();
        let m = convert_segments(&s);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].kind, crate::metadata::MarkerKind::Intro);
        assert_eq!((m[0].start_ms, m[0].end_ms), (5_000, 90_000));
        assert_eq!(m[1].kind, crate::metadata::MarkerKind::Credits);
    }
}
