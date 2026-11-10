//! **The hero's track picker** — the menu the audio and subtitle discs open, so a language is
//! chosen BEFORE the picture starts (the owner's pass; it replaces the pre-playback window's
//! language card).
//!
//! A `Style::Compact` surface on the shared `ModalStack`, anchored under the disc that opened it,
//! exactly as `screens::alt_sources` and the item menus are: a `TableView` on the panel ground, a
//! checkmark on the track the play would start with, and the item's own languages with their
//! codecs — which is the information the About footer's Languages column used to print.
//!
//! **What it is NOT.** It does not start anything, seek anything or touch the playing session: it
//! reports ONE chosen stream id on its effects and lets the page keep it until its Play press
//! hands it to [`crate::route::StartTracks`]. The subject is `metadata::current()` — the item the
//! page that opened the menu is standing on, the same fact `screens::tracks_panel` relies on.
//!
//! ## Why the rows are built here and not read from a store
//!
//! A track list is part of the loaded `Detail` (`audio`/`subs`), not an addressable store, and it
//! cannot change while the page is mounted: the landing that would replace it raises
//! `StoreChanged(Metadata)`, which rebuilds these rows in place. The cursor is kept when the row
//! set did not move, exactly as the pre-playback menu's did.

use std::borrow::Cow;

use crate::metadata::Detail;
use crate::screens::registry::AppLike;
use crate::ui::frame::Budget;
use crate::ui::icons::Icon;
use crate::ui::machine::{
    Canon, Cx, Delivery, Edge, Effects, EntryId, FocusKey, Fx, GroupId, Handled, InputKind, Key,
    LogicalState, Machine, MachineId, NavOp,
};
use crate::ui::screen::{
    At, AxisMask, Dir, DrawFrame, EdgeRule, ElemKind, Focusable, GroupKind, GroupSpec, Hover,
    Placed, RenderStrategy, Screen, ScreenEvent, Scrim, Seat, Step, Stop,
};
use crate::ui::table::{Row, Section, TableView};
use crate::ui::{theme, Rect};

/// The panel's width. One column of language + codec lines.
const PANEL_W: f32 = 520.0;
/// The pinned ~20px corner radius, the item menu's and the alt-sources panel's.
const PANEL_RAD: f32 = 20.0;
/// How far the panel rises into place, the one number the whole panel family shares.
const RISE: f32 = crate::ui::popover::Popover::RISE;

/// The fields [`TrackPickScreen::write`] canonicalises, for the recorder's shape pin (§5.4).
pub(crate) const SHAPE: [&str; 2] = [
    "TrackPickScreen{arg:TrackPickArg{audio:u8,chosen:i64,anchor:[u32;4]},rows:[u32],sel:i32,table:TableViewMotion}",
    TableView::MOTION_SHAPE,
];

/// What the container is asked to present.
///
/// `host` is the PAGE the disc belongs to, and it rides on the argument rather than being looked
/// up, for `AltSourcesArg`'s reason: the surface outlives any one frame's idea of what is on top,
/// and the commit has to reach that page. `audio` names which list; `chosen` is the stream id the
/// page would play today (`0` for Off on the subtitle list, and `-1`/`0` for "the ladder's own
/// answer" on the audio one), so the checkmark is drawn from the same number the play would use.
/// `anchor` is the disc's rest rectangle, bit-preserving so a canonical argument needs no float
/// equality.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct TrackPickArg {
    pub(crate) host: crate::ui::machine::InstanceId,
    pub(crate) audio: bool,
    pub(crate) chosen: i64,
    pub(crate) anchor: [u32; 4],
}

impl LogicalState for TrackPickArg {
    fn write(&self, c: &mut Canon) {
        c.u32(self.host.0);
        c.u32(u32::from(self.audio));
        c.u64(self.chosen as u64);
        for value in self.anchor {
            c.u32(value);
        }
    }
    fn probe(&self, out: &mut String) {
        out.push_str("track_pick_arg");
    }
}

