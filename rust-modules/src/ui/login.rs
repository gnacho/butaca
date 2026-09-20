//! The sign-in screen: Plex's own server-rendered QR PNG (fetched by the auth flow, decoded +
//! tinted here) plus the typed short-code fallback, driven by the [`crate::auth`] flow phase.
//! Scanning the QR on a phone opens plex.tv pre-filled with the pin; the flow's background poll
//! then advances us onward.
#![allow(non_upper_case_globals)]
use crate::auth::{self, Phase};
use crate::ui::consts::*;
use crate::ui::label::{HAlign, Label};
use crate::ui::route_screen::{ActionRow, RouteGround, RouteLayout};
use crate::ui::text_view::TextView;
use crate::ui::widgets::{Button, Spinner, StatusKind, StatusOverlay};
use crate::ui::{theme, Env, Painter, Rect, View};
use std::ffi::CString;
use std::os::raw::{c_int, c_uint};
use std::ptr::addr_of_mut;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Scene {
    spin_ms: f32,
    qr_tex: u32, // GL texture of Plex's QR PNG (0 until decoded+uploaded)
    /// Which code `qr_tex` holds ([`auth::qr_generation`]). The cache key, and the reason a code
    /// replaced mid-`Waiting` cannot be drawn after it has died.
    qr_gen: u64,
    ground: RouteGround,
    /// How long the CURRENT wait has been on screen. Distinct from `spin_ms`, which is a
    /// free-running rotation clock: this one is reset by [`update`] whenever the wait changes,
    /// because the only question it answers is "has this particular wait gone on too long".
    phase_ms: f32,
    /// What that wait IS — see [`wait_id`]. A phase alone was not enough once a code could be
    /// replaced without leaving [`Phase::Waiting`].
    wait: (Phase, u64),
    /// How many files the last **Delete all local data** could not unlink. Drives the wording of
    /// the `Deleted` read-out, which must not claim a wipe it did not achieve.
    delete_leftovers: usize,
    /// The link-health sentence ([`auth::link_detail`]) this screen last sampled, or `None` while
    /// plex.tv was answering. Cached so [`update`] can tell whether the underlying state actually
    /// changed — the link snapshot is sampled every frame, but the sentence itself only moves
    /// changes roughly once a poll (~2 s), without waking every frame for an equal value.
    last_link_detail: Option<String>,
    persistence_warning_seen: Option<auth::PersistenceWarningKey>,
}

/// How long a working phase runs before the read-out grows a way out.
///
/// **Not zero**: a healthy LAN discovery finishes in well under a second, and a control that
/// flashes past on every sign-in is noise that teaches people to ignore it. **Not longer**: from
/// the sofa a spinner that will never stop looks exactly like one that is about to, and until this
/// existed there was no way at all out of a wedged sign-in — BACK is swallowed on a first-ever
/// boot (`auth::cancel` has no stored session to resume), so the only exit was killing the app.
const ESCAPE_AFTER_MS: f32 = 12_000.0;

/// Whether a wait has run long enough to be worth offering an escape from.
///
/// Pure and separate from the draw so the threshold is gradeable on the host; the state it reads
/// lives in the SDL loop's own scene.
fn escape_offered(phase_ms: f32) -> bool {
    phase_ms >= ESCAPE_AFTER_MS
}

/// **The sealed-sign-in-could-not-be-read read-out (issue #76 review).**
///
/// It names the SUBJECT (the sign-in saved on this television), the CAUSE in the words a person
/// can act on (the television's own key service did not answer — not "keymanager3", not an error
/// code; the footer already carries the build and the set for a photograph), and the fact that
/// matters most and is least guessable: **nothing has been changed**. The stored sign-in is still
/// there, byte for byte, and the next launch — or the button under this copy — may well open it.
///
/// The two ways out are stated in the order they should be tried, and the second one is the QR
/// stack already drawn beside this column, which is why the copy points at it rather than the
/// screen growing a second control for it. "Only signing in again replaces it" is the promise the
/// storage layer actually keeps (`plex::session::seal_permitted`'s own doc), and saying it here is
/// what stops *Try again* reading like a gamble with the stored credential.
const STORAGE_TITLE: &str = "Your saved sign-in couldn\u{2019}t be read";
const STORAGE_COPY: &str = "This television\u{2019}s key service didn\u{2019}t answer, so the \
sign-in saved here couldn\u{2019}t be unlocked. Nothing has been changed \u{2014} try again, or \
scan the code to sign in again. Only signing in again replaces what is stored here.";

/// The label on the storage read-out's one control. Deliberately the same words as [`ESCAPE`] —
/// it is the same offer (ask the thing that did not answer, again) about a different subject, and
/// two different verbs for one gesture is how a screen teaches somebody that it is unpredictable.
const STORAGE_RETRY: &std::ffi::CStr = c"Try again";

/// What the *Try again* press is doing right now — see [`start_storage_retry`].
///
/// An atomic rather than a [`Scene`] field because the ask itself runs on a WORKER: reopening the
/// envelope is one or two LS2 calls against a service that has already proved it can go quiet, and
/// `keymanager::platform::BUDGET` is measured in seconds. Doing that inline would freeze the SDL
/// main loop — the spinner, the QR, every key — for the whole budget, on the one screen whose
/// subject is a television that seems stuck.
static STORAGE_RETRY_STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(RETRY_IDLE);
const RETRY_IDLE: u8 = 0;
/// A worker is asking the key service right now.
const RETRY_BUSY: u8 = 1;
/// The worker opened the envelope. The MAIN thread picks this up in [`update`] — resuming the
/// session installs a server registry and a profile, which is main-thread work.
const RETRY_OPENED: u8 = 2;
/// The worker finished and the envelope is still not open.
const RETRY_FAILED: u8 = 3;

/// **What [`update`] last saw [`STORAGE_RETRY_STATE`] at** — the main thread's own edge detector,
/// so the Details panel can be re-read the moment a *Try again* press settles.
///
/// The press changes exactly the facts that panel shows (the storage class, the key service's last
/// refusal, and — on a set that grants the app id on the second registration — the identity), and
/// it changes them from a WORKER, which may not touch `Scene`. So the worker publishes a state and
/// this notices the edge one frame later on the thread that owns the screen. Only settled states
/// trigger the re-read: the transition INTO `RETRY_BUSY` has learned nothing yet.
static STORAGE_RETRY_SEEN: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(RETRY_IDLE);

