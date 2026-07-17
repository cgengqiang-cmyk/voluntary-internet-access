use std::{io, path::PathBuf};

pub type NetworkResult<T> = Result<T, NetworkError>;

#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("invalid loopback proxy target: {0}")]
    InvalidTarget(String),

    #[error("network proxy operation is unsupported ({operation}): {reason}")]
    Unsupported {
        operation: &'static str,
        reason: String,
    },

    #[error(
        "cannot safely replace authenticated {proxy_kind} proxy on network service {service}; the password is not available for exact restoration"
    )]
    AuthenticatedProxyBaseline {
        service: String,
        proxy_kind: &'static str,
    },

    #[error("system proxy adapter failed during {operation}: {reason}")]
    Adapter {
        operation: &'static str,
        reason: String,
    },

    #[error("I/O failed during {operation} at {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("network lease at {path} is not valid JSON: {source}")]
    JournalDecode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("network lease checksum mismatch (expected {expected}, found {actual})")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("unsupported network lease version {found}; this build supports {supported}")]
    UnsupportedLeaseVersion { found: u32, supported: u32 },

    #[error("network lease is not marked dirty")]
    CleanLease,

    #[error("network operation lock is poisoned")]
    LockPoisoned,

    #[error("proxy apply failed: {apply_error}; cleanup result: {cleanup}")]
    ApplyFailed {
        apply_error: String,
        cleanup: String,
    },

    #[error("system proxy state did not match the VIA-owned state after apply")]
    ApplyVerificationFailed,

    #[error("system proxy state did not return to the recorded baseline")]
    RestoreVerificationFailed,
}

impl NetworkError {
    pub(crate) fn io(operation: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }

    pub(crate) fn adapter(operation: &'static str, reason: impl Into<String>) -> Self {
        Self::Adapter {
            operation,
            reason: reason.into(),
        }
    }
}
