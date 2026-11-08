//! **The pre-playback window's three-dot menu** (issue #369): the actions that belong to the item
//! the window is about, behind the one control at the head of its options column.
//!
//! A `Style::Compact` surface on the shared `ModalStack`, anchored under the control that opened it,
//! exactly as `screens::item_menu` and `screens::alt_sources` are: a `TableView` on the panel
//! ground, rows of `[leading glyph] [label]`, the focused row a filled pill. The window stays on
//! screen behind it, the menu is about the item the window describes, and closing the window to
//! ask about the item would be the wrong way round.
//!
//! **What is here, and what is deliberately not.** Two entries, and the absence of the other two is
//! a decision rather than an omission (they are the four the issue names):
//!
//! * **Media information** opens `ContentPanel::Tracks`, the read-only Track-information sheet the
//!   detail page already presents. It is the entry that proves the menu, because it needs no write.
//! * **Identify** opens `ContentPanel::Identify`, the provider-match list. Jellyfin's lookup routes
//!   cover `Movie` and not `Episode`, so the row exists **only for a movie**, see [`identify_offered`].
//! * **Edit images** and **Edit subtitles** are NOT here. Both are uploads: they take a file the
//!   viewer chooses, and a webOS television has no file picker this app can reach, so there is no
//!   source for the bytes. An entry that cannot work is a lie the viewer discovers by pressing it,
//!   so the row is not drawn at all.
//!
//! Like every other menu here it only REPORTS: it names the surface it wants on its own effects and
//! lets the loop present it. It mutates no server state and no page.
//!
//! ## Why the rows come from `metadata::current()`
//!
//! The window that opens this menu is presented over the item's own page and describes
//! `metadata::current()` (`screens::preplay_panel`'s module doc has the full argument). The menu is
//! one more surface on that same stack, so the loaded item IS the item the menu is about, the fact
//! `screens::tracks_panel` and `screens::about_panel` already rely on. The argument therefore
//! carries only where it hangs.

use std::borrow::Cow;

use crate::metadata::Detail;
use crate::screens::registry::{AppFx, AppLike, ContentPanel, ContentReq};
use crate::ui::frame::Budget;
use crate::ui::icons::Icon;
use crate::ui::machine::{
    Canon, Cx, Edge, Effects, EntryId, FocusKey, Fx, GroupId, Handled, InputKind, Key,
    LogicalState, Machine, NavOp,
};
use crate::ui::screen::{
    Activate, At, AxisMask, Dir, DrawFrame, EdgeRule, ElemKind, Focusable, GroupKind, GroupSpec,
    Hover, Placed, RenderStrategy, Screen, ScreenEvent, Scrim, Seat, Step, Stop,
};
use crate::ui::table::{Row, Section, TableView};
use crate::ui::{theme, Rect};

/// The panel's width. Narrow on purpose: two rows of a label and a glyph, and the window behind it
/// is what carries the item's facts.
const PANEL_W: f32 = 460.0;
/// The pinned ~20px corner radius, the item menu's and the alt-sources panel's.
const PANEL_RAD: f32 = 20.0;
/// How far the panel rises into place, the one number the whole panel family shares. The
/// container's appear spring drives it (`DrawFrame::page_alpha` IS the surface's spring).
const RISE: f32 = crate::ui::popover::Popover::RISE;

/// The fields [`PrePlaybackMenuScreen::write`] canonicalises, for the recorder's shape pin (§5.4).
/// The selected row is in it for `AltSourcesScreen::SHAPE`'s reason: UP/DOWN here moves nothing else
/// in the app, so without it a replay grades the menu opening and closing with a hole between.
pub(crate) const SHAPE: [&str; 2] = [
    "PrePlaybackMenuScreen{arg:PrePlaybackMenuArg{host:u32,anchor:[u32;4]},rows:[u32],sel:i32,table:TableViewMotion}",
    TableView::MOTION_SHAPE,
];

