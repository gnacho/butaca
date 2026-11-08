//! **Identify**, the metadata providers' candidates for the item the pre-playback window is about,
//! and the press that makes one of them the item's match (issue #369).
//!
//! A `Style::Compact` surface on the shared `ModalStack`, anchored where the menu that opened it
//! was, for `screens::preplay_menu`'s reasons: a list you walk with the D-pad, on the panel ground,
//! over the window that describes the item.
//!
//! **The surface owns no request of its own.** It reads the store (`metadata::identify`), asks for a
//! lookup on MOUNT (`MetadataCmd::IdentifySearch`), and reports the chosen candidate
//! (`MetadataCmd::IdentifyApply`). The worker, the phases and the re-fetch after a successful apply
//! are `crate::metadata`'s and `crate::jellyfin::identify`'s: a screen may name neither a thread nor
//! a server (§2.1).
//!
//! **The write is the server's, and it says so.** `POST /Items/RemoteSearch/Apply/{itemId}`
//! requires elevation, so an ordinary viewer's account is refused, the panel then states that in
//! one line instead of looking like a network wobble. It never claims the change landed: `Applied`
//! is set only from the server's own 2xx.
//!
//! The candidate rows are the server's list verbatim, in its order: re-sorting would be inventing a
//! ranking (the same rule `screens::alt_sources` follows for its copies).

use std::borrow::Cow;

use crate::metadata::{IdentifyHit, IdentifyStatus};
use crate::screens::registry::{AppFx, AppLike, MetadataLike};
use crate::stores::metadata::MetadataCmd;
use crate::stores::{StoreCmd, StoreId};
use crate::ui::frame::Budget;
use crate::ui::machine::{
    Canon, Cx, Edge, Effects, EntryId, FocusKey, Fx, GroupId, Handled, InputKind, Key,
    LogicalState, Machine, NavOp,
};
use crate::ui::screen::{
    Activate, At, AxisMask, Dir, DrawFrame, EdgeRule, ElemKind, FocusSource, Focusable, GroupKind,
    GroupSpec, HitSource, Hover, Placed, RenderStrategy, Screen, ScreenEvent, Scrim, Seat, Step,
    Stop,
};
use crate::ui::table::{Row, Section, TableView};
use crate::ui::{theme, Rect};

/// The panel's width: a title, its provider under it and a year on the right, wide enough that a
/// long film title elides rather than colliding with the year column.
const PANEL_W: f32 = 720.0;
/// The pinned ~20px corner radius, the menu family's.
const PANEL_RAD: f32 = 20.0;
/// How far the panel rises into place, the one number the whole panel family shares.
const RISE: f32 = crate::ui::popover::Popover::RISE;
/// The status line's band, under the list: one `LABEL` line of prose and its air.
const STATUS_LEAD: f32 = theme::size::LABEL as f32 * 1.35;
const STATUS_GAP: f32 = theme::space::SM;

/// The fields [`IdentifyScreen::write`] canonicalises, for the recorder's shape pin (§5.4). The
/// selected ROW and the phase are in it: UP/DOWN and a landing move nothing else in the app, so
/// without them a replay would grade the panel opening and closing with a hole between.
pub(crate) const SHAPE: [&str; 2] = [
    "IdentifyScreen{arg:IdentifyArg{anchor:[u32;4]},rows:[{name:str,year:i32,provider:str}],status:u32,sel:i32,table:TableViewMotion}",
    TableView::MOTION_SHAPE,
];

/// What the container is asked to present: only WHERE it hangs. The subject is
/// `metadata::current()`, the item the window under it describes, exactly as the Track-information
/// and About sheets take theirs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct IdentifyArg {
    /// Bit-preserving rest rectangle of the three-dot control that opened the menu this list
    /// replaced, so the list appears where the menu was.
    pub(crate) anchor: [u32; 4],
}

impl LogicalState for IdentifyArg {
    fn write(&self, c: &mut Canon) {
        for value in self.anchor {
            c.u32(value);
        }
    }
    fn probe(&self, out: &mut String) {
        out.push_str("identify_arg");
    }
}

pub(crate) struct IdentifyScreen {
    entry: EntryId,
    arg: IdentifyArg,
    /// The rows the table was built from, the rebuild stamp.
    hits: Vec<IdentifyHit>,
    /// The phase the rows were built under; a change re-seats the cursor and repaints the line.
    status: IdentifyStatus,
    pub(crate) table: TableView,
}

impl IdentifyScreen {
    pub(crate) fn new(entry: EntryId, arg: IdentifyArg) -> Self {
        Self {
            entry,
            arg,
            hits: Vec::new(),
            status: IdentifyStatus::Idle,
            table: TableView::new(),
        }
    }

