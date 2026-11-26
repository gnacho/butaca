//! The **account menu** - a registered `Style::Sheet` surface on the shared `ModalStack`, opened
//! from the top-left profile chip. Jellyfin sign-out or sign-in, Settings, and - in a lab build -
//! "Send diagnostics". No profile action is offered.
//!
//! **Navigation owns its lifetime, its phase, its input scope and its dim.** The page under the
//! panel stays on screen and stays the top PAGE, so there is nothing to name and nowhere to close
//! back to.
//!
//! The rows come only from the installed Jellyfin client. An old account session is never an
//! account source. [`chip_label`] stays beside the rows because the app chrome and this menu must
//! agree on the account words they present.

use std::borrow::Cow;

use crate::server::session::Account;
use crate::screens::registry::{AppFx, AppLike, LoopReq};
use crate::ui::frame::Budget;
use crate::ui::machine::{
    Canon, Cx, Edge, Effects, EntryId, FocusKey, Fx, GroupId, Handled, InputKind, Key,
    LogicalState, Machine, NavOp,
};
use crate::ui::screen::{
    Activate, At, AxisMask, Dir, DrawFrame, EdgeRule, ElemKind, Focusable, GroupKind, GroupSpec,
    Hover, Placed, RenderStrategy, Screen, ScreenEvent, Scrim, Seat, Step, Stop,
};
use crate::ui::Rect;

/// What the highlighted row does on OK.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    None,
    SignIn,
    /// **Settings** — the reachable owner of Home sources, Privacy, Legal notices and About
    /// ([`crate::screens::registry::AppArg::Settings`]). Offered in EVERY build and in **both**
    /// account states:
    /// someone who cannot sign in has still received a copy of this software, and LG's Privacy
    /// Guideline requires the policy to be readable *in the app* rather than only on the store
    /// listing.
    Settings,
    /// **Lab builds only** — snapshot the diagnostic ring and upload it (`crate::lab`). It is in
    /// this menu because it must be reachable with the D-PAD ALONE: the remote trigger is a colour
    /// button (BLUE, `wcode` 489 on the dev set), and an LG Cloud Test Lab virtual remote may not
    /// offer colour buttons at all — nor is that code guaranteed on a set nobody here has touched
    /// (`docs/lab-diagnostics.md` §7). Never offered in any other build —
    /// [`crate::lab::menu_row_enabled`] is `false` at compile time.
    SendDiagnostics,
    /// A configured server row (index into `jellyfin::boot::load_servers`) — sign in against it
    /// and make it active, the same real action `ServersPage` requests.
    SwitchServer(usize),
    /// The *Add by password* row — arm add-server mode on the sign-in form.
    AddServer,
}

/// The profile menu's element namespace for the two dynamic row kinds. The four fixed actions keep
/// their `Action` discriminants (`1..=4`); server rows sit above `SERVER_BASE` so their stable
/// identity is the server LIST index, not a position the menu might reorder.
const SERVER_BASE: u32 = 0x100;
const ADD_ELEM: u32 = 0x1FF;

impl Action {
    /// The stable element id this action's row is keyed by. Server rows are `SERVER_BASE + index`,
    /// so the engine's focus and the loop's `JfSwitchServer(index)` name the same index.
    pub(crate) fn key(self) -> u32 {
        match self {
            Action::None => 0,
            Action::SignIn => 1,
            Action::Settings => 3,
            Action::SendDiagnostics => 4,
            Action::SwitchServer(i) => SERVER_BASE + i as u32,
            Action::AddServer => ADD_ELEM,
        }
    }
}


