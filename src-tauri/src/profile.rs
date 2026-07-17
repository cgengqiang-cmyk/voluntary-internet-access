use std::{
    path::{Component, Path},
    process::Stdio,
    time::Duration,
};

use chrono::Utc;
use sha2::{Digest, Sha256};
use tokio::process::Command;
use url::Url;

use crate::{
    config::{
        DnsMode as ConfigDnsMode, MAX_PROFILE_BYTES, SanitizeOptions, Transport, sanitize_profile,
    },
    core::{core_binary_path, find_free_loopback_port, random_controller_secret},
    error::{ViaError, ViaResult},
    fetch::fetch_https,
    model::{DnsMode, ProfileSourceKind, ProfileSummary, TransportMode},
    state::{AppState, atomic_write_bytes},
};

const MAX_PROVIDER_BYTES: usize = 8 * 1024 * 1024;
const VALIDATION_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_PRIVILEGED_CONFIG_BYTES: usize = 2 * 1024 * 1024;

pub struct PreparedTunProfile {
    pub yaml: String,
    pub sha256: String,
    pub mixed_port: u16,
    pub controller_port: u16,
    pub controller_secret: String,
}

pub async fn import_subscription(state: &AppState, raw_url: &str) -> ViaResult<ProfileSummary> {
    let url = Url::parse(raw_url)
        .map_err(|error| ViaError::InvalidInput(format!("订阅 URL 无效：{error}")))?;
    let source_label = masked_source_label(&url);
    let fetched = fetch_https(&url, MAX_PROFILE_BYTES).await?;
    let profile = import_profile_bytes(
        state,
        &source_label,
        ProfileSourceKind::Subscription,
        &fetched.bytes,
    )
    .await?;
    state.credentials.set_subscription_url(raw_url).await?;
    Ok(profile)
}

pub async fn import_local_profile(
    state: &AppState,
    file_name: &str,
    contents: &[u8],
) -> ViaResult<ProfileSummary> {
    let safe_name = Path::new(file_name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("本地 YAML")
        .chars()
        .filter(|character| !character.is_control())
        .take(120)
        .collect::<String>();
    let profile =
        import_profile_bytes(state, &safe_name, ProfileSourceKind::File, contents).await?;
    state.credentials.clear_subscription_url().await?;
    Ok(profile)
}

pub async fn refresh_subscription(state: &AppState) -> ViaResult<ProfileSummary> {
    let url = state
        .credentials
        .subscription_url()
        .await?
        .ok_or_else(|| ViaError::InvalidInput("当前配置不是订阅 URL".to_string()))?;
    import_subscription(state, &url).await
}

/// Rebuild the only profile Mihomo is allowed to execute from the retained
/// untrusted source. Ports are allocated immediately before launch and all
/// app-owned security fields are regenerated on every connection.
pub async fn prepare_runtime_profile(state: &AppState) -> ViaResult<()> {
    let _operation = state.operation_lock.lock().await;
    let source = std::fs::read(state.paths.source_profile_file()).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ViaError::InvalidConfig("配置源文件缺失，请重新导入".to_string())
        } else {
            error.into()
        }
    })?;
    let sanitized = sanitize_for_current_settings(state, &source).await?;
    fetch_profile_providers(state, &sanitized, false).await?;
    atomic_write_bytes(
        state.paths.runtime_profile_file(),
        sanitized.effective_yaml.as_bytes(),
    )?;
    validate_effective_profile(state, &state.paths.runtime_profile_file()).await?;
    atomic_write_bytes(
        state.paths.effective_profile_file(),
        sanitized.effective_yaml.as_bytes(),
    )?;
    Ok(())
}

