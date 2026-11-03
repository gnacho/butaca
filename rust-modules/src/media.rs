//! serde response DTOs — only the fields the app consumes. The top-level `MediaContainer` is
//! required; fields inside it default to mirror the "all optional" reality of PMS JSON (a trimmed
//! container does not fail to deserialize). `kind` renames the JSON `type` field (a Rust keyword).
//! These are read-only: the app's own view structs are populated *from* them.
use serde::Deserialize;

/// `{ "MediaContainer": … }` — every list/detail response.
#[derive(Deserialize, Default)]
pub struct Envelope {
    #[serde(rename = "MediaContainer")]
    pub media_container: MediaContainer,
}

/// One flat container; every list field is optional so the same type deserializes a sections
/// list (`Directory`), an items/detail list (`Metadata`), or a hub list (`Hub`).
#[derive(Deserialize, Default)]
pub struct MediaContainer {
    #[serde(rename = "Metadata", default)]
    pub metadata: Vec<Metadata>,
    /// Position of the selected item within the WHOLE queue (not within the returned window).
    /// /transcode/universal/decision verdict codes — Option (not defaulted 0) so the route
    /// log distinguishes "absent" from a real code, matching the old find_num scan.
    #[serde(
        rename = "generalDecisionCode",
        default,
        deserialize_with = "de_opt_i64"
    )]
    pub general_decision_code: Option<i64>,
    /// The TRANSCODE lane's own verdict, beside the general one above (`4007` = "cannot convert
    /// this item"). Same `Option` discipline and for the same reason, doubled here: the pre-flight
    /// refusal is graded on a CODE, and a defaulted 0 would be a code this server never sent.
    /// …and the two HUMAN sentences PMS pairs with those codes — "Cannot convert this item.
    /// Implementation for video encoder 'vp9' not found." / "Neither direct play nor conversion is
    /// available." (both probed live against PMS 1.43.3). They are the server's own wording, quoted
    /// verbatim by the player's failure read-out, so they are read but never parsed: the CODE is
    /// what the app decides on. Absent stays `""` — an empty sentence draws no line at all, which
    /// is the honest reading of "the server named no reason".
    #[serde(rename = "transcodeDecisionText", default, deserialize_with = "de_str")]
    pub transcode_decision_text: String,
    #[serde(rename = "generalDecisionText", default, deserialize_with = "de_str")]
    pub general_decision_text: String,
    /// `GET /` (the server root) only: does the server's OWNER hold a Plex Pass? Bool on PMS 1.43
    /// (probed live 2026-08-10), and lenient like every flag here because PMS string-encodes
    /// freely. `Option`, not a defaulted 0: a server old enough not to say must stay UNKNOWN —
    /// defaulting would read as "no Plex Pass", a confident wrong answer on the field issue #22
    /// (docs/plex-pass-audit.md) exists to get right. Consumed by `serverinfo.rs` only.
    #[serde(
        rename = "myPlexSubscription",
        default,
        deserialize_with = "de_opt_i64"
    )]
    pub my_plex_subscription: Option<i64>,
    /// `GET /` only: the PMS build string ("1.43.3.10861-cd85035e7"). Diagnostics
    /// (`serverinfo.rs`); every other endpoint simply omits it.
    #[serde(default)]
    pub version: String,
}



/// A SHOW's language settings — its Advanced dialog in Plex Web. `None` / `-1` mean "Account
/// default", inherited from the active Plex profile when its preferences are available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShowLangPrefs {
    /// `audioLanguage`, e.g. `"hu-HU"`
    pub audio: Option<String>,
    /// `subtitleLanguage`, e.g. `"hu-HU"`
    pub subtitle: Option<String>,
    /// `subtitleMode`: -1 account default · 0 manually selected · 1 shown with foreign audio ·
    /// 2 always enabled
    pub subtitle_mode: i32,
}

impl Default for ShowLangPrefs {
    fn default() -> Self { Self { audio: None, subtitle: None, subtitle_mode: -1 } }
}

impl ShowLangPrefs {
}


