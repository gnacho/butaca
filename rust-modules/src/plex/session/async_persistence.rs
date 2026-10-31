//! Session persistence outcome types and synchronous write classification.

use super::persistence::{CanonicalCommit, HelperEvidence, ProtectionFailure};
use crate::storage::wire::{AuthPreservation, ProtectionOutcome};
use crate::storage::{CommitStage, StoreError};
use crate::storage_worker::SubmitError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PersistOutcome {
    PersistedPlaintext,
    PersistedSealed,
    WriteFailed,
}
impl PersistOutcome {
    pub(crate) fn persisted(self) -> bool {
        self != Self::WriteFailed
    }
}

/// What a durable write is FOR. The caller uses this to refuse treating a background refresh as
/// proof that a login was saved; it is carried on admission and on the completion so the owner
/// never has to infer it from the outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum PersistencePurpose {
    Discovery,
    Final,
    Profile,
    Background,
}

impl PersistencePurpose {
    /// A background refresh is maintenance, never evidence that credentials reached disk.
    pub(crate) fn proves_saved_login(self) -> bool {
        matches!(self, Self::Discovery | Self::Final | Self::Profile)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Write {
        outcome: PersistOutcome,
        verified: bool,
        protection: Option<ProtectionOutcome>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Admission(SubmitError),
    Persistence(PersistOutcome),
    Storage(StoreError),
    Helper(crate::storage::wire::failure::HelperFailure, [Option<i32>; 8]),
    Protection(ProtectionFailure),
    WorkerDropped,
    Superseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompletionOutcome {
    Durable(Operation),
    Uncertain {
        stage: CommitStage,
        errno: i32,
        helper: Option<HelperEvidence>,
    },
    Failed(Failure),
    ProtectionUncertain(ProtectionFailure),
    /// The disk callback may have run, but a later admitted clear revoked this process tenure.
    Superseded,
}

impl CompletionOutcome {
    /// The warning and report consume the same completion-owned diagnostic, regardless of
    /// whether the helper failed outright or could not establish the commit's durability.
    pub(crate) fn helper_evidence(&self) -> (Option<crate::storage::wire::failure::HelperFailure>, [Option<i32>; 8]) {
        match self {
            Self::Uncertain { helper: Some((failure, errnos)), .. }
            | Self::Failed(Failure::Helper(failure, errnos)) => (Some(*failure), *errnos),
            _ => (None, [None; 8]),
        }
    }
}

/// Serializable projection of an [`AdmissionFailure`]. `SubmitError` belongs to the storage
/// worker's transport vocabulary and is deliberately not serialized into owner state, so the
/// owner-side typed rejection carries this closed, replay-safe enum instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum RejectionKind {
    /// Bounded FIFO had no capacity, or the executor was stopped/failed to start.
    Capacity,
    /// The same edit was already admitted; nothing new was enqueued.
    Duplicate,
    /// Protected fields were locked by storage protection.
    Locked,
    /// A public-only edit tried to change protected credentials.
    PublicOnly,
    /// No loaded snapshot to edit.
    Uninitialized,
    /// The process tenure was revoked; a fresh authorization is required.
    Revoked,
    /// The revision counter cannot advance.
    RevisionExhausted,
}

/// Owner-side identity of one persistence decision. The completion repeats it so a late result
/// can be fenced by request, epoch, arrival AND revision before it activates anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PersistenceCorrelation {
    pub(crate) req: u32,
    pub(crate) epoch: u64,
    pub(crate) arrival: u64,
}

/// A typed durability verdict for one admitted revision, fenced by owner correlation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PersistenceCompletion {
    pub(crate) req: u32,
    pub(crate) epoch: u64,
    pub(crate) arrival: u64,
    pub(crate) revision: u64,
    pub(crate) purpose: PersistencePurpose,
    pub(crate) outcome: CompletionOutcome,
}

