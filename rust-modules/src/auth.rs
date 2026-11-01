//! Auth policy, immutable worker observations, and main-thread resource operations.
//!
//! The application's Bridge contains the concrete [`SessionMachine`] and a separate Session
//! resource adapter. Commands and addressed observations enter that owner; the UI borrows its
//! coherent publication. No module-global controller, epoch allocator or progress queue remains.
//! Workers receive captured inputs and an observation sink, never session/registry write authority.
//! Resource commits and Ready handoff are accepted on main with exact request/epoch/lifecycle
//! checks. Credential patches preserve newer disk preferences. Tokens are never logged.
//!
//! Stored Home, picker and explicit developer bootstrap use the same owner, with distinct typed
//! authority. Network/PIN derivation remain worker operations; offline policy is retained below.
use self::observation::Resource;
use crate::plex::probe::{self, Outcome, PlaintextEligibility};
use crate::plex::session::{self, PlaintextChoice, ProfileCreds, Session, SourceRef};
use crate::origin::Origin;
use crate::registry::ServerId;

pub(crate) mod owner;
pub(crate) mod observation;
pub(crate) use owner::{SessionInit, SessionMachine, SessionRead};

/// Application commands are the concrete owner's domain vocabulary, not global operations.
pub(crate) use owner::Command as SessionCmd;
pub(crate) use crate::grant::PlaintextVerdict;

/// Which stage the flow is in — the Login/Profiles screens switch on this each frame.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum Phase {
    /// Not in the login flow (offline / dev-token path handles startup instead).
    #[default]
    Idle,
    /// Credentials resolved — the main loop should install them and go Home.
    Ready,
    /// A step failed; show the message and allow a retry.
    Error,
    /// All local state was erased.
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum DiscoveryRetryRun { Resources, HomeUsers }

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct DiscoveryRetryProgress {
    pub run: DiscoveryRetryRun,
    pub misses: u32,
    pub elapsed_ms: u32,
}

/// **Which who's-watching picker is on screen** — the one fact [`cancel`] cannot work out for
/// itself, and the difference between an escape hatch and a privilege escalation.
///
/// It is ONE screen raised from THREE places, and BACK means something different on each. At BOOT
/// nobody has identified themselves this run: there is nothing behind the picker but the persisted
/// session, and reinstating that silently is exactly the thing a PIN is there to stop — so BACK
/// there resumes only an UNPROTECTED stored profile. The other two resume nothing at all, for two
/// different reasons. After a QR sign-in the standing person has proved they hold the ACCOUNT, but
/// an account credential is not a household PIN and no profile has been chosen yet. And *Change
/// profile* DETACHES what was behind it ([`detaches_active_profile`]), which is what makes its
/// picker a root — the paragraph that used to sit here said Home is behind it and backing out hands
/// the user what they were already holding, and that reasoned about the person who PRESSED the
/// control rather than the one now holding the remote.
///
/// Nothing in the state below could tell them apart (all three arrive at [`Phase::Profiles`] with
/// the same roster), so every raise site names its own kind. Boot/change-profile use the owner's
/// StartSwitch command; accepted QR completion selects SignedIn after its resource commit ACK.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum Picker {
    /// The boot gate's who's-watching, before any profile has been chosen this run.
    ///
    /// **The default, and deliberately the STRICT one.** Every picker names its own kind, so the
    /// default also covers Login's BACK before a picker is raised. "We cannot say who is asking"
    /// must not resolve to "hand
    /// over the credentials" — a permissive default is the shape of the bug this enum exists to
    /// fix, and it is what left the dev-only `/tmp/butaca-login` boot on the wrong side of it.
    #[default]
    Boot,
    /// Home's *Change profile*: a profile WAS active, and raising this picker detaches it.
    ///
    /// **The strictest of the three, despite being raised from the most authenticated place.** Home
    /// is no longer behind it, so there is nothing to back out to and BACK restores nothing at all
    /// — not even an unprotected previous profile. The whole argument is on
    /// [`detaches_active_profile`] and [`may_resume`].
    ChangeProfile,
    /// The picker the QR sign-in raises when the account turns out to have a Plex Home roster —
    /// accepted QR completion, not StartSwitch. Whoever is standing there completed a plex.tv sign-in
    /// seconds ago, but an account credential is not a household PIN and no profile has been chosen
    /// yet, so BACK resumes nothing here either — [`may_resume`].
    SignedIn,
}

