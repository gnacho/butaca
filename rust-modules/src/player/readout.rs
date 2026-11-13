//! **The player's read-out** — the small, read-only view of the pipeline that the chrome, the
//! reporter and the event log ask their questions of.
//!
//! Upstream plx-native grew this as `media/src/player/readout.rs` and its absence here is why three
//! of its fixes (`9be7a43f6`, `62354d903`, `93697adcf`'s neighbours) do not port: they ask questions
//! like *"is a replacement start queued?"* and the truthful answer lives in three different places
//! (a `SHARED` atomic, the route control's phase, and the seek target). Gathering them here means a
//! caller asks one thing once, instead of reaching into the atomics and hoping its two reads came
//! from the same instant.
//!
//! # What is a snapshot and what is a lookup
//!
//! [`Transport::now`] is a SNAPSHOT: three relaxed atomic loads, cheap enough for a per-frame caller,
//! taken once so a caller's position and duration cannot come from two frames. Everything else here
//! is a LOOKUP that takes a lock (`route::pending_route_start` locks the player control), and is
//! therefore for decisions and for the log — never for the render path.
//!
//! This is deliberately small. It is the first piece of the seam, added where it has consumers that
//! already needed the answer, not a façade over the whole `SHARED` block.

use std::sync::atomic::Ordering;

/// The transport's own clock, as one snapshot.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Transport {
    /// The displayed playhead. Seeded with a seek's target the moment the reload is armed
    /// (`engine::arm_seek`), so it is never 0:00 across a reload.
    pub(crate) position_ns: i64,
    pub(crate) duration_ns: i64,
    pub(crate) paused: bool,
}

impl Transport {
    pub(crate) fn now() -> Self {
        let shared = &super::SHARED;
        Self {
            position_ns: shared.playpos_ns.load(Ordering::Relaxed),
            duration_ns: shared.duration_ns.load(Ordering::Relaxed),
            paused: super::TX.paused.load(Ordering::Relaxed),
        }
    }

    /// How far into the item the viewer is, clamped: a negative playhead is a state, not a position.
    pub(crate) fn position_s(&self) -> i64 {
        (self.position_ns / 1_000_000_000).max(0)
    }
}

/// **A replacement start is queued**: a route start in flight, which is what a seek's own reload is
/// while it waits for the previous object to let go.
///
/// Takes the player control's lock. That is the right price for the event log and for a decision, and
/// the wrong one for the render path — see this module's note.
pub(crate) fn start_pending() -> bool {
    crate::route::pending_route_start().is_some()
}

/// Where the queued start is headed, when it has a target.
///
/// `engine::arm_seek` publishes the target; the route control clears it to -1 when no reload is
/// pending, which is why a negative value reads as "no target" rather than as a seek to zero.
pub(crate) fn reload_target_ns() -> Option<i64> {
    let target = super::SHARED.seek_target_ns.load(Ordering::Relaxed);
    (target >= 0).then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_atoms<R>(position: i64, duration: i64, paused: bool, body: impl FnOnce() -> R) -> R {
        let _serial = crate::testlock::serial();
        let shared = &super::super::SHARED;
        let was = (
            shared.playpos_ns.load(Ordering::Relaxed),
            shared.duration_ns.load(Ordering::Relaxed),
            shared.seek_target_ns.load(Ordering::Relaxed),
            super::super::TX.paused.load(Ordering::Relaxed),
        );
        struct Restore((i64, i64, i64, bool));
        impl Drop for Restore {
            fn drop(&mut self) {
                let shared = &super::super::SHARED;
                shared.playpos_ns.store(self.0 .0, Ordering::Relaxed);
                shared.duration_ns.store(self.0 .1, Ordering::Relaxed);
                shared.seek_target_ns.store(self.0 .2, Ordering::Relaxed);
                super::super::TX.paused.store(self.0 .3, Ordering::Relaxed);
            }
        }
        let _restore = Restore(was);
        shared.playpos_ns.store(position, Ordering::Relaxed);
        shared.duration_ns.store(duration, Ordering::Relaxed);
        super::super::TX.paused.store(paused, Ordering::Relaxed);
        body()
    }

    /// **One snapshot, one instant**: the three fields a caller reads together come from together,
    /// and the position is reported in seconds with a negative playhead clamped rather than
    /// wrapped into a huge positive one.
    #[test]
    fn the_transport_is_one_snapshot_of_the_pipeline() {
        with_atoms(120_500_000_000, 7_200_000_000_000, false, || {
            let t = Transport::now();
            assert_eq!(t.position_ns, 120_500_000_000);
            assert_eq!(t.duration_ns, 7_200_000_000_000);
            assert!(!t.paused);
            assert_eq!(t.position_s(), 120);
        });
        with_atoms(-1, 0, true, || {
            let t = Transport::now();
            assert_eq!(t.position_s(), 0, "a negative playhead is not a position");
            assert!(t.paused);
        });
    }

    /// **The two questions the seam exists for**: whether a replacement start is queued, and where
    /// it is headed. A cleared target (-1) is "no reload", not "a reload to zero", and that
    /// distinction is exactly what a log reader needs to tell a queued reload from a seek to 0:00.
    #[test]
    fn the_queued_start_and_its_target_are_readable() {
        with_atoms(0, 0, false, || {
            super::super::SHARED.seek_target_ns.store(-1, Ordering::Relaxed);
            assert_eq!(reload_target_ns(), None, "no reload pending");
            super::super::SHARED.seek_target_ns.store(90_000_000_000, Ordering::Relaxed);
            assert_eq!(reload_target_ns(), Some(90_000_000_000));
            super::super::SHARED.seek_target_ns.store(0, Ordering::Relaxed);
            assert_eq!(
                reload_target_ns(),
                Some(0),
                "a reload to the very start is a target, not an absence"
            );
        });
        // A fresh process has no route start in flight; anything else would mean the control
        // outlived a session, which its own teardown settles.
        let _serial = crate::testlock::serial();
        assert!(!start_pending());
    }
}