/// **The supplied mockup's server dialog, measured off its DOM at 1920x1080** (2026-11-25):
/// `.sm-panel{width:560px;padding:34px;border-radius:24px}`, its `h2` `Fraunces 600 30px`, its `.sub`
/// `500 14.5px`, and `.sm-server{height 82, border-radius:16px, padding:18px 20px, gap:16px}` with a
/// 10px `.dot`, a `600 16px` name over a `500 13.5px` line, and a right-hand `.tag` in
/// `700 12px` with `.1em` tracking. The type is the reference's, not the app's couch ladder — the
/// same call this bar already makes, recorded in the plan.
const PANEL_W: f32 = 560.0;
const PANEL_PAD: f32 = 34.0;
/// `.sm-panel`'s corner, and the row's.
const PANEL_RAD: f32 = 24.0;
const ROW_RAD: f32 = 16.0;
const ROW_H: f32 = 82.0;
/// **The add card is 84, not 82.** The reference's `.who span` rule does not reach that row, so its
/// subtitle renders at the button's own 16px instead of 13.5 — a taller line box, a taller card. It
/// is the whole of why the mockup's three-row panel measures 453 and two-thirds of why ours used to
/// measure 451.
const ROW_H_ADD: f32 = 84.0;
/// And its subtitle's size, for the same reason: 16, not the server rows' 13.5.
const DETAIL_SZ_ADD: std::os::raw::c_int = 16;
const ROW_GAP: f32 = 10.0;
/// `.sm-panel` and `.sm-server` both carry a 1px border, and the reference's padding is measured
/// INSIDE it: its content starts 35 from the panel's outer edge (`.sm-panel` x 680, its `h2` 715)
/// and 21 from the card's (`x 715`, its `.dot` 736). The row's own box is 490 wide, not 492.
const PANEL_BORDER: f32 = 1.0;
const ROW_PAD_X: f32 = 20.0 + PANEL_BORDER;
/// The row's `.who` block, from the card's top edge: the name's line box, then 3.06, then the
/// detail's — 44.88 tall in all, centred in the 82px card.
const NAME_Y: f32 = 22.0;
const DETAIL_Y: f32 = 44.0;
const DOT_D: f32 = 10.0;
const DOT_GAP: f32 = 16.0;
const TITLE_SZ: std::os::raw::c_int = 30;
const SUB_SZ: std::os::raw::c_int = 15;
const NAME_SZ: std::os::raw::c_int = 16;
const DETAIL_SZ: std::os::raw::c_int = 14;
const TAG_SZ: std::os::raw::c_int = 12;
/// `.sm-server .tag { letter-spacing: .1em }` at 12px.
const TAG_TRACK: f32 = 1.2;
const TITLE_H: f32 = 37.0;
const TITLE_GAP: f32 = 6.0;
const SUB_H: f32 = 38.0;
const SUB_GAP: f32 = 24.0;
/// The subtitle's LINE BOX is the reference's (its top is 78 from the panel, and the rows start off
/// that number), but this cut's ascent sits 2px higher inside an equal box than Chrome's does — the
/// prescaled 500 carries the reference's size, not its baseline. The ink is what is matched here.
const SUB_INK_DY: f32 = 2.0;
/// Same story for the tag's run, whose ink lands 2px high in the card.
const TAG_INK_DY: f32 = 2.0;
/// Content offset from the panel's OUTER edge, and the width it leaves.
const PANEL_INSET: f32 = PANEL_PAD + PANEL_BORDER;
const PANEL_INNER_W: f32 = PANEL_W - 2.0 * PANEL_INSET;

pub(crate) const SHAPE: &str = "AccountMenu{header:str,rows:[u32],sel:u32}";

/// The rows BELOW the configured-server list. **While an account is live: none.** The supplied
/// mockup's dialog is the servers and *Add server*, so this is the whole of it — the panel measures
/// 453px at three rows because nothing else is in it.
///
/// Neither door is lost by not repeating them here: *Sign out* is Settings' own **Desconectar** row
/// (`LoopReq::JfDisconnectServer`) and Settings opens from the bar's gear, which every bar-wearing
/// surface carries.
///
/// Signed OUT the menu still offers *Sign in*, and Settings beside it. The reference draws no
/// signed-out dialog, and the login gate is the only other home that press has.
fn rows_for(signed_in: bool) -> Vec<Action> {
    let mut rows = Vec::new();
    if !signed_in {
        rows.extend([Action::SignIn, Action::Settings]);
    }
    if crate::lab::menu_row_enabled() {
        rows.push(Action::SendDiagnostics);
    }
    rows
}

/// **What the profile CHIP calls the user** — the unfurled name beside the avatar, and the initial
/// inside it (its first character).
///
/// It lives here, not in `ui::widgets`, because it is a statement about the ACCOUNT and it has to
/// agree with the menu the chip opens. Every arm is one of this module's own answers:
///
/// - a name — the active managed profile, else the persisted roster's owner ([`Account::name`]);
/// - signed in and nameless — the localized Account label, the same word the menu heads itself with, which
///   is a missing NAME and not a missing user;
/// - signed out — the label of the one row the menu then offers, so the chip and the menu behind it
///   cannot say different things about the same press.
///
/// **The bug this replaced** was the chip deciding all three from `current().title.is_empty()`. An
/// account **without a roster** never gets a profile written at all, so that title is empty for a
/// signed-in owner and the chip offered them "Sign in" — which is the first thing a reviewer on a
/// fresh test account sees, and the last thing they should.
pub(crate) fn chip_label(acc: &Account) -> String {
    match (&acc.name, acc.signed_in) {
        (Some(n), _) => n.clone(),
        (None, true) => crate::i18n::msg::settings_account_title().to_string(),
        (None, false) => label(Action::SignIn).to_string(),
    }
}

