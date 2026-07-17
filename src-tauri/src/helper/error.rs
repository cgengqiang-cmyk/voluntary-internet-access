use std::{io, path::PathBuf};

use thiserror::Error;

pub type HelperResult<T> = Result<T, HelperError>;

#[derive(Debug, Error)]
pub enum HelperError {
    #[error("helper I/O failed during {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("helper JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("helper request is invalid: {0}")]
    InvalidRequest(String),
    #[error("helper authentication failed")]
    Unauthorized,
    #[error("unsupported helper protocol version {found}; expected {expected}")]
    UnsupportedVersion { found: u32, expected: u32 },
    #[error("privileged installation is incomplete: {0}")]
    NotInstalled(&'static str),
    #[error("fixed privileged path failed validation: {0}")]
    UnsafeFixedPath(PathBuf),
    #[error("privileged core digest does not match the pinned release")]
    CoreDigestMismatch,
    #[error("TUN configuration failed validation: {0}")]
    UnsafeConfiguration(String),
    #[error("TUN session conflict: {0}")]
    SessionConflict(String),
    #[error("TUN core failed: {0}")]
    Core(String),
    #[error("helper state lock was poisoned")]
    LockPoisoned,
    #[error("helper protocol frame is too large")]
    FrameTooLarge,
    #[error("helper response did not match its request")]
    MismatchedResponse,
}

impl HelperError {
    pub(crate) fn io(operation: &'static str, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Io { .. } => "io_error",
            Self::Json(_) => "invalid_json",
            Self::InvalidRequest(_) => "invalid_request",
            Self::Unauthorized => "unauthorized",
            Self::UnsupportedVersion { .. } => "unsupported_version",
            Self::NotInstalled(_) => "not_installed",
            Self::UnsafeFixedPath(_) => "unsafe_fixed_path",
            Self::CoreDigestMismatch => "core_digest_mismatch",
            Self::UnsafeConfiguration(_) => "unsafe_configuration",
            Self::SessionConflict(_) => "session_conflict",
            Self::Core(_) => "core_error",
            Self::LockPoisoned => "lock_poisoned",
            Self::FrameTooLarge => "frame_too_large",
            Self::MismatchedResponse => "mismatched_response",
        }
    }
}