/// The movie/show/season/episode item. Missing fields default (Plex omits optionals).
#[derive(Deserialize, Default)]
pub struct Metadata {
    #[serde(rename = "ratingKey", default, deserialize_with = "de_route_key")]
    pub rating_key: String,
    /// The LIBRARY this row lives in, on the server that answered ("Movies", "Film Club"). Sent on
    /// a cross-library query such as `/library/all?guid=…`, which is the one place the app asks a
    /// server something without already knowing which of its libraries will answer.
    #[serde(rename = "librarySectionTitle", default, deserialize_with = "de_str")]
    pub library_section_title: String,
    #[serde(default, deserialize_with = "de_i64")]
    pub duration: i64, // ms
    #[serde(rename = "grandparentRatingKey", default, deserialize_with = "de_route_key")]
    pub grandparent_rating_key: String, // episode → its show
    #[serde(rename = "Media", default)]
    pub media: Vec<Media>,
    #[serde(rename = "Chapter", default)]
    pub chapter: Vec<Chapter>,
    #[serde(rename = "Marker", default)]
    pub marker: Vec<Marker>,
    // D-1: PMS returns UltraBlurColors as an ARRAY `[{…}]` (the old code reads it as an object
    // and misses it). `de_ultrablur` accepts object OR array and yields the first.
    #[serde(rename = "UltraBlurColors", default, deserialize_with = "de_ultrablur")]
    pub ultra_blur_colors: Option<UltraBlurColors>,
}

impl Metadata {
    /// Media[0].Part[0] — the primary playable part (None for a show container, which carries
    /// no Media). The part holds the direct-play `key` and the `Stream[]` list.
    pub fn first_part(&self) -> Option<&MediaPart> {
        self.media.first().and_then(|m| m.part.first())
    }

    /// `Media[0]` — the FIRST version listed, **not** a chosen-best one, and the honest name for
    /// what every caller here actually reads. An item can carry SEVERAL `Media[]` versions
    /// (docs/pms-api.md §4; the dev library really does — one episode ships a 4k and a 1080
    /// version, another two 4k versions at different bitrates), and picking among them by
    /// codec/resolution needs a version picker this client does not have yet. So anything derived
    /// from this describes **version 0**, and a UI that shows it should be read that way.
    /// [`first_part`](Self::first_part) carries the same caveat.
    pub fn primary_media(&self) -> Option<&Media> {
        self.media.first()
    }
}

#[derive(Deserialize, Default)]
pub struct Media {
    /// Whole-stream bitrate in **kbps** (PMS's unit) — the source rung of the video-quality ladder
    /// ("26.1 Mbps 4K (Original)") and the input to any bandwidth cap.
    #[serde(default, deserialize_with = "de_i64")]
    pub bitrate: i64,
    /// Coded frame size. Note it is the STORED size, not the resolution class: a 2.35:1 1080p
    /// movie is 1918x802 on this server, so `height` alone would label it 720p — prefer
    /// [`video_resolution`](Self::video_resolution) for a badge and keep these for the ladder.
    #[serde(default, deserialize_with = "de_i64")]
    pub width: i64,
    #[serde(default, deserialize_with = "de_i64")]
    pub height: i64,
    /// PMS's coarse resolution class, always a STRING even when it looks numeric — verified live:
    /// `"4k"`, `"1080"`, `"720"`, `"576"`, `"sd"`. The version-picker key per docs/pms-api.md §4.
    #[serde(rename = "videoResolution", default, deserialize_with = "de_str")]
    pub video_resolution: String,
    #[serde(rename = "Part", default)]
    pub part: Vec<MediaPart>,
}

#[derive(Deserialize, Default)]
pub struct MediaPart {
    #[serde(rename = "Stream", default)]
    pub stream: Vec<Stream>,
}

impl MediaPart {
}

