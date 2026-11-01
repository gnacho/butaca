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
use crate::plex::account::{AccountClient, CallEvidence, Resource};
use crate::grant::PlaintextAsk;
use crate::plex::probe::{
    self, Candidate, HttpsRoutes, InsecureEvidence, Outcome, PlaintextEligibility, ProbePlan, RouteOutcome,
};
use crate::plex::session::{self, PlaintextChoice, ProfileCreds, ServerRef, Session, SourceRef};
use crate::origin::{CredentialPolicy, Origin};
use crate::plex::ServerId;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

pub(crate) mod owner;
pub(crate) mod observation;
pub(crate) use owner::{SessionInit, SessionMachine, SessionRead};

/// Resource executor entry. Every credential and network-policy input is captured by the
/// requesting owner/adapter; workers can only observe cancellation and publish stream facts.
///
/// `ask` is the person's plaintext consent and the grant generations, captured by the adapter on
/// main when it admitted this work (`grant::PlaintextAsk`).
pub(crate) fn run_session_work(key: owner::SessionWorkKey,
    input: owner::SessionWork, ask: PlaintextAsk, output: &dyn owner::ObservationSink) {
    fn identity(value: owner::Identity) -> SessionIdentity {
        SessionIdentity { client_id: value.client_id, account_token: value.account_token,
            profile_uuid: value.profile_uuid }
    }
    let epoch = key.epoch;
    match input {
        owner::SessionWork::ServerRoster { session, expected } => {
            let household = session.household_ids();
            server_roster_worker_with_output(session, epoch, identity(expected), household, &ask, output);
        }
        owner::SessionWork::Endpoint { session, expected, lifecycle, machine_id } => {
            endpoint_worker_with_io(epoch, session, expected, lifecycle, machine_id, output,
                |ac, remaining| ac.resources_with(account_timeouts(remaining)),
                |resource, household| probe_profile_resource_live(resource, household, &ask));
        }
    }
}

/// Shared endpoint worker body; only account/probe IO is injectable. Native lifecycle remains
/// adapter metadata and only main can apply the terminal observation.
pub(crate) fn endpoint_worker_with_io(epoch: u64, session: Session, expected: owner::Identity,
    lifecycle: owner::ServerLifecycle, machine_id: String, output: &dyn owner::ObservationSink,
    resources: impl FnMut(&AccountClient, Duration) -> Result<Vec<Resource>, CallEvidence>,
    probe: impl FnOnce(&Resource, &[i64]) -> (Option<SourceRef>, SettledProbe)) {
    let (fresh, probe) = probe_endpoint_work(ServerId::from_raw(lifecycle.sid), &machine_id, &session,
        resources, probe, &|| output.live());
    output.terminal(endpoint_work_fact(epoch, expected, lifecycle, machine_id, fresh, probe));
}

/// Endpoint transport projection shared by the real worker and injected network-result tests.
/// Admission, interest and native lifecycle validation remain in the adapter/owner protocol.
pub(crate) fn endpoint_work_fact(epoch: u64, expected: owner::Identity,
    lifecycle: owner::ServerLifecycle, machine_id: String, fresh: Option<SourceRef>,
    probe: Option<SettledProbe>) -> AuthProgress {
    AuthProgress::Endpoint(EndpointProgress { epoch,
        expected: SessionIdentity { client_id: expected.client_id, account_token: expected.account_token,
            profile_uuid: expected.profile_uuid },
        id: ServerId::from_raw(lifecycle.sid), machine_id, lifecycle: None, fresh, probe })
}

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
use crate::log;

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
pub(crate) struct ClientLifecycle {
    client: &'static crate::plex::Client,
    token_gen: u32,
}

impl ClientLifecycle {
    pub(crate) fn machine_id(self) -> &'static str { self.client.machine_id() }

    pub(crate) fn capture(client: &'static crate::plex::Client) -> Self {
        Self { client, token_gen: client.token_gen() }
    }
    pub(crate) fn logical(self, sid: u16) -> owner::ServerLifecycle {
        owner::ServerLifecycle { sid, instance_gen: self.client.instance_gen(), token_gen: self.token_gen }
    }
    pub(crate) fn is_current(self, expected: owner::ServerLifecycle) -> bool {
        self.logical(expected.sid) == expected && crate::plex::commit_if_current(
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
        owner::RegistryPlan::Primary { server, token } => {
            // #95 step 8 / A2: carry the tier + address-derived IP into the same registration
            // write, rather than restoring them in a later separate call the boot picker's avatar
            // client used to skip.
            let connection = crate::plex::ConnectionFacts::new(
                server.tier,
                crate::plex::IpVersion::of_host(&server.address),
            );
            let id = register_observed_origin("", &server.origin(), token, server.resolve_pin().as_ref(), connection, client_id);
            crate::plex::set_current(id);
        }
        owner::RegistryPlan::Activate { source, ipv6 } => {
            let Some(origin) = source.origin() else { return false };
            let Some(location) = source.tier else { return false };
            apply_candidate_activation(CandidateActivation {
                machine_id: source.machine_id.clone(), token: source.token.clone(),
                name: source.name.clone(), credit: source.shared_by.clone(), owned: source.owned,
                home: source.home, owner_id: source.owner_id,
                origin: origin.clone(), address: source.address.clone(),
                location, ipv6: *ipv6,
            }, client_id);
            retire_grant_on_https(&source.machine_id, &origin);
        }
        owner::RegistryPlan::Install { sources, primary, replace } => {
            if *replace {
                // The profile's identity changes HERE, at the commit, with the tokens: grants
                // survive only for the exact origins this roster installs (`grant`).
                let installing: Vec<(String, Origin)> = sources.iter()
                    .filter_map(|s| s.origin().map(|origin| (s.machine_id.clone(), origin)))
                    .collect();
                crate::grant::roster_replaced(&installing);
                crate::plex::revoke_for_profile_switch();
            }
            let installed = install_roster(sources, *primary, client_id);
            if *replace { crate::plex::finish_profile_switch(&installed); }
            for source in sources {
                if let Some(origin) = source.origin() { retire_grant_on_https(&source.machine_id, &origin); }
            }
        }
        owner::RegistryPlan::Endpoint { expected, source } => {
            let Some(origin) = source.origin() else { return false };
            let connection = crate::plex::ConnectionFacts::new(
                source.tier,
                crate::plex::IpVersion::of_host(&source.address),
            );
            let id = register_observed_origin(&source.machine_id, &origin, &source.token,
                source.resolve_pin().as_ref(), connection, client_id);
            if id.raw() != expected.sid { return false; }
            crate::plex::describe_server(id, &source.name, &source.shared_by, grant_of(source));
            crate::plex::publish_probe_result(id, Outcome::Reachable);
            retire_grant_on_https(&source.machine_id, &origin);
        }
        owner::RegistryPlan::Probe(probe) => publish_settled_probe(probe),
        owner::RegistryPlan::Revoke => crate::plex::revoke_all(),
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
        pin: Option<&crate::origin::ResolvePin>, connection: crate::plex::ConnectionFacts| {
        if let Some(cid) = client_id {
            crate::plex::register_captured_origin_with_connection(machine, origin, token, pin, cid,
                connection)
        } else {
            register_observed_origin(machine, origin, token, pin, connection, &session::peek().client_id)
        }
    };
    let primary_connection =
        crate::plex::ConnectionFacts::new(tier, crate::plex::IpVersion::of_host(address));
    let id = register("", origin, token, pin, primary_connection);
    crate::plex::set_current(id);
    for source in extras {
        let Some(origin) = source.origin() else { continue };
        if source.token.is_empty() { continue; }
        let connection = crate::plex::ConnectionFacts::new(
            source.tier,
            crate::plex::IpVersion::of_host(&source.address),
        );
        let id = register(&source.machine_id, &origin, &source.token,
            source.resolve_pin().as_ref(), connection);
        crate::plex::describe_server(id, &source.name, &source.shared_by, grant_of(source));
    }
}

/// The one ordered auth stream. `Login` is the already-shipped multi-observation QR protocol;
/// R2A adds the remaining immutable worker observations beside it without changing its variants or
/// terminal ordering.
pub(crate) enum AuthProgress {
    Registry(RegistryProgress),
    ServerRoster(ServerRosterProgress),
    Endpoint(EndpointProgress),
}

/// Register with the identity captured by the owner, including before a fresh login is durable.
fn register_observed_origin(
    machine_id: &str,
    origin: &Origin,
    token: &str,
    pin: Option<&crate::origin::ResolvePin>,
    connection: crate::plex::ConnectionFacts,
    client_id: &str,
) -> ServerId {
    crate::plex::register_captured_origin_with_connection(
        machine_id, origin, token, pin, client_id, connection)
}

fn apply_candidate_activation(candidate: CandidateActivation, client_id: &str) {
    let pin = crate::origin::ResolvePin::for_origin(&candidate.origin, &candidate.address);
    let connection = crate::plex::ConnectionFacts::new(
        Some(candidate.location),
        Some(if candidate.ipv6 {
            crate::plex::IpVersion::V6
        } else {
            crate::plex::IpVersion::V4
        }),
    );
    let id = register_observed_origin(
        &candidate.machine_id,
        &candidate.origin,
        &candidate.token,
        pin.as_ref(),
        connection,
        client_id,
    );
    if crate::plex::client_for(id).is_some() {
        crate::plex::publish_probe_result(id, Outcome::Reachable);
    }
    crate::plex::describe_server(id, &candidate.name, &candidate.credit, crate::plex::GrantEvidence {
        owned: candidate.owned, home: candidate.home, owner_id: candidate.owner_id,
    });
}

// ---- worker threads ----

struct AccountRetryPolicy {
    max_attempts: u32,
    budget: Duration,
}

/// The account-call policy table.
const BACKGROUND_ACCOUNT: AccountRetryPolicy = AccountRetryPolicy {
    max_attempts: 2, budget: Duration::from_secs(30),
};

#[derive(Debug, PartialEq, Eq)]
enum AccountCallEnd<T> {
    Answer(T),
    Failed(CallEvidence),
    Cancelled,
}

#[derive(Debug, PartialEq, Eq)]
struct Retried<T> {
    result: AccountCallEnd<T>,
    attempts: u32,
    elapsed: Duration,
}

trait RetryClock {
    fn elapsed(&self) -> Duration;
    fn wait(&mut self, duration: Duration) -> bool;
}

/// Run one account edge under a bounded, cancellable retry policy. A call already in flight is
/// always allowed to answer; the deadline is inspected only after it returns. plex.tv's 429 is
/// deliberately terminal here because `Resp` exposes no Retry-After header, so the app cannot
/// prove that another request fits inside the budget.
fn retry_account_call<T>(policy: AccountRetryPolicy, clock: &mut impl RetryClock,
    mut on_miss: impl FnMut(u32, Duration, CallEvidence),
    mut call: impl FnMut(Duration) -> Result<T, CallEvidence>) -> Retried<T> {
    let mut attempts = 0u32;
    loop {
        attempts = attempts.saturating_add(1);
        let remaining = policy.budget.saturating_sub(clock.elapsed());
        match call(remaining) {
            Ok(value) => return Retried { result: AccountCallEnd::Answer(value), attempts,
                elapsed: clock.elapsed() },
            Err(evidence) => {
                let elapsed = clock.elapsed();
                on_miss(attempts, elapsed, evidence);
                let status = match evidence { Ok(status) => Some(status), Err(f) => f.status };
                let can_retry = crate::plex::account::transient(&evidence)
                    && status != Some(429)
                    && attempts < policy.max_attempts
                    && elapsed < policy.budget;
                if !can_retry {
                    return Retried { result: AccountCallEnd::Failed(evidence), attempts, elapsed };
                }
                let pause = crate::plex::account::backoff(attempts - 1,
                    Duration::from_secs(2), Duration::from_secs(4))
                    .min(policy.budget.saturating_sub(elapsed));
                if !clock.wait(pause) {
                    return Retried { result: AccountCallEnd::Cancelled, attempts,
                        elapsed: clock.elapsed() };
                }
            }
        }
    }
}

fn account_timeouts(remaining: Duration) -> crate::net::Timeouts {
    let capped = remaining.min(Duration::from_secs(8));
    let millis = capped.as_millis().max(1).min(i32::MAX as u128) as _;
    crate::net::Timeouts { total_ms: millis, connect_s: capped.as_secs().max(1) as _,
        ..crate::net::API }
}

fn cancellable_wait_while(duration: Duration, live: impl Fn() -> bool) -> bool {
    const SLICE: Duration = Duration::from_secs(1);
    let deadline = Instant::now() + duration;
    loop {
        if !live() { return false; }
        let now = Instant::now();
        if now >= deadline { return true; }
        std::thread::sleep((deadline - now).min(SLICE));
    }
}

struct LiveRetryClock<'a> {
    output: &'a dyn owner::ObservationSink,
    started: Instant,
}
impl RetryClock for LiveRetryClock<'_> {
    fn elapsed(&self) -> Duration { self.started.elapsed() }
    fn wait(&mut self, duration: Duration) -> bool {
        cancellable_wait_while(duration, || self.output.live())
    }
}