    /// The candidates the store holds for THIS item, or nothing, an answer about another item is
    /// not this surface's to draw.
    fn live_hits(&self, meta: crate::metadata::MetadataView<'_>) -> Vec<IdentifyHit> {
        let Some(d) = meta.current() else { return Vec::new() };
        let state = meta.identify();
        if state.is_about(d.sid, &d.rk) {
            state.hits().to_vec()
        } else {
            Vec::new()
        }
    }

    fn live_status(&self, meta: crate::metadata::MetadataView<'_>) -> IdentifyStatus {
        let Some(d) = meta.current() else { return IdentifyStatus::Idle };
        let state = meta.identify();
        if state.is_about(d.sid, &d.rk) {
            state.status()
        } else {
            IdentifyStatus::Idle
        }
    }

    /// Materialise the store into the table, when and only when the drawn content would differ.
    fn refresh(&mut self, meta: crate::metadata::MetadataView<'_>) -> bool {
        let hits = self.live_hits(meta);
        let status = self.live_status(meta);
        if hits == self.hits && status == self.status {
            return false;
        }
        let keep = self.table.sel;
        self.hits = hits;
        self.status = status;
        let mut sec = Section::new("");
        for hit in &self.hits {
            let mut row = Row::new(hit.name.clone()).detail(hit.provider.clone());
            if hit.year > 0 {
                row = row.value(hit.year.to_string());
            }
            sec = sec.row(row);
        }
        // Two-line rows: the provider under the title is what tells two providers' copies of the
        // same film apart, and it is a column you compare down.
        self.table.compact = false;
        let sel = keep.clamp(0, (self.hits.len() as i32 - 1).max(0));
        self.table.set_sections(vec![sec], sel, false);
        true
    }

    /// Ask the store for the lookup, ONCE per mount, which is what makes re-opening the surface a
    /// fresh search and a landed answer stable.
    fn request<H: AppLike + MetadataLike>(&self, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) {
        let Some(d) = H::metadata(cx).current() else { return };
        fx.push(Fx::App(AppFx::Store(
            StoreId::Metadata,
            StoreCmd::Metadata(MetadataCmd::IdentifySearch {
                sid: d.sid,
                rk: d.rk.clone(),
            }),
        )));
    }

    /// The line under the list, if the phase has one to say.
    fn status_text(&self) -> Option<&'static str> {
        match self.status {
            IdentifyStatus::Idle | IdentifyStatus::Searching => {
                Some(crate::i18n::msg::widgets_identify_searching())
            }
            IdentifyStatus::Failed => Some(crate::i18n::msg::widgets_identify_failed()),
            IdentifyStatus::Ready if self.hits.is_empty() => {
                Some(crate::i18n::msg::widgets_identify_empty())
            }
            IdentifyStatus::Ready => None,
            IdentifyStatus::Applying => Some(crate::i18n::msg::widgets_identify_applying()),
            IdentifyStatus::Applied => Some(crate::i18n::msg::widgets_identify_applied()),
            IdentifyStatus::ApplyFailed => Some(crate::i18n::msg::widgets_identify_apply_failed()),
        }
    }

    fn status_h(&self) -> f32 {
        if self.status_text().is_some() {
            STATUS_GAP + STATUS_LEAD
        } else {
            0.0
        }
    }

    pub(crate) fn frame(&self) -> Rect {
        let [x, y, w, h] = self.arg.anchor.map(f32::from_bits);
        crate::ui::widgets::anchored_menu(
            Rect::new(x, y, w, h),
            PANEL_W,
            self.table.measured_height() + self.status_h(),
            true,
        )
    }

    fn dismiss<H: AppLike>(&self, fx: &mut Effects<'_, H>) {
        fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
    }

    /// Commit the highlighted candidate. Only a READY list can be applied: while the lookup is out
    /// there is nothing to pick, and while an apply is in flight a second one would race the first.
    fn commit<H: AppLike + MetadataLike>(&mut self, elem: u32, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) {
        if self.status != IdentifyStatus::Ready {
            return;
        }
        let Some(hit) = self.hits.get(elem as usize).cloned() else { return };
        let Some(d) = H::metadata(cx).current() else { return };
        fx.push(Fx::App(AppFx::Store(
            StoreId::Metadata,
            StoreCmd::Metadata(MetadataCmd::IdentifyApply {
                sid: d.sid,
                rk: d.rk.clone(),
                hit,
            }),
        )));
    }
}

