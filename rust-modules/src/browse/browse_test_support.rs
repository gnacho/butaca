//! Shared fixtures and helpers for the `browse` module's test files split out below.

use super::*;

pub(super) struct TestBrowse {
    pub(super) state: BrowseState,
    pub(super) adapter: Arc<BrowseAdapter>,
}
impl Default for TestBrowse {
    fn default() -> Self {
        Self {
            state: BrowseState::default(),
            adapter: Arc::new(BrowseAdapter::default()),
        }
    }
}
impl TestBrowse {
    pub(super) fn reset(&mut self) {
        self.state.reset_owned(&self.adapter);
    }

    pub(super) fn seed_sources(&mut self, sources: Vec<BrowseSource>) {
        self.reset();
        self.state.sources = sources;
    }

    pub(super) fn append_sections(&mut self, source: usize, sections: Vec<(i64, String, SecKind)>) {
        self.state.append_sections_with(source, sections, None);
    }

    pub(super) fn section_title(&self, index: usize) -> &str {
        self.state.sections.get(index).map_or("", |section| section.title.as_str())
    }

    pub(super) fn section_count(&self) -> usize {
        self.state.sections.len()
    }

    pub(super) fn pinned(&self, index: usize) -> bool {
        self.state.pinned_for_test(index)
    }

    pub(super) fn source_rows(&self) -> Vec<SrcRow> {
        let Some(kind) = self.state.section_kind(self.state.cur()) else {
            return Vec::new();
        };
        self.state.rows_where(|section| section.kind == kind && section.pinned)
    }

    pub(super) fn kind_position(&self, index: usize) -> Option<(usize, usize)> {
        let kind = self.state.section_kind(index)?;
        if !self.pinned(index) {
            return None;
        }
        let (mut position, mut count) = (0, 0);
        for (candidate, section) in self.state.sections.iter().enumerate() {
            if section.kind != kind || !section.pinned {
                continue;
            }
            if candidate == index {
                position = count;
            }
            count += 1;
        }
        (count > 1).then_some((position + 1, count))
    }

    pub(super) fn tab_count(&self) -> usize {
        self.state.tab_kinds().count()
    }

    pub(super) fn tab_title(&self, tab: usize) -> &str {
        match self.state.tab_kinds().nth(tab) {
            Some(SecKind::Movie) => "Movies",
            Some(SecKind::Show) => "TV Shows",
            Some(SecKind::Video) => "Other Videos",
            None => "",
        }
    }

    pub(super) fn tab_of_section(&self, section: usize) -> Option<usize> {
        self.state.section_kind(section).and_then(|kind| self.state.tab_of_kind(kind))
    }

    pub(super) fn loading_initial(&self) -> bool {
        self.state.cur_state().is_some_and(|state| {
            state.total < 0 && matches!(state.fetch, SecFetch::Loading)
        })
    }


    pub(super) fn first_run_asks(&self) -> bool {
        let session = crate::server::session::peek();
        crate::server::pins::asks(
            self.state.sources.len(),
            session.pins_for(&crate::server::session::current_profile_key()),
        )
    }
}
// ---- the three-state fetch machine ---------------------------------------------------------
//
// These drive the same owned pump core that reports to `ui::idle`'s process-global flag — the
// exact obligation `ui/xfade.rs` inherited when its `tick` started doing the same — so they
// take the CRATE-wide serial lock, not a module-local one. Their local state has one default
// row and no sections; `maybe_spawn` returns before it can reach the network, so nothing here
// spawns a worker.

// ---- the SOURCE's own state, one layer up ---------------------------------------------------
//
// Same three states, one layer up, and graded through the per-source flags rather than through
// `ensure_sections`: the fetch half needs a server, and a host test that reached for one would
// be dialling whatever address another module's test had just registered.
//
// `cur_source_state` is a PROJECTION of `reachable`/`sections_done` — there is no fourth field
// to set, which is the point of resolving it that way: the flags the Sources list already dims
// a group by are the flags the read-out reads.

