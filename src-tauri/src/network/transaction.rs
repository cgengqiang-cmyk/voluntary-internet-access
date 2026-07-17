use std::{fmt::Debug, sync::Mutex};

use chrono::Utc;
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use super::{
    LeaseJournal, LeaseMode, LoopbackProxyTarget, NetworkError, NetworkLease, NetworkResult,
};

// The desktop application is single-instance, and this process-wide lock also
// prevents accidental races if separate subsystems construct their own
// `ProxyTransaction` handles around the same journal.
static NETWORK_OPERATION_LOCK: Mutex<()> = Mutex::new(());

/// Platform boundary for proxy-state capture and mutation.
///
/// Implementations must capture every field they may write. They should order
/// writes so an incomplete apply fails open; the durable lease is retained when
/// a partial state cannot be proven to be VIA-owned. `restore_snapshot` is
/// intentionally unconditional at the adapter boundary;
/// `ProxyTransaction` invokes it only after exact VIA-ownership verification.
pub trait ProxyAdapter: Send + Sync {
    type Snapshot: Clone
        + Debug
        + PartialEq
        + Eq
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static;

    fn capture(&self) -> NetworkResult<Self::Snapshot>;

    fn build_via_owned_state(
        &self,
        baseline: &Self::Snapshot,
        target: LoopbackProxyTarget,
    ) -> NetworkResult<Self::Snapshot>;

    fn apply_snapshot(&self, via_owned_state: &Self::Snapshot) -> NetworkResult<()>;

    fn restore_snapshot(&self, baseline: &Self::Snapshot) -> NetworkResult<()>;