/// One "who's watching" tile.
#[derive(Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UserTile {
    /// This member's plex.tv account id. Nothing on screen reads it — it rides through so that
    /// [`session::Session::household_ids`] is filled on the one path that writes the persisted
    /// roster, which is what lets the "Shared by …" rule tell the household's own server from a
    /// friend's share (`plex::servers::owner_credit`).
    pub id: i64,
    pub title: String,
    pub thumb: String,
    pub uuid: String,
    pub protected: bool, // needs a PIN
    pub admin: bool,
}
/// PMS credentials the main loop installs once the flow resolves.
pub struct ReadyCreds {
    pub(crate) install: owner::ReadyInstall,
    /// **Where the primary server is** — an [`Origin`], not a `(host, port)` pair, because the
    /// pair cannot say `https` and the host a certificate is issued for is not the address behind
    /// it (`plex::origin`). Read straight off the stored [`session::ServerRef`], which is the
    /// value discovery wrote and the one `can_go_local` gates.
    pub origin: Origin,
    /// The advertised address BEHIND `origin` — a dotted quad or v6 literal, never a
    /// `plex.direct` hostname. `plex::IpVersion::of_host` can classify this even when `origin`'s
    /// own host is a certificate NAME it cannot parse as an address (#95 step 8 / R3(a)).
    pub address: String,
    pub token: String,
    /// The tier that won discovery, restored only after the main thread installs/re-points the
    /// client because a fresh client deliberately starts with an unknown link.
    pub tier: Option<probe::Location>,
    /// The origin's resolve pin, read off the same stored record ([`session::ServerRef::resolve_pin`])
    /// so the install this hands off dials the LAN name with no resolver, exactly as the boot
    /// gate does.
    pub pin: Option<crate::origin::ResolvePin>,
}

/// Append a line to the shared on-device event log (never a token — only ids/counts/status).

/// Every seating of a PIN-free profile also writes its cache record, whichever path seated it —
/// the switch worker writes protected ones because only it holds a PIN to verify against, but an
/// unprotected profile's record is the session itself, so a stored sign-in that predates the cache
/// becomes seatable offline the first time it is used, with no online switch required.
fn remember_unprotected_active(sess: &mut Session) {
    // An existing record (protected or not) follows the session; only a MISSING record for an
    // unprotected profile is created here.
    sess.refresh_profile_record();
    if sess.user.uuid.is_empty()
        || sess.pms_token().is_empty()
        || sess.active_profile_is_protected()
        || sess.cached_profile(&sess.user.uuid).is_some()
    {
        return;
    }
    sess.remember_profile(ProfileCreds {
        uuid: sess.user.uuid.clone(),
        user: sess.user.clone(),
        server: sess.server.clone(),
        sources: sess.sources.clone(),
        pin: None,
        extensions: Default::default(),
    });
}

/// The credential identity a worker captured at its spawn. It is validation only: accepted
/// observations patch the latest controller/disk value field-by-field and never write this stale
/// snapshot back over preferences that changed while the request was in flight.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SessionIdentity {
    client_id: String,
    account_token: String,
    profile_uuid: String,
}

impl SessionIdentity {
    #[cfg(test)]
    pub(crate) fn of(s: &Session) -> Self {
        Self {
            client_id: s.client_id.clone(),
            account_token: s.account_token.clone(),
            profile_uuid: s.user.uuid.clone(),
        }
    }

}