/// The reference's subtitle wraps to the dialog's inner width; this splits on spaces and measures
/// with the frame's own capability, so the two lines land where the mockup puts them.
fn wrap_words(text: &str, budget: f32, sz: std::os::raw::c_int, measure: &dyn crate::ui::machine::Measure) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let mut probe = cur.clone();
        if !probe.is_empty() { probe.push(' '); }
        probe.push_str(word);
        let w = match std::ffi::CString::new(probe.clone()) {
            Ok(c) => measure.width(&c, sz, false),
            Err(_) => break,
        };
        if w > budget && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(word);
        } else {
            cur = probe;
        }
    }
    if !cur.is_empty() { lines.push(cur); }
    lines
}

/// **A server's address as the reference writes it**: `192.168.1.10 · Jellyfin 10.9` — the url, the
/// separator, and the server's own version cut to major.minor. The version is the one the last
/// connection learned ([`crate::jellyfin::known_server_version`]); a server this process has not
/// talked to yet says only its address, because inventing a version would be a lie in the row.
fn server_line(url: &str) -> String {
    match crate::jellyfin::known_server_version(url) {
        Some(v) => {
            let mut parts = v.split('.');
            let short = match (parts.next(), parts.next()) {
                (Some(major), Some(minor)) => format!("{major}.{minor}"),
                _ => v.clone(),
            };
            format!("{url} · Jellyfin {short}")
        }
        None => url.to_string(),
    }
}

/// **A run at a tracked pitch, right-aligned** — the reference's `letter-spacing` on the tag.
///
/// SDL_ttf composes a string at the font's own advances, so a tracked run has to be placed glyph by
/// glyph: [`crate::kern::glyphs`] gives each one's offset inside the run, and the pitch is that
/// offset plus `tracking` for every gap before it. The same per-glyph path the top bar's labels use
/// (`TabPill::draw_kerned`), which exists because FreeType's `kern` is ignored on this toolchain.
fn draw_tracked(
    p: crate::ui::Painter,
    text: &str,
    sz: std::os::raw::c_int,
    ink: [f32; 4],
    right: f32,
    cy: f32,
    tracking: f32,
) {
    let role = crate::text::FontRole::Interface;
    let Some((total, glyphs)) = crate::kern::glyphs(role, text, sz) else {
        return;
    };
    let n = glyphs.len().saturating_sub(1) as f32;
    let width = total + tracking * n;
    let x0 = right - width;
    for (i, g) in glyphs.iter().enumerate() {
        let Some(bytes) = text.as_bytes().get(g.at..g.at + g.len) else {
            return;
        };
        let mut buf = [0u8; 8];
        if bytes.len() >= buf.len() || buf.get_mut(..bytes.len()).is_none() {
            return;
        }
        buf[..bytes.len()].copy_from_slice(bytes);
        crate::ui::label::Label::new(buf.as_ptr() as *const std::os::raw::c_char, sz, ink)
            .face(role)
            .bold()
            .h(crate::ui::label::HAlign::Left)
            .v(crate::ui::label::VAlign::Middle)
            .draw(p, Rect::new(x0 + g.dx + i as f32 * tracking, cy - 10.0, 60.0, 20.0));
    }
}

fn label(a: Action) -> &'static str {
    match a {
        Action::SignIn => crate::i18n::msg::settings_account_sign_in(),
        Action::Settings => crate::i18n::msg::settings_account_settings(),
        Action::SendDiagnostics => crate::i18n::msg::settings_account_diagnostics(),
        Action::AddServer => crate::i18n::msg::settings_servers_add_password(),
        Action::None | Action::SwitchServer(_) => "",
    }
}

/// The row list IS the mapping — a selection outside it (an empty menu, a stale index) is `None`
/// rather than whatever action happens to sit at that position in the other row set.
fn action_at(rows: &[Action], sel: i32) -> Action {
    usize::try_from(sel)
        .ok()
        .and_then(|i| rows.get(i))
        .copied()
        .unwrap_or(Action::None)
}

/// The account sheet as the mockup's centred server dialog: a fixed-width panel centred on the
/// canvas, clear of the overscan frame. It is a dialog now, not a chip-tucked corner sheet, because
/// it lists every configured server — the same content `ServersPage` presents full-screen.
fn panel_rect(heights: &[f32]) -> Rect {
    let ph = rows_height(heights);
    let px = (crate::surface::LOGICAL_W - PANEL_W) * 0.5;
    let py = (crate::surface::LOGICAL_H - ph) * 0.5;
    Rect::new(px, py, PANEL_W, ph)
}

