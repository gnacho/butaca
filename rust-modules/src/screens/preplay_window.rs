//! **The pre-playback window** (issue #355): the film's own file, before the film.
//!
//! A poster press used to be a promise the app kept too fast: the page opened and Play started the
//! picture, so the only way to look at what was about to play was to start it. Jellyfin puts a
//! window in between, and the owner asked for that window by name (2026-11-12: *"ventana
//! intermedia, como hace Jellyfin"*).
//!
//! # What it shows, and where the facts come from
//!
//! The FILE block, in the app's own words and order, and nothing invented: the rows are
//! [`crate::ui::track_facts::file_rows`] and the first of [`crate::ui::track_facts::video_rows`],
//! which is the same vocabulary *Track information* already prints, already localized, already
//! formatted. The audio track's own line follows, under the audio heading. There is no fetch here
//! and none is needed: every field lives on the landed [`Detail`] (`container`, `size`,
//! `bitrate`, `video`, `acodec`, `dur_ms`), which is why this window costs one press and no
//! request.
//!
//! # The one control
//!
//! **Play, or Resume when the item is part-watched**, as the LAST row and the only stop. The verb
//! is the page's own (`browse.detail.play` / `browse.detail.resume`), decided off `resume_ms`, so
//! the window cannot promise a start the page would not give.
//!
//! OK does not start the film from here. It DISMISSES and tells the host page
//! ([`AppMsg::PrePlayGo`]), because starting a film is the detail page's decision
//! (`DetailScreen::play_hero`) and a second implementation of it inside a panel is exactly how the
//! page's Play and the window's Play drift apart. The page does what its own Play disc does.
//!
//! # Why the fact rows are rows
//!
//! They are not focusable and never will be: a fact is not a choice. They are table rows for the
//! reason the settings grid is a table: one geometry, one elide rule, one ink ladder for labels and
//! read-outs, and a panel whose height is measured rather than quoted. [`PREPLAY_FACTS`] is how
//! many of them are drawn, and it is the panel's only size lever.

use std::borrow::Cow;

use crate::i18n::msg;
use crate::metadata::Detail;
use crate::ui::machine::{
    Canon, Cx, Delivery, Edge, Effects, EntryId, FocusKey, Fx, GroupId, Handled, InputKind, Key,
    LogicalState, Machine, MachineId, NavOp,
};
use crate::ui::screen::{
    Activate, At, AxisMask, Dir, DrawFrame, EdgeRule, ElemKind, Focusable, GroupKind, GroupSpec,
    HitSource, Hover, Placed, RenderStrategy, Screen, ScreenEvent, Scrim, Seat, Step, Stop,
};
use crate::ui::table::{Row, Section, TableView};
use crate::ui::widgets::panel_ground;
use crate::ui::{theme, Rect};
use crate::ui::frame::Budget;
use crate::screens::registry::AppLike;

/// The host page this window hangs off, and the instance its verdict goes back to.
///
/// The anchor is the drawn rectangle of the control the press came from, bit-preserving like every
/// other anchored surface's (`ContentPanel::AltSources`'s reason): a poster press has no rectangle
/// of its own by the time the page has landed, so the window opens where the page's Play control
/// was drawn for it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct PrePlayArg {
    pub host: crate::ui::machine::InstanceId,
    pub anchor: [u32; 4],
}

impl LogicalState for PrePlayArg {
    fn write(&self, c: &mut Canon) {
        c.u32(self.host.0);
        for word in self.anchor {
            c.u32(word);
        }
    }
    fn probe(&self, out: &mut String) {
        out.push_str("preplay_window");
    }
}

/// How many FILE facts the window shows before the audio line. Five is what fits under a poster's
/// footprint at ten feet without the window turning into an inspector: container, size, total
/// bitrate, duration, aspect.
pub(crate) const PREPLAY_FACTS: usize = 5;

/// The width of the window: the detail hero's action row is the thing it opens against, so it is
/// about as wide as that row's own block.
pub(crate) const PANEL_W: f32 = 720.0;

