//! The Jellyfin flavor's sign-in screen: a form, not a QR (flavor `jellyfin` only — this module
//! does not exist on a Plex build). Three fields (server, user name, password) and a Connect row,
//! drawn with the settings-table widget so focus, scrolling and the selection pill come from code
//! that already ships. OK on a field raises the television's own keyboard (`crate::textinput`);
//! OK again — or BACK — commits it. The connect itself is [`crate::jellyfin::signin`]'s worker;
//! this screen only mirrors its phase.
//!
//! Mounted in place of the QR screen on this build: the route word is still `"login"` (the
//! heartbeat stays byte-identical, spec §15.3), and the boot gate and the account menu's sign-in
//! both land here exactly as they landed on `Route::Login` before. BACK stays swallowed — a
//! first-ever boot has nothing behind this screen, same rule the QR screen documents.
#![cfg(feature = "jellyfin")]

use crate::jellyfin::signin::{self, Fail, Phase};
use super::family::table_focus;
use crate::screens::registry::AppLike;
use crate::ui::machine::{Cx, Effects, EntryId, FocusKey, GroupId, Handled, Machine};
use crate::ui::route_screen::{RouteGround, RouteLayout};
use crate::ui::screen::{
    At, Dir, DrawFrame, Focusable, Placed, Screen, ScreenEvent, Step,
};
use crate::ui::table_screen::{Header, TableScreen};
use crate::ui::table::{Row, Section, TableView};
use crate::ui::text_view::TextView;
use crate::ui::widgets::Spinner;
use crate::ui::{theme, Env, Painter, Rect, View};

/// Field indices are also their TABLE ROW indices — the Connect row is the fourth.
const F_SERVER: usize = 0;
const F_USER: usize = 1;
const F_PASS: usize = 2;
const ROW_CONNECT: u32 = 3;

fn labels() -> [&'static str; 3] {
    [
        crate::i18n::msg::browse_jellyfin_server(),
        crate::i18n::msg::browse_jellyfin_user_name(),
        crate::i18n::msg::browse_jellyfin_password(),
    ]
}
/// What an untouched field says. The server's is an example (the one thing a person must invent
/// is also the one with a shape worth showing); the other two state their requirement.
// The password hint is honest about the rule `connectable` actually applies: plenty of LAN
// Jellyfin users have no password, and the field must not claim otherwise.
pub(crate) struct JfLoginScreen {
    entry: EntryId,
    ground: RouteGround,
    table: TableView,
    fields: [String; 3],
    /// Which field the TV keyboard is editing, if it is up.
    editing: Option<usize>,
    /// The last failed attempt's reason — from `signin`'s phase, or a validation the screen
    /// itself did (empty fields) without ever starting one.
    fail: Option<Fail>,
    /// Rebuild the table's rows next tick — text landed, or the phase changed under them.
    dirty: bool,
    spin_ms: f32,
    /// The butaca wordmark from the app directory, as a GL texture (0 = not tried / not there).
    logo_tex: u32,
    logo_tried: bool,
    logo_px: (u32, u32),
}

impl JfLoginScreen {
    pub(crate) fn new(entry: EntryId) -> Self {
        // Prefill from the persisted config: a boot that lands here DESPITE one means the server
        // refused or was gone, and re-typing credentials on a remote is the worst possible tax
        // for it. The password prefills too — this television is the credential store, and the
        // file it is read from is 0600 beside the session.
        let fields = match crate::jellyfin::boot::stored_config() {
            Some((url, user, password)) => [url, user, password],
            None => Default::default(),
        };
        let mut screen = Self {
            entry,
            ground: RouteGround::new(),
            table: TableView::new(),
            fields,
            editing: None,
            fail: None,
            dirty: false,
            spin_ms: 0.0,
            logo_tex: 0,
            logo_tried: false,
            logo_px: (0, 0),
        };
        // The rows are built HERE, not on the first Tick: the mount's focus seat reads
        // `groups()` before any Tick has run, and an empty table would leave the new root with
        // nothing to seat on.
        screen.rebuild();
        screen
    }