struct LiveFnClock<'a> { live: &'a dyn Fn() -> bool, started: Instant }
impl RetryClock for LiveFnClock<'_> {
    fn elapsed(&self) -> Duration { self.started.elapsed() }
    fn wait(&mut self, duration: Duration) -> bool {
        cancellable_wait_while(duration, self.live)
    }
}

// ---- server discovery ----
//
// **Every server the account can reach, not the first one that looks local.** What this replaced
// filtered both of its passes on `c.local && !c.relay`, kept exactly one server, and threw the rest
// of the account away. Against a real share that filter is worse than useless: `Connection.local`
// means "this address is RFC1918", not "you are on that LAN", so it selected the OWNER's
// `172.20.x.x` — 8 s of timeout from here, and the *worse* outcome is that it succeeds against
// somebody else's box at that address on our own LAN (`docs/shared-servers.md` §2a).
//
// So the shape is: ranked candidates from `plex::probe` (pure policy keeps only identity-safe
// forms of an unmatched shared-LAN address), race one server's direct candidates, and **verify
// identity on the answer** before believing it. Servers remain serial, with relay as a second phase.

/// How far one server got. Only [`Reach::At`] is a server we can use; the others are the
/// distinction `probe.rs`'s module doc refuses to let a caller collapse, because they send the
/// user to different places.
///
/// **Precedence, best first: `At` > `InsecureOnly` > `Refused` > `No`.** A verified identity —
/// even one this build cannot put a credential on — beats a 401 from a different, parallel or
/// proxied candidate; silence is the weakest signal of all.
enum Reach {
    /// This address answered `/identity` **as the server we asked for**, over a transport that
    /// can carry a credential.
    ///
    /// Two values, and the split is the point: the [`Origin`] is **what was actually dialled**, and
    /// so the only thing the roster may record as this server's address; the [`Candidate`] is kept
    /// beside it for the DIAGNOSTIC fields (`address`, `port`) that the log and the Sources panel
    /// say. Deriving the record from `Candidate::url` while the dial came from `Candidate::address`
    /// left exactly one gap — a plex.tv `uri` whose port disagrees with `port` would be verified at
    /// one and written down as the other — and this pairing closes it by construction.
    At(Candidate, Origin),
    /// This address answered `/identity` **as the server we asked for**, but
    /// [`Candidate::credential_eligible`] is `false` — a plaintext answer in a store build.
    /// **Alive, and provably the right server, but nothing may register it or put a token on it.**
    /// It is not [`Reach::At`], because the whole point of this app's credential guard is that a
    /// verified-but-ineligible candidate must not read as success (`probe.rs`'s module doc); it is
    /// not [`Reach::No`] either, because "the server did not answer" and "the server answered and
    /// this build cannot use it" are two different facts to hand the user.
    ///
    /// The [`HttpsRoutes`] beside it say what became of every HTTPS route to the server — the
    /// evidence that tells "this server has no HTTPS" from "HTTPS failed from here".
    InsecureOnly(Candidate, HttpsRoutes),
    /// One or more candidates answered 401 and no candidate verified the server. A proxy-specific
    /// 401 does not cancel parallel direct probes or the relay fallback; it survives only as the
    /// final reason when none of those proves reachability. Reporting that as generic silence would
    /// send the user to the router for an authorization/access-policy problem.
    Refused,
    /// Nothing answered as this server.
    No,
}

/// Which read-out a [`plaintext_copy`] is for: the two differ only in where an answered question
/// can be changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// The probe path. **Unauthenticated on purpose** — `/identity` answers 200 to anybody, which
/// makes it useless as a token test and perfect as a reachability + identity one.
///
/// The token is deliberately NOT sent. A probe can land on a *different machine* (that is rule 1
/// of `probe.rs`, and the reason identity is verified at all), and a request that carried the
/// per-(user, server) token would hand that stranger a live credential before we had any reason to
/// believe who they are.
///
/// **So this identity stage does not, and cannot, prove the token works.** A per-(user, server)
/// grant revoked between the `/api/v2/resources` fetch and now still probes as
/// [`Outcome::Reachable`] here — the server really is reachable; it is the credential that is dead.
/// Online admission follows this proof with authenticated `GET /library/sections` before selecting
/// or activating the endpoint, so that request classifies the 401 without exposing the token to an
/// unverified machine. The
/// [`Outcome::Unauthorized`] arm below is not dead code for that: a PMS behind an auth proxy, or one
/// whose `allowedNetworks` refuses this subnet, answers 401 to the probe itself, and *that* must not
/// be reported as an unreachable address.
const IDENTITY: &str = "/identity";

/// Can this app's transport dial that candidate? **Every one of them, now** — see [`dial_target`],
/// which this is the boolean face of.
///
/// It used to be the narrowest predicate in the app: plain HTTP at a dotted quad and nothing else,
/// because `stream.rs` was the only transport there was. Every https `plex.direct` origin and every
/// hostname was "unspoken" rather than unreachable — a true distinction, and no comfort at all to
/// an account signed in from anywhere but the server's own LAN, which had nothing left to dial.
/// That was the dead end this one predicate was responsible for.
/// [`dialable`] and the ORIGIN to dial, from one expression — so the predicate that admits a
/// candidate and the value handed to the transport can never disagree.
///
/// **It is [`Candidate::origin`] and nothing else now**, and the emptiness is the achievement. Two
/// separate narrowings used to live in this function, one per gap in the transport, and they were
/// closed by two different pieces of work:
///
/// * *No TLS* — every `https://` candidate was skipped, which is every `plex.direct` uri plex.tv
///   advertises. `crate::http` closed that one by routing an https origin through libcurl.
/// * *No resolver, no IPv6* — a plaintext candidate had to be four decimal octets, because
///   `http_open` built a `sockaddr_in` by hand. `stream.rs` closed that one with `getaddrinfo` and
///   a walk down the whole resolved chain, so a name and a v6 literal are both ordinary now.
///
/// What survives is the port narrowing, and it survives *inside* [`Origin::parse`]: an out-of-range
/// `i64` from plex.tv is refused by [`probe::dial_port`] rather than wrapped by `as i32` into a
/// plausible-looking 32400 (that function's doc has the arithmetic). A candidate refused there is
/// a connection this client cannot open, not a server that failed to answer, so it is skipped and
/// the next address gets its turn.
fn dial_target(c: &Candidate) -> Option<Origin> {
    c.origin()
}

