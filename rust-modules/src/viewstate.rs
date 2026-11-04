//! viewstate — the server-side **view-state writes**, off the SDL thread.
//!
//! Three actions share one shape: Mark as Watched, Mark as Unwatched and Remove from
//! Continue Watching. Each is one request to the item's server, followed by a refresh of every
//! surface that describes that item - Home's shelves always, plus the detail page when the press
//! came from it.
//!
//! ## Why this module exists
//!
//! With the Jellyfin-only conversion there is one backend and one server, so a write is exactly
//! one request: the fan-out that used to repeat a watched claim across every share that held
//! the title is gone with the old wire protocol, and the item's identity is simply the `(sid, rk)`
//! pair the press carried. What remains is the discipline the queue existed for:
//!
//! * **The press is optimistic.** [`request`] flips what the shelves and the loaded detail item say
//!   about the item BEFORE anything leaves the machine ([`crate::catalog::edit_item`],
//!   [`crate::metadata::set_watched_local`]), so the tick, the veil and the resume bar all change on
//!   the frame of the press however far away the server is. The refresh that follows the write is
//!   the reconcile: it is the server's own answer, and it silently corrects an optimistic edit that
//!   turned out to be wrong. A server that never answers leaves the edit standing — the same "a
//!   failure never blanks a populated Home" rule `catalog.rs` keeps for a fetch, applied to a write.
//! * **One write at a time, in order.** These are not idempotent reads: a watched/unwatched pair
//!   landing out of order leaves the server saying the opposite of what the user last asked for. So
//!   the queue is drained strictly one worker at a time, and a second press on the SAME item
//!   replaces the queued one rather than joining it ([`coalesce`]) — the newest intent for an item
//!   is the only one worth a round trip.
//! * **One refresh per burst.** The hub refetch (and the detail re-read) is deferred until the queue
//!   empties, so ticking four episodes watched in a row costs four writes and one refresh.
//! * **A refused spawn is a retry, never a panic and never a fallback to blocking.**
//!   `task::spawn_small` returning false means the OS is out of threads; the queue keeps its head
//!   and [`pump`] tries again after [`RETRY_FRAMES`], the same degradation `search.rs` and
//!   `browse.rs` apply to a refused fetch.
//!
//! [`Write::RemoveFromDeck`] stays a hide, not a reset: the resume point survives and only the deck
//! row goes, while [`Write::Unwatched`] discards both the watched flag and the resume position, the
//! same thing an unscrobble always did to the copy in hand.
//!
//! Physical ownership now follows Browse's pattern: `stores::viewstate::ViewStateStore` owns this
//! module's main-thread `ViewStateState`, an `Arc<ViewStateAdapter>` mailbox captured by every
//! worker, and its notice. Reset rotates the adapter, and request IDs fence malformed or stale
//! completions even within one adapter. There is no process-global ViewState transport or state.

use crate::registry::ServerId;
use std::panic::catch_unwind;
use std::sync::{Arc, Mutex};

/// What to ask the item's server for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Write {
    /// Mark watched - plays state on, resume position discarded.
    Watched,
    /// Mark unwatched; clears played state **and** the resume point.
    Unwatched,
    /// HIDE from the Continue Watching deck, keeping the resume point (the documented
    /// resume-point merge, `clear_resume`).
    RemoveFromDeck,
    /// Jellyfin's heart: `POST/DELETE /Users/{uid}/FavoriteItems/{id}`.
    Favorite(bool),
}

impl Write {
    /// The toggle this write belongs to. [`coalesce`] replaces a queued write only within one
    /// family: Watched and Unwatched are two ends of ONE control, so a later press supersedes an
    /// earlier one, while a deck removal is a different decision about the same item and must not
    /// be swallowed by a watched toggle that happened to follow it.
    fn family(self) -> u8 {
        match self {
            Write::Watched | Write::Unwatched => 0,
            Write::RemoveFromDeck => 1,
            // A heart toggle is its own decision about the item: a later favorite supersedes an
            // earlier one, and it must not be swallowed by a watched toggle that followed it.
            Write::Favorite(_) => 2,
        }
    }

    /// The Jellyfin flavor's `perform`: the same queue, the backend's client. Played-state
    /// and favorite writes ride `PlayedItems`/`FavoriteItems`; the deck removal is the documented
    /// resume-point merge (`clear_resume`).
    fn perform_jellyfin(self, c: &crate::jellyfin::JfClient, rk: &str) -> bool {
        match self {
            Write::Watched => c.mark_played(rk),
            Write::Unwatched => c.mark_unplayed(rk),
            Write::RemoveFromDeck => c.clear_resume(rk),
            Write::Favorite(want) => c.set_favorite(rk, want),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Write::Favorite(on) => if on { "favorite" } else { "unfavorite" },
            Write::Watched => "watched",
            Write::Unwatched => "unwatched",
            Write::RemoveFromDeck => "deck-remove",
        }
    }
}

/// One queued write. Identity only — the backend's client is resolved in [`kick`], on the main
/// thread, at the moment the worker is spawned. `catalog::kick`'s rule ("capture at the spawn site")
/// says the worker must never ask which server is current; resolving the client there is that
/// rule applied to the one live backend.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RequestId(u64);

