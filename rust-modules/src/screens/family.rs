//! **The Settings family's shared plumbing** (restructure spec §6.2 `SettingsSurface`, phase 5b):
//! the surface's INNER host and its argument vocabulary ([`SettingsPage`]), the conversions a
//! surface makes when it steps a page of its own stack under the outer dispatcher's context, and
//! the two things every page in the family does the same way — seating a `TableView` on the
//! engine's focus and naming its element keys.
//!
//! Not a screen: `screens/mod.rs`'s rule that a screen never names a sibling is what this module
//! exists to satisfy — Legal pushes a document, Privacy pushes a preview, the root pushes all of
//! them, and each names the destination through this vocabulary rather than through the module
//! that implements it.

use crate::ui::machine::{Canon, Chrome, Cx, Host, LogicalState, ScreenId};
use crate::ui::screen::ScreenArg;
use crate::ui::table::TableView;
use crate::ui::widgets::ControlPalette;

use super::registry::{AppFx, AppMsg, DirectoryLike};

/// A first-run background may mount before the persisted hero seed arrives. Keep the same
/// generation cursor as other session-derived views, while a live hub seed remains authoritative.
pub(crate) struct SessionGround {
    ground: crate::ui::route_screen::RouteGround,
    watch: crate::plex::session::VisibleSessionWatch,
    from_session: bool,
    seed: Option<[[f32; 3]; 4]>,
}
impl SessionGround {
    pub(crate) fn new() -> Self {
        Self { ground: crate::ui::route_screen::RouteGround::new(), watch: Default::default(),
            from_session: false, seed: None }
    }
    pub(crate) fn refresh(&mut self) -> bool {
        if !self.from_session || !self.watch.changed() { return false; }
        let Some(session) = crate::plex::session::peek_settled() else { return false; };
        if self.seed == session.last_hero_blur { return false; }
        self.seed = session.last_hero_blur;
        self.ground = crate::ui::route_screen::RouteGround::for_home(self.seed);
        true
    }
}
impl std::ops::Deref for SessionGround {
    type Target = crate::ui::route_screen::RouteGround;
    fn deref(&self) -> &Self::Target { &self.ground }
}
impl std::ops::DerefMut for SessionGround {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.ground }
}

/// Prefer the already-published Home hero; otherwise observe the persisted seed as it lands.
pub(crate) fn pre_home_ground(hubs: crate::pms::HubsView<'_>) -> SessionGround {
    let live = hubs.hero(0).filter(|hero| hero.item.has_blur).map(|hero| hero.item.blur);
    if let Some(blur) = live {
        let _ = crate::storage_worker::submit_retained(move || crate::plex::session::record_last_hero(blur));
    }
    let seed = live.or_else(crate::plex::session::last_hero);
    SessionGround { ground: crate::ui::route_screen::RouteGround::for_home(seed),
        watch: Default::default(), from_session: live.is_none(), seed }
}

/// A page of the surface's own stack (§6.2: root → Privacy | Legal | Favourites → Document).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SettingsPage {
    /// The Settings root (`ui::settings`'s table).
    Root,
    /// Install-wide playback defaults.
    Playback,
    /// Plex account audio and subtitle preferences.
    AudioSubtitles,
    /// Favorite libraries — the onboard screen in Settings mode.
    Favourites,
    /// The Legal index.
    Legal,
    /// About PlxNative, a document pushed straight from the root.
    About,
    /// One Legal document, by index into `screens::legal`'s page list.
    Document(u8),
}

impl ScreenArg for SettingsPage {
    fn chrome(&self) -> Chrome {
        Chrome::None
    }
    fn id(&self) -> ScreenId {
        ScreenId(match self {
            SettingsPage::Root => 100,
            SettingsPage::Playback => 108,
            SettingsPage::AudioSubtitles => 109,
            SettingsPage::Favourites => 101,
            SettingsPage::Legal => 103,
            SettingsPage::About => 104,
            SettingsPage::Document(_) => 105,
        })
    }
    fn title(&self) -> Option<&str> {
        None
    }
    fn same_instance(&self, other: &Self) -> bool {
        self == other
    }
}