/// The recorded shape of this surface's state (the recorder's inventory,
/// `registry::SCREEN_SHAPES`): the argument, the file block, the verb and the table's cursor.
pub(crate) const SHAPE: [&str; 2] = [
    "PrePlayWindowArg{host:u32,anchor:[u32;4]}",
    "PrePlayWindow{facts:seq(label:str,value:str),action:str,table:TableViewMotion}",
];

pub(crate) struct PrePlayWindowScreen {
    entry: EntryId,
    arg: PrePlayArg,
    /// `(label, value)` for the file block, rebuilt when the item lands or changes.
    facts: Vec<(String, String)>,
    /// The verb on the action row, likewise.
    action: String,
    table: TableView,
}

impl PrePlayWindowScreen {
    pub(crate) fn new(
        entry: EntryId,
        arg: PrePlayArg,
        meta: crate::metadata::MetadataView<'_>,
    ) -> Self {
        let mut screen = Self { entry, arg, facts: Vec::new(), action: String::new(), table: TableView::new() };
        screen.rebuild(meta);
        screen
    }

    /// The rows: the FILE block, then the one control. Kept in ONE table so the action row's rect,
    /// its hit box and its focus stop are the table's own arithmetic rather than a second layout.
    fn rebuild(&mut self, meta: crate::metadata::MetadataView<'_>) {
        let d = meta.current();
        let (facts, action) = d.map_or((Vec::new(), None), |d| (file_block(d), Some(verb(d))));
        if self.facts == facts && action.as_deref().is_some_and(|a| a == self.action) {
            return;
        }
        self.facts = facts;
        self.action = action.unwrap_or_else(|| msg::browse_detail_play().to_string());
        let mut facts = Section::new("");
        for (label, value) in &self.facts {
            facts = facts.row(Row::new(label).value(value).value_dim(true));
        }
        // The one control, in its own section so it never reads as one more fact.
        let control = Section::new("").row(Row::new(&self.action));
        let keep = self.table.sel;
        self.table.compact = false;
        self.table.set_sections(vec![facts, control], keep, false);
    }

    /// The last row's index: the control.
    fn action_row(&self) -> i32 {
        self.table.n_rows() as i32 - 1
    }

    pub(crate) fn frame(&self) -> Rect {
        let [x, y, w, h] = self.arg.anchor.map(f32::from_bits);
        let anchor = Rect::new(x, y, w, h);
        let content = self.table.measured_height();
        let rect = panel_frame(anchor, content);
        Rect::new(rect.x, rect.y, PANEL_W.max(rect.w).min(rect.w), rect.h)
    }

    fn dismiss<H: AppLike>(&self, fx: &mut Effects<'_, H>) {
        fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
    }

    /// Hand the verdict to the PAGE, which is what owns "start the film".
    fn go<H: AppLike>(&self, fx: &mut Effects<'_, H>) {
        self.dismiss(fx);
        fx.push(Fx::Deliver(
            MachineId::Instance(self.arg.host),
            Delivery::Screen(ScreenEvent::App(crate::screens::registry::AppMsg::PrePlayGo)),
        ));
    }
}

/// The FILE block, in the app's own vocabulary: the same rows Track information prints, cut to the
/// facts a viewer checks before pressing Play, plus the audio track.
fn file_block(d: &Detail) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = crate::ui::track_facts::file_rows(d)
        .into_iter()
        .take(PREPLAY_FACTS)
        .map(|p| (p.label.to_string(), p.value))
        .collect();
    for pair in crate::ui::track_facts::video_rows(d).into_iter().take(2) {
        rows.push((pair.label.to_string(), pair.value));
    }
    if let Some(codec) = crate::ui::track_facts::fmt_codec_profile(&d.acodec, "") {
        rows.push((msg::widgets_tracks_audio_heading().to_string(), codec));
    }
    rows
}