/// The dialog's own height: title, subtitle and the row stack inside the reference's padding.
fn rows_height(heights: &[f32]) -> f32 {
    // Every row carries its own 10px foot, the last one included — that is why the reference's
    // 3-row panel measures 453 and not 443.
    let n = heights.len().max(1) as f32;
    2.0 * (PANEL_BORDER + PANEL_PAD)
        + TITLE_H
        + TITLE_GAP
        + SUB_H
        + SUB_GAP
        + heights.iter().sum::<f32>()
        + n * ROW_GAP
}

/// Row `i`'s RESTING rect — the one focus and the hit map use. The focused row is drawn at
/// `.sm-server.is-focused`'s `scale(1.02)` about this rect's centre.
fn row_rect_in(panel: Rect, heights: &[f32], i: usize) -> Rect {
    let mut y = panel.y + PANEL_INSET + TITLE_H + TITLE_GAP + SUB_H + SUB_GAP;
    for h in heights.iter().take(i) {
        y += h + ROW_GAP;
    }
    Rect::new(
        panel.x + PANEL_INSET,
        y,
        PANEL_INNER_W,
        heights.get(i).copied().unwrap_or(ROW_H),
    )
}

/// What a row says: a server, the add door, or one of the app's own actions.
enum RowFace<'a> {
    Server { name: &'a str, detail: String, active: bool },
    Add { name: &'a str, detail: &'a str },
    Action { name: &'a str },
}

/// The panel at its TALLEST, for the overscan audit ([`crate::ui::consts::SAFE`]) — the clamp
/// ceiling rather than a measured height, since the audit grades the widest state a surface can be
/// in and the height comes from a `TableView` no host test can measure.
#[cfg(test)]
pub(crate) fn overscan_rects(out: &mut Vec<(&'static str, crate::ui::Rect)>) {
    let heights = [ROW_H, ROW_H, ROW_H, ROW_H_ADD];
    let r = panel_rect(&heights);
    out.push((
        "account menu panel",
        crate::ui::Rect::new(r.x, r.y, r.w, rows_height(&heights)),
    ));
}

pub(crate) struct AccountMenuScreen {
    entry: EntryId,
    header: String,
    rows: Vec<Action>,
    /// The configured servers, in the same order as the rows (server rows first, one per server).
    servers: Vec<crate::jellyfin::boot::ServerEntry>,
    active: usize,
    /// The highlighted row, as an index into [`Self::rows`].
    sel: i32,
    built: bool,
}

impl AccountMenuScreen {
    pub(crate) fn new(entry: EntryId) -> Self {
        Self {
            entry,
            header: crate::i18n::msg::settings_servers_change().to_string(),
            rows: Vec::new(),
            servers: Vec::new(),
            active: 0,
            sel: 0,
            built: false,
        }
    }

    /// Build from the configured servers and the installed Jellyfin client. An old account session
    /// is never an account source for this menu.
    fn build(&mut self) {
        if self.built {
            return;
        }
        self.built = true;
        let selected = action_at(&self.rows, self.sel);
        let name = crate::jellyfin::client().and_then(|client| client.user_name());
        let signed_in = name.is_some();
        // The real server list, exactly as `ServersPage` reads it: one row per configured server
        // (user + url), the active one checked, then the *Add by password* door.
        let (servers, active) = match crate::jellyfin::boot::load_servers() {
            Some(list) => (list.servers, list.active),
            None => (Vec::new(), 0),
        };
        self.servers = servers;
        self.active = active;
        self.rows.clear();
        for i in 0..self.servers.len() {
            self.rows.push(Action::SwitchServer(i));
        }
        self.rows.push(Action::AddServer);
        // The native additions follow, below the server choices: sign-in/sign-out, Settings, and
        // the compile-time lab action.
        self.rows.extend(rows_for(signed_in));
        self.header = crate::i18n::msg::settings_servers_change().to_string();

        // The reference's dialog has no scrollable list: it is a short stack of cards, and the
        // panel's height comes from the row count. Selection lands on the row that is safe to press:
        // the ACTIVE server, never *Add server*.
        self.sel = match self.rows.iter().position(|a| *a == selected) {
            Some(row) => row as i32,
            None => self.opening_row(),
        };
    }

    fn row_of(&self, elem: u32) -> Option<usize> {
        self.rows.iter().position(|action| action.key() == elem)
    }

    /// Each row's height, in order: the reference's own (82 for a server card, 84 for the add one).
    fn heights(&self) -> Vec<f32> {
        (0..self.rows.len())
            .map(|i| match self.face(i) {
                RowFace::Add { .. } => ROW_H_ADD,
                _ => ROW_H,
            })
            .collect()
    }

    fn frame(&self) -> Rect {
        panel_rect(&self.heights())
    }

    /// Commit the focused row. **Every action dismisses**, exactly as the legacy `on_ok` did by
    /// closing before it returned; what differs per action is the request the loop then performs.
    fn activate<H: AppLike>(&mut self, elem: u32, fx: &mut Effects<'_, H>) {
        let act = self.row_of(elem).map_or(Action::None, |row| self.rows[row]);
        let req = match act {
            Action::SignIn => Some(LoopReq::AccountSignIn),
            Action::Settings => Some(LoopReq::AccountSettings),
            Action::SendDiagnostics => Some(LoopReq::AccountSendDiagnostics),
            Action::SwitchServer(i) => Some(LoopReq::JfSwitchServer(i)),
            Action::AddServer => Some(LoopReq::JfAddServer),
            Action::None => None,
        };
        if let Some(req) = req {
            fx.push(Fx::App(AppFx::Loop(req)));
        }
        fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
    }

    /// The action a focus with nowhere better to go lands on — the table's opening row, which is
    /// never a destructive one while any other is on offer.
    fn opening_action(&self) -> Action {
        action_at(&self.rows, self.opening_row())
    }

    /// The row a freshly opened dialog lands on: the ACTIVE server's, which is the card the
    /// reference draws `.is-focused` on (`.sm-server.is-focused` is the `data-server` the screens
    /// are showing). A dialog with no configured server falls to the first row.
    fn opening_row(&self) -> i32 {
        self.rows
            .iter()
            .position(|a| matches!(a, Action::SwitchServer(i) if *i == self.active))
            .unwrap_or(0) as i32
    }

    /// What a row says, from the state — the paint and the tests read the same answer.
    fn face(&self, i: usize) -> RowFace<'_> {
        match self.rows.get(i) {
            Some(Action::SwitchServer(idx)) => {
                let server = self.servers.get(*idx);
                RowFace::Server {
                    name: server.map_or("", |s| s.user.as_str()),
                    detail: server.map_or(String::new(), |s| server_line(&s.url)),
                    active: *idx == self.active,
                }
            }
            Some(Action::AddServer) => RowFace::Add {
                name: crate::i18n::msg::settings_servers_add(),
                detail: crate::i18n::msg::settings_servers_add_detail(),
            },
            Some(a) => RowFace::Action { name: label(*a) },
            None => RowFace::Action { name: "" },
        }
    }

    /// The highlighted row, for the focus probe — a READ of the cursor the engine moves.
    pub(crate) fn sel(&self) -> i32 {
        self.sel
    }
}

impl<H: AppLike> Machine<H> for AccountMenuScreen {
    type Ev = ScreenEvent<H>;
    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        match ev {
            ScreenEvent::Mount => self.build(),
            ScreenEvent::Tick(tick) => {
                if !self.built {
                    self.build();
                    fx.invalidate(crate::ui::present::Provenance::Landing(crate::ui::machine::MachineId::Session));
                }
                self.sel = cx
                    .focus
                    .current
                    .filter(|key| key.entry == self.entry)
                    .and_then(|key| self.row_of(key.elem).map(|row| row as i32))
                    .unwrap_or(self.sel);
                let _ = tick;
            }
            ScreenEvent::FocusMoved { to, .. } => {
                if let Some(row) = self.row_of(to.elem) { self.sel = row as i32; }
            }
            ScreenEvent::Activate(elem) => self.activate(*elem, fx),
            ScreenEvent::PressCommit(_) => {
                if let Some(key) = cx.focus.current {
                    self.activate(key.elem, fx);
                }
            }
            ScreenEvent::Input(input) => {
                if let InputKind::Key {
                    key: Key::Back,
                    edge: Edge::Down,
                    ..
                } = input.kind
                {
                    fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
                    return Handled::Yes;
                }
            }
            _ => {}
        }
        Handled::No
    }
}

