//! person — the actor/person page's data layer (the store `ui/person.rs` draws).
//!
//! Sibling of `metadata.rs` (the detail page's item store) and `browse.rs` (the Library's paged
//! catalog), and built on the same three pieces: [`crate::task::spawn_small`] + a `Mutex` mailbox
//! + a generation, applied on the MAIN thread by [`PersonState::pump`] once a frame. One
//! `PersonState` and rotated `Arc<PersonAdapter>` belong to each production Bridge; screens borrow
//! a `PersonView` through their frame `Cx`, so neither reads nor workers select process state.
//!
//! ## One backend answers; the other slots contribute nothing
//!
//! This page used to ask every registered server — the machine whose credit row opened it first,
//! then a `/hubs/search` resolve plus a `/library/people/{id}/media` sweep on every share,
//! merged into one filmography. That read layer is gone (issue #166): the ONLY fetch that
//! remains is the origin's, on the Jellyfin backend. Every other registry slot is born settled
//! ([`sources`]) and contributes nothing — an empty shelf from a slot this build cannot ask is an
//! answer, not a fetch to retry forever.
//!
//! ## The fetches, and their index space
//!
//! Three requests **per source**, plus TWO that are not per-source at all — in practice only the
//! origin's three ever fire:
//!
//! * **[`K_MEDIA`]** — the shelves: the backend's person-items read returns everything the person
//!   appears in as movies and series. Each row is stamped with the source's `ServerId`, which is
//!   what makes a card open on the machine it came from. A source that cannot be asked (`local`
//!   empty) never spawns this.
//! * **[`K_ROLES`]** — the character names, off the credit rows the shelf listing already carries.
//!   It is the ONE fetch here that DEPENDS on another — it can only be addressed once that
//!   source's shelves have landed, and [`address`] expresses that by keying it on that source's
//!   shelf keys, which are empty until then.
//! * **[`K_RESOLVE`]** - RETIRED with that read layer. The slot kind survives in the index
//!   space ([`fx`]/[`un_fx`]) but no worker is ever spawned for it: the origin's `personId` rides
//!   in on the credit row, and no other source is ever asked.
//! * **[`F_PROFILE`]** - the biography. It was a global read from the metadata provider
//!   (`GET <provider>/library/people/{tagKey}`, the wire facts are the field docs on the DTO
//!   vocabulary (`person::types`) below). Today the Jellyfin arm answers it from the person ITEM
//!   the credit row already named (`fetch_profile_jf`), and every other slot answers nothing.
//!   **Deliberately still single.** It is about the person, not about anybody's library, so
//!   fanning it out would be the same request N times for one answer.
//! * **[`F_CREDITS`]** - the filmography. It was the global credits read on that same host
//!   (`…/library/people/{tagKey}/credits` → `types::CreditGroup[]`), and its availability JOIN
//!   rode on the [`K_MEDIA`] answers this page already makes. There is no counterpart on the
//!   Jellyfin backend: the filmography IS the per-source shelves, so this mailbox asks for
//!   nothing there. The screen is [`crate::ui::filmography`] and the display model it draws is
//!   [`filmography`].
//!
//! Every fetch's plumbing is indexed by ONE flat key ([`fx`]/[`un_fx`]): its mailbox and its
//! single-flight claim as one [`Fetch`] ([`FETCH`]), its retry countdown beside them ([`RETRY_CD`],
//! whose doc says why that one cannot join the struct). The key is the registry **slot** rather
//! than a position in [`Person::srcs`]: a slot is stable for the life of the process, so a landing
//! can never be applied to a different server because the source list moved under it.
//!
//! **The header still MOUNTS on what it was handed.** `Role[]` already carries the name and the
//! headshot (`server::Tag`), so the page draws instantly and the biography fields fade in under the
//! name when they land. That ordering is the whole degrade strategy: the biography request can
//! fail, or the person can be one the server has never heard of, and the page must then read as
//! *finished* at portrait + name — never as a row of fields that failed to load. Nothing in the
//! header is a placeholder; each line is drawn only when it has content.
//!
//! **The id spaces are not interchangeable.** [`Person::key`] is the ORIGIN server's local
//! `personId` (the numeric `Tag::id`, or its `tagKey` when that server omitted the number) and
//! addresses `/media` **on `Person::sid` only**; every other source carries NO id at all —
//! there is no resolve left to obtain one ([`sources`]); [`Person::guid`] is the portable guid.
//! Keep them apart; do not collapse them.
use crate::registry::ServerId;
use crate::catalog::CatalogMovie;
use std::panic::catch_unwind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// The person screen's DTO vocabulary (`types::PersonProfile`, `types::CreditType`, `types::CreditGroup`, `Credit`,
// `types::CreditItem`) lives in the `types` child module. The page's DISPLAY model already owns a
// top-level `Credit`, so the DTO names stay qualified as `types::...` at every use site.
pub(crate) mod types;

/// Per-shelf item cap. A `CardRow` owns exactly [`crate::ui::card_row::MAX_ROW_ITEMS`] focus-scale
/// springs and `scale(i)` clamps past the end, so an item beyond the cap would draw with the last
/// cell's pop and — worse — never pop at all when focused (`update`'s loop can't reach its index).
/// It is also the perf ceiling the A53 budget wants: a shelf is a horizontal strip, not a grid.
const SHELF_MAX: usize = crate::ui::card_row::MAX_ROW_ITEMS;

/// How many departments the roles line names before it stops. The wire names every one, and on
/// a couch that turns a one-line kicker into "Actor, Writer, Producer, Composer, Costume Makeup"
/// for a jobbing actor (Peter Sallis has five). The wire order is most-credits-first, so the first
/// three ARE the ones worth naming.
const MAX_ROLES: usize = 3;

/// One shelf of a person's page. The three fields move together and must never be updated apart:
/// the captions describe THESE items by index, and the total is the number to print rather than
/// `items.len()`.
#[derive(Default)]
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Shelf {
    /// the tiles, capped at [`SHELF_MAX`]
    pub(crate) items: Vec<CatalogMovie>,
    /// How many items of this kind the `/media` response REALLY held. Distinct from `items.len()`,
    /// which is capped at [`SHELF_MAX`] (a `CardRow` spring-array limit): a prolific actor has 60
    /// movies in the library, and a heading that read "24" would be the cap masquerading as a fact
    /// about the person. On the MERGED shelf it is the sum across every source, for the same
    /// reason — the count is a fact about the person, not about the row that fitted.
    pub(crate) total: usize,
    /// The character each tile is this person's credit for (`"Wallace (voice)"`), PARALLEL to
    /// `items` — index `i` captions tile `i`, `""` where the server named no part. A parallel vector
    /// rather than a field on `CatalogMovie`, because a character name is a fact about a *person's
    /// credit in* an item, not about the item: the same movie on a home shelf has no role.
    pub(crate) roles: Vec<String>,
}

/// The shelves a person's page has, in flow order. `ui/person.rs` indexes everything by this.
pub(crate) const NSHELF: usize = 2;

/// One SOURCE of the person's filmography — a registered server, its own local id for this person,
/// what it answered with, and the three per-server flags [`address`] reads. Main-thread only, like
/// the `Person` that owns it.
///
/// A source is *settled* ([`Src::settled`]) once it has finished contributing, whichever way: its
/// media landed, or its resolve came back "never heard of them". Both are answers.
struct Src {
    /// the registry slot every fetch for this source is issued through, and the id stamped onto
    /// every row it parses
    sid: ServerId,
    /// **This server's own `personId`** for this person. `None` after a settled resolve means it
    /// has no record of them, which is why the media fetch is gated on `resolved` as well: `None`
    /// before the resolve answers means "not known yet", and the two must not be confused.
    local: Option<String>,
    /// the resolve has ANSWERED — found or not. Every source now starts `true`: the resolve was
    /// the retired read layer's `/hubs/search` round trip, and with that layer gone there is
    /// nothing left to ask. The origin carries its `personId` from the credit row, and every other
    /// contributes nothing and is settled at birth (see [`sources`]).
    resolved: bool,
    /// this source's own contribution, before the merge
    shelves: [Shelf; NSHELF],
    /// **The availability index** — `(guid, ratingKey)` for every row this source's `/media` answer
    /// carried, uncapped. See [`MediaLanding::matches`]; read only by [`filmography`].
    matches: Vec<(String, String)>,
    /// its `/media` fetch has landed successfully at least once
    landed: bool,
    /// its batched character-name read has ANSWERED for ITS CURRENT shelves. Cleared by every media
    /// landing (the keys it was addressed to are gone), which is what re-asks for the new list.
    roled: bool,
}

impl Src {
    /// Every key this source's shelves hold, in flow order — and its roles fetch's cache key: empty
    /// until its media lands (so nothing is asked too early) and it CHANGES with them (so a
    /// re-landing re-asks). At most `NSHELF * SHELF_MAX` keys.
    ///
    /// PER SOURCE, because the batch it addresses is a `/library/metadata/{csv}` of server-local
    /// `ratingKey`s: hand another machine this list and it answers about different items entirely.
    fn shelf_keys(&self) -> Vec<&str> {
        self.shelves
            .iter()
            .flat_map(|s| s.items.iter())
            .map(|m| m.rk.as_str())
            .filter(|rk| !rk.is_empty())
            .collect()
    }

    /// Has this source finished contributing? Either its filmography landed, or it answered that it
    /// has never heard of them. Distinct from `landed` because "nothing from this server" is an
    /// answer the page is allowed to stop waiting on — see [`resettle`].
    fn settled(&self) -> bool {
        self.landed || (self.resolved && self.local.is_none())
    }

    /// Did this source actually put a tile on the page?
    ///
    /// **Not the same question as `landed`,** and the difference is a visible flash. A media
    /// response can succeed and still yield nothing: the shelf split keeps only the rows the page
    /// has shelves for, so a person credited on this server in episodes alone lands *successfully*
    /// with two empty shelves. Reading that as "the page has something to show" draws the empty
    /// read-out over a share that is still resolving, and its films then pop in on top of the words.
    fn has_content(&self) -> bool {
        self.shelves.iter().any(|sh| !sh.items.is_empty())
    }
}

/// One person's page: the header handed in by the cast row, the biography fields, and the
/// MERGED Movies / Shows shelves every source contributed to.
pub(crate) struct Person {
    /// The ORIGIN server — the one whose credit row opened this page, and the only one
    /// [`Person::key`] means anything on. A personId is server-local exactly like a ratingKey,
    /// so `key` alone names a person on no machine in particular once
    /// a share is registered; the pair is the identity, compared through
    /// [`crate::registry::same_item`]. `guid` needs no such scoping: it is portable, and global.
    ///
    /// It is NOT "the server this page reads" any more — that is every entry of [`Person::srcs`].
    /// What it still decides is the header (`Art::Person` fetches the headshot from it) and the
    /// trail's person identity.
    pub(crate) sid: ServerId,
    /// the ORIGIN server's local `personId` (numeric tag id, or the guid when that server sent no
    /// number) — addresses `/library/people/{key}/media` on [`Person::sid`] and nowhere else
    pub(crate) key: String,
    /// the `tagKey` guid - the global person id carried over from the inherited metadata. It is
    /// the id the profile fetch is addressed with (module docs). Empty when the credit row carried
    /// none, and then there is nothing to ask with, which costs the biography.
    pub(crate) guid: String,
    pub(crate) name: String,
    pub(crate) thumb: String,
    /// Biography, from the profile fetch. Empty until that fetch lands - and STAYS empty for a
    /// person the server has no record of. Drawn only when non-empty.
    pub(crate) bio: String,
    /// The departments, already shortened + prettified for display, in flow order and capped at
    /// [`MAX_ROLES`] — `["Actor", "Producer"]`, one entry per department. See [`roles_line`]; a
    /// caller joins them with whatever separator its own line uses (the header's dotted run, the
    /// bio panel's `", "`).
    pub(crate) roles: Vec<String>,
    /// ISO `YYYY-MM-DD` birth / death dates and the birthplace, verbatim from the profile record -
    /// the SCREEN formats them (`ui::fmt::pretty_date`), because how a date reads is a display
    /// decision. `died` is empty for someone living, which is why the page tests "non-empty", never
    /// "unknown".
    pub(crate) born: String,
    pub(crate) died: String,
    pub(crate) birthplace: String,
    /// The two shelves, indexed by KIND (0 = Movies, 1 = Shows) — the same index the screen's focus
    /// model, headings and hit-test use, so "which shelf" is spelled one way everywhere.
    ///
    /// The MERGE of every source's own shelves ([`merge_shelves`]), rebuilt by [`resettle`] on each
    /// landing. Never written directly by a landing: a source owns its rows, and this is a
    /// projection of them.
    pub(crate) shelves: [Shelf; NSHELF],
    /// Every server this page asks, in [`sources`] order. Read once at [`open`].
    srcs: Vec<Src>,
    /// Something is worth showing, so the spinner can stop. The `browse.rs` `total < 0` sentinel in
    /// bool form, folded across the sources by [`resettle`], and the fold is **content, then
    /// consensus**:
    ///
    /// * ANY source having put a TILE on the page settles it — a share whose machine is asleep must
    ///   not hold shelves we already have off the screen;
    /// * with nothing anywhere, it takes EVERY source having answered.
    ///
    /// Both halves stop the same flash from a different side: a server that says "never heard of
    /// them" instantly, and one that answers with a successful but empty filmography ([`Src::
    /// has_content`] — an episodes-only credit lands exactly like that), must neither of them draw
    /// "nothing in your libraries" over a fetch still out on the server that does have them.
    pub(crate) landed: bool,
    /// the profile fetch has ANSWERED at least once - including the answer "no such
    /// person", which arrives as an all-empty profile. Its only job is to stop the retry; the page
    /// never renders a "loading" state for the header, because a header that is complete without a
    /// biography must not flicker a spinner into a space it will never fill.
    pub(crate) profiled: bool,
    /// Whether the profile fetch has COME BACK at all, success or failure - the bound on
    /// [`facts_pending`]'s wait. Distinct from `profiled`, which says the answer had content:
    /// "still waiting" and "asked, and got nothing" look identical to a placeholder and must not.
    pub(crate) profile_tried: bool,
    /// The filmography the retired global read delivered - every department, in the provider's own
    /// order, with nothing folded, sorted or joined. The DISPLAY model is [`filmography`], derived
    /// on demand, because its availability column is a function of the sources and moves under it.
    credits: Vec<types::CreditGroup>,
    /// the credits fetch has ANSWERED at least once — including the answer "no credits", which is
    /// an empty vector. [`Person::profiled`]'s twin, and the gate the Filmography entry row is
    /// drawn on: an entry naming no count is an entry that cannot promise a list.
    pub(crate) credited: bool,
    /// Exact registry identity this source vector was built from. Background roster refresh can
    /// add/remove/re-point while a page remains open, without going through `install_server::reset`.
    roster_gen: u32,
}