/// **What a committed row tells the page**: the stream id, on the half the list was about.
/// `None` on the other half is the point — a picker for audio never clears a subtitle the viewer
/// set from the other disc. Split out of [`TrackPickScreen::commit`] so the pair's shape is graded
/// without a host.
pub(crate) fn commit_message(audio: bool, sid: i64) -> crate::screens::registry::AppMsg {
    crate::screens::registry::AppMsg::DetailTracks {
        audio_sid: audio.then_some(sid),
        sub_sid: (!audio).then_some(sid),
    }
}

/// One drawn row: the stream id it commits, and whether it is the `Off` row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Choice {
    pub(crate) sid: i64,
    pub(crate) off: bool,
}

/// **The rows this item offers**, in order. PURE, so the "Off is always first on the subtitle
/// list" and "an item with one audio track offers that one" rules are graded without a surface.
pub(crate) fn choices(d: &Detail, audio: bool) -> Vec<Choice> {
    if audio {
        d.audio
            .iter()
            .map(|s| Choice { sid: s.id, off: false })
            .collect()
    } else {
        let mut rows = vec![Choice { sid: 0, off: true }];
        rows.extend(d.subs.iter().map(|s| Choice { sid: s.id, off: false }));
        rows
    }
}

/// A track's line: its language and its codec, plus the two flags a viewer chooses between.
/// The same facts the About footer's Languages column printed, which is where the owner moved
/// them from.
pub(crate) fn row_label(stream: &crate::metadata::Stream) -> String {
    let lang = if stream.lang.is_empty() {
        crate::i18n::msg::browse_detail_unknown().to_string()
    } else {
        crate::metadata::language_name(&stream.lang)
    };
    let mut text = if stream.codec.is_empty() {
        lang
    } else {
        format!("{lang} \u{b7} {}", crate::metadata::friendly_codec(&stream.codec))
    };
    for (on, tag) in [
        (stream.forced, crate::i18n::msg::widgets_tracks_forced()),
        (stream.sdh, crate::i18n::msg::widgets_badge_sdh()),
    ] {
        if on {
            text = format!("{text} \u{b7} {tag}");
        }
    }
    text
}

/// Under the disc when there is room, above it when there is not, never over it, so the control
/// that opened the panel stays readable beside its own list.
pub(crate) fn panel_at(a: Rect, content_h: f32) -> Rect {
    crate::ui::widgets::anchored_menu(a, PANEL_W, content_h, true)
}

pub(crate) struct TrackPickScreen {
    entry: EntryId,
    arg: TrackPickArg,
    /// The rows the table was built from and the index→choice map.
    choices: Vec<Choice>,
    labels: Vec<String>,
    pub(crate) table: TableView,
}

impl TrackPickScreen {
    pub(crate) fn new(entry: EntryId, arg: TrackPickArg, meta: crate::metadata::MetadataView<'_>) -> Self {
        let mut screen = Self {
            entry,
            arg,
            choices: Vec::new(),
            labels: Vec::new(),
            table: TableView::new(),
        };
        screen.rebuild(meta);
        screen
    }

    /// Materialise the item's rows. The cursor is kept when the row set did not move.
    fn rebuild(&mut self, meta: crate::metadata::MetadataView<'_>) {
        let d = meta.current();
        let next = d.map(|d| choices(d, self.arg.audio)).unwrap_or_default();
        let labels: Vec<String> = match d {
            Some(d) if self.arg.audio => d.audio.iter().map(row_label).collect(),
            Some(d) => d.subs.iter().map(row_label).collect(),
            None => Vec::new(),
        };
        if next == self.choices && labels == self.labels && !self.choices.is_empty() {
            return;
        }
        let keep = self.table.sel;
        self.choices = next;
        self.labels = labels;
        let mut sec = Section::new("");
        for (i, choice) in self.choices.iter().enumerate() {
            let label = if choice.off {
                crate::i18n::msg::widgets_tracks_off().to_string()
            } else {
                self.labels.get(i).cloned().unwrap_or_default()
            };
            let row = Row::new(&label).checked(choice.sid == self.arg.chosen);
            // The Off row leads with the crossed-out mark the tracks panel uses for the same
            // answer; a track row's own line already says what it is.
            sec = sec.row(if choice.off { row.licon(Icon::Close) } else { row });
        }
        self.table.compact = true;
        let sel = keep.clamp(0, (self.choices.len() as i32 - 1).max(0));
        self.table.set_sections(vec![sec], sel, false);
    }