/// D-2: channels/title/hearingImpaired/audioDescription/forced are real PMS fields the spec
/// omits — kept here. Plex 0/1 booleans stay i64; the app tests `!= 0`.
#[derive(Deserialize, Default)]
pub struct Stream {
    #[serde(default, deserialize_with = "de_i64")]
    pub id: i64,
    #[serde(rename = "streamType", default, deserialize_with = "de_i64")]
    pub stream_type: i64, // 1 video, 2 audio, 3 subtitle
    /// PMS's stream index within the part — container (ffmpeg) order. The track mapping sorts
    /// by this instead of trusting Stream[] document order (PMS may reorder), so the demuxer's
    /// nth-of-type selection can't drift from the row the user picked.
    #[serde(default, deserialize_with = "de_i64")]
    pub index: i64,
    /// Delivery path — present ONLY on external/sidecar streams (a downloaded .srt has
    /// key=/library/streams/{id}; embedded container streams carry no key). The client
    /// renderer uses this to exclude sidecars from the embedded-subtitle mapping.
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub codec: String,
    /// `/decision` only: this lane's verdict — `"directplay"` | `"transcode"` | `"copy"`.
    /// [`MediaPart::video_forbids_copy`] is the playback reader; a library metadata body omits it.
    #[serde(default)]
    pub decision: String,
    #[serde(default)]
    pub language: String,
    #[serde(rename = "languageCode", default)]
    pub language_code: String, // ISO-639 code, e.g. "eng" (route audio-track pick)
    /// The BCP-47 tag ("es-419", "en-GB") — `language_code` is only the primary subtag.
    /// `default` because PMS omits it on a stream with no language.
    #[serde(rename = "languageTag", default)]
    pub language_tag: String,
    // Video stream only: source fps for the Load esInfo. Lenient (number OR numeric string)
    // so a non-numeric frameRate never fails the whole detail parse — matches the old jfloat.
    #[serde(rename = "frameRate", default, deserialize_with = "de_f64")]
    pub frame_rate: f64,
    /// Video stream only: Dolby Vision present — HDR whatever the transfer characteristic says.
    /// Lenient because PMS sends bools as `true` and as `"1"` depending on the endpoint.
    #[serde(rename = "DOVIPresent", default, deserialize_with = "de_i64")]
    pub dovi_present: i64,
    // ---- Dolby Vision, beyond "present". These three decide whether the file's BASE LAYER is a
    // correct picture on its own, which is the only question the buffer-feed pipeline can act on:
    // we feed ONE elementary stream to a decoder that knows nothing about an RPU, so what the
    // panel gets IS the base layer. `metadata::Dovi` is where they are read together, and its
    // `base_layer_unusable` is the rule.
    //
    // The field names are NOT in the upstream Plex API reference — its `stream` schema carries 29
    // properties and not one DOVI among them, so the spec cannot settle them and neither can
    // pms-api.md. These spellings were probed LIVE against the dev PMS 2026-08-21, by sweeping
    // every movie and episode on it — 540 leaves, of which 28 carry Dolby Vision (8 movies and
    // 20 episodes) across 34 video streams, and **every one of the 34 sends the same eight keys**,
    // which is the fact that matters: no shape on this server reports a profile without also
    // reporting a compatibility id. The full set is `DOVIPresent`,
    // `DOVIProfile`, `DOVILevel`, `DOVIVersion`, `DOVIBLPresent`, `DOVIELPresent`,
    // `DOVIRPUPresent`, `DOVIBLCompatID`. Numbers arrive as JSON numbers and the flags as real
    // JSON booleans (`DOVIELPresent: false`), which is why every one of them is [`de_i64`].
    /// `DOVIProfile` — 5, 7, 8 … (0 = the server did not say). Profile is what names the
    /// single-layer IPT-PQ case (5) that has no HDR10 fallback at all.
    #[serde(rename = "DOVIProfile", default, deserialize_with = "de_i64")]
    pub dovi_profile: i64,
    /// `DOVIBLCompatID` — the base layer's cross-compatibility id: 0 = none (Profile 5, IPT-PQ,
    /// displayable only by a DV-aware decoder), 1 = HDR10, 2 = SDR, 4 = HLG. Measured live:
    /// every P8 item on the dev server sends 1, the P5 item sends 0, and the P7 item sends **6**
    /// — so a `== 0` test alone does NOT catch Profile 7. See `DOVIELPresent`.
    #[serde(rename = "DOVIBLCompatID", default, deserialize_with = "de_i64")]
    pub dovi_bl_compat_id: i64,
    /// `DOVIELPresent` — an enhancement layer rides in the file (Profile 7's dual layer). We feed
    /// a single elementary stream and cannot interleave a second one, so this is disqualifying on
    /// its own, and it is the ONLY field that identifies P7 (whose compat id is 6, not 0).
    #[serde(rename = "DOVIELPresent", default, deserialize_with = "de_i64")]
    pub dovi_el_present: i64,
    // ---- the remaining four DOVI keys, DESCRIPTIVE ONLY. The three above decide whether the base
    // layer is a picture we can show; these four are read by the Track-information panel and by
    // nothing else, and adding a playback decision to them would need the same live sweep the
    // comment above records. All four were confirmed on the wire 2026-08-21 against
    // `/library/metadata/5` on the dev server (`DOVILevel: 6`, `DOVIVersion: "1.0"`,
    // `DOVIBLPresent: true`, `DOVIRPUPresent: true`).
    /// `DOVILevel` — the Dolby Vision LEVEL (a bitrate/resolution tier), `6` on the probed title.
    #[serde(rename = "DOVILevel", default, deserialize_with = "de_i64")]
    pub dovi_level: i64,
    /// `DOVIVersion` — a DOTTED version, sent as the JSON **string** `"1.0"`. [`de_str`] rather
    /// than [`de_i64`] because it is not an integer, and because a server that sent the bare
    /// number `1.0` would otherwise fail the whole `MediaContainer` parse.
    #[serde(rename = "DOVIVersion", default, deserialize_with = "de_str")]
    pub dovi_version: String,
    /// `DOVIBLPresent` — the base layer is in the file. A real JSON boolean on the dev server,
    /// hence [`de_i64`] like every other Plex flag.
    #[serde(rename = "DOVIBLPresent", default, deserialize_with = "de_i64")]
    pub dovi_bl_present: i64,
    /// `DOVIRPUPresent` — the reference processing unit (the dynamic metadata) is in the file.
    #[serde(rename = "DOVIRPUPresent", default, deserialize_with = "de_i64")]
    pub dovi_rpu_present: i64,
    /// Per-STREAM bitrate in **kbps** — `24723` for the video track and `448`…`768` for the audio
    /// ones on the probed title. Distinct from [`Media::bitrate`], which is the whole file: it is
    /// exactly what tells seven same-language AC3 tracks apart in the Track-information panel.
    #[serde(default, deserialize_with = "de_i64")]
    pub bitrate: i64,
    /// The codec profile, lower-case. **On an AUDIO stream this is where Atmos lives** —
    /// `"dolby digital plus + dolby atmos"`, confirmed on the wire 2026-08-21. Neither
    /// `audioChannelLayout` (`"5.1(side)"`) nor `title` (null) carries it, so a client that
    /// checked those would silently never badge an Atmos track. On a VIDEO stream it is
    /// `"main 10"` and matches the video codec profile.
    #[serde(default)]
    pub profile: String,
    /// Video stream only: bits per component — `10` on a Main 10 HEVC source.
    #[serde(rename = "bitDepth", default, deserialize_with = "de_i64")]
    pub bit_depth: i64,
    /// Video stream only: chroma subsampling as PMS spells it — `"4:2:0"`.
    #[serde(rename = "chromaSubsampling", default)]
    pub chroma_subsampling: String,
    #[serde(default, deserialize_with = "de_i64")]
    pub channels: i64,
    #[serde(rename = "audioChannelLayout", default)]
    pub audio_channel_layout: String,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "hearingImpaired", default, deserialize_with = "de_i64")]
    pub hearing_impaired: i64,
    #[serde(rename = "audioDescription", default, deserialize_with = "de_i64")]
    pub audio_description: i64,
    #[serde(default, deserialize_with = "de_i64")]
    pub forced: i64,
    // "default"/"selected" mark the file's default track and the server's current pick. Lenient
    // (number OR numeric string) like the other flags. `default` drives the "Original:" audio label.
    #[serde(rename = "default", default, deserialize_with = "de_i64")]
    pub is_default: i64,
    #[serde(default, deserialize_with = "de_i64")]
    pub selected: i64,
    /// Audio stream only: PMS 1.43.4+ with Plex Pass says this track's loudness was analyzed and
    /// the server can honor `normalizeLoudness=1` (and, paired with it, `boostDialog=1`) on the
    /// universal transcoder for it (issue #266). `de_bool` because it arrives as `"1"` on some
    /// endpoints and a real JSON bool on others; `default` because an old PMS or a video/subtitle
    /// stream never sends the key at all, and absent must mean false, never "unknown but assume
    /// yes" — offering the enhancement on a track the server can't actually normalize is exactly
    /// the failure mode I2/I5 exist to prevent.
    #[serde(rename = "canNormalizeLoudness", default, deserialize_with = "de_bool")]
    pub can_normalize_loudness: bool,
}

