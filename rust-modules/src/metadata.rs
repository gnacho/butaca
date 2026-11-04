//! Item detail data layer for the detail page: full metadata (genres, cast, crew,
//! audio/subtitle streams), and the TV season/episode hierarchy — fetched on demand into a single
//! CURRENT item. Idiomatic Rust (String/Vec), like the browse catalog (catalog.rs) — the fixed C
//! buffers from the C port are gone.
use std::os::raw::c_int;
pub(crate) mod record;
pub(crate) mod sub_layout;
pub(crate) mod track_label;
use std::panic::catch_unwind;

/// **Stage B of the store-ownership migration** (`docs/stores-as-machines.md`, D4): a borrowed
/// handle onto this layer's read surface, shaped like `crate::person::PersonView`. Every method
/// reads straight off the owning `MetadataStore`'s own `MetadataState`/`MetadataAdapter` — there is
/// no process-wide state left for it to forward to. The `'a` lifetime borrows the owner (`new`
/// takes `&'a MetadataStore`), so a `MetadataView` can only be produced from an owner — there is no
/// argument-less constructor, `Default` impl or `'static` substitute. See
/// `crate::stores::metadata::MetadataStore::view(&self) -> MetadataView<'_>`.
#[derive(Clone, Copy)]
pub(crate) struct MetadataView<'a> {
    state: &'a MetadataState,
    adapter: &'a MetadataAdapter,
}

impl<'a> MetadataView<'a> {
    pub(crate) fn new(owner: &'a crate::stores::metadata::MetadataStore) -> Self {
        Self { state: owner.state(), adapter: owner.adapter_ref() }
    }
    pub(crate) fn current(&self) -> Option<&'a Detail> {
        self.state.current.as_ref()
    }
    pub(crate) fn now_playing(&self) -> Option<&'a NowPlaying> {
        self.state.now.as_ref()
    }
    pub(crate) fn playing(&self) -> Option<&'a PlayingItem> {
        self.state.playing.as_ref()
    }
    #[allow(dead_code)]
    pub(crate) fn playing_markers(&self) -> &'a [Marker] {
        self.playing().map(|p| p.markers.as_slice()).unwrap_or(&[])
    }
    #[allow(dead_code)]
    pub(crate) fn playing_chapters(&self) -> &'a [Chapter] {
        self.playing().map(|p| p.chapters.as_slice()).unwrap_or(&[])
    }
    pub(crate) fn detail_loading(&self) -> bool {
        detail_loading(self.adapter)
    }
    pub(crate) fn season_loading(&self) -> bool {
        season_loading(self.adapter)
    }
    pub(crate) fn detail_request_status(&self, sid: crate::registry::ServerId, rk: &str) -> Option<bool> {
        detail_request_status(self.adapter, sid, rk)
    }
    /// See [`detail_generation`] (free fn) for why a bare terminal `bool` is not enough identity.
    pub(crate) fn detail_generation(&self) -> u32 {
        detail_generation(self.adapter)
    }
    pub(crate) fn cached_playing(&self, sid: crate::registry::ServerId, rk: &str) -> Option<PlayingItem> {
        cached_playing(self.state, sid, rk)
    }
    pub(crate) fn active_marker(&self, ps: &crate::route::PlaybackSession) -> Option<Marker> {
        if !crate::player::is_playing(ps) {
            return None;
        }
        let m = marker_at(self.playing_markers(), crate::player::playpos_ns() / 1_000_000)?;
        (!self.state.skipped.contains(&(m.kind, m.start_ms))).then_some(m)
    }
    pub(crate) fn synthesized_tail_marker(&self, ps: &crate::route::PlaybackSession, has_next: bool) -> Option<Marker> {
        if !has_next || !crate::player::is_playing(ps) {
            return None;
        }
        if self.playing_markers().iter().any(|m| m.kind == MarkerKind::Credits) {
            return None;
        }
        let dur_ms = crate::player::duration_ns() / 1_000_000;
        let pos_ms = crate::player::playpos_ns() / 1_000_000;
        tail_marker_with(pos_ms, dur_ms, crate::route::up_next_window_ms())
    }
    pub(crate) fn alt_copies(&self, sid: crate::registry::ServerId, rk: &str) -> &'a [AltCopy] {
        alt_copies(self.state, sid, rk)
    }
    pub(crate) fn alt_available(&self, sid: crate::registry::ServerId, rk: &str) -> bool {
        alt_available(self.state, sid, rk)
    }
}

/// One Metadata owner's main-thread-only logical state (`docs/stores-as-machines.md`). Production
/// gains one through `stores::metadata::MetadataStore`; the worker-touched half is
/// [`MetadataAdapter`].
#[derive(Default)]
pub(crate) struct MetadataState {
    current: Option<Detail>,
    now: Option<NowPlaying>,
    playing: Option<PlayingItem>,
    /// Segments the user has already skipped in THIS playback — see the retired `SKIPPED` static's
    /// doc.
    skipped: Vec<(MarkerKind, i64)>,
    alt: AltStore,
}

/// The `Arc`'d worker half of one Metadata owner: the detail/season/alt-sources landing mailboxes,
/// their generation counters, and (D3) the replay recorder's admission ledger over the detail
/// landing. A worker captures a clone of the owning `Bridge`'s `Arc<MetadataAdapter>` before it
/// spawns; the adapter is never rotated (D3), so an old worker can always land into it.
pub(crate) struct MetadataAdapter {
    detail_gen: std::sync::atomic::AtomicU32,
    detail_done: std::sync::atomic::AtomicU32,
    detail_landing: crate::ui::landing::Landing<DetailKey, Option<Detail>>,
    detail_want: std::sync::Mutex<Option<DetailKey>>,
    alt_gen: std::sync::atomic::AtomicU32,
    alt_roster_gen: std::sync::atomic::AtomicU32,
    alt_facts_gen: std::sync::atomic::AtomicU32,
    alt_slot: std::sync::Mutex<Option<AltResult>>,
    season_gen: std::sync::atomic::AtomicU32,
    season_done: std::sync::atomic::AtomicU32,
    season_result: std::sync::Mutex<Option<SeasonResult>>,
    tracker: std::sync::Mutex<record::Tracker>,
    /// TEST ONLY: the detail fetches [`request_detail`] admitted, parked here instead of on a
    /// worker thread — see [`MetadataAdapter::run_held_detail_fetches_for_test`].
    #[cfg(test)]
    held_detail: std::sync::Mutex<Vec<(crate::registry::ServerId, String, u32)>>,
}

impl Default for MetadataAdapter {
    fn default() -> Self {
        Self {
            detail_gen: std::sync::atomic::AtomicU32::new(0),
            detail_done: std::sync::atomic::AtomicU32::new(0),
            detail_landing: crate::ui::landing::Landing::with_inflight(2, 4),
            detail_want: std::sync::Mutex::new(None),
            alt_gen: std::sync::atomic::AtomicU32::new(0),
            alt_roster_gen: std::sync::atomic::AtomicU32::new(0),
            alt_facts_gen: std::sync::atomic::AtomicU32::new(0),
            alt_slot: std::sync::Mutex::new(None),
            season_gen: std::sync::atomic::AtomicU32::new(0),
            season_done: std::sync::atomic::AtomicU32::new(0),
            season_result: std::sync::Mutex::new(None),
            tracker: std::sync::Mutex::new(record::Tracker::new(false)),
            #[cfg(test)]
            held_detail: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[cfg(test)]
impl MetadataAdapter {
    /// Run every detail fetch [`request_detail`] admitted and has not yet run, on THIS thread,
    /// through the production completion path (`finish_detail_fetch` over `fetch_full`); return
    /// how many ran. Their landings are queued for the next `pump_detail`.
    ///
    /// Under `cfg(test)` a detail request never starts a worker thread. It did until 2026-09-19,
    /// and every test that drove a request through a real `Bridge` raced that thread: a fetch
    /// for an unknown server returns in microseconds, so a fast run settled the request before
    /// an "in flight" assertion (`Some(false)` where `Some(true)` was pinned), while a loaded CI
    /// runner had not settled it after the hundred `yield_now`s a drain helper spun
    /// (`Some(true)` where `Some(false)` was pinned). Both halves failed intermittently in
    /// `app::content::library_publication_tests`. Holding the CURRENT fetch until a test asks
    /// makes "in flight" and "settled" states the test chooses rather than the scheduler.
    pub(crate) fn run_held_detail_fetches_for_test(&self) -> usize {
        self.run_held_detail_fetches(|_| true)
    }

    /// The superseded half only: fetches whose generation is no longer current, whose landings
    /// `pump_detail` discards anyway. [`begin_detail_request`] runs these right after it
    /// supersedes them, BEFORE admitting the new request, so a superseded fetch hands back its
    /// admission reservation — as the thread it replaces did, well inside a test's next step —
    /// instead of holding one of the landing's four in-flight slots until a test drains it.
    fn run_superseded_detail_fetches(&self) -> usize {
        let current = self.detail_gen.load(std::sync::atomic::Ordering::SeqCst);
        self.run_held_detail_fetches(|gen| gen != current)
    }

    fn run_held_detail_fetches(&self, pick: impl Fn(u32) -> bool) -> usize {
        let run: Vec<_> = {
            let mut held = self.held_detail.lock().unwrap_or_else(|e| e.into_inner());
            let (run, keep) = std::mem::take(&mut *held).into_iter().partition(|(_, _, gen)| pick(*gen));
            *held = keep;
            run
        };
        let n = run.len();
        for (sid, rk, gen) in run {
            finish_detail_fetch(self, sid, &rk, gen, || fetch_full(sid, &rk));
        }
        n
    }
}

impl MetadataAdapter {
    fn detail_landing_ref(&self) -> &crate::ui::landing::Landing<DetailKey, Option<Detail>> {
        &self.detail_landing
    }
    fn tracker_mutex(&self) -> &std::sync::Mutex<record::Tracker> {
        &self.tracker
    }
}

/// Where the Detail page was standing — enough to put it back when BACK returns to it, and nothing
/// more. **Moved here from `ui::detail` for restructure phase 7a** (`stores/metadata.rs`'s module
/// doc has the full read/write contract this type is §4 of); `ui::detail` re-exported it so every
/// caller compiled unchanged, and only the OWNERSHIP moved, not the shape. **Its callers today are
/// the container's, not a trail's** — restructure phase 12 (D1) retired `ui::trail::Node::Detail`
/// and `app::nav`'s `leaving_spot`/`Origin`, which were the two that bundled a `Spot` with a page
/// identity. A `Spot` now rides `screens::registry::PageMemory::Detail(DetailMemory { spot, .. })`
/// on the entry's own `ReturnState`, which is the split `stores/metadata.rs`'s module doc argues
/// for: `(sid, rk)` is identity and the `Spot` is position. `screens::detail`'s `spot()` is the
/// other reader.
///
/// A `Spot` names FOCUS, not pixels: the vertical scroll is derived from the focused section
/// (`scroll_target`) and both h-scrolls from `col`, so recording them too would be two sources for
/// one fact — and the second would be wrong the moment the item came back with a different-length
/// list. `Default` is a page that has been entered and not left yet: hero, item 0.
///
/// `season` is the season NUMBER, not `cur_season`'s POSITION: a `/children` refetch can reorder or
/// re-title the list, and the number is what the user actually saw on the tab. `None` for a movie —
/// and that `None` is load-bearing, because it is what stops a movie's restore waiting for a season
/// list that will never arrive (see `ui::detail::spot_season_gate`).
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct Spot {
    /// section id (0 hero, 1 tabs, 2 episodes, 3 related, 4 cast, 5 about, 6 extras).
    /// Indexed by identity, not visual position — see [`SPOT_SECTION_SLOTS`].
    pub(crate) section: c_int,
    /// focused item within that section
    pub(crate) col: c_int,
    /// the episode filmstrip's sub-row (still vs. its metadata block) — a bool because the enum
    /// naming the two (`ui::detail::EpRow`) is private to that module and this type has no
    /// business naming it
    pub(crate) ep_text: bool,
    /// the per-section focus memory, so LEFT/RIGHT in a row the user never returned to still comes
    /// back where they left it. Indexed by section id. A new id without a slot here is a compile
    /// failure at the array length, not a silent drop.
    pub(crate) saved_col: [c_int; SPOT_SECTION_SLOTS],
    /// the selected season's NUMBER, or `None` for an item with no seasons
    pub(crate) season: Option<i64>,
}

/// Plex's resume rule, in ONE place (home Continue-Watching, the detail Play button, and the
/// butaca-play harness all apply it): resume only past 10s and before 95% watched, else start
/// from the beginning. Both args are MILLISECONDS; the returned position is NANOSECONDS
/// (what `player::resume_at` takes).
pub(crate) fn resume_ns(resume_ms: i64, dur_ms: i64) -> i64 {
    if resume_ms > 10_000 && (dur_ms <= 0 || (resume_ms as f64) < 0.95 * dur_ms as f64) {
        resume_ms * 1_000_000
    } else {
        0
    }
}

/// Friendly display name for an audio/subtitle codec id — the ONE codec→name map (the track
/// menu's section accessory and the Info card's track line both read it, so the same track
/// can't be named two ways).
pub(crate) fn friendly_codec(codec: &str) -> String {
    match codec.to_lowercase().as_str() {
        "truehd" => "Dolby TrueHD".to_string(),
        "eac3" | "ec-3" => "Dolby Digital Plus".to_string(),
        "ac3" => "Dolby Digital".to_string(),
        "dts" | "dca" => "DTS".to_string(),
        "aac" => "AAC".to_string(),
        "flac" => "FLAC".to_string(),
        "opus" => "Opus".to_string(),
        "mp3" => "MP3".to_string(),
        other if other.is_empty() => String::new(),
        other => other.to_uppercase(),
    }
}

/// One credit on the Cast & Crew shelf. The server ships crew (`Director[]`/`Writer[]`) in the SAME
/// shape as the actors (`Role[]`) minus the `role` attribute, so a crew credit is this same struct
/// with its JOB in `role` — see [`crew_credits`].
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Cast {
    pub(crate) tag: String,   // person's name
    pub(crate) role: String,  // character (an actor) — or the job, "Director"/"Writer" (crew)
    pub(crate) thumb: String, // headshot (often an external metadata-static.plex.tv URL)
    /// The person's numeric library id (`Role[].id`), 0 when absent.
    pub(crate) id: i64,
    /// The person's global Plex guid (`Role[].tagKey`) — the id's stand-in when the server
    /// omits the numeric one.
    pub(crate) tag_key: String,
}

/// Stable identities for the app-owned jobs derived from the server crew arrays. The keys stay in
/// recorded metadata and semantic comparisons; only a display accessor translates them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CrewRole {
    Director,
    Writer,
    DirectorWriter,
}
impl CrewRole {

    fn from_key(key: &str) -> Option<Self> {
        match key {
            "Director" => Some(Self::Director),
            "Writer" => Some(Self::Writer),
            "Director, Writer" => Some(Self::DirectorWriter),
            _ => None,
        }
    }

    fn display(self, locale: &crate::i18n::LocaleContext) -> &'static str {
        match self {
            Self::Director => crate::i18n::msg::browse_crew_director_in(locale),
            Self::Writer => crate::i18n::msg::browse_crew_writer_in(locale),
            Self::DirectorWriter => crate::i18n::msg::browse_crew_director_writer_in(locale),
        }
    }
}

impl Cast {
    /// The `personId` for `/library/people/{personId}/media` — the numeric id when the server
    /// sent one, else the global guid (the server accepts EITHER; both verified live 2026-07-29).
    /// Empty when the row carries neither, which is the "this headshot opens nothing" case the
    /// cast row's OK arm gates on.
    pub(crate) fn person_key(&self) -> String {
        if self.id > 0 {
            self.id.to_string()
        } else {
            self.tag_key.clone()
        }
    }
}

/// What the server says about a video stream's **Dolby Vision** layering — read together, because
/// the only question worth asking of them is a joint one: **is the base layer, alone, a correct
/// picture?**
///
/// It has to be a joint question because the buffer-feed pipeline has no other option. We feed one
/// elementary stream to a decoder that has never heard of an RPU, so whatever the base layer
/// contains is exactly what reaches the panel. That is fine for **Profile 8.1**, whose base layer
/// IS an HDR10 stream — ignoring the RPU costs the dynamic metadata and nothing else. It is not
/// fine for **Profile 5**, which is single-layer IPT-PQ with no HDR10 fallback: decoded as
/// ordinary HEVC it is not a dimmer picture but a WRONG one, with the washed, pink-green cast of
/// IPT read as YCbCr. And it cannot work at all for **Profile 7**, whose picture is split across
/// two layers we cannot interleave.
///
/// All zero is "the server said nothing", which is also what a non-DV stream produces — see
/// [`Dovi::base_layer_unusable`] for why silence must never convict.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Dovi {
    /// `DOVIPresent` — the file carries Dolby Vision at all. Everything below is meaningless
    /// without it, because a non-DV stream sends none of these fields and so reads as all-zero.
    pub(crate) present: bool,
    /// `DOVIProfile` — 5 / 7 / 8 …; 0 means the server did not say.
    pub(crate) profile: i64,
    /// `DOVIBLCompatID` — 0 none (P5) / 1 HDR10 / 2 SDR / 4 HLG. NB the dev server's P7 item
    /// sends **6**, so this field alone does not identify a dual-layer file.
    pub(crate) bl_compat: i64,
    /// `DOVIELPresent` — an enhancement layer is present (P7).
    pub(crate) el_present: bool,
    // ---- DESCRIPTIVE ONLY, below. The four fields above answer the PLAYBACK question this struct
    // exists for ("is the base layer a correct picture"); these three are read by
    // `screens::tracks_panel` and by nothing else. They live here rather than on `Detail` so there is
    // one home for "what the server said about Dolby Vision" — but do not reach for them in a
    // decision without the live sweep `server::Stream`'s DOVI comment records.
    /// `DOVILevel` — the DV level (a bitrate/resolution tier); 0 = the server did not say.
    pub(crate) level: i64,
    /// `DOVIVersion`, DECOMPOSED — the server sends the dotted string `"1.0"`, and this holds it
    /// as `(1, 0)`. Rendered back by [`Dovi::version_str`].
    ///
    /// **Why not a `String`:** this struct is `Copy` and rides `route::Session`, whose `IDLE` is a
    /// `const`; more to the point `route::playback_preview_of` reads a `Dovi` **on the detail
    /// page's per-frame draw path**, so a `String` field would put a heap allocation in a hot
    /// frame — the same cost `ui::info_panel` refuses when it declines to clone `route::url()`.
    /// A DV version is dotted-numeric by specification, so the decomposition is lossless for every
    /// shape the field can take; anything unparseable lands as `(0, 0)` and draws no row at all.
    pub(crate) version: (i64, i64),
    /// `DOVIBLPresent` — the base layer is in the file.
    pub(crate) bl_present: bool,
    /// `DOVIRPUPresent` — the RPU (the dynamic metadata) is in the file.
    pub(crate) rpu_present: bool,
}

impl Dovi {
    /// The all-zero record — "the server said nothing about Dolby Vision", which is also exactly
    /// what an ordinary SDR file produces. A `const` rather than [`Default::default`] because
    /// `route`'s idle `Session` is a `const` item and cannot call one.
    pub(crate) const NONE: Dovi = Dovi {
        present: false,
        profile: 0,
        bl_compat: 0,
        el_present: false,
        level: 0,
        version: (0, 0),
        bl_present: false,
        rpu_present: false,
    };

    /// [`Self::version`] back as the dotted string the server sent (`"1.0"`), or `None` when it
    /// sent none. `None` and `Some("0.0")` are different answers and the panel draws only the
    /// former's absence — a server that really reported version 0 would still get a row.
    pub(crate) fn version_str(&self) -> Option<String> {
        (self.version != (0, 0)).then(|| format!("{}.{}", self.version.0, self.version.1))
    }

