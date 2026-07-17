use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{NetworkError, NetworkResult};

pub const NETWORK_LEASE_VERSION: u32 = 1;
pub const LOOPBACK_PROXY_HOST: &str = "127.0.0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseMode {
    SystemProxy,
    Tun,
}

/// A mixed HTTP/SOCKS listener that is guaranteed to stay on loopback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopbackProxyTarget {
    mixed_port: u16,
}

impl LoopbackProxyTarget {
    pub fn new(mixed_port: u16) -> NetworkResult<Self> {
        if mixed_port == 0 {
            return Err(NetworkError::InvalidTarget(
                "port zero cannot identify a listening socket".to_string(),
            ));
        }
        Ok(Self { mixed_port })
    }

    pub fn host(&self) -> &'static str {
        LOOPBACK_PROXY_HOST
    }

    pub fn mixed_port(&self) -> u16 {
        self.mixed_port
    }

    pub fn proxy_server(&self) -> String {
        format!("{}:{}", self.host(), self.mixed_port)
    }

    pub(crate) fn validate(&self) -> NetworkResult<()> {
        if self.mixed_port == 0 {
            return Err(NetworkError::InvalidTarget(
                "lease contains port zero".to_string(),
            ));
        }
        Ok(())
    }
}

/// The crash-recovery record persisted before any operating-system mutation.
///
/// `baseline` contains every platform field the adapter may mutate.
/// `via_owned_state` is the exact state that VIA is allowed to restore from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkLease<S> {
    pub version: u32,
    pub dirty: bool,
    pub session_id: Uuid,
    pub mode: LeaseMode,
    pub created_at: DateTime<Utc>,
    pub target: LoopbackProxyTarget,
    pub baseline: S,
    pub via_owned_state: S,
    pub checksum: String,
}

#[derive(Serialize)]
struct LeaseIntegrityView<'a, S> {
    version: u32,
    dirty: bool,
    session_id: Uuid,
    mode: LeaseMode,
    created_at: DateTime<Utc>,
    target: LoopbackProxyTarget,
    baseline: &'a S,
    via_owned_state: &'a S,
}

impl<S: Serialize> NetworkLease<S> {
    pub fn new_dirty(
        session_id: Uuid,
        mode: LeaseMode,
        created_at: DateTime<Utc>,
        target: LoopbackProxyTarget,
        baseline: S,
        via_owned_state: S,
    ) -> NetworkResult<Self> {
        target.validate()?;
        let mut lease = Self {
            version: NETWORK_LEASE_VERSION,
            dirty: true,
            session_id,
            mode,
            created_at,
            target,
            baseline,
            via_owned_state,
            checksum: String::new(),
        };
        lease.checksum = lease.calculate_checksum()?;
        Ok(lease)
    }

    pub fn verify(&self) -> NetworkResult<()> {
        if self.version != NETWORK_LEASE_VERSION {
            return Err(NetworkError::UnsupportedLeaseVersion {
                found: self.version,
                supported: NETWORK_LEASE_VERSION,
            });
        }
        if !self.dirty {
            return Err(NetworkError::CleanLease);
        }
        self.target.validate()?;

        let expected = self.calculate_checksum()?;
        if expected != self.checksum {
            return Err(NetworkError::ChecksumMismatch {
                expected,
                actual: self.checksum.clone(),
            });
        }
        Ok(())
    }

    fn calculate_checksum(&self) -> NetworkResult<String> {
        let payload = LeaseIntegrityView {
            version: self.version,
            dirty: self.dirty,
            session_id: self.session_id,
            mode: self.mode,
            created_at: self.created_at,
            target: self.target,
            baseline: &self.baseline,
            via_owned_state: &self.via_owned_state,
        };
        let encoded = serde_json::to_vec(&payload).map_err(|source| {
            NetworkError::adapter("calculate lease checksum", source.to_string())
        })?;
        Ok(hex::encode(Sha256::digest(encoded)))
    }
}