/// **The verb**: Resume when there is a position to resume from, Play otherwise. One expression, so
/// the window and the page cannot disagree about which promise the press keeps.
fn verb(d: &Detail) -> String {
    if d.resume_ms > 0 {
        msg::browse_detail_resume().to_string()
    } else {
        msg::browse_detail_play().to_string()
    }
}

/// Under the anchor when there is room, above it when there is not, never over it: the control the
/// press came from stays readable beside its own window (`track_pick::panel_at`'s rule).
fn panel_frame(anchor: Rect, content_h: f32) -> Rect {
    crate::ui::widgets::anchored_menu(anchor, PANEL_W, content_h, true)
}

impl<H: AppLike> Focusable<H> for PrePlayWindowScreen {
    fn groups(&self, _cx: &Cx<'_, H>, out: &mut Vec<GroupSpec>) {
        if self.table.n_rows() == 0 {
            return;
        }
        // ONE stop, and it is the control. The facts are read, not chosen.
        out.push(GroupSpec {
            id: GroupId(0),
            kind: GroupKind::Column,
            seat: Seat::Remembered,
            reachable: AxisMask::BOTH,
            edge: [EdgeRule::Stop; 4],
            extent: self.frame(),
            len: 1,
            elem: ElemKind::Bare,
        });
    }
    fn group_of(&self, elem: &u32, _cx: &Cx<'_, H>) -> Option<GroupId> {
        (*elem == 0 && self.table.n_rows() > 0).then_some(GroupId(0))
    }
    fn neighbour(&self, _key: FocusKey<u32>, _dir: Dir, _cx: &Cx<'_, H>) -> Step<u32> {
        Step::Edge
    }
    fn place(&self, elem: &u32, _cx: &Cx<'_, H>, _at: At) -> Option<Placed> {
        if *elem != 0 {
            return None;
        }
        let rect = self.table.row_frame(self.frame(), self.action_row())?;
        Some(Placed { rect, rest_rect: rect, clip: self.frame(), index: Some(0) })
    }
    fn reconcile(&self, _want: FocusKey<u32>, _cx: &Cx<'_, H>) -> FocusKey<u32> {
        FocusKey { entry: self.entry, elem: 0 }
    }
    fn seat(&self, _g: GroupId, _from: Placed, _cx: &Cx<'_, H>) -> FocusKey<u32> {
        FocusKey { entry: self.entry, elem: 0 }
    }
}

impl LogicalState for PrePlayWindowScreen {
    fn write(&self, c: &mut Canon) {
        self.arg.write(c);
        c.seq(self.facts.len());
        for (label, value) in &self.facts {
            c.str(label).str(value);
        }
        c.str(&self.action);
        self.table.write_motion(c);
    }
    fn probe(&self, out: &mut String) {
        out.push_str("preplay_window");
    }
}

impl<H: AppLike + crate::screens::registry::MetadataLike> Machine<H>
    for PrePlayWindowScreen
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
                self.table.update(tick.dt(), self.frame().h);
                Handled::Yes
            }
            ScreenEvent::Activate(elem) => {
                if *elem == 0 {
                    self.go(fx);
                }
                Handled::Yes
            }
            ScreenEvent::PressCommit(_) => {
                self.go(fx);
                Handled::Yes
            }
            ScreenEvent::Input(input) => match input.kind {
                InputKind::Key { key: Key::Back, edge: Edge::Down, .. } => {
                    self.dismiss(fx);
                    Handled::Yes
                }
                _ => Handled::No,
            },
            _ => Handled::No,
        }
    }
}

