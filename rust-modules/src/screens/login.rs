//! **The sign-in screen, as an owned `Screen`** (restructure spec §13, phase 6 — `ui/login.rs`
//! moved). Plex's own server-rendered QR PNG (fetched by [`crate::auth`], decoded + tinted here)
//! plus the typed short-code fallback, driven by the flow's phase. Scanning the QR on a phone
//! opens plex.tv pre-filled with the pin; the flow's background poll then advances us onward.
//!
//! It has no panel of its own and is not a table. Its controls are ONE group of up to two
//! `ElemKind::Bare` elements, walked in order: the read-out's primary action (the failed/stalled/
//! deleted pill, or — while the code is simply unscanned for a long time — the QR screen's own
//! "press OK for a new code" sentence). It fires on the OK key-down edge with no hold and no
//! press bounce, exactly as the lone action did under the old `key()` ladder.
//!
//! This fork sends nothing: the onboarding report (Session's incident offers, the Report ID,
//! *Details* and *Send report*) was removed with the telemetry module. A sign-in failure is the
//! design system's `StatusOverlay` failed and nothing else: verdict, reason, *Try again*.
//!
//! The constructor and each `Tick` consume one immutable [`auth::SessionRead`] publication through
//! [`AuthLike`]. Its login client-id capture resolves on the storage worker when the visible
//! cache is empty (including local revocation), so a transient peek is never latched as the ID.
//! The QR code, bitmap and generation therefore come from one retained snapshot; draw,
//! prepare and focus geometry read only the screen's cached fields. Commands travel as typed
//! [`auth::SessionCmd`] effects. A stalled-wait clock is reset only by its matching accepted
//! [`AppMsg::RestartReply`], never when the request is merely emitted.

use std::ffi::{CStr, CString};
use std::os::raw::c_int;
use std::sync::Arc;

use crate::auth::{self, Phase};
use crate::ui::frame::Budget;
use crate::ui::label::HAlign;
use crate::ui::machine::{
    Canon, Cx, Delivery, Edge, Effects, EntryId, Fx, GroupId, Handled, InputEvent, InputKind, Key,
    LogicalState, Machine, Measure, Tick,
};
use crate::ui::route_screen::{RouteGround, RouteLayout};
use crate::ui::screen::{
    Activate, At, AxisMask, Dir, DrawFrame, EdgeRule, ElemKind, Enter, FocusSource, FocusTarget,
    Focusable, GroupKind, GroupSpec, HitSource, Hover, Placed, RenderStrategy, Screen, ScreenEvent,
    Seat, Step, Stop,
};
use crate::ui::text_view::TextView;
use crate::ui::decision_alert::DecisionAlert;
use crate::ui::widgets::{CtlPop, Spinner, StatusKind, StatusOverlay};
use crate::ui::{theme, Env, Painter, Rect, View};

use super::plaintext_question::{self, PlaintextQuestion};
use super::registry::{word, AppFx, AppLike, AppMsg, AuthLike};

/// The screen's elements. The read-out's primary action owns [`CONTROL_GROUP`]; the plaintext
/// question's answers are [`ALERT_GROUP`], the only group while the alert is open. `GroupId(0)`
/// matches the container's own default fresh-mount target
/// (`stack.rs::fresh`), which is what lets a screen that mounts straight into `Phase::Deleted` (a
/// real control from frame one) get seated by the ordinary Mount → Enter sequence with no
/// correction of its own.
const CONTROL: u32 = 0;
/// The alert's cancel slot.
const ALERT_CANCEL: u32 = 3;
/// The alert's affirm slot: the plaintext question's *Connect*.
const ALERT_AFFIRM: u32 = 4;
const CONTROL_GROUP: GroupId = GroupId(0);
const ALERT_GROUP: GroupId = GroupId(1);


/// How long a working phase runs before the read-out grows a way out.
///
/// **Not zero**: a healthy LAN discovery finishes in well under a second, and a control that
/// flashes past on every sign-in is noise that teaches people to ignore it. **Not longer**: from
/// the sofa a spinner that will never stop looks exactly like one that is about to, and until this
/// existed there was no way at all out of a wedged sign-in — BACK is the root press (see
/// `Machine::step`'s BACK arm), and on a first-ever boot there is no stored session for it to
/// resume, so the only exit was killing the app.
const ESCAPE_AFTER_MS: f32 = 12_000.0;

/// Whether a wait has run long enough to be worth offering an escape from.
///
/// Pure and separate from the draw so the threshold is gradeable on the host.
fn escape_offered(phase_ms: f32) -> bool {
    phase_ms >= ESCAPE_AFTER_MS
}

/// The phases that are genuinely WAITING ON A NETWORK CALL and can therefore stall.
///
/// **An allowlist, not "everything that is not terminal".** It was the latter for an hour, which
/// swept in `Ready`, `Profiles` and `Switching` — phases the main loop routes away from on its next
/// pass. A stalled-wait control answered on one of those would call `auth::retry`/
/// `auth::restart_stalled_wait` on a flow that had already SUCCEEDED, replacing a completed handoff
/// with a fresh sign-in. `Idle` is excluded for the same reason from the other side: nothing is
/// owed, so there is nothing to retry.
fn working_phase(phase: Phase) -> bool {
    matches!(phase, Phase::Creating | Phase::Discovering)
}

fn discovery_trouble_visible(phase: Phase, retry: Option<auth::DiscoveryRetryProgress>,
    phase_ms: f32, observed_phase_ms: f32) -> bool {
    let Some(retry) = retry.filter(|_| phase == Phase::Discovering) else { return false };
    retry.misses >= 2
        || retry.elapsed_ms as f32 + (phase_ms - observed_phase_ms).max(0.0) >= 8_000.0
}

/// The verb on both the failed and the stuck read-out, because it is the same call underneath.
///
/// `auth::retry`/`auth::restart_stalled_wait` bump the auth epoch, so a worker still blocked in the
/// wedged request has its result discarded when it finally returns, and it re-runs only the leg
/// that failed — discovery when the pin already yielded an account credential, a whole fresh pin
/// when it did not.

/// AUTH-03: acknowledges a fresh save the disk could not confirm — proceed, knowing the next
/// launch may ask you to sign in again.


/// How long a QR code may go unscanned before the screen offers to replace it on request.
///
/// **A separate, much longer clock than [`ESCAPE_AFTER_MS`], because this wait is not a stall.**
/// Twelve seconds is right for a spinner that should have finished in one; a code on screen is
/// waiting for a person to find their phone, unlock it, open a camera and tap a link, and nagging
/// them at twelve seconds would be wrong every time. A full minute of a code that has already been
/// scanned is not.
///
/// It exists because the automatic replacement (a pin that runs out is re-minted automatically)
/// cannot cover the case the issue reported: the phone says *Account linked* while our polls are
/// being answered `Pending` or nothing at all, and the person watching knows something the
/// television does not. Waiting out the rest of a fifteen-minute lease is not a recovery.
const QR_ESCAPE_AFTER_MS: f32 = 60_000.0;

/// Whether the QR screen is offering its own replacement right now. Pure, and — like
/// [`escape_offered`] — the ONE predicate behind both the sentence and the key, so a control that
/// is not drawn can never be activated.
fn qr_escape_offered(phase_ms: f32) -> bool {
    phase_ms >= QR_ESCAPE_AFTER_MS
}

/// **What the screen is waiting ON**, as the pair [`LoginScreen::phase_ms`] is timing.
///
/// The phase alone was the whole identity while a code could only change by leaving `Waiting`. It
/// cannot be any more: a pin that runs out is replaced automatically, `Waiting → Creating →
/// Waiting`, and a `Tick` samples once a frame — so a replacement completed between two samples (a
/// paused main loop, a long frame) is invisible, and the FRESH code inherits the dead one's age. It
/// would then offer "press OK for a new code" about a code that had existed for a millisecond.
/// Including the generation makes the reset exact rather than probable.
type Wait = (Phase, u64);

/// Is what the screen is waiting on a DIFFERENT thing from what it was waiting on last frame?
///
/// Trivial, and separate anyway, because the rule it encodes is not: a new CODE restarts the clock
/// exactly as a new PHASE does, and the version that compared phases alone is the one that would
/// offer to replace a code a millisecond old.
fn wait_restarted(seen: Wait, live: Wait) -> bool {
    seen != live
}

/// Release the cached QR texture as soon as it stops describing the code the flow is showing.
///
/// **Keyed on the QR generation, not on the phase, and that swap is this screen's half of issue
/// #30.** The rule used to be "a retry enters `Creating`, so drop it there" — which was true of the
/// only way a code could ever change. It no longer is: a pin that runs out is now replaced
/// automatically, and the flow returns to the same `Waiting` it was already in. A cache keyed on
/// the phase would have gone on drawing the dead code — sharp, scannable, and pointing at a pin
/// plex.tv had forgotten — for the whole of its successor's life. `Creating` is still checked, as
/// the belt to the generation's braces: it is the one moment a flow is known to have thrown its
/// code away before any replacement exists.
fn qr_cache_stale(cached: u64, live: u64, phase: Phase) -> bool {
    cached != live || phase == Phase::Creating
}

/// What the delete actually achieved, as the two lines it is honest to draw.
///
/// **A partial wipe may not be reported as a whole one**, and that is not pedantry: the files this
/// sweep can fail on include the TELEMETRY decision, so a survivor is re-read on the next launch
/// and a consent the user believed they had deleted comes back. The session is gone either way, so
/// the verdict stays true and the reason carries the qualification.
/// The delete read-out's verdict and reason as KEYS (`&'static str`): the draw site renders them
/// through `i18n::tcstr`, because both follow the television's locale.
fn deleted_readout(leftovers: usize) -> (&'static str, &'static str) {
    if leftovers == 0 {
        (
            crate::i18n::t("Local data deleted"),
            crate::i18n::t("Credentials, preferences, telemetry and local diagnostics have been removed."),
        )
    } else {
        (
            crate::i18n::t("Signed out, and most local data deleted"),
            crate::i18n::t("Some files could not be removed and may still be on this television."),
        )
    }
}

/// The line under the code, which has to answer a question that only exists now that a code can be
/// replaced: *why is this not the code I was looking at*.
///
/// A pin lives fifteen minutes and is re-minted when it runs out, so somebody who walked away
/// mid-sign-in — or whose phone has just told them the OLD code was linked — comes back to
/// different digits. Saying nothing there reads as the television having lost track of itself, and
/// it is exactly the moment they need to be told to scan again. **`stalled` outranks
/// `code_replaced`**: one of these sentences carries an ACTION, and a line that explains history is
/// worth less than the one that offers a way forward.
///
/// **`unreachable` outranks both.** While plex.tv is not answering at all, a scan cannot complete
/// and a new code cannot be issued, so neither of the other sentences is true; the one useful
/// thing to say is where the fault most likely is.
fn waiting_status(code_replaced: bool, stalled: bool, unreachable: bool) -> &'static CStr {
    if unreachable {
        c"Can\u{2019}t reach Plex. Check your TV\u{2019}s internet connection."
    } else if stalled {
        c"Still waiting — press OK for a new code"
    } else if code_replaced {
        c"That code expired — scan this one"
    } else {
        c"Waiting for you to sign in…"
    }
}

/// The complete manual-link stack in the content column.
///
/// The QR itself is centred on the television's Y axis as requested, while the URL, manual code
/// and waiting state flow from its edges. That makes the scan target visually central without
/// separating the fallback credentials into unrelated screen coordinates.
#[derive(Clone, Copy)]
struct QrLayout {
    url: Rect,
    card: Rect,
    code: Rect,
    status: Rect,
}

fn qr_layout(layout: RouteLayout) -> QrLayout {
    const SIDE: f32 = 420.0;
    let card = Rect::new(
        layout.content.cx() - SIDE * 0.5,
        Rect::FULL.cy() - SIDE * 0.5,
        SIDE,
        SIDE,
    );
    let url_h = theme::size::TITLE as f32 + theme::space::XS;
    let code_h = theme::size::DISPLAY as f32 + theme::space::XS;
    let status_h = theme::size::BODY as f32 + theme::space::SM;
    QrLayout {
        url: Rect::new(
            layout.content.x,
            card.y - theme::space::LG - url_h,
            layout.content.w,
            url_h,
        ),
        card,
        code: Rect::new(
            layout.content.x,
            card.y + card.h + theme::space::LG,
            layout.content.w,
            code_h,
        ),
        status: Rect::new(
            layout.content.x,
            card.y + card.h + theme::space::LG + code_h + theme::space::MD,
            layout.content.w,
            status_h,
        ),
    }
}

/// **The read-out, built ONCE for both its uses** — the draw, and the geometry the focus engine and
/// the hit test read ([`status_row_rects`]), so the two can never disagree about where a control
/// is (`ui/table_screen.rs`'s rule: "the DRAW reads the same formula"). `labels` is the row by
/// slot — the primary, then *Details* — and `note` the report's one quiet status line under it.
/// It fills the page, so a `Failed` one hangs from `StatusOverlay::FULL_ANCHOR_TOP` through
/// `.page()`, the same as Home's and the Library's.
fn readout_overlay<'a>(
    caption: &'a CStr,
    kind: StatusKind,
    reason: Option<&'a CStr>,
    labels: [Option<&'a CStr>; 2],
    note: Option<&'a Note>,
) -> StatusOverlay<'a> {
    let mut o = StatusOverlay::new(Rect::FULL, caption, kind).page();
    if let Some(r) = reason {
        o = o.reason(r);
    }
    if let Some(note) = note {
        o = o.note(Some(note.text.as_c_str())).note_busy(note.busy);
    }
    if let Some(primary) = labels[0] {
        o = o.action(primary).secondary(labels[1]);
    }
    o
}

/// The read-out's controls, placed by the WIDGET through the [`Measure`] capability — the same
/// `StatusOverlay::action_frames_measured` its draw uses. The overlay built here carries no
/// caption or reason TEXT: the row moves with the KIND (a `Working` caption sits lower, a
/// page-filling `Failed` one stands on the shared page lines) and with whether a reason exists (a
/// `Failed` reason is a two-line slot whatever it says).
fn status_row_rects(
    measure: &dyn Measure,
    labels: [Option<&CStr>; 2],
    kind: StatusKind,
    has_reason: bool,
) -> [Option<Rect>; 2] {
    readout_overlay(c"", kind, has_reason.then_some(c""), labels, None).action_frames_measured(measure)
}

