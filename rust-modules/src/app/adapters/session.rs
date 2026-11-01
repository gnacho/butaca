//! Per-application Session resources. Landing capacity bounds records, not payload bytes.
//! Neither a worker nor a fixture adapter can obtain a mutable SessionMachine through this API.

use crate::auth::owner::SessionEnvelope;
use crate::ui::machine::{Addr, RequestId};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use crate::auth::owner::{CommitPermit, CommitPlan, CommitReply, Receipt,
    SESSION_TRANSFER_RECORDS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionIngressError { BatchTooLarge, AddressMismatch }

/// The revision of the last ordinary asynchronous persistence admission, used to give the live
/// adapter's typed reply a real revision without inventing one. Zero before any bootstrap.
fn ordinary_revision() -> u64 {
    crate::plex::session::async_persistence::ordinary_revision()
}

enum Resources {
    Live { publisher: crate::plex::session::ProfilePublisher },
    #[cfg(test)]
    Fixture(FixtureResources),
}

/// Test resource boundary, not a decision machine. Writes use the same CredentialPatch merge
/// as live disk. Registry operations are recorded; only explicitly injected native endpoint
/// slots execute the shared registry implementation (using its non-network test constructor).
#[cfg(test)]
pub(crate) struct FixtureResources {
    pub disk: crate::plex::session::Session,
    pub registry_writes: Vec<crate::auth::owner::RegistryPlan>,
    pub profile: Option<crate::auth::owner::ProfilePublication>,
    pub coordinator_events: Vec<crate::auth::owner::CoordinatorAction>,
    pub sweep_leftovers: usize,
    /// One-shot override for the next admitted durable write's completion outcome (e.g. a
    /// `Failed`/`Uncertain` verdict) — drained by `commit` the same way a real disk failure would
    /// be, so an app-level test can drive `PersistenceWarning` routing without a real filesystem
    /// fault. `None` keeps the default `Durable(PersistedPlaintext)` outcome.
    pub next_completion_outcome: Option<crate::plex::session::async_persistence::CompletionOutcome>,
}

/// Only auxiliary IO is stubbed: this mode executes the real disk and registry paths.
/// Tests must hold the resource serial lock and a scoped TempSession throughout its lifetime.
#[cfg(test)]
struct ResourceTestIo {
    recording_root: Option<std::path::PathBuf>,
    coordinator_events: Vec<crate::auth::owner::CoordinatorAction>,
    erase_sweeps: Vec<bool>,
}


type DiskCommit = Result<Option<crate::plex::session::async_persistence::LiveWrite>, ()>;

/// Disk fencing and mutation execute together under session::IO on the persistence worker.
#[cfg(test)]
static CREDENTIAL_IO_STARTED: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>> = std::sync::Mutex::new(None);

fn write_credentials(plan: &CommitPlan, cancelled: &AtomicBool) -> DiskCommit {
    #[cfg(test)]
    if let Some(started) = CREDENTIAL_IO_STARTED.lock().unwrap().take() { let _ = started.send(()); }
    let Some(patch) = &plan.credentials else { return Ok(None); };
    if plan.authority == crate::plex::session::SaveAuthority::FreshReauthentication {
        crate::plex::session::replace_after_reauthentication_guarded_with_outcome(
            |disk| plan.expected_disk.matches(disk), |disk| patch.merge_into(disk),
            || !cancelled.load(Ordering::Acquire))
    } else {
        let mut stale = false;
        let write = crate::plex::session::update_guarded_with_outcome(|disk| {
            stale = !plan.expected_disk.matches(disk);
            (!stale).then(|| patch.merge_into(disk))
        }, || !cancelled.load(Ordering::Acquire));
        if stale { Err(()) } else { Ok(write) }
    }
}

// Admission retries are polled by the bridge on the frame thread, not by the worker.
// Use logical milliseconds so replay/test ticks control the same one-second backoff.
const STORAGE_RETRY_MS: u32 = 1_000;

/// Ceiling on the wait between clears that came back incomplete. An incomplete clear is a
/// credential that really survived (an absent candidate counts as retired), so the erase is never
/// abandoned — but each attempt rewrites and fsyncs the revocation marker and logs, and a
/// candidate that stays undeletable must not do that every second for the life of the process.
const ERASE_RETRY_CAP_MS: u32 = 30_000;

/// 1 s, 2 s, 4 s … after the `incomplete`-th consecutive incomplete clear, capped.
fn erase_retry_delay(incomplete: u32) -> u32 {
    STORAGE_RETRY_MS
        .saturating_mul(1 << incomplete.saturating_sub(1).min(15))
        .min(ERASE_RETRY_CAP_MS)
}

struct PendingCommit {
    req: u32,
    epoch: u64,
    arrival: u64,
    plan: CommitPlan,
    cancelled: Arc<AtomicBool>,
    ticket: Option<crate::storage_worker::TypedTicket<DiskCommit>>,
    retry_at: u32,
}

pub(crate) struct CompletedCommit {
    pub req: u32,
    pub epoch: u64,
    pub arrival: u64,
    pub plan: CommitPlan,
    disk: DiskCommit,
}

impl CompletedCommit {
    pub(crate) fn disk_event(&self) -> Option<crate::auth::owner::SessionEvent> {
        let write = self.disk.as_ref().ok()?.as_ref()?;
        let patch = self.plan.credentials.as_ref()?;
        Some(crate::auth::owner::SessionEvent::DiskWrite {
            before: self.plan.expected_disk.clone(), after: patch.identity(), outcome: write.classify(),
        })
    }
}

struct PendingErase {
    epoch: u64,
    all_local: bool,
    diagnostics: bool,
    ticket: Option<crate::storage_worker::TypedTicket<EraseWorkerOutcome>>,
    retry_at: u32,
    /// Consecutive clears that reported a surviving credential; drives [`erase_retry_delay`].
    incomplete: u32,
    /// The install language the first attempt read before clearing. A retry carries it, because
    /// once the credential file is gone a re-read can no longer see what to retain.
    language: Option<crate::i18n::Preference>,
}

struct EraseWorkerOutcome {
    complete: bool,
    failures: Vec<String>,
    /// The next-launch language after this erase: retained by sign-out, System after delete-all.
    language: crate::i18n::Preference,
}

pub(crate) struct SessionAdapter {
    resources: Resources,
    controlled_home: bool,
    replay_resources: bool,
    recording_leftovers: usize,
    /// Metadata only. These credits cover owner-held AND dispatcher-carried envelopes, so
    /// cancelling a request must not clear them before those unique records are discarded.
    /// The real durability verdict of the last live credential write, with the correlation
    /// it belongs to. The worker has finished by the time the deferred
    /// typed admission is returned; the bridge drains it as a typed completion rather than
    /// letting the owner infer durability from `accepted`.
    live_completion: Option<crate::plex::session::async_persistence::PersistenceCompletion>,
    erasures: std::collections::VecDeque<PendingErase>,
    commits: std::collections::VecDeque<PendingCommit>,
    spawn: fn(&'static str, Box<dyn FnOnce() + Send>) -> bool,
    #[cfg(test)]
    resource_test_io: Option<ResourceTestIo>,
    // Construction proves the live adapter originated on main without duplicating or moving the
    // Player adapter's exclusive token. The owned adapter remains !Send/!Sync afterwards.
    main_thread: PhantomData<Rc<()>>,
}

impl Drop for SessionAdapter {
    fn drop(&mut self) {
        // Retire interest, not the resource reservation. WorkerOutput retains Landing until
        // the running closure acknowledges cancellation through its completion guard.
        self.cancel_all();
    }
}

impl SessionAdapter {
    pub(crate) fn recording_retired_for_erasure(&mut self, failed: bool) {
        self.recording_leftovers += usize::from(failed);
        if self.controlled_home && !self.replay_resources {
            self.controlled_home = false;
            self.spawn = |name, job| crate::task::spawn_small(name, job);
            match &mut self.resources {
                Resources::Live { publisher } => publisher.resume_live(),
                #[cfg(test)] Resources::Fixture(_) => {}
            }
        }
    }
    /// Controlled Home executes NO resource operation: the owner never leaves Idle in the
    /// synthetic Jellyfin world (#166), so any commit that reaches the adapter is stale
    /// authority. It cannot launch account workers, mint identity, write credentials, or invoke
    /// platform coordination.
    pub(crate) fn controlled_home(mt: &crate::task::MainThread, replay: bool) -> Self {
        let mut adapter = Self::empty(|_, _| false, Resources::Live {
            publisher: crate::plex::session::ProfilePublisher::scoped(mt),
        });
        adapter.controlled_home = true;
        adapter.replay_resources = replay;
        adapter
    }
    #[cfg(test)]
    pub(crate) fn live_resources_for_test(mt: &crate::task::MainThread) -> Self {
        crate::testlock::assert_held("live Session resource test");
        let mut adapter = Self::empty(|_, _| false, Resources::Live {
            publisher: crate::plex::session::ProfilePublisher::new(mt),
        });
        adapter.resource_test_io = Some(ResourceTestIo { recording_root: None,
            coordinator_events: Vec::new(), erase_sweeps: Vec::new() });
        adapter
    }

    pub(crate) fn live(_mt: &crate::task::MainThread) -> Self {
        Self::empty(|name, job| crate::task::spawn_small(name, job), Resources::Live {
            publisher: crate::plex::session::ProfilePublisher::new(_mt),
        })
    }
    #[cfg(all(test, feature = "jellyfin"))]
    pub(crate) fn live_recording_resources_for_test(mt: &crate::task::MainThread, root: std::path::PathBuf) -> Self {
        assert!(root.is_absolute() && root.is_dir());
        let mut adapter = Self::live_resources_for_test(mt);
        adapter.resource_test_io.as_mut().unwrap().recording_root = Some(root);
        adapter
    }
    #[cfg(test)]
    pub(crate) fn fixture() -> Self { Self::fixture_with(crate::plex::session::Session::default()) }

    #[cfg(test)]
    pub(crate) fn fixture_with(disk: crate::plex::session::Session) -> Self {
        Self::empty(|_, _| false, Resources::Fixture(FixtureResources {
            disk, registry_writes: Vec::new(), profile: None,
            coordinator_events: Vec::new(), sweep_leftovers: 0,
            next_completion_outcome: None,
        }))
    }

    fn empty(spawn: fn(&'static str, Box<dyn FnOnce() + Send>) -> bool, resources: Resources) -> Self {
        Self { resources, controlled_home: false, replay_resources: false,
            live_completion: None, erasures: Default::default(), commits: Default::default(), spawn, main_thread: PhantomData, recording_leftovers: 0,
            #[cfg(test)] resource_test_io: None }
    }

    fn commit_current(&self, _req: u32, _plan: &CommitPlan) -> bool {
        // The registry plans this validated are gone with the plex.tv account surface;
        // the remaining registry plan (Install) carries no per-request lifecycle.
        true
    }

    pub(crate) fn begin_commit(&mut self, permit: CommitPermit<'_>, plan: &CommitPlan) -> Option<CommitReply> {
        if !self.controlled_home && matches!(self.resources, Resources::Live { .. }) && plan.credentials.is_some() {
            if !self.commit_current(permit.request(), plan) {
                return Some(permit.reply(crate::auth::owner::CommitAdmission::StaleAuthority));
            }
            self.commits.push_back(PendingCommit {
                req: permit.request(), epoch: permit.epoch(), arrival: permit.arrival(),
                plan: plan.clone(), cancelled: Arc::new(AtomicBool::new(false)), ticket: None, retry_at: crate::app::clock::now(),
            });
            self.submit_commit();
            None
        } else { Some(self.commit(permit, plan)) }
    }

    fn submit_commit(&mut self) {
        // A refused clear retains its place. Wait for its completion before admitting newer credentials.
        if !self.erasures.is_empty() { return; }
        let Some(pending) = self.commits.front_mut() else { return; };
        let now = crate::app::clock::now();
        // SDL's millisecond counter wraps; the retry is due within the forward half-range.
        if pending.ticket.is_some() || now.wrapping_sub(pending.retry_at) >= u32::MAX / 2 { return; }
        let plan = pending.plan.clone();
        let cancelled = Arc::clone(&pending.cancelled);
        pending.ticket = crate::storage_worker::submit(move || {
            let result = if cancelled.load(Ordering::Acquire) { Err(()) } else { write_credentials(&plan, &cancelled) };
            crate::ui::idle::wake();
            crate::ui::present::wake_from_worker();
            result
        }).ok();
        pending.retry_at = now.wrapping_add(STORAGE_RETRY_MS);
    }

    pub(crate) fn cancel_superseded_commits(&mut self, current: impl Fn(u32, u64, u64) -> bool) {
        for pending in &self.commits {
            if !current(pending.req, pending.epoch, pending.arrival) {
                pending.cancelled.store(true, Ordering::Release);
            }
        }
    }

    pub(crate) fn take_committed(&mut self) -> Option<CompletedCommit> {
        self.submit_commit();
        let pending = self.commits.front()?;
        let disk = match pending.ticket.as_ref()?.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return None,
            // A failed worker cannot establish a successful commit; the owner receives refusal.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(()),
            Ok(disk) => disk,
        };
        let pending = self.commits.pop_front().expect("pending credential commit");
        Some(CompletedCommit { req: pending.req, epoch: pending.epoch, arrival: pending.arrival,
            plan: pending.plan, disk })
    }

    pub(crate) fn finish_commit(&mut self, permit: CommitPermit<'_>, completed: CompletedCommit) -> CommitReply {
        self.commit_with_disk(permit, &completed.plan, Some(completed.disk))
    }

    #[cfg(all(test, feature = "jellyfin"))]
    pub(crate) fn persistence_pending(&self) -> bool { !self.commits.is_empty() || !self.erasures.is_empty() }

    pub(crate) fn commit(&mut self, permit: CommitPermit<'_>, plan: &CommitPlan) -> CommitReply {
        self.commit_with_disk(permit, plan, None)
    }

    fn commit_with_disk(&mut self, permit: CommitPermit<'_>, plan: &CommitPlan,
        disk: Option<DiskCommit>) -> CommitReply {

        use crate::auth::owner::CommitAdmission;
        if self.controlled_home {
            // The controlled world's only resource operation was the DevPms DevInstall, and the
            // dev activation that issued it is gone with that world (#166): the owner never
            // leaves Idle here, so any commit that still arrives is stale authority.
            return permit.reply(CommitAdmission::StaleAuthority);
        }
        if !self.commit_current(permit.request(), plan) {
            return permit.reply(CommitAdmission::StaleAuthority);
        }
        // Filled inside the borrow of `self.resources`, stored after it ends.
        let mut pending_completion = None;
        match &mut self.resources {
            Resources::Live { .. } => {
                let mut admitted_revision = None;
                if plan.credentials.is_some() {
                    let write = match disk.unwrap_or_else(|| write_credentials(plan, &AtomicBool::new(false))) {
                        Ok(write) => write,
                        Err(()) => return permit.reply(CommitAdmission::StaleAuthority),
                    };
                    if plan.writes_durable && write.is_none()
                        && plan.authority == crate::plex::session::SaveAuthority::FreshReauthentication {
                        // The edit produced nothing to write (no account token to persist) —
                        // never admit a write that never happened.
                        return permit.reply(CommitAdmission::Rejected {
                            revision: None,
                            rejection: crate::plex::session::async_persistence::RejectionKind::Uninitialized,
                        });
                    }
                    if plan.writes_durable {
                        let revision = ordinary_revision();
                        admitted_revision = Some(revision);
                        // Build the completion from what the write ACTUALLY did, and decide it in
                        // exactly ONE place: `LiveWrite::classify` rebuilds the synchronous write
                        // as a `DiskOutcome` and routes it through `DiskOutcome::classify`, the
                        // same arms the asynchronous path uses. So a canonical commit that came
                        // back `Uncertain` reports `CompletionOutcome::Uncertain{stage,errno}` —
                        // even though the legacy write beneath it succeeded and the old bool
                        // collapse therefore reported it as a durable saved login — and a
                        // `ProtectionFailed` commit reports `ProtectionUncertain` or
                        // `Failed(Failure::Protection(..))` by its own preservation evidence.
                        pending_completion = write.map(|write| {
                            crate::plex::session::async_persistence::PersistenceCompletion {
                                req: permit.request(), epoch: permit.epoch(),
                                arrival: permit.arrival(), revision,
                                purpose: plan.purpose,
                                outcome: write.classify(),
                            }
                        });
                    }
                }
                for operation in &plan.registry {
                    if !crate::auth::execute_session_registry(operation, &plan.registry_client_id) {
                        return permit.reply(CommitAdmission::StaleAuthority);
                    }
                }
                if let Some(revision) = admitted_revision {
                    self.live_completion = pending_completion;
                    return permit.reply(CommitAdmission::Admitted { revision, purpose: plan.purpose });
                }
            }
            #[cfg(test)]
            Resources::Fixture(resources) => {
                let mut admitted = None;
                if let Some(patch) = &plan.credentials {
                    // Mirror the Live arm's authority split (`replace_after_reauthentication_
                    // with_outcome`'s doc has the full reasoning): a Fresh write is fenced on
                    // disk identity only when the disk holds a READABLE record; an unreadable
                    // (empty `client_id`) disk never refuses a Fresh write, but — unlike a
                    // Routine write over the same unreadable disk — still merges and admits it,
                    // since Fresh's whole point is surviving exactly that case. A Routine write
                    // over an unreadable disk stays a no-op, matching the live
                    // `update_with_outcome`'s own `client_id.is_empty()` early return.
                    let fresh = plan.authority == crate::plex::session::SaveAuthority::FreshReauthentication;
                    let readable = !resources.disk.client_id.is_empty();
                    if readable && !plan.expected_disk.matches(&resources.disk) {
                        return permit.reply(CommitAdmission::StaleAuthority);
                    }
                    if readable || fresh {
                        resources.disk = patch.merge_into(&resources.disk);
                        admitted = Some(CommitAdmission::Admitted {
                            revision: 0, purpose: plan.purpose,
                        });
                    }
                }
                for operation in &plan.registry {
                    if !crate::auth::execute_session_registry(operation, &plan.registry_client_id) {
                        return permit.reply(CommitAdmission::StaleAuthority);
                    }
                }
                resources.registry_writes.extend(plan.registry.iter().cloned());
                if let Some(admission) = admitted {
                    if plan.writes_durable {
                        if let CommitAdmission::Admitted { revision, purpose } = admission {
                            // Fixture-driven flows still need a real completion to drain — a held
                            // handoff (AUTH-03/04) would otherwise stay held forever in a fixture
                            // test, since nothing else produces one for this path.
                            let outcome = resources.next_completion_outcome.take().unwrap_or(
                                crate::plex::session::async_persistence::CompletionOutcome::Durable(
                                    crate::plex::session::async_persistence::Operation::Write {
                                        outcome: crate::plex::session::async_persistence::PersistOutcome::PersistedPlaintext,
                                        verified: true, protection: None,
                                    }));
                            self.live_completion = Some(
                                crate::plex::session::async_persistence::PersistenceCompletion {
                                    req: permit.request(), epoch: permit.epoch(),
                                    arrival: permit.arrival(), revision, purpose, outcome,
                                });
                        }
                    }
                    return permit.reply(admission);
                }
            }
        }
        permit.reply(CommitAdmission::RegistryOnly)
    }

    /// Take the durability verdict of the last live credential write, if one is outstanding.
    /// Draining exactly once is what keeps a verdict from being delivered twice.
    pub(crate) fn take_live_completion(
        &mut self,
    ) -> Option<crate::plex::session::async_persistence::PersistenceCompletion> {
        self.live_completion.take()
    }

    pub(crate) fn publish_profile(&mut self, publication: crate::auth::owner::ProfilePublication) {
        match &mut self.resources {
            Resources::Live { publisher } => publisher.publish(publication.profile, publication.scope.0),
            #[cfg(test)]
            Resources::Fixture(resources) => resources.profile = Some(publication),
        }
    }

    /// Credential revocation is immediate; the physical clear runs on the persistence FIFO.
    pub(crate) fn begin_erase(&mut self, epoch: u64, all_local: bool,
        meta: &mut crate::stores::metadata::MetadataStore) -> Option<usize> {
        if matches!(self.resources, Resources::Live { .. }) {
            for commit in &self.commits { commit.cancelled.store(true, Ordering::Release); }
            crate::registry::revoke_all();
            crate::plex::session::revoke_cached_session();
            // The Jellyfin flavor's sign-out retires ITS credential instead of a Plex PIN: the
            // config file goes (the next boot asks again), the installed client is retired (the
            // stores fail closed at once), and the form's flow resets. One site covers both
            // sign-out and delete-all-local-data: every account teardown funnels through here.
            #[cfg(feature = "jellyfin")]
            {
                crate::jellyfin::boot::erase_config();
                crate::jellyfin::uninstall();
                crate::jellyfin::signin::reset();
                crate::jellyfin::quick_connect::reset();
            }
            let diagnostics = all_local;
            #[cfg(test)]
            let diagnostics = diagnostics && self.resource_test_io.is_none();
            if diagnostics { crate::storage::diagnostics::disable(); }
            self.erasures.push_back(PendingErase {
                epoch, all_local, diagnostics, ticket: None, retry_at: crate::app::clock::now(), incomplete: 0,
                language: None,
            });
            self.submit_erase();
            None
        } else { Some(self.finish_erase(all_local, meta, Vec::new())) }
    }

    fn submit_erase(&mut self) {
        let Some(pending) = self.erasures.front_mut() else { return; };
        let now = crate::app::clock::now();
        // SDL's millisecond counter wraps; the retry is due within the forward half-range.
        if pending.ticket.is_some() || now.wrapping_sub(pending.retry_at) >= u32::MAX / 2 { return; }
        let (diagnostics, all_local, language) = (pending.diagnostics, pending.all_local, pending.language);
        pending.ticket = crate::storage_worker::submit(move || {
            let erasure = crate::plex::session::clear_for_erase(all_local, language);
            let cleared = matches!(erasure.outcome,
                crate::plex::session::ClearOutcome::Durable { legacy_swept: true });
            // A sign-out that could not retain the language retries, carrying it; delete-all
            // reports a language it could not reset among its leftovers instead.
            let complete = cleared && (all_local || erasure.preference_failures.is_empty());
            if !complete {
                crate::log("session: queued clear incomplete; retaining revocation and retrying");
            }
            crate::plex::session::revoke_cached_session();
            let mut failures: Vec<String> = if diagnostics {
                crate::storage::diagnostics::finish_disable(crate::paths::runtime_dir())
                    .err()
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            };
            if all_local { failures.extend(erasure.preference_failures); }
            crate::ui::idle::wake();
            crate::ui::present::wake_from_worker();
            EraseWorkerOutcome { complete, failures, language: erasure.retained_language }
        }).ok();
        pending.retry_at = now.wrapping_add(STORAGE_RETRY_MS);
    }

    pub(crate) fn take_erased(&mut self, meta: &mut crate::stores::metadata::MetadataStore)
        -> Option<crate::auth::owner::SessionEvent> {
        self.submit_erase();
        let pending = self.erasures.front()?;
        let worker_failures = match pending.ticket.as_ref()?.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // Keep the request and retry through a restarted shared worker.
                self.erasures.front_mut()?.ticket = None;
                return None;
            }
            Ok(outcome) if !outcome.complete => {
                let pending = self.erasures.front_mut()?;
                pending.ticket = None;
                pending.language = Some(outcome.language);
                pending.incomplete = pending.incomplete.saturating_add(1);
                pending.retry_at = crate::app::clock::now().wrapping_add(erase_retry_delay(pending.incomplete));
                return None;
            }
            Ok(outcome) => {
                crate::i18n::set_saved_preference(outcome.language);
                outcome.failures
            }
        };
        let pending = self.erasures.pop_front().expect("pending clear");
        let leftovers = self.finish_erase(pending.all_local, meta, worker_failures);
        Some(crate::auth::owner::SessionEvent::Erased { epoch: pending.epoch, leftovers })
    }

    fn finish_erase(&mut self, all_local: bool, meta: &mut crate::stores::metadata::MetadataStore,
        worker_failures: Vec<String>) -> usize {
        let recording_leftovers = if all_local { std::mem::take(&mut self.recording_leftovers) } else { 0 };
        recording_leftovers + match &mut self.resources {
            Resources::Live { .. } => {
                #[cfg(test)]
                if let Some(io) = &mut self.resource_test_io {
                    io.erase_sweeps.push(all_local);
                    return recording_leftovers + if all_local {
                        io.recording_root.as_ref().map_or(0, |root| crate::ui::rec::erase_owned_artifacts(root,
                        super::super::input::remove_or_prove_absent).len())
                    } else { 0 }; // Other host cache/runtime sweeps remain disabled in resource tests.
                }
                crate::imgcache::clear();
                if all_local {
                    let leftovers = super::super::input::delete_all_local_data(meta, worker_failures);
                    if super::super::input::delete_outcome(leftovers.len()).report_leftovers {
                        crate::log(&format!("privacy: local data erased; {} file(s) could not be removed: {}",
                            leftovers.len(), leftovers.join("; ")));
                    }
                    leftovers.len()
                } else { 0 }
            }
            #[cfg(test)]
            Resources::Fixture(resources) => {
                resources.disk = Default::default();
                resources.registry_writes.clear();
                if all_local { resources.sweep_leftovers } else { 0 }
            }
        }
    }

    pub(crate) fn coordinator(&mut self, action: crate::auth::owner::CoordinatorAction) {
        use crate::auth::owner::CoordinatorAction;
        #[cfg(test)]
        if let Some(io) = &mut self.resource_test_io {
            io.coordinator_events.push(action);
            return;
        }
        #[cfg(test)]
        if let Resources::Fixture(resources) = &mut self.resources {
            resources.coordinator_events.push(action);
            return;
        }
        match action {
            CoordinatorAction::LocalDataErased => {}
        }
    }

    pub(crate) fn profile_resource_view(&self) -> Option<std::sync::Arc<crate::plex::session::CurrentProfile>> {
        match &self.resources {
            Resources::Live { publisher } => Some(publisher.snapshot()),
            #[cfg(test)]
            Resources::Fixture(_) => None,
        }
    }

    /// Recorded-ingest shape checks. No session envelopes can be produced any more (the
    /// roster/endpoint workers are gone), so every recording supplies an empty batch; the checks
    /// keep the recorded-ingest contract's shape for it.
    pub(crate) fn validate_supplied(&self, records: &[(Addr, SessionEnvelope)]) -> Result<(), SessionIngressError> {
        if records.len() > SESSION_TRANSFER_RECORDS { return Err(SessionIngressError::BatchTooLarge); }
        for (outer, envelope) in records {
            if *outer != envelope.addr { return Err(SessionIngressError::AddressMismatch); }
        }
        Ok(())
    }

    /// Receipts no longer exist (the envelope producers are gone); the owner's acknowledge
    /// effect remains part of the frozen effect vocabulary and is accepted as a no-op.
    pub(crate) fn acknowledge(&mut self, _receipts: &[Receipt]) {}

    pub(crate) fn cancel(&mut self, req: RequestId) {
        for commit in &self.commits {
            if commit.req == req.0 { commit.cancelled.store(true, Ordering::Release); }
        }
    }

    pub(crate) fn cancel_all(&mut self) {
        for commit in &self.commits { commit.cancelled.store(true, Ordering::Release); }
    }


}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stage B bridge wiring, exercised against the REAL disk writer (not the fixture arm, which
    /// is not a durability authority). The adapter must hand the bridge the verdict the write
    /// actually produced so the owner can receive it as a typed completion, and the drain must
    /// hand it over exactly once.
    fn queued_plan(disk: &crate::plex::session::Session) -> CommitPlan {
        let mut next = disk.clone();
        next.account_token = "new-credential".into();
        CommitPlan { registry_client_id: disk.client_id.clone(), expected_disk: crate::auth::owner::Identity::of(disk),
            credentials: Some(crate::auth::owner::CredentialPatch::of(&next)),
            lifecycle: None, registry: Vec::new(), writes_durable: true,
            purpose: crate::plex::session::async_persistence::PersistencePurpose::Final,
            authority: crate::plex::session::SaveAuthority::Routine }
    }

    fn enqueue_test_commit(adapter: &mut SessionAdapter, plan: CommitPlan) {
        adapter.commits.push_back(PendingCommit { req: 1, epoch: 1, arrival: 0, plan,
            cancelled: Arc::new(AtomicBool::new(false)), ticket: None, retry_at: crate::app::clock::now() });
    }

    fn check_retry_uses_frame_time(erase: bool, start: u32) {
        let _serial = crate::testlock::serial();
        let _session = crate::plex::session::TempSession::new("logical-storage-retry");
        struct ResetClock;
        impl Drop for ResetClock {
            fn drop(&mut self) { crate::app::clock::set_replay(0); }
        }
        let _clock = ResetClock;
        crate::app::clock::set_replay(start);
        let mt = unsafe { crate::task::MainThread::assume() };
        let mut adapter = SessionAdapter::live_resources_for_test(&mt);
        if erase {
            adapter.erasures.push_back(PendingErase { epoch: 1, all_local: false, diagnostics: false, ticket: None,
                retry_at: crate::app::clock::now(), incomplete: 0, language: None });
        } else {
            enqueue_test_commit(&mut adapter, queued_plan(&crate::plex::session::Session::default()));
        }
        let (release, held) = std::sync::mpsc::channel();
        let (started, entered) = std::sync::mpsc::channel();
        let _block = crate::storage_worker::submit(move || {
            started.send(()).unwrap();
            let _ = held.recv();
        }).unwrap();
        entered.recv().unwrap();
        for _ in 0..crate::storage_worker::CAPACITY {
            let _ = crate::storage_worker::submit(|| ()).unwrap();
        }
        let submit = |adapter: &mut SessionAdapter| {
            let _frame = crate::task::FrameScope::enter();
            if erase { adapter.submit_erase(); } else { adapter.submit_commit(); }
        };
        let admitted = |adapter: &SessionAdapter| {
            if erase { adapter.erasures.front().unwrap().ticket.is_some() }
            else { adapter.commits.front().unwrap().ticket.is_some() }
        };
        submit(&mut adapter);
        let initially_admitted = admitted(&adapter);
        release.send(()).unwrap();
        crate::storage_worker::drain_for_test();
        assert!(!initially_admitted, "the full queue must arm backoff");
        crate::app::clock::set_replay(start.wrapping_add(999));
        submit(&mut adapter);
        assert!(!admitted(&adapter), "retry must wait for a full logical second");
        crate::app::clock::set_replay(start.wrapping_add(1_000));
        submit(&mut adapter);
        assert!(admitted(&adapter), "advancing frame time must release the retry without a real sleep");
    }

    #[test]
    fn credential_retry_uses_logical_frame_time() {
        check_retry_uses_frame_time(false, 100);
    }

    #[test]
    fn erase_retry_uses_logical_frame_time_across_tick_wrap() {
        check_retry_uses_frame_time(true, u32::MAX - 500);
    }

    #[test]
    fn cancellation_while_waiting_for_session_io_prevents_the_credential_write() {
        let _serial = crate::testlock::serial();
        let _session = crate::plex::session::TempSession::new("cancel-before-io");
        let disk = crate::plex::session::Session { client_id: "install".into(), account_token: "old".into(), ..Default::default() };
        crate::plex::session::save(&disk);
        let mt = unsafe { crate::task::MainThread::assume() };
        let mut adapter = SessionAdapter::live_resources_for_test(&mt);
        enqueue_test_commit(&mut adapter, queued_plan(&disk));
        let (started, entered) = std::sync::mpsc::channel();
        *CREDENTIAL_IO_STARTED.lock().unwrap() = Some(started);
        crate::plex::session::with_io_for_test(|| {
            adapter.submit_commit();
            entered.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            adapter.cancel(RequestId(1));
        });
        crate::storage_worker::drain_for_test();
        assert_eq!(crate::plex::session::load().account_token, "old");
    }

    #[test]
    fn a_refused_erase_keeps_later_credentials_out_of_the_worker_queue() {
        let _serial = crate::testlock::serial();
        let _session = crate::plex::session::TempSession::new("erase-before-commit");
        let mt = unsafe { crate::task::MainThread::assume() };
        let mut adapter = SessionAdapter::live_resources_for_test(&mt);
        adapter.erasures.push_back(PendingErase { epoch: 1, all_local: false, diagnostics: false, ticket: None,
            retry_at: crate::app::clock::now().wrapping_add(STORAGE_RETRY_MS), incomplete: 0, language: None });
        enqueue_test_commit(&mut adapter, queued_plan(&crate::plex::session::Session::default()));
        adapter.submit_commit();
        assert!(adapter.commits.front().unwrap().ticket.is_none(), "erase retains its FIFO position");
    }

    /// The webOS 4.10.2 mount: the session candidate does not exist and every unlink in its
    /// directory answers EROFS. Returns the fault guard and the adapter.
    fn erase_behind_a_read_only_mount(session: &crate::plex::session::TempSession,
        mt: &crate::task::MainThread) -> (crate::storage::UnlinkFaultForTest, SessionAdapter) {
        crate::plex::session::save(&crate::plex::session::Session {
            client_id: "install".into(), account_token: "old".into(), ..Default::default() });
        std::fs::remove_file(session.path()).unwrap();
        let erofs = crate::storage::UnlinkFaultForTest::install(session.path().parent().unwrap(), libc::EROFS);
        (erofs, SessionAdapter::live_resources_for_test(mt))
    }

    /// Before the fix the erase never completed, and `submit_commit` holds every credential commit
    /// behind a pending erase — so a sign-in after that sign-out was never persisted at all.
    #[test]
    fn a_signin_after_signout_behind_a_read_only_mount_is_written() {
        let _serial = crate::testlock::serial();
        let session = crate::plex::session::TempSession::new("signin-after-erofs-signout");
        let mt = unsafe { crate::task::MainThread::assume() };
        let (_erofs, mut adapter) = erase_behind_a_read_only_mount(&session, &mt);
        let mut meta = crate::stores::metadata::MetadataStore::default();
        assert!(adapter.begin_erase(1, false, &mut meta).is_none());
        crate::storage_worker::drain_for_test();
        assert!(matches!(adapter.take_erased(&mut meta), Some(crate::auth::owner::SessionEvent::Erased { epoch: 1, .. })),
            "an absent candidate behind EROFS must not hold the sign-out open");
        // A QR sign-in persists under the fresh-reauthentication authority.
        let plan = CommitPlan { authority: crate::plex::session::SaveAuthority::FreshReauthentication,
            ..queued_plan(&crate::plex::session::load()) };
        enqueue_test_commit(&mut adapter, plan);
        assert!(adapter.take_committed().is_none(), "admitted, not yet run");
        crate::storage_worker::drain_for_test();
        let completed = adapter.take_committed().expect("the new sign-in's commit runs");
        assert!(matches!(completed.disk, Ok(Some(_))), "the new credential is written");
        assert_eq!(crate::plex::session::load().account_token, "new-credential");
    }

    /// "Delete all local data" rides the same erase queue; its sweep and the route to sign-in only
    /// run from `finish_erase`, which the stuck clear never reached on the television.
    #[test]
    fn delete_all_local_data_behind_a_read_only_mount_reaches_its_sweep() {
        let _serial = crate::testlock::serial();
        let session = crate::plex::session::TempSession::new("erase-local-erofs");
        let mt = unsafe { crate::task::MainThread::assume() };
        let (_erofs, mut adapter) = erase_behind_a_read_only_mount(&session, &mt);
        let mut meta = crate::stores::metadata::MetadataStore::default();
        assert!(adapter.begin_erase(7, true, &mut meta).is_none());
        crate::storage_worker::drain_for_test();
        assert!(matches!(adapter.take_erased(&mut meta), Some(crate::auth::owner::SessionEvent::Erased { epoch: 7, .. })),
            "the all-local erase completes");
        assert_eq!(adapter.resource_test_io.as_ref().unwrap().erase_sweeps, [true],
            "finish_erase ran the all-local sweep");
    }

    /// PR #265 review: the app language is install-wide ("all users on this TV"), so the erase
    /// queue's ordinary sign-out must keep it across a relaunch while revoking every credential.
    /// Only "Delete all local data" returns it to System, both on disk and as the confirmed
    /// next-launch preference the Language screen shows.

    #[test]
    fn erase_retry_delay_doubles_to_its_cap() {
        let delays: Vec<u32> = (1..=8).map(erase_retry_delay).collect();
        assert_eq!(delays, [1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000, 30_000]);
        assert_eq!(erase_retry_delay(u32::MAX), ERASE_RETRY_CAP_MS);
    }

    /// A credential that really survives the clear keeps the erase queued (the revocation is
    /// never dropped), but its retries back off to [`ERASE_RETRY_CAP_MS`] instead of rewriting
    /// the revocation marker and logging every second; once the file can go, the same erase
    /// completes.
    #[test]
    fn a_clear_that_leaves_a_credential_retries_with_capped_backoff_and_never_gives_up() {
        use std::os::unix::fs::PermissionsExt;
        let _serial = crate::testlock::serial();
        let session = crate::plex::session::TempSession::new("erase-capped-backoff");
        crate::plex::session::save(&crate::plex::session::Session {
            client_id: "install".into(), account_token: "old".into(), ..Default::default() });
        let file = session.path();
        let dir = file.parent().unwrap().to_path_buf();
        struct Restore(std::path::PathBuf, std::path::PathBuf);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
                let _ = std::fs::set_permissions(&self.1, std::fs::Permissions::from_mode(0o600));
                crate::app::clock::set_replay(0);
            }
        }
        let _restore = Restore(dir.clone(), file.clone());
        // Neither unlink (directory) nor neutralize (file) is permitted: a real survivor.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o400)).unwrap();
        crate::app::clock::set_replay(100);
        let mt = unsafe { crate::task::MainThread::assume() };
        let mut adapter = SessionAdapter::live_resources_for_test(&mt);
        let mut meta = crate::stores::metadata::MetadataStore::default();
        assert!(adapter.begin_erase(1, false, &mut meta).is_none());
        let mut waits = Vec::new();
        for _ in 0..7 {
            crate::storage_worker::drain_for_test();
            assert!(adapter.take_erased(&mut meta).is_none(), "the revocation is retained");
            let due = adapter.erasures.front().expect("the erase stays queued").retry_at;
            waits.push(due.wrapping_sub(crate::app::clock::now()));
            crate::app::clock::set_replay(due.wrapping_sub(1));
            adapter.submit_erase();
            assert!(adapter.erasures.front().unwrap().ticket.is_none(), "no retry before it is due");
            crate::app::clock::set_replay(due);
            adapter.submit_erase();
            assert!(adapter.erasures.front().unwrap().ticket.is_some(), "the due retry runs");
        }
        assert_eq!(waits, [1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000]);
        crate::storage_worker::drain_for_test();
        assert!(adapter.take_erased(&mut meta).is_none(), "the eighth attempt fails too");
        assert!(file.exists(), "the credential really survived every attempt");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        crate::app::clock::set_replay(adapter.erasures.front().unwrap().retry_at);
        let completed = adapter.take_erased(&mut meta);
        crate::storage_worker::drain_for_test();
        let completed = completed.or_else(|| adapter.take_erased(&mut meta));
        assert!(matches!(completed, Some(crate::auth::owner::SessionEvent::Erased { epoch: 1, .. })),
            "the next due retry retires the file once it can go");
        assert!(!file.exists());
    }



    /// A disk write can fail (a revoked, missing or unwritable credential path), and the bridge
    /// must surface exactly that — `CompletionOutcome::Failed`, never a silently upgraded
    /// `Durable` — mirroring `DiskOutcome::classify`'s convention that only a persisted write is
    /// durable. Without this, `auth::owner::apply_persistence_completion` would key
    /// `commit_phase.durable` / `commit_phase.proves_saved_login` off a write that never reached
    /// disk.
    ///
    /// RED: observed live (against the shape this mapping had at `f6e19098`). Reverting `commit`'s
    /// write-outcome mapping to the old unconditional
    /// `CompletionOutcome::Durable(Operation::Write { outcome, verified: outcome.persisted(),
    /// protection: None })` construction (i.e. dropping the `if outcome.persisted() { .. } else
    /// { Failed(..) }` branch this test exists to pin) and re-running just this test failed on
    /// the final `matches!` assertion: `take_live_completion()` returned `Durable(..)` even
    /// though the underlying disk write on the read-only directory genuinely failed
    /// (`PersistOutcome::WriteFailed`). Reapplying the fix turns it back green. That branch is no
    /// longer written out here — `commit` now hands the whole `LiveWrite` to
    /// `LiveWrite::classify`, so the same verdict comes from `DiskOutcome::classify`'s arms — and
    /// this test pins the behavior across that move: a `TempSession` fixture bypasses the
    /// canonical store, so the write arrives with no `CanonicalCommit` and takes exactly the
    /// absent-commit-detail arm the old inline branch imitated.

    /// Finding 1's regression: a canonical commit that comes back `Uncertain` must surface as
    /// `CompletionOutcome::Uncertain`, never `Durable`, even though the legacy plaintext
    /// fall-through write beneath it succeeds. This is the exact scenario the finding names — a
    /// `CanonicalCommit::Uncertain` (a real state: a first sign-in where keymanager3 is absent)
    /// falling through to a legacy write that succeeds, previously collapsed to `Some(false)` and
    /// reported as a durable saved login.
    ///
    /// This must run through the REAL canonical authority, not `TempSession`'s `TEST_FILE`
    /// bypass — `save_locked_with_authority`'s own `#[cfg(test)]` guard routes a `TEST_FILE`
    /// redirect straight to the legacy writer and never calls `persistence::write_session` at
    /// all, so a `TempSession`-based test cannot see this finding. Instead this redirects the
    /// canonical persistence root (mirroring `plex::session`'s own private `TempCanonicalRoot`,
    /// which cannot be reused across the crate boundary) and forces a real `CanonicalCommit::
    /// Uncertain` through the production fault-injection seam `storage::
    /// inject_next_commit_failure_for_test`, consumed only at `CommitStage::ParentSync` — after
    /// the record has actually been renamed into place, so the write is real, just reported
    /// uncertain, exactly the state the finding describes.
    ///
    /// Isolation: holds `testlock::serial()` for the whole body; owns its own canonical-root
    /// scratch directory (RAII, restored on drop); and snapshots/restores the process-global
    /// legacy fallback file the fall-through write lands in, so no other test in this process
    /// observes residue from it.
    ///
    /// RED: OBSERVED, against a narrower mutation than an earlier version of this note claimed.
    /// The `save_locked_with_authority`/`LiveWrite`/`classify` widening this test guards is
    /// already in place at the base this test was written against, and reverting that signature
    /// cannot even compile against this test (it calls `write.classify()`, which does not exist
    /// on `Option<bool>`). The mutation actually run left the widening untouched and replaced
    /// only THIS `commit` arm's `outcome: write.classify()` with the pre-widening inline mapping
    /// — `if write.outcome.persisted() { Durable(Operation::Write{ verified: true, .. }) } else {
    /// Failed(Failure::Persistence(write.outcome)) }` — applied to the current `LiveWrite` value.
    /// Under that mutation this test failed, observed as:
    /// `Durable(Write { outcome: PersistedPlaintext, verified: true, protection: None })`.
    /// Discrimination check: the same mutation leaves `the_bridge_reports_a_failed_write_as_
    /// failed_not_durable` passing — that test's `TempSession` fixture drives a genuinely failed
    /// disk write, so `outcome.persisted()` is `false` either way and the inline mapping produces
    /// the same `Failed` verdict `classify()` would — so this test discriminates a distinct
    /// failure mode from that one, and the claim above is the one this test's own mutation run
    /// actually supports.
    ///
    /// A second, narrower mutation also goes red under this test: leaving `commit` untouched and
    /// instead changing `LiveWrite::disk_outcome`'s `CanonicalCommit::Uncertain` arm
    /// (`async_persistence.rs`) to build its `DiskOutcome::Write` with `commit: None` instead of
    /// `commit: Some(CommitDetail::Uncertain{..})` — `classify`'s absent-commit-detail arm then
    /// falls through to `outcome.persisted()`, which is `true` for the successful legacy write,
    /// again yielding `Durable(..)`.

    /// Companion to `a_fresh_sign_in_over_an_unopenable_envelope_is_persisted_for_the_next_launch`:
    /// that test proves the UNREADABLE-disk half of the narrowed fence in
    /// `replace_after_reauthentication_with_outcome`; this one proves the other half — a write
    /// over a disk that DOES hold a readable record a concurrent actor has already replaced must
    /// still be refused, for both `Routine` and `FreshReauthentication` authority. Before this
    /// test nothing in this module exercised the live adapter's `examined && !matches` refusal at
    /// all: every other `StaleAuthority` assertion here is either against a synthetic
    /// `CommitReply` or against the `Fixture` resource arm, never the real disk writer.
    ///
    /// RED: OBSERVED. Deleting `if !cur.client_id.is_empty() && !fence(&cur) { return Err(()); }`
    /// from `replace_after_reauthentication_with_outcome` (the fresh half of this test) and
    /// deleting `matches = plan.expected_disk.matches(disk);` from the `Routine` arm of
    /// `SessionAdapter::commit` (the routine half) each turned the corresponding iteration's
    /// `assert!(matches!(reply.admission, CommitAdmission::StaleAuthority))` into a failure — the
    /// write was silently admitted over the external replacement instead.


    /// A worker can start as soon as its job is submitted. Native and cache revocation must
    /// therefore precede submission, not merely precede a later completion being delivered.
    #[test]
    fn erase_revokes_in_memory_credentials_before_the_canonical_clear() {
        let source = include_str!("session.rs");
        let begin = source.split("pub(crate) fn begin_erase(").nth(1).unwrap()
            .split("fn submit_erase(").next().unwrap();
        let submitted = begin.find("self.submit_erase()").unwrap();
        assert!(begin.find("crate::registry::revoke_all()").unwrap() < submitted);
        assert!(begin.find("crate::plex::session::revoke_cached_session()").unwrap() < submitted);
        let worker = source.split("fn submit_erase(").nth(1).unwrap()
            .split("pub(crate) fn take_erased(").next().unwrap();
        assert!(worker.contains("crate::plex::session::clear_for_erase(all_local, language)"));
    }
}

