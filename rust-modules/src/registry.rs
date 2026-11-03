//! The SERVER REGISTRY — the backend-neutral slot vocabulary every store keys on.
//!
//! This was the table of leaked `server::Client`s behind `client_for`/`client()`/`register`. The
//! dispatch collapse (#202) removed every path that could publish a Plex client, so the table
//! over `server::Client` was unreachable code, and this slice (#166) retired it together with the
//! wire layer it existed to index (`plex/client.rs` and siblings). What survives is the
//! vocabulary the app still shares across backends:
//!
//! * [`ServerId`] — the stable per-process slot handle that sits in UI state, routes and queued
//!   jobs. The Jellyfin backend names its one server `crate::jellyfin::SERVER_ID` (raw slot 0);
//!   every Plex slot is permanently empty.
//! * [`same_item`] — THE item-identity rule every server-scoped key comparison goes through.
//! * [`current`]/[`ids`] — the reads, honest over an always-empty Plex roster:
//!   `current()` answers [`ServerId::UNSET`], `ids()` yields nothing.
//! * the generation counters ([`roster_gen`]/[`facts_gen`]) and the client-generation sequence
//!   ([`capture_generation_seed`]) the persisted bootstrap vocabulary carries.
//! * [`revoke_all`] — sign-out retires whatever state there is (always the empty roster today)
//!   and stays the one funnel every account teardown goes through.
//! * [`register_origin`] — the retired registration's shape, kept only so `session.rs`'s
//!   frame-boundary test keeps its subject until that slice lands.
//!
//! **Keyed by slot number.** A slot number names one server for the life of the process; the
//! per-server stores (`search`'s mailboxes, `serverinfo`, `person`'s per-source records) are flat
//! arrays indexed by [`ServerId::raw`]. The reserved [`ServerId::UNSET`] is the pre-registration
//! state and never resolves to anything.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
#[cfg(test)]
use crate::origin::{Origin, ResolvePin};
use std::sync::Mutex;

/// The registration-time connection facts went with the wire layer (`server::Client` carried the
/// tier/IP atomics they fed). The name survives only because `session.rs`'s frame-boundary test
/// still spells a full registration call; the future `session.rs` transition slice retires both.
#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ConnectionFacts;

/// Slot ceiling. A Plex account's server list is a handful (own + shared); this is the number
/// that decides which slot ids can ever exist.
///
/// `pub` because [`crate::server::serverinfo`] holds one entry PER SERVER in a flat array indexed
/// by [`ServerId::raw`], and a second, independent ceiling there is a silent out-of-bounds
/// waiting to happen: this is the number that decides which ids can ever exist. `crate::person`
/// keys its per-source mailboxes the same way and is OUTSIDE this module tree, which is what
/// moved this from `pub(super)` — a store that guessed its own 16 would go out of bounds the day
/// this number moved.
pub const MAX_SERVERS: usize = 16;

/// A registry slot — a small `Copy` handle that names a server without borrowing it. Stable for
/// the life of the process, so it can sit in UI state, a route, or a queued job.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct ServerId(u16);

impl ServerId {
    /// The reserved "no server" value — what [`current`] answers on every boot of the
    /// single-source app, and what a host test builds by literal when it needs an id no store can
    /// resolve. Never resolves to a client.
    pub const UNSET: ServerId = ServerId(u16::MAX);

    pub fn is_set(self) -> bool {
        self.0 != Self::UNSET.0
    }
    /// The raw slot number, for logging/persistence. Round trips through [`ServerId::from_raw`].
    pub const fn raw(self) -> u16 {
        self.0
    }
    /// `const` so a fixed slot can name a server in a `const` — which is what lets the host tests
    /// that grade the identity rules build one without a registry, a socket or the serial lock.
    pub const fn from_raw(v: u16) -> ServerId {
        ServerId(v)
    }
}

impl Default for ServerId {
    fn default() -> Self {
        Self::UNSET
    }
}