/// The lone action's rect — [`status_row_rects`] for a read-out with one control.
#[cfg(test)]
fn status_action_rect(
    measure: &dyn Measure,
    label: &CStr,
    kind: StatusKind,
    has_reason: bool,
) -> Rect {
    status_row_rects(measure, [Some(label), None], kind, has_reason)[0].unwrap_or(Rect::FULL)
}

/// What the one control (when it exists at all) does when pressed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ControlKind {
    /// The stalled-wait escape — either the Working spinner's `Try again` (12 s) or the QR
    /// screen's `press OK for a new code` (60 s). Both restart the SAME wait this screen has been
    /// timing (`auth::restart_stalled_wait`), which is why they share one verb here even though
    /// they read different sentences on screen.
    RestartWait,
    /// `Phase::Error`'s `Try again` — acts unconditionally; there is no live worker to race.
    Retry,
    /// `Phase::Deleted`'s `Sign in` — starts a whole fresh flow.
    StartLogin,
    /// AUTH-03: acknowledges an unconfirmed fresh save and releases the held Ready handoff (or,
    /// for a Discovery-site warning, re-admits the final commit that produces one).
    ContinueUnsaved,
    /// `Phase::Error` for an eligible plaintext-only server not yet answered
    /// (`plaintext_question::asks`): `Connect` asks the consent question ([`Sheet::Plaintext`]) —
    /// the read-out keeps one primary, never a third button. Once answered the primary is *Try
    /// again* ([`ControlKind::Retry`]), and the reason says what it does.
    ConnectPlaintext,
}

/// The control labels are keys, translated per call (the locale is fixed at boot): the draw
/// site renders the returned word through `i18n::tcstr` beside the button.
fn label_for(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::RestartWait | ControlKind::Retry => crate::i18n::t("Try again"),
        ControlKind::StartLogin => crate::i18n::t("Sign in"),
        ControlKind::ContinueUnsaved => crate::i18n::t("Continue"),
        ControlKind::ConnectPlaintext => crate::i18n::t("Connect"),
    }
}

/// Every branch but the two SETTLED read-outs (`Failed`/`Deleted`) draws the spinner — the one
/// thing on this screen that animates from a raw clock (`spin_ms`) rather than a spring `ui::idle`
/// can see on its own. `Spinner::draw`'s own module note is the standing warning that this class of
/// animator ships FROZEN if it forgets to report every frame it is on screen.
fn control_has_spinner(phase: Phase, warning_showing: bool) -> bool {
    !warning_showing && !matches!(phase, Phase::Error | Phase::Deleted)
}

fn phase_disc(p: Phase) -> u8 {
    match p {
        Phase::Idle => 0,
        Phase::Creating => 1,
        Phase::Waiting => 2,
        Phase::Discovering => 3,
        Phase::Profiles => 4,
        Phase::Switching => 5,
        Phase::Ready => 6,
        Phase::Error => 7,
        Phase::Deleted => 8,
    }
}

struct LoginState {
    phase: u8,
    qr_gen: u64,
    qr_replaced: bool,
    has_control: bool,
    delete_leftovers: u32,
    next_correlation: Option<u32>,
    pending_restart: Option<u32>,
    /// AUTH-03: whether a `PersistenceWarning` is currently shown. The key itself is not part of
    /// the logical state a container needs to notice a change — only whether one is showing.
    warning: bool,
    /// Session's "plex.tv is not answering the wait" — the QR screen's status line.
    link_trouble: bool,
    /// Whether the plaintext question's alert is open.
    alert_open: bool,
    /// `Some(question_open)` while the read-out offers an unencrypted connection. Written to the
    /// canon only when `Some`, so every other state's digest is unchanged.
    plaintext: Option<bool>,
}

