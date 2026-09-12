//! dto — serde shapes of the Jellyfin REST payloads this app reads.
//!
//! Field-for-field mirrors of the JSON as Jellyfin 10.9+ sends it (verified against a live
//! 10.11 server during the port). Only the fields the app READS are declared: a BaseItemDto
//! carries over a hundred members, and declaring one nobody reads is a lie about what the code
//! depends on. `serde_json` ignores everything undeclared, so a newer server adding fields is
//! never a parse failure.
//!
//! Naming: Jellyfin uses PascalCase. Every field carries its `rename` explicitly rather than
//! leaning on a container-wide rename rule, so each line greps to the wire name it parses.

use serde::Deserialize;

/// `POST /Users/AuthenticateByName` 200. The access token is a credential: it is stored in the
/// client and never logged (the client's parse log names the endpoint only).
#[derive(Debug, Deserialize)]
pub(crate) struct AuthResponse {
    #[serde(rename = "AccessToken")]
    pub(crate) access_token: String,
    #[serde(rename = "User")]
    pub(crate) user: AuthUser,
    /// The server's own machine id — the Jellyfin counterpart of a Plex `machineIdentifier`,
    /// persisted so a later session can notice it is talking to a DIFFERENT server at the same
    /// address rather than silently wearing stale credentials.
    #[serde(rename = "ServerId")]
    pub(crate) server_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AuthUser {
    #[serde(rename = "Id")]
    pub(crate) id: String,
    #[serde(rename = "Name")]
    pub(crate) name: String,
}

/// `GET /Users/{userId}/Views` — the library list (Jellyfin's counterpart of PMS
/// `/library/sections`). Each view is one browsable library; its `Id` is the `ParentId` a
/// section listing is scoped by.
#[derive(Debug, Deserialize)]
pub(crate) struct ViewsResult {
    #[serde(rename = "Items")]
    pub(crate) items: Vec<ViewDto>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ViewDto {
    #[serde(rename = "Id")]
    pub(crate) id: String,
    #[serde(rename = "Name")]
    pub(crate) name: String,
    /// `CollectionType`: "movies" / "tvshows" / "music" / … — `None` on a mixed library, which
    /// the browse layer must therefore tolerate rather than assume.
    #[serde(rename = "CollectionType")]
    pub(crate) collection_type: Option<String>,
}

/// `GET /System/Info/Public` — the server naming ITSELF, for the Sources list's group header
/// (Plex's `GET /` `friendlyName` counterpart). Public by design: no user scope, no token.
#[derive(Debug, Deserialize)]
pub(crate) struct PublicInfo {
    #[serde(rename = "ServerName")]
    pub(crate) server_name: String,
}

/// A paged listing: `/Users/{userId}/Items`, `/Items/Latest`, resume and search all answer in
/// this envelope.
#[derive(Debug, Deserialize)]
pub(crate) struct ItemsResult {
    #[serde(rename = "Items")]
    pub(crate) items: Vec<BaseItemDto>,
    #[serde(rename = "TotalRecordCount")]
    pub(crate) total: i64,
}

/// The one item shape every Jellyfin endpoint returns — movie, series, season and episode all
/// arrive as a `BaseItemDto`; `kind` is what tells them apart.
#[derive(Debug, Deserialize)]
pub(crate) struct BaseItemDto {
    #[serde(rename = "Id")]
    pub(crate) id: String,
    #[serde(rename = "Name")]
    pub(crate) name: String,
    /// "Movie" | "Series" | "Season" | "Episode" (kept a String, not an enum: an unknown kind —
    /// "BoxSet", "MusicAlbum" — must be skippable by the converter, not a parse error).
    #[serde(rename = "Type")]
    pub(crate) kind: String,
    #[serde(rename = "ProductionYear")]
    pub(crate) year: Option<i32>,
    #[serde(rename = "Overview")]
    pub(crate) overview: Option<String>,
    /// 100-ns ticks — see the module doc's units rule. `None` on folders and unprobed files.
    #[serde(rename = "RunTimeTicks")]
    pub(crate) run_time_ticks: Option<i64>,
    /// The content rating string as the server stores it ("PG-13", "TV-MA").
    #[serde(rename = "OfficialRating")]
    pub(crate) official_rating: Option<String>,
    #[serde(rename = "CommunityRating")]
    pub(crate) community_rating: Option<f32>,
    /// Per-image-type cache-buster tags. A Primary exists iff this map names one. (`BTreeMap`,
    /// not `HashMap`: this crate builds serde no-std, where only the alloc maps implement
    /// `Deserialize`.)
    #[serde(rename = "ImageTags", default)]
    pub(crate) image_tags: std::collections::BTreeMap<String, String>,
    #[serde(rename = "BackdropImageTags", default)]
    pub(crate) backdrop_tags: Vec<String>,
    /// Episode/season only: the containing series.
    #[serde(rename = "SeriesId")]
    pub(crate) series_id: Option<String>,
    #[serde(rename = "SeriesName")]
    pub(crate) series_name: Option<String>,
    /// Episode: season number. (A season's own number arrives in `index_number`.)
    #[serde(rename = "ParentIndexNumber")]
    pub(crate) parent_index: Option<i32>,
    /// Episode: episode number within the season. Season: the season number.
    #[serde(rename = "IndexNumber")]
    pub(crate) index_number: Option<i32>,
    #[serde(rename = "UserData")]
    pub(crate) user_data: Option<UserDataDto>,
    /// Present only when the query asked for `Fields=MediaSources` — the listing queries that
    /// feed direct-play gating do, and any converter reading it must treat `None` as "unknown",
    /// never as "no media".
    #[serde(rename = "MediaSources")]
    pub(crate) media_sources: Option<Vec<MediaSourceDto>>,
    // ---- detail-page fields (present on the single-item fetch; empty on listings) ----
    /// Marketing one-liners; the first is the About panel's line (Plex's `tagline`).
    #[serde(rename = "Taglines", default)]
    pub(crate) taglines: Vec<String>,
    #[serde(rename = "Genres", default)]
    pub(crate) genres: Vec<String>,
    /// Cast AND crew in one array, told apart by `PersonDto::kind` ("Actor"/"Director"/"Writer").
    #[serde(rename = "People", default)]
    pub(crate) people: Vec<PersonDto>,
    #[serde(rename = "Chapters", default)]
    pub(crate) chapters: Vec<ChapterDto>,
    /// Where it was made — Plex's `Country[]` row on the About panel.
    #[serde(rename = "ProductionLocations", default)]
    pub(crate) production_locations: Vec<String>,
    /// ISO timestamp ("1976-11-12T00:00:00.0000000Z") — only its date part is ever drawn.
    #[serde(rename = "PremiereDate")]
    pub(crate) premiere_date: Option<String>,
    /// The other bookend, present only for finished runs: a person's death date, a series' end.
    /// Same ISO shape; same date-part-only consumption.
    #[serde(rename = "EndDate")]
    pub(crate) end_date: Option<String>,
    /// Season/folder only: how many leaves it holds (Plex's `leafCount`).
    #[serde(rename = "ChildCount")]
    pub(crate) child_count: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UserDataDto {
    /// Fully watched — the direct counterpart of Plex's `viewCount > 0` for a leaf, and of
    /// `viewedLeafCount >= leafCount` when the leaf counts are aggregated server-side.
    #[serde(rename = "Played")]
    pub(crate) played: bool,
    #[serde(rename = "PlayCount")]
    pub(crate) play_count: i64,
    /// Resume point in 100-ns ticks; 0 when unwatched or finished.
    #[serde(rename = "PlaybackPositionTicks")]
    pub(crate) playback_position_ticks: i64,
    /// Container rows only: leaves NOT yet played under it. Plex sends `viewedLeafCount` and the
    /// app computes the complement; Jellyfin sends the complement directly.
    #[serde(rename = "UnplayedItemCount")]
    pub(crate) unplayed_item_count: Option<i64>,
    /// RFC3339 — the Continue Watching deck's merge key (Plex's `lastViewedAt`).
    #[serde(rename = "LastPlayedDate")]
    pub(crate) last_played_date: Option<String>,
}

/// One cast/crew row (`People[]`). `kind` is Jellyfin's job vocabulary: "Actor" carries `role`
/// as the character name; "Director"/"Writer" are crew and their job is the kind itself —
/// exactly the shape `metadata::Cast` already uses for Plex crew rows.
#[derive(Debug, Deserialize)]
pub(crate) struct PersonDto {
    #[serde(rename = "Name")]
    pub(crate) name: String,
    /// The person's own item id — their headshot is `/Items/{id}/Images/Primary`, and the id is
    /// the person page's address (Plex's `tagKey`).
    #[serde(rename = "Id")]
    pub(crate) id: String,
    #[serde(rename = "Role")]
    pub(crate) role: Option<String>,
    #[serde(rename = "Type")]
    pub(crate) kind: String,
    #[serde(rename = "PrimaryImageTag")]
    pub(crate) primary_image_tag: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChapterDto {
    #[serde(rename = "Name")]
    pub(crate) name: Option<String>,
    /// 100-ns ticks — the module doc's units rule applies (`/ 10_000` for the seek target).
    #[serde(rename = "StartPositionTicks")]
    pub(crate) start_position_ticks: i64,
}

/// `GET /MediaSegments/{itemId}` (Jellyfin 10.9+) — intro/credits ranges, the counterpart of
/// Plex's `Marker[]`. Absent endpoint (older server, plugin not installed) is a 404, which the
/// fetch folds to an empty list: a show simply shows no Skip pill.
#[derive(Debug, Deserialize)]
pub(crate) struct SegmentsResult {
    #[serde(rename = "Items", default)]
    pub(crate) items: Vec<SegmentDto>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SegmentDto {
    /// "Intro" | "Outro" | "Commercial" | "Preview" | "Recap" — only the first two are offered
    /// to the viewer (the Skip pill's vocabulary; the rest are skipped at conversion).
    #[serde(rename = "Type")]
    pub(crate) kind: String,
    #[serde(rename = "StartTicks")]
    pub(crate) start_ticks: i64,
    #[serde(rename = "EndTicks")]
    pub(crate) end_ticks: i64,
}

/// The `POST /Items/{id}/PlaybackInfo` answer — Jellyfin's Media Decision Engine verdict, the
/// counterpart of Plex's `/decision`. We read exactly two things off it: WHICH source the server
/// chose (so a multi-version item plays the one the server adjudicated) and HOW it must be
/// delivered (`TranscodingUrl` present → HLS; absent → the file direct-plays). The full stream
/// list of the chosen source is NOT re-read here — the playing-item store already carries it
/// from the detail fetch.
#[derive(Debug, Deserialize)]
pub(crate) struct PlaybackInfoResult {
    #[serde(rename = "MediaSources", default)]
    pub(crate) media_sources: Vec<PlaybackMediaSource>,
    /// The session id every progress/started/stopped POST of this playback must quote, and the
    /// transcode session's kill handle. On a DIRECT play the server leaves this empty and the
    /// client mints its own — `route` already carries one (`Plan::sess`).
    #[serde(rename = "PlaySessionId")]
    pub(crate) play_session_id: Option<String>,
    /// Present when the server refuses outright (`"NotImplemented"` et al.) — the pre-flight
    /// refusal read-out quotes it, like `Plan::verdict` quoting `generalDecisionCode` 2000.
    #[serde(rename = "ErrorCode")]
    pub(crate) error_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PlaybackMediaSource {
    #[serde(rename = "Id")]
    pub(crate) id: String,
    /// The transcoder's HLS address, RELATIVE to the server root ("/videos/{id}/master.m3u8?…"
    /// — note lower-case, unlike the API's own "/Videos" routes). Absent on a direct play.
    #[serde(rename = "TranscodingUrl")]
    pub(crate) transcoding_url: Option<String>,
    #[serde(rename = "SupportsDirectStream", default)]
    pub(crate) supports_direct_stream: bool,
    #[serde(rename = "SupportsTranscoding", default)]
    pub(crate) supports_transcoding: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MediaSourceDto {
    #[serde(rename = "Id")]
    pub(crate) id: String,
    /// Container short name ("mkv", "mp4", "mov,mp4,m4a,…") — informational; the direct-play
    /// decision is taken on STREAMS, matching how the Plex path trusts `Stream` over `Part`.
    #[serde(rename = "Container")]
    pub(crate) container: Option<String>,
    #[serde(rename = "MediaStreams", default)]
    pub(crate) streams: Vec<MediaStreamDto>,
    /// The part's absolute path ON THE SERVER (the Track-information panel's header line; Plex's
    /// `Part.file`). Not a URL, not reachable from the TV.
    #[serde(rename = "Path")]
    pub(crate) path: Option<String>,
    #[serde(rename = "Size")]
    pub(crate) size: Option<i64>,
    /// Whole-stream bitrate in bps (Plex's `Media.bitrate` is kbps — the converter divides).
    #[serde(rename = "Bitrate")]
    pub(crate) bitrate: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MediaStreamDto {
    /// "Video" | "Audio" | "Subtitle" — same String-not-enum reasoning as [`BaseItemDto::kind`].
    #[serde(rename = "Type")]
    pub(crate) kind: String,
    /// Lower-case codec id as Jellyfin reports it ("hevc", "h264", "eac3", "aac", "subrip",
    /// "pgs"). The converter up-cases nothing; `metadata::friendly_codec` already knows these.
    #[serde(rename = "Codec")]
    pub(crate) codec: Option<String>,
    /// The stream's index WITHIN THE MEDIA SOURCE — what playback selection addresses
    /// (`?AudioStreamIndex=`/`SubtitleStreamIndex=`). Plex has both a stream id and an index;
    /// Jellyfin addresses by index alone, so the internal `Stream.id` mirrors this value.
    #[serde(rename = "Index")]
    pub(crate) index: Option<i32>,
    #[serde(rename = "Language")]
    pub(crate) language: Option<String>,
    /// The server's own display name for the track ("English - AAC - Stereo - Default") — used
    /// as the track TITLE; the language column keeps the ISO code.
    #[serde(rename = "DisplayTitle")]
    pub(crate) display_title: Option<String>,
    #[serde(rename = "Channels")]
    pub(crate) channels: Option<i32>,
    #[serde(rename = "ChannelLayout")]
    pub(crate) channel_layout: Option<String>,
    /// Per-stream bitrate, bps (the quality ladder's source-kbps read; divided by 1000).
    #[serde(rename = "BitRate")]
    pub(crate) bit_rate: Option<i64>,
    #[serde(rename = "Profile")]
    pub(crate) profile: Option<String>,
    #[serde(rename = "BitDepth")]
    pub(crate) bit_depth: Option<i32>,
    /// "SDR" | "HDR10" | "HDR10Plus" | "HLG" | "DOVI" | "DOVIWithHDR10" | … — the `hdr` flag's
    /// whole input. Anything that is not SDR is HDR for the tone-mapping warning; the finer
    /// Dolby Vision layering (`metadata::Dovi`) is NOT derived from it — see convert's note.
    #[serde(rename = "VideoRangeType")]
    pub(crate) video_range_type: Option<String>,
    #[serde(rename = "Width")]
    pub(crate) width: Option<i32>,
    #[serde(rename = "Height")]
    pub(crate) height: Option<i32>,
    #[serde(rename = "AspectRatio")]
    pub(crate) aspect_ratio: Option<String>,
    #[serde(rename = "RealFrameRate")]
    pub(crate) real_frame_rate: Option<f32>,
    #[serde(rename = "IsDefault", default)]
    pub(crate) is_default: bool,
    #[serde(rename = "IsForced", default)]
    pub(crate) is_forced: bool,
    /// Sidecar file vs embedded — the player treats them differently (Plex's `Stream.external`).
    #[serde(rename = "IsExternal", default)]
    pub(crate) is_external: bool,
    /// SDH flag (Plex's `hearingImpaired`); Jellyfin 10.9+.
    #[serde(rename = "IsHearingImpaired")]
    pub(crate) is_sdh: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real 10.11 server answer (demo.jellyfin.org, 2026-09), trimmed to one item — the point
    /// of the fixture is that the field NAMES are the wire's own, so a rename typo fails here.
    const MOVIE_JSON: &str = r#"{
        "Items": [{
            "Name": "The Boy in the Plastic Bubble",
            "ServerId": "f0b3381645f04afb9a0e392e74b6a1b0",
            "Id": "edb39341c5039551a5157e51fe4a3364",
            "Container": "mov,mp4,m4a,3gp,3g2,mj2",
            "PremiereDate": "1976-11-12T00:00:00.0000000Z",
            "OfficialRating": "PG",
            "Genres": ["TV Movie", "Drama"],
            "CommunityRating": 5.7,
            "RunTimeTicks": 55842000000,
            "ProductionYear": 1976,
            "IsFolder": false,
            "Type": "Movie",
            "UserData": {
                "PlayedPercentage": 21.704078471401452,
                "PlaybackPositionTicks": 12119991500,
                "PlayCount": 20,
                "IsFavorite": false,
                "Played": false,
                "Key": "edb39341-c503-9551-a515-7e51fe4a3364",
                "ItemId": "edb39341c5039551a5157e51fe4a3364"
            },
            "ImageTags": { "Primary": "a131fdbdca9ddadcda7d3c86756c6b67" },
            "BackdropImageTags": ["408be2dfc34f1ce61108cf049a5aa82e"],
            "LocationType": "FileSystem",
            "MediaType": "Video",
            "MediaSources": [{
                "Protocol": "File", "Id": "edb39341c5039551a5157e51fe4a3364",
                "Container": "mov,mp4,m4a,3gp,3g2,mj2", "SupportsDirectPlay": true,
                "MediaStreams": [
                    { "Codec": "h264", "Type": "Video" },
                    { "Codec": "aac", "Type": "Audio" }
                ]
            }]
        }],
        "TotalRecordCount": 11
    }"#;

    #[test]
    fn items_result_parses_wire_names() {
        let r: ItemsResult = serde_json::from_str(MOVIE_JSON).unwrap();
        assert_eq!(r.total, 11);
        let it = &r.items[0];
        assert_eq!(it.id, "edb39341c5039551a5157e51fe4a3364");
        assert_eq!(it.kind, "Movie");
        assert_eq!(it.year, Some(1976));
        assert_eq!(it.run_time_ticks, Some(55842000000));
        assert_eq!(it.official_rating.as_deref(), Some("PG"));
        assert_eq!(
            it.image_tags.get("Primary").map(String::as_str),
            Some("a131fdbdca9ddadcda7d3c86756c6b67")
        );
        assert_eq!(it.backdrop_tags.len(), 1);
        let ud = it.user_data.as_ref().unwrap();
        assert!(!ud.played);
        assert_eq!(ud.play_count, 20);
        assert_eq!(ud.playback_position_ticks, 12119991500);
        let ms = it.media_sources.as_ref().unwrap();
        assert_eq!(ms[0].streams.len(), 2);
        assert_eq!(ms[0].streams[0].codec.as_deref(), Some("h264"));
    }

    #[test]
    fn unknown_kind_and_missing_fields_do_not_fail_parsing() {
        let r: ItemsResult = serde_json::from_str(
            r#"{"Items": [{"Id": "x", "Name": "Mix", "Type": "BoxSet"}], "TotalRecordCount": 1}"#,
        )
        .unwrap();
        let it = &r.items[0];
        assert_eq!(it.kind, "BoxSet");
        assert!(it.user_data.is_none());
        assert!(it.media_sources.is_none());
        assert!(it.image_tags.is_empty());
    }
}