impl<H: AppLike<Memory = crate::screens::registry::PageMemory> + MetadataLike> Machine<H> for IdentifyScreen {
    type Ev = ScreenEvent<H>;
    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        match ev {
            ScreenEvent::Mount => {
                self.refresh(H::metadata(cx));
                self.request(cx, fx);
                Handled::Yes
            }
            ScreenEvent::Enter(_) => {
                self.refresh(H::metadata(cx));
                Handled::Yes
            }
            ScreenEvent::StoreChanged(ord, _) => {
                if *ord == StoreId::Metadata.ord() && self.refresh(H::metadata(cx)) {
                    fx.invalidate(crate::ui::present::Provenance::Landing(fx.from()));
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
                self.commit(*elem, cx, fx);
                Handled::Yes
            }
            ScreenEvent::PressCommit(_) => {
                if let Some(key) = cx.focus.current {
                    self.commit(key.elem, cx, fx);
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

/// A single-column `Focusable` over the table's own cursor, `AltSourcesScreen`'s shape. A list with
/// no candidates has NO element: the group is empty and BACK is the only press that does anything,
/// which is the honest state of a lookup that found nothing.
impl<H: AppLike<Memory = crate::screens::registry::PageMemory>> Focusable<H> for IdentifyScreen {
    fn groups(&self, _cx: &Cx<'_, H>, out: &mut Vec<GroupSpec>) {
        out.push(GroupSpec {
            id: GroupId(0),
            kind: GroupKind::Column,
            seat: Seat::Remembered,
            reachable: AxisMask::BOTH,
            edge: [EdgeRule::Stop; 4],
            extent: self.frame(),
            len: self.hits.len(),
            elem: ElemKind::Bare,
        });
    }
    fn group_of(&self, elem: &u32, _cx: &Cx<'_, H>) -> Option<GroupId> {
        ((*elem as usize) < self.hits.len()).then_some(GroupId(0))
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

impl LogicalState for IdentifyScreen {
    fn write(&self, c: &mut Canon) {
        self.arg.write(c);
        c.seq(self.hits.len());
        for hit in &self.hits {
            c.str(&hit.name).u32(u32::try_from(hit.year).unwrap_or(0)).str(&hit.provider);
        }
        c.u32(match self.status {
            IdentifyStatus::Idle => 0,
            IdentifyStatus::Searching => 1,
            IdentifyStatus::Ready => 2,
            IdentifyStatus::Failed => 3,
            IdentifyStatus::Applying => 4,
            IdentifyStatus::Applied => 5,
            IdentifyStatus::ApplyFailed => 6,
        });
        c.u32(self.table.sel as u32);
        self.table.write_motion(c);
    }
    fn probe(&self, out: &mut String) {
        out.push_str("identify");
    }
}

impl<H: AppLike<Memory = crate::screens::registry::PageMemory> + MetadataLike> Screen<H> for IdentifyScreen {
    fn name(&self) -> &'static str {
        "identify"
    }
    fn state(&self) -> &dyn LogicalState {
        self
    }
    fn crumb(&self, _cx: &Cx<'_, H>) -> Option<Cow<'_, str>> {
        None
    }
    fn prepare(&mut self, _b: &mut Budget, _cx: &Cx<'_, H>) {}
    /// The modal dim, asked for rather than drawn, `AltSourcesScreen::scrim`'s contract.
    fn scrim(&self) -> Scrim {
        Scrim::dim(theme::underlay::DIM_PANEL)
    }
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, H>) {
        let appear = f.page_alpha;
        let r = self.frame();
        let p = f.painter.alpha(appear).translate(0.0, RISE * (1.0 - appear));
        let measure = f.measure;
        let field = f.underlay;
        crate::ui::profile::phase("dt.identify", || {
            crate::ui::widgets::panel_ground(p, r, PANEL_RAD, field);
            let list_h = self.table.measured_height();
            self.table.draw(p, Rect::new(r.x, r.y, r.w, list_h.min(r.h)), measure);
            if let Some(text) = self.status_text() {
                if let Ok(cs) = std::ffi::CString::new(text) {
                    let y = r.y + list_h.min(r.h - self.status_h()) + STATUS_GAP;
                    crate::ui::label::Label::new(
                        cs.as_ptr(),
                        theme::size::LABEL,
                        theme::TEXT_SECONDARY,
                    )
                    .v(crate::ui::label::VAlign::CapTop)
                    .h(theme::alert::TEXT_ALIGN)
                    .draw(p, Rect::new(r.x, y, r.w, STATUS_LEAD));
                }
            }
        });
        // The hit map's stops ride the SETTLED geometry, like every other surface here.
        let hit_p = f.painter.alpha(appear);
        for elem in 0..self.hits.len() as u32 {
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
                        hover: Hover::Focus,
                        activate: Activate::Immediate,
                    },
                );
            }
        }
    }
    fn render(&self) -> RenderStrategy {
        RenderStrategy::Page
    }
    fn focus_source(&self) -> FocusSource {
        FocusSource::Engine
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