/// Plex `Chapter[]` on a leaf item (movies/episodes with chapter data). Sibling of `Media[]`,
/// present only with `?includeChapters=1`. `tag` is the title (often empty → synthesize
/// "Chapter N"); `thumb` is a server image path for the poster pipeline (empty if the server
/// never generated chapter thumbs). Offsets are ms; PMS string-encodes them → de_i64.
#[derive(Deserialize, Default)]
pub struct Chapter {
    #[serde(default, deserialize_with = "de_i64")]
    pub index: i64,
    #[serde(rename = "startTimeOffset", default, deserialize_with = "de_i64")]
    pub start_time_offset: i64,
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub thumb: String,
}

/// Plex `Marker[]` on a leaf item — the server-detected intro / credits segments, present only
/// with `?includeMarkers=1`. `kind` is `"intro"` | `"credits"` (PMS also emits `"commercial"` on
/// recorded content); offsets are ms into the item. `is_final` marks a credits segment that runs
/// to the end of the file — the usual case, and what makes "skip credits" equivalent to "finish".
///
/// Verified against the live server 2026-07-29: one episode carries both an intro
/// (0.99s–99.6s) and a `final` credits marker (3065.6s–3130.7s); two other episodes on the same
/// server carry credits only. The `id` field repeats across items (every marker came back as
/// `id: 3096`), so it is NOT an identity — the app keys markers by kind + offsets, never by id.
#[derive(Deserialize, Default)]
pub struct Marker {
    #[serde(rename = "type", default)]
    pub kind: String, // intro | credits | commercial
    #[serde(rename = "startTimeOffset", default, deserialize_with = "de_i64")]
    pub start_time_offset: i64,
    #[serde(rename = "endTimeOffset", default, deserialize_with = "de_i64")]
    pub end_time_offset: i64,
    /// `final: true` — this credits marker runs to the end of the item. (`final` is a reserved
    /// Rust keyword, hence the rename.) Arrives as a JSON bool; `de_i64` folds it to 1/0.
    #[serde(rename = "final", default, deserialize_with = "de_i64")]
    pub is_final: i64,
}


