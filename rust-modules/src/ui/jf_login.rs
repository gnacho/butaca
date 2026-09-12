//! The Jellyfin flavor's sign-in screen: a form, not a QR. Three fields (server, user name,
//! password) and a Connect row, drawn with the app's settings-table widget so focus, scrolling
//! and the selection pill come from code that already ships. OK on a field raises the
//! television's own keyboard (`crate::textinput`); OK again — or BACK — commits it. The connect
//! itself is [`crate::jellyfin::signin`]'s worker; this screen only mirrors its phase.
//!
//! Reached exactly like the Plex QR screen: the boot gate and the account menu's sign-in both
//! land on `Route::Login`, and `ui::login` delegates draw/update/key here on this build. BACK
//! stays swallowed — a first-ever boot has nothing behind this screen, same rule the QR screen
//! documents.
#![allow(non_upper_case_globals)]

use crate::jellyfin::signin::{self, Fail, Phase};
use crate::ui::consts::*;
use crate::ui::route_screen::{RouteGround, RouteLayout};
use crate::ui::table::{Row, Section, TableView};
use crate::ui::text_view::TextView;
use crate::ui::widgets::Spinner;
use crate::ui::{theme, Env, Painter, Rect, View};
use std::os::raw::c_uint;
use std::ptr::addr_of_mut;

/// Field indices are also their TABLE ROW indices — the Connect row is the fourth.
const F_SERVER: usize = 0;
const F_USER: usize = 1;
const F_PASS: usize = 2;
const ROW_CONNECT: i32 = 3;

fn labels() -> [&'static str; 3] {
    [
        crate::i18n::t("Server"),
        crate::i18n::t("User name"),
        crate::i18n::t("Password"),
    ]
}
/// What an untouched field says. The server's is an example (the one thing a person must invent
/// is also the one with a shape worth showing); the other two state their requirement.
// The password hint is honest about the rule `connectable` actually applies: plenty of LAN
// Jellyfin users have no password, and the field must not claim otherwise.
const HINTS: [&str; 3] = ["e.g. 192.168.1.20:8096", "required", "may be empty"];

struct Scene {
    ground: RouteGround,
    table: TableView,
    fields: [String; 3],
    /// Which field the TV keyboard is editing, if it is up.
    editing: Option<usize>,
    /// The last failed attempt's reason — from `signin`'s phase, or a validation the screen
    /// itself did (empty fields) without ever starting one.
    fail: Option<Fail>,
    /// Rebuild the table's rows next update — text landed, or the phase changed under them.
    dirty: bool,
    spin_ms: f32,
    /// The butaca wordmark from the app directory, as a GL texture (0 = not tried / not there).
    logo_tex: u32,
    logo_tried: bool,
}

static mut SCENE: Option<Scene> = None;

fn scene() -> &'static mut Scene {
    unsafe { (*addr_of_mut!(SCENE)).as_mut().expect("jf_login::init not called") }
}

pub fn init() {
    // Prefill from the persisted config: a boot that lands here DESPITE one means the server
    // refused or was gone, and re-typing credentials on a remote is the worst possible tax for
    // it. The password prefills too — this television is the credential store, and the file it
    // is read from is 0600 beside the session.
    let fields = match crate::jellyfin::boot::stored_config() {
        Some((url, user, password)) => [url, user, password],
        None => Default::default(),
    };
    unsafe {
        *addr_of_mut!(SCENE) = Some(Scene {
            ground: RouteGround::new(),
            table: TableView::new(),
            fields,
            editing: None,
            fail: None,
            dirty: true,
            spin_ms: 0.0,
            logo_tex: 0,
            logo_tried: false,
        });
    }
}

/// Mount the route: fresh ground, keyboard down, and the flow's phase respected — a Working
/// attempt (a relaunch mid-connect is not a thing, but sign-out → sign-in keeps the module
/// warm) keeps its spinner rather than being lied about.
pub fn enter() {
    let s = scene();
    s.ground.reset();
    s.editing = None;
    crate::textinput::stop();
    s.dirty = true;
    crate::ui::idle::invalidate();
}