/// **Which sign-in attempt [`start_storage_retry`] was pressed for — [`settle_storage_retry`]'s
/// stale-result guard.** `0` is never a real attempt (`auth`'s counter starts at 1), so it also
/// means "nothing captured yet".
///
/// The press and the worker it starts are asking about the flow's CURRENT attempt at that
/// instant, not about whatever the flow happens to be doing several hundred milliseconds later
/// when the worker returns — the whole gap this screen's own async note (`storage_note`) exists
/// to narrate. If, in that gap, the SAME running attempt reaches its own `Phase::Ready` (a QR scan
/// completed on the phone while the television was still asking its key service about a
/// completely different, older account), the attempt id alone cannot see it — a normal completion
/// never bumps `Ctl::attempt`, only a fresh reset does. [`settle_storage_retry`] therefore checks
/// the PHASE this was captured for as well: it only ever fires while `auth::phase()` is still
/// [`Phase::Waiting`], the one phase the storage read-out draws over, so a flow that has already
/// moved on — Ready, Profiles, Error, a fresh attempt, anything — is left alone and the newer
/// result wins.
static STORAGE_RETRY_ATTEMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// ---- Issue #76's report lane: the "Details" panel over the storage read-out ----------------------
//
// Owner correction, 2026-09-11: no diagnostic words in the read-out line or the footer — instead a
// second pill, "Details", beside "Try again", opening an in-screen panel on the app's existing menu
// idiom (`ui::CLAUDE.md`: Popover + TableView, tokens and components only). One row per fact, label
// left / value right, photographable. BACK closes it.

/// One candidate session-file location's read outcome, in the read lane's own closed vocabulary —
static STORAGE_FOCUS: AtomicUsize = AtomicUsize::new(0);
/// `0` = the "Try again" pill — the only control this row carries.
const STORAGE_FOCUS_RETRY: usize = 0;

const STORAGE_NEW_CODE: &std::ffi::CStr = c"New code";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoragePrimaryAction {
    Continue(auth::PersistenceWarningKey),
    RetrySecureOpen,
    NewCode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StorageAction {
    Continue(auth::PersistenceWarningKey),
    RetrySecureOpen,
    NewCode,
}

static ARMED_STORAGE_ACTION: std::sync::Mutex<Option<(StorageAction, u64)>> =
    std::sync::Mutex::new(None);

fn storage_primary_for(
    waiting: bool,
    secure_unavailable: bool,
    new_code_ready: bool,
) -> Option<StoragePrimaryAction> {
    if !waiting {
        None
    } else if secure_unavailable {
        Some(StoragePrimaryAction::RetrySecureOpen)
    } else if new_code_ready {
        Some(StoragePrimaryAction::NewCode)
    } else {
        None
    }
}

fn storage_primary_action() -> Option<StoragePrimaryAction> {
    if let Some(warning) = auth::pending_persistence_warning() {
        return Some(StoragePrimaryAction::Continue(warning.key));
    }
    storage_primary_for(
        storage_action_showing(),
        crate::plex::session::secure_unavailable(),
        qr_escape_offered(scene().phase_ms),
    )
}

pub(crate) fn arm_storage_action() -> bool {
    if !storage_action_showing() || modal_open() {
        return false;
    }
    let action = match storage_primary_action() {
        Some(StoragePrimaryAction::Continue(key)) => StorageAction::Continue(key),
        Some(StoragePrimaryAction::RetrySecureOpen) => StorageAction::RetrySecureOpen,
        Some(StoragePrimaryAction::NewCode) => StorageAction::NewCode,
        None => return false,
    };
    *ARMED_STORAGE_ACTION.lock().unwrap_or_else(|e| e.into_inner()) =
        Some((action, auth::current_attempt()));
    true
}

fn armed_storage_action_valid(
    action: StorageAction,
    armed_attempt: u64,
    live_attempt: u64,
    waiting: bool,
    secure_unavailable: bool,
    new_code_ready: bool,
    modal_visible: bool,
) -> bool {
    !modal_visible
        && waiting
        && armed_attempt == live_attempt
        && match action {
            StorageAction::Continue(_) => false, // validated against the exact live warning below
            StorageAction::RetrySecureOpen => secure_unavailable,
            StorageAction::NewCode => new_code_ready && !secure_unavailable,
        }
}

fn warning_action_valid(
    action: StorageAction,
    armed_attempt: u64,
    live: auth::PersistenceWarningKey,
    modal_visible: bool,
) -> bool {
    !modal_visible && armed_attempt == live.attempt && match action {
        StorageAction::Continue(key) => key == live,
        _ => false,
    }
}

/// PURE. The line under the control, or `None` when the read-out has nothing to add to it. The
/// three states are three different sentences because they answer three different questions:
/// whether anything is happening, whether it worked, and — the one that has to be said out loud —
/// that a second failure is not a dead end, because the code beside it still is a way in.
///
/// `&'static str`, not a `CStr`: [`draw_storage_action`] draws this through [`TextView`] now
/// (review finding — see [`storage_note_room`]), which wants a `str` to measure and wrap, and
/// nothing else here still needed the raw-pointer shape.
fn storage_note(state: u8) -> Option<&'static str> {
    match state {
        RETRY_BUSY => Some("Asking the television\u{2019}s key service\u{2026}"),
        RETRY_FAILED => Some("Still no answer. You can scan the code to sign in again."),
        _ => None,
    }
}

