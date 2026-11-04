//! **May a credential go to this origin — the ONE answer, and the plaintext grants behind it.**
//!
//! Every layer that puts a Plex token on a request asks [`credential_allowed`] (or, in a pure
//! function that receives the build's policy as a parameter, [`allowed_under`]): the registry
//! when it admits an origin, endpoint admission, the control-plane transport (`crate::http`), the
//! media plane (`crate::curlio`, `crate::stream_redirect`'s plaintext hops, the player's stream
//! start). Nothing else re-derives the rule, and nothing asks `Origin::is_tls` for a credential
//! decision on its own.
//!
//! ## The rule
//!
//! A TLS origin may always carry a credential. A plaintext origin may when the BUILD allows it
//! ([`CredentialPolicy::AllowPlaintext`], developer builds only — [`CredentialPolicy::build`] is
//! still the only `cfg!` for this), or when a live [`PlaintextGrant`] names that exact origin. A
//! store build therefore sends a token over plaintext to exactly the origins the person consented
//! to, on the network and under the identity they consented on — and to nothing else.
//!
//! ## What a grant is bound to
//!
//! A grant is minted only by discovery ([`mint`]), from a FRESH verdict — plex.tv's resources and
//! a tokenless `/identity` answer — that [`InsecureEvidence::plaintext_eligibility`] calls
//! eligible, and only when the person's recorded consent for that (account, server) allows it.
//! Consent is the ACCOUNT's ([`account_key`]): an answer is honoured only while the plex.tv
//! account that gave it is the one signed in, so a different account signing in on the same
//! television is asked again.
//! It names:
//!
//! * the server's `machineIdentifier` — a grant admits THAT server's credential ([`allowed_for`]),
//!   never another machine's that happens to register at the same address;
//! * the exact plaintext origin — scheme, NUMERIC private host and port — that verified; a name is
//!   never granted, because a resolver could answer it with anything;
//! * the **identity generation** it was minted under ([`identity_changed`]: sign-out, a sign-in
//!   starting);
//! * the **network generation** it was minted under ([`network_changed`]: the app returning to the
//!   foreground, where nothing proves the television is still on the network it consented on).
//!   Every server that loses its grant there is queued for a fresh endpoint discovery, which
//!   re-proves the network and, the consent being persisted, mints again without asking.
//!
//! A grant whose generations are not the current ones is dead: it admits nothing, and the table
//! drops it. Nothing about a grant is persisted — a session file that names a plaintext origin
//! reactivates nothing by itself (`server::session::SourceRef`); the next discovery re-proves
//! eligibility and mints a fresh one, or the origin stays tokenless.
//!
//! ## Revocation
//!
//! [`revoke`], a refusing [`answer`], [`identity_changed`] and [`network_changed`] change the
//! table at once, and the authority is asked when a request STARTS: every request that starts after
//! the change — queued, retried, a cached token-bearing URL, a media reopen — is refused at the
//! transport, and the registry re-grades every published client
//! ([`crate::registry::regrade_credentials`]), blanking the token of any whose origin may no
//! longer carry one. A request already on the wire is not interrupted: it completes (or fails)
//! with the credential it started with, and nothing after it gets a new one. A refusal also moves
//! the consent generation, so a discovery that captured *Allowed* before it cannot mint the grant
//! back when it settles; and a refusal whose write has not reached the disk is written again
//! until it has ([`UpgradeRetry::due`]), so a restart never reads back the *Allowed* it replaced —
//! for as long as the account that refused is signed in: [`identity_changed`] drops it, and a
//! write never lands in a session file that belongs to another account.
//!
//! A grant also lives only as long as a FRESH verdict keeps reaching its server: discovery
//! withdraws it when the latest probe of that server did not reach the granted origin
//! (`auth::publish_settled_probe`). Nothing here watches the network itself — the platform
//! offers this app no network-change signal it subscribes to — so the foreground return is the
//! one continuity break treated as a new network.
//!
//! ## Offers
//!
//! The table also holds what may be ASKED: a fresh eligible insecure-only verdict under the
//! current generations ([`offered`]). The consent surfaces — the sign-in read-out, Home's and the
//! Library's failure read-out, Settings' *Unencrypted connections* — read [`offers`]; an offer is
//! never a grant, and dies with the generations exactly as a grant does.
//!
//! ## Backend boundary
//!
//! [`credential_allowed`] and [`allowed_under`] are the backend-neutral transport authority used
//! by both Plex and Jellyfin. The consent lifecycle below is still Plex-specific: discovery
//! evidence comes from `crate::server::probe`, consent persistence uses `crate::server::session`, and
//! registry invalidation goes through `crate::plex`. [`UpgradeRetry`] also shares `crate::pms`'s
//! backoff. Those dependencies remain unchanged until B2 rewires the Plex lifecycle.
//! `Session::owner_identity` stays in Plex session storage.
#![allow(dead_code)]
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::origin::{CredentialPolicy, Origin};
use crate::server::probe::PlaintextEligibility;
use crate::server::session::{PlaintextChoice, Session};

