//! signin — the onboarding form's flow: one `AuthenticateByName` attempt on a worker, then
//! Ready (client installed, config persisted) or a reason the form can say out loud.
//!
//! This is the Jellyfin flavor's counterpart to the retired QR/pin flow, at a tenth of the moving
//! parts: there is no account service, no pin to poll, no server discovery, so the user types the
//! three facts (server, name, password) and the answer comes back from one POST. The shape it
//! shares with every other store in this crate: the worker never touches the screen's statics, the
//! mailbox carries the outcome, and an epoch discards a slow attempt's answer after a retry.
//!
//! Success does THREE things, in this order, all on the worker: authenticate (token in RAM),
//! install the client (stores can answer the moment the route flips), persist the config (the
//! next boot signs in by itself — `boot::save_config`). The main thread then sees [`take_ready`]
//! and routes Home, exactly as the retired flow's handoff did.

use super::client::{AuthError, JfClient};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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
/// **Add-server mode**: the Settings "Conexión rápida" arm sets this before routing to the form.
/// A successful attempt then APPENDS the new server to the configured list (and activates it)
/// instead of REPLACING the whole config with a single entry. Reset by the caller once the
/// landing follower consumes the success.
static ADD_SERVER: AtomicBool = AtomicBool::new(false);

pub(crate) fn phase() -> Phase {
    unsafe { std::ptr::addr_of!(PHASE).read() }
}

fn set_phase(p: Phase) {
    unsafe { *std::ptr::addr_of_mut!(PHASE) = p };
    crate::ui::idle::invalidate();
}

/// Arm or disarm add-server mode, the flag [`start`]'s worker reads once when it captures it.
pub(crate) fn set_add_mode(on: bool) {
    ADD_SERVER.store(on, Ordering::SeqCst);
}

/// Read it: the sign-in SCREEN needs to know whether BACK is a way out (add-server from
/// Settings, pushed over the page that asked) or stays swallowed (the boot gate's own form,
/// which has nothing behind it).
pub(crate) fn add_mode() -> bool {
    ADD_SERVER.load(Ordering::SeqCst)
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
    // `EPOCH`/`PHASE` are crate globals a retry mutates; a test that starts an attempt outside
    // `crate::testlock::serial()` races the other sign-in tests on the same counter — see
    // `lib.rs::testlock`.
    #[cfg(test)]
    crate::testlock::assert_held("the jellyfin sign-in store (start)");
    let url = normalize_url(url_raw);
    let Some(origin) = crate::origin::Origin::parse(&url) else {
        return Err(Fail::Parse);
    };
    // The identity header is a credential shape even before it carries a token (client.rs says
    // so at the call site), so this is the exact check the POST would face — asked up front so
    // the form can say WHY instead of reporting a refusal as "unreachable". It is the SIGN-IN gate,
    // not the per-request one: this POST installs the client the anchoring rule then measures
    // every other request against (`http::credential_transport_allowed_at_signin`).
    if !crate::http::credential_transport_allowed_at_signin(
        &origin,
        "/Users/AuthenticateByName",
        &["Authorization: MediaBrowser …"],
    ) {
        crate::log("jellyfin: sign-in refused at the gate — plaintext credentials off-LAN");
        return Err(Fail::Plaintext);
    }
    let epoch = EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    let (user, password) = (user.to_string(), password.to_string());
    let add_server = ADD_SERVER.load(Ordering::SeqCst);
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
                    if add_server {
                        super::boot::append_server(&url, &ok.user_name, &ok.token, &device_id);
                    } else {
                        super::boot::save_config(&url, &ok.user_name, &ok.token, &device_id);
                    }
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
    // `EPOCH`/`MAIL`/`PHASE` are crate globals; a test that flips them outside
    // `crate::testlock::serial()` lands in the middle of some other module's test — see
    // `lib.rs::testlock`.
    #[cfg(test)]
    crate::testlock::assert_held("the jellyfin sign-in store (reset)");
    EPOCH.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut slot) = MAIL.lock() {
        *slot = None;
    }
    set_phase(Phase::Editing);
}

