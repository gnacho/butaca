//! Collection-page data model: one physically owned state machine, one generation-stamped
//! mailbox, and one small worker at a time. Header metadata and paged children are separate
//! jobs. All network work runs off the frame thread; landings are applied by
//! [`CollectionState::pump_with_gate`]. The page is the shared collections route: a Jellyfin
//! BoxSet open and a search collection hit both land here.

use crate::registry::ServerId;
use crate::pms::PmsMovie;
use std::panic::catch_unwind;
use std::sync::Arc;

pub(crate) const PAGE_SIZE: usize = 60;
const RETRY_FRAMES: u32 = 120;

/// One collection as a route names it: by `rk` (its server item id) when the link carries one,
/// else by `sec` + `tag`, the second identity a search fold can carry for servers that have a
/// collection tag id space. `name` is the title shown until the collection's own metadata
/// lands — it is never identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CollectionRef {
    pub(crate) sid: ServerId,
    pub(crate) rk: String,
    pub(crate) sec: i64,
    pub(crate) tag: i64,
    pub(crate) name: String,
}

impl CollectionRef {
    /// A link that knows the collection's item id (a library tile, a box set card).
    pub(crate) fn by_rk(sid: ServerId, rk: &str, sec: i64, name: &str) -> Self {
        Self { sid, rk: rk.to_owned(), sec, tag: 0, name: name.to_owned() }
    }

    /// THE collection identity rule. Two item ids compare when both sides carry one; otherwise
    /// the server, section and a non-zero tag id must all agree. An item id is never compared
    /// with a tag id, and a zero tag names nothing.
    pub(crate) fn same_collection(&self, other: &Self) -> bool {
        if self.sid != other.sid {
            return false;
        }
        if !self.rk.is_empty() && !other.rk.is_empty() {
            self.rk == other.rk
        } else {
            self.sec == other.sec && self.tag != 0 && self.tag == other.tag
        }
    }

    /// Names no collection at all — neither an item id nor a tag to resolve.
    pub(crate) fn is_identityless(&self) -> bool {
        self.rk.is_empty() && self.tag == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CollectionTarget {
    pub(crate) id: CollectionRef,
    /// Number of children the visible grid currently asks the store to make available.
    pub(crate) want: usize,
}

/// The order a collection lists its members in — its owner's `collectionSort`, which the page names
/// over its grid ("Items · Release order"). An order the server did not state is not guessed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum CollectionOrder {
    Release,
    Title,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CollectionStatus {
    #[default]
    Loading,
    Ready,
    Empty,
    Unavailable,
    Failed,
}

pub(crate) struct Collection {
    /// The identity the page was opened with; `id.rk` is filled in once a tag route resolves.
    pub(crate) id: CollectionRef,
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) thumb: String,
    pub(crate) child_count: usize,
    /// The member order the header stated, if it stated one.
    pub(crate) order: Option<CollectionOrder>,
    pub(crate) items: Vec<PmsMovie>,
    pub(crate) total: usize,
    pub(crate) status: CollectionStatus,
    pub(crate) more: bool,
    /// Server rows consumed so far — the next page's `X-Plex-Container-Start`. NOT
    /// `items.len()`: `run_job` drops rows that are not `pms::listable` (a clip, an artist), so
    /// the grid can hold fewer members than the server has handed over, and paging from
    /// `items.len()` would re-request rows already seen and append them twice.
    offset: usize,
    want: usize,
    header_ready: bool,
}

impl Collection {
    /// A collection opened (or reopened) with nothing landed yet: its title is the link's name
    /// until the header replaces it.
    fn loading(id: CollectionRef, want: usize) -> Self {
        Self { title: id.name.clone(), id, summary: String::new(), thumb: String::new(),
            child_count: 0, order: None, items: Vec::new(), total: 0, status: CollectionStatus::Loading,
            more: true, offset: 0, want, header_ready: false }
    }

    /// Whether the header (title, art, summary) has landed — before it, an empty `thumb` is not
    /// yet an answer.
    pub(crate) fn header_ready(&self) -> bool { self.header_ready }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct CollectionView<'a> { current: Option<&'a Collection> }

impl<'a> CollectionView<'a> {
    pub(crate) fn current(self) -> Option<&'a Collection> { self.current }
}

pub(crate) struct CollectionState {
    current: Option<Collection>,
    generation: u32,
    retry_cd: u32,
}

impl Default for CollectionState {
    fn default() -> Self { Self { current: None, generation: 0, retry_cd: 0 } }
}

impl CollectionState {
    pub(crate) fn view(&self) -> CollectionView<'_> { CollectionView { current: self.current.as_ref() } }