/// Seed one source in a chosen phase and make it the CURRENT server, so the empty-table
/// fallback in [`cur_source_state`] resolves to it rather than to whatever the registry was
/// left holding. Registration dials nothing — it publishes a slot.
pub(super) fn seed_one_source(
    browse: &mut TestBrowse,
    reachable: bool,
    sections_done: bool,
) -> usize {
    // The CURRENT server's id, registered or not — see `seed_sources_for_test` for why nothing
    // is registered here. It makes `cur_source_idx`'s empty-table fallback resolve to this row.
    browse.seed_sources(vec![BrowseSource {
        sid: crate::registry::current(),
        machine_id: "mach-0".into(),
        household: true,
        name: "nas-home".into(),
        handle: "friend".into(),
        state: if reachable {
            SourceState::Reachable
        } else {
            SourceState::Unreachable
        },
        tier: None,
        sections_done,
        counts_done: true,
        retry_cd: 0,
    }]);
    0
}
// ---- the (source, section) table ------------------------------------------------------------
//
// These seed SOURCES directly and mark every phase done, so `maybe_discover` picks nothing and
// no worker is spawned — the same discipline as the fetch-machine tests above, one layer up.
// Their `sid` is `UNSET`, which resolves to no client, so even a spawn could reach no socket.

pub(super) fn a_source(name: &str, handle: &str, reachable: bool) -> BrowseSource {
    BrowseSource {
        sid: ServerId::UNSET,
        // the machine id doubles as the fixture's identity
        machine_id: name.to_string(),
        // A fixture that volunteers an owner handle is a share (never the household's), the
        // same reading the Sources list draws from the credit.
        household: handle.is_empty(),
        name: name.into(),
        handle: handle.into(),
        state: if reachable {
            SourceState::Reachable
        } else {
            SourceState::Unreachable
        },
        tier: None,
        sections_done: true,
        counts_done: true,
        retry_cd: 0,
    }
}
// ---- the Home selection: defaults, persistence, and one answer per PROFILE ------------------
//
// The RULES are `server::pins` and are graded there, pure. What is graded here is the plumbing
// around them, which is where the failures actually live: does an answer reach the disk, does
// it come back, and does it come back to the person who gave it.

/// The session-isolation guard, under the name this module's tests have always called it.
/// It is `server::session::TempSession` now — one guard rather than the three near-identical
/// copies that had grown here, in `ui::onboard` and in `auth`; the local alias is kept only so
/// the dozens of call sites below still read as pinning THIS module's per-profile answer.
pub(super) use crate::server::session::TempSession as TempPins;

/// **The household's own server as a MANAGED or Guest profile sees it** — the shape
/// [`a_source`] cannot express, because it derives the verdict from the handle.
///
/// `household:true` beside no credit, because a household server is credited to nobody
/// (`owner_credit`): what the pins rules read is the verdict alone now that the grant evidence
/// the roster used to carry went with the Plex arm (#166).
pub(super) fn a_household_source(name: &str) -> BrowseSource {
    BrowseSource {
        household: true,
        ..a_source(name, "", true)
    }
}

/// One account, two servers — seeded and discovered exactly as a boot does it.
///
/// **Owner-shaped**: `mac-mini` is `owned:true`, which is the ADMIN's view of the house. The
/// managed profile's view of the very same pair is [`seed_two_servers_managed`], and the two
/// differ in nothing a user could see — which is the property the household fix exists to give
/// back.
pub(super) fn seed_two_servers(browse: &mut TestBrowse) {
    seed_pair(browse, a_source("mac-mini", "", true));
}

/// The same two servers, granted to a Plex Home MANAGED profile: the family server arrives
/// `owned:false` with the admin's id on it, exactly as a friend's share does, and only the
/// household verdict tells them apart.
pub(super) fn seed_two_servers_managed(browse: &mut TestBrowse) {
    seed_pair(browse, a_household_source("mac-mini"));
}

fn seed_pair(browse: &mut TestBrowse, house: BrowseSource) {
    browse.seed_sources(vec![house, a_source("nas-home", "friend", true)]);
    browse.append_sections(
        0,
        vec![
            (1, "Movies".into(), SecKind::Movie),
            (2, "TV Shows".into(), SecKind::Show),
        ],
    );
    browse.append_sections(1, vec![(1, "Film Club".into(), SecKind::Movie)]);
}
