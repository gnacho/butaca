//! Quick Connect: the passwordless sign-in flow (flavor `jellyfin` only — this module does not
//! exist on a Plex build).
//!
//! One worker runs the whole dance: `POST /QuickConnect/Initiate` for a secret + code, then a
//! BOUNDED, CANCELABLE poll of `GET /QuickConnect/Connect?secret=` until the server reports the
//! request authorized, then `POST /Users/AuthenticateWithQuickConnect` trades the secret for the
//! access token. The screen mirrors [`phase`] and reads the code from [`code`]; a code becomes
//! available only after Initiate answered, so the screen shows the spinner until then and the code
//! after. Success lands exactly where the password flow's does (`super::install` +
//! `super::boot::save_config`/`append_server`); the shape it shares with every other store in this
//! crate: the worker never touches the screen's statics, the mailbox carries the outcome, and an
//! epoch discards a slow attempt's answer after a retry.

use super::client::{AuthError, JfClient};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

/// Why a Quick Connect attempt ended without a session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fail {
    /// The address will not parse at all — the request was never built.
    Parse,
    /// The address parses but would carry the identity over plaintext off the LAN.
    Plaintext,
    /// The server is too old: Quick Connect arrived in Jellyfin 10.8.0.
    Unsupported,
    /// The poll exhausted its budget without authorization, or the viewer cancelled it.
    TimedOut,
    /// The secret was authorized but the token trade was refused (400/401/403).
    Refused,
    /// Nothing answered, or the answer made no sense.
    Unreachable,
}

/// What the screen should be drawing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Phase {
    /// No attempt in flight — the form owns the remote.
    Editing,
    /// An attempt is out (Initiate, or the poll after it).
    Working,
    /// The last attempt failed; the form owns the remote AND shows the reason.
    Failed(Fail),
    /// Set by [`take_ready`] the one time it answers true — the route is leaving.
    Ready,
}

/// The poll's cadence and budget: the worker asks `Connect` every [`POLL_EVERY`] for at most
/// [`POLL_BUDGET`], then gives up. A viewer who stops waiting cancels through [`cancel`].
pub(crate) const POLL_EVERY: std::time::Duration = std::time::Duration::from_secs(2);
pub(crate) const POLL_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

struct Outcome {
    epoch: u32,
    result: Result<(), Fail>,
}

static MAIL: Mutex<Option<Outcome>> = Mutex::new(None);
static EPOCH: AtomicU32 = AtomicU32::new(0);
static CODE: Mutex<Option<String>> = Mutex::new(None);
static CANCEL: AtomicBool = AtomicBool::new(false);
static ADD_SERVER: AtomicBool = AtomicBool::new(false);
static mut PHASE: Phase = Phase::Editing;

pub(crate) fn phase() -> Phase {
    unsafe { std::ptr::addr_of!(PHASE).read() }
}

fn set_phase(p: Phase) {
    unsafe { *std::ptr::addr_of_mut!(PHASE) = p };
    crate::ui::idle::invalidate();
}

/// The code to show, once Initiate has answered. `None` while the attempt is still starting.
pub(crate) fn code() -> Option<String> {
    CODE.lock().ok().and_then(|c| c.clone())
}

/// Arm or disarm add-server mode, exactly as [`super::signin::set_add_mode`] does: a success then
/// appends the new server instead of replacing the configured list.
pub(crate) fn set_add_mode(on: bool) {
    ADD_SERVER.store(on, Ordering::SeqCst);
}

pub(crate) fn add_mode() -> bool {
    ADD_SERVER.load(Ordering::SeqCst)
}