    /// PURE: **true when the base layer on its own is not a picture we can put on the panel
    /// correctly** — i.e. when this bitstream, decoded as ordinary HEVC by a pipeline that was
    /// never TOLD it is Dolby Vision, shows the user wrong colours (or nothing).
    ///
    /// Note the "never told" clause: it is the whole question this predicate asks, and since
    /// 2026-08-21 it is no longer the only thing we can do — [`Dovi::presentation`] can declare
    /// the stream to the pipeline, and a declared Profile 5 is displayed correctly. What survives
    /// unchanged is every path where the bitstream reaches a decoder with NO declaration attached,
    /// and that is now this predicate's job:
    ///
    /// - the direct-play gate **when no node will be sent** (`presentation` is written in terms of
    ///   this, so the two can never contradict each other), and
    /// - the server's permission to **COPY** the video — a remux or a `directStream` transcode
    ///   hands us the identical elementary stream one container down, and the Load payload built
    ///   for that path declares nothing. `route::build_stream` reads it for both
    ///   ([`crate::playback::EncodeContract::no_video_copy`] and the `remux` gate) and that is why a
    ///   declared Profile 5 still refuses a copy: the declaration rides the DIRECT PLAY, not the
    ///   file, so the same pixels arriving by another route are as wrong as they ever were.
    ///
    /// Two disqualifiers, and they are found by different fields:
    /// - **an enhancement layer** (`el_present`, Profile 7) — one elementary stream is all the
    ///   pipeline feeds, so the other layer simply never arrives. This is the only test that
    ///   catches P7: measured live 2026-08-21, the dev server's P7 item reports `bl_compat = 6`,
    ///   which sails through any `== 0` check.
    /// - **no cross-compatible base layer** (Profile 5 / `bl_compat == 0`) — IPT-PQ, which an
    ///   HEVC decoder will happily decode and a panel will happily show, incorrectly.
    ///
    /// **Silence must not convict, and that is the whole subtlety here.** Every field is 0 both
    /// when the server omits it and when there is no Dolby Vision at all, so a bare
    /// `bl_compat == 0` would refuse direct play for *every ordinary SDR file in the library*.
    /// `present` guards the outer question and a KNOWN `profile` guards the compat-id test, so a
    /// server that reports `DOVIPresent` and nothing else falls through to the existing gates
    /// unchanged. That direction is deliberate, and the price of getting it wrong is higher than
    /// it first looks: a true answer here does not merely reroute an item, it also withdraws the
    /// server's permission to COPY the video
    /// ([`crate::playback::EncodeContract::no_video_copy`] — without which the refusal accomplishes
    /// nothing at all), and a server that cannot encode the result then refuses the playback
    /// outright. So a false positive costs the film, not just its 4K and its HDR10. The same
    /// misread-degrades-to-assumed rule [`crate::route::video_direct_plays`] applies to an unknown
    /// frame size, applied to a field whose silence is indistinguishable from a legitimate zero.
    ///
    /// The one measured reassurance, and it is worth having before trusting the `profile > 0`
    /// guard: every Dolby Vision stream on the dev server (34 of them, swept 2026-08-21) sends all
    /// eight `DOVI*` keys together. No shape there reports a profile without also reporting a
    /// compatibility id, which is the only combination that guard could misread.
    pub(crate) fn base_layer_unusable(&self) -> bool {
        if !self.present {
            return false;
        }
        self.el_present || self.profile == 5 || (self.profile > 0 && self.bl_compat == 0)
    }

    /// PURE: **how this stream will be presented** — the ONE predicate behind both halves of the
    /// Dolby Vision decision. The direct-play gate and the eventual Load payload receive this
    /// same value, frozen into route state rather than re-reading capability at different times.
    ///
    /// `signal` retains `nodv`'s diagnostic asymmetry: it withholds a Profile-5-style declaration,
    /// but does not suppress a compatible Profile 8 on a supported set. `capability` must be a
    /// definite [`Supported`](crate::webos::caps::DvCapability::Supported), and
    /// `video_is_hevc` closes the old Profile 9 disagreement where the gate declared AVC and the
    /// payload's H265 guard silently discarded the node.
    ///
    /// The four arms, and why each is where it is:
    ///
    /// - **not `present`** → [`DvPresentation::NotDv`]. No Dolby Vision, nothing to say, and
    ///   nothing to refuse. This is every ordinary file in the library.
    /// - **`el_present`** → refuse, always, `signal` or not. Profile 7 splits its picture across a
    ///   base and an enhancement layer; the pipeline feeds ONE elementary stream and cannot
    ///   interleave the other, so no payload key makes it displayable. (This is also the only
    ///   thing that identifies a dual-layer file: the dev server's P7 reports `bl_compat = 6`.)
    ///   It is deliberately checked BEFORE the declaration arm — which is what keeps the emitted
    ///   node's `trackType` at `"single"` and, with `encryptionType` fixed at `"clear"`, makes the
    ///   pipeline's `dv-dual-svp` secure-video-path flag unreachable. We cannot satisfy that flag.
    /// - **no node will be sent** (capability absent/unknown, non-HEVC output, a profile the
    ///   server never named, or Profile 5 with the trigger armed) → fall back to
    ///   exactly the pre-declaration rule: refuse iff [`base_layer_unusable`](Self::base_layer_unusable).
    ///   That is what makes "keep the refusal for any case where we would not send the node" a
    ///   property of the code rather than of a reviewer's memory. The `profile <= 0` half also
    ///   preserves the never-convict-on-silence rule: a server that reports `DOVIPresent` and
    ///   nothing else still falls through to `NotDv` and plays as it always has, because we cannot
    ///   tell that silence from an SDR file, and `getInt` wants a real profile id anyway.
    /// - otherwise → **declare it**, and direct play is then correct — including for Profile 5,
    ///   whose refusal this inverts. The decompile of this TV's own `libpf` (2026-08-21) is the
    ///   evidence: `CustomPipeline::parseOptionStringSpi` sets `hasDolbyHdrInfo` on the mere
    ///   PRESENCE of the key, and `getVideoCaps` then adds `dolby-vision=TRUE` (+ the profile
    ///   hint) to the `video/x-h265` caps it was already going to build. The codec string does not
    ///   change; the node is the entire difference between an IPT-PQ stream shown in wrong colours
    ///   and one the panel puts in Dolby Vision mode.
    pub(crate) fn presentation(
        &self,
        signal: bool,
        capability: crate::webos::caps::DvCapability,
        video_is_hevc: bool,
    ) -> DvPresentation {
        if !self.present {
            return DvPresentation::NotDv;
        }
        if self.el_present {
            return DvPresentation::Refuse("dual-layer");
        }
        // Presence of our node enables libpf's DV path even on a television which cannot display
        // it. libplayerAPIs' own platform metadata does not protect that seam, so only this app's
        // affirmative configd result may make the declaration eligible.
        let declare = capability == crate::webos::caps::DvCapability::Supported
            && video_is_hevc
            && (signal || !self.base_layer_unusable());
        if !declare || self.profile <= 0 {
            return if self.base_layer_unusable() {
                DvPresentation::Refuse("no cross-compatible base layer")
            } else {
                DvPresentation::NotDv
            };
        }
        DvPresentation::Declare(DolbyHdrInfo {
            profile_id: self.profile,
            // Honest derivation, and unreachable as `"dual"` while the `el_present` arm above
            // returns first — written this way so that the day an interleaver exists, the payload
            // follows the refusal being relaxed instead of quietly lying about the track.
            track_type: if self.el_present { "dual" } else { "single" },
            // Never `"all"`: paired with `trackType: "dual"` that is what sets `dv-dual-svp`, the
            // secure-video-path flag, which this app cannot satisfy.
            encryption_type: "clear",
        })
    }

    /// Resolve a fresh decision from the cached platform answer and the boot-latched diagnostic
    /// signal. Call once at a route boundary; installed playback must retain the returned value.
    pub(crate) fn presentation_now(&self, video_is_hevc: bool) -> DvPresentation {
        self.presentation(
            !dv_withheld(),
            crate::webos::caps::capability(),
            video_is_hevc,
        )
    }

}

/// The `option.externalStreamingInfo.contents.DolbyHdrInfo` node of the Starfish Load payload:
/// what we tell LG's pipeline about this stream's Dolby Vision.
///
/// The three fields are the ones the TV's own parser reads, at the paths and in the types the
/// decompile proved (`Options::checkKeyExistance` for the node itself, then `getInt` for
/// `profileId` and `getString` for the other two). `profileId` **must** be a JSON integer;
/// omitting it leaves the pipeline's `-1` sentinel, which still yields `dolby-vision=TRUE` with
/// only the profile hint missing — a legitimate fallback, not a failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DolbyHdrInfo {
    /// `DOVIProfile` as the server reported it. 5 (single-layer IPT-PQ) is the case this exists
    /// for; 8.x declares fine too and gains the dynamic metadata its base layer alone lacks.
    pub(crate) profile_id: i64,
    /// `"single"` or `"dual"` — one elementary stream or a base + enhancement pair.
    pub(crate) track_type: &'static str,
    /// `"clear"`. See [`Dovi::presentation`] for why this is never `"all"`.
    pub(crate) encryption_type: &'static str,
}

/// What [`Dovi::presentation`] decided: the single value the direct-play gate and the Load payload
/// both read. Three states rather than a bool, because "there is no Dolby Vision here" and "there
/// is, and we are declaring it" are the same answer to the GATE and opposite answers to the
/// PAYLOAD — which is precisely the pair that used to be two predicates and could disagree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DvPresentation {
    /// Not Dolby Vision, or not identifiably so. Play it as ordinary HEVC, declare nothing.
    NotDv,
    /// Direct play, with this node spliced into the Load payload.
    Declare(DolbyHdrInfo),
    /// Direct play is refused, with the short reason for the log line at the decision.
    ///
    /// "cross-compatible" rather than "HDR10" deliberately: HDR10 (`bl_compat` 1) is merely the
    /// common case, and an SDR (2) or HLG (4) base layer is equally displayable. What Profile 5
    /// lacks is a base layer conformant to ANY ordinary transfer, which is what id 0 means.
    Refuse(&'static str),
}

/// The platform answer and the presentation derived from it at one route boundary. Both are
/// copyable so reload, recovery and rollback preserve the installed decision exactly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DvDecision {
    pub(crate) capability: crate::webos::caps::DvCapability,
    pub(crate) presentation: DvPresentation,
}

impl DvDecision {
    pub(crate) const NONE: Self = Self {
        capability: crate::webos::caps::DvCapability::Unknown,
        presentation: DvPresentation::NotDv,
    };
}

impl Default for DvDecision {
    fn default() -> Self {
        Self::NONE
    }
}

impl DvPresentation {
    pub(crate) const fn label(&self) -> &'static str {
        match self {
            Self::NotDv => "base-layer",
            Self::Declare(_) => "declare",
            Self::Refuse(_) => "refuse",
        }
    }

    /// [`Self::label`] in the UI language, for the diagnostics read-out. Logs keep `label`.
    pub(crate) fn display(&self) -> &'static str {
        match self {
            Self::NotDv => crate::i18n::msg::browse_diagnostics_dv_base_layer(),
            Self::Declare(_) => crate::i18n::msg::browse_diagnostics_dv_declare(),
            Self::Refuse(_) => crate::i18n::msg::browse_diagnostics_dv_refuse(),
        }
    }

    /// Direct play is refused (and, at `build_stream`, the reason for the log line).
    pub(crate) fn refusal(&self) -> Option<&'static str> {
        match self {
            Self::Refuse(why) => Some(why),
            _ => None,
        }
    }
    /// The node to splice into the Load payload, if any.
    pub(crate) fn declared(&self) -> Option<DolbyHdrInfo> {
        match self {
            Self::Declare(d) => Some(*d),
            _ => None,
        }
    }
}

crate::dev::latched_flag!(
    /// `/tmp/butaca-dvnonode` — after a supported route has frozen `Declare`, keep its Dolby
    /// Vision **direct play** but send **no** `DolbyHdrInfo` node. Diagnostic only: it is the
    /// explicitly logged exception to gate/payload agreement.
    ///
    /// The two things that changed together the day Profile 5 first direct-played are the
    /// DECLARATION and the 4K HEVC direct play of a file that had never been fed before. Every
    /// other combination is reachable by choosing a title — a Profile 8 direct-plays with the node
    /// or without it, because its gate does not depend on the trigger — but the P5 file has only
    /// two states, "declared and direct-played" and "refused", since [`Dovi::presentation`]
    /// answers both halves from one value. This is the missing cell: the same bytes, the same
    /// path, the declaration alone removed.
    ///
    /// Applied at the payload ([`crate::player::engine`]), NEVER at the gate — suppressing the
    /// node at the gate would send the file to the transcoder and measure a different pipeline.
    /// Note the resulting picture is expected to be WRONG (an IPT-PQ stream shown as ordinary
    /// HDR); this knob is for judging cadence, not colour.
    pub(crate) fn dv_node_suppressed = "dvnonode";
);

crate::dev::latched_flag!(
    /// `/tmp/butaca-nodv` — **withhold the Dolby Vision declaration**, for a bisect. The
    /// polarity is inverted from what it was, and the inversion is the point.
    ///
    /// This was `/tmp/butaca-dv`, an opt-IN, default off, with a note in this doc saying to
    /// flip the default "once the node has been seen to put a correct picture on a real panel".
    /// The reason for the caution was real — the payload is the `sourceInfo` envelope, which the
    /// pipeline parses before anything decodes, and a malformed one does not fail loudly, it
    /// wedges the video sink. The condition has now been met, twice over and on the last profile
    /// that had not met it:
    ///
    /// - **the picture is correct** — Profile 5 direct-played, photographed, with the set's own
    ///   "Dolby Vision / Dolby Atmos" read-out on screen;
    /// - **and its one measured defect is fixed.** A declared P5 used to lose the display-
    ///   management lookup for 2 frames in 12 (a ~2 Hz tone pulse); that was one 90 kHz tick in
    ///   the LUT key and is gone — 3 misses in 90 s on the shipped default, against 160 in 45 s.
    ///   See `player::engine::pts_nudge_ns`.
    ///
    /// So every eligible Dolby Vision stream on a set with confirmed support is declared by
    /// default, and this knob only takes it away. Note what it does NOT do: withholding re-imposes
    /// the old refusal on Profile 5 (`base_layer_unusable`), so this bisects "declared vs
    /// transcoded", not "declared vs direct-played-undeclared". [`dv_node_suppressed`]
    /// (`/tmp/butaca-dvnonode`) is the finer instrument for that, and is why both exist.
    ///
    /// Latched once per process so route decisions never observe a changing trigger. The stronger
    /// gate/payload guarantee now comes from storing [`DvDecision`] on the route: neither a later
    /// capability answer nor a reload re-evaluates the installed play.
    pub(crate) fn dv_withheld = "nodv";
);

/// Prewarm both payload diagnostics before the first frame scope. Their getters are subsequently
/// filesystem-free even when preview or payload construction first reaches them during a frame.
pub(crate) fn prewarm_dv_latches() {
    let _ = dv_withheld();
    let _ = dv_node_suppressed();
}

// `Default` is for TESTS: every field is a zero/empty that means "the server did not say", so
// a fixture can name the two or three fields its case is about instead of the fifteen it is not.
#[derive(Clone, Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Stream {
    pub(crate) id: i64,      // Plex stream id (for &audioStreamID / &subtitleStreamID)
    pub(crate) index: i64,   // PMS stream index (container order) — the ordinal mapping sorts by it
    pub(crate) lang: String, // display name ("English")
    pub(crate) lang_code: String, // ISO code ("eng") — the route's language preference matches this
    /// The server `Stream.languageTag` — the BCP-47 tag ("es-419", "en-GB"), region and all, where
    /// `lang_code` is only the ISO-639 primary subtag. `metadata::track_label`'s region fallback
    /// is the one reader (a nameless track with a regional tag names the region instead of
    /// nothing). This struct is a recorded wire value: controlled replay carries whole `Detail`s
    /// (`metadata::record`), and `record::validate` refuses any reply that does not re-serialize
    /// byte-for-byte. So an absent tag must stay absent both ways — `default` reads a recording
    /// made before the field existed, `skip_serializing_if` keeps an empty tag off the wire so
    /// that recording (fixture 12, whose mock server sends no `languageTag`) still round-trips.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) language_tag: String,
    pub(crate) codec: String,
    pub(crate) channels: i64,
    pub(crate) layout: String, // audioChannelLayout, e.g. "5.1(side)"
    /// Per-STREAM bitrate in kbps (0 = the server did not say) — NOT the file's. It is what tells
    /// seven same-language AC3 tracks apart in `screens::tracks_panel`, where language and codec alone
    /// cannot.
    pub(crate) bitrate: i64,
    /// The codec profile, lower-case as the server sends it. **On an audio track this is where
    /// Atmos is** — `"dolby digital plus + dolby atmos"` (probed live 2026-08-21); on the video
    /// track it is `"main 10"`. See [`Stream::has_atmos`].
    pub(crate) profile: String,
    /// Video track only: bits per component (10 for Main 10); 0 = not said.
    pub(crate) bit_depth: i64,
    /// Video track only: chroma subsampling as the server spells it, e.g. `"4:2:0"`.
    pub(crate) chroma: String,
    pub(crate) title: String,
    pub(crate) sdh: bool,
    pub(crate) ad: bool,
    pub(crate) forced: bool,
    pub(crate) default: bool, // the file's default track (drives the "Original:" audio label)
    /// external/sidecar stream (downloaded .srt etc. — NOT inside the container). The DEMUXER
    /// cannot reach it; on direct play a TEXT sidecar is fetched and drawn by `player::sidecar`
    /// instead ([`Stream::sidecar_renderable`]), and a transcode burns it.
    pub(crate) external: bool,
    /// The server `Stream.key` — the sidecar's delivery path (`/library/streams/{id}`). Empty for
    /// every embedded stream, which is exactly how `external` is derived.
    pub(crate) key: String,
    /// The server `Stream.selected` — the server's CURRENT pick for this part, i.e. the track
    /// a user chose on ANY Plex client (phone, web, another TV) and the one `select_streams`
    /// writes. `route`'s selection ladder prefers it over its own defaults, which is what makes
    /// a pick made elsewhere survive here instead of being silently overwritten. NB for AUDIO
    /// the server marks a selected stream on essentially every part — for an untouched one that is
    /// just the container `default` echoed back — so `route::pick_dp_audio` only treats it as
    /// a choice when it names a DIFFERENT stream, and never as a reason to transcode. Read its doc
    /// before using this flag anywhere else.
    pub(crate) selected: bool,
    /// Audio track only: a 1.43.4+ server with Plex Pass can honor
    /// `boostDialog=1`/`normalizeLoudness=1` for this track on the transcoder (issue #266). Same
    /// round-trip concern as `language_tag`: a `Detail` recorded before this field existed carries
    /// no key for it, so `skip_serializing_if` keeps a false value off the wire and `default`
    /// reads the recording back as false — never a confident "capable" for a fixture that never
    /// said so.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) can_normalize_loudness: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Stream {
    /// Can the CLIENT draw this sidecar on direct play? Only a TEXT format: `player::sidecar`
    /// preserves ASS/SSA scripts for native styled rendering and asks the server for other formats
    /// as UTF-8 SubRip (with SubRip/WebVTT parsing fallbacks). An image sidecar
    /// (`.idx`/`.sub` VobSub, `.sup` PGS) has no such path — only a server burn can show it, so
    /// it stays a transcode-only row. An allow-list rather than "not an image codec", so a
    /// format nobody here has met is hidden instead of offered and then silently blank.
    pub(crate) fn sidecar_renderable(&self) -> bool {
        self.external
            && !self.key.is_empty()
            && matches!(
                self.codec.to_ascii_lowercase().as_str(),
                "srt" | "subrip" | "ass" | "ssa" | "vtt" | "webvtt" | "smi" | "sami"
            )
    }

    /// Does this audio track carry **Dolby Atmos**?
    ///
    /// **The answer is in `profile`, and only there** — probed live against the dev server
    /// 2026-08-21. The Atmos track on the P5 test item sends `profile: "dolby digital plus + dolby
    /// atmos"` while its `audioChannelLayout` is the ordinary `"5.1(side)"` and its `title` is
    /// `null`. A client that looked at the layout, the title or the channel count would badge
    /// nothing, forever and silently. (the server *also* composes it into `displayTitle`, but
    /// that is a pre-formatted user string in the server's own words; the profile is the structured
    /// field.) Dolby's own AC-4 spec §3.1.1.1 says the same thing from the other end — *"It is not
    /// possible to derive whether content is branded as Dolby Atmos by inspecting the channel
    /// configuration."*
    ///
    /// Two consumers, and they are why this lives here rather than in the panel that first needed
    /// it: the track menu's `EAC3 5.1 + Atmos` detail line, and the Load payload's
    /// `contents.immersive` node ([`crate::route::stream_immersive`]) — one is a caption and the
    /// other is a statement to the television's pipeline, so the predicate has to be the data
    /// layer's, not a screen's.
    ///
    /// Deliberately a substring test rather than an equality: the field is a human-readable
    /// composition and the codec half of it varies (`"dolby digital plus + dolby atmos"` here,
    /// but TrueHD and AC-4 compose the same way). "atmos" is the part that means Atmos.
    pub(crate) fn has_atmos(&self) -> bool {
        self.profile.to_ascii_lowercase().contains("atmos")
    }
}

#[derive(Default, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Episode {
    pub(crate) rk: String,
    pub(crate) index: i64,  // episode number
    pub(crate) season: i64, // parentIndex
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) aired: String, // originallyAvailableAt
    pub(crate) dur_ms: i64,
    pub(crate) thumb: String,
    pub(crate) resume_ms: i64, // viewOffset (0 = not started)
    /// `viewCount ≥ 1` — played through at least once. Deliberately INDEPENDENT of `resume_ms` on
    /// the wire: the server keeps both on an episode that was finished and then started again, so
    /// which of the two a tile shows is a presentation rule at the draw site (see `ui/detail.rs`'s
    /// filmstrip), not a mutual exclusion the data layer can assume.
    pub(crate) watched: bool,
    pub(crate) part: String, // Media[0].Part[0].key (to play)
    pub(crate) rating: String,
    pub(crate) vcodec: String, // Media[0].videoCodec (for the direct-play/transcode decision)
    pub(crate) acodec: String, // Media[0].audioCodec
}