    fn view(&self) -> TableScreen<'_> {
        TableScreen::new(
            Header::new(
                RouteLayout::screen(),
                None,
                crate::i18n::msg::browse_jellyfin_sign_in_title(),
                crate::i18n::msg::browse_jellyfin_sign_in_explainer(),
            ),
            &self.table,
            GroupId(0),
            self.entry,
        )
    }

    /// Mount/reset: fresh ground, keyboard down, and the flow's phase respected — a Working
    /// attempt (sign-out -> sign-in keeps the module warm) keeps its spinner rather than being
    /// lied about.
    fn reset_for_entry(&mut self) {
        self.ground.reset();
        self.editing = None;
        crate::textinput::stop();
        self.dirty = true;
    }

    /// The form as table sections: one section of fields, one of the action. Rebuilt on change
    /// only — `TableView`'s springs live across these, `slide=true` keeps the pill where it was.
    fn build_rows(&self) -> Vec<Section> {
        let working = signin::phase() == Phase::Working;
        let mut fields = Section::new("");
        for (i, label) in labels().iter().enumerate() {
            let empty = self.fields[i].is_empty();
            let value = if empty {
                [
            crate::i18n::msg::browse_jellyfin_hint_address(),
            crate::i18n::msg::browse_jellyfin_hint_required(),
            crate::i18n::msg::browse_jellyfin_hint_optional(),
        ][i]
        .to_string()
            } else if i == F_PASS {
                masked(&self.fields[i])
            } else {
                self.fields[i].clone()
            };
            fields = fields.row(Row::new(*label).value(value).value_dim(empty).dim(working));
        }
        let action = Section::new("").row(
            Row::new(crate::i18n::msg::settings_plaintext_connect())
                .chevron(true)
                .dim(working || !connectable(&self.fields)),
        );
        vec![fields, action]
    }

    fn rebuild(&mut self) {
        let rows = self.build_rows();
        let sel = self.table.sel;
        self.table.set_sections(rows, sel, true);
        self.dirty = false;
    }

    /// OK (or the pointer) landed on a row. Rows 0..3 are fields; row 3 is Connect.
    fn activate(&mut self, elem: u32, fx: &mut Effects<'_, impl AppLike>) {
        if signin::phase() == Phase::Working {
            return; // an attempt is out; the form is inert until it answers
        }
        if elem == ROW_CONNECT {
            if !connectable(&self.fields) {
                // Saying WHY costs a failure line; doing nothing reads as a dead button.
                self.fail = Some(Fail::Parse);
                self.dirty = true;
            } else {
                let [url, user, pass] = &self.fields;
                if let Err(f) = signin::start(url, user, pass) {
                    self.fail = Some(f);
                } else {
                    self.fail = None;
                }
                self.dirty = true;
            }
        } else if (0..3).contains(&elem) {
            self.editing = Some(elem as usize);
            crate::textinput::start();
        }
        fx.invalidate(crate::ui::present::Provenance::Input);
    }

    /// Load the wordmark once, on the render thread (GL upload). A missing asset degrades to a
    /// text wordmark — the screen still brands itself, just less prettily.
    fn prepare_logo(&mut self) {
        if self.logo_tried {
            return;
        }
        self.logo_tried = true;
        let path = crate::paths::in_app_dir("butaca-name.png");
        if let Ok(png) = std::fs::read(&path) {
            let (mut w, mut h): (std::os::raw::c_int, std::os::raw::c_int) = (0, 0);
            let px = crate::img::img_decode_rgba(png.as_ptr(), png.len() as _, &mut w, &mut h);
            if !px.is_null() {
                self.logo_tex = crate::img::img_upload_rgba(px, w, h);
                self.logo_px = if self.logo_tex != 0 {
                    (w.max(0) as u32, h.max(0) as u32)
                } else {
                    (0, 0)
                };
                crate::img::img_free(px);
            }
        }
    }
}

/// The password's on-row rendering. Masked ALWAYS, including while editing: the row is drawn
/// under the system keyboard, which shows the real text itself — doubling it in clear on our
/// surface would only make shoulder-surfing easier. Dots count CHARACTERS, not bytes.
fn masked(s: &str) -> String {
    "•".repeat(s.chars().count().min(24))
}

/// The one validation this screen does without the flow: Connect needs a server and a name.
/// The password may legitimately be empty (a LAN-only account), so it is not in the rule.
fn connectable(fields: &[String; 3]) -> bool {
    !fields[F_SERVER].trim().is_empty() && !fields[F_USER].trim().is_empty()
}

