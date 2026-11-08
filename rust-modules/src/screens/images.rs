//! **Edit images**, the pictures the server's metadata providers already hold for the item the
//! pre-playback window is about, and the press that makes one of them the item's own (issue #369).
//!
//! A `Style::Compact` surface on the shared `ModalStack`, anchored where the menu that opened it
//! was, for `screens::identify`'s reasons: a grid you walk with the D-pad, on the panel ground,
//! over the window that describes the item.
//!
//! **The surface owns no request of its own.** It reads the store (`metadata::images`), asks for
//! the list on MOUNT (`MetadataCmd::ImagesLoad`), and reports the chosen picture
//! (`MetadataCmd::ImagesApply`). The worker, the phases and the re-fetch after a successful apply
//! are `crate::metadata`'s and `crate::jellyfin::images`'s: a screen may name neither a thread nor
//! a server (§2.1).
//!
//! **The write is the server's, and it says so.** `POST /Items/{itemId}/RemoteImages/Download`
//! requires elevation, so an ordinary viewer's account is refused, and the panel then states that
//! in one line instead of looking like a network wobble. It never claims the change landed:
//! `Applied` is set only from the server's own 2xx.
//!
//! **Every tile is somebody else's JPEG.** The pictures are NOT on this server: each one is an
//! absolute URL on the provider's host, and the grid draws the provider's own smaller copy
//! (`ThumbnailUrl`) through the ordinary poster pipeline, whose absolute-URL arm dials it
//! headerless (`jf::client::get_bytes`). That is also why the page is BOUNDED
//! (`jellyfin::images::PAGE`): the surface draws every tile it asks for, and each one is a decode
//! on a television.
//!
//! **One image type: `Primary`.** `jellyfin::images` argues it; the grid is a poster grid.

use std::borrow::Cow;

use crate::metadata::{ImagesStatus, RemoteImage};
use crate::screens::registry::{AppFx, AppLike, MetadataLike};
use crate::stores::metadata::MetadataCmd;
use crate::stores::{StoreCmd, StoreId};
use crate::ui::frame::Budget;
use crate::ui::label::Label;
use crate::ui::machine::{
    Canon, Cx, Edge, Effects, EntryId, FocusKey, Fx, GroupId, Handled, InputKind, Key,
    LogicalState, Machine, NavOp,
};
use crate::ui::screen::{
    Activate, At, AxisMask, Dir, DrawFrame, EdgeRule, ElemKind, Focusable, GroupKind, GroupSpec,
    Hover, Placed, RenderStrategy, Screen, ScreenEvent, Scrim, Seat, Step, Stop,
};
use crate::ui::{theme, Rect};

/// The grid's columns. Six posters across, the app's own poster grid (`ui::poster_grid::COLS`),
/// so a picker reads as the shelf it belongs to.
const COLS: usize = 6;
/// One poster tile: 2:3, the aspect every poster in this app is drawn at.
const TILE_W: f32 = 190.0;
const TILE_H: f32 = 285.0;
const TILE_GAP: f32 = 20.0;
/// The caption band under a tile: the provider on one line and the picture's size on the next.
const CAP_LEAD: f32 = theme::size::CAPTION as f32 * 1.25;
const CAP_H: f32 = CAP_LEAD * 2.0;
/// The panel's inset, the alert family's pad.
const PAD: f32 = theme::alert::PAD;
/// The panel's width: the grid's own, so no tile is ever short of its column.
const PANEL_W: f32 = PAD * 2.0 + COLS as f32 * TILE_W + (COLS as f32 - 1.0) * TILE_GAP;
/// The pinned ~20px corner radius, the menu family's.
const PANEL_RAD: f32 = 20.0;
/// How far the panel rises into place, the one number the whole panel family shares.
const RISE: f32 = crate::ui::popover::Popover::RISE;
/// The status line's band, under the grid: one `LABEL` line of prose and its air.
const STATUS_LEAD: f32 = theme::size::LABEL as f32 * 1.35;
const STATUS_GAP: f32 = theme::space::SM;
/// The grid's cells. A `Grid` group's `holes` is the app's "no gaps in this lattice".
const NO_HOLES: &[(usize, usize)] = &[];