/// The `machineIdentifier` in an `/identity` body, read out of **either** encoding.
///
/// PMS answers JSON only for an explicit `Accept: application/json` and XML for anything else
/// (`plex/CLAUDE.md`), and a probe is exactly the request most likely to meet a proxy, a cache or
/// an older build that ignores the header — so the one field that decides whether we trust the
/// connection is scanned for rather than deserialized. The two forms differ only in the
/// punctuation between the name and the value: `"machineIdentifier":"abc"` and
/// `machineIdentifier="abc"`.
fn machine_id_in(body: &[u8]) -> Option<String> {
    const NAME: &[u8] = b"machineIdentifier";
    let after = body.windows(NAME.len()).position(|w| w == NAME)? + NAME.len();
    let rest = &body[after..];
    let start = rest
        .iter()
        .position(|b| !matches!(b, b'"' | b':' | b'=' | b' ' | b'\t' | b'\r' | b'\n'))?;
    let rest = &rest[start..];
    let end = rest
        .iter()
        .position(|b| matches!(b, b'"' | b'\'' | b'<' | b',' | b'}' | b' '))
        .unwrap_or(rest.len());
    let v = &rest[..end];
    (!v.is_empty()).then(|| String::from_utf8_lossy(v).into_owned())
}

/// Turn one probe response into the outcome the caller must not collapse. Pure, so the acceptance
/// policy is gradeable on the dev Mac — which is the only tier that can grade it, since the
/// failures it prevents are "a stranger's server answered" and "a token problem reported as a dead
/// router".
fn classify(status: i32, body: &[u8], want_machine_id: &str) -> Outcome {
    if status == 401 {
        // 401 ONLY. PMS refuses a credential with 401; a 403 is an endpoint saying "not for you"
        // (the owner-only surfaces), which is not something re-fetching `/resources` can fix.
        return Outcome::Unauthorized;
    }
    if !(200..300).contains(&status) {
        return Outcome::Unreachable;
    }
    if want_machine_id.is_empty() {
        // Nothing to verify against, so nothing is verified. plex.tv sent a resource with no
        // `clientIdentifier`; accepting whatever answered would be accepting an unnamed machine.
        return Outcome::WrongServer;
    }
    match machine_id_in(body) {
        Some(id) if id == want_machine_id => Outcome::Reachable,
        _ => Outcome::WrongServer,
    }
}

/// One unauthenticated `GET {origin}/identity`, as (status, body).
///
/// Goes through [`crate::http`], which is what makes this ONE function able to probe both a
/// plaintext LAN address and an `https://…plex.direct` name: the dispatch is on the origin's
/// scheme, and every candidate `dial_target` admits carries the transport it needs in that field.
/// It hand-rolled the socket before, which is also why it could only ever probe the first kind.
///
/// The STATUS is half the answer, which is why this cannot be a `stream::http_get`: that wrapper
/// folds every non-2xx into `None`, and folding is precisely the collapse of 401 into "unreachable"
/// that this module exists to avoid. [`crate::http::Reply`] carries both halves over either
/// transport.
///
/// A transport failure — nothing answered, DNS said no, the certificate would not validate — comes
/// back as [`ProbeReply::Failed`], keeping libcurl's evidence when the TLS transport had any, so a
/// verdict can later say HOW each route failed ([`RouteOutcome::of_failure`]) instead of only that
/// it did.
///
/// `pin`, when [`race_batch`] built one for this candidate, is forwarded to
/// [`crate::http::request_probe`] exactly as `apply_candidate_activation` forwards one to
/// `register_origin` — the same [`crate::origin::ResolvePin`], used one step earlier: at the DIAL
/// that decides the winner, not only at the registration of one already decided.
fn get_identity(
    origin: &Origin,
    pin: Option<&crate::origin::ResolvePin>,
    budget: Duration,
) -> ProbeReply {
    match crate::http::request_probe(
        origin,
        IDENTITY,
        crate::http::Method::Get,
        &[crate::http::ACCEPT_JSON],
        64 * 1024,
        budget.as_secs().max(1) as i32,
        pin,
    ) {
        // `/identity` is one small MediaContainer. The ceiling is enforced by each transport
        // WHILE it reads, before a machine we have not accepted can make this worker allocate an
        // unbounded body; an over-limit answer is therefore a transport failure, never a prefix
        // that might happen to contain a plausible machine id.
        Ok(r) => ProbeReply::Answered { status: r.status, body: r.body },
        Err(failure) => ProbeReply::Failed(failure),
    }
}

/// What one identity probe observed: an answer (any status), or a transport failure with the
/// evidence the transport kept. Never collapsed to a sentinel status — the failure's `CURLcode` is
/// what an insecure-only verdict names per route.
#[derive(Debug)]
enum ProbeReply {
    Answered { status: i32, body: Vec<u8> },
    Failed(Option<crate::net::RequestFailure>),
}

impl ProbeReply {
    /// The acceptance verdict ([`classify`]) and the route evidence for this reply.
    fn grade(&self, want_machine_id: &str) -> (Outcome, RouteOutcome) {
        match self {
            Self::Answered { status, body } => {
                let outcome = classify(*status, body, want_machine_id);
                (outcome, RouteOutcome::of_answer(*status, outcome))
            }
            Self::Failed(failure) => (Outcome::Unreachable, RouteOutcome::of_failure(*failure)),
        }
    }
}

/// The legacy `(status, body)` shape the synchronous test seams script: status `0` is "nothing
/// answered", with no evidence.
impl From<(i32, Vec<u8>)> for ProbeReply {
    fn from((status, body): (i32, Vec<u8>)) -> Self {
        if status == 0 { Self::Failed(None) } else { Self::Answered { status, body } }
    }
}

/// Candidate probing deadlines belong here, where the connection tier is known. They are
/// deliberately not transport settings: ordinary PMS requests and media reads have different
/// timeout contracts, while discovery alone distinguishes a local path from a remote one.
#[derive(Clone, Copy)]
struct ProbeDeadlines {
    local: Duration,
    remote: Duration,
}

const PROBE_DEADLINES: ProbeDeadlines = ProbeDeadlines {
    local: Duration::from_secs(5),
    remote: Duration::from_secs(10),
};
const SERVER_GAP: Duration = Duration::from_secs(4);
// Authenticated admission starts only after identity probing has produced a candidate. Endpoint
// fallbacks share this ceiling, while each sections request keeps its local/remote attempt cap.
const ADMISSION_BUDGET: Duration = Duration::from_secs(20);

/// Shared authenticated-request budget for discovery and profile switching.
///
/// Identity probes and inter-server pacing happen outside [`Self::attempt`], so they cannot spend
/// this budget. Each authenticated request receives the time still available and is charged only
/// for its own elapsed wall time.
struct AdmissionBudget {
    remaining: Duration,
}

impl AdmissionBudget {
    fn new(remaining: Duration) -> Self { Self { remaining } }

    fn exhausted(&self) -> bool { self.remaining.is_zero() }

    fn attempt(
        &mut self,
        request: impl FnOnce(Instant) -> crate::plex::EndpointAdmission,
    ) -> crate::plex::EndpointAdmission {
        if self.exhausted() {
            return crate::plex::EndpointAdmission::Timeout;
        }
        let started = Instant::now();
        let deadline = started.checked_add(self.remaining).unwrap_or(started);
        let outcome = request(deadline);
        self.remaining = self.remaining.saturating_sub(started.elapsed());
        outcome
    }
}

type ProbeDial = Arc<
    dyn Fn(&Origin, Option<&crate::origin::ResolvePin>, Duration) -> ProbeReply + Send + Sync + 'static,
>;
type ProbeJob = Box<dyn FnOnce() + Send + 'static>;

#[derive(Clone)]
struct Winner {
    index: usize,
    candidate: Candidate,
    origin: Origin,
    score: i32,
}

struct ProbeMessage {
    index: usize,
    on_time: bool,
    outcome: Outcome,
    route: RouteOutcome,
}

const PROBE_PENDING: u8 = 0;
const PROBE_COMPLETED: u8 = 1;
const PROBE_EXPIRED: u8 = 2;

#[derive(Clone)]
struct PendingProbe {
    deadline: Instant,
    state: Arc<AtomicU8>,
}

#[derive(Default)]
struct BatchResult {
    first: Option<Winner>,
    best: Option<Winner>,
    /// The best-scored verified-but-ineligible answer in this batch — set instead of `first`/
    /// `best` when [`Candidate::credential_eligible`] is `false`. Never activated; kept as
    /// evidence for [`Reach::InsecureOnly`] when nothing eligible verifies.
    insecure: Option<Winner>,
    refused: bool,
    /// What each candidate's probe came to, indexed like [`ProbePlan::candidates`]; `None` for
    /// one this batch never dialled. The raw material of [`HttpsRoutes`].
    observed: Vec<Option<RouteOutcome>>,
}

fn probe_deadline(c: &Candidate, policy: ProbeDeadlines) -> Duration {
    if c.location == probe::Location::Local {
        policy.local
    } else {
        policy.remote
    }
}

fn loopback_host(host: &str) -> bool {
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|a| a.is_loopback())
}

/// The official client's additive candidate score. `+6 reachable` is included here even though
/// this function is called only for a reachable answer, so the code remains a literal rendering
/// of the contract rather than a relative shorthand that can drift when another term is added.
fn candidate_score(c: &Candidate, origin: &Origin) -> i32 {
    6 + if loopback_host(&c.address) || loopback_host(origin.host()) {
        3
    } else {
        0
    } + if c.location == probe::Location::Local {
        2
    } else {
        0
    } + if c.scheme == probe::Scheme::Https {
        1
    } else {
        0
    } - if c.location == probe::Location::Relay {
        1
    } else {
        0
    }
}