/// **Claim the one worker slot for this press.** `true` once, for the press that takes it; `false`
/// for a press that arrives while the slot is already spoken for.
///
/// **Claimed from either SETTLED state — [`RETRY_IDLE`] and [`RETRY_FAILED`]** (review finding,
/// 2026-09-11). It used to claim from `RETRY_IDLE` alone, which reads as the same no-stacking rule
/// and is not: a worker that got nowhere leaves the slot at `RETRY_FAILED` and nothing on this
/// route ever puts it back (`enter` does, but that is the NEXT visit), so from the second press
/// onward the only control on the screen did nothing at all, silently, for the rest of the visit —
/// on the one screen whose subject is a television that seems stuck, beside a note that invites
/// exactly that second press. Asking a service that did not answer whether it will answer NOW is
/// the whole point of the control.
///
/// The two states it still refuses are the two in-flight ones: [`RETRY_BUSY`] (a worker is asking)
/// and [`RETRY_OPENED`] (one succeeded and [`settle_storage_retry`] has not applied it yet). That
/// is the rule that was always right — leaning on OK must not spawn a worker per frame against a
/// service whose whole problem is that it answers slowly.
fn claim_storage_retry() -> bool {
    use std::sync::atomic::Ordering;
    [RETRY_IDLE, RETRY_FAILED].iter().any(|&from| {
        STORAGE_RETRY_STATE
            .compare_exchange(from, RETRY_BUSY, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    })
}

/// Ask the key service again, off the main thread — the *Try again* press.
///
/// Refuses to stack: a second press while one is IN FLIGHT is swallowed rather than queued (see
/// [`claim_storage_retry`], which is also why a press after a FAILED one is not). A refused spawn
/// (`task::spawn_small` returning `false` under EAGAIN — see `task.rs`) falls back to asking
/// inline: a frozen frame is worse than nothing, but a button that silently does nothing at all is
/// worse than both.

fn start_storage_retry() {
    use std::sync::atomic::Ordering;
    if !claim_storage_retry() {
        return;
    }
    // Captured BEFORE the worker starts, under the same compare-exchange that claims the press —
    // see [`STORAGE_RETRY_ATTEMPT`]'s own doc for why the settle needs it.
    STORAGE_RETRY_ATTEMPT.store(auth::current_attempt(), Ordering::Release);
    crate::ui::idle::invalidate();
    let ask = || {
        let opened = crate::plex::session::retry_secure_open();
        STORAGE_RETRY_STATE.store(
            if opened { RETRY_OPENED } else { RETRY_FAILED },
            Ordering::Release,
        );
        crate::ui::idle::invalidate();
    };
    if !crate::task::spawn_small("keyretry", ask) {
        ask();
    }
}

/// PURE. Should an opened storage envelope still be applied to the flow, or has it gone stale?
///
/// `captured` is [`STORAGE_RETRY_ATTEMPT`] — the attempt id the press was about; `now_phase` and
/// `now_attempt` are what the flow is AT SETTLE TIME. Split out from [`settle_storage_retry`] so
/// the race it guards against is a fact a host test can assert directly, with no worker, no `Ctl`
/// and no dependence on `auth`'s process-global state.
///
/// **Both checks are required, and neither alone is enough.** The attempt id alone misses a
/// same-attempt race: a QR sign-in that completes NORMALLY (the phone's own poll landing while the
/// television is still asking its key service about a different, older account) never bumps
/// `Ctl::attempt` — only a fresh reset does — so a worker that returns after that still reads a
/// matching id while `Ctl` already holds the newly-signed-in account. The phase alone misses the
/// other direction: a BACK-then-retry that lands back on `Waiting` (a fresh reset, e.g. via
/// `auth::retry`) is phase-equal to the press's own wait but is a different attempt entirely, and
/// applying the old envelope over it would resurrect an account the user had already moved past.
/// Requiring `Phase::Waiting` — the one phase the storage read-out draws over — is what makes
/// "still on screen" and "still this attempt" the same claim from two directions.
fn storage_retry_still_applies(captured: u64, now_phase: Phase, now_attempt: u64) -> bool {
    now_phase == Phase::Waiting && now_attempt == captured
}

/// **Main-thread half of [`start_storage_retry`]: an opened envelope becomes a running app.**
///
/// The worker can only publish the session; entering it registers servers, installs a PMS client
/// and picks a profile, all of which belong to the loop. `auth::resume_secure_session` puts the
/// flow exactly where the boot gate would have — the who's-watching picker, or `Phase::Ready` for
/// the main loop's own `take_ready` to install — so from the next frame this launch is
/// indistinguishable from one whose key service answered the first time.
///
/// **Review finding: a worker's success is stale the instant the flow it was asked about has
/// moved on, and this used to act on it regardless.** [`storage_retry_still_applies`] is the gate:
/// a flow that has moved on wins either way — the opened envelope is simply not applied, `Ctl` is
/// left exactly as the newer result made it, and this state resets to idle rather than
/// [`RETRY_FAILED`], because nothing here failed — the read-out is just about to leave.
fn settle_storage_retry() {
    use std::sync::atomic::Ordering;
    if STORAGE_RETRY_STATE.load(Ordering::Acquire) != RETRY_OPENED {
        return;
    }
    let captured_attempt = STORAGE_RETRY_ATTEMPT.load(Ordering::Acquire);
    if !storage_retry_still_applies(captured_attempt, auth::phase(), auth::current_attempt()) {
        // The flow this press was about is no longer the one on screen — a newer sign-in (this
        // same attempt reaching Ready, or a fresh one entirely) already won. Applying the opened
        // envelope now would silently replace it with the stale stored account.
        STORAGE_RETRY_STATE.store(RETRY_IDLE, Ordering::Release);
        crate::log(
            "login: storage retry opened the envelope after a newer sign-in already won — dropped",
        );
        return;
    }
    let resumed = crate::auth::resume_secure_session(crate::plex::session::load());
    STORAGE_RETRY_STATE.store(
        if resumed { RETRY_IDLE } else { RETRY_FAILED },
        Ordering::Release,
    );
    crate::ui::idle::invalidate();
}

/// PURE. Vertical room actually left for the retry note, above the pill.
///
/// **Review finding, 2026-09-10**: this note used to draw at a fixed one-line offset from the
/// pill, on the strength of a comment claiming it "borrows" the `space::XL` gap `draw_narrative`
/// already leaves between its own body copy and the action slot
/// (`RouteLayout::draw_narrative_with_note`'s `copy_bottom = action.y - space::XL`) — but nothing
/// in the fixed offset actually measured that gap, so once the pill was lifted clear of the footer
/// the note's fixed offset from the PILL routinely ran past `copy_bottom`
/// and into the body's own territory, overprinting it whenever the body filled its allowance. The
/// fix mirrors `draw_narrative_with_note`'s own rule for its body's `max_lines` (route_screen.rs):
/// derive the cap from what is actually LEFT, never assume a constant fits.
///
/// The room is bounded above by `copy_bottom` (the body's own reserved floor) and below by the
/// pill's own top, `space::SM` clear of it — the same gap [`draw_storage_action`] already used to
/// place a fixed-height note, now spent as a budget instead of an offset.
fn storage_note_room(action: Rect, r: Rect) -> f32 {
    let copy_bottom = action.y - theme::space::XL;
    (r.y - theme::space::SM - copy_bottom).max(0.0)
}

/// The storage read-out's control and its note, in the shared bottom action slot of the narrative
/// column — the same slot `ui::onboard` and `ui::consent` put their own commit control in, rather
/// than a position of this screen's own.
fn draw_storage_action(p: Painter, env: &Env, layout: RouteLayout) {
    let primary = storage_primary_action();
    // Clamp rather than trust the stored focus: one slot owns this row, and it is the pill.
    let focus = STORAGE_FOCUS.load(Ordering::Acquire).min(STORAGE_FOCUS_RETRY);
    let y = layout.action.y;
    unsafe { (*addr_of_mut!(STORAGE_ACTION_POP)).clear() };
    let x = layout.action.x;
    if let Some(primary) = primary {
        let label = match primary {
            StoragePrimaryAction::Continue(_) => c"Continue",
            StoragePrimaryAction::RetrySecureOpen => STORAGE_RETRY,
            StoragePrimaryAction::NewCode => STORAGE_NEW_CODE,
        };
        let retry_w = Button::pill_w(label.as_ptr(), theme::size::BODY, false);
        let r = Rect::new(x, y, retry_w.min(layout.action.w), layout.action.h);
        Button::new(label.as_ptr(), theme::size::BODY, r)
            .focused(focus == STORAGE_FOCUS_RETRY)
            .scale(unsafe { (*addr_of_mut!(STORAGE_ACTION_POP)).scale(STORAGE_FOCUS_RETRY) })
            .draw(env, p);
        unsafe { (*addr_of_mut!(STORAGE_ACTION_POP)).place(STORAGE_FOCUS_RETRY, r) };
        let _ = x;
    }
    let Some(note) = matches!(primary, Some(StoragePrimaryAction::RetrySecureOpen))
        .then(|| storage_note(STORAGE_RETRY_STATE.load(std::sync::atomic::Ordering::Acquire)))
        .flatten()
    else {
        return;
    };
    let line_h = crate::text::text_height(theme::size::CAPTION, 0);
    // ABOVE the pill, not under it. The budget is the room actually LEFT above the pill, never a fixed
    // one-line assumption — see [`storage_note_room`] for why that used to overprint the body.
    let room = storage_note_room(
        layout.action,
        Rect::new(layout.action.x, y, layout.action.w, layout.action.h),
    );
    let leading = theme::size::CAPTION as f32 + theme::space::XS;
    let max_lines = ((room / leading).floor() as usize).clamp(1, 2);
    let note_view = TextView::new(note, theme::size::CAPTION, theme::TEXT_SECONDARY)
        .leading(leading)
        .max_lines(max_lines);
    // Bottom-aligned to the pill and clamped to the room itself, so a `max_lines` rounded up by
    // the `.clamp(1, …)` floor above still cannot draw above `copy_bottom` — the two-line minimum
    // exists so a genuinely tiny room still shows SOMETHING rather than silence, at the cost of a
    // partial overlap the room could not avoid either way.
    let note_h = note_view.measure_h(layout.action.w).min(room.max(line_h));
    let bottom = y - theme::space::SM;
    note_view.draw(
        p,
        Rect::new(layout.action.x, bottom - note_h, layout.action.w, note_h),
    );
}

/// The verb on both the failed and the stuck read-out, because it is the same call underneath.
///
/// `auth::retry` bumps the auth epoch, so a worker still blocked in the wedged request has its
/// result discarded when it finally returns, and it re-runs only the leg that failed — discovery
/// when the pin already yielded an account credential, a whole fresh pin when it did not.
const ESCAPE: &std::ffi::CStr = c"Try again";

/// The storage read-out's one control's own press surface — the shared type every route-family
/// action row uses ([`ActionRow`]) rather than a bespoke `Button` draw with no focus pop at all.
///
/// **Review finding, 2026-09-10**: this pill was the first control this screen puts in
/// `RouteLayout::action`, and it drew as a flat `Button` at a constant scale — every sibling pill
/// on this route family (Settings' Retry/Done, first-run consent's two answers) already animates
/// through one of these, so this one alone read as dead on arrival: focus moved onto it with no
/// grow, the one motion `CtlPop::step`/`::scale` give every other control the instant they are
/// wired at all (`widgets.rs`'s own doc: "the pop is the tile's spring now"). Wiring `.step()` in
/// [`update`] and `.scale(0)` in [`draw_storage_action`] fixes exactly that.
///
/// **What this does NOT reach: the press DIP itself.** `CtlPop::scale`'s own doc says the dip is
/// folded in "once the caller has armed `press::begin_ctl` on its OK-down" — and arming is the
/// caller's job specifically because [`crate::ui::press`] is ONE GLOBAL sequencer app.rs drives:
/// arm on the key-DOWN that takes a control face, release on the matching key-UP (gated on the
/// SAME local `ok_armed` app.rs's own dispatcher sets), poll for the deferred commit. Every other
/// route-family pill arms it from app.rs's per-route keydown ladder (see `Route::Onboard`'s and
/// `Route::Profiles`' own arms beside this route's). This screen's OK dispatch is the one
/// fallback case that still calls straight into [`key`] with no matching arm — and this fix's
/// scope is `ui/login.rs`/`auth.rs` only, so wiring that ladder entry is left for whoever next
/// touches `app.rs`'s `key_onboarding`, rather than armed here with no release to pair it: a
/// `begin_ctl` this module called with nothing setting app.rs's `ok_armed` would leave
/// `press::State` latched at `Phase::Down` forever, breaking the dip for every OTHER control in
/// the app after the first press on this one — a worse bug than the missing dip it would "fix".
/// The focus pop above is real and device-verified (`docs/measurements/
/// keymanager-unavailable-retry-pill-sim-2026-09-10.md`); the ring-on-press is a follow-up.
static mut STORAGE_ACTION_POP: ActionRow<2> = ActionRow::new();

static mut SCENE: Option<Scene> = None;

fn scene() -> &'static mut Scene {
    unsafe {
        (*addr_of_mut!(SCENE))
            .as_mut()
            .expect("login::init not called")
    }
}