/// **A page's canonical encoding, for the surface's own logical state** (§5.4).
///
/// `screens::settings`'s `RouteSurface` hashes its inner stack entry by entry, and one entry is
/// distinguished from its neighbour by WHICH page it is — so both halves have to be written, and
/// they are written HERE rather than at the hash site so that a variant added to this enum is
/// answered by a census `match` beside its own declaration. A `_ =>` arm at a distant call site
/// would silently hash two different documents as one, which is exactly the class of miss the
/// recorder exists to catch.
///
/// The kind is [`ScreenArg::id`]'s number rather than a second numbering of the same question —
/// that is already the id the container keys page KIND on. The index is written for EVERY variant,
/// `0` where there is none, so a payload-less variant that later grows one cannot keep hashing
/// identically to its old self.
impl LogicalState for SettingsPage {
    fn write(&self, w: &mut Canon) {
        w.discriminant(ScreenArg::id(self).0);
        w.u8(match self {
            SettingsPage::Root
            | SettingsPage::Playback
            | SettingsPage::AudioSubtitles
            | SettingsPage::Favourites
            | SettingsPage::Legal
            | SettingsPage::About => 0,
            SettingsPage::Document(i) => *i,
        });
    }
    fn probe(&self, out: &mut String) {
        out.push_str(match self {
            SettingsPage::Root => "root",
            SettingsPage::Playback => "playback",
            SettingsPage::AudioSubtitles => "audio-subtitles",
            SettingsPage::Favourites => "favourites",
            SettingsPage::Legal => "legal",
            SettingsPage::About => "about",
            SettingsPage::Document(_) => "document",
        });
        if let SettingsPage::Document(i) = self {
            out.push_str(&format!("[{i}]"));
        }
    }
}

/// The inner host: the same bundle, the family's own argument, and the retained Browse directory
/// its Onboard child reads. It has no initial conditions of its own (the surface's `LogicalState`
/// covers its pages).
pub(crate) struct InnerHost;

#[derive(Default)]
pub(crate) struct NoInit;

impl LogicalState for NoInit {
    fn write(&self, _w: &mut Canon) {}
    fn probe(&self, _out: &mut String) {}
}

impl Host for InnerHost {
    type Arg = SettingsPage;
    type Fx = AppFx;
    type Msg = AppMsg;
    type Elem = u32;
    type Views<'a> = crate::stores::browse::DirectoryView<'a>;
    type Init = NoInit;
    // No family page remembers anything on its own `ReturnState` today (`ui/machine.rs`'s
    // `Host::Memory` doc) — the surface's own `NavStack<InnerHost>` restores its child pages by
    // focus alone.
    type Memory = ();
}

impl DirectoryLike for InnerHost {
    fn directory<'a>(cx: &Cx<'a, Self>) -> crate::stores::browse::DirectoryView<'a> {
        cx.views
    }
}

/// The outer context as the inner pages see it, carrying the same retained Browse directory the
/// outer host published for this frame.
/// The output lifetime is FREE (`'o`, bounded by the measure's): `Cx` is invariant in its
/// lifetime through the `Views` projection, so a caller must be able to shape one to a local
/// borrow to hand it to a `DrawFrame`.
pub(crate) fn inner_cx<'o, 'a: 'o, H: DirectoryLike>(cx: &Cx<'a, H>) -> Cx<'o, InnerHost> {
    Cx {
        views: H::directory(cx),
        tick: cx.tick,
        measure: cx.measure,
        press: cx.press,
        focus: cx.focus.clone(),
        owner: cx.owner,
    }
}

/// Seat a table on the engine's focus: a row key parks the selection on that row and lights the
/// list; a key elsewhere (the band, the alert) dims it. Every page in the family answers
/// `FocusMoved` with this, so the drawn selection and the engine never disagree.
pub(crate) fn table_focus(table: &mut TableView, elem: u32) {
    if elem < super::registry::BAND && (elem as i32) < table.n_rows() {
        table.sel = elem as i32;
        table.list_focused = true;
    } else {
        table.list_focused = false;
    }
}

/// The band's group in every page of the family; the table is `GroupId(0)`, an alert `GroupId(2)`.
pub(crate) const TABLE_GROUP: GroupId = GroupId(0);
pub(crate) const BAND_GROUP: GroupId = GroupId(1);
pub(crate) const ALERT_GROUP: GroupId = GroupId(2);

use crate::ui::machine::GroupId;

thread_local! {
    /// The surface's ground palette, published for the frame so the pages' controls are keyed to
    /// the ground they sit on (what `settings::control_palette` answered). RENDER state: set by
    /// the surface at draw, read by the pages' draws, never hashed.
    static PALETTE: std::cell::Cell<Option<ControlPalette>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn set_palette(p: ControlPalette) {
    PALETTE.with(|c| c.set(Some(p)));
}

pub(crate) fn palette() -> ControlPalette {
    PALETTE.with(|c| c.get()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    // Imported by its ABSOLUTE path, and that is the whole point. The module body above reaches
    // the registry as `super::registry`, because there `super` is `crate::screens` — but inside
    // this nested `mod tests` the same spelling means `family::registry`, which does not exist.
    // Copying a working path down one module level is how that breaks, and no gate but the test
    // build can see it, so name the module once here and let every test below say `registry::`.

}