impl Person {
    /// The tiles of shelf `kind`.
    pub(crate) fn shelf(&self, kind: usize) -> &[CatalogMovie] {
        &self.shelves[kind].items
    }
    /// The character tile `i` of shelf `kind` credits this person as — `""` until the batched read
    /// lands, and `""` forever for a credit the server names no part for (every CREW credit, since
    /// a director appears in the item's `Director[]`, not `Role[]`). Bounds-checked rather than
    /// indexed: the roles vector is filled a frame or more after the shelf it captions.
    pub(crate) fn role(&self, kind: usize, i: usize) -> &str {
        self.shelves[kind]
            .roles
            .get(i)
            .map(String::as_str)
            .unwrap_or("")
    }
    /// How many items of kind `kind` every source really had — what a heading prints (see
    /// [`Shelf::total`]).
    pub(crate) fn total(&self, kind: usize) -> usize {
        self.shelves[kind].total
    }
}

/// Immutable publication borrowed from one concrete [`PersonState`] owner for a dispatcher
/// step/draw. It carries no selector and cannot outlive the owner borrow that produced it.
#[derive(Clone, Copy, Default)]
pub(crate) struct PersonView<'a> {
    current: Option<&'a Person>,
}

impl<'a> PersonView<'a> {
    pub(crate) fn current(self) -> Option<&'a Person> {
        self.current
    }

    pub(crate) fn loading(self) -> bool {
        // A failed fetch remains pending because the owner retries it after the backoff.
        self.current.is_some_and(|person| !person.landed)
    }
}

/// Main-thread logical state for one Person owner. Generation, retry ladders and the dev-seed
/// latch move with the model instead of selecting process state.
pub(crate) struct PersonState {
    current: Option<Person>,
    generation: u32,
    retry_cd: [u32; NFETCH],
    dev_held: usize,
    session_watch: crate::server::session::VisibleSessionWatch,
    session_identity: (String, String),
}

impl Default for PersonState {
    fn default() -> Self {
        Self {
            current: None,
            generation: 0,
            retry_cd: [0; NFETCH],
            dev_held: usize::MAX,
            session_watch: Default::default(),
            session_identity: Default::default(),
        }
    }
}

impl PersonState {
    pub(crate) fn view(&self) -> PersonView<'_> {
        PersonView { current: self.current.as_ref() }
    }

    fn current(&self) -> Option<&Person> {
        self.current.as_ref()
    }
}

/// The open person publication is borrowed from its owner. Main-thread only; the reference is
/// valid for the frame context that lent it.
/// **The profile has not answered yet** - the canvas's phase 0, and what the portrait, the
/// identity pair and the bio draw a placeholder for.
///
/// It is PENDING, not ABSENT: a profile that answered with no biography is `profiled` with an empty
/// `bio`, and that page must read as finished rather than sprouting three grey bars forever. The
/// two states were one before the placeholders existed, which is why `person.rs`'s guide says a
/// header line is drawn only when it has content — that rule is about the ANSWER, and this is about
/// the wait.
pub(crate) fn facts_pending(p: &Person) -> bool {
    // **Bounded by a real ATTEMPT, not only by success.** `!p.profiled` alone is unbounded whenever
    // the profile cannot be fetched - an offline or LAN-only television, a provider outage, or any
    // headless boot, where the injected `/tmp/butaca-token` was a SERVER token the metadata
    // provider answered 401 to. The profile mailbox then re-arms its back-off
    // forever, `profiled` never turns true, and `draw_header` paints five `skeleton_bar`s a frame,
    // each of which calls `idle::invalidate()` — so the whole-frame present gate never closes and
    // the page repaints at 60fps for as long as it is open, for a sweep over content that is never
    // arriving. One failed attempt is enough to say "this is what the page is": the band degrades
    // to the name it already has, which is exactly the finished-not-broken read the doc above asks
    // for. A later success still fills it in — `profiled` is what draws the content.
    !p.profiled && !p.profile_tried
}

// Shelves have their OWN pending question already — [`loading`], just below — which is `!landed`
// rather than `!all(Src::settled)` and for a reason worth keeping in mind here: `landed` also goes
// true the moment ANY source has actually put a tile on the page, so a shelf shows real content the
// instant it is real rather than staying skeletonized while a second, slower source is still out.

// ---- fetch plumbing (generation + single-flight + mailbox + retry backoff) -------------------

/// Bumped by every [`open`]/[`close`]/[`reset`]: a landing whose generation no longer matches is
/// discarded by [`PersonState::pump`], so a slow fetch for the actor you just left can never repopulate the
/// one you are looking at now.
/// The three requests this page makes OF ONE SERVER. Adding one is a constant, one arm in
/// [`address`] and one in [`maybe_spawn`]; [`supersede`] picks it up with no edit at all.
const K_RESOLVE: usize = 0;
const K_MEDIA: usize = 1;
const K_ROLES: usize = 2;
const NKIND: usize = 3;

/// The two fetches that are not per-server: the biography and the filmography, both of which were
/// which are already global (module doc). They sit past every per-source mailbox, which is what
/// keeps [`un_fx`] total.
const F_PROFILE: usize = crate::registry::MAX_SERVERS * NKIND;
/// The FILMOGRAPHY - the retired global credits read, `{DISCOVER}/library/people/{tagKey}/credits`.
/// Global for [`F_PROFILE`]'s reason and then some: it is a fact about the person's CAREER, which
/// no server has an opinion about at all. What the servers contribute is the AVAILABILITY join,
/// and that rides on the `/media` answers ([`K_MEDIA`]) the page already makes. On the Jellyfin
/// backend the filmography IS the per-source shelves, so this mailbox asks for nothing there.
const F_CREDITS: usize = F_PROFILE + 1;
/// Mailboxes, single-flight claims and retry countdowns, all over this one index space.
const NFETCH: usize = F_CREDITS + 1;

/// Mailbox index of source `sid`'s fetch of kind `k`, `None` for a `ServerId` that names no slot.
///
/// **Keyed on the registry SLOT, never on a position in [`Person::srcs`].** A slot is stable for
/// the life of the process, so a worker's landing is applied to the server it asked even if the
/// source list is rebuilt under it; an index into a `Vec` would silently start meaning a different
/// machine. [`crate::registry::MAX_SERVERS`] is the registry's own ceiling, imported rather than
/// restated — a second 16 here would go out of bounds the day that one moves.
fn fx(sid: ServerId, k: usize) -> Option<usize> {
    let slot = sid
        .is_set()
        .then(|| sid.raw() as usize)
        .filter(|&s| s < crate::registry::MAX_SERVERS)?;
    Some(slot * NKIND + k)
}

/// …and back: which source and which kind mailbox `i` belongs to. `None` for [`F_PROFILE`], which
/// belongs to no server.
fn un_fx(i: usize) -> Option<(ServerId, usize)> {
    (i < F_PROFILE).then(|| (ServerId::from_raw((i / NKIND) as u16), i % NKIND))
}

/// Frames left before fetch `i` may spawn again after a FAILED attempt (main-thread; [`PersonState::pump`]
/// decrements). Stops a fast-failing network from spawning a worker every frame. PER FETCH AND PER
/// SOURCE on purpose, which is the third of `catalog.rs`'s three multi-source lessons: a biography
/// biography that cannot be reached (the TV is on a LAN with no internet — the case this app is
/// built to keep working) must not hold the shelves off for two seconds a go, and a share that
/// takes eight seconds to time out must not put the server that IS answering on its ladder.
///
/// **The one piece of a fetch that stays its own array, and both halves of that are deliberate.**
/// It is written by the main thread alone, which a static the workers share cannot express — a
/// `Cell` is not `Sync`, and a `static mut` [`FETCH`] would put these writes inside an object
/// worker threads hold references into. And unlike `search.rs`'s `Source::retry_cd`, which its doc
/// records as never moving independently of the `status` beside it, this countdown genuinely moves
/// on its own: [`PersonState::pump`] ticks it every frame whether or not the fetch is claimed,
/// [`apply_landing`] arms it
/// for a landing whose claim the take has already released, and [`maybe_spawn`] arms it for a
/// source with no client — a fetch it never claimed at all.
/// ~2s at 60fps — the same backoff `browse.rs` uses for a failed page.
const RETRY_FRAMES: u32 = 120;

/// What a finished fetch delivers. Every arm carries an `Option`, and in ALL of them the `None`
/// means the same thing: the fetch FAILED (transport, parse, or a panicking worker) and must be
/// retried. It has to stay distinguishable from a successful answer that happens to be empty —
/// installing empty shelves on a failure is the "one wifi hiccup blanked a populated grid" bug
/// `browse.rs` carries its `total < 0` sentinel for; the profile has the same trap in a subtler
/// form (a backend answering 200 with an empty record for a person it has never heard of is an
/// ANSWER).
#[derive(serde::Serialize, serde::Deserialize)]
enum Landing {
    Media(Option<MediaLanding>),
    Profile(Option<types::PersonProfile>),
    /// The whole filmography, ungrouped and unsorted — the display model is derived on the main
    /// thread by [`filmography`], because the AVAILABILITY half of it changes every time a source
    /// lands and cannot be baked into a worker's answer.
    Credits(Option<Vec<types::CreditGroup>>),
    Roles(Option<RolesLanding>),
}

/// One source's finished `/media` read: the two shelves it drew, and the **uncapped** guid index
/// the Filmography route joins against.
///
/// The two travel together for [`RolesLanding`]'s reason — they describe the same response, and a
/// landing that installed one without the other would leave the route claiming a server holds
/// items it has since replaced. The index is separate from the shelves rather than derived from
/// them because [`SHELF_MAX`] caps what a `CardRow` can spring: a prolific actor's 60 movies draw
/// as 24 tiles, and joining a filmography against the 24 would silently un-own 36 films the user
/// really has.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct MediaLanding {
    shelves: [Shelf; NSHELF],
    /// `(guid, ratingKey)` for EVERY `movie`/`show` row the response carried. Short strings, a few
    /// hundred at the very worst.
    matches: Vec<(String, String)>,
}

/// A finished character-name batch. `keys` is the shelf-key list it was ADDRESSED to, which rides
/// along because [`apply_landing`] must refuse a landing for a shelf list that has since been replaced
/// (see there). `pairs` is already filtered to THIS person's credit per item — the worker walks the
/// batched response so ~30 KB of tag arrays never crosses the mailbox.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct RolesLanding {
    keys: Vec<String>,
    pairs: Vec<(String, String)>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Mail {
    gen: u32,
    what: Landing,
}
pub(crate) fn validate_record(slot: u32, value: &serde_json::Value) -> Result<(), &'static str> {
    if slot as usize >= NFETCH { return Err("invalid person slot"); }
    let mail: Mail = serde_json::from_value(value.clone()).map_err(|_| "invalid person reply")?;
    if serde_json::to_value(&mail).ok().as_ref() != Some(value) {
        return Err("noncanonical person reply");
    }
    let legal = match slot as usize {
        F_PROFILE => matches!(mail.what, Landing::Profile(_)),
        F_CREDITS => matches!(mail.what, Landing::Credits(_)),
        i => match un_fx(i).map(|(_, kind)| kind) {
            Some(K_MEDIA) => matches!(mail.what, Landing::Media(_)),
            Some(K_ROLES) => matches!(mail.what, Landing::Roles(_)),
            _ => false,
        },
    };
    if !legal { return Err("mismatched person terminal kind"); }
    if mail.gen == 0 { return Err("invalid person reply generation"); }
    Ok(())
}

/// One fetch's two WORKER-VISIBLE halves: the claim that it is out, and the mailbox its answer
/// lands in. Bundled because they move together — the claim of a worker that is out is cleared by
/// the take of the mail answering it ([`Fetch::take`]), so emptying that mailbox any other way
/// owes the release ([`Fetch::clear`]), and a spawn that never happened owes it too
/// ([`Fetch::release`]). Three methods, each spelling one of those, rather than two arrays and the
/// same rule restated at every site that touches both.
///
/// The retry countdown is NOT a third field here; [`RETRY_CD`] documents why it cannot be.
struct Fetch {
    /// The claim that this fetch is out. Once a worker holds it, it is cleared ONLY by a mailbox
    /// take, so anything that drops the mailbox ([`supersede`]) must clear it too — otherwise the
    /// fetch stays latched and the page spins forever. Same latch `browse.rs` documents on its
    /// `IN_FLIGHT` array.
    ///
    /// It bounds spawns per *pump*, which is what matters; it is NOT a hard one-worker-at-a-time
    /// interlock, and claiming otherwise would be wrong. Two ways a second worker can briefly
    /// exist: [`supersede`] releases the claim while the old worker is still running, and a take
    /// releases it before the generation check (so a stale landing can free a NEWER fetch's claim,
    /// costing one duplicate request). Neither can wedge or corrupt — [`land`] is monotone on the
    /// generation and [`PersonState::pump`] discards anything stale — and `browse.rs` has the identical shape.
    in_flight: AtomicBool,
    /// Where the worker posts what it came back with — the one half of a [`Fetch`] a worker
    /// touches, the claim beside it being moved by the main thread alone. `None` means nothing has
    /// landed since the last take.
    slot: Mutex<Option<Mail>>,
}