/// The fields [`ImagesScreen::write`] canonicalises, for the recorder's shape pin (§5.4). The
/// selected CELL and the phase are in it for `IdentifyScreen::SHAPE`'s reason: LEFT/RIGHT and a
/// landing move nothing else in the app, so without them a replay would grade the panel opening
/// and closing with a hole between.
pub(crate) const SHAPE: &str =
    "ImagesScreen{arg:ImagesArg{anchor:[u32;4]},rows:[{provider:str,url:str,w:i32,h:i32}],status:u32,sel:i32}";

/// What the container is asked to present: only WHERE it hangs. The subject is
/// `metadata::current()`, the item the window under it describes, exactly as the Identify list's
/// is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ImagesArg {
    /// Bit-preserving rest rectangle of the three-dot control that opened the menu this grid
    /// replaced, so the grid appears where the menu was.
    pub(crate) anchor: [u32; 4],
}

impl LogicalState for ImagesArg {
    fn write(&self, c: &mut Canon) {
        for value in self.anchor {
            c.u32(value);
        }
    }
    fn probe(&self, out: &mut String) {
        out.push_str("images_arg");
    }
}

pub(crate) struct ImagesScreen {
    entry: EntryId,
    arg: ImagesArg,
    /// The pictures the grid was built from, the rebuild stamp.
    images: Vec<RemoteImage>,
    /// How many the server holds in total; the status line names it when it exceeds the page.
    total: i32,
    /// The phase the grid was built under; a change repaints the line.
    status: ImagesStatus,
    /// The selected cell. The grid is the surface's own cursor, like the table's `sel`.
    sel: i32,
}

impl ImagesScreen {
    pub(crate) fn new(entry: EntryId, arg: ImagesArg) -> Self {
        Self {
            entry,
            arg,
            images: Vec::new(),
            total: 0,
            status: ImagesStatus::Idle,
            sel: 0,
        }
    }

    /// The pictures the store holds for THIS item, or nothing: an answer about another item is not
    /// this surface's to draw.
    fn live(&self, meta: crate::metadata::MetadataView<'_>) -> Option<(Vec<RemoteImage>, i32, ImagesStatus)> {
        let d = meta.current()?;
        let state = meta.images();
        if !state.is_about(d.sid, &d.rk) {
            return None;
        }
        Some((state.images().to_vec(), state.total(), state.status()))
    }

    /// Materialise the store into the grid, when and only when the drawn content would differ.
    fn refresh(&mut self, meta: crate::metadata::MetadataView<'_>) -> bool {
        let (images, total, status) = self.live(meta).unwrap_or_default();
        if images == self.images && total == self.total && status == self.status {
            return false;
        }
        self.images = images;
        self.total = total;
        self.status = status;
        self.sel = self.sel.clamp(0, (self.images.len() as i32 - 1).max(0));
        true
    }

    /// Ask the store for the list, ONCE per mount, which is what makes re-opening the surface a
    /// fresh read and a landed answer stable.
    fn request<H: AppLike + MetadataLike>(&self, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) {
        let Some(d) = H::metadata(cx).current() else { return };
        fx.push(Fx::App(AppFx::Store(
            StoreId::Metadata,
            StoreCmd::Metadata(MetadataCmd::ImagesLoad {
                sid: d.sid,
                rk: d.rk.clone(),
            }),
        )));
    }

    /// The line under the grid, if the phase has one to say.
    fn status_text(&self) -> Option<String> {
        match self.status {
            ImagesStatus::Idle | ImagesStatus::Loading => {
                Some(crate::i18n::msg::widgets_images_loading().to_string())
            }
            ImagesStatus::Failed => Some(crate::i18n::msg::widgets_images_failed().to_string()),
            ImagesStatus::Ready if self.images.is_empty() => {
                Some(crate::i18n::msg::widgets_images_empty().to_string())
            }
            // A bounded page is not the whole set, and the viewer is told so rather than left to
            // believe the provider only had twelve pictures.
            ImagesStatus::Ready if self.total > self.images.len() as i32 => Some(
                crate::i18n::msg::widgets_images_page(
                    self.images.len() as i64,
                    self.total as i64,
                ),
            ),
            ImagesStatus::Ready => None,
            ImagesStatus::Applying => Some(crate::i18n::msg::widgets_images_applying().to_string()),
            ImagesStatus::Applied => Some(crate::i18n::msg::widgets_images_applied().to_string()),
            ImagesStatus::ApplyFailed => {
                Some(crate::i18n::msg::widgets_images_apply_failed().to_string())
            }
        }
    }

