//! Shared fixtures and helpers for the `route::decision` test modules split out below.

use super::*;

/// Same provenance as `plan::tests::p5`: single-layer IPT-PQ with no HDR10 fallback.
// Dev-only: used only by the `#[cfg(feature = "devtriggers")]` tests in this module (see the
// comment on the first one).
#[cfg(feature = "devtriggers")]
pub(super) fn p5() -> crate::metadata::Dovi {
    crate::metadata::Dovi {
        present: true,
        profile: 5,
        bl_compat: 0,
        el_present: false,
        ..crate::metadata::Dovi::NONE
    }
}

/// Most route tests install a projection without constructing a native Engine. Make that
/// synthetic boundary explicit in the fixture layer so production `apply_plan` and tests of
/// the start reducer both retain the real `Prepared -> Starting -> result` semantics.
pub(super) fn apply_plan(ps: &mut PlaybackSession, plan: Plan, rk: &str) {
    let start = super::apply_plan(ps, &mut crate::stores::metadata::MetadataStore::default(), plan, rk);
    settle_plan_start_in_unit_test(ps, start);
}

pub(super) fn settle_pending_native_start(ps: &mut PlaybackSession, result: RouteStartResult) -> RouteStartAttempt {
    let transaction = pending_route_start().expect("prepared native start transaction");
    let attempt = claim_route_start_attempt(transaction).expect("physical Load attempt");
    assert!(settle_route_start(ps, attempt, result));
    attempt
}

pub(super) fn rollback_seconds(ps: &mut PlaybackSession) -> Option<i64> {
    rollback_original_recovery(ps).map(|rollback| rollback.offset_ns / 1_000_000_000)
}

pub(super) fn test_original_candidate(subtitle_ordinal: Option<i32>) -> AutoOriginalCandidate {
    AutoOriginalCandidate {
        url: "https://example.invalid/source.mkv".into(),
        probe_part: "https://example.invalid/source.mkv".into(),
        direct: true,
        vcodec: "hevc".into(),
        fps: 23.976,
        dovi: crate::metadata::Dovi::NONE,
        dv_decision: crate::metadata::DvDecision::NONE,
        audio: Some(CarriedAudio {
            sid: 42,
            ordinal: 1,
            codec: "eac3".into(),
            channels: 6,
            can_normalize_loudness: false,
            immersive: true,
        }),
        subtitle_ordinal,
    }
}

// ---- the playing item's SERVER: captured once, carried by value ---------------------------
// A ratingKey, a Part id, a Stream id, a playQueueID and a resume point are all keys on ONE
// server. Every server call in this file used to find its server by asking which one was current
// at the instant of the call, so an item borrowed from a shared source was resolved, queued,
// PUT, stopped and — every ten seconds — reported to whichever server the user had since
// wandered off to. None of that is observable from inside the app, which is why these exist.

/// Take the crate-wide serialization lock, empty the server registry AND idle the session, so
/// each test below starts from "nothing installed" and, more importantly, LEAVES the table that
/// way. A test that registers a loopback server and walks off owes the next one an empty table:
/// its ports close when it returns, so what it leaves behind is a `client_opt()` that answers
/// `Some` with a client nothing will ever answer.
///
/// The session goes with it because it holds a `ServerId` INTO that table: a leftover `cur_sid`
/// names a slot the next test is about to re-fill with a different server, and `machine_id` is
/// a cache keyed on exactly that id — which `the_machine_id_cache_is_scoped_to_the_server_that_taught_it`
/// then reads. `reset_session` is the whole-session write, and this is what it is for.
pub(super) fn fresh_registry(ps: &mut PlaybackSession) -> crate::testlock::Serial {
    let g = crate::testlock::serial();
    // These are process-global route transactions, not Session fields. A host test has no
    // Engine pump to spend them, so leaving either behind makes a later loopback server see a
    // stop for an encoder from a completely different case.
    let _ = take_pending_original();
    // Establish the idle projection before resetting the reducer: its applied snapshot must
    // describe this test's empty route, not the previous test's final encoder.  Quality is
    // part of the same baseline; cases which need Auto opt in after this boundary and then
    // land a route explicitly.
    reset_session(ps);
    restore_quality(Quality::Original);
    restore_direct_play_mode(DirectPlayMode::Auto);
    // Issue #266: `ResolveEnv::snapshot` reads the enhancement preference from a global, so a
    // test that set it must not leak an enhanced resolve into the next one.
    crate::player::restore_audio_enhancements(crate::playback::AudioEnhancements::NONE);
    crate::player::reset_route_requests_for_test(ps);
    crate::registry::reset_for_test();
    crate::player::clear_original_failure();
    g
}

/// A `ServerId` naming a slot nothing can ever be registered in — `build_stream` takes its
/// no-client exit without opening a socket. The registry's client half is retired (#166), so
/// every non-Jellyfin slot is permanently empty; the assertion below pins that.
pub(super) fn unregistered_sid() -> ServerId {
    let id = ServerId::from_raw((crate::registry::MAX_SERVERS - 1) as u16);
    assert!(
        id != crate::jellyfin::SERVER_ID,
        "the test needs a slot the live backend does not own"
    );
    id
}


pub(super) fn fourk_item_with_subs(
    sid: ServerId,
    audio: Vec<crate::metadata::Stream>,
    subs: Vec<crate::metadata::Stream>,
) -> crate::metadata::PlayingItem {
    crate::metadata::PlayingItem {
        sid,
        rk: "rk-4k".into(),
        container: String::new(),
        show_rk: String::new(),
        audio,
        subs,
        video_fps: 23.976,
        width: 3840,
        height: 2160,
        bitrate: 48_000,
        dovi: crate::metadata::Dovi::NONE,
        markers: Vec::new(),
        chapters: Vec::new(),
        blur: None,
    }
}
