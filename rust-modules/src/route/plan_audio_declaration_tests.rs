//! **The Load declares the codec of the bytes the demuxer feeds** (upstream plx-native
//! `62354d903`, whose report is a stalled picture rather than a cosmetic mislabel: a server
//! reporting `eac3` over a file whose bytes are AAC configured the wrong decoder, and with
//! audioSync the video sink slaved to a dead audio clock and the film stopped).
//!
//! On this backend a direct play can only be reached WITH a doable audio pick — `directplay`'s own
//! gate includes `audio_sel.is_some()` — so the no-pick branch is unreachable today. It is still
//! graded here, because it is the branch that declares the wrong codec the day that gate widens,
//! and because "unreachable" is a property of the gate rather than of this function.

use super::audio_declaration;

/// A pick names the track's codec: the demuxer is told to feed THAT stream, so that is the codec
/// of the bytes on the wire, whatever the file's own default happens to be.
#[test]
fn a_picked_track_declares_its_own_codec() {
    for picked in ["aac", "eac3", "ac3", "dts", "truehd"] {
        assert_eq!(
            audio_declaration(2, picked, "eac3"),
            picked,
            "picked {picked} over a source declared eac3"
        );
    }
}

/// No pick declares the FILE's ground truth, never a track the server lists: the server's track
/// codec can describe the source of a remux rather than the bytes it feeds.
#[test]
fn a_no_pick_declares_the_files_own_codec() {
    assert_eq!(
        audio_declaration(-1, "eac3", "aac"),
        "aac",
        "a remux whose server still lists eac3 feeds AAC, and must declare it"
    );
    // …and the fallback's own sentinel: the callers pass `src_acodec` as `picked` when they have no
    // track, which must not turn into a track codec by accident.
    assert_eq!(audio_declaration(-1, "aac", "aac"), "aac");
}

/// The two answers are different whenever the pick and the file's default disagree — the case the
/// upstream report is made of — so a regression that collapses them is visible here.
#[test]
fn the_two_answers_are_distinguishable_when_they_disagree() {
    assert_ne!(
        audio_declaration(0, "aac", "eac3"),
        audio_declaration(-1, "aac", "eac3"),
        "picked AAC and fed AAC by default are the same string, so the test needs a case that differs"
    );
    assert_eq!(audio_declaration(0, "ac3", "eac3"), "ac3");
    assert_eq!(audio_declaration(-1, "ac3", "eac3"), "eac3");
}
