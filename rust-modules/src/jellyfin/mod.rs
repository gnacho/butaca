//! jellyfin/ — the Jellyfin data layer (typed Jellyfin server client)
//!
//! This is the **only backend** of the application: one typed HTTP client
//! ([`client::JfClient`]), the serde DTOs it parses ([`dto`]), and the converters that turn
//! those DTOs into the app's own catalog rows ([`convert`]).
//!
//! ## The adapter principle: the UI's structs are the contract
//!
//! The screens do not read this module's DTOs. They read the SAME internal structs they always
//! have — `catalog::CatalogMovie` for grids and shelves, `metadata::Detail` for the detail page — because
//! those structs are already backend-neutral in everything but name: a title, a year, a duration,
//! a resume point, an image path, a watched flag. What is backend-specific is how those fields
//! are FETCHED and PARSED, and that is the only thing this module replaces. A Jellyfin
//! `BaseItemDto` goes in one side of [`convert::movie_from_dto`] and a `CatalogMovie` comes out of
//! the other, indistinguishable from the row every screen already draws.
//!
//! That choice is deliberate, and the alternative was considered and rejected: introducing a
//! `trait MediaBackend` and migrating the UI onto it would have touched ~20 UI files before a
//! single Jellyfin byte could be validated on hardware. The adapter validates the backend FIRST;
//! a trait, if one is ever wanted, can name itself over code that is known to work.
//!
//! ## What "a server" means here
//!
//! The retired wire had a cloud account: servers were discovered through the account service,
//! and a shared server arrived with its own token. Jellyfin has none of that: a server is a URL
//! the user types, and the login is that server's own user and password. The PoC therefore
//! registers exactly ONE Jellyfin server, in the registry's slot model ([`SERVER_ID`]), and the
//! onboarding asks for address + username + password instead of showing a QR code. The
//! multi-source machinery (`browse`'s `(sid, section)` addressing, `posters`'s per-server keying)
//! is flavor-blind and works unchanged with one Jellyfin source in it.
//!
//! ## The credential-transport rule this backend lives under
//!
//! The retired wire's token could ride only TLS, because every server was reachable through a
//! public wildcard certificate. Jellyfin has no equivalent: a home Jellyfin server is plain HTTP
//! on a LAN address in the overwhelming majority of installs, so `crate::http`'s credential gate
//! carries a Jellyfin-only relaxation — its credential shapes (the
//! `Authorization: MediaBrowser` header, and the `api_key=` query that media/image/stream
//! URLs still carry) may cross plaintext ONLY to the address the credential was issued for: the
//! origin of the installed client, the one signed in against (and itself a literal that cannot
//! route off the LAN, `crate::http::is_lan_host`). Another LAN address is refused like anywhere
//! else, no installed client means no relaxation at all outside the two sign-in attempts, and a
//! server that moves comes back through sign-in, which re-anchors the address. Those two attempts
//! are the one exception, and they have to be: they are what INSTALLS the anchor the rule measures
//! against, so `crate::http::credential_transport_allowed_at_signin` dials the private literal on
//! the form rather than refusing the request that would have created the anchor. That is a
//! considered weakening of the transport floor for this backend alone, and it is why the
//! onboarding asks for an IP literal rather than a hostname.
//!
//! ## Units, spelled out once
//!
//! Jellyfin measures times in **ticks of 100 ns** (`RunTimeTicks`, `PlaybackPositionTicks`); the
//! app measures durations in NANOSECONDS and resume points in MILLISECONDS (the retired units,
//! baked into `CatalogMovie`). The conversions live in [`convert`]: multiply ticks by 100 for `dur_ns`
//! and divide by 10_000 for `resume_ms`, and nowhere else. A tick value must never reach a struct
//! field unconverted.

// PoC scaffolding: person pages and parts of the DTO surface are still unwired, so nothing
// outside the module consumes them yet, and warnings-are-errors would refuse the crate.
// Module-wide and TEMPORARY on purpose — the last consumer to land removes this attribute.
#![allow(dead_code, unused_imports)]