struct Req {
    id: RequestId,
    sid: ServerId,
    rk: String,
    w: Write,
    /// Re-read the originating Detail page when this lands, by its exact `(sid, rk)` address.
    /// `keep` retains a filmstrip episode; `None` here means the press came from another screen.
    detail: Option<crate::stores::viewstate::DetailRefresh>,
}

/// The worker's ONLY output: whether the server took the write it was given.
#[derive(Default)]
struct Done {
    ok: bool,
}

struct Completion {
    id: RequestId,
    done: Done,
}

/// Cross-thread transport for one physical owner. Every worker captures the exact adapter that
/// admitted it; reset replaces the owner's `Arc`, fencing any completion still headed here.
#[derive(Default)]
pub(crate) struct ViewStateAdapter {
    mail: Mutex<Option<Completion>>,
}

/// Main-thread state for one ViewState owner. Nothing in this value selects another owner.
pub(crate) struct ViewStateState {
    queue: Vec<Req>,
    sent: Option<Req>,
    retry_cd: u32,
    want_hubs: bool,
    want_detail: Option<crate::stores::viewstate::DetailRefresh>,
    next_request_id: u64,
}

impl Default for ViewStateState {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            sent: None,
            retry_cd: 0,
            want_hubs: false,
            want_detail: None,
            next_request_id: 1,
        }
    }
}

/// ~2 s at 60 fps — the same wait `search.rs` gives a refused spawn. Without it, a process at the
/// thread ceiling would re-attempt (and log a refusal) every single frame. Only the jellyfin
/// transport arms a refusal, so the constant is read only behind that feature.
const RETRY_FRAMES: u32 = 120;

impl ViewStateState {
    fn mint_request_id(&mut self) -> RequestId {
        let id = RequestId(self.next_request_id);
        self.next_request_id = self.next_request_id.checked_add(1)
            .expect("viewstate request identity exhausted");
        id
    }

    /// Is a write queued or in flight? What gates the deferred refresh, and the predicate any
    /// future "saving…" affordance would read.
    pub(crate) fn is_busy(&self) -> bool {
        !self.queue.is_empty() || self.sent.is_some()
    }

/// Ask the item's server to change `(sid, rk)`'s view state, and update every surface that describes
/// it AT ONCE. **MAIN THREAD, NON-BLOCKING.** Returns false when the write never left — the one case
/// being a server slot that holds no client, which is a dropped write and says so in the log
/// rather than being the one that goes unrecorded.
///
/// `detail` says exactly what to re-read when the write lands: `None` for a press outside Detail,
/// an addressed page plus kept episode for its filmstrip, and the addressed page with no episode
/// for the hero toggle. The hub refetch happens either way — Continue Watching changes with any of
/// these three writes, whichever screen asked for it.
    fn request(
    &mut self,
    adapter: &Arc<ViewStateAdapter>,
    sid: ServerId,
    rk: &str,
    w: Write,
    detail: Option<crate::stores::viewstate::DetailRefresh>,
    browse: &mut dyn FnMut(crate::stores::browse::BrowseCmd) -> bool,
    hubs: &mut dyn FnMut(crate::stores::hubs::HubsCmd) -> crate::stores::StoreOutcome,
    person: &mut dyn FnMut(crate::stores::person::PersonCmd) -> bool,
    collection: &mut dyn FnMut(crate::stores::collection::CollectionCmd) -> bool,
    search: &mut dyn FnMut(crate::stores::search::SearchCmd) -> bool,
    metadata: &mut dyn FnMut(crate::stores::metadata::MetadataCmd) -> bool,
) -> bool {
    // One backend, one guard: a write flies only when the sid names the Jellyfin slot AND the
    // Jellyfin client is signed in. Any other sid names a retired slot that no client can
    // ever fill (#166) — a view-state write is exactly the operation to skip and log rather than
    // guess a destination for.
    let jellyfin = sid == crate::jellyfin::SERVER_ID && crate::jellyfin::client().is_some();
    #[cfg(any())]
    let jellyfin = false;
    if !jellyfin {
        crate::log(&format!(
            "viewstate: rk={rk} {} DROPPED — server {} is not registered",
            w.name(),
            sid.raw()
        ));
        return false;
    }
    // OPTIMISTIC, before the request: the press must land on the panel now, not one WAN round trip
    // from now. Only THIS copy — the other sources' keys are not known until the fan-out resolves
    // them, which is what [`pump`] finishes the job with.
    edit_local_with_owners(sid, rk, w, browse, hubs, person, collection, search, metadata);
    self.coalesce(sid, rk, w);
    let id = self.mint_request_id();
    self.queue.push(Req {
        id,
        sid,
        rk: rk.to_string(),
        w,
        detail,
    });
    self.kick(adapter);
    true
    }
}

