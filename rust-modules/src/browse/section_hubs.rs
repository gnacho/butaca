//! **One library's own shelves** — the `Library Recommended` rows a server's owner arranged,
//! quoted rather than composed. The endpoint that fed them and its client method went with the
//! server browse arm (#166 pass 3);
//! what stays is the publication machine the Library screen renders above its A–Z grid.
//!
//! ## Why this is a FIELD on `SecState` and not a store of its own
//!
//! The obvious shape was a sibling of [`crate::catalog`] keyed by `(ServerId, section_key)` with its
//! own LRU. [`super::SecState`] is *already* the per-library-section aggregate — it owns that
//! section's query, verdict, items, focus and scroll — so a second store meant a second section
//! identity, a second generation gate, a second mailbox and a second retry policy for one library,
//! plus a cache that would **survive a Plex Home profile switch** and show the next profile the
//! previous one's resume and watched state. [`super::reset`] already solves that, and `EPOCH`
//! already gates landings. One identity, not two that must agree.
//!
//! ## The publication machine, and why it is not just "commit when terminal"
//!
//! The grid and the hubs land independently, and **the shelf count sets the grid's absolute
//! offset** — a late 12-shelf landing would move a focused grid row by 6,588px. "Commit when the
//! fetch is terminal" does not work, because [`crate::catalog::backoff_secs`] is an INFINITE ladder
//! (2/4/8/16/30s and then 30s forever): terminal never arrives on a dead server, and an ordinary
//! fail-then-succeed would still shift the grid under a reader.
//!
//! So there are **two independent axes**, which is the thing to hold on to here. A *publication*
//! state ([`Publication`]) and a *fetch* state (idle / in flight / backing off) running underneath
//! it: zero shelves can be `Committed` while the ladder is still retrying, and a refresh can be in
//! flight over a `Committed` or `Staged` set.
//!
//! | [`Publication`] | Meaning | Effect on layout |
//! |---|---|---|
//! | `Fetching` | nothing published yet; the first attempt is out, inside a bounded first-paint window | not committed; the grid draws where it already is |
//! | `Staged` | a landing exists but publishing it would move a grid the user is looking at | held; nothing moves |
//! | `Committed` | the shelf set the current layout was built from | the document |
//!
//! The **first-paint window is a separate policy from the retry ladder** — when it expires the
//! section commits ZERO shelves and the page is usable, while the ladder keeps retrying underneath
//! and any later success arrives as `Staged`. A newer success replaces an older staged set; a
//! FAILURE preserves both the staged success and the last committed set, because a transient
//! failure must never be able to subtract shelves.
//!
//! That last rule is not an edge case here, it is the ORDINARY refresh path: §3a measured two of
//! these hubs changing identity between requests — the genre and actor/director shelves rotate
//! their subject on every call and sometimes answer empty — so a plain refetch legitimately returns
//! a different shelf count and different ids with nothing changed on the server.
//!
//! ## Who drives it
//!
//! The owned Library emits `PublishShelves`; the application combines its visibility flags with
//! live press state and delivers addressed `LibraryWork::Hubs` to `stores::browse`, which calls
//! [`BrowseState::hubs_kick`] and gates staged publication. The server fetch itself
//! (`GET /hubs/sections/{id}`) is gone with the server browse arm (#166 pass 3): Jellyfin has no
//! per-library shelves feed, so `kick` arms the window and the first-paint expiry commits ZERO
//! shelves — the same end state a dead server section reached, and the same screen contract. The
//! publication machine stays because it is what the Library screen renders and what a future
//! shelves feed would seat its landings beside.
#![allow(dead_code)] // the accessors are Landing 3's; see the Dormant note above

use crate::registry::ServerId;
use crate::catalog::CatalogMovie;
use std::sync::Arc;

/// Frames the page waits for the FIRST answer before committing zero shelves and becoming usable
/// — ~4 s at 60 fps. Deliberately not derived from a retry ladder (there is no fetch left to
/// back off), and counted in FRAMES because [`super::pump`] is a per-frame call with no `dt`,
/// exactly as its own `RETRY_CD` is.
const FIRST_PAINT_FRAMES: u32 = 240;

/// One published shelf: the owner's own row, with its items already filtered and capped.
#[derive(Clone, Default)]
pub(crate) struct Shelf {
    /// `hubIdentifier`. **Not a stable key across refetches** for the genre and actor/director
    /// rows, which rotate their subject on every call (§3a) — anything remembering a position by
    /// it owes a fallback for an id that is simply not there any more.
    pub(crate) id: String,
    /// The hub's provider listing `key`, verbatim.
    pub(crate) key: String,
    /// Every item the hub's listing holds (`server::Hub::total`), which `items` caps. 0 when the
    /// server named no total.
    pub(crate) total: usize,
    pub(crate) title: String,
    /// A Continue Watching row scoped to this section. **Not `home.continue`**, which is
    /// `catalog`'s whole-server id and does not appear here — see [`shelf_is_continue`].
    pub(crate) is_continue: bool,
    /// **This row is EPISODES, so it is drawn as landscape stills** rather than portrait posters —
    /// see [`is_episode_shelf`] for why that is decided from the items and not from the hub.
    pub(crate) landscape: bool,
    pub(crate) items: Vec<CatalogMovie>,
}

/// Where a section's shelves are in the publication axis. The fetch axis is separate and runs
/// underneath this; see the module doc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Publication {
    Fetching,
    Staged,
    Committed,
}