/// Leaving for Home (or anywhere): the system panel is a compositor surface over ours, and it
/// does not follow the route — take it down here, the same rule `search::leave` documents.
pub fn leave() {
    scene().editing = None;
    crate::textinput::stop();
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
fn commit_text(s: &mut Scene, text: &str) {
    let Some(f) = s.editing else { return };
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    if clean.is_empty() {
        return;
    }
    s.fields[f].push_str(&clean);
    s.dirty = true;
}

pub fn update(dt: f32) {
    let s = scene();
    s.spin_ms += dt * 1000.0;
    // Typed text first — the frame that receives a character draws it.
    for text in crate::textinput::drain() {
        commit_text(s, &text);
    }
    // Mirror the flow's phase into the one fact the draw reads. Editing means no attempt has
    // failed since the last keypress; a stale failure is cleared the moment a new attempt runs.
    match signin::phase() {
        Phase::Working | Phase::Ready => {
            if s.fail.is_some() {
                s.fail = None;
                s.dirty = true;
            }
        }
        Phase::Failed(f) => {
            if s.fail != Some(f) {
                s.fail = Some(f);
                s.dirty = true;
            }
        }
        Phase::Editing => {}
    }
    if s.dirty {
        let rows = build_rows(s);
        let sel = s.table.sel;
        s.table.set_sections(rows, sel, true);
        s.dirty = false;
    }
    s.table.update(dt, Rect::FULL.h);
}

/// The form as table sections: one section of fields, one of the action. Rebuilt on change
/// only — `TableView`'s springs live across these, `slide=true` keeps the pill where it was.
fn build_rows(s: &Scene) -> Vec<Section> {
    let working = signin::phase() == Phase::Working;
    let mut fields = Section::new("");
    for (i, label) in labels().iter().enumerate() {
        let empty = s.fields[i].is_empty();
        let value = if empty {
            crate::i18n::t(HINTS[i]).to_string()
        } else if i == F_PASS {
            masked(&s.fields[i])
        } else {
            s.fields[i].clone()
        };
        fields = fields.row(Row::new(*label).value(value).value_dim(empty).dim(working));
    }
    let action = Section::new("").row(
        Row::new(crate::i18n::t("Connect"))
            .chevron(true)
            .dim(working || !connectable(&s.fields)),
    );
    vec![fields, action]
}

/// The reason line under the narrative — one sentence per failure, worded for a couch, not a
/// console. The two address failures are told apart because their fixes are different verbs:
/// retype it, or serve it over TLS / from the LAN.
fn fail_text(f: Fail) -> &'static str {
    match f {
        Fail::Parse => {
            crate::i18n::t("That doesn't look like a server address \u{2014} try e.g. 192.168.1.20:8096")
        }
        Fail::Plaintext => {
            crate::i18n::t("That address would carry your password unprotected \u{2014} use https:// or a local network address")
        }
        Fail::Refused => crate::i18n::t("The server didn't recognize that user name or password"),
        Fail::Unreachable => {
            crate::i18n::t("Couldn't reach the server \u{2014} check the address and that it's on")
        }
    }
}

/// Load the wordmark once, on the draw thread (GL upload). A missing asset degrades to a text
/// wordmark — the screen still brands itself, just less prettily.
fn ensure_logo(s: &mut Scene) {
    if s.logo_tried {
        return;
    }
    s.logo_tried = true;
    let path = crate::paths::in_app_dir("butaca-name.png");
    if let Ok(png) = std::fs::read(&path) {
        let (mut w, mut h): (std::os::raw::c_int, std::os::raw::c_int) = (0, 0);
        let px = crate::img::img_decode_rgba(png.as_ptr(), png.len() as _, &mut w, &mut h);
        if !px.is_null() {
            s.logo_tex = crate::img::img_upload_rgba(px, w, h);
            crate::img::img_free(px);
        }
    }
}

