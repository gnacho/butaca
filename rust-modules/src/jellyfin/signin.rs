//! signin — the onboarding form's flow: one `AuthenticateByName` attempt on a worker, then
//! Ready (client installed, config persisted) or a reason the form can say out loud.
//!
//! This is the Jellyfin flavor's counterpart to Plex's QR/pin flow, at a tenth of the moving
//! parts: there is no plex.tv, no pin to poll, no server discovery — the user types the three
//! facts (server, name, password) and the answer comes back from one POST. The shape it shares
//! with every other store in this crate: the worker never touches the screen's statics, the
//! mailbox carries the outcome, and an epoch discards a slow attempt's answer after a retry.
//!
//! Success does THREE things, in this order, all on the worker: authenticate (token in RAM),
//! install the client (stores can answer the moment the route flips), persist the config (the
//! next boot signs in by itself — `boot::save_config`). The main thread then sees [`take_ready`]
//! and routes Home, exactly as the Plex flow's handoff does.

use super::client::{AuthError, JfClient};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

/// Why a connect attempt ended without a session. The form words these; the flow logs them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fail {
    /// The address will not parse at all — the request was never built.
    Parse,
    /// The address parses but would carry the password over plaintext to somewhere off the
    /// LAN — the gate's rule, stated up front instead of surfacing as a silent refusal.
    Plaintext,
    /// The server answered 401/403: the name or the password is wrong.
    Refused,
    /// Nothing answered, or the answer made no sense.
    Unreachable,
}

/// The one question the screen asks: what should it be drawing?
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Phase {
    /// No attempt in flight — the form owns the remote.
    Editing,
    /// A connect attempt is out; the form shows a spinner and ignores Connect.
    Working,
    /// The last attempt failed; the form owns the remote AND shows the reason.
    Failed(Fail),
    /// Set by [`take_ready`] the one time it answers true — the route is leaving, nothing here
    /// is drawn again this run.
    Ready,
}

/// Worker → main: the attempt's epoch and its outcome. `Err(false)`… is not a state: every
/// failure is one of [`Fail`]'s three words.
struct Outcome {
    epoch: u32,
    result: Result<(), Fail>,
}

static MAIL: Mutex<Option<Outcome>> = Mutex::new(None);
static EPOCH: AtomicU32 = AtomicU32::new(0);
static mut PHASE: Phase = Phase::Editing;

pub(crate) fn phase() -> Phase {
    unsafe { std::ptr::addr_of!(PHASE).read() }
}

fn set_phase(p: Phase) {
    unsafe { *std::ptr::addr_of_mut!(PHASE) = p };
    crate::ui::idle::invalidate();
}

/// What the user typed, tidied for the wire: scheme defaults to `http://` because nobody should
/// have to type it on a TV remote, and trailing slashes come off because `Origin::parse` keeps
/// the address verbatim and every endpoint here is joined with a leading one.
pub(crate) fn normalize_url(raw: &str) -> String {
    let t = raw.trim();
    let with_scheme = if t.contains("://") {
        t.to_string()
    } else {
        format!("http://{t}")
    };
    with_scheme.trim_end_matches('/').to_string()
}

/// Begin one connect attempt. Validates the address BEFORE any network I/O — the two refusals
/// that need no server (unparseable, and plaintext-off-the-LAN) fail synchronously with the
/// attempt never leaving the device.
///
/// Answers `Err(fail)` synchronously for those; `Ok(())` means the worker is out and [`phase`]
/// is `Working` until [`take_ready`] or the next [`take_outcome`].
pub(crate) fn start(url_raw: &str, user: &str, password: &str) -> Result<(), Fail> {
    let url = normalize_url(url_raw);
    let Some(origin) = crate::plex::Origin::parse(&url) else {
        return Err(Fail::Parse);
    };
    // The identity header is a credential shape even before it carries a token (client.rs says
    // so at the call site), so this is the exact check the POST would face — asked up front so
    // the form can say WHY instead of reporting a refusal as "unreachable".
    if !crate::http::credential_transport_allowed(
        &origin,
        "/Users/AuthenticateByName",
        &["Authorization: MediaBrowser …"],
    ) {
        crate::log("jellyfin: sign-in refused at the gate — plaintext credentials off-LAN");
        return Err(Fail::Plaintext);
    }
    let epoch = EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    let (user, password) = (user.to_string(), password.to_string());
    set_phase(Phase::Working);
    let spawned = crate::task::spawn_small("jf-signin", move || {
        let outcome = std::panic::catch_unwind(move || {
            let device_id = super::boot::device_id_persistent();
            let client = JfClient::new(origin, device_id.clone());
            match client.authenticate_by_name(&user, &password) {
                Ok(ok) => {
                    crate::log(&format!(
                        "jellyfin: signed in as {} — server {}",
                        ok.user_name, ok.server_id
                    ));
                    super::install(client);
                    super::boot::save_config(&url, &ok.user_name, &password, &device_id);
                    Ok(())
                }
                Err(AuthError::Unauthorized) => Err(Fail::Refused),
                Err(AuthError::Unreachable) => Err(Fail::Unreachable),
            }
        })
        .unwrap_or(Err(Fail::Unreachable));
        if let Ok(mut slot) = MAIL.lock() {
            *slot = Some(Outcome { epoch, result: outcome });
        }
    });
    if !spawned {
        // The thread ceiling said no; nothing will fill the mailbox. Say so synchronously rather
        // than spinning forever — `maybe_spawn`'s refusal rule, applied to a one-shot.
        set_phase(Phase::Failed(Fail::Unreachable));
    }
    Ok(())
}