/// The library section whose publication a retained snapshot belongs to. A section-table index is
/// not an identity: [`super::reset`] reuses it for the next profile, and two servers may both have
/// section `1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct HubsId {
    pub(crate) epoch: u32,
    pub(crate) sid: ServerId,
    pub(crate) section: i64,
}

/// One retained immutable publication. Cloning this copies one [`Arc`], never a shelf, item or
/// string; later commits and optimistic edits detach their writable backing as needed.
#[derive(Clone)]
pub(crate) struct HubsSnapshot {
    data: Option<Arc<Vec<Shelf>>>,
    id: Option<HubsId>,
    publication: Publication,
    revision: Option<u64>,
}

impl HubsSnapshot {
    pub(crate) fn empty() -> Self {
        Self {
            data: None,
            id: None,
            publication: Publication::Fetching,
            revision: None,
        }
    }
    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self::empty()
    }

    pub(crate) fn view(&self) -> HubsView<'_> {
        HubsView {
            shelves: self.data.as_deref().map(Vec::as_slice).unwrap_or(&[]),
            id: self.id,
            publication: self.publication,
            revision: self.revision,
        }
    }
}

/// A frame-borrowed read of one retained publication. Absence is explicit in [`HubsView::id`];
/// the empty slice then means no data, not a fabricated section with a static identity.
#[derive(Clone, Copy)]
pub(crate) struct HubsView<'a> {
    shelves: &'a [Shelf],
    id: Option<HubsId>,
    publication: Publication,
    revision: Option<u64>,
}

impl<'a> HubsView<'a> {
    pub(crate) fn shelves(self) -> &'a [Shelf] {
        self.shelves
    }

    pub(crate) fn publication(self) -> Publication {
        self.publication
    }

    pub(crate) fn id(self) -> Option<HubsId> {
        self.id
    }

    /// Revision of the committed contents, scoped by `id()`. Unlike an allocation address this
    /// changes for in-place edits too. Staged-only changes do not invalidate a visible layout.
    pub(crate) fn revision(self) -> Option<u64> {
        self.revision
    }
}

/// One section's shelves and the two axes' state. A field on [`super::SecState`].
#[derive(Clone, Default)]
pub(crate) struct SecHubs {
    /// The set the current layout was built from. Empty and `committed == false` is "nothing
    /// published yet"; empty and `committed == true` is the answer "this library has no shelves",
    /// which is a real state the design draws as "this screen with a shorter top".
    committed: Arc<Vec<Shelf>>,
    revision: u64,
    /// Has anything ever been published? Distinct from `committed.is_empty()` for that reason.
    published: bool,
    /// A landing held back because publishing it would move a grid the user is looking at.
    staged: Option<Arc<Vec<Shelf>>>,
    /// Frames left in the first-paint window. Only counts down while nothing is published.
    first_paint_left: u32,
    /// Has [`kick`](super::BrowseState::hubs_kick) ever been called for this section? Nothing
    /// here ticks until it has.
    armed: bool,
}

impl SecHubs {
    pub(super) fn needs_tick(&self) -> bool {
        self.armed && !self.published && self.first_paint_left > 0
    }
    fn revised(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("section shelf revision exhausted");
    }

    /// The published shelves — what a layout may be built from, and the only set a screen draws.
    pub(crate) fn shelves(&self) -> &[Shelf] {
        self.committed.as_slice()
    }

    pub(crate) fn publication(&self) -> Publication {
        // **`Staged` outranks `Fetching`**, which is the ordering the first-paint commit made
        // reachable: before it, an answer held back on the page's FIRST landing had nowhere to be
        // reported but `Fetching`, i.e. "nothing has arrived", of a set that had.
        if self.staged.is_some() {
            Publication::Staged
        } else if !self.published {
            Publication::Fetching
        } else {
            Publication::Committed
        }
    }

    /// Advance the first-paint window by one frame. Returns whether anything a screen can see
    /// changed — the window expiring, which publishes zero shelves.
    fn tick(&mut self) -> bool {
        if !self.armed {
            return false;
        }
        if self.published {
            return false;
        }
        self.first_paint_left = self.first_paint_left.saturating_sub(1);
        if self.first_paint_left > 0 {
            return false;
        }
        if self.staged.is_some() {
            // An answer IS in hand and is simply not allowed through yet. The window is not what is
            // holding it, so there is nothing to publish here and nothing to report — the screen's
            // own `commit_staged` is the one that will let it in.
            return false;
        }
        // The window closed with no answer. Publish NOTHING — the page becomes usable at the
        // offset it already has, the ladder keeps retrying underneath, and any later success will
        // arrive as `Staged` rather than moving a grid somebody is reading.
        self.published = true;
        self.revised();
        true
    }

    /// A successful landing. It **always stages**, and a newer success replaces an older staged
    /// set: the freshest answer is the one worth showing when the page finally lets it through.
    ///
    /// **The FIRST landing stages too, and that is a correction (2026-09-05).** It used to commit
    /// itself whenever nothing had been published, on the reasoning that inside the first-paint
    /// window there is no layout yet to protect. That reasoning is about the first FRAMES and the
    /// window is four SECONDS: `/tmp/butaca-library` boots straight into this screen, two
    /// presses reach the grid, and the shelves then inserted themselves above a grid row somebody
    /// was already looking at — the exact 6,588px jump the rest of this machine exists to prevent,
    /// arriving through the one path that did not ask. Whether the ground may move is the SCREEN's
    /// answer and it is already computed every frame, so the first paint goes through
    /// [`SecHubs::commit_staged`] like every later one. At the head — where a page four seconds old
    /// almost always is — `BrowseStore::pump` lands and the screen commits in the SAME frame, so
    /// nothing waits.
    ///
    /// Production has no landing left to call this (no server fetch), so it survives as the test
    /// seeders' way to place a set exactly where a worker would have.
    #[cfg(test)]
    fn land_ok(&mut self, shelves: Vec<Shelf>) {
        self.staged = Some(Arc::new(shelves));
    }