fn better(a: &Winner, b: &Winner) -> bool {
    a.score > b.score || (a.score == b.score && a.index < b.index)
}

fn settle_probe_message(
    plan: &ProbePlan,
    message: ProbeMessage,
    pending: &mut [Option<PendingProbe>],
    live: &mut usize,
    result: &mut BatchResult,
    activate: &mut dyn FnMut(&ProbePlan, &Candidate, &Origin),
) {
    let Some(_pending) = pending.get_mut(message.index).and_then(Option::take) else {
        return; // expired or already settled: late/duplicate messages are inert
    };
    *live -= 1;
    if let Some(slot) = result.observed.get_mut(message.index) {
        // A late answer is a timeout as far as this race is concerned: it was not waited for.
        *slot = Some(if message.on_time { message.route } else { RouteOutcome::Timeout });
    }
    if !message.on_time {
        return;
    }
    let c = &plan.candidates[message.index];
    match message.outcome {
        Outcome::Reachable => {
            let Some(origin) = dial_target(c) else { return };
            let winner = Winner {
                index: message.index,
                score: candidate_score(c, &origin),
                candidate: c.clone(),
                origin,
            };
            if !c.credential_eligible {
                // Verified — this really is the server we asked for — but over a transport this
                // build may not put a credential on by policy (a plaintext twin in a store build;
                // only `settle_plaintext`, after the race, can add a consented grant). It
                // must not become `first`/`best`: those are what `activate` acts on, and
                // activating this origin would re-point the live server to something every
                // credentialed request then fails on (device, 2026-09-06: `security: refused
                // plaintext PMS credentials`, a hub fetch and the picker's first avatar lost in
                // the gap while the LAN plaintext twin answered before the TLS winner did). It
                // is still evidence the server is alive here, so it is kept — best-scored — as
                // `insecure`, and `probe_server_racing` decides what that means once nothing
                // eligible has verified.
                log(&format!(
                    "auth: '{}' answered only over plaintext at {}",
                    plan.name,
                    winner.origin.log_form()
                ));
                if result.insecure.as_ref().is_none_or(|old| better(&winner, old)) {
                    result.insecure = Some(winner);
                }
                return;
            }
            if result.first.is_none() {
                // "First usable immediately" — every candidate that reaches this branch is
                // already `credential_eligible`, so nothing here needs to ask again whether the
                // build can put a token on it.
                activate(plan, &winner.candidate, &winner.origin);
                result.first = Some(winner.clone());
            }
            if result.best.as_ref().is_none_or(|old| better(&winner, old)) {
                result.best = Some(winner);
            }
        }
        Outcome::Unauthorized => {
            result.refused = true;
            log(&format!(
                "auth: '{}' answered 401 at {} — a token problem, not the network",
                plan.name, c.address
            ));
        }
        Outcome::WrongServer => log(&format!(
            "auth: '{}' — {}:{} answered as a DIFFERENT machine",
            plan.name, c.address, c.port
        )),
        Outcome::Unreachable => {}
        // `classify` never produces this — it is the whole-server AGGREGATE verdict this
        // function's own caller derives from `result.insecure`, not a per-candidate answer.
        Outcome::InsecureOnly => {}
    }
}

/// Race one phase of a server's candidates. The spawner is injected because refusal is a result
/// the coordinator must settle, not an exceptional path a unit test can reach through real OS
/// exhaustion. Only a successful spawn creates a pending entry. Each entry owns an absolute
/// deadline; expiring one local worker never settles a still-live remote worker. Direct and relay
/// are separate phases, so each phase receives the full per-candidate budget appropriate to it.
fn race_batch(
    plan: &ProbePlan,
    indices: &[usize],
    dial: ProbeDial,
    spawn: &dyn Fn(usize, ProbeJob) -> bool,
    policy: ProbeDeadlines,
    activate: &mut dyn FnMut(&ProbePlan, &Candidate, &Origin),
) -> BatchResult {
    let (tx, rx) = mpsc::channel::<ProbeMessage>();
    let mut pending = vec![None; plan.candidates.len()];
    let mut live = 0usize;

    for &index in indices {
        let c = &plan.candidates[index];
        let Some(origin) = dial_target(c) else {
            continue;
        };
        let started = Instant::now();
        let budget = probe_deadline(c, policy);
        let deadline = started + budget;
        let state = Arc::new(AtomicU8::new(PROBE_PENDING));
        let tx = tx.clone();
        let dial = Arc::clone(&dial);
        let worker_state = Arc::clone(&state);
        let machine_id = plan.machine_id.clone();
        // Built here, at the dial that decides a winner — not only at `apply_candidate_activation`,
        // which pins the same way after one already has. `None` for a plaintext candidate (a pin
        // belongs to a TLS name only) or an unmatched/undecodable label; the request then resolves
        // through DNS exactly as before.
        let pin = crate::origin::ResolvePin::for_origin(&origin, &c.address);
        let job = Box::new(move || {
            let (outcome, route) = dial(&origin, pin.as_ref(), budget).grade(&machine_id);
            let on_time = Instant::now() <= deadline;
            // Claim completion before publishing the message. If the coordinator expires first,
            // this result is inert. If this claim wins and the worker is descheduled before send,
            // the coordinator sees COMPLETED and waits for the already-decided result rather than
            // erasing it on its own later wall-clock sample.
            if worker_state
                .compare_exchange(
                    PROBE_PENDING,
                    PROBE_COMPLETED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                let _ = tx.send(ProbeMessage {
                    index,
                    on_time,
                    outcome,
                    route,
                });
            }
        });
        if spawn(index, job) {
            pending[index] = Some(PendingProbe { deadline, state });
            live += 1;
        }
    }
    // Only worker-held senders remain. If one panics (or an injected spawner accepts then drops its
    // job), disconnect settles the remaining pending set instead of parking the coordinator.
    drop(tx);

    let mut result = BatchResult {
        observed: vec![None; plan.candidates.len()],
        ..BatchResult::default()
    };
    while live > 0 {
        // Drain results that completed on time BEFORE expiring by the coordinator's current clock.
        // Spawn setup and queue backlog are allowed to delay observation; `finished` is the fact
        // that decides whether the candidate met its own absolute deadline.
        loop {
            match rx.try_recv() {
                Ok(message) => settle_probe_message(
                    plan,
                    message,
                    &mut pending,
                    &mut live,
                    &mut result,
                    activate,
                ),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if live > 0 {
                        live = 0;
                        pending.fill(None);
                    }
                    break;
                }
            }
        }
        if live == 0 {
            break;
        }
        let now = Instant::now();
        for &index in indices {
            let expired = pending[index].as_ref().is_some_and(|p| {
                p.deadline <= now
                    && p.state
                        .compare_exchange(
                            PROBE_PENDING,
                            PROBE_EXPIRED,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
            });
            if expired {
                pending[index] = None;
                live -= 1;
                result.observed[index] = Some(RouteOutcome::Timeout);
                let c = &plan.candidates[index];
                log(&format!(
                    "auth: '{}' probe timed out at {}:{}",
                    plan.name, c.address, c.port
                ));
            }
        }
        if live == 0 {
            break;
        }
        let next = indices
            .iter()
            .filter_map(|&i| pending[i].as_ref())
            .filter(|p| p.state.load(Ordering::Acquire) == PROBE_PENDING)
            .map(|p| p.deadline)
            .min();
        let received = match next {
            Some(next) => rx.recv_timeout(next.saturating_duration_since(Instant::now())),
            // Every live worker has already claimed completion and owes exactly one message.
            // Blocking here avoids a zero-timeout spin in the tiny claim-before-send window.
            None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        match received {
            Ok(message) => settle_probe_message(
                plan,
                message,
                &mut pending,
                &mut live,
                &mut result,
                activate,
            ),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // Every sender is gone, so no pending candidate can ever report. This includes a
                // worker panic and an injected accepted-but-dropped job.
                live = 0;
                for slot in pending.iter_mut() {
                    *slot = None;
                }
            }
        }
    }
    result
}

/// **Whom to CREDIT for one `/api/v2/resources` row** — the app's single "Shared by …" decision,
/// applied at the boundary where a plex.tv row becomes a persisted [`SourceRef`].
///
/// The rule and its evidence are `plex::servers::owner_credit`; this is only the place discovery
/// calls it, and the reason it is a named function rather than three inline expressions is that
/// there ARE three ingest sites ([`resolve_roster_using`], [`source_from_reach`],
/// [`refreshed_sources`]) and one of them disagreeing is exactly how the raw `sourceTitle` got onto
/// the household's own server in the first place.
///
/// `household` is [`session::Session::household_ids`], captured by the caller from the live session
/// rather than read here: these functions are pure so the whole of discovery is host-gradeable, and
/// a worker that read the session file mid-probe would be reading it under whoever switched profile
/// meanwhile ([`crate::plex`]'s "capture the server at the spawn site" rule, one identity up).
fn credit_of(res: &Resource, household: &[i64]) -> String {
    crate::plex::owner_credit(res.grant(), household).to_string()
}