/// The generations a grant is bound to — captured by a discovery worker when it starts, so a grant
/// it mints after the identity or the network moved is dead on arrival.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GrantScope {
    identity: u64,
    network: u64,
}

/// One consented plaintext origin. Never persisted; see the module doc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlaintextGrant {
    scope: GrantScope,
    machine_id: String,
    origin: Origin,
}

static GRANTS: Mutex<Vec<PlaintextGrant>> = Mutex::new(Vec::new());
/// How many grants the table holds — the lock-free fast path: a household that never consented
/// (every household but the few this exists for) answers every request with one relaxed load.
static COUNT: AtomicUsize = AtomicUsize::new(0);
static IDENTITY: AtomicU64 = AtomicU64::new(1);
static NETWORK: AtomicU64 = AtomicU64::new(1);
/// Bumped by every answer that does not allow (a *Not now*, a switch turned off), under the grant
/// table's lock and before the grant goes. A [`PlaintextAsk`] records it at capture, so a discovery
/// already in flight when the person said no finds its capture stale and mints nothing back.
static CONSENT: AtomicU64 = AtomicU64::new(1);
/// Answers given THIS launch, under the current identity, overlaying the persisted ones
/// (`Session::plaintext_consent`) — so a *Connect* is honoured by the retry it triggers even
/// before (or without) the session write landing. Cleared with the identity; never a grant.
/// Refusals recorded this launch whose write has not yet been confirmed on disk — see [`record`].
/// Cleared with the identity ([`identity_changed`]).
/// The fresh eligible verdicts that may be asked about; see the module doc's *Offers*.
static OFFERS: Mutex<Vec<(GrantScope, PlaintextVerdict)>> = Mutex::new(Vec::new());
/// Bumped whenever anything a consent surface or the upgrade retry reads moves — the grant table,
/// the generations, the offers, this launch's answers — so a reader polling it every frame takes
/// no lock until something did.
static REVISION: AtomicU64 = AtomicU64::new(1);

fn moved() {
    #[cfg(test)]
    crate::testlock::assert_held("the plaintext grant table (revision)");
    REVISION.fetch_add(1, Ordering::AcqRel);
}

/// The current revision of everything this module publishes; see [`REVISION`].
pub(crate) fn revision() -> u64 {
    REVISION.load(Ordering::Acquire)
}

/// **The account half of a consent's key** - the roster admin's stable plex.tv account id, so a
/// fresh token for the same owner keeps every profile's answer. Sessions written before that id
/// was persisted fall back to the legacy token fingerprint. An empty key matches nothing.
pub(crate) fn account_key(session: &Session) -> String {
    account_key_for(session.owner_identity(), &session.account_token)
}

fn account_key_for(owner: Option<i64>, account_token: &str) -> String {
    if account_token.is_empty() {
        return String::new();
    }
    owner.map_or_else(|| legacy_account_key(account_token), |id| format!("plex-tv:{id}"))
}