/// Convert helper-inaccessible file providers into validated inline payloads,
/// then pass the complete document through the same sanitizer a second time.
pub async fn prepare_privileged_tun_profile(state: &AppState) -> ViaResult<PreparedTunProfile> {
    let bytes = std::fs::read(state.paths.runtime_profile_file())?;
    let mut document: serde_yaml::Value = serde_yaml::from_slice(&bytes)
        .map_err(|error| ViaError::InvalidConfig(error.to_string()))?;
    let root = document
        .as_mapping_mut()
        .ok_or_else(|| ViaError::InvalidConfig("TUN 配置根节点无效".to_string()))?;
    let mixed_port = yaml_port(root, "mixed-port")?;
    let controller_port = root
        .get(serde_yaml::Value::String("external-controller".to_string()))
        .and_then(serde_yaml::Value::as_str)
        .and_then(|value| value.rsplit(':').next())
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port > 0)
        .ok_or_else(|| ViaError::InvalidConfig("TUN 控制端口无效".to_string()))?;

    inline_file_providers(state, root, "proxy-providers", true)?;
    inline_file_providers(state, root, "rule-providers", false)?;

    let candidate = serde_yaml::to_string(&document)
        .map_err(|error| ViaError::InvalidConfig(error.to_string()))?;
    let controller_secret = random_controller_secret();
    let dns_mode = match state.snapshot.read().await.settings.dns_mode {
        DnsMode::FakeIp => ConfigDnsMode::FakeIp,
        DnsMode::RedirHost => ConfigDnsMode::RedirHost,
    };
    let sanitized = sanitize_profile(
        candidate.as_bytes(),
        &SanitizeOptions {
            mixed_port,
            controller_port,
            controller_secret: controller_secret.clone(),
            transport: Transport::Tun,
            dns_mode,
        },
    )
    .map_err(|error| ViaError::InvalidConfig(error.to_string()))?;
    if !sanitized.provider_downloads.is_empty() {
        return Err(ViaError::InvalidConfig(
            "高权限 TUN 配置仍包含外部 Provider".to_string(),
        ));
    }
    if sanitized.effective_yaml.len() > MAX_PRIVILEGED_CONFIG_BYTES {
        return Err(ViaError::InvalidConfig(
            "内联 Provider 后的 TUN 配置超过 2 MiB".to_string(),
        ));
    }
    let sha256 = hex::encode(Sha256::digest(sanitized.effective_yaml.as_bytes()));
    Ok(PreparedTunProfile {
        yaml: sanitized.effective_yaml,
        sha256,
        mixed_port,
        controller_port,
        controller_secret,
    })
}

fn inline_file_providers(
    state: &AppState,
    root: &mut serde_yaml::Mapping,
    section: &str,
    proxy_provider: bool,
) -> ViaResult<()> {
    let Some(providers) = root
        .get_mut(serde_yaml::Value::String(section.to_string()))
        .and_then(serde_yaml::Value::as_mapping_mut)
    else {
        return Ok(());
    };
    for definition in providers.values_mut() {
        let mapping = definition
            .as_mapping_mut()
            .ok_or_else(|| ViaError::InvalidConfig(format!("{section} 条目必须是映射")))?;
        let provider_type = mapping
            .get(serde_yaml::Value::String("type".to_string()))
            .and_then(serde_yaml::Value::as_str)
            .unwrap_or_default();
        if provider_type == "inline" {
            continue;
        }
        if provider_type != "file" {
            return Err(ViaError::InvalidConfig(format!(
                "{section} 包含不受支持的 Provider 类型"
            )));
        }
        let raw_path = mapping
            .get(serde_yaml::Value::String("path".to_string()))
            .and_then(serde_yaml::Value::as_str)
            .ok_or_else(|| ViaError::InvalidConfig("Provider 缓存路径缺失".to_string()))?;
        let relative = Path::new(raw_path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(ViaError::InvalidConfig(
                "Provider 缓存路径不安全".to_string(),
            ));
        }
        let provider_path = state.paths.mihomo_home_dir.join(relative);
        if !provider_path.starts_with(&state.paths.providers_dir) {
            return Err(ViaError::InvalidConfig(
                "Provider 缓存路径越出受控目录".to_string(),
            ));
        }
        let raw_metadata = std::fs::symlink_metadata(&provider_path)?;
        if raw_metadata.file_type().is_symlink() {
            return Err(ViaError::InvalidConfig(
                "Provider 缓存不允许使用符号链接".to_string(),
            ));
        }
        let canonical_provider = std::fs::canonicalize(&provider_path)?;
        let canonical_root = std::fs::canonicalize(&state.paths.providers_dir)?;
        if !canonical_provider.starts_with(canonical_root) {
            return Err(ViaError::InvalidConfig(
                "Provider 缓存真实路径越出受控目录".to_string(),
            ));
        }
        let metadata = std::fs::metadata(&canonical_provider)?;
        if metadata.len() > MAX_PROVIDER_BYTES as u64 {
            return Err(ViaError::InvalidConfig("Provider 缓存过大".to_string()));
        }
        let provider_bytes = std::fs::read(canonical_provider)?;
        let rule_behavior = if proxy_provider {
            None
        } else {
            Some(
                mapping
                    .get(serde_yaml::Value::String("behavior".to_string()))
                    .and_then(serde_yaml::Value::as_str)
                    .ok_or_else(|| {
                        ViaError::InvalidConfig("规则 Provider 缺少 behavior".to_string())
                    })?
                    .to_string(),
            )
        };
        let format = mapping
            .get(serde_yaml::Value::String("format".to_string()))
            .and_then(serde_yaml::Value::as_str)
            .unwrap_or("yaml");
        let payload = provider_payload(&provider_bytes, proxy_provider, format)?;
        mapping.clear();
        mapping.insert(
            serde_yaml::Value::String("type".to_string()),
            serde_yaml::Value::String("inline".to_string()),
        );
        if let Some(behavior) = rule_behavior {
            mapping.insert(
                serde_yaml::Value::String("behavior".to_string()),
                serde_yaml::Value::String(behavior),
            );
        }
        mapping.insert(serde_yaml::Value::String("payload".to_string()), payload);
    }
    Ok(())
}