/// The delegation condition, as a runtime predicate rather than a bare cfg block: the four
/// entry points below early-return through it, and a `#[cfg]`d block ending in `return` would
/// leave the Plex bodies statically unreachable on this build — which `-D warnings` grades as
/// an error. A function call is not analyzed, so both bodies stay live to the compiler.
#[cfg(feature = "jellyfin")]
fn jellyfin_flavor() -> bool {
    true
}

pub fn init() {
    // The Jellyfin flavor's sign-in is a different screen wearing this route — its own module,
    // reached through every entry point below's delegation arm.
    #[cfg(feature = "jellyfin")]
    crate::ui::jf_login::init();
    unsafe {
        *addr_of_mut!(SCENE) = Some(Scene {
            spin_ms: 0.0,
            qr_tex: 0,
            qr_gen: auth::qr_generation(),
            ground: RouteGround::new(),
            phase_ms: 0.0,
            wait: wait_id(),
            delete_leftovers: 0,
            last_link_detail: None,
            persistence_warning_seen: None,
        });
    }
}

/// Mount the auth route without replacing the cached QR texture. A fresh auth flow invalidates the
/// texture from [`update`] when it reaches `Creating`; this only resets the visit's visual ground.
pub fn enter() {
    #[cfg(feature = "jellyfin")]
    if jellyfin_flavor() {
        crate::ui::jf_login::enter();
        return;
    }
    let s = scene();
    s.ground.reset();
    // A fresh visit is a fresh wait, whatever phase the last one died in.
    s.phase_ms = 0.0;
    s.wait = wait_id();
    // A "still no answer" note belongs to the press that earned it, not to the next visit to this
    // route — a sign-out and a re-entry must not open on somebody else's failed retry. Only the
    // settled failure is cleared: a retry still in flight is still in flight.
    let _ = STORAGE_RETRY_STATE.compare_exchange(
        RETRY_FAILED,
        RETRY_IDLE,
        std::sync::atomic::Ordering::AcqRel,
        std::sync::atomic::Ordering::Relaxed,
    );
    scene().persistence_warning_seen = None;
    STORAGE_FOCUS.store(STORAGE_FOCUS_RETRY, Ordering::Release);
    crate::ui::idle::invalidate();
}