    /// Publish a staged set. `may_move` is the caller's answer to "can the user see the ground
    /// this would move" — `true` at a route or section transition floor (the page is not on screen,
    /// so nothing can be SEEN to move) and, on a visible page, only once a BACK-driven head
    /// transition has settled. Returns whether anything was published, so the caller knows to
    /// re-resolve focus against the POST-commit layout.
    fn commit_staged(&mut self, may_move: bool) -> bool {
        if !may_move {
            return false;
        }
        match self.staged.take() {
            Some(s) => {
                self.committed = s;
                self.published = true;
                self.revised();
                true
            }
            None => false,
        }
    }
}

/// Arm a section's shelves' publication window. Idempotent; call it every frame the screen wants
/// shelves. There is no fetch to issue any more (module doc): this arms the first-paint window
/// whose expiry commits zero shelves, which is the screen contract a shelves feed would seat
/// beside.
impl super::BrowseState {
    pub(super) fn hubs_kick(&mut self, sec: usize) {
        let Some(st) = self.state_mut(sec) else {
            return;
        };
        if !st.hubs.armed {
            st.hubs.armed = true;
            st.hubs.first_paint_left = FIRST_PAINT_FRAMES;
        }
    }

    /// Advance every armed section's first-paint window by one frame.
    pub(crate) fn hubs_tick_all(&mut self) -> bool {
        let mut moved = false;
        for st in self.states_mut() {
            moved |= st.hubs.tick();
        }
        moved
    }

    fn hubs_shelves(&self, sec: usize) -> &[Shelf] {
        self.states()
            .get(sec)
            .map(|s| s.hubs.shelves())
            .unwrap_or(&[])
    }

    fn hubs_publication(&self, sec: usize) -> Publication {
        self.states()
            .get(sec)
            .map(|s| s.hubs.publication())
            .unwrap_or(Publication::Fetching)
    }

    pub(crate) fn hubs_snapshot(&self, sec: usize) -> HubsSnapshot {
        let Some(section) = self.sections().get(sec) else {
            return HubsSnapshot::empty();
        };
        let Some(state) = self.states().get(sec) else {
            return HubsSnapshot::empty();
        };
        let Some(source) = self.sources().get(section.src) else {
            return HubsSnapshot::empty();
        };
        HubsSnapshot {
            data: Some(Arc::clone(&state.hubs.committed)),
            id: Some(HubsId {
                epoch: self.table_epoch(),
                sid: source.sid,
                section: section.key,
            }),
            publication: state.hubs.publication(),
            revision: Some(state.hubs.revision),
        }
    }

    pub(super) fn hubs_commit_staged(&mut self, sec: usize, may_move: bool) -> bool {
        self.state_mut(sec)
            .map(|s| s.hubs.commit_staged(may_move))
            .unwrap_or(false)
    }

    pub(super) fn hubs_set_watched_local(&mut self, sid: ServerId, rk: &str, on: bool) -> bool {
        let mut hit = false;
        for st in self.states_mut() {
            if edit_watched(&mut st.hubs.committed, sid, rk, on) {
                st.hubs.revised();
                hit = true;
            }
            if let Some(staged) = st.hubs.staged.as_mut() {
                edit_watched(staged, sid, rk, on);
            }
        }
        hit
    }

    pub(super) fn hubs_left_the_deck(&mut self, sid: ServerId, rk: &str) -> bool {
        let mut hit = false;
        for st in self.states_mut() {
            if drop_from_deck(&mut st.hubs.committed, sid, rk) {
                st.hubs.revised();
                hit = true;
            }
            if let Some(staged) = st.hubs.staged.as_mut() {
                drop_from_deck(staged, sid, rk);
            }
        }
        hit
    }

    pub(super) fn hubs_invalidate_all(&mut self) {
        // An invalidation used to re-arm a fetch. With no fetch left, the one thing it can still
        // mean is: an uncommitted landing must not survive whatever event asked to invalidate.
        for st in self.states_mut() {
            st.hubs.staged = None;
        }
    }
}

fn edit_watched(shelves: &mut Arc<Vec<Shelf>>, sid: ServerId, rk: &str, on: bool) -> bool {
    if !shelves
        .iter()
        .flat_map(|shelf| shelf.items.iter())
        .any(|item| crate::registry::same_item((item.sid, &item.rk), (sid, rk)))
    {
        return false;
    }
    for item in Arc::make_mut(shelves)
        .iter_mut()
        .flat_map(|shelf| shelf.items.iter_mut())
    {
        if crate::registry::same_item((item.sid, &item.rk), (sid, rk)) {
            crate::catalog::set_watched(item, on);
        }
    }
    true
}

fn drop_from_deck(shelves: &mut Arc<Vec<Shelf>>, sid: ServerId, rk: &str) -> bool {
    if !shelves
        .iter()
        .filter(|shelf| shelf.is_continue)
        .flat_map(|shelf| shelf.items.iter())
        .any(|item| crate::registry::same_item((item.sid, &item.rk), (sid, rk)))
    {
        return false;
    }
    let shelves = Arc::make_mut(shelves);
    let mut hit = false;
    for shelf in shelves.iter_mut().filter(|shelf| shelf.is_continue) {
        let before = shelf.items.len();
        shelf
            .items
            .retain(|item| !crate::registry::same_item((item.sid, &item.rk), (sid, rk)));
        hit |= shelf.items.len() != before;
    }
    shelves.retain(|shelf| !shelf.items.is_empty());
    hit
}

