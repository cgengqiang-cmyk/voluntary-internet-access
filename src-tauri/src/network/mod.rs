//! Durable ownership and recovery for operating-system proxy settings.
//!
//! The transaction layer is intentionally independent from Mihomo. A caller
//! can restore a dirty lease during early application startup, before the
//! controller or the proxy core is available.

mod error;
mod journal;
mod lease;
mod native;
mod transaction;

// Also compile the macOS command/parser skeleton in host-side unit tests. It
// performs no mutation in tests, but this catches cfg drift before a Mac build.
#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod macos;
#[cfg(windows)]
mod windows;

pub use error::{NetworkError, NetworkResult};
pub use journal::{FileLeaseJournal, LeaseJournal};
pub use lease::{LeaseMode, LoopbackProxyTarget, NETWORK_LEASE_VERSION, NetworkLease};
#[cfg(target_os = "macos")]
pub use macos::{
    MacAutoProxyState, MacNetworkServiceProxy, MacOsProxyAdapter, MacOsProxySnapshot,
    MacProxyEndpoint,
};
pub use native::{apply_native_proxy, recover_native_proxy, restore_native_proxy};
pub use transaction::{ApplyReceipt, ProxyAdapter, ProxyTransaction, RecoveryOutcome};
#[cfg(windows)]
pub use windows::{
    RegistryValueKind, RegistryValueSnapshot, WindowsProxyAdapter, WindowsProxySnapshot,
};

#[cfg(windows)]
pub type NativeProxyAdapter = WindowsProxyAdapter;
#[cfg(target_os = "macos")]
pub type NativeProxyAdapter = MacOsProxyAdapter;
