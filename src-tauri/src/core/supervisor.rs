use std::{
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

use shared_child::SharedChild;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    error::{ViaError, ViaResult},
    paths::AppPaths,
};

use super::{ControllerClient, core_binary_path};

pub struct CoreSupervisor {
    runtime: Mutex<Option<CoreRuntime>>,
}

struct CoreRuntime {
    session_id: Uuid,
    child: Arc<SharedChild>,
    controller: ControllerClient,
}

#[derive(Clone)]
pub struct CoreStartInfo {
    pub session_id: Uuid,
    pub controller: ControllerClient,
    pub mixed_port: u16,
    pub version: String,
}

impl Default for CoreSupervisor {
    fn default() -> Self {
        Self {
            runtime: Mutex::new(None),
        }
    }
}

impl CoreSupervisor {
    pub async fn start_user_mode(&self, paths: &AppPaths) -> ViaResult<CoreStartInfo> {
        let mut runtime = self.runtime.lock().await;
        if runtime.is_some() {
            return Err(ViaError::Core("Mihomo 已经在运行".to_string()));
        }

        let profile_bytes = std::fs::read(paths.runtime_profile_file())?;
        let profile: serde_yaml::Value = serde_yaml::from_slice(&profile_bytes)
            .map_err(|error| ViaError::InvalidConfig(error.to_string()))?;
        let root = profile
            .as_mapping()
            .ok_or_else(|| ViaError::InvalidConfig("有效配置根节点无效".to_string()))?;
        let number = |key: &str| {
            root.get(serde_yaml::Value::String(key.to_string()))
                .and_then(serde_yaml::Value::as_u64)
                .and_then(|value| u16::try_from(value).ok())
                .filter(|value| *value > 0)
        };
        let mixed_port = number("mixed-port")
            .ok_or_else(|| ViaError::InvalidConfig("配置缺少受控 mixed-port".to_string()))?;
        let controller_port = root
            .get(serde_yaml::Value::String("external-controller".to_string()))
            .and_then(serde_yaml::Value::as_str)
            .and_then(|value| value.rsplit(':').next())
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| ViaError::InvalidConfig("配置缺少受控控制端口".to_string()))?;
        let secret = root
            .get(serde_yaml::Value::String("secret".to_string()))
            .and_then(serde_yaml::Value::as_str)
            .filter(|value| {
                (32..=128).contains(&value.len())
                    && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            .ok_or_else(|| ViaError::InvalidConfig("配置缺少受控控制密钥".to_string()))?
            .to_string();
        let controller = ControllerClient::new(controller_port, secret.clone())?;
        let binary = core_binary_path()?;

        let mut command = Command::new(binary);
        command
            .arg("-d")
            .arg(&paths.mihomo_home_dir)
            .arg("-f")
            .arg(paths.runtime_profile_file())
            .arg("-ext-ctl")
            .arg(format!("127.0.0.1:{controller_port}"))
            .env_remove("SAFE_PATHS")
            .current_dir(&paths.mihomo_home_dir)
            .stdin(Stdio::null())
            // Mihomo may echo node names or provider URLs. VIA retains only
            // controlled app events, so raw core streams are never persisted.
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = Arc::new(
            SharedChild::spawn(&mut command)
                .map_err(|error| ViaError::Core(format!("无法启动 Mihomo：{error}")))?,
        );
        let readiness = wait_until_ready(&child, &controller, mixed_port).await;
        let version = match readiness {
            Ok(version) => version,
            Err(error) => {
                let child = Arc::clone(&child);
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = child.kill();
                    let _ = child.wait();
                })
                .await;
                return Err(error);
            }
        };

        let session_id = Uuid::new_v4();
        *runtime = Some(CoreRuntime {
            session_id,
            child,
            controller: controller.clone(),
        });
        let _ = crate::logging::event(&paths.log_dir, "core_started");
        Ok(CoreStartInfo {
            session_id,
            controller,
            mixed_port,
            version,
        })
    }

    pub async fn stop(&self) -> ViaResult<()> {
        let mut runtime = self.runtime.lock().await;
        let Some(active) = runtime.as_ref() else {
            return Ok(());
        };
        let child = Arc::clone(&active.child);
        tokio::task::spawn_blocking(move || {
            if child.try_wait()?.is_none() {
                child.kill()?;
            }
            child.wait()?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|error| ViaError::Core(error.to_string()))?
        .map_err(|error| ViaError::Core(error.to_string()))?;
        runtime.take();
        Ok(())
    }

    pub async fn controller(&self) -> Option<ControllerClient> {
        self.runtime
            .lock()
            .await
            .as_ref()
            .map(|runtime| runtime.controller.clone())
    }

    /// Poll a specific process session without confusing a later reconnect
    /// with the process being watched. An exited child is reaped and removed.
    pub async fn is_session_running(&self, session_id: Uuid) -> ViaResult<bool> {
        let mut runtime = self.runtime.lock().await;
        let Some(current) = runtime.as_ref() else {
            return Ok(false);
        };
        if current.session_id != session_id {
            return Ok(false);
        }
        match current
            .child
            .try_wait()
            .map_err(|error| ViaError::Core(error.to_string()))?
        {
            None => Ok(true),
            Some(_) => {
                runtime.take();
                Ok(false)
            }
        }
    }
}

async fn wait_until_ready(
    child: &Arc<SharedChild>,
    controller: &ControllerClient,
    mixed_port: u16,
) -> ViaResult<String> {
    for _ in 0..60 {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| ViaError::Core(error.to_string()))?
        {
            return Err(ViaError::Core(format!("Mihomo 在就绪前退出：{status}")));
        }
        if let Ok(version) = controller.version().await
            && tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, mixed_port))
                .await
                .is_ok()
        {
            return Ok(version);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(ViaError::Core("Mihomo 启动就绪检查超时".to_string()))
}
