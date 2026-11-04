//! What the connected server says about ITSELF — its version and whether its owner holds a
//! **Plex Pass** — held as process state for the diagnostics surfaces.
//!
//! Exists because of issue #22: a free server cannot encode HEVC, dropped the video track, and
//! the app "had no idea such a thing existed" — the reviewer derived the cause from the server's
//! own transcoder logs. This is the bug class it kills: a claim true on the development environment (a Pass'd server) asserted as universal.
//! This module is the app finally *knowing*, so `app::diagnostics` can print it and
//! `player::error_shape` can name the cause in words.
//!
//! **Visibility only, never behavior.** Nothing here may feed a routing or profile decision —
//! see [`subscription`]'s doc for why that is a rule and not a gap. "Visibility" includes whether
//! a Plex Pass feature is OFFERED at all (issue #266's audio enhancement, `route::enhancements_offered`):
//! the subscription decides that a toggle exists, and never which flavour or profile a stream gets.
//!
//! **PER SERVER, keyed the way the registry is.** This was one process-global answer, justified
//! by "host/port fix at the first install (a later install is only a token swap)" — a premise
//! [`super::servers`] retired: the table re-targets the current server, and a household can hold
//! its own server and a friend's share at once. A single global would mean the stats panel and
//! `player::error_shape` describing SERVER A's build and Plex Pass while the user is looking at
//! (or failing to play) something from server B — issue #22's bug with the polarity flipped, a
//! claim true of the development environment asserted as universal. So state is an array indexed
//! by [`ServerId::raw`], the registry's own slot number, and every accessor comes in two forms:
//! `_of(id)` for a caller that knows which server it is talking about, and the bare form for one
//! that means "the current server" (which is what every reader meant when there was only one).
//!
//! One fetch per registration: `servers::register` used to call [`refresh`] for the server it
//! just registered — own or shared. The registration world and its fetch went with the wire
//! layer (#166), so nothing populates the per-slot answers any more and every read answers the
//! boot state (`Unknown` / build string unknown). The read surface stays exactly as it was:
//! per-slot, keyed by [`ServerId::raw`], and the test seam (`store_for_test`) still seeds it for
//! the suites grading its readers.

use super::servers::{ServerId, MAX_SERVERS};
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};
use std::sync::Mutex;

/// Does the server's owner hold a Plex Pass? A TRISTATE, not a bool with a default: `Unknown`
/// (never fetched, fetch failed, or a server old enough not to say) must stay distinguishable from
/// a real "no" — the error wording in `player::error_shape` blames a missing Pass only on a
/// known-free server, never on one we merely haven't heard from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Subscription {
    #[default]
    Unknown = 0,
    No = 1,
    Yes = 2,
}

impl Subscription {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::No,
            2 => Self::Yes,
            _ => Self::Unknown,
        }
    }
    /// The wire field → the tristate. `None` is a server that never said (the field predates
    /// nothing we can date, so absence is only ever "unknown", never "free"); the lenient adapter
    /// has already folded `true`/`"1"`/`1` to nonzero and `false`/`"0"` to 0. The wire read that
    /// fed it went with the fetch (#166); the mapping stays for the module suite.
    #[cfg(test)]
    fn from_wire(v: Option<i64>) -> Self {
        match v {
            None => Self::Unknown,
            Some(0) => Self::No,
            Some(_) => Self::Yes,
        }
    }
}

/// `Subscription` as u8 (the enum's own discriminants), PER SERVER. 0 = Unknown is the boot
/// state, and also what every slot no server has been registered into permanently reads.
static SUBSCRIPTION: [AtomicU8; MAX_SERVERS] = [const { AtomicU8::new(0) }; MAX_SERVERS];
/// Each server's full build string ("1.43.3.10861-cd85035e7"), "" until its fetch lands. ONE
/// Mutex over the whole array rather than one per slot: it is read at 2 Hz by the stats panel's
/// sample step and written once per registration — never in a per-frame path, and never held
/// across a round trip (the fetch happens first, the store second).
static VERSION: Mutex<[String; MAX_SERVERS]> = Mutex::new([const { String::new() }; MAX_SERVERS]);

/// The array index for a server, `None` for [`ServerId::UNSET`] and for any id the registry
/// cannot have issued. The registry's own ceiling is the bound (see [`MAX_SERVERS`]), so this
/// cannot silently disagree with it — and an unknown id reads as "nothing known", never as
/// another server's answer or a panic.
fn slot(id: ServerId) -> Option<usize> {
    let i = id.raw() as usize;
    (id.is_set() && i < MAX_SERVERS).then_some(i)
}

