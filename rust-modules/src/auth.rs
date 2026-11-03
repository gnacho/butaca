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
use crate::plex::session::{PlaintextChoice, SourceRef};
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

/// Append a line to the shared on-device event log (never a token — only ids/counts/status).

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
    pub(crate) fn of(s: &crate::plex::session::Session) -> Self {
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

/// Canon-only like `RegistryProgress` above: a roster observation no worker produces, kept for
/// the frozen observation vocabulary old recordings replay.
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
    /// all). The owner commits these as observation-only probes: the roster itself
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

/// Canon-only (no producer): the observation vocabulary is frozen. The endpoint progress carried
/// the registry's `ClientLifecycle`; that vocabulary went with the wire layer (#166) and the
/// struct with it — the frozen canon digest never named it field-by-field, only through the
/// owner's logical `ServerLifecycle` (sid + instance_gen + token_gen), which survives.
#[allow(dead_code)]
pub(crate) struct EndpointProgress {
    epoch: u64,
    expected: SessionIdentity,
    id: ServerId,
    machine_id: String,
    fresh: Option<SourceRef>,
    probe: Option<SettledProbe>,
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