#[cfg(test)]
pub(crate) mod mock_server;
#[cfg(test)]
pub(crate) use mock_server::MockServer;

pub(crate) mod boot;
pub(crate) mod browse;
pub(crate) mod client;
mod blurhash;
mod convert;
pub(crate) mod detail;
pub(crate) mod discovery;
pub(crate) mod identify;
mod dto;
pub(crate) mod profile;
pub(crate) mod search;
pub(crate) mod quick_connect;
pub(crate) mod signin;

pub(crate) use client::{AuthError, AuthOk, JfClient};
pub(crate) use convert::{boxset_from_dto, movie_from_dto};
pub(crate) use dto::{BaseItemDto, ItemsResult, ViewDto};

use crate::registry::ServerId;

/// The registry slot the PoC's one Jellyfin server answers to. Jellyfin item ids are GUIDs and
/// cannot collide with an integer ratingKey the way two servers' keys do, but the row identity
/// rule is kept anyway: `(sid, rk)` stays the address of a catalog row, so a future
/// second Jellyfin server slots in without a structural change.
pub(crate) const SERVER_ID: ServerId = ServerId::from_raw(0);

/// `RwLock<Option<&'static …>>` rather than a `OnceLock`: sign-out must be able to RETIRE the
/// client mid-run (a OnceLock cannot), and the reference is leaked for the registry's own
/// reason — a worker that captured it mid-request keeps a live client however the sign-out
/// lands. The lock is the only write path; `client()` is a read.
static CLIENT: std::sync::RwLock<Option<&'static JfClient>> = std::sync::RwLock::new(None);