#[cfg(test)]
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
        // The module keeps flow state across tests in this process (the servers-list tests
        // legitimately leave the form signed in) — start from a known phase rather than
        // whatever a neighbour left.
        let _guard = crate::testlock::serial();
        reset();
        let before = EPOCH.load(Ordering::SeqCst);
        assert_eq!(start("http://", "u", "p"), Err(Fail::Parse));
        assert_eq!(start("://", "u", "p"), Err(Fail::Parse));
        assert_eq!(EPOCH.load(Ordering::SeqCst), before, "no attempt was started");
        assert_eq!(phase(), Phase::Editing);
    }

    /// The regression #204: on a clean install there is no client to anchor the plaintext
    /// relaxation to, and the sign-in POST is what installs one. Graded on the SHIPPED policy
    /// (`HttpsOnly`), not this dev-featured build's: the gate the form asks is the sign-in one.
    #[test]
    fn plaintext_credentials_off_the_lan_are_refused_before_the_wire() {
        let _guard = crate::testlock::serial();
        crate::jellyfin::uninstall();
        let store = crate::origin::CredentialPolicy::HttpsOnly;
        let path = "/Users/AuthenticateByName";
        let headers = ["Authorization: MediaBrowser …"];
        let origin = crate::origin::Origin::parse("http://203.0.113.10:8096").unwrap();
        assert!(
            !crate::http::credential_transport_allowed_at_signin_by_policy(
                &origin, path, &headers, store
            ),
            "a public address over http must not carry the password"
        );
        let lan = crate::origin::Origin::parse("http://10.9.9.7:8096").unwrap();
        assert!(
            crate::http::credential_transport_allowed_at_signin_by_policy(
                &lan, path, &headers, store
            ),
            "…while the LAN literal a home server lives on is exactly what sign-in must reach"
        );
        assert!(
            !crate::http::credential_transport_allowed_by_policy(
                &lan,
                "/Users/me/Items?api_key=t",
                &[],
                store,
            ),
            "…and nothing else may follow it there until the sign-in installs the anchor"
        );
        let named = crate::origin::Origin::parse("http://jellyfin.local:8096").unwrap();
        assert!(
            !crate::http::credential_transport_allowed_at_signin_by_policy(
                &named, path, &headers, store
            ),
            "a hostname is never the carve-out"
        );
    }

    /// The other half of #204: passing the sign-in gate is not enough, because the POST itself
    /// then goes through `http::request_with`, which asks the PER-REQUEST gate. With no client
    /// installed that gate failed closed, so the request never reached the wire and the form said
    /// "couldn't reach the server" against a server that was answering. The sign-in exchange
    /// runs under a `SigninAnchor` for the client's own origin, and only that origin passes.
    #[test]
    fn the_signin_request_itself_reaches_the_lan_literal_on_the_form() {
        let _guard = crate::testlock::serial();
        crate::jellyfin::uninstall();
        let store = crate::origin::CredentialPolicy::HttpsOnly;
        let path = "/Users/AuthenticateByName";
        let headers = ["Accept: application/json", "Authorization: MediaBrowser …"];
        let lan = crate::origin::Origin::parse("http://10.9.9.7:8096").unwrap();
        let neighbour = crate::origin::Origin::parse("http://10.9.9.8:8096").unwrap();
        assert!(
            !crate::http::credential_transport_allowed_by_policy(&lan, path, &headers, store),
            "outside a sign-in exchange the per-request gate still fails closed"
        );
        {
            let _anchor = crate::jellyfin::SigninAnchor::enter(&lan);
            assert!(
                crate::http::credential_transport_allowed_by_policy(&lan, path, &headers, store),
                "the sign-in POST must reach the address the form names"
            );
            assert!(
                !crate::http::credential_transport_allowed_by_policy(
                    &neighbour, path, &headers, store
                ),
                "…and only that address, not a neighbour on the same subnet"
            );
            let named = crate::origin::Origin::parse("http://jellyfin.local:8096").unwrap();
            let _inner = crate::jellyfin::SigninAnchor::enter(&named);
            assert!(
                !crate::http::credential_transport_allowed_by_policy(
                    &named, path, &headers, store
                ),
                "a hostname is never the carve-out, even as the anchor"
            );
        }
        assert!(
            !crate::http::credential_transport_allowed_by_policy(&lan, path, &headers, store),
            "dropping the anchor closes the crossing again"
        );
    }

    #[test]
    fn a_stale_outcome_is_discarded() {
        // MUTATES the shared `EPOCH`/`MAIL` globals, so it holds the lock `serial()` guards
        // them with — the parse test beside it reads `EPOCH` and asserts it did not move, and a
        // store here without the lock lands between that read and its assertion.
        let _guard = crate::testlock::serial();
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