/// Publish a section's shelves directly, for the screen tests that need a document with a shelf
/// run above the grid. Not a fetch and not a landing: no worker, no gate, no ladder — the shape
/// only, which is all a geometry or focus-walk assertion is about.
#[cfg(test)]
pub(crate) fn seed_shelves_for_owner_test(
    state: &mut super::BrowseState,
    sec: usize,
    titles: &[&str],
    per_row: usize,
) {
    let rows: Vec<_> = titles.iter().map(|title| (*title, "", *title)).collect();
    seed_named_shelves_for_owner_test(state, sec, &rows, per_row);
}

/// [`seed_shelves_for_owner_test`] with each shelf's `(hubIdentifier, key, title)` named — a
/// promoted collection shelf needs its identifier and listing key.
#[cfg(test)]
pub(crate) fn seed_named_shelves_for_owner_test(
    state: &mut super::BrowseState,
    sec: usize,
    rows: &[(&str, &str, &str)],
    per_row: usize,
) {
    let Some(st) = state.state_mut(sec) else {
        return;
    };
    st.hubs = SecHubs {
        revision: st.hubs.revision,
        ..Default::default()
    };
    st.hubs.armed = true;
    st.hubs.land_ok(
        rows
            .iter()
            .map(|(id, key, title)| Shelf {
                id: (*id).into(),
                key: (*key).into(),
                total: per_row,
                title: (*title).into(),
                is_continue: shelf_is_continue(id, key),
                landscape: false,
                items: (0..per_row)
                    .map(|index| CatalogMovie {
                        rk: format!("{id}-{index}"),
                        title: format!("{title} {index}"),
                        ..Default::default()
                    })
                    .collect(),
            })
            .collect(),
    );
    st.hubs.commit_staged(true);
}

/// A worker success held behind the Library's publication gate, without starting a worker.
#[cfg(test)]
pub(crate) fn stage_shelves_for_owner_test(state: &mut super::BrowseState, sec: usize) {
    if let Some(st) = state.state_mut(sec) {
        st.hubs.land_ok(vec![Shelf { title: "staged".into(), ..Default::default() }]);
    }
}

/// Turn a seeded section's shelves into EPISODE shelves — landscape rows of `kind == 3` items with
/// a show behind them. The half [`seed_shelves_for_test`] deliberately does not do, because a
/// geometry test wants the poster shape it can name; a test about the episode TILE wants this.
#[cfg(test)]
pub(crate) fn seed_landscape_for_owner_test(
    state: &mut super::BrowseState,
    sec: usize,
    show: &str,
) {
    let Some(st) = state.state_mut(sec) else {
        return;
    };
    for shelf in Arc::make_mut(&mut st.hubs.committed).iter_mut() {
        shelf.landscape = true;
        for item in &mut shelf.items {
            item.kind = 3;
            item.show_title = show.into();
            item.season_index = 1;
            item.ep_index = 1;
        }
    }
    st.hubs.revised();
}

/// **Every published shelf is a store that can be DRAWING the item** — so it takes the same
/// optimistic edit `catalog`, `metadata`, `browse`, `search` and `person` already take, and for the same
/// reason: the press writes correctly to the server and changes nothing on the screen the user is
/// looking at until a refetch, which reads as the row having done nothing.
///
/// This was the sixth store and it was not on the list; `viewstate::edit_local`'s five calls
/// predate the Library growing shelves of its own.
// ---- the pure half --------------------------------------------------------------------------

/// One response into published shelves. **Pure**, so every rule below is host-graded against the
/// fixture in this module's tests rather than inferred from a screenshot.
///
/// The filter is [`crate::catalog`]'s: allow only the types this product can list, require a title
/// and a thumb, and read the ITEM's own `type` rather than the hub's — which can be `mixed`. An
/// EMPTY hub is dropped entirely, which §3a measured as the common case (one movie section
/// answered with 6 hubs of which 5 were empty), so this is required rather than tidy: a client
/// that draws what it is given draws five headings over nothing.
///
/// RETIRED with the server fetch (#166 pass 3): its rules and the measured fixture that graded them
/// went with it. `shelf_is_continue` below stays — it is the identifier vocabulary a future
/// shelves feed must speak, and the test seeders key on it.

/// Prints the shelf's identity and how many items it holds — `CatalogMovie` has no `Debug` and a
/// derived one would dump a whole shelf of DTOs into a failing assertion, which is noise rather
/// than evidence. The count is what a layout question is ever about.
impl std::fmt::Debug for Shelf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Shelf({:?} {:?} cw={} items={})",
            self.id,
            self.title,
            self.is_continue,
            self.items.len()
        )
    }
}

