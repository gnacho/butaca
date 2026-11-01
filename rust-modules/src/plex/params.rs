//! Typed request params for the playback protocol ops — built per call by `route` from its
//! playback state (the client itself is stateless about the current item/selection).
//!
//! These were rebuilt FROM the live `route.rs` (not the original design sketch): the shipped
//! protocol carries a per-playback session id on every request, distinguishes a container-only
//! REMUX from a re-encode, and reports PlayQueue + stream-selection ids on the timeline. The
//! backend-neutral vocabulary these wrap (the encode contract, delivery, offset, ceiling, audio
//! DSP flags, timeline state) lives in [`crate::playback`].

use crate::playback::{EncodeContract, TimelineState, TranscodeOffset};

/// One universal-transcoder request (decision registration + the delivery's start endpoint). Mirrors
/// `route::universal_base`: the CURRENT audio/subtitle selection rides every transcode of the
/// item. `session` is the PMS playback/timeline wire id; `encoder_session` owns the physical
/// transcoder. The mismatch probe measured the two independently, but the simultaneous-encoder
/// TV spike proved PMS cannot prime a replacement while it shares the old X-Plex id. Production
/// therefore keeps them equal for each encoder; the app's stable playback generation is internal
/// and an adaptive replacement changes both wire fields together.
///
/// **A ceiling is answered by choosing the FLAVOR, and only then by asking for a number.** The
/// only bitrate literal in the whole spec is `maxVideoBitrate`, on the RE-ENCODE branch of
/// `transcoder::transcode_query`; the [`remux`](EncodeContract::remux) branch deliberately sends
/// no cap at all, because a resolution or bitrate cap is precisely what makes PMS re-encode instead
/// of copy. And on the third path — direct play, this client's default and its whole point — a cap
/// means nothing, since no encoder is running to obey it. So a link with a ceiling (Plex's 2 Mbit/s
/// relay) is answered by denying direct play and denying the remux, leaving the one branch that
/// can pick a rate: `transcoder::link_policy` is that decision, with the reasoning and the
/// honesty note.
///
/// **[`ceiling`](EncodeContract::ceiling) does not contradict that paragraph — it is its second
/// half, and it arrives only after the first half has run.** This field said "there is no bitrate field
/// here" until 2026-08-23, and the sentence was right about a LINK ceiling and wrong as a general
/// rule, because a link is not the only thing that can impose one. A **user-chosen** ceiling
/// (`route::Quality` — the picker behind the player's `…` menu, for LG checklist #43 CASE1) is a
/// different input with the identical mechanism: `route::quality_policy` returns the same
/// [`LinkPolicy`](super::LinkPolicy) two flags `link_policy` does, `build_stream` composes the two
/// by AND so the stricter always wins, and only once direct play and the remux have BOTH been
/// denied does a number reach the wire here. Adding the field without that gate is the change that
/// looks like it works and does nothing: the file that most needs the cap — a 30 Mbit/s source on
/// a 4 Mbit/s link — is exactly the one that direct-plays, where no encoder ever reads it.
///
/// The invariant that keeps the two halves honest: a spec may carry a ceiling **and**
/// `remux == true` only when the source was already measured under it, so the remux branch still
/// sends no cap and still does not need one. It is asserted in two places because it is two
/// claims — `transcoder`'s tests pin that a ceiling is INERT on a remux, and `route`'s gates 2
/// and 4 pin that a remux is only ever reached for a source under it.
pub struct TranscodeSpec<'a> {
    pub rating_key: &'a str,
    /// The active encoder's PMS playback/timeline wire id.
    pub session: &'a str,
    /// Physical universal-transcoder identity. Production keeps it equal to `session`; the two
    /// fields remain explicit because the protocol probe and fixtures grade their wire roles.
    pub encoder_session: &'a str,
    /// The encode's flavor/delivery/ceiling/audio-DSP shape — see [`EncodeContract`]. Was four
    /// separate fields (`delivery`, `remux`, `no_video_copy`, `ceiling`) until issue #266 needed a
    /// fifth (`audio`) that only ever means anything alongside `remux == true`; one value keeps
    /// that coupling a type can't be built without stating.
    pub contract: EncodeContract,
    /// Source audio stream id to select (0 = server default).
    pub audio_stream_id: i64,
    /// Subtitle stream id to BURN (0 = none). Burn is Plex's decision for our profile —
    /// it advertises no soft-sub support (direct-play subs are client-rendered instead).
    pub subtitle_stream_id: i64,
    /// Restart at this exact content boundary, or omit `offset` for a fresh start.
    pub offset: TranscodeOffset,
}

/// Server-side stream selection for a part (`PUT /library/parts/{id}`) — the transcoder
/// encodes the part's SELECTED audio and burns its SELECTED subtitle; only a PUT changes
/// them (query params on the stream URL do not).
pub struct StreamSelection {
    pub part_id: i64,
    /// 0 = keep the server default (the PUT omits audioStreamID).
    pub audio_stream_id: i64,
    /// Always sent: 0 = subtitles OFF (suppresses a default-selected burn), else burn this id.
    pub subtitle_stream_id: i64,
}

/// One `/:/timeline` progress report (POST — the spec verb). `play_queue_*` empty = omit;
/// `*_stream_id` 0 = omit. PMS exposes timeline playback and encoder ownership independently,
/// but active HLS playback reports the current encoder's coupled wire identity so they remain
/// correlated.
pub struct TimelineReport<'a> {
    pub rating_key: &'a str,
    pub state: TimelineState,
    pub time_ms: i64,
    pub duration_ms: i64,
    pub session: &'a str,
    pub play_queue_id: &'a str,
    pub play_queue_item_id: &'a str,
    pub audio_stream_id: i64,
    pub subtitle_stream_id: i64,
}