    /// Does the phase have a line to say? Asked WITHOUT building it: `frame` runs on every draw
    /// and every placement, and the page-count line is formatted, so a status band derived from
    /// `status_text` would allocate twice a frame for an answer that is a property of the phase.
    fn has_status(&self) -> bool {
        match self.status {
            ImagesStatus::Ready => {
                self.images.is_empty() || self.total > self.images.len() as i32
            }
            _ => true,
        }
    }

    fn status_h(&self) -> f32 {
        if self.has_status() {
            STATUS_GAP + STATUS_LEAD
        } else {
            0.0
        }
    }

    /// The grid's own height for `n` pictures, caption bands included.
    fn grid_h(n: usize) -> f32 {
        if n == 0 {
            return 0.0;
        }
        let rows = n.div_ceil(COLS) as f32;
        rows * (TILE_H + CAP_H) + (rows - 1.0) * TILE_GAP
    }

    pub(crate) fn frame(&self) -> Rect {
        let [x, y, w, h] = self.arg.anchor.map(f32::from_bits);
        let grid = if self.images.is_empty() {
            0.0
        } else {
            Self::grid_h(self.images.len())
        };
        crate::ui::widgets::anchored_menu(
            Rect::new(x, y, w, h),
            PANEL_W,
            PAD + grid + self.status_h() + PAD,
            true,
        )
    }

    /// One cell: the poster on top and its caption band under it. ONE function for the painter,
    /// the focus model and the hit stops, so the capsule a person aims at cannot drift off the
    /// tile it lights.
    fn cell(&self, index: usize, frame: Rect) -> Option<Rect> {
        if index >= self.images.len() {
            return None;
        }
        let (row, col) = (index / COLS, index % COLS);
        Some(Rect::new(
            frame.x + PAD + col as f32 * (TILE_W + TILE_GAP),
            frame.y + PAD + row as f32 * (TILE_H + CAP_H + TILE_GAP),
            TILE_W,
            TILE_H + CAP_H,
        ))
    }

    /// Where the status line sits: under the grid when there is one, at the top of the panel's
    /// content box when the grid is empty.
    fn status_y(&self, frame: Rect) -> f32 {
        if self.images.is_empty() {
            frame.y + PAD
        } else {
            frame.y + PAD + Self::grid_h(self.images.len()) + STATUS_GAP
        }
    }

    fn dismiss<H: AppLike>(&self, fx: &mut Effects<'_, H>) {
        fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
    }

    /// Commit the highlighted picture. Only a READY grid can be applied: while the list is out
    /// there is nothing to pick, and while a download is in flight a second one would race the
    /// first.
    fn commit<H: AppLike + MetadataLike>(
        &mut self,
        elem: u32,
        cx: &Cx<'_, H>,
        fx: &mut Effects<'_, H>,
    ) {
        if self.status != ImagesStatus::Ready {
            return;
        }
        let Some(img) = self.images.get(elem as usize).cloned() else { return };
        let Some(d) = H::metadata(cx).current() else { return };
        fx.push(Fx::App(AppFx::Store(
            StoreId::Metadata,
            StoreCmd::Metadata(MetadataCmd::ImagesApply {
                sid: d.sid,
                rk: d.rk.clone(),
                img,
            }),
        )));
    }
}