/// The key written by older builds: the first 64 bits of SHA-256 over a domain tag and the token,
/// hex. It is also the migration proof: only a row matching the current token may be adopted.
fn legacy_account_key(account_token: &str) -> String {
    if account_token.is_empty() {
        return String::new();
    }
    let mut input = b"butaca-consent:".to_vec();
    input.extend_from_slice(account_token.as_bytes());
    crate::sha256::sha256(&input)[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// **One insecure-only server, as a read-out needs it** — which server, whose it is, whether the
/// same-network rule lets the person be asked ([`InsecureEvidence::plaintext_eligibility`]), and
/// what they already answered. The name and the credit are for the SCREEN only: the report carries
/// the closed eligibility and consent codes, never these strings (`PRIVACY.md`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PlaintextVerdict {
    pub machine_id: String,
    pub name: String,
    /// Whom to name for a SHARED server (`auth::credit_of`); empty for the household's own.
    pub shared_by: String,
    pub eligibility: PlaintextEligibility,
    pub choice: PlaintextChoice,
}

impl PlaintextVerdict {
    /// The person may be asked "Connect without encryption?" about this server.
    pub(crate) fn offers(&self) -> bool {
        self.eligibility == PlaintextEligibility::Eligible
    }

    /// Of two insecure-only servers, the one the read-out should speak about: the first that can
    /// be offered, else the first at all. A household's own non-eligible server does not hide a
    /// share the person could connect to, and the order stays plex.tv's (ours first).
    pub(crate) fn prefer<E>(held: Option<(E, PlaintextVerdict)>, next: (E, PlaintextVerdict))
        -> Option<(E, PlaintextVerdict)> {
        match held {
            Some(held) if held.1.offers() || !next.1.offers() => Some(held),
            _ => Some(next),
        }
    }
}

/// The generations a grant minted NOW would be bound to.
pub(crate) fn scope() -> GrantScope {
    GrantScope {
        identity: IDENTITY.load(Ordering::Acquire),
        network: NETWORK.load(Ordering::Acquire),
    }
}

/// **May a credential go to `origin` — the live answer**, under the build's own policy, for a
/// request that holds a URL rather than a server: the transport (`crate::http`, `crate::curlio`,
/// `crate::stream_redirect`). The token on such a request was put there by a registration that
/// asked [`allowed_for`], so here a grant admits its origin whichever server it was minted for.
pub fn credential_allowed(origin: &Origin) -> bool {
    allowed_under(CredentialPolicy::build(), origin)
}

/// [`credential_allowed`] with the policy supplied — for the pure functions that receive the
/// build's policy as a parameter, and their tests. The grant half is always the live table.
pub(crate) fn allowed_under(policy: CredentialPolicy, origin: &Origin) -> bool {
    policy.may_carry_credential(origin) || (!origin.is_tls() && granted(None, origin))
}

/// **May `machine_id`'s credential go to `origin`** — the question for whoever is about to put a
/// server's own token on an origin: registry admission (`crate::plex`), endpoint admission
/// (`crate::server::admit_source_until`). A grant admits only the server it was minted for, so another
/// machine whose address collides with a granted origin (a DHCP reshuffle, a share advertising
/// the same private address) never gets its token sent there. An empty `machine_id` names no
/// server, and no grant admits it.
pub(crate) fn allowed_for(policy: CredentialPolicy, machine_id: &str, origin: &Origin) -> bool {
    policy.may_carry_credential(origin) || granted_now(machine_id, origin)
}

/// [`allowed_for`] under the build's own policy.
pub(crate) fn credential_allowed_for(machine_id: &str, origin: &Origin) -> bool {
    allowed_for(CredentialPolicy::build(), machine_id, origin)
}

/// **May a REMEMBERED origin carry a credential** — an address read back from the session file
/// (`auth`'s cached-source probe), not one a fresh verdict just reached. The build's policy alone:
/// a grant is re-proved by discovery's own race, which already dials the granted origin, so a
/// cached plaintext origin standing in for a fresh probe that did NOT reach it (HTTPS now refuses,
/// the server now requires HTTPS) would override exactly the verdict that should withdraw it.
pub(crate) fn remembered_allowed(policy: CredentialPolicy, origin: &Origin) -> bool {
    policy.may_carry_credential(origin)
}

/// Does `machine_id`'s credential at `origin` rest on a grant — `policy` alone refuses it and a
/// live grant admits it? The registry records this per slot so a revocation re-grades exactly the
/// slots a grant was carrying, whatever the build's own policy.
pub(crate) fn rests_on_grant(policy: CredentialPolicy, machine_id: &str, origin: &Origin) -> bool {
    !policy.may_carry_credential(origin) && granted_now(machine_id, origin)
}

/// Is `machine_id` at `origin` admitted by a live grant right now (the grant half of
/// [`allowed_for`] alone)?
pub(crate) fn granted_now(machine_id: &str, origin: &Origin) -> bool {
    !machine_id.is_empty() && !origin.is_tls() && granted(Some(machine_id), origin)
}

fn granted(machine_id: Option<&str>, origin: &Origin) -> bool {
    if COUNT.load(Ordering::Acquire) == 0 {
        return false;
    }
    let grants = GRANTS.lock().unwrap_or_else(|e| e.into_inner());
    admits(&grants, scope(), machine_id, origin)
}

/// Pure: does any grant in `grants` admit `origin` at `now` — for `machine_id` when one is named?
/// Only a grant minted under the current generations counts, and only for its exact origin —
/// scheme, host and port.
fn admits(grants: &[PlaintextGrant], now: GrantScope, machine_id: Option<&str>, origin: &Origin) -> bool {
    grants.iter().any(|g| {
        g.scope == now && g.origin == *origin && machine_id.is_none_or(|m| g.machine_id == m)
    })
}

/// Why [`mint`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MintRefusal {
    /// The verdict is not eligible, with the rule's own reason.
    Ineligible(PlaintextEligibility),
    /// The origin is not plaintext — TLS needs no grant, and must never be recorded as one.
    NotPlaintext,
    /// The origin's host is not the numeric private literal the verdict classified.
    NotPrivateLiteral,
    /// The identity or the network moved since the minting worker started, or the person answered
    /// a refusal since it captured their consent.
    Stale,
    /// The person has not allowed plaintext for this server (undecided, declined or revoked).
    NotConsented,
}