    pub(crate) fn frame(&self) -> Rect {
        let [x, y, w, h] = self.arg.anchor.map(f32::from_bits);
        panel_at(Rect::new(x, y, w, h), self.table.measured_height())
    }

    /// **The rows as pointer stops** (rule 11: hover parks focus, a click activates), split out of
    /// `draw` for the host suite's sake (`PlayerScreen::record_stops`'s reason).
    pub(crate) fn record_stops<H: AppLike<Memory = crate::screens::registry::PageMemory>>(
        &self,
        f: &mut DrawFrame<'_, '_, H>,
    ) {
        let hit_p = f.painter.alpha(f.page_alpha);
        for elem in 0..self.choices.len() as u32 {
            if let Some(placed) = <Self as Focusable<H>>::place(self, &elem, f.cx, At::Drawn) {
                f.stop(
                    hit_p,
                    Stop {
                        key: FocusKey { entry: self.entry, elem },
                        rect: placed.rect,
                        rest_rect: placed.rest_rect,
                        clip: placed.clip,
                        hover: Hover::Focus,
                        activate: crate::ui::screen::Activate::Immediate,
                    },
                );
            }
        }
    }

    fn dismiss<H: AppLike>(&self, fx: &mut Effects<'_, H>) {
        fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
    }

    /// Commit row `elem` and close. The page keeps the number until its Play press.
    ///
    /// **The message goes to the PAGE, named on the argument.** Reporting it as a
    /// `ContentReq::Tracks` sent it to this surface's own instance, which dismisses itself in the
    /// same frame: the choice landed on a screen that was already leaving, and Play started with
    /// the ladder's own track (measured on the panel, 2026-11-09: picking `spa · SUBRIP` and
    /// pressing Play logged `ff: sub tracks=[...] selected=-1`). `AltSourcesScreen::commit` is the
    /// same shape.
    fn commit<H: AppLike>(&mut self, elem: u32, fx: &mut Effects<'_, H>) {
        let Some(choice) = self.choices.get(elem as usize).copied() else {
            return;
        };
        self.dismiss(fx);
        fx.push(Fx::Deliver(
            MachineId::Instance(self.arg.host),
            Delivery::Screen(ScreenEvent::App(commit_message(self.arg.audio, choice.sid))),
        ));
    }
}

impl<H: AppLike<Memory = crate::screens::registry::PageMemory> + crate::screens::registry::MetadataLike>
    Machine<H> for TrackPickScreen
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

/// A single-column `Focusable` over the table's own cursor, the pre-playback menu's shape.
impl<H: AppLike<Memory = crate::screens::registry::PageMemory>> Focusable<H> for TrackPickScreen {
    fn groups(&self, _cx: &Cx<'_, H>, out: &mut Vec<GroupSpec>) {
        out.push(GroupSpec {
            id: GroupId(0),
            kind: GroupKind::Column,
            seat: Seat::Remembered,
            reachable: AxisMask::BOTH,
            edge: [EdgeRule::Stop; 4],
            extent: self.frame(),
            len: self.choices.len(),
            elem: ElemKind::Bare,
        });
    }
    fn group_of(&self, elem: &u32, _cx: &Cx<'_, H>) -> Option<GroupId> {
        ((*elem as usize) < self.choices.len()).then_some(GroupId(0))
    }
    fn neighbour(&self, key: FocusKey<u32>, dir: Dir, _cx: &Cx<'_, H>) -> Step<u32> {
        let next = match dir {
            Dir::Up => self.table.next_selectable(key.elem as i32, -1),
            Dir::Down => self.table.next_selectable(key.elem as i32, 1),
            _ => None,
        };
        match next {
            Some(i) => Step::Move(FocusKey { entry: self.entry, elem: i as u32 }),
            None => Step::Edge,
        }
    }
    fn place(&self, elem: &u32, _cx: &Cx<'_, H>, _at: At) -> Option<Placed> {
        let rect = self.table.row_frame(self.frame(), *elem as i32)?;
        Some(Placed { rect, rest_rect: rect, clip: self.frame(), index: Some(*elem) })
    }
    fn reconcile(&self, want: FocusKey<u32>, cx: &Cx<'_, H>) -> FocusKey<u32> {
        if self.group_of(&want.elem, cx).is_some() {
            want
        } else {
            FocusKey { entry: self.entry, elem: 0 }
        }
    }
    fn seat(&self, _g: GroupId, _from: Placed, _cx: &Cx<'_, H>) -> FocusKey<u32> {
        FocusKey { entry: self.entry, elem: self.table.sel.max(0) as u32 }
    }
}