/// Is this the section's own Continue Watching row?
///
/// **It is not `home.continue`**, and that is the whole reason this function exists rather than a
/// call to `catalog::hub_is_continue`: that id is the WHOLE-SERVER hub's and does not appear on this
/// route at all. A per-section deck is `movie.inprogress.N` / `tv.inprogress.N`, or its
/// `key=/hubs/sections/N/continueWatching/items` (§3a). Both are matched, because the id is the
/// part a server could plausibly spell differently and the key is the route it must answer at.
///
/// **The key match is SECTION-SCOPED on purpose.** The whole-server deck's key is
/// `/hubs/continueWatching/items` — the same tail — so a bare `contains` on it answers `true` for a
/// hub belonging to a different question entirely. Nothing puts that hub on this route today, so
/// the loose form was harmless and would have stayed harmless right up until somebody reached for
/// this function from the `/hubs` side.
///
/// Its only caller was `parse_hubs`, retired with the server fetch; the test seeders keep speaking
/// the same identifier vocabulary a future shelves feed must.
#[cfg_attr(not(test), expect(dead_code))]
pub(crate) fn shelf_is_continue(id: &str, key: &str) -> bool {
    id.contains(".inprogress.")
        || (key.contains("/hubs/sections/") && key.contains("/continueWatching/items"))
}

