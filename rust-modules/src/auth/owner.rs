//! Concrete auth decisions and immutable publications. Resource operations belong to the
//! application adapter; constructing or observing this value performs no external work.

use super::{Phase, Picker, UserTile};
use crate::plex::session::async_persistence::{PersistencePurpose, RejectionKind};
use crate::plex::session::{Session as PersistedSession, UserRef};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use crate::ui::machine::{Addr, Canon, LogicalState, MachineId};


pub(crate) const SESSION_DATA_RECORDS: usize = 64;
pub(crate) const SESSION_TOTAL_RESERVATIONS: u32 = 32;
pub(crate) const SESSION_TRANSFER_RECORDS: usize = SESSION_DATA_RECORDS + SESSION_TOTAL_RESERVATIONS as usize;

/// A credit for one distinct transferred record, not a worker-completion acknowledgement.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Receipt {
    pub arrival: u64,
    #[serde(with = "super::observation::address")]
    pub addr: Addr,
    pub key: SessionWorkKey,
}

impl Receipt {
    pub fn of(envelope: &SessionEnvelope) -> Self {
        Self { arrival: envelope.arrival, addr: envelope.addr, key: envelope.key }
    }
    fn write(&self, w: &mut Canon) {
        self.addr.to.write_canon(w);
        w.u32(self.addr.req.0).u64(self.arrival).u64(self.key.epoch);
        write_op(w, self.key.op);
    }
}

pub(super) fn write_purpose(w: &mut Canon, purpose: PersistencePurpose) {
    w.u8(match purpose {
        PersistencePurpose::Discovery => 0,
        PersistencePurpose::Final => 1,
        PersistencePurpose::Profile => 2,
        PersistencePurpose::Background => 3,
    });
}

fn write_op(w: &mut Canon, op: SessionOp) {
    match op {
        SessionOp::ServerRoster => { w.u8(3); }
        SessionOp::Endpoint(sid) => { w.u8(5).u32(u32::from(sid)); }
        SessionOp::Ready => { w.u8(6); }
    }
}