impl<H: AppLike> Focusable<H> for AccountMenuScreen {
    fn groups(&self, _: &Cx<'_, H>, out: &mut Vec<GroupSpec>) {
        out.push(GroupSpec {
            id: GroupId(0),
            kind: GroupKind::Column,
            seat: Seat::Remembered,
            reachable: AxisMask::BOTH,
            edge: [EdgeRule::Stop; 4],
            extent: self.frame(),
            len: self.rows.len(),
            elem: ElemKind::Bare,
        });
    }
    fn group_of(&self, elem: &u32, _: &Cx<'_, H>) -> Option<GroupId> {
        self.row_of(*elem).map(|_| GroupId(0))
    }
    fn neighbour(&self, key: FocusKey<u32>, dir: Dir, _: &Cx<'_, H>) -> Step<u32> {
        let Some(row) = self.row_of(key.elem) else { return Step::Edge };
        let next = match dir {
            Dir::Up => row.checked_sub(1),
            Dir::Down => Some(row + 1),
            _ => None,
        };
        match next.filter(|i| *i < self.rows.len()) {
            Some(i) => Step::Move(FocusKey {
                entry: self.entry,
                elem: self.rows[i].key(),
            }),
            None => Step::Edge,
        }
    }
    fn place(&self, elem: &u32, _: &Cx<'_, H>, _: At) -> Option<Placed> {
        let row = self.row_of(*elem)?;
        let frame = self.frame();
        let rest = row_rect_in(frame, &self.heights(), row);
        // `.sm-server.is-focused { transform: scale(1.02) }` — a static transform about the card's
        // own centre, not a spring: the reference's row grows in place the moment it holds focus.
        let rect = if self.sel == row as i32 { rest.scaled(1.02) } else { rest };
        Some(Placed {
            rect,
            rest_rect: rest,
            clip: frame,
            index: Some(row as u32),
        })
    }
    fn reconcile(&self, want: FocusKey<u32>, cx: &Cx<'_, H>) -> FocusKey<u32> {
        if self.group_of(&want.elem, cx).is_some() {
            want
        } else {
            FocusKey {
                entry: self.entry,
                elem: self.opening_action().key(),
            }
        }
    }
    fn seat(&self, _: GroupId, _: Placed, _: &Cx<'_, H>) -> FocusKey<u32> {
        FocusKey {
            entry: self.entry,
            elem: action_at(&self.rows, self.sel).key(),
        }
    }
}