impl LogicalState for LoginState {
    fn write(&self, w: &mut Canon) {
        w.u8(self.phase);
        w.u64(self.qr_gen);
        w.bool(self.qr_replaced);
        w.bool(self.has_control);
        w.u32(self.delete_leftovers);
        w.option(self.next_correlation, |w, correlation| {
            w.u32(correlation);
        });
        w.option(self.pending_restart, |w, correlation| {
            w.u32(correlation);
        });
        w.bool(self.warning);
        w.bool(self.link_trouble);
        w.option(self.alert_open.then_some(()), |w, ()| {
            w.bool(true);
        });
        if let Some(open) = self.plaintext {
            w.bool(open);
        }
    }
    fn probe(&self, out: &mut String) {
        out.push_str(&format!(
            "login phase={} qr_gen={} replaced={} control={} leftovers={} next={:?} restart={:?} warning={} link_trouble={} alert_open={} plaintext={:?}",
            self.phase,
            self.qr_gen,
            self.qr_replaced,
            self.has_control,
            self.delete_leftovers,
            self.next_correlation,
            self.pending_restart,
            self.warning,
            self.link_trouble,
            self.alert_open,
            self.plaintext,
        ));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct PendingRestart {
    correlation: u32,
    wait: Wait,
}

/// A short status line under the read-out, and whether a spinner turns beside it.
#[derive(Debug, PartialEq, Eq)]
struct Note {
    text: CString,
    busy: bool,
}

/// The control group's elements in walk order. At most two, so a fixed array.
#[derive(Clone, Copy, Default)]
struct Row {
    elems: [u32; 2],
    n: usize,
}

impl Row {
    fn push(&mut self, e: u32) {
        self.elems[self.n] = e;
        self.n += 1;
    }
    fn as_slice(&self) -> &[u32] {
        &self.elems[..self.n]
    }
    fn position(&self, e: u32) -> Option<usize> {
        self.as_slice().iter().position(|&x| x == e)
    }
}

/// The pop/`StatusOverlay` slot an element draws in.
fn slot_of(elem: u32) -> Option<usize> {
    match elem {
        CONTROL => Some(0),
        _ => None,
    }
}

pub(crate) struct LoginScreen {
    entry: EntryId,
    /// Free-running rotation clock for the spinner, in ms — cached each tick from
    /// [`spin_phase`](Self::spin_phase)'s `advance`. Render-only, never hashed.
    spin_ms: f32,
    /// How long the CURRENT wait has been on screen, in ms — cached each tick from
    /// [`phase_clock`](Self::phase_clock)'s `advance`, reset whenever [`Wait`] changes.
    phase_ms: f32,
    /// The underlying clocks for [`spin_ms`](Self::spin_ms)/[`phase_ms`](Self::phase_ms)
    /// (`motion::Phase` — spec phase 12 D4): an UNBOUNDED clock-driven animator reports `Motion`
    /// from inside its own `advance`, the way [`motion::Ramp`](crate::ui::motion::Ramp) does for a
    /// bounded one, rather than the raw `+= dt` these two fields used to accumulate with
    /// `fx.note(Motion)` called separately, out of band, below.
    spin_phase: crate::ui::motion::Phase,
    phase_clock: crate::ui::motion::Phase,
    wait: Wait,
    /// The uploaded GL texture of Plex's QR PNG (0 until decoded+uploaded) and which generation it
    /// describes. Render resources only — built and freed in [`Screen::prepare`], never in `step`,
    /// so a host test driving `step` alone never touches GL.
    qr_tex: u32,
    qr_tex_gen: u64,
    /// The pixel size that texture was uploaded at, so [`Screen::render_report`] can state the
    /// bytes this screen holds of the frame's render residency (§8.3) instead of guessing them:
    /// the bitmap is whatever plex.tv's PNG decoded to, not a constant. `(0, 0)` while `qr_tex`
    /// is 0, and the two are set and cleared together.
    qr_px: (u32, u32),
    /// PNG bytes `tick` captured this frame, awaiting [`Screen::prepare`]'s decode+upload — the
    /// hand-off between "retain one Session publication" (step) and "touch GL" (prepare). `None` once
    /// consumed, or when nothing new has been published.
    qr_png_pending: Option<(u64, Arc<[u8]>)>,
    /// Everything this screen retains from Session's immutable publication, refreshed once per
    /// `Tick`; `draw` and `prepare` never perform another read.
    phase: Phase,
    qr_gen: u64,
    qr_code: Arc<str>,
    qr_replaced: bool,
    error: Arc<str>,
    discovery_retry: Option<auth::DiscoveryRetryProgress>,
    discovery_retry_observed_phase_ms: f32,
    delete_leftovers: usize,
    next_correlation: Option<u32>,
    pending_restart: Option<PendingRestart>,
    /// AUTH-03: the warning this screen is answering, if any — synced every `resync` from the
    /// Session publication.
    persistence_warning: Option<auth::owner::PersistenceWarning>,
    /// The insecure-only verdict the failure read-out is about (`SessionSnapshot::plaintext`),
    /// synced every `resync`.
    plaintext: Option<auth::PlaintextVerdict>,
    /// The shared plaintext question's alert.
    question: PlaintextQuestion,
    ground: RouteGround,
    /// Session's "plex.tv is not answering the wait" — the QR screen's status line.
    link_trouble: bool,
    /// The plaintext question's decision alert (the only alert this screen has since the
    /// onboarding report was removed with the telemetry module).
    alert: DecisionAlert,
    /// The alert's answers as last drawn — `Focusable` answers with `&self`.
    alert_frames: Option<(Rect, Rect)>,
    /// The control group's focus pops.
    pop: CtlPop<1>,
    state: LoginState,
}

impl LoginScreen {
    /// Unused on a Jellyfin build (the registry mounts `screens::jf_login` for the login route
    /// there) — kept compiling so the flavor's test suite keeps driving it directly.
    #[cfg_attr(feature = "jellyfin", allow(dead_code))]
    pub(crate) fn new(entry: EntryId, auth: auth::SessionRead<'_>) -> Self {
        let mut s = Self {
            entry,
            spin_ms: 0.0,
            phase_ms: 0.0,
            spin_phase: crate::ui::motion::Phase::default(),
            phase_clock: crate::ui::motion::Phase::default(),
            wait: (Phase::Idle, 0),
            qr_tex: 0,
            qr_tex_gen: 0,
            qr_px: (0, 0),
            qr_png_pending: None,
            phase: Phase::Idle,
            qr_gen: 0,
            qr_code: Arc::from(""),
            qr_replaced: false,
            error: Arc::from(""),
            discovery_retry: None,
            discovery_retry_observed_phase_ms: 0.0,
            delete_leftovers: 0,
            next_correlation: Some(1),
            pending_restart: None,
            persistence_warning: None,
            plaintext: None,
            question: PlaintextQuestion::new(),
            ground: RouteGround::new(),
            link_trouble: false,
            alert: DecisionAlert::new(),
            alert_frames: None,
            pop: CtlPop::new(),
            state: LoginState {
                phase: 0,
                qr_gen: 0,
                qr_replaced: false,
                has_control: false,
                delete_leftovers: 0,
                next_correlation: Some(1),
                pending_restart: None,
                warning: false,
                link_trouble: false,
                alert_open: false,
                plaintext: None,
            },
        };
        // Read once at construction — not a `draw`-time poll — so the first frame is coherent
        // even when Mount/Tick are still queued behind it.
        // Without it, a mount that lands straight in `Phase::Deleted` (Settings' "Delete all local
        // data", confirmed) would draw its FIRST frame from the constructor's own placeholder
        // `Phase::Idle` — the wrong branch — because the container's default `Enter` (and this
        // screen's own first `draw`) both run before this screen's first `Tick` ever could.
        s.wait = s.resync(auth);
        s
    }

    /// Refresh every field this screen caches from one immutable Session publication. Called once
    /// at construction and again on every `Tick` through [`AuthLike`].
    ///
    /// **Returns the `(phase, qr_generation)` pair it just sampled**, so a caller that ALSO needs
    /// to know what changed — `tick`'s own wait-restart clock — reads the exact values this method
    /// cached instead of asking for a second publication. `tick` used to do exactly that: it
    /// computed `let live: Wait = (auth::phase(), auth::qr_generation());` several lines before
    /// calling `resync`, which then read the identical pair out of `crate::auth` again on its own.
    /// That is precisely the "read it twice" bug the module doc calls out by name — a retry (or a
    /// pin running out and being replaced) landing in the gap between the two independent reads
    /// could hand the wait-restart clock a phase that disagreed with the one this method cached,
    /// so the clock could reset (or fail to reset) against a `Wait` that was never actually drawn.
    /// Threading the sample through the return value makes the two agree by construction.
    fn resync(&mut self, auth: auth::SessionRead<'_>) -> Wait {
        let snapshot = auth.0;
        self.phase = snapshot.phase;
        self.qr_gen = snapshot.qr_generation;
        if self.phase == Phase::Waiting {
            // ONE read of the code, generation and bitmap together — `auth::QrCode`'s own doc says
            // why: taking them separately can mix the new digits with the old bitmap, or cache a
            // fresh bitmap under a stale generation.
            self.qr_code = Arc::clone(&snapshot.code);
            self.qr_replaced = snapshot.code_replaced;
            self.qr_png_pending = Some((snapshot.qr_generation, Arc::clone(&snapshot.png)));
        }
        if matches!(self.phase, Phase::Error | Phase::Discovering) {
            self.error = Arc::clone(&snapshot.error);
        } else {
            self.error = Arc::from("");
        }
        if self.discovery_retry != snapshot.discovery_retry {
            self.discovery_retry = snapshot.discovery_retry;
        }
        if self.phase == Phase::Deleted {
            self.delete_leftovers = snapshot.delete_leftovers;
        }
        self.persistence_warning = snapshot.persistence_warning;
        self.link_trouble = snapshot.link_trouble;
        self.plaintext = snapshot.plaintext.clone();
        self.sync_state();
        (self.phase, self.qr_gen)
    }

    fn sync_state(&mut self) {
        self.state = LoginState {
            phase: phase_disc(self.phase),
            qr_gen: self.qr_gen,
            qr_replaced: self.qr_replaced,
            has_control: self.has_control(),
            delete_leftovers: self.delete_leftovers as u32,
            next_correlation: self.next_correlation,
            pending_restart: self.pending_restart.map(|pending| pending.correlation),
            warning: self.persistence_warning.is_some(),
            link_trouble: self.link_trouble,
            alert_open: self.alert.is_open(),
            plaintext: self.plaintext_asks().then(|| self.alert.is_open()),
        };
    }

    fn tick<H: AuthLike>(&mut self, t: Tick, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) {
        let had_control = self.has_control();
        let alert_was_open = self.alert.is_open();

        // ONE sample of `crate::auth` feeds both the wait-restart clock below and every cached
        // field `resync` publishes — see `resync`'s own doc, and the module doc's "read it twice
        // is an observable bug" rule. This used to read `(auth::phase(), auth::qr_generation())`
        // again independently right here, a few lines before calling `resync`, which read the
        // identical pair a second time on its own; a retry (or an automatic pin replacement)
        // landing in the gap between the two reads could disagree with itself within one tick.
        let retry_before = self.discovery_retry;
        let live: Wait = self.resync(H::auth(cx));

        // Each wait gets its own clock. A flow that walks Creating → Waiting → Discovering is
        // making progress, and restarting the timer at every step is what stops a slow-but-healthy
        // sign-in from being offered a way out of itself.
        if wait_restarted(self.wait, live) {
            self.wait = live;
            self.pending_restart = None;
            self.phase_clock.reset(t);
            self.phase_ms = 0.0;
        }
        if retry_before != self.discovery_retry {
            self.discovery_retry_observed_phase_ms = self.phase_ms;
        }

        // Both clocks are `motion::Phase` (spec phase 12 D4): the raw `+= dt` this used to be, and
        // the `fx.note(Motion)` below it, are now one call each — `advance` both reads the elapsed
        // ms AND reports motion, so a caller that forgets to note motion for a still-running clock
        // (the exact "ships frozen" bug class `control_has_spinner`'s own doc names) cannot
        // separate the two any more. Both clocks are gated on the SAME `control_has_spinner`
        // condition as the `fx.note` call they replace: `working_phase`/`Phase::Waiting`, the only
        // phases whose escape thresholds `phase_ms` is timed against, are themselves a SUBSET of
        // `control_has_spinner`'s true set, so this changes nothing about when the escape offer
        // can appear — it only stops accumulating (freezes, harmlessly, since nothing reads it)
        // while the spinner is not drawn at all (`Error`/`Deleted`).
        //
        // A report on its way draws the same spinner beside its line, on a read-out that draws
        // none of its own (a failed sign-in), so the spinner's clock also runs while it is busy —
        // and ONLY the spinner's: `phase_ms` stays on the control's own condition.
        let control_spins = control_has_spinner(self.phase, self.persistence_warning.is_some());
        let report_spins = false;
        if control_spins || report_spins {
            let mut present = fx.present();
            self.spin_ms = self.spin_phase.advance(t, &mut present);
            if control_spins {
                self.phase_ms = self.phase_clock.advance(t, &mut present);
            }
        }

        self.tick_report(t, cx, fx);

        let reseat = (self.has_control() && !had_control)
            || (alert_was_open && !self.alert.is_open() && self.has_control());
        if reseat && !self.alert.is_open() {
            // The control just appeared (a stalled wait grew its escape, or a phase moved straight
            // to `Error`) — seat focus on it now. The container's default `Enter` already ran, at
            // mount time, against whatever `groups()` answered THEN; nothing else will ever ask
            // the engine to look again unless this screen does, exactly the correction
            // `screens::onboard`'s first-run constructor makes for the same underlying reason
            // (that module's own doc has the longer argument for why a REACTION is the right shape
            // rather than a second write at some earlier point).
            Self::enter_group(fx, CONTROL_GROUP);
        }
        self.sync_state();
    }


    fn enter_group<H: AppLike>(fx: &mut Effects<'_, H>, group: GroupId) {
        let me = fx.from();
        fx.push(Fx::Deliver(
            me,
            Delivery::Screen(ScreenEvent::Enter(Enter::Fresh {
                focus: FocusTarget::ContainerGroup(group),
            })),
        ));
    }

    /// The plaintext question's frame: take the alert down when the question it asks about has
    /// gone, and step the motion.
    fn tick_report<H: AuthLike>(&mut self, t: Tick, cx: &Cx<'_, H>, _fx: &mut Effects<'_, H>) {
        if self.alert.is_open() && !self.plaintext_asks() {
            // The server the question is about is no longer the failure on screen.
            self.question.withdraw(&mut self.alert);
        }
        self.alert.update(t.dt());
        let focused = cx
            .focus
            .current
            .filter(|k| k.entry == self.entry)
            .and_then(|k| slot_of(k.elem));
        let pops = if self.alert.visible() { None } else { focused };
        self.pop.step(pops, t.dt());
    }
    fn control_kind(&self) -> Option<ControlKind> {
        if self.persistence_warning.is_some() {
            return Some(ControlKind::ContinueUnsaved);
        }
        match self.phase {
            Phase::Deleted => Some(ControlKind::StartLogin),
            Phase::Error if self.plaintext_asks() => Some(ControlKind::ConnectPlaintext),
            Phase::Error => Some(ControlKind::Retry),
            Phase::Waiting if qr_escape_offered(self.phase_ms) => Some(ControlKind::RestartWait),
            p if working_phase(p) && escape_offered(self.phase_ms) => {
                Some(ControlKind::RestartWait)
            }
            _ => None,
        }
    }


    /// Whether the failure on screen still ASKS: eligible and not yet answered
    /// (`plaintext_question::asks`).
    fn plaintext_asks(&self) -> bool {
        self.phase == Phase::Error && plaintext_question::asks(self.plaintext.as_ref())
    }

    /// Ask "Connect without encryption?" — the shared question on this screen's alert.
    fn open_plaintext<H: AppLike>(&mut self, fx: &mut Effects<'_, H>) {
        let Some(v) = self.plaintext.as_ref().filter(|_| self.plaintext_asks()) else {
            return;
        };
        self.question.open(&mut self.alert, &v.machine_id, None);
        Self::enter_group(fx, ALERT_GROUP);
    }

    /// Whether the read-out's primary action is on screen.
    fn has_control(&self) -> bool {
        self.row().n > 0
    }

    /// The control group in walk order. On the read-out the primary leads (it is the row's centre
    /// of gravity); on the QR screen *Details* sits bottom-left in the narrative column and the
    /// escape sentence in the content column, so the walk goes left to right.
    fn row(&self) -> Row {
        let mut row = Row::default();
        let primary = self.control_kind().is_some();
        // Both phases used to walk differently — the Waiting arm led with the telemetry
        // *Details* control, removed with the reporting surface itself — and now agree: the
        // primary alone, when there is one.
        if primary {
            row.push(CONTROL);
        }
        row
    }

    /// The persistence warning's one status line: the helper failure's photographable storage
    /// stage.
    fn report_note(&self) -> Option<Note> {
        let helper = self.persistence_warning.and_then(|warning| warning.helper)?;
        Some(Note { text: CString::new(helper.line()).unwrap(), busy: false })
    }
    /// The read-out's control labels by slot (primary, *Details*), and whether the read-out
    /// carries a reason. How tall the reason is — one line, or a `Failed` read-out's
    /// two-line slot — is the widget's answer from the kind.
    fn readout_labels(&self) -> ([Option<&'static str>; 2], bool) {
        let Some(kind) = self.control_kind() else {
            return ([None; 2], false);
        };
        let has_reason = match kind {
            ControlKind::RestartWait => true, // Working's own stall reason is unconditional once offered
            ControlKind::Retry => !self.error.is_empty(),
            ControlKind::StartLogin => true, // `deleted_readout` always states one
            ControlKind::ContinueUnsaved => true, // the warning sentence is unconditional
            ControlKind::ConnectPlaintext => true, // `auth::insecure_only_copy` always states one
        };
        ([Some(label_for(kind)), None], has_reason)
    }

    /// The read-out's treatment — the one answer `draw` and the control geometry both read: the
    /// unconfirmed save and a failed sign-in are `Failed`, the finished delete `Empty`, a phase
    /// still in flight `Working`.
    fn readout_kind(&self) -> StatusKind {
        if self.persistence_warning.is_some() {
            return StatusKind::Failed;
        }
        match self.phase {
            Phase::Error => StatusKind::Failed,
            Phase::Deleted => StatusKind::Empty,
            _ => StatusKind::Working,
        }
    }

    /// Every element's rect — shared verbatim by `draw`'s stops and every `Focusable` query, so
    /// the two can never drift apart (`ui/table_screen.rs`'s rule).
    fn elem_rect(&self, elem: u32, measure: &dyn Measure) -> Option<Rect> {
        self.row().position(elem)?;
        if self.phase == Phase::Waiting && self.persistence_warning.is_none() {
            return match elem {
                // The QR screen's escape is a SENTENCE, not a button (see `waiting_status`'s doc),
                // so its geometry is the status line's own rect rather than a computed pill.
                CONTROL => Some(qr_layout(RouteLayout::screen()).status),
                _ => None,
            };
        }
        let (labels, has_reason) = self.readout_labels();
        let mut b0 = [0u8; crate::i18n::TC_MAX];
        let mut b1 = [0u8; crate::i18n::TC_MAX];
        let l0 = match labels[0] { Some(l) => Some(crate::i18n::tcstr(l, &mut b0)), None => None };
        let l1 = match labels[1] { Some(l) => Some(crate::i18n::tcstr(l, &mut b1)), None => None };
        let frames = status_row_rects(measure, [l0, l1], self.readout_kind(), has_reason);
        frames[slot_of(elem)?]
    }

    /// The control, pressed. Every action is a typed Session command. Restart additionally records
    /// its addressed correlation and leaves the elapsed clock untouched until acceptance returns.
    fn allocate_reply<H: AppLike>(&mut self, fx: &Effects<'_, H>) -> Option<auth::owner::ReplyTo> {
        let crate::ui::machine::MachineId::Instance(instance) = fx.from() else {
            return None;
        };
        let correlation = self.next_correlation?;
        let next = correlation.checked_add(1)?;
        self.next_correlation = Some(next);
        Some(auth::owner::ReplyTo {
            instance: instance.0,
            correlation,
        })
    }

    fn request_root_back<H: AppLike>(&mut self, fx: &mut Effects<'_, H>) {
        let Some(reply) = self.allocate_reply(fx) else {
            self.sync_state();
            return;
        };
        fx.push(Fx::App(AppFx::Session(auth::SessionCmd::BackAtRoot {
            reply,
        })));
        self.sync_state();
    }

    /// An element of the control group, pressed.
    fn activate_elem<H: AppLike>(&mut self, elem: u32, fx: &mut Effects<'_, H>) {
        if self.row().position(elem).is_none() {
            // Not on screen this frame — a control that is not drawn is never activated.
            return;
        }
        self.activate(fx);
    }
    /// The alert's answer. *Connect* allows this server and retries at once; *Not now* and BACK
    /// record the refusal, and the read-out says how to be asked again. Session persists either.
    /// The shared question dismisses the alert.
    fn alert_answer<H: AppLike>(&mut self, send: bool, fx: &mut Effects<'_, H>) {
        if !self.alert.is_open() {
            return;
        }
        if let Some(super::plaintext_question::QuestionAnswer::Plaintext(cmd)) =
            self.question.answer(&mut self.alert, send)
        {
            fx.push(Fx::App(AppFx::Session(cmd)));
        }
        if self.has_control() {
            Self::enter_group(fx, CONTROL_GROUP);
        }
        self.sync_state();
    }
    fn activate<H: AppLike>(&mut self, fx: &mut Effects<'_, H>) {
        match self.control_kind() {
            Some(ControlKind::ContinueUnsaved) => {
                if let Some(warning) = self.persistence_warning {
                    fx.push(Fx::App(AppFx::Session(
                        auth::SessionCmd::AcknowledgePersistenceWarning { key: warning.key },
                    )));
                }
            }
            Some(ControlKind::StartLogin) => {
                fx.push(Fx::App(AppFx::Session(auth::SessionCmd::StartLogin)));
            }
            Some(ControlKind::Retry) => {
                fx.push(Fx::App(AppFx::Session(auth::SessionCmd::Retry)));
            }
            Some(ControlKind::ConnectPlaintext) => self.open_plaintext(fx),
            Some(ControlKind::RestartWait) => {
                // "requested", not "restarted": the press may still be refused (the flow moved on
                // between this screen's last `Tick` and this key), and the event log is the one
                // place that failure is read from — a claim it did something is exactly the wrong
                // thing to have written there.
                crate::log("login: user requested a restart of a stalled sign-in");
                if self.pending_restart.is_none() {
                    if let Some(reply) = self.allocate_reply(fx) {
                        self.pending_restart = Some(PendingRestart {
                            correlation: reply.correlation,
                            wait: self.wait,
                        });
                        fx.push(Fx::App(AppFx::Session(auth::SessionCmd::RestartWait {
                            phase: self.wait.0,
                            qr_generation: self.wait.1,
                            reply,
                        })));
                    }
                }
            }
            None => {}
        }
        self.sync_state();
    }

    fn restart_reply(&mut self, request: u32, correlation: u32, accepted: bool) {
        if request != correlation {
            return;
        }
        let Some(pending) = self.pending_restart else {
            return;
        };
        if pending.correlation != correlation || pending.wait != self.wait {
            return;
        }
        self.pending_restart = None;
        if accepted {
            self.phase_ms = 0.0;
            self.phase_clock = crate::ui::motion::Phase::default();
        }
        self.sync_state();
    }

    fn draw_readout<H: AppLike>(
        &self,
        f: &mut DrawFrame<'_, '_, H>,
        p: Painter,
        env: &Env,
        caption: &CStr,
        kind: StatusKind,
        reason: Option<&CStr>,
        focus: Option<u32>,
    ) {
        let note = self.report_note();
        let mut b0 = [0u8; crate::i18n::TC_MAX];
        let mut b1 = [0u8; crate::i18n::TC_MAX];
        let o = self.readout(caption, kind, reason, note.as_ref(), [&mut b0, &mut b1]);
        self.draw_overlay(f, p, env, o, focus);
    }

    /// **The read-out as it is drawn** — the one overlay `draw_overlay` paints and the tests read.
    /// Its controls are [`Self::readout_labels`], the same answer the geometry and the press read,
    /// so the pill on screen always names what OK does: no stage passes a label of its own.
    /// `label_bufs` are the CALLER's: the overlay this returns borrows the rendered labels, so the
    /// buffers live in the draw fn that also draws it — one borrow, one frame.
    fn readout<'a>(
        &self,
        caption: &'a CStr,
        kind: StatusKind,
        reason: Option<&'a CStr>,
        note: Option<&'a Note>,
        label_bufs: [&'a mut [u8; crate::i18n::TC_MAX]; 2],
    ) -> StatusOverlay<'a> {
        debug_assert_eq!(kind, self.readout_kind(), "the geometry reads the same kind the draw paints");
        let (labels, _) = self.readout_labels();
        let [lb0, lb1] = label_bufs;
        let l0 = match labels[0] { Some(l) => Some(crate::i18n::tcstr(l, lb0)), None => None };
        let l1 = match labels[1] { Some(l) => Some(crate::i18n::tcstr(l, lb1)), None => None };
        readout_overlay(caption, kind, reason, [l0, l1], note).phase(self.spin_ms as u32)
    }

    /// The failed sign-in's read-out: the verdict, the phase's reason and its controls. `bufs`
    /// are the caller's render buffers (see [`Self::readout`]).
    fn failed_readout<'a>(
        &self,
        reason: &'a CStr,
        note: Option<&'a Note>,
        bufs: [&'a mut [u8; crate::i18n::TC_MAX]; 3],
    ) -> StatusOverlay<'a> {
        let [cap, lb0, lb1] = bufs;
        self.readout(
            crate::i18n::tcstr(crate::i18n::t("Couldn\u{2019}t sign in"), cap),
            StatusKind::Failed,
            (!reason.is_empty()).then_some(reason),
            note,
            [lb0, lb1],
        )
    }

    fn draw_overlay<H: AppLike>(
        &self,
        f: &mut DrawFrame<'_, '_, H>,
        p: Painter,
        env: &Env,
        mut o: StatusOverlay<'_>,
        focus: Option<u32>,
    ) {
        let press = f.press.scale;
        let pop = &self.pop;
        if o.action.is_some() {
            o = o
                .focus(focus.and_then(slot_of))
                .scales([0, 1].map(|i| pop.scale_with(i, press)));
        }
        o.draw_measured(env, p, f.measure);
        if self.alert.visible() {
            // The alert owns the pointer while it is up; nothing under it is a target.
            return;
        }
        let frames = o.action_frames_measured(f.measure);
        for elem in self.row().as_slice() {
            let Some(rect) = slot_of(*elem).and_then(|i| frames[i]) else {
                continue;
            };
            self.control_stop(f, p, *elem, rect);
        }
    }

    fn control_stop<H: AppLike>(&self, f: &mut DrawFrame<'_, '_, H>, p: Painter, elem: u32, rect: Rect) {
        f.stop(
            p,
            Stop {
                key: crate::ui::machine::FocusKey {
                    entry: self.entry,
                    elem,
                },
                rect,
                rest_rect: rect,
                clip: Rect::FULL,
                hover: Hover::Focus,
                activate: Activate::Direct,
            },
        );
    }

    fn draw_working<H: AppLike>(
        &self,
        f: &mut DrawFrame<'_, '_, H>,
        p: Painter,
        env: &Env,
        msg: &str,
        focus: Option<u32>,
    ) {
        let caption = CString::new(msg).unwrap_or_default();
        let stuck = self.has_control();
        let discovery_trouble = discovery_trouble_visible(self.phase, self.discovery_retry,
            self.phase_ms, self.discovery_retry_observed_phase_ms)
            .then(|| CString::new(auth::DISCOVERY_TROUBLE).unwrap_or_default());
        self.draw_readout(
            f,
            p,
            env,
            &caption,
            StatusKind::Working,
            // The reason arrives WITH the control, and only then: it exists to explain why a
            // button just appeared under a spinner that was doing fine a moment ago.
            discovery_trouble.as_deref().or_else(|| stuck.then_some(c"This is taking longer than usual.")),
            focus,
        );
    }

    /// AUTH-03: the fresh save the disk could not confirm durable. Drawn instead of the phase's
    /// ordinary read-out whenever a warning is showing — see `draw`'s own check.
    fn draw_warning<H: AppLike>(
        &self,
        f: &mut DrawFrame<'_, '_, H>,
        p: Painter,
        env: &Env,
        focus: Option<u32>,
    ) {
        let mut cap = [0u8; crate::i18n::TC_MAX];
        let caption = crate::i18n::tcstr(crate::i18n::t("Couldn\u{2019}t save your sign-in"), &mut cap);
        self.draw_readout(
            f,
            p,
            env,
            caption,
            StatusKind::Failed,
            Some(c"Your sign-in couldn\u{2019}t be saved on this TV. You can continue, but you\u{2019}ll be asked to sign in again next time."),
            focus,
        );
    }

    fn draw_failed<H: AppLike>(
        &self,
        f: &mut DrawFrame<'_, '_, H>,
        p: Painter,
        env: &Env,
        focus: Option<u32>,
    ) {
        let reason = CString::new(self.error.as_ref()).unwrap_or_default();
        let note = self.report_note();
        let mut cap = [0u8; crate::i18n::TC_MAX];
        let mut b0 = [0u8; crate::i18n::TC_MAX];
        let mut b1 = [0u8; crate::i18n::TC_MAX];
        let o = self.failed_readout(&reason, note.as_ref(), [&mut cap, &mut b0, &mut b1]);
        self.draw_overlay(f, p, env, o, focus);
    }

    /// **Empty, not Failed.** Deleting everything is a completed action the user asked for, so it
    /// must not read as a failure — the same distinction `StatusKind::Empty` carries for a
    /// library with nothing in it. A partial one is still not a FAILURE either: what it did do, it
    /// did.
    fn draw_deleted<H: AppLike>(
        &self,
        f: &mut DrawFrame<'_, '_, H>,
        p: Painter,
        env: &Env,
        focus: Option<u32>,
    ) {
        let (verdict, reason) = deleted_readout(self.delete_leftovers);
        let mut cap = [0u8; crate::i18n::TC_MAX];
        let mut why = [0u8; crate::i18n::TC_MAX];
        let verdict = crate::i18n::tcstr(verdict, &mut cap);
        let reason = crate::i18n::tcstr(reason, &mut why);
        self.draw_readout(
            f,
            p,
            env,
            verdict,
            StatusKind::Empty,
            Some(reason),
            focus,
        );
    }

    /// **The escape SENTENCE takes no focus look, unlike its three siblings' pills.**
    /// `draw_readout` (shared by `draw_working`/`draw_failed`/`draw_deleted`) paints its action as
    /// a real `Button` face, whose fill genuinely differs when focused — legacy hardcoded
    /// `.focused(true)` there because "the only control on the screen … holds focus by
    /// construction"; the owned screen instead threads the engine's actual focus state through,
    /// which happens to answer the same `true` here since there is still nowhere else focus could
    /// be. The QR screen's 60-second escape is not that kind of control: it is a SENTENCE (the
    /// module doc's own word for it), drawn as plain text beside a spinner with no button chrome
    /// for a `focused` bool to vary — legacy's own `draw_waiting` (what this ports) never took one
    /// either, for the identical reason, so this is not a dropped behaviour. Giving this sentence a
    /// focus-dependent look would be a NEW affordance, and a visual one belongs in front of
    /// `ui/CLAUDE.md`'s own design review, not slipped in unreviewed by a bug-fix pass. The report's
    /// *Details* pill is an ordinary control in the route's action band and DOES take `focus`.
    fn draw_waiting<H: AppLike>(&self, f: &mut DrawFrame<'_, '_, H>, p: Painter, _focus: Option<u32>) {
        let layout = RouteLayout::screen();
        layout.draw_narrative(
            p,
            None,
            "Sign in to Plex",
            "Use your phone camera to scan the code, or link this television manually with the address and code shown here.",
            theme::size::LABEL,
            f.measure,
        );
        let right = qr_layout(layout);

        TextView::new("plex.tv/link", theme::size::TITLE, theme::TEXT_HEADING)
            .bold()
            .h(HAlign::Center)
            .draw(p, right.url);

        // QR on a bright card (the white border is the scan quiet-zone). Plex's own PNG → we just
        // show it.
        let card = right.card;
        p.rrect(card, 24.0, 24.0, theme::SURFACE_QR_PLATE);
        if self.qr_tex != 0 {
            let pad = 30.0;
            let inner = Rect::new(
                card.x + pad,
                card.y + pad,
                card.w - 2.0 * pad,
                card.h - 2.0 * pad,
            );
            // Plex's PNG is WHITE modules on a transparent ground; tint black so the modules render
            // dark on the white card (the transparent ground shows the card) → a scannable
            // black-on-white QR.
            p.tex(self.qr_tex, inner, 0.0, theme::scrim_black(1.0));
        } else {
            Spinner::new(card.x + card.w * 0.5, card.y + card.h * 0.5, 22.0)
                .phase(self.spin_ms as u32)
                .tint(theme::scrim_black(0.5))
                .draw(&Env::inert(), p);
        }

        // The manual code and waiting state remain in the same right-column stack as the URL and
        // QR. Both use couch-readable type rungs; this is an alternative sign-in path, not fine
        // print.
        if let Ok(code) = CString::new(self.qr_code.to_uppercase()) {
            p.text(
                code.as_ptr(),
                right.code.cx(),
                right.code.y,
                theme::size::DISPLAY,
                theme::TEXT_PRIMARY,
                1,
                1,
            );
        }

        let wr = 15.0;
        let wy = right.status.cy();
        let escaping = qr_escape_offered(self.phase_ms);
        let status = waiting_status(self.qr_replaced, escaping, self.link_trouble);
        let status_w = f.measure.width(status, theme::size::BODY, false);
        let sx = right.status.cx() - (wr * 2.0 + theme::space::SM + status_w) * 0.5;
        Spinner::new(sx + wr, wy, wr)
            .phase(self.spin_ms as u32)
            .tint(theme::TEXT_SECONDARY)
            .draw(&Env::inert(), p);
        let ty = crate::text::text_vcenter_y(theme::size::BODY, 0, wy);
        p.text(
            status.as_ptr(),
            sx + wr * 2.0 + theme::space::SM,
            ty,
            theme::size::BODY,
            theme::TEXT_SECONDARY,
            0,
            0,
        );

        self.draw_disclosure(p, layout, f.measure);

        if self.alert.visible() {
            return;
        }
        if escaping {
            self.control_stop(f, p, CONTROL, right.status);
        }
    }

    /// The QR screen's status line — [`Self::report_note`], the failed read-out's own line — as
    /// fine print sitting on the action band, in the narrative column.
    fn draw_disclosure(&self, p: Painter, layout: RouteLayout, measure: &dyn crate::ui::machine::Measure) {
        let bottom = layout.action.y - theme::space::MD;
        if let Some(note) = self.report_note() {
            // A report on its way: the shared inline spinner in a leading gutter, on the first
            // line's cap band (TextView sets each line cap-top), the text one gutter over.
            let gutter = if note.busy { Spinner::inline_gutter() } else { 0.0 };
            let col = Rect::new(layout.narrative.x + gutter, 0.0, layout.narrative.w - gutter, 0.0);
            let line = note.text.to_string_lossy();
            let view = TextView::new(&line, theme::size::CAPTION, theme::TEXT_TERTIARY).max_lines(2);
            let h = view.measure_h(col.w);
            let top = bottom - h;
            if note.busy {
                let cap = measure.cap_h(theme::size::CAPTION);
                Spinner::leading(layout.narrative.x, top + cap / 2.0)
                    .phase(self.spin_ms as u32)
                    .tint(theme::TEXT_TERTIARY)
                    .draw(&Env::inert(), p);
            }
            view.draw(p, Rect::new(col.x, top, col.w, h));
        }
    }

    /// The alert — the report question or the Details card — over whatever the phase drew, and its
    /// answers' stops once it has settled (a pointer hit is positional — `DecisionAlert::settled`).
    fn draw_alert<H: AppLike>(&mut self, f: &mut DrawFrame<'_, '_, H>) {
        if !self.alert.visible() {
            return;
        }
        self.alert.draw_scrim();
        let (cancel, affirm) = self.question.verbs();
        self.alert.draw(&cancel, &affirm);
        let frames = self.alert.frames();
        self.alert_frames = Some(frames);
        if self.alert.is_open() && self.alert.settled() {
            for &elem in self.alert_elems() {
                let rect = if elem == ALERT_AFFIRM { frames.1 } else { frames.0 };
                f.stop(
                    Painter::root(),
                    Stop {
                        key: crate::ui::machine::FocusKey { entry: self.entry, elem },
                        rect,
                        rest_rect: rect,
                        clip: Rect::FULL,
                        hover: Hover::Focus,
                        activate: Activate::Press,
                    },
                );
            }
        }
    }

    /// Build+free the QR texture. GL only, called from [`Screen::prepare`] — never from `step`, so
    /// a host test driving `Machine::step` alone never touches it.
    fn prepare_qr_tex(&mut self) {
        self.drop_stale_qr_tex(self.qr_gen);
        let Some((gen, png)) = self.qr_png_pending.take() else {
            return;
        };
        // The belt to the drop above's braces: a code replaced again between the `Tick` that
        // captured this PNG and this `prepare` (rare — it needs two replacements inside one frame)
        // must not upload a bitmap for a code that has already died.
        self.drop_stale_qr_tex(gen);
        if self.qr_tex != 0 || png.is_empty() {
            return;
        }
        let (mut w, mut h): (c_int, c_int) = (0, 0);
        let px = crate::img::img_decode_rgba(png.as_ptr(), png.len() as c_int, &mut w, &mut h);
        if !px.is_null() {
            self.qr_tex = crate::img::img_upload_rgba(px, w, h);
            self.qr_px = if self.qr_tex != 0 {
                (w.max(0) as u32, h.max(0) as u32)
            } else {
                (0, 0)
            };
            self.qr_tex_gen = gen;
            crate::img::img_free(px);
        }
    }

    fn drop_stale_qr_tex(&mut self, live: u64) {
        if !qr_cache_stale(self.qr_tex_gen, live, self.phase) {
            return;
        }
        crate::gfx::delete_tex(self.qr_tex);
        self.qr_tex = 0;
        self.qr_px = (0, 0);
        self.qr_tex_gen = live;
    }
}

impl LoginScreen {
    /// The alert's answer rects as last drawn. Before the first draw there is no measured panel;
    /// the answers are not stops until the sheet has settled anyway.
    fn alert_rect(&self, elem: u32) -> Rect {
        self.alert_frames
            .map(|(cancel, affirm)| if elem == ALERT_AFFIRM { affirm } else { cancel })
            .unwrap_or(Rect::FULL)
    }