fn provider_payload(
    bytes: &[u8],
    proxy_provider: bool,
    format: &str,
) -> ViaResult<serde_yaml::Value> {
    if format == "mrs" {
        return Err(ViaError::InvalidConfig(
            "TUN helper v1 暂不支持 MRS Provider；请改用 YAML 或文本格式".to_string(),
        ));
    }
    if format == "text" {
        if proxy_provider {
            return Err(ViaError::InvalidConfig(
                "代理 Provider 只支持 YAML".to_string(),
            ));
        }
        let text = std::str::from_utf8(bytes)
            .map_err(|_| ViaError::InvalidConfig("文本 Provider 不是 UTF-8".to_string()))?;
        return Ok(serde_yaml::Value::Sequence(
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(|line| serde_yaml::Value::String(line.to_string()))
                .collect(),
        ));
    }
    let document: serde_yaml::Value = serde_yaml::from_slice(bytes)
        .map_err(|error| ViaError::InvalidConfig(format!("Provider YAML 无效：{error}")))?;
    if let Some(sequence) = document.as_sequence() {
        return Ok(serde_yaml::Value::Sequence(sequence.clone()));
    }
    let mapping = document
        .as_mapping()
        .ok_or_else(|| ViaError::InvalidConfig("Provider YAML 根节点无效".to_string()))?;
    let key = if proxy_provider { "proxies" } else { "payload" };
    mapping
        .get(serde_yaml::Value::String(key.to_string()))
        .filter(|value| value.is_sequence())
        .cloned()
        .ok_or_else(|| ViaError::InvalidConfig(format!("Provider YAML 缺少 {key} 列表")))
}

fn yaml_port(root: &serde_yaml::Mapping, key: &str) -> ViaResult<u16> {
    root.get(serde_yaml::Value::String(key.to_string()))
        .and_then(serde_yaml::Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| ViaError::InvalidConfig(format!("配置缺少受控 {key}")))
}

async fn import_profile_bytes(
    state: &AppState,
    source_label: &str,
    source_kind: ProfileSourceKind,
    bytes: &[u8],
) -> ViaResult<ProfileSummary> {
    let _operation = state.operation_lock.lock().await;

    let sanitized = sanitize_for_current_settings(state, bytes).await?;
    fetch_profile_providers(state, &sanitized, true).await?;

    atomic_write_bytes(
        state.paths.runtime_profile_file(),
        sanitized.effective_yaml.as_bytes(),
    )?;
    validate_effective_profile(state, &state.paths.runtime_profile_file()).await?;

    // Keep the untrusted source only after the candidate has passed Mihomo's
    // own parser. It is re-sanitized before every later launch.
    atomic_write_bytes(state.paths.source_profile_file(), bytes)?;
    atomic_write_bytes(
        state.paths.last_valid_profile_file(),
        sanitized.effective_yaml.as_bytes(),
    )?;
    atomic_write_bytes(
        state.paths.effective_profile_file(),
        sanitized.effective_yaml.as_bytes(),
    )?;

    let (proxy_count, rule_count) = count_profile_items(&sanitized.effective_yaml);
    let now = Utc::now().to_rfc3339();
    let profile = ProfileSummary {
        id: "active".to_string(),
        name: match source_kind {
            ProfileSourceKind::Subscription => "我的订阅".to_string(),
            ProfileSourceKind::File => "本地配置".to_string(),
        },
        source_kind,
        masked_source: source_label.to_string(),
        updated_at: now.clone(),
        last_valid_at: now,
        is_valid: true,
        proxy_count,
        rule_count,
    };
    {
        let mut snapshot = state.snapshot.write().await;
        snapshot.profile = Some(profile.clone());
        snapshot.runtime.local_port = None;
        snapshot.connection.error_message = None;
    }
    state.persist_profile_metadata().await?;
    let _ = crate::logging::event(&state.paths.log_dir, "profile_imported");
    Ok(profile)
}