impl LogicalState for TrackPickScreen {
    fn write(&self, c: &mut Canon) {
        self.arg.write(c);
        c.seq(self.choices.len());
        for choice in &self.choices {
            c.u64(choice.sid as u64).bool(choice.off);
        }
        c.u32(self.table.sel as u32);
        self.table.write_motion(c);
    }
    fn probe(&self, out: &mut String) {
        out.push_str("track_pick");
    }
}

impl<H: AppLike<Memory = crate::screens::registry::PageMemory> + crate::screens::registry::MetadataLike> Screen<H>
    for TrackPickScreen
{
    fn name(&self) -> &'static str {
        "track_pick"
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
        let p = f.painter.alpha(appear).translate(0.0, RISE * (1.0 - appear));
        let measure = f.measure;
        let field = f.underlay;
        crate::ui::widgets::panel_ground(p, r, PANEL_RAD, field);
        self.table.draw(p, r, measure);
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

    fn stream(id: i64, lang: &str, codec: &str) -> crate::metadata::Stream {
        crate::metadata::Stream {
            id,
            lang: lang.into(),
            codec: codec.into(),
            ..Default::default()
        }
    }

    fn item() -> Detail {
        Detail {
            kind: "movie".into(),
            rk: "mv-1".into(),
            audio: vec![stream(1, "eng", "eac3"), stream(2, "spa", "aac")],
            subs: vec![stream(3, "eng", "srt")],
            ..Default::default()
        }
    }

    /// The subtitle list always opens with Off, and Off commits the id the plan reads as "none".
    #[test]
    fn the_subtitle_list_opens_with_off_and_the_audio_list_does_not() {
        let d = item();
        assert_eq!(choices(&d, true), vec![
            Choice { sid: 1, off: false },
            Choice { sid: 2, off: false },
        ]);
        assert_eq!(choices(&d, false), vec![
            Choice { sid: 0, off: true },
            Choice { sid: 3, off: false },
        ]);
    }

    /// The message names ONE half, so a pick from one disc cannot clear the other's choice.
    #[test]
    fn a_commit_names_only_the_half_the_list_was_about() {
        match commit_message(false, 3) {
            crate::screens::registry::AppMsg::DetailTracks { audio_sid, sub_sid } => {
                assert_eq!((audio_sid, sub_sid), (None, Some(3)));
            }
            _ => panic!("a picker commit must be a DetailTracks message"),
        }
        match commit_message(true, 2) {
            crate::screens::registry::AppMsg::DetailTracks { audio_sid, sub_sid } => {
                assert_eq!((audio_sid, sub_sid), (Some(2), None));
            }
            _ => panic!("a picker commit must be a DetailTracks message"),
        }
    }

    /// A row states the language and the codec — the two facts the About footer's Languages
    /// column printed, which is where the owner moved them from.
    #[test]
    fn a_row_states_the_language_and_its_codec() {
        // The language is the human name (a Spanish UI still reads "English" here, the language's
        // own name), and the codec is `friendly_codec`'s — the owner's 2026-11-10 "lenguaje más
        // entendible": "eng · EAC3" reads "English · Dolby Digital Plus".
        assert_eq!(row_label(&stream(1, "eng", "eac3")), "English \u{b7} Dolby Digital Plus");
        let mut flagged = stream(2, "spa", "srt");
        flagged.sdh = true;
        assert!(row_label(&flagged).contains("SDH"));
        assert_eq!(row_label(&stream(3, "", "")), crate::i18n::msg::browse_detail_unknown());
    }
}
