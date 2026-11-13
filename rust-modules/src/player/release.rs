//! **What one native release proved** — the typed handoff the seam family is missing.
//!
//! Upstream plx-native spells this `ReleaseEvent` (`UnloadHandedOff` and its siblings). This tree
//! performs the same sequence in `engine::release_native_object` — `Unload` → callback gate closed
//! → Rust epoch retired → D1 or quarantine — but it reports the result as three loose logs, and the
//! reload path that depends on it cannot say what it was waiting for.
//!
//! **The order is the contract and the three proofs are independent.** `Unload` waits for the
//! GStreamer state transition, but neither libpf's raw callback fields nor `type=23` is an exported
//! producer/in-flight barrier: the executable interposes the callback slot, and after `Unload` the C
//! gate refuses new calls and waits for calls already inside. Only then can the Rust epoch be
//! retired and the object be considered for destruction. **Any missing proof leaves the object
//! quarantined**, which is a leak by choice: D1 frees private state a late firmware hook can still
//! dereference, and a leak is cheaper than a use-after-free.
//!
//! So a [`Release`] is not a status flag, it is the evidence a caller needs to decide whether it may
//! hand the next `Load` the same slot: `destroyable()` is exactly the D1 conjunction, and a reload
//! that starts on a quarantined object is asking the C seam for an object it will refuse.

/// One native release, as the three proofs it produced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Release {
    /// The native epoch this release is about. Never reused, which is what makes a second release
    /// of the same one a defect rather than a retry.
    pub(crate) epoch: u32,
    /// `Unload` returned AND `type=23` named this epoch: the pipeline stopped and said so.
    pub(crate) unloaded: bool,
    /// The interposed callback gate refused new calls and drained those already inside
    /// (`sf_callback_gate_retire`). Without it a late firmware hook can still reach private state.
    pub(crate) gate_closed: bool,
    /// The Rust epoch is retired: no callback can name it any more, and `sf_destroy` is safe.
    pub(crate) epoch_retired: bool,
}

impl Release {
    pub(crate) fn new(epoch: u32, unloaded: bool, gate_closed: bool, epoch_retired: bool) -> Self {
        Self { epoch, unloaded, gate_closed, epoch_retired }
    }

    /// **D1: the object may be destroyed.** Every proof, and there is deliberately no best-effort
    /// or timeout branch — a caller that "mostly" proved it gets a quarantine.
    pub(crate) fn destroyable(&self) -> bool {
        self.unloaded && self.gate_closed && self.epoch_retired
    }

    /// Which proof is missing, as one word for the log — `None` when nothing is.
    pub(crate) fn missing_proof(&self) -> Option<&'static str> {
        if !self.unloaded {
            Some("unload")
        } else if !self.gate_closed {
            Some("callback_gate")
        } else if !self.epoch_retired {
            Some("epoch_retirement")
        } else {
            None
        }
    }

    /// The line the native lifecycle writes, once, per release.
    pub(crate) fn describe(&self) -> String {
        match self.missing_proof() {
            None => format!("native lifecycle: epoch={} released (D1)", self.epoch),
            Some(proof) => format!(
                "native lifecycle: epoch={} quarantined — no {} proof",
                self.epoch, proof
            ),
        }
    }
}

/// **A release must happen once per epoch**, and this is where that is said rather than trusted.
///
/// The native epoch is never reused, so a second release of the same one means a caller released an
/// object twice — the shape that would let D1 free state a live hook still points at. The tracker is
/// per process and remembers only the last epoch: two releases in a row of the same epoch is the
/// defect, and anything older has already been retired from the callback registry.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ReleaseTracker {
    last: Option<u32>,
}

impl ReleaseTracker {
    /// Record a release. `false` when this epoch was already released — the caller is releasing the
    /// same object twice, and the D1 path must not run again.
    pub(crate) fn note(&mut self, release: Release) -> bool {
        if self.last == Some(release.epoch) {
            return false;
        }
        self.last = Some(release.epoch);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **D1 is the conjunction, with no partial credit**: each missing proof on its own quarantines
    /// the object, and the word the log carries names which one.
    #[test]
    fn every_proof_is_required_and_the_missing_one_is_named() {
        let full = Release::new(7, true, true, true);
        assert!(full.destroyable());
        assert_eq!(full.missing_proof(), None);
        for (missing, event) in [
            ("unload", Release::new(7, false, true, true)),
            ("callback_gate", Release::new(7, true, false, true)),
            ("epoch_retirement", Release::new(7, true, true, false)),
        ] {
            assert!(!event.destroyable(), "{missing} is missing, so the object is quarantined");
            assert_eq!(event.missing_proof(), Some(missing));
        }
        assert!(Release::new(7, true, true, true).describe().contains("released (D1)"));
        assert!(Release::new(
            7,
            true,
            false,
            true
        )
        .describe()
        .contains("no callback_gate proof"));
    }

    /// **Once per epoch.** A second release of the same object is refused rather than counted, which
    /// is the property that keeps D1 from running twice on one object; a NEW epoch is a new object
    /// and goes through.
    #[test]
    fn a_release_fires_once_per_epoch() {
        let mut tracker = ReleaseTracker::default();
        assert!(tracker.note(Release::new(3, true, true, true)), "the first release of epoch 3");
        assert!(
            !tracker.note(Release::new(3, true, true, true)),
            "the same epoch released twice is the defect, not a retry"
        );
        assert!(tracker.note(Release::new(4, true, true, true)), "the next epoch is a new object");
        assert!(!tracker.note(Release::new(4, true, true, true)));
    }
}