/// Tear down route-owned modals before the app hands the route to Home, Profiles or Onboard.
/// `Popover::close` is required here: the route no longer draws the fade, so leaving it in the
/// global open counter would leak modal ownership for the rest of the process.
pub fn leave() {
    scene().persistence_warning_seen = None;
    STORAGE_FOCUS.store(STORAGE_FOCUS_RETRY, Ordering::Release);
    unsafe { (*addr_of_mut!(STORAGE_ACTION_POP)).clear() };
    *ARMED_STORAGE_ACTION.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn update(dt: f32) {
    #[cfg(feature = "jellyfin")]
    if jellyfin_flavor() {
        crate::ui::jf_login::update(dt);
        return;
    }
    let warning = auth::pending_persistence_warning().map(|w| w.key);
    if scene().persistence_warning_seen != warning {
        scene().persistence_warning_seen = warning;
        STORAGE_FOCUS.store(STORAGE_FOCUS_RETRY, Ordering::Release);
        crate::ui::idle::invalidate();
    }
    let s = scene();
    s.spin_ms += dt * 1000.0;
    settle_storage_retry();
    let focused_slot = storage_action_showing().then_some(STORAGE_FOCUS_RETRY);
    unsafe {
        (*addr_of_mut!(STORAGE_ACTION_POP)).step(focused_slot, dt);
    }
    // Each wait gets its own clock. A flow that walks Creating → Waiting → Discovering is making
    // progress, and restarting the timer at every step is what stops a slow-but-healthy sign-in
    // from being offered a way out of itself.
    let live = wait_id();
    if wait_restarted(s.wait, live) {
        s.wait = live;
        s.phase_ms = 0.0;
    } else {
        s.phase_ms += dt * 1000.0;
    }
    drop_a_stale_qr(s, auth::qr_generation());
    // `link_detail` is sampled from a mutex updated roughly once a poll (~2 s), not from a spring
    // `ui::idle` can already see — so this screen must report the discrete change itself, exactly
    // as `Xfade::tick`/`Spinner::draw` do for their own clocks. Comparing against the LAST DRAWN
    // sentence (not e.g. a bare `unanswered` counter) is what stops a healthy link — where every
    // poll answers and `link_detail` stays `None` forever — from invalidating every frame.
    let detail = auth::link_detail(&auth::link_state());
    let changed = link_detail_changed(&s.last_link_detail, &detail);
    if changed {
        s.last_link_detail = detail;
    }
    if changed {
        crate::ui::idle::invalidate();
    }
}

/// PURE. Has the sentence the sign-in screen draws under its status changed since the last frame?
/// Trivial today (`!=`), but factored out because it is the one thing standing between a settled
/// FAILED read-out and a per-frame `ui::idle::invalidate()` — a later refinement (e.g. rounding
/// `failing_for` to the second so the trailing "…, 41 s" does not itself force a wake every
/// second) belongs here, not inlined into [`update`].
/// Is a modal on this route open? There is none any more — the one-off report alert and the
/// storage Details panel left with the telemetry system — but `app.rs`'s Login arms still ask, so
/// the predicate stays as the single answer.
pub fn modal_open() -> bool {
    false
}

fn link_detail_changed(prev: &Option<String>, next: &Option<String>) -> bool {
    prev != next
}

/// Release the cached QR texture as soon as it stops describing the code the flow is showing.
///
/// **Keyed on [`auth::qr_generation`], not on the phase, and that swap is this screen's half of
/// issue #30.** The rule used to be "a retry enters `Creating`, so drop it there" — which was true
/// of the only way a code could ever change. It no longer is: a pin that runs out is now replaced
/// automatically, and the flow returns to the same `Waiting` it was already in. A cache keyed on
/// the phase would have gone on drawing the dead code — sharp, scannable, and pointing at a pin
/// plex.tv had forgotten — for the whole of its successor's life. `Creating` is still checked, as
/// the belt to the generation's braces: it is the one moment a flow is known to have thrown its
/// code away before any replacement exists.
///
/// The texture has to be DELETED, not merely forgotten: [`ensure_qr_tex`] allocates a fresh id on
/// every miss (`img_upload_rgba` never reuses the old one), so zeroing the handle alone orphaned a
/// full 400x400-ish RGBA QR bitmap per sign-in retry, with nothing left holding its id to free it
/// later. `gfx::delete_tex` no-ops on 0, and both callers are on the main thread (the app loop's
/// `Route::Login` arm), which is where GL deletes must happen.
fn drop_a_stale_qr(s: &mut Scene, live: u64) {
    if !qr_cache_stale(s.qr_gen, live, auth::phase()) {
        return;
    }
    crate::gfx::delete_tex(s.qr_tex);
    s.qr_tex = 0;
    s.qr_gen = live;
}

/// Whether the cached QR bitmap has stopped describing the code the flow is showing.
///
/// Pure and split out from the delete for the reason every other rule on this screen is: the
/// caller frees a GL texture, so no host test can reach it, and this is the half that decides
/// whether a dead code stays on the television.
fn qr_cache_stale(cached: u64, live: u64, phase: Phase) -> bool {
    cached != live || phase == Phase::Creating
}

/// Decode + upload Plex's QR PNG once, caching the GL texture. Main (draw) thread only.
fn ensure_qr_tex(s: &mut Scene, qr: &auth::QrCode) {
    // The SNAPSHOT's generation, not a fresh read: the texture about to be uploaded and the number
    // it is cached under must come from one lock, or a later frame keys the new bitmap by the old
    // code. Both call sites run the same rule, so a replaced code can never be drawn out of a
    // cache that `update` happened not to have reached yet this frame.
    drop_a_stale_qr(s, qr.generation);
    if s.qr_tex != 0 || qr.png.is_empty() {
        return;
    }
    let (mut w, mut h): (c_int, c_int) = (0, 0);
    let px = crate::img::img_decode_rgba(qr.png.as_ptr(), qr.png.len() as c_int, &mut w, &mut h);
    if !px.is_null() {
        s.qr_tex = crate::img::img_upload_rgba(px, w, h);
        crate::img::img_free(px);
    }
}

pub fn draw() {
    #[cfg(feature = "jellyfin")]
    if jellyfin_flavor() {
        crate::ui::jf_login::draw();
        return;
    }
    crate::gfx::frame_clear(theme::CLEAR_RGB.0, theme::CLEAR_RGB.1, theme::CLEAR_RGB.2);
    let p = Painter::root();
    let s = scene();
    // The QR screen is the first thing a new user sees, before Home has any artwork to lend it.
    // Use the shared pre-content route ground rather than a local grey clear.
    s.ground.draw_default(p);
    let env = Env::inert();

    if auth::pending_persistence_warning().is_some() {
        let layout = RouteLayout::screen();
        layout.draw_narrative(
            p, None, "Your sign-in couldn’t be saved",
            "You’re signed in for this session, but this TV may ask you to sign in again after you close the app.",
            theme::size::LABEL,
        );
        TextView::new(
            "Choose Continue to use Plex now. If the app asks you to sign in again after closing, sign in once more.",
            theme::size::BODY, theme::TEXT_SECONDARY,
        ).draw(p, layout.content);
        draw_storage_action(p, &env, layout);
    } else { match auth::phase() {
        Phase::Waiting => draw_waiting(p, &env, s),
        Phase::Error => draw_failed(p, &env, s),
        Phase::Deleted => draw_deleted(p, &env, s),
        Phase::Discovering => draw_working(p, &env, s, "Finding your server\u{2026}"),
        _ => draw_working(p, &env, s, "Connecting to Plex\u{2026}"),
    } }
}

/// The three non-QR states are ONE centred read-out, not the two-column route.
///
/// **They used to be that route**, with the title and a sentence in the narrative column and a
/// lone spinner floating in the content column — which is the composition for a screen that has a
/// LIST or a document on the right, and reads as a broken one when the right-hand side holds a
/// single 26px ring. `StatusOverlay` is the app's existing answer for "the whole surface is
/// waiting": spinner over verdict over an optional reason over the one action, centred on the area
/// the wait is ABOUT — `Rect::FULL` here, since none of this screen exists yet.
fn draw_readout(
    p: Painter,
    env: &Env,
    s: &Scene,
    caption: &std::ffi::CStr,
    kind: StatusKind,
    reason: Option<&std::ffi::CStr>,
    action: Option<&'static std::ffi::CStr>,
    // The current report's delivery status, distinguishing queued from confirmed delivery.
    note: Option<&std::ffi::CStr>,
) {
    let mut o = StatusOverlay::new(Rect::FULL, caption, kind).phase(s.spin_ms as u32);
    if let Some(r) = reason {
        o = o.reason(r);
    }
    if let Some(a) = action {
        // The only control on the screen, so it holds focus by construction — there is nowhere
        // else for the ring to be, and OK must reach it without a press to move focus first.
        o = o.action(a).focused(true);
    }
    o.draw(env, p);
    let line_h = crate::text::text_height(theme::size::CAPTION, 0);
    let below = o.action_frame().map(|a| a.y + a.h);
    if let Some(n) = note {
        let y = below.map_or(o.frame.y + o.frame.h, |b| b + theme::space::SM);
        Label::new(n.as_ptr(), theme::size::CAPTION, theme::TEXT_SECONDARY)
            .h(HAlign::Center)
            .draw(p, Rect::new(o.frame.x, y, o.frame.w, line_h));
    }
}

fn draw_working(p: Painter, env: &Env, s: &Scene, msg: &str) {
    let caption = CString::new(msg).unwrap_or_default();
    let stuck = escape_ready(s);
    draw_readout(
        p,
        env,
        s,
        &caption,
        StatusKind::Working,
        // The reason arrives WITH the control, and only then: it exists to explain why a button
        // just appeared under a spinner that was doing fine a moment ago.
        stuck.then_some(c"This is taking longer than usual."),
        stuck.then_some(ESCAPE),
        None,
    );
}

fn draw_failed(p: Painter, env: &Env, s: &Scene) {
    let reason = CString::new(auth::error()).unwrap_or_default();
    draw_readout(
        p,
        env,
        s,
        c"Couldn\u{2019}t sign in",
        StatusKind::Failed,
        (!reason.is_empty()).then_some(reason.as_c_str()),
        Some(ESCAPE),
        None,
    );
}

/// What the delete actually achieved, as the two lines it is honest to draw.
///
/// **A partial wipe may not be reported as a whole one**, and that is not pedantry: the files this
/// sweep can fail on include the TELEMETRY decision, so a survivor is re-read on the next launch
/// and a consent the user believed they had deleted comes back. The session is gone either way —
/// `auth::erase_local_state` is unconditional — so the verdict stays true and the reason carries
/// the qualification.
fn deleted_readout(leftovers: usize) -> (&'static std::ffi::CStr, &'static std::ffi::CStr) {
    if leftovers == 0 {
        (
            c"Local data deleted",
            c"Credentials, preferences, telemetry and local diagnostics have been removed.",
        )
    } else {
        (
            c"Signed out, and most local data deleted",
            c"Some files could not be removed and may still be on this television.",
        )
    }
}

/// Record what the delete left behind, before the app routes here.
pub fn note_delete_leftovers(n: usize) {
    scene().delete_leftovers = n;
}

/// **Empty, not Failed.** Deleting everything is a completed action the user asked for, so it must
/// not wear the danger tint — the same distinction `StatusKind::Empty` carries for a library with
/// nothing in it. A partial one is still not a FAILURE either: what it did do, it did.
fn draw_deleted(p: Painter, env: &Env, s: &Scene) {
    let (verdict, reason) = deleted_readout(s.delete_leftovers);
    draw_readout(
        p,
        env,
        s,
        verdict,
        StatusKind::Empty,
        Some(reason),
        Some(c"Sign in"),
        None,
    );
}

fn draw_waiting(p: Painter, env: &Env, s: &mut Scene) {
    let layout = RouteLayout::screen();
    // **Issue #76 review: a stalled key service is not a signed-out television.** When a sealed
    // sign-in is sitting on this install that this launch could not read, the narrative column
    // says so and carries the one control that can fix it, while the QR stack on the right stays
    // exactly as it is — the explicit "sign in again" choice, which is the OTHER answer and must
    // not be the only one offered. See `plex::session::LOCKED_UNAVAILABLE`.
    if crate::plex::session::secure_unavailable() {
        layout.draw_narrative(p, None, STORAGE_TITLE, STORAGE_COPY, theme::size::LABEL);
    } else {
        // Rule 11, and [`ActionRow::clear`]'s own instruction: the branch that does not draw the
        // band forgets its frame, so the pill that is no longer on screen is not hit-testable
        // where it used to be. The read-out comes and goes WITHIN one visit to this route —
        // `secure_unavailable()` flips the moment a retry opens the envelope — so `enter()`
        // resetting the scene is not what covers this.
        layout.draw_narrative(
            p,
            None,
            "Sign in to Plex",
            "Use your phone camera to scan the code, or link this television manually with the address and code shown here.",
            theme::size::LABEL,
        );
    }
    // Details is a small, always-available information control on the QR screen.  Try again is
    // added only for the retryable secure-unavailable state; the normal sign-in surface stays
    // visually quiet until the user opens the panel.
    draw_storage_action(p, env, layout);
    let right = qr_layout(layout);
    // ONE read of the code, used for the bitmap, the digits and the sentence beneath them.
    let qr = auth::qr_snapshot();

    TextView::new("plex.tv/link", theme::size::TITLE, theme::TEXT_HEADING)
        .bold()
        .h(HAlign::Center)
        .draw(p, right.url);

    // QR on a bright card (the white border is the scan quiet-zone). Plex's own PNG → we just show it.
    let card = right.card;
    p.rrect(card, 24.0, 24.0, theme::SURFACE_QR_PLATE);
    ensure_qr_tex(s, &qr);
    if s.qr_tex != 0 {
        let pad = 30.0;
        let inner = Rect::new(
            card.x + pad,
            card.y + pad,
            card.w - 2.0 * pad,
            card.h - 2.0 * pad,
        );
        // Plex's PNG is WHITE modules on a transparent ground; tint black so the modules render dark
        // on the white card (the transparent ground shows the card) → a scannable black-on-white QR.
        p.tex(s.qr_tex, inner, 0.0, theme::scrim_black(1.0));
    } else {
        Spinner::new(card.x + card.w * 0.5, card.y + card.h * 0.5, 22.0)
            .phase(s.spin_ms as u32)
            .tint(theme::scrim_black(0.5))
            .draw(env, p);
    }

    // The manual code and waiting state remain in the same right-column stack as the URL and QR.
    // Both use couch-readable type rungs; this is an alternative sign-in path, not fine print.
    if let Ok(code) = CString::new(qr.code.to_uppercase()) {
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

    let link = auth::link_state();
    let wr = 15.0;
    let wy = right.status.cy();
    // `qr_escape_ready`, not the bare clock: while the storage read-out owns OK it also owns the
    // sentence that says to press it — see `storage_readout_showing`. The phase is `Waiting` by
    // construction here (this function is the `Waiting` arm), so the two agree everywhere else.
    let status = waiting_status(
        qr.replaced,
        qr_escape_ready(s),
        auth::link_unreachable(&link),
    );
    let status_w = crate::text::text_width(status.as_ptr(), theme::size::BODY, 0);
    let sx = right.status.cx() - (wr * 2.0 + theme::space::SM + status_w) * 0.5;
    Spinner::new(sx + wr, wy, wr)
        .phase(s.spin_ms as u32)
        .tint(theme::TEXT_SECONDARY)
        .draw(env, p);
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

}

/// The line under the code, which has to answer a question that only exists now that a code can be
/// replaced: *why is this not the code I was looking at*.
///
/// A pin lives fifteen minutes and is re-minted when it runs out, so somebody who walked away
/// mid-sign-in — or whose phone has just told them the OLD code was linked — comes back to
/// different digits. Saying nothing there reads as the television having lost track of itself, and
/// it is exactly the moment they need to be told to scan again. Same position, same rung, same
/// spinner: one sentence swapped for another, never a second read-out.
/// **`stalled` outranks `code_replaced`**: one of these sentences carries an ACTION, and a line
/// that explains history is worth less than the one that offers a way forward.
///
/// **`unreachable` outranks BOTH, for the same argument carried one step further (issue #75).** A
/// new code cannot help while plex.tv itself is not answering — pressing OK just mints another pin
/// nobody can poll — so once two consecutive polls have come back empty, the sentence that names
/// the real action (check this TV's connection) is worth more than either a code offer or a
/// history note, exactly as `stalled`'s own sentence already outranks `code_replaced`'s. The
/// [`auth::link_detail`] line drawn under this one carries the *why*; this one only ever needs to
/// say *what to do*.
fn waiting_status(
    code_replaced: bool,
    stalled: bool,
    unreachable: bool,
) -> &'static std::ffi::CStr {
    if unreachable {
        c"Can\u{2019}t reach Plex. Check your TV\u{2019}s internet connection."
    } else if stalled {
        c"Still waiting — choose New code to try again"
    } else if code_replaced {
        c"That code expired — scan this one"
    } else {
        c"Waiting for you to sign in…"
    }
}

/// How long a QR code may go unscanned before the screen offers to replace it on request.
///
/// **A separate, much longer clock than [`ESCAPE_AFTER_MS`], because this wait is not a stall.**
/// Twelve seconds is right for a spinner that should have finished in one; a code on screen is
/// waiting for a person to find their phone, unlock it, open a camera and tap a link, and nagging
/// them at twelve seconds would be wrong every time. A full minute of a code that has already been
/// scanned is not.
///
/// It exists because the automatic replacement below cannot cover the case the issue reported: the
/// phone says *Account linked* while our polls are being answered `Pending` or nothing at all, and
/// the person watching knows something the television does not. Waiting out the rest of a
/// fifteen-minute lease is not a recovery.
const QR_ESCAPE_AFTER_MS: f32 = 60_000.0;

/// **What the screen is waiting ON**, as the pair the clock in [`Scene::phase_ms`] is timing.
///
/// The phase alone was the whole identity while a code could only change by leaving `Waiting`. It
/// cannot be any more: a pin that runs out is replaced automatically, `Waiting → Creating →
/// Waiting`, and `update` samples once a frame — so a replacement completed between two samples
/// (a paused main loop, a long frame) is invisible, and the FRESH code inherits the dead one's
/// age. It would then offer New code about a code that had existed for a
/// millisecond. Including the generation makes the reset exact rather than probable.
fn wait_id() -> (Phase, u64) {
    (auth::phase(), auth::qr_generation())
}

/// Is what the screen is waiting on a DIFFERENT thing from what it was waiting on last frame?
///
/// Trivial, and separate anyway, because the rule it encodes is not: a new CODE restarts the clock
/// exactly as a new PHASE does, and the version that compared phases alone is the one that would
/// offer to replace a code a millisecond old.
fn wait_restarted(seen: (Phase, u64), live: (Phase, u64)) -> bool {
    seen != live
}

/// Whether the QR screen is offering its own replacement right now. Pure, and — like
/// [`escape_ready`] — the ONE predicate behind both the sentence and the key, so a control that
/// is not drawn can never be activated.
fn qr_escape_offered(phase_ms: f32) -> bool {
    phase_ms >= QR_ESCAPE_AFTER_MS
}

/// **Is the sealed-storage read-out on screen right now?** — the ONE predicate behind its copy,
/// its control and the key that activates it, so, exactly as [`escape_ready`] does for the stalled
/// spinner, a control that is not drawn can never be pressed.
///
/// It draws over `Waiting` alone: the other three phases are their own full-screen read-outs
/// (`draw_working`/`draw_failed`/`draw_deleted`), each already carrying one control of its own,
/// and a second offer stacked on those is a screen with two answers to one press.
///
/// `pub(crate)` (not just this module's own `key`) since 2026-09-10: it is also the predicate
/// `app.rs`'s `key_onboarding` uses to decide whether an OK-down on this route arms the shared
/// [`crate::ui::press`] control-face press (see [`commit_storage_retry`]) instead of falling
/// through to [`key`] — the same one-predicate-for-draw-and-key rule, now spanning the module
/// boundary the arm/commit split puts between the two halves of one press.
pub(crate) fn storage_readout_showing() -> bool {
    auth::phase() == Phase::Waiting && crate::plex::session::secure_unavailable()
}

/// The action row exists for every waiting QR screen: Details is the quiet information control,
/// while the retry pill is present only when the key service can be asked again.
pub(crate) fn storage_action_showing() -> bool {
    auth::phase() == Phase::Waiting || auth::pending_persistence_warning().is_some()
}

/// **`app.rs`'s deferred half of the storage-retry press.** [`key_onboarding`](crate::app) arms
/// `press::begin_ctl` on the OK-down (see [`storage_readout_showing`]) instead of calling
/// straight into [`start_storage_retry`], so the spring-back bounce from `press::release` is
/// actually on screen before the retry fires — the same arm/commit split every other
/// route-family action pill uses. Called from the per-frame `press::take_commit` dispatch, once,
/// which is what makes "exactly once" a property of `press` (one commit per press) rather than of
/// this function. Commit validation also rejects the arm if a modal took ownership after OK-down.
pub(crate) fn commit_storage_retry() {
    let armed = ARMED_STORAGE_ACTION.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some((action, attempt)) = armed else { return };
    if let Some(warning) = auth::pending_persistence_warning() {
        if !warning_action_valid(action, attempt, warning.key, modal_open()) { return; }
        match action {
            StorageAction::Continue(key) => { auth::acknowledge_persistence_warning(key); }
            _ => {}
        }
        crate::ui::idle::invalidate();
        return;
    }
    if !armed_storage_action_valid(
        action,
        attempt,
        auth::current_attempt(),
        auth::phase() == Phase::Waiting,
        crate::plex::session::secure_unavailable(),
        qr_escape_ready(scene()),
        modal_open(),
    ) {
        return;
    }
    match action {
        StorageAction::Continue(_) => {}
        StorageAction::RetrySecureOpen => start_storage_retry(),
        StorageAction::NewCode => {
            let _ = auth::restart_stalled_wait(scene().wait);
        }
    }
}

/// Pointer twin of the action-row key path.  Hit-testing uses the same shared ActionRow rects the
/// draw recorded, then moves focus before the app arms the ordinary control-face press.
pub(crate) fn action_press_at(mx: f32, my: f32) -> bool {
    if !storage_action_showing() || modal_open() {
        return false;
    }
    let Some(slot) = (unsafe { (*addr_of_mut!(STORAGE_ACTION_POP)).hit(mx, my) }) else {
        return false;
    };
    if slot == STORAGE_FOCUS_RETRY && storage_primary_action().is_none() {
        return false;
    }
    STORAGE_FOCUS.store(slot, Ordering::Release);
    crate::ui::idle::invalidate();
    true
}

fn qr_escape_ready(s: &Scene) -> bool {
    auth::phase() == Phase::Waiting && qr_escape_offered(s.phase_ms)
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
    /// **Issue #75.** "A report about this sign-in was sent", directly under the status — drawn only
    /// once the current attempt's trouble has actually been sent, in the same secondary stack.
    note: Rect,
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
    // The raw curl/HTTP diagnostic used to reserve two extra lines here. It remains in reports,
    // while the ordinary QR screen shows one plain connection status.
    let note_h = theme::size::CAPTION as f32;
    let status_y = card.y + card.h + theme::space::LG + code_h + theme::space::MD;
    let note_y = status_y + status_h + theme::space::SM;
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
        status: Rect::new(layout.content.x, status_y, layout.content.w, status_h),
        note: Rect::new(
            layout.content.x,
            note_y,
            layout.content.w,
            note_h,
        ),
    }
}

