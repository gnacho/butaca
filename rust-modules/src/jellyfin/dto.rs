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

use serde::{Deserialize, Serialize};

/// `POST /Users/AuthenticateByName` 200. The access token is a credential: it is stored in the
/// client and never logged (the client's parse log names the endpoint only).
#[derive(Debug, Deserialize)]
pub(crate) struct AuthResponse {
    #[serde(rename = "AccessToken")]
    pub(crate) access_token: String,
    #[serde(rename = "User")]
    pub(crate) user: AuthUser,
    /// The server's own machine id, the counterpart of the retired `machineIdentifier`,
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

/// `POST /QuickConnect/Initiate` / `GET /QuickConnect/Connect` — one Quick Connect request's
/// state. Only what the flow reads is declared: the `Secret` (the poll/authenticate credential),
/// the `Code` the viewer types on another device, and the `Authenticated` flag the poll waits on.
/// The other members (`DeviceId`, `DeviceName`, `AppName`, `AppVersion`, `DateAdded`) are
/// deliberately undeclared — this app does not read them, and `serde_json` ignores the rest.
#[derive(Debug, Deserialize)]
pub(crate) struct QuickConnectResult {
    #[serde(rename = "Authenticated", default)]
    pub(crate) authenticated: bool,
    #[serde(rename = "Secret")]
    pub(crate) secret: String,
    #[serde(rename = "Code")]
    pub(crate) code: String,
}

/// `GET /Users/{userId}/Views`: the library list (the counterpart of the retired
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
/// (the retired `GET /` `friendlyName` counterpart). Public by design: no user scope, no token.
#[derive(Debug, Deserialize)]
pub(crate) struct PublicInfo {
    #[serde(rename = "ServerName")]
    pub(crate) server_name: String,
    /// The server's machine id, the retired `machineIdentifier` counterpart.
    #[serde(rename = "Id", default)]
    pub(crate) id: String,
    /// Dotted `Major.Minor.Patch` version — `client::quick_connect_supported` reads it to gate
    /// Quick Connect on 10.8+.
    #[serde(rename = "Version", default)]
    pub(crate) version: String,
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
    /// The extra's kind when the item is one ("Trailer", "BehindTheScenes", "DeletedScene",
    /// "Interview", "Scene", "Sample", "Featurette"). Absent on ordinary items.
    #[serde(rename = "ExtraType")]
    pub(crate) extra_type: Option<String>,
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
    /// The CRITICS' score as a percentage, 0-100. It is Rotten Tomatoes' own number: the server's
    /// OMDb provider parses RT's `"85%"` into this field (`OmdbProvider`), and no other provider
    /// writes it. Absent whenever no provider supplied one, which is the ordinary case on a server
    /// without OMDb, and absence is what keeps the window from inventing a score (issue #317).
    #[serde(rename = "CriticRating")]
    pub(crate) critic_rating: Option<f32>,
    /// Per-image-type cache-buster tags. A Primary exists iff this map names one. (`BTreeMap`,
    /// not `HashMap`: this crate builds serde no-std, where only the alloc maps implement
    /// `Deserialize`.)
    #[serde(rename = "ImageTags", default)]
    pub(crate) image_tags: std::collections::BTreeMap<String, String>,
    /// Per-image-type BlurHash strings: image type (`Primary`, `Backdrop`) to image tag to hash.
    /// The hash identifies the picture it was computed from, so a new image is a new hash.
    #[serde(rename = "ImageBlurHashes", default)]
    pub(crate) image_blur_hashes:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
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
    /// Marketing one-liners, whose first is the About panel's line (the retired `tagline`).
    #[serde(rename = "Taglines", default)]
    pub(crate) taglines: Vec<String>,
    #[serde(rename = "Genres", default)]
    pub(crate) genres: Vec<String>,
    /// Cast AND crew in one array, told apart by `PersonDto::kind` ("Actor"/"Director"/"Writer").
    #[serde(rename = "People", default)]
    pub(crate) people: Vec<PersonDto>,
    /// Production companies / studios, in server order. The detail hero's "Studios" line.
    #[serde(rename = "Studios", default)]
    pub(crate) studios: Vec<StudioDto>,
    #[serde(rename = "Chapters", default)]
    pub(crate) chapters: Vec<ChapterDto>,
    /// Where it was made, the retired `Country[]` row on the About panel.
    #[serde(rename = "ProductionLocations", default)]
    pub(crate) production_locations: Vec<String>,
    /// ISO timestamp ("1976-11-12T00:00:00.0000000Z") — only its date part is ever drawn.
    #[serde(rename = "PremiereDate")]
    pub(crate) premiere_date: Option<String>,
    /// The other bookend, present only for finished runs: a person's death date, a series' end.
    /// Same ISO shape; same date-part-only consumption.
    #[serde(rename = "EndDate")]
    pub(crate) end_date: Option<String>,
    /// Season/folder only: how many leaves it holds (the retired `leafCount`).
    #[serde(rename = "ChildCount")]
    pub(crate) child_count: Option<i64>,
    /// The trickplay manifest, present only when the server generated trickplay images AND the
    /// query asked for them (`Fields=Trickplay`). Outer key is the tile width as a string ("320"),
    /// inner key the sprite-sheet index; each value describes one sheet's grid and interval.
    /// Empty/absent means the item has no trickplay.
    #[serde(rename = "Trickplay", default)]
    pub(crate) trickplay: std::collections::BTreeMap<String, std::collections::BTreeMap<String, TrickplayInfoDto>>,
}

/// One sprite sheet of the trickplay manifest (`BaseItemDto::trickplay`). Width/Height are a
/// single thumbnail's pixels, TileWidth/TileHeight the grid (thumbnails per row/column),
/// Interval the milliseconds between thumbnails, Bandwidth the sheet's peak bits/second. Mirrors
/// Jellyfin's `TrickplayInfoDto` field-for-field; only what the seek-bar reader needs is declared.
#[derive(Debug, Deserialize)]
pub(crate) struct TrickplayInfoDto {
    #[serde(rename = "Width")]
    pub(crate) width: i32,
    #[serde(rename = "Height")]
    pub(crate) height: i32,
    #[serde(rename = "TileWidth")]
    pub(crate) tile_width: i32,
    #[serde(rename = "TileHeight")]
    pub(crate) tile_height: i32,
    #[serde(rename = "ThumbnailCount")]
    pub(crate) thumbnail_count: i32,
    #[serde(rename = "Interval")]
    pub(crate) interval: i32,
    #[serde(rename = "Bandwidth")]
    pub(crate) bandwidth: i32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UserDataDto {
    /// Fully watched, the direct counterpart of the retired `viewCount > 0` for a leaf, and of
    /// `viewedLeafCount >= leafCount` when the leaf counts are aggregated server-side.
    #[serde(rename = "Played")]
    pub(crate) played: bool,
    #[serde(rename = "PlayCount")]
    pub(crate) play_count: i64,
    /// Resume point in 100-ns ticks; 0 when unwatched or finished.
    #[serde(rename = "PlaybackPositionTicks")]
    pub(crate) playback_position_ticks: i64,
    /// Container rows only: leaves NOT yet played under it. The retired wire sent `viewedLeafCount`
    /// and the app computed the complement. Jellyfin sends the complement directly.
    /// The heart: `UserData.IsFavorite` (absent/false = not favorited).
    #[serde(default, rename = "IsFavorite")]
    pub(crate) is_favorite: bool,
    #[serde(rename = "UnplayedItemCount")]
    pub(crate) unplayed_item_count: Option<i64>,
    /// RFC3339, the Continue Watching deck's merge key (the retired `lastViewedAt`).
    #[serde(rename = "LastPlayedDate")]
    pub(crate) last_played_date: Option<String>,
}

/// One cast/crew row (`People[]`). `kind` is Jellyfin's job vocabulary: "Actor" carries `role`
/// as the character name; "Director"/"Writer" are crew and their job is the kind itself —
/// exactly the shape `metadata::Cast` already uses for crew rows.
#[derive(Debug, Deserialize)]
pub(crate) struct PersonDto {
    #[serde(rename = "Name")]
    pub(crate) name: String,
    /// The person's own item id — their headshot is `/Items/{id}/Images/Primary`, and the id is
    /// the person page's address (the retired `tagKey`).
    #[serde(rename = "Id")]
    pub(crate) id: String,
    #[serde(rename = "Role")]
    pub(crate) role: Option<String>,
    #[serde(rename = "Type")]
    pub(crate) kind: String,
    #[serde(rename = "PrimaryImageTag")]
    pub(crate) primary_image_tag: Option<String>,
}

/// **One candidate from a metadata provider's lookup**, `POST /Items/RemoteSearch/{type}` 200,
/// and the body the APPLY call takes back (`POST /Items/RemoteSearch/Apply/{itemId}`).
///
/// It is `Deserialize` AND `Serialize` on purpose: the server re-reads the chosen candidate's
/// `ProviderIds` and refreshes the item from them, so the object the search returned is posted back
/// **verbatim**. Every field is optional and skipped when absent, because a candidate carries only
/// what its provider answered, an IMDb hit has no `IndexNumber`, a TMDB one no `ImageUrl`, and
/// rebuilding the payload from this client's own idea of a query is exactly what would make Apply
/// search for something the viewer never picked.
///
/// `ProviderIds` is the load-bearing member (it is what Apply looks up); the rest is what the row
/// says and what the server may keep for the item's own record.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub(crate) struct RemoteSearchResultDto {
    #[serde(rename = "Name", default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(rename = "ProviderIds", default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(crate) provider_ids: std::collections::BTreeMap<String, String>,
    #[serde(rename = "ProductionYear", default, skip_serializing_if = "Option::is_none")]
    pub(crate) production_year: Option<i32>,
    #[serde(rename = "PremiereDate", default, skip_serializing_if = "Option::is_none")]
    pub(crate) premiere_date: Option<String>,
    #[serde(rename = "ImageUrl", default, skip_serializing_if = "Option::is_none")]
    pub(crate) image_url: Option<String>,
    /// WHICH provider offered this candidate ("TheMovieDb", "The Open Movie Database"). The row's
    /// read-out: two providers routinely return the same film, and only this tells them apart.
    #[serde(rename = "SearchProviderName", default, skip_serializing_if = "Option::is_none")]
    pub(crate) search_provider_name: Option<String>,
    #[serde(rename = "Overview", default, skip_serializing_if = "Option::is_none")]
    pub(crate) overview: Option<String>,
    #[serde(rename = "IndexNumber", default, skip_serializing_if = "Option::is_none")]
    pub(crate) index_number: Option<i32>,
    #[serde(rename = "IndexNumberEnd", default, skip_serializing_if = "Option::is_none")]
    pub(crate) index_number_end: Option<i32>,
    #[serde(rename = "ParentIndexNumber", default, skip_serializing_if = "Option::is_none")]
    pub(crate) parent_index_number: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChapterDto {    #[serde(rename = "Name")]
    pub(crate) name: Option<String>,
    /// 100-ns ticks — the module doc's units rule applies (`/ 10_000` for the seek target).
    #[serde(rename = "StartPositionTicks")]
    pub(crate) start_position_ticks: i64,
}

/// One studio / production company on an item (`BaseItemDto::studios`). Only the name is drawn.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct StudioDto {
    #[serde(rename = "Name")]
    pub(crate) name: String,
}

/// `GET /Items/{itemId}/RemoteImages`: the pictures the metadata providers have for one image
/// type of one item, as the server already knows them (issue #369, *Edit images*). Only what the
/// picker draws is declared: the URL it fetches the thumbnail from, the type the server filed it
/// under, the provider that offered it, and the pixel size the provider reports.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct RemoteImageResultDto {
    #[serde(rename = "Images", default)]
    pub(crate) images: Vec<RemoteImageInfoDto>,
    /// How many the server holds IN TOTAL, which may exceed the page asked for. The picker says
    /// so rather than letting a bounded page read as the whole set.
    #[serde(rename = "TotalRecordCount", default)]
    pub(crate) total_record_count: i32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RemoteImageInfoDto {
    /// The provider's own absolute URL. It is NOT on this server: the thumbnail fetch dials it
    /// as it stands (see `JfClient::get_bytes`), and `RemoteImages/Download` takes it back.
    #[serde(rename = "Url", default)]
    pub(crate) url: String,
    /// The provider's own SMALLER copy of the same picture, when it publishes one (the server
    /// fills this for TheMovieDb: `…/original/…` becomes `…/w300/…`). The picker draws this and
    /// keeps `Url` for the download, so a grid of twenty posters does not decode twenty
    /// full-resolution JPEGs on a television.
    #[serde(rename = "ThumbnailUrl", default)]
    pub(crate) thumbnail_url: Option<String>,
    /// "Primary" | "Backdrop" | "Logo" | "Art" | "Banner" | "Thumb" | "Disc".
    #[serde(rename = "Type", default)]
    pub(crate) kind: String,
    #[serde(rename = "ProviderName", default)]
    pub(crate) provider_name: String,
    #[serde(rename = "Width", default)]
    pub(crate) width: i32,
    #[serde(rename = "Height", default)]
    pub(crate) height: i32,
    /// The picture's language as the provider tags it ("en", "es", …). Absent for the ones that
    /// carry no language at all, which is most posters.
    #[serde(rename = "Language", default)]
    pub(crate) language: Option<String>,
}

/// `GET /Items/{itemId}/Intros` — the `IIntroProvider` seam the Local Intros plugin plugs into
/// (issue #370). Verified against Jellyfin's own OpenAPI (`BaseItemDtoQueryResult`, stable and
/// 13.x alike): the answer is the ordinary `{ Items: [...] }` envelope, and a server WITHOUT the
/// plugin answers `Items: []` rather than a 404, because the route is core and the plugin only
/// supplies the providers. `StartIndex`/`TotalRecordCount` are not read: the endpoint takes no
/// paging and answers the whole list.
#[derive(Debug, Deserialize)]
pub(crate) struct IntrosResult {
    #[serde(rename = "Items", default)]
    pub(crate) items: Vec<BaseItemDto>,
}

/// `GET /MediaSegments/{itemId}` (Jellyfin 10.9+) — intro/credits ranges, the counterpart of
/// the retired `Marker[]`. Absent endpoint (older server, plugin not installed) is a 404, which the
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

/// `GET /Plugins/MdbListRatings/CachedByItemId?itemId={id}`, the MDBList Ratings plugin's own
/// cache (issue #370).
///
/// **There is no ratings array on `BaseItemDto`.** Jellyfin 12.4's schema carries exactly two
/// scores (`CommunityRating` 0-10 and `CriticRating` 0-100) and the plugin does not add a third
/// field to the item payload: it writes ONE admin-chosen source into those two fields and keeps
/// every other source in a cache it exposes over its own REST route
/// (`Jellyfin.Plugin.MdbListRatings/Api/CachedRatingsController.cs`). That route is the proof the
/// plugin is installed: a server without it answers 404, which is folded to "no extra sources".
#[derive(Debug, Default, Deserialize)]
pub(crate) struct MdbListCached {
    #[serde(default)]
    pub(crate) ratings: Vec<MdbListRatingDto>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MdbListRatingDto {
    /// The provider's own slug: "imdb", "tmdb", "trakt", "tomatoes", "popcorn", "metacritic",
    /// "letterboxd", "rogerebert", "myanimelist", "anilist" (the plugin's documented list).
    #[serde(default)]
    pub(crate) source: String,
    /// **The normalised 0-100 score**, which the plugin's own model calls "the best field to use
    /// consistently". The native `value` beside it is deliberately NOT read: it is 0-10 for IMDb,
    /// 0-100 for Trakt and 0-5 for Letterboxd, a per-source scale the payload never states.
    #[serde(default, deserialize_with = "de_soft_score")]
    pub(crate) score: Option<f64>,
}

/// The plugin hands MDBList's numbers through lenient converters (the wire sometimes carries
/// `"7.4"`, sometimes `""` or `"N/A"`), so a value this client cannot read is an absent score
/// rather than a parse failure that would cost the whole payload.
fn de_soft_score<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    let number = match value {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    Ok(number.filter(|n| n.is_finite()))
}

/// The `POST /Items/{id}/PlaybackInfo` answer — Jellyfin's Media Decision Engine verdict, the
/// counterpart of the retired `/decision`. We read two things off it: WHICH source the server
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
    /// decision is taken on STREAMS, matching how the retired path trusted `Stream` over `Part`.
    #[serde(rename = "Container")]
    pub(crate) container: Option<String>,
    #[serde(rename = "MediaStreams", default)]
    pub(crate) streams: Vec<MediaStreamDto>,
    /// The part's absolute path ON THE SERVER (the Track-information panel's header line, the
    /// retired `Part.file`). Not a URL, not reachable from the TV.
    #[serde(rename = "Path")]
    pub(crate) path: Option<String>,
    #[serde(rename = "Size")]
    pub(crate) size: Option<i64>,
    /// Whole-stream bitrate in bps (the retired `Media.bitrate` was kbps, the converter divides).
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
    /// (`?AudioStreamIndex=`/`SubtitleStreamIndex=`). The old wire had a stream id and an index,
    /// while Jellyfin addresses by index alone, so the internal `Stream.id` mirrors this value.
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
    /// whole input. Anything that is not SDR is HDR for the tone-mapping warning; the base-layer
    /// Dolby Vision layering (`metadata::Dovi`) is derived from the `DOVI…` spellings by
    /// `detail::dovi_from_range` (present + base-layer compat; bare `DOVI` reads as Profile 5).
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
    /// Sidecar file vs embedded, the player treats the two differently (retired `Stream.external`).
    #[serde(rename = "IsExternal", default)]
    pub(crate) is_external: bool,
    /// SDH flag (the retired `hearingImpaired`), Jellyfin 10.9+.
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
        assert_eq!(it.critic_rating, None, "a server without OMDb sends no critic score");
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

