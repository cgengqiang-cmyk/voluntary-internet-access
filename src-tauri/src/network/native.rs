use std::path::PathBuf;

use super::{ApplyReceipt, NetworkResult, RecoveryOutcome};

#[cfg(not(any(windows, target_os = "macos")))]
use super::NetworkError;

#[cfg(any(windows, target_os = "macos"))]
use super::{
    FileLeaseJournal, LeaseMode, LoopbackProxyTarget, NativeProxyAdapter, ProxyAdapter,
    ProxyTransaction,
};

#[cfg(any(windows, target_os = "macos"))]
type NativeJournal = FileLeaseJournal<<NativeProxyAdapter as ProxyAdapter>::Snapshot>;

#[cfg(any(windows, target_os = "macos"))]
fn transaction(lease_path: PathBuf) -> ProxyTransaction<NativeProxyAdapter, NativeJournal> {
    ProxyTransaction::new(NativeProxyAdapter::new(), FileLeaseJournal::new(lease_path))
}

/// Apply VIA's loopback mixed proxy and durably record ownership first.
///
/// This blocking function is intended to be called from `spawn_blocking` by an
/// async command handler. A stale previous lease is recovered before the new
/// baseline is captured.
#[cfg(any(windows, target_os = "macos"))]
pub fn apply_native_proxy(
    lease_path: impl Into<PathBuf>,
    mixed_port: u16,
) -> NetworkResult<ApplyReceipt> {
    let target = LoopbackProxyTarget::new(mixed_port)?;
    transaction(lease_path.into()).apply(target, LeaseMode::SystemProxy)
}

/// Conditionally restore a live VIA-owned proxy state.
#[cfg(any(windows, target_os = "macos"))]
pub fn restore_native_proxy(lease_path: impl Into<PathBuf>) -> NetworkResult<RecoveryOutcome> {
    transaction(lease_path.into()).restore()
}

/// Recover a dirty lease without requiring Mihomo or its controller.
#[cfg(any(windows, target_os = "macos"))]
pub fn recover_native_proxy(lease_path: impl Into<PathBuf>) -> NetworkResult<RecoveryOutcome> {
    transaction(lease_path.into()).recover_stale()
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn apply_native_proxy(
    lease_path: impl Into<PathBuf>,
    _mixed_port: u16,
) -> NetworkResult<ApplyReceipt> {
    let _ = lease_path.into();
    Err(unsupported("apply native system proxy"))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn restore_native_proxy(lease_path: impl Into<PathBuf>) -> NetworkResult<RecoveryOutcome> {
    let _ = lease_path.into();
    Err(unsupported("restore native system proxy"))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn recover_native_proxy(lease_path: impl Into<PathBuf>) -> NetworkResult<RecoveryOutcome> {
    let _ = lease_path.into();
    Err(unsupported("recover native system proxy"))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn unsupported(operation: &'static str) -> NetworkError {
    NetworkError::Unsupported {
        operation,
        reason: "VIA system-proxy ownership is implemented only for Windows and macOS".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_native_journal_is_an_idempotent_no_op() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("network-lease.json");

        assert_eq!(
            recover_native_proxy(path.clone()).unwrap(),
            RecoveryOutcome::NoLease
        );
        assert_eq!(
            restore_native_proxy(path).unwrap(),
            RecoveryOutcome::NoLease
        );
    }
}