    fn key(&self, elem: u32) -> crate::ui::machine::FocusKey<u32> {
        crate::ui::machine::FocusKey { entry: self.entry, elem }
    }

    /// The open alert's answers: the plaintext question's *Not now* / *Connect*.
    fn alert_elems(&self) -> &'static [u32] {
        match self.alert.answers() {
            crate::ui::decision_alert::Answers::Two => &[ALERT_CANCEL, ALERT_AFFIRM],
            crate::ui::decision_alert::Answers::One => &[ALERT_CANCEL],
        }
    }
}

/// **Two groups, never at once.** While the alert is OPEN its answers are the only group — the player's repair alert's shape — and the read-out's controls return the moment it
/// is answered (its fade is drawn, not focusable: `groups` gates on `is_open`, not `visible`, so
/// the focus a dismissal hands back has somewhere to land).
impl<H: AppLike> Focusable<H> for LoginScreen {
    fn groups(&self, cx: &Cx<'_, H>, out: &mut Vec<GroupSpec>) {
        if self.alert.is_open() {
            out.push(GroupSpec {
                id: ALERT_GROUP,
                kind: GroupKind::Row { wrap: false },
                seat: Seat::First,
                reachable: AxisMask::BOTH,
                edge: [EdgeRule::Stop; 4],
                extent: self
                    .alert_elems()
                    .iter()
                    .map(|&e| self.alert_rect(e))
                    .reduce(|a, b| a.union(b))
                    .unwrap_or(Rect::FULL),
                len: self.alert_elems().len(),
                elem: ElemKind::Control,
            });
            return;
        }
        let row = self.row();
        let mut extent: Option<Rect> = None;
        for elem in row.as_slice() {
            if let Some(r) = self.elem_rect(*elem, cx.measure) {
                extent = Some(extent.map_or(r, |e| e.union(r)));
            }
        }
        let Some(extent) = extent else {
            return;
        };
        out.push(GroupSpec {
            id: CONTROL_GROUP,
            kind: GroupKind::Free,
            seat: Seat::First,
            reachable: AxisMask::BOTH,
            // Nothing else is ever focusable on this screen, so a walk off either end of the
            // group stays put rather than searching for a sibling group that does not exist.
            edge: [EdgeRule::Stop; 4],
            extent,
            len: row.n,
            elem: ElemKind::Bare,
        });
    }
    fn group_of(&self, key: &u32, _cx: &Cx<'_, H>) -> Option<GroupId> {
        if self.alert.is_open() {
            return self.alert_elems().contains(key).then_some(ALERT_GROUP);
        }
        self.row().position(*key).map(|_| CONTROL_GROUP)
    }
    fn neighbour(
        &self,
        key: crate::ui::machine::FocusKey<u32>,
        dir: Dir,
        _cx: &Cx<'_, H>,
    ) -> Step<u32> {
        if self.alert.is_open() {
            return match (key.elem, dir) {
                (ALERT_CANCEL, Dir::Right) if self.alert_elems().contains(&ALERT_AFFIRM) => {
                    Step::Move(self.key(ALERT_AFFIRM))
                }
                (ALERT_AFFIRM, Dir::Left) => Step::Move(self.key(ALERT_CANCEL)),
                _ => Step::Edge,
            };
        }
        // The group is one walk: LEFT/UP to the previous element, RIGHT/DOWN to the next.
        let row = self.row();
        let Some(at) = row.position(key.elem) else {
            return Step::Edge;
        };
        let next = match dir {
            Dir::Left | Dir::Up => at.checked_sub(1),
            Dir::Right | Dir::Down => Some(at + 1).filter(|&i| i < row.n),
        };
        next.map_or(Step::Edge, |i| Step::Move(self.key(row.elems[i])))
    }
    fn place(&self, key: &u32, cx: &Cx<'_, H>, _at: At) -> Option<Placed> {
        if self.alert.is_open() {
            if !self.alert_elems().contains(key) {
                return None;
            }
            let rect = self.alert_rect(*key);
            return Some(Placed {
                rect,
                rest_rect: rect,
                clip: Rect::FULL,
                index: Some((*key == ALERT_AFFIRM) as u32),
            });
        }
        let index = self.row().position(*key)?;
        let rect = self.elem_rect(*key, cx.measure)?;
        Some(Placed {
            rect,
            rest_rect: rect,
            clip: Rect::FULL,
            index: Some(index as u32),
        })
    }
    /// A focused element that has left the row — the primary that stopped being offered — hands
    /// focus to the head of the group.
    fn reconcile(
        &self,
        want: crate::ui::machine::FocusKey<u32>,
        cx: &Cx<'_, H>,
    ) -> crate::ui::machine::FocusKey<u32> {
        if self.group_of(&want.elem, cx).is_some() {
            return want;
        }
        if self.alert.is_open() {
            return self.key(ALERT_CANCEL);
        }
        let row = self.row();
        if row.n == 0 {
            return want;
        }
        self.key(row.elems[0])
    }
    fn seat(
        &self,
        g: GroupId,
        _from: Placed,
        _cx: &Cx<'_, H>,
    ) -> crate::ui::machine::FocusKey<u32> {
        if g == ALERT_GROUP {
            // The question seats on *Not now*.
            return self.key(ALERT_CANCEL);
        }
        let row = self.row();
        self.key(if row.n > 0 { row.elems[0] } else { CONTROL })
    }
}