/// What the container is asked to present.
///
/// `host` is the pre-playback entry the menu hangs off; it rides on the argument rather than being
/// looked up, for `AltSourcesArg`'s reason, the entry outlives any one frame's idea of what is on
/// top. `anchor` is the three-dot control's rest rectangle, bit-preserving so a canonical argument
/// needs no float equality.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct PrePlaybackMenuArg {
    pub(crate) host: crate::ui::machine::InstanceId,
    pub(crate) anchor: [u32; 4],
}

impl LogicalState for PrePlaybackMenuArg {
    fn write(&self, c: &mut Canon) {
        c.u32(self.host.0);
        for value in self.anchor {
            c.u32(value);
        }
    }
    fn probe(&self, out: &mut String) {
        out.push_str("preplay_menu_arg");
    }
}

/// What a row does on OK. Captured when the rows are built, so a landing under an open menu cannot
/// re-point a press at another action.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    /// *Media information*, the read-only Track-information sheet.
    MediaInfo,
    /// *Identify*, the metadata providers' candidate list.
    Identify,
}

/// **Does this item offer Identify?** Only a MOVIE does, and that is the server's shape rather than
/// a choice of this client: Jellyfin's remote-search routes exist for `Movie`, `Series`, `BoxSet`,
/// `MusicAlbum`, `MusicArtist`, `MusicVideo`, `Person`, `Trailer` and `Book`, and for no other leaf.
/// An EPISODE, the other leaf this window can stand on, has no lookup route of its own, and its
/// match is its series': replacing that from an episode's window would be a write on a different
/// item than the one on screen. So the row is absent rather than present-and-failing.
pub(crate) fn identify_offered(d: &Detail) -> bool {
    d.kind == "movie"
}

/// The rows this item offers, in order. PURE, so the "an episode offers no Identify" rule is graded
/// without mounting a surface.
pub(crate) fn rows_for(d: &Detail) -> Vec<Action> {
    let mut rows = vec![Action::MediaInfo];
    if identify_offered(d) {
        rows.push(Action::Identify);
    }
    rows
}

/// The glyph a row leads with.
fn icon_for(action: Action) -> Icon {
    match action {
        Action::MediaInfo => Icon::Info,
        Action::Identify => Icon::Search,
    }
}

/// The words a row says.
fn label_for(action: Action) -> &'static str {
    match action {
        Action::MediaInfo => crate::i18n::msg::widgets_preplay_menu_media_info(),
        Action::Identify => crate::i18n::msg::widgets_preplay_menu_identify(),
    }
}

/// Under the control when there is room, above it when there is not, never over it, so the control
/// that opened the panel stays readable beside its own list. Right-aligned with the control: this
/// menu hangs off a corner control, which is `widgets::anchored_menu`'s right-aligned case.
pub(crate) fn panel_at(a: Rect, content_h: f32) -> Rect {
    crate::ui::widgets::anchored_menu(a, PANEL_W, content_h, true)
}

pub(crate) struct PrePlaybackMenuScreen {
    entry: EntryId,
    arg: PrePlaybackMenuArg,
    /// The actions the table was built from, the rebuild stamp, and the index→action map.
    actions: Vec<Action>,
    pub(crate) table: TableView,
}

impl PrePlaybackMenuScreen {
    pub(crate) fn new(entry: EntryId, arg: PrePlaybackMenuArg, meta: crate::metadata::MetadataView<'_>) -> Self {
        let mut screen = Self {
            entry,
            arg,
            actions: Vec::new(),
            table: TableView::new(),
        };
        screen.rebuild(meta);
        screen
    }