async fn sanitize_for_current_settings(
    state: &AppState,
    bytes: &[u8],
) -> ViaResult<crate::config::SanitizedProfile> {
    let mixed_port = find_free_loopback_port()?;
    let mut controller_port = find_free_loopback_port()?;
    while controller_port == mixed_port {
        controller_port = find_free_loopback_port()?;
    }
    let snapshot = state.snapshot.read().await;
    let transport = match snapshot.connection.transport_mode {
        TransportMode::System => Transport::System,
        TransportMode::Tun => Transport::Tun,
    };
    let dns_mode = match snapshot.settings.dns_mode {
        DnsMode::FakeIp => ConfigDnsMode::FakeIp,
        DnsMode::RedirHost => ConfigDnsMode::RedirHost,
    };
    drop(snapshot);
    sanitize_profile(
        bytes,
        &SanitizeOptions {
            mixed_port,
            controller_port,
            controller_secret: random_controller_secret(),
            transport,
            dns_mode,
        },
    )
    .map_err(|error| ViaError::InvalidConfig(error.to_string()))
}

async fn fetch_profile_providers(
    state: &AppState,
    sanitized: &crate::config::SanitizedProfile,
    refresh: bool,
) -> ViaResult<()> {
    for provider in &sanitized.provider_downloads {
        let destination = state.paths.mihomo_home_dir.join(&provider.cache_path);
        if !destination.starts_with(&state.paths.providers_dir) {
            return Err(ViaError::InvalidConfig(
                "Provider 缓存路径越出受控目录".to_string(),
            ));
        }
        if !refresh && destination.is_file() {
            continue;
        }
        let url = Url::parse(&provider.url)
            .map_err(|error| ViaError::InvalidConfig(error.to_string()))?;
        let fetched = fetch_https(&url, MAX_PROVIDER_BYTES).await?;
        atomic_write_bytes(destination, &fetched.bytes)?;
    }
    Ok(())
}

async fn validate_effective_profile(state: &AppState, profile: &Path) -> ViaResult<()> {
    let binary = core_binary_path()?;
    let mut command = Command::new(binary);
    command
        .arg("-t")
        .arg("-d")
        .arg(&state.paths.mihomo_home_dir)
        .arg("-f")
        .arg(profile)
        .env_remove("SAFE_PATHS")
        .current_dir(&state.paths.mihomo_home_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let output = tokio::time::timeout(VALIDATION_TIMEOUT, command.output())
        .await
        .map_err(|_| ViaError::InvalidConfig("Mihomo 配置校验超时".to_string()))?
        .map_err(|error| ViaError::Core(format!("无法运行 Mihomo 校验：{error}")))?;
    if output.status.success() {
        return Ok(());
    }

    // Mihomo may echo complete proxy definitions on parser errors. Never pass
    // its raw stderr across the untrusted-profile boundary.
    Err(ViaError::InvalidConfig(
        "Mihomo 拒绝了生成的配置；请检查 YAML 语法和节点字段".to_string(),
    ))
}

fn masked_source_label(url: &Url) -> String {
    match url.host_str() {
        Some(host) => format!("{host} 订阅"),
        None => "HTTPS 订阅".to_string(),
    }
}

fn count_profile_items(effective_yaml: &str) -> (usize, usize) {
    let Ok(root) = serde_yaml::from_str::<serde_yaml::Value>(effective_yaml) else {
        return (0, 0);
    };
    let Some(mapping) = root.as_mapping() else {
        return (0, 0);
    };
    let sequence_len = |key: &str| {
        mapping
            .get(serde_yaml::Value::String(key.to_string()))
            .and_then(serde_yaml::Value::as_sequence)
            .map_or(0, Vec::len)
    };
    (sequence_len("proxies"), sequence_len("rules"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_yaml_and_text_provider_payloads() {
        let proxies = provider_payload(
            b"proxies:\n  - {name: node, type: ss, server: example.test, port: 443, cipher: aes-128-gcm, password: value}\n",
            true,
            "yaml",
        )
        .unwrap();
        assert_eq!(proxies.as_sequence().unwrap().len(), 1);

        let rules = provider_payload(b"# comment\nDOMAIN,example.test\n\n", false, "text").unwrap();
        assert_eq!(rules.as_sequence().unwrap().len(), 1);
    }

    #[test]
    fn privileged_profile_rejects_binary_mrs_provider() {
        assert!(provider_payload(b"MRS", false, "mrs").is_err());
    }
}