    /// The critics' score parses off the wire name and keeps the server's percentage. Absence is
    /// the ordinary case and stays `None` (see the field's own doc).
    #[test]
    fn the_critic_rating_is_optional_and_keeps_its_percentage() {
        let r: ItemsResult = serde_json::from_str(
            r#"{"Items": [{"Id": "x", "Name": "A", "Type": "Movie", "CriticRating": 85.0}],
                "TotalRecordCount": 1}"#,
        )
        .unwrap();
        assert_eq!(r.items[0].critic_rating, Some(85.0));
    }

    /// A candidate from a provider's lookup parses its wire names and posts back the SAME object:
    /// Apply looks the item up by the ids in it, so the round trip is the contract.
    #[test]
    fn a_remote_search_candidate_round_trips_through_apply() {
        let wire = r#"{"Name": "Alien", "ProviderIds": {"Tmdb": "348", "Imdb": "tt0078748"},
            "ProductionYear": 1979, "ImageUrl": "https://image.tmdb.org/x.jpg",
            "SearchProviderName": "TheMovieDb"}"#;
        let hit: RemoteSearchResultDto = serde_json::from_str(wire).unwrap();
        assert_eq!(hit.name.as_deref(), Some("Alien"));
        assert_eq!(hit.production_year, Some(1979));
        assert_eq!(hit.provider_ids.get("Imdb").map(String::as_str), Some("tt0078748"));
        let back = serde_json::to_value(&hit).unwrap();
        assert_eq!(back["ProviderIds"]["Tmdb"], "348", "the ids Apply looks up survive");
        assert_eq!(back["Name"], "Alien");
        assert_eq!(back["SearchProviderName"], "TheMovieDb");
        assert!(back.get("IndexNumber").is_none(), "absent members are not nulled into the body");
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