/// One candidate activation observed by a probe coordinator. This carries the exact origin,
/// credential and link facts the old worker-side `activate_candidate` call used; applying them is
/// delayed until the main thread accepts the epoch/session identity.
///
/// Canon-only since the plex.tv roster workers were removed: the observation vocabulary and its
/// canonical digest are frozen (SessionInit::write layout), so the type and its serialization
/// survive with no producer.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct CandidateActivation {
    machine_id: String,
    token: String,
    name: String,
    credit: String,
    owned: bool,
    /// The grant EVIDENCE beside the credit — plex.tv's `home` and `ownerId`, carried so the
    /// registry slot this activation publishes can answer "is this our household's server?"
    /// rather than only "does this account own it?". Defaulted on a legacy observation for the
    /// same reason, and with the same self-correction, as `SourceRef::owner_id` documents.
    #[serde(default)]
    home: bool,
    #[serde(default)]
    owner_id: i64,
    #[serde(with = "observation::origin")]
    origin: Origin,
    address: String,
    location: probe::Location,
    ipv6: bool,
}

/// Canon-only since the plex.tv roster workers were removed: the observation vocabulary and its
/// canonical digest are frozen (SessionInit::write layout), so the type and its serialization
/// survive with no producer.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum RegistryProgress {
    Activate {
        epoch: u64,
        expected: Option<SessionIdentity>,
        candidate: CandidateActivation,
    },
    Settled {
        epoch: u64,
        expected: Option<SessionIdentity>,
        probe: SettledProbe,
    },
    Install {
        epoch: u64,
        expected: Option<SessionIdentity>,
        sources: Vec<SourceRef>,
        primary: Option<usize>,
    },
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct ServerRosterProgress {
    epoch: u64,
    expected: SessionIdentity,
    outcome: ServerRosterOutcome,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum ServerRosterOutcome {
    Unreachable,
    /// Nothing was found to REGISTER, but at least the probes themselves ran (R2/A5) — `settled`
    /// carries every one, empty only when there was truly nothing to probe (no server named at
    /// all). The owner commits these as registry-only [`RegistryPlan::Probe`]s: the roster itself
    /// is unchanged, so there is nothing to write to disk.
    NoReachable { settled: Vec<SettledProbe> },
    Reconcile {
        #[serde(with = "observation::resources")]
        resources: Vec<Resource>,
        found: Vec<SourceRef>,
        /// Machine in `found` whose profile grant passed authenticated admission.
        admitted_machine_id: String,
        household: Vec<i64>,
        settled: Vec<SettledProbe>,
    },
}

/// Canon-only (no producer): the observation vocabulary is frozen.
#[allow(dead_code)]
pub(crate) struct EndpointProgress {
    epoch: u64,
    expected: SessionIdentity,
    id: ServerId,
    machine_id: String,
    lifecycle: Option<ClientLifecycle>,
    fresh: Option<SourceRef>,
    probe: Option<SettledProbe>,
}

/// The exact registry incarnation an endpoint request was issued through. `ServerId` and
/// `machine_id` survive a re-point and a profile retoken, so neither can prove that a late route
/// result still belongs to the client/token that launched it.
#[derive(Clone, Copy)]
/// Canon-only (no producer): Pending.lifecycle stays part of the frozen owner canon.
#[allow(dead_code)]
pub(crate) struct ClientLifecycle {
    client: &'static crate::plex::Client,
    token_gen: u32,
}

#[allow(dead_code)]
impl ClientLifecycle {
    pub(crate) fn machine_id(self) -> &'static str { self.client.machine_id() }

    pub(crate) fn capture(client: &'static crate::plex::Client) -> Self {
        Self { client, token_gen: client.token_gen() }
    }
    pub(crate) fn logical(self, sid: u16) -> owner::ServerLifecycle {
        owner::ServerLifecycle { sid, instance_gen: self.client.instance_gen(), token_gen: self.token_gen }
    }
    pub(crate) fn is_current(self, expected: owner::ServerLifecycle) -> bool {
        self.logical(expected.sid) == expected && crate::registry::commit_if_current(
            ServerId::from_raw(expected.sid), self.client, self.token_gen, || ()).is_some()
    }
}

/// Native registry effects executed only by the Session resource adapter, after its borrowed
/// owner permit and (for an endpoint) exact captured Client lifecycle have been validated.
pub(crate) fn execute_session_registry(plan: &owner::RegistryPlan, client_id: &str) -> bool {
    match plan {
        owner::RegistryPlan::DevInstall { primary, extras, client_id } => {
            install_captured_registry(&primary.origin(), &primary.address, &primary.token,
                primary.tier, primary.resolve_pin().as_ref(), extras, Some(client_id));
        }
        owner::RegistryPlan::Install { sources, primary, replace } => {
            if *replace {
                // The profile's identity changes HERE, at the commit, with the tokens: grants
                // survive only for the exact origins this roster installs (`grant`).
                let installing: Vec<(String, Origin)> = sources.iter()
                    .filter_map(|s| s.origin().map(|origin| (s.machine_id.clone(), origin)))
                    .collect();
                crate::grant::roster_replaced(&installing);
                crate::registry::revoke_for_profile_switch();
            }
            let installed = install_roster(sources, *primary, client_id);
            if *replace { crate::registry::finish_profile_switch(&installed); }
            for source in sources {
                if let Some(origin) = source.origin() { retire_grant_on_https(&source.machine_id, &origin); }
            }
        }
    }
    true
}

/// **The HTTPS upgrade lands.** A server on a plaintext grant (`crate::grant`) whose
/// endpoint just registered at a TLS origin needs plaintext no more: the grant is withdrawn, so
/// nothing — a queued request, a cached URL, a reconnect — can put the credential back on the old
/// origin. Called after the registration, outside the registry's write lock, which the grant's
/// re-grade takes.
fn retire_grant_on_https(machine_id: &str, origin: &Origin) {
    if origin.is_tls() && crate::grant::granted_origin(machine_id).is_some() {
        crate::grant::revoke(machine_id);
        crate::log("security: server verified over HTTPS — plaintext upgrade complete");
    }
}

/// Shared resource installer. Dev boot supplies its captured device identity so registration
/// cannot mint/read a session file; legacy boot callers can use the already-loaded session cache.
///
/// `address` is the advertised address BEHIND `origin` (#95 step 8 / R3(a)): `origin.host()` is
/// usually a `plex.direct` certificate NAME that `IpVersion::of_host` cannot parse, so deriving
/// the IP family from it silently produced `None` on every real boot. The stored/advertised
/// address is the one value that is actually a literal.
pub(crate) fn install_captured_registry(origin: &Origin, address: &str, token: &str,
    tier: Option<probe::Location>, pin: Option<&crate::origin::ResolvePin>, extras: &[SourceRef],
    client_id: Option<&str>) {
    // Applies `connection` atomically, inside the same registration write, on whichever branch
    // runs (#95 step 8) — the primary and every extra used to register first and set the
    // connection facts in a SEPARATE call afterward, which is exactly the gap `ConnectionFacts`
    // exists to close.
    let register = |machine: &str, origin: &Origin, token: &str,
        pin: Option<&crate::origin::ResolvePin>, connection: crate::registry::ConnectionFacts| {
        if let Some(cid) = client_id {
            crate::registry::register_captured_origin_with_connection(machine, origin, token, pin, cid,
                connection)
        } else {
            register_observed_origin(machine, origin, token, pin, connection, &session::peek().client_id)
        }
    };
    let primary_connection =
        crate::registry::ConnectionFacts::new(tier, crate::plex::IpVersion::of_host(address));
    let id = register("", origin, token, pin, primary_connection);
    crate::registry::set_current(id);
    for source in extras {
        let Some(origin) = source.origin() else { continue };
        if source.token.is_empty() { continue; }
        let connection = crate::registry::ConnectionFacts::new(
            source.tier,
            crate::plex::IpVersion::of_host(&source.address),
        );
        let id = register(&source.machine_id, &origin, &source.token,
            source.resolve_pin().as_ref(), connection);
        crate::registry::describe(id, &source.name, &source.shared_by, grant_of(source));
    }
}

/// Register with the identity captured by the owner, including before a fresh login is durable.
fn register_observed_origin(
    machine_id: &str,
    origin: &Origin,
    token: &str,
    pin: Option<&crate::origin::ResolvePin>,
    connection: crate::registry::ConnectionFacts,
    client_id: &str,
) -> ServerId {
    crate::registry::register_captured_origin_with_connection(
        machine_id, origin, token, pin, client_id, connection)
}

pub(crate) enum ReadoutSurface {
    /// Home's and a Library source's failure read-out: an answered question is changed in
    /// Settings → Unencrypted connections.
    SignedIn,
}

/// **What a read-out says about an insecure-only verdict** — one table for sign-in, rediscovery,
/// the owner's re-wording after an answer, and the signed-in failure read-outs
/// (`screens::plaintext_question`), so no two of them can describe the same verdict differently.
/// Every line fits the failed read-out's two-line reason slot (`StatusOverlay::reason_view`) with
/// its action last, and every one that can be acted on names the action:
///
/// * **eligible, never asked** — the server is on this network; the primary is *Connect*, which
///   asks "Connect without encryption?";
/// * **eligible, allowed** — the primary is *Try again*, which connects without encryption;
/// * **eligible, declined or turned off** — the primary is *Try again*; on the sign-in read-out it
///   asks again, signed in the reason points at Settings → Unencrypted connections;
/// * **remote-only plaintext** — never offered; the fix is the server's Remote Access;
/// * everything else — the owner-approved [`discovery_insecure_only_message`], unchanged for the
///   household's own server, and a line naming the owner for a shared one.
///
/// A SHARED server is named by its owner ([`PlaintextVerdict::shared_by`]) — on screen only; the
/// report never carries it.
pub(crate) fn plaintext_copy(verdict: Option<&PlaintextVerdict>, surface: ReadoutSurface) -> std::borrow::Cow<'static, str> {
    plaintext_copy_in(verdict, surface, crate::i18n::current())
}