fn resolve_roster_live_while(
    resources: &[Resource],
    household: &[i64],
    client_id: &str,
    ask: &PlaintextAsk,
    activate: &mut dyn FnMut(&ProbePlan, &Candidate, &Origin),
    observe: &mut dyn FnMut(&ProbePlan, Outcome, Option<probe::Location>, Option<String>),
    live: &dyn Fn() -> bool,
) -> Resolution {
    let policy = CredentialPolicy::build();
    let dial: ProbeDial = Arc::new(get_identity);
    let spawn = |_index: usize, job: ProbeJob| crate::task::spawn_small("probe", job);
    let mut probe_one = |plan: &ProbePlan, rejected: &[String]| {
        if !live() { return Reach::No; }
        probe_server_racing(&plan.without(rejected), Arc::clone(&dial), &spawn, PROBE_DEADLINES,
            &mut |_, _, _| {})
    };
    let mut admission_budget = AdmissionBudget::new(ADMISSION_BUDGET);
    let mut admit = |source: &SourceRef| {
        if !live() { return crate::plex::EndpointAdmission::Transport; }
        admission_budget.attempt(|deadline|
            crate::plex::admit_source_until(source, client_id, deadline))
    };
    let mut between_servers = || {
        if !live() { return; }
        std::thread::sleep(SERVER_GAP);
    };
    resolve_roster_using_admission(
        resources,
        household,
        policy,
        ask,
        &mut probe_one,
        &mut admit,
        activate,
        &mut between_servers,
        observe,
    )
}

fn probe_server_racing(
    plan: &ProbePlan,
    dial: ProbeDial,
    spawn: &dyn Fn(usize, ProbeJob) -> bool,
    policy: ProbeDeadlines,
    activate: &mut dyn FnMut(&ProbePlan, &Candidate, &Origin),
) -> Reach {
    let direct: Vec<usize> = plan
        .candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| (c.location != probe::Location::Relay).then_some(i))
        .collect();
    let relay: Vec<usize> = plan
        .candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| (c.location == probe::Location::Relay).then_some(i))
        .collect();

    let mut batch = race_batch(plan, &direct, Arc::clone(&dial), spawn, policy, activate);
    // Relay is the reachability fallback whenever nothing eligible verified directly — `first` is
    // only ever set for a `credential_eligible` winner now (`settle_probe_message`), so a
    // plaintext-only direct answer still starves nothing here — including when a proxy on one
    // direct origin answered 401. Preserve that refusal, and the direct-only insecure answer, only
    // as the final reason if relay also produces no eligible winner; a verified identity always
    // beats a parallel/proxy 401.
    if batch.first.is_none() && !relay.is_empty() {
        let direct_refused = batch.refused;
        let direct_insecure = batch.insecure.take();
        let direct_observed = std::mem::take(&mut batch.observed);
        batch = race_batch(plan, &relay, dial, spawn, policy, activate);
        batch.refused |= direct_refused;
        if batch.insecure.is_none() {
            batch.insecure = direct_insecure;
        }
        // The two phases dial disjoint candidates, so their observations merge slot by slot.
        for (slot, direct) in batch.observed.iter_mut().zip(direct_observed) {
            if slot.is_none() {
                *slot = direct;
            }
        }
    }

    let Some(best) = batch.best else {
        return if let Some(insecure) = batch.insecure {
            Reach::InsecureOnly(insecure.candidate, HttpsRoutes::of(&plan.candidates, &batch.observed))
        } else if batch.refused {
            Reach::Refused
        } else {
            Reach::No
        };
    };
    let first = batch
        .first
        .as_ref()
        .expect("a best winner is also a first winner");
    // The final re-point to the best score. Both `first` and `best` are `credential_eligible` by
    // construction (`settle_probe_message` never lets an ineligible winner become either), so the
    // activation question this used to re-ask here is already settled.
    if first.index != best.index {
        activate(plan, &best.candidate, &best.origin);
    }
    Reach::At(best.candidate, best.origin)
}