impl<H: AppLike> Screen<H> for AccountMenuScreen {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn name(&self) -> &'static str {
        crate::screens::registry::word::ACCOUNT
    }
    fn state(&self) -> &dyn LogicalState {
        self
    }
    fn crumb(&self, _: &Cx<'_, H>) -> Option<Cow<'_, str>> {
        None
    }
    /// The modal dim. The shared Back control takes the chip's top-left frame, so the chip is
    /// deliberately not lifted over it. The menu still hangs from that same place.
    fn scrim(&self) -> Scrim {
        // The SHEET role: how dark the page goes behind this menu, the peak the container ramps
        // with the appear spring (`ModalStack::draw_scrims`).
        Scrim::dim(crate::ui::theme::underlay::DIM_SHEET)
    }
    fn prepare(&mut self, _: &mut Budget, _: &Cx<'_, H>) {}
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, H>) {
        let p = f.painter.alpha(f.page_alpha);
        let r = self.frame();
        let measure = f.measure;
        // ---- the reference's card: opaque, warm, 24px, one 14% hairline ----------------------
        p.focus_shadow(r, PANEL_RAD, 1.0);
        p.rect_rimmed(
            r,
            PANEL_RAD,
            crate::ui::theme::CHROME_PANEL,
            crate::ui::theme::CHROME_PANEL,
            crate::ui::theme::CHROME_PANEL_RIM,
            0.0,
        );
        // ---- title (Fraunces 600 30) and the line under it ----------------------------------
        let title = std::ffi::CString::new(self.header.as_str()).unwrap_or_default();
        crate::ui::label::Label::new(title.as_ptr(), TITLE_SZ, crate::ui::theme::CHROME_INK)
            .display()
            .h(crate::ui::label::HAlign::Left)
            .draw(p, Rect::new(r.x + PANEL_INSET, r.y + PANEL_INSET, PANEL_INNER_W, TITLE_H));
        let sub = std::ffi::CString::new(crate::i18n::msg::settings_servers_change_detail())
            .unwrap_or_default();
        let sub_y = r.y + PANEL_INSET + TITLE_H + TITLE_GAP;
        let mut line_y = sub_y;
        for line in wrap_words(
            crate::i18n::msg::settings_servers_change_detail(),
            PANEL_INNER_W,
            SUB_SZ,
            measure,
        ) {
            let cs = std::ffi::CString::new(line).unwrap_or_default();
            crate::ui::label::Label::new(cs.as_ptr(), SUB_SZ, crate::ui::theme::CHROME_MUTED)
                .face(crate::text::FontRole::InterfaceSubTight)
                .h(crate::ui::label::HAlign::Left)
                .draw(p, Rect::new(r.x + PANEL_INSET, line_y + SUB_INK_DY, PANEL_INNER_W, 19.0));
            line_y += 19.0;
        }
        let _ = sub;
        // ---- the row stack -------------------------------------------------------------------
        let heights = self.heights();
        for i in 0..self.rows.len() {
            let rest = row_rect_in(r, &heights, i);
            let focused = self.sel == i as i32;
            let q = if focused { rest.scaled(1.02) } else { rest };
            let (fill, rim) = if focused {
                (crate::ui::theme::CHROME_ROW_FOCUS, crate::ui::theme::CHROME_ROW_FOCUS_RIM)
            } else {
                (crate::ui::theme::CHROME_ROW_FILL, crate::ui::theme::CHROME_ROW_RIM)
            };
            p.rect_rimmed(q, ROW_RAD, fill, fill, rim, 0.0);
            let face = self.face(i);
            let mut x = q.x + ROW_PAD_X;
            match &face {
                RowFace::Server { active, .. } => {
                    let dot = Rect::new(x, q.cy() - DOT_D * 0.5, DOT_D, DOT_D);
                    let col = if *active {
                        crate::ui::theme::CHROME_OK
                    } else {
                        crate::ui::theme::with_a(crate::ui::theme::CHROME_OK, 0.25)
                    };
                    p.rrect(dot, DOT_D * 0.5, DOT_D * 0.5, col);
                    x += DOT_D + DOT_GAP;
                }
                _ => {}
            }
            let (name, detail, active, tag) = match &face {
                RowFace::Server { name, detail, active } => {
                    (*name, Some(detail.as_str()), *active, *active)
                }
                RowFace::Add { name, detail } => (*name, Some(*detail), false, false),
                RowFace::Action { name } => (*name, None, false, false),
            };
            let name_text = match &face {
                RowFace::Add { .. } => format!("+ {name}"),
                _ => name.to_string(),
            };
            let name_cs = std::ffi::CString::new(name_text).unwrap_or_default();
            // `.sm-server .who b` computes to **900** — Manrope's ExtraBold cut, one notch above the
            // avatar's 800 and two above the interface's 700. It is the heaviest cut the reference
            // asks for anywhere and the one that makes a server name read as a heading.
            crate::ui::label::Label::new(name_cs.as_ptr(), NAME_SZ, crate::ui::theme::CHROME_INK)
                .face(crate::text::FontRole::Avatar)
                .h(crate::ui::label::HAlign::Left)
                .draw(p, Rect::new(x, q.y + NAME_Y, q.x + q.w - ROW_PAD_X - x, 22.0));
            if let Some(detail) = detail.filter(|d| !d.is_empty()) {
                let dcs = std::ffi::CString::new(detail).unwrap_or_default();
                let detail_sz = match &face {
                    RowFace::Add { .. } => DETAIL_SZ_ADD,
                    _ => DETAIL_SZ,
                };
                let detail_y = match &face {
                    RowFace::Add { .. } => DETAIL_Y - 1.0,
                    _ => DETAIL_Y,
                };
                let detail_face = match &face {
                    RowFace::Add { .. } => crate::text::FontRole::InterfaceSub,
                    _ => crate::text::FontRole::InterfaceSubTight,
                };
                crate::ui::label::Label::new(dcs.as_ptr(), detail_sz, crate::ui::theme::CHROME_MUTED)
                    .face(detail_face)
                    .h(crate::ui::label::HAlign::Left)
                    .draw(p, Rect::new(x, q.y + detail_y, q.x + q.w - ROW_PAD_X - x, 20.0));
            }
            if tag {
                let t = crate::i18n::msg::settings_servers_active_tag().to_uppercase();
                // `.sm-server .tag` sets `letter-spacing:.1em` — 1.2px at its 12px size. SDL_ttf has
                // no tracking, so the run is drawn glyph by glyph at the reference's own pitch,
                // right-aligned: measuring it as one string leaves the tag 9px narrow.
                let right = q.x + q.w - ROW_PAD_X;
                draw_tracked(
                    p,
                    &t,
                    TAG_SZ,
                    crate::ui::theme::CHROME_GOLD,
                    right,
                    q.cy() + TAG_INK_DY,
                    TAG_TRACK,
                );
                let _ = active;
            }
        }
        for action in &self.rows {
            let elem = action.key();
            if let Some(placed) = <Self as Focusable<H>>::place(self, &elem, f.cx, At::Drawn) {
                f.stop(
                    p,
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
}

impl LogicalState for AccountMenuScreen {
    fn write(&self, c: &mut Canon) {
        c.str(&self.header).seq(self.rows.len());
        for a in &self.rows {
            c.u32(a.key());
        }
        c.u32(self.sel as u32);
    }
    fn probe(&self, out: &mut String) {
        out.push_str("account_menu");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_out_menu_offers_the_server_dialog_with_sign_in_and_settings() {
        let _guard = crate::testlock::serial();
        crate::jellyfin::uninstall();
        let mut menu = AccountMenuScreen::new(EntryId(0));
        menu.build();
        // The reference's dialog plus the one press a signed-out menu has nowhere else.
        assert_eq!(menu.rows, vec![Action::AddServer, Action::SignIn, Action::Settings]);
    }

    #[test]
    fn the_jellyfin_client_builds_the_menu_without_a_session() {
        let _guard = crate::testlock::serial();
        let server = crate::jellyfin::MockServer::start(vec![
            (200, r#"{"User":{"Id":"u-1","Name":"demo"},"AccessToken":"tok-1","ServerId":"srv-1","SessionInfo":null}"#.to_string()),
        ]);
        let client = crate::jellyfin::JfClient::new(
            crate::origin::Origin::http("127.0.0.1", server.port as i32), "dev-1".into());
        client.authenticate_by_name("demo", "").unwrap();
        crate::jellyfin::install(client);

        let mut menu = AccountMenuScreen::new(EntryId(0));
        menu.build();
        assert!(menu.built);
        assert_eq!(menu.header, crate::i18n::msg::settings_servers_change());
        // **Signed in, the dialog is exactly the mockup's**: the server cards and *Add server*.
        // `Cerrar sesión` and `Ajustes` are not rows here — they live in Settings, off the gear.
        assert_eq!(menu.rows, vec![Action::AddServer]);

        crate::jellyfin::uninstall();
        let _ = server.finish();
    }

    /// **The reference's own panel arithmetic**, from its DOM at 1920x1080: `.sm-panel` is
    /// `2*1px border + 2*34 padding + 37 h2 + 6 + 38 sub + 24` = 175, then every card is 82 tall and
    /// carries a 10px foot — the last one included — which is why the mockup's three-row dialog
    /// measures **453** and not 439.
    #[test]
    fn the_dialog_is_the_references_measured_panel() {
        let heights = [ROW_H, ROW_H, ROW_H_ADD];
        assert_eq!(rows_height(&heights), 453.0);
        // Centred: the mockup's own panel lands at x 680, y 313.5 on the 1920x1080 canvas.
        let three = panel_rect(&heights);
        assert_eq!((three.x, three.y, three.w, three.h), (680.0, 313.5, 560.0, 453.0));
        // Rows start 35 in from the panel's outer edge (1px border + 34 padding) and are 490 wide,
        // and the first one begins directly under the subtitle's 24px foot.
        let first = row_rect_in(three, &heights, 0);
        assert_eq!((first.x, first.y, first.w, first.h), (715.0, 453.5, 490.0, 82.0));
        let third = row_rect_in(three, &heights, 2);
        assert_eq!((third.y, third.h), (453.5 + 2.0 * (82.0 + 10.0), 84.0));
    }

    /// A server's row says its address and the server's own version, the way the reference's do
    /// (`192.168.1.10 · Jellyfin 10.9`), with the process's memo consulted — never a probe from a
    /// paint pass. An unknown server says only its address rather than inventing a version.
    #[test]
    fn a_server_row_carries_the_version_its_last_connection_learned() {
        let unknown = "http://never-contacted.invalid:8096";
        assert_eq!(server_line(unknown), unknown);
        crate::jellyfin::note_server_version(unknown, "10.10.7");
        assert_eq!(server_line(unknown), format!("{unknown} · Jellyfin 10.10"));
        crate::jellyfin::note_server_version("http://no-minor.invalid", "11");
        assert_eq!(
            server_line("http://no-minor.invalid"),
            "http://no-minor.invalid · Jellyfin 11"
        );
    }

    #[test]
    fn selection_maps_by_the_drawn_row_list() {
        let signed_out = rows_for(false);
        assert_eq!(action_at(&signed_out, 0), Action::SignIn);
        assert_eq!(action_at(&signed_out, 1), Action::Settings);
        assert_eq!(action_at(&signed_out, 2), Action::None);
        let signed_in = rows_for(true);
        assert!(
            signed_in.is_empty(),
            "signed in, the mockup's dialog adds nothing below its server cards"
        );
        assert_eq!(action_at(&signed_in, 0), Action::None);
        assert_eq!(action_at(&signed_in, -1), Action::None);
    }
}