impl Fetch {
    const IDLE: Fetch = Fetch {
        in_flight: AtomicBool::new(false),
        slot: Mutex::new(None),
    };

    /// Is the claim held? [`maybe_spawn`]'s first gate.
    fn busy(&self) -> bool {
        self.in_flight.load(Ordering::SeqCst)
    }

    /// Claim the fetch, on the way into a spawn.
    fn claim(&self) {
        self.in_flight.store(true, Ordering::SeqCst);
    }

    /// Release the claim without touching the mailbox — what a REFUSED spawn (`task.rs`'s thread
    /// ceiling) owes, since nothing will ever land to release it, and the second half of
    /// [`Fetch::clear`].
    fn release(&self) {
        self.in_flight.store(false, Ordering::SeqCst);
    }

    /// Take whatever landed, RELEASING the claim with it. The release does not depend on what the
    /// mail turns out to be — an answer, a failure, or a generation the pump is about to discard —
    /// because once a worker is out this take and [`supersede`] are the two things that can clear
    /// its claim: hold it while dropping a landing and this fetch never spawns again.
    ///
    /// An EMPTY mailbox releases nothing, which is the other half of the rule: the claim it would
    /// clear belongs to a worker still running, and the next frame would spawn a duplicate.
    fn take(&self) -> Option<Mail> {
        let mail = self.slot.lock().unwrap_or_else(|e| e.into_inner()).take()?;
        self.in_flight.store(false, Ordering::SeqCst);
        Some(mail)
    }

    /// Drop the mailbox and release the claim with it — [`supersede`]'s per-fetch half. Releases
    /// unconditionally, unlike [`Fetch::take`], and that difference is the point: the worker
    /// holding this claim is still out, and what it will answer about is a person no longer open.
    fn clear(&self) {
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.release();
    }
}

/// Cross-thread transport for one physical owner. Every worker captures this exact `Arc`; reset
/// rotates the store to a fresh adapter, so a detached old worker can only fill retired slots.
pub(crate) struct PersonAdapter {
    fetch: [Fetch; NFETCH],
}

impl Default for PersonAdapter {
    fn default() -> Self {
        Self {
            fetch: [const { Fetch::IDLE }; NFETCH],
        }
    }
}

/// Post a finished fetch to its mailbox. MONOTONE: an older fetch landing late must never clobber
/// a newer result the pump has not consumed yet. Named (not inlined in the worker closure) for the
/// same reason as `metadata::land_detail` — the guard is the one piece of this machinery a test
/// cannot reach through [`open`], because reaching it needs two overlapping real fetches.
impl PersonAdapter {
    fn land(&self, i: usize, generation: u32, what: Landing) {
        let mut slot = self.fetch[i].slot.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().map(|r| r.gen < generation).unwrap_or(true) {
            *slot = Some(Mail { gen: generation, what });
        }
    }
}

/// Invalidate everything in flight: bump the generation (a late landing is discarded), drop every
/// mailbox and the claim held with it ([`Fetch::clear`]), and clear the retry backoffs. The ONE
/// place those three move together — and it walks the WHOLE index space, not just the sources
/// currently held, so a slot that has left the roster cannot leave a latched claim behind it.
impl PersonState {
    fn supersede(&mut self, adapter: &PersonAdapter) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("person generation exhausted");
        for fetch in &adapter.fetch {
            fetch.clear();
        }
        self.retry_cd.fill(0);
    }
}

// ---- the sources, and the merge ---------------------------------------------------------------

/// The servers this page asks: **every registered one**, in registry order.
///
/// Registry order, not "the one you arrived through first", is the point of the whole unit — the
/// page has to be the same page whichever credit row opened it. The ORIGIN is force-included when
/// the registry has never heard of it, which covers a host test and a boot that never signed in.
/// A `ServerId` that names no slot ([`ServerId::UNSET`]) is not a source at all, since
/// there is nothing to dial and no mailbox to land in.
///
/// Read on open and whenever the registry generation moves. The roster is tiny; rebuilding only at
/// that explicit boundary prevents an open page retaining a revoked share or missing a new one.
fn sources(origin: ServerId, key: &str) -> Vec<Src> {
    let mut out: Vec<Src> = Vec::new();
    for sid in crate::registry::ids().chain(std::iter::once(origin)) {
        if fx(sid, K_RESOLVE).is_none() || out.iter().any(|s| s.sid == sid) {
            continue;
        }
        let is_origin = sid == origin;
        out.push(Src {
            sid,
            // The origin needs no resolve: the credit row handed us its own `personId`, which is
            // both the round trip saved and the only id we could ever be sure of.
            local: is_origin.then(|| key.to_string()).filter(|k| !k.is_empty()),
            // Every source starts RESOLVED: the cross-server resolve was a `/hubs/search` query
            // through the retired read layer, which no longer exists. A slot other than the origin
            // cannot be asked for anything, so contributing nothing is its answer, not a fetch to
            // retry forever (`settled` reads `resolved && local.is_none()`).
            resolved: true,
            shelves: Default::default(),
            matches: Vec::new(),
            landed: false,
            roled: false,
        });
    }
    out
}

/// Merge every source's shelves into the pair the screen draws. PURE — no statics, no I/O — which
/// is what makes the order, the budget and the parallel captions gradeable on the host.
///
/// Sources are taken in [`sources`] order and each keeps its own rows: every row a source landed
/// was stamped with that source's `ServerId`, so a card opens on the right machine however far
/// down the merged shelf it lands (`ui::person`'s activation reads `mm.sid`, and `Art::Poster`
/// resolves the artwork on that server too).
///
/// The per-shelf budget is divided by [`crate::catalog::allot`] rather than spent first-come, for the
/// reason that function documents: a prolific actor's 24 films on the first source would otherwise
/// fill the row and leave every share behind it nothing — which is this unit's own bug, re-created
/// one level down. The heading's count is the sum of the REAL totals, not of what fitted.
///
/// **Nothing is deduplicated, deliberately.** Two servers that both hold a film contribute two
/// cards. The only identity that could join them is the portable `Metadata::guid`, which a
/// `CatalogMovie` does not carry — and a title+year join would merge two different films *and* miss the
/// case measured on this very pair of servers, where the share's copy is
/// titled in another language. A visible duplicate is honest; a wrong merge hides a film.
/// (With that read layer gone only the origin lands rows today, but the merge stays the one
/// place that shapes what the screen draws.)
fn merge_shelves(srcs: &[Src]) -> [Shelf; NSHELF] {
    let mut out: [Shelf; NSHELF] = Default::default();
    for (kind, sh_out) in out.iter_mut().enumerate() {
        let want: Vec<usize> = srcs.iter().map(|s| s.shelves[kind].items.len()).collect();
        let take = crate::catalog::allot(SHELF_MAX, &want);
        for (i, s) in srcs.iter().enumerate() {
            let sh = &s.shelves[kind];
            sh_out.total += sh.total;
            for (j, m) in sh.items.iter().take(take[i]).enumerate() {
                sh_out.items.push(m.clone());
                // captions ride WITH their items, so the merged pair stays parallel even while one
                // source's roles batch is still out — `""` until it lands, exactly as `Person::role`
                // already reports for a credit with no named part
                sh_out
                    .roles
                    .push(sh.roles.get(j).cloned().unwrap_or_default());
            }
        }
    }
    out
}

/// Re-derive everything that is a function of the SOURCES: the merged shelves the screen draws, and
/// whether the page has an answer to show. The ONE place those two move, so no landing can update
/// one without the other.
fn resettle(p: &mut Person) {
    p.shelves = merge_shelves(&p.srcs);
    p.landed = p.srcs.iter().any(Src::has_content) || p.srcs.iter().all(Src::settled);
}

/// Flip `(sid, rk)`'s watched state on this person's shelves — the optimistic half of a view-state
/// write. `catalog::edit_item`'s twin, for `browse::set_watched_local`'s reason: that one reaches the
/// HOME hubs alone, so a film marked watched from a filmography tile's context menu kept its old
/// mark until a refetch.
///
/// **Through the SOURCES, then [`resettle`]** — the module's own rule that a source owns its rows
/// and [`Person::shelves`] is a projection of them. Editing the merged shelves directly would be
/// undone by the next landing.
///
/// Returns whether anything matched. **MAIN THREAD.**
fn set_watched_local(state: &mut PersonState, sid: ServerId, rk: &str, on: bool) -> bool {
    let Some(p) = state.current.as_mut() else {
        return false;
    };
    let mut hit = false;
    for m in p
        .srcs
        .iter_mut()
        .flat_map(|s| s.shelves.iter_mut())
        .flat_map(|sh| sh.items.iter_mut())
    {
        if crate::registry::same_item((m.sid, &m.rk), (sid, rk)) {
            crate::catalog::set_watched(m, on);
            hit = true;
        }
    }
    if hit {
        resettle(p);
    }
    hit
}

// ---- public surface --------------------------------------------------------------------------

/// Open the page for the person the ORIGIN server `sid` knows as `key`, with the header the caller
/// already has — the cast row's name + headshot. MAIN THREAD, NON-BLOCKING: nothing is fetched
/// here, [`PersonState::pump`] spawns every request on the next frame, so the page mounts on its header
/// immediately and fills in around it.
///
/// `sid` is the server whose credit row this is, captured by the caller. It is where `key` means
/// something and where the headshot comes from — but it is NOT the only server read: [`sources`]
/// takes the whole roster, and every other entry resolves its own id from `guid` first.
///
/// **Idempotent on the identity already held** (review round 1, P1): a redundant `Open` for the
/// `(sid, key)` the store already holds is a no-op — no generation bump, no fetch-claim clear, no
/// rebuilt `Person`. This is what lets `screens::person`'s `Enter(_) | Uncover` arm call
/// `request_store` unconditionally rather than only when its own `person(cx)` read is `None`: the
/// two-phase `AppFx::Store` queue can deliver a same-identity `Close` (an evicted covered/stacked
/// body's `Unmount`) ahead of a fresh page's `Open` in the same drain, and the guard below is what
/// keeps a *genuinely* redundant Open — the store already correctly settled on this identity —
/// from refetching or dropping in-flight work for a page that never actually lost it.
fn open(
    state: &mut PersonState,
    adapter: &PersonAdapter,
    sid: ServerId,
    key: &str,
    guid: &str,
    name: &str,
    thumb: &str,
) {
    if state
        .current
        .as_ref()
        .is_some_and(|p| crate::registry::same_item((p.sid, p.key.as_str()), (sid, key)))
    {
        return;
    }
    // Controlled content uses captured authority and supplied provider replies. The ambient
    // session cache recovers asynchronously and is not an input in that transcript.
    if !crate::app::bootstrap::stores::active() {
        let _ = state.session_watch.changed();
        if let Some(session) = crate::server::session::peek_settled() {
            state.session_identity = (session.client_id.clone(), session.account_token.clone());
        }
    }
    state.supersede(adapter);
    let srcs = sources(sid, key);
    state.current = Some(Person {
        sid,
        key: key.to_string(),
        guid: guid.to_string(),
        name: name.to_string(),
        thumb: thumb.to_string(),
        bio: String::new(),
        roles: Vec::new(),
        born: String::new(),
        died: String::new(),
        birthplace: String::new(),
        shelves: Default::default(),
        srcs,
        landed: false,
        profiled: false,
        profile_tried: false,
        credits: Vec::new(),
        credited: false,
        roster_gen: crate::registry::roster_gen(),
    });
    // A page with NO source at all has already answered — see `resettle`'s fold. Deriving it
    // here rather than hardcoding `landed: false` is what keeps that rule in one place.
    if let Some(p) = state.current.as_mut() {
        resettle(p);
        if !crate::app::bootstrap::stores::active() {
            seed_dev_profile(p);
        }
    }
}

/// `/tmp/butaca-personbio[=<text>]` - **stand in for the biography record**, so the
/// person page's header bio and the bio alert panel behind it can be reached headlessly.
///
/// **Why this trigger has to exist.** Everything below the name on this page - roles, dates,
/// birthplace and the biography itself - used to come from the metadata provider, which was a
/// different identity from the server's own. An automated boot signed in with `/tmp/butaca-token`,
/// a *server* token, and the provider answered that `401`, while a real interactive session got a
/// biography and an automated one never did. That path is retired, and no capture can depend on
/// what a server happens to hold: the panel this seeds is by construction only reachable when
/// there is MORE prose than the header shows. That is the same argument
/// `/tmp/butaca-search`'s query seed makes (no harness can type) and `/tmp/butaca-failtest`'s
/// (no server will refuse on cue) - the screen is real, the route to it is not automatable.
///
/// Empty file = a long built-in sample with paragraph breaks in it, which is the shape that
/// exercises the panel (a one-paragraph blob would never show [`PARA_GAP`], and a short one would
/// leave the rail and both feather edges undrawn). A value = that text, so a specific length or a
/// specific wrap can be reproduced.
///
/// It sets `profiled`, which is the flag [`address`] gates the profile fetch on — so a seeded page makes no
/// provider request at all, rather than racing one that would overwrite the seed a second later.
/// The invented roles/dates/birthplace are what make the identity line's separator logic visible;
/// they are as fictional as the rest of the trigger and never reach a release build (`dev::read` is
/// `None` at compile time without `devtriggers`).
#[allow(unused_variables)]
fn seed_dev_profile(p: &mut Person) {
    let Some(text) = crate::dev::read("personbio") else {
        return;
    };
    p.bio = if text.is_empty() {
        DEV_BIO.trim().replace("\\n", "\n").to_string()
    } else {
        text
    };
    if p.roles.is_empty() {
        p.roles = vec![
            "Actress".to_string(),
            "Singer".to_string(),
            "Songwriter".to_string(),
        ];
    }
    if p.born.is_empty() {
        p.born = "1987-01-08".to_string();
    }
    if p.birthplace.is_empty() {
        p.birthplace = "Stockwell, London".to_string();
    }
    p.profiled = true;
    #[cfg(feature = "devtriggers")]
    crate::log(&format!(
        "person: DEV bio seeded ({}B) — /tmp/butaca-personbio",
        p.bio.len()
    ));
}

