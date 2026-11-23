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
use crate::ui::table::{Row, Section, TableView};
use crate::ui::Rect;

/// What the highlighted row does on OK.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    None,
    SignIn,
    SignOut,
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
            Action::SignOut => 2,
            Action::Settings => 3,
            Action::SendDiagnostics => 4,
            Action::SwitchServer(i) => SERVER_BASE + i as u32,
            Action::AddServer => ADD_ELEM,
        }
    }
}


/// The pinned ~24px corner radius.
const PANEL_RAD: f32 = 24.0;

pub(crate) const SHAPE: &str =
    "AccountMenu{header:str,rows:[u32],sel:u32,table:TableViewMotion}";

/// The rows for a Jellyfin account state, in order, BELOW the configured-server list. Signed out
/// offers sign-in; signed in offers sign-out. Settings and the compile-time lab action are
/// independent of account state.
fn rows_for(signed_in: bool) -> Vec<Action> {
    let mut rows = Vec::new();
    match (signed_in, crate::lab::menu_row_enabled()) {
        (false, false) => rows.extend([Action::SignIn, Action::Settings]),
        (false, true) => rows.extend([Action::SignIn, Action::Settings, Action::SendDiagnostics]),
        (true, false) => rows.extend([Action::SignOut, Action::Settings]),
        (true, true) => rows.extend([Action::SignOut, Action::Settings, Action::SendDiagnostics]),
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

fn label(a: Action) -> &'static str {
    match a {
        Action::SignIn => crate::i18n::msg::settings_account_sign_in(),
        Action::SignOut => crate::i18n::msg::settings_account_sign_out(),
        Action::Settings => crate::i18n::msg::settings_account_settings(),
        Action::SendDiagnostics => crate::i18n::msg::settings_account_diagnostics(),
        Action::AddServer => crate::i18n::msg::settings_servers_add_password(),
        Action::None | Action::SwitchServer(_) => "",
    }
}

/// Rows that leave for another screen carry the drill-in chevron; "Sign out" acts in place.
fn drills_in(a: Action) -> bool {
    matches!(a, Action::SignIn | Action::Settings | Action::AddServer)
}

/// Rows whose action ends something in place. Never where the menu's focus starts
/// ([`crate::ui::table::TableView::opening_row`]): *Sign out* is the FIRST row while signed in,
/// and a stray OK on a freshly opened menu must not sign anyone out.
fn destructive(a: Action) -> bool {
    matches!(a, Action::SignOut)
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
fn panel_rect(table: &TableView) -> Rect {
    let pw = 560.0f32;
    let px = (crate::surface::LOGICAL_W - pw) * 0.5;
    let ph = table.measured_height().clamp(120.0, 520.0);
    let py = (crate::surface::LOGICAL_H - ph) * 0.5;
    Rect::new(px, py, pw, ph)
}

/// The panel at its TALLEST, for the overscan audit ([`crate::ui::consts::SAFE`]) — the clamp
/// ceiling rather than a measured height, since the audit grades the widest state a surface can be
/// in and the height comes from a `TableView` no host test can measure.
#[cfg(test)]
pub(crate) fn overscan_rects(out: &mut Vec<(&'static str, crate::ui::Rect)>) {
    let r = panel_rect(&TableView::new());
    out.push((
        "account menu panel",
        crate::ui::Rect::new(r.x, r.y, r.w, 440.0),
    ));
}

pub(crate) struct AccountMenuScreen {
    entry: EntryId,
    header: String,
    rows: Vec<Action>,
    /// The configured servers, in the same order as the rows (server rows first, one per server).
    servers: Vec<crate::jellyfin::boot::ServerEntry>,
    active: usize,
    table: TableView,
    built: bool,
}

impl AccountMenuScreen {
    pub(crate) fn new(entry: EntryId) -> Self {
        Self {
            entry,
            header: crate::i18n::msg::settings_servers_select().to_string(),
            rows: Vec::new(),
            servers: Vec::new(),
            active: 0,
            table: TableView::new(),
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
        let selected = action_at(&self.rows, self.table.sel);
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
        self.header = crate::i18n::msg::settings_servers_select().to_string();

        let mut sec = Section::new(self.header.clone());
        for (i, a) in self.rows.iter().enumerate() {
            let row = match a {
                Action::SwitchServer(idx) => {
                    let server = self.servers.get(*idx);
                    Row::new(server.map_or_else(String::new, |s| s.user.clone()))
                        .detail(server.map_or_else(String::new, |s| s.url.clone()))
                        .checked(*idx == self.active)
                }
                _ => Row::new(label(*a).to_string())
                    .chevron(drills_in(*a))
                    .destructive(destructive(*a)),
            };
            sec = sec.row(row);
            let _ = i;
        }
        self.table.compact = true;
        match self.rows.iter().position(|a| *a == selected) {
            Some(row) => self.table.set_sections(vec![sec], row as i32, false),
            None => self.table.open_sections(vec![sec]),
        }
        debug_assert_eq!(self.rows.len() as i32, self.table.n_rows());
    }

    fn row_of(&self, elem: u32) -> Option<usize> {
        self.rows.iter().position(|action| action.key() == elem)
    }

    fn frame(&self) -> Rect {
        panel_rect(&self.table)
    }

    /// Commit the focused row. **Every action dismisses**, exactly as the legacy `on_ok` did by
    /// closing before it returned; what differs per action is the request the loop then performs.
    fn activate<H: AppLike>(&mut self, elem: u32, fx: &mut Effects<'_, H>) {
        let act = self.row_of(elem).map_or(Action::None, |row| self.rows[row]);
        let req = match act {
            Action::SignIn => Some(LoopReq::AccountSignIn),
            Action::SignOut => Some(LoopReq::AccountSignOut),
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
        action_at(&self.rows, self.table.opening_row())
    }

    /// The highlighted row, for the focus probe — a READ of the cursor the engine moves.
    pub(crate) fn sel(&self) -> i32 {
        self.table.sel
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
                self.table.sel = cx
                    .focus
                    .current
                    .filter(|key| key.entry == self.entry)
                    .and_then(|key| self.row_of(key.elem).map(|row| row as i32))
                    .unwrap_or(self.table.sel);
                self.table.update(tick.dt(), self.frame().h);
            }
            ScreenEvent::FocusMoved { to, .. } => {
                if let Some(row) = self.row_of(to.elem) { self.table.sel = row as i32; }
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
        let rect = self.table.row_frame(self.frame(), row as i32)?;
        Some(Placed {
            rect,
            rest_rect: rect,
            clip: self.frame(),
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
            elem: action_at(&self.rows, self.table.sel).key(),
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
        crate::ui::widgets::panel_ground(p, r, PANEL_RAD, f.underlay);
        crate::ui::profile::phase("glass.foreground", || {
            self.table.draw(p, r, measure);
        });
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
        c.u32(self.table.sel as u32);
        self.table.write_motion(c);
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
        assert_eq!(menu.rows, vec![Action::AddServer, Action::SignIn, Action::Settings]);
        assert!(!menu.rows.contains(&Action::SignOut));
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
        assert_eq!(menu.header, crate::i18n::msg::settings_servers_select());
        assert_eq!(menu.rows, vec![Action::AddServer, Action::SignOut, Action::Settings]);

        crate::jellyfin::uninstall();
        let _ = server.finish();
    }

    #[test]
    fn selection_maps_by_the_drawn_row_list() {
        let signed_out = rows_for(false);
        assert_eq!(action_at(&signed_out, 0), Action::SignIn);
        assert_eq!(action_at(&signed_out, 1), Action::Settings);
        assert_eq!(action_at(&signed_out, 2), Action::None);
        let signed_in = rows_for(true);
        assert_eq!(action_at(&signed_in, 0), Action::SignOut);
        assert_eq!(action_at(&signed_in, 1), Action::Settings);
        assert_eq!(action_at(&signed_in, 2), Action::None);
        assert_eq!(action_at(&signed_in, -1), Action::None);
    }
}