/// One slot per [`Spot`] section id. Section 6 is extras. Do not shrink this without a migration
/// of remembered columns.
pub(crate) const SPOT_SECTION_SLOTS: usize = 7;

/// One extra row. Play fields match [`Episode`]. A row with an empty part is still a shelf tile;
/// OK refuses it.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Extra {
    pub(crate) rk: String,
    pub(crate) title: String,
    pub(crate) subtype: String,
    pub(crate) extra_type: i64,
    pub(crate) part: String,
    pub(crate) vcodec: String,
    pub(crate) acodec: String,
    pub(crate) dur_ms: i64,
    /// `Media[0].bitrate` in kbps. The quality ceiling judges this file, not the parent.
    #[serde(default)]
    pub(crate) bitrate: i64,
    /// Still for the extras shelf. Empty draws the card placeholder, not a broken image.
    #[serde(default)]
    pub(crate) thumb: String,
}

impl Extra {
    pub(crate) fn playable(&self) -> bool {
        !self.rk.is_empty() && !self.part.is_empty()
    }

    pub(crate) fn is_trailer(&self) -> bool {
        self.subtype == "trailer" || self.extra_type == 1
    }

    /// Human subtype for the extras shelf caption, in the UI language. Unknown subtypes read as
    /// the generic extra. The match is on the server's own subtype names, which are never drawn.
    pub(crate) fn caption(&self) -> &'static str {
        use crate::i18n::msg;
        match self.subtype.as_str() {
            "trailer" => msg::browse_detail_trailer(),
            "behindTheScenes" => msg::browse_extra_behind_the_scenes(),
            "featurette" => msg::browse_extra_featurette(),
            "sceneOrSample" => msg::browse_extra_scene(),
            "deletedScene" => msg::browse_extra_deleted_scene(),
            "interview" => msg::browse_extra_interview(),
            _ => match self.extra_type {
                1 => msg::browse_detail_trailer(),
                5 => msg::browse_extra_behind_the_scenes(),
                6 => msg::browse_extra_scene(),
                _ => ExtraContext::Extra.label(),
            },
        }
    }

    /// HUD / PlayIntent title: the extra's own name, or the parent item's if the server sent none.
    pub(crate) fn hud_title<'a>(&'a self, parent: &'a str) -> &'a str {
        if self.title.is_empty() {
            parent
        } else {
            &self.title
        }
    }
}

/// **What an extra's playback request carries as its context**, typed so the comparison and the
/// words cannot be the same string. [`ExtraContext::key`] is the stable value a request carries
/// and [`crate::route::request_play`] compares — it omits `continuous` for both kinds, so EOS
/// cannot Up-Next into a sibling extra — and it is never drawn; [`ExtraContext::label`] is what the
/// HUD prints in its place ([`context_label`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExtraContext {
    Trailer,
    /// Any non-trailer extra.
    Extra,
}

impl ExtraContext {
    /// The comparison key. Deliberately not a word, so no display text can be mistaken for it.
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Trailer => "butaca:context/trailer",
            Self::Extra => "butaca:context/extra",
        }
    }

    pub(crate) fn of(ctx: &str) -> Option<Self> {
        [Self::Trailer, Self::Extra].into_iter().find(|kind| kind.key() == ctx)
    }

    /// The HUD's word for this kind, in the UI language.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Trailer => crate::i18n::msg::browse_detail_trailer(),
            Self::Extra => crate::i18n::msg::browse_extra_extra(),
        }
    }
}

/// The trailer's request context: see [`ExtraContext::key`].
pub(crate) const TRAILER_CONTEXT: &str = ExtraContext::Trailer.key();
/// Non-trailer extras. Same queue rule as a trailer: omit `continuous` so EOS cannot Up-Next.
pub(crate) const EXTRA_CONTEXT: &str = ExtraContext::Extra.key();

pub(crate) fn context_omits_queue_continuous(ctx: &str) -> bool {
    ExtraContext::of(ctx).is_some()
}

/// The HUD context line for a request context: an extra's kind in the UI language, and any other
/// context unchanged — a feature's context line is already display text.
pub(crate) fn context_label(ctx: &str) -> &str {
    match ExtraContext::of(ctx) {
        Some(kind) => kind.label(),
        None => ctx,
    }
}

pub(crate) fn extra_play_context(extra: &Extra) -> &'static str {
    if extra.is_trailer() {
        TRAILER_CONTEXT
    } else {
        EXTRA_CONTEXT
    }
}

// Deliberately NOT `Default`: every construction site spells every field, so adding one to a
// season is a compile error at each of them rather than a silent zero (the counts below are
// exactly the kind of field that reads as a legitimate value when it defaults).
#[derive(Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Season {
    pub(crate) rk: String,
    pub(crate) index: i64,
    pub(crate) title: String,
    /// episodes in this season (`leafCount`); 0 when the server sent no count
    pub(crate) leaf_count: i64,
    /// how many of those are watched (`viewedLeafCount`)
    pub(crate) viewed_leaf_count: i64,
}

impl Season {
    /// Every episode of this season is watched — the season-scope form of the container rule
    /// `fetch_detail` applies to a show (`viewed >= leaf && leaf > 0`). The `leaf_count > 0` half is
    /// load-bearing: a season the server sent no counts for is `0 >= 0`, which would otherwise read
    /// as watched.
    ///
    /// **No caller outside its own tests.** This doc used to claim the season tab's tick read it —
    /// that tick does not exist, and the two sites that DO spell the rule out (`fetch_detail`'s
    /// container test below, `catalog::unwatched`) hold a `CatalogMovie`, not a `Season`, so neither can
    /// call it. Kept for the "Mark Season Watched" row, and because the tests below are where the
    /// `leaf_count > 0` guard is actually written down.
    #[allow(dead_code)]
    pub(crate) fn watched(&self) -> bool {
        self.leaf_count > 0 && self.viewed_leaf_count >= self.leaf_count
    }
}

/// A tile of the Related shelf — the **shared catalog row**, not a private three-field struct.
///
/// It used to be `{ rk, title, thumb }`: the poster art and nothing about the item. That shortfall
/// was load-bearing in two visible ways. The shelf could draw no watched tick and no resume bar,
/// while every other poster surface in the app draws both; and the press-and-hold context menu had
/// nothing to build rows from, so a hold on a Related tile did nothing at all — the owner-reported
/// gap — while the same hold on Home, the Library grid, Search and a person's filmography opened a
/// menu.
///
/// **The data was never missing.** `/related`'s rows are the SAME wire DTO every other listing
/// parses, carrying `viewCount`, `viewOffset`, `duration`, `type` and `Media[0].Part[0]`;
/// `fetch_related` simply copied three fields out and dropped the rest. So the fix is not to widen
/// this struct field by field but to stop having one: `crate::catalog::parse_item` WAS the ONE
/// `server::Metadata` → row mapping that the hub catalog, the Library grid and the person page
/// shared, and it owned rules a re-derivation gets wrong. The sharpest is that a related
/// **SHOW** is watched on `viewedLeafCount >= leafCount` and never on `viewCount > 0`, so a series
/// you are three episodes into is neither watched nor unwatched — which is exactly the state whose
/// menu must offer BOTH write verbs (`ui::widgets::row_watch_state`).
///
/// Carrying `sid` is the second thing this buys, and it is a correctness property rather than a
/// convenience: a related item is a key on the server THIS PAGE is mounted on, and both servers
/// number their ratingKeys from 1. The fetch that fills the shelf
/// stamps each row with the sid it fetched from, so every downstream use — the art request, the
/// context menu's `SID`, the scrobble — addresses the right machine BY CONSTRUCTION rather than by
/// a comment asking the next caller to remember `server::current_server()` is the wrong answer here.
pub(crate) type Related = crate::catalog::CatalogMovie;

/// Clone because the playing-item store keeps the played leaf's OWN chapters (see [`PlayingItem`]) —
/// on the detail-page play path they are cloned from the already-loaded `Detail` rather than refetched.
#[derive(Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Chapter {
    pub(crate) index: i64,    // 1-based chapter number
    pub(crate) start_ms: i64, // startTimeOffset — the seek target + timestamp label
    pub(crate) title: String, // Chapter.tag; empty → UI shows "Chapter {index}"
    pub(crate) thumb: String, // server image path → resolve_tex_wh_on (empty if no chapter thumbs)
}

/// Which timeline segment a [`Marker`] describes. Only the two the player acts on are modelled —
/// the server also emits `commercial` on recorded content, which [`convert_markers`] drops, so an
/// unhandled kind can never be mistaken for one of these.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum MarkerKind {
    Intro,
    Credits,
}

/// A server-detected intro / credits segment of the playing item (`?includeMarkers=1`). Drives
/// the in-player Skip prompt and — for an episode with something queued after it — the moment the
/// Up Next control takes over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Marker {
    pub(crate) kind: MarkerKind,
    pub(crate) start_ms: i64,
    pub(crate) end_ms: i64,
    /// this credits segment runs to the end of the item (the server `final: true`)
    pub(crate) final_seg: bool,
}

/// Record that `m` has been skipped, so it is never offered again for this item.
///
/// This is what makes skipping terminal, and it is not belt-and-braces. `av_seek_frame` is called
/// with `AVSEEK_FLAG_BACKWARD` (`ff.rs`), so it lands on the keyframe **at or before** the target —
/// seeking to a marker's `end_ms` therefore resumes a few seconds INSIDE the segment, whose keyframe
/// spacing is the file's, not ours. Without this latch the button reappeared moments after the skip
/// and pressing it seeked to the same place again: press → jump back a little → press → forever.
/// Padding the seek target cannot fix that (keyframe intervals vary from 2 s to 10 s); refusing to
/// re-offer a segment the user has already dismissed can, and is what they meant by the press.
fn mark_skipped(state: &mut MetadataState, m: Marker) {
    let key = (m.kind, m.start_ms);
    if !state.skipped.contains(&key) {
        state.skipped.push(key);
    }
}

/// The last stretch of an episode counts as its credits when the server never said where the
/// credits are. Credits DETECTION is a Plex Pass feature: on a server without one, no item ever
/// carries a credits marker, so the Up Next tile — armed exclusively off that marker — could
/// never appear, and binge-watching ended every episode by dropping the user back to the detail
/// page (found by the Plex Pass dependency audit after issue #22). Synthesizing the segment
/// reuses the entire existing chain — tile, countdown, cancel latch, HUD hold — instead of
/// growing a parallel EOS path.
///
/// Deliberately narrow: only when a successor EXISTS (a movie's tail must not grow a Skip
/// Credits pill pointing nowhere), only when the item carries no credits marker AT ALL (a server
/// that said "credits start at 41:03" must not be second-guessed at 30s-before-end), and only
/// when the item is long enough that its tail is clearly an ending (> 3x the window, so a short
/// clip does not spend a third of its runtime offering the next one).
/// The pure half of [`MetadataView::synthesized_tail_marker`] — the window geometry alone, host-testable.
/// `window_ms` is the user's `Up Next` timing (`route::up_next_window_ms`), whose default is
/// [`crate::server::session::UP_NEXT_WINDOW_DEFAULT_S`]. There is no second constant here on purpose:
/// a default spelled in two places drifts. The guard scales with it, so a short episode
/// never spends a third of its running time offering a card, and a window of zero (the card at the
/// very end) would leave nothing to show, hence `None`.
pub(crate) fn tail_marker_with(pos_ms: i64, dur_ms: i64, window_ms: i64) -> Option<Marker> {
    if window_ms <= 0 || dur_ms < window_ms * 3 {
        return None;
    }
    let start_ms = dur_ms - window_ms;
    (pos_ms >= start_ms).then_some(Marker {
        kind: MarkerKind::Credits,
        start_ms,
        end_ms: dur_ms,
        final_seg: true,
    })
}

/// The marker containing `pos_ms`, if any — the ONE "am I inside a skippable segment" rule, shared
/// by the skip prompt and the end-of-episode handoff so they can never disagree about where a segment
/// begins. The range is half-open (`start <= pos < end`) so the prompt clears itself the instant a
/// skip lands on `end_ms` rather than re-offering the segment it just left.
///
/// A `final` credits marker is treated as running to `i64::MAX` rather than its stated `end_ms`:
/// the server sets that end to the container duration, but our playhead is the DECODER's,
/// which routinely stops a few hundred ms short of it — so the prompt would blink out over the last
/// frames.
pub(crate) fn marker_at(markers: &[Marker], pos_ms: i64) -> Option<Marker> {
    markers
        .iter()
        .find(|m| {
            let end = if m.final_seg { i64::MAX } else { m.end_ms };
            pos_ms >= m.start_ms && pos_ms < end
        })
        .copied()
}

/// Which badge artwork a `Rating.image` string names — the provider AND its icon state, both of
/// which the server encodes in that one string. Rotten Tomatoes has **five** states:
/// `rottentomatoes://image.rating.ripe` is the fresh tomato, `…rating.certified` the Certified
/// Fresh one, `…rating.rotten` the green splat, `…rating.upright` the standing popcorn bucket and
/// `…rating.spilled` the tipped one; `imdb://image.rating` and `themoviedb://image.rating` carry no
/// state because those providers have only one mark.
///
/// The art is chosen by parsing that string and **never** by comparing `value` to a threshold:
/// Rotten Tomatoes' critic and audience cutoffs differ from each other and move, so a 6.0 can be
/// fresh on one axis and rotten on the other — the server already knows which, and says so here.
/// The PROVIDER likewise comes from the URI scheme and never from `Rating.type`: IMDb and TMDB both
/// arrive as `audience`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum RatingArt {
    TomatoFresh,
    /// Certified Fresh — a distinct Rotten Tomatoes mark (the wreathed tomato), not a synonym for
    /// [`RatingArt::TomatoFresh`]. It is a rarer, higher bar than plain fresh, the server takes the
    /// trouble to name it, and it is the one state whose art we would otherwise be inventing by
    /// substitution. Folding it onto the plain tomato threw that away silently.
    TomatoCertified,
    TomatoRotten,
    PopcornUpright,
    PopcornSpilled,
    Imdb,
    Tmdb,
}

impl RatingArt {


    /// The provider's name, **as the provider sets it** — the row spells this in words instead of
    /// drawing anyone's mark, so it is the only thing identifying the source and it has to be
    /// right. `IMDb`, not `IMDB`.
    ///
    /// It is also the GROUPING key: Rotten Tomatoes' critic and audience scores are two readings
    /// from one source, so they share one caption and sit under it as a pair, while IMDb and TMDB
    /// are a caption and a number each. Equal names group; [`RatingArt::rank`] already orders the
    /// five RT states adjacently, so grouping is a run-length pass over the sorted list and never
    /// needs to reorder anything.
    pub(crate) fn provider(self) -> &'static str {
        match self {
            RatingArt::Imdb => "IMDb",
            RatingArt::TomatoFresh
            | RatingArt::TomatoCertified
            | RatingArt::TomatoRotten
            | RatingArt::PopcornUpright
            | RatingArt::PopcornSpilled => "ROTTEN TOMATOES",
            RatingArt::Tmdb => "TMDB",
        }
    }
}

/// One review score to badge on the detail hero: the artwork the server named, the score as
/// the server normalises it (0–10 for every provider — a 91% tomato arrives as 9.1), and whether
/// the server filed it as a critic or an audience score.
#[derive(Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Rating {
    pub(crate) art: RatingArt,
    #[serde(with = "record::float_bits")]
    pub(crate) value: f64,
    pub(crate) critic: bool,
}

#[derive(Default, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Detail {
    /// WHICH SERVER this item was fetched from — the other half of its identity. `rk` on its own
    /// names an item on no machine in particular the moment a shared server is registered (both
    /// number from 1), and every equality test that reads this struct
    /// therefore compares the pair through [`crate::registry::same_item`]: `cached_playing`'s cache
    /// hit, `pump_season`'s ownership test, `detail::reselect`, and the BACK trail's node.
    #[serde(with = "record::server_id")]
    pub(crate) sid: crate::registry::ServerId,
    pub(crate) rk: String,
    /// Which SERVER this item was fetched from, as the OWNER'S HANDLE ("friend") — empty whenever
    /// it came from the signed-in user's own server, which is every item today.
    ///
    /// The item's PORTABLE identity (`plex://movie/…`) — the same string on every server that
    /// matched this film, and the only one that is. It is what "Also available" asks the other
    /// sources about, because their copy has a different `rk` and may even have a different title:
    /// measured across this household's two servers, one film is `2029` here and `5274` there,
    /// under a Russian title. Empty when the server sent none, which simply means no cross-source
    /// lookup is possible for this item.
    pub(crate) guid: String,
    pub(crate) is_show: bool,
    pub(crate) kind: String, // this item's own type: movie | episode | show | season
    pub(crate) show_title: String, // grandparentTitle — the show name, when this item is an episode
    pub(crate) show_rk: String, // grandparentRatingKey — the show's rk (episode → its show)
    pub(crate) season: i64,  // parentIndex — season number, when an episode
    pub(crate) index: i64,   // index — episode number, when an episode
    pub(crate) title: String,
    pub(crate) year: i64,
    pub(crate) rating: String, // contentRating
    pub(crate) summary: String,
    /// The marketing one-liner (`tagline`) — *"Everyone deserves the chance to fly."*
    ///
    /// **Atmosphere, never content**, which is why it is drawn only in the About alert
    /// ([`crate::screens::about_panel`]) under the synopsis and nowhere on the page itself: it says
    /// nothing a viewer needs in order to decide, so it earns a line only where there is room to
    /// read the whole record. Empty for most items and for every episode — absence is the ordinary
    /// case, and the panel drops the line AND its gap rather than reserving a hole.
    ///
    /// It arrives on the SINGLE-key `/library/metadata/{rk}` fetch, which asks for no field
    /// exclusions. Both of the other reads in `plex/library.rs` pass
    /// `excludeFields=summary,tagline` — the batched `metadata_many` and the section listing — so
    /// anything derived from THOSE has never seen it and never will.
    pub(crate) tagline: String,
    pub(crate) aired: String,
    pub(crate) dur_ms: i64,
    pub(crate) resume_ms: i64, // viewOffset (0 = not partially watched) — the resume position
    pub(crate) watched: bool,  // movie: viewCount ≥ 1; show: viewedLeafCount ≥ leafCount
    /// The heart's state (Jellyfin's `UserData.IsFavorite`; always false on Plex — the backend
    /// has no such flag, and the heart is only drawn on the flavor that does). False stays OFF
    /// the wire: an unfavorited item's record is byte-identical to one that predates the field
    /// (the committed fixtures' canonical gate prices exactly that).
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) is_favorite: bool,
    pub(crate) part: String,   // Media[0].Part[0].key for a leaf (movie/episode); empty for a show
    pub(crate) vcodec: String, // Media[0].videoCodec (drives the direct-play/transcode decision)
    pub(crate) acodec: String, // Media[0].audioCodec
    #[serde(with = "record::float_bits")]
    pub(crate) video_fps: f64, // video Stream frameRate (0 = unknown); feeds the Load esInfo
    // ---- the PRIMARY version's technical fields (server::Metadata::primary_media = Media[0], NOT a
    // best-of pick — a multi-version item has more, and choosing among them needs a version picker
    // that does not exist yet). For a SHOW these are borrowed from its first episode, like the
    // About footer's audio/subtitle lists. `bitrate`/`width`/`height` are unused by the UI today
    // and carried for the video-quality ladder ("26.1 Mbps 4K (Original)").
    pub(crate) video_resolution: String, // "4k" | "1080" | "720" | "sd" — the hero's media badge
    pub(crate) width: i64,               // stored frame size, not the resolution class (1918x802
    pub(crate) height: i64,              // is a 1080p scope movie) — badge off video_resolution
    pub(crate) bitrate: i64,             // kbps, whole-stream
    // ---- the rest of the primary version's technical record, added for `screens::tracks_panel` — and
    // since 2026-08-23 ON THE ROUTING PATH too: `route::source_kbps` takes `video`'s own bitrate
    // from here to judge a source against the user's quality ceiling, preferring it over the
    // whole-file `bitrate` above precisely so a rung does not bite one AC-3 track early. So this
    // block is no longer only the inspector's, and dropping the backfill would silently move every
    // rung's threshold with a green suite. Same caveat as the block above: version 0, not a
    // best-of pick.
    /// `Part[0].container`, falling back to `Media[0].container` — `"mp4"`, `"mkv"`, ….
    pub(crate) container: String,
    /// `Part[0].file` — the part's absolute path ON THE SERVER. Shown as the Track-information
    /// panel's header line. Not a URL, not reachable from here, and the one field on `Detail` most
    /// likely to be non-ASCII, so elide it by CHARACTER.
    pub(crate) file: String,
    /// `Part[0].size` in BYTES (0 = the server did not say).
    pub(crate) size: i64,
    /// `Media[0].aspectRatio` as a number — `2.35` (0.0 = not said).
    #[serde(with = "record::float_bits")]
    pub(crate) aspect_ratio: f64,
    /// The primary version's VIDEO track, whole. `vcodec`/`width`/`height`/`bitrate` above are the
    /// Media-level summary the play path reads; this is the stream's own record, and the only
    /// place its profile, bit depth, chroma and per-stream bitrate live. `None` for a show
    /// container that never got an episode backfill, and for an audio-only part.
    pub(crate) video: Option<Stream>,
    /// the video stream is HDR (PQ/HLG transfer or Dolby Vision) — with [`Self::hdr`] true AND
    /// the item facing a real RE-ENCODE (`route::Preview::Converts`, **not** merely "not
    /// direct-playable": a container-only remux copies the picture and keeps HDR10 intact) AND the
    /// server known Pass-less, the facts row warns that the transcode will be HDR→SDR without
    /// tone-mapping (a server feature that may be absent). Any weaker
    /// combination shows nothing.
    pub(crate) hdr: bool,
    /// The video stream's Dolby Vision layering. [`Self::hdr`] answers "should the facts row warn
    /// about tone mapping"; this answers the harder one the PLAY path needs — whether the base
    /// layer alone is a correct picture, i.e. whether direct play is honest for this file. Read by
    /// [`crate::route::playback_preview_of`] so the page's "how this plays" answer agrees with
    /// what Play will actually do.
    pub(crate) dovi: Dovi,
    pub(crate) art: String,
    pub(crate) thumb: String,
    /// The item's own `UltraBlurColors` corners (tl, tr, br, bl — the ring order
    /// [`server::UltraBlurColors::corners`](crate::media::UltraBlurColors::corners) owns) and whether
    /// the server sent a usable envelope — what keys the detail page's ambient GROUND. It lives on
    /// the LOADED item and not only on the catalog row because a page opened from the Library grid,
    /// a Related tile or the person page is never in the home catalog (`catalog::index_of_rk` searches
    /// the hubs only), and those were exactly the pages sitting on flat grey with no wash at all.
    #[serde(with = "record::blur_bits")]
    pub(crate) blur: [[f32; 3]; 4],
    pub(crate) has_blur: bool,
    pub(crate) genres: Vec<String>,
    pub(crate) countries: Vec<String>,
    pub(crate) cast: Vec<Cast>,
    /// Director[] tags, in server order — the hero's "Directed by …" line.
    pub(crate) directors: Vec<String>,
    /// Director[] + Writer[] as credits (each carrying its JOB in `role`), drawn on the Cast &
    /// Crew shelf AFTER the actors. Kept apart from `cast` so "who acted" stays answerable; the
    /// shelf addresses both through [`Detail::credit`].
    pub(crate) crew: Vec<Cast>,
    pub(crate) audio: Vec<Stream>,
    pub(crate) subs: Vec<Stream>,
    pub(crate) seasons: Vec<Season>,   // shows only
    pub(crate) episodes: Vec<Episode>, // the currently-selected season
    /// SHOWS: the episode the SERVER says is next to watch (`OnDeck`, one request, no extra round
    /// trip). Show-level and therefore **independent of the selected season tab**, which is the whole
    /// reason it is here: `episodes` above holds one season, so a next-episode the client worked out
    /// itself changed every time you browsed to another tab. `None` for a movie, for a show with
    /// nothing on deck (never started, or finished), and on any server that omitted the hub.
    pub(crate) on_deck: Option<Episode>,
    pub(crate) cur_season: usize,
    pub(crate) related: Vec<Related>,
    pub(crate) chapters: Vec<Chapter>,
    pub(crate) markers: Vec<Marker>, // intro / credits segments (leaf items only)
    pub(crate) ratings: Vec<Rating>, // review scores, critic-first (see convert_ratings)
    /// Every extras row, server order. Empty when the server sent none, the GET was refused and no
    /// primary trailer could be filled, or this item is an episode/season (those never request
    /// extras). The Trailer control reads [`Self::trailer`], not this vec's first element.
    #[serde(default)]
    pub(crate) extras: Vec<Extra>,
    /// Rating key of the picker winner inside [`Self::extras`]. Empty when there is none.
    #[serde(default)]
    pub(crate) trailer_rk: String,
}