/// **THE item-identity rule**: do two server-scoped keys name the same item?
///
/// Every `ratingKey`, `librarySectionID`, `Part.key`, `Stream.id` and `personId` Plex issues is a
/// server-local integer dense from 1, so with two servers registered a bare key names an item on
/// *neither* of them in particular. Two servers colliding is therefore the NORMAL case, not an edge
/// one — it is measured (docs/shared-servers.md §2): the share's only section key is `1`, exactly
/// like our own `Movies`, and its ratingKeys start at 1 alongside ours. Anything that stores an
/// item and later looks it up again — the home catalog, the loaded detail, the playing-item store,
/// the BACK trail, a play queue — has to compare the PAIR, and they all compare it here so the
/// answer cannot drift between them.
///
/// [`ServerId::UNSET`] matches only itself, and that is the pre-registry state rather than a
/// wildcard: a row parsed before any server was installed (and every row a host test builds by
/// literal) carries UNSET, so UNSET-vs-UNSET is the single-server app behaving exactly as it did.
/// UNSET vs a real slot is deliberately NOT a match — a "server unknown" row must not be handed to
/// a caller asking about a specific machine, because the whole failure this rule exists to stop is
/// a confident answer about the wrong one. Every such miss degrades safely at its call site (a
/// catalog miss opens the page off-catalog, a playing-item miss costs one PMS fetch), which is the
/// property that makes strict equality the safe default here.
pub fn same_item(a: (ServerId, &str), b: (ServerId, &str)) -> bool {
    a.0 == b.0 && a.1 == b.1
}

/// Slots populated so far — the high-water mark, MONOTONE for the life of the process. Nothing
/// raises it any more (the registration world is retired), so the roster it bounds is empty; the
/// static stays because [`revoke_all`] and the test reset still speak the same state machine.
static COUNT: AtomicUsize = AtomicUsize::new(0);
/// Which slots in the current account window are granted to the active profile. Always zero
/// today; see [`COUNT`].
static ACTIVE: AtomicU32 = AtomicU32::new(0);
/// Monotone epoch of the active roster's identity. It moved when a slot appeared/disappeared or
/// was re-pointed; nothing moves it today, but cached fan-out stores (`search`, `person`,
/// `metadata`'s projections, `pms`'s source table) still read it as "the server set has not
/// changed", and [`revoke_all`] still bumps it so a sign-out visibly retires whatever a cache
/// projected.
static ROSTER_GEN: AtomicU32 = AtomicU32::new(1);
/// Monotone epoch of the published roster facts — it moved on every description commit, and on
/// nothing else. Nothing commits descriptions today, so this stays at 1; the caches that key on
/// it (`pms`'s facts fingerprint, `metadata`'s owner re-stamp) keep reading it unchanged.
static FACTS_GEN: AtomicU32 = AtomicU32::new(1);
/// The first slot that still belongs to the signed-in account. Raised by [`revoke_all`];
/// everything below it is a credential of an account that has left.
static FLOOR: AtomicUsize = AtomicUsize::new(0);
/// The current/primary server as a raw `ServerId` — what `current()` answers with. The
/// registration world that wrote it is retired, so this rests at [`ServerId::UNSET`]; it is
/// reset by [`revoke_all`] and the test reset like every other piece of roster state.
static CURRENT: AtomicU32 = AtomicU32::new(ServerId::UNSET.0 as u32);
/// Writers only (sign-out / reset). Readers never take it — that is the whole point.
static WRITE: Mutex<()> = Mutex::new(());

/// The source of every client generation value, process-wide. Only the UNIQUENESS matters — it is
/// never compared for order. The clients that consumed it are retired (#166); the sequence
/// survives because the persisted bootstrap vocabulary (`Initial.primary_client`) captures and
/// restores it, and the two vocabularies (a captured seed >= 1, the dev Jellyfin world's 0) must
/// never collide.
static GEN_SEQ: AtomicU32 = AtomicU32::new(1);

/// Called only before constructing controlled resources in an empty registry. Read the sequence
/// the retired clients issued generations from, so a replay can restore it byte-identically.
pub(crate) fn capture_generation_seed() -> u32 {
    GEN_SEQ.load(Ordering::Relaxed)
}

/// Restore a captured generation seed. Refuses `0`: that is the dev world's "no client ever
/// minted" marker, and storing it would let a later minted generation collide with the value a
/// captured vocabulary means.
pub(crate) fn restore_generation_seed(seed: u32) -> Result<(), &'static str> {
    if seed == 0 {
        return Err("invalid client initialization boundary");
    }
    GEN_SEQ.store(seed, Ordering::Relaxed);
    Ok(())
}

// ---- reads ----

