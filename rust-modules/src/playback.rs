//! Backend-neutral playback vocabulary: the encode-shape contract and its parts, the quality
//! ceiling, the timeline state, and the retained play-queue row — concepts every backend speaks
//! and that `route/` (both halves), the ABR ladder, the Jellyfin device profile, the player and
//! the UI consume. Nothing here should force a caller to name a Plex path for a noun it uses
//! without Plex in sight.
//!
//! Extracted verbatim from `plex/params.rs` and `plex/timeline.rs` (the campaign's
//! vocabulary-move pattern: `origin`, `librecord`, `grant`, `collection`, `person/types`,
//! `registry`). This module holds NOUNS, not protocol: the Plex wire types that wrap them
//! (`TranscodeSpec`, `StreamSelection`, `TimelineReport`) stayed in `plex/params.rs`, and the
//! queue row's PRODUCER (`QueueRow::of`, consuming a Plex `models::Metadata` tree) stayed in
//! `plex/timeline.rs`.

/// The wire/container contract for one universal-transcoder session.
///
/// This is carried beside the request instead of inferred from its eventual URL: the capability
/// profile, decision query, start endpoint and demux strategy are one choice and must never drift
/// independently. Fixed HLS is deliberately a single-rendition session on the measured PMS; an
/// adaptive quality change primes a replacement encoder session rather than pretending that the
/// one-entry master playlist is client-selectable ABR.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TranscodeDelivery {
    /// The established progressive Matroska stream (`start.mkv`).
    #[default]
    ProgressiveMkv,
    /// A fixed rendition delivered as a complete HLS VOD playlist of MPEG-TS segments.
    FixedHls { seconds_per_segment: u8 },
}

/// Exact content boundary for a universal-transcoder start.
///
/// PMS declares this query parameter as a number of seconds, not an integer. Keeping the value in
/// microseconds matches the six-decimal `EXTINF`/`EXT-X-START` contract and prevents an adaptive
/// encoder handoff at 2.002 s from being rounded back into the segment which already played.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TranscodeOffset {
    Fresh,
    AtMicros(i64),
}

impl TranscodeOffset {
    pub fn from_seconds(seconds: i64) -> Self {
        if seconds < 0 {
            Self::Fresh
        } else {
            Self::AtMicros(seconds.saturating_mul(1_000_000))
        }
    }

    pub fn from_micros(micros: i64) -> Self {
        Self::AtMicros(micros.max(0))
    }

    pub(crate) fn wire_seconds(self) -> Option<String> {
        let Self::AtMicros(micros) = self else {
            return None;
        };
        Some(format!("{}.{:06}", micros / 1_000_000, micros % 1_000_000))
    }
}

/// Plex Pass's per-track transcoder DSP (issue #266): dialog boost and loudness normalization.
/// PMS accepts the two independently (`boostDialog=1`, `normalizeLoudness=1` on the transcode
/// query — never on MDE, per M1), so they are two flags rather than one; this client always
/// offers them together with the same gate (`route::plan::desired_audio`), but the wire and the
/// type stay as honest about the server's contract as the server itself is.
///
/// `Copy`, `Default` (= [`NONE`](Self::NONE)), `Serialize`/`Deserialize` — it rides every
/// [`EncodeContract`] built on the resolve worker and is persisted as the user's preference
/// (`plex::session::Session::audio_enhancements`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct AudioEnhancements {
    #[serde(default)]
    pub boost_dialog: bool,
    #[serde(default)]
    pub normalize_loudness: bool,
}

impl AudioEnhancements {
    pub const NONE: AudioEnhancements = AudioEnhancements {
        boost_dialog: false,
        normalize_loudness: false,
    };

    /// Either toggle on — the ONLY question `transcode_query` asks before it emits a param, and
    /// the one `route::plan::enhancements_offered`/`desired_audio` gate before any of this reaches
    /// the wire.
    pub fn any(&self) -> bool {
        self.boost_dialog || self.normalize_loudness
    }

    pub fn is_none(&self) -> bool {
        !self.any()
    }
}