/// Flip every LOCAL surface that describes `(sid, rk)` to what `w` is about to make true, and ask
/// for a repaint. Both edits are no-ops where the item does not appear, so a Home press with no
/// detail page mounted costs one catalog walk and nothing else — and those two ARE every surface
/// that holds an editable copy: the Library grid, Search and the person page re-read rather than
/// cache a watch flag.
///
/// Called from two places, for one reason. [`request`] calls it optimistically for the copy the
/// user pressed, and [`pump`] calls it for each OTHER source's copy the fan-out reached — which
/// cannot be known any earlier than that, because discovering them is the round trip.
fn edit_local_with_owners(
    sid: ServerId,
    rk: &str,
    w: Write,
    browse: &mut dyn FnMut(crate::stores::browse::BrowseCmd) -> bool,
    hubs: &mut dyn FnMut(crate::stores::hubs::HubsCmd) -> crate::stores::StoreOutcome,
    person: &mut dyn FnMut(crate::stores::person::PersonCmd) -> bool,
    collection: &mut dyn FnMut(crate::stores::collection::CollectionCmd) -> bool,
    search: &mut dyn FnMut(crate::stores::search::SearchCmd) -> bool,
    metadata: &mut dyn FnMut(crate::stores::metadata::MetadataCmd) -> bool,
) {
    match w {
        Write::Watched | Write::Unwatched => {
            let on = w == Write::Watched;
            // **Every store that can be DRAWING the item, not just the hub catalog.** The first two
            // lines were the whole edit while the context menu opened on Home and the detail page
            // alone; it opens on the Library grid, a Search result, a person's filmography and — since
            // 2026-08-21 — the detail page's Related shelf, and each of those keeps its own rows.
            // Left at two, the press wrote correctly to the server and changed nothing on the screen
            // the user was looking at until a refetch, which reads as the row having done nothing.
            //
            // `metadata::set_watched_local` covers TWO of those surfaces, which is why the list below
            // is six calls and not seven: the detail page holds the loaded item, its season's episodes
            // AND the Related tiles, and that one call walks all three.
            //
            // All six are no-ops where the item does not appear — a walk of an empty or unrelated
            // store — so a press from any one screen still costs about what it did.
            //
            // Through the store VOCABULARY (`crate::stores`, restructure phase 4) rather than the
            // data modules directly, so every store a press flips raises its own notice and a
            // migrated screen hears the optimistic edit the same way it hears a landing.
            hubs(crate::stores::hubs::HubsCmd::EditItem {
                sid,
                rk: rk.to_string(),
                edit: crate::catalog::LocalEdit::Watched(on),
            }).changed;
            metadata(crate::stores::metadata::MetadataCmd::SetWatchedLocal {
                sid,
                rk: rk.to_string(),
                on,
            });
            browse(crate::stores::browse::BrowseCmd::SetWatchedLocal {
                sid,
                rk: rk.to_string(),
                on,
            });
            search(crate::stores::search::SearchCmd::SetWatchedLocal {
                sid,
                rk: rk.to_string(),
                on,
            });
            person(crate::stores::person::PersonCmd::SetWatchedLocal {
                sid,
                rk: rk.to_string(),
                on,
            });
            collection(crate::stores::collection::CollectionCmd::SetWatchedLocal {
                sid,
                rk: rk.to_string(),
                on,
            });
        }
        // The one edit that can be stated with certainty: the server hides the item from the deck
        // and keeps everything else about it (`server::Client::remove_from_continue_watching`).
        Write::Favorite(on) => {
            // The heart is only drawn on the detail page: the loaded item's flag. The show-level
            // roll-up the watched pair performs is NOT wanted here — a heart has no children.
            metadata(crate::stores::metadata::MetadataCmd::SetFavoriteLocal {
                sid, rk: rk.to_string(), on,
            });
        }
        Write::RemoveFromDeck => {
            hubs(crate::stores::hubs::HubsCmd::EditItem {
                sid,
                rk: rk.to_string(),
                edit: crate::catalog::LocalEdit::LeftTheDeck,
            }).changed;
            // …and the LIBRARY's own deck, which is a different shelf on a different screen and is
            // where this row is now reachable from at all (the Library's section Continue Watching
            // shelf, 2026-09-05).
            browse(crate::stores::browse::BrowseCmd::LeftTheDeck {
                sid,
                rk: rk.to_string(),
            });
        }
    }
    crate::ui::idle::invalidate(); // the tick/veil/bar just changed with no spring behind it
}