/// Which server `client()` used to answer with. [`ServerId::UNSET`] on every boot of the
/// single-source app: the installation path that wrote this retired with the Plex wire layer, so
/// the current-server slot is permanently unnamed. Callers that need the LIVE backend name
/// `crate::jellyfin::SERVER_ID` directly.
///
/// **`Acquire`** for the store side that no longer exists is still the right order here: the
/// read is on poster/resolution worker paths, and the relaxation of one atomic buys nothing.
pub fn current() -> ServerId {
    ServerId(CURRENT.load(Ordering::Acquire) as u16)
}

pub fn roster_gen() -> u32 {
    ROSTER_GEN.load(Ordering::Acquire)
}

/// The facts epoch — read it beside [`roster_gen`] when a cache projects an AUTHORITATIVE
/// description (the "Shared by …" credit, and the machine name as a describer stated it) and not
/// merely which servers are in the table. Constant at 1 today; see the static's doc.
pub fn facts_gen() -> u32 {
    FACTS_GEN.load(Ordering::Acquire)
}

/// Every LIVE slot, in registration order — **the granted roster**. With registration retired
/// (#166) the roster is always empty; the walk keeps its exact shape (floor-bounded, active-bit
/// filtered, ordered) because `browse`'s discovery wire, `search`'s fan-out, `person`'s sources
/// and `metadata`'s alt panel all read it and must keep reading "no Plex sources" rather than a
/// guessed slot.
pub fn ids() -> impl Iterator<Item = ServerId> {
    let hi = COUNT.load(Ordering::Acquire);
    let active = ACTIVE.load(Ordering::Acquire);
    (FLOOR.load(Ordering::Acquire).min(hi)..hi)
        .filter(move |&i| active & (1u32 << i) != 0)
        .map(|i| ServerId(i as u16))
}

/// The active slot whose machine id is `machine_id`, if any. Machine ids lived on the retired
/// `server::Client`s, so this answers `None` for every input; the signature stays for the one
/// caller (the plaintext alert's subject resolution) until the plaintext slice retires it.
pub(crate) fn id_of_machine(_machine_id: &str) -> Option<ServerId> {
    None
}

// ---- writes ----

/// **Sign-out.** Retire every registered server: `current()` answers with nothing again, [`ids`]
/// walks nothing, and the roster epochs move so every cache that projected the roster rebuilds
/// empty.
///
/// The token-blanking half the old table performed died with the clients — a `&'static Client`
/// can no longer exist to hold a credential — but the funnel stays: every account teardown
/// (`app::adapters::session`'s `begin_erase`) passes through here, and the state machine it
/// resets is the one the whole app reads.
///
/// It does NOT free anything and does not lower [`COUNT`] — slot numbers are never handed out
/// twice, which is what keeps a stale [`ServerId`] filed under the account that left from
/// silently naming the next account's server.
pub(crate) fn revoke_all() {
    // Same crate-global registry guard the registration paths held — see `lib.rs::testlock`.
    #[cfg(test)]
    crate::testlock::assert_held("the plex server registry (revoke_all)");
    // Every plaintext grant dies with the identity that consented.
    crate::grant::identity_changed();
    let _w = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let n = COUNT.load(Ordering::Acquire);
    let floor = FLOOR.load(Ordering::Acquire);
    // CURRENT first: for the instant between these two stores a reader must never see an id whose
    // slot the floor has already killed.
    CURRENT.store(ServerId::UNSET.0 as u32, Ordering::Release);
    ACTIVE.store(0, Ordering::Release);
    FLOOR.store(n, Ordering::Release);
    ROSTER_GEN.fetch_add(1, Ordering::AcqRel);
    if n > floor {
        crate::log(&format!(
            "plex: {} server(s) revoked - signed out",
            n - floor
        ));
    }
    // The Sources list, the Search scope line and every shelf heading are drawn from this table,
    // and `ui::idle` gates the whole present on detected motion — it cannot see a `static` being
    // written.
    crate::ui::idle::invalidate();
}