#[derive(Deserialize, Default, Clone, Copy)]
pub struct UltraBlurColors {
    #[serde(rename = "topLeft", default)]
    pub top_left: HexColor,
    #[serde(rename = "topRight", default)]
    pub top_right: HexColor,
    #[serde(rename = "bottomRight", default)]
    pub bottom_right: HexColor,
    #[serde(rename = "bottomLeft", default)]
    pub bottom_left: HexColor,
}

impl UltraBlurColors {
    /// The four corners in `Painter::ambient`'s order — top-left, top-**right**, bottom-**right**,
    /// bottom-left (a RING, not the reading order the JSON field names suggest) — or `None` when the
    /// envelope is empty. PMS sends the key on some items with every corner defaulted to black, and
    /// a pure-black gradient is not a colour scheme: keyed as one it paints a page darker than the
    /// app's own ground for no reason. TWO stores parse this field (the hub catalog in `pms.rs` and
    /// the detail store in `metadata.rs`), so the shape and the guard live here rather than being
    /// written twice.
    pub fn corners(self) -> Option<[[f32; 3]; 4]> {
        let c = [
            self.top_left.0,
            self.top_right.0,
            self.bottom_right.0,
            self.bottom_left.0,
        ];
        c.iter().any(|x| *x != [0.0, 0.0, 0.0]).then_some(c)
    }
}

/// "1a2b3c" (with/without '#') → linear [r,g,b] 0..1. Replaces pms::hex3.
#[derive(Deserialize, Default, Clone, Copy)]
#[serde(from = "String")]
pub struct HexColor(pub [f32; 3]);
impl From<String> for HexColor {
    fn from(s: String) -> Self {
        let v = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
        HexColor([
            ((v >> 16) & 0xff) as f32 / 255.0,
            ((v >> 8) & 0xff) as f32 / 255.0,
            (v & 0xff) as f32 / 255.0,
        ])
    }
}



/// Lenient f64: a JSON number, a numeric string, or null → 0.0 (matches the old `jfloat`
/// scrape). Called only when the field is present; a missing field uses `default` (0.0).
fn de_f64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumOrStr {
        N(f64),
        S(String),
    }
    Ok(match Option::<NumOrStr>::deserialize(d)? {
        Some(NumOrStr::N(n)) => n,
        Some(NumOrStr::S(s)) => s.parse().unwrap_or(0.0),
        None => 0.0,
    })
}

/// Lenient i64: a JSON integer, a float (truncated), a numeric string, or null → 0. The old
/// code read every number with `.as_i64().unwrap_or(0)`, degrading a bad value to 0 for THAT
/// field only. serde's strict i64 would instead fail the WHOLE container parse on a
/// string-encoded int (PMS does this: e.g. `size:"40"`, `streamType:"2"`), dropping every
/// item in the response. This keeps one field's-worth of degradation (and actually recovers a
/// stringy int rather than zeroing it). Applied to every i64 in a parsed DTO.
pub(crate) fn de_i64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IntFloatStrBool {
        I(i64),
        F(f64),
        S(String),
        // PMS sends the flag fields (default/selected/forced/…) as JSON booleans on some
        // endpoints (e.g. Stream inside /hubs) and as "1"/"0" strings on others. A missing bool
        // arm here fails the untagged enum → the WHOLE MediaContainer parse fails (blast radius),
        // so accept it: true→1, false→0.
        B(bool),
    }
    Ok(match Option::<IntFloatStrBool>::deserialize(d)? {
        Some(IntFloatStrBool::I(n)) => n,
        Some(IntFloatStrBool::F(f)) => f as i64,
        Some(IntFloatStrBool::S(s)) => s.trim().parse().unwrap_or(0),
        Some(IntFloatStrBool::B(b)) => b as i64,
        None => 0,
    })
}

