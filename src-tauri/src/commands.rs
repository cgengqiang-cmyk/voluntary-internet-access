use std::{fs, path::PathBuf, time::Duration};

use chrono::Utc;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::{
    core::ControllerClient,
    error::{ViaError, ViaResult, command_error, redact_sensitive},
    helper::{HelperClient, HelperOperation, HelperResponse, HelperResponseBody},
    helper_install,
    model::{
        AppSettings, AppSnapshot, ConnectionStatus, DelayState, DiagnosticExportResult, ProxyGroup,
        ProxyGroupKind, ProxyMode, ProxyNode, RepairResult, SettingsPatch, TransportMode,
    },
    profile,
    state::{AppState, TunRuntime, atomic_write_bytes},
};

struct StartedConnection {
    session_id: uuid::Uuid,
    controller: ControllerClient,
    mixed_port: u16,
    version: String,
    transport: TransportMode,
    helper_client: Option<HelperClient>,
}

#[tauri::command]
pub async fn get_app_snapshot(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    Ok(state.snapshot().await)
}

#[tauri::command]
pub async fn import_subscription(
    url: String,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let _connection = state.connection_lock.lock().await;
    ensure_disconnected(&state).await?;
    profile::import_subscription(&state, &url)
        .await
        .map_err(command_error)?;
    Ok(state.snapshot().await)
}

#[tauri::command]
pub async fn import_profile_file(
    path: String,
    contents: Option<String>,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let _connection = state.connection_lock.lock().await;
    ensure_disconnected(&state).await?;
    let contents = contents.ok_or_else(|| "未读取到 YAML 文件内容".to_string())?;
    profile::import_local_profile(&state, &path, contents.as_bytes())
        .await
        .map_err(command_error)?;
    Ok(state.snapshot().await)
}

#[tauri::command]
pub async fn refresh_profile(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let _connection = state.connection_lock.lock().await;
    ensure_disconnected(&state).await?;
    profile::refresh_subscription(&state)
        .await
        .map_err(command_error)?;
    Ok(state.snapshot().await)
}

#[tauri::command]
pub async fn set_transport_mode(
    mode: TransportMode,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let _connection = state.connection_lock.lock().await;
    if state.snapshot.read().await.connection.status != ConnectionStatus::Disconnected {
        return Err("请先断开连接再切换代理方式".to_string());
    }
    if mode == TransportMode::Tun {
        match helper_install::ensure_installed(&app).await {
            Ok(_) => state.snapshot.write().await.runtime.helper_installed = true,
            Err(error) => {
                state.snapshot.write().await.runtime.helper_installed = false;
                return Err(command_error(error));
            }
        }
    }
    state.snapshot.write().await.connection.transport_mode = mode;
    Ok(state.snapshot().await)
}

#[tauri::command]
pub async fn set_proxy_mode(
    mode: ProxyMode,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    if let Some(controller) = state.active_controller().await {
        controller.set_mode(mode).await.map_err(command_error)?;
    }
    state.snapshot.write().await.connection.proxy_mode = mode;
    Ok(state.snapshot().await)
}

#[tauri::command]
pub async fn select_proxy(
    group_id: String,
    proxy_id: String,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let controller = state
        .active_controller()
        .await
        .ok_or_else(|| "请先连接代理".to_string())?;
    controller
        .select_proxy(&group_id, &proxy_id)
        .await
        .map_err(command_error)?;
    refresh_proxy_groups(&state).await.map_err(command_error)?;
    Ok(state.snapshot().await)
}

#[tauri::command]
pub async fn test_proxy_delay(
    group_id: String,
    proxy_id: String,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let controller = state
        .active_controller()
        .await
        .ok_or_else(|| "请先连接代理".to_string())?;
    let delay = controller
        .test_delay(&proxy_id)
        .await
        .map_err(command_error)?;
    let mut snapshot = state.snapshot.write().await;
    if let Some(proxy) = snapshot
        .proxy_groups
        .iter_mut()
        .find(|group| group.id == group_id)
        .and_then(|group| group.proxies.iter_mut().find(|proxy| proxy.id == proxy_id))
    {
        proxy.delay_ms = Some(delay);
        proxy.delay_state = DelayState::Available;
    }
    drop(snapshot);
    Ok(state.snapshot().await)
}