impl Detail {
    /// **Does this item have a FILE of its own?** — the rule behind the *Track information* sheet
    /// (`screens::tracks_panel`) and the Languages column's press gate on the detail page.
    ///
    /// It is `part` rather than `is_show` on purpose. A SHOW container carries no `Media` of its
    /// own — the backend's own fetch backfills its audio/subtitle lists from the hero episode for
    /// the About footer — so on a show page a track sheet would print episode 1's path, size and
    /// bitrate under the show's name. That is not a truncation of the truth, it is a different
    /// file, and `part` is exactly the field that is empty when the item has no file of its own
    /// ("Media[0].Part[0].key
    /// for a leaf (movie/episode); empty for a show").
    ///
    /// The rule lives on the DATA so both the page and the sheet read one answer: phase 10's
    /// `sibling` gate forbids a screen naming another screen, and the predicate used to be the
    /// panel's `is_available()`, a module function reading `metadata::current()` unfiltered, which
    /// Detail's seven layout reads asked before the page's own item had landed.
    pub(crate) fn has_own_file(&self) -> bool {
        !self.part.is_empty()
    }

    /// The extras row whose rating key is `rk`, if this detail carries it.
    pub(crate) fn extra(&self, rk: &str) -> Option<&Extra> {
        self.extras.iter().find(|e| e.rk == rk)
    }

    /// Picker winner: the playable trailer `primaryExtraKey` named, else the first playable
    /// trailer in server order. Not a stored second copy of the extra.
    pub(crate) fn trailer(&self) -> Option<&Extra> {
        if !self.trailer_rk.is_empty() {
            if let Some(e) = self.extra(&self.trailer_rk).filter(|e| e.playable() && e.is_trailer())
            {
                return Some(e);
            }
        }
        self.extras.iter().find(|e| e.playable() && e.is_trailer())
    }

    /// **WHOSE copy this is** — the credit for the server this item came from, or empty when there
    /// is nobody to credit. `server::servers::owner_credit` decides it, `ServerFacts::handle` holds
    /// the answer, and `ui::fmt::shared_by` turns it into the words; this is only where the detail
    /// page asks.
    ///
    /// The person, never the machine: the machine's name (`nas-home`) belongs to the Sources list
    /// and to a failure read-out, and appears nowhere else in the product. Empty is the ABSENCE of
    /// an attribution, not an empty one — the detail hero draws no separator and no run at all for
    /// it (`ui::detail`'s facts row), so a single-server library pays nothing for the feature: no
    /// gap, no dot, no draw call.
    ///
    /// **Read from the registry AT USE, and it used to be a `String` captured at FETCH time.** The
    /// reasoning for storing it was that the page outlives the fetch and the current server can
    /// move under it — which [`Detail::sid`] answers: with the id in hand the credit can be
    /// re-asked at any moment, and it has to be. The stored copy could go stale two ways and
    /// neither had a repair: a roster refresh re-grades the credit under a MOUNTED page (nothing
    /// invalidates one), and a detail fetch begun before that correction lands after it, carrying
    /// the old answer past every epoch that would otherwise have caught it.
    ///
    /// dev: **`/tmp/butaca-shared` WINS WHEN ARMED** — the precedence every trigger in this app
    /// has (`crate::dev`'s module doc: `butaca-token` beats the signed-in session), and the
    /// phrase to grep for, because the same stand-in is read by the owned Home's hero run and
    /// the two must agree. (The owned `screens::search::render`'s owner annotation reads the real
    /// registry unconditionally and does not consult this trigger — a gap the cutover left open,
    /// not a third agreement point.) A trigger exists to FORCE
    /// a state, so an armed one outranks the real answer, and an armed EMPTY file forces the
    /// absence of a handle rather than doing nothing. It stamps one handle onto every item this
    /// session loads, which is what a fully-borrowed library looks like. Read ONCE (see
    /// [`dev_source`]) — the trigger surface is boot state, and this is reached from a draw.
    pub(crate) fn source(&self) -> String {
        // The roster credit (`facts(sid)`'s handle) went with the describers (#166): a
        // single-source library is this account's own, so the answer is the dev stand-in or
        // nothing.
        dev_source().map(str::to_owned).unwrap_or_default()
    }

    /// How many tiles the Cast & Crew shelf holds: every actor, then every crew credit.
    pub(crate) fn credits_len(&self) -> usize {
        self.cast.len() + self.crew.len()
    }
    /// The `i`th shelf credit in that one flat index space, so the screen's focus/geometry can
    /// address a tile by position without re-deriving which vec it fell in.
    pub(crate) fn credit(&self, i: usize) -> Option<&Cast> {
        if i < self.cast.len() {
            self.cast.get(i)
        } else {
            self.crew.get(i - self.cast.len())
        }
    }

    /// Only the crew array owns job identities. An actor whose server-provided character is
    /// named "Director" must retain that exact character name in every locale.
    pub(crate) fn credit_role(&self, i: usize) -> Option<&str> {
        self.credit_role_in(i, crate::i18n::current())
    }

    fn credit_role_in(&self, i: usize, locale: &crate::i18n::LocaleContext) -> Option<&str> {
        let credit = self.credit(i)?;
        if i < self.cast.len() {
            return Some(&credit.role);
        }
        Some(CrewRole::from_key(&credit.role).map(|role| role.display(locale)).unwrap_or(&credit.role))
    }
}

/// the currently-loaded detail item, or None
fn current(state: &MetadataState) -> Option<&Detail> {
    state.current.as_ref()
}
/// TEST-ONLY installer for the loaded item. The UI's layout tests need a `Detail` on screen with
/// no server behind them; every other writer of `CURRENT` goes through the landing mailbox,
/// which is exactly the invariant those tests must not have to fake. Crate-global, so callers hold
/// [`crate::testlock::serial`].
#[cfg(test)]
pub(crate) fn install_for_test(state: &mut MetadataState, d: Option<Detail>) {
    crate::testlock::assert_held("the detail store (install_for_test)");
    state.current = d;
}

/// **OPTIMISTIC**, MAIN THREAD: flip what the LOADED item says about `(sid, rk)`'s watch state,
/// before the server has been told. Returns whether anything on this page was about that item.
///
/// The detail page's twin of [`crate::catalog::edit_item`], and it exists for the same reason: the write
/// that justifies it now happens on a worker (`crate::viewstate`), so without this the hero's toggle
/// and the filmstrip's checks would sit unchanged for as long as the item's server takes to answer —
/// which on a share is seconds, and reads as the press having missed.
///
/// THREE things can be about the item, and all three are updated:
/// * **the loaded item itself** — a movie, or the show whose hero toggle was pressed;
/// * **one EPISODE of the loaded season** — the filmstrip's context menu, whose rk is a leaf of the
///   show `CURRENT` holds. Its season's `viewedLeafCount` moves with it, because the season tab's
///   tick is derived from that count ([`Season::watched`]) and a tick that disagreed with the
///   episode row beneath it is precisely the "two surfaces describing one item two ways" this page
///   already refuses elsewhere.
/// * **a tile of the RELATED shelf** — a different item entirely, which is what makes it the odd
///   one out: it is matched on its OWN `sid` (each row carries one, see [`Related`]) rather than on
///   the page's, and it is checked unconditionally rather than as one arm of a chain. Since
///   2026-08-21 that shelf's tiles have a context menu of their own, so this page can mark a third
///   item watched — and without this pass the tile it was pressed on kept its old tick and resume
///   bar until a refetch, which reads as the row having done nothing.
///
/// `resume_ms` is cleared with the flag for the same reason `catalog::set_watched` clears it: the
/// still's own resolver (`ui::detail::ep_state`) reads progress ahead of the watched mark, so an
/// episode keeping its old `viewOffset` would still draw its resume bar and no check.
///
/// The landed refresh is the truth and silently corrects any of this; see [`crate::viewstate`].
/// The heart toggle's optimistic half: only the loaded item's flag — the heart is not drawn on
/// cards, so there is no shelf fan-out to walk.
fn set_favorite_local(state: &mut MetadataState, sid: crate::registry::ServerId, rk: &str, on: bool) -> bool {
    let Some(d) = state.current.as_mut() else {
        return false;
    };
    if d.sid == sid && d.rk == rk && d.is_favorite != on {
        d.is_favorite = on;
        return true;
    }
    false
}

fn set_watched_local(state: &mut MetadataState, sid: crate::registry::ServerId, rk: &str, on: bool) -> bool {
    {
        let Some(d) = state.current.as_mut() else {
            return false;
        };
        // The RELATED shelf first, and unconditionally — it is the one store here whose rows are
        // OTHER items, so it is neither the loaded item nor one of its episodes and must not be
        // reached through either of their early returns below. A related tile is also the one the
        // press most often came FROM (its context menu is why this page can mark a third item
        // watched at all), and left out, the tile kept its old tick and bar until a refetch.
        //
        // Not `else`-chained with the two arms under it for a subtler reason as well: the same
        // title can legitimately be BOTH the loaded item and a row of some other page's shelf, and
        // a hub that lists an item alongside itself is not something this function should trust
        // itself to rule out.
        let mut hit = false;
        for m in d.related.iter_mut() {
            if crate::registry::same_item((m.sid, &m.rk), (sid, rk)) {
                // the shared three-field flip (`watched`/`unwatched`/`resume_ms` move together, or
                // the tile wears the progress bar it had before and shows no tick at all)
                crate::catalog::set_watched(m, on);
                hit = true;
            }
        }
        if crate::registry::same_item((d.sid, &d.rk), (sid, rk)) {
            d.watched = on;
            d.resume_ms = 0;
            return true;
        }
        // An episode is only ever a leaf of the loaded show, so it is matched on the PAGE's server —
        // `Episode` carries no `sid` of its own precisely because it cannot come from anywhere else.
        // Both misses below return `hit` rather than `false`: the Related pass above may already
        // have changed this page, and reporting "nothing here was about that item" after editing a
        // tile would be this function contradicting itself. (`viewstate` only logs the verdict, but
        // a false negative is the kind that goes unnoticed until something starts trusting it.)
        if d.sid != sid {
            return hit;
        }
        let Some(i) = d.episodes.iter().position(|e| e.rk == rk) else {
            return hit;
        };
        let was = d.episodes[i].watched;
        d.episodes[i].watched = on;
        d.episodes[i].resume_ms = 0;
        if was != on {
            let cur = d.cur_season;
            if let Some(s) = d.seasons.get_mut(cur) {
                // clamped to the season's own leaf count: a server that sent none leaves this 0, and
                // `Season::watched` reads `leaf_count > 0` first, so an unknown season stays unknown
                let step = if on { 1 } else { -1 };
                s.viewed_leaf_count = (s.viewed_leaf_count + step).clamp(0, s.leaf_count.max(0));
            }
        }
        true
    }
}

/// drop the loaded detail (on leaving the detail page). Also supersedes any in-flight async
/// fetch — otherwise a load requested on the way in lands after the page closed and silently
/// repopulates CURRENT (and NOW, via `sync_now_playing`) behind whatever screen is now mounted.
fn clear(state: &mut MetadataState, adapter: &MetadataAdapter) {
    supersede_detail(adapter);
    state.current = None;
    // The *Also available* copies describe the item that is going, so they go with it. Nothing
    // reads them afterwards — the store is addressed and no page's pair can match an empty one —
    // but a departing page should not leave another item's list in memory for the next one to be
    // handed if it ever happened to share both halves of the address.
    alt_clear(state);
}

/// The server/profile switch: unlike `clear()`, this drops the COMPLETE owned state — the `now`
/// caption and `playing` track store `clear()` deliberately spares (D3, for a Detail page torn
/// down and reopened mid-playback) do not belong to the NEXT profile. Adapter rotation is done by
/// the caller (`stores::metadata::MetadataStore::run`, mirroring `PersonStore`/`SearchStore`),
/// and it rotates BEFORE this runs, so `adapter` here is already the fresh one and nothing still
/// targeting the retired `Arc` can land into this state. `supersede_detail` is still called, for
/// symmetry with `clear()`, but rotation alone is what fences the old adapter's in-flight work.
fn reset(state: &mut MetadataState, adapter: &MetadataAdapter) {
    supersede_detail(adapter);
    state.current = None;
    state.now = None;
    state.playing = None;
    state.skipped.clear();
    alt_clear(state);
}

/// TEST ONLY — install `d` as the loaded item, bypassing the fetch and its mailbox. The screens'
/// pure focus/label math reads `current()`, and the only real way to populate it is a server round
/// trip, which the host suite has no server for. Compiled out of the shipped binary. `state` is
/// the owner's own `MetadataState`, not a global — the detail and season mailboxes are per-owner
/// fields too, like the other five stores — but `assert_held` below enforces the same crate-wide
/// lock the genuinely-still-global seams (route's play mailbox, the player's SHARED block) also
/// take, by convention one lock rather than one per module. Hold `crate::testlock::serial()`
/// across any test that calls this.
#[cfg(test)]
pub(crate) fn set_current_for_test(state: &mut MetadataState, d: Option<Detail>) {
    crate::testlock::assert_held("the detail store (set_current_for_test)");
    state.current = d;
}

/// A compact descriptor of the item currently *playing*, for the in-player Info card. Unlike
/// `current()` (which stays on the detail page's show/movie), this always describes the playing
/// **leaf**: an episode carries the show title + SxEy + episode name + its still; a movie carries the
/// movie title + landscape art. Set by the play paths — `sync_now_playing()` after a leaf load, or
/// explicitly by show-page episode play (where `current()` is still the show).
#[derive(Clone, Debug)]
pub(crate) struct NowPlaying {
    /// Whether the "Go to X" target (`detail_rk`) is a show rather than a movie, and whether the
    /// info card should title itself from `ep_title` rather than `title`. True for a real episode
    /// AND for a show's trailer/extra — a show trailer still labels "Go to Show" and titles itself
    /// from the extra's own name. **Not** "does this leaf carry a real episode address" — see
    /// [`Self::is_real_episode`] for that, which is a strictly narrower question.
    pub(crate) is_episode: bool,
    /// True only for a genuine episode leaf, where `season`/`index` are a real address. False for
    /// a movie AND for every extra (a trailer's `season`/`index` are placeholder zeros, never a
    /// real address) — including a show's trailer, where [`Self::is_episode`] is true but this is
    /// not. Gates the player HUD's `S# · E#` kicker line: filtering on `is_episode` there rendered
    /// `S0 · E0` under a show trailer, since a show trailer has no episode address to print.
    pub(crate) is_real_episode: bool,
    pub(crate) title: String, // big title: show title (episode) or movie title
    pub(crate) ep_title: String, // episode name (episode only)
    pub(crate) season: i64,
    pub(crate) index: i64,
    pub(crate) summary: String,
    pub(crate) year: i64,
    pub(crate) dur_ms: i64,
    pub(crate) rating: String,
    pub(crate) thumb: String, // 16:9 still (episode) / landscape art (movie)
    pub(crate) detail_rk: String, // "Go to Show"/"Go to Movie" target
}
fn set_now_playing(state: &mut MetadataState, np: Option<NowPlaying>) {
    state.now = np;
}
/// Refresh `now_playing` from `current()` — call after a leaf `load_detail` (Continue-Watching /
/// off-catalog play, where `current()` becomes the played leaf). A show/season load leaves it None.
fn sync_now_playing(state: &mut MetadataState) {
    let np = current(state).and_then(|d| match d.kind.as_str() {
        "episode" => Some(NowPlaying {
            is_episode: true,
            is_real_episode: true,
            title: d.show_title.clone(),
            ep_title: d.title.clone(),
            season: d.season,
            index: d.index,
            summary: d.summary.clone(),
            year: d.year,
            dur_ms: d.dur_ms,
            rating: d.rating.clone(),
            thumb: d.thumb.clone(),
            detail_rk: d.show_rk.clone(),
        }),
        "movie" => Some(NowPlaying {
            is_episode: false,
            is_real_episode: false,
            title: d.title.clone(),
            ep_title: String::new(),
            season: 0,
            index: 0,
            summary: d.summary.clone(),
            year: d.year,
            dur_ms: d.dur_ms,
            rating: d.rating.clone(),
            thumb: if !d.art.is_empty() {
                d.art.clone()
            } else {
                d.thumb.clone()
            },
            detail_rk: d.rk.clone(),
        }),
        _ => None, // show / season → not a playing leaf
    });
    state.now = np;
}

/// Info-card descriptor for a trailer extra: parent identity (Go to Movie/Show, art, summary)
/// with the extra's own duration and title. `None` when `current()` is not that extra's parent.
/// Call only after `request_play` accepted the session, so a refused play cannot wipe a leftover
/// episode descriptor.
pub(crate) fn trailer_now_playing(
    state: &MetadataState,
    sid: crate::registry::ServerId,
    extra_rk: &str,
) -> Option<NowPlaying> {
    let d = current(state)?;
    let extra = d
        .extras
        .iter()
        .find(|e| crate::registry::same_item((d.sid, e.rk.as_str()), (sid, extra_rk)))?;
    Some(NowPlaying {
        is_episode: d.is_show,
        // An extra is never a real episode leaf, whatever kind its parent is — `season`/`index`
        // below are placeholder zeros, not an address, so the HUD kicker must not read them.
        is_real_episode: false,
        title: d.title.clone(),
        ep_title: extra.hud_title(&d.title).to_string(),
        season: 0,
        index: 0,
        summary: d.summary.clone(),
        year: d.year,
        dur_ms: extra.dur_ms,
        rating: d.rating.clone(),
        thumb: if !d.art.is_empty() {
            d.art.clone()
        } else {
            d.thumb.clone()
        },
        detail_rk: d.rk.clone(),
    })
}