    /// Build a recovery target that restores each field still equal to VIA's
    /// recorded value while retaining fields changed by another application.
    fn merge_recovery_state(
        &self,
        current: &Self::Snapshot,
        baseline: &Self::Snapshot,
        via_owned_state: &Self::Snapshot,
    ) -> NetworkResult<Self::Snapshot>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryOutcome {
    NoLease,
    Restored,
    AlreadyAtBaseline,
    PreservedExternalChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyReceipt {
    pub session_id: Uuid,
    pub stale_recovery: RecoveryOutcome,
}

/// Serializes all apply/restore/recovery transitions for one process and uses
/// an on-disk dirty lease to extend the transaction across process crashes.
pub struct ProxyTransaction<A, J>
where
    A: ProxyAdapter,
    J: LeaseJournal<A::Snapshot>,
{
    adapter: A,
    journal: J,
}

impl<A, J> ProxyTransaction<A, J>
where
    A: ProxyAdapter,
    J: LeaseJournal<A::Snapshot>,
{
    pub fn new(adapter: A, journal: J) -> Self {
        Self { adapter, journal }
    }

    pub fn adapter(&self) -> &A {
        &self.adapter
    }

    pub fn journal(&self) -> &J {
        &self.journal
    }

    pub fn apply(
        &self,
        target: LoopbackProxyTarget,
        mode: LeaseMode,
    ) -> NetworkResult<ApplyReceipt> {
        let _guard = NETWORK_OPERATION_LOCK
            .lock()
            .map_err(|_| NetworkError::LockPoisoned)?;
        let stale_recovery = self.recover_locked()?;

        // Capture the complete baseline before deriving or writing anything.
        let baseline = self.adapter.capture()?;
        let via_owned_state = self.adapter.build_via_owned_state(&baseline, target)?;
        let session_id = Uuid::new_v4();
        let lease = NetworkLease::new_dirty(
            session_id,
            mode,
            Utc::now(),
            target,
            baseline,
            via_owned_state,
        )?;

        // This syncs and atomically renames the dirty lease before OS mutation.
        self.journal.persist_dirty(&lease)?;

        if let Err(apply_error) = self.adapter.apply_snapshot(&lease.via_owned_state) {
            let cleanup = self.cleanup_failed_apply(&lease);
            return Err(NetworkError::ApplyFailed {
                apply_error: apply_error.to_string(),
                cleanup,
            });
        }

        let current = self.adapter.capture()?;
        if current != lease.via_owned_state {
            // Keep the lease. Startup recovery will preserve a genuinely
            // external value, while a fully VIA-owned value remains recoverable.
            return Err(NetworkError::ApplyVerificationFailed);
        }

        Ok(ApplyReceipt {
            session_id,
            stale_recovery,
        })
    }

    /// Restore a live lease, but only when the exact current state is still the
    /// VIA-owned state recorded in that lease.
    pub fn restore(&self) -> NetworkResult<RecoveryOutcome> {
        let _guard = NETWORK_OPERATION_LOCK
            .lock()
            .map_err(|_| NetworkError::LockPoisoned)?;
        self.recover_locked()
    }

    /// Recover an unclean previous process session. Safe to call repeatedly.
    pub fn recover_stale(&self) -> NetworkResult<RecoveryOutcome> {
        let _guard = NETWORK_OPERATION_LOCK
            .lock()
            .map_err(|_| NetworkError::LockPoisoned)?;
        self.recover_locked()
    }

    fn recover_locked(&self) -> NetworkResult<RecoveryOutcome> {
        let Some(lease) = self.journal.load()? else {
            return Ok(RecoveryOutcome::NoLease);
        };
        let current = self.adapter.capture()?;

        if current == lease.via_owned_state {
            self.adapter.restore_snapshot(&lease.baseline)?;
            if self.adapter.capture()? != lease.baseline {
                // Retain the lease so a later repair attempt can retry.
                return Err(NetworkError::RestoreVerificationFailed);
            }
            self.journal.clear()?;
            return Ok(RecoveryOutcome::Restored);
        }

        if current == lease.baseline {
            self.journal.clear()?;
            return Ok(RecoveryOutcome::AlreadyAtBaseline);
        }

        // Something outside VIA changed at least one tracked proxy field.
        // Restore only fields that are still provably VIA-owned so a partial
        // external edit cannot leave a dead loopback proxy after the core stops.
        let merged =
            self.adapter
                .merge_recovery_state(&current, &lease.baseline, &lease.via_owned_state)?;
        if merged != current {
            self.adapter.restore_snapshot(&merged)?;
            if self.adapter.capture()? != merged {
                return Err(NetworkError::RestoreVerificationFailed);
            }
        }
        self.journal.clear()?;
        Ok(RecoveryOutcome::PreservedExternalChange)
    }

    fn cleanup_failed_apply(&self, lease: &NetworkLease<A::Snapshot>) -> String {
        let current = match self.adapter.capture() {
            Ok(current) => current,
            Err(error) => return format!("lease retained; state capture failed: {error}"),
        };

        if current == lease.baseline {
            return match self.journal.clear() {
                Ok(()) => "OS remained at baseline and dirty lease was removed".to_string(),
                Err(error) => format!("OS remained at baseline but lease removal failed: {error}"),
            };
        }

        if current == lease.via_owned_state {
            if let Err(error) = self.adapter.restore_snapshot(&lease.baseline) {
                return format!("lease retained; conditional rollback failed: {error}");
            }
            match self.adapter.capture() {
                Ok(restored) if restored == lease.baseline => match self.journal.clear() {
                    Ok(()) => {
                        "VIA-owned state was rolled back and dirty lease was removed".to_string()
                    }
                    Err(error) => format!("rollback succeeded but lease removal failed: {error}"),
                },
                Ok(_) => "lease retained; rollback did not reproduce the baseline".to_string(),
                Err(error) => format!("lease retained; rollback verification failed: {error}"),
            }
        } else {
            "lease retained; current state is neither baseline nor fully VIA-owned".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct FakeSnapshot {
        enabled: bool,
        server: Option<String>,
        marker: String,
    }

    impl FakeSnapshot {
        fn baseline() -> Self {
            Self {
                enabled: false,
                server: Some("user-pac.example.invalid".to_string()),
                marker: "baseline".to_string(),
            }
        }

        fn external() -> Self {
            Self {
                enabled: false,
                server: Some("user-change.example.invalid:8080".to_string()),
                marker: "external".to_string(),
            }
        }
    }

    #[derive(Clone)]
    struct FakeAdapter {
        state: Arc<Mutex<FakeSnapshot>>,
        failure: Arc<Mutex<ApplyFailure>>,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ApplyFailure {
        None,
        BeforeMutation,
        AfterMutation,
    }

    impl FakeAdapter {
        fn new(state: FakeSnapshot) -> Self {
            Self {
                state: Arc::new(Mutex::new(state)),
                failure: Arc::new(Mutex::new(ApplyFailure::None)),
            }
        }

        fn state(&self) -> FakeSnapshot {
            self.state.lock().unwrap().clone()
        }

        fn set_state(&self, state: FakeSnapshot) {
            *self.state.lock().unwrap() = state;
        }

        fn fail_next(&self, failure: ApplyFailure) {
            *self.failure.lock().unwrap() = failure;
        }
    }

    impl ProxyAdapter for FakeAdapter {
        type Snapshot = FakeSnapshot;

        fn capture(&self) -> NetworkResult<Self::Snapshot> {
            Ok(self.state())
        }

        fn build_via_owned_state(
            &self,
            _baseline: &Self::Snapshot,
            target: LoopbackProxyTarget,
        ) -> NetworkResult<Self::Snapshot> {
            Ok(FakeSnapshot {
                enabled: true,
                server: Some(target.proxy_server()),
                marker: "via".to_string(),
            })
        }

        fn apply_snapshot(&self, via_owned_state: &Self::Snapshot) -> NetworkResult<()> {
            let failure = *self.failure.lock().unwrap();
            *self.failure.lock().unwrap() = ApplyFailure::None;
            if failure == ApplyFailure::BeforeMutation {
                return Err(NetworkError::adapter("fake apply", "injected failure"));
            }
            self.set_state(via_owned_state.clone());
            if failure == ApplyFailure::AfterMutation {
                return Err(NetworkError::adapter(
                    "fake apply",
                    "injected post-mutation failure",
                ));
            }
            Ok(())
        }

        fn restore_snapshot(&self, baseline: &Self::Snapshot) -> NetworkResult<()> {
            self.set_state(baseline.clone());
            Ok(())
        }

        fn merge_recovery_state(
            &self,
            current: &Self::Snapshot,
            baseline: &Self::Snapshot,
            via_owned_state: &Self::Snapshot,
        ) -> NetworkResult<Self::Snapshot> {
            Ok(FakeSnapshot {
                enabled: if current.enabled == via_owned_state.enabled {
                    baseline.enabled
                } else {
                    current.enabled
                },
                server: if current.server == via_owned_state.server {
                    baseline.server.clone()
                } else {
                    current.server.clone()
                },
                marker: if current.marker == via_owned_state.marker {
                    baseline.marker.clone()
                } else {
                    current.marker.clone()
                },
            })
        }
    }

    #[derive(Clone)]
    struct MemoryJournal<S> {
        lease: Arc<Mutex<Option<NetworkLease<S>>>>,
    }

    impl<S> Default for MemoryJournal<S> {
        fn default() -> Self {
            Self {
                lease: Arc::new(Mutex::new(None)),
            }
        }
    }

    impl<S> MemoryJournal<S> {
        fn has_lease(&self) -> bool {
            self.lease.lock().unwrap().is_some()
        }
    }

    impl<S> LeaseJournal<S> for MemoryJournal<S>
    where
        S: Clone + Serialize + Send + Sync,
    {
        fn load(&self) -> NetworkResult<Option<NetworkLease<S>>> {
            let lease = self.lease.lock().unwrap().clone();
            if let Some(lease) = &lease {
                lease.verify()?;
            }
            Ok(lease)
        }

        fn persist_dirty(&self, lease: &NetworkLease<S>) -> NetworkResult<()> {
            lease.verify()?;
            *self.lease.lock().unwrap() = Some(lease.clone());
            Ok(())
        }

        fn clear(&self) -> NetworkResult<()> {
            *self.lease.lock().unwrap() = None;
            Ok(())
        }
    }

    fn target() -> LoopbackProxyTarget {
        LoopbackProxyTarget::new(17890).unwrap()
    }

    #[test]
    fn apply_failure_before_mutation_removes_dirty_lease() {
        let adapter = FakeAdapter::new(FakeSnapshot::baseline());
        adapter.fail_next(ApplyFailure::BeforeMutation);
        let journal = MemoryJournal::default();
        let transaction = ProxyTransaction::new(adapter.clone(), journal.clone());

        let error = transaction
            .apply(target(), LeaseMode::SystemProxy)
            .unwrap_err();

        assert!(matches!(error, NetworkError::ApplyFailed { .. }));
        assert_eq!(adapter.state(), FakeSnapshot::baseline());
        assert!(!journal.has_lease());
    }

    #[test]
    fn apply_failure_after_owned_mutation_rolls_back_conditionally() {
        let adapter = FakeAdapter::new(FakeSnapshot::baseline());
        adapter.fail_next(ApplyFailure::AfterMutation);
        let journal = MemoryJournal::default();
        let transaction = ProxyTransaction::new(adapter.clone(), journal.clone());

        let error = transaction
            .apply(target(), LeaseMode::SystemProxy)
            .unwrap_err();

        assert!(matches!(error, NetworkError::ApplyFailed { .. }));
        assert_eq!(adapter.state(), FakeSnapshot::baseline());
        assert!(!journal.has_lease());
    }

    #[test]
    fn restore_only_replaces_the_exact_via_owned_state() {
        let baseline = FakeSnapshot::baseline();
        let adapter = FakeAdapter::new(baseline.clone());
        let journal = MemoryJournal::default();
        let transaction = ProxyTransaction::new(adapter.clone(), journal.clone());
        transaction.apply(target(), LeaseMode::SystemProxy).unwrap();

        assert_eq!(transaction.restore().unwrap(), RecoveryOutcome::Restored);
        assert_eq!(adapter.state(), baseline);
        assert!(!journal.has_lease());
    }

    #[test]
    fn restore_preserves_user_changes_made_while_via_was_running() {
        let adapter = FakeAdapter::new(FakeSnapshot::baseline());
        let journal = MemoryJournal::default();
        let transaction = ProxyTransaction::new(adapter.clone(), journal.clone());
        transaction.apply(target(), LeaseMode::SystemProxy).unwrap();
        adapter.set_state(FakeSnapshot::external());

        assert_eq!(
            transaction.restore().unwrap(),
            RecoveryOutcome::PreservedExternalChange
        );
        assert_eq!(adapter.state(), FakeSnapshot::external());
        assert!(!journal.has_lease());
    }

    #[test]
    fn partial_external_change_restores_remaining_via_owned_fields() {
        let baseline = FakeSnapshot::baseline();
        let adapter = FakeAdapter::new(baseline.clone());
        let journal = MemoryJournal::default();
        let transaction = ProxyTransaction::new(adapter.clone(), journal.clone());
        transaction.apply(target(), LeaseMode::SystemProxy).unwrap();
        adapter.set_state(FakeSnapshot {
            enabled: true,
            server: Some("external.example.invalid:8080".to_string()),
            marker: "via".to_string(),
        });

        assert_eq!(
            transaction.restore().unwrap(),
            RecoveryOutcome::PreservedExternalChange
        );
        assert_eq!(
            adapter.state(),
            FakeSnapshot {
                enabled: baseline.enabled,
                server: Some("external.example.invalid:8080".to_string()),
                marker: baseline.marker,
            }
        );
        assert!(!journal.has_lease());
    }

    #[test]
    fn a_stale_owned_lease_is_recovered_after_restart() {
        let baseline = FakeSnapshot::baseline();
        let adapter = FakeAdapter::new(baseline.clone());
        let journal = MemoryJournal::default();
        ProxyTransaction::new(adapter.clone(), journal.clone())
            .apply(target(), LeaseMode::SystemProxy)
            .unwrap();

        let restarted = ProxyTransaction::new(adapter.clone(), journal.clone());
        assert_eq!(
            restarted.recover_stale().unwrap(),
            RecoveryOutcome::Restored
        );
        assert_eq!(adapter.state(), baseline);
        assert!(!journal.has_lease());
    }

    #[test]
    fn checksum_corruption_blocks_os_mutation_and_retains_evidence() {
        let adapter = FakeAdapter::new(FakeSnapshot::baseline());
        let journal = MemoryJournal::default();
        ProxyTransaction::new(adapter.clone(), journal.clone())
            .apply(target(), LeaseMode::SystemProxy)
            .unwrap();
        let state_before_recovery = adapter.state();
        journal.lease.lock().unwrap().as_mut().unwrap().checksum = "corrupt".to_string();

        let restarted = ProxyTransaction::new(adapter.clone(), journal.clone());
        let error = restarted.recover_stale().unwrap_err();

        assert!(matches!(error, NetworkError::ChecksumMismatch { .. }));
        assert_eq!(adapter.state(), state_before_recovery);
        assert!(journal.has_lease());
    }

    #[test]
    fn stale_recovery_is_idempotent() {
        let baseline = FakeSnapshot::baseline();
        let adapter = FakeAdapter::new(baseline.clone());
        let journal = MemoryJournal::default();
        ProxyTransaction::new(adapter.clone(), journal.clone())
            .apply(target(), LeaseMode::SystemProxy)
            .unwrap();

        let restarted = ProxyTransaction::new(adapter.clone(), journal);
        assert_eq!(
            restarted.recover_stale().unwrap(),
            RecoveryOutcome::Restored
        );
        assert_eq!(restarted.recover_stale().unwrap(), RecoveryOutcome::NoLease);
        assert_eq!(adapter.state(), baseline);
    }

    #[test]
    fn lease_timestamp_and_mode_are_integrity_protected() {
        let baseline = FakeSnapshot::baseline();
        let owned = FakeSnapshot {
            enabled: true,
            server: Some(target().proxy_server()),
            marker: "via".to_string(),
        };
        let mut lease = NetworkLease::new_dirty(
            Uuid::new_v4(),
            LeaseMode::SystemProxy,
            Utc::now(),
            target(),
            baseline,
            owned,
        )
        .unwrap();
        lease.mode = LeaseMode::Tun;

        assert!(matches!(
            lease.verify().unwrap_err(),
            NetworkError::ChecksumMismatch { .. }
        ));
    }
}