pub fn key(sym: c_uint, wcode: c_uint) {
    #[cfg(feature = "jellyfin")]
    if jellyfin_flavor() {
        crate::ui::jf_login::key(sym, wcode);
        return;
    }
    // BACK leaves the app in the platform handler, never replacing this unsaved session with
    // cached credentials. Only the explicitly labelled Continue acknowledges the warning.
    if auth::pending_persistence_warning().is_some() && is_back(sym, wcode) {
        return;
    }
    if auth::phase() == Phase::Deleted && is_ok(sym) {
        auth::start_login();
        return;
    }
    // **The storage read-out owns OK while it is drawn**, and it is drawn only over `Waiting`
    // (`draw_waiting`) — the same one-predicate-for-draw-and-key rule the two timed escapes below
    // follow, and the reason `qr_escape_ready` refuses while this is up: one key cannot mean both
    // "ask the key service again" and "mint a new pin", and the control the user can SEE is the
    // one it has to mean.
    //
    // **This branch is now a defensive no-op on the real key path, not a dead one to delete.**
    // Since 2026-09-10 `app.rs`'s `key_onboarding` intercepts an OK-down here BEFORE calling into
    // this function at all — it arms `press::begin_ctl` instead (see
    // [`storage_readout_showing`]'s doc), and the actual retry now fires once from
    // `press::take_commit`'s per-frame dispatch, via [`commit_storage_retry`]. Swallowing the
    // press here rather than re-triggering it is what keeps a caller that reaches this function
    // some other way (a test, or a future key source with no press arm of its own) from starting
    // a SECOND worker underneath the deferred one — `start_storage_retry`'s own compare-exchange
    // guard already refuses to stack, so this is belt-and-suspenders, not the primary gate.
    if is_ok(sym) && storage_action_showing() {
        return;
    }
    if auth::phase() == Phase::Error && is_ok(sym) {
        auth::retry();
        return;
    }
    // **The two timed escapes are ONE press, and they must be, because they share one clock.**
    // The QR screen's *New code* action (60 s) and the stalled spinner's *Try again*
    // (12 s) both hang off `phase_ms`, so a wait that leaves `Waiting` for `Discovering` between
    // the draw and the key made the first predicate false and the SECOND one true — on the old
    // code's timer, down the unguarded path. `auth::restart_stalled_wait` takes the wait this
    // screen actually timed and refuses if the flow has moved on, so the phase it lands on cannot
    // disagree with the phase that earned the control. A `false` means exactly that happened and
    // the press is swallowed; the main loop is about to route away from here anyway.
    if is_ok(sym) && (qr_escape_ready(scene()) || escape_ready(scene())) {
        // "requested", not "restarted": the press may still be refused a line later, and the
        // event log is the one place this failure is read from — a claim it did something is
        // exactly the wrong thing to have written there.
        crate::log("login: user requested a restart of a stalled sign-in");
        if auth::restart_stalled_wait(scene().wait) {
            // The restart usually re-enters the phase it just left (a stalled `Creating` starts
            // another `Creating`), and `update` only zeroes the clock when the wait's IDENTITY
            // changes — a fresh code changes it, a re-entered phase may not — so without this the
            // new attempt could inherit the dead one's age and show its way out immediately.
            scene().phase_ms = 0.0;
        }
        return;
    }
    // BACK backs out of the sign-in — but only when there is somewhere to back out TO. This screen
    // is reached two ways: a first-ever boot with no session (nothing behind it — the QR screen is
    // the whole app) and the Home account menu's "Sign in" (a working session is still on disk).
    // `auth::cancel` is the one that knows which, so it decides: it resumes the stored session and
    // the main loop routes Home, or reports false and leaves the flow running. In practice this
    // arm is reached only from a path that bypassed `app::key_onboarding`'s root rule — that rule
    // claims every BACK here first and sends a refused one to the television's Home.
    if is_back(sym, wcode) {
        auth::cancel();
    }
    // otherwise the login screen just waits — the pin poll drives the phase from a worker thread.
}

/// The phases that are genuinely WAITING ON A NETWORK CALL and can therefore stall.
///
/// **An allowlist, not "everything that is not terminal".** It was the latter for an hour, which
/// swept in `Ready`, `Profiles` and `Switching` — phases the main loop routes away from on its
/// next pass. Key input is dispatched before that pass, so an OK aimed at the escape control the
/// user could still see would have called `auth::retry` on a flow that had already SUCCEEDED,
/// replacing a completed handoff with a fresh sign-in. `Idle` is excluded for the same reason
/// from the other side: nothing is owed, so there is nothing to retry.
fn working_phase(phase: Phase) -> bool {
    matches!(phase, Phase::Creating | Phase::Discovering)
}

/// Whether the read-out is showing its way out right now.
///
/// One predicate for the draw AND the key handler, so a control that is not drawn can never be
/// activated — the rule `player_hud::transport_hidden` states for the same hazard.
fn escape_ready(s: &Scene) -> bool {
    working_phase(auth::phase()) && escape_offered(s.phase_ms)
}