// Explicit locale keeps the entire verdict testable without changing the process locale.
// Each catalog sentence owns the server/owner grammar and the named action.
fn plaintext_copy_in(verdict: Option<&PlaintextVerdict>, surface: ReadoutSurface,
    locale: &crate::i18n::LocaleContext) -> std::borrow::Cow<'static, str> {
    use crate::i18n::msg;
    use std::borrow::Cow;
    let fallback = || Cow::Borrowed(msg::browse_auth_insecure_in(locale));
    let Some(v) = verdict else { return fallback() };
    let owner = v.shared_by.as_str();
    if v.offers() {
        return if owner.is_empty() {
            Cow::Borrowed(match (v.choice, surface) {
                (PlaintextChoice::Undecided, _) => msg::browse_auth_plaintext_offer_in(locale),
                (PlaintextChoice::Allowed, _) => msg::browse_auth_plaintext_allowed_in(locale),
                (PlaintextChoice::Declined, ReadoutSurface::SignedIn) => msg::browse_auth_plaintext_declined_signed_in_in(locale),
                (PlaintextChoice::Revoked, ReadoutSurface::SignedIn) => msg::browse_auth_plaintext_revoked_signed_in_in(locale),
            })
        } else {
            Cow::Owned(match (v.choice, surface) {
                (PlaintextChoice::Undecided, _) => msg::browse_auth_plaintext_shared_offer_in(locale, owner),
                (PlaintextChoice::Allowed, _) => msg::browse_auth_plaintext_shared_allowed_in(locale, owner),
                (PlaintextChoice::Declined, ReadoutSurface::SignedIn) => msg::browse_auth_plaintext_shared_declined_signed_in_in(locale, owner),
                (PlaintextChoice::Revoked, ReadoutSurface::SignedIn) => msg::browse_auth_plaintext_shared_revoked_signed_in_in(locale, owner),
            })
        };
    }
    match (v.eligibility, owner.is_empty()) {
        (PlaintextEligibility::NotLocal, true) => Cow::Borrowed(msg::browse_auth_plaintext_remote_in(locale)),
        (PlaintextEligibility::NotLocal, false) => Cow::Owned(msg::browse_auth_plaintext_shared_remote_in(locale, owner)),
        (_, false) => Cow::Owned(msg::browse_auth_plaintext_shared_insecure_in(locale, owner)),
        (_, true) => fallback(),
    }
}
/// Publish a completed server race onto the already-registered slot for that machine. A newly
/// granted server that never verified an address has no slot yet and is deliberately ignored:
/// probe failure is not authority to register an unverified endpoint. A retained/offline source,
/// however, is already registered from its cached verified origin and receives the new state.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SettledProbe {
    machine_id: String,
    #[serde(with = "observation::outcome")]
    outcome: Outcome,
    tier: Option<probe::Location>,
    /// The candidate address that answered, kept ONLY so a background application of this
    /// verdict (`publish_settled_probe`) can derive the connection's IP generation without ever
    /// reading `origin.host()` (R3/A3) — a `plex.direct` NAME there, not the dotted quad. `None`
    /// whenever `tier` is: nothing verified, so there is nothing to derive from.
    #[serde(default)]
    address: Option<String>,
}
/// The grant evidence a persisted [`SourceRef`] carries, for the registry describers.
fn grant_of(source: &SourceRef) -> crate::registry::GrantEvidence {
    crate::registry::GrantEvidence {
        owned: source.owned,
        home: source.home,
        owner_id: source.owner_id,
    }
}
/// Owned entries are registered FIRST even when `primary` is `None` — see [`registration_order`].
fn install_roster(sources: &[SourceRef], primary: Option<usize>, client_id: &str) -> Vec<ServerId> {
    let order = registration_order(sources);
    let mut installed = Vec::with_capacity(order.len());
    for &i in &order {
        let s = &sources[i];
        // `registration_order` already filtered on `usable()`, which IS `origin().is_some()` —
        // so this `else` is unreachable today and is a `continue` rather than an `expect` because
        // a roster entry has never been allowed to cost more than itself (`de_soft_vec`).
        let Some(origin) = s.origin() else { continue };
        // Applied atomically inside the same write (#95 step 8) — a re-pointed slot's fresh
        // `Client` gets its connection facts from THIS call rather than a separate one after.
        let connection =
            crate::registry::ConnectionFacts::new(s.tier, crate::plex::IpVersion::of_host(&s.address));
        let id = register_observed_origin(&s.machine_id, &origin, &s.token,
            s.resolve_pin().as_ref(), connection, client_id);
        if !id.is_set() {
            continue;
        }
        installed.push(id);
        // …and say WHOSE it is. Registering without this was the bug that made the whole shared-
        // source feature invisible on the only path a real user takes: `ServerFacts` stayed unset,
        // so every source read as owned with no handle, and each surface then correctly drew
        // nothing — no "Shared by" on a detail page, no handle on a shelf heading or the Source
        // chip, no owner on a failure read-out, and a friend's library pinned to Home by the
        // ownership default. It looked like five separate features not working. The one
        // `describe_server` call that existed was in `app.rs`'s DEV-TRIGGER path, which is exactly
        // why a headless capture showed the handle and a signed-in television did not.
        //
        // `owned` comes from the roster rather than from an empty handle: a share whose
        // `sourceTitle` plex.tv did not send is still a share.
        crate::registry::describe(id, &s.name, &s.shared_by, grant_of(s));
        if primary == Some(i) {
            crate::registry::set_current(id);
        }
    }
    installed
}

/// Which roster entries to register, and in what order: the ones that can actually be dialled,
/// **ours first**.
///
/// The order is load-bearing, not tidiness. The registry makes the FIRST registration current when
/// nothing is current yet (`servers.rs`), which is exactly the state a boot is in — so a roster
/// that happens to list a share first would silently come up pointed at the friend's server, and
/// Home would be built from their library. Stable, so plex.tv's own order survives inside each
/// group.
fn registration_order(sources: &[SourceRef]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..sources.len())
        .filter(|&i| sources[i].dialable())
        .collect();
    order.sort_by_key(|&i| !sources[i].owned);
    order
}

#[cfg(test)]
#[path = "auth_registry_tests.rs"]
mod registry_tests;