/// The plaintext origin `machine_id` is credentialed at right now, if any — what Settings shows as
/// "Connected without encryption", and what the upgrade retry is trying to leave.
pub(crate) fn granted_origin(machine_id: &str) -> Option<Origin> {
    if COUNT.load(Ordering::Acquire) == 0 {
        return None;
    }
    let now = scope();
    let grants = GRANTS.lock().unwrap_or_else(|e| e.into_inner());
    grants.iter().find(|g| g.scope == now && g.machine_id == machine_id).map(|g| g.origin.clone())
}

/// **Revoke `machine_id`'s grant now** — consent withdrawn in Settings, or the server verified
/// over HTTPS and needs plaintext no more. Returns whether there was one.
pub(crate) fn revoke(machine_id: &str) -> bool {
    let removed = retain(|g| g.machine_id != machine_id);
    if removed {
        crate::log("security: plaintext credentials withdrawn for one server");
    }
    removed
}

/// The signed-in identity changed (sign-out, a sign-in starting): every grant and every offer is
/// dead from this instant.
///
/// This launch's answers go with it: a refusal is the account's that
/// gave it, and one written after this would land in whatever session follows (the next account's,
/// whose own answers `Session::with_plaintext_choice` would drop on the way). The row it was
/// overwriting is keyed to a sign-in that no longer exists, so it can never be honoured again.
/// The new identity discovers everything itself.
pub(crate) fn identity_changed() {
    #[cfg(test)]
    crate::testlock::assert_held("the plaintext grant table (identity_changed)");
    IDENTITY.fetch_add(1, Ordering::AcqRel);
    clear_offers();
    if retain(|_| false) {
        crate::log("security: plaintext credentials withdrawn — identity changed");
    }
    moved();
}

/// The television may be on a different network (the app returned to the foreground): every grant
/// and offer is dead from this instant. Each server that lost its grant is queued for a fresh
/// endpoint discovery ([`STRANDED`], requested by the next [`UpgradeRetry::due`]), which re-proves
/// the network and — the person's consent being persisted — mints again without asking.
pub(crate) fn network_changed() {
    #[cfg(test)]
    crate::testlock::assert_held("the plaintext grant table (network_changed)");
    NETWORK.fetch_add(1, Ordering::AcqRel);
    clear_offers();
    if retain(|_| false) {
        crate::log("security: plaintext credentials withdrawn — network continuity unknown");
    }
    moved();
}