#[tauri::command]
pub async fn update_settings(
    settings: SettingsPatch,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    if settings.dns_mode.is_some()
        && state.snapshot.read().await.connection.status != ConnectionStatus::Disconnected
    {
        return Err("请先断开连接再切换 DNS 模式".to_string());
    }
    if let Some(enabled) = settings.launch_on_startup {
        let autostart = app.autolaunch();
        if enabled {
            autostart.enable().map_err(command_error)?;
        } else {
            autostart.disable().map_err(command_error)?;
        }
    }
    {
        let mut snapshot = state.snapshot.write().await;
        if let Some(enabled) = settings.launch_on_startup {
            snapshot.settings.launch_on_startup = enabled;
        }
        if let Some(enabled) = settings.auto_connect {
            snapshot.settings.auto_connect = enabled;
        }
        if let Some(mode) = settings.dns_mode {
            snapshot.settings.dns_mode = mode;
        }
    }
    state.persist_settings().await.map_err(command_error)?;
    Ok(state.snapshot().await)
}

#[tauri::command]
pub async fn export_diagnostics(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DiagnosticExportResult, String> {
    let snapshot = state.snapshot().await;
    let desktop = app.path().desktop_dir().map_err(command_error)?;
    let destination = desktop.join(format!(
        "VIA-诊断报告-{}.json",
        Utc::now().format("%Y%m%d-%H%M%S")
    ));
    // Export a deliberately narrow DTO. Names and identifiers originate in
    // untrusted profiles and may themselves contain credentials or URLs.
    let payload = serde_json::to_vec_pretty(&serde_json::json!({
        "schemaVersion": 1,
        "generatedAt": Utc::now().to_rfc3339(),
        "connection": {
            "status": snapshot.connection.status,
            "transportMode": snapshot.connection.transport_mode,
            "proxyMode": snapshot.connection.proxy_mode,
            "connectedSince": snapshot.connection.connected_since,
            "errorMessage": snapshot.connection.error_message.as_deref().map(redact_sensitive),
        },
        "profile": snapshot.profile.as_ref().map(|profile| serde_json::json!({
            "sourceKind": profile.source_kind,
            "updatedAt": profile.updated_at,
            "lastValidAt": profile.last_valid_at,
            "isValid": profile.is_valid,
            "proxyCount": profile.proxy_count,
            "ruleCount": profile.rule_count,
        })),
        "proxyGroupCount": snapshot.proxy_groups.len(),
        "proxyNodeCount": snapshot.proxy_groups.iter().map(|group| group.proxies.len()).sum::<usize>(),
        "settings": {
            "launchOnStartup": snapshot.settings.launch_on_startup,
            "autoConnect": snapshot.settings.auto_connect,
            "dnsMode": snapshot.settings.dns_mode,
        },
        "runtime": {
            "coreVersion": snapshot.runtime.core_version,
            "appVersion": snapshot.runtime.app_version,
            "controllerHealthy": snapshot.runtime.controller_healthy,
            "helperInstalled": snapshot.runtime.helper_installed,
            "platform": snapshot.runtime.platform_label,
        }
    }))
    .map_err(command_error)?;
    atomic_write_bytes(&destination, &payload).map_err(command_error)?;
    Ok(DiagnosticExportResult {
        path: destination.display().to_string(),
    })
}

#[tauri::command]
pub async fn repair_network(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RepairResult, String> {
    let _connection = state.connection_lock.lock().await;
    state.next_connection_epoch();
    let tun_result = restore_any_tun(&app, &state).await;
    let proxy_result = restore_system_proxy(&state).await;
    if proxy_result.is_ok() {
        state.core.stop().await.map_err(command_error)?;
    }
    let mut errors = Vec::new();
    if let Err(error) = &tun_result {
        errors.push(command_error(error));
    }
    if let Err(error) = &proxy_result {
        errors.push(command_error(error));
    }
    if !errors.is_empty() {
        let message = format!("网络修复未完全成功：{}", errors.join("；"));
        let mut snapshot = state.snapshot.write().await;
        snapshot.connection.status = ConnectionStatus::Error;
        snapshot.connection.error_message = Some(message.clone());
        return Err(message);
    }
    set_disconnected(&state).await;
    let outcome = proxy_result.expect("checked above");
    Ok(RepairResult {
        summary: format!("Mihomo/TUN 已停止；{}", recovery_summary(outcome)),
    })
}

#[tauri::command]
pub async fn uninstall_components(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RepairResult, String> {
    let _connection = state.connection_lock.lock().await;
    state.next_connection_epoch();
    restore_any_tun(&app, &state).await.map_err(command_error)?;
    restore_system_proxy(&state).await.map_err(command_error)?;
    state.core.stop().await.map_err(command_error)?;
    if HelperClient::from_installed().is_ok() {
        let client = helper_install::ensure_installed(&app)
            .await
            .map_err(command_error)?;
        let response = client
            .request(HelperOperation::Remove { confirm: true })
            .await
            .map_err(command_error)?;
        expect_helper_response(response, |body| matches!(body, HelperResponseBody::Removed))
            .map_err(command_error)?;
        helper_install::remove_installed(&app)
            .await
            .map_err(command_error)?;
    }
    let _ = app.autolaunch().disable();
    state
        .credentials
        .clear_subscription_url()
        .await
        .map_err(command_error)?;
    for path in [
        state.paths.effective_profile_file(),
        state.paths.source_profile_file(),
        state.paths.last_valid_profile_file(),
        state.paths.profile_metadata_file(),
        state.paths.runtime_profile_file(),
        state.paths.settings_file(),
    ] {
        if path.exists() {
            fs::remove_file(path).map_err(command_error)?;
        }
    }
    remove_managed_tree(&state.paths.providers_dir, &state.paths.cache_dir)?;
    remove_managed_tree(&state.paths.log_dir, &state.paths.log_dir)?;
    fs::create_dir_all(&state.paths.providers_dir).map_err(command_error)?;
    fs::create_dir_all(&state.paths.log_dir).map_err(command_error)?;
    {
        let mut snapshot = state.snapshot.write().await;
        snapshot.profile = None;
        snapshot.settings = AppSettings::default();
        snapshot.runtime.helper_installed = false;
    }
    set_disconnected(&state).await;
    Ok(RepairResult {
        summary: "用户配置、凭据与运行组件已清理".to_string(),
    })
}

#[tauri::command]
pub async fn set_connection(
    enabled: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    set_connection_inner(enabled, app, &state).await
}

pub(crate) async fn set_connection_inner(
    enabled: bool,
    app: AppHandle,
    state: &AppState,
) -> Result<AppSnapshot, String> {
    let _connection = state.connection_lock.lock().await;
    let current_status = state.snapshot.read().await.connection.status;
    if enabled && current_status == ConnectionStatus::Connected {
        return Ok(state.snapshot().await);
    }
    if enabled
        && matches!(
            current_status,
            ConnectionStatus::Connecting | ConnectionStatus::Disconnecting
        )
    {
        return Err("连接状态正在切换，请稍候".to_string());
    }
    if !enabled {
        state.next_connection_epoch();
        state.snapshot.write().await.connection.status = ConnectionStatus::Disconnecting;
        if let Err(error) = stop_active_connection(state).await {
            let message = redact_sensitive(&format!(
                "网络恢复失败，代理进程将暂时保留以避免断网：{error}"
            ));
            let mut snapshot = state.snapshot.write().await;
            snapshot.connection.status = ConnectionStatus::Error;
            snapshot.connection.error_message = Some(message.clone());
            return Err(message);
        }
        set_disconnected(state).await;
        return Ok(state.snapshot().await);
    }
    if state.snapshot.read().await.profile.is_none() {
        return Err("请先导入一份有效配置".to_string());
    }
    let transport = state.snapshot.read().await.connection.transport_mode;
    state.snapshot.write().await.connection.status = ConnectionStatus::Connecting;
    if let Err(error) = profile::prepare_runtime_profile(state).await {
        let mut snapshot = state.snapshot.write().await;
        snapshot.connection.status = ConnectionStatus::Error;
        snapshot.connection.error_message = Some(command_error(&error));
        return Err(command_error(error));
    }
    let started = match start_connection(state, transport).await {
        Ok(started) => started,
        Err(error) => {
            let mut snapshot = state.snapshot.write().await;
            snapshot.connection.status = ConnectionStatus::Error;
            snapshot.connection.error_message = Some(command_error(&error));
            return Err(command_error(error));
        }
    };

    let proxy_mode = state.snapshot.read().await.connection.proxy_mode;
    if let Err(error) = started.controller.set_mode(proxy_mode).await {
        let _ = stop_active_connection(state).await;
        let mut snapshot = state.snapshot.write().await;
        snapshot.connection.status = ConnectionStatus::Error;
        snapshot.connection.error_message = Some(command_error(&error));
        return Err(command_error(error));
    }

    let epoch = state.next_connection_epoch();
    let mut startup_warning = None;
    {
        let mut snapshot = state.snapshot.write().await;
        snapshot.connection.status = ConnectionStatus::Connected;
        snapshot.connection.connected_since = Some(Utc::now().to_rfc3339());
        snapshot.connection.error_message = None;
        snapshot.runtime.core_version = format!("Mihomo {}", started.version);
        snapshot.runtime.local_port = Some(started.mixed_port);
        snapshot.runtime.controller_healthy = true;
        if !snapshot.settings.first_connection_completed {
            snapshot.settings.first_connection_completed = true;
            snapshot.settings.auto_connect = true;
            match app.autolaunch().enable() {
                Ok(()) => snapshot.settings.launch_on_startup = true,
                Err(error) => {
                    snapshot.settings.launch_on_startup = false;
                    startup_warning = Some(redact_sensitive(&format!(
                        "已连接，但无法启用开机启动：{error}"
                    )));
                }
            }
        }
    }

    // Establish crash recovery before any later operation that can fail. Once
    // the OS proxy/TUN is active, callers must never observe an error without
    // either a watchdog or an explicit rollback path.
    match started.transport {
        TransportMode::System => spawn_core_watchdog(app, started.session_id, epoch),
        TransportMode::Tun => {
            let client = started
                .helper_client
                .expect("TUN connections always retain a helper client");
            spawn_tun_watchdog(app, client, started.session_id, epoch);
        }
    }

    if let Err(error) = state.persist_settings().await {
        startup_warning.get_or_insert_with(|| {
            redact_sensitive(&format!("已连接，但无法保存启动偏好：{error}"))
        });
    }
    if let Err(error) = refresh_proxy_groups(state).await {
        startup_warning
            .get_or_insert_with(|| redact_sensitive(&format!("代理组读取失败：{error}")));
    }
    if let Some(warning) = startup_warning {
        state.snapshot.write().await.connection.error_message = Some(warning);
    }
    Ok(state.snapshot().await)
}

pub(crate) async fn auto_connect_after_startup(app: AppHandle) {
    // Give the WebView enough time to subscribe to state changes. The command
    // remains authoritative; failure is reflected in the shared snapshot.
    tokio::time::sleep(Duration::from_millis(350)).await;
    let state = app.state::<AppState>();
    let helper_available = helper_install::installed_client().await.is_ok();
    state.snapshot.write().await.runtime.helper_installed = helper_available;
    let should_connect = {
        let snapshot = state.snapshot.read().await;
        snapshot.settings.auto_connect
            && snapshot.profile.is_some()
            && snapshot.connection.status != ConnectionStatus::Error
    };
    if should_connect {
        let _ = set_connection_inner(true, app.clone(), &state).await;
    }
}

async fn start_connection(
    state: &AppState,
    transport: TransportMode,
) -> ViaResult<StartedConnection> {
    match transport {
        TransportMode::System => {
            let started = state.core.start_user_mode(&state.paths).await?;
            if let Err(error) = apply_system_proxy(state, started.mixed_port).await {
                let _ = state.core.stop().await;
                return Err(error);
            }
            Ok(StartedConnection {
                session_id: started.session_id,
                controller: started.controller,
                mixed_port: started.mixed_port,
                version: started.version,
                transport,
                helper_client: None,
            })
        }
        TransportMode::Tun => start_tun_connection(state).await,
    }
}

async fn start_tun_connection(state: &AppState) -> ViaResult<StartedConnection> {
    let client = helper_install::installed_client().await?;
    let prepared = profile::prepare_privileged_tun_profile(state).await?;
    let install = client
        .request(HelperOperation::Install {
            config_yaml: prepared.yaml,
            config_sha256: prepared.sha256.clone(),
        })
        .await
        .map_err(|error| ViaError::Other(format!("TUN 配置安装失败：{error}")))?;
    expect_helper_response(install, |body| {
        matches!(
            body,
            HelperResponseBody::Installed { config_sha256 }
                if config_sha256 == &prepared.sha256
        )
    })?;

    let session_id = uuid::Uuid::new_v4();
    let start = client
        .request(HelperOperation::StartTun {
            session_id,
            config_sha256: prepared.sha256,
            controller_secret: prepared.controller_secret.clone(),
            heartbeat_timeout_seconds: 15,
        })
        .await
        .map_err(|error| ViaError::Other(format!("TUN 启动失败：{error}")))?;
    expect_helper_response(start, |body| {
        matches!(body, HelperResponseBody::Started { .. })
    })?;

    let controller = ControllerClient::new(prepared.controller_port, prepared.controller_secret)?;
    let version = match wait_for_tun_ready(&controller, prepared.mixed_port).await {
        Ok(version) => version,
        Err(error) => {
            let _ = client
                .request(HelperOperation::Restore {
                    session_id: Some(session_id),
                })
                .await;
            return Err(error);
        }
    };
    *state.tun.lock().await = Some(TunRuntime {
        session_id,
        controller: controller.clone(),
    });
    state.snapshot.write().await.runtime.helper_installed = true;
    let _ = crate::logging::event(&state.paths.log_dir, "tun_started");
    Ok(StartedConnection {
        session_id,
        controller,
        mixed_port: prepared.mixed_port,
        version,
        transport: TransportMode::Tun,
        helper_client: Some(client),
    })
}

fn expect_helper_response(
    response: HelperResponse,
    expected: impl FnOnce(&HelperResponseBody) -> bool,
) -> ViaResult<()> {
    if response.ok && expected(&response.body) {
        return Ok(());
    }
    match response.body {
        HelperResponseBody::Error { message, .. } => {
            Err(ViaError::Other(format!("TUN helper 拒绝操作：{message}")))
        }
        _ => Err(ViaError::Other("TUN helper 返回了意外响应".to_string())),
    }
}

async fn wait_for_tun_ready(controller: &ControllerClient, mixed_port: u16) -> ViaResult<String> {
    for _ in 0..80 {
        if let Ok(version) = controller.version().await
            && tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, mixed_port))
                .await
                .is_ok()
        {
            return Ok(version);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(ViaError::Core("高权限 TUN 内核就绪检查超时".to_string()))
}

async fn stop_active_connection(state: &AppState) -> ViaResult<()> {
    let tun_session = state
        .tun
        .lock()
        .await
        .as_ref()
        .map(|runtime| runtime.session_id);
    if let Some(session_id) = tun_session {
        let client = helper_install::installed_client().await?;
        let response = client
            .request(HelperOperation::StopTun { session_id })
            .await
            .map_err(|error| ViaError::Other(format!("TUN 停止失败：{error}")))?;
        if expect_helper_response(response, |body| matches!(body, HelperResponseBody::Stopped))
            .is_err()
        {
            let restore = client
                .request(HelperOperation::Restore {
                    session_id: Some(session_id),
                })
                .await
                .map_err(|error| ViaError::Other(format!("TUN 恢复失败：{error}")))?;
            expect_helper_response(restore, |body| matches!(body, HelperResponseBody::Restored))?;
        }
        state.tun.lock().await.take();
        let _ = crate::logging::event(&state.paths.log_dir, "tun_stopped");
        return Ok(());
    }

    restore_system_proxy(state).await?;
    state.core.stop().await?;
    Ok(())
}

async fn restore_any_tun(app: &AppHandle, state: &AppState) -> ViaResult<bool> {
    if HelperClient::from_installed().is_err() {
        state.tun.lock().await.take();
        state.snapshot.write().await.runtime.helper_installed = false;
        return Ok(false);
    }
    let client = helper_install::ensure_installed(app).await?;
    let response = client
        .request(HelperOperation::Restore { session_id: None })
        .await
        .map_err(|error| ViaError::Other(format!("TUN 恢复失败：{error}")))?;
    expect_helper_response(response, |body| {
        matches!(body, HelperResponseBody::Restored)
    })?;
    state.tun.lock().await.take();
    state.snapshot.write().await.runtime.helper_installed = true;
    let _ = crate::logging::event(&state.paths.log_dir, "tun_restored");
    Ok(true)
}

async fn apply_system_proxy(state: &AppState, mixed_port: u16) -> ViaResult<()> {
    let lease_path = state.paths.network_lease_file();
    tokio::task::spawn_blocking(move || crate::network::apply_native_proxy(lease_path, mixed_port))
        .await
        .map_err(|error| ViaError::Network(format!("系统代理任务异常：{error}")))?
        .map_err(|error| ViaError::Network(error.to_string()))?;
    let _ = crate::logging::event(&state.paths.log_dir, "system_proxy_applied");
    Ok(())
}

async fn restore_system_proxy(state: &AppState) -> ViaResult<crate::network::RecoveryOutcome> {
    let lease_path = state.paths.network_lease_file();
    let outcome =
        tokio::task::spawn_blocking(move || crate::network::restore_native_proxy(lease_path))
            .await
            .map_err(|error| ViaError::Network(format!("系统代理恢复任务异常：{error}")))?
            .map_err(|error| ViaError::Network(error.to_string()))?;
    let _ = crate::logging::event(&state.paths.log_dir, "system_proxy_restored");
    Ok(outcome)
}

fn recovery_summary(outcome: crate::network::RecoveryOutcome) -> &'static str {
    use crate::network::RecoveryOutcome;
    match outcome {
        RecoveryOutcome::NoLease => "未发现 VIA 网络租约",
        RecoveryOutcome::Restored => "系统代理已恢复到连接前状态",
        RecoveryOutcome::AlreadyAtBaseline => "系统代理原本已恢复",
        RecoveryOutcome::PreservedExternalChange => "检测到外部修改，已保留用户当前设置",
    }
}

async fn ensure_disconnected(state: &AppState) -> Result<(), String> {
    let status = state.snapshot.read().await.connection.status;
    match status {
        ConnectionStatus::Disconnected => Ok(()),
        ConnectionStatus::Error if state.active_controller().await.is_none() => Ok(()),
        _ => Err("请先断开连接再更换或刷新配置".to_string()),
    }
}

fn spawn_tun_watchdog(app: AppHandle, client: HelperClient, session_id: uuid::Uuid, epoch: u64) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let state = app.state::<AppState>();
            if state.current_connection_epoch() != epoch {
                return;
            }
            let heartbeat = client
                .request(HelperOperation::Heartbeat { session_id })
                .await;
            let healthy = matches!(
                heartbeat,
                Ok(HelperResponse {
                    ok: true,
                    body: HelperResponseBody::HeartbeatAccepted,
                    ..
                })
            );
            if healthy {
                continue;
            }
            let _ = client
                .request(HelperOperation::Restore {
                    session_id: Some(session_id),
                })
                .await;
            if state.current_connection_epoch() != epoch {
                return;
            }
            if state
                .tun
                .lock()
                .await
                .as_ref()
                .is_some_and(|runtime| runtime.session_id == session_id)
            {
                state.tun.lock().await.take();
            }
            let mut snapshot = state.snapshot.write().await;
            snapshot.connection.status = ConnectionStatus::Error;
            snapshot.connection.error_message = Some(
                "TUN helper 心跳中断；高权限内核已进入自动停止流程，请运行网络修复".to_string(),
            );
            snapshot.connection.connected_since = None;
            snapshot.runtime.local_port = None;
            snapshot.runtime.controller_healthy = false;
            snapshot.proxy_groups.clear();
            let _ = crate::logging::event(&state.paths.log_dir, "tun_watchdog_failed");
            return;
        }
    });
}