impl PersistenceCompletion {
    /// True only when this completion still describes the current authority and revision. A stale
    /// completion must not activate registry/profile, clear a newer error, or free another
    /// request's admission credit.
    pub(crate) fn acts_on(self, current: PersistenceCorrelation, latest_revision: u64) -> bool {
        self.req == current.req
            && self.epoch == current.epoch
            && self.arrival == current.arrival
            && self.revision == latest_revision
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommitDetail {
    Durable,
    Uncertain { stage: CommitStage, errno: i32, helper: Option<HelperEvidence> },
    Failed(StoreError),
}

#[derive(Clone, Copy)]
enum DiskOutcome {
    ProtectionFailed(ProtectionFailure),
    Write {
        outcome: PersistOutcome,
        verified: bool,
        protection: Option<ProtectionOutcome>,
        commit: Option<CommitDetail>,
    },
}

impl DiskOutcome {
    fn classify(self) -> CompletionOutcome {
        match self {
            Self::ProtectionFailed(evidence) => {
                if evidence.preservation == AuthPreservation::Uncertain {
                    CompletionOutcome::ProtectionUncertain(evidence)
                } else {
                    CompletionOutcome::Failed(Failure::Protection(evidence))
                }
            }
            Self::Write {
                outcome,
                verified,
                protection,
                commit,
            } => match commit {
                Some(CommitDetail::Uncertain { stage, errno, helper }) => {
                    CompletionOutcome::Uncertain { stage, errno, helper }
                }
                Some(CommitDetail::Failed(error)) => {
                    CompletionOutcome::Failed(Failure::Storage(error))
                }
                Some(CommitDetail::Durable) | None if outcome.persisted() => {
                    CompletionOutcome::Durable(Operation::Write {
                        outcome,
                        verified,
                        protection,
                    })
                }
                _ => CompletionOutcome::Failed(Failure::Persistence(outcome)),
            },
        }
    }
}

/// What ONE synchronous live write actually did, with the canonical verdict un-collapsed.
///
/// The live adapter writes synchronously, so there is no `Receipt`/`Completion` round trip to
/// carry the canonical authority's own verdict back to the owner — the writer returns it here
/// instead, beside the legacy sealed/plaintext attempt's [`PersistOutcome`].
///
/// **Keeping the [`CanonicalCommit`] is the whole point.** `save_locked_with_authority` used to
/// reduce it to `Option<bool>` (sealed / plaintext / nothing) before anyone downstream could see
/// it, so a canonical commit that reported `Uncertain` — a real state on a first sign-in where
/// keymanager3 is absent, where control then falls through to the legacy write and that write
/// SUCCEEDS — reached the adapter as an ordinary "persisted" and became
/// `CompletionOutcome::Durable`, i.e. `proves_saved_login = true` for a write the canonical store
/// never confirmed. A bool cannot express that difference; this can.
pub(crate) struct LiveWrite {
    /// What the sealed/plaintext write attempt ended up doing.
    pub(crate) outcome: PersistOutcome,
    /// The canonical authority's own verdict, when this path consulted it. `None` only where the
    /// canonical store was deliberately bypassed (the `TEST_FILE` legacy-fixture path).
    pub(crate) commit: Option<CanonicalCommit>,
    helper_failure: Option<crate::storage::wire::failure::HelperFailure>,
    candidate_errnos: [Option<i32>; 8],
}

impl LiveWrite {
    /// The legacy-only path: `Some(true)` sealed, `Some(false)` plaintext, `None` nothing — with
    /// no canonical verdict to report, because the canonical store was never consulted.
    pub(crate) fn legacy(sealed: Option<bool>) -> Self {
        Self {
            outcome: Self::outcome_of(sealed),
            commit: None, helper_failure: None, candidate_errnos: [None; 8],
        }
    }

    /// The production path: the canonical verdict, plus whatever the legacy write did after it.
    pub(crate) fn canonical(commit: CanonicalCommit, sealed: Option<bool>) -> Self {
        Self {
            outcome: Self::outcome_of(sealed),
            commit: Some(commit), helper_failure: None,
            candidate_errnos: super::candidate_errnos(),
        }
    }

    pub(crate) fn with_helper_failure(mut self, failure: Option<crate::storage::wire::failure::HelperFailure>) -> Self {
        self.helper_failure = failure;
        self
    }

    fn outcome_of(sealed: Option<bool>) -> PersistOutcome {
        match sealed {
            Some(true) => PersistOutcome::PersistedSealed,
            Some(false) => PersistOutcome::PersistedPlaintext,
            None => PersistOutcome::WriteFailed,
        }
    }

    /// The durability verdict this write should be reported as.
    ///
    /// **This is not a second decision site.** It rewrites the write into a [`DiskOutcome`] and
    /// hands it to [`DiskOutcome::classify`], keeping exactly one set of match arms for turning a
    /// durability verdict into a [`CompletionOutcome`]. The helper diagnostic snapshot only
    /// enriches a non-durable verdict after classification; it cannot change durability.
    pub(crate) fn classify(&self) -> CompletionOutcome {
        let outcome = self.disk_outcome().classify();
        match (outcome, self.helper_failure) {
            (CompletionOutcome::Failed(Failure::Storage(StoreError::HelperUnavailable
                | StoreError::HelperAuthentication | StoreError::HelperProtocol)), Some(failure)) =>
                CompletionOutcome::Failed(Failure::Helper(failure, self.candidate_errnos)),
            (CompletionOutcome::Uncertain { stage, errno, helper: Some((failure, _)) }, _) =>
                CompletionOutcome::Uncertain { stage, errno, helper: Some((failure, self.candidate_errnos)) },
            _ => outcome,
        }
    }

    fn disk_outcome(&self) -> DiskOutcome {
        match &self.commit {
            Some(CanonicalCommit::ProtectionFailed(evidence)) => {
                DiskOutcome::ProtectionFailed(*evidence)
            }
            Some(CanonicalCommit::Durable {
                verified,
                protection,
                ..
            }) => DiskOutcome::Write {
                outcome: self.outcome,
                verified: *verified,
                protection: *protection,
                commit: Some(CommitDetail::Durable),
            },
            Some(CanonicalCommit::Uncertain { stage, errno, helper }) => DiskOutcome::Write {
                outcome: self.outcome,
                verified: false,
                protection: None,
                commit: Some(CommitDetail::Uncertain {
                    stage: *stage,
                    errno: *errno,
                    helper: *helper,
                }),
            },
            Some(CanonicalCommit::Failed(error)) => DiskOutcome::Write {
                outcome: self.outcome,
                verified: false,
                protection: None,
                commit: Some(CommitDetail::Failed(*error)),
            },
            // No canonical verdict was taken here, so the legacy write's own result is the only
            // evidence there is — `classify`'s absent-commit-detail arm.
            None => DiskOutcome::Write {
                outcome: self.outcome,
                verified: self.outcome.persisted(),
                protection: None,
                commit: None,
            },
        }
    }
}

static ORDINARY_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(crate) fn ordinary_revision() -> u64 {
    ORDINARY_REVISION.load(std::sync::atomic::Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncertain_write_keeps_helper_evidence_without_becoming_failed_or_durable() {
        use crate::storage::wire::failure::{HelperFailure, Stage};
        let helper = Some((HelperFailure::new(Stage::Db8, Some(-3963)), [None; 8]));
        let commit = CommitDetail::Uncertain { stage: CommitStage::Readback, errno: 0, helper };
        for disk in [
            DiskOutcome::Write { outcome: PersistOutcome::WriteFailed, verified: false,
                protection: None, commit: Some(commit) },
            DiskOutcome::Write { outcome: PersistOutcome::PersistedPlaintext, verified: false,
                protection: None, commit: Some(commit) },
        ] {
            assert_eq!(disk.classify(), CompletionOutcome::Uncertain {
                stage: CommitStage::Readback, errno: 0, helper });
        }
    }

    fn keymanager_failure() -> crate::storage::wire::KeymanagerFailure {
        crate::storage::wire::KeymanagerFailure {
            operation: crate::storage::wire::KeymanagerOperation::Seal,
            stage: crate::storage::wire::KeymanagerStage::Finish,
            code: crate::storage::wire::ErrorCode::Unavailable,
            category: crate::storage::wire::KeymanagerFailureCategory::Unavailable,
            service_code: None,
        }
    }

    fn protection_failure(preservation: AuthPreservation) -> ProtectionFailure {
        ProtectionFailure {
            failure: keymanager_failure(),
            preservation,
            db8_commit_verified: false,
        }
    }

    /// `live-write-protection-failed-arm-unpinned`: `LiveWrite::disk_outcome` must keep routing a
    /// `CanonicalCommit::ProtectionFailed` verdict through `DiskOutcome::ProtectionFailed` (never
    /// through `DiskOutcome::Write`, which is what silently upgrades it to `Durable` whenever the
    /// legacy fall-through write beneath it happens to succeed). This is a real production shape:
    /// a first sign-in where keymanager3 is present but sealing itself fails.
    ///
    /// RED: OBSERVED. Temporarily replacing the `Some(CanonicalCommit::ProtectionFailed(evidence))
    /// => DiskOutcome::ProtectionFailed(*evidence)` arm in `disk_outcome` with
    /// `DiskOutcome::Write { outcome: self.outcome, verified: self.outcome.persisted(),
    /// protection: None, commit: None }` (the exact collapse P1's report described making, then
    /// backing out) makes both assertions below fail: with a successful legacy write
    /// (`Some(false)` => `PersistedPlaintext`, which `persisted()` reports `true`), the mutated
    /// code reports `CompletionOutcome::Durable(..)` for every `preservation` value instead of
    /// `ProtectionUncertain`/`Failed(Failure::Protection(..))`.
    #[test]
    fn a_protection_failure_never_reports_durable_even_when_the_legacy_write_succeeds() {
        let uncertain = LiveWrite::canonical(
            CanonicalCommit::ProtectionFailed(protection_failure(AuthPreservation::Uncertain)),
            Some(false),
        );
        assert!(
            matches!(
                uncertain.classify(),
                CompletionOutcome::ProtectionUncertain(evidence)
                    if evidence.preservation == AuthPreservation::Uncertain
            ),
            "an Uncertain-preservation protection failure must report ProtectionUncertain, not \
             Durable, even though the legacy write succeeded: {:?}",
            uncertain.classify(),
        );

        for preservation in [AuthPreservation::Unchanged, AuthPreservation::Restored] {
            let write = LiveWrite::canonical(
                CanonicalCommit::ProtectionFailed(protection_failure(preservation)),
                Some(false),
            );
            assert!(
                matches!(
                    write.classify(),
                    CompletionOutcome::Failed(Failure::Protection(evidence))
                        if evidence.preservation == preservation
                ),
                "a non-Uncertain protection failure ({:?}) must report Failed(Protection(..)), \
                 not Durable, even though the legacy write succeeded: {:?}",
                preservation,
                write.classify(),
            );
        }
    }

    /// Companion to the test above: the `CanonicalCommit::Failed(StoreError)` arm must likewise
    /// stay `Failed`, never `Durable`, when the legacy fall-through write succeeds underneath it.
    /// Not the same arm as `ProtectionFailed` — pinned separately because the two are matched on
    /// different `CanonicalCommit` variants in `disk_outcome`, and a fix that only widens one of
    /// them would leave the other collapsed.
    #[test]
    fn a_failed_canonical_commit_never_reports_durable_even_when_the_legacy_write_succeeds() {
        let write = LiveWrite::canonical(CanonicalCommit::Failed(StoreError::RootChanged), Some(true));
        assert!(
            matches!(
                write.classify(),
                CompletionOutcome::Failed(Failure::Storage(StoreError::RootChanged))
            ),
            "a failed canonical commit must report Failed(Storage(..)), not Durable, even though \
             the legacy write succeeded: {:?}",
            write.classify(),
        );
    }

    /// Third sibling of the two tests above, for the `CanonicalCommit::Uncertain` arm — the one
    /// the runtime-dir fallback (`paths::session_candidates`) actually produces in the field: the
    /// 2026-09-20 report's own log line (`session: canonical write is uncertain stage=… errno=…`)
    /// is this exact variant, immediately followed by the legacy write that now also tries the
    /// runtime directory. A save that lands ONLY there is not durable — `/tmp` is swept on
    /// reboot — and this pins that `classify()` already reports it as `Uncertain`, the same
    /// non-durable class ANY legacy-only success under an unconfirmed canonical commit gets, never
    /// as `Durable`, regardless of which candidate in `session_candidates()` the legacy write
    /// actually landed on.
    #[test]
    fn an_uncertain_canonical_commit_never_reports_durable_even_when_the_runtime_dir_fallback_succeeds() {
        let write = LiveWrite::canonical(
            CanonicalCommit::Uncertain { stage: CommitStage::Rename, errno: 13, helper: None },
            Some(false), // the runtime-dir candidate's own write: plaintext, no key manager
        );
        assert!(
            matches!(
                write.classify(),
                CompletionOutcome::Uncertain { stage: CommitStage::Rename, errno: 13, helper: None }
            ),
            "an uncertain canonical commit must report Uncertain, not Durable, even though the \
             runtime-dir fallback accepted the write: {:?}",
            write.classify(),
        );
    }
}