/// What the keyboard committed, appended to the field being edited. Newlines and control
/// characters are dropped — the panel's Enter ends editing through the key path, so text
/// carrying one is paste debris, not intent.
fn commit_text(fields: &mut [String; 3], editing: Option<usize>, text: &str) -> bool {
    let Some(f) = editing else { return false };
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    if clean.is_empty() {
        return false;
    }
    fields[f].push_str(&clean);
    true
}

/// The reason line under the narrative — one sentence per failure, worded for a couch, not a
/// console. The two address failures are told apart because their fixes are different verbs:
/// retype it, or serve it over TLS / from the LAN.
fn fail_text(f: Fail) -> &'static str {
    match f {
        Fail::Parse => {
            crate::i18n::msg::browse_jellyfin_error_address()
        }
        Fail::Plaintext => {
            crate::i18n::msg::browse_jellyfin_error_https()
        }
        Fail::Refused => crate::i18n::msg::browse_jellyfin_error_auth(),
        Fail::Unreachable => {
            crate::i18n::msg::browse_jellyfin_error_reach()
        }
    }
}

impl crate::ui::machine::LogicalState for JfLoginScreen {
    fn write(&self, c: &mut crate::ui::machine::Canon) {
        c.u32(self.table.sel as u32);
        c.u8(self.editing.map_or(3, |f| f as u8));
        c.u8(self.fail.map_or(0, |f| f as u8 + 1));
    }
    fn probe(&self, out: &mut String) {
        out.push_str("jf_login");
    }
}

impl<H: AppLike> Machine<H> for JfLoginScreen {
    type Ev = ScreenEvent<H>;
    fn step(&mut self, ev: &Self::Ev, _cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        use crate::ui::machine::{Edge, InputKind, Key};
        match ev {
            ScreenEvent::Enter(_) => {
                self.reset_for_entry();
                Handled::Yes
            }
            ScreenEvent::Tick(t) => {
                self.spin_ms += t.dt() * 1000.0;
                // Typed text first — the frame that receives a character draws it.
                for text in crate::textinput::drain() {
                    if commit_text(&mut self.fields, self.editing, &text) {
                        self.dirty = true;
                    }
                }
                // Mirror the flow's phase into the one fact the draw reads. Editing means no
                // attempt has failed since the last keypress; a stale failure is cleared the
                // moment a new attempt runs.
                match signin::phase() {
                    Phase::Working | Phase::Ready => {
                        if self.fail.is_some() {
                            self.fail = None;
                            self.dirty = true;
                        }
                    }
                    Phase::Failed(f) => {
                        if self.fail != Some(f) {
                            self.fail = Some(f);
                            self.dirty = true;
                        }
                    }
                    Phase::Editing => {}
                }
                if self.dirty {
                    self.rebuild();
                }
                self.table.update(t.dt(), RouteLayout::screen().sectioned_table().h);
                Handled::Yes
            }
            ScreenEvent::FocusMoved { to, .. } => {
                table_focus(&mut self.table, to.elem);
                Handled::Yes
            }
            ScreenEvent::Activate(elem) => {
                // The panel's edit protocol: while a field edit is up, OK commits and closes the
                // keyboard instead of re-activating the row beneath it.
                if self.editing.is_some() {
                    self.editing = None;
                    crate::textinput::stop();
                    fx.invalidate(crate::ui::present::Provenance::Input);
                    Handled::Yes
                } else {
                    self.activate(*elem, fx);
                    Handled::Yes
                }
            }
            ScreenEvent::Input(input) => match input.kind {
                // While editing, backspace/clear arrive as keys and edit the field; OK and BACK
                // both commit and close (Activate handles OK's row half, this is the raw key).
                InputKind::Key { key, sym, edge: Edge::Down, .. } => {
                    if let Some(f) = self.editing {
                        // The panel's edit protocol, per search's key handler: backspace and
                        // clear arrive as keys (the system keyboard's own codes), the printable
                        // text as SDL_TEXTINPUT.
                        match (key, sym) {
                            (Key::Other, crate::ui::consts::SDLK_BACKSPACE) => {
                                self.fields[f].pop();
                                self.dirty = true;
                            }
                            (Key::Other, crate::ui::consts::SDLK_CLEAR) => {
                                self.fields[f].clear();
                                self.dirty = true;
                            }
                            _ => {}
                        }
                        fx.invalidate(crate::ui::present::Provenance::Input);
                        Handled::Yes
                    } else {
                        // BACK with no edit in flight is swallowed on purpose — see the module
                        // doc. Nothing lives behind the first-ever boot's sign-in.
                        (key == Key::Back).then_some(Handled::Yes).unwrap_or(Handled::No)
                    }
                }
                _ => Handled::No,
            },
            _ => Handled::No,
        }
    }
}