    fn supersede(&mut self, adapter: &CollectionAdapter) {
        self.generation = self.generation.checked_add(1).expect("collection generation exhausted");
        adapter.fetch.clear();
        self.retry_cd = 0;
    }

    pub(crate) fn run(&mut self, adapter: &Arc<CollectionAdapter>, cmd: crate::stores::collection::CollectionCmd) -> bool {
        use crate::stores::collection::CollectionCmd;
        match cmd {
            CollectionCmd::Open { target } => {
                if let Some(current) = self.current.as_mut().filter(|c| c.id.same_collection(&target.id)) {
                    let old = current.want;
                    current.want = current.want.max(target.want);
                    let retry = current.status == CollectionStatus::Failed;
                    if retry {
                        current.status = CollectionStatus::Loading;
                        self.retry_cd = 0;
                    }
                    return current.want != old || retry;
                }
                self.supersede(adapter);
                self.current = Some(Collection::loading(target.id, target.want.max(PAGE_SIZE)));
                true
            }
            CollectionCmd::Close | CollectionCmd::Reset => {
                self.supersede(adapter);
                self.current.take().is_some()
            }
            // The item menu's Mark watched/unwatched reaches every store that can be drawing the
            // item (`viewstate`'s fan-out); a member's disc must flip on this page too, not one
            // refetch later.
            CollectionCmd::SetWatchedLocal { sid, rk, on } => {
                let Some(c) = self.current.as_mut() else { return false };
                let mut hit = false;
                for item in c.items.iter_mut()
                    .filter(|item| crate::registry::same_item((item.sid, &item.rk), (sid, &rk))) {
                    crate::pms::set_watched(item, on);
                    hit = true;
                }
                hit
            }
        }
    }

    pub(crate) fn pump_with_gate(&mut self, adapter: &Arc<CollectionAdapter>, gate: &crate::ui::landgate::Gate) -> bool {
        let mut changed = false;
        if self.retry_cd > 0 { self.retry_cd -= 1; }
        let reply = crate::app::bootstrap::stores::take_store_landing(
            gate, crate::stores::StoreId::Collection, "collection", 0, &adapter.fetch);
        if let Some(reply) = reply {
            crate::ui::idle::invalidate();
            if reply.gen == self.generation { changed |= self.apply(reply.what); }
        }
        self.maybe_spawn(adapter);
        changed
    }

    fn job(&self) -> Option<Job> {
        let c = self.current.as_ref()?;
        if c.status == CollectionStatus::Unavailable || c.status == CollectionStatus::Empty { return None; }
        if !c.header_ready { return Some(Job::Header { rk: c.id.rk.clone() }); }
        if c.more && c.items.len() < c.want {
            return Some(Job::Children { rk: c.id.rk.clone(), start: c.offset });
        }
        None
    }

    fn maybe_spawn(&mut self, adapter: &Arc<CollectionAdapter>) {
        if adapter.fetch.busy() || self.retry_cd > 0 { return; }
        let Some(c) = self.current.as_ref() else { return };
        // The installed-client guard is load-bearing: slot 0 is also a valid Plex slot in this
        // build. A collection opened with no item id (a search tag fold's second identity) names
        // nothing this server can answer — there is no tag-id space to resolve it against — so
        // the page it opened stays Loading rather than guessing a fetch.
        if c.id.sid != crate::jellyfin::SERVER_ID || c.id.rk.is_empty() { return; }
        let Some(jc) = crate::jellyfin::client() else { return; };
        let Some(job) = self.job() else { return; };
        let generation = self.generation;
        let sid = c.id.sid;
        adapter.fetch.claim();
        let worker_adapter = Arc::clone(adapter);
        let request = serde_json::json!({"store":"collection","slot":0,"gen":generation,
            "sid":sid.raw(),"client":0,"job":job});
        let spawned = crate::app::bootstrap::stores::admit(request, || crate::task::spawn_small("collection", move || {
            let what = catch_unwind(|| run_job_jellyfin(jc, sid, job)).unwrap_or(Landing::Transport);
            worker_adapter.land(generation, what);
        }));
        if !spawned { adapter.fetch.release(); }
    }

