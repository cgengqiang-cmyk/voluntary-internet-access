use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
    Error,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum TransportMode {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "tun")]
    Tun,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProxyMode {
    #[default]
    Rule,
    Global,
    Direct,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum DnsMode {
    #[default]
    #[serde(rename = "fake-ip")]
    FakeIp,
    #[serde(rename = "redir-host")]
    RedirHost,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DelayState {
    #[default]
    Idle,
    Testing,
    Available,
    Timeout,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSnapshot {
    pub status: ConnectionStatus,
    pub transport_mode: TransportMode,
    pub proxy_mode: ProxyMode,
    pub connected_since: Option<String>,
    pub active_group_name: Option<String>,
    pub active_proxy_name: Option<String>,
    pub error_message: Option<String>,
}

impl Default for ConnectionSnapshot {
    fn default() -> Self {
        Self {
            status: ConnectionStatus::Disconnected,
            transport_mode: TransportMode::System,
            proxy_mode: ProxyMode::Rule,
            connected_since: None,
            active_group_name: None,
            active_proxy_name: None,
            error_message: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProfileSourceKind {
    Subscription,
    File,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub id: String,
    pub name: String,
    pub source_kind: ProfileSourceKind,
    pub masked_source: String,
    pub updated_at: String,
    pub last_valid_at: String,
    pub is_valid: bool,
    pub proxy_count: usize,
    pub rule_count: usize,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyNode {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub delay_ms: Option<u32>,
    pub delay_state: DelayState,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyGroupKind {
    #[default]
    Select,
    UrlTest,
    Fallback,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyGroup {
    pub id: String,
    pub name: String,
    pub kind: ProxyGroupKind,
    pub selected_proxy_id: String,
    pub proxies: Vec<ProxyNode>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    pub launch_on_startup: bool,
    pub auto_connect: bool,
    pub dns_mode: DnsMode,
    #[serde(default)]
    pub first_connection_completed: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            launch_on_startup: false,
            auto_connect: false,
            dns_mode: DnsMode::FakeIp,
            first_connection_completed: false,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SettingsPatch {
    pub launch_on_startup: Option<bool>,
    pub auto_connect: Option<bool>,
    pub dns_mode: Option<DnsMode>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSummary {
    pub core_version: String,
    pub app_version: String,
    pub local_port: Option<u16>,
    pub controller_healthy: bool,
    pub helper_installed: bool,
    pub platform_label: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub connection: ConnectionSnapshot,
    pub profile: Option<ProfileSummary>,
    pub proxy_groups: Vec<ProxyGroup>,
    pub settings: AppSettings,
    pub runtime: RuntimeSummary,
    pub is_mock: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticExportResult {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairResult {
    pub summary: String,
}

impl AppSnapshot {
    pub fn initial(app_version: impl Into<String>) -> Self {
        Self {
            connection: ConnectionSnapshot::default(),
            profile: None,
            proxy_groups: Vec::new(),
            settings: AppSettings::default(),
            runtime: RuntimeSummary {
                core_version: "Mihomo v1.19.28".to_string(),
                app_version: format!("VIA {}", app_version.into()),
                local_port: None,
                controller_healthy: false,
                helper_installed: false,
                platform_label: platform_label(),
            },
            is_mock: false,
        }
    }
}

fn platform_label() -> String {
    match std::env::consts::OS {
        "windows" => "Windows 11 x64",
        "macos" => "macOS Apple Silicon",
        other => other,
    }
    .to_string()
}