/// Delivery keeps the exact request even though generic non-instance delivery drops its outer
/// request field. The application verifies the outer address before constructing this event.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SessionEnvelope {
    #[serde(with = "super::observation::address")]
    pub addr: Addr,
    pub key: SessionWorkKey,
    pub admission: AdmissionId,
    pub arrival: u64,
    pub terminal: bool,
    pub lifecycle: Option<ServerLifecycle>,
    pub outcome: SessionArrival,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum SessionArrival {
    Data(#[serde(with = "super::observation::arc")] Arc<super::observation::Observation>),
    Refused,
    Dropped,
}

impl SessionEnvelope {
    fn write(&self, w: &mut Canon) {
        Receipt::of(self).write(w);
        w.u32(self.admission.0).bool(self.terminal);
        w.option(self.lifecycle, |w, life| {
            w.u32(u32::from(life.sid)).u32(life.instance_gen).u32(life.token_gen);
        });
        match &self.outcome {
            SessionArrival::Data(data) => { w.u8(0); data.write(w); }
            SessionArrival::Refused => { w.u8(1); }
            SessionArrival::Dropped => { w.u8(2); }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ProfilePublication {
    pub epoch: u64,
    pub profile: Option<UserRef>,
    pub scope: ProfileScope,
}

/// Credentials-owned patch data. The adapter merges it into the current disk value, retaining
/// newer favourite/search/quality/ambient fields owned by other machines.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CredentialPatch {
    pub client_id: String,
    pub account_token: String,
    pub server: crate::plex::session::ServerRef,
    pub user: UserRef,
    pub home_users: Vec<crate::plex::session::HomeUserRef>,
    pub sources: Vec<crate::plex::session::SourceRef>,
    pub profiles: Vec<crate::plex::session::ProfileCreds>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum Command {
    TakeReady,
    /// Answer the currently shown [`PersistenceWarning`]. A key that does not match the warning
    /// currently held is inert — it may be stale (a newer warning replaced it).
    AcknowledgePersistenceWarning { key: PersistenceWarningKey },
    SignOut,
    EraseLocal,
    NoteDeleteLeftovers(usize),
    /// The person's answer to "Connect without encryption?" (or Settings' switch) for one server
    /// — every consent surface sends this one command. Signed in, it answers a server Home, the
    /// Library or Settings showed (`grant::offers`, or an earlier answer), and an *Allowed*
    /// re-finds `sid`'s endpoint — or the whole roster when the server has no slot — so a fresh
    /// probe can mint the grant.
    AnswerPlaintext {
        machine_id: String,
        choice: crate::plex::session::PlaintextChoice,
        #[serde(with = "super::observation::optional_server_id")]
        sid: Option<crate::registry::ServerId>,
    },
}

/// Ordered application/coordinator work; the owner describes it without invoking another/// machine, platform API or global publication from inside its transition.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) enum CoordinatorAction {
    LocalDataErased,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum RegistryPlan {
    Install { sources: Vec<crate::plex::session::SourceRef>, primary: Option<usize>, replace: bool },
}

/// Which site's fresh save produced the warning being shown to the user — the discovery write
/// that ran right after sign-in, or the final write `take_ready` issues once discovery settles.
/// Both are `FreshReauthentication` writes; only the site differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PersistenceWarningSite { Discovery, Final }

/// Identity of the fresh write a `PersistenceWarning` is reporting on, so an acknowledgement can
/// be checked against the exact warning it is answering rather than any warning currently shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersistenceWarningKey { pub epoch: u64, pub req: u32 }

/// A fresh-reauthentication write that did NOT confirm durable, surfaced to the owner/UI and held
/// until the user explicitly acknowledges it — the AUTH-03 gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersistenceWarning {
    pub key: PersistenceWarningKey,
    pub site: PersistenceWarningSite,
    #[serde(default)]
    pub helper: Option<crate::storage::wire::failure::HelperFailure>,
    #[serde(default)]
    pub candidate_errnos: [Option<i32>; 8],
    /// Closed evidence for the warning; retained independently of the consent-gated report.
    #[serde(default)]
    pub persistence: Option<PersistenceEvidence>,
}

/// Why saving (or re-reading) the sign-in failed, from the session persistence completion.
/// Closed codes only — the same classification the removed telemetry module reported, kept
/// because the persistence warning's read-out and the repair flow use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PersistenceFailure {
    /// The storage worker refused the job.
    Admission,
    /// The write itself failed.
    WriteFailed,
    /// The store refused or failed the record.
    Storage,
    /// The commit may or may not have reached the disk.
    CommitUncertain,
    /// Sealing the credentials with the platform key service failed.
    Protection,
    /// Sealing may or may not have happened.
    ProtectionUncertain,
    /// The worker went away before answering.
    WorkerDropped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersistenceEvidence {
    class: PersistenceFailure,
    keymanager_stage: Option<crate::storage::wire::KeymanagerStage>,
    service_error_code: Option<i32>,
}

impl PersistenceWarning {
    pub(crate) fn from_outcome(key: PersistenceWarningKey, site: PersistenceWarningSite,
        outcome: &crate::plex::session::async_persistence::CompletionOutcome) -> Self {
        use crate::plex::session::async_persistence::{CompletionOutcome, Failure};
        let (class, protection) = match outcome {
            CompletionOutcome::Durable(_)
            | CompletionOutcome::Superseded
            | CompletionOutcome::Failed(Failure::Superseded) => (None, None),
            CompletionOutcome::Uncertain { .. } => (Some(PersistenceFailure::CommitUncertain), None),
            CompletionOutcome::ProtectionUncertain(p) =>
                (Some(PersistenceFailure::ProtectionUncertain), Some(p)),
            CompletionOutcome::Failed(Failure::Admission(_)) =>
                (Some(PersistenceFailure::Admission), None),
            CompletionOutcome::Failed(Failure::Persistence(_)) =>
                (Some(PersistenceFailure::WriteFailed), None),
            CompletionOutcome::Failed(Failure::Storage(_) | Failure::Helper(..)) =>
                (Some(PersistenceFailure::Storage), None),
            CompletionOutcome::Failed(Failure::Protection(p)) =>
                (Some(PersistenceFailure::Protection), Some(p)),
            CompletionOutcome::Failed(Failure::WorkerDropped) =>
                (Some(PersistenceFailure::WorkerDropped), None),
        };
        let (helper, candidate_errnos) = outcome.helper_evidence();
        Self {
            key, site, helper, candidate_errnos,
            persistence: class.map(|class| PersistenceEvidence {
                class,
                keymanager_stage: protection.map(|p| p.failure.stage),
                service_error_code: protection.and_then(|p| p.failure.service_code),
            }),
        }
    }

}

/// A Ready handoff whose fresh final write has been admitted but not yet confirmed durable, or
/// confirmed NOT durable and awaiting the user's acknowledgement. Nothing is emitted for it until
/// `release_held_handoff` runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HeldHandoff { pub epoch: u64, pub req: u32 }

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CommitPlan {
    /// Identity captured for this login, independent of the old disk identity used for OCC.
    pub registry_client_id: String,
    pub expected_disk: Identity,
    pub credentials: Option<CredentialPatch>,
    pub registry: Vec<RegistryPlan>,
    pub lifecycle: Option<ServerLifecycle>,
    /// What a durable write would be FOR. The caller uses this to refuse treating a background
    /// refresh as proof of a saved login, and the completion echoes it back.
    pub purpose: PersistencePurpose,
    /// Whether this commit asks storage for a durable write. Registry-only commits do not.
    pub writes_durable: bool,
    /// `Routine` for every ordinary write; `FreshReauthentication` only for a write the owner
    /// issues on the strength of THIS flow's own PIN authorization (a completed sign-in or
    /// device-code exchange) — never on a discovery/rediscovery retry that merely reuses it.
    pub authority: crate::plex::session::SaveAuthority,
}

/// Only the changes whose side effects are awaiting acknowledgement. This is not a second
/// controller snapshot: unchanged fields remain solely in SessionInit.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct CommitDelta {
    pub credentials: Option<CredentialPatch>,
    pub phase: Option<Phase>,
    pub picker: Option<Picker>,
    pub users: Option<Vec<UserTile>>,
    pub clear_error: bool,
    pub activate_profile: bool,
    pub complete_signin: bool,
    pub profile_seated: bool,
    pub ready: Option<bool>,
    /// plex.tv answered this identity with a roster (a sign-in's own, or the picker's refresh):
    /// the question [`SessionInit::switch_refused_for`] recorded a refusal to is answered again,
    /// so the verdict goes with the commit that carries the answer.
    #[serde(default)]
    pub roster_answered: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum BootstrapAuthority {
    Account { extras: Vec<crate::plex::session::SourceRef> },
    /// The controlled Jellyfin world (#166 replay flip). There is no plex.tv account surface
    /// behind it: no registry install, no token rotation, no profile machinery — the typed
    /// Initial carries the exact client identity the controlled boot installs unprobed, and
    /// every fetch the recording admits rides that one origin. `activate_dev` refuses it, so
    /// the owner never leaves `Idle` in this world; the boot arm reads the identity itself.
    /// Values are synthetic (see `Initial::synthetic_home`), never household bytes.
    DevJf { origin: String, token: String, user_id: String, user_name: String },
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum ReadyInstall {
    PrimaryAndExtras(Vec<crate::plex::session::SourceRef>),
    AlreadyInstalled,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PendingCommit {
    pub req: u32,
    pub epoch: u64,
    pub arrival: u64,
    pub terminal: bool,
    pub writes_credentials: bool,
    pub receipt: Option<Receipt>,
    pub delta: CommitDelta,
    /// The revision an admitted durable write consumed, if any. A completion must repeat it
    /// exactly; a completion naming any other revision belongs to a different operation.
    pub admitted_revision: Option<u64>,
    /// The purpose that admitted write was for. A background refresh is never saved-login
    /// evidence, so this is what the completion is graded against.
    pub purpose: Option<PersistencePurpose>,
    /// Whether this commit's write is on `SaveAuthority::FreshReauthentication` — a durable
    /// confirmation for one of these is held rather than announced until acknowledged.
    pub fresh: bool,
}

/// Owner-side durability state of one commit consumption. Replaces the old boolean that advertised
/// `accepted` for a registry-only commit that never asked storage for anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CommitPhase {
    /// Authority was current; a durable write was admitted and enqueued, not yet durable.
    pub(crate) admitted: bool,
    /// Storage CONFIRMED durability for the admitted revision. Still false while merely enqueued,
    /// and still false for a completion that was fenced out.
    pub(crate) durable: bool,
    /// That confirmed durable write had a purpose which may stand as evidence a login was saved.
    /// A background refresh leaves this false even when it becomes durable.
    pub(crate) proves_saved_login: bool,
}

/// The durable write this owner is waiting on, identified exactly. A completion settles only this
/// identity; anything else is a stale or superseded verdict and must change nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AdmittedPersistence {
    pub req: u32,
    pub epoch: u64,
    pub arrival: u64,
    pub revision: u64,
    /// Purpose of the admitted write, when the plan named one.
    pub purpose: Option<PersistencePurpose>,
    /// Whether the admitted write is on `SaveAuthority::FreshReauthentication`. A non-durable
    /// completion for a fresh write raises a `PersistenceWarning`; a routine one does not.
    pub fresh: bool,
    /// Which site (`Discovery` or `Final`) the fresh write belongs to, for the warning it may
    /// raise. Meaningless when `fresh` is false.
    pub site: PersistenceWarningSite,
}

/// Typed result of consuming a commit permit. The four cases are deliberately distinct: treating
/// "the authority was current" as "durable" is the defect this type exists to prevent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CommitAdmission {
    /// The permit was not current (wrong request/epoch/arrival/endpoint lifecycle). Nothing ran.
    StaleAuthority,
    /// Authority was current and the commit asked for no durable write (registry-only).
    RegistryOnly,
    /// Authority was current and a durable write was admitted and enqueued. NOT yet durable.
    Admitted { revision: u64, purpose: PersistencePurpose },
    /// Authority was current but capacity/duplicate/lock/public-only refused the write.
    Rejected { revision: Option<u64>, rejection: RejectionKind },
}

impl CommitAdmission {
    pub(crate) fn accepted(self) -> bool {
        !matches!(self, Self::StaleAuthority | Self::Rejected { .. })
    }
    pub(crate) fn admitted_revision(self) -> Option<u64> {
        match self {
            Self::Admitted { revision, .. } => Some(revision),
            _ => None,
        }
    }
    /// The purpose storage was actually asked to write for. This is the authoritative purpose for
    /// fencing a completion: the plan's intent and the admission can disagree, and only what was
    /// enqueued can be answered.
    pub(crate) fn admitted_purpose(self) -> Option<PersistencePurpose> {
        match self {
            Self::Admitted { purpose, .. } => Some(purpose),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) struct CommitReply {
    pub req: u32,
    pub epoch: u64,
    pub arrival: u64,
    pub admission: CommitAdmission,
}

/// Execution-time permission borrowed from the sole owner. Not serialized state or an effect:
/// keeping it alive prevents changing the owner while its separate adapter commits resources.
pub(crate) struct CommitPermit<'a> {
    req: u32,
    epoch: u64,
    arrival: u64,
    owner: std::marker::PhantomData<&'a SessionMachine>,
}

impl CommitPermit<'_> {
    pub fn request(&self) -> u32 { self.req }
    /// The epoch and arrival this permit was issued for. The adapter needs them so the durability
    /// verdict it reports can be fenced against the exact operation that produced it.
    pub fn epoch(&self) -> u64 { self.epoch }
    pub fn arrival(&self) -> u64 { self.arrival }
    pub fn reply(self, admission: CommitAdmission) -> CommitReply {
        CommitReply { req: self.req, epoch: self.epoch, arrival: self.arrival, admission }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum CaptureIntent {
    Endpoint { sid: u16 },
}

/// One launch per checked request. The explicit result state distinguishes "accepted, no
/// observations yet" from a never-admitted request; an arrival watermark cannot do that.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AdmissionId(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AdmissionState {
    NotRequested,
    Awaiting(AdmissionId),
    Accepted(AdmissionId),
}

/// Every variant is data. Closures, native Clients and MainThread cannot enter logical effects.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum SessionFx {
    Commit { req: u32, epoch: u64, arrival: u64, plan: CommitPlan },
    Pump,
    Acknowledge(Vec<Receipt>),
    Retire { req: u32 },
    Ready { epoch: u64, scope: ProfileScope, server: crate::plex::session::ServerRef, token: String, install: ReadyInstall },
    Cancel { requests: Vec<u32>, epoch: u64 },
    PublishProfile(ProfilePublication),
    Erase { req: u32, epoch: u64, all_local: bool },
    Coordinator(CoordinatorAction),
}

pub(crate) trait SessionHost: crate::ui::machine::Host {
    fn session_effect(effect: SessionFx) -> Self::Fx;
}

pub(crate) enum SessionEvent {
    Command(Command),
    Commit(CommitReply),
    /// Resource evidence is delivered even when its original authority permit is gone.
    DiskWrite { before: Identity, after: Identity,
        outcome: crate::plex::session::async_persistence::CompletionOutcome },
    Pump,
    /// A typed durability verdict from the persistence worker, delivered through the ordinary
    /// effect/FIFO path. It is fenced by request/epoch/arrival/revision before it settles anything.
    Persistence(crate::plex::session::async_persistence::PersistenceCompletion),
    Erased { epoch: u64, leftovers: usize },
}

impl CredentialPatch {
    pub fn identity(&self) -> Identity {
        Identity { client_id: self.client_id.clone(), account_token: self.account_token.clone(),
            profile_uuid: self.user.uuid.clone() }
    }
    pub fn of(s: &PersistedSession) -> Self {
        Self { client_id: s.client_id.clone(), account_token: s.account_token.clone(),
            server: s.server.clone(), user: s.user.clone(), home_users: s.home_users.clone(),
            sources: s.sources.clone(), profiles: s.profiles.clone() }
    }
    pub fn merge_into(&self, disk: &PersistedSession) -> PersistedSession {
        PersistedSession { client_id: self.client_id.clone(), account_token: self.account_token.clone(),
            server: self.server.clone(), user: self.user.clone(), home_users: self.home_users.clone(),
            sources: self.sources.clone(), profiles: self.profiles.clone(), ..disk.clone() }
    }
}

/// Serializable identity of a registry resource, never the native Client pointer.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ServerLifecycle {
    pub sid: u16,
    pub instance_gen: u32,
    pub token_gen: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SessionOp { ServerRoster, Endpoint(u16), Ready }

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionWorkKey {
    pub epoch: u64,
    pub op: SessionOp,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Identity {
    pub client_id: String,
    pub account_token: String,
    pub profile_uuid: String,
}

impl Identity {
    pub fn of(session: &PersistedSession) -> Self {
        Self { client_id: session.client_id.clone(), account_token: session.account_token.clone(),
            profile_uuid: session.user.uuid.clone() }
    }
    pub fn matches(&self, session: &PersistedSession) -> bool {
        self.client_id == session.client_id && self.account_token == session.account_token
            && self.profile_uuid == session.user.uuid
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum StreamPhase { Running, ProfileSeated }

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Pending {
    pub key: SessionWorkKey,
    pub expected: Identity,
    pub lifecycle: Option<ServerLifecycle>,
    pub last_arrival: Option<u64>,
    pub phase: StreamPhase,
    pub capture: Option<CaptureIntent>,
    pub admission: AdmissionState,
}

/// Only the owner may allocate this generation; adapters publish the supplied value verbatim.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProfileScope(pub u32);

/// Private persisted init data, not diagnostic output. No constructor consults the filesystem.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SessionInit {
    pub authority: BootstrapAuthority,
    /// Comparison-only disk baseline; NEVER an input to workers, profile selection or BACK.
    pub disk_identity: Identity,
    pub phase: Phase,
    pub picker: Picker,
    pub persisted: PersistedSession,
    /// Last accepted credential commit, distinct from a QR token or profile candidate which has
    /// not been saved yet. BACK may resume this version; it never resumes a provisional token.
    pub committed_credentials: CredentialPatch,
    pub pin_code: String,
    pub qr_png: Vec<u8>,
    pub users: Vec<UserTile>,
    pub error: String,
    pub pin_denied: bool,
    pub authorized_in_flow: bool,
    pub signin_active: bool,
    pub apply_pending: bool,
    pub code_replaced: bool,
    pub qr_gen: u64,
    pub next_qr: u64,
    pub epoch: u64,
    pub next_req: u32,
    pub pending: BTreeMap<u32, Pending>,
    pub pending_commit: Option<PendingCommit>,
    /// Purpose of the commit currently being resolved, if it asked for durability.
    pub persistence_purpose: Option<PersistencePurpose>,
    /// Durability state of the last consumed commit.
    pub commit_phase: CommitPhase,
    /// Identity of the durable write this owner is waiting to hear about. Cleared once a matching
    /// completion settles it, or superseded by a newer admission. Only this identity can settle it.
    pub admitted_persistence: Option<AdmittedPersistence>,
    /// A fresh write that did not confirm durable, held for acknowledgement (AUTH-03). While
    /// present, `take_ready` refuses to issue a second fresh write over it.
    pub persistence_warning: Option<PersistenceWarning>,
    /// A Ready handoff whose fresh write is admitted but not yet released to the owner/UI.
    pub held_handoff: Option<HeldHandoff>,
    /// Whether THIS authorization has already had one unsaved-login (AUTH-03) warning answered.
    /// Authorization succeeding and credentials being savable are different conditions: once the
    /// user has answered for this authorization, a later storage failure (Discovery then Final,
    /// or a retry) must release its own held handoff rather than raise the same question again.
    /// Per-authorization, so it is cleared everywhere `persistence_warning`/`held_handoff` reset
    /// together — a new sign-in must start with a fresh, unanswered question.
    pub persistence_warning_answered: bool,
    /// The disk identity a fresh credential write replaced at admission, until that write proves
    /// durable. A definite failure restores it: the record never changed, and fencing the next
    /// fresh write on the identity that never landed would refuse every retry this run.
    #[serde(default)]
    pub unconfirmed_fresh_prior: Option<Identity>,
    /// One ordered erase awaiting resource completion: existing epoch and whether to sign in.
    pub pending_erase: Option<(u64, bool)>,
    pub inbox: VecDeque<SessionEnvelope>,
    /// Covers both the queued Fx::Pump and its eventual typed delivery. Cancellation does not
    /// forget a marker already in the dispatcher; it can safely pump a newer current inbox.
    pub pump_pending: bool,
    pub active_profile: Option<UserRef>,
    pub profile_scope: ProfileScope,
    pub delete_leftovers: usize,
    /// plex.tv has left at least two consecutive polls of the code on screen unanswered.
    #[serde(default)]
    pub link_trouble: bool,
    /// The one account retry run currently outstanding during discovery. Its elapsed value is
    /// sampled at the last miss; the screen advances from that anchor with its frame clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_retry: Option<super::DiscoveryRetryProgress>,
    /// The server the failure read-out may offer a consented plaintext connection to, with the
    /// person's answer so far (`auth::PlaintextVerdict`). Set only by an insecure-only discovery
    /// failure whose server is eligible; cleared by every other ending and by a new attempt.
    #[serde(default)]
    pub plaintext: Option<super::PlaintextVerdict>,
    /// The identity plex.tv REFUSED a roster to while nothing was cached to switch from — the
    /// verdict behind [`roster_refused`]'s read-out, kept so the account menu stops offering
    /// *Change profile* into that dead end ([`SessionSnapshot::switch_refused`]). Keyed by the
    /// identity, so a profile switch to anyone else reads it as unrefused. The identity's
    /// lifecycle retires it outright: `erase` (sign-out; the same account signed back in is the
    /// same `Identity`), and any commit carrying a roster for it
    /// ([`CommitDelta::roster_answered`] — a sign-in's own or the picker's refresh). In memory
    /// only: a relaunch asks plex.tv again.
    #[serde(default)]
    pub switch_refused_for: Option<Identity>,
}

impl SessionInit {
    pub fn captured(persisted: PersistedSession) -> Self {
        let committed_credentials = CredentialPatch::of(&persisted);
        let disk_identity = Identity::of(&persisted);
        Self { authority: BootstrapAuthority::Account { extras: Vec::new() }, disk_identity,
            phase: Phase::Idle, picker: Picker::Boot, persisted, committed_credentials,
            pin_code: String::new(), qr_png: Vec::new(), users: Vec::new(), error: String::new(),
            pin_denied: false, authorized_in_flow: false, signin_active: false, apply_pending: false,
            code_replaced: false, qr_gen: 0, next_qr: 0, epoch: 1, next_req: 0,
            pending: BTreeMap::new(), pending_commit: None, persistence_purpose: None,
            commit_phase: CommitPhase { admitted: false, durable: false, proves_saved_login: false }, admitted_persistence: None,
            persistence_warning: None, held_handoff: None, persistence_warning_answered: false,
            unconfirmed_fresh_prior: None, pending_erase: None, inbox: VecDeque::new(), pump_pending: false,
            active_profile: None, profile_scope: ProfileScope(0),
            delete_leftovers: 0,
            link_trouble: false, discovery_retry: None, plaintext: None, switch_refused_for: None }
    }
}

pub(super) fn write_plaintext_verdict(w: &mut Canon, v: &super::PlaintextVerdict) {
    use crate::plex::session::PlaintextChoice as C;
    w.str(&v.machine_id).str(&v.name).str(&v.shared_by).str(v.eligibility.code())
        .u8(match v.choice { C::Undecided => 0, C::Allowed => 1, C::Declined => 2, C::Revoked => 3 });
}

pub(super) fn write_user(w: &mut Canon, user: &UserRef) {
    w.u64(user.id as u64).str(&user.uuid).str(&user.title).str(&user.thumb).str(&user.token);
    w.option(user.plex_tv_token.as_ref(), |w, t| { w.str(t); });
}

pub(super) fn write_profile(w: &mut Canon, profile: &crate::plex::session::ProfileCreds) {
    w.str(&profile.uuid);
    write_user(w, &profile.user);
    write_server(w, &profile.server);
    write_sources(w, &profile.sources);
    w.option(profile.pin.as_ref(), |w, pin| { w.str(&pin.salt).str(&pin.hash).u32(pin.iters); });
}

pub(super) fn write_server(w: &mut Canon, server: &crate::plex::session::ServerRef) {
    w.str(&server.name).str(&server.machine_id).str(&server.address)
        .u64(server.port as u64).str(&server.token).str(&server.origin_url);
    write_tier(w, server.tier);
}

pub(super) fn write_tier(w: &mut Canon, tier: Option<crate::plex::probe::Location>) {
    use crate::plex::probe::Location;
    w.u8(match tier { None => 0, Some(Location::Local) => 1,
        Some(Location::Remote) => 2, Some(Location::Relay) => 3 });
}

pub(super) fn write_sources(w: &mut Canon, sources: &[crate::plex::session::SourceRef]) {
    w.seq(sources.len());
    for s in sources {
        // The carried household EVIDENCE joins the canonical digest beside raw `owned`. It has to:
        // a state whose only difference is that plex.tv now names the grant's owner is a different
        // state, and a digest blind to it would grade the correction as no change at all.
        w.str(&s.machine_id).str(&s.name).str(&s.shared_by).bool(s.owned)
            .bool(s.home).u64(s.owner_id as u64)
            .str(&s.address).u64(s.port as u64).str(&s.token).str(&s.origin_url);
        write_tier(w, s.tier);
    }
}

/// Explicit field encoding. Serde is only the private init round-trip, never the state hash.
pub(super) fn write_persisted(w: &mut Canon, s: &PersistedSession) {
    w.str(&s.client_id).str(&s.account_token);
    write_server(w, &s.server);
    write_user(w, &s.user);
    write_sources(w, &s.sources);
    w.seq(s.home_users.len());
    for user in &s.home_users {
        w.u64(user.id as u64).str(&user.uuid).str(&user.title).str(&user.thumb)
            .bool(user.protected).bool(user.admin);
    }
    w.seq(s.profiles.len());
    for profile in &s.profiles { write_profile(w, profile); }
    // Preferences are captured input retained by the controller, not authority for disk writes.
    // Include their exact captured values in init/canonical state even though patches never write
    // them over a newer store-owned value.
    w.bool(s.auto_sign_in());
    if s.language != crate::i18n::Preference::System {
        w.str("language").str(s.language.tag());
    }
    w.seq(s.recent_searches.len());
    for recent in &s.recent_searches {
        w.str(&recent.user).seq(recent.terms.len());
        for term in &recent.terms { w.str(term); }
    }
    w.seq(s.last_library.len());
    for last in &s.last_library {
        w.str(&last.user).seq(last.libs.len());
        for lib in &last.libs { w.str(&lib.kind).str(&lib.machine_id).u64(lib.key as u64); }
    }
    w.seq(s.home_pins.len());
    for pins in &s.home_pins {
        w.str(&pins.user).bool(pins.asked);
        for list in [&pins.on, &pins.off] {
            w.seq(list.len());
            for lib in list { w.str(&lib.machine_id).u64(lib.key as u64); }
        }
    }
    w.option(s.last_hero_blur, |w, blur| {
        for corner in blur { for channel in corner { w.f32(channel); } }
    });
    w.option(s.playback_quality, |w, quality| {
        use crate::plex::session::PlaybackQuality;
        w.u8(match quality { PlaybackQuality::Auto => 0, PlaybackQuality::Original => 1,
            PlaybackQuality::P1080High => 2, PlaybackQuality::P1080 => 3,
            PlaybackQuality::P720 => 4, PlaybackQuality::P720Low => 5, PlaybackQuality::P480 => 6 });
    });
}

impl LogicalState for SessionInit {
    fn write(&self, w: &mut Canon) {
        w.str(&self.disk_identity.client_id).str(&self.disk_identity.account_token)
            .str(&self.disk_identity.profile_uuid);
        match &self.authority {
            BootstrapAuthority::Account { extras } => { w.u8(0); write_sources(w, extras); }
            BootstrapAuthority::DevJf { origin, token, user_id, user_name } => {
                w.u8(2); w.str(origin).str(token).str(user_id).str(user_name);
            }
        }
        w.u8(match self.phase { Phase::Idle => 0,
            Phase::Ready => 6, Phase::Error => 7, Phase::Deleted => 8 });
        w.u8(match self.picker { Picker::Boot => 0, Picker::SignedIn => 1, Picker::ChangeProfile => 2 });
        write_persisted(w, &self.persisted);
        write_persisted(w, &self.committed_credentials.merge_into(&PersistedSession::default()));
        w.str(&self.pin_code).seq(self.qr_png.len());
        for byte in &self.qr_png { w.u8(*byte); }
        w.seq(self.users.len());
        for user in &self.users {
            w.u64(user.id as u64).str(&user.uuid).str(&user.title).str(&user.thumb)
                .bool(user.protected).bool(user.admin);
        }
        w.str(&self.error).bool(self.pin_denied).bool(self.authorized_in_flow)
            .bool(self.signin_active).bool(self.apply_pending).bool(self.code_replaced)
            .u64(self.qr_gen).u64(self.next_qr).u64(self.epoch).u32(self.next_req)
            .u64(self.delete_leftovers as u64).u32(self.profile_scope.0);
        w.option(self.active_profile.as_ref(), write_user);
        w.seq(self.pending.len());
        for (&req, pending) in &self.pending {
            w.u32(req).u64(pending.key.epoch);
            write_op(w, pending.key.op);
            w.str(&pending.expected.client_id).str(&pending.expected.account_token)
                .str(&pending.expected.profile_uuid);
            w.option(pending.lifecycle, |w, life| { w.u32(life.sid as u32)
                .u32(life.instance_gen).u32(life.token_gen); });
            w.option(pending.last_arrival, |w, arrival| { w.u64(arrival); });
            w.u8(match pending.phase { StreamPhase::Running => 0, StreamPhase::ProfileSeated => 1 });
            match pending.admission {
                AdmissionState::NotRequested => { w.u8(0); }
                AdmissionState::Awaiting(id) => { w.u8(1).u32(id.0); }
                AdmissionState::Accepted(id) => { w.u8(2).u32(id.0); }
            }
            w.option(pending.capture.as_ref(), |w, capture| match capture {
                CaptureIntent::Endpoint { sid } => { w.u8(2).u32(u32::from(*sid)); }
            });
        }
        w.option(self.persistence_purpose, |w, purpose| { write_purpose(w, purpose); });
        w.bool(self.commit_phase.admitted);
        w.bool(self.commit_phase.durable);
        w.bool(self.commit_phase.proves_saved_login);
        w.option(self.persistence_warning.as_ref(), |w, warning| {
            w.u64(warning.key.epoch).u32(warning.key.req).u8(match warning.site {
                PersistenceWarningSite::Discovery => 0,
                PersistenceWarningSite::Final => 1,
            });
            w.option(warning.persistence, |w, evidence| { w.str(&serde_json::to_string(&evidence).unwrap()); });
            w.option(warning.helper, |w, helper| { w.str(&serde_json::to_string(&helper).unwrap()); });
            for errno in warning.candidate_errnos { w.option(errno, |w, n| { w.u32(n as u32); }); }
        });
        w.option(self.held_handoff.as_ref(), |w, held| { w.u64(held.epoch).u32(held.req); });
        w.bool(self.persistence_warning_answered);
        w.option(self.pending_commit.as_ref(), |w, commit| {
            w.u32(commit.req).u64(commit.epoch).u64(commit.arrival).bool(commit.terminal)
                .bool(commit.writes_credentials);
            w.option(commit.receipt.as_ref(), |w, receipt| receipt.write(w));
            let delta = &commit.delta;
            w.option(delta.credentials.as_ref(), |w, patch| {
                write_persisted(w, &patch.merge_into(&PersistedSession::default()));
            });
            w.option(delta.phase, |w, phase| { w.u8(match phase {
                Phase::Idle => 0,
                Phase::Ready => 6, Phase::Error => 7, Phase::Deleted => 8,
            }); });
            w.option(delta.picker, |w, picker| { w.u8(match picker {
                Picker::Boot => 0, Picker::SignedIn => 1, Picker::ChangeProfile => 2,
            }); });
            w.option(delta.users.as_ref(), |w, users| {
                w.seq(users.len());
                for user in users {
                    w.u64(user.id as u64).str(&user.uuid).str(&user.title).str(&user.thumb)
                        .bool(user.protected).bool(user.admin);
                }
            });
            w.bool(delta.clear_error).bool(delta.activate_profile).bool(delta.complete_signin)
                .bool(delta.profile_seated).bool(delta.roster_answered);
            w.option(delta.ready, |w, ready| { w.bool(ready); });
        });
        w.option(self.pending_erase, |w, (epoch, sign_in)| { w.u64(epoch).bool(sign_in); });
        w.bool(self.pump_pending).seq(self.inbox.len());
        for envelope in &self.inbox { envelope.write(w); }
        w.bool(self.link_trouble);
        w.option(self.switch_refused_for.as_ref(), |w, id| {
            w.str(&id.client_id).str(&id.account_token).str(&id.profile_uuid);
        });
        // Appended only when present, so every state without an offer keeps its digest.
        if let Some(verdict) = &self.plaintext {
            write_plaintext_verdict(w, verdict);
        }
        if let Some(retry) = self.discovery_retry {
            w.u8(match retry.run {
                super::DiscoveryRetryRun::Resources => 0,
                super::DiscoveryRetryRun::HomeUsers => 1,
            }).u32(retry.misses).u32(retry.elapsed_ms);
        }
    }
    fn probe(&self, out: &mut String) {
        use std::fmt::Write;
        let _ = write!(out, "session phase={:?} requests={} scope={}",
            self.phase, self.pending.len(), self.profile_scope.0);
    }
}

impl LogicalState for SessionMachine {
    fn write(&self, w: &mut Canon) { self.state.write(w); }
    fn probe(&self, out: &mut String) { self.state.probe(out); }
}

/// Derived UI facts only. Credentials and writable controller state are not part of the view.
#[derive(PartialEq, Eq)]
pub(crate) struct SessionSnapshot {
    pub phase: Phase,
    pub delete_leftovers: usize,
    pub persistence_warning: Option<PersistenceWarning>,
}

#[derive(Clone, Copy)]
pub(crate) struct SessionRead<'a>(pub &'a SessionSnapshot);

impl SessionSnapshot {
    pub fn read(&self) -> SessionRead<'_> { SessionRead(self) }
    fn from_state(state: &SessionInit) -> Self {
        Self { phase: state.phase, delete_leftovers: state.delete_leftovers,
            persistence_warning: state.persistence_warning }
    }
    fn matches_state(&self, state: &SessionInit) -> bool {
        self.phase == state.phase && self.delete_leftovers == state.delete_leftovers
            && self.persistence_warning == state.persistence_warning
    }
}

pub(crate) struct SessionMachine {
    state: SessionInit,
    publication: Arc<SessionSnapshot>,
    subhash: u64,
    logical_dirty: bool,
}

impl SessionMachine {
    pub fn from_init(state: SessionInit) -> Self {
        assert!(state.inbox.len() + usize::from(state.pending_commit.as_ref().is_some_and(|c| c.receipt.is_some()))
            <= SESSION_TRANSFER_RECORDS, "invalid Session transfer state");
        let mut canon = Canon::new();
        state.write(&mut canon);
        let subhash = canon.finish();
        let publication = Arc::new(SessionSnapshot::from_state(&state));
        Self { state, publication, subhash, logical_dirty: true }
    }
    // Cached hashes already change on logical transitions; aggregate dirty consumption is
    // separate from UI Arc damage and remains a following-stage App recording integration.
    fn refresh_subhash(&mut self) {
        let mut canon = Canon::new();
        self.state.write(&mut canon);
        let next = canon.finish();
        if next != self.subhash {
            self.subhash = next;
            self.logical_dirty = true;
        }
    }
    pub fn snapshot_init(&self) -> SessionInit { self.state.clone() }
    pub fn read(&self) -> SessionRead<'_> { self.publication.read() }
    pub fn publication(&self) -> Arc<SessionSnapshot> { Arc::clone(&self.publication) }
    pub fn subhash(&self) -> u64 { self.subhash }

    pub fn needs_ready_commit(&self) -> bool {
        self.state.phase == Phase::Ready && self.state.apply_pending && self.state.pending_commit.is_none()
            && self.state.persistence_warning.is_none()
    }

    pub fn publication_is_current(&self, publication: &ProfilePublication) -> bool {
        publication.epoch == self.state.epoch && publication.scope == self.state.profile_scope
    }

    /// Physical completion remains evidence even after its authority permit was superseded.
    /// Advance only the comparison fence; never publish obsolete credentials as trusted state.
    fn observe_disk_write(&mut self, before: &Identity, after: &Identity,
        outcome: crate::plex::session::async_persistence::CompletionOutcome) -> bool {
        use crate::plex::session::async_persistence::{CompletionOutcome, Operation};
        if self.state.disk_identity != *before || before == after
            || !matches!(outcome, CompletionOutcome::Durable(Operation::Write { .. })) { return false; }
        self.state.disk_identity = after.clone();
        true
    }

    /// Read-only authorization at effect execution, after any carried cancellation command.
    /// The bridge checks before admission and again when the worker completes. Supersession
    /// cancels pending work; a write already completed still reports its physical disk evidence.
    pub fn commit_is_current(&self, req: u32, epoch: u64, arrival: u64) -> bool {
        self.state.epoch == epoch
            && self.state.pending.contains_key(&req)
            && self.state.pending_commit.as_ref().is_some_and(|commit|
                commit.req == req && commit.epoch == epoch && commit.arrival == arrival)
    }

    pub fn commit_permit(&self, req: u32, epoch: u64, arrival: u64) -> Option<CommitPermit<'_>> {
        self.commit_is_current(req, epoch, arrival).then_some(CommitPermit {
            req, epoch, arrival, owner: std::marker::PhantomData,
        })
    }

    /// Bridge wiring hook for Stage B: the caller that owns the decision can name the purpose
    /// explicitly instead of inheriting a default. The adapter reaches the plan's own purpose
    /// through [`CommitPlan`], so this is not yet called inside this crate.
    #[allow(dead_code)]
    pub fn commit_permit_for(&self, req: u32, epoch: u64, arrival: u64) -> Option<CommitPermit<'_>> {
        self.commit_is_current(req, epoch, arrival).then_some(CommitPermit {
            req, epoch, arrival, owner: std::marker::PhantomData,
        })
    }

    fn begin_commit(&mut self, req: u32, arrival: u64, terminal: bool,
        plan: CommitPlan, delta: CommitDelta, emit: &mut impl FnMut(SessionFx)) -> bool {
        if self.state.pending_commit.is_some() { return false; }
        let Some(pending) = self.state.pending.get_mut(&req) else { return false };
        if pending.key.epoch != self.state.epoch { return false; }
        pending.last_arrival = Some(arrival);
        let epoch = pending.key.epoch;
        let fresh = plan.writes_durable && plan.credentials.is_some()
            && plan.authority == crate::plex::session::SaveAuthority::FreshReauthentication;
        self.state.pending_commit = Some(PendingCommit { req, epoch, arrival, terminal,
            writes_credentials: plan.writes_durable && plan.credentials.is_some(),
            receipt: None, delta,
            admitted_revision: None,
            purpose: plan.writes_durable.then_some(plan.purpose),
            fresh });
        self.state.persistence_purpose = Some(plan.purpose);
        emit(SessionFx::Commit { req, epoch, arrival, plan });
        true
    }

    fn take_ready(&mut self, emit: &mut impl FnMut(SessionFx)) -> bool {
        if self.state.phase != Phase::Ready || !self.state.apply_pending
            || self.state.pending_commit.is_some() { return false; }
        // A discovery failure left a warning unacknowledged: the final fresh write must not run
        // over it (AUTH-03) — acknowledging is what re-admits `needs_ready_commit`.
        if self.state.persistence_warning.is_some() { return false; }
        let Some(req) = self.allocate(SessionOp::Ready, None) else {
            self.state.phase = Phase::Error;
            self.state.apply_pending = false;
            self.state.error = crate::i18n::msg::browse_auth_authority_failed().into();
            self.replace_publication();
            return true;
        };
        let mut next = self.state.persisted.clone();
        super::remember_unprotected_active(&mut next);
        let patch = CredentialPatch::of(&next);
        // Consumed exactly once: a discovery/rediscovery retry only PEEKS this flag
        // (`apply_resource_observation`), so it cannot spend the authority the final write needs.
        let authority = if std::mem::take(&mut self.state.authorized_in_flow) {
            crate::plex::session::SaveAuthority::FreshReauthentication
        } else { crate::plex::session::SaveAuthority::Routine };
        let plan = CommitPlan { registry_client_id: self.state.persisted.client_id.clone(),
            expected_disk: self.state.disk_identity.clone(),
            credentials: Some(patch.clone()), lifecycle: None,
            registry: vec![RegistryPlan::Install { sources: next.sources.clone(), primary: None, replace: false }],
            purpose: PersistencePurpose::Final, writes_durable: true, authority };
        self.begin_commit(req, 0, true, plan, CommitDelta {
            credentials: Some(patch), activate_profile: true, ready: Some(false),
            ..Default::default()
        }, emit)
    }

    /// Settle a durability verdict ONLY if it still describes the write this owner is waiting on.
    ///
    /// Fenced by request, epoch, arrival and revision together. A completion that fails the fence
    /// is inert: it must not activate registry or profile, must not clear a newer error, and must
    /// not release admission credit that belongs to a different operation. The owner keeps only
    /// this logical identity; channels and receipts stay in the adapter.
    fn apply_persistence_completion(
        &mut self,
        completion: crate::plex::session::async_persistence::PersistenceCompletion,
        emit: &mut impl FnMut(SessionFx),
    ) -> bool {
        use crate::plex::session::async_persistence::CompletionOutcome;
        let Some(admitted) = self.state.admitted_persistence else { return false };
        let correlation = crate::plex::session::async_persistence::PersistenceCorrelation {
            req: admitted.req, epoch: admitted.epoch, arrival: admitted.arrival,
        };
        if !completion.acts_on(correlation, admitted.revision) { return false; }
        // A verdict the worker resolved for a DIFFERENT purpose than the one admitted is not this
        // operation's verdict either.
        if admitted.purpose.is_some_and(|purpose| purpose != completion.purpose) { return false; }
        let durable = matches!(completion.outcome, CompletionOutcome::Durable(_));
        self.state.admitted_persistence = None;
        let prior = self.state.unconfirmed_fresh_prior.take();
        if let (Some(prior), CompletionOutcome::Failed(_)) = (prior, &completion.outcome) {
            if admitted.fresh { self.state.disk_identity = prior; }
        }
        self.state.commit_phase.durable = durable;
        self.state.commit_phase.proves_saved_login =
            durable && completion.purpose.proves_saved_login();
        let correlation_key = PersistenceWarningKey { epoch: completion.epoch, req: completion.req };
        if durable {
            // 0.6.6 parity: a later FRESH write landing durably supersedes any warning still
            // showing about an earlier fresh attempt — the failure it was reporting on no longer
            // describes the session's current state, so holding it would strand an acknowledgement
            // over a problem that already resolved itself.
            let superseded = admitted.fresh && self.state.persistence_warning.is_some();
            if superseded {
                self.state.persistence_warning = None;
            }
            // Release only the handoff THIS completion is for — a routine completion arriving
            // while an unrelated fresh handoff is held must not free it.
            let released = self.state.held_handoff == Some(HeldHandoff { epoch: completion.epoch, req: completion.req });
            if released { self.release_held_handoff(emit); }
            if superseded || released { self.replace_publication(); }
        } else if admitted.fresh {
            if self.state.persistence_warning_answered {
                // This authorization already had its one unsaved-login question answered
                // (AUTH-03's field bug: a device unwritable on BOTH Discovery and Final used to
                // ask it twice). Storage failing again must not ask it a second time — release
                // whatever handoff this completion was holding so entry proceeds regardless.
                let released = self.state.held_handoff == Some(HeldHandoff { epoch: completion.epoch, req: completion.req });
                if released { self.release_held_handoff(emit); self.replace_publication(); }
            } else {
                // The final write's own non-durable completion replaces a Discovery warning still
                // showing (`Discovery` and `Final` never coexist: an unacknowledged Discovery warning
                // blocks `take_ready`), so the newer one wins by direct overwrite.
                let warning = PersistenceWarning::from_outcome(correlation_key, admitted.site, &completion.outcome);
                self.state.persistence_warning = Some(warning);
                self.replace_publication();
            }
        }
        true
    }

    fn apply_commit_reply(&mut self, reply: CommitReply, emit: &mut impl FnMut(SessionFx)) -> bool {
        if !self.commit_is_current(reply.req, reply.epoch, reply.arrival) { return false; }
        // `take()` is what makes a duplicate reply inert. The admitted revision and purpose are
        // copied into `admitted_persistence` first, because that is the identity a later
        // completion must repeat exactly to be allowed to settle anything.
        let admit_revision = reply.admission.admitted_revision();
        // The admission names what storage was actually asked for; the plan only records intent.
        let admit_purpose = reply.admission.admitted_purpose()
            .or_else(|| self.state.pending_commit.as_ref().and_then(|c| c.purpose));
        let commit = self.state.pending_commit.take().unwrap();
        self.state.admitted_persistence = admit_revision.map(|revision| AdmittedPersistence {
            req: commit.req, epoch: commit.epoch, arrival: commit.arrival, revision,
            purpose: admit_purpose, fresh: commit.fresh,
            site: match admit_purpose {
                Some(PersistencePurpose::Discovery) => PersistenceWarningSite::Discovery,
                _ => PersistenceWarningSite::Final,
            } });
        self.state.commit_phase = CommitPhase {
            admitted: admit_revision.is_some(),
            durable: false,
            proves_saved_login: false,
        };
        self.state.persistence_purpose = None;
        if !reply.admission.accepted() {
            let op = self.state.pending.remove(&reply.req).unwrap().key.op;
            match op {
                SessionOp::Ready => {
                    self.state.phase = Phase::Error;
                    self.state.apply_pending = false;
                    self.state.error = crate::i18n::msg::browse_auth_authority_failed().into();
                }
                SessionOp::ServerRoster | SessionOp::Endpoint(_) => {}
            }
            emit(SessionFx::Retire { req: reply.req });
            if let Some(receipt) = commit.receipt { emit(SessionFx::Acknowledge(vec![receipt])); }
            self.schedule_pump(emit);
            self.replace_publication();
            return true;
        }
        let delta = commit.delta;
        if let Some(patch) = delta.credentials {
            self.state.persisted = patch.merge_into(&self.state.persisted);
            if commit.writes_credentials {
                let prior = std::mem::replace(&mut self.state.disk_identity, patch.identity());
                self.state.unconfirmed_fresh_prior =
                    (commit.fresh && admit_revision.is_some()).then_some(prior);
                self.state.committed_credentials = patch;
            }
        }
        if let Some(phase) = delta.phase { self.state.phase = phase; }
        if let Some(picker) = delta.picker { self.state.picker = picker; }
        if let Some(users) = delta.users { self.state.users = users; }
        if delta.roster_answered { self.state.switch_refused_for = None; }
        if let Some(ready) = delta.ready { self.state.apply_pending = ready; }
        if delta.clear_error {
            self.state.error.clear();
            self.state.pin_denied = false;
        }
        if delta.profile_seated {
            let pending = self.state.pending.get_mut(&reply.req).unwrap();
            pending.expected = Identity::of(&self.state.persisted);
            pending.phase = StreamPhase::ProfileSeated;
            self.cancel_obsolete_interests(reply.req, emit);
        }
        if delta.complete_signin && std::mem::take(&mut self.state.signin_active) {
        }
        if delta.activate_profile {
            if commit.fresh && admit_revision.is_some() && !self.state.persistence_warning_answered {
                // The fresh write is admitted but not yet confirmed durable: hold the handoff
                // rather than announce Ready/publish the profile until a completion (or an
                // acknowledged warning) releases it — the AUTH-03 gate.
                self.state.held_handoff = Some(HeldHandoff { epoch: commit.epoch, req: commit.req });
            } else if self.state.persistence_warning.is_none() {
                // `persistence_warning_answered` true means this authorization already spent its
                // one Continue: nothing is left to wait for, so a fresh write lands here exactly
                // like a routine one — publish immediately rather than hold a handoff whose
                // completion (or loss) would otherwise be the only way out.
                // 0.6.6 parity (`discovery-warning-not-cleared-on-retry-or-fresh-success`): a
                // ROUTINE `activate_profile` commit (StartSwitch, …) must not free
                // a handoff still held behind an unacknowledged warning — that release is
                // `acknowledge_persistence_warning`'s alone. An unrelated held handoff with NO live
                // warning (already superseded above, or never one to begin with) is unaffected.
                self.release_held_handoff(emit);
            }
        }
        if commit.terminal {
            self.state.pending.remove(&reply.req);
            emit(SessionFx::Retire { req: reply.req });
        }
        if let Some(receipt) = commit.receipt { emit(SessionFx::Acknowledge(vec![receipt])); }
        self.schedule_pump(emit);
        self.replace_publication();
        true
    }

    fn retire(&mut self, req: u32, emit: &mut impl FnMut(SessionFx)) {
        self.state.pending.remove(&req);
        emit(SessionFx::Retire { req });
    }

    /// The single site that publishes a profile and announces `SessionFx::Ready`. A fresh
    /// handoff reaches it only once its write is released — durably confirmed, or its warning
    /// acknowledged; a non-fresh (`Routine`) commit reaches it immediately, exactly as before.
    fn release_held_handoff(&mut self, emit: &mut impl FnMut(SessionFx)) {
        self.state.held_handoff = None;
        self.publish_profile(Some(self.state.persisted.user.clone()), emit);
        if self.state.phase == Phase::Ready {
            emit(SessionFx::Ready { epoch: self.state.epoch, scope: self.state.profile_scope,
                server: self.state.persisted.server.clone(), token: self.state.persisted.pms_token().into(),
                install: ReadyInstall::PrimaryAndExtras(match &self.state.authority {
                    BootstrapAuthority::Account { extras } => extras.clone(),
                    BootstrapAuthority::DevJf { .. } => Vec::new(),
                }) });
        }
    }

    /// The AUTH-03 acknowledgement door: clears the warning and, if a handoff is still held for
    /// it, releases it — exactly ONE `SessionFx::Ready` for the flow, through
    /// [`Self::release_held_handoff`], never a second one from here.
    fn acknowledge_persistence_warning(&mut self, key: PersistenceWarningKey,
        emit: &mut impl FnMut(SessionFx)) -> bool {
        if self.state.persistence_warning.map(|warning| warning.key) != Some(key) { return false; }
        self.state.persistence_warning = None;
        // This authorization's unsaved-login question is answered now: storage may never gate
        // entry a second time for it (AUTH-03's field bug). A later failure — Discovery then
        // Final, or a retry — releases the handoff itself instead of asking again.
        self.state.persistence_warning_answered = true;
        if self.state.held_handoff.is_some() {
            self.release_held_handoff(emit);
        }
        self.replace_publication();
        true
    }

    fn advance_epoch(&mut self, emit: &mut impl FnMut(SessionFx)) -> Option<u64> {
        let epoch = self.state.epoch.checked_add(1)?;
        let requests = self.state.pending.keys().copied().collect();
        self.discard_owned_envelopes(emit);
        self.state.pending.clear();
        self.state.epoch = epoch;
        self.state.discovery_retry = None;
        emit(SessionFx::Cancel { requests, epoch });
        Some(epoch)
    }

    fn schedule_pump(&mut self, emit: &mut impl FnMut(SessionFx)) {
        if !self.state.inbox.is_empty() && self.state.pending_commit.is_none() && !self.state.pump_pending {
            self.state.pump_pending = true;
            emit(SessionFx::Pump);
        }
    }

    fn discard_owned_envelopes(&mut self, emit: &mut impl FnMut(SessionFx)) {
        let mut receipts: Vec<_> = self.state.inbox.drain(..).map(|record| Receipt::of(&record)).collect();
        if let Some(commit) = self.state.pending_commit.take() {
            emit(SessionFx::Cancel { requests: vec![commit.req], epoch: self.state.epoch });
            if let Some(receipt) = commit.receipt { receipts.push(receipt); }
        }
        // Keep pump_pending: the marker may still be an App effect or a carried typed event.
        // Carried envelope receipts are not ours yet and therefore are NOT acknowledged here.
        if !receipts.is_empty() { emit(SessionFx::Acknowledge(receipts)); }
    }

    fn pump_one(&mut self, emit: &mut impl FnMut(SessionFx)) {
        if self.state.pending_commit.is_some() { return; }
        let Some(mut envelope) = self.state.inbox.pop_front() else { return };
        let receipt = Receipt::of(&envelope);
        // Completion authority is independent of payload authority. Recheck at the FIFO
        // head (a preceding commit may have changed this request), never at ingress.
        // A trusted terminal with rejected data still ends its own work, using the same
        // failure policy as a dropped producer; none of its payload reaches resource IO.
        if !self.accepts(&envelope) && envelope.terminal
            && matches!(envelope.outcome, SessionArrival::Data(_))
            && self.accepts_header(&envelope)
            && self.state.pending.get(&envelope.addr.req.0).is_some_and(|pending|
                !matches!(pending.key.op, SessionOp::Endpoint(_))
                    || pending.lifecycle == envelope.lifecycle)
        {
            envelope.outcome = SessionArrival::Dropped;
        }
        if self.accepts(&envelope) {
            self.apply_resource_observation(&envelope, emit);
        }
        if let Some(commit) = &mut self.state.pending_commit {
            assert!(commit.req == receipt.addr.req.0 && commit.epoch == receipt.key.epoch
                && commit.arrival == receipt.arrival, "Session commit receipt mismatch");
            commit.receipt = Some(receipt);
        } else {
            emit(SessionFx::Acknowledge(vec![receipt]));
        }
        self.schedule_pump(emit);
    }

    fn publish_profile(&mut self, profile: Option<UserRef>, emit: &mut impl FnMut(SessionFx)) {
        // This is the sole scope allocator. Resource publication will store this explicit value
        // rather than independently incrementing another generation.
        self.state.profile_scope.0 = self.state.profile_scope.0.wrapping_add(1);
        self.state.active_profile = profile;
        emit(SessionFx::PublishProfile(ProfilePublication {
            epoch: self.state.epoch, profile: self.state.active_profile.clone(), scope: self.state.profile_scope,
        }));
    }

    fn cancel_obsolete_interests(&mut self, retained: u32, emit: &mut impl FnMut(SessionFx)) {
        let requests: Vec<_> = self.state.pending.iter().filter_map(|(&req, pending)|
            (req != retained && !pending.expected.matches(&self.state.persisted)).then_some(req)).collect();
        if requests.is_empty() { return; }
        for req in &requests { self.state.pending.remove(req); }
        let mut receipts = Vec::new();
        self.state.inbox.retain(|record| {
            if requests.contains(&record.addr.req.0) {
                receipts.push(Receipt::of(record));
                false
            } else { true }
        });
        emit(SessionFx::Cancel { requests, epoch: self.state.epoch });
        if !receipts.is_empty() { emit(SessionFx::Acknowledge(receipts)); }
    }

    fn erase(&mut self, sign_in: bool, emit: &mut impl FnMut(SessionFx)) -> bool {
        if self.state.pending_erase.is_some() || self.state.epoch.checked_add(1).is_none() { return false; }
        self.advance_epoch(emit).expect("epoch preflight");
        self.state.persisted = PersistedSession::default();
        self.state.committed_credentials = CredentialPatch::of(&self.state.persisted);
        // The identity ends here, and its verdicts with it: the same account signed back in is
        // the same `Identity`, and nothing has refused it anything in that sign-in.
        self.state.switch_refused_for = None;
        self.state.pending_erase = Some((self.state.epoch, sign_in));
        self.state.authority = BootstrapAuthority::Account { extras: Vec::new() };
        self.state.picker = Picker::Boot;
        self.state.pin_code.clear();
        self.state.qr_png.clear();
        self.state.qr_gen = 0;
        self.state.users.clear();
        self.state.error.clear();
        self.state.pin_denied = false;
        self.state.authorized_in_flow = false;
        self.state.persistence_warning = None;
        self.state.held_handoff = None;
        self.state.persistence_warning_answered = false;
        self.state.signin_active = false;
        self.state.apply_pending = false;
        self.state.code_replaced = false;
        self.publish_profile(None, emit);
        emit(SessionFx::Erase { req: self.state.next_req, epoch: self.state.epoch, all_local: !sign_in });
        self.replace_publication();
        true
    }

    fn erased(&mut self, epoch: u64, leftovers: usize, emit: &mut impl FnMut(SessionFx)) -> bool {
        let Some((expected, _sign_in)) = self.state.pending_erase else { return false };
        if expected != epoch || self.state.epoch != epoch { return false; }
        self.state.pending_erase = None;
        self.state.disk_identity = Identity::of(&self.state.persisted);
        self.state.delete_leftovers = leftovers;
        self.state.phase = Phase::Deleted;
        emit(SessionFx::Coordinator(CoordinatorAction::LocalDataErased));
        self.replace_publication();
        true
    }

    fn apply_resource_observation(&mut self, envelope: &SessionEnvelope,
        emit: &mut impl FnMut(SessionFx)) -> bool {
        // The roster/endpoint workers are gone with the plex.tv account surface. Any envelope
        // that still arrives (none can be produced) ends its own request without planning a
        // commit: the transport, its receipts and the canon of an arriving envelope are frozen
        // (SessionInit::write layout), so this intake retires instead of disappearing.
        if !self.accepts(envelope) { return false; }
        let req = envelope.addr.req.0;
        self.retire(req, emit);
        self.replace_publication();
        true
    }

    /// The checked allocator is independent from the full-width auth epoch. Exhaustion cannot
    /// reuse an address, including when a cancelled worker still owns a Landing reservation.
    fn allocate(&mut self, op: SessionOp, lifecycle: Option<ServerLifecycle>) -> Option<u32> {
        let req = self.state.next_req.checked_add(1)?;
        self.state.next_req = req;
        self.state.pending.insert(req, Pending {
            key: SessionWorkKey { epoch: self.state.epoch, op },
            expected: Identity::of(&self.state.persisted), lifecycle,
            last_arrival: None, phase: StreamPhase::Running, capture: None, admission: AdmissionState::NotRequested,
        });
        Some(req)
    }

    /// Admission to logical application, not a resource reservation. Landing owns the latter.
    /// Merely inspecting an invalid envelope must not mutate pending state or its publication.
    fn accepts_header(&self, envelope: &SessionEnvelope) -> bool {
        if envelope.addr.to != MachineId::Session { return false; }
        let Some(pending) = self.state.pending.get(&envelope.addr.req.0) else { return false; };
        pending.key == envelope.key && pending.key.epoch == self.state.epoch
            && pending.capture.is_none()
            && matches!(pending.admission, AdmissionState::Awaiting(id) | AdmissionState::Accepted(id) if id == envelope.admission)
            && pending.last_arrival.is_none_or(|last| envelope.arrival > last)
    }

    fn accepts(&self, envelope: &SessionEnvelope) -> bool {
        if !self.accepts_header(envelope) { return false; }
        let pending = &self.state.pending[&envelope.addr.req.0];
        match &envelope.outcome {
                SessionArrival::Data(data) => data.matches_request(pending, envelope.terminal)
                    && (!matches!(pending.key.op, SessionOp::Endpoint(_))
                        || pending.lifecycle == envelope.lifecycle),
                SessionArrival::Dropped | SessionArrival::Refused => envelope.terminal,
        }
    }

    fn replace_publication(&mut self) {
        if self.publication.matches_state(&self.state) { return; }
        self.publication = Arc::new(SessionSnapshot::from_state(&self.state));
    }
}

impl<H: SessionHost> crate::ui::machine::Machine<H> for SessionMachine {
    type Ev = SessionEvent;

    fn step(&mut self, ev: &SessionEvent, _cx: &crate::ui::machine::Cx<'_, H>,
        fx: &mut crate::ui::machine::Effects<'_, H>) -> crate::ui::machine::Handled {
        use crate::ui::machine::{Fx, Handled};
        let before = self.publication();
        let mut emit = |effect| fx.push(Fx::App(H::session_effect(effect)));
        let handled = match ev {
            SessionEvent::Command(Command::TakeReady) => self.take_ready(&mut emit),
            SessionEvent::Command(Command::AcknowledgePersistenceWarning { key }) =>
                self.acknowledge_persistence_warning(*key, &mut emit),
            SessionEvent::Command(Command::SignOut) => self.erase(true, &mut emit),
            SessionEvent::Command(Command::EraseLocal) => self.erase(false, &mut emit),
            // The roster commands are gone with the plex.tv account surface; the answer command
            // survives only as the alert UI's vocabulary, and there is nothing left to answer:
            // no probe can produce an insecure-only verdict to ask about.
            SessionEvent::Command(Command::AnswerPlaintext { .. }) => false,
            SessionEvent::Command(Command::NoteDeleteLeftovers(count)) => {
                self.state.delete_leftovers = *count;
                self.replace_publication();
                true
            }
            SessionEvent::Commit(reply) => self.apply_commit_reply(*reply, &mut emit),
            SessionEvent::DiskWrite { before, after, outcome } => self.observe_disk_write(before, after, *outcome),
            SessionEvent::Persistence(completion) => self.apply_persistence_completion(*completion, &mut emit),
            SessionEvent::Erased { epoch, leftovers } => self.erased(*epoch, *leftovers, &mut emit),
            SessionEvent::Pump => {
                self.state.pump_pending = false;
                self.pump_one(&mut emit);
                true
            }
        };
        if !Arc::ptr_eq(&before, &self.publication) {
            fx.invalidate(crate::ui::present::Provenance::Landing(MachineId::Session));
        }
        self.refresh_subhash();
        if handled { Handled::Yes } else { Handled::No }
    }
}

#[cfg(test)]
mod tests {
    fn captured_session() -> SessionInit {
        SessionInit::captured(PersistedSession { client_id: "synthetic-client".into(), ..Default::default() })
    }



    use super::*;

    struct OwnerHost;
    impl crate::ui::machine::Host for OwnerHost {
        type Arg = crate::ui::fixture::FixtureArg;
        type Fx = SessionFx;
        type Msg = SessionEvent;
        type Elem = u32;
        type Views<'a> = SessionRead<'a>;
        type Init = SessionInit;
        type Memory = ();
    }
    impl SessionHost for OwnerHost {
        fn session_effect(effect: SessionFx) -> SessionFx { effect }
    }

    fn step(owner: &mut SessionMachine, event: SessionEvent) -> Vec<SessionFx> {
        use crate::ui::machine::{Cx, Effects, Fx, InputOwner, EntryId, Machine, Tick};
        let publication = owner.publication();
        let cx = Cx::<OwnerHost> { views: publication.read(), tick: Tick::default(),
            measure: &crate::ui::fixture::FixtureMeasure, press: Default::default(),
            focus: Default::default(), owner: InputOwner::Entry(EntryId(0)) };
        let mut present = crate::ui::present::Present::new();
        let mut effects = Vec::new();
        owner.step(&event, &cx, &mut Effects::new(&mut effects, MachineId::Session, &mut present));
        effects.into_iter().map(|effect| match effect.fx {
            Fx::App(effect) => effect,
            _ => panic!("Session emitted a non-domain effect"),
        }).collect()
    }



    #[test]
    fn delete_leftovers_is_recorded_and_reread_without_being_consumed() {
        let mut a = SessionMachine::from_init(captured_session());
        let b = SessionMachine::from_init(captured_session());
        step(&mut a, SessionEvent::Command(Command::NoteDeleteLeftovers(3)));
        let retained = a.publication();
        assert_eq!(a.read().0.delete_leftovers, 3);
        assert_eq!(a.read().0.delete_leftovers, 3, "the read is not a consuming queue");
        assert_eq!(b.read().0.delete_leftovers, 0, "another owner cannot inherit the sweep count");
        step(&mut a, SessionEvent::Command(Command::NoteDeleteLeftovers(0)));
        assert_eq!(a.read().0.delete_leftovers, 0, "a clean sweep replaces the earlier count");
        assert_eq!(retained.read().0.delete_leftovers, 3, "retained reads stay coherent");
    }


    /// ACCEPTANCE SPEC 1/5. Consuming a commit returns a TYPED admission separating authority
    /// currency from durability, and a stale completion settles nothing.


    #[test]
    fn auto_sign_in_only_changes_init_and_cached_owner_hash_and_round_trips() {
        let off = captured_session();
        let mut on = off.clone();
        on.persisted = on.persisted.with_auto_sign_in(true);
        assert_ne!(off.hash(), on.hash(), "captured auto-sign-in preference is canonical input");
        let a = SessionMachine::from_init(off);
        let b = SessionMachine::from_init(on.clone());
        assert_ne!(a.subhash(), b.subhash(), "cached owner hash must include the preference");
        let restored: SessionInit = serde_json::from_slice(&serde_json::to_vec(&on).unwrap()).unwrap();
        assert!(restored.persisted.auto_sign_in());
        assert_eq!(restored.hash(), on.hash());
        assert_eq!(SessionMachine::from_init(restored).subhash(), b.subhash());
    }


    #[test]
    fn plex_tv_token_changes_user_and_cached_owner_hashes() {
        let none = captured_session();
        let mut first = none.clone();
        first.persisted.user.plex_tv_token = Some("synthetic-plex-tv-token-a".into());
        let mut second = first.clone();
        second.persisted.user.plex_tv_token = Some("synthetic-plex-tv-token-b".into());

        assert_ne!(none.hash(), first.hash(), "None and Some plex.tv credentials are distinct canonical state");
        assert_ne!(first.hash(), second.hash(), "users differing only by plex.tv credential are distinct canonical state");
        assert_ne!(SessionMachine::from_init(none).subhash(), SessionMachine::from_init(first).subhash(),
            "cached owner hash must include the optional plex.tv credential");
    }
}