/// Register (or update) a server, returning its stable id.
///
/// **RETIRED SHAPE.** The registration world this entry point drove — slot lookup, re-point,
/// credential admission, the leaked `server::Client` — went with the wire layer (#166): no product
/// path can publish a Plex client since the dispatch collapse (#202), and the only remaining
/// caller is `session.rs`'s frame-boundary test, which grades that a registration call inside a
/// frame resolves the session identity through the NON-BLOCKING cache and never touches a file.
/// That boundary contract is what this shell keeps alive; the `session.rs` transition slice
/// retires it together with that suite. Always [`ServerId::UNSET`]: there is no slot to compute.
#[cfg(test)]
pub(crate) fn register_origin(
    _machine_id: &str,
    _origin: &Origin,
    _token: &str,
    _pin: Option<&ResolvePin>,
    _connection: ConnectionFacts,
) -> ServerId {
    let _identity = crate::server::session::peek().client_id.clone();
    ServerId::UNSET
}

/// Empty the table so each test starts from "nothing installed". Must be called under
/// [`crate::testlock::serial`] — the registry is a crate global.
///
/// The neutral reset the whole suite still uses: roster state (`COUNT`/`ACTIVE`/`FLOOR`/
/// `CURRENT`) and both epochs go back to their boot values. The client
/// generation sequence is deliberately NOT reset — the seeds a test restores are process-global
/// vocabulary, and `restore_generation_seed` refuses `0` for the same reason.
#[cfg(test)]
pub(crate) fn reset_for_test() {
    crate::testlock::assert_held("the plex server registry (reset)");
    let _w = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    COUNT.store(0, Ordering::Release);
    ACTIVE.store(0, Ordering::Release);
    ROSTER_GEN.store(1, Ordering::Release);
    FACTS_GEN.store(1, Ordering::Release);
    // The floor goes back with the count, or every test after one that signed out would register
    // into slots the walk starts above — a table that reads as empty however much is in it.
    FLOOR.store(0, Ordering::Release);
    CURRENT.store(ServerId::UNSET.0 as u32, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The identity rule the whole app's stored rows compare on. The case it exists for is the
    /// FIRST assertion: the share's ratingKeys are dense from 1 exactly like ours, so a bare key
    /// matching is the bug, not the feature. No registry state is touched, so no lock is needed.
    #[test]
    fn one_rating_key_on_two_servers_is_two_different_items() {
        let (a, b) = (ServerId::from_raw(0), ServerId::from_raw(1));
        assert!(
            !same_item((a, "1"), (b, "1")),
            "the same key on two servers is two items"
        );
        assert!(
            same_item((a, "1"), (a, "1")),
            "…and the same key on ONE server is one item"
        );
        assert!(
            !same_item((a, "1"), (a, "2")),
            "a different key is a different item"
        );

        // UNSET is the pre-registry / host-test state: it matches itself (the single-server app,
        // unchanged) and nothing else — a row whose server is unknown must never answer for a
        // named one.
        assert!(same_item((ServerId::UNSET, "1"), (ServerId::UNSET, "1")));
        assert!(!same_item((ServerId::UNSET, "1"), (a, "1")));
        assert!(!same_item((a, "1"), (ServerId::UNSET, "1")));
    }

    /// The retired registry's whole contract, pinned in one place: every read answers the empty
    /// roster honestly, sign-out on an empty roster moves the epochs and nothing else, and the
    /// generation seed boundary refuses the dev world's zero.
    #[test]
    fn the_retired_registry_answers_an_empty_roster() {
        let _g = crate::testlock::serial();
        reset_for_test();
        assert!(!current().is_set(), "nothing is current on a fresh registry");
        assert_eq!(ids().collect::<Vec<_>>(), Vec::new(), "the roster walk yields nothing");
        assert_eq!(id_of_machine("mach-A"), None, "no client carries a machine id any more");
        assert!(roster_gen() >= 1 && facts_gen() >= 1, "the epochs start at 1");

        revoke_all();
        assert!(!current().is_set(), "sign-out on an empty roster changes no current server");
        assert_eq!(ids().collect::<Vec<_>>(), Vec::new());
        assert!(roster_gen() > 1, "the roster epoch still moves on a sign-out");

        let seed = capture_generation_seed();
        assert!(seed >= 1, "the generation sequence starts at 1, never 0");
        assert_eq!(
            restore_generation_seed(0).unwrap_err(),
            "invalid client initialization boundary",
            "the dev world's zero marker must never be stored"
        );
        restore_generation_seed(seed + 7).expect("a nonzero seed restores");
        assert_eq!(capture_generation_seed(), seed + 7);
        reset_for_test();
    }
}