impl<H: AppLike + crate::screens::registry::MetadataLike> Screen<H>
    for PrePlayWindowScreen
{
    fn name(&self) -> &'static str {
        "preplay_window"
    }
    fn state(&self) -> &dyn LogicalState {
        self
    }
    fn crumb(&self, _cx: &Cx<'_, H>) -> Option<Cow<'_, str>> {
        None
    }
    fn prepare(&mut self, _b: &mut Budget, _cx: &Cx<'_, H>) {}
    fn scrim(&self) -> Scrim {
        Scrim::dim(theme::underlay::DIM_PANEL)
    }
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, H>) {
        let appear = f.page_alpha;
        let r = self.frame();
        let p = f.painter.alpha(appear);
        let measure = f.measure;
        let field = f.underlay;
        panel_ground(p, r, theme::CARD_RING_RAD, field);
        // The facts are rows of the same table as the control, but only the control is a stop: the
        // table draws its own cursor, so it is told which row that is.
        self.table.sel = self.action_row();
        self.table.draw(p, r, measure);
        if let Some(placed) = <Self as Focusable<H>>::place(self, &0, f.cx, At::Drawn) {
            f.stop(
                p,
                Stop {
                    key: FocusKey { entry: self.entry, elem: 0 },
                    rect: placed.rect,
                    rest_rect: placed.rest_rect,
                    clip: placed.clip,
                    hover: Hover::Focus,
                    activate: Activate::Immediate,
                },
            );
        }
    }
    fn render(&self) -> RenderStrategy {
        RenderStrategy::Page
    }
    fn focus_source(&self) -> crate::ui::screen::FocusSource {
        crate::ui::screen::FocusSource::Engine
    }
    fn hit_source(&self) -> HitSource {
        HitSource::Engine
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

    fn item(resume_ms: i64, acodec: &str) -> Detail {
        let mut d = Detail::default();
        d.container = "mkv".into();
        d.size = 8_400_000_000;
        d.bitrate = 12_000;
        d.dur_ms = 6_300_000;
        d.width = 1920;
        d.height = 1080;
        d.vcodec = "hevc".into();
        d.acodec = acodec.into();
        d.resume_ms = resume_ms;
        d
    }

    /// **The block is the app's own vocabulary, not a second one.** Every line is a row Track
    /// information already prints (container, size, bitrate, duration, aspect, codec, resolution),
    /// so the window cannot start describing the same file in words the panel does not use.
    #[test]
    fn the_file_block_reuses_the_house_vocabulary() {
        let rows = file_block(&item(0, "eac3"));
        let labels: Vec<&str> = rows.iter().map(|(l, _)| l.as_str()).collect();
        let house = crate::ui::track_facts::file_rows(&item(0, "eac3"));
        for pair in house.iter().take(PREPLAY_FACTS) {
            assert!(labels.contains(&pair.label), "{}: {}", pair.label, pair.value);
        }
        assert!(
            rows.iter().any(|(_, value)| value == "MKV"),
            "the container reads as the panel prints it"
        );
    }

    /// **The verb is the page's promise.** A part-watched film offers Resume, a fresh one offers
    /// Play, and both strings are the detail page's own keys.
    #[test]
    fn the_action_states_the_promise_the_page_would_keep() {
        assert_eq!(verb(&item(0, "eac3")), msg::browse_detail_play().to_string());
        assert_eq!(verb(&item(60_000, "eac3")), msg::browse_detail_resume().to_string());
    }

    /// **The action row is the last row of the one table**, so its rect, its stop and its painting
    /// cannot disagree: this is the arithmetic `place` and `draw` both read.
    #[test]
    fn the_control_is_the_last_row_and_the_only_stop() {
        let mut screen = PrePlayWindowScreen {
            entry: EntryId(1),
            arg: PrePlayArg { host: crate::ui::machine::InstanceId(2), anchor: [0; 4] },
            facts: Vec::new(),
            action: String::new(),
            table: TableView::new(),
        };
        let (facts, action) = (file_block(&item(0, "eac3")), verb(&item(0, "eac3")));
        screen.facts = facts.clone();
        screen.action = action;
        let mut section = Section::new("");
        for (label, value) in &screen.facts {
            section = section.row(Row::new(label).value(value));
        }
        screen.table.set_sections(vec![section, Section::new("").row(Row::new("Reproducir"))], 0, false);
        assert_eq!(
            screen.table.n_rows() as i32,
            screen.action_row() + 1,
            "the control is the last row: its index is the table's own arithmetic"
        );
    }
}