impl<H: AppLike> Focusable<H> for JfLoginScreen {
    fn groups(&self, cx: &Cx<'_, H>, out: &mut Vec<crate::ui::screen::GroupSpec>) {
        Focusable::<H>::groups(&self.view(), cx, out)
    }
    fn group_of(&self, key: &H::Elem, cx: &Cx<'_, H>) -> Option<GroupId> {
        Focusable::<H>::group_of(&self.view(), key, cx)
    }
    fn neighbour(&self, key: FocusKey<H::Elem>, dir: Dir, cx: &Cx<'_, H>) -> Step<H::Elem> {
        Focusable::<H>::neighbour(&self.view(), key, dir, cx)
    }
    fn place(&self, key: &H::Elem, cx: &Cx<'_, H>, at: At) -> Option<Placed> {
        Focusable::<H>::place(&self.view(), key, cx, at)
    }
    fn reconcile(&self, want: FocusKey<H::Elem>, cx: &Cx<'_, H>) -> FocusKey<H::Elem> {
        Focusable::<H>::reconcile(&self.view(), want, cx)
    }
    fn seat(&self, g: GroupId, from: Placed, cx: &Cx<'_, H>) -> FocusKey<H::Elem> {
        Focusable::<H>::seat(&self.view(), g, from, cx)
    }
}

impl<H: AppLike> Screen<H> for JfLoginScreen {
    fn name(&self) -> &'static str {
        // The heartbeat word stays the QR screen's route word (spec §15.3): on this build the
        // login route IS this screen, and a different spelling would unselect its fps scenes.
        "login"
    }
    fn state(&self) -> &dyn crate::ui::machine::LogicalState {
        self
    }
    fn crumb(&self, _cx: &Cx<'_, H>) -> Option<std::borrow::Cow<'_, str>> {
        None
    }
    fn prepare(&mut self, _b: &mut crate::ui::frame::Budget, _cx: &Cx<'_, H>) {
        self.prepare_logo();
    }
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, H>) {
        crate::gfx::frame_clear(theme::CLEAR_RGB.0, theme::CLEAR_RGB.1, theme::CLEAR_RGB.2);
        let p = f.painter;
        // The sign-in screen is the first thing a new user sees, before Home has any artwork to
        // lend it — `Painter::root()`, not `p`, for the ground, the same rule `screens::login`
        // documents for its own mounting.
        self.ground.draw_default(Painter::root());
        let env = Env::inert();
        let layout = RouteLayout::screen();

        // The narrative column (wordmark + heading + helper + failure reason in danger ink)
        // is this screen's own; the table is the shared settings widget.
        let nar = layout.narrative;
        let mut y = nar.y;
        if self.logo_tex != 0 {
            // The asset is 981x293; drawn ~300px wide it sits over the narrative column's start.
            let w = 300.0f32;
            let h = w * 293.0 / 981.0;
            // Identity tint (pure white, full alpha) — the asset carries its own colour.
            p.tex(self.logo_tex, Rect::new(nar.x, y, w, h), 0.0, theme::CONTROL_IDLE_INK);
            y += h + theme::space::XL;
        } else {
            TextView::new("butaca", theme::size::DISPLAY, theme::TEXT_HEADING)
                .bold()
                .draw(p, Rect::new(nar.x, y, nar.w, theme::size::DISPLAY as f32 * 1.3));
            y += theme::size::DISPLAY as f32 * 1.6;
        }
        TextView::new(crate::i18n::msg::browse_jellyfin_sign_in_title(), theme::size::TITLE, theme::TEXT_HEADING)
            .bold()
            .draw(p, Rect::new(nar.x, y, nar.w, theme::size::TITLE as f32 * 1.4));
        y += theme::size::TITLE as f32 * 1.4 + theme::space::SM;
        TextView::new(
            crate::i18n::msg::browse_jellyfin_sign_in_explainer(),
            theme::size::LABEL,
            theme::TEXT_SECONDARY,
        )
        .draw(p, Rect::new(nar.x, y, nar.w, theme::size::LABEL as f32 * 3.2));
        y += theme::size::LABEL as f32 * 3.2 + theme::space::MD;
        if let Some(fail) = self.fail {
            TextView::new(fail_text(fail), theme::size::LABEL, theme::DANGER)
                .draw(p, Rect::new(nar.x, y, nar.w, theme::size::LABEL as f32 * 3.2));
        }

        // The form, vertically centred in the content column, with the working spinner beside it.
        let h = self.table.measured_height().min(layout.content.h);
        let frame = Rect::new(
            layout.content.x,
            layout.content.cy() - h * 0.5,
            layout.content.w,
            h,
        );
        self.table.draw(p, frame, f.measure);
        if signin::phase() == Phase::Working {
            Spinner::new(frame.cx(), frame.y - theme::space::XL, 15.0)
                .phase(self.spin_ms as u32)
                .tint(theme::TEXT_SECONDARY)
                .draw(&env, p);
        }
    }
    fn render(&self) -> crate::ui::screen::RenderStrategy {
        crate::ui::screen::RenderStrategy::Page
    }
    /// The wordmark bitmap is the one render this screen owns (§8.3 rule (c)) — its own
    /// `upload_rgba` in `prepare_logo`. Everything else is immediate-mode or a shared cache.
    fn render_report(&self) -> crate::ui::frame::RenderReport {
        if self.logo_tex == 0 {
            return crate::ui::frame::RenderReport::NONE;
        }
        crate::ui::frame::RenderReport::one(self.logo_px.0, self.logo_px.1)
    }
}

