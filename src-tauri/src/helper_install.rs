use std::{path::PathBuf, process::Command};

use tauri::{AppHandle, Manager};

use crate::{
    error::{ViaError, ViaResult},
    helper::{HelperClient, HelperOperation, HelperResponseBody},
};

pub async fn installed_client() -> ViaResult<HelperClient> {
    let client = HelperClient::from_installed()
        .map_err(|error| ViaError::Other(format!("TUN helper 未安装或不可用：{error}")))?;
    let response = client
        .request(HelperOperation::Status)
        .await
        .map_err(|error| ViaError::Other(format!("无法连接 TUN helper：{error}")))?;
    match response.body {
        HelperResponseBody::Status { status }
            if response.ok && status.core_installed && status.protocol_version == 1 =>
        {
            Ok(client)
        }
        HelperResponseBody::Error { message, .. } => Err(ViaError::Other(format!(
            "TUN helper 拒绝状态检查：{message}"
        ))),
        _ => Err(ViaError::Other(
            "TUN helper 状态不完整，请重新安装权限组件".to_string(),
        )),
    }
}

pub async fn ensure_installed(app: &AppHandle) -> ViaResult<HelperClient> {
    if let Ok(client) = installed_client().await {
        return Ok(client);
    }
    run_platform_installer(app, "Install").await?;
    // Scheduled task / LaunchDaemon startup may lag behind installer exit.
    let mut last_error = None;
    for _ in 0..20 {
        match installed_client().await {
            Ok(client) => return Ok(client),
            Err(error) => last_error = Some(error),
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let last_error = last_error
        .unwrap_or_else(|| ViaError::Other("权限组件安装完成，但 helper 未响应".to_string()));
    #[cfg(target_os = "macos")]
    {
        Err(ViaError::Other(format!(
            "权限组件已安装，但当前登录会话还不能访问 helper。首次安装后请注销并重新登录 macOS，再重新启用 TUN：{last_error}"
        )))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(last_error)
    }
}

pub async fn remove_installed(app: &AppHandle) -> ViaResult<()> {
    run_platform_installer(app, "Remove").await
}

async fn run_platform_installer(app: &AppHandle, action: &'static str) -> ViaResult<()> {
    let (script, payload_root) = installer_paths(app)?;
    tokio::task::spawn_blocking(move || run_installer_blocking(script, payload_root, action))
        .await
        .map_err(|error| ViaError::Other(format!("权限组件安装任务异常：{error}")))?
}

fn installer_paths(app: &AppHandle) -> ViaResult<(PathBuf, Option<PathBuf>)> {
    if let Ok(resource_dir) = app.path().resource_dir() {
        let payload = resource_dir.join("helper-payload");
        let script = payload.join(installer_file_name());
        if script.is_file() {
            return Ok((script, Some(payload)));
        }
    }

    #[cfg(debug_assertions)]
    {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repository = manifest
            .parent()
            .ok_or_else(|| ViaError::Other("无法定位开发仓库中的 helper 安装脚本".to_string()))?;
        let script = repository.join("scripts").join(installer_file_name());
        if script.is_file() {
            return Ok((script, None));
        }
    }

    Err(ViaError::Other(
        "安装包缺少 TUN helper 载荷，请重新安装 VIA".to_string(),
    ))
}

#[cfg(windows)]
fn installer_file_name() -> &'static str {
    "install-helper.ps1"
}

#[cfg(target_os = "macos")]
fn installer_file_name() -> &'static str {
    "install-helper.sh"
}

#[cfg(windows)]
fn run_installer_blocking(
    script: PathBuf,
    payload_root: Option<PathBuf>,
    action: &'static str,
) -> ViaResult<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script)
        .args(["-Action", action])
        .creation_flags(CREATE_NO_WINDOW);
    if let Some(payload_root) = payload_root {
        command.arg("-PayloadRoot").arg(payload_root);
    }
    let status = command
        .status()
        .map_err(|error| ViaError::Other(format!("无法启动 Windows 权限安装器：{error}")))?;
    if !status.success() {
        return Err(ViaError::Other(format!(
            "Windows 权限安装器未完成（{status}）"
        )));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_installer_blocking(
    script: PathBuf,
    payload_root: Option<PathBuf>,
    action: &'static str,
) -> ViaResult<()> {
    let client_user = current_macos_user()?;
    let payload_root = payload_root
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let shell_command = format!(
        "/bin/sh {} {} {} {}",
        shell_quote(&script.to_string_lossy()),
        action.to_ascii_lowercase(),
        shell_quote(&payload_root),
        shell_quote(&client_user),
    );
    let apple_script = format!(
        "do shell script \"{}\" with administrator privileges",
        shell_command.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let status = Command::new("/usr/bin/osascript")
        .args(["-e", &apple_script])
        .status()
        .map_err(|error| ViaError::Other(format!("无法启动 macOS 权限安装器：{error}")))?;
    if !status.success() {
        return Err(ViaError::Other(format!(
            "macOS 权限安装器未完成（{status}）"
        )));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn current_macos_user() -> ViaResult<String> {
    let output = Command::new("/usr/bin/id")
        .arg("-un")
        .output()
        .map_err(|error| ViaError::Other(format!("无法识别当前 macOS 用户：{error}")))?;
    if !output.status.success() {
        return Err(ViaError::Other(format!(
            "无法识别当前 macOS 用户（{}）",
            output.status
        )));
    }
    let user = String::from_utf8(output.stdout)
        .map_err(|_| ViaError::Other("macOS 用户名不是有效 UTF-8".to_string()))?;
    let user = user.trim();
    if user.is_empty()
        || user == "root"
        || !user
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ViaError::Other(
            "当前 macOS 用户名不能安全地交给权限安装器".to_string(),
        ));
    }
    Ok(user.to_string())
}

#[cfg(target_os = "macos")]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn installer_file_name() -> &'static str {
    "unsupported"
}

#[cfg(not(any(windows, target_os = "macos")))]
fn run_installer_blocking(
    _script: PathBuf,
    _payload_root: Option<PathBuf>,
    _action: &'static str,
) -> ViaResult<()> {
    Err(ViaError::Other(
        "TUN helper 只支持 Windows 与 macOS".to_string(),
    ))
}