    /// Materialise the item's rows. The cursor is kept when the row set did not move, and re-seated
    /// on the first row when it did (a landing that turned out to be an episode drops Identify).
    fn rebuild(&mut self, meta: crate::metadata::MetadataView<'_>) {
        let next = meta.current().map(rows_for).unwrap_or_default();
        if next == self.actions && !self.actions.is_empty() {
            return;
        }
        let keep = self.table.sel;
        self.actions = next;
        let mut sec = Section::new("");
        for action in &self.actions {
            // Both rows open a sheet, so both carry the drill-in chevron rather than inventing a
            // trailing glyph per action; the LEADING glyph is what says which action it is.
            sec = sec.row(
                Row::new(label_for(*action))
                    .licon(icon_for(*action))
                    .ticon(Icon::Chevron),
            );
        }
        self.table.compact = true;
        let sel = keep.clamp(0, (self.actions.len() as i32 - 1).max(0));
        self.table.set_sections(vec![sec], sel, false);
    }

    pub(crate) fn frame(&self) -> Rect {
        let [x, y, w, h] = self.arg.anchor.map(f32::from_bits);
        panel_at(Rect::new(x, y, w, h), self.table.measured_height())
    }

    /// **The rows as pointer stops** (rule 11: hover parks focus, a click activates). Split out of
    /// `draw` for `PlayerScreen::record_stops`'s reason: a host unit build links no GL and no
    /// SDL2_ttf, so the suite can grade these without the paint pass that would issue both. They
    /// ride the SETTLED geometry, like every other surface's: at rest (`appear == 1`) the drawn and
    /// settled rects coincide exactly.
    pub(crate) fn record_stops<H: AppLike<Memory = crate::screens::registry::PageMemory>>(
        &self,
        f: &mut DrawFrame<'_, '_, H>,
    ) {
        let hit_p = f.painter.alpha(f.page_alpha);
        for elem in 0..self.actions.len() as u32 {
            if let Some(placed) = <Self as Focusable<H>>::place(self, &elem, f.cx, At::Drawn) {
                f.stop(
                    hit_p,
                    Stop {
                        key: FocusKey {
                            entry: self.entry,
                            elem,
                        },
                        rect: placed.rect,
                        rest_rect: placed.rest_rect,
                        clip: placed.clip,
                        // The row lights in the SAME face the arrows give it: the pointer parks
                        // focus through the engine, so there is no second hover style here.
                        hover: Hover::Focus,
                        activate: Activate::Immediate,
                    },
                );
            }
        }
    }

    fn dismiss<H: AppLike>(&self, fx: &mut Effects<'_, H>) {
        fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
    }

    /// Commit row `elem` and close. The surface it names is presented by the LOOP, over this
    /// window's own page, and the dismiss above is what keeps the menu from staying up under it.
    fn commit<H: AppLike>(&mut self, elem: u32, fx: &mut Effects<'_, H>) {
        let Some(action) = self.actions.get(elem as usize).copied() else {
            return;
        };
        let panel = match action {
            Action::MediaInfo => ContentPanel::Tracks { page: 1 },
            Action::Identify => ContentPanel::Identify {
                anchor: self.arg.anchor,
            },
        };
        self.dismiss(fx);
        fx.push(Fx::App(AppFx::Content(ContentReq::Panel(panel))));
    }
}

impl<H: AppLike<Memory = crate::screens::registry::PageMemory> + crate::screens::registry::MetadataLike>
    Machine<H> for PrePlaybackMenuScreen
{
    type Ev = ScreenEvent<H>;
    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        match ev {
            ScreenEvent::Mount | ScreenEvent::Enter(_) => {
                self.rebuild(H::metadata(cx));
                Handled::Yes
            }
            ScreenEvent::StoreChanged(ord, _) => {
                if *ord == crate::stores::StoreId::Metadata.ord() {
                    self.rebuild(H::metadata(cx));
                }
                Handled::Yes
            }
            ScreenEvent::Tick(tick) => {
                self.table.sel = cx
                    .focus
                    .current
                    .filter(|key| key.entry == self.entry)
                    .map(|key| key.elem as i32)
                    .unwrap_or(self.table.sel);
                self.table.update(tick.dt(), self.frame().h);
                Handled::Yes
            }
            ScreenEvent::FocusMoved { to, .. } => {
                self.table.sel = to.elem as i32;
                fx.invalidate(crate::ui::present::Provenance::Input);
                Handled::Yes
            }
            ScreenEvent::Activate(elem) => {
                self.commit(*elem, fx);
                Handled::Yes
            }
            ScreenEvent::PressCommit(_) => {
                if let Some(key) = cx.focus.current {
                    self.commit(key.elem, fx);
                }
                Handled::Yes
            }
            ScreenEvent::Input(input) => match input.kind {
                InputKind::Key {
                    key: Key::Back,
                    edge: Edge::Down,
                    ..
                } => {
                    self.dismiss(fx);
                    Handled::Yes
                }
                _ => Handled::No,
            },
            _ => Handled::No,
        }
    }
}