// ---- fetches (all via the typed crate::plex client; serde DTOs, no Value scraping) ----
//
// Every one of them takes the `ServerId` rather than reaching for `server::client()`: they run on the
// detail/season workers, and the house rule is that a worker reads no statics. It is also what
// stamps the row — an item fetched from slot 1 must be
// recorded as slot 1's whatever `client()` answers with by the time the fetch returns.
/// The stand-in handle, read ONCE. The trigger surface is boot state, and [`Detail::source`] is
/// reached from a draw — a `stat` per frame inside one is exactly what `crate::dev`'s doc forbids.
/// Compiled out with the `devtriggers` feature, like every other trigger, so a release build folds
/// this to `None` and the whole call to the registry read below it.
#[cfg(not(test))]
fn dev_source() -> Option<&'static str> {
    if crate::app::bootstrap::stores::active() { return None; }
    // Function-local, not process-wide mutable state: one dev-trigger stat per process, kept off
    // the per-frame draw path the doc above forbids. Without `devtriggers`, `crate::dev::read`
    // is a `None`-returning stub, so a release build pays one cheap `get_or_init` for a value
    // that is always `None` — not worth a cfg to avoid.
    static SEEN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    SEEN.get_or_init(|| crate::dev::read("shared")).as_deref()
}
/// The host suite must not depend on what this dev Mac happens to have under `/tmp`: an armed
/// `butaca-shared` would outrank the registry and make every credit assertion here read the
/// trigger's handle instead. [`alt_dev_stand_in`] states the same rule the same way.
#[cfg(test)]
fn dev_source() -> Option<&'static str> {
    None
}

fn fetch_detail(sid: crate::registry::ServerId, rk: &str) -> Option<Detail> {
    // Jellyfin is the only backend: the store's own full fetch (item + NextUp + seasons + the
    // hero episode's streams). Trailer identity comes from the DTO itself; the old
    // `primaryExtraKey` side-channel went with the Plex extras machinery.
    if sid != crate::jellyfin::SERVER_ID {
        return None;
    }
    let c = crate::jellyfin::client()?;
    crate::jellyfin::detail::fetch_full(c, sid, rk)
}

/// Convert a part's Stream[] into (audio, subs, video_fps, video_is_hdr, dovi) — the ONE
/// server::Stream → Stream mapping (the detail parse and the playing-tracks store both use it).
/// HDR is the video stream's transfer characteristic (PQ or HLG) or a Dolby Vision flag — the
/// input to the facts row's "HDR → SDR without tone-mapping" warning, which only matters where
/// the item would transcode on a server that cannot tone-map. [`Dovi`] is the finer-grained
/// companion to that flag and answers a different question — not "is this HDR" but "can we show
/// the base layer at all" (`route::video_direct_plays` gates direct play on it).

// ---- the PLAYING-item store — the in-player source of truth ---------------------------------
// Unlike `current()` (the detail page's item — it stays on the SHOW during an episode play, and
// can be a different item entirely when playing straight from Home), this always holds the
// played leaf's OWN data. The track menu and the route's audio pick read its streams; feeding a
// menu built from episode 1's streams to a playback of episode 5 was a real track-identity bug.
// `markers` is here for exactly that reason and not on `Detail`: skipping episode 1's intro
// timing during episode 5 is the same bug wearing a different hat. `chapters` rides along for the
// third instance of it: the Chapters tab and strip used to read `current()`, so an episode played
// from a SHOW page found a show container (which carries no Chapter[]) and the tab silently
// vanished — while a `current()` holding some OTHER leaf would have seeked with its offsets.

#[derive(Clone)]
pub(crate) struct PlayingItem {
    /// The server the played leaf lives on. This is the store that feeds PLAYBACK — the track
    /// menu's `Stream.id`s (which are PUT back to a server), the direct-play gate's frame size, the
    /// esInfo fps, the chapters and the markers — so a bare-rk cache hit against a colliding item on
    /// the other machine is the silent failure this field exists to stop: every one of those values
    /// would be the wrong file's, with nothing on screen to say so.
    pub(crate) sid: crate::registry::ServerId,
    pub(crate) rk: String,
    /// The show's ratingKey when this is an episode (`grandparentRatingKey`), else empty — what
    /// the resolve asks the show's own language settings of (`route::plan::build_stream`).
    pub(crate) show_rk: String,
    pub(crate) audio: Vec<Stream>,
    pub(crate) subs: Vec<Stream>,
    pub(crate) video_fps: f64, // the played leaf's video fps (0 = unknown) — feeds the Load esInfo
    /// The source's stored frame size, `Media[0]` (0 = unknown). `route.rs`'s local direct-play
    /// gate tests it against the device table's bound before MDE (`!video_dp` → `skip_mde`). An
    /// unreachable MDE still remuxes through `transcode_decision`; the client checks
    /// width/height here so a 4K source on a 1080p-bounded SoC does not wait on a missing
    /// `/decision` (issue #22's over-claim class).
    pub(crate) width: i64,
    pub(crate) height: i64,
    /// Whole-file bitrate in kbps (`Media[0].bitrate`). Auto uses this—not merely the video
    /// stream's rate—when deciding whether a remote connection has enough headroom to carry the
    /// original file, because the transport also has to carry audio and container overhead.
    pub(crate) bitrate: i64,
    /// The source's container word (`Part[0].container`, `Media[0].container` on Plex; the
    /// MediaSource's own name on Jellyfin) — the direct-play gate's demuxer test where the part
    /// key carries no extension to read (a Jellyfin MediaSource id is a GUID). Empty means "not
    /// said", and the gate then falls back to the part key's extension, which is the same fact
    /// on Plex.

    pub(crate) container: String,
    /// The played leaf's Dolby Vision layering — the direct-play gate's other refusal, beside
    /// the frame size above and for the same reason: the local fallback never asks the server, so
    /// a file whose base layer we cannot display correctly (Profile 5, or a dual-layer Profile 7)
    /// would otherwise be fed to the decoder verbatim and shown in the wrong colours. See [`Dovi`].
    pub(crate) dovi: Dovi,
    pub(crate) markers: Vec<Marker>, // intro / credits segments — the in-player Skip prompt
    pub(crate) chapters: Vec<Chapter>, // chapter boundaries — the in-player Chapters tab/strip
    /// The played leaf's own `UltraBlurColors` corners (tl, tr, br, bl), or `None` when the server
    /// sent no usable envelope. What the player's panels dim WITH: GL cannot read the video plane,
    /// so this is the one honest source of "the light under the panel" there
    /// (`screen::Scrim::over_video`).
    pub(crate) blur: Option<[[f32; 3]; 4]>,
}

#[cfg(test)]
impl PlayingItem {
    /// A playing item on server slot 0 carrying only `subs` — the fixture the track-menu and
    /// overlay tests install, where nothing but the subtitle list is read.
    pub(crate) fn with_subs(subs: Vec<Stream>) -> Self {
        PlayingItem {
            sid: crate::registry::ServerId::from_raw(0),
            rk: "rk".into(),
            container: String::new(),
            show_rk: String::new(),
            audio: Vec::new(),
            subs,
            video_fps: 0.0,
            width: 0,
            height: 0,
            bitrate: 0,
            dovi: Default::default(),
            markers: Vec::new(),
            chapters: Vec::new(),
            blur: None,
        }
    }
}
/// Load the playing-item track store for `rk` at play time (route::build_stream). Reuses the
/// loaded detail's streams when it IS this item (no extra GET on the play path — the same
/// optimization the old `audio_tracks` fetch had); otherwise one metadata fetch. An empty `rk`
/// (local-sample / URL-override play) clears the store.
/// PURE: fetch the playing item's track lists. Safe on a worker — reads and writes no statics.
/// The cache-hit shortcut is `cached_playing` (main thread) and the install is `install_playing`,
/// because `playing()` (via `MetadataView`) hands out a `&'a` borrow whose Vecs the track menu
/// and info panel hold slices into during playback.
/// MAIN THREAD: the cache-hit half of the old `load_playing` — reuse the loaded detail's streams
/// when it IS this item, so playing from a detail page costs no extra GET. Snapshotted into
/// `ResolveEnv` and handed to the worker; splitting the fetch out lost this and quietly added a
/// server round trip to every play from a detail page.
///
/// The hit test is the PAIR `(sid, rk)`, and this is the site where a bare key is most dangerous:
/// it SKIPS the server fetch, so a collision here silently substitutes the loaded page's item for
/// the one about to play — its audio/subtitle `Stream.id`s (then PUT to the other server),
/// its frame size (so the direct-play gate reasons about the wrong resolution), its fps, chapters
/// and markers. A miss costs one round trip; a false hit costs the whole playback.
///
/// This closed a TODO that stood here through the foundation commits: `Detail` had no server, so
/// the filter was the rk alone and the parameter was deliberately unused. `Detail.sid` is what
/// made the pair test possible.
fn cached_playing(state: &MetadataState, sid: crate::registry::ServerId, rk: &str) -> Option<PlayingItem> {
    current(state)
        .filter(|d| crate::registry::same_item((d.sid, &d.rk), (sid, rk)) && !d.audio.is_empty())
        .map(|d| PlayingItem {
            sid,
            rk: rk.to_string(),
            container: d.container.clone(),
            show_rk: d.show_rk.clone(),
            audio: d.audio.clone(),
            subs: d.subs.clone(),
            video_fps: d.video_fps,
            width: d.width,
            height: d.height,
            bitrate: d.bitrate,
            dovi: d.dovi,
            markers: d.markers.clone(),
            chapters: d.chapters.clone(),
            blur: d.has_blur.then_some(d.blur),
        })
}

/// `sid` names the server `rk` is a key on. It runs on the resolve worker, so the server must
/// arrive by value: `client_opt()` here would fetch whichever server is CURRENT, and a ratingKey
/// that also exists there would come back with a different film's stream list.
pub(crate) fn fetch_playing_item(sid: crate::registry::ServerId, rk: &str) -> Option<PlayingItem> {
    if rk.is_empty() {
        return None;
    }
    // Jellyfin arm — same store, the other server (`jellyfin::detail::playing_item` builds the
    // same PlayingItem off /Items + /MediaSegments). The installed-client guard is every
    // jellyfin arm's guard: slot 0 is also a valid Plex slot in this build.
    if sid == crate::jellyfin::SERVER_ID {
        if let Some(c) = crate::jellyfin::client() {
            return crate::jellyfin::detail::playing_item(c, sid, rk);
        }
    }
    // The Plex arm (`client_for(sid)` + `Client::metadata`) went with the wire layer (#166): no
    // slot other than the Jellyfin one can hold a playing item, and the Jellyfin slot is fully
    // answered above. A miss here is the honest None the store treats as "no item".
    None
}

/// Retire BOTH descriptions of the item that was playing, together.
///
/// They have to move as one. `NOW` feeds the HUD caption and Info card; `PLAYING` feeds the track
/// menu and — since markers landed here — the skip/Up Next controls. Clearing only `NOW` (which is
/// what each play path used to do by hand) leaves the FINISHED episode's markers live for the whole
/// resolve + pre-roll of the next one, and a `final` credits marker is deliberately open-ended to
/// `i64::MAX`, so a stale one matches any playhead: the new episode would offer to skip its own
/// credits seconds after starting. Nothing fires today, but only by incidental ordering — this
/// makes it a contract instead.
fn retire_playing(state: &mut MetadataState) {
    set_now_playing(state, None);
    retire_playing_item(state);
}

/// Retire ONLY the track/marker/chapter store, leaving the `NowPlaying` caption alone — what a NEW
/// play REQUEST does at its start ([`crate::route::request_play`]), beside the same retirement of
/// `UP_NEXT`.
///
/// The caption cannot be retired there because `detail::play_episode_at` sets it just BEFORE
/// requesting the play. The store must be, though: it is the PREVIOUS leaf's for the whole resolve
/// window (0.5-3 s, longer through a `/decision` handshake) and the HUD is up for all of it. With
/// chapters in here that became user-reachable — the transport advertised a Chapters tab whose OK
/// seeked the NEW episode to some other item's offset.
fn retire_playing_item(state: &mut MetadataState) {
    state.playing = None;
    state.skipped.clear();
}

/// MAIN THREAD: install a fetched playing-item store.
fn install_playing(state: &mut MetadataState, pt: Option<PlayingItem>) {
    state.skipped.clear(); // a different leaf's markers, so a fresh slate
    if let Some(pt) = &pt {
        crate::player::log(&format!(
            "playing item: rk={} audio={} subs={} markers={} chapters={}",
            pt.rk,
            pt.audio.len(),
            pt.subs.len(),
            pt.markers.len(),
            pt.chapters.len()
        ));
    }
    state.playing = pt;
}

// ---- list-position → demuxer-ordinal conversion --------------------------------------------
// The demuxer selects "the Nth stream of its type" in CONTAINER order; the menu/metadata lists are
// in the server document order. These convert a list position to that container ordinal by sorting
// on the server `Stream.index` (stable tie-break on list position, so an index-less response
// degrades to document order — the previous behavior).

/// Container-audio ordinal of `audio[i]` — what `player::set_audio_track`/`request_audio_track`
/// (→ ff's nth_audio_stream) consume.
pub(crate) fn audio_ordinal(audio: &[Stream], i: usize) -> i32 {
    if i >= audio.len() {
        return i as i32;
    }
    let me = (audio[i].index, i);
    audio
        .iter()
        .enumerate()
        .filter(|(j, s)| (s.index, *j) < me)
        .count() as i32
}

/// The external subtitle the SERVER already has selected for this part and that this client can
/// draw itself — the one `player::sidecar::restore_server_selection` switches on at a direct-play
/// landing. ONE definition, because two readers must agree on it: the restore that turns it on,
/// and `route::renders_subtitle`, which withholds the Plex Pass audio enhancement (issue #266 I6)
/// whenever a subtitle will be on screen. A route that offered the enhancement for a part whose
/// sidecar the restore then showed would pair a remux (no client-drawn sidecar on a transcode)
/// with a subtitle the viewer was promised.
pub(crate) fn server_selected_sidecar(item: &PlayingItem) -> Option<&Stream> {
    item.subs.iter().find(|s| s.selected && s.sidecar_renderable())
}

/// Container ordinal of `subs[i]` among the EMBEDDED subtitle streams (all ff.rs enumerates —
/// sidecars are not in the container), or -1 when `subs[i]` is itself external (nothing to
/// client-render on direct-play; only a server transcode can burn it).
pub(crate) fn sub_render_ordinal(subs: &[Stream], i: usize) -> i32 {
    let s0 = match subs.get(i) {
        Some(s) if !s.external => s,
        _ => return -1,
    };
    let me = (s0.index, i);
    subs.iter()
        .enumerate()
        .filter(|(j, s)| !s.external && (s.index, *j) < me)
        .count() as i32
}

// ---- language matching (moved from `route::plan`, §1) -------------------------------------
// `route::plan`'s audio/subtitle preference ladder and `metadata::sub_layout`'s "yours"/"Other
// languages" grouping both need to know whether two language tags name the same language, so
// this lives in the data layer both sides already name rather than in either one's own module.

/// ISO 639-1 / 639-2 bibliographic and terminology spellings from Debian iso-codes:
/// https://salsa.debian.org/iso-codes-team/iso-codes/-/raw/main/data/iso_639-2.json
/// (retrieved 2026-09-27). Plex preferences use 639-1; the server commonly uses either 639-2 form.
const LANG_SPELLINGS: &[&[&str]] = &[
    &["aa", "aar"],
    &["ab", "abk"],
    &["af", "afr"],
    &["ak", "aka"],
    &["am", "amh"],
    &["ar", "ara"],
    &["an", "arg"],
    &["as", "asm"],
    &["av", "ava"],
    &["ae", "ave"],
    &["ay", "aym"],
    &["az", "aze"],
    &["ba", "bak"],
    &["bm", "bam"],
    &["be", "bel"],
    &["bn", "ben"],
    &["bi", "bis"],
    &["bo", "bod", "tib"],
    &["bs", "bos"],
    &["br", "bre"],
    &["bg", "bul"],
    &["ca", "cat"],
    &["cs", "ces", "cze"],
    &["ch", "cha"],
    &["ce", "che"],
    &["cu", "chu"],
    &["cv", "chv"],
    &["kw", "cor"],
    &["co", "cos"],
    &["cr", "cre"],
    &["cy", "cym", "wel"],
    &["da", "dan"],
    &["de", "deu", "ger"],
    &["dv", "div"],
    &["dz", "dzo"],
    &["el", "ell", "gre"],
    &["en", "eng"],
    &["eo", "epo"],
    &["et", "est"],
    &["eu", "eus", "baq"],
    &["ee", "ewe"],
    &["fo", "fao"],
    &["fa", "fas", "per"],
    &["fj", "fij"],
    &["fi", "fin"],
    &["fr", "fra", "fre"],
    &["fy", "fry"],
    &["ff", "ful"],
    &["gd", "gla"],
    &["ga", "gle"],
    &["gl", "glg"],
    &["gv", "glv"],
    &["gn", "grn"],
    &["gu", "guj"],
    &["ht", "hat"],
    &["ha", "hau"],
    &["he", "heb"],
    &["hz", "her"],
    &["hi", "hin"],
    &["ho", "hmo"],
    &["hr", "hrv", "scr"],
    &["hu", "hun"],
    &["hy", "hye", "arm"],
    &["ig", "ibo"],
    &["io", "ido"],
    &["ii", "iii"],
    &["iu", "iku"],
    &["ie", "ile"],
    &["ia", "ina"],
    &["id", "ind"],
    &["ik", "ipk"],
    &["is", "isl", "ice"],
    &["it", "ita"],
    &["jv", "jav"],
    &["ja", "jpn"],
    &["kl", "kal"],
    &["kn", "kan"],
    &["ks", "kas"],
    &["ka", "kat", "geo"],
    &["kr", "kau"],
    &["kk", "kaz"],
    &["km", "khm"],
    &["ki", "kik"],
    &["rw", "kin"],
    &["ky", "kir"],
    &["kv", "kom"],
    &["kg", "kon"],
    &["ko", "kor"],
    &["kj", "kua"],
    &["ku", "kur"],
    &["lo", "lao"],
    &["la", "lat"],
    &["lv", "lav"],
    &["li", "lim"],
    &["ln", "lin"],
    &["lt", "lit"],
    &["lb", "ltz"],
    &["lu", "lub"],
    &["lg", "lug"],
    &["mh", "mah"],
    &["ml", "mal"],
    &["mr", "mar"],
    &["mk", "mkd", "mac"],
    &["mg", "mlg"],
    &["mt", "mlt"],
    &["mn", "mon"],
    &["mi", "mri", "mao"],
    &["ms", "msa", "may"],
    &["my", "mya", "bur"],
    &["na", "nau"],
    &["nv", "nav"],
    &["nr", "nbl"],
    &["nd", "nde"],
    &["ng", "ndo"],
    &["ne", "nep"],
    &["nl", "nld", "dut"],
    &["nn", "nno"],
    &["nb", "nob"],
    &["no", "nor", "nb", "nob"],
    &["ny", "nya"],
    &["oc", "oci"],
    &["oj", "oji"],
    &["or", "ori"],
    &["om", "orm"],
    &["os", "oss"],
    &["pa", "pan"],
    &["pi", "pli"],
    &["pl", "pol"],
    &["pt", "por"],
    &["ps", "pus"],
    &["qu", "que"],
    &["rm", "roh"],
    &["ro", "ron", "rum"],
    &["rn", "run"],
    &["ru", "rus"],
    &["sg", "sag"],
    &["sa", "san"],
    &["si", "sin"],
    &["sk", "slk", "slo"],
    &["sl", "slv"],
    &["se", "sme"],
    &["sm", "smo"],
    &["sn", "sna"],
    &["sd", "snd"],
    &["so", "som"],
    &["st", "sot"],
    &["es", "spa"],
    &["sq", "sqi", "alb"],
    &["sc", "srd"],
    &["sr", "srp"],
    &["ss", "ssw"],
    &["su", "sun"],
    &["sw", "swa"],
    &["sv", "swe"],
    &["ty", "tah"],
    &["ta", "tam"],
    &["tt", "tat"],
    &["te", "tel"],
    &["tg", "tgk"],
    &["tl", "tgl"],
    &["th", "tha"],
    &["ti", "tir"],
    &["to", "ton"],
    &["tn", "tsn"],
    &["ts", "tso"],
    &["tk", "tuk"],
    &["tr", "tur"],
    &["tw", "twi"],
    &["ug", "uig"],
    &["uk", "ukr"],
    &["ur", "urd"],
    &["uz", "uzb"],
    &["ve", "ven"],
    &["vi", "vie"],
    &["vo", "vol"],
    &["wa", "wln"],
    &["wo", "wol"],
    &["xh", "xho"],
    &["yi", "yid"],
    &["yo", "yor"],
    &["za", "zha"],
    &["zh", "zho", "chi"],
    &["zu", "zul"],
];