/// Drop any QUEUED write for the same item and toggle — see [`Write::family`]. The write already in
/// flight (`ViewStateState::sent`) is deliberately untouched: it is on the wire, the client cannot
/// recall it, and
/// the one that supersedes it is queued behind it in the order pressed.
impl ViewStateState {
    fn coalesce(&mut self, sid: ServerId, rk: &str, w: Write) {
    let fam = w.family();
    self.queue.retain(|r| !(crate::registry::same_item((r.sid, &r.rk), (sid, rk)) && r.w.family() == fam));
    }

/// Spend one frame of the refused-spawn ladder; true when the next attempt is due. Split out so the
/// ladder is gradeable without a registry or a socket — `catalog::retry_due`'s twin.
    fn retry_tick(&mut self) -> bool {
        let cd = self.retry_cd;
        if cd > 0 {
            self.retry_cd = cd - 1;
        }
        cd <= 1
    }

/// Start the next queued write if nothing is out. MAIN THREAD.
/// The backend client only exists behind the `jellyfin` feature; without it every queued write
/// drains through the DROPPED arm below and the adapter has nothing to feed.
    fn kick(&mut self, adapter: &Arc<ViewStateAdapter>) {
        if self.sent.is_some() || self.retry_cd > 0 {
            return;
        }
    while !self.queue.is_empty() {
        let req = self.queue.remove(0);
        // Resolved HERE, at the spawn site, and never inside the worker. A slot that has since
        // stopped holding a client is a write with nowhere to go: dropping it silently would be the
        // one write with no line at all, which is what the request-time check above exists to
        // prevent, so it says so here too.
        let jellyfin = req.sid == crate::jellyfin::SERVER_ID;
        if jellyfin {
            let Some(c) = crate::jellyfin::client() else {
                crate::log(&format!(
                    "viewstate: rk={} {} DROPPED — the jellyfin client left while queued",
                    req.rk,
                    req.w.name(),
                ));
                continue;
            };
            let (id, rk, w) = (req.id, req.rk.clone(), req.w);
            let worker_adapter = Arc::clone(adapter);
            let spawned = crate::task::spawn_small("viewstate", move || {
                let done = catch_unwind(move || w.perform_jellyfin(c, &rk));
                *worker_adapter.mail.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(Completion { id, done: Done { ok: done.unwrap_or(false) } });
            });
            if !spawned {
                // Nothing will ever fill the mailbox. Put the write back at the HEAD, exactly as
                // the retired arm did, and wait out the ladder before trying again.
                self.queue.insert(0, req);
                self.retry_cd = RETRY_FRAMES;
                crate::log("viewstate: jellyfin write refused at spawn");
                return;
            }
            // The in-flight identity is what the completion matches against — without it the
            // answer lands as "ignored: in-flight identity is none" and the queue jams behind it.
            self.sent = Some(req);
            continue;
        }
        // Non-Jellyfin sids cannot be queued (`request` drops them) and no other backend exists,
        // but a queued write's sid is still checked here rather than assumed: an impossible request
        // that DID reach the queue is dropped with a line instead of jamming the queue behind it.
        crate::log(&format!(
            "viewstate: rk={} {} DROPPED — no backend for server {}",
            req.rk,
            req.w.name(),
            req.sid.raw()
        ));
        continue;
    }
    }

/// MAIN THREAD, once a frame, ROUTE-UNCONDITIONAL — a landing must never depend on which screen is
/// mounted, because the user can walk off Home (or off the detail page) between the press and the
/// answer, and the refresh is owed either way.
pub(crate) fn pump_with_gate(
    &mut self,
    adapter: &Arc<ViewStateAdapter>,
    gate: &crate::ui::landgate::Gate,
    browse: &mut dyn FnMut(crate::stores::browse::BrowseCmd) -> bool,
    hubs: &mut dyn FnMut(crate::stores::hubs::HubsCmd) -> crate::stores::StoreOutcome,
) {
    let due = self.retry_tick();
    // the landing GATE (§3.3 step 3, `ui::landgate`): under a replay the server's answer is taken
    // on the frame the recording took it on. The retry tick and `kick` below stay outside it.
    let landed = crate::stores::take_landing(gate, crate::stores::StoreId::ViewState, || {
        adapter.mail.lock().unwrap_or_else(|e| e.into_inner()).take()
    });
    if let Some(completion) = landed {
        let exact = self.sent.as_ref().is_some_and(|request| request.id == completion.id);
        if exact {
            let r = self.sent.take().expect("the exact in-flight request remains present");
            let done = completion.done;
            crate::log(&format!(
                "viewstate: rk={} {} ok={}",
                r.rk,
                r.w.name(),
                done.ok as i32
            ));
            // The refresh is owed whether or not the server took it: on success it is the reconcile,
            // and on failure it is what puts the optimistic edit back to whatever the server really
            // says. A server that can answer neither question leaves the shelves exactly as they
            // are — `catalog.rs`'s per-source rule, unchanged.
            self.want_hubs = true;
            if let Some(detail) = r.detail {
                self.want_detail = Some(detail);
            }
        } else {
            crate::log(&format!(
                "viewstate: completion {} ignored — in-flight identity is {}",
                completion.id.0,
                self.sent.as_ref().map(|request| request.id.0.to_string())
                    .unwrap_or_else(|| "none".into()),
            ));
        }
    }
    if due {
        self.kick(adapter);
    }
    if self.is_busy() {
        return; // a burst still has writes to send
    }
    let refresh_hubs = std::mem::take(&mut self.want_hubs);
    if refresh_hubs {
        let _refetched = hubs(crate::stores::hubs::HubsCmd::RefetchHubs);
        // the same staleness, one screen over: a library's own shelves carry watch state and its
        // own Continue Watching row, so the burst that made Home's hubs stale made these stale too
        browse(crate::stores::browse::BrowseCmd::HubsInvalidateAll);
        crate::ui::idle::invalidate();
    }
}

#[cfg(test)]
pub(crate) fn pump(
    &mut self,
    adapter: &Arc<ViewStateAdapter>,
    browse: &mut dyn FnMut(crate::stores::browse::BrowseCmd) -> bool,
    hubs: &mut dyn FnMut(crate::stores::hubs::HubsCmd) -> crate::stores::StoreOutcome,
) {
    self.pump_with_gate(adapter, crate::ui::landgate::fixture_gate(), browse, hubs)
}

/// Drain the addressed Detail reconciliation only after this owner's burst is idle.
pub(crate) fn take_detail_refresh(&mut self) -> Option<crate::stores::viewstate::DetailRefresh> {
    if self.is_busy() { return None; }
    self.want_detail.take()
}

#[cfg(test)]
pub(crate) fn hold_inflight_for_test(&mut self, sid: ServerId, rk: &str) {
    let id = self.mint_request_id();
    self.sent = Some(Req {
        id,
        sid,
        rk: rk.into(),
        w: Write::Watched,
        detail: None,
    });
}
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OwnershipFixture {
    pub(crate) queue: Vec<String>,
    pub(crate) sent: Option<String>,
    pub(crate) retry_cd: u32,
    pub(crate) want_hubs: bool,
    pub(crate) want_detail: Option<crate::stores::viewstate::DetailRefresh>,
    pub(crate) mail: bool,
}

#[cfg(test)]
impl ViewStateState {
    fn fixture_req(&mut self, rk: &str) -> Req {
        let id = self.mint_request_id();
        Req {
            id,
            sid: ServerId::UNSET,
            rk: rk.into(),
            w: Write::Watched,
            detail: None,
        }
    }