/// A single-column `Focusable` over the table's own cursor, `AltSourcesScreen`'s shape: one
/// `GroupKind::Column` of `Bare` elements (OK activates on the down edge, there is nothing to
/// hold), `EdgeRule::Stop` on every side, and `place`/`neighbour` read straight off `TableView`.
impl<H: AppLike<Memory = crate::screens::registry::PageMemory>> Focusable<H> for PrePlaybackMenuScreen {
    fn groups(&self, _cx: &Cx<'_, H>, out: &mut Vec<GroupSpec>) {
        out.push(GroupSpec {
            id: GroupId(0),
            kind: GroupKind::Column,
            seat: Seat::Remembered,
            reachable: AxisMask::BOTH,
            edge: [EdgeRule::Stop; 4],
            extent: self.frame(),
            len: self.actions.len(),
            elem: ElemKind::Bare,
        });
    }
    fn group_of(&self, elem: &u32, _cx: &Cx<'_, H>) -> Option<GroupId> {
        ((*elem as usize) < self.actions.len()).then_some(GroupId(0))
    }
    fn neighbour(&self, key: FocusKey<u32>, dir: Dir, _cx: &Cx<'_, H>) -> Step<u32> {
        let next = match dir {
            Dir::Up => self.table.next_selectable(key.elem as i32, -1),
            Dir::Down => self.table.next_selectable(key.elem as i32, 1),
            _ => None,
        };
        match next {
            Some(i) => Step::Move(FocusKey {
                entry: self.entry,
                elem: i as u32,
            }),
            None => Step::Edge,
        }
    }
    fn place(&self, elem: &u32, _cx: &Cx<'_, H>, _at: At) -> Option<Placed> {
        let rect = self.table.row_frame(self.frame(), *elem as i32)?;
        Some(Placed {
            rect,
            rest_rect: rect,
            clip: self.frame(),
            index: Some(*elem),
        })
    }
    fn reconcile(&self, want: FocusKey<u32>, cx: &Cx<'_, H>) -> FocusKey<u32> {
        if self.group_of(&want.elem, cx).is_some() {
            want
        } else {
            FocusKey {
                entry: self.entry,
                elem: 0,
            }
        }
    }
    fn seat(&self, _g: GroupId, _from: Placed, _cx: &Cx<'_, H>) -> FocusKey<u32> {
        FocusKey {
            entry: self.entry,
            elem: self.table.sel.max(0) as u32,
        }
    }
}

impl LogicalState for PrePlaybackMenuScreen {
    fn write(&self, c: &mut Canon) {
        self.arg.write(c);
        c.seq(self.actions.len());
        for action in &self.actions {
            c.u32(match action {
                Action::MediaInfo => 0,
                Action::Identify => 1,
            });
        }
        c.u32(self.table.sel as u32);
        self.table.write_motion(c);
    }
    fn probe(&self, out: &mut String) {
        out.push_str("preplay_menu");
    }
}