/// **The one canonical spelling of a language tag's LANGUAGE**, the key [`lang_matches`] compares
/// by: the primary subtag, lower-cased, folded onto the first spelling of its [`LANG_SPELLINGS`]
/// row(s) when it has one (`"fr-CA"`, `"fre"` and `"fra"` all answer `"fr"`), or itself when it
/// has none. `None` for an empty tag, which names no language and so matches nothing. A caller that
/// groups many tracks computes this ONCE per track and buckets by it, rather than asking
/// `lang_matches` pairwise.
pub(crate) fn lang_key(tag: &str) -> Option<std::borrow::Cow<'static, str>> {
    let primary = tag.trim().split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    if primary.is_empty() {
        return None;
    }
    // The smallest spelling over EVERY row that lists it, not the first row's head: Norwegian is
    // listed both as `nb`/`nob` and as `no`/`nor`/`nb`/`nob`, and all four must share one key.
    let canonical = LANG_SPELLINGS
        .iter()
        .filter(|spellings| spellings.contains(&primary.as_str()))
        .flat_map(|spellings| spellings.iter().copied())
        .min();
    Some(match canonical {
        Some(spelling) => std::borrow::Cow::Borrowed(spelling),
        None => std::borrow::Cow::Owned(primary),
    })
}

/// Do two language tags name the same language? Either side may be a Plex preference
/// (`"hu-HU"`, `"pt-BR"`) or a stream's ISO-639-2 `languageCode` (`"hun"`, `"ger"`/`"deu"`), so the
/// same test serves a preference against a stream AND a picked stream against its siblings (a
/// `fre` pick and a `fra` sibling are one language). Only the primary subtag counts — a stream
/// says "Portuguese", never "Brazilian". An empty tag matches nothing. Same [`lang_key`] = same
/// language.
pub(crate) fn lang_matches(a: &str, b: &str) -> bool {
    matches!((lang_key(a), lang_key(b)), (Some(a), Some(b)) if a == b)
}

#[cfg(test)]
mod lang_matches_tests {
    use super::lang_matches;

    #[test]
    fn a_regional_bcp47_tag_matches_the_iso_639_2_bibliographic_and_terminology_forms() {
        assert!(lang_matches("fr-CA", "fra"));
        assert!(lang_matches("fr-CA", "fre"));
        assert!(!lang_matches("fr-CA", "eng"));
    }

    /// **`lang_key` agrees with the pairwise row rule it replaced** — two spellings share a key
    /// exactly when some [`LANG_SPELLINGS`](super::LANG_SPELLINGS) row lists both (Norwegian's
    /// overlapping rows included).
    #[test]
    fn lang_key_agrees_with_the_pairwise_row_rule() {
        let rows = super::LANG_SPELLINGS;
        // every pair a row lists is one language…
        for row in rows {
            for a in row.iter() {
                for b in row.iter() {
                    assert!(lang_matches(a, b), "{a} vs {b}");
                }
            }
        }
        // …and two rows that share no spelling are two
        for r1 in rows {
            for r2 in rows {
                if !r1.iter().any(|a| r2.contains(a)) {
                    assert!(!lang_matches(r1[0], r2[0]), "{} vs {}", r1[0], r2[0]);
                }
            }
        }
        assert_eq!(super::lang_key("fre").as_deref(), Some("fr"));
        assert_eq!(super::lang_key("xx-YY").as_deref(), Some("xx"));
        assert_eq!(super::lang_key("  "), None);
    }
}

fn fetch_seasons(sid: crate::registry::ServerId, rk: &str) -> Vec<Season> {
    // The store's own season list, beside fetch_full's: the same data through the same client —
    // without this arm the tab strip read "0 seasons" off the empty Plex registry and the show
    // rendered season 1 with no way to reach the rest (TV-reported).
    // Jellyfin is the only backend: the store's own season list, beside fetch_full's.
    if sid != crate::jellyfin::SERVER_ID {
        return Vec::new();
    }
    crate::jellyfin::client()
        .map(|c| c.seasons(rk)
            .map(|r| r.items.iter().map(crate::jellyfin::detail::season_from_dto).collect())
            .unwrap_or_default())
        .unwrap_or_default()
}

/// A season's episode list, or **None when the `/children` GET failed**. The failure has to stay
/// distinguishable from a genuinely empty season all the way to the pump: returning an empty Vec
/// for both is what let one transient GET blank a populated episode row, with no spinner, no error
/// and no way to ask the tab again. Same rule browse.rs's page fetch carries with its `total < 0`
/// sentinel ("a wiped-to-empty store here was a review-confirmed bug").
///
/// NB its sibling `fetch_seasons` deliberately KEEPS the degrade-to-empty: both are only ever
/// called from `fetch_full`, which builds a Detail from nothing — there is no previous list there
/// to protect, and neither is worth failing the whole page over.
fn fetch_episodes(sid: crate::registry::ServerId, series_rk: &str, season_rk: &str) -> Option<Vec<Episode>> {
    // Jellyfin addresses a season's episodes by BOTH ids (`/Shows/{series}/Episodes?SeasonId=`),
    // where Plex's `/children` needs the season key alone — that asymmetry is the whole reason
    // this signature carries `series_rk`. Every caller has it in hand (the loaded show's rk).
    // Jellyfin is the only backend, and it addresses a season's episodes by BOTH ids.
    if sid != crate::jellyfin::SERVER_ID {
        return None;
    }
    crate::jellyfin::client()
        .and_then(|c| crate::jellyfin::detail::fetch_episodes(c, series_rk, season_rk))
}

/// The full detail fetch for `rk` (movie or show): the backend's own full record (item, cast,
/// streams, and — for shows — on-deck, seasons, episodes and the hero episode's stream backfill),
/// then the store's own seasons/episodes lists beside it.
///
/// PURE NETWORK + PARSING — it touches no `static mut`, which is what let it run either on the
/// main thread or on a worker ([`request_detail`]) while a synchronous caller still existed
/// (phase 12/D7 deleted the last one, `load_detail_now`; see that deletion's note below). Keep it
/// that way: installing the result is the caller's job, and on the async path that must happen on
/// the main thread (see the `DETAIL_LANDING`/`land_detail` note).
fn fetch_full(sid: crate::registry::ServerId, rk: &str) -> Option<Detail> {
    // `ms=` is the whole chain's wall clock. It is the exact cost `request_detail` moves off the
    // SDL loop, so it is the number to read when judging whether a call site can afford to block
    // — note the framedrop breakdown CANNOT show it (fd_pc0 starts after event handling).
    let t0 = std::time::Instant::now();
    // The ONE line that says a detail page was asked for and got nothing — the summary at the end
    // of this function prints on success only, and nothing downstream can speak for the failure.
    // Its worst shape is the page opened through `detail::open_rk`, which clears before requesting:
    // `current()` is then None, `detail_loading()` settles the moment this lands so the spinner
    // GOES, and the page sits on the catalog row's hero art with an empty body that nothing
    // re-requests. The other request sites keep whatever was loaded, so they degrade more quietly —
    // but all four are equally silent in the log without this, and byte-identical to the user
    // never having pressed OK.
    //
    // Worded as "no metadata" rather than "the GET failed": `fetch_detail` refuses a non-Jellyfin
    // server id before any request, and an uninstalled client lands as `None` too, so this arm is
    // also taken when no request was ever issued. One line for both is right — the page is equally
    // empty either way — but it must not assert a round trip that may not have happened.
    let Some(mut d) = fetch_detail(sid, rk) else {
        crate::log(&format!(
            "detail: rk={rk} sid={sid:?} — no metadata (server unresolved, or it refused)"
        ));
        return None;
    };
    if d.is_show {
        d.seasons = fetch_seasons(sid, rk);
        if let Some(s0) = d.seasons.first() {
            // a first-season failure is not worth failing the whole page over — the hero, cast
            // and Related still load, and there is no previous list here to protect. It is still
            // named, because the `eps=` below cannot tell it from a season with no episodes.
            d.episodes = fetch_episodes(sid, rk, &s0.rk).unwrap_or_else(|| {
                crate::log(&format!(
                    "detail: rk={rk} season rk={} /children did not answer — the eps= below is that refusal",
                    s0.rk));
                Vec::new()
            });
        }
    }
    // The item's IDENTITY and the SHAPE of what came back — never its title. `scrub_local` runs
    // on every line in every build, but nothing in a line distinguishes a programme title from
    // ordinary prose (`diag::scrub`'s `a_bare_quoted_title_is_explicitly_out_of_scope_for_the_scrubber`),
    // so the only mechanism for viewing content is that no call site writes it. This one did,
    // from the day it was added until phase 11 — `'{}'` with `d.title` in it, on every detail
    // open, in a log the maintainer routinely pastes into a public issue.
    crate::player::log(&format!(
        "detail: sid={} rk={} show={} genres={} cast={} crew={} seasons={} eps={} related={} audio={} subs={} ms={}",
        d.sid.raw(), d.rk, d.is_show, d.genres.len(), d.cast.len(), d.crew.len(), d.seasons.len(), d.episodes.len(),
        d.related.len(), d.audio.len(), d.subs.len(), t0.elapsed().as_millis()
    ));
    Some(d)
}

// `load_detail_now` — `request_detail` but BLOCKING, `pub(crate) fn load_detail_now(sid, rk)` —
// was deleted here (phase 12/D7). Its doc named three callers that read `current()` on the NEXT
// statement (`open_rk_season`, `home_activate`'s play-a-show arm, the headless `butaca-play`/
// `butaca-detail` triggers), and by df3520e1 every one of those had already migrated away from
// it without this doc noticing: `home_activate` itself was retired with the legacy `ui::home`
// module (`app/input.rs`'s own note on the extraction that produced `activate_card`), and neither
// `open_rk_season` nor the headless triggers named it either — `grep -rn load_detail_now` found
// exactly one call site left, `stores/metadata.rs`'s `MetadataCmd::LoadDetailNow` arm, itself
// called only by `app/input.rs`'s `activate_card`'s show/season Play. That conversion (D7: the
// press now issues `RequestDetail` and defers play-vs-open to a landing-driven continuation,
// `app::input::menu_play_tick`) removed the last caller, so the function — and the `MetadataCmd`
// variant that wrapped it — are gone rather than left as unreachable dead code.

// ---- async detail load ---------------------------------------------------------------------
// Opening a detail page used to block the SDL loop on 2 (movie) to 5 (show) sequential server
// round-trips, straight off the key handler. `request_detail` spawns the fetch and `pump_detail`
// installs the result — the page mounts THIS frame on the catalog row's art/title/summary and
// fills in a beat later. Same shape as the season mailbox below and route.rs's play resolve.
//
// The worker MUST NOT write CURRENT. `current()` (via `MetadataView`) hands out a `&'a Detail`,
// borrowed from the owner, that ~25 draw sites read within a frame, so a background store would
// drop the old `Detail` under a live reference — a use-after-free, not a lint. Keeping the main
// thread the sole writer is precisely what makes that borrow sound, so the worker's only output
// is the mailbox.
type DetailKey = (crate::registry::ServerId, String);

/// The addressed request's status: None means another item (or no request), true means
/// in flight, false means the matching request settled, including failure/refusal.
fn detail_request_status(adapter: &MetadataAdapter, sid: crate::registry::ServerId, rk: &str) -> Option<bool> {
    let want = adapter.detail_want.lock().unwrap_or_else(|e| e.into_inner());
    want.as_ref().filter(|(wanted_sid, wanted_rk)| *wanted_sid == sid && wanted_rk == rk)
        .map(|_| detail_loading(adapter))
}

/// Generation of the most recently admitted detail request (bumped once, synchronously, by
/// [`begin_detail_request`]). A bare `Option<bool>` from [`detail_request_status`] answers
/// "settled?" with no identity — it cannot distinguish "MY request settled" from "an older
/// request for the same address settled". Callers that need to pin an obligation to their own
/// request read this generation before admission and require a later read to be strictly newer.
fn detail_generation(adapter: &MetadataAdapter) -> u32 {
    adapter.detail_gen.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(test)]
pub(crate) fn begin_detail_for_test(adapter: &MetadataAdapter, sid: crate::registry::ServerId, rk: &str) -> u32 {
    crate::testlock::assert_held("the detail store (begin_detail_for_test)");
    let (gen, _, admission) = begin_detail_request(adapter, sid, rk);
    admission.expect("synthetic detail request must have a reserved completion");
    gen
}

#[cfg(test)]
pub(crate) fn detail_generation_for_test(adapter: &MetadataAdapter) -> u32 {
    adapter.detail_gen.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(test)]
pub(crate) fn land_detail_for_test(state: &mut MetadataState, adapter: &std::sync::Arc<MetadataAdapter>, sid: crate::registry::ServerId, rk: &str, gen: u32, detail: Option<Detail>) -> bool {
    crate::testlock::assert_held("the detail store (land_detail_for_test)");
    land_detail(adapter, sid, rk, gen, detail);
    pump_detail(state, adapter)
}

fn detail_addr(gen: u32) -> crate::ui::machine::Addr {
    crate::ui::machine::Addr {
        to: crate::ui::machine::MachineId::Store(crate::stores::StoreId::Metadata.ord()),
        req: crate::ui::machine::RequestId(gen),
    }
}

/// Invalidate any in-flight/pending detail fetch and mark the mailbox settled: bump the
/// generation (so a late landing is discarded by `pump_detail`), catch DETAIL_DONE up to it
/// (`detail_loading()` → false), and clear the landing. Returns the fresh generation.
fn supersede_detail(adapter: &MetadataAdapter) -> u32 {
    use std::sync::atomic::Ordering;
    let gen = adapter.detail_gen.fetch_add(1, Ordering::SeqCst) + 1;
    adapter.detail_done.store(gen, Ordering::SeqCst);
    record::cancel_all(adapter);
    *adapter.detail_want.lock().unwrap_or_else(|e| e.into_inner()) = None;
    gen
}

/// Post a finished fetch to the landing, addressed by the generation the request minted and
/// keyed by the item it fetched. An older fetch landing late is refused by [`pump_detail`]'s
/// generation check, so ordering in the queue is never what protects a newer result. Called from
/// the worker (and from the tests, which is the point of it being a named function).
fn land_detail(adapter: &MetadataAdapter, sid: crate::registry::ServerId, rk: &str, gen: u32, d: Option<Detail>) {
    let addr = detail_addr(gen);
    // Full has already queued one Dropped terminal. Unknown/duplicate results queue nothing;
    // cancelled worker completions acknowledge only their own retained reservation.
    record::put(adapter, addr, (sid, rk.to_string()), d);
}

/// Mint the request: supersede the season, bump the generation, record what the page awaits
/// and admit the request. The spawn is the caller's; a refused one is `refused` back.
fn begin_detail_request(adapter: &MetadataAdapter, sid: crate::registry::ServerId, rk: &str) -> (
    u32, crate::ui::machine::Addr, Result<(), crate::ui::landing::AdmissionError>,
) {
    use std::sync::atomic::Ordering;
    // drop any season fetch in flight for the OLD item — its landing would patch the new one
    supersede_season(adapter);
    // NOT supersede_detail(): the generation must move (a stale landing is discarded) but
    // DETAIL_DONE must stay behind so `detail_loading()` reports this fetch as in flight
    let gen = adapter.detail_gen.fetch_add(1, Ordering::SeqCst) + 1;
    record::cancel_all(adapter);
    #[cfg(test)]
    adapter.run_superseded_detail_fetches();
    *adapter.detail_want.lock().unwrap_or_else(|e| e.into_inner()) = Some((sid, rk.to_string()));
    let addr = detail_addr(gen);
    let admission = record::admit(adapter, addr);
    if admission.is_err() {
        // Rejected admission owns no queued terminal: settle this new generation synchronously.
        // clear() cancelled previous workers but kept their reservations until acknowledgement.
        adapter.detail_done.store(gen, Ordering::SeqCst);
        crate::log(&format!("detail: request rk={rk} REFUSED — {} in flight", adapter.detail_landing_ref().inflight(addr.to)));
    }
    (gen, addr, admission)
}

/// MAIN THREAD, NON-BLOCKING. Supersedes any in-flight load and spawns the fetch; the result
/// lands via [`pump_detail`]. The caller mounts the detail page this same frame.
///
/// `sid` names the server to ask and is captured by the CALLER, on the main thread — the worker
/// must not read the current server (see the fetch block's note), and the page being opened may
/// belong to a machine that is not the current one at all.
fn request_detail(adapter: &std::sync::Arc<MetadataAdapter>, sid: crate::registry::ServerId, rk: &str) {
    request_detail_with_spawn(adapter, sid, rk, |gen| {
        let rk = rk.to_string();
        let adapter = std::sync::Arc::clone(adapter);
        // The recorded client identity mirrors the discovery/hubs vocabulary: the Jellyfin
        // source's single client is instance 0 (it has no registry slot behind it), and every
        // other source carries its Plex registry instance as before. `null` would record a
        // resource the replay validator can never match.
        // The recorded client identity: Jellyfin's single client is instance 0; any other
        // source has no registry slot behind it any more, which the recorder spells null.
        let client = if sid == crate::jellyfin::SERVER_ID {
            serde_json::json!(0)
        } else {
            serde_json::Value::Null
        };
        crate::app::bootstrap::stores::admit(serde_json::json!({"store":"metadata",
            "sid":sid.raw(),"rk":rk,"gen":gen,
            "client":client}), || {
            // See `MetadataAdapter::run_held_detail_fetches_for_test`: a test runs the fetch
            // when it chooses, never on a thread racing its assertions.
            #[cfg(test)]
            {
                adapter.held_detail.lock().unwrap_or_else(|e| e.into_inner()).push((sid, rk, gen));
                true
            }
            #[cfg(not(test))]
            crate::task::spawn_small("detail", move || {
                finish_detail_fetch(&adapter, sid, &rk, gen, || fetch_full(sid, &rk));
            })
        })
    });
}

/// Shared admission/spawn path; tests inject a spawn outcome without starting network workers.
fn request_detail_with_spawn(adapter: &MetadataAdapter, sid: crate::registry::ServerId, rk: &str, spawn: impl FnOnce(u32) -> bool) {
    let (gen, addr, admission) = begin_detail_request(adapter, sid, rk);
    if admission.is_err() { return; }
    if !spawn(gen) {
        // no worker means nothing will ever land on its own: the refusal record is what settles
        // the spinner (`pump_detail`), exactly one event for the request (§5.2)
        record::refused(adapter, addr);
    }
}

fn finish_detail_fetch(
    adapter: &MetadataAdapter, sid: crate::registry::ServerId, rk: &str, gen: u32,
    fetch: impl FnOnce() -> Option<Detail> + std::panic::UnwindSafe,
) {
    // Publish outside the catch so fetch failure/unwind still acknowledges this reservation.
    let d = catch_unwind(fetch).unwrap_or(None);
    land_detail(adapter, sid, rk, gen, d);
}

/// `stores::metadata`'s one door onto every [`MetadataCmd`](crate::stores::metadata::MetadataCmd)
/// variant (D3): the match used to live in `stores/metadata.rs::run`, calling `pub(crate)`
/// mutators across the module boundary. Relocating it here is what lets those go private. The
/// `#[cfg(test)]` arm (`AltInstall`) exists only so tests can seed the
/// alt-sources store the way a landed resolve or a facts-epoch move would — production reaches
/// both through `pump_alt_sources`, a same-file call, never as a dispatched `Cmd`.
pub(crate) fn run(state: &mut MetadataState, adapter: &std::sync::Arc<MetadataAdapter>, cmd: crate::stores::metadata::MetadataCmd) -> bool {
    use crate::stores::metadata::MetadataCmd;
    match cmd {
        MetadataCmd::RequestDetail { sid, rk } => {
            request_detail(adapter, sid, &rk);
            true
        }
        MetadataCmd::Clear => {
            clear(state, adapter);
            true
        }
        MetadataCmd::Reset => {
            reset(state, adapter);
            true
        }
        MetadataCmd::LoadSeason(i) => {
            load_season(state, adapter, i);
            true
        }
        MetadataCmd::LoadSeasonNow(i) => {
            load_season_now(state, adapter, i);
            true
        }
        MetadataCmd::SetNowPlaying(np) => {
            set_now_playing(state, np);
            true
        }
        MetadataCmd::SetWatchedLocal { sid, rk, on } => set_watched_local(state, sid, &rk, on),
        MetadataCmd::SetFavoriteLocal { sid, rk, on } => set_favorite_local(state, sid, &rk, on),
        MetadataCmd::InstallPlaying(p) => {
            install_playing(state, p);
            true
        }
        MetadataCmd::MarkSkipped(m) => {
            mark_skipped(state, m);
            true
        }
        MetadataCmd::RetirePlaying => {
            retire_playing(state);
            true
        }
        MetadataCmd::RetirePlayingItem => {
            retire_playing_item(state);
            true
        }
        #[cfg(test)]
        MetadataCmd::AltInstall { sid, rk, copies } => alt_install(state, sid, &rk, copies),
    }
}