/// The single value that describes the SHAPE of one universal-transcoder encode: flavor, delivery,
/// the re-encode/no-copy switch, a user ceiling, and — issue #266 — which Plex Pass audio DSP the
/// carried track gets. These five used to be four independent fields on
/// [`crate::plex::TranscodeSpec`]; folding them into one value is what keeps "enhancement"
/// honestly scoped to a remux: the audio params only ever mean anything alongside `remux == true`
/// (M1 — MDE and a re-encode both ignore or mishandle them), so a caller that builds a contract
/// states the whole shape in one place instead of four call sites that could each be edited
/// without the others noticing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct EncodeContract {
    /// true = container-only remux (the source codecs are direct-playable, the container
    /// isn't): copy video+audio into progressive MKV, no re-encode, keeps 4K/HDR.
    /// false = full re-encode to the profile's HEVC/AC3 target at up to 4K.
    pub remux: bool,
    /// The coupled profile/query/endpoint/demux contract for this encoder session.
    pub delivery: TranscodeDelivery,
    /// **The server must not COPY the video track — an encoder has to run.** Only meaningful on
    /// the re-encode flavor (`remux == false`), where it swaps `directStream=1` for
    /// `directStream=0` + `directStreamAudio=1`: video re-encoded, audio still copied when it can
    /// be. Off for every ordinary transcode, where a copy is a free win.
    ///
    /// It exists because "we refused direct play" and "the server will re-encode" are NOT the same
    /// statement, and the gap between them shipped a fix that fixed nothing. `directStream=1`
    /// permits a stream copy, and PMS takes it whenever the source fits the caps the query carries
    /// — resolution, bitrate, and the profile's own limitation axes (width/height/bitDepth).
    /// **None of those axes can express Dolby Vision**, so a Profile 5 file refused by
    /// `route::video_direct_plays` came back `Part.decision=transcode` with the VIDEO stream's own
    /// decision `copy`: the identical IPT-PQ bitstream, one container down, the identical wrong
    /// colours (measured against the dev PMS 2026-08-21 — `docs/pms-api.md` §"What the server
    /// actually does with a Dolby Vision source").
    ///
    /// Setting it makes the ask honest, and the answer honest with it: a server that CAN convert
    /// the file does, and a server that cannot says so (this PMS answers
    /// `generalDecisionCode 2000` / *"File is unplayable. DoVi (Profile 5) color space is not
    /// supported."*), which `route::refusal` turns into the player's read-out quoting that
    /// sentence. Both beat a picture in the wrong colours with nothing on any surface to explain
    /// it.
    pub no_video_copy: bool,
    /// The bound this playback's RE-ENCODE may not exceed — the user's pick off the quality
    /// ladder or Auto controller's current rung. `None` = Original/unrestricted and resolves to
    /// the historical [`Ceiling::NATIVE_4K`] query values.
    ///
    /// Read on the re-encode branch **only**, and that is not an oversight: see the type doc
    /// above. By the time a `Some` reaches here, `route::quality_policy` has already refused the
    /// two flavors no number can bind.
    pub ceiling: Option<Ceiling>,
    /// Plex Pass DSP for the carried audio track (issue #266). Set only by `build_stream`, from
    /// `route::desired_audio` — `NONE` unless the enhancement is offered (Plex Pass, a capable
    /// carried track, the Original route, no subtitle, no Dolby Vision) and the viewer opted in.
    pub audio: AudioEnhancements,
}

impl EncodeContract {
    /// The plain-progressive-remux shape with no re-encode and no ceiling: session idle, or a
    /// return to an ordinary (un-enhanced unless `audio` says otherwise) Original after a
    /// recovery. `const fn` so it can seed a `const` value (`PlaybackSession::IDLE`).
    pub const fn original(remux: bool, audio: AudioEnhancements) -> Self {
        EncodeContract {
            remux,
            delivery: TranscodeDelivery::ProgressiveMkv,
            no_video_copy: false,
            ceiling: None,
            audio,
        }
    }
}

/// A bound a playback must come in under, in the units PMS's own params use: `maxVideoBitrate` is
/// **kbps**, `videoResolution` is a `WxH` pair. One value rather than two scattered ints, because
/// the two axes have to move together — a rung that halves the rate and keeps 4K asks the server
/// for something it cannot make look like anything.
///
/// Lives here rather than in `route` so the ladder's rungs and the query that spends them are one
/// type: `route::Quality::ceiling` is the only constructor of a non-default one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ceiling {
    /// `maxVideoBitrate`, in **kbps** (PMS's unit — not bits, not bytes).
    pub max_kbps: i64,
    /// `videoResolution`, as the frame the encode may not exceed.
    pub max_w: i64,
    pub max_h: i64,
}