/// Lenient `Vec<T>`: a JSON array, or null → empty. Same trap as [`de_str`], one container up:
/// `#[serde(default)]` fills an ABSENT field and does nothing for one that is present and `null`,
/// and a strict `Vec` meeting a `null` fails the whole array it sits in. `connections` is the field
/// that matters — a server with a null connection list must cost that server, never the roster.
pub(super) fn de_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

/// Lenient bool: a JSON bool, the `1`/`0` (or `"1"`/`"0"`) forms Plex also uses, or null → false.
///
/// The PMS DTOs above keep their 0/1 flags as `i64` and test `!= 0`, because those fields are read
/// once, near the wire. This adapter exists for the plex.tv **connection-policy** flags
/// (`account.rs`'s `httpsRequired`/`publicAddressMatches`/`local`/`relay`/`IPv6`), which are read in
/// boolean position by `probe.rs` on every candidate: folding the encodings here once beats
/// spelling `!= 0` at every branch of a policy that has to stay readable to be trusted. Leniency
/// itself is not optional — plex.tv sends these as real JSON bools today, and a strict `bool` that
/// meets a `"1"` fails the WHOLE resources parse, which is a silent sign-in failure.
pub(super) fn de_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BoolIntStr {
        B(bool),
        I(i64),
        S(String),
    }
    Ok(match Option::<BoolIntStr>::deserialize(d)? {
        Some(BoolIntStr::B(b)) => b,
        Some(BoolIntStr::I(n)) => n != 0,
        Some(BoolIntStr::S(s)) => matches!(s.trim(), "1" | "true" | "True"),
        None => false,
    })
}

/// Lenient Option<i64>: like `de_i64` but preserves "field absent/null" as None (the decision
/// verdict codes are logged as `Some(code)`/`None`, mirroring the old find_num scan).
fn de_opt_i64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IntStrBool {
        I(i64),
        S(String),
        // `myPlexSubscription` on the root envelope is a JSON BOOL on PMS 1.43. A missing arm
        // here fails the untagged enum → the WHOLE MediaContainer parse (`de_i64` learned the
        // same lesson) — which on `GET /` reads as "server info unavailable" forever.
        B(bool),
    }
    Ok(match Option::<IntStrBool>::deserialize(d)? {
        Some(IntStrBool::I(n)) => Some(n),
        Some(IntStrBool::S(s)) => s.trim().parse().ok(),
        Some(IntStrBool::B(b)) => Some(b as i64),
        None => None,
    })
}

/// Lenient String: a JSON string, a number stringified, or null → "". `videoResolution` is a
/// string on this server ("1080"/"4k", verified live) but reads like a number, and a strict
/// `String` that meets one is a WHOLE-`MediaContainer` parse failure — the same blast radius
/// `de_i64` exists to avoid, just pointing the other way. Use it for any stringly-typed number.
///
/// `pub(super)` because `account.rs` needs the null half of it: plex.tv sends an explicit `null` for
/// an absent string, and there a whole-container failure is the account's whole server list.
pub(super) fn de_str<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StrIntFloatBool {
        S(String),
        I(i64),
        F(f64),
        // no arm for a shape = the whole container fails on it, which is the one outcome this
        // adapter exists to prevent — so carry the bool too (`de_i64` learned that the hard way)
        B(bool),
    }
    Ok(match Option::<StrIntFloatBool>::deserialize(d)? {
        Some(StrIntFloatBool::S(s)) => s,
        Some(StrIntFloatBool::I(n)) => n.to_string(),
        Some(StrIntFloatBool::F(f)) => f.to_string(),
        Some(StrIntFloatBool::B(b)) => b.to_string(),
        None => String::new(),
    })
}

