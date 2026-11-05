//! Frame-thread regression tests for the series track memory.
//!
//! A track pick is handled while the page is stepping, inside [`crate::task::FrameScope`]. The
//! memory of that pick must not turn into session storage I/O on that thread: the guard aborts a
//! debug build before the pick can finish.

use super::*;

#[test]
fn remembering_a_series_track_never_blocks_the_frame_thread() {
    let _guard = crate::testlock::serial();
    let session = crate::server::session::TempSession::new("track-commit-frame");
    session.watching("u-track-commit-frame");
    let entry = crate::server::session::SeriesTracks {
        series: "show-rk".into(),
        audio_index: 2,
        sub_index: 3,
        ..Default::default()
    };
    let _frame = crate::task::FrameScope::enter();
    assert!(remember_series_tracks(entry));
}