    pub(crate) fn seed_ownership_fixture(&mut self, adapter: &Arc<ViewStateAdapter>) {
        self.reset();
        let queued = self.fixture_req("queued");
        let flight = self.fixture_req("flight");
        let id = flight.id;
        self.queue.push(queued);
        self.sent = Some(flight);
        self.retry_cd = 9;
        self.want_hubs = true;
        self.want_detail = Some(crate::stores::viewstate::DetailRefresh {
            sid: ServerId::UNSET,
            rk: "detail".into(),
            keep: Some("episode".into()),
        });
        *adapter.mail.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Completion { id, done: Done::default() });
    }

    pub(crate) fn ownership_fixture(&self, adapter: &Arc<ViewStateAdapter>) -> OwnershipFixture {
        OwnershipFixture {
            queue: self.queue.iter().map(|r| r.rk.clone()).collect(),
            sent: self.sent.as_ref().map(|r| r.rk.clone()),
            retry_cd: self.retry_cd,
            want_hubs: self.want_hubs,
            want_detail: self.want_detail.clone(),
            mail: adapter.mail.lock().unwrap_or_else(|e| e.into_inner()).is_some(),
        }
    }

    pub(crate) fn late_completion(
        &self,
        adapter: &Arc<ViewStateAdapter>,
    ) -> Box<dyn FnOnce()> {
        let id = self.sent.as_ref().expect("an old request is held").id;
        let adapter = Arc::clone(adapter);
        Box::new(move || {
            *adapter.mail.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(Completion { id, done: Done::default() });
        })
    }

    pub(crate) fn seed_post_reset_flight(&mut self) {
        let req = self.fixture_req("post-reset");
        self.sent = Some(req);
    }

}

impl ViewStateState {
/// Drop everything — the identity-change twin of `catalog::reset`/`browse::reset`, called from the same
/// place. The request counter deliberately survives: identities remain monotone across reset, while
/// the owning store rotates the adapter so an old worker has no route into the replacement owner.
fn reset(&mut self) {
    self.queue.clear();
    self.sent = None;
    self.retry_cd = 0;
    self.want_hubs = false;
    self.want_detail = None;
}

pub(crate) fn run(
    &mut self,
    adapter: &Arc<ViewStateAdapter>,
    cmd: crate::stores::viewstate::ViewStateCmd,
    browse: &mut dyn FnMut(crate::stores::browse::BrowseCmd) -> bool,
    hubs: &mut dyn FnMut(crate::stores::hubs::HubsCmd) -> crate::stores::StoreOutcome,
    person: &mut dyn FnMut(crate::stores::person::PersonCmd) -> bool,
    collection: &mut dyn FnMut(crate::stores::collection::CollectionCmd) -> bool,
    search: &mut dyn FnMut(crate::stores::search::SearchCmd) -> bool,
    metadata: &mut dyn FnMut(crate::stores::metadata::MetadataCmd) -> bool,
) -> bool {
    use crate::stores::viewstate::ViewStateCmd;
    match cmd {
        ViewStateCmd::Request { sid, rk, write, detail } => {
            self.request(adapter, sid, &rk, write, detail, browse, hubs, person, collection, search, metadata)
        }
        ViewStateCmd::Reset => {
            self.reset();
            true
        }
    }
}
}