    fn apply(&mut self, landing: Landing) -> bool {
        let Some(c) = self.current.as_mut() else { return false };
        match landing {
            Landing::Header { rk, head } => {
                // A tag route's resolution arrives with the row it was resolved from, which IS the
                // header: no second GET for the same fields.
                if let Some(rk) = rk { c.id.rk = rk; }
                if !head.title.is_empty() { c.title = head.title; }
                c.thumb = head.thumb;
                c.summary = head.summary;
                c.child_count = head.child_count;
                c.order = head.order;
                c.header_ready = true;
                c.status = CollectionStatus::Loading;
                true
            }
            Landing::Page { start, got, items, total } => {
                if start != c.offset { return false; }
                c.offset = start + got;
                c.total = total.max(c.offset);
                c.items.extend(items);
                // A page of zero rows ends the listing whatever `totalSize` claimed, or a server
                // that over-reports would be asked for the same empty offset forever.
                c.more = got > 0 && c.offset < c.total;
                c.status = if !c.items.is_empty() { CollectionStatus::Ready }
                    else if c.more { CollectionStatus::Loading }
                    else { CollectionStatus::Empty };
                true
            }
            Landing::Denied | Landing::Missing => {
                c.status = CollectionStatus::Unavailable;
                c.more = false;
                true
            }
            Landing::Transport => {
                self.retry_cd = RETRY_FRAMES;
                if c.items.is_empty() { c.status = CollectionStatus::Failed; }
                true
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn generation(&self) -> u32 { self.generation }

    #[cfg(test)]
    pub(crate) fn install_for_test(&mut self, items: Vec<PmsMovie>, status: CollectionStatus) {
        let Some(c) = self.current.as_mut() else { return };
        c.title = if c.id.name.is_empty() { "Collection".into() } else { c.id.name.clone() };
        c.summary = "A collection summary long enough for screen layout tests.".into();
        c.child_count = items.len();
        c.total = items.len();
        c.offset = items.len();
        c.items = items;
        c.header_ready = true;
        c.more = false;
        c.status = status;
    }

    #[cfg(test)]
    pub(crate) fn edit_for_test(&mut self, edit: impl FnOnce(&mut Collection)) {
        if let Some(c) = self.current.as_mut() { edit(c); }
    }

    #[cfg(test)]
    pub(crate) fn take_landing_for_test(&mut self, adapter: &CollectionAdapter) -> bool {
        let Some(mail) = adapter.fetch.take() else { return false };
        mail.gen == self.generation && self.apply(mail.what)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
enum Job {
    Header { rk: String },
    Children { rk: String, start: usize },
}

/// A collection row's header fields, as the metadata read returns them.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Header {
    title: String,
    thumb: String,
    summary: String,
    child_count: usize,
    #[serde(default)]
    order: Option<CollectionOrder>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum Landing {
    /// The collection's header.
    Header { rk: Option<String>, head: Header },
    /// `got` is the server rows this page consumed; `items` only the listable ones among them.
    Page { start: usize, got: usize, items: Vec<PmsMovie>, total: usize },
    Denied,
    Missing,
    Transport,
}

/// A BoxSet open's worker: the header is the box set's own item (`/Items/{id}`), and its members
/// are `/Items?ParentId={id}`. A page of members re-uses the browse row mapping
/// (`movie_from_dto`), so the grid draws them exactly as browse would.
fn run_job_jellyfin(
    jc: &'static crate::jellyfin::JfClient,
    sid: ServerId,
    job: Job,
) -> Landing {
    let run = || -> Result<Landing, Landing> {
        Ok(match job {
            Job::Header { rk } => {
                let dto = jc.item_detail(&rk).ok_or(Landing::Transport)?;
                Landing::Header { rk: None, head: boxset_header(&dto) }
            }
            Job::Children { rk, start } => {
                let page = jc
                    .collection_children(&rk, start as i64, PAGE_SIZE as i64)
                    .ok_or(Landing::Transport)?;
                let total = page.total.max(0) as usize;
                let got = page.items.len();
                let items = page
                    .items
                    .iter()
                    .filter_map(|it| crate::jellyfin::movie_from_dto(it, sid, 0))
                    .collect();
                Landing::Page { start, got, items, total }
            }
        })
    };
    run().unwrap_or_else(|failed| failed)
}

/// A BoxSet's header fields from its own item DTO — the same Name/art/ChildCount the browse card
/// was built from, so the collection page's title and art match the tile that opened it. A box
/// set states no member order, so `order` is never guessed.
fn boxset_header(dto: &crate::jellyfin::BaseItemDto) -> Header {
    let m = crate::jellyfin::boxset_from_dto(dto, crate::jellyfin::SERVER_ID).unwrap_or_default();
    Header {
        title: if m.title.is_empty() { dto.name.clone() } else { m.title },
        thumb: m.thumb,
        summary: m.summary,
        child_count: m.child_count.max(0) as usize,
        order: None,
    }
}

#[cfg(test)]
fn header(title: &str, child_count: usize) -> Header {
    Header { title: title.into(), thumb: String::new(), summary: String::new(), child_count, order: None }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Mail { gen: u32, what: Landing }

pub(crate) fn validate_record(slot: u32, value: &serde_json::Value) -> Result<(), &'static str> {
    if slot != 0 { return Err("invalid collection slot"); }
    let mail: Mail = serde_json::from_value(value.clone()).map_err(|_| "invalid collection reply")?;
    if mail.gen == 0 { return Err("invalid collection reply generation"); }
    if serde_json::to_value(&mail).ok().as_ref() != Some(value) { return Err("noncanonical collection reply"); }
    Ok(())
}

#[derive(Default)]
pub(crate) struct CollectionAdapter { fetch: crate::stores::Fetch<Mail> }

impl CollectionAdapter {
    fn land(&self, generation: u32, what: Landing) {
        self.fetch.post(Mail { gen: generation, what }, |old| old.gen < generation);
    }

    #[cfg(test)]
    pub(crate) fn land_status_for_test(&self, generation: u32, status: CollectionStatus) {
        let what = match status {
            CollectionStatus::Empty => Landing::Page { start: 0, got: 0, items: Vec::new(), total: 0 },
            CollectionStatus::Unavailable => Landing::Denied,
            CollectionStatus::Failed => Landing::Transport,
            _ => Landing::Missing,
        };
        self.land(generation, what);
    }

    #[cfg(test)]
    pub(crate) fn land_missing_for_test(&self, generation: u32) {
        self.land(generation, Landing::Missing);
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::stores::collection::CollectionCmd;

    fn set_target(rk: &str, name: &str) -> CollectionTarget {
        CollectionTarget { id: CollectionRef::by_rk(ServerId::UNSET, rk, 1, name), want: PAGE_SIZE }
    }

    #[test]
    fn paging_asks_for_the_next_page_only_when_the_grid_wants_more() {
        let adapter = Arc::new(CollectionAdapter::default());
        let mut state = CollectionState::default();
        let target = set_target("50001", "Set");
        state.run(&adapter, CollectionCmd::Open { target: target.clone() });
        assert!(matches!(state.job(), Some(Job::Header { .. })));
        let generation = state.generation();
        adapter.land(generation, Landing::Header { rk: None, head: header("Set", 130) });
        assert!(state.take_landing_for_test(&adapter));
        assert!(matches!(state.job(), Some(Job::Children { start: 0, .. })));
        let page = (0..PAGE_SIZE).map(|i| PmsMovie { rk: i.to_string(), ..Default::default() }).collect();
        adapter.land(generation, Landing::Page { start: 0, got: PAGE_SIZE, items: page, total: 130 });
        assert!(state.take_landing_for_test(&adapter));
        let c = state.view().current().unwrap();
        assert_eq!((c.items.len(), c.more, c.status), (PAGE_SIZE, true, CollectionStatus::Ready));
        assert!(state.job().is_none(), "the grid has not asked for more yet");

        let mut more = target;
        more.want = 2 * PAGE_SIZE;
        assert!(state.run(&adapter, CollectionCmd::Open { target: more }),
            "a larger want on the same identity is a change, not a reopen");
        assert_eq!(state.generation(), generation, "paging does not supersede the collection");
        assert!(matches!(state.job(), Some(Job::Children { start, .. }) if start == PAGE_SIZE));
        adapter.land(generation, Landing::Page { start: 0, got: 0, items: Vec::new(), total: 130 });
        assert!(!state.take_landing_for_test(&adapter), "a page for the wrong offset is dropped");
    }

    /// Regression: paging used `items.len()` as the next server offset, so a page whose rows were
    /// not all listable re-requested rows it had already consumed and appended them twice. The
    /// offset is the server's, not the grid's.
    #[test]
    fn a_page_with_unlisted_rows_advances_by_the_rows_the_server_sent() {
        let adapter = Arc::new(CollectionAdapter::default());
        let mut state = CollectionState::default();
        state.run(&adapter, CollectionCmd::Open { target: set_target("50001", "Set") });
        let generation = state.generation();
        adapter.land(generation, Landing::Header { rk: None, head: header("Set", 130) });
        assert!(state.take_landing_for_test(&adapter));
        let listed = (0..PAGE_SIZE - 5).map(|i| PmsMovie { rk: i.to_string(), ..Default::default() }).collect();
        adapter.land(generation, Landing::Page { start: 0, got: PAGE_SIZE, items: listed, total: 130 });
        assert!(state.take_landing_for_test(&adapter));
        state.run(&adapter, CollectionCmd::Open { target: CollectionTarget {
            want: 2 * PAGE_SIZE, ..set_target("50001", "Set") } });
        assert!(matches!(state.job(), Some(Job::Children { start, .. }) if start == PAGE_SIZE),
            "the next page starts after every row the server sent, listed or not");

        // A collection whose every row is unlisted ends Empty rather than paging forever.
        let mut state = CollectionState::default();
        state.run(&adapter, CollectionCmd::Open { target: set_target("50002", "Clips") });
        let generation = state.generation();
        adapter.land(generation, Landing::Header { rk: None, head: header("Clips", 3) });
        assert!(state.take_landing_for_test(&adapter));
        adapter.land(generation, Landing::Page { start: 0, got: 3, items: Vec::new(), total: 3 });
        assert!(state.take_landing_for_test(&adapter));
        let c = state.view().current().unwrap();
        assert_eq!((c.status, c.more), (CollectionStatus::Empty, false));
        assert!(state.job().is_none());
    }

    /// The identity rule moved here with [`CollectionRef`]: two item ids compare as ids, a tag
    /// identity compares server, section and non-zero tag, and the two spaces never mix.
    #[test]
    fn collection_identity_never_compares_a_tag_with_an_item_id() {
        let sid = ServerId::from_raw(2);
        let at = |rk: &str, tag| CollectionRef { sid, rk: rk.into(), sec: 4, tag, name: "A".into() };
        assert!(at("50077", 77).same_collection(&at("50077", 99)),
            "two resolved identities compare their item id");
        assert!(!at("50077", 77).same_collection(&at("50078", 77)),
            "different non-empty item ids do not fall through to tag identity");
        assert!(at("", 77).same_collection(&at("", 77)),
            "tag-only identities compare server, section and non-zero tag");
        assert!(at("50077", 77).same_collection(&at("", 77)),
            "a resolved identity still matches the tag route it was resolved from");
        assert!(!at("", 0).same_collection(&at("", 0)), "zero is not a tag identity");
        assert!(!at("50077", 0).same_collection(&at("", 50077)),
            "an item id is never compared to a numeric tag id");
        assert!(!at("50077", 0).same_collection(&CollectionRef { sid: ServerId::from_raw(5), ..at("50077", 0) }),
            "another server's collection is another collection");
        assert!(at("", 0).is_identityless() && !at("", 7).is_identityless() && !at("1", 0).is_identityless());
    }

    #[test]
    fn a_local_watched_edit_flips_the_member_in_place() {
        let adapter = Arc::new(CollectionAdapter::default());
        let mut state = CollectionState::default();
        state.run(&adapter, CollectionCmd::Open { target: set_target("50001", "Set") });
        state.install_for_test(vec![PmsMovie { rk: "a".into(), unwatched: true, ..Default::default() }],
            CollectionStatus::Ready);
        assert!(!state.run(&adapter, CollectionCmd::SetWatchedLocal { sid: ServerId::UNSET,
            rk: "b".into(), on: true }), "an item not on the page changes nothing");
        assert!(state.run(&adapter, CollectionCmd::SetWatchedLocal { sid: ServerId::UNSET,
            rk: "a".into(), on: true }));
        let item = &state.view().current().unwrap().items[0];
        assert!(item.watched && !item.unwatched);
    }
}