fn candidate_activation(
    plan: &ProbePlan,
    c: &Candidate,
    origin: &Origin,
    credit: &str,
    evidence: crate::plex::GrantEvidence,
) -> CandidateActivation {
    CandidateActivation {
        machine_id: plan.machine_id.clone(),
        token: plan.token.clone(),
        name: plan.name.clone(),
        credit: credit.to_owned(),
        // `owned` still comes from the PLAN — it is what the plan was built to dial under — while
        // `home`/`ownerId` come from the paired wire row, exactly as the credit does. A plan
        // deliberately carries only what is needed to DIAL, and the pairing by `clientIdentifier`
        // is the one identity that cannot drift.
        owned: plan.owned,
        home: evidence.home,
        owner_id: evidence.owner_id,
        origin: origin.clone(),
        address: c.address.clone(),
        location: c.location,
        ipv6: c.ipv6,
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

pub(crate) fn settled_probe(
    plan: &ProbePlan,
    outcome: Outcome,
    tier: Option<probe::Location>,
    address: Option<String>,
) -> SettledProbe {
    SettledProbe {
        machine_id: plan.machine_id.clone(),
        outcome,
        tier,
        address,
    }
}

#[cfg(test)]
pub(crate) fn server_roster_unreachable_for_test(epoch: u64) -> AuthProgress {
    AuthProgress::ServerRoster(ServerRosterProgress {
        epoch,
        expected: SessionIdentity::of(&Session::default()),
        outcome: ServerRosterOutcome::Unreachable,
    })
}


/// Test-only convenience: build a [`SettledProbe`] directly by machine id, for fixtures that have
/// a `SourceRef`/machine id in hand but no [`ProbePlan`] worth constructing just to read one field
/// off it.
#[cfg(test)]
pub(crate) fn settled_probe_for_test(
    machine_id: &str,
    outcome: Outcome,
    tier: Option<probe::Location>,
    address: Option<String>,
) -> SettledProbe {
    SettledProbe { machine_id: machine_id.to_owned(), outcome, tier, address }
}

/// Turn one settled race into what gets published and, when something verified, recorded —
/// `resolve_roster_using` and `probe_profile_resource_live` each needed this same mapping once per
/// server (plan §4). [`Reach::InsecureOnly`] keeps its candidate's tier and address (S9): the
/// state itself already carries the verdict, so a diagnostic tier is not a claim of usability.
fn probe_verdict(reach: &Reach) -> (Outcome, Option<probe::Location>, Option<String>) {
    match reach {
        Reach::At(c, _) => (Outcome::Reachable, Some(c.location), Some(c.address.clone())),
        Reach::InsecureOnly(c, _) => (Outcome::InsecureOnly, Some(c.location), Some(c.address.clone())),
        Reach::Refused => (Outcome::Unauthorized, None, None),
        Reach::No => (Outcome::Unreachable, None, None),
    }
}

fn publish_settled_probe(probe: &SettledProbe) {
    // **A grant lives exactly as long as a fresh verdict keeps reaching its server.** A reach at
    // the granted origin re-minted (or kept) it in `settle_plaintext`, and a reach over HTTPS
    // retires it at the registration commit (`retire_grant_on_https`); every other settled answer
    // — ineligible now, insecure-only without consent, 401, silence — withdraws it here, at the
    // one point every discovery path publishes through, so a token is never left standing for an
    // origin the latest probe did not re-prove.
    if probe.outcome != Outcome::Reachable {
        crate::grant::revoke(&probe.machine_id);
    }
    // …and an offer lives exactly as long as the latest verdict is still insecure-only.
    if probe.outcome != Outcome::InsecureOnly {
        crate::grant::withdraw_offer(&probe.machine_id);
    }
    let Some((id, client)) = crate::plex::server_ids()
        .filter_map(|id| crate::plex::client_for(id).map(|client| (id, client)))
        .find(|(_, client)| client.machine_id() == probe.machine_id)
    else {
        return;
    };
    if let Some(link) = probe.tier {
        // `apply_connection`, not the bare `set_link` this used to call: the same rule (R3) — the
        // IP generation comes from the candidate's own ADDRESS, never from `origin.host()`, which
        // is a `plex.direct` NAME for exactly the connections this fix is about — and `None` from
        // an unparseable address LEAVES a previously known ip alone (A1) rather than forcing it
        // back to unknown, since this function updates an ALREADY-registered client rather than
        // applying facts inside a fresh registration write.
        let ip = probe.address.as_deref().and_then(crate::plex::IpVersion::of_host);
        client.apply_connection(crate::plex::ConnectionFacts::new(Some(link), ip));
    }
    crate::plex::publish_probe_result(id, probe.outcome);
}

/// Legacy synchronous seam for the older acceptance fixtures. Production uses
/// [`probe_server_racing`]; these tests still exercise identity mismatch, 401 and roster-recording
/// semantics without timing or worker scheduling in their assertions.
/// One insecure-only server, as the read-out needs it — `grant` owns the type, because the
/// consent surfaces outside sign-in read it from the grant table's offers.

/// **A verified plaintext answer becomes usable only under a grant.** The race holds plaintext
/// aside until every HTTPS route, the relay included, has settled ([`probe_server_racing`]); what
/// comes back as [`Reach::InsecureOnly`] is turned into [`Reach::At`] here exactly when `ask` — the
/// person's consent, captured at the worker's spawn site — allows this server AND
/// `grant::mint` finds the fresh verdict eligible. Anything else leaves it insecure-only, so
/// no admission, registration or token follows. A dev build whose policy already allows plaintext
/// never reaches this (its candidates are eligible at synthesis).
///
/// A verdict left insecure-only is published as the server's OFFER (`grant::offered`) —
/// eligible ones become the question every consent surface can ask, anything else withdraws it.
///
/// `plan` is the WHOLE plan and `rejected` the origins admission already refused for this server
/// (the race itself dialled [`ProbePlan::without`] them): an HTTPS origin that verified and was
/// then refused is an HTTPS answer ([`probe::HttpsRoutes::with_admission_rejected`]), so the
/// verdict this returns — and the read-out built from it — never calls that server eligible.
fn settle_plaintext(
    res: &Resource,
    plan: &ProbePlan,
    rejected: &[String],
    household: &[i64],
    reach: Reach,
    ask: &PlaintextAsk,
) -> Reach {
    let Reach::InsecureOnly(c, routes) = reach else { return reach };
    let routes = routes.with_admission_rejected(&plan.candidates, rejected);
    let Some(origin) = dial_target(&c) else { return Reach::InsecureOnly(c, routes) };
    let evidence = InsecureEvidence::new(res, &c, routes);
    match ask.settle(&plan.machine_id, &origin, &evidence) {
        Ok(()) => {
            log(&format!(
                "auth: '{}' verified only over plaintext on this network — connecting at {} with the person's consent",
                plan.name,
                origin.log_form()
            ));
            Reach::At(c, origin)
        }
        Err(_) => {
            crate::grant::offered(ask.scope(),
                plaintext_verdict(res, plan, &evidence, household, ask));
            Reach::InsecureOnly(c, routes)
        }
    }
}

/// The verdict for one server that settled insecure-only.
fn plaintext_verdict(
    res: &Resource,
    plan: &ProbePlan,
    evidence: &InsecureEvidence,
    household: &[i64],
    ask: &PlaintextAsk,
) -> PlaintextVerdict {
    PlaintextVerdict {
        machine_id: plan.machine_id.clone(),
        name: plan.name.clone(),
        shared_by: credit_of(res, household),
        eligibility: evidence.plaintext_eligibility(),
        choice: ask.choice(&plan.machine_id),
    }
}

/// What probing a whole `/api/v2/resources` response came to.
enum Resolved {
    /// Servers were probed and none produced a usable credential — nothing was found to register
    /// (the account names no server, everything was silent, or every verified answer was over a
    /// transport this build may not put a credential on).
    Nothing,
    /// The roster, **ours first**, each entry carrying the address that actually answered.
    Reached(Vec<SourceRef>),
}

/// A discovery result plus the authenticated evidence that chose its primary.
///
/// `Resolved::Reached` intentionally keeps every identity-verified secondary, but only one of
/// those entries has passed `GET /library/sections`. Keeping that fact beside the roster prevents
/// a later reconciliation from mistaking an unauthenticated secondary for an eligible primary.
struct Resolution {
    outcome: Resolved,
    admitted_machine_id: Option<String>,
}

/// The whole of discovery except the two impure edges — fetching `/resources` and holding a socket.
///
/// Everything that decides what the app ends up talking to lives here: which servers are tried and
/// in what order, which of a server's addresses is accepted, and what is written down about it. It
/// takes the response and a `dial`, so a full sign-in against a two-server account is a host test
/// rather than a screenshot — which matters because this function is the gate on the whole feature:
/// register the wrong connection and no other unit's work is reachable, however correct it is.
///
/// `household` is [`session::Session::household_ids`] — see [`credit_of`]. `policy` is passed
/// explicitly, as it is to every pure function in this file's discovery path — this function has
/// no live edge of its own and must not re-derive the build's policy on its own.
fn resolve_roster_using_admission(
    resources: &[Resource],
    household: &[i64],
    policy: CredentialPolicy,
    ask: &PlaintextAsk,
    probe_one: &mut dyn FnMut(&ProbePlan, &[String]) -> Reach,
    admit: &mut dyn FnMut(&SourceRef) -> crate::plex::EndpointAdmission,
    activate: &mut dyn FnMut(&ProbePlan, &Candidate, &Origin),
    between_servers: &mut dyn FnMut(),
    observe: &mut dyn FnMut(&ProbePlan, Outcome, Option<probe::Location>, Option<String>),
) -> Resolution {
    let mut servers: Vec<&Resource> = resources.iter().filter(|r| r.is_server()).collect();
    if servers.is_empty() {
        return Resolution { outcome: Resolved::Nothing, admitted_machine_id: None };
    }
    // Ours first, then shared servers whose publicAddressMatches says we share the server's NAT.
    // `sort_by_key` is stable, so plex.tv's own order survives inside each group.
    servers.sort_by_key(|r| (!r.owned, !r.public_address_matches));

    let mut found: Vec<SourceRef> = Vec::new();
    let mut admitted_machine_id = None;
    for (server_index, r) in servers.into_iter().enumerate() {
        if server_index != 0 {
            between_servers();
        }
        let plan = probe::plan(r, policy);
        let mut rejected_origins = Vec::new();
        let reach = loop {
            let reach = settle_plaintext(r, &plan, &rejected_origins, household,
                probe_one(&plan, &rejected_origins), ask);
            let Some(s) = source_from_reach(r, &plan, &reach, household) else { break reach };
            // Admission chooses the primary. Once one source is seated, every secondary keeps the
            // profile-specific grant plex.tv returned; its identity probe still decides endpoint
            // and reachability facts, but it does not have to browse before becoming live.
            if !found.is_empty() {
                if let Reach::At(c, origin) = &reach { activate(&plan, c, origin); }
                break reach;
            }
            match admit(&s) {
                crate::plex::EndpointAdmission::Usable => {
                    admitted_machine_id = Some(s.machine_id.clone());
                    if let Reach::At(c, origin) = &reach { activate(&plan, c, origin); }
                    break reach;
                }
                crate::plex::EndpointAdmission::Refused(status) => {
                    log(&format!("auth: {:?} refused its per-profile grant (HTTP {status})",
                        plan.name));
                }
                crate::plex::EndpointAdmission::InsecureOnly => {},
                evidence => log(&format!("auth: {:?} did not admit its grant ({evidence:?})",
                    plan.name)),
            }
            // A consented plaintext origin that did not admit served nothing: its grant goes with
            // it, so a token is not left standing for an origin this run rejected.
            if crate::grant::granted_origin(&s.machine_id)
                .is_some_and(|granted| granted.base() == s.origin_url)
            {
                crate::grant::revoke(&s.machine_id);
            }
            let origin = s.origin_url;
            if rejected_origins.contains(&origin) { break Reach::No; }
            rejected_origins.push(origin);
        };
        let (outcome, tier, address) = probe_verdict(&reach);
        // Publish one aggregate result per server, after all of its direct/relay candidates have
        // settled. In particular a 401 remains distinct from silence, while wrong-machine-only
        // races fold to Unreachable because no address verified this server.
        observe(&plan, outcome, tier, address);
        match reach {
            Reach::At(c, origin) => {
                let s = source_from_reach(r, &plan, &Reach::At(c, origin.clone()), household)
                    .expect("matched reachable source");
                // **`origin.log_form()`, not just `describe()`.** `SourceRef::describe` prints the
                // diagnostic `address:port`, and both candidates of one connection carry the SAME
                // address — plex.tv advertises `192.168.0.10` alongside a
                // `192-168-0-10.<hash>.plex.direct` uri — so that line alone cannot say which of
                // the two answered, i.e. whether this run reached the server over TLS at all. That
                // is the `[[silent-instrument-trap]]` exactly: an instrument that cannot see the
                // one thing the change was made to do. `log_form` is byte-identical to the old
                // half for a plaintext origin (the bare authority), so an archived log stays
                // comparable, and says the whole URL the moment it is anything else.
                log(&format!(
                    "auth: reached {} via {}",
                    s.describe(),
                    origin.log_form()
                ));
                found.push(s);
            }
            // The verified-but-ineligible probe itself already reached the registry through
            // `observe` above (one `RegistryProgress::Settled`/`RegistryPlan::Probe` per server,
            // published before this match runs) — R2/A5's registry-only commit besides.
            Reach::InsecureOnly(c, _) => {
                log(&format!(
                    "auth: '{}' verified at {}:{} but only over plaintext — not recorded",
                    plan.name, c.address, c.port
                ));
            }
            Reach::Refused | Reach::No => {}
        }
    }
    let outcome = if found.is_empty() { Resolved::Nothing } else { Resolved::Reached(found) };
    Resolution { outcome, admitted_machine_id }
}

/// [`credit_of`] for a machine named by a [`ProbePlan`] rather than by the row itself — the early
/// per-candidate publication ([`activate_candidate`]) has the plan and the response, but not the
/// pairing, and a plan deliberately carries only what is needed to DIAL.
///
/// An id that names no row in this response credits nobody. That is the same "absence is the safe
/// direction" the rule itself states: the alternative is attributing a server to whoever plex.tv
/// last mentioned, and the pairing is by `clientIdentifier`, the one identity that cannot drift.
fn credit_for_machine(resources: &[Resource], machine_id: &str, household: &[i64]) -> String {
    if machine_id.is_empty() {
        return String::new();
    }
    resources
        .iter()
        .find(|r| r.is_server() && r.client_identifier == machine_id)
        .map(|r| credit_of(r, household))
        .unwrap_or_default()
}

/// [`credit_for_machine`]'s sibling for the grant EVIDENCE — plex.tv's `home` and `ownerId` for
/// the machine a [`ProbePlan`] names, paired out of the same response by the same
/// `clientIdentifier`.
///
/// It is separate from `credit_for_machine` rather than folded into it because the two answers
/// have different lifetimes at the call site: a credit is graded against the household roster and
/// is a decided STRING, while this is raw wire evidence that outlives any particular roster and is
/// re-graded downstream. An id that names no row carries no evidence, which degrades to exactly
/// what raw `owned` already said — the same "absence is the safe direction" the credit rule takes.
fn evidence_for_machine(resources: &[Resource], machine_id: &str) -> crate::plex::GrantEvidence {
    if machine_id.is_empty() {
        return crate::plex::GrantEvidence::default();
    }
    resources
        .iter()
        .find(|r| r.is_server() && r.client_identifier == machine_id)
        .map(|r| crate::plex::GrantEvidence::of(r.grant()))
        .unwrap_or_default()
}

/// The grant evidence a persisted [`SourceRef`] carries, for the registry describers.
fn grant_of(source: &SourceRef) -> crate::plex::GrantEvidence {
    crate::plex::GrantEvidence {
        owned: source.owned,
        home: source.home,
        owner_id: source.owner_id,
    }
}

/// Project one verified probe winner into the persisted roster shape. A profile switch uses this
/// without registering anything: credentials and endpoints become visible only at its atomic
/// activation commit, never candidate-by-candidate while the previous profile is still live.
///
/// It takes the `Resource` as well as the plan because [`credit_of`] reads two fields off the wire
/// row that the plan does not carry; the plan remains the source of everything about the CONNECTION.
fn source_from_reach(
    res: &Resource,
    plan: &ProbePlan,
    reach: &Reach,
    household: &[i64],
) -> Option<SourceRef> {
    let Reach::At(c, origin) = reach else {
        return None;
    };
    Some(SourceRef {
        machine_id: plan.machine_id.clone(),
        name: plan.name.clone(),
        shared_by: credit_of(res, household),
        owned: plan.owned,
        home: res.home,
        owner_id: res.owner_id,
        origin_url: origin.base(),
        address: c.address.clone(),
        port: c.port,
        token: plan.token.clone(),
        tier: Some(c.location),
        extensions: Default::default(),
    })
}

/// Probe exactly one resource, with no live-registry side effect. This is the bounded critical
/// path of a profile choice; whole-roster discovery deliberately remains a different operation.
fn probe_profile_resource_live(
    resource: &Resource,
    household: &[i64],
    ask: &PlaintextAsk,
) -> (Option<SourceRef>, SettledProbe) {
    probe_profile_resource_live_after(resource, household, &[], ask)
}

fn probe_profile_resource_live_after(
    resource: &Resource,
    household: &[i64],
    rejected_origins: &[String],
    ask: &PlaintextAsk,
) -> (Option<SourceRef>, SettledProbe) {
    let plan = probe::plan(resource, CredentialPolicy::build());
    let dial: ProbeDial = Arc::new(get_identity);
    let spawn = |_index: usize, job: ProbeJob| crate::task::spawn_small("probe", job);
    let reach = probe_server_racing(&plan.without(rejected_origins), dial, &spawn, PROBE_DEADLINES,
        &mut |_, _, _| {});
    let reach = settle_plaintext(resource, &plan, rejected_origins, household, reach, ask);
    let (outcome, tier, address) = probe_verdict(&reach);
    let source = source_from_reach(resource, &plan, &reach, household);
    (source, settled_probe(&plan, outcome, tier, address))
}

fn refreshed_sources(
    stored: &[SourceRef],
    reached: &[SourceRef],
    resources: &[Resource],
    household: &[i64],
) -> Vec<SourceRef> {
    let mut grants: Vec<&Resource> = resources
        .iter()
        .filter(|r| r.is_server() && !r.client_identifier.is_empty() && !r.access_token.is_empty())
        .collect();
    grants.sort_by_key(|r| (!r.owned, !r.public_address_matches));

    let mut out = Vec::new();
    for r in grants {
        if out
            .iter()
            .any(|s: &SourceRef| s.machine_id == r.client_identifier)
        {
            continue;
        }
        if let Some(s) = reached.iter().find(|s| s.machine_id == r.client_identifier) {
            out.push(s.clone());
            continue;
        }
        let Some(mut cached) = stored
            .iter()
            .find(|s| s.machine_id == r.client_identifier)
            .cloned()
        else {
            // A newly granted but unreachable server has no verified address to preserve yet.
            continue;
        };
        cached.token = r.access_token.clone();
        cached.owned = r.owned;
        if !r.name.is_empty() {
            cached.name = r.name.clone();
        }
        // **Assigned, not merged.** The credit follows the CURRENT grant unconditionally, because
        // "no credit" is a positive answer here and not a missing one: this is the exact path a
        // Plex Home profile switch takes, and a stored entry that already names somebody (the
        // admin, from a build that wrote the raw `sourceTitle`) has to lose that name rather than
        // keep it for want of a fresher one. A share whose handle plex.tv stops sending likewise
        // stops being credited — see `plex::servers::owner_credit` on why absence is the safe way
        // to be wrong.
        cached.shared_by = credit_of(r, household);
        // Assigned, not merged, for the same reason the credit is: this is the path a Plex Home
        // profile switch takes, and the stored evidence belongs to whichever profile wrote it.
        cached.home = r.home;
        cached.owner_id = r.owner_id;
        if cached.dialable() {
            out.push(cached);
        }
    }
    out
}

fn same_sources(a: &[SourceRef], b: &[SourceRef]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b.iter()).all(|(a, b)| {
            a.machine_id == b.machine_id
                && a.name == b.name
                && a.shared_by == b.shared_by
                && a.owned == b.owned
                // The carried household evidence, or a roster whose ONLY change is that plex.tv
                // now names an owner would compare equal and never be republished.
                && a.home == b.home
                && a.owner_id == b.owner_id
                && a.address == b.address
                && a.port == b.port
                && a.token == b.token
                && a.origin_url == b.origin_url
                && a.tier == b.tier
        })
}

fn server_ref(source: &SourceRef) -> ServerRef {
    ServerRef {
        name: source.name.clone(),
        machine_id: source.machine_id.clone(),
        address: source.address.clone(),
        port: if source.port != 0 { source.port } else { 32400 },
        token: source.token.clone(),
        tier: source.tier,
        origin_url: source.origin_url.clone(),
        extensions: source.extensions.clone(),
    }
}

/// Select the one source whose authenticated admission succeeded.
///
/// Identity verification alone is not primary evidence: an existing primary may still answer
/// `/identity` after its profile token has begun returning 401. Secondary grants remain in
/// `sources`, but neither their fresh identity result nor a cached endpoint can keep or make one
/// primary without the separately carried admission result.
fn reconcile_refresh_primary(
    server: &mut ServerRef,
    sources: &[SourceRef],
    admitted_machine_id: &str,
) -> bool {
    let Some(next) = sources.iter().find(|source| source.machine_id == admitted_machine_id) else {
        return false;
    };
    if server.machine_id == admitted_machine_id {
        return reconcile_primary(server, sources);
    }
    log(&format!(
        "auth: using admitted primary {:?} at {}:{}",
        next.name, next.address, next.port
    ));
    *server = server_ref(next);
    true
}

/// Reconcile both records of the active owner's primary credential.
///
/// `Session::user.token` is the selected Plex Home user's token for the PRIMARY, so a refresh that
/// rotates that grant—or promotes another machine—must move it together with `Session::server`.
/// Owner sessions without a Home user token already fall back to `server.token` and need no copy.
fn reconcile_refresh_session(
    s: &mut Session,
    sources: &[SourceRef],
    admitted_machine_id: &str,
) -> bool {
    let mut changed = reconcile_refresh_primary(&mut s.server, sources, admitted_machine_id);
    if !s.user.token.is_empty() && s.user.token != s.server.token {
        s.user.token = s.server.token.clone();
        changed = true;
    }
    changed
}

fn server_roster_worker_with_output(sess: Session, epoch: u64, expected: SessionIdentity,
    household: Vec<i64>, ask: &PlaintextAsk, output: &dyn owner::ObservationSink) {
    if !output.live() { return; }
    let ac = AccountClient::new(&sess.client_id, Some(&sess.account_token));
    let mut clock = LiveRetryClock { output, started: Instant::now() };
    let run = retry_account_call(BACKGROUND_ACCOUNT, &mut clock, |_, _, _| {},
        |remaining| ac.resources_with(account_timeouts(remaining)));
    let resources = match run.result {
        AccountCallEnd::Answer(resources) => resources,
        AccountCallEnd::Cancelled => return,
        AccountCallEnd::Failed(evidence) => {
            log(&format!("auth: server roster refresh could not list resources ({})",
                crate::plex::account::describe_evidence(&evidence)));
            output.terminal(AuthProgress::ServerRoster(ServerRosterProgress {
                epoch,
                expected,
                outcome: ServerRosterOutcome::Unreachable,
            }));
            return;
        }
    };
    let mut activate = |plan: &ProbePlan, c: &Candidate, origin: &Origin| {
        let credit = credit_for_machine(&resources, &plan.machine_id, &household);
        let evidence = evidence_for_machine(&resources, &plan.machine_id);
        output.progress(AuthProgress::Registry(RegistryProgress::Activate {
            epoch,
            expected: Some(expected.clone()),
            candidate: candidate_activation(plan, c, origin, &credit, evidence),
        }));
    };
    let mut settled = Vec::new();
    // Unlike `activate` above, this does NOT also `output.progress(RegistryProgress::Settled)`
    // per server as the race runs. It used to, and the owner committed BOTH: the live progress
    // arm (`Observation::Registry` in `owner.rs`) planned one `RegistryPlan::Probe` per server as
    // it settled, and the terminal `ServerRosterOutcome` below planned the identical set again
    // through `settled`/`roster_plan` — every probe this worker ever runs was published twice.
    // This worker is a BACKGROUND refresh (`refresh_roster`) with no live race screen watching
    // it — unlike `discover_and_store`'s sign-in flow, which really does drive a Sources/QR
    // screen off `RegistryProgress::Activate` while candidates settle, and keeps publishing both
    // — so there is nothing here for the extra progress arrival to reach before the terminal
    // commit does the same work. `settled` alone, folded into the terminal outcome, is authoritative.
    let resolution = resolve_roster_live_while(
        &resources,
        &household,
        &sess.client_id,
        ask,
        &mut activate,
        &mut |plan, outcome, tier, address| {
            settled.push(settled_probe(plan, outcome, tier, address));
        },
        &|| output.live(),
    );
    let found = match resolution.outcome {
        Resolved::Reached(found) => found,
        _ => {
            // R2/A5: a background refresh that found nothing to REGISTER may still have PROBED
            // something worth recording — carry it rather than throwing `settled` away, so the
            // owner can commit it as a registry-only `RegistryPlan::Probe` (`owner.rs`).
            output.terminal(AuthProgress::ServerRoster(ServerRosterProgress {
                epoch,
                expected,
                outcome: ServerRosterOutcome::NoReachable { settled },
            }));
            return;
        }
    };
    let admitted_machine_id = resolution.admitted_machine_id
        .expect("a reached roster has one authenticated primary");
    output.terminal(AuthProgress::ServerRoster(ServerRosterProgress {
        epoch,
        expected,
        outcome: ServerRosterOutcome::Reconcile {
            resources,
            found,
            admitted_machine_id,
            household,
            settled,
        },
    }));
}

/// Replace only the route facts of one already-granted source.
///
/// This is deliberately narrower than [`refreshed_sources`]. A recovery probe may use the
/// install owner's account token merely to obtain the current connection list after the network
/// topology changes, while the live Plex Home profile owns a different per-server PMS token and a
/// smaller grant set. Therefore it may neither add/remove a source nor copy the Resource token:
/// it updates the verified origin, address and tier of the exact machine already in the profile.
fn apply_refreshed_endpoint(
    session: &mut Session,
    machine_id: &str,
    fresh: &SourceRef,
) -> Option<(SourceRef, bool)> {
    let source = session
        .sources
        .iter_mut()
        .find(|source| source.machine_id == machine_id)?;
    let next = SourceRef {
        address: fresh.address.clone(),
        port: fresh.port,
        origin_url: fresh.origin_url.clone(),
        tier: fresh.tier,
        // Grant/profile facts remain exactly the active profile's. In particular, `fresh.token`
        // may be the account owner's token when this recovery follows a managed-profile switch.
        token: source.token.clone(),
        machine_id: source.machine_id.clone(),
        name: source.name.clone(),
        shared_by: source.shared_by.clone(),
        owned: source.owned,
        home: source.home,
        owner_id: source.owner_id,
        extensions: source.extensions.clone(),
    };
    let changed = source.address != next.address
        || source.port != next.port
        || source.origin_url != next.origin_url
        || source.tier != next.tier;
    *source = next.clone();
    if session.server.machine_id == machine_id {
        reconcile_primary(&mut session.server, std::slice::from_ref(&next));
    }
    Some((next, changed))
}

fn probe_endpoint_work(
    id: ServerId,
    machine_id: &str,
    sess: &Session,
    mut resources: impl FnMut(&AccountClient, Duration) -> Result<Vec<Resource>, CallEvidence>,
    probe: impl FnOnce(&Resource, &[i64]) -> (Option<SourceRef>, SettledProbe),
    live: &dyn Fn() -> bool,
) -> (Option<SourceRef>, Option<SettledProbe>) {
    // Nothing was actually probed on any of the early exits below — plex.tv never answered, or
    // this machine is no longer among its resources — so there is no verdict to report at all:
    // fabricating `Unreachable` here would widen a real, more specific probe result
    // (`InsecureOnly`/`Unauthorized`) into "Not reachable" once it reached the registry.
    if !live() { return (None, None); }
    let ac = AccountClient::new(&sess.client_id, Some(&sess.account_token));
    let mut clock = LiveFnClock { live, started: Instant::now() };
    let run = retry_account_call(BACKGROUND_ACCOUNT, &mut clock, |_, _, _| {},
        |remaining| resources(&ac, remaining));
    let resources = match run.result {
        AccountCallEnd::Answer(resources) => resources,
        AccountCallEnd::Cancelled => return (None, None),
        AccountCallEnd::Failed(evidence) => {
            log(&format!(
                "auth: endpoint refresh for source {} could not list resources ({})",
                id.raw(),
                crate::plex::account::describe_evidence(&evidence)
            ));
            return (None, None);
        }
    };
    let Some(resource) = resources
        .iter()
        .find(|resource| resource.is_server() && resource.client_identifier == machine_id)
    else {
        log(&format!(
            "auth: endpoint refresh for source {} found no matching resource",
            id.raw()
        ));
        return (None, None);
    };
    if !live() { return (None, None); }
    let (fresh, probe) = probe(resource, &sess.household_ids());
    (fresh, Some(probe))
}

/// Point the persisted PRIMARY at wherever the refreshed roster says that machine now answers.
/// Returns whether anything moved, so the caller knows the save is owed.
///
/// [`Session::server`] and [`Session::sources`] are two records of the same servers and only the
/// second was being rewritten here, so the moment the primary PMS changed LAN address the two
/// disagreed permanently. Two symptoms, both durable and neither self-healing:
///
/// * `app.rs`'s boot gate dials `session.server`, so every boot went to the dead address first;
/// * and `plex::install` of that address registers a SECOND slot for a machine already in the table
///   — `servers::same_server` can only match on the address when the legacy `install` supplies no
///   machine id — with the dead copy made `current`. The house's own server, listed twice, the
///   working one not the one being used.
///
/// It cannot be fixed by re-running discovery either: the refresh persists `sources` only when they
/// changed, so the very first boot after the move wrote the new address into the roster and left
/// `server` stale, and every boot after that found the roster already correct and saved nothing.
/// That is why the reconcile is part of the CHANGED decision and not a rider on it.
///
/// Matched on `machine_id` and nothing else — the identity that survives an address moving is the
/// only thing that can decide this — and an empty id matches nothing, [`retoken`]'s rule: an entry
/// that cannot be identified must never match a resource that also happens to have no id.
fn reconcile_primary(server: &mut ServerRef, found: &[SourceRef]) -> bool {
    if server.machine_id.is_empty() {
        return false;
    }
    let Some(s) = found
        .iter()
        .find(|s| s.machine_id == server.machine_id && s.dialable())
    else {
        return false;
    };
    if server.address == s.address
        && server.port == s.port
        && server.token == s.token
        && server.origin_url == s.origin_url
        && server.tier == s.tier
    {
        return false;
    }
    // The line says the server MOVED, so it must not fire when only the stored origin was
    // LEARNED. A primary written before that field existed carries an empty one, so the first boot
    // after the upgrade populates it beside an identical address, port and token — a write, and not
    // news. Logging it would read as DHCP churn in the file this project treats as its primary
    // evidence surface, on every existing install, exactly once, which is the worst kind of false
    // positive: unreproducible afterwards.
    let learned_origin = server.origin_url.is_empty() && !s.origin_url.is_empty();
    let moved = server.address != s.address
        || server.port != s.port
        || (server.origin_url != s.origin_url && !learned_origin);
    if moved {
        // The machine name and the address, never the token and never the machine id — the same
        // line `SourceRef::describe` draws.
        log(&format!(
            "auth: primary {:?} now answers at {}:{}",
            server.name, s.address, s.port
        ));
    }
    server.address = s.address.clone();
    server.port = s.port;
    // The origin moves with the address for the same reason the token does: it came out of the
    // same answer. Leaving it behind would keep dialling the old one, which is the bug this
    // whole function exists to close, one field further in.
    server.origin_url = s.origin_url.clone();
    server.tier = s.tier;
    // The token moves with the address because it came from the same answer: this is the OWNER's
    // per-(user, server) grant, which is exactly what `ServerRef::token` means (and the refresh
    // above only runs for the owner). `pms_token()` still prefers a switched profile's own token.
    server.token = s.token.clone();
    true
}

/// Register a roster with the [server registry](crate::plex::register), optionally naming which
/// entry is the current server.
///
/// The registry is keyed on `machineIdentifier`, so this is idempotent: re-running discovery
/// re-points a server that moved rather than adding a second slot for it, and a re-registration at
/// the same address just swaps the token in place — which is what the ~30 call sites holding a
/// `&'static Client` rely on.
///
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
            crate::plex::ConnectionFacts::new(s.tier, crate::plex::IpVersion::of_host(&s.address));
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
        crate::plex::describe_server(id, &s.name, &s.shared_by, grant_of(s));
        if primary == Some(i) {
            crate::plex::set_current(id);
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

/// Re-key a stored roster to a newly switched profile.
///
/// `accessToken` is per **(user, server)**, so switching profile invalidates every stored token at
/// once, not only the primary's — a share left on the previous profile's token answers 401 to
/// everything. The switch already fetches `/api/v2/resources` as the new user to find the primary's
/// token, so this re-keys the whole roster from that same response: no extra round trip.
///
/// A source the response no longer names is retained only as TOKENLESS connection metadata. It is
/// therefore unusable and omitted by every registry/install walk, but a later switch back to a
/// profile that is granted it can restore the new token without having forgotten the verified
/// address while it was hidden. A brand new share is not added here: it has no probed address yet,
/// and inventing one is what discovery is for. An entry with no machine id is dropped entirely: it
/// cannot be identified, and emptiness must never match another empty id.
#[cfg(test)]
fn retoken(sources: &[SourceRef], resources: &[Resource]) -> Vec<SourceRef> {
    sources
        .iter()
        .filter(|s| !s.machine_id.is_empty())
        .map(|s| {
            let token = resources
                .iter()
                .find(|r| r.is_server() && r.client_identifier == s.machine_id)
                .map(|r| r.access_token.clone())
                .unwrap_or_default();
            SourceRef { token, ..s.clone() }
        })
        .collect()
}

#[cfg(test)]
#[path = "auth_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "auth_discovery_tests.rs"]
mod discovery_tests;

#[cfg(test)]
#[path = "auth_registry_tests.rs"]
mod registry_tests;

#[cfg(test)]
#[path = "auth_storage_extension_tests.rs"]
mod storage_extension_tests;