/// The main thread's read of the mailbox: the LATEST attempt's outcome, once. A stale epoch (a
/// slow attempt answered after a retry) is discarded without touching the phase.
pub(crate) fn take_outcome() -> Option<Result<(), Fail>> {
    let out = MAIL.lock().ok()?.take()?;
    if out.epoch != EPOCH.load(Ordering::SeqCst) {
        crate::log("jellyfin: a superseded sign-in attempt answered — discarded");
        return None;
    }
    Some(out.result)
}

/// The boot handoff: true exactly once per successful attempt, after which the phase is Ready
/// and the route moves to Home. Failure outcomes move the phase to Failed and answer false —
/// the form keeps the remote and shows the reason.
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

/// Back to a clean form — the sign-out path, and the screen's own "edit these answers" after a
/// failure. Bumps the epoch so an attempt still in flight cannot come back as a success.
pub(crate) fn reset() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut slot) = MAIL.lock() {
        *slot = None;
    }
    set_phase(Phase::Editing);
}

#[cfg(all(test, feature = "jellyfin"))]
mod tests {
    use super::*;

    #[test]
    fn the_scheme_is_filled_and_trailing_slashes_trimmed() {
        assert_eq!(normalize_url("192.168.1.20:8096"), "http://192.168.1.20:8096");
        assert_eq!(normalize_url(" http://10.0.0.2:8096/ "), "http://10.0.0.2:8096");
        assert_eq!(
            normalize_url("https://jellyfin.example.com/"),
            "https://jellyfin.example.com"
        );
    }

    #[test]
    fn an_unparseable_address_never_reaches_the_network() {
        let before = EPOCH.load(Ordering::SeqCst);
        assert_eq!(start("http://", "u", "p"), Err(Fail::Parse));
        assert_eq!(start("://", "u", "p"), Err(Fail::Parse));
        assert_eq!(EPOCH.load(Ordering::SeqCst), before, "no attempt was started");
        assert_eq!(phase(), Phase::Editing);
    }

    #[test]
    fn plaintext_credentials_off_the_lan_are_refused_before_the_wire() {
        // The SHIPPED rule is graded here, not this (dev-featured) build's: `devtriggers` relaxes
        // the gate for lab work, so the test asks the policy directly with the relaxation off —
        // exactly what a release .ipk ships.
        let origin = crate::plex::Origin::parse("http://203.0.113.10:8096").unwrap();
        assert!(
            !crate::http::credential_transport_allowed_by_policy(
                &origin,
                "/Users/AuthenticateByName",
                &["Authorization: MediaBrowser …"],
                false,
            ),
            "a public address over http must not carry the password"
        );
        let lan = crate::plex::Origin::parse("http://192.168.1.20:8096").unwrap();
        assert!(
            crate::http::credential_transport_allowed_by_policy(
                &lan,
                "/Users/AuthenticateByName",
                &["Authorization: MediaBrowser …"],
                false,
            ),
            "…while the LAN literal the PoC targets stays allowed"
        );
    }

    #[test]
    fn a_stale_outcome_is_discarded() {
        // An answer from epoch N arriving while the counter sits at N+1 is a slow attempt
        // coming home after a retry: take_outcome must drop it, not fail the fresh form.
        let epoch = EPOCH.load(Ordering::SeqCst);
        *MAIL.lock().unwrap() = Some(Outcome {
            epoch,
            result: Err(Fail::Refused),
        });
        EPOCH.store(epoch + 1, Ordering::SeqCst);
        assert!(take_outcome().is_none());
        EPOCH.store(epoch, Ordering::SeqCst); // hand the counter back for the other tests
    }
}