/// The draw-time measure capability out of the frame (§4.3) — the one argument `TableView`'s

#[cfg(test)]
mod tests {
    use super::*;

    /// The cross-build artifact gate refuses a binary with a private IP baked in, and these
    /// example strings ARE shipped in it. The example address has to stay a non-routable shape
    /// (a .local host name), never a real LAN octet - the literals below are the four-dot quad
    /// patterns the gate scans for.
    #[test]
    fn the_example_address_carries_no_private_ip_literal() {
        let private_octets = [
            "192.168.", "10.0.", "10.1.", "172.16.", "172.17.", "172.18.", "172.19.", "172.20.",
            "172.21.", "172.22.", "172.23.", "172.24.", "172.25.", "172.26.", "172.27.", "172.28.",
            "172.29.", "172.30.", "172.31.",
        ];
        for s in [crate::i18n::msg::browse_jellyfin_hint_address(), fail_text(Fail::Parse)] {
            for octet in private_octets {
                assert!(
                    !s.contains(octet),
                    "{s:?} still carries the private prefix {octet:?}"
                );
            }
        }
    }

    #[test]
    fn the_password_masks_by_character_and_caps() {
        assert_eq!(masked(""), "");
        assert_eq!(masked("señoría"), "•••••••"); // ñ and í are one dot each
        assert_eq!(masked(&"x".repeat(40)), "•".repeat(24));
    }

    #[test]
    fn connect_needs_a_server_and_a_name_but_not_a_password() {
        let mut f: [String; 3] = Default::default();
        assert!(!connectable(&f));
        f[F_SERVER] = "  ".into();
        f[F_USER] = "gleb".into();
        assert!(!connectable(&f), "whitespace is not a server address");
        f[F_SERVER] = "192.168.1.20:8096".into();
        assert!(connectable(&f), "an empty password can be the truth on a LAN account");
    }

    #[test]
    fn committed_text_drops_control_characters() {
        let mut fields: [String; 3] = Default::default();
        assert!(commit_text(&mut fields, Some(F_SERVER), "192.168.\n1.20"));
        assert_eq!(fields[F_SERVER], "192.168.1.20");
        // …and with no edit in flight, text is dropped rather than smeared across field 0.
        assert!(!commit_text(&mut fields, None, "junk"));
        assert_eq!(fields[F_SERVER], "192.168.1.20");
    }
}