/// [`subscription`] for a NAMED server — what a caller uses once it knows which server the item,
/// the failed playback or the panel row belongs to. `Unknown` for a server that has not answered
/// yet, and for an id that names nothing. Nothing populates the slots any more (the registration
/// fetch went with the wire layer, #166), so this answers the boot state: `Unknown`.
pub(crate) fn subscription_of(id: ServerId) -> Subscription {
    slot(id)
        .map(|i| Subscription::from_u8(SUBSCRIPTION[i].load(Relaxed)))
        .unwrap_or(Subscription::Unknown)
}

/// [`version`] for a NAMED server; "" while unknown.
pub(crate) fn version_of(id: ServerId) -> String {
    let Some(i) = slot(id) else {
        return String::new();
    };
    VERSION.lock().map(|g| g[i].clone()).unwrap_or_default()
}


/// The CURRENT server's build string, "" while unknown. Clones under the lock — callers sample it
/// (the stats panel holds each sample for 500 ms), they don't call it per frame. A caller that
/// knows which server it means wants [`version_of`], for [`subscription`]'s reason.
pub(crate) fn version() -> String {
    version_of(super::servers::current())
}

/// Publish one server's answer into its slot. Split from the retired fetch so the SCOPING is
/// host-testable without a socket: below this line is per-slot state, and the property that
/// matters — one server's answer never reaching another's reader — lives entirely there.
///
/// **This is an async LANDING, so it owes the present gate a poke.** It runs on a worker, seconds
/// after boot or after a share is registered, and it changes what a SETTLED screen says: the stats
/// panel's Server row, the detail hero's "hardware conversion needs \[PLEX PASS\]" note, and the
/// failure read-out's capsule all read it. Without the [`crate::ui::idle::invalidate`] call the new
/// fact arrives invisibly and the screen keeps stating the old one until the next keypress — bounded
/// only by the 2 s keepalive, which is insurance and not pacing (that module's rule: *a new async
/// landing that repaints must add a call there*). Unconditional rather than change-guarded: a
/// re-registration that stores the same answer costs one frame, and a guard here would be a second
/// place that has to agree with what the readers draw.
#[cfg(test)]
fn store(id: ServerId, sub: Subscription, version: &str) {
    let Some(i) = slot(id) else { return };
    SUBSCRIPTION[i].store(sub as u8, Relaxed);
    if let Ok(mut g) = VERSION.lock() {
        g[i] = version.to_owned();
    }
    crate::ui::idle::invalidate();
}