/// **Offer `verdict`** — a fresh insecure-only verdict discovery settled under `scope`. An eligible
/// one replaces the server's previous offer; anything else (ineligible now, or captured under
/// generations that have since moved) withdraws it. See the module doc's *Offers*.
pub(crate) fn offered(scope: GrantScope, verdict: PlaintextVerdict) {
    let mut offers = OFFERS.lock().unwrap_or_else(|e| e.into_inner());
    offers.retain(|(_, v)| v.machine_id != verdict.machine_id);
    if verdict.offers() && scope == self::scope() {
        offers.push((scope, verdict));
    }
    drop(offers);
    moved();
}

/// The server is no longer insecure-only (it was reached, refused, or went silent): nothing to ask.
pub(crate) fn withdraw_offer(machine_id: &str) {
    let mut offers = OFFERS.lock().unwrap_or_else(|e| e.into_inner());
    let n = offers.len();
    offers.retain(|(_, v)| v.machine_id != machine_id);
    let changed = offers.len() != n;
    drop(offers);
    if changed {
        moved();
    }
}

fn clear_offers() {
    OFFERS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// The live offers, in the order discovery settled them.
pub(crate) fn offers() -> Vec<PlaintextVerdict> {
    let now = scope();
    OFFERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|(scope, _)| *scope == now)
        .map(|(_, v)| v.clone())
        .collect()
}

/// `machine_id`'s live offer, if it has one.
pub(crate) fn offer(machine_id: &str) -> Option<PlaintextVerdict> {
    offers().into_iter().find(|v| v.machine_id == machine_id)
}

/// Adopt matching token-stamped rows on their first read. The worker revalidates both identities
/// under the session IO lock, so a sign-in cannot redirect the migration. Once one write lands,
/// later reads find no legacy row and queue nothing.
pub(crate) fn adopt_legacy_consent(session: &Session) -> bool {
    let account = account_key(session);
    let legacy = legacy_account_key(&session.account_token);
    if account.is_empty()
        || account == legacy
        || !session.plaintext_consent.iter().any(|c| c.account == legacy)
    {
        return false;
    }
    crate::server::session::queue_update(move |current| {
        if account_key(current) != account || legacy_account_key(&current.account_token) != legacy {
            return None;
        }
        let mut next = current.clone();
        let mut changed = false;
        let mut consents = Vec::with_capacity(next.plaintext_consent.len());
        for mut consent in next.plaintext_consent.drain(..) {
            if consent.account == legacy {
                changed = true;
                if current.plaintext_consent.iter().any(|c|
                    c.account == account && c.machine_id == consent.machine_id)
                {
                    continue;
                }
                consent.account = account.clone();
            }
            consents.push(consent);
        }
        next.plaintext_consent = consents;
        changed.then_some(next)
    })
}

/// Keep only the grants `keep` accepts. Returns whether any went. (The registry re-grade that
/// used to follow went with the client slots: no published client exists to strip a token from,
/// #166.)
fn retain(keep: impl Fn(&PlaintextGrant) -> bool) -> bool {
    let mut grants = GRANTS.lock().unwrap_or_else(|e| e.into_inner());
    let before = grants.len();
    grants.retain(|g| keep(g));
    let removed = grants.len() != before;
    COUNT.store(grants.len(), Ordering::Release);
    drop(grants);
    if removed {
        moved();
    }
    removed
}

/// Empty the table (the generations keep counting). Under [`crate::testlock::serial`].
#[cfg(test)]
pub(crate) fn reset_for_test() {
    crate::testlock::assert_held("the plaintext grant table (reset)");
    let mut grants = GRANTS.lock().unwrap_or_else(|e| e.into_inner());
    grants.clear();
    COUNT.store(0, Ordering::Release);
    drop(grants);
    clear_offers();
    moved();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lan() -> Origin {
        Origin::http("192.168.0.10", 32400)
    }

    fn account_session(owner: Option<i64>, token: &str) -> Session {
        Session {
            account_token: token.into(),
            home_users: owner
                .into_iter()
                .map(|id| crate::server::session::HomeUserRef {
                    id,
                    admin: true,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

}
