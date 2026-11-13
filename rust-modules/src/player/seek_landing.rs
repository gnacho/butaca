//! **A seek that reached the clock but never the picture** — upstream plx-native `9be7a43f6`'s
//! case, in this tree's terms.
//!
//! This tree already recovers the seek that never RE-ANCHORS: `pump.rs`'s stuck watchdog keys on
//! `flushed && rebase_pending` past `SEEK_STUCK_MS`, retries the reopen twice and then reloads. What
//! it had no answer for is the other shape, and it is the one the upstream report is made of: the
//! demux re-anchors, `Play` is accepted, `seeking` goes down and the HUD spinner leaves — and the
//! presented position simply never comes to the target. The pipeline is presenting (the clock is
//! running), the picture is playing *somewhere*, and no watchdog is looking, because every signal
//! the app trusts has already said the seek was satisfied.
//!
//! # What makes it a decision rather than a guess
//!
//! The window is armed by the moment `seeking` FALLS: that is the app's own record of "a seek
//! landed", and it is the only moment at which "the position has not come to the target" means
//! anything. Comparing the playhead against `seek_target_ns` at any other time would fire on
//! ordinary playback, which walks away from an old target by design.
//!
//! Then it waits [`LANDING_MS`] with the clock running, and reports ONCE if the playhead is still
//! more than [`TOLERANCE_NS`] away. A paused player is not a lost seek: the clock stopped on
//! purpose.

/// How far the playhead may sit from the seek's target and still count as "it landed". Wide on
/// purpose: a keyframe-accurate landing can be a second or two off, and a reload costs the viewer a
/// visible re-prime.
pub(crate) const TOLERANCE_NS: i64 = 5_000_000_000;

/// How long the picture has to come to the target after `Play` before the seek is called lost.
/// Longer than the stuck watchdog's budget because this one is about the SINK moving, and a
/// post-seek prime on a 4K stream can spend a second or two there legitimately.
pub(crate) const LANDING_MS: u64 = 4_000;

/// Watches the one window in which "the playhead is short of the target" is a fault.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SeekLanding {
    /// The seek whose `Play` we accepted: its target and the instant `seeking` fell.
    watching: Option<(i64, u64)>,
    /// Reported once per arming, so one lost seek is one report.
    reported: bool,
    /// The frame before this one, to see `seeking` FALL.
    was_seeking: bool,
}

impl SeekLanding {
    pub(crate) const fn new() -> Self {
        Self { watching: None, reported: false, was_seeking: false }
    }

    /// One call per frame. `true` exactly once for a seek that landed on the clock and not on the
    /// screen.
    ///
    /// `target_ns` is the seek's own target (`SHARED.seek_target_ns`), `playpos_ns` the presented
    /// position, and `paused` the transport's own state: a paused player's clock is stopped on
    /// purpose and must never be read as a lost seek.
    pub(crate) fn observe(
        &mut self,
        now_ms: u64,
        seeking: bool,
        paused: bool,
        target_ns: i64,
        playpos_ns: i64,
    ) -> bool {
        let armed_now = self.was_seeking && !seeking && target_ns >= 0;
        self.was_seeking = seeking;
        if armed_now {
            self.watching = Some((target_ns, now_ms));
            self.reported = false;
            return false;
        }
        // A NEW seek supersedes the window: its own landing will be judged when it falls.
        if seeking {
            self.watching = None;
            self.reported = false;
            return false;
        }
        let Some((target, since)) = self.watching else { return false };
        if self.reported || paused {
            return false;
        }
        // The picture came to it: the seek is satisfied, and the window is closed for good rather
        // than left open to fire later on ordinary playback drifting away from an old target.
        if (playpos_ns - target).abs() <= TOLERANCE_NS {
            self.watching = None;
            return false;
        }
        if now_ms.saturating_sub(since) < LANDING_MS {
            return false;
        }
        self.reported = true;
        self.watching = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: i64 = 90_000_000_000;
    /// The position the viewer sought AWAY from: where a stuck picture stays.
    const BEFORE: i64 = 20_000_000_000;

    /// A seek's own sequence: `seeking` up, then down on the accepted `Play`, then the playhead
    /// arrives. Nothing is reported.
    #[test]
    fn a_seek_whose_picture_arrives_is_never_reported() {
        let mut w = SeekLanding::new();
        assert!(!w.observe(0, true, false, TARGET, BEFORE), "the seek is in flight");
        assert!(!w.observe(100, false, false, TARGET, BEFORE), "Play accepted, picture not yet");
        assert!(!w.observe(400, false, false, TARGET, TARGET - 1_000_000_000), "it arrived");
        // …and a later drift away from the target is ordinary playback, not a lost seek.
        for ms in 1..40 {
            assert!(!w.observe(ms * 1_000, false, false, TARGET, TARGET + ms as i64 * 1_000_000_000));
        }
    }

    /// **The upstream case**: the playhead stays where the viewer sought away from, the clock runs,
    /// and after the window the seek is called lost — once, not every frame.
    #[test]
    fn a_seek_whose_picture_never_arrives_is_reported_once() {
        let mut w = SeekLanding::new();
        assert!(!w.observe(0, true, false, TARGET, BEFORE));
        assert!(!w.observe(100, false, false, TARGET, BEFORE), "the window opens here");
        assert!(!w.observe(100 + LANDING_MS - 1, false, false, TARGET, BEFORE), "not yet");
        assert!(w.observe(100 + LANDING_MS, false, false, TARGET, BEFORE), "the seek is lost");
        assert!(
            !w.observe(100 + LANDING_MS + 5_000, false, false, TARGET, BEFORE),
            "and said once"
        );
    }

    /// A paused player is not a lost seek: its clock is stopped on purpose, so the window waits
    /// rather than counting toward the deadline.
    #[test]
    fn a_paused_clock_is_not_a_lost_seek() {
        let mut w = SeekLanding::new();
        assert!(!w.observe(0, true, false, TARGET, BEFORE));
        assert!(!w.observe(100, false, false, TARGET, BEFORE));
        for ms in 1..20 {
            assert!(!w.observe(ms * 1_000, false, true, TARGET, BEFORE), "paused at {ms}s");
        }
        assert!(
            w.observe(100 + LANDING_MS, false, false, TARGET, BEFORE),
            "and the deadline is still met, counted from the accepted Play"
        );
    }

    /// A seek that lands within the tolerance closes its window instead of leaving one armed to
    /// fire later, and a NEW seek replaces the old window rather than being judged by it.
    #[test]
    fn a_landed_or_superseded_window_never_fires_for_the_wrong_seek() {
        let mut w = SeekLanding::new();
        w.observe(0, true, false, TARGET, BEFORE);
        w.observe(100, false, false, TARGET, TARGET - 1_000_000_000);
        for ms in 1..6 {
            assert!(!w.observe(ms * 1_000, false, false, TARGET, TARGET - 1_000_000_000));
        }
        // A new seek to a different target: the old window must not judge it.
        assert!(!w.observe(6_000, true, false, 300_000_000_000, TARGET));
        assert!(!w.observe(6_100, false, false, 300_000_000_000, TARGET));
        assert!(
            !w.observe(6_200, false, false, 300_000_000_000, 300_000_000_000 - 1_000_000_000),
            "the new seek landed; the old one is history"
        );
    }
}
