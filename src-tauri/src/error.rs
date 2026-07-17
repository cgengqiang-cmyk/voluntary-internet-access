use std::io;

#[derive(Debug, thiserror::Error)]
pub enum ViaError {
    #[error("输入无效：{0}")]
    InvalidInput(String),
    #[error("配置无效：{0}")]
    InvalidConfig(String),
    #[error("网络操作失败：{0}")]
    Network(String),
    #[error("Mihomo 内核错误：{0}")]
    Core(String),
    #[error("系统凭据库错误：{0}")]
    Credential(String),
    #[error("文件操作失败：{0}")]
    Io(#[from] io::Error),
    #[error("序列化失败：{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

pub type ViaResult<T> = Result<T, ViaError>;

pub fn command_error(error: impl std::fmt::Display) -> String {
    redact_sensitive(&error.to_string())
}

/// Remove URL-bearing and credential-like tokens before anything reaches the
/// WebView, diagnostics export, or the app-owned log. The original error is
/// intentionally not retained anywhere by VIA.
pub fn redact_sensitive(input: &str) -> String {
    let mut output = String::new();
    for raw_line in input.lines() {
        if output.len() >= 1_200 {
            break;
        }
        let lower = raw_line.to_ascii_lowercase();
        if [
            "authorization",
            "proxy-authorization",
            "password",
            "passwd",
            "api-key",
            "apikey",
            "access_token",
            "refresh_token",
            "auth-key",
            "auth_key",
            "client-secret",
            "client_secret",
            "uuid",
            "psk",
            "secret:",
            "bearer ",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
        {
            output.push_str("[敏感详情已隐藏]\n");
            continue;
        }

        for token in raw_line.split_inclusive(char::is_whitespace) {
            let trimmed = token.trim();
            let unsafe_url = trimmed.contains("://")
                || (trimmed.contains('@') && trimmed.contains(':'))
                || trimmed.to_ascii_lowercase().contains("token=")
                || trimmed.to_ascii_lowercase().contains("secret=");
            if unsafe_url {
                output.push_str("[URL 已隐藏]");
                if token.ends_with(char::is_whitespace) {
                    output.push(' ');
                }
            } else {
                output.extend(token.chars().filter(|character| {
                    !character.is_control() || matches!(character, '\n' | '\t')
                }));
            }
        }
        if !raw_line.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
    }
    let mut output = output.trim().chars().take(1_200).collect::<String>();
    if output.is_empty() {
        output = "操作失败，敏感详情已隐藏".to_string();
    }
    output
}

#[cfg(test)]
mod tests {
    use super::redact_sensitive;

    #[test]
    fn redacts_urls_and_credential_fields() {
        let redacted = redact_sensitive(
            "fetch https://user:pass@example.test/sub?token=abc failed\npassword: hunter2",
        );
        assert!(!redacted.contains("example.test"));
        assert!(!redacted.contains("hunter2"));
        assert!(redacted.contains("URL 已隐藏"));
    }

    #[test]
    fn redacts_common_proxy_credential_keys() {
        for secret in [
            "uuid: 11111111-1111-1111-1111-111111111111",
            "auth-key: tskey-secret",
            "psk: hidden",
            "client-secret: hidden",
        ] {
            assert_eq!(redact_sensitive(secret), "[敏感详情已隐藏]");
        }
    }
}