/// A route key the server handed us, safe to interpolate back into a request path: ids
/// (`ratingKey` and friends) and `/library/...` paths whose bytes are
/// `[A-Za-z0-9./_-]`. Anything else — a `?` that would truncate the path into a query, a `&`
/// that would inject a parameter, spaces, percent-escapes, control or non-ASCII bytes, or an
/// over-long blob — comes back EMPTY, which every consumer already treats as "no route" rather
/// than a request. The value only ever rides back to the server that sent it, so this is
/// request-shape hygiene at the parse boundary, not a trust boundary.
pub(super) fn de_route_key<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let raw = de_str(d)?;
    Ok(if is_route_key(&raw) { raw } else { String::new() })
}

fn is_route_key(s: &str) -> bool {
    if s.is_empty() || s.len() > 200 {
        return false;
    }
    let plain = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-');
    // Directory keys carry their own query (`/library/sections/1/all?actor=921`): the server
    // authored it, the app forwards it verbatim. ONE `?` separates it; its charset stays on the
    // list, so a bare `?` (truncation), spaces, `%`, control and non-ASCII bytes still refuse.
    let path = match s.split_once('?') {
        Some((p, q)) => {
            if q.is_empty()
                || !q
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b'&' | b'/' | b'.' | b'_' | b'-'))
            {
                return false;
            }
            p
        }
        None => s,
    };
    !path.is_empty() && path.bytes().all(plain)
}

/// Accept `{…}` OR `[{…}]` (D-1) and return the first, or None.
fn de_ultrablur<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<UltraBlurColors>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(UltraBlurColors),
        Many(Vec<UltraBlurColors>),
    }
    Ok(match Option::<OneOrMany>::deserialize(d)? {
        Some(OneOrMany::One(u)) => Some(u),
        Some(OneOrMany::Many(v)) => v.into_iter().next(),
        None => None,
    })
}

#[cfg(test)]
mod tests {
    use super::Envelope;