impl<H: AppLike<Memory = crate::screens::registry::PageMemory> + crate::screens::registry::MetadataLike> Screen<H>
    for PrePlaybackMenuScreen
{
    fn name(&self) -> &'static str {
        "preplay_menu"
    }
    fn state(&self) -> &dyn LogicalState {
        self
    }
    fn crumb(&self, _cx: &Cx<'_, H>) -> Option<Cow<'_, str>> {
        None
    }
    fn prepare(&mut self, _b: &mut Budget, _cx: &Cx<'_, H>) {}
    /// The modal dim, asked for rather than drawn, `AltSourcesScreen::scrim`'s contract: the
    /// window behind recedes and stays readable, because the menu is about the item it describes.
    fn scrim(&self) -> Scrim {
        Scrim::dim(theme::underlay::DIM_PANEL)
    }
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, H>) {
        let appear = f.page_alpha;
        let r = self.frame();
        let p = f.painter.alpha(appear).translate(0.0, RISE * (1.0 - appear));
        let measure = f.measure;
        let field = f.underlay;
        crate::ui::profile::phase("dt.preplay_menu", || {
            crate::ui::widgets::panel_ground(p, r, PANEL_RAD, field);
            self.table.draw(p, r, measure);
        });
        self.record_stops(f);
    }
    fn render(&self) -> RenderStrategy {
        RenderStrategy::Page
    }
    fn focus_source(&self) -> crate::ui::screen::FocusSource {
        crate::ui::screen::FocusSource::Engine
    }
    fn hit_source(&self) -> crate::ui::screen::HitSource {
        crate::ui::screen::HitSource::Engine
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie() -> Detail {
        Detail {
            kind: "movie".into(),
            ..Default::default()
        }
    }

    // ---- the pointer (#359) --------------------------------------------------------------------

    struct HostFixture;
    impl crate::ui::machine::Host for HostFixture {
        type Arg = crate::ui::fixture::FixtureArg;
        type Fx = AppFx;
        type Msg = crate::screens::registry::AppMsg;
        type Elem = u32;
        type Views<'a> = ();
        type Init = crate::ui::fixture::FixtureArg;
        type Memory = crate::screens::registry::PageMemory;
    }

    thread_local! {
        // TEST ONLY, `screens::detail::tests`' pattern: the store has to live somewhere for the
        // free helpers below, and a per-owner one would be a second mechanism for reaching it.
        static TEST_METADATA: std::cell::UnsafeCell<crate::stores::metadata::MetadataStore> =
            std::cell::UnsafeCell::new(crate::stores::metadata::MetadataStore::default());
    }

    fn test_store() -> &'static mut crate::stores::metadata::MetadataStore {
        TEST_METADATA.with(|cell| unsafe { &mut *cell.get() })
    }

    impl crate::screens::registry::MetadataLike for HostFixture {
        fn metadata<'a>(_cx: &Cx<'a, Self>) -> crate::metadata::MetadataView<'a> {
            test_store().view()
        }
    }

    const ENTRY: EntryId = EntryId(5);

    fn cx(focus: Option<u32>) -> Cx<'static, HostFixture> {
        Cx {
            views: (),
            tick: crate::ui::machine::Tick::default(),
            measure: &crate::ui::fixture::FixtureMeasure,
            press: crate::ui::machine::PressRead::default(),
            focus: crate::ui::machine::FocusRead {
                current: focus.map(|elem| FocusKey { entry: ENTRY, elem }),
                ..Default::default()
            },
            owner: crate::ui::machine::InputOwner::Entry(ENTRY),
        }
    }

    /// A mounted menu on a MOVIE, which is the item that offers both rows.
    fn menu() -> (PrePlaybackMenuScreen, crate::testlock::Serial) {
        let guard = crate::testlock::serial();
        crate::metadata::set_current_for_test(test_store().state_mut(), Some(movie()));
        let context = cx(None);
        let arg = PrePlaybackMenuArg { host: crate::ui::machine::InstanceId(1), anchor: [1400.0f32.to_bits(), 120.0f32.to_bits(), 96.0f32.to_bits(), 56.0f32.to_bits()] };
        (PrePlaybackMenuScreen::new(
            ENTRY,
            arg,
            <HostFixture as crate::screens::registry::MetadataLike>::metadata(&context),
        ), guard)
    }

    /// **Rule 11 on the three-dot menu** (#359): both rows are stops that HOVER like every other
    /// stop in the app, and the row a hover parks is the row the pill is drawn on, because the
    /// screen takes its selection from the engine's `FocusMoved` rather than keeping its own.
    #[test]
    fn a_hover_parks_the_row_and_the_pill_follows_it() {
        use crate::ui::hit::{HitMap, PointerKind};
        use crate::ui::screen::DrawFrame;

        let (mut screen, _guard) = menu();
        let context = cx(None);
        let mut f = DrawFrame::new(&context, crate::ui::Painter::root());
        screen.record_stops(&mut f);
        let stops = f.into_stops();
        assert_eq!(stops.len(), 2, "the movie's two rows");
        for stop in &stops {
            assert!(matches!(stop.hover, Hover::Focus), "a row hovers: the pointer parks focus");
            assert!(matches!(stop.activate, Activate::Immediate), "a click opens the row at once");
        }
        let mut map: HitMap<u32> = HitMap::new();
        map.fill(stops);
        map.swap();
        let second = screen.table.row_frame(screen.frame(), 1).expect("row 1 is drawn");
        assert_eq!(
            map.resolve(Some(ENTRY), PointerKind::Move, second.cx(), second.cy(), None).focus.map(|k| k.elem),
            Some(1),
            "hovering row 1 parks row 1"
        );
        // …and the pill lands on the row the pointer parked: the drawn face IS the focus face.
        let event = ScreenEvent::FocusMoved {
            from: Some(FocusKey { entry: ENTRY, elem: 0 }),
            to: FocusKey { entry: ENTRY, elem: 1 },
            by: crate::ui::screen::By::Pointer,
        };
        let mut effects = Vec::new();
        let mut present = crate::ui::present::Present::new();
        let handled = {
            let mut sink = Effects::new(&mut effects, crate::ui::machine::MachineId::Input, &mut present);
            screen.step(&event, &context, &mut sink)
        };
        assert_eq!(handled, Handled::Yes);
        assert_eq!(screen.table.sel, 1, "the hovered row is the row drawn lit");
    }

    /// The four entries the issue names, and the two that are absent on purpose: an entry that
    /// cannot work is not drawn, so the row set is a statement rather than a wish list.
    #[test]
    fn the_menu_offers_the_entries_that_can_work_and_no_others() {
        assert_eq!(rows_for(&movie()), vec![Action::MediaInfo, Action::Identify]);
        // Media information is always there: it reads the item's own file.
        let episode = Detail {
            kind: "episode".into(),
            ..movie()
        };
        assert_eq!(rows_for(&episode), vec![Action::MediaInfo]);
        assert!(!identify_offered(&episode), "Jellyfin has no episode lookup route");
        assert!(identify_offered(&movie()));
    }

    /// The panel hangs UNDER its control and never over it, is right-aligned with it, and stays
    /// inside the screen's keep-out, the placement rules, graded without a surface.
    #[test]
    fn the_panel_hangs_under_its_control_inside_the_keep_out() {
        use crate::ui::consts::{SCR_H, SCR_W};
        let control = Rect::new(1400.0, 120.0, 96.0, 56.0);
        let r = panel_at(control, 200.0);
        assert!(r.y >= control.y + control.h, "never over the control");
        assert!((r.x + r.w - (control.x + control.w)).abs() < 0.01, "right edges flush");
        assert!(r.x >= crate::ui::consts::MARGIN_X && r.x + r.w <= SCR_W - crate::ui::consts::MARGIN_X);
        // A control low on the screen flips the panel above itself rather than running off.
        let low = Rect::new(1400.0, SCR_H - 120.0, 96.0, 56.0);
        let r = panel_at(low, 200.0);
        assert!(r.y + r.h <= low.y, "above the control when there is no room below");
        assert!(r.y >= theme::space::XL);
        // …and a control at the LEFT edge cannot push the panel off the left side either.
        let left = Rect::new(0.0, 120.0, 96.0, 56.0);
        assert!(panel_at(left, 200.0).x >= crate::ui::consts::MARGIN_X);
    }
}