impl<H: AppLike<Memory = crate::screens::registry::PageMemory> + MetadataLike> Machine<H>
    for ImagesScreen
{
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
            ScreenEvent::Tick(_) => Handled::Yes,
            ScreenEvent::FocusMoved { to, .. } => {
                self.sel = to.elem as i32;
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

/// A `Grid` of `Card` elements over the surface's own cursor, `screens::library`'s lattice shape
/// at a fixed size. A grid with no pictures has NO element: the group is empty and BACK is the
/// only press that does anything, which is the honest state of a provider that offered nothing.
impl<H: AppLike<Memory = crate::screens::registry::PageMemory>> Focusable<H> for ImagesScreen {
    fn groups(&self, _cx: &Cx<'_, H>, out: &mut Vec<GroupSpec>) {
        out.push(GroupSpec {
            id: GroupId(0),
            kind: GroupKind::Grid { cols: COLS, holes: NO_HOLES },
            seat: Seat::Remembered,
            reachable: AxisMask::BOTH,
            // A wall on every side: the lattice is bounded by the panel it is drawn in, and a
            // directional press at its edge must not hand focus to the page behind the surface.
            edge: [EdgeRule::Stop; 4],
            extent: self.frame(),
            len: self.images.len(),
            elem: ElemKind::Card,
        });
    }
    fn group_of(&self, elem: &u32, _cx: &Cx<'_, H>) -> Option<GroupId> {
        ((*elem as usize) < self.images.len()).then_some(GroupId(0))
    }
    fn neighbour(&self, key: FocusKey<u32>, dir: Dir, _cx: &Cx<'_, H>) -> Step<u32> {
        match crate::ui::poster_grid::neighbour(key.elem as usize, self.images.len(), COLS, dir) {
            Some(i) => Step::Move(FocusKey {
                entry: self.entry,
                elem: i as u32,
            }),
            None => Step::Edge,
        }
    }
    fn place(&self, elem: &u32, _cx: &Cx<'_, H>, _at: At) -> Option<Placed> {
        let rect = self.cell(*elem as usize, self.frame())?;
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
            elem: self.sel.max(0) as u32,
        }
    }
}

impl LogicalState for ImagesScreen {
    fn write(&self, c: &mut Canon) {
        self.arg.write(c);
        c.seq(self.images.len());
        for img in &self.images {
            c.str(&img.provider).str(&img.url).u32(img.width as u32).u32(img.height as u32);
        }
        c.u32(match self.status {
            ImagesStatus::Idle => 0,
            ImagesStatus::Loading => 1,
            ImagesStatus::Ready => 2,
            ImagesStatus::Failed => 3,
            ImagesStatus::Applying => 4,
            ImagesStatus::Applied => 5,
            ImagesStatus::ApplyFailed => 6,
        });
        c.u32(self.sel as u32);
    }
    fn probe(&self, out: &mut String) {
        out.push_str("images");
    }
}

impl<H: AppLike<Memory = crate::screens::registry::PageMemory> + MetadataLike> Screen<H>
    for ImagesScreen
{
    fn name(&self) -> &'static str {
        "images"
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
        let field = f.underlay;
        crate::ui::profile::phase("dt.images", || {
            crate::ui::widgets::panel_ground(p, r, PANEL_RAD, field);
            let focused = f
                .cx
                .focus
                .current
                .filter(|key| key.entry == self.entry)
                .map(|key| key.elem);
            for (index, img) in self.images.iter().enumerate() {
                let Some(cell) = self.cell(index, r) else { continue };
                let art = Rect::new(cell.x, cell.y, TILE_W, TILE_H);
                // The tile is drawn through the ordinary card pipeline, so a provider's picture
                // wears the same skeleton, edge sheen and focus lift as every poster in the app.
                crate::ui::widgets::draw_card(
                    p,
                    art,
                    crate::jellyfin::SERVER_ID,
                    &img.thumb,
                    (TILE_W as i32, TILE_H as i32),
                    12.0,
                    focused == Some(index as u32),
                    1.0,
                );
                self.caption(p, img, cell, f.measure);
            }
            if let Some(text) = self.status_text() {
                if let Ok(cs) = std::ffi::CString::new(text) {
                    Label::new(cs.as_ptr(), theme::size::LABEL, theme::TEXT_SECONDARY)
                        .v(crate::ui::label::VAlign::CapTop)
                        .h(theme::alert::TEXT_ALIGN)
                        .draw(p, Rect::new(r.x + PAD, self.status_y(r), r.w - 2.0 * PAD, STATUS_LEAD));
                }
            }
        });
        // The hit map's stops ride the SETTLED geometry, like every other surface here.
        let hit_p = f.painter.alpha(appear);
        for elem in 0..self.images.len() as u32 {
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

impl ImagesScreen {
    /// The two caption lines under a tile: which provider offered the picture, and its size.
    /// ELIDED to the tile's own column rather than clipped: a provider's name is as long as it
    /// likes ("The Open Movie Database"), and a clipped one would bleed into the tile beside it,
    /// while `text::elide_by` ends in an ellipsis that says the name was longer.
    fn caption(
        &self,
        p: crate::ui::Painter,
        img: &RemoteImage,
        cell: Rect,
        measure: &dyn crate::ui::machine::Measure,
    ) {
        let top = cell.y + TILE_H + theme::space::XS;
        let provider = crate::text::elide_by(&img.provider, cell.w, false, |t| {
            measure.width_str(t, theme::size::CAPTION, false)
        });
        if let Ok(cs) = std::ffi::CString::new(provider) {
            Label::new(cs.as_ptr(), theme::size::CAPTION, theme::TEXT_SECONDARY)
                .v(crate::ui::label::VAlign::CapTop)
                .draw(p, Rect::new(cell.x, top, cell.w, CAP_LEAD));
        }
        let size = img.size_label();
        if !size.is_empty() {
            if let Ok(cs) = std::ffi::CString::new(size) {
                Label::new(cs.as_ptr(), theme::size::CAPTION, theme::TEXT_TERTIARY)
                    .v(crate::ui::label::VAlign::CapTop)
                    .draw(p, Rect::new(cell.x, top + CAP_LEAD, cell.w, CAP_LEAD));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(n: usize) -> ImagesScreen {
        let mut screen = ImagesScreen::new(
            EntryId(5),
            ImagesArg {
                anchor: [
                    1400.0f32.to_bits(),
                    120.0f32.to_bits(),
                    96.0f32.to_bits(),
                    56.0f32.to_bits(),
                ],
            },
        );
        screen.images = (0..n)
            .map(|i| {
                crate::jellyfin::images::image_for_test(
                    &format!("https://img.example/{i}.jpg"),
                    "TheMovieDb",
                    1000,
                    1500,
                )
            })
            .collect();
        screen.total = n as i32;
        screen.status = ImagesStatus::Ready;
        screen
    }

    /// The panel is the GRID's width, it hangs off its control inside the screen's keep-out, and
    /// every tile is a cell of a six-column lattice with no two cells overlapping.
    #[test]
    fn the_grid_hangs_off_its_control_and_its_cells_never_overlap() {
        use crate::ui::consts::{MARGIN_X, SCR_W};
        let screen = screen(12);
        let r = screen.frame();
        assert!(r.x >= MARGIN_X && r.x + r.w <= SCR_W - MARGIN_X, "inside the horizontal keep-out");
        assert!(r.w >= PANEL_W - 0.01, "the panel is at least the grid's own width");
        let first = screen.cell(0, r).expect("the first picture has a cell");
        let sixth = screen.cell(5, r).expect("the sixth picture closes the first row");
        let seventh = screen.cell(6, r).expect("the seventh opens the second");
        assert_eq!(first.y, sixth.y, "six across");
        assert!(seventh.y > first.y, "the seventh starts a new row");
        assert!(sixth.x + sixth.w <= r.x + r.w, "the row fits the panel");
        assert!(screen.cell(12, r).is_none(), "a picture past the page has no cell");
    }

    /// The lattice walks like every other poster grid: LEFT/RIGHT along a row, UP/DOWN a row at a
    /// time, and a WALL (not a wrap) at every edge, so a press at the edge cannot hand focus to the
    /// page behind the surface.
    #[test]
    fn the_lattice_walks_and_stops_at_its_edges() {
        use crate::ui::poster_grid::neighbour;
        use crate::ui::screen::Dir;
        assert_eq!(neighbour(0, 12, COLS, Dir::Right), Some(1));
        assert_eq!(neighbour(0, 12, COLS, Dir::Left), None);
        assert_eq!(neighbour(0, 12, COLS, Dir::Down), Some(6));
        assert_eq!(neighbour(6, 12, COLS, Dir::Up), Some(0));
        assert_eq!(neighbour(0, 12, COLS, Dir::Up), None);
        assert_eq!(neighbour(11, 12, COLS, Dir::Right), None);
        assert_eq!(neighbour(11, 12, COLS, Dir::Down), None, "the last row is the floor");
    }

    /// A bounded page says so, and a complete one says nothing: the line under the grid is the
    /// difference between "these are the pictures" and "these are SOME of the pictures".
    #[test]
    fn a_bounded_page_says_how_many_of_how_many() {
        let mut screen = screen(12);
        screen.total = 31;
        assert!(screen.has_status(), "twelve of thirty-one is a line");
        screen.total = 12;
        assert!(!screen.has_status(), "the whole page is not");
        screen.images.clear();
        screen.total = 0;
        assert!(screen.has_status(), "an empty page is a line of its own");
    }
}