/// MAIN THREAD, once a frame, ROUTE-UNCONDITIONAL (a landing must never depend on which screen is
/// mounted — the play paths request a detail from Home and flip straight to the player). Installs
/// a landed fetch into CURRENT and returns true when a fresh item was published. A stale landing —
/// superseded by a newer request, by a blocking load, or by `clear()` when the page closed — is
/// dropped.
pub(crate) fn pump_detail_with_gate(state: &mut MetadataState, adapter: &std::sync::Arc<MetadataAdapter>,
    gate: &crate::ui::landgate::Gate) -> bool {
    use crate::ui::landing::Lane;
    use std::sync::atomic::Ordering;
    let want = adapter.detail_want.lock().unwrap_or_else(|e| e.into_inner()).clone();
    // Under a replay this drains on the frame the recording drained it on (§3.3 step 3,
    // `ui::landgate`); off one it is the same call. The gate wraps the QUEUE drain and not the
    // supersede/install below, so a held frame leaves the record in the landing untouched.
    let out = if crate::app::bootstrap::stores::active() {
        crate::stores::take_landings(gate, crate::stores::StoreId::Metadata, || {
            crate::app::bootstrap::stores::poll_apply("metadata", 0,
                || record::drain_live(adapter, &want), |replies| record::supply(adapter, replies, &want))
                .into_iter().collect::<Vec<_>>()
        }).into_iter().flat_map(|drain| drain.landed).collect()
    } else {
        crate::stores::take_landings(gate, crate::stores::StoreId::Metadata, || {
            let mut out = Vec::new();
            adapter.detail_landing_ref().take_for(&|_| true, &|key| want.as_ref().is_none_or(|wanted| key == wanted), &mut out);
            out
        })
    };
    let mut fresh = false;
    for rec in out {
        let gen = rec.addr.req.0;
        if gen != adapter.detail_gen.load(Ordering::SeqCst) {
            continue; // superseded while in flight
        }
        adapter.detail_done.store(gen, Ordering::SeqCst);
        let d = match rec.lane {
            Lane::Data(_, d) => d,
            // nothing arrived and nothing will: the spinner is settled, the page keeps its item
            Lane::Dropped(_) | Lane::Refused(_) => None,
        };
        fresh |= install_landed_detail(state, adapter, d);
    }
    fresh
}

#[cfg(test)]
pub(crate) fn pump_detail(state: &mut MetadataState,
    adapter: &std::sync::Arc<MetadataAdapter>) -> bool {
    pump_detail_with_gate(state, adapter, crate::ui::landgate::fixture_gate())
}

/// Install a landed fetch: a `None` (the fetch failed or panicked) keeps the previously loaded
/// item. The landing carries no title of its own, so the line naming WHICH page it was is
/// written at the fetch site (`fetch_full`) — read it there rather than adding an anonymous one
/// here. ONE arrival is not covered by that: a worker that PANICKED lands `None` too, and never
/// reached `fetch_full`'s line. `task`'s panic logger names the thread and the source location,
/// so the failure is in the log — it just is not in these words, and a `None` here with no
/// `detail:` line above it is that case.
fn install_landed_detail(state: &mut MetadataState, adapter: &std::sync::Arc<MetadataAdapter>, d: Option<Detail>) -> bool {
    let Some(d) = d else { return false };
    // The LANDING is what defines which show's episodes are current, so the season supersede has
    // to happen here as well as at request time: a tab hop issued while this load was in flight
    // spawned a fetch against the OLD item, and its landing would patch these fresh episodes.
    supersede_season(adapter);
    // Ask the other sources about this item BEFORE the move: the resolve needs the item's own
    // server and its portable guid, and this is the one place both are known on the main thread.
    // A page with no guid, or a one-server install, spawns nothing.
    request_alt_sources(state, adapter, d.sid, &d.rk, &d.guid);
    state.current = Some(d);
    // if this load is a playing leaf (episode/movie), refresh the Info card's descriptor from it
    sync_now_playing(state);
    true
}

// ---- "Also available": the same film on the OTHER sources ------------------------------------
//
// **The whole chain lives here**: the copy record, the ADDRESSED store, the cross-source resolve
// that fills it, the per-frame landing and the headless stand-in. Until restructure phase 10 the
// store half sat in `ui/alt_sources.rs` beside the panel that drew it, which put a page's DATA
// inside a screen — so the panel's conversion to a `ModalStack` surface had nowhere to leave it,
// and the Detail page had to ask a POPOVER whether one of its own buttons should be drawn.
//
// The cross-source resolve is in the mailbox shape the detail fetch uses and for the same reason: it
// is one round trip PER REGISTERED SOURCE, and doing it on the SDL loop would park the frame for a
// `connect(2)` timeout per unreachable share.
//
// It runs off the back of a landed detail rather than beside it, because it needs that detail's
// `guid` — which only the fetch can supply — and because a page with no guid (a server that sent
// none) must cost nothing at all.
struct AltResult {
    gen: u32,
    roster_gen: u32,
    /// The SERVER the resolve was asked for — the other half of `rk`, and carried for exactly the
    /// reason [`SeasonResult::sid`] is. A `ratingKey` is a server-local integer dense from 1,
    /// so leaving OUR film 4 and opening the SHARE's film 4 while a
    /// resolve is out passes an rk-only test — and the generation guard cannot see that hop either,
    /// since it only moves when a DETAIL lands and the new page's is still in flight. The panel
    /// would then list the other machine's copies, and OK on one would open a different film.
    sid: crate::registry::ServerId,
    /// The rk the resolve was asked FOR, carried so the landing can be matched against the page
    /// that is mounted now — the ADDRESSED store files it under that pair and a reader on another page
    /// sees nothing.
    rk: String,
    list: Vec<AltCopy>,
}

/// One copy of the item on ONE source — everything an *Also available* row needs, and nothing
/// about layout.
///
/// Built by [`resolve_alt_sources`], which asks every registered source for the item's `guid`;
/// `screens::alt_sources` only orders, marks and draws them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AltCopy {
    /// The registry slot the copy lives on ([`crate::server::servers`]). It is the row's IDENTITY —
    /// the gate counts distinct values of it, and OK resolves the destination client through it —
    /// so a copy whose source is not registered carries [`crate::registry::ServerId::UNSET`] and can be
    /// listed but never navigated to.
    pub(crate) sid: crate::registry::ServerId,
    /// The LIBRARY this copy is in, on that server ("Movies", "Film Club") — the row's label.
    /// Libraries are what a person browses; the machine name (`nas-home`) belongs to the Sources
    /// list and to a failure read-out, and appears nowhere else in the product.
    pub(crate) library: String,
    /// **The CREDIT for that server** — `server::servers::owner_credit`'s answer, the same one the
    /// shelf headings and the Sources list draw, and NOT the raw `sourceTitle` it used to be.
    /// `Some(handle)` for a person outside this household; `None` when there is nobody to credit,
    /// which is our own server, the household's own server whichever Plex Home profile is watching,
    /// and a share plex.tv never named.
    ///
    /// `None` is the ABSENCE of an owner, not an empty one: it is what the row spells
    /// "This account", and it is the ordering's own-before-a-friend's tiebreak. That read-out is
    /// exactly right for the first two cases and a slight over-claim for the third, because an
    /// unnamed external share is not distinguishable here today.
    ///
    /// Stamped when the resolve lands and RE-stamped by [`alt_restamp_owners`] whenever the
    /// registry re-describes a source, because this is a copy of a fact that can be corrected under
    /// an open page.
    pub(crate) owner: Option<String>,
    /// That server's ratingKey for the item — per-server, and the whole reason the identity above
    /// it is a guid. Where OK navigates.
    pub(crate) rk: String,
    /// Runtime in ms, for the row's sub-line. 0 = unknown, and then the sub-line is the owner
    /// alone rather than "0 min".
    pub(crate) dur_ms: i64,
    /// `Media.videoResolution` ("4k" / "1080" / "sd"), plus the stored frame size as its fallback —
    /// exactly the trio `ui::fmt::resolution` takes, so the badge in the panel and the hero's media
    /// chip are one function and cannot spell the same file two ways.
    pub(crate) res: String,
    pub(crate) width: i64,
    pub(crate) height: i64,
}

/// **The copies resolved for ONE item, held ADDRESSED.**
///
/// The pair `(sid, rk)` is stored WITH the list and every read supplies its own — so a reader gets
/// the copies for the item it asked about, or nothing at all. That is the whole design, and it
/// replaces a MAILBOX KEY (`FOR_SID`/`FOR_RK`) that a page had to stamp on every mount before a
/// landing could be accepted.
///
/// **That mailbox had stopped being stamped, and nothing said so.** `ui::detail::mount_rk` called
/// `alt_sources::reset(sid, rk)` from every mount; the phase-7 owned `DetailScreen` that replaced
/// it kept only the teardown call (`reset(UNSET, "")`), so from that commit the key was empty for
/// the whole life of every page, `install`'s `same_item` guard refused every real landing, and the
/// *Also available* control could not appear on a device however many sources held the film. A
/// landing that is correctly refused writes no log line, and the panel's own tests stamped the key
/// themselves, so both halves of the evidence agreed with a broken product. An addressed store
/// cannot fail that way: there is no stamp to forget, and the guard the mailbox existed for — a
/// resolve outliving the page that asked for it, which matters because a `ratingKey` is a
/// server-local integer dense from 1 and both servers in a household have a film 4 — is
/// enforced by the READER, which always knows which page it is.
#[derive(Default)]
struct AltStore {
    sid: crate::registry::ServerId,
    rk: String,
    copies: Vec<AltCopy>,
    /// Does the headless stand-in own `copies`? Set where [`alt_dev_stand_in`] writes them, cleared
    /// by a real landing. Always `false` in a release build, where the stand-in is compiled out.
    stand_in: bool,
    /// Which item the stand-in has already had its one chance at, so it is built ONCE per item
    /// rather than on every frame.
    stand_in_rk: String,
}

/// The copies held for `(sid, rk)` — EMPTY for any other item, and for an item nothing has landed
/// for yet.
fn alt_copies<'a>(state: &'a MetadataState, sid: crate::registry::ServerId, rk: &str) -> &'a [AltCopy] {
    let held = &state.alt;
    if crate::registry::same_item((held.sid, held.rk.as_str()), (sid, rk)) {
        &held.copies
    } else {
        &[]
    }
}

/// How many distinct SOURCES hold this item. The gate is stated in sources rather than in copies
/// because that is the design's own wording — two copies in two libraries of ONE server is not
/// "also available *elsewhere*", and the row that would name the other one has nowhere to send you
/// that you are not already.
pub(crate) fn alt_source_count(list: &[AltCopy]) -> usize {
    let mut n = 0;
    for (i, c) in list.iter().enumerate() {
        if !list[..i].iter().any(|p| p.sid == c.sid) {
            n += 1;
        }
    }
    n
}

/// **The gate**: is a second pinned source holding `(sid, rk)`? The Detail page's actions row asks
/// before it draws the control, so with one source there is no button, no layout for it and no
/// draw call.
fn alt_available(state: &MetadataState, sid: crate::registry::ServerId, rk: &str) -> bool {
    alt_source_count(alt_copies(state, sid, rk)) >= 2
}

/// Install the copies resolved for `(item_sid, item_rk)`. Answers whether anything a reader can
/// see changed, which is what raises the Metadata store's notice.
fn alt_install(
    state: &mut MetadataState,
    item_sid: crate::registry::ServerId,
    item_rk: &str,
    list: Vec<AltCopy>,
) -> bool {
    let mut list = list;
    // **The worker's stamp is not trusted.** `resolve_alt_sources` reads the credit on a background
    // thread, and a resolve dispatched before a roster refresh re-grades that credit lands after
    // it — past `pump_alt_sources`' facts epoch, which it has already consumed. Regrading here is
    // the only point that sees both the list and the current answer.
    alt_regrade(&mut list, false);
    let held = &mut state.alt;
    let same = crate::registry::same_item((held.sid, held.rk.as_str()), (item_sid, item_rk))
        && held.copies == list
        && !held.stand_in;
    held.sid = item_sid;
    held.rk = item_rk.to_string();
    held.copies = list;
    // A real resolve replaces whatever was there, the stand-in's list included.
    held.stand_in = false;
    !same
}

/// **Re-read every retained copy's CREDIT from the registry.** Called when the facts epoch moves —
/// `server::servers::facts_gen`, i.e. a source has been re-described — which is a different event
/// from the roster changing and must not be answered the same way.
///
/// The copies themselves are still correct: a re-grade of who is credited for a server does not
/// change which servers hold the item, so discarding the list (or the resolve in flight for it)
/// would throw away good work and leave the panel's control absent until the page was remounted.
/// What IS stale is [`AltCopy::owner`], taken at resolve time — and it is the sixth and last
/// surface that draws the "Shared by …" decision, so leaving it saying the old thing puts the
/// account holder's name back on the household's own library after every other surface has dropped
/// it.
///
/// **MAIN THREAD**, like every other reader and writer of this store — [`pump_alt_sources`] is its
/// only caller and runs on the SDL loop. The workers in this chain touch the mutex-protected
/// result slot and never the store.
fn alt_restamp_owners(state: &mut MetadataState) -> bool {
    let held = &mut state.alt;
    let stand_in = held.stand_in;
    alt_regrade(&mut held.copies, stand_in)
}

/// **Re-read a copy list's CREDITS from the registry**, `true` when any of them moved.
///
/// The ONE place [`AltCopy::owner`] is decided, applied at both boundaries where a list can be
/// wrong: on the way IN ([`alt_install`], because a resolve dispatched before a correction lands
/// after it and no epoch downstream can see that) and on an already-installed list
/// ([`alt_restamp_owners`], because a roster refresh re-grades the credit under a mounted page).
/// Stamping it on the worker as well would be a third opinion; the worker's value is simply
/// overwritten here.
///
/// **Skipped entirely while the headless stand-in owns the list** — see [`alt_dev_stand_in`], whose
/// whole purpose is to FABRICATE a borrowed copy on a slot the registry describes as ours. The
/// trigger outranks the real answer everywhere else in this app for the same reason.
///
/// The guard is "the current copies came from the stand-in", not "the trigger is armed" — which is
/// what it was for one round, and is a different statement. A real resolve can land over a stand-in
/// list, and an armed-but-EMPTY trigger builds no stand-in at all; on both, "armed" would have gone
/// on suppressing the regrade for a list the stand-in does not own.
fn alt_regrade(list: &mut [AltCopy], stand_in_owns: bool) -> bool {
    if stand_in_owns {
        return false;
    }
    // The roster credit (`facts(sid)`'s handle) went with the describers (#166): there is no
    // "Shared by" answer to re-grade, so the re-stamp can never move a row.
    let moved = false;
    let _ = list;
    moved
}

/// Drop copies whose grant left the live registry while the page holding them stayed mounted.
/// Called only when the registry generation moves, so the ordinary per-frame path pays nothing.
fn alt_prune_inactive(state: &mut MetadataState) -> bool {
    let held = &mut state.alt;
    let before = held.copies.len();
    held.copies
        .retain(|c| c.sid == crate::jellyfin::SERVER_ID && crate::jellyfin::client().is_some());
    held.copies.len() != before
}

/// Forget the whole store — paired with [`clear`], whose caller is a page being torn down.
fn alt_clear(state: &mut MetadataState) {
    let held = &mut state.alt;
    held.sid = crate::registry::ServerId::UNSET;
    held.rk = String::new();
    held.copies = Vec::new();
    held.stand_in = false;
    held.stand_in_rk = String::new();
}

/// MAIN THREAD. Ask EVERY registered source whether it holds this guid — the item's own included.
///
/// Including our own copy is not redundancy, it is what the panel is: a list of every copy with the
/// one you are on ticked, whose first row is normally "This account". Querying rather than
/// synthesising that row from the open `Detail` also gets the one field the page does not have —
/// which LIBRARY the copy is in, since a detail page knows its item and not the shelf it came from.
/// And the gate counts distinct SOURCES, so a list built of the others alone can never reach two
/// and the control would never appear however many servers held the film. (It didn't: this
/// function skipped `sid` on its first outing and the button stayed absent on a device with two
/// copies of the same guid.)
///
/// Sources are captured here, on the main thread, as a plain list of ids — the worker resolves each
/// through `client_for` and never asks what is current.
///
/// `sid` is the ITEM's own server, captured with `rk` at the call site because the two are one
/// identity: it rides through [`AltResult`] to [`alt_install`], which files the landing under it rather than
/// under the rk alone. Without it a resolve parked on a dead share's `connect(2)` timeout lands on
/// whatever page holds the same ratingKey when it finally answers, which across two servers is the
/// ordinary case rather than an exotic one.
fn request_alt_sources(
    _state: &mut MetadataState,
    adapter: &std::sync::Arc<MetadataAdapter>,
    sid: crate::registry::ServerId,
    rk: &str,
    guid: &str,
) {
    use std::sync::atomic::Ordering;
    let gen = adapter.alt_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let roster_gen = crate::registry::roster_gen();
    adapter.alt_roster_gen.store(roster_gen, Ordering::SeqCst);
    *adapter.alt_slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if guid.is_empty() {
        return; // nothing portable to match on; the panel stays absent
    }
    let others: Vec<crate::registry::ServerId> = crate::registry::ids().collect();
    if others.len() < 2 {
        return; // a one-server install pays nothing: no worker, no query, no control
    }
    let (rk, guid) = (rk.to_string(), guid.to_string());
    let n = others.len();
    let adapter = std::sync::Arc::clone(adapter);
    let _ = crate::task::spawn_small("altsrc", move || {
        let list = catch_unwind(|| resolve_alt_sources(&others, &guid)).unwrap_or_default();
        // The one line that makes this chain debuggable from a device log. A guid is a public
        // metadata id — not an address, a token or a machine. It was filed as "safe to log" on
        // exactly that reasoning, and the reasoning is incomplete: a Plex GUID names the WORK,
        // globally and stably, which is LG's "Content Viewing Information" and the one category
        // this app's Data Safety declaration answers "Not collected" to. It stays here because it
        // is the only string that says WHICH lookup this was, and `diag::scrub::scrub_viewing`
        // rewrites it to `plex://<guid>` before the line reaches the disk.
        crate::log(&format!(
            "altsrc: asked {n} source(s) for {guid} -> {} copy(ies)",
            list.len()
        ));
        *adapter.alt_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(AltResult {
            gen,
            roster_gen,
            sid,
            rk,
            list,
        });
    });
}

/// WORKER. One `find_by_guid` per source, projected into rows. Pure of app state apart from the
/// registry (an atomic read whose clients are never freed), so it is gradeable against a fixture.
fn resolve_alt_sources(
    others: &[crate::registry::ServerId],
    guid: &str,
) -> Vec<AltCopy> {
    // Jellyfin is the only backend: there is no second source to look the same title up in.
    let _ = (others, guid);
    Vec::new()
}

/// MAIN THREAD, once a frame. Runs the store's whole per-frame half — the two registry epochs, the
/// headless stand-in, and a landed cross-source resolve — and answers whether a reader can see any
/// difference, which is what raises the Metadata store's notice (`stores::metadata`).
///
/// It answers a bool rather than invalidating the frame gate itself because it is a STORE pump:
/// `stores::metadata::pump_alt_sources` folds it through `note(StoreId::Metadata, …)` exactly as
/// `pump_detail` and `pump_season` are folded, so the Detail page hears one `StoreChanged` and
/// repaints from its own arm. `alt_sources::install` used to call `idle::invalidate()` from inside
/// the data layer instead, which is the shape phase 4 replaced.
pub(crate) fn pump_alt_sources_with_gate(state: &mut MetadataState, adapter: &MetadataAdapter,
    gate: &crate::ui::landgate::Gate) -> bool {
    pump_alt_sources_with_library(state, adapter, None, gate)
}

pub(crate) fn pump_alt_sources_with_directory_and_gate(
    state: &mut MetadataState,
    adapter: &MetadataAdapter,
    directory: crate::stores::browse::DirectoryView<'_>,
    gate: &crate::ui::landgate::Gate,
) -> bool {
    let library = directory.current()
        .and_then(|section| directory.sections().get(section))
        .map(|section| section.row.title.as_str())
        .unwrap_or("");
    pump_alt_sources_with_library(state, adapter, Some(library), gate)
}