/// The built-in sample for an empty [`seed_dev_profile`] trigger: several paragraphs of plausible
/// biography prose, long enough to scroll for a handful of pages at `size::BODY` in a 1120px sheet.
/// Written out rather than generated so a shot taken today and one taken next month are comparable.
#[cfg(feature = "devtriggers")]
const DEV_BIO: &str = "\
Cynthia Erivo is a British actress, singer and songwriter whose work spans the stage, the concert \
hall and the screen. She trained at the Royal Academy of Dramatic Art, graduating in 2010 after an \
earlier spell reading music psychology, and spent her first professional years in ensemble and \
understudy work in London before the role that would define the next decade found her.\n\n\
That role was Celie in the Menier Chocolate Factory's 2013 revival of The Color Purple, a stripped \
back staging that traded spectacle for the voice at its centre. When the production transferred to \
Broadway in 2015 it won two Tony Awards, one of them hers; the cast recording took a Grammy, and a \
Daytime Emmy followed for a televised performance. Three of the four American entertainment awards \
in under three years left her one short of a set that fewer than twenty people have completed.\n\n\
Her screen career began in earnest with a run of ensemble thrillers before Harriet in 2019, in \
which she played Harriet Tubman and earned nominations for both Best Actress and Best Original \
Song in the same year. She has since moved between prestige television, animation and the kind of \
large studio musical that asks a performer to sing live on camera, a discipline she has spoken \
about as closer to theatre than to film.\n\n\
Alongside acting she writes and records her own music, and has been open about the relationship \
between the two: songs, she has said, are where the parts she plays go when the run ends. She \
continues to divide her time between London and New York.";
/// The release build has no sample — `seed_dev_profile` cannot be reached (`dev::read` is `None` at
/// compile time), and a few hundred bytes of fiction has no business in a shipped binary.
#[cfg(not(feature = "devtriggers"))]
const DEV_BIO: &str = "";

/// Drop the open person and supersede any fetch for it — on leaving the page. Without the
/// supersede, a landing arriving after the page closed would repopulate the owner's model behind whatever
/// screen is now mounted (the bug `metadata::clear` carries the same guard for).
fn close(state: &mut PersonState, adapter: &PersonAdapter) {
    state.supersede(adapter);
    state.current = None;
}

/// Wipe the store on a profile/account switch — the browse/catalog twin of `install_server`'s reset. A
/// new user must never inherit the previous one's page, and the flags must move with the mailbox.
/// It is also what makes [`sources`]' read-once safe: the roster only changes on the paths that
/// call this.
fn reset(state: &mut PersonState, adapter: &PersonAdapter) {
    close(state, adapter);
    state.dev_held = usize::MAX;
}

/// `stores::person`'s one door onto every [`PersonCmd`](crate::stores::person::PersonCmd) (D3):
/// the match used to live in `stores/person.rs::run`, calling `open`/`close`/`reset` across the
/// module boundary. Relocating it here is what lets those three go private — `set_watched_local`
/// was already `pub(crate)` for the same reason and joins them.
impl PersonState {
    pub(crate) fn run(
        &mut self,
        adapter: &Arc<PersonAdapter>,
        cmd: crate::stores::person::PersonCmd,
    ) -> bool {
        use crate::stores::person::PersonCmd;
        match cmd {
            PersonCmd::Open {
                sid,
                key,
                guid,
                name,
                thumb,
            } => {
                open(self, adapter, sid, &key, &guid, &name, &thumb);
                true
            }
            PersonCmd::Close => {
                close(self, adapter);
                true
            }
            PersonCmd::Reset => {
                reset(self, adapter);
                true
            }
            PersonCmd::SetWatchedLocal { sid, rk, on } => {
                set_watched_local(self, sid, &rk, on)
            }
        }
    }
}

/// Whether one server's contribution to this person's shelves is still unresolved.
///
/// This is deliberately narrower than [`PersonView::loading`]: the page may stop its global spinner as soon
/// as any source contributes content, while a saved card from another source still needs its own
/// `/media` answer before the screen may decide that identity is gone. A failed resolve or media
/// request remains pending here because [`maybe_spawn`] retries it after backoff. An absent source
/// is not pending (the roster no longer offers a way for that card to return), and a successful
/// empty media answer or resolved "no record" is settled through [`Src::settled`].
pub(crate) fn media_resolving(p: &Person, sid: ServerId) -> bool {
    p.srcs
        .iter()
        .find(|source| source.sid == sid)
        .is_some_and(|source| !source.settled())
}

/// MAIN THREAD, once a frame while the page is up: apply every landed fetch and schedule the next.
/// Returns true when the store just changed — the screen re-clamps its focus and rebuilds its
/// cached header strings on it.
impl PersonState {
    pub(crate) fn pump_with_gate(&mut self, adapter: &Arc<PersonAdapter>, gate: &crate::ui::landgate::Gate) -> bool {
        let mut session_changed = false;
        if !crate::app::bootstrap::stores::active() && self.session_watch.changed() {
            if let Some(session) = crate::server::session::peek_settled() {
                let identity = (session.client_id.clone(), session.account_token.clone());
                if self.session_identity != identity {
                    self.session_identity = identity;
                    self.supersede(adapter);
                    if let Some(person) = &mut self.current {
                        person.profiled = false;
                        person.credited = false;
                        session_changed = true;
                    }
                }
            }
        }
        let mut changed = sync_roster(self, adapter) || session_changed;
        for i in 0..NFETCH {
            if self.retry_cd[i] > 0 {
                self.retry_cd[i] -= 1;
            }
            // The take releases the single-flight claim with the mail, whatever the landing turns
            // out to be. Under replay it happens on the recorded frame; spawning remains outside
            // that gate so the request still leaves on time.
            let reply = if crate::app::bootstrap::stores::active() {
                let reply = crate::app::bootstrap::stores::poll("person", i as u32, || {
                    adapter.fetch[i].take()
                });
                if reply.is_some() {
                    gate.landed(crate::stores::StoreId::Person.ord());
                }
                reply
            } else {
                crate::stores::take_landing(gate, crate::stores::StoreId::Person, || {
                    adapter.fetch[i].take()
                })
            };
            if let Some(reply) = reply {
                adapter.fetch[i].release();
                // Every landing repaints, failures included: a shelf or stopped spinner must not
                // wait for the next keypress to become visible.
                crate::ui::idle::invalidate();
                if reply.gen == self.generation {
                    changed |= apply_landing(self, i, reply.what);
                }
                // A superseded landing is news about neither person. In particular, its failure
                // must not arm a retry delay for the replacement identity.
            }
            maybe_spawn(self, adapter, i);
        }
        // The credits seed runs HERE rather than at `open`, unlike the biography's, and the difference
        // is what it needs: it joins itself against the shelves so the availability column and the
        // trailing chevron are actually visible, and the shelves do not exist until a landing.
        if self.current.is_some() {
            changed |= seed_dev_credits(self);
        }
        changed
    }

    #[cfg(test)]
    pub(crate) fn pump(&mut self, adapter: &Arc<PersonAdapter>) -> bool {
        self.pump_with_gate(adapter, crate::ui::landgate::fixture_gate())
    }
}

/// `/tmp/butaca-personcredits[=<rows>]` - **stand in for the FILMOGRAPHY record**, so
/// [`crate::ui::filmography`] can be reached headlessly.
///
/// It exists for exactly [`seed_dev_profile`]'s reason and the argument is not repeated here: the
/// credits came from the metadata provider, an automated boot signed in with a *server* token, and
/// the provider answered that `401`. So no harness and no `make sim-shot` could reach this screen
/// with a single row on it.
///
/// **It seeds the JOIN as well as the rows**, which the biography's twin has no equivalent of and
/// which is the whole reason this runs from [`PersonState::pump`] instead of from [`open`]: every row the page's
/// own shelves can cover gets that item's `(guid, ratingKey)` written into the origin source's
/// index, so those rows draw the source annotation and the chevron and their OK really opens a
/// detail page. The rest are the majority the screen exists to show — a career you do not hold.
///
/// The value is a row count per department (default 9); the departments themselves are fixed, and
/// deliberately include one under the [`FOLD`] so the folding into `Other` is visible in a capture —
/// and enough ABOVE it that the department strip overflows its column, which is the only way a
/// headless capture reaches the strip's scroll and the left edge fade over its cut.
#[allow(unused_variables)]
fn seed_dev_credits(state: &mut PersonState) -> bool {
    let arg = if crate::app::bootstrap::stores::active() {
        crate::app::bootstrap::stores::credits().map(|v| v.to_string())
    } else { crate::dev::read("personcredits") };
    let Some(arg) = arg else {
        return false;
    };
    let Some(p) = state.current.as_mut() else { return false };
    // **The rows the page really holds, as `(title, guid)`** — taken from the source's OWN index
    // (`Src::matches`, filled by a real `/media` landing) joined back to the shelf item by
    // ratingKey. Nothing here fabricates a match: a seeded row that names one of these carries that
    // server's real guid, so the availability column and the chevron come out of the same
    // `match_local` a live filmography would go through.
    let held: Vec<(String, String, String)> = (0..NSHELF)
        .flat_map(|k| p.shelf(k).iter())
        .filter_map(|m| {
            let guid = p
                .srcs
                .iter()
                .flat_map(|s| s.matches.iter())
                .find(|(_, rk)| *rk == m.rk)?;
            // the shelf item's OWN art path travels with it, so a seeded run exercises the real
            // `/photo/:/transcode` fetch rather than only the empty plate — the half of the poster
            // path that shipped broken and that a placeholder cannot tell you is fixed
            Some((m.title.clone(), guid.0.clone(), m.thumb.clone()))
        })
        .collect();
    // Seeded once, and re-seeded when the page's holdings change — the join is the second half and
    // it has nothing to work with until a source lands. A COUNT rather than a "have I seeded" flag,
    // because the real `/media` landing fills `Src::matches` itself and a flag keyed on that cannot
    // tell the source's own index apart from a seeded one.
    if p.credited && state.dev_held == held.len() {
        return false;
    }
    state.dev_held = held.len();
    let n: usize = arg.trim().parse().unwrap_or(9);
    let row = |i: usize, dept: &str| {
        let (title, id, thumb) = match held.get(i) {
            Some((t, g, th)) => (t.clone(), guid_tail(g).to_string(), th.clone()),
            // …and everything past them is the majority this screen exists to show: a career you
            // do not hold. The ID is deliberately one no server can answer for.
            None => (
                format!("{dept} credit {i}"),
                format!("dev-{dept}-{i}"),
                String::new(),
            ),
        };
        types::Credit {
            order: i as i64,
            role: format!("Character {i}"),
            item: Some(types::CreditItem {
                kind: "movie".into(),
                title,
                // one row per department is deliberately UNDATED, which is the sort's own edge
                year: if i == 2 { 0 } else { 2024 - i as i64 },
                rating_key: id,
                thumb,
                ..Default::default()
            }),
        }
    };
    // **Seven groups, not three, and the extra four are all ABOVE the [`FOLD`]** — so the strip
    // really overflows the content column and its horizontal scroll, its reveal rule and the left
    // EDGE FADE `filmography::draw` lays over the cut are all reachable from a boot. Three pills fit
    // in 1064px with room to spare, which meant the one state a capture could never show was the
    // one an owner reports: a capsule sliced mid-glyph at the column edge. `Writer` stays under the
    // fold, because the OTHER thing only a seed can show is the folding itself.
    p.credits = [
        ("Actor", n),
        ("Appeared", n / 2 + 1),
        ("Director", FOLD + 2),
        ("Producer", FOLD + 1),
        ("Composer", FOLD + 3),
        ("Cinematographer", FOLD + 1),
        ("Writer", 2),
    ]
        .iter()
        .map(|(title, rows)| types::CreditGroup {
            kind: title.to_lowercase(),
            title: title.to_string(),
            size: *rows as i64,
            credits: (0..*rows).map(|i| row(i, title)).collect(),
        })
        .collect();
    p.credited = true;
    // Gated: `personcredits` is a `dev::CONTROLLED` name (a controlled/recorded boot may carry
    // it), and `ci/check-package.py`'s dev-trigger-catalog check greps a release binary for that
    // exact vocabulary. This function is already unreachable without `devtriggers` (`arg` above
    // is always `None`), but the log line's literal `/tmp/butaca-personcredits` would still
    // have shipped in the bytes regardless of whether the branch ever ran.
    #[cfg(feature = "devtriggers")]
    crate::log(&format!(
        "person: DEV credits seeded ({} groups, {} held) — /tmp/butaca-personcredits",
        p.credits.len(),
        held.len()
    ));
    true
}

/// Rebuild an open page's per-source projection at an exact registry identity boundary. Header
/// profile facts survive; server-derived shelves and every old worker/mailbox do not.
fn sync_roster(state: &mut PersonState, adapter: &PersonAdapter) -> bool {
    let gen = crate::registry::roster_gen();
    let Some((old, sid, key)) =
        state.current().map(|p| (p.roster_gen, p.sid, p.key.clone()))
    else {
        return false;
    };
    if old == gen {
        return false;
    }
    state.supersede(adapter);
    let srcs = sources(sid, &key);
    let Some(p) = state.current.as_mut() else {
        return false;
    };
    p.srcs = srcs;
    p.shelves = Default::default();
    p.landed = false;
    p.roster_gen = gen;
    resettle(p);
    crate::ui::idle::invalidate();
    true
}

/// Which server, and which entry of [`Person::srcs`], mailbox `i` belongs to. `None` when the page
/// does not hold that slot — a worker for a roster that has since been rebuilt, dropped whole and
/// WITHOUT the failure backoff, because nothing failed.
fn src_of(p: &Person, i: usize) -> Option<(ServerId, usize)> {
    let (sid, _) = un_fx(i)?;
    Some((sid, p.srcs.iter().position(|s| s.sid == sid)?))
}