impl Ceiling {
    /// What the re-encode branch asked for before user ceilings existed: the panel's own native 4K
    /// at a rate high enough to be no bound in practice. This is the `None`/Original substitute;
    /// Auto always supplies an explicit bootstrap or controller rung.
    pub const NATIVE_4K: Ceiling = Ceiling {
        max_kbps: 60000,
        max_w: 3840,
        max_h: 2160,
    };

    /// `videoResolution`'s value for this bound.
    pub fn resolution(&self) -> String {
        format!("{}x{}", self.max_w, self.max_h)
    }

    /// Does a source measured at `kbps` and `w`x`h` come in under this bound?
    ///
    /// **An unmeasured source does NOT** — `0` is "the server did not say", and it fails CLOSED
    /// here while the same `0` PASSES `route::video_direct_plays`'s device-capability gate. The
    /// asymmetry is the point and it is a decision, not an inconsistency: a device bound is a
    /// CAPABILITY, so an unknown degrades to "assume yesterday's behaviour works" (the rule
    /// `devcaps::parse` applies); a user ceiling is an explicit ASK, and the only way to honour an
    /// ask about a file you have not measured is to route it to the branch where the server
    /// applies the bound for you. Failing open here would reproduce, for every play from a shelf
    /// that never loaded a detail page, exactly the do-nothing bug the type doc describes.
    pub fn admits(&self, kbps: i64, w: i64, h: i64) -> bool {
        kbps > 0 && w > 0 && h > 0 && kbps <= self.max_kbps && w <= self.max_w && h <= self.max_h
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TimelineState {
    Playing,
    Paused,
    Stopped,
}

impl TimelineState {
    pub fn as_str(self) -> &'static str {
        match self {
            TimelineState::Playing => "playing",
            TimelineState::Paused => "paused",
            TimelineState::Stopped => "stopped",
        }
    }
}

/// One retained row of the play queue: everything a queue list draws, plus everything
/// [`crate::route::request_play`] needs to START that row — and nothing else.
///
/// The projection is the whole point. A `Metadata` row carries the entire Media/Part/Stream/Role
/// tree, this runs on the resolve worker of a 32-bit TV, and a `continuous=1` queue can be a whole
/// show — so the rows are retained ONLY in this shape. Field names mirror `route::UpNext` (which
/// is built from one of these) rather than the wire: `dur_ms` is `duration`, `resume_ms` is
/// `viewOffset`, `part`/`vcodec`/`acodec` come from `Media[0]`/`Media[0].Part[0]` exactly as the
/// single-successor projection always did.
///
/// NOT episode-gated: a queue row may be a movie, and the list must be able to show it. The
/// "episodes only" rule belongs to the one-item Up Next control, not to the queue.
#[derive(Clone, Default)]
pub struct QueueRow {
    /// `playQueueItemID` — identity WITHIN the queue. A ratingKey can repeat in a queue (the same
    /// item queued twice), a playQueueItemID cannot, so every lookup keys off this. 0 = a server
    /// that omitted the field.
    pub item_id: i64,
    /// The server that ISSUED this queue, and so the server `rk` is a key on. A PlayQueue is
    /// per-server by construction (it is created with a `server://{machineIdentifier}/…` uri), so
    /// every row of one shares this — but the ratingKey FALLBACK below compares against a key
    /// supplied by the caller, and that one can come from anywhere.
    pub sid: crate::registry::ServerId,
    pub rk: String,
    /// `type` — movie | episode | clip …
    pub kind: String,
    /// the item's own title (the episode title for an episode)
    pub title: String,
    /// `grandparentTitle` — the show, empty for a movie
    pub show_title: String,
    /// `parentIndex` — season number (0 for a movie)
    pub season: i64,
    pub index: i64,
    pub thumb: String,
    pub dur_ms: i64,
    /// `viewOffset` — the resume point, 0 = unwatched/from the start
    pub resume_ms: i64,
    /// `Media[0].Part[0].key` — the direct-play part
    pub part: String,
    pub vcodec: String,
    pub acodec: String,
}