/// Test-only: publish one server's answer without a round trip.
///
/// [`store`] is this module's private seam, and the suites that need it are the ones grading its
/// READERS — `route`'s Pass-gated audio-enhancement offering and `app::diagnostics`' stats Server
/// row, neither of which can reach a real server on the host. Callers must
/// hold `crate::testlock::serial()`: the slot arrays are crate globals, and they deliberately
/// outlive `servers::reset_for_test` (they are keyed on the SLOT, not on the client), so a test
/// must seed every slot it reads rather than assume a boot state.
#[cfg(test)]
pub(crate) fn store_for_test(id: ServerId, sub: Subscription, version: &str) {
    store(id, sub, version);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tristate mapping is the module's contract: absence is UNKNOWN (an old server that
    /// never says must not read as free), zero is a real "no", anything else a real "yes" — and
    /// the u8 round trip through the atomic must not invent a state.
    #[test]
    fn the_wire_field_maps_to_the_tristate_without_inventing_knowledge() {
        assert_eq!(Subscription::from_wire(None), Subscription::Unknown);
        assert_eq!(Subscription::from_wire(Some(0)), Subscription::No);
        assert_eq!(Subscription::from_wire(Some(1)), Subscription::Yes);
        for s in [Subscription::Unknown, Subscription::No, Subscription::Yes] {
            assert_eq!(Subscription::from_u8(s as u8), s, "u8 round trip");
        }
        // an unmapped u8 (a torn write could not produce one, but the match must still be total)
        assert_eq!(Subscription::from_u8(7), Subscription::Unknown);
    }


    /// **The reason this module stopped being one global.** A Plex Pass claim is about ONE
    /// server's owner: with our own server and a friend's share both filed, an answer stored
    /// under A must be invisible to a reader asking about B.
    ///
    /// The slot arrays are crate globals, so this holds `testlock::serial()` and seeds through
    /// the private [`store`] — no fetch has produced a real answer since the wire layer went
    /// (#166), which is exactly why the test seeds both slots by hand before asserting the
    /// isolation.
    #[test]
    fn one_servers_plex_pass_is_never_read_as_the_others() {
        let (a, b) = (ServerId::from_raw(0), ServerId::from_raw(1));
        // the slot arrays are keyed on the SLOT and never reset by anything else, so start from
        // the boot state explicitly rather than from what an earlier test left
        store(a, Subscription::Unknown, "");
        store(b, Subscription::Unknown, "");

        store(a, Subscription::Yes, "1.43.3.10861-cd85035e7");
        assert_eq!(subscription_of(a), Subscription::Yes);
        assert_eq!(version_of(a), "1.43.3.10861-cd85035e7");
        assert_eq!(
            subscription_of(b),
            Subscription::Unknown,
            "B has not answered — not 'free'"
        );
        assert_eq!(version_of(b), "", "and it has no build string either");

        store(b, Subscription::No, "1.32.0.6918-free");
        assert_eq!(
            subscription_of(a),
            Subscription::Yes,
            "B's answer left A's alone"
        );
        assert_eq!(version_of(a), "1.43.3.10861-cd85035e7");

        // the bare accessor means "the current server", and the current server is UNSET on every
        // boot of the single-source app: the row it answers with is the empty one, never a slot's
        assert_eq!(version(), "");

        // an id that names no server is "nothing known" — never slot 0's answer, never a panic,
        // and a write through one must not reach the array at all
        assert_eq!(subscription_of(ServerId::UNSET), Subscription::Unknown);
        assert_eq!(version_of(ServerId::UNSET), "");
        let past_end = ServerId::from_raw(MAX_SERVERS as u16);
        assert_eq!(subscription_of(past_end), Subscription::Unknown);
        store(ServerId::UNSET, Subscription::Yes, "nowhere");
        store(past_end, Subscription::Yes, "nowhere");
        assert_eq!(subscription_of(a), Subscription::Yes);
        assert_eq!(
            subscription_of(b),
            Subscription::No,
            "no write landed in a real slot"
        );
    }

    /// **This is an async LANDING, so it owes the frame gate a poke.** The answer arrives on a
    /// worker seconds after boot — later still for a share registered while the user is reading a
    /// page — by which time the screen that reads it has settled and stopped presenting: the stats
    /// panel's Server row, the detail hero's "hardware conversion needs [PLEX PASS]" note, and the
    /// failure read-out's capsule. Without the `ui::idle::invalidate` inside [`store`] the new fact
    /// reaches the panel only on the next keypress (bounded by the 2 s keepalive, which is
    /// insurance and not pacing) — the "arrives invisibly" failure `ui::idle::invalidate`'s
    /// call-site list exists to prevent.
    ///
    /// Takes `testlock::serial()`: the gate's flag is a crate global that `ui::idle`'s own
    /// "a settled screen does not repaint" assertions read, so an unlocked test here would fail
    /// ANOTHER module's tests intermittently — `ui::xfade`'s test module states the same rule, and
    /// this borrows its isolation helper.
    #[test]
    fn a_landed_server_fact_asks_the_frame_gate_to_repaint() {
        let _g = crate::testlock::serial();
        // `frame_begin` forgets last frame's motion, `note_present` retires the 2 s keepalive, and
        // `should_present` TAKES-AND-CLEARS — so each call answers for exactly the frames since the
        // previous one, and nothing but the store below can be what answered.
        let asked = || {
            crate::ui::idle::set_enabled(true); // a test that disabled the gate leaves it off
            crate::ui::idle::frame_begin(1.0 / 60.0);
            crate::ui::idle::note_present(0);
            crate::ui::idle::should_present(0)
        };
        asked(); // drain whatever the previous test left on the shared flag
        assert!(
            !asked(),
            "the gate is quiet before the landing — or this proves nothing"
        );

        let id = ServerId::from_raw(0);
        store(id, Subscription::No, "1.32.0.6918-free");
        assert!(
            asked(),
            "a subscription that just landed must reach the screen stating it"
        );

        store(id, Subscription::Unknown, "");
        assert!(asked(), "…and so must the boot state being put back");

        // a write through an id that names no server publishes nothing, so it wakes nothing
        store(ServerId::UNSET, Subscription::Yes, "nowhere");
        assert!(!asked(), "a write that landed in no slot changed no screen");
    }
}