/// Install ONE landing on the open person. Returns whether anything actually changed.
fn apply_landing(state: &mut PersonState, i: usize, what: Landing) -> bool {
    let Some(p) = state.current.as_mut() else {
        return false;
    };
    match what {
        // FAILED: leave the store exactly as it was and back off before retrying. One arm for all
        // the fetch kinds, because the countdown is per MAILBOX — a dead source backs off its own
        // two and nothing else's.
        Landing::Media(None)
        | Landing::Profile(None)
        | Landing::Credits(None)
        | Landing::Roles(None) => {
            state.retry_cd[i] = RETRY_FRAMES;
            // The retry still stands; what ends here is the PLACEHOLDER's licence to keep sweeping.
            // See [`facts_pending`] — a wait nothing can end is not a wait, it is a repaint loop.
            if matches!(what, Landing::Profile(None)) {
                p.profile_tried = true;
            }
            false
        }
        Landing::Profile(Some(prof)) => {
            // The name is NOT overwritten from here: the local `Role[]` tag is what the credits
            // shelf showed a moment ago, and swapping it for the profile's spelling mid-fetch would
            // rename the person under the user's eyes. Same for the headshot.
            p.roles = roles_line(&prof);
            p.bio = prof.summary;
            p.born = prof.born_at;
            p.died = prof.died_at;
            p.birthplace = prof.birth_place;
            p.profiled = true;
            p.profile_tried = true;
            crate::log(&format!(
                "person: profile guid={} roles='{}' born={} died={} bio={}B",
                p.guid,
                p.roles.join(", "),
                !p.born.is_empty(),
                !p.died.is_empty(),
                p.bio.len()
            ));
            true
        }
        Landing::Media(Some(MediaLanding { shelves, matches })) => {
            let Some((sid, si)) = src_of(p, i) else {
                return false;
            };
            {
                let s = &mut p.srcs[si];
                crate::log(&format!(
                    "person: source {} '{}' movies={}/{} shows={}/{} joinable={}",
                    sid.raw(),
                    p.name,
                    shelves[0].items.len(),
                    shelves[0].total,
                    shelves[1].items.len(),
                    shelves[1].total,
                    matches.len()
                ));
                // Assigning the whole array is what carries the invariant: the captions described
                // the OLD items, so they go WITH them (a `Shelf` lands with `roles` empty) and
                // `roled` re-asks. As three loose fields this was three things to remember. The
                // guid index is the same rule one field further: it describes THESE rows, so a
                // response that replaces them replaces it.
                s.shelves = shelves;
                s.matches = matches;
                s.landed = true;
                s.roled = false;
            }
            resettle(p);
            true
        }
        Landing::Credits(Some(groups)) => {
            crate::log(&format!(
                "person: credits guid={} groups={} rows={}",
                p.guid,
                groups.len(),
                groups.iter().map(|g| g.credits.len()).sum::<usize>()
            ));
            p.credits = groups;
            p.credited = true;
            true
        }
        Landing::Roles(Some(RolesLanding { keys, pairs })) => {
            let Some((_, si)) = src_of(p, i) else {
                return false;
            };
            {
                let s = &mut p.srcs[si];
                // A landing addressed to a shelf list that has since been REPLACED must not settle
                // the current one: `Fetch::in_flight`'s doc allows a brief duplicate media worker
                // at one generation, so a second media landing can swap this source's shelves while
                // a roles batch for the first list is in flight. Refusing it (without arming the
                // failure backoff — nothing failed) leaves `roled` false, and `address` simply
                // re-asks with the keys that are now true.
                if keys != s.shelf_keys() {
                    return false;
                }
                // match by ratingKey, never by index: the pairs arrive in the batched response's
                // order, which the server does keep, but nothing about THIS store should depend on
                // it. The keys are this source's own, so no cross-server collision can reach here.
                for sh in s.shelves.iter_mut() {
                    sh.roles = sh
                        .items
                        .iter()
                        .map(|m| {
                            pairs
                                .iter()
                                .find(|(rk, _)| *rk == m.rk)
                                .map(|(_, role)| role.clone())
                                .unwrap_or_default()
                        })
                        .collect();
                }
                s.roled = true;
            }
            resettle(p);
            true
        }
    }
}

/// The roles list: the person's departments, most-credited first, prettified and capped at
/// [`MAX_ROLES`]. Pure, so the wire→display mapping is host-testable. Returns one entry per
/// department — the CALLER joins them (the header's dotted run, the bio panel's `", "`), because
/// two screens want two different separators for the same list.
///
/// `types::CreditType.title` is the display name — **except** when the provider has none, where it repeats
/// the raw slug (`"costume-makeup"`, live on Peter Sallis). A leading lower-case letter is what
/// gives that away, so those are un-slugged and title-cased rather than printed as typed.
pub(crate) fn roles_line(prof: &types::PersonProfile) -> Vec<String> {
    prof.credit_types
        .iter()
        .filter_map(|c| {
            let raw = if c.title.is_empty() {
                &c.kind
            } else {
                &c.title
            };
            let pretty = pretty_department(raw);
            (!pretty.is_empty()).then_some(pretty)
        })
        .take(MAX_ROLES)
        .collect()
}