    /// The `Media[]` technical fields feed the resolution badge and (later) the quality ladder, and
    /// PMS is free to string-encode any of them. A strict field there does not lose one number —
    /// it fails the WHOLE `MediaContainer`, i.e. the entire detail page. Both encodings must land,
    /// and the several-versions case must stay several (`primary_media` is version 0, not the only
    /// one) — see docs/pms-api.md §4.
    #[test]
    fn media_technical_fields_survive_both_encodings_and_keep_every_version() {
        let json = br#"{"MediaContainer":{"Metadata":[{"ratingKey":"1859","title":"Example Movie",
            "Media":[
              {"bitrate":14663,"width":3840,"height":2160,"videoResolution":"4k","videoCodec":"hevc",
               "Part":[{"id":1,"key":"/library/parts/1/1/file.mkv"}]},
              {"bitrate":"2372","width":"1920","height":"1080","videoResolution":1080,
               "videoCodec":"h264","Part":[{"id":2,"key":"/library/parts/2/1/file.mkv"}]}
            ]}]}}"#;
        let env: Envelope = serde_json::from_slice(json).expect("lenient parse");
        let it = &env.media_container.metadata[0];
        assert_eq!(
            it.media.len(),
            2,
            "both versions are kept for a future picker"
        );

        let v0 = it.primary_media().expect("Media[0]");
        assert_eq!((v0.bitrate, v0.width, v0.height), (14663, 3840, 2160));
        assert_eq!(v0.video_resolution, "4k");
        // version 1 sends the same fields string-encoded, and videoResolution as a bare NUMBER
        let v1 = &it.media[1];
        assert_eq!((v1.bitrate, v1.width, v1.height), (2372, 1920, 1080));
        assert_eq!(v1.video_resolution, "1080");
        // the two versions therefore badge differently ("4K" vs "1080p" — see ui::fmt::resolution's
        // own tests), which is the whole reason the accessor has to name WHICH one it returned.
    }

    /// A show container carries no `Media` at all — the accessor must say so rather than panic,
    /// because the detail page asks every item for its primary version.
    #[test]
    fn a_show_container_has_no_primary_media() {
        let json = br#"{"MediaContainer":{"Metadata":[{"ratingKey":"9","type":"show","title":"A Show"}]}}"#;
        let env: Envelope = serde_json::from_slice(json).expect("parse");
        assert!(env.media_container.metadata[0].primary_media().is_none());
    }

    /// `canNormalizeLoudness` (issue #266) arrives as a real bool, as the string PMS also uses for
    /// its other flags, and — on an old server or a non-audio stream — not at all. Absent must
    /// parse to `false`, never fail the whole part like a strict field would.
    #[test]
    fn stream_parses_can_normalize_loudness_forms() {
        for (raw, want) in [
            (r#"{"streamType":2,"canNormalizeLoudness":"1"}"#, true),
            (r#"{"streamType":2,"canNormalizeLoudness":1}"#, true),
            (r#"{"streamType":2,"canNormalizeLoudness":true}"#, true),
            (r#"{"streamType":2}"#, false),
            (r#"{"streamType":2,"canNormalizeLoudness":"0"}"#, false),
        ] {
            let part: super::MediaPart =
                serde_json::from_slice(format!(r#"{{"Stream":[{raw}]}}"#).as_bytes())
                    .expect("parse");
            assert_eq!(
                part.stream[0].can_normalize_loudness, want,
                "input {raw}"
            );
        }
    }
}

#[cfg(test)]
mod route_key_allowlist_tests {
    use super::*;

    #[derive(Deserialize)]
    struct Probe {
        #[serde(default, deserialize_with = "de_route_key")]
        key: String,
    }

    fn key_of(raw: &str) -> String {
        serde_json::from_str::<Probe>(raw).expect("parse").key
    }

    #[test]
    fn route_keys_are_allow_listed_at_the_deserialization_boundary() {
        assert_eq!(key_of(r#"{"key":"12345"}"#), "12345");
        assert_eq!(
            key_of(r#"{"key":"/library/parts/1/1/file.mkv"}"#),
            "/library/parts/1/1/file.mkv"
        );
        assert_eq!(key_of(r#"{"key":"?next=/evil"}"#), "", "a ? would truncate the path into a query");
        assert_eq!(
            key_of(r#"{"key":"/library/sections/1/all?actor=921"}"#),
            "/library/sections/1/all?actor=921",
            "a directory key's query IS the key, and its charset is on the list",
        );
        assert_eq!(key_of(r#"{"key":"/x?"}"#), "", "an empty query is truncation");
        assert_eq!(key_of(r#"{"key":"1&x=2"}"#), "", "a & would inject a parameter");
        assert_eq!(key_of(r#"{"key":"a b"}"#), "", "a space would split the request line");
        assert_eq!(key_of(r#"{"key":"café"}"#), "", "non-ASCII is not on the list");
        assert_eq!(key_of(r#"{"key":"%2F..%2F"}"#), "", "percent-escapes are not on the list");
        let long = "a".repeat(201);
        assert_eq!(key_of(&format!(r#"{{"key":"{long}"}}"#)), "", "over-long is refused");
    }
}

/// Canon-only wire shapes since the plex.tv account surface was removed: the session-protocol
/// observation digest is frozen (SessionInit::write layout), so the recorded observation
/// vocabulary and the `/api/v2/resources` rows it named survive here with no producer. The
/// lenient adapters are `plex::models`' — the shapes must keep tolerating exactly what the
/// account API used to send, because old recordings are replayed against this code.
#[derive(serde::Deserialize, Default)]
pub(crate) struct Resource {
    #[serde(default, deserialize_with = "de_str")]
    pub name: String,
    #[serde(rename = "clientIdentifier", default, deserialize_with = "de_str")]
    pub client_identifier: String,
    #[serde(default, deserialize_with = "de_str")]
    pub provides: String,
    #[serde(default, deserialize_with = "de_bool")]
    pub owned: bool,
    #[serde(rename = "accessToken", default, deserialize_with = "de_str")]
    pub access_token: String,
    #[serde(rename = "sourceTitle", default)]
    pub source_title: Option<String>,
    #[serde(rename = "ownerId", default, deserialize_with = "de_i64")]
    pub owner_id: i64,
    #[serde(default, deserialize_with = "de_bool")]
    pub home: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub presence: bool,
    #[serde(rename = "publicAddressMatches", default, deserialize_with = "de_bool")]
    pub public_address_matches: bool,
    #[serde(rename = "httpsRequired", default, deserialize_with = "de_bool")]
    pub https_required: bool,
    #[serde(default, deserialize_with = "de_vec")]
    pub connections: Vec<Connection>,
}

/// Canon-only: one recorded address of a recorded [`Resource`].
#[derive(serde::Deserialize, Default)]
pub(crate) struct Connection {
    #[serde(default, deserialize_with = "de_str")]
    pub protocol: String,
    #[serde(default, deserialize_with = "de_str")]
    pub address: String,
    #[serde(default, deserialize_with = "de_i64")]
    pub port: i64,
    #[serde(default, deserialize_with = "de_str")]
    pub uri: String,
    #[serde(default, deserialize_with = "de_bool")]
    pub local: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub relay: bool,
    #[serde(rename = "IPv6", default, deserialize_with = "de_bool")]
    pub ipv6: bool,
}