#[cfg(test)]
thread_local! {
    /// Retires the client when the installing test's thread ends. Every test runs on its own
    /// thread, so this covers both the ordinary end and a panic: a test that installs a synthetic
    /// client and never reaches its teardown cannot leave that client grading the next test's
    /// `Session::account`, which reads this global and prefers the signed-in name over the
    /// session's own roster.
    static RETIRE_ON_THREAD_EXIT: RetireOnThreadExit = const { RetireOnThreadExit };
    /// The client THIS thread installed last. The teardown retires only when the installed client
    /// is still that one: a test thread that ends while a later test has already installed its own
    /// must not uninstall somebody else's client.
    static INSTALLED_HERE: std::cell::Cell<Option<*const JfClient>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
struct RetireOnThreadExit;

#[cfg(test)]
impl Drop for RetireOnThreadExit {
    fn drop(&mut self) {
        let _ = INSTALLED_HERE.try_with(|mine| {
            if let Some(mine) = mine.get() {
                retire_if_current(mine);
            }
        });
    }
}

/// Clear the installed client only if it is still `mine`, comparing and clearing under one write
/// lock: a check followed by a separate [`uninstall`] would let a concurrent install slip in
/// between and be retired by a thread that never installed it.
#[cfg(test)]
fn retire_if_current(mine: *const JfClient) {
    let mut slot = CLIENT.write().unwrap_or_else(|e| e.into_inner());
    if slot.is_some_and(|current| std::ptr::eq(current, mine)) {
        *slot = None;
    }
}

/// Install the authenticated client. Called by the boot flow and by the sign-in form's worker,
/// after [`JfClient::authenticate_by_name`] has succeeded. A SECOND install (sign out → a
/// different server) replaces the first — the leaked predecessor stays alive for any worker
/// still holding it and is simply never handed out again.
pub(crate) fn install(client: JfClient) {
    #[cfg(test)]
    {
        crate::testlock::assert_held("the installed Jellyfin client");
        RETIRE_ON_THREAD_EXIT.with(|_| {});
    }
    let leaked: &'static JfClient = Box::leak(Box::new(client));
    #[cfg(test)]
    INSTALLED_HERE.with(|mine| mine.set(Some(leaked as *const JfClient)));
    *CLIENT.write().unwrap_or_else(|e| e.into_inner()) = Some(leaked);
}

/// Retire the installed client — the sign-out path. After this, every `sid == SERVER_ID` guard
/// in the stores fails closed until the next install.
pub(crate) fn uninstall() {
    *CLIENT.write().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The installed client, `None` until the boot flow authenticates (and again after sign-out).
/// The host half of the Jellyfin plaintext relaxation — [`crate::http`]'s credential gate
/// calls this from its `Credential::Jellyfin` arm. Scope: plaintext credential shapes may cross
/// ONLY to the address the credential was issued for, the origin of the installed client (the
/// one signed in against, the address the persisted config carries). Another LAN address is
/// denied even a hop away on the same subnet; with no client installed the answer is no (fail
/// closed), and the sign-in attempt that would create that anchor does not ask this function at
/// all — it asks [`crate::http::credential_transport_allowed_at_signin`], the one crossing this
/// rule cannot grade without a deadlock; and the address itself must still be a literal that
/// cannot route off the LAN.
pub(crate) fn plaintext_host_allowed(origin: &crate::origin::Origin) -> bool {
    let installed_match = client().is_some_and(|installed| {
        origin.host() == installed.origin().host()
            && crate::http::is_lan_host(installed.origin().host())
    });
    installed_match || signin_anchor_matches(origin)
}

thread_local! {
    /// The host a sign-in attempt on THIS thread is creating the anchor for. The sign-in gate
    /// (`http::credential_transport_allowed_at_signin`) already admits that crossing up front,
    /// but the request itself then went through the per-request gate, which (with no client
    /// installed yet) failed closed — so a LAN sign-in never left the device and was reported as
    /// "unreachable". The anchor carries the sign-in gate's decision down to that request, and
    /// only for the client's own origin, only while the guard is alive.
    static SIGNIN_ANCHOR: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

fn signin_anchor_matches(origin: &crate::origin::Origin) -> bool {
    SIGNIN_ANCHOR.with(|a| {
        a.borrow()
            .as_deref()
            .is_some_and(|host| host == origin.host() && crate::http::is_lan_host(host))
    })
}

/// RAII scope for one sign-in exchange (password, Quick Connect, or stored-token probe) against
/// `origin`. While alive, plaintext Jellyfin credentials may reach exactly that LAN literal from
/// this thread. Dropping it restores whatever anchor was there before.
pub(crate) struct SigninAnchor {
    prev: Option<String>,
}

impl SigninAnchor {
    pub(crate) fn enter(origin: &crate::origin::Origin) -> SigninAnchor {
        let host = origin.host().to_string();
        let prev = SIGNIN_ANCHOR.with(|a| a.borrow_mut().replace(host));
        SigninAnchor { prev }
    }
}

impl Drop for SigninAnchor {
    fn drop(&mut self) {
        let prev = self.prev.take();
        SIGNIN_ANCHOR.with(|a| *a.borrow_mut() = prev);
    }
}

pub(crate) fn client() -> Option<&'static JfClient> {
    *CLIENT.read().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod plaintext_scope_tests {
    use super::*;

    /// The relaxation is an allow-list of ONE address, not of the LAN: the signed-in host passes,
    /// a neighbour LAN address and a hostname are refused, and with the client retired the whole
    /// thing fails closed.
    #[test]
    fn the_lan_relaxation_reaches_only_the_signed_in_address() {
        let _serial = crate::testlock::serial();
        install(JfClient::new(
            crate::origin::Origin::http("192.168.1.20", 8096),
            "dev-1".into(),
        ));
        let registered = crate::origin::Origin::http("192.168.1.20", 8096);
        let neighbour = crate::origin::Origin::http("192.168.1.99", 8096);
        let named = crate::origin::Origin::parse("http://jellyfin.local:8096").expect("parse");
        assert!(plaintext_host_allowed(&registered));
        assert!(!plaintext_host_allowed(&neighbour));
        assert!(!plaintext_host_allowed(&named), "a hostname is never the carve-out");
        uninstall();
        assert!(
            !plaintext_host_allowed(&registered),
            "no installed client, no relaxation: fail closed"
        );
    }
}
