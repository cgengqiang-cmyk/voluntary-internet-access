use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{HelperError, HelperResult};

pub const HELPER_PROTOCOL_VERSION: u32 = 1;
pub const MIN_HEARTBEAT_TIMEOUT_SECONDS: u16 = 5;
pub const MAX_HEARTBEAT_TIMEOUT_SECONDS: u16 = 60;
pub const AUTH_TOKEN_HEX_LENGTH: usize = 64;
pub const SHA256_HEX_LENGTH: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelperRequest {
    pub version: u32,
    pub request_id: Uuid,
    pub auth_token: String,
    #[serde(flatten)]
    pub operation: HelperOperation,
}

impl HelperRequest {
    pub fn new(auth_token: String, operation: HelperOperation) -> Self {
        Self {
            version: HELPER_PROTOCOL_VERSION,
            request_id: Uuid::new_v4(),
            auth_token,
            operation,
        }
    }

    pub fn validate_shape(&self) -> HelperResult<()> {
        if self.version != HELPER_PROTOCOL_VERSION {
            return Err(HelperError::UnsupportedVersion {
                found: self.version,
                expected: HELPER_PROTOCOL_VERSION,
            });
        }
        if !is_lower_hex(&self.auth_token, AUTH_TOKEN_HEX_LENGTH) {
            return Err(HelperError::InvalidRequest(
                "auth token must be 32 lowercase hexadecimal bytes".to_string(),
            ));
        }
        self.operation.validate()
    }
}

/// Strictly allowlisted privileged operations.
///
/// `install` stages already-sanitized YAML into the helper's fixed runtime
/// location. It does not install an arbitrary service or copy an arbitrary
/// executable. The platform installer scripts own that one-time operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum HelperOperation {
    Status,
    Install {
        config_yaml: String,
        config_sha256: String,
    },
    Remove {
        confirm: bool,
    },
    StartTun {
        session_id: Uuid,
        config_sha256: String,
        controller_secret: String,
        heartbeat_timeout_seconds: u16,
    },
    Heartbeat {
        session_id: Uuid,
    },
    StopTun {
        session_id: Uuid,
    },
    Restore {
        session_id: Option<Uuid>,
    },
}

impl HelperOperation {
    fn validate(&self) -> HelperResult<()> {
        match self {
            Self::Status | Self::Heartbeat { .. } | Self::StopTun { .. } | Self::Restore { .. } => {
                Ok(())
            }
            Self::Install {
                config_yaml,
                config_sha256,
            } => {
                if config_yaml.is_empty() {
                    return Err(HelperError::InvalidRequest(
                        "install requires non-empty configuration contents".to_string(),
                    ));
                }
                validate_sha256(config_sha256)
            }
            Self::Remove { confirm } => {
                if !confirm {
                    return Err(HelperError::InvalidRequest(
                        "remove requires explicit confirmation".to_string(),
                    ));
                }
                Ok(())
            }
            Self::StartTun {
                config_sha256,
                controller_secret,
                heartbeat_timeout_seconds,
                ..
            } => {
                validate_sha256(config_sha256)?;
                if !(32..=128).contains(&controller_secret.len())
                    || !controller_secret
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(HelperError::InvalidRequest(
                        "controller secret must be 32-128 hexadecimal characters".to_string(),
                    ));
                }
                if !(MIN_HEARTBEAT_TIMEOUT_SECONDS..=MAX_HEARTBEAT_TIMEOUT_SECONDS)
                    .contains(heartbeat_timeout_seconds)
                {
                    return Err(HelperError::InvalidRequest(format!(
                        "heartbeat timeout must be {MIN_HEARTBEAT_TIMEOUT_SECONDS}-{MAX_HEARTBEAT_TIMEOUT_SECONDS} seconds"
                    )));
                }
                Ok(())
            }
        }
    }
}

fn validate_sha256(value: &str) -> HelperResult<()> {
    if !is_lower_hex(value, SHA256_HEX_LENGTH) {
        return Err(HelperError::InvalidRequest(
            "SHA-256 must be 64 lowercase hexadecimal characters".to_string(),
        ));
    }
    Ok(())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelperResponse {
    pub version: u32,
    pub request_id: Uuid,
    pub ok: bool,
    #[serde(flatten)]
    pub body: HelperResponseBody,
}

impl HelperResponse {
    pub(crate) fn success(request_id: Uuid, body: HelperResponseBody) -> Self {
        Self {
            version: HELPER_PROTOCOL_VERSION,
            request_id,
            ok: true,
            body,
        }
    }

    pub(crate) fn error(request_id: Uuid, error: &HelperError) -> Self {
        Self {
            version: HELPER_PROTOCOL_VERSION,
            request_id,
            ok: false,
            body: HelperResponseBody::Error {
                code: error.code().to_string(),
                message: error.to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum HelperResponseBody {
    Status { status: HelperStatus },
    Installed { config_sha256: String },
    Removed,
    Started { pid: u32 },
    HeartbeatAccepted,
    Stopped,
    Restored,
    Error { code: String, message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperStatus {
    pub protocol_version: u32,
    pub core_installed: bool,
    pub config_installed: bool,
    pub running: bool,
    pub session_id: Option<Uuid>,
    pub pid: Option<u32>,
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token() -> String {
        "a".repeat(AUTH_TOKEN_HEX_LENGTH)
    }

    #[test]
    fn rejects_unknown_path_fields_before_dispatch() {
        let request = format!(
            r#"{{"version":1,"request_id":"{}","auth_token":"{}","operation":"start_tun","session_id":"{}","config_sha256":"{}","controller_secret":"{}","heartbeat_timeout_seconds":15,"executable_path":"C:\\\\attacker.exe"}}"#,
            Uuid::new_v4(),
            token(),
            Uuid::new_v4(),
            "b".repeat(64),
            "c".repeat(32),
        );

        assert!(serde_json::from_str::<HelperRequest>(&request).is_err());
    }

    #[test]
    fn validates_timeout_and_lowercase_digests() {
        let request = HelperRequest::new(
            token(),
            HelperOperation::StartTun {
                session_id: Uuid::new_v4(),
                config_sha256: "A".repeat(64),
                controller_secret: "c".repeat(32),
                heartbeat_timeout_seconds: 4,
            },
        );
        assert!(request.validate_shape().is_err());
    }

    #[test]
    fn remove_requires_confirmation() {
        let request = HelperRequest::new(token(), HelperOperation::Remove { confirm: false });
        assert!(request.validate_shape().is_err());
    }
}