fn spawn_core_watchdog(app: AppHandle, session_id: uuid::Uuid, epoch: u64) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let state = app.state::<AppState>();
            if state.current_connection_epoch() != epoch {
                return;
            }
            match state.core.is_session_running(session_id).await {
                Ok(true) => continue,
                Ok(false) => {
                    if state.current_connection_epoch() != epoch {
                        return;
                    }
                    let recovery = restore_system_proxy(&state).await;
                    let mut snapshot = state.snapshot.write().await;
                    snapshot.connection.status = ConnectionStatus::Error;
                    snapshot.connection.error_message = Some(redact_sensitive(&match recovery {
                        Ok(outcome) => format!("Mihomo 意外退出；{}", recovery_summary(outcome)),
                        Err(error) => format!(
                            "Mihomo 意外退出，系统代理自动恢复失败：{error}。请运行网络修复"
                        ),
                    }));
                    snapshot.connection.connected_since = None;
                    snapshot.runtime.local_port = None;
                    snapshot.runtime.controller_healthy = false;
                    snapshot.proxy_groups.clear();
                    return;
                }
                Err(error) => {
                    let mut snapshot = state.snapshot.write().await;
                    snapshot.connection.error_message = Some(redact_sensitive(&format!(
                        "Mihomo 运行状态检查失败：{error}"
                    )));
                }
            }
        }
    });
}