/// A department name as the provider hands it over → the display form.
///
/// Extracted from [`roles_line`] because the FILMOGRAPHY's tabs have the identical problem on the
/// identical data: `types::CreditGroup::title` repeats the raw `type` slug (`"costume-makeup"`) wherever
/// the provider has no display name, exactly as `types::CreditType::title` does. Two screens un-slugging
/// the same wire field two ways is how "Costume Makeup" and "costume-makeup" end up on one page.
pub(crate) fn pretty_department(raw: &str) -> String {
    raw.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What fetch `i` should be addressed to right now, or `None` when it wants nothing — the ONE place
/// each fetch's precondition and its key live together, so a fetch cannot be gated on one thing and
/// keyed off another.
///
/// `None` covers both "already answered" and **"not actionable"**, and the second is now the common
/// case rather than the exception: a person with no `tagKey` never asks for a biography, a source
/// whose resolve came back empty never asks for a filmography, and neither is a failure or may spin
/// a worker every frame. The two per-source dependencies are expressed as keys rather than as
/// ordering machinery — `K_MEDIA` keys on the source's resolved id, `K_ROLES` on its shelf list,
/// and both are absent until the fetch before them has landed.
///
/// Called every frame for the page's whole life, once per mailbox, so it allocates only when it
/// will actually spawn.
fn address(i: usize, p: &Person) -> Option<Vec<String>> {
    if i == F_PROFILE {
        return (!p.profiled && !p.guid.is_empty()).then(|| vec![p.guid.clone()]);
    }
    if i == F_CREDITS {
        return (!p.credited && !p.guid.is_empty()).then(|| vec![p.guid.clone()]);
    }
    let (sid, kind) = un_fx(i)?;
    let s = p.srcs.iter().find(|s| s.sid == sid)?;
    match kind {
        // K_RESOLVE never asks: its worker went with the retired read layer. The origin's
        // `personId` rode in on the credit row and every other source is born settled (`sources`).
        K_RESOLVE => None,
        K_MEDIA if s.resolved && !s.landed => s.local.clone().map(|id| vec![id]),
        K_ROLES if s.landed && !s.roled => {
            let keys = s.shelf_keys();
            (!keys.is_empty()).then(|| keys.into_iter().map(str::to_string).collect())
        }
        _ => None,
    }
}

/// One fetch per mailbox at a time, and only while the open person still wants it. Re-entered every
/// frame by [`PersonState::pump`], which is what makes the failure path self-healing: a refused `spawn_small`
/// (the device's thread ceiling) or a transient network error simply retries after the backoff
/// instead of latching the page on a spinner forever.
fn maybe_spawn(state: &mut PersonState, adapter: &Arc<PersonAdapter>, i: usize) {
    if adapter.fetch[i].busy() || state.retry_cd[i] > 0 {
        return;
    }
    let Some(p) = state.current() else { return };
    let Some(arg) = address(i, p) else { return };
    let generation = state.generation;
    // the person's global id rides along: the resolve joins its search answer on it, and the roles
    // worker matches a credit row by either id space
    let guid = p.guid.clone();
    if i == F_PROFILE || i == F_CREDITS {
        // The Jellyfin flavor's page is its own provider: the biography is the person ITEM the
        // credit row already named (`fetch_profile_jf`), and the global filmography read has no
        // counterpart on this backend at all — the filmography the route draws there is the
        // per-source shelves below, so the F_CREDITS mailbox asks for nothing.
        if p.sid == crate::jellyfin::SERVER_ID && crate::jellyfin::client().is_some() {
            if i == F_CREDITS {
                return;
            }
            adapter.fetch[i].claim();
            // Same controlled-resource rule every other arm below lives under: a controlled
            // boot records (or replays) the spawn answer, and the landing's admission evidence
            // is what its recorded terminal is fenced by. Global slot: no client/sid/local in
            // the request — the same shape `validate_admission` spells for global person rows.
            let spawned = crate::app::bootstrap::stores::admit(serde_json::json!({
                "store":"person","slot":i,"gen":generation,"arg":arg,"guid":guid}), move || {
                let worker_adapter = Arc::clone(adapter);
                crate::task::spawn_small("person", move || {
                    // filled OUTSIDE the guard so a panicking fetch still lands — as a FAILURE
                    // (None), not as an empty biography
                    let prof = catch_unwind(|| fetch_profile_jf(&arg[0])).unwrap_or(None);
                    worker_adapter.land(i, generation, Landing::Profile(prof));
                })
            });
            if !spawned {
                adapter.fetch[i].release();
            }
            return;
        }
        return;
    }
    let Some((sid, kind)) = un_fx(i) else { return };
    // The Jellyfin slot's own answers. That backend's client lives outside the registry
    // `client_for` reads, so without this arm the fetch backs off forever and the page never
    // fills — the "opening a cast member does nothing" of issue #8 was exactly that silence, plus
    // a provider biography a signed-out session could never fetch. K_RESOLVE is unreachable here
    // by construction (the origin src is born resolved and no other source exists on this
    // backend) and is spelled out anyway so a future caller of the fx space cannot turn that
    // silence into a spawn loop. The filmography landing carries an EMPTY match index: that
    // table joins catalog guids, and a Jellyfin item has none.
    if sid == crate::jellyfin::SERVER_ID && crate::jellyfin::client().is_some() {
        if kind == K_RESOLVE {
            return;
        }
        // The source's own local id rides the recorded request exactly as the retired arm did
        // (this backend's roles worker matches credit rows by key, so the value is carried, not
        // consumed) — and the controlled admission's client identity is the Jellyfin world's 0,
        // mirroring discovery/hubs/metadata.
        let local = p.srcs.iter().find(|s| s.sid == sid)
            .and_then(|s| s.local.clone()).unwrap_or_default();
        let pkey = p.key.clone();
        adapter.fetch[i].claim();
        let spawned = crate::app::bootstrap::stores::admit(serde_json::json!({
            "store":"person","slot":i,"gen":generation,"arg":arg,"guid":guid,
            "local":local,"client":0,"sid":sid.raw()}), move || {
            let worker_adapter = Arc::clone(adapter);
            crate::task::spawn_small("person", move || {
                // same contract as every worker here: a panicking fetch lands as a FAILURE, never as an
                // empty filmography or silently-missing credit names
                let what = match kind {
                    K_MEDIA => Landing::Media(
                        catch_unwind(|| person_media_jf(sid, &arg[0])).unwrap_or(None),
                    ),
                    _ => Landing::Roles(
                        catch_unwind(|| person_roles_jf(&pkey, &arg)).unwrap_or(None),
                    ),
                };
                worker_adapter.land(i, generation, what);
            })
        });
        if !spawned {
            adapter.fetch[i].release();
        }
        return;
    }
    // A non-Jellyfin slot has no fetch left: that read layer (`/hubs/search` resolve,
    // `/library/people/{id}/media`, the batched metadata read) is gone with issue #166. Such a
    // source was born settled (`sources`) and its mailbox simply never fires.
    }

// ---- the Jellyfin backend's answers -----------------------------------------------------------
//
// There is no per-server join on this backend: the person IS an item id the credit row already
// carried, so the whole arm is three direct reads (profile item, filmography page per shelf kind,
// credit rows inline on the filmography) mapped into the same landings the store applies. The
// pure halves are split out (`jf_profile_from`, `jf_shelf`, `jf_roles`)
// because they are the parts worth grading: the date trim, the cap-vs-total split and the
// credit-row match are each exactly the kind of off-by-one a worker test would never catch once
// the real server answers.

/// Map the person's own item record onto the profile landing. The name and headshot are
/// deliberately NOT carried: `apply`'s Profile arm keeps the credit row's spelling and picture
/// for exactly the mid-fetch-rename reason its comment records, and an empty `thumb` here is
/// what leaves that untouched. Jellyfin has no departments and no birthplace on the record;
/// both stay empty, which the header draws as absent rather than as a blank line.
fn jf_profile_from(it: &crate::jellyfin::BaseItemDto) -> types::PersonProfile {
    let date = |v: &Option<String>| {
        v.as_deref()
            .and_then(|s| s.split('T').next())
            .unwrap_or_default()
            .to_string()
    };
    types::PersonProfile {
        title: it.name.clone(),
        summary: it.overview.clone().unwrap_or_default(),
        born_at: date(&it.premiere_date),
        died_at: date(&it.end_date),
        birth_place: String::new(),
        known_for: String::new(),
        thumb: String::new(),
        credit_types: Vec::new(),
    }
}

/// One shelf from one filmography page: rows converted and capped to [`SHELF_MAX`], the REAL
/// total kept beside them (a prolific actor has more credits than tiles, and the heading counts
/// facts, not what fitted), and the roles vector pre-sized parallel with empty strings — the
/// credit names land through their own fetch, never this one.
fn jf_shelf(
    items: &[crate::jellyfin::BaseItemDto],
    total: i64,
    sid: ServerId,
) -> Shelf {
    let mut sh = Shelf::default();
    sh.total = total.max(0) as usize;
    for it in items.iter().take(SHELF_MAX) {
        if let Some(m) = crate::jellyfin::movie_from_dto(it, sid, 0) {
            sh.items.push(m);
        }
    }
    sh.roles = vec![String::new(); sh.items.len()];
    sh
}

/// The credit rows: for every filmography item the shelf holds, the part THIS person played in
/// it, read off the item's own `People[]`. Items outside `keys` are skipped rather than carried —
/// the landing is addressed to one shelf list, exactly the contract `RolesLanding`'s `keys`
/// exists to enforce. A credit with no `Role` string (crew, or a sparse record) answers `""`,
/// which `apply` already reads as "name no part".
fn jf_roles(
    items: &[crate::jellyfin::BaseItemDto],
    person_id: &str,
    keys: &[&str],
) -> Vec<(String, String)> {
    items
        .iter()
        .filter(|it| keys.contains(&it.id.as_str()))
        .map(|it| {
            let role = it
                .people
                .iter()
                .find(|p| p.id == person_id)
                .and_then(|p| p.role.clone())
                .unwrap_or_default();
            (it.id.clone(), role)
        })
        .collect()
}

/// WORKER THREAD: the Jellyfin biography. The detail fetch answers the person's own record; the
/// mapping is the whole job.
fn fetch_profile_jf(person_id: &str) -> Option<types::PersonProfile> {
    crate::jellyfin::client()?.item_detail(person_id).map(|it| jf_profile_from(&it))
}

/// WORKER THREAD: the Jellyfin filmography, both shelves. One page per kind keeps each shelf's
/// REAL total a fact about its own answer rather than a split of a shared count.
fn person_media_jf(sid: ServerId, person_id: &str) -> Option<MediaLanding> {
    let c = crate::jellyfin::client()?;
    let movies = c.person_items(person_id, "Movie", "")?;
    let shows = c.person_items(person_id, "Series", "")?;
    Some(MediaLanding {
        shelves: [
            jf_shelf(&movies.items, movies.total, sid),
            jf_shelf(&shows.items, shows.total, sid),
        ],
        // The cross-source availability join reads a (guid, ratingKey) index. That vocabulary
        // has no catalog guid behind it on this backend - but the filmography query here IS a
        // library query, so every row it returns is an item this server holds, and the honest
        // index joins each id to ITSELF: `match_local` and `seed_dev_credits` read the same
        // shape, a Jellyfin id is its own guid for that purpose, and a filmography row carries
        // the availability mark and the chevron a library-held credit has on the other backend.
        matches: movies
            .items
            .iter()
            .chain(shows.items.iter())
            .map(|it| (it.id.clone(), it.id.clone()))
            .collect(),
    })
}

/// WORKER THREAD: the Jellyfin credit rows, off one filmography page that asks for `People`.
fn person_roles_jf(person_id: &str, keys: &[String]) -> Option<RolesLanding> {
    let c = crate::jellyfin::client()?;
    let res = c.person_items(person_id, "Movie,Series", "People")?;
    let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    Some(RolesLanding {
        keys: keys.to_vec(),
        pairs: jf_roles(&res.items, person_id, &key_refs),
    })
}

/// `(ratingKey, character)` for every row of a batched `/library/metadata/{csv}` response, keeping
/// only THIS person's credit in each — the full record's `Role[]` names every cast member, and the
/// page wants one line, "what did *this* person play in it".
///
/// Which tag IS this person is [`crate::media::Tag::is_person`]'s job — both id spaces, because the
/// local id is the `tagKey` guid whenever the credit row carried no number. `id` is the id local to
/// **the server this response came from** ([`Src::local`]), never the origin's: they are different
/// numbers for the same person, and filtering a share's response with ours matched nothing while
/// still paying for the batch. Rows with **no part for them** (every CREW credit — a director
/// appears in `Director[]`, not `Role[]`) are simply absent from the result rather than carried as
/// empty pairs, and `apply` reads a missing key as `""`. Naming their department instead would need
/// the crew arrays this batch excludes, for a line the mockup does not ask for.
// ---- the filmography, as the route draws it ----------------------------------------------------

/// **A department with fewer credits than this does not earn a tab.** `other` is already one of the
/// provider's own groups, so a thin department JOINS it rather than getting a pill nobody can read
/// a total off — and the fold is by COUNT, so the strip's length does not track a person's
/// obscurity. Peter Sallis's `actor`(222) / `appeared`(21) / `other`(3) folds to two tabs plus
/// Other; a jobbing character actor with six one-credit departments folds to one tab plus Other,
/// rather than to a strip of eight pills each promising a single row.
const FOLD: usize = 5;

/// One row of the Filmography route: a credit, and whether anybody you can reach holds it.
pub(crate) struct Credit {
    /// Discover's stable catalog identity, retained through the availability projection.
    pub(crate) catalog_id: String,
    pub(crate) title: String,
    /// **The poster, as an ABSOLUTE URL on somebody else's host** — see
    /// [`types::CreditItem::thumb`]. Carried through to the row rather than dropped
    /// because `posters` proxies exactly this through the current server's photo transcoder; the
    /// first version of the screen threw it away on an unverified claim that it could not.
    pub(crate) thumb: String,
    /// the character or job, `""` where the provider named none
    pub(crate) role: String,
    /// `0` = the wire carried no year. See [`types::CreditItem::year`] for why that
    /// is not "upcoming".
    pub(crate) year: i32,
    /// **The library match**: which server holds this credit and under which `ratingKey`, or `None`
    /// for a credit nothing in the registry answered for. `None` is UNKNOWN rather than ABSENT — a
    /// source may still be fetching, and one that sent no guids cannot be joined at all — which is
    /// why the route annotates a match and does nothing whatever to a row without one.
    pub(crate) local: Option<(ServerId, String)>,
}

/// One TAB of the Filmography route.
pub(crate) struct Department {
    /// the display name, un-slugged by [`pretty_department`]
    pub(crate) title: String,
    /// the department's OWN count — what the pill states. Not `rows.len()` where the provider gave
    /// a size and sent fewer rows, and never [`types::CreditType`]'s number.
    pub(crate) total: usize,
    pub(crate) rows: Vec<Credit>,
}

/// **The filmography as the route draws it** — folded into tabs, sorted newest-first, and joined
/// against every source's library.
///
/// Pure over the open person, and derived on demand rather than stored, because two of its three
/// steps depend on data that keeps moving: the availability join is a function of the sources, and
/// a share that lands five seconds in must turn "no server behind this" into "Dad's server" without
/// anybody remembering to invalidate a cache. The screen rebuilds it on the frames
/// [`PersonState::pump`] reports a change, which is the same discipline the Person screen's header flow uses.
///
/// **Newest first, and an undated credit goes to the BOTTOM with no year.** The alternative —
/// reading year 0 as "upcoming" and pinning it to the top — makes a title whose metadata is merely
/// missing claim to be unreleased, and the wire cannot tell the two apart until the model carries a
/// release DATE beside the year.
pub(crate) fn filmography(p: &Person) -> Vec<Department> {
    let dept = |g: &types::CreditGroup| {
        let raw = if g.title.is_empty() {
            &g.kind
        } else {
            &g.title
        };
        let rows: Vec<Credit> = g
            .credits
            .iter()
            .filter_map(|c| {
                let it = c.item.as_ref()?;
                (!it.title.is_empty()).then(|| Credit {
                    catalog_id: it.rating_key.clone(),
                    title: it.title.clone(),
                    role: c.role.clone(),
                    year: it.year.clamp(0, i32::MAX as i64) as i32,
                    thumb: it.thumb.clone(),
                    local: match_local(p, &it.rating_key),
                })
            })
            .collect();
        Department {
            title: pretty_department(raw),
            total: (g.size.max(0) as usize).max(rows.len()),
            rows,
        }
    };
    // "Other" is the provider's own group name, so a group already called that folds with the thin
    // ones rather than sitting beside a second Other.
    let is_thin = |g: &types::CreditGroup, d: &Department| {
        d.total < FOLD || g.kind.eq_ignore_ascii_case("other")
    };
    let mut kept: Vec<Department> = Vec::new();
    let mut other = Department {
        title: crate::i18n::msg::browse_person_other().to_string(),
        total: 0,
        rows: Vec::new(),
    };
    for g in &p.credits {
        let d = dept(g);
        if d.rows.is_empty() && d.total == 0 {
            continue;
        }
        if is_thin(g, &d) {
            other.total += d.total;
            other.rows.extend(d.rows);
        } else {
            kept.push(d);
        }
    }
    if other.total > 0 || !other.rows.is_empty() {
        kept.push(other);
    }
    for d in kept.iter_mut() {
        // A STABLE sort, so two credits from the same year keep the provider's own billing order
        // rather than an arbitrary one — and `0` sorts last by construction rather than by a second
        // pass, since it is the only value below every real year.
        d.rows.sort_by_key(|r| match r.year {
            0 => i32::MAX,
            y => -y,
        });
    }
    kept
}

/// **How many credits the filmography holds, without building it.**
///
/// The sum of the department counts — what the Filmography entry row states and what gates the row
/// existing at all. Separate from [`filmography`] because the entry row is drawn every frame and
/// that function folds, sorts and joins a few hundred rows; this walks a handful of group headers.
///
/// It is the credits response's OWN count, never [`types::CreditType`]'s — see that
/// type's doc for the two numbers and why spending the wrong one is a promise the list breaks.
pub(crate) fn filmography_total(p: &Person) -> usize {
    p.credits
        .iter()
        .map(|g| (g.size.max(0) as usize).max(g.credits.len()))
        .sum()
}

/// Which registered server holds `guid`, and under which key — the availability join, over every
/// source's [`Src::matches`] index in registry order.
///
/// The FIRST match wins, and registry order is what makes that deterministic: with a film on two
/// servers the route names the one nearer the front of the roster, which is the same machine every
/// other surface in the app would open it on. An empty guid never matches.
/// **The last path segment of a guid - the catalog ID, which is the ONLY form both sides of the
/// join agree on.** A local row states an item's identity as a full guid, `<prefix>/movie/<id>`.
/// The credits answer states the SAME identity as a bare `ratingKey` of `<id>`, and carries no
/// `guid` field at all (measured 2026-09-06 against a real response). Comparing the two whole
/// strings therefore never matches, which is exactly the bug this exists to end: the page
/// reported `joinable=5`, drew 544 credit rows, and marked none of them.
///
/// Taking the TAIL rather than stripping a `<prefix>/<type>/` prefix is deliberate - the type
/// segment differs between the two sides for the same title (a show credit and the local movie
/// row are two types), and an ID is the same ID whichever side named it.
fn guid_tail(s: &str) -> &str {
    match s.rfind('/') {
        Some(i) => &s[i + 1..],
        None => s,
    }
}

/// Where a credit is held locally, or `None` for the majority of a career that is not.
///
/// Takes the Discover `ratingKey` (a bare catalog ID) and matches it against the tail of each
/// source's own guid index — see [`guid_tail`] for why neither side's string is compared whole.
/// First match wins in registry order, which is the same "yours before a friend's" precedence
/// `screens::alt_sources` states explicitly.
fn match_local(p: &Person, id: &str) -> Option<(ServerId, String)> {
    let id = guid_tail(id);
    if id.is_empty() {
        return None;
    }
    p.srcs.iter().find_map(|s| {
        s.matches
            .iter()
            .find(|(g, _)| guid_tail(g) == id)
            .map(|(_, rk)| (s.sid, rk.clone()))
    })
}

/// TEST ONLY: publish a credits answer onto the open person exactly as a successful landing would.
///
/// `(title, count)` per department, with that many rows apiece — the shape the folding, the sort
/// and the entry row's count all read. The rows carry no guid, so nothing joins: the availability
/// column is the one thing this helper cannot stand in for, and a test that wants it seeds
/// `Src::matches` instead.
#[cfg(test)]
impl PersonState {
pub(crate) fn install_credits_for_test(&mut self, groups: &[(&str, usize)]) {
    let Some(p) = self.current.as_mut() else {
        return;
    };
    p.credits = groups
        .iter()
        .enumerate()
        .map(|(department, (title, n))| types::CreditGroup {
            kind: title.to_lowercase(),
            title: title.to_string(),
            size: *n as i64,
            credits: (0..*n)
                .map(|i| types::Credit {
                    order: i as i64,
                    role: format!("Part {i}"),
                    item: Some(types::CreditItem {
                        rating_key: format!("s{:08x}", ((department as u32) << 16) | i as u32),
                        kind: "movie".into(),
                        title: format!("{title} {i}"),
                        year: 2000 + i as i64,
                        ..Default::default()
                    }),
                })
                .collect(),
        })
        .collect();
    p.credited = true;
}

/// TEST ONLY: publish shelves onto the open person exactly as a successful landing would, so the
/// screen's focus/flow tests need neither a server nor the mailbox. Lands them on the FIRST source
/// (minting one for the open person's own server when the registry is empty, which is the state
/// `ui::person`'s tests open in) and then re-derives the merge, so what the screen reads came
/// through the same projection a real landing does.
///
/// **Also settles `credited`**, for the same reason: a real landing that populates the shelves
/// answers the filmography question in the same event, and `ui::person::clamp_focus` holds the
/// page's default focus on the entry row exactly until `credited` says so — a caller that wants a
/// PENDING person (focus still parked, nothing walkable yet) should not call this at all rather
/// than call it and expect `credited` to stay false.
pub(crate) fn install_for_test(&mut self, movies: Vec<CatalogMovie>, shows: Vec<CatalogMovie>) {
    let Some(p) = self.current.as_mut() else {
        return;
    };
    p.credited = true;
    if p.srcs.is_empty() {
        p.srcs.push(Src {
            sid: p.sid,
            local: None,
            resolved: true,
            shelves: Default::default(),
            matches: Vec::new(),
            landed: false,
            roled: false,
        });
    }
    p.srcs[0].shelves = [movies, shows].map(|items| Shelf {
        total: items.len(),
        items,
        roles: Vec::new(),
    });
    p.srcs[0].landed = true;
    p.srcs[0].roled = false;
    resettle(p);
}

/// TEST ONLY: land one named source's shelves through the same per-source ownership and merge
/// projection as [`apply_landing`]'s successful media arm. Unlike [`PersonState::install_for_test`], this does not
/// settle unrelated credits state and therefore supports multi-source pending-return tests.
pub(crate) fn install_source_for_test(&mut self, sid: ServerId, movies: Vec<CatalogMovie>, shows: Vec<CatalogMovie>) {
    let Some(p) = self.current.as_mut() else {
        return;
    };
    let Some(source) = p.srcs.iter_mut().find(|source| source.sid == sid) else {
        return;
    };
    source.shelves = [movies, shows].map(|items| Shelf {
        total: items.len(),
        items,
        roles: Vec::new(),
    });
    source.landed = true;
    source.roled = false;
    resettle(p);
}

pub(crate) fn seed_ownership_fixture_for_test(&mut self, adapter: &Arc<PersonAdapter>) {
    self.retry_cd[0] = 17;
    adapter.fetch[1].claim();
    adapter.land(1, self.generation.max(1), Landing::Media(None));
}

pub(crate) fn ownership_fixture_for_test(&self, adapter: &Arc<PersonAdapter>) -> OwnershipFixture {
    OwnershipFixture {
        name: self.current().map(|person| person.name.clone()),
        generation: self.generation,
        retry_cd: self.retry_cd,
        flights: std::array::from_fn(|i| adapter.fetch[i].busy()),
        mail: std::array::from_fn(|i| adapter.fetch[i].slot.lock()
            .unwrap_or_else(|e| e.into_inner()).is_some()),
    }
}

pub(crate) fn late_completion_for_test(
    &self,
    adapter: &Arc<PersonAdapter>,
) -> Box<dyn FnOnce()> {
    let adapter = Arc::clone(adapter);
    let generation = self.generation;
    Box::new(move || {
        adapter.land(F_PROFILE, generation, Landing::Profile(Some(
            types::PersonProfile {
                summary: "late-old-worker".into(),
                ..Default::default()
            },
        )));
    })
}
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OwnershipFixture {
    name: Option<String>,
    generation: u32,
    retry_cd: [u32; NFETCH],
    flights: [bool; NFETCH],
    mail: [bool; NFETCH],
}

// ---------------------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// Slot 0 — the server every single-source test opens on. A real slot, so its mailboxes exist,
    /// but nothing is registered at it: `maybe_spawn` therefore refuses to dial (`client_for` is
    /// `None`) and every test below drives the mailbox by hand.
    const S0: ServerId = ServerId::from_raw(0);
    const S1: ServerId = ServerId::from_raw(1);

    struct Owner {
        state: PersonState,
        adapter: Arc<PersonAdapter>,
    }

    impl Default for Owner {
        fn default() -> Self {
            Self { state: Default::default(), adapter: Arc::new(Default::default()) }
        }
    }

    impl Owner {
        fn current(&self) -> Option<&Person> { self.state.current() }
        fn gen(&self) -> u32 { self.state.generation }
        fn open(&mut self, sid: ServerId, key: &str, guid: &str, name: &str, thumb: &str) {
            open(&mut self.state, &self.adapter, sid, key, guid, name, thumb);
        }
        fn close(&mut self) { close(&mut self.state, &self.adapter); }
        fn reset(&mut self) {
            self.adapter = Arc::new(Default::default());
            reset(&mut self.state, &self.adapter);
        }
        fn land(&self, slot: usize, generation: u32, landing: Landing) {
            self.adapter.land(slot, generation, landing);
        }
        fn apply(&mut self, slot: usize, landing: Landing) -> bool {
            apply_landing(&mut self.state, slot, landing)
        }
        fn pump(&mut self) -> bool { self.state.pump(&self.adapter) }
        fn sync_roster(&mut self) -> bool { sync_roster(&mut self.state, &self.adapter) }
        fn hold_off(&mut self) { self.state.retry_cd = [RETRY_FRAMES; NFETCH]; }
        fn supersede(&mut self) { self.state.supersede(&self.adapter); }
    }

    /// The controlled person world boots the synthetic Jellyfin home; refused without the feature.
    #[test]
    fn controlled_person_ignores_ambient_session_recovery() {
        let _g = crate::testlock::serial();
        struct ResetTape;
        impl Drop for ResetTape {
            fn drop(&mut self) { crate::app::bootstrap::stores::reset_for_test(); }
        }
        for replay in [false, true] {
            let _session = crate::server::session::TempSession::new("controlled-person-recovery");
            crate::registry::reset_for_test();
            let saved = crate::server::session::peek();
            crate::server::session::install_transient_for_test(true);
            let initial = crate::app::bootstrap::Initial::synthetic_home(23, 32517, None).unwrap();
            crate::app::bootstrap::stores::init(&initial, replay);
            let _tape = ResetTape;
            let mut owner = Owner::default();
            owner.open(S0, "1001", "", "Synthetic person", "");
            owner.state.current.as_mut().unwrap().profiled = true;
            owner.state.current.as_mut().unwrap().credited = true;
            let generation = owner.gen();
            // Minimized from ARM: the ambient cache recovered at frame 26 while
            // recording, but frame 20 on replay, retiring/reissuing Person work.
            crate::server::session::save(&saved);
            let changed = owner.pump();
            assert_eq!(owner.gen(), generation,
                "controlled Person must not retire recorded work on an ambient storage completion");
            assert!(!changed);
            assert!(owner.current().unwrap().profiled);
            assert!(owner.current().unwrap().credited);
            let (requests, failure) = crate::app::bootstrap::stores::finish();
            assert!(requests.is_empty());
            assert_eq!(failure, None);
        }
    }

    #[test]
    fn session_refresh_retries_person_metadata_after_storage_recovers() {
        let _g = crate::testlock::serial();
        let _session = crate::server::session::TempSession::new("person-session-refresh");
        crate::registry::reset_for_test();
        let saved = crate::server::session::peek();
        crate::server::session::install_transient_for_test(true);
        let mut owner = Owner::default();
        owner.open(S0, "1001", "", "Synthetic person", "");
        owner.state.current.as_mut().unwrap().profiled = true;
        owner.state.current.as_mut().unwrap().credited = true;
        let old_generation = owner.gen();
        crate::server::session::save(&saved);
        assert!(owner.pump());
        assert!(owner.gen() > old_generation, "old credential-bound replies must be retired");
        assert!(!owner.current().unwrap().profiled);
        assert!(!owner.current().unwrap().credited);
    }

    #[test]
    fn filmography_preserves_provider_identity_without_a_local_library_copy() {
        let mut owner = Owner::default();
        let _g = crate::testlock::serial();
        crate::registry::reset_for_test();
        owner.reset();
        owner.open(S0, "1001", "", "s00000001", "");
        owner.state.install_credits_for_test(&[("Actor", 4)]);
        let departments = filmography(owner.current().unwrap());
        let rows: Vec<_> = departments.iter().flat_map(|d| &d.rows).collect();
        assert_eq!(rows.iter().map(|r| r.catalog_id.as_str()).collect::<Vec<_>>(),
            ["s00000003", "s00000002", "s00000001", "s00000000"]);
        assert!(rows.iter().all(|r| r.local.is_none()),
            "provider identity survives even when no the server can supply a local ratingKey");
        owner.reset();
        crate::registry::reset_for_test();
    }

    #[test]
    fn an_open_page_sources_exactly_the_origin_on_the_single_source_roster() {
        let mut owner = Owner::default();
        let _g = crate::testlock::serial();
        crate::registry::reset_for_test();
        owner.reset();
        let a = crate::registry::ServerId::from_raw(0);
        owner.open(a, "7", "catalog://person/7", "Actor", "");
        // The roster the page used to append went with the registry (#166): the only source is
        // the ORIGIN itself, force-included because the page must answer for the credit row that
        // opened it. `sync_roster` can never report a change: nothing moves the roster epoch.
        assert_eq!(
            owner.current()
                .unwrap()
                .srcs
                .iter()
                .map(|s| s.sid)
                .collect::<Vec<_>>(),
            [a]
        );
        assert!(!owner.sync_roster());

        owner.reset();
        crate::registry::reset_for_test();
    }

    /// A [`Landing::Media`] payload the way a worker builds one — totals = lens, i.e. an uncapped
    /// response, and no guid index (these rows are minted, not parsed, so they carry no guids to
    /// join on).
    fn media(movies: Vec<CatalogMovie>, shows: Vec<CatalogMovie>) -> Landing {
        Landing::Media(Some(MediaLanding {
            shelves: [movies, shows].map(|items| Shelf {
                total: items.len(),
                items,
                roles: Vec::new(),
            }),
            matches: Vec::new(),
        }))
    }

    /// A movie row on a named server — the pair that has to survive the merge.
    fn movie_on(sid: ServerId, rk: &str) -> CatalogMovie {
        CatalogMovie {
            sid,
            rk: rk.to_string(),
            ..Default::default()
        }
    }

    /// One source, already landed with these shelves — the input [`merge_shelves`] takes.
    fn src_with(sid: ServerId, movies: Vec<CatalogMovie>, total: usize, roles: Vec<String>) -> Src {
        let mut s = Src {
            sid,
            local: Some("1".into()),
            resolved: true,
            shelves: Default::default(),
            matches: Vec::new(),
            landed: true,
            roled: false,
        };
        s.shelves[0] = Shelf {
            total,
            items: movies,
            roles,
        };
        s
    }

    #[test]
    fn per_source_media_resolution_stays_pending_across_failure_until_an_answer() {
        let mut owner = Owner::default();
        let _serial = crate::testlock::serial();
        owner.reset();
        owner.open(S0, "6059", "guid", "Somebody", "");
        assert!(media_resolving(owner.current().unwrap(), S0));

        assert!(
            !owner.apply(at(S0, K_MEDIA), Landing::Media(None)),
            "a transport failure changes no published shelves"
        );
        assert!(
            media_resolving(owner.current().unwrap(), S0),
            "failure backs off and retries; it is not a terminal missing answer"
        );

        assert!(owner.apply(at(S0, K_MEDIA), media(Vec::new(), Vec::new())));
        assert!(
            !media_resolving(owner.current().unwrap(), S0),
            "a successful empty media response is a terminal answer for this source"
        );
        assert!(
            !media_resolving(owner.current().unwrap(), S1),
            "a source absent from the current roster cannot restore a card"
        );
        owner.reset();
    }

    // ---- the merge ---------------------------------------------------------------------------

    /// The merge is the filmography: every source's rows, in source order, each still carrying the
    /// server it came from — which is what makes the card open on the right machine. The heading's
    /// count is the sum of the REAL totals, not of the tiles that fitted, and the captions stay
    /// index-parallel across the seam even while one source's roles batch is still out.
    #[test]
    fn the_merge_keeps_every_sources_rows_their_server_and_their_captions() {
        let srcs = vec![
            src_with(
                S0,
                vec![movie_on(S0, "1"), movie_on(S0, "2")],
                30,
                vec!["Elsa".into(), "Nancy".into()],
            ),
            // …the share's roles batch has not landed yet: its captions are absent, not empty
            src_with(
                S1,
                vec![movie_on(S1, "1"), movie_on(S1, "9")],
                5,
                Vec::new(),
            ),
        ];
        let m = merge_shelves(&srcs);
        assert_eq!(m[0].items.len(), 4, "both sources contribute");
        assert_eq!(
            m[0].items
                .iter()
                .map(|x| (x.sid, x.rk.as_str()))
                .collect::<Vec<_>>(),
            vec![(S0, "1"), (S0, "2"), (S1, "1"), (S1, "9")],
            "one ratingKey on two servers is two different films — the pair is the identity"
        );
        assert_eq!(
            m[0].total, 35,
            "the heading counts what the PERSON has, across every source"
        );
        assert_eq!(
            m[0].roles.len(),
            m[0].items.len(),
            "captions must stay index-parallel"
        );
        assert_eq!(&m[0].roles[..2], &["Elsa".to_string(), "Nancy".to_string()]);
        assert!(
            m[0].roles[2..].iter().all(String::is_empty),
            "an un-captioned source reads blank"
        );
        assert!(m[1].items.is_empty(), "neither source had a show");

        // and with nothing anywhere the merge is empty rather than a panic
        assert!(merge_shelves(&[])
            .iter()
            .all(|s| s.items.is_empty() && s.total == 0));
    }

    /// A greedy source must not spend the whole shelf. Two sources with more films than the row can
    /// hold split it — the bug this unit exists for, re-created one level down: fill the row from
    /// the server you arrived through and the share is invisible again.
    #[test]
    fn one_prolific_source_cannot_starve_another_out_of_the_shelf() {
        let many = |sid: ServerId, n: usize| {
            (0..n)
                .map(|i| movie_on(sid, &format!("{i}")))
                .collect::<Vec<_>>()
        };
        let srcs = vec![
            src_with(S0, many(S0, SHELF_MAX), SHELF_MAX, Vec::new()),
            src_with(S1, many(S1, SHELF_MAX), SHELF_MAX, Vec::new()),
        ];
        let m = merge_shelves(&srcs);
        assert_eq!(
            m[0].items.len(),
            SHELF_MAX,
            "the row still caps at the spring count"
        );
        assert!(
            m[0].items.iter().any(|x| x.sid == S1),
            "the second source was starved out of the shelf entirely"
        );
        assert_eq!(
            m[0].total,
            2 * SHELF_MAX,
            "…while the heading still counts everything"
        );
    }

    // ---- the fetch machine -------------------------------------------------------------------

    /// Park EVERY fetch, so a take's RELEASE of a single-flight flag is observable rather than
    /// immediately masked by the next fetch claiming it — and, more importantly, so the host suite
    /// spawns NO worker: one would reach for a server client that isn't installed and for a
    /// provider these tests must never touch, and a stray background thread also perturbs the
    /// process-wide fd count `stream.rs`'s tests assert on. Call it before every `owner.pump()`.
    fn profile(bio: &str, born: &str, died: &str) -> types::PersonProfile {
        types::PersonProfile {
            summary: bio.to_string(),
            born_at: born.to_string(),
            died_at: died.to_string(),
            ..Default::default()
        }
    }

    /// The mailbox of source `sid`'s fetch of kind `k` — every test below drives one by hand.
    fn at(sid: ServerId, k: usize) -> usize {
        fx(sid, k).expect("a real slot")
    }

    /// The claim and the mailbox are ONE thing, and [`Fetch::take`] is where that is spelled: the
    /// claim goes with whatever it hands back — an answer, a failure, or a generation the pump is
    /// about to discard — while an EMPTY mailbox releases nothing, because the claim it would clear
    /// belongs to a worker still out and the next frame would spawn a duplicate. Asserted on the
    /// pair directly, since both halves are properties of the take rather than of `pump`'s sweep.
    #[test]
    fn a_take_releases_the_claim_and_an_empty_mailbox_leaves_it_alone() {
        let owner = Owner::default();
        let _serial = crate::testlock::serial();
        let f = &owner.adapter.fetch[at(S0, K_ROLES)];
        f.clear();

        f.claim();
        assert!(f.take().is_none(), "nothing has landed");
        assert!(
            f.busy(),
            "an empty mailbox must not un-claim a worker still out"
        );

        owner.land(
            at(S0, K_ROLES),
            owner.gen(),
            Landing::Roles(None),
        );
        assert!(f.take().is_some());
        assert!(
            !f.busy(),
            "the claim must go with the mail — a FAILURE released it too"
        );

        f.clear();
    }

    /// A late landing for the actor you already left must not repopulate the one you are looking
    /// at. `open` bumps the generation; `pump` drops anything older — and it must still clear the
    /// single-flight flag while doing so, or the NEW person can never fetch.
    #[test]
    fn a_landing_from_the_previous_person_is_discarded_but_still_releases_the_fetch() {
        let mut owner = Owner::default();
        let _serial = crate::testlock::serial();
        owner.open(S0, "161", "5d776", "Idina Menzel", "");
        let stale = owner.gen();
        owner.open(S0, "465", "5d777", "Cynthia Erivo", ""); // supersedes: the fetch above is now obsolete

        owner.adapter.fetch[at(S0, K_MEDIA)].claim();
        owner.hold_off();
        owner.land(
            at(S0, K_MEDIA),
            stale,
            media(vec![CatalogMovie::default()], Vec::new()),
        );
        assert!(!owner.pump(), "a superseded landing must not publish");

        let p = owner.current().expect("the new person stays open");
        assert_eq!(p.key, "465");
        assert!(
            p.shelf(0).is_empty(),
            "the previous actor's filmography leaked in"
        );
        assert!(!p.landed, "a discarded landing must not settle the spinner");
        assert!(
            !owner.adapter.fetch[at(S0, K_MEDIA)].busy(),
            "the take must release the single-flight even for a landing it drops"
        );
        owner.close();
    }

    /// A FAILED fetch (None) must leave a populated page alone and schedule a retry — the
    /// "one wifi hiccup blanked a populated grid" regression, in this store's shape.
    #[test]
    fn a_failed_fetch_keeps_the_shelves_and_backs_off_instead_of_publishing_empty() {
        let mut owner = Owner::default();
        let _serial = crate::testlock::serial();
        owner.open(S0, "161", "5d776", "Idina Menzel", "");
        let gen = owner.gen();
        // seed a populated, landed page the honest way (through the pump)
        owner.land(
            at(S0, K_MEDIA),
            gen,
            media(vec![CatalogMovie::default()], Vec::new()),
        );
        owner.hold_off();
        assert!(owner.pump());
        assert_eq!(owner.current().unwrap().shelf(0).len(), 1);

        owner.land(at(S0, K_MEDIA), gen, Landing::Media(None)); // the retry fails
        owner.hold_off();
        assert!(!owner.pump(), "a failure publishes nothing");
        assert_eq!(
            owner.current().unwrap().shelf(0).len(),
            1,
            "the failure wiped a populated shelf"
        );
        assert_eq!(
            owner.state.retry_cd[at(S0, K_MEDIA)],
            RETRY_FRAMES,
            "a failure must back off before retrying"
        );
        owner.close();
    }

    /// `close`/`reset` drop the mailboxes, and a single-flight flag is cleared ONLY by a successful
    /// take — so they must clear EVERY one (and the backoffs) themselves or the next person opened
    /// never fetches. The `browse.rs` latch, one store over, now once per fetch kind PER SOURCE:
    /// the whole index space is walked, so a slot that has left the roster cannot latch either.
    #[test]
    fn close_clears_every_single_flight_flag_and_retry_backoff() {
        let mut owner = Owner::default();
        let _serial = crate::testlock::serial();
        owner.open(S0, "161", "5d776", "Idina Menzel", "");
        for i in 0..NFETCH {
            owner.adapter.fetch[i].claim();
            owner.state.retry_cd[i] = RETRY_FRAMES;
        }

        owner.reset();

        for i in 0..NFETCH {
            assert!(
                !owner.adapter.fetch[i].busy(),
                "fetch {i} stayed latched — the page wedges"
            );
            assert_eq!(owner.state.retry_cd[i], 0);
        }
        assert!(owner.current().is_none());
    }

    /// The profile lands into the HEADER fields and settles the fetch - and it must not disturb
    /// the shelves, which come from a different mailbox. It stays a SINGLE fetch on purpose: it is
    /// about the person, not about anybody's library.
    #[test]
    fn a_profile_landing_fills_the_header_without_touching_the_shelves() {
        let mut owner = Owner::default();
        let _serial = crate::testlock::serial();
        owner.open(S0, "6059", "5d7768268718ba001e311be6", "Peter Sallis", "");
        let gen = owner.gen();
        owner.land(
            at(S0, K_MEDIA),
            gen,
            media(vec![CatalogMovie::default()], Vec::new()),
        );
        owner.hold_off();
        assert!(owner.pump());

        owner.land(
            F_PROFILE,
            gen,
            Landing::Profile(Some(profile(
                "An English actor.",
                "1921-02-01",
                "2017-06-02",
            ))),
        );
        owner.hold_off();
        assert!(
            owner.pump(),
            "the profile landing is a change the screen must see"
        );
        let p = owner.current().unwrap();
        assert_eq!(p.bio, "An English actor.");
        assert_eq!(p.born, "1921-02-01");
        assert_eq!(
            p.died, "2017-06-02",
            "a deceased person's Died line has to survive the landing"
        );
        assert!(p.profiled);
        assert_eq!(
            p.shelf(0).len(),
            1,
            "the biography landing wiped the shelves"
        );
        owner.close();
    }

    /// The backend answers **200 with an empty record** for a person it has never heard of, which
    /// the profile mapping turns into a DEFAULT profile. That is an answer, not a failure: it must
    /// settle `profiled` (so the page stops asking) and arm NO backoff - while a real failure does
    /// the opposite. Getting this backwards is a page that re-requests a biography forever.
    #[test]
    fn an_unknown_person_settles_the_profile_while_a_failure_backs_off() {
        let mut owner = Owner::default();
        let _serial = crate::testlock::serial();
        owner.open(S0, "6059", "0000000000000000000000ff", "Nobody", "");
        let gen = owner.gen();
        owner.hold_off();

        owner.land(
            F_PROFILE,
            gen,
            Landing::Profile(Some(types::PersonProfile::default())),
        );
        assert!(owner.pump());
        let p = owner.current().unwrap();
        assert!(
            p.profiled,
            "an 'unknown person' answer must settle, not retry forever"
        );
        assert!(
            p.bio.is_empty() && p.roles.is_empty(),
            "nothing may be invented for an unknown person"
        );
        assert!(
            owner.state.retry_cd[F_PROFILE] < RETRY_FRAMES,
            "an ANSWER must not arm the failure backoff"
        );

        owner.land(F_PROFILE, gen, Landing::Profile(None)); // now a real transport failure
        owner.hold_off();
        assert!(!owner.pump());
        assert_eq!(
            owner.state.retry_cd[F_PROFILE],
            RETRY_FRAMES,
            "a failure must back off before retrying"
        );
        owner.close();
    }

    /// A roles landing captions the shelves BY KEY (order-independent), and the NEXT media landing
    /// clears both vectors with the shelves they described — a caption must never outlive the list
    /// it was addressed to, and the cleared `roled` is what re-asks for the new one.
    #[test]
    fn a_roles_landing_captions_by_key_and_a_media_landing_resets_it() {
        let mut owner = Owner::default();
        let _serial = crate::testlock::serial();
        owner.open(S0, "6059", "5d7768268718ba001e311be6", "Peter Sallis", "");
        let gen = owner.gen();
        let (fm, fr) = (at(S0, K_MEDIA), at(S0, K_ROLES));
        let movie = |rk: &str| movie_on(S0, rk);
        owner.land(
            fm,
            gen,
            media(vec![movie("1971"), movie("2005")], vec![movie("1975")]),
        );
        owner.hold_off();
        assert!(owner.pump());

        // a landing addressed to keys the store no longer holds must be REFUSED — and without the
        // failure backoff: it is stale, not broken, and the re-ask must be free to go at once. The
        // sentinel countdown still parks the spawn (see `hold_off`), but is small enough that a
        // wrongly-armed RETRY_FRAMES would overwrite it visibly.
        let stale = RolesLanding {
            keys: vec!["9999".to_string()],
            pairs: vec![("9999".to_string(), "Nobody".to_string())],
        };
        owner.land(fr, gen, Landing::Roles(Some(stale)));
        owner.hold_off();
        owner.state.retry_cd[fr] = 5;
        assert!(!owner.pump(), "a mis-addressed caption landing published");
        assert_eq!(
            owner.current().unwrap().role(0, 0),
            "",
            "a stale landing captioned the CURRENT list"
        );
        assert_eq!(
            owner.state.retry_cd[fr],
            4,
            "staleness is not a failure — no backoff armed (the sentinel just ticked)"
        );

        // pairs arrive REVERSED relative to the shelves — the match is by rk, so it must not matter
        let keys = vec!["1971".to_string(), "2005".to_string(), "1975".to_string()];
        let pairs = vec![
            ("1975".to_string(), "Wallace".to_string()),
            ("2005".to_string(), "Wallace / Hutch (voice)".to_string()),
            ("1971".to_string(), "Wallace (voice)".to_string()),
        ];
        owner.land(fr, gen, Landing::Roles(Some(RolesLanding { keys, pairs })));
        owner.hold_off();
        assert!(owner.pump(), "a caption landing is a change the screen must see");
        let p = owner.current().unwrap();
        assert_eq!(p.role(0, 0), "Wallace (voice)");
        assert_eq!(p.role(0, 1), "Wallace / Hutch (voice)");
        assert_eq!(p.role(1, 0), "Wallace");
        assert_eq!(
            p.role(0, 99),
            "",
            "past-the-end reads are blank, not a panic"
        );

        // a fresh media landing (same person) replaces the shelves — the captions go WITH them
        owner.land(fm, gen, media(vec![movie("2005")], Vec::new()));
        owner.hold_off();
        assert!(owner.pump());
        assert_eq!(
            owner.current().unwrap().role(0, 0),
            "",
            "a caption survived the list it was addressed to"
        );
        // …and the cleared flag is what re-asks: the roles fetch is addressable again
        assert!(
            address(fr, owner.current().unwrap()).is_some(),
            "the reset is what re-asks for the new list's captions"
        );
        owner.close();
    }

    /// The roles line is the header's "Actor, Producer" kicker: display titles, most-credited
    /// first, capped - and the profile's own un-named departments (`title == the raw slug`, live on
    /// Peter Sallis's `costume-makeup`) are un-slugged rather than printed as typed.
    #[test]
    fn the_roles_line_prettifies_slugs_and_caps_the_list() {
        let ct = |kind: &str, title: &str| types::CreditType {
            kind: kind.to_string(),
            title: title.to_string(),
        };
        let mut prof = types::PersonProfile::default();
        prof.credit_types = vec![
            ct("actor", "Actor"),
            ct("writer", "Writer"),
            ct("producer", "Producer"),
            ct("music", "Composer"), // past the cap
        ];
        assert_eq!(
            roles_line(&prof),
            vec!["Actor".to_string(), "Writer".to_string(), "Producer".to_string()]
        );

        prof.credit_types = vec![ct("costume-makeup", "costume-makeup"), ct("art", "")];
        assert_eq!(
            roles_line(&prof),
            vec!["Costume Makeup".to_string(), "Art".to_string()],
            "a raw slug reached the screen"
        );

        assert_eq!(
            roles_line(&types::PersonProfile::default()),
            Vec::<String>::new()
        );
    }
    /// **A wait nothing can end is not a wait.** `facts_pending` drives the header's placeholder
    /// sweep, and every one of those bars calls `idle::invalidate()` — so a pending state that can
    /// never resolve defeats the whole-frame present gate and repaints the page at 60fps forever.
    /// That is the ordinary case whenever the profile cannot be fetched: an offline or LAN-only
    /// set, a provider outage, or any headless boot, where the injected token was a SERVER token
    /// the metadata provider answered 401 to.
    #[test]
    fn the_header_stops_sweeping_once_the_profile_has_actually_been_asked() {
        let mut owner = Owner::default();
        let _serial = crate::testlock::serial();
        owner.open(ServerId::from_raw(0), "1", "guid", "Somebody", "");
        let p = owner.state.current.as_mut().expect("open mounts a person");
        assert!(facts_pending(p), "before any attempt, the band is genuinely waiting");
        p.profile_tried = true;
        assert!(
            !facts_pending(p),
            "one failed attempt ends the wait: the band degrades to the name it already has"
        );
        // …and a later success still fills it in — `profiled` is what draws the content.
        p.profiled = true;
        assert!(!facts_pending(p));
        owner.supersede();
    }

}