// ---------------------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_browse_state_values_share_no_core_state() {
        let mut first = super::super::BrowseState::default();
        let mut second = super::super::BrowseState::default();

        first
            .sources
            .push(super::super::test_support::a_source("machine-a", "", true));
        for (key, title) in [(7, "Films A"), (8, "Films B")] {
            first.sections.push(super::super::BrowseSection {
                src: 0,
                key,
                title: title.into(),
                kind: super::super::SecKind::Movie,
                count: 12,
                pinned: true,
            });
            first.states.push(super::super::SecState::default());
        }
        first.set_cur(1);
        first.want(60, 120);
        first.bump_gen();
        first.bump_sections_gen();
        first.src_facts_gen = first.src_facts_gen.wrapping_add(1);
        first
            .remembered
            .push((super::super::SecKind::Movie, "machine-a".into(), 7));
        first.recorded = Some(crate::server::session::HomePins {
            user: "profile-a".into(),
            asked: true,
            on: Vec::new(),
            off: Vec::new(),
            extensions: Default::default(),
        });
        first.retry_cd = 120;
        first.states[1].hubs.armed = true;
        first.states[1].hubs.land_ok(vec![Shelf {
            id: "movie.recent.8".into(),
            title: "Recently Added".into(),
            ..Default::default()
        }]);
        assert!(first.states[1].hubs.commit_staged(true));
        let first_tabs_gen = first.tabs_gen();

        second
            .sources
            .push(super::super::test_support::a_source("machine-b", "", false));
        second.sections.push(super::super::BrowseSection {
            src: 0,
            key: 70,
            title: "Other Films".into(),
            kind: super::super::SecKind::Movie,
            count: 3,
            pinned: true,
        });
        second.states.push(super::super::SecState::default());
        second.states[0].hubs.armed = true;
        second.states[0].hubs.land_ok(vec![Shelf {
            id: "movie.recent.70".into(),
            title: "Other Recent".into(),
            ..Default::default()
        }]);
        assert!(second.states[0].hubs.commit_staged(true));
        second.want(1, 3);

        let retained_listing = first.listing_snapshot();
        let retained_hubs = first.hubs_snapshot(1);
        let second_hubs = second.hubs_snapshot(0);

        assert_eq!(first.sources().len(), 1);
        assert_eq!(first.sections().len(), 2);
        assert_eq!(first.states().len(), 2);
        assert_eq!(first.cur(), 1, "the test exercises a real selection move");
        assert_eq!(first.want, (60, 120));
        assert_eq!(first.query_gen(), 2);
        assert_eq!(first.sections_gen(), 1);
        assert_eq!(first.source_list_gen(), 2);
        assert!(first_tabs_gen > 1);
        assert_eq!(first.pinned_count(), 2);
        assert_eq!(first.remembered.len(), 1);
        assert!(first.recorded.is_some());
        assert_eq!(first.retry_cd, 120);
        assert_eq!(first.states[1].hubs.publication(), Publication::Committed);
        assert_eq!(first.states[1].hubs.shelves().len(), 1);

        assert_eq!(second.sources().len(), 1);
        assert_eq!(second.sections().len(), 1);
        assert_eq!(second.states().len(), 1);
        assert_eq!(second.cur(), 0);
        assert_eq!(second.want, (1, 3));
        assert_eq!(second.query_gen(), 0);
        assert_eq!(second.sections_gen(), 0);
        assert_eq!(second.source_list_gen(), 0);
        assert_eq!(second.tabs_gen, 1);
        assert_eq!(second.tab_shape, u32::MAX);
        assert_eq!(second.pinned_count(), 1);
        assert!(second.remembered.is_empty());
        assert!(second.recorded.is_none());
        assert_eq!(second.retry_cd, 0);

        first.reset();
        assert_eq!(first.query_gen(), 3);
        assert_eq!(first.sections_gen(), 2);
        assert_eq!(first.table_epoch(), 1);
        assert!(first.sections().is_empty());
        assert_eq!(
            first.want,
            (60, 120),
            "the extraction preserves legacy reset timing/state"
        );
        assert!(
            retained_listing.view().id().is_some(),
            "retained listing survives owner reset"
        );
        assert_eq!(
            retained_hubs.view().shelves().len(),
            1,
            "retained hubs survive owner reset"
        );

        assert_eq!(
            second.sections().len(),
            1,
            "resetting first cannot clear second"
        );
        assert_eq!(second.table_epoch(), 0);
        assert_eq!(second.want, (1, 3));
        assert_eq!(second.hubs_snapshot(0).view().shelves().len(), 1);
        assert_eq!(second_hubs.view().shelves().len(), 1);
    }

    // ---- the publication machine, on the pure struct -----------------------------------------

    fn armed() -> SecHubs {
        SecHubs {
            armed: true,
            first_paint_left: FIRST_PAINT_FRAMES,
            ..Default::default()
        }
    }
    fn row(title: &str) -> Shelf {
        Shelf {
            id: title.into(),
            title: title.into(),
            ..Default::default()
        }
    }

    fn seeded_section() -> (super::super::BrowseState, usize) {
        let mut state = super::super::BrowseState::default();
        super::super::seed_two_source_table_for_owner_test(&mut state);
        let sec = state.cur();
        seed_shelves_for_owner_test(&mut state, sec, &["published"], 3);
        (state, sec)
    }

    #[test]
    fn publication_revision_observes_in_place_deck_removal() {
        let _g = crate::testlock::serial();
        let (mut state, sec) = seeded_section();
        seed_shelves_for_owner_test(&mut state, sec, &["movie.inprogress.1"], 3);
        let before = state.hubs_snapshot(sec);
        let revision = before.view().revision();
        let allocation = Arc::as_ptr(before.data.as_ref().unwrap());
        let item = &before.view().shelves()[0].items[1];
        let (sid, rk) = (item.sid, item.rk.clone());
        drop(before); // No retained reader: the store can mutate the same allocation.
        assert!(state.hubs_left_the_deck(sid, &rk));
        let after = state.hubs_snapshot(sec);
        assert_eq!(Arc::as_ptr(after.data.as_ref().unwrap()), allocation);
        assert_eq!(after.view().shelves()[0].items.len(), 2);
        assert_ne!(
            after.view().revision(),
            revision,
            "pointer equality cannot detect this edit"
        );
    }

    #[test]
    fn publication_revision_tracks_commits_not_staging_or_snapshot_reads() {
        let _g = crate::testlock::serial();
        let (mut state, sec) = seeded_section();
        let before = state.hubs_snapshot(sec);
        let revision = before.view().revision();
        assert!(revision.is_some());
        assert_eq!(state.hubs_snapshot(sec).view().revision(), revision);
        state.state_mut(sec)
            .unwrap()
            .hubs
            .land_ok(vec![row("next")]);
        assert_eq!(state.hubs_snapshot(sec).view().revision(), revision);
        assert!(!state.hubs_commit_staged(sec, false));
        assert_eq!(state.hubs_snapshot(sec).view().revision(), revision);
        assert!(state.hubs_commit_staged(sec, true));
        assert_ne!(state.hubs_snapshot(sec).view().revision(), revision);
        assert_eq!(before.view().revision(), revision);
        assert_eq!(before.view().shelves()[0].title, "published");
        assert!(!state.hubs_commit_staged(sec, true));
        assert_eq!(state.hubs_snapshot(usize::MAX).view().revision(), None);
    }

    #[test]
    fn publication_revision_changes_for_visible_watch_edits_but_not_staged_only_edits() {
        let _g = crate::testlock::serial();
        let (mut state, sec) = seeded_section();
        let before = state.hubs_snapshot(sec);
        let revision = before.view().revision();
        let item = &before.view().shelves()[0].items[0];
        let (sid, rk) = (item.sid, item.rk.clone());
        assert!(!state.hubs_set_watched_local(sid, "absent-item", true));
        assert_eq!(state.hubs_snapshot(sec).view().revision(), revision);
        assert!(state.hubs_set_watched_local(sid, &rk, true));
        let edited = state.hubs_snapshot(sec);
        assert_ne!(edited.view().revision(), revision);
        assert!(!before.view().shelves()[0].items[0].watched);
        assert!(edited.view().shelves()[0].items[0].watched);
        let staged = Shelf {
            items: vec![CatalogMovie {
                sid,
                rk: "staged-only".into(),
                ..Default::default()
            }],
            ..row("staged")
        };
        state.state_mut(sec)
            .unwrap()
            .hubs
            .land_ok(vec![staged]);
        assert!(!state.hubs_set_watched_local(sid, "staged-only", true));
        assert_eq!(state.hubs_snapshot(sec).view().revision(), edited.view().revision());
        assert!(state.hubs_commit_staged(sec, true));
        let published = state.hubs_snapshot(sec);
        assert!(published.view().shelves()[0].items[0].watched);
        assert_ne!(published.view().revision(), edited.view().revision());
    }

    #[test]
    fn publication_revision_marks_empty_first_paint_once_and_later_ticks_stay_quiet() {
        let mut hubs = armed();
        let initial = hubs.revision;
        hubs.first_paint_left = 1;
        assert!(hubs.tick());
        assert_ne!(hubs.revision, initial);
        let committed = hubs.revision;
        assert!(!hubs.tick());
        assert_eq!(hubs.revision, committed);
    }

    #[test]
    fn snapshot_acquisition_shares_the_publication_and_captures_its_real_identity() {
        let _g = crate::testlock::serial();
        let (state, sec) = seeded_section();
        let sid = state.section_sid(sec).unwrap();
        let section = state.sections()[sec].key;

        let first = state.hubs_snapshot(sec);
        let same = state.hubs_snapshot(sec);
        assert!(
            Arc::ptr_eq(first.data.as_ref().unwrap(), same.data.as_ref().unwrap()),
            "capturing a frame view must not clone shelves or items"
        );
        let view = first.view();
        let copied = view;
        assert_eq!(copied.shelves()[0].title, "published");
        assert_eq!(copied.publication(), Publication::Committed);
        assert_eq!(
            copied.id(),
            Some(HubsId {
                epoch: state.table_epoch(),
                sid,
                section,
            })
        );

        let missing_snapshot = state.hubs_snapshot(usize::MAX);
        let missing = missing_snapshot.view();
        assert_eq!(
            missing.id(),
            None,
            "absence has no invented section identity"
        );
        assert!(missing.shelves().is_empty());
        assert_eq!(missing.publication(), Publication::Fetching);
    }

    #[test]
    fn a_retained_snapshot_survives_commit_staged() {
        let _g = crate::testlock::serial();
        let (mut state, sec) = seeded_section();
        state.state_mut(sec)
            .unwrap()
            .hubs
            .land_ok(vec![row("replacement")]);

        let retained = state.hubs_snapshot(sec);
        let old = retained.view();
        assert_eq!(old.publication(), Publication::Staged);
        assert_eq!(old.shelves()[0].title, "published");
        assert!(state.hubs_commit_staged(sec, true));

        let current = state.hubs_snapshot(sec);
        assert!(!Arc::ptr_eq(
            retained.data.as_ref().unwrap(),
            current.data.as_ref().unwrap()
        ));
        assert_eq!(current.view().publication(), Publication::Committed);
        assert_eq!(current.view().shelves()[0].title, "replacement");
        assert_eq!(
            old.shelves()[0].title,
            "published",
            "the old frame stays valid"
        );
    }

    #[test]
    fn watched_edits_are_copy_on_write_and_an_unmatched_edit_copies_nothing() {
        let _g = crate::testlock::serial();
        let (mut state, sec) = seeded_section();
        let (sid, rk) = {
            let section = state.state_mut(sec).unwrap();
            section.hubs.staged = Some(section.hubs.committed.clone());
            let item = &section.hubs.committed[0].items[1];
            (item.sid, item.rk.clone())
        };

        let retained = state.hubs_snapshot(sec);
        let old = retained.view();
        let staged_before = state.state_mut(sec)
            .unwrap()
            .hubs
            .staged
            .as_ref()
            .unwrap()
            .clone();
        assert!(!state.hubs_set_watched_local(sid, "not-present", true));
        let after_miss = state.hubs_snapshot(sec);
        assert!(Arc::ptr_eq(
            retained.data.as_ref().unwrap(),
            after_miss.data.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            &staged_before,
            state.state_mut(sec)
                .unwrap()
                .hubs
                .staged
                .as_ref()
                .unwrap()
        ));

        assert!(state.hubs_set_watched_local(sid, &rk, true));
        let edited = state.hubs_snapshot(sec);
        assert!(!Arc::ptr_eq(
            retained.data.as_ref().unwrap(),
            edited.data.as_ref().unwrap()
        ));
        assert!(
            !old.shelves()[0].items[1].watched,
            "retained data is immutable"
        );
        assert!(edited.view().shelves()[0].items[1].watched);
        assert!(state.hubs_commit_staged(sec, true));
        assert!(
            state.hubs_snapshot(sec).view().shelves()[0].items[1].watched,
            "held data was edited too"
        );
    }

    #[test]
    fn a_retained_snapshot_survives_reset_and_the_reused_index_gets_a_new_identity() {
        let _g = crate::testlock::serial();
        let (mut state, sec) = seeded_section();
        let retained = state.hubs_snapshot(sec);
        let old = retained.view();
        let old_id = old.id().unwrap();

        state.reset();
        assert_eq!(state.hubs_snapshot(sec).view().id(), None);
        super::super::seed_two_source_table_for_owner_test(&mut state);
        let replacement_snapshot = state.hubs_snapshot(sec);
        let replacement = replacement_snapshot.view();
        assert_ne!(replacement.id().unwrap().epoch, old_id.epoch);
        assert_eq!(old.shelves()[0].title, "published");
        assert_eq!(old.id(), Some(old_id));
    }

    /// **The first-paint window expiring publishes ZERO shelves**, and the page becomes usable at
    /// the offset it already has. It has to be a separate policy from the retry ladder, because
    /// that ladder is INFINITE (2/4/8/16/30s then 30s forever) — "commit when the fetch is
    /// terminal" never arrives on a dead server, and the page would wait for it.
    #[test]
    fn the_first_paint_window_commits_nothing_and_makes_the_page_usable() {
        let mut h = armed();
        assert_eq!(h.publication(), Publication::Fetching);
        for _ in 0..FIRST_PAINT_FRAMES - 1 {
            assert!(!h.tick(), "still inside the window");
        }
        assert!(h.tick(), "the window closed, and that is a repaint");
        assert_eq!(h.publication(), Publication::Committed);
        assert!(h.shelves().is_empty(), "committed to nothing, deliberately");
        assert!(
            !h.tick(),
            "…and it settles rather than reporting every frame"
        );
    }

    /// **A landing INSIDE the window still asks whether the ground may move.** It used to commit
    /// itself, on the reasoning that a page with no layout yet has nothing to protect — but the
    /// window is four SECONDS, and `/tmp/butaca-library` plus two presses is a viewer standing
    /// in the grid well inside it. So the first paint takes the same permission every later one
    /// does; at the head that is granted in the same frame the landing pumps, which is why nothing
    /// waits in the ordinary case.
    #[test]
    fn the_first_answer_inside_the_window_still_asks_before_it_moves_the_ground() {
        let mut h = armed();
        h.land_ok(vec![row("A"), row("B")]);
        assert_eq!(h.publication(), Publication::Staged);
        assert!(
            h.shelves().is_empty(),
            "a viewer already in the grid does not get the document moved under them"
        );

        assert!(!h.commit_staged(false), "…not while they are looking at it");
        assert!(h.commit_staged(true), "…and at the head it goes in at once");
        assert_eq!(h.publication(), Publication::Committed);
        assert_eq!(h.shelves().len(), 2);
    }

    /// …and the window is not what holds a staged first landing, so it neither publishes over it
    /// nor keeps reporting a change nobody can see.
    #[test]
    fn the_window_expiring_over_a_staged_first_landing_publishes_nothing() {
        let mut h = armed();
        h.land_ok(vec![row("A")]);
        for _ in 0..FIRST_PAINT_FRAMES + 5 {
            assert!(
                !h.tick(),
                "the window has nothing to say about a held answer"
            );
        }
        assert_eq!(h.publication(), Publication::Staged);
        assert!(h.shelves().is_empty());
        assert!(h.commit_staged(true));
        assert_eq!(h.shelves().len(), 1, "and the answer was not lost");
    }

    /// **A late success STAGES rather than moving a grid the user is looking at.** This is the
    /// case the whole machine exists for: the shelf count sets the grid's absolute offset, so a
    /// 12-shelf landing after the window closed would move a focused row by 6,588px.
    #[test]
    fn a_late_success_is_staged_and_commits_only_when_the_ground_may_move() {
        let mut h = armed();
        for _ in 0..FIRST_PAINT_FRAMES {
            h.tick();
        }
        h.land_ok(vec![row("A")]);
        assert_eq!(h.publication(), Publication::Staged);
        assert!(h.shelves().is_empty(), "the document has not moved");

        assert!(
            !h.commit_staged(false),
            "the user is looking at it: nothing publishes"
        );
        assert_eq!(h.publication(), Publication::Staged);

        assert!(h.commit_staged(true), "…and at the floor it does");
        assert_eq!(h.publication(), Publication::Committed);
        assert_eq!(h.shelves().len(), 1);
        assert!(!h.commit_staged(true), "exactly once");
    }

    /// A NEWER success replaces an older staged one — the freshest answer is the one worth showing
    /// when the page finally lets it through.
    #[test]
    fn a_newer_landing_replaces_an_older_staged_set() {
        let mut h = armed();
        h.land_ok(vec![row("first")]);
        h.land_ok(vec![row("second")]);
        h.land_ok(vec![row("third")]);
        assert_eq!(h.publication(), Publication::Staged);
        h.commit_staged(true);
        assert_eq!(h.shelves()[0].title, "third");
    }

    /// **Removing an item from Continue Watching removes it from the HELD set too.** The
    /// optimistic edit took the committed shelves and not the staged ones, so a refresh waiting to
    /// publish still carried the row: the tile vanished on the press and came back the moment the
    /// ground was next allowed to move, which reads as the press having failed. Its sibling
    /// `set_watched_local` already edited both.
    ///
    /// Driven through the real `left_the_deck` against seeded `browse` state, because the defect
    /// is precisely which of the two sets that function reaches.
    #[test]
    fn removing_from_the_deck_reaches_the_staged_set_as_well() {
        let _g = crate::testlock::serial();
        let _t = crate::server::session::TempSession::new("deckstage");
        _t.watching("u-deckstage");
        let mut state = super::super::BrowseState::default();
        super::super::seed_two_source_table_for_owner_test(&mut state);
        let sec = state.cur();

        // a committed deck…
        seed_shelves_for_owner_test(&mut state, sec, &["movie.inprogress.1"], 3);
        let (sid, rk) = {
            let st = state.state_mut(sec).expect("a seeded section");
            let m = &st.hubs.committed[0].items[1];
            (m.sid, m.rk.clone())
        };

        // …and an older refresh held behind it, carrying the same item
        {
            let st = state.state_mut(sec).expect("a seeded section");
            st.hubs.staged = Some(st.hubs.committed.clone());
        }

        let retained = state.hubs_snapshot(sec);
        let old = retained.view();

        assert!(state.hubs_left_the_deck(sid, &rk), "the visible deck loses the row");
        let edited = state.hubs_snapshot(sec);
        assert!(!Arc::ptr_eq(
            retained.data.as_ref().unwrap(),
            edited.data.as_ref().unwrap()
        ));
        assert_eq!(
            old.shelves()[0].items.len(),
            3,
            "the retained deck is immutable"
        );
        assert_eq!(edited.view().shelves()[0].items.len(), 2);
        {
            let st = state.state_mut(sec).unwrap();
            assert!(st.hubs.committed[0].items.iter().all(|m| m.rk != rk));
        }

        // publishing the held answer must not bring it back
        assert!(state.hubs_commit_staged(sec, true));
        {
            let st = state.state_mut(sec).unwrap();
            assert!(
                st.hubs
                    .committed
                    .iter()
                    .flat_map(|sh| sh.items.iter())
                    .all(|m| m.rk != rk),
                "the staged set resurrected a row the user removed"
            );
        }
    }
    /// A section nobody armed is inert: it neither ticks toward publishing nor counts anything
    /// down. That is what makes this module DORMANT while `pump` already calls `tick_all` — the
    /// wiring is real, the behaviour is not reachable until a screen calls `kick`.
    #[test]
    fn an_unarmed_section_is_inert() {
        let mut h = SecHubs::default();
        for _ in 0..FIRST_PAINT_FRAMES * 2 {
            assert!(!h.tick());
        }
        assert_eq!(h.publication(), Publication::Fetching);
        assert!(h.shelves().is_empty());
    }
}