/// Ask the worker to stop polling. Idempotent; [`reset`] clears it.
pub(crate) fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// Begin one Quick Connect attempt. Validates the address BEFORE any network I/O — the two
/// refusals that need no server (unparseable, and plaintext-off-the-LAN) fail synchronously with
/// the attempt never leaving the device. `Ok(())` means the worker is out and [`phase`] is
/// `Working` until [`take_ready`] or the next [`take_outcome`].
pub(crate) fn start(url_raw: &str) -> Result<(), Fail> {
    let url = super::signin::normalize_url(url_raw);
    let Some(origin) = crate::origin::Origin::parse(&url) else {
        return Err(Fail::Parse);
    };
    // The identity header is a credential shape even before it carries a token, so this is the
    // exact check the POST would face, asked up front so the form can say WHY.
    if !crate::http::credential_transport_allowed(
        &origin,
        "/QuickConnect/Initiate",
        &["Authorization: MediaBrowser …"],
    ) {
        crate::log("jellyfin: quick connect refused at the gate — plaintext credentials off-LAN");
        return Err(Fail::Plaintext);
    }
    let epoch = EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    let add_server = ADD_SERVER.load(Ordering::SeqCst);
    *CODE.lock().unwrap() = None;
    CANCEL.store(false, Ordering::SeqCst);
    set_phase(Phase::Working);
    let spawned = crate::task::spawn_small("jf-quick-connect", move || {
        let outcome = std::panic::catch_unwind(move || {
            let device_id = super::boot::device_id_persistent();
            let client = JfClient::new(origin, device_id.clone());
            if !client.quick_connect_supported() {
                return Err(Fail::Unsupported);
            }
            let Some(result) = client.quick_connect_initiate() else {
                return Err(Fail::Unreachable);
            };
            *CODE.lock().unwrap() = Some(result.code.clone());
            crate::ui::idle::invalidate();
            let deadline = std::time::Instant::now() + POLL_BUDGET;
            loop {
                if CANCEL.load(Ordering::SeqCst) || std::time::Instant::now() >= deadline {
                    return Err(Fail::TimedOut);
                }
                match client.quick_connect_state(&result.secret) {
                    Some(state) if state.authenticated => {
                        match client.authenticate_with_quick_connect(&result.secret) {
                            Ok(ok) => {
                                crate::log(&format!(
                                    "jellyfin: quick connect signed in as {} — server {}",
                                    ok.user_name, ok.server_id
                                ));
                                super::install(client);
                                if add_server {
                                    super::boot::append_server(&url, &ok.user_name, &ok.token, &device_id);
                                } else {
                                    super::boot::save_config(&url, &ok.user_name, &ok.token, &device_id);
                                }
                                return Ok(());
                            }
                            Err(AuthError::Unauthorized) => return Err(Fail::Refused),
                            Err(AuthError::Unreachable) => return Err(Fail::Unreachable),
                        }
                    }
                    Some(_) => {} // not yet authorized; keep polling
                    None => return Err(Fail::Unreachable),
                }
                std::thread::sleep(POLL_EVERY);
            }
        })
        .unwrap_or(Err(Fail::Unreachable));
        if let Ok(mut slot) = MAIL.lock() {
            *slot = Some(Outcome { epoch, result: outcome });
        }
    });
    if !spawned {
        set_phase(Phase::Failed(Fail::Unreachable));
    }
    Ok(())
}

/// The main thread's read of the mailbox: the LATEST attempt's outcome, once. A stale epoch is
/// discarded without touching the phase.
pub(crate) fn take_outcome() -> Option<Result<(), Fail>> {
    let out = MAIL.lock().ok()?.take()?;
    if out.epoch != EPOCH.load(Ordering::SeqCst) {
        crate::log("jellyfin: a superseded quick connect attempt answered — discarded");
        return None;
    }
    Some(out.result)
}

/// The boot handoff: true exactly once per successful attempt, after which the phase is Ready and
/// the route moves to Home.
pub(crate) fn take_ready() -> bool {
    match take_outcome() {
        Some(Ok(())) => {
            set_phase(Phase::Ready);
            true
        }
        Some(Err(f)) => {
            set_phase(Phase::Failed(f));
            false
        }
        None => false,
    }
}

/// Back to a clean form. Bumps the epoch so an attempt still in flight cannot come back as a
/// success, and clears the code and the cancel flag.
pub(crate) fn reset() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
    CANCEL.store(false, Ordering::SeqCst);
    if let Ok(mut slot) = MAIL.lock() {
        *slot = None;
    }
    if let Ok(mut c) = CODE.lock() {
        *c = None;
    }
    set_phase(Phase::Editing);
}

#[cfg(all(test, feature = "jellyfin"))]
mod tests {
    use super::*;

    #[test]
    fn an_unparseable_address_never_reaches_the_network() {
        let _guard = crate::testlock::serial();
        reset();
        let before = EPOCH.load(Ordering::SeqCst);
        assert_eq!(start("http://"), Err(Fail::Parse));
        assert_eq!(start("://"), Err(Fail::Parse));
        assert_eq!(EPOCH.load(Ordering::SeqCst), before, "no attempt was started");
        assert_eq!(phase(), Phase::Editing);
    }

    #[test]
    fn plaintext_credentials_off_the_lan_are_refused_before_the_wire() {
        // The SHIPPED rule is graded here, not this (dev-featured) build's: `devtriggers` relaxes
        // the gate for lab work, so the test asks the policy directly with the relaxation off.
        let origin = crate::origin::Origin::parse("http://203.0.113.10:8096").unwrap();
        assert!(
            !crate::http::credential_transport_allowed_by_policy(
                &origin,
                "/QuickConnect/Initiate",
                &["Authorization: MediaBrowser …"],
                crate::origin::CredentialPolicy::HttpsOnly,
            ),
            "a public address over http must not carry the identity"
        );
        let lan = crate::origin::Origin::parse("http://192.168.1.20:8096").unwrap();
        assert!(
            crate::http::credential_transport_allowed_by_policy(
                &lan,
                "/QuickConnect/Initiate",
                &["Authorization: MediaBrowser …"],
                crate::origin::CredentialPolicy::HttpsOnly,
            ),
            "…while the LAN literal stays allowed"
        );
    }

    #[test]
    fn a_stale_outcome_is_discarded() {
        let epoch = EPOCH.load(Ordering::SeqCst);
        *MAIL.lock().unwrap() = Some(Outcome {
            epoch,
            result: Err(Fail::Refused),
        });
        EPOCH.store(epoch + 1, Ordering::SeqCst);
        assert!(take_outcome().is_none());
        EPOCH.store(epoch, Ordering::SeqCst);
    }
}