impl<H: AuthLike> Machine<H> for LoginScreen {
    type Ev = ScreenEvent<H>;
    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        // **The alert traps everything under it** while it is on screen, fade included — the
        // player repair alert's trap, verbatim in shape: its answers are `Control` elements (the
        // press dip, a commit on release), BACK is the cancel slot (*Not now* / *Close*), the
        // arrows and OK go on to the engine for the answers, and nothing reaches the read-out.
        if self.alert.visible() {
            match ev {
                ScreenEvent::FocusMoved { to, .. } => {
                    self.alert.set_choice(if to.elem == ALERT_AFFIRM {
                        crate::ui::decision_alert::Choice::Destructive
                    } else {
                        crate::ui::decision_alert::Choice::Cancel
                    });
                    return Handled::Yes;
                }
                ScreenEvent::PressCommit(_) => {
                    if let Some(key) = cx.focus.current {
                        if self.alert_elems().contains(&key.elem) {
                            self.alert_answer(key.elem == ALERT_AFFIRM, fx);
                        }
                    }
                    return Handled::Yes;
                }
                ScreenEvent::Activate(_) => return Handled::Yes,
                ScreenEvent::Input(input) => {
                    if !self.alert.is_open() {
                        return Handled::Yes;
                    }
                    use crate::ui::consts;
                    return match input.kind {
                        InputKind::Key { key, sym, wcode, edge, .. } => match consts::classify_input(key, sym, wcode) {
                            consts::Key::Back | consts::Key::Stop if edge == Edge::Down => {
                                self.alert_answer(false, fx);
                                Handled::Yes
                            }
                            consts::Key::Left { .. }
                            | consts::Key::Right { .. }
                            | consts::Key::Ok
                            | consts::Key::Exit => Handled::No,
                            _ => Handled::Yes,
                        },
                        InputKind::Pointer { .. } | InputKind::Click { .. } => Handled::No,
                        _ => Handled::Yes,
                    };
                }
                _ => {}
            }
        }
        match ev {
            ScreenEvent::Tick(t) => {
                self.tick(*t, cx, fx);
                Handled::Yes
            }
            ScreenEvent::Activate(elem) => {
                self.activate_elem(*elem, fx);
                fx.invalidate(crate::ui::present::Provenance::Input);
                Handled::Yes
            }
            ScreenEvent::Async(
                crate::ui::machine::RequestId(request),
                AppMsg::RestartReply {
                    correlation,
                    accepted,
                },
            ) => {
                self.restart_reply(*request, *correlation, *accepted);
                Handled::Yes
            }
            ScreenEvent::Async(
                crate::ui::machine::RequestId(request),
                AppMsg::BackReply { correlation, .. },
            ) if request == correlation => Handled::Yes,
            // BACK asks Session for its addressed root decision; Session/core owns stored-session
            // resume, cooldown and platform handling. (An open Details card is the alert, and the
            // trap above has already made BACK its *Close*.)
            ScreenEvent::Input(InputEvent {
                kind:
                    InputKind::Key {
                        key: Key::Back,
                        edge: Edge::Down,
                        ..
                    },
                ..
            }) => {
                self.request_root_back(fx);
                Handled::Yes
            }
            // **The QR texture has to be freed exactly here, and `Unmount` is the one event this
            // screen is guaranteed to see before it dies.** `AppMounter::mount` builds a brand new
            // `LoginScreen` on every `Replace` onto `Route::Login` (`app/bridge.rs:275`), so every
            // sign-in retry, sign-out and "Delete all local data" round trip mints a fresh
            // instance — and, before this arm existed, orphaned the outgoing one's GL texture on
            // every one of those visits: nothing ever deleted `qr_tex` for an instance that was
            // simply going away rather than replacing its OWN code. This is the same failure the
            // legacy single-`Scene` screen already had to name once — `ui/login.rs`'s
            // `drop_a_stale_qr` doc: "zeroing the handle alone orphaned a full 400x400-ish RGBA QR
            // bitmap per sign-in retry, with nothing left holding its id to free it later" — except
            // that comment guarded a re-upload inside one long-lived `Scene` across retries, which
            // never covered a whole INSTANCE being torn down, since the legacy screen had no such
            // thing. Freeing it in `Screen::prepare` (where the struct's own field doc says GL
            // resources are otherwise built and freed) is not an option: an unmounting instance
            // gets no further `prepare` call to do it in, so the free has to run wherever teardown
            // is actually observed, which for an owned `Screen` is here. `gfx::delete_tex` no-ops
            // on 0, so a screen that never uploaded a texture (never reached `Phase::Waiting`, or
            // reached it with no QR PNG yet) frees nothing.
            ScreenEvent::Unmount => {
                crate::gfx::delete_tex(self.qr_tex);
                self.qr_tex = 0;
                self.qr_px = (0, 0);
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}

impl<H: AuthLike> Screen<H> for LoginScreen {
    fn name(&self) -> &'static str {
        word::LOGIN
    }
    fn state(&self) -> &dyn LogicalState {
        &self.state
    }
    fn crumb(&self, _cx: &Cx<'_, H>) -> Option<std::borrow::Cow<'_, str>> {
        // This is one of the family's three routes with nowhere for BACK to go INSIDE the app
        // (`ui/CLAUDE.md`'s route-family rule) — see this screen's BACK arm.
        None
    }
    fn prepare(&mut self, _b: &mut Budget, _cx: &Cx<'_, H>) {
        self.prepare_qr_tex();
    }
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, H>) {
        let p = f.painter;
        // The QR screen is the first thing a new user sees, before Home has any artwork to lend
        // it. `Painter::root()`, not `p`, for the ground — the ambient wash must not ride whatever
        // page-transition cascade this screen's own painter carries, exactly as
        // `screens::onboard`'s first-run mounting draws its own ground the same way.
        self.ground.draw_default(Painter::root());
        let env = Env::inert();
        let focus = f.focus.current.map(|k| k.elem);

        if self.persistence_warning.is_some() {
            // AUTH-03: reachable before consent/profile routing, exactly as the phase-keyed
            // branches below — see `app/run.rs`'s own routing gate for the mirror of this check.
            self.draw_warning(f, p, &env, focus);
        } else {
            match self.phase {
                Phase::Waiting => self.draw_waiting(f, p, focus),
                Phase::Error => self.draw_failed(f, p, &env, focus),
                Phase::Deleted => self.draw_deleted(f, p, &env, focus),
                Phase::Discovering => {
                    self.draw_working(f, p, &env, "Finding your server\u{2026}", focus)
                }
                _ => self.draw_working(f, p, &env, "Connecting to Plex\u{2026}", focus),
            }
        }
        self.draw_alert(f);
    }
    fn render(&self) -> RenderStrategy {
        RenderStrategy::Page
    }
    /// The QR bitmap is the one render this screen owns (§8.3 rule (c)) — its own `upload_rgba` in
    /// [`Self::prepare_qr_tex`], its own `delete_tex` on `Unmount`. Everything else here is drawn
    /// immediate-mode or comes from a shared cache. The size is whatever plex.tv's PNG decoded to,
    /// which is why it is recorded rather than assumed.
    fn render_report(&self) -> crate::ui::frame::RenderReport {
        if self.qr_tex == 0 {
            return crate::ui::frame::RenderReport::NONE;
        }
        crate::ui::frame::RenderReport::one(self.qr_px.0, self.qr_px.1)
    }
    fn focus_source(&self) -> FocusSource {
        FocusSource::Engine
    }
    // **A click only lands inside the one control's own rect, never anywhere else on the screen —
    // a deliberate departure from the pre-phase-6 behaviour, not an oversight.** The legacy loop's
    // `Route::Login` click arm (`app/run.rs`, before 957bdc4d) fired
    // `crate::ui::login::key(SDLK_RETURN, 0)` on ANY click anywhere on this route, because that
    // screen predates real per-widget hit testing and "one actionable thing on the login screen"
    // was reason enough to skip building it; `key()` itself still gated on whether a control was
    // actually offered, so the only visible effect was that a tap on the QR image, the URL, or
    // empty space fired Retry/Sign-in/the stalled-wait escape whenever one happened to be showing.
    // That is exactly the failure mode real hit testing exists to rule out everywhere else in this
    // family (a tap near a poster's shadow must not activate a card three rows away).
    // `HitSource::Engine` resolves a click against `Focusable::place`'s own rect like every other
    // owned screen, so a tap has to land on the pill (or, on the QR screen, the status sentence)
    // to do anything. Restoring click-anywhere would be a step backward, not a port.
    fn hit_source(&self) -> HitSource {
        HitSource::Engine
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, LazyLock};

    use crate::ui::consts::inside_safe;
    use crate::ui::machine::{
        FocusRead, Host, InputOwner, InstanceId, MachineId, PressRead, Source, Stamped,
    };
    use crate::ui::present::Present;

    struct SessionHost;

    impl Host for SessionHost {
        type Arg = super::super::family::SettingsPage;
        type Fx = AppFx;
        type Msg = AppMsg;
        type Elem = u32;
        type Views<'a> = auth::SessionRead<'a>;
        type Init = super::super::family::NoInit;
        type Memory = ();
    }

    impl AuthLike for SessionHost {
        fn auth<'a>(cx: &Cx<'a, Self>) -> auth::SessionRead<'a> {
            cx.views
        }
    }

    fn snapshot(phase: Phase, qr_generation: u64, code: &str) -> auth::owner::SessionSnapshot {
        auth::owner::SessionSnapshot {
            flow_epoch: 0,
            phase,
            qr_generation,
            code: Arc::from(code),
            png: Arc::from(Vec::<u8>::new()),
            code_replaced: false,
            users: Arc::from(Vec::<auth::UserTile>::new()),
            error: Arc::from(""),
            pin_denied: false,
            profile: None,
            scope: auth::owner::ProfileScope(0),
            delete_leftovers: 0,
            persistence_warning: None,
            link_trouble: false,
            discovery_retry: None,
            plaintext: None,
            switch_refused: false,
            readout_back_resumes: false,
        }
    }

    static EMPTY_SNAPSHOT: LazyLock<auth::owner::SessionSnapshot> =
        LazyLock::new(|| snapshot(Phase::Idle, 0, ""));

    /// **A partial wipe may not be reported as a whole one.** The sweep's candidate lists span
    /// both webOS install prefixes and the jail profiles disagree about which are writable, so a
    /// survivor is ordinary — and the survivor can be the TELEMETRY decision, which is then
    /// re-read on the next launch. Saying "telemetry has been removed" over that is the one
    /// sentence on this screen that could be actively false. Ported verbatim from `ui/login.rs`.
    #[test]
    fn uncertain_db8_reply_appears_on_the_login_warning_stage_line() {
        for reconcile in [false, true] {
            let outcome = crate::plex::session::persistence::uncertain_helper_reply_for_test(reconcile);
            let mut screen = bare_screen(Phase::Ready, 0.0);
            screen.persistence_warning = Some(auth::owner::PersistenceWarning::from_outcome(
                auth::owner::PersistenceWarningKey { epoch: 1, req: 1 },
                auth::owner::PersistenceWarningSite::Final, &outcome,
            ));
            assert_eq!(screen.report_note().unwrap().text.to_str().unwrap(), "storage: helper · db8 (-3963)");
        }
    }

    #[test]
    fn helper_failure_warning_line_is_only_for_helper_failures() {
        use crate::storage::wire::failure::{HelperFailure, Stage};
        let mut screen = bare_screen(Phase::Ready, 0.0);
        assert!(screen.report_note().is_none());
        screen.persistence_warning = Some(auth::owner::PersistenceWarning {
            key: auth::owner::PersistenceWarningKey { epoch: 1, req: 1 },
            site: auth::owner::PersistenceWarningSite::Final,
            helper: Some(HelperFailure { start_timeout: true,
                ..HelperFailure::new(Stage::RuntimeAbsent, Some(libc::ENOENT)) }),
            candidate_errnos: [None; 8], persistence: None,
        });
        assert!(screen.report_note().unwrap().text.to_string_lossy().contains("storage: start-timeout · no runtime dir"));
        screen.persistence_warning.as_mut().unwrap().helper = None;
        assert!(screen.report_note().is_none());
    }

    #[test]
    fn a_partial_wipe_does_not_claim_a_whole_one() {
        let (whole, whole_why) = deleted_readout(0);
        let (partial, partial_why) = deleted_readout(2);
        assert_ne!(whole, partial);
        assert!(whole_why.as_bytes().windows(9).any(|w| w == b"telemetry"));
        assert!(
            !partial_why.as_bytes().windows(9).any(|w| w == b"telemetry"),
            "a partial wipe must not name what it may have failed to delete"
        );
        assert!(
            partial.as_bytes().windows(10).any(|w| w == b"Signed out"),
            "…but it still states what it DID do: the session is gone either way"
        );
    }

    /// **A stalled sign-in has to be escapable, and until 2026-09-02 it was not.** The control
    /// appears on a clock, so the whole rule is a pure predicate.
    ///
    /// **Pins the DESIGN NUMBER itself, not just the `>=` operator.** Every boundary used to be
    /// phrased in terms of `ESCAPE_AFTER_MS` (`ESCAPE_AFTER_MS - 1.0`, `ESCAPE_AFTER_MS`), so a
    /// mutation that changed the constant's value — the reviewer tried `120.0` — left this test
    /// green regardless: it was grading the comparison, not the twelve seconds. The literal
    /// `assert_eq!` and the literal millisecond boundaries below close that gap.
    #[test]
    fn a_wait_that_stops_looking_normal_grows_a_way_out() {
        assert_eq!(
            ESCAPE_AFTER_MS, 12_000.0,
            "the documented twelve-second design number"
        );
        assert!(!escape_offered(0.0), "a fresh wait offers nothing");
        assert!(
            !escape_offered(11_999.0),
            "nor does a healthy one — a button that flashes past teaches people to ignore it"
        );
        assert!(escape_offered(12_000.0));
    }

    #[test]
    fn discovery_reason_is_absent_on_the_first_miss_and_present_after_progress() {
        let first = auth::DiscoveryRetryProgress { run: auth::DiscoveryRetryRun::HomeUsers,
            misses: 1, elapsed_ms: 100 };
        assert!(!discovery_trouble_visible(Phase::Discovering, None, 20_000.0, 0.0),
            "a settled resources retry stays silent through slow probing");
        assert!(!discovery_trouble_visible(Phase::Discovering, Some(first),
            17_899.0, 10_000.0), "the Home-users run is only 7.999 seconds old");
        assert!(discovery_trouble_visible(Phase::Discovering, Some(first),
            17_900.0, 10_000.0), "eight seconds belongs to this retry run, not the phase");
        assert!(discovery_trouble_visible(Phase::Discovering,
            Some(auth::DiscoveryRetryProgress { misses: 2, ..first }), 10_001.0, 10_000.0),
            "the second miss is visible immediately");
        assert!(!discovery_trouble_visible(Phase::Waiting, Some(first), 20_000.0, 0.0));
    }

    #[test]
    fn first_discovery_miss_arriving_with_the_phase_anchors_after_its_clock_reset() {
        let waiting = snapshot(Phase::Waiting, 7, "AAAA");
        let mut discovering = snapshot(Phase::Discovering, 7, "");
        discovering.discovery_retry = Some(auth::DiscoveryRetryProgress {
            run: auth::DiscoveryRetryRun::Resources,
            misses: 1,
            elapsed_ms: 0,
        });
        let m = crate::ui::fixture::FixtureMeasure;
        let mut screen = LoginScreen::new(EntryId(0), waiting.read());

        step_ev_with(&mut screen, &tick_ev(100), &waiting, InstanceId(0), &m);
        step_ev_with(&mut screen, &tick_ev(5_100), &waiting, InstanceId(0), &m);
        assert_eq!(screen.phase_ms, 5_000.0, "the QR wait has its own older clock");

        step_ev_with(&mut screen, &tick_ev(5_100), &discovering, InstanceId(0), &m);
        assert_eq!(screen.phase_ms, 0.0, "Discovering resets the phase clock");
        assert!(!discovery_trouble_visible(screen.phase, screen.discovery_retry,
            screen.phase_ms, screen.discovery_retry_observed_phase_ms));

        step_ev_with(&mut screen, &tick_ev(13_100), &discovering, InstanceId(0), &m);
        assert!(discovery_trouble_visible(screen.phase, screen.discovery_retry,
            screen.phase_ms, screen.discovery_retry_observed_phase_ms),
            "the line appears eight seconds after this retry run began");
    }

    /// **The escape belongs ONLY to the two phases that wait on a network call.** Ported verbatim.
    #[test]
    fn only_a_phase_waiting_on_the_network_can_be_stalled() {
        assert!(working_phase(Phase::Creating));
        assert!(working_phase(Phase::Discovering));
        for settled in [
            Phase::Idle,
            Phase::Waiting,
            Phase::Profiles,
            Phase::Switching,
            Phase::Ready,
            Phase::Error,
            Phase::Deleted,
        ] {
            assert!(
                !working_phase(settled),
                "{settled:?} is not a wait this screen may offer to restart"
            );
        }
    }

    /// **A code that has been replaced may not go on being drawn**, which is the login screen's
    /// half of issue #30. Ported verbatim.
    #[test]
    fn a_replaced_code_invalidates_the_cached_qr_even_without_a_phase_change() {
        assert!(
            qr_cache_stale(4, 5, Phase::Waiting),
            "a new code was published while the screen never left Waiting"
        );
        assert!(
            !qr_cache_stale(5, 5, Phase::Waiting),
            "…and the settled case must not re-upload a texture every frame"
        );
        assert!(qr_cache_stale(5, 5, Phase::Creating));
    }

    /// The one line under the code has to explain a swap the user did not ask for. Ported
    /// verbatim.
    #[test]
    fn a_swapped_code_says_so_rather_than_changing_under_the_user() {
        let says = |s: &CStr, word: &[u8]| s.to_bytes().windows(word.len()).any(|w| w == word);
        assert!(says(waiting_status(false, false, false), b"Waiting"));
        assert!(
            says(waiting_status(true, false, false), b"expired"),
            "it names what happened; a code that simply changes reads as a fault"
        );
        assert!(says(waiting_status(true, true, false), b"press OK"));
        assert!(says(waiting_status(false, true, false), b"press OK"));
        // While plex.tv is not answering at all, neither a scan nor a new code can work: the line
        // says where the fault most likely is, whatever else is true.
        for (replaced, stalled) in [(false, false), (true, false), (false, true), (true, true)] {
            assert!(says(waiting_status(replaced, stalled, true), b"internet connection"));
        }
    }

    /// **The QR screen's clock is not the spinner's, and it must not be.**
    ///
    /// **Pins both DESIGN NUMBERS, not just their ratio.** The relative check
    /// (`QR_ESCAPE_AFTER_MS > ESCAPE_AFTER_MS * 4.0`) survives mutating BOTH constants together —
    /// the reviewer's example, `ESCAPE_AFTER_MS = 120.0` and `QR_ESCAPE_AFTER_MS = 600.0`, still
    /// satisfies `600 > 120 * 4`. The `assert_eq!`s and the literal millisecond boundaries below
    /// pin sixty seconds and twelve seconds as VALUES, so that mutation fails here even though the
    /// ratio it preserves would not have caught it.
    #[test]
    fn the_qr_screen_offers_a_new_code_on_a_much_longer_clock_than_a_stalled_spinner() {
        assert_eq!(ESCAPE_AFTER_MS, 12_000.0);
        assert_eq!(QR_ESCAPE_AFTER_MS, 60_000.0);
        assert!(QR_ESCAPE_AFTER_MS > ESCAPE_AFTER_MS * 4.0);
        assert!(!qr_escape_offered(0.0));
        assert!(
            !qr_escape_offered(12_000.0),
            "a sign-in that is merely twelve seconds old is going fine"
        );
        assert!(
            QR_ESCAPE_AFTER_MS < 900_000.0,
            "…and it must arrive well inside a code's own fifteen-minute life, or it is not a \
             recovery from anything"
        );
        assert!(!qr_escape_offered(59_999.0));
        assert!(qr_escape_offered(60_000.0));
    }

    /// **A new code starts a new clock, even if the phase change between them was never sampled.**
    /// Ported verbatim.
    #[test]
    fn a_replaced_code_restarts_the_wait_even_when_the_phase_never_appeared_to_change() {
        let old_code: Wait = (Phase::Waiting, 7u64);
        assert!(
            !wait_restarted(old_code, (Phase::Waiting, 7)),
            "the same code in the same phase is the same wait, and the clock must keep running"
        );
        assert!(
            wait_restarted(old_code, (Phase::Waiting, 8)),
            "a new code is a new wait, whatever the phase appeared to do in between — this is \
             the case a phase-only comparison misses, and it hands a one-millisecond-old code \
             its predecessor's sixty seconds"
        );
        assert!(
            wait_restarted(old_code, (Phase::Discovering, 7)),
            "…and the original rule still holds: a step forward is a fresh wait"
        );
    }

    /// Ported verbatim.
    #[test]
    fn qr_is_vertically_centred_and_the_whole_link_stack_stays_in_the_right_column() {
        let route = RouteLayout::screen();
        let q = qr_layout(route);
        assert_eq!(q.card.cy(), Rect::FULL.cy());
        for r in [q.url, q.card, q.code, q.status] {
            assert!(r.x >= route.content.x);
            assert!(r.x + r.w <= route.content.x + route.content.w);
            assert!(inside_safe(r));
        }
    }

    /// A bare screen, built with NO read of `crate::auth` at all — for testing [`ControlKind`]'s
    /// decision and the `Focusable` geometry without consulting even the local test host's
    /// Session publication.
    fn bare_screen(phase: Phase, phase_ms: f32) -> LoginScreen {
        LoginScreen {
            entry: EntryId(0),
            spin_ms: 0.0,
            phase_ms,
            spin_phase: crate::ui::motion::Phase::default(),
            phase_clock: crate::ui::motion::Phase::default(),
            wait: (phase, 0),
            qr_tex: 0,
            qr_tex_gen: 0,
            qr_px: (0, 0),
            qr_png_pending: None,
            phase,
            qr_gen: 0,
            qr_code: Arc::from(""),
            qr_replaced: false,
            error: Arc::from(""),
            discovery_retry: None,
            discovery_retry_observed_phase_ms: 0.0,
            delete_leftovers: 0,
            next_correlation: Some(1),
            pending_restart: None,
            persistence_warning: None,
            plaintext: None,
            question: PlaintextQuestion::new(),
            ground: RouteGround::new(),
            link_trouble: false,
            alert: DecisionAlert::new(),
            alert_frames: None,
            pop: CtlPop::new(),
            state: LoginState {
                phase: 0,
                qr_gen: 0,
                qr_replaced: false,
                has_control: false,
                delete_leftovers: 0,
                next_correlation: Some(1),
                pending_restart: None,
                warning: false,
                link_trouble: false,
                alert_open: false,
                plaintext: None,
            },
        }
    }

    /// **The one property this port exists to protect**: a control that is not drawn must not be
    /// activatable, restated for the owned screen as "a control exists only in exactly the phases
    /// `ui/login.rs`'s `key()` ladder ever acted on". `escape_ready`/`qr_escape_ready`'s old bodies
    /// are gone; this is the test that pins the same property against their replacement,
    /// `LoginScreen::control_kind`.
    ///
    /// **The boundaries below are literal milliseconds, not `ESCAPE_AFTER_MS`/`QR_ESCAPE_AFTER_MS`
    /// themselves.** Feeding a threshold its OWN constant as input proves only that `control_kind`
    /// compares two copies of the same symbol — it stays green under any value that constant takes,
    /// including the reviewer's mutated `ESCAPE_AFTER_MS = 120.0`. Literal `11_999.0`/`12_000.0` and
    /// `59_999.0`/`60_000.0` pin the twelve- and sixty-second design numbers themselves, matching
    /// the two tests above that pin the same constants directly.
    #[test]
    fn the_control_exists_only_where_legacy_ever_acted_on_a_press() {
        assert_eq!(bare_screen(Phase::Waiting, 0.0).control_kind(), None);
        assert_eq!(bare_screen(Phase::Waiting, 59_999.0).control_kind(), None);
        assert_eq!(
            bare_screen(Phase::Waiting, 60_000.0).control_kind(),
            Some(ControlKind::RestartWait)
        );
        assert_eq!(bare_screen(Phase::Creating, 0.0).control_kind(), None);
        assert_eq!(bare_screen(Phase::Creating, 11_999.0).control_kind(), None);
        assert_eq!(
            bare_screen(Phase::Creating, 12_000.0).control_kind(),
            Some(ControlKind::RestartWait)
        );
        assert_eq!(bare_screen(Phase::Discovering, 0.0).control_kind(), None);
        assert_eq!(
            bare_screen(Phase::Discovering, 11_999.0).control_kind(),
            None
        );
        assert_eq!(
            bare_screen(Phase::Discovering, 12_000.0).control_kind(),
            Some(ControlKind::RestartWait)
        );
        // Error and Deleted offer their control unconditionally — no clock involved.
        assert_eq!(
            bare_screen(Phase::Error, 0.0).control_kind(),
            Some(ControlKind::Retry)
        );
        assert_eq!(
            bare_screen(Phase::Deleted, 0.0).control_kind(),
            Some(ControlKind::StartLogin)
        );
        // The allowlist `working_phase` states explicitly: a phase the main loop is about to route
        // away from must never grow an escape that could call `auth::retry` on a flow that already
        // succeeded.
        for settled in [Phase::Idle, Phase::Profiles, Phase::Switching, Phase::Ready] {
            assert_eq!(
                bare_screen(settled, 1_000_000.0).control_kind(),
                None,
                "{settled:?}"
            );
        }
    }

    fn cx_with<'a>(
        m: &'a crate::ui::fixture::FixtureMeasure,
        snapshot: &'a auth::owner::SessionSnapshot,
    ) -> Cx<'a, SessionHost> {
        Cx {
            views: snapshot.read(),
            tick: crate::ui::machine::Tick::default(),
            measure: m,
            press: PressRead::default(),
            focus: FocusRead {
                current: None,
                ..Default::default()
            },
            owner: InputOwner::Entry(EntryId(0)),
        }
    }

    fn test_cx<'a>(m: &'a crate::ui::fixture::FixtureMeasure) -> Cx<'a, SessionHost> {
        cx_with(m, &EMPTY_SNAPSHOT)
    }

    /// The focus GROUP exists exactly when [`LoginScreen::has_control`] does, and its one element
    /// is `ElemKind::Bare` — the escape acts on the key-down edge with no hold and no press dip,
    /// like every read-out action in this family, never a `Control`/`Card`.
    #[test]
    fn the_control_group_exists_only_while_a_control_is_offered() {
        let m = crate::ui::fixture::FixtureMeasure;
        let cx = test_cx(&m);

        let quiet = bare_screen(Phase::Waiting, 0.0);
        let mut groups = Vec::new();
        Focusable::<SessionHost>::groups(&quiet, &cx, &mut groups);
        assert!(
            groups.is_empty(),
            "nothing to press while the code is fresh"
        );
        assert!(Focusable::<SessionHost>::place(&quiet, &CONTROL, &cx, At::Drawn).is_none());
        assert!(Focusable::<SessionHost>::group_of(&quiet, &CONTROL, &cx).is_none());

        let stuck = bare_screen(Phase::Waiting, QR_ESCAPE_AFTER_MS);
        groups.clear();
        Focusable::<SessionHost>::groups(&stuck, &cx, &mut groups);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].elem, ElemKind::Bare);
        assert_eq!(groups[0].id, CONTROL_GROUP);
        assert!(Focusable::<SessionHost>::place(&stuck, &CONTROL, &cx, At::Drawn).is_some());
        assert_eq!(
            Focusable::<SessionHost>::group_of(&stuck, &CONTROL, &cx),
            Some(CONTROL_GROUP)
        );
    }

    /// A stalled Working read-out draws its action pill LOWER than a settled one carrying the same
    /// reason — `StatusOverlay::bands`'s `Working` branch offsets the caption down from the frame
    /// centre instead of straddling it, and this is the one number that formula depends on that a
    /// host test can actually see (the caption/reason heights come from [`FixtureMeasure`], not a
    /// loaded font, so this pins the ORDERING the two `cap_y` branches produce, not literal
    /// on-device pixels).
    ///
    /// **Trimmed to the one assertion that is actually about this ORDERING.** This test used to
    /// also carry `assert_eq!(working.h, StatusOverlay::CTRL_H)` and a centering check — both
    /// restate a literal `status_action_rect` writes into its own return unconditionally
    /// (`Rect::new(frame.cx() - w * 0.5, …, w, StatusOverlay::CTRL_H)` centres on `frame.cx()` and
    /// carries `CTRL_H` for ANY `w`, by construction, whatever the surrounding logic does), so
    /// they could not fail no matter how broken the geometry got. The property they were reaching
    /// for — that this hand-reproduced Rect actually agrees with the widget it reproduces — is the
    /// real question, and it needed a real widget on the other side of the assertion:
    /// [`status_action_rect_matches_the_widget_it_is_reproducing`] below is that test.
    #[test]
    fn a_stalled_working_readout_sits_its_action_pill_lower_than_a_settled_one() {
        let m = crate::ui::fixture::FixtureMeasure;
        let mut b = [0u8; crate::i18n::TC_MAX];
        let working = status_action_rect(&m, crate::i18n::tcstr(crate::i18n::t("Try again"), &mut b), StatusKind::Working, true);
        let mut settled_buf = [0u8; crate::i18n::TC_MAX];
        let settled = status_action_rect(&m, crate::i18n::tcstr(crate::i18n::t("Try again"), &mut settled_buf), StatusKind::Empty, true);
        assert!(
            working.y > settled.y,
            "Working straddles the centre with the spinner above it; a settled read-out centres \
             the caption alone, which sits higher"
        );
    }

    /// A reason line pushes the action pill further down — `bands`'s `below` accumulator, ported
    /// onto `Measure` — for a centred read-out and for the page-filling `Failed` one alike: the
    /// row always stacks under the copy.
    #[test]
    fn a_reason_line_pushes_the_action_pill_down_further() {
        let m = crate::ui::fixture::FixtureMeasure;
        for kind in [StatusKind::Working, StatusKind::Empty, StatusKind::Failed] {
            let mut with_reason_buf = [0u8; crate::i18n::TC_MAX];
            let with_reason = status_action_rect(&m, crate::i18n::tcstr(crate::i18n::t("Try again"), &mut with_reason_buf), kind, true);
            let mut without_buf = [0u8; crate::i18n::TC_MAX];
            let without = status_action_rect(&m, crate::i18n::tcstr(crate::i18n::t("Try again"), &mut without_buf), kind, false);
            assert!(with_reason.y > without.y, "{kind:?}");
        }
    }

    /// **The property `status_action_rect`'s own doc claims, actually checked.** Neither test above
    /// it compares against a real [`StatusOverlay`] at all — both only compare two calls to
    /// `status_action_rect` against EACH OTHER, which can pin an ordering but can never catch the
    /// two formulas drifting apart from each other, term for term, in lockstep. This test builds a
    /// real `StatusOverlay` with the same shape (`kind`, `reason`, `action`) `LoginScreen::draw`
    /// ever configures one with and asserts its `action_frame()` Rect is bit-for-bit the Rect
    /// `status_action_rect` computes for the equivalent inputs, across both axes
    /// (`working`/`has_reason`) this screen ever combines. `RawTextMeasure` is what makes an exact
    /// comparison possible on the host at all — see its own doc for why `FixtureMeasure` cannot do
    /// this job.
    /// A [`Measure`] that forwards straight to `crate::text`'s own raw, no-font-loaded fallbacks,
    /// the exact numbers `StatusOverlay::bands`/`action_frame` themselves fall back to.
    struct RawTextMeasure;
    impl Measure for RawTextMeasure {
        fn width(&self, s: &CStr, sz: i32, bold: bool) -> f32 {
            crate::text::text_width(s.as_ptr(), sz, bold as c_int)
        }
        fn cap_h(&self, sz: i32) -> f32 {
            crate::text::cap_h(sz, 0)
        }
        fn line_h(&self, sz: i32) -> f32 {
            crate::text::text_height(sz, 0)
        }
    }

    #[test]
    fn status_action_rect_matches_the_widget_it_is_reproducing() {
        for kind in [StatusKind::Working, StatusKind::Failed, StatusKind::Empty] {
            for has_reason in [false, true] {
                let mut action_buf = [0u8; crate::i18n::TC_MAX];
                let mut o = StatusOverlay::new(Rect::FULL, c"caption", kind).page().action(crate::i18n::tcstr(crate::i18n::t("Try again"), &mut action_buf));
                if has_reason {
                    o = o.reason(c"This is taking longer than usual.");
                }
                let want = o.action_frame().expect("an action was set above");
                let mut got_buf = [0u8; crate::i18n::TC_MAX];
                let got = status_action_rect(&RawTextMeasure, crate::i18n::tcstr(crate::i18n::t("Try again"), &mut got_buf), kind, has_reason);
                assert_eq!(
                    (got.x, got.y, got.w, got.h),
                    (want.x, want.y, want.w, want.h),
                    "kind={kind:?} has_reason={has_reason}"
                );
            }
        }
    }

    /// **The frozen-animator regression class, closed for this screen's clock (phase 12 D4).**
    /// `spin_ms`/`phase_ms` used to be raw `+= dt` accumulators with a SEPARATE, easy-to-forget
    /// `fx.note(Motion)` a few lines below them — exactly the shape `Xfade`/`Spinner` shipped
    /// frozen in before (`docs/agent-reference.md`'s idle section). Now both are `motion::Phase`,
    /// which reports from inside its own `advance`, so this proves the report survives the
    /// refactor: three real `Tick`s in a row, driven through `Machine::step` exactly as the loop
    /// drives one, must each present. (The complementary "a settled read-out's clock does not
    /// report" half is `control_has_spinner`'s own predicate, unchanged by this conversion, and is
    /// not re-driven here through `resync`; the pure predicate already pins that settled half.)
    #[test]
    fn the_spinner_phase_reports_motion_on_every_tick_while_a_control_has_one() {
        let mut s = LoginScreen::new(EntryId(0), EMPTY_SNAPSHOT.read());
        let m = crate::ui::fixture::FixtureMeasure;
        let cx = test_cx(&m);
        let mut present = Present::new();
        let _ = present.take(0);
        let mut buf: Vec<Stamped<SessionHost>> = Vec::new();
        for ms in [16, 32, 48] {
            let mut fx = Effects::new(&mut buf, MachineId::Instance(InstanceId(0)), &mut present);
            let ev = ScreenEvent::Tick(Tick { ms, dt_us: 16_667 });
            Machine::<SessionHost>::step(&mut s, &ev, &cx, &mut fx);
            assert!(
                present.take(ms),
                "a live sign-in spinner must present every frame it is on screen (ms={ms})"
            );
        }
    }

    fn step_ev(
        s: &mut LoginScreen,
        ev: &ScreenEvent<SessionHost>,
    ) -> (Handled, Vec<Stamped<SessionHost>>) {
        let m = crate::ui::fixture::FixtureMeasure;
        step_ev_with(s, ev, &EMPTY_SNAPSHOT, InstanceId(0), &m)
    }

    fn step_ev_with(
        s: &mut LoginScreen,
        ev: &ScreenEvent<SessionHost>,
        snapshot: &auth::owner::SessionSnapshot,
        instance: InstanceId,
        m: &crate::ui::fixture::FixtureMeasure,
    ) -> (Handled, Vec<Stamped<SessionHost>>) {
        let _frame_scope = crate::task::FrameScope::enter();
        let cx = cx_with(m, snapshot);
        let mut present = Present::new();
        let mut buf: Vec<Stamped<SessionHost>> = Vec::new();
        let handled = {
            let mut fx = Effects::new(&mut buf, MachineId::Instance(instance), &mut present);
            Machine::<SessionHost>::step(s, ev, &cx, &mut fx)
        };
        (handled, buf)
    }

    fn key_back_down() -> ScreenEvent<SessionHost> {
        ScreenEvent::Input(InputEvent {
            at: crate::ui::machine::Tick::default(),
            source: Source::Script,
            kind: InputKind::Key {
                key: Key::Back,
                // the remote's own BACK code, so the alert trap (which classifies the raw press
                // to tell BACK from Stop and Exit) reads it too
                sym: 0,
                wcode: crate::ui::consts::WCODE_BACK,
                edge: Edge::Down,
                at_edge: false,
            },
        })
    }

    #[test]
    fn constructor_and_ticks_retain_coherent_host_publications_per_instance() {
        let mut first_snapshot = snapshot(Phase::Waiting, 41, "AAAA");
        first_snapshot.png = Arc::from(vec![1, 2, 3]);
        let mut second_snapshot = snapshot(Phase::Error, 99, "BBBB");
        second_snapshot.error = Arc::from("second failed");

        let first = LoginScreen::new(EntryId(1), first_snapshot.read());
        let mut second = LoginScreen::new(EntryId(2), second_snapshot.read());
        assert_eq!(
            (first.phase, first.qr_gen, first.qr_code.as_ref()),
            (Phase::Waiting, 41, "AAAA")
        );
        assert_eq!(
            first.qr_png_pending.as_ref().map(|(_, png)| png.as_ref()),
            Some(&[1, 2, 3][..])
        );
        assert_eq!(
            (second.phase, second.error.as_ref()),
            (Phase::Error, "second failed")
        );

        let replacement = snapshot(Phase::Waiting, 100, "CCCC");
        let m = crate::ui::fixture::FixtureMeasure;
        step_ev_with(
            &mut second,
            &ScreenEvent::Tick(Tick {
                ms: 16,
                dt_us: 16_667,
            }),
            &replacement,
            InstanceId(2),
            &m,
        );

        assert_eq!(
            (second.phase, second.qr_gen, second.qr_code.as_ref()),
            (Phase::Waiting, 100, "CCCC")
        );
        assert_eq!(
            (first.phase, first.qr_gen, first.qr_code.as_ref()),
            (Phase::Waiting, 41, "AAAA")
        );
    }

    #[test]
    fn controls_emit_typed_session_commands_without_mutating_the_wait_clock() {
        let mut retry = bare_screen(Phase::Error, 0.0);
        let (_, retry_fx) = step_ev(&mut retry, &ScreenEvent::Activate(CONTROL));
        assert!(retry_fx
            .iter()
            .any(|st| matches!(st.fx, Fx::App(AppFx::Session(auth::SessionCmd::Retry)))));

        let mut start = bare_screen(Phase::Deleted, 0.0);
        let (_, start_fx) = step_ev(&mut start, &ScreenEvent::Activate(CONTROL));
        assert!(start_fx
            .iter()
            .any(|st| matches!(st.fx, Fx::App(AppFx::Session(auth::SessionCmd::StartLogin)))));

        let mut restart = bare_screen(Phase::Waiting, QR_ESCAPE_AFTER_MS);
        restart.wait = (Phase::Waiting, 7);
        restart.qr_gen = 7;
        let m = crate::ui::fixture::FixtureMeasure;
        let published = snapshot(Phase::Waiting, 7, "AAAA");
        let (_, restart_fx) = step_ev_with(
            &mut restart,
            &ScreenEvent::Activate(CONTROL),
            &published,
            InstanceId(44),
            &m,
        );
        assert_eq!(
            restart.phase_ms, QR_ESCAPE_AFTER_MS,
            "emission is not acceptance"
        );
        assert!(restart_fx.iter().any(|st| matches!(
            &st.fx,
            Fx::App(AppFx::Session(auth::SessionCmd::RestartWait {
                phase: Phase::Waiting,
                qr_generation: 7,
                reply,
            })) if reply.instance == 44 && reply.correlation == 1
        )));
    }

    /// Regression for `warning-routing-and-continue-untested`. Pins the UI half of the AUTH-03
    /// gate: while `persistence_warning` is showing, the one offered control is `ContinueUnsaved`,
    /// and activating it emits EXACTLY one `AcknowledgePersistenceWarning` carrying the shown
    /// warning's OWN key — never `Retry`/`StartLogin`/nothing. MUTATION for this finding: make
    /// `LoginScreen::activate`'s `Some(ControlKind::ContinueUnsaved)` arm push nothing (or push the
    /// wrong key) — this test must then fail, because acknowledging is the only door that can ever
    /// release a held Final handoff (`auth/owner.rs::acknowledge_persistence_warning`), so a
    /// no-op here strands the user in front of the warning forever.
    #[test]
    fn the_warning_screens_one_control_acknowledges_exactly_that_warning() {
        let key = auth::owner::PersistenceWarningKey { epoch: 3, req: 9 };
        let warning = auth::owner::PersistenceWarning {
            key,
            site: auth::owner::PersistenceWarningSite::Final,
            helper: None, candidate_errnos: [None; 8], persistence: None,
        };
        let mut screen = bare_screen(Phase::Ready, 0.0);
        screen.persistence_warning = Some(warning);
        assert_eq!(
            screen.control_kind(),
            Some(ControlKind::ContinueUnsaved),
            "a live warning is the only control offered, regardless of the underlying phase"
        );

        let (_, effects) = step_ev(&mut screen, &ScreenEvent::Activate(CONTROL));
        let acks: Vec<_> = effects
            .iter()
            .filter(|st| {
                matches!(
                    st.fx,
                    Fx::App(AppFx::Session(auth::SessionCmd::AcknowledgePersistenceWarning { .. }))
                )
            })
            .collect();
        assert_eq!(acks.len(), 1, "exactly one acknowledgement, never zero and never a second");
        assert!(
            matches!(
                acks[0].fx,
                Fx::App(AppFx::Session(auth::SessionCmd::AcknowledgePersistenceWarning { key: acked }))
                    if acked == key
            ),
            "the acknowledgement must carry the SHOWN warning's own key, not a stale or default one"
        );
        assert!(
            !effects.iter().any(|st| matches!(
                st.fx,
                Fx::App(AppFx::Session(auth::SessionCmd::Retry | auth::SessionCmd::StartLogin))
            )),
            "activating the warning control must never also fire an unrelated session command"
        );
    }

    #[test]
    fn only_the_matching_accepted_restart_reply_resets_the_stalled_wait() {
        let mut screen = bare_screen(Phase::Waiting, QR_ESCAPE_AFTER_MS);
        screen.wait = (Phase::Waiting, 7);
        screen.qr_gen = 7;
        let published = snapshot(Phase::Waiting, 7, "AAAA");
        let m = crate::ui::fixture::FixtureMeasure;
        step_ev_with(
            &mut screen,
            &ScreenEvent::Activate(CONTROL),
            &published,
            InstanceId(44),
            &m,
        );
        assert_eq!(screen.pending_restart.map(|p| p.correlation), Some(1));

        step_ev_with(
            &mut screen,
            &ScreenEvent::Async(
                crate::ui::machine::RequestId(1),
                AppMsg::RestartReply {
                    correlation: 1,
                    accepted: false,
                },
            ),
            &published,
            InstanceId(44),
            &m,
        );
        assert_eq!(screen.phase_ms, QR_ESCAPE_AFTER_MS);
        assert!(
            screen.pending_restart.is_none(),
            "a matching refusal is terminal"
        );

        step_ev_with(
            &mut screen,
            &ScreenEvent::Activate(CONTROL),
            &published,
            InstanceId(44),
            &m,
        );
        assert_eq!(screen.pending_restart.map(|p| p.correlation), Some(2));
        step_ev_with(
            &mut screen,
            &ScreenEvent::Async(
                crate::ui::machine::RequestId(99),
                AppMsg::RestartReply {
                    correlation: 2,
                    accepted: true,
                },
            ),
            &published,
            InstanceId(44),
            &m,
        );
        assert_eq!(
            screen.phase_ms, QR_ESCAPE_AFTER_MS,
            "a foreign request id is not acceptance"
        );
        assert_eq!(screen.pending_restart.map(|p| p.correlation), Some(2));

        step_ev_with(
            &mut screen,
            &ScreenEvent::Async(
                crate::ui::machine::RequestId(2),
                AppMsg::RestartReply {
                    correlation: 2,
                    accepted: true,
                },
            ),
            &published,
            InstanceId(44),
            &m,
        );
        assert_eq!(screen.phase_ms, 0.0);
        assert!(screen.pending_restart.is_none());

        screen.phase_ms = 321.0;
        step_ev_with(
            &mut screen,
            &ScreenEvent::Async(
                crate::ui::machine::RequestId(2),
                AppMsg::RestartReply {
                    correlation: 2,
                    accepted: true,
                },
            ),
            &published,
            InstanceId(44),
            &m,
        );
        assert_eq!(
            screen.phase_ms, 321.0,
            "a duplicate accepted reply is stale"
        );
    }

    #[test]
    fn phase_progress_retires_a_carried_restart_reply_and_exhaustion_emits_nothing() {
        let waiting = snapshot(Phase::Waiting, 7, "AAAA");
        let progressed = snapshot(Phase::Discovering, 7, "");
        let m = crate::ui::fixture::FixtureMeasure;
        let mut screen = bare_screen(Phase::Waiting, QR_ESCAPE_AFTER_MS);
        screen.wait = (Phase::Waiting, 7);
        screen.qr_gen = 7;
        step_ev_with(
            &mut screen,
            &ScreenEvent::Activate(CONTROL),
            &waiting,
            InstanceId(44),
            &m,
        );
        step_ev_with(
            &mut screen,
            &ScreenEvent::Tick(Tick {
                ms: 16,
                dt_us: 16_667,
            }),
            &progressed,
            InstanceId(44),
            &m,
        );
        assert!(screen.pending_restart.is_none());
        screen.phase_ms = 222.0;
        step_ev_with(
            &mut screen,
            &ScreenEvent::Async(
                crate::ui::machine::RequestId(1),
                AppMsg::RestartReply {
                    correlation: 1,
                    accepted: true,
                },
            ),
            &progressed,
            InstanceId(44),
            &m,
        );
        assert_eq!(
            screen.phase_ms, 222.0,
            "the old wait's carried reply is stale"
        );

        let mut exhausted = bare_screen(Phase::Waiting, QR_ESCAPE_AFTER_MS);
        exhausted.wait = (Phase::Waiting, 7);
        exhausted.next_correlation = Some(u32::MAX);
        let (_, effects) = step_ev_with(
            &mut exhausted,
            &ScreenEvent::Activate(CONTROL),
            &waiting,
            InstanceId(44),
            &m,
        );
        assert!(effects
            .iter()
            .all(|st| !matches!(st.fx, Fx::App(AppFx::Session(_)))));
        assert!(exhausted.pending_restart.is_none());
        assert_eq!(exhausted.phase_ms, QR_ESCAPE_AFTER_MS);
    }

    /// **This screen has no panel of its own**, so every BACK is the typed Session root request.
    #[test]
    fn back_is_always_the_root_press() {
        let mut s = bare_screen(Phase::Waiting, 0.0);
        let (handled, effs) = step_ev(&mut s, &key_back_down());
        assert_eq!(handled, Handled::Yes);
        assert!(
            effs.iter().any(|st| matches!(
                &st.fx,
                Fx::App(AppFx::Session(auth::SessionCmd::BackAtRoot { reply }))
                    if reply.instance == 0 && reply.correlation == 1
            )),
            "login has no panel of its own to close first — BACK asks Session for the root press"
        );
    }

    /// The old route classifier had a dedicated `Switching` arm. The owned screen's decision is
    /// phase-independent, so pin that replacement with a real Switching publication and the
    /// addressed instance/correlation Session receives.
    #[test]
    fn back_during_switching_is_still_the_owned_logins_root_press() {
        let switching = snapshot(Phase::Switching, 17, "");
        let mut s = LoginScreen::new(EntryId(9), switching.read());
        let m = crate::ui::fixture::FixtureMeasure;
        let (handled, effects) = step_ev_with(&mut s, &key_back_down(), &switching,
            InstanceId(44), &m);
        assert_eq!(handled, Handled::Yes);
        assert!(effects.iter().any(|stamped| matches!(
            &stamped.fx,
            Fx::App(AppFx::Session(auth::SessionCmd::BackAtRoot { reply }))
                if reply.instance == 44 && reply.correlation == 1
        )), "Switching must not turn Login BACK into a local dismissal or an ignored key");
    }

    /// **The QR texture must not survive its screen — the leak `ScreenEvent::Unmount`'s own arm
    /// doc explains.** `999` stands in for a real GL id: a host test never uploads one for real
    /// (`prepare_qr_tex`'s own doc — GL work happens in `Screen::prepare`, never in `step`), so
    /// this pins the ONE property that matters without touching GL — that `step` zeroes `qr_tex`
    /// on `Unmount` regardless of what it held. Delete that arm, or let it fall back through to
    /// the catch-all `_ => Handled::No` the way it did before this fix, and this goes red:
    /// `qr_tex` stays `999` and `handled` reads `Handled::No`.
    #[test]
    fn unmount_frees_the_qr_texture() {
        let mut s = bare_screen(Phase::Waiting, 0.0);
        s.qr_tex = 999;
        s.qr_px = (400, 400);
        let (handled, _) = step_ev(&mut s, &ScreenEvent::Unmount);
        assert_eq!(handled, Handled::Yes);
        assert_eq!(
            s.qr_tex, 0,
            "an unmounting screen must not leak its GL texture id"
        );
        assert_eq!(
            s.qr_px,
            (0, 0),
            "…nor go on claiming its bytes in the frame's render set"
        );
    }
    fn tick_ev(ms: u32) -> ScreenEvent<SessionHost> {
        ScreenEvent::Tick(Tick { ms, dt_us: 16_667 })
    }


    /// Build a cx whose engine focus is on `elem`, for a `PressCommit` the trap reads it from.

    /// BACK with no alert up asks Session for the root press.
    #[test]
    fn with_the_card_closed_back_is_the_root_press() {
        let failed = snapshot(Phase::Error, 0, "");
        let mut s = LoginScreen::new(EntryId(0), failed.read());
        let m = crate::ui::fixture::FixtureMeasure;
        step_ev_with(&mut s, &tick_ev(16), &failed, InstanceId(0), &m);
        let (_, fx) = step_ev_with(&mut s, &key_back_down(), &failed, InstanceId(0), &m);
        assert_eq!(fx.iter().filter(|st| matches!(&st.fx,
            Fx::App(AppFx::Session(auth::SessionCmd::BackAtRoot { .. })))).count(), 1,
            "with the card closed, BACK is the root press");
    }
}