fn pump_alt_sources_with_library(
    state: &mut MetadataState,
    adapter: &MetadataAdapter,
    library: Option<&str>,
    gate: &crate::ui::landgate::Gate,
) -> bool {
    use std::sync::atomic::Ordering;
    let mut changed = false;
    let roster_gen = crate::registry::roster_gen();
    if adapter.alt_roster_gen.swap(roster_gen, Ordering::SeqCst) != roster_gen {
        adapter.alt_gen.fetch_add(1, Ordering::SeqCst);
        *adapter.alt_slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
        changed |= alt_prune_inactive(state);
    }
    // A source being RE-DESCRIBED is not the server set changing, and answering it the same way
    // would be wrong twice: the copies are still the right copies (a re-graded "Shared by …" credit
    // says nothing about which servers hold the item), and invalidating a resolve in flight would
    // leave the control absent until the page was remounted, since nothing here re-asks. So the
    // two epochs are read separately and this one only re-stamps what the rows SAY.
    let facts_gen = crate::registry::facts_gen();
    if adapter.alt_facts_gen.swap(facts_gen, Ordering::SeqCst) != facts_gen {
        changed |= alt_restamp_owners(state);
    }
    changed |= alt_pump_stand_in(state, library);
    // the landing GATE (§3.3 step 3): a replay takes this on its recorded frame. The roster and
    // facts re-stamps above are NOT gated — they follow other stores' landings, which are gated
    // where those land.
    let taken = crate::stores::take_landing(gate, crate::stores::StoreId::Metadata, || {
        adapter.alt_slot.lock().unwrap_or_else(|e| e.into_inner()).take()
    });
    let Some(r) = taken else { return changed };
    if r.gen != adapter.alt_gen.load(Ordering::SeqCst) || r.roster_gen != roster_gen {
        return changed; // superseded: the page moved on while this was in flight
    }
    // The store is ADDRESSED, so this landing is filed under the PAIR it was asked for and a reader
    // on another page sees nothing — the generation test alone cannot see a page that was opened,
    // left and re-opened between spawn and landing, and the rk alone cannot see the two servers'
    // keys colliding, which they do by default.
    changed | alt_install(state, r.sid, &r.rk, r.list)
}

// ---- the headless stand-in ---------------------------------------------------------------------
//
// Reached through `dev::read`, so the whole of it is absent from a `RELEASE=1` build at compile
// time along with the rest of the `/tmp` surface. The trigger literal is
// `/tmp/butaca-shared`, spelled here for the catalog grep in `docs/agent-reference.md`.

/// Build the headless stand-in once the item has LANDED — called every frame by
/// [`pump_alt_sources`], and doing nothing at all in the ordinary case (two string compares, no
/// allocation).
///
/// It cannot be built when the fetch is dispatched: a mount deliberately clears [`current`] for the
/// whole 2-5 round-trip window, so at that moment there is no runtime, no resolution class and no
/// title to build a copy of the item FROM.
fn alt_pump_stand_in(state: &mut MetadataState, library: Option<&str>) -> bool {
    if crate::app::bootstrap::stores::active() { return false; }
    let Some(d) = state.current.as_ref() else { return false };
    if d.rk == state.alt.stand_in_rk {
        return false; // this item has already had its chance — one string compare
    }
    // Marked whatever the outcome, so the ordinary build — where the trigger is not armed at all —
    // opens the `/tmp` file ONCE per item rather than on every frame of it.
    state.alt.stand_in_rk = d.rk.clone();
    let d = state.current.as_ref().unwrap();
    let Some(list) = alt_dev_stand_in(d, library) else { return false };
    let d = state.current.as_ref().unwrap();
    let held = &mut state.alt;
    held.sid = d.sid;
    held.rk = d.rk.clone();
    held.copies = list;
    held.stand_in = true;
    true
}

/// A DRAW-ONLY copy list for `/tmp/butaca-shared=<handle>`, so the *Also available* panel and
/// the control that opens it can be judged on a television before the multi-server data layer
/// exists.
///
/// **It reuses the trigger the hero's own "Shared by …" run is judged through** rather than adding
/// a second one for the same deliverable: that trigger already means "pretend this item came from
/// `<handle>`", and a second copy of the film on `<handle>`'s source is the other half of the same
/// pretence.
///
/// What it stands in for, exactly:
///
/// * **your copy** is the real one — the page's own ratingKey, on the current server, with the
///   loaded item's own runtime and resolution class, in the library the browse store says you are
///   in. Nothing about it is invented, which is what makes the TICK meaningful.
/// * **their copy** is that same film on a SECOND REGISTRY SLOT ([`alt_stand_in_slot`]). It is a
///   real slot with a real client, so OK on it performs the real source switch and opens a real
///   page — the one behaviour of the panel that cannot be judged from a still.
/// * its resolution is **one class better than yours**, and that is the stand-in's ONE invention.
///   It exists because the ordering rule is otherwise unobservable: the design's own example is a
///   1080p copy you are standing on sorting above a friend's 4K one, which is the case that
///   separates "the copy that plays" from "the best copy". With two equal badges the panel cannot
///   show that it got the rule right.
///
/// `None` (and no stand-in) when the trigger is not armed.
///
/// NB this is **not** one of the two "WINS WHEN ARMED" precedence sites ([`fetch_detail`],
/// `screens::home`'s `dev_source`), which pick between the trigger and a real answer. There is no
/// real answer here to outrank: an unarmed trigger means no panel content at all, and an
/// armed-but-EMPTY file means the same, because a copy list has to be attributed to somebody.
#[cfg(not(test))]
fn alt_dev_stand_in(_d: &Detail, _library: Option<&str>) -> Option<Vec<AltCopy>> {
    None
}
#[cfg(test)]
fn alt_dev_stand_in(_d: &Detail, _library: Option<&str>) -> Option<Vec<AltCopy>> {
    None // the host suite must not depend on what this dev Mac happens to have under /tmp
}

/// True while a detail fetch is in flight — drives the detail page's loading spinner.
pub(crate) fn detail_loading(adapter: &MetadataAdapter) -> bool {
    use std::sync::atomic::Ordering;
    let gen = adapter.detail_gen.load(Ordering::SeqCst);
    gen != 0 && gen != adapter.detail_done.load(Ordering::SeqCst)
}

// ---- season switching ----------------------------------------------------------------------
// The tab UI's season switch is ASYNC: `load_season` flips `cur_season` optimistically (the tab
// highlight moves at once), fetches the episodes on a worker thread, and `pump_season` (called by
// the detail page once a frame) applies the landed list on the main thread. The blocking
// `/children` GET used to run on the main loop, freezing the UI for every rapid season hop.
// Generations guard against out-of-order landings; results for a different item are discarded.
struct SeasonResult {
    gen: u32,
    /// the SERVER the show is on — the other half of `rk`. Without it, hopping from server A's show
    /// page to server B's page with the same rk while a `/children` fetch is in flight installs A's
    /// episode list onto B's page: the generation guard cannot see it (the hop bumped nothing that
    /// distinguishes them) and the rk test passes.
    sid: crate::registry::ServerId,
    rk: String,                // the show the fetch was for
    idx: usize,                // the season it was for
    prev: usize, // the season `cur_season` held before the optimistic flip — restored on failure
    eps: Option<Vec<Episode>>, // None = the fetch failed or panicked — the row keeps its episodes
}

/// Post a finished season fetch to the mailbox. MONOTONE: an older fetch landing late must never
/// clobber a newer result the pump hasn't consumed yet — that lost the newest season forever, and
/// with it the SEASON_DONE catch-up, wedging the loading spinner on. Named rather than inlined in
/// the worker closure for the same reason as `land_detail`: the guard is the one piece of this
/// machinery a test cannot reach through `load_season`.
fn land_season(
    adapter: &MetadataAdapter,
    gen: u32,
    sid: crate::registry::ServerId,
    rk: String,
    idx: usize,
    prev: usize,
    eps: Option<Vec<Episode>>,
) {
    let mut slot = adapter.season_result.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().map(|r| r.gen < gen).unwrap_or(true) {
        *slot = Some(SeasonResult {
            gen,
            sid,
            rk,
            idx,
            prev,
            eps,
        });
    }
}

/// Invalidate any in-flight/pending season fetch and mark the mailbox settled: bump the generation
/// (so a late async landing is discarded), catch SEASON_DONE up to it (season_loading() → false),
/// and clear the slot. Returns the fresh generation. The ONE place the three season atomics move
/// together — used by the blocking `load_season_now`, and by both detail entry points (a new item
/// supersedes the old show's pending fetch): `request_detail` (dropping the OLD item's fetch) and
/// `pump_detail` (dropping one issued WHILE the load was in flight). A third caller,
/// `load_detail_now`, was deleted in phase 12/D7 — see its old definition site's note.
fn supersede_season(adapter: &MetadataAdapter) -> u32 {
    use std::sync::atomic::Ordering;
    let gen = adapter.season_gen.fetch_add(1, Ordering::SeqCst) + 1;
    adapter.season_done.store(gen, Ordering::SeqCst);
    *adapter.season_result.lock().unwrap_or_else(|e| e.into_inner()) = None;
    gen
}

/// Switch the loaded show to season `idx` (the season tabs): `cur_season` flips immediately, the
/// episodes arrive via [`pump_season`]. Main-thread only (touches `state.current`).
fn load_season(state: &mut MetadataState, adapter: &std::sync::Arc<MetadataAdapter>, idx: usize) {
    use std::sync::atomic::Ordering;
    // `prev` rides along so a FAILED fetch can put the tab back on the season whose episodes are
    // still listed (see `pump_season`) — the optimistic flip below is what has to be undone.
    // the loaded show's own server, read here on the MAIN thread — a season belongs to the item it
    // hangs off, so this is the one honest source for it (never `server::current_server()`, which the
    // user may have moved since the page was opened)
    let (sid, rk, season_rk, prev) = match state.current.as_ref().and_then(|d| {
        d.seasons
            .get(idx)
            .map(|s| (d.sid, d.rk.clone(), s.rk.clone(), d.cur_season))
    }) {
        Some(t) => t,
        None => return,
    };
    if let Some(d) = state.current.as_mut() {
        d.cur_season = idx;
    }
    let gen = adapter.season_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let adapter_worker = std::sync::Arc::clone(adapter);
    let spawned = crate::task::spawn_small("season", move || {
        // the mailbox is filled OUTSIDE the guard so a panicking fetch still lands — as a
        // FAILURE (None), not as an empty season: a panic is not "this season has no episodes",
        // and otherwise season_loading() would report an in-flight fetch forever
        let eps = catch_unwind(|| fetch_episodes(sid, &rk, &season_rk)).unwrap_or(None);
        land_season(&adapter_worker, gen, sid, rk, idx, prev, eps);
    });
    if !spawned {
        // no worker means nothing will ever land: catch DONE up or the episode row keeps its
        // loading dim + spinner for the rest of the session. `cur_season` already moved, so the
        // tab highlight stays where the user put it and the old episodes stay listed.
        adapter.season_done.store(gen, Ordering::SeqCst);
    }
}

/// [`load_season`] but BLOCKING — for the page-open paths (`open_rk_season`, and any caller that
/// plays `episodes[0]` right after) where the episode list must be right before the next line
/// runs. Invalidates any in-flight async fetch so a stale landing can't overwrite this one.
fn load_season_now(state: &mut MetadataState, adapter: &MetadataAdapter, idx: usize) {
    let _ = catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (sid, rk, season_rk) =
            match state.current.as_ref().and_then(|d| d.seasons.get(idx).map(|s| (d.sid, d.rk.clone(), s.rk.clone()))) {
                Some(t) => t,
                None => return,
            };
        // `unwrap_or_default`, NOT propagation: this blocking twin still degrades to an empty
        // list on failure. Making it preserve the previous season would silently change what
        // `open_rk_season`'s chained play of `episodes[0]` launches — the WRONG season's first
        // episode under the requested season's name — and that path has no host coverage and needs
        // the full on-device suite. Deferred deliberately.
        let eps = fetch_episodes(sid, &rk, &season_rk).unwrap_or_default();
        supersede_season(adapter); // drop any async fetch in flight; this synchronous list wins
        if let Some(d) = state.current.as_mut() {
            d.episodes = eps;
            d.cur_season = idx;
        }
    }));
}

/// True while a season fetch is in flight — drives the episode row's loading dim + spinner.
pub(crate) fn season_loading(adapter: &MetadataAdapter) -> bool {
    use std::sync::atomic::Ordering;
    let gen = adapter.season_gen.load(Ordering::SeqCst);
    gen != 0 && gen != adapter.season_done.load(Ordering::SeqCst)
}

/// Main-thread pump: apply a landed season fetch to `state.current`, discarding stale generations
/// (a newer request is in flight) and results for a different item. Returns true when the episode
/// list just changed — the detail page resets its episode focus/scroll on it.
pub(crate) fn pump_season_with_gate(state: &mut MetadataState, adapter: &MetadataAdapter,
    gate: &crate::ui::landgate::Gate) -> bool {
    use std::sync::atomic::Ordering;
    // the landing GATE (§3.3 step 3): a replay takes this on its recorded frame
    let res = crate::stores::take_landing(gate, crate::stores::StoreId::Metadata, || {
        adapter.season_result
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    });
    let Some(r) = res else { return false };
    if r.gen != adapter.season_gen.load(Ordering::SeqCst) {
        return false; // superseded — the newer fetch will land after this
    }
    // SETTLE THE SPINNER FIRST — on failure as much as on success. `season_loading()` drives the
    // episode row's loading dim + spinner AND gates `play_episode_at`, so a failure that returned
    // before this store would spin that row and refuse every episode press for the rest of the
    // session.
    adapter.season_done.store(r.gen, Ordering::SeqCst);
    let Some(d) = state.current.as_mut() else {
        return false;
    };
    // OWNERSHIP, as the (server, key) PAIR. The rk alone was enough while one machine was
    // reachable; with a share registered, hopping from A's show to B's show with the same rk
    // while a `/children` is in flight passes an rk-only test and installs A's episodes onto
    // B's page.
    if !crate::registry::same_item((d.sid, &d.rk), (r.sid, &r.rk)) {
        return false; // the page moved to another item — not ours to patch
    }
    match r.eps {
        Some(eps) => {
            d.episodes = eps;
            d.cur_season = r.idx;
            true
        }
        None => {
            // THE FETCH FAILED. Keep the episodes already on screen — one transient
            // `/children` failure used to blank a populated row, with no spinner and no error.
            // And put `cur_season` back on the season those episodes belong to: the tab
            // highlight and the row must agree (`play_episode_at` launches `episodes[i]` under
            // whichever tab reads selected), and it is what makes the tab RETRYABLE — both
            // load paths fetch only when the target `!= cur_season`, so a tab left marked
            // selected could never be asked for again.
            d.cur_season = r.prev;
            false
        }
    }
}


#[cfg(test)]
pub(crate) fn pump_season(state: &mut MetadataState, adapter: &MetadataAdapter) -> bool {
    pump_season_with_gate(state, adapter, crate::ui::landgate::fixture_gate())
}

#[cfg(test)]
mod marker_tests {
    use super::*;

    /// An episode's markers as the live server actually returned them (2026-07-29): an intro and a
    /// `final` credits marker, in that wire order — credits FIRST, which is why nothing here may
    /// assume the array is sorted by time. The wire→model conversion went with the Plex parse
    /// (#166); these fixtures are the MODEL shapes the store holds, which is what the live
    /// playhead logic below reads.
    fn episode_markers() -> Vec<Marker> {
        vec![
            Marker { kind: MarkerKind::Credits, start_ms: 3_065_648, end_ms: 3_130_720, final_seg: true },
            Marker { kind: MarkerKind::Intro, start_ms: 990, end_ms: 99_625, final_seg: false },
        ]
    }

    #[test]
    fn the_playhead_selects_the_segment_it_is_inside() {
        let m = episode_markers();
        assert!(
            marker_at(&m, 0).is_none(),
            "before the intro starts (it begins at 990ms)"
        );
        assert_eq!(
            marker_at(&m, 990).unwrap().kind,
            MarkerKind::Intro,
            "inclusive at the start"
        );
        assert_eq!(marker_at(&m, 50_000).unwrap().kind, MarkerKind::Intro);
        assert!(
            marker_at(&m, 99_625).is_none(),
            "EXCLUSIVE at the end: skipping to it clears the pill"
        );
        assert!(
            marker_at(&m, 2_000_000).is_none(),
            "the long middle of the episode"
        );
        assert_eq!(marker_at(&m, 3_065_648).unwrap().kind, MarkerKind::Credits);
        assert!(marker_at(&[], 1234).is_none());
    }

    /// The Plex-Pass-free tail: a server without credits detection ships no markers, so the last
    /// window of an episode (30s by default) stands in as the credits segment (final, ending at
    /// the duration) — the geometry Up Next arms from. Short items never grow one: a 60s clip
    /// offering "next" for half its runtime is worse than no offer. The window is the user's, so
    /// the geometry is asserted against the configured value rather than a constant.
    #[test]
    fn the_tail_window_stands_in_for_undetected_credits() {
        let w = crate::server::session::UP_NEXT_WINDOW_DEFAULT_S * 1_000;
        let tail = |pos_ms, dur_ms| tail_marker_with(pos_ms, dur_ms, w);
        let dur = 22 * 60 * 1000; // a 22-minute episode
        assert!(tail(0, dur).is_none(), "the open");
        assert!(
            tail(dur - w - 1, dur).is_none(),
            "one ms before the window"
        );
        let m = tail(dur - w, dur).expect("inclusive at the window edge");
        assert_eq!((m.kind, m.final_seg), (MarkerKind::Credits, true));
        assert_eq!((m.start_ms, m.end_ms), (dur - w, dur));
        assert!(
            tail(dur - 1, dur).is_some(),
            "the last frame still offers it"
        );
        assert!(
            tail(80_000, 89_999).is_none(),
            "short items never grow a tail"
        );
        assert!(
            tail(0, 0).is_none(),
            "no duration published (a transcode mid-probe)"
        );
    }

    #[test]
    fn a_final_credits_marker_holds_past_its_stated_end() {
        // The server sets a `final` marker's end to the CONTAINER duration, but our playhead is the
        // decoder's and routinely stops short of it — an exclusive end there made the pill blink
        // out over the last frames, exactly when it is being reached for.
        let m = episode_markers();
        assert!(marker_at(&m, 3_130_720).is_some(), "at the stated end");
        assert!(marker_at(&m, 3_130_720 + 5_000).is_some(), "and past it");

        // A NON-final credits marker (credits before a post-credits scene) must still end, or
        // playback past it would keep offering a skip for a segment already behind the playhead.
        let mid = vec![Marker { kind: MarkerKind::Credits, start_ms: 1000, end_ms: 2000, final_seg: false }];
        assert!(marker_at(&mid, 1500).is_some());
        assert!(
            marker_at(&mid, 2000).is_none(),
            "a non-final segment ends where it says it does"
        );
    }
}

#[cfg(test)]
mod trailer_tests {
    use super::*;
    #[test]
    fn hud_title_falls_back_to_the_parent() {
        let named = Extra {
            title: "Official Trailer".into(),
            ..Default::default()
        };
        assert_eq!(named.hud_title("Movie"), "Official Trailer");
        let untitled = Extra {
            title: String::new(),
            ..Default::default()
        };
        assert_eq!(untitled.hud_title("Movie"), "Movie");
    }

    #[test]
    fn trailer_now_playing_keeps_the_parent_and_the_extra_duration() {
        let _g = crate::testlock::serial();
        let mut state = MetadataState::default();
        set_current_for_test(&mut state, Some(Detail {
            sid: crate::registry::ServerId::UNSET,
            rk: "movie".into(),
            kind: "movie".into(),
            title: "Movie".into(),
            summary: "Blurb".into(),
            year: 2024,
            art: "/art".into(),
            extras: vec![Extra {
                rk: "9".into(),
                title: "Official Trailer".into(),
                dur_ms: 120_000,
                bitrate: 2500,
                part: "/p".into(),
                ..Default::default()
            }],
            ..Default::default()
        }));
        let np = trailer_now_playing(&state, crate::registry::ServerId::UNSET, "9").unwrap();
        assert!(!np.is_episode);
        assert!(!np.is_real_episode, "an extra is never a real episode leaf");
        assert_eq!(np.title, "Movie");
        assert_eq!(np.ep_title, "Official Trailer");
        assert_eq!(np.dur_ms, 120_000);
        assert_eq!(np.detail_rk, "movie");
        assert_eq!(np.thumb, "/art");
        assert!(trailer_now_playing(&state, crate::registry::ServerId::UNSET, "other").is_none());
        set_current_for_test(&mut state, Some(Detail {
            sid: crate::registry::ServerId::UNSET,
            rk: "show".into(),
            kind: "show".into(),
            is_show: true,
            title: "Show".into(),
            extras: vec![Extra {
                rk: "9".into(),
                title: String::new(),
                dur_ms: 90_000,
                part: "/p".into(),
                ..Default::default()
            }],
            ..Default::default()
        }));
        let show = trailer_now_playing(&state, crate::registry::ServerId::UNSET, "9").unwrap();
        assert!(show.is_episode, "a show parent labels Go to Show");
        assert!(
            !show.is_real_episode,
            "a show trailer is not an episode — the HUD must not print an S0 · E0 kicker for it \
             (`is_episode` alone said the opposite of what the player HUD needed here)"
        );
        assert_eq!(show.title, "Show");
        assert_eq!(show.ep_title, "Show");
        assert_eq!(show.dur_ms, 90_000);
        assert_eq!(show.detail_rk, "show");
        set_current_for_test(&mut state, None);
    }
}

#[cfg(test)]
#[path = "metadata_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "metadata_season_mailbox_tests.rs"]
mod season_mailbox_tests;