async fn refresh_proxy_groups(state: &AppState) -> ViaResult<()> {
    let Some(controller) = state.active_controller().await else {
        return Ok(());
    };
    let groups = controller.proxy_groups().await?;
    let groups = groups
        .into_iter()
        .map(|group| ProxyGroup {
            id: group.name.clone(),
            name: group.name,
            kind: match group.group_type.to_ascii_lowercase().as_str() {
                "urltest" | "url-test" => ProxyGroupKind::UrlTest,
                "fallback" => ProxyGroupKind::Fallback,
                _ => ProxyGroupKind::Select,
            },
            selected_proxy_id: group.selected,
            proxies: group
                .proxies
                .into_iter()
                .map(|proxy| ProxyNode {
                    id: proxy.name.clone(),
                    name: proxy.name,
                    protocol: proxy.protocol,
                    delay_ms: proxy.delay_ms,
                    delay_state: if proxy.available {
                        DelayState::Available
                    } else {
                        DelayState::Idle
                    },
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let mut snapshot = state.snapshot.write().await;
    snapshot.connection.active_group_name = groups.first().map(|group| group.name.clone());
    snapshot.connection.active_proxy_name =
        groups.first().map(|group| group.selected_proxy_id.clone());
    snapshot.proxy_groups = groups;
    Ok(())
}

async fn set_disconnected(state: &AppState) {
    let mut snapshot = state.snapshot.write().await;
    snapshot.connection.status = ConnectionStatus::Disconnected;
    snapshot.connection.connected_since = None;
    snapshot.connection.error_message = None;
    snapshot.runtime.local_port = None;
    snapshot.runtime.controller_healthy = false;
    snapshot.proxy_groups.clear();
}

#[allow(dead_code)]
fn ensure_managed_path(path: PathBuf, root: &std::path::Path) -> Result<PathBuf, ViaError> {
    if path.starts_with(root) {
        Ok(path)
    } else {
        Err(ViaError::InvalidInput("路径越出受控目录".to_string()))
    }
}

fn remove_managed_tree(path: &std::path::Path, root: &std::path::Path) -> Result<(), String> {
    if !path.starts_with(root) || path.parent().is_none() {
        return Err("拒绝清理越出 VIA 受控目录的路径".to_string());
    }
    if path.exists() {
        fs::remove_dir_all(path).map_err(command_error)?;
    }
    Ok(())
}