// ---------------------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    /// The optimistic write flies only for the Jellyfin slot with a signed-in client
    /// (`request`'s single-source guard), so these tests install the synthetic backend.
    fn install_backend_for_viewstate() -> crate::registry::ServerId {
        crate::jellyfin::install(crate::jellyfin::JfClient::synthetic(
            crate::origin::Origin::http("127.0.0.1", 8096),
            "viewstate-test-device".into(),
            "tok".to_string(),
            "user-viewstate-test".into(),
            "Viewstate Test".into(),
        ));
        crate::jellyfin::SERVER_ID
    }

    #[test]
    fn owner_callback_receives_the_optimistic_browse_edit_before_run_returns() {
        let _g = crate::testlock::serial();
        crate::registry::reset_for_test();
        let sid = install_backend_for_viewstate();
        let mut store = crate::stores::viewstate::ViewStateStore::default();
        store.hold_inflight_for_test(sid, "held");
        let mut browse = Vec::new();
        let mut hubs = Vec::new();
        let mut person = Vec::new();
        let mut collection = Vec::new();
        let mut search = Vec::new();
        let mut metadata_store = crate::stores::metadata::MetadataStore::default();

        assert!(store.run(
            crate::stores::viewstate::ViewStateCmd::Request {
                sid,
                rk: "7".into(),
                write: Write::Watched,
                detail: None,
            },
            &mut |cmd| { browse.push(cmd); true },
            &mut |cmd| {
                hubs.push(cmd);
                crate::stores::StoreOutcome::default()
            },
            &mut |cmd| { person.push(cmd); true },
            &mut |cmd| { collection.push(cmd); true },
            &mut |cmd| { search.push(cmd); true },
            &mut |cmd| metadata_store.run(cmd),
        ));

        assert!(matches!(hubs.as_slice(), [crate::stores::hubs::HubsCmd::EditItem {
            sid: seen, rk, edit: crate::catalog::LocalEdit::Watched(true)
        }] if *seen == sid && rk == "7"));
        assert!(matches!(browse.as_slice(), [crate::stores::browse::BrowseCmd::SetWatchedLocal {
            sid: seen, rk, on: true
        }] if *seen == sid && rk == "7"));
        assert!(matches!(person.as_slice(), [crate::stores::person::PersonCmd::SetWatchedLocal {
            sid: seen, rk, on: true
        }] if *seen == sid && rk == "7"));
        assert!(matches!(collection.as_slice(), [crate::stores::collection::CollectionCmd::SetWatchedLocal {
            sid: seen, rk, on: true
        }] if *seen == sid && rk == "7"), "a collection member's disc flips with the press");
        assert!(matches!(search.as_slice(), [crate::stores::search::SearchCmd::SetWatchedLocal {
            sid: seen, rk, on: true
        }] if *seen == sid && rk == "7"));
        assert!(metadata_store.take_notice().is_some(),
            "the optimistic edit reaches {} before run returns", crate::stores::StoreId::Metadata.name());
        assert!(store.take_notice().is_some(), "the owning ViewState store notices its command");
        crate::jellyfin::uninstall();
        crate::registry::reset_for_test();
    }

    #[test]
    fn stores_route_the_optimistic_edit_to_the_addressed_browse_owner() {
        let _g = crate::testlock::serial();
        crate::registry::reset_for_test();
        let sid = install_backend_for_viewstate();
        let mut selected = crate::stores::Stores::default();
        selected.browse.borrow_mut().seed_registered_table_for_test([sid, sid]);
        selected.browse.borrow_mut().seed_items_for_test(1);
        let decoy = crate::stores::Stores::default();
        selected.viewstate.borrow_mut().hold_inflight_for_test(sid, "held");
        let directory = crate::stores::browse::DirectorySnapshot::default();

        assert!(selected.viewstate_run(crate::stores::viewstate::ViewStateCmd::Request {
            sid,
            rk: "1".into(),
            write: Write::Watched,
            detail: None,
        }, directory.view()));

        assert!(!selected.browse.borrow_mut().listing_snapshot().view().item(0).unwrap().unwatched,
            "the selected owner changes before the command returns");
        assert!(decoy.browse.borrow_mut().listing_snapshot().view().item(0).is_none(),
            "the unaddressed decoy is not used by the owner-aware callback");
        drop((selected, decoy));
        crate::jellyfin::uninstall();
        crate::registry::reset_for_test();
    }

    use super::*;

    // Tests that touch the remaining compatibility stores hold the crate-wide serial lock. The
    // ViewState queue, latches and adapter below are ordinary local values and need no lock by
    // themselves.
    //
    // None of these calls `kick`. A host test has no backend client, so a kicked write is dropped
    // with a log line — which is correct behaviour and useless as a subject. What is worth grading
    // is the bookkeeping around it, which is what these drive directly.

    /// The two fixture slots the identity rules are graded against. They are `ServerId`s and NOT a
    /// registry: `from_raw` is `const` exactly so a test can name a server without a table, a
    /// client or a socket behind it. Both hold a `ratingKey` in common, which is why the optimistic
    /// edit is graded on the exact `(sid, rk)` pair.
    const SRV_A: ServerId = ServerId::from_raw(0);
    const SRV_B: ServerId = ServerId::from_raw(1);

    fn detail(episode: &str) -> crate::stores::viewstate::DetailRefresh {
        crate::stores::viewstate::DetailRefresh {
            sid: SRV_A,
            rk: "show".into(),
            keep: Some(episode.into()),
        }
    }

    fn req(state: &mut ViewStateState, rk: &str, w: Write,
        detail: Option<crate::stores::viewstate::DetailRefresh>) -> Req {
        let id = state.mint_request_id();
        Req {
            id,
            sid: ServerId::UNSET,
            rk: rk.into(),
            w,
            detail,
        }
    }

    /// Queue a write without spawning anything — the half of [`request`] under test here.
    fn enqueue(state: &mut ViewStateState, rk: &str, w: Write,
        detail: Option<crate::stores::viewstate::DetailRefresh>) {
        state.coalesce(ServerId::UNSET, rk, w);
        let req = req(state, rk, w, detail);
        state.queue.push(req);
    }

    fn queued(state: &ViewStateState) -> Vec<(String, Write)> {
        state.queue.iter().map(|r| (r.rk.clone(), r.w)).collect()
    }

    /// Two presses on one item are ONE write, and it is the latest one — a scrobble and an
    /// unscrobble that both went out would leave the server saying whichever landed last, which is
    /// not what the user asked for and is not even deterministic.
    #[test]
    fn a_second_press_on_one_item_replaces_the_queued_write_rather_than_joining_it() {
        let mut state = ViewStateState::default();

        enqueue(&mut state, "7", Write::Watched, None);
        enqueue(&mut state, "9", Write::Watched, None);
        enqueue(&mut state, "7", Write::Unwatched, None); // the user changed their mind about 7

        assert_eq!(
            queued(&state),
            vec![("9".into(), Write::Watched), ("7".into(), Write::Unwatched)],
            "7's earlier write is gone and its latest one is queued behind 9's, in press order"
        );
    }

    /// …but a deck removal is a DIFFERENT decision about the same item, so a watched toggle must not
    /// swallow it. Both are owed a round trip.
    #[test]
    fn a_deck_removal_and_a_watched_toggle_on_one_item_are_two_writes() {
        let mut state = ViewStateState::default();

        enqueue(&mut state, "7", Write::RemoveFromDeck, None);
        enqueue(&mut state, "7", Write::Watched, None);

        assert_eq!(
            queued(&state),
            vec![
                ("7".into(), Write::RemoveFromDeck),
                ("7".into(), Write::Watched)
            ],
            "two families, two writes, in the order pressed"
        );
    }

    /// A write already ON THE WIRE is not coalesced away: the client cannot recall it, so pretending
    /// otherwise would drop the press that supersedes it and leave the server on the older answer.
    #[test]
    fn a_write_already_in_flight_is_never_coalesced_away() {
        let mut state = ViewStateState::default();
        state.sent = Some(req(&mut state, "7", Write::Watched, None));

        enqueue(&mut state, "7", Write::Unwatched, None);

        assert!(state.sent.is_some(), "the one on the wire stays out");
        assert_eq!(
            queued(&state),
            vec![("7".into(), Write::Unwatched)],
            "and its successor is queued behind it"
        );
    }

    /// The refresh is owed ONCE, at the end of a burst — not per write. Ticking four episodes
    /// watched in a row must cost four writes and one `/hubs` pair, and the detail re-read must
    /// carry the episode the filmstrip has to land back on.
    #[test]
    fn the_refresh_waits_for_the_last_write_of_a_burst_and_carries_the_kept_episode() {
        let mut state = ViewStateState::default();

        state.sent = Some(req(&mut state, "1", Write::Watched, Some(detail("1"))));
        enqueue(&mut state, "2", Write::Watched, Some(detail("2")));
        assert!(state.is_busy());

        // write 1 answers — the latches arm, but the queue is not empty, so nothing is refreshed
        let landed = state.sent.take().expect("a write was out");
        state.want_hubs = true;
        state.want_detail = landed.detail;
        assert!(
            state.is_busy(),
            "write 2 is still queued — the refresh is not owed yet"
        );

        // …and write 2 answers, superseding which episode the filmstrip lands back on
        let two = state.queue.remove(0);
        state.want_hubs = true;
        state.want_detail = two.detail;
        assert!(!state.is_busy());
        let hubs = std::mem::take(&mut state.want_hubs);
        let detail = state.want_detail.take();
        assert!(hubs, "exactly one hub refetch is owed for the burst");
        assert_eq!(
            detail.as_ref().and_then(|target| target.keep.as_deref()),
            Some("2"),
            "…and the LAST write's episode is the one kept"
        );
    }

    /// A refused spawn must neither panic nor block nor silently lose the press: the write goes back
    /// to the HEAD of the queue and the ladder holds the next attempt off. The real refusal needs
    /// the OS out of threads, which no host test can arrange, so what is graded is the recovery.
    #[test]
    fn a_refused_spawn_keeps_the_write_at_the_head_of_the_queue_and_waits_out_the_ladder() {
        let mut state = ViewStateState::default();

        // exactly what `kick`'s refusal branch does
        enqueue(&mut state, "5", Write::Watched, None);
        let head = state.queue.remove(0);
        state.queue.insert(0, head);
        state.retry_cd = RETRY_FRAMES;

        assert_eq!(
            queued(&state),
            vec![("5".into(), Write::Watched)],
            "the press survives the refusal"
        );
        assert!(
            state.sent.is_none(),
            "and nothing is recorded as in flight"
        );

        for i in 1..RETRY_FRAMES {
            assert!(!state.retry_tick(), "frame {i} of the wait is not the due one");
        }
        assert!(
            state.retry_tick(),
            "the ladder is spent after RETRY_FRAMES frames"
        );
        assert_eq!(state.retry_cd, 0);
        assert!(
            state.retry_tick(),
            "…and stays due until a fresh refusal re-arms it"
        );
    }

    /// `reset` is what an identity change leans on: nothing of the previous account's may be left
    /// queued, in flight, or owed a refresh.
    #[test]
    fn reset_leaves_nothing_queued_in_flight_or_owed() {
        let mut state = ViewStateState::default();
        let adapter = Arc::new(ViewStateAdapter::default());
        enqueue(&mut state, "1", Write::Watched, Some(detail("1")));
        let sent = req(&mut state, "2", Write::Unwatched, None);
        let sent_id = sent.id;
        state.sent = Some(sent);
        state.want_hubs = true;
        state.want_detail = Some(detail("3"));
        state.retry_cd = 9;
        *adapter.mail.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Completion { id: sent_id, done: Done::default() });

        state.reset();

        assert!(!state.is_busy());
        assert!(!state.want_hubs);
        assert!(state.want_detail.is_none());
        assert_eq!(state.retry_cd, 0);
        assert!(adapter.mail.lock().unwrap_or_else(|e| e.into_inner()).is_some(),
            "state reset does not mutate an adapter; the owning store retires it wholesale");
    }

    /// The optimistic edit is applied to the exact `(sid, rk)` PAIR, so a row that merely shares
    /// the key with another server's item is left alone: `sid` addresses the server, `rk` the item
    /// on it. Graded through the detail store, which is the surface that shows it.
    #[test]
    fn the_landing_flips_another_sources_copy_the_press_could_not_reach() {
        let _g = crate::testlock::serial();
        let mut metadata_store = crate::stores::metadata::MetadataStore::default();
        // the page is mounted on the SHARE's copy, which the press on our own copy never touched
        crate::metadata::install_for_test(metadata_store.state_mut(), Some(crate::metadata::Detail {
            sid: SRV_B,
            rk: "4".into(),
            resume_ms: 900_000,
            ..Default::default()
        }));

        edit_local_with_owners(SRV_A, "4", Write::Watched, &mut |_| false,
            &mut |_cmd| crate::stores::StoreOutcome::default(), &mut |_| false, &mut |_| false, &mut |_| false,
            &mut |cmd| metadata_store.run(cmd));
        assert!(
            !metadata_store.view().current().unwrap().watched,
            "A's 4 is not B's 4"
        );

        edit_local_with_owners(SRV_B, "4", Write::Watched, &mut |_| false,
            &mut |_cmd| crate::stores::StoreOutcome::default(), &mut |_| false, &mut |_| false, &mut |_| false,
            &mut |cmd| metadata_store.run(cmd));
        assert!(metadata_store.view().current().unwrap().watched);
        assert_eq!(
            metadata_store.view().current().unwrap().resume_ms,
            0,
            "watched stops offering to resume"
        );

        crate::metadata::install_for_test(metadata_store.state_mut(), None);
    }

    #[test]
    fn only_the_exact_monotone_request_identity_can_retire_a_flight() {
        let _guard = crate::testlock::serial();
        let mut state = ViewStateState::default();
        let adapter = Arc::new(ViewStateAdapter::default());
        let first = req(&mut state, "before-reset", Write::Watched, None);
        let first_id = first.id;
        state.sent = Some(first);
        state.reset();
        let second = req(&mut state, "after-reset", Write::Unwatched, None);
        let second_id = second.id;
        assert!(second_id.0 > first_id.0, "request identity stays monotone across reset");
        state.sent = Some(second);
        *adapter.mail.lock().unwrap_or_else(|e| e.into_inner()) = Some(Completion {
            id: first_id,
            done: Done::default(),
        });

        let _ = state.pump(&adapter, &mut |_| false,
            &mut |_| crate::stores::StoreOutcome::default());

        assert_eq!(state.sent.as_ref().map(|request| request.id), Some(second_id),
            "a stale or mismatched completion cannot satisfy the current in-flight identity");
        assert!(!state.want_hubs);
        assert!(state.want_detail.is_none());
    }

    #[test]
    fn deferred_detail_refresh_keeps_the_originating_page_address() {
        let _guard = crate::testlock::serial();
        let target = crate::stores::viewstate::DetailRefresh {
            sid: SRV_B,
            rk: "origin-show".into(),
            keep: Some("episode-7".into()),
        };
        let mut state = ViewStateState::default();
        let adapter = Arc::new(ViewStateAdapter::default());
        let request = req(&mut state, "episode-7", Write::Watched, Some(target.clone()));
        let id = request.id;
        state.sent = Some(request);
        *adapter.mail.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Completion { id, done: Done::default() });

        let _ = state.pump(&adapter, &mut |_| false,
            &mut |_| crate::stores::StoreOutcome::default());

        assert_eq!(state.take_detail_refresh(), Some(target));
    }
}