pub fn draw() {
    crate::gfx::frame_clear(theme::CLEAR_RGB.0, theme::CLEAR_RGB.1, theme::CLEAR_RGB.2);
    let p = Painter::root();
    let s = scene();
    s.ground.draw_default(p);
    let env = Env::inert();
    let layout = RouteLayout::screen();

    // Narrative column: wordmark (image when the package carries it, type when it does not),
    // then the heading + helper, then the failure reason in the danger ink.
    let nar = layout.narrative;
    ensure_logo(s);
    let mut y = nar.y;
    if s.logo_tex != 0 {
        // The asset is 981×293; drawn ~300px wide it sits over the narrative column's start.
        let w = 300.0f32;
        let h = w * 293.0 / 981.0;
        // Identity tint (pure white, full alpha) — the asset carries its own colour.
        p.tex(s.logo_tex, Rect::new(nar.x, y, w, h), 0.0, theme::CONTROL_IDLE_INK);
        y += h + theme::space::XL;
    } else {
        TextView::new("butaca", theme::size::DISPLAY, theme::TEXT_HEADING)
            .bold()
            .draw(p, Rect::new(nar.x, y, nar.w, theme::size::DISPLAY as f32 * 1.3));
        y += theme::size::DISPLAY as f32 * 1.6;
    }
    TextView::new(crate::i18n::t("Sign in to Jellyfin"), theme::size::TITLE, theme::TEXT_HEADING)
        .bold()
        .draw(p, Rect::new(nar.x, y, nar.w, theme::size::TITLE as f32 * 1.4));
    y += theme::size::TITLE as f32 * 1.4 + theme::space::SM;
    TextView::new(
        crate::i18n::t("Your server's address, your user name and its password. OK opens the keyboard for a field; OK again commits it."),
        theme::size::LABEL,
        theme::TEXT_SECONDARY,
    )
    .draw(p, Rect::new(nar.x, y, nar.w, theme::size::LABEL as f32 * 3.2));
    y += theme::size::LABEL as f32 * 3.2 + theme::space::MD;
    if let Some(f) = s.fail {
        TextView::new(fail_text(f), theme::size::LABEL, theme::DANGER)
            .draw(p, Rect::new(nar.x, y, nar.w, theme::size::LABEL as f32 * 3.2));
    }

    // The form, vertically centred in the content column, with the working spinner beside it.
    let h = s.table.measured_height().min(layout.content.h);
    let frame = Rect::new(
        layout.content.x,
        layout.content.cy() - h * 0.5,
        layout.content.w,
        h,
    );
    s.table.draw(p, frame);
    if signin::phase() == Phase::Working {
        Spinner::new(frame.cx(), frame.y - theme::space::XL, 15.0)
            .phase(s.spin_ms as u32)
            .tint(theme::TEXT_SECONDARY)
            .draw(&env, p);
    }
}

/// Begin or commit the field edit on OK; move on the d-pad; swallow BACK. Answers nothing —
/// like `login::key`, the flow drives the route from its own state.
pub fn key(sym: c_uint, wcode: c_uint) {
    let s = scene();
    if let Some(f) = s.editing {
        // The panel's edit protocol, per search's key handler: backspace and clear arrive as
        // keys, the printable text as SDL_TEXTINPUT. OK and BACK both commit and close.
        if sym == SDLK_BACKSPACE {
            s.fields[f].pop();
            s.dirty = true;
        } else if sym == SDLK_CLEAR {
            s.fields[f].clear();
            s.dirty = true;
        } else if is_ok(sym) || is_back(sym, wcode) {
            s.editing = None;
            crate::textinput::stop();
        }
        // Anything else (d-pad included — the panel forwards some of it) is the keyboard's,
        // not the form's.
        crate::ui::idle::invalidate();
        return;
    }
    if signin::phase() == Phase::Working {
        return; // an attempt is out; the form is inert until it answers
    }
    if sym == SDLK_UP {
        s.table.move_sel(-1);
    } else if sym == SDLK_DOWN {
        s.table.move_sel(1);
    } else if is_ok(sym) {
        let sel = s.table.sel;
        if sel == ROW_CONNECT {
            if !connectable(&s.fields) {
                // Saying WHY costs a failure line; doing nothing reads as a dead button.
                s.fail = Some(Fail::Parse);
                s.dirty = true;
            } else {
                let [url, user, pass] = &s.fields;
                if let Err(f) = signin::start(url, user, pass) {
                    s.fail = Some(f);
                } else {
                    s.fail = None;
                }
                s.dirty = true;
            }
        } else if (0..3).contains(&(sel as usize)) {
            s.editing = Some(sel as usize);
            crate::textinput::start();
        }
    }
    // BACK with no edit in flight is swallowed on purpose — see the module doc.
    crate::ui::idle::invalidate();
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut s = Scene {
            ground: RouteGround::new(),
            table: TableView::new(),
            fields: Default::default(),
            editing: Some(F_SERVER),
            fail: None,
            dirty: false,
            spin_ms: 0.0,
            logo_tex: 0,
            logo_tried: true,
        };
        commit_text(&mut s, "192.168.\n1.20");
        assert_eq!(s.fields[F_SERVER], "192.168.1.20");
        assert!(s.dirty);
        // …and with no edit in flight, text is dropped rather than smeared across field 0.
        s.editing = None;
        commit_text(&mut s, "junk");
        assert_eq!(s.fields[F_SERVER], "192.168.1.20");
    }
}
