use std::{
    fs,
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
};

use tauri::AppHandle;
use tokio::sync::{Mutex, RwLock};

use crate::{
    core::{ControllerClient, CoreSupervisor},
    credentials::CredentialStore,
    error::ViaResult,
    model::{AppSettings, AppSnapshot, ProfileSummary},
    network,
    paths::AppPaths,
};

pub struct AppState {
    pub snapshot: RwLock<AppSnapshot>,
    pub operation_lock: Mutex<()>,
    pub connection_lock: Mutex<()>,
    pub connection_epoch: AtomicU64,
    pub paths: AppPaths,
    pub credentials: CredentialStore,
    pub core: CoreSupervisor,
    pub tun: Mutex<Option<TunRuntime>>,
}

#[derive(Clone)]
pub struct TunRuntime {
    pub session_id: uuid::Uuid,
    pub controller: ControllerClient,
}

impl AppState {
    pub fn initialize(app: &AppHandle, version: &str) -> ViaResult<Self> {
        let paths = AppPaths::discover(app)?;
        crate::logging::initialize(&paths.log_dir)?;
        let mut snapshot = AppSnapshot::initial(version);

        if let Err(error) = network::recover_native_proxy(paths.network_lease_file()) {
            snapshot.connection.status = crate::model::ConnectionStatus::Error;
            snapshot.connection.error_message = Some(crate::error::redact_sensitive(&format!(
                "上次异常退出后的系统代理恢复失败：{error}。请先运行网络修复"
            )));
        }

        if let Ok(bytes) = fs::read(paths.settings_file())
            && let Ok(settings) = serde_json::from_slice::<AppSettings>(&bytes)
        {
            snapshot.settings = settings;
        }

        if let Ok(bytes) = fs::read(paths.profile_metadata_file())
            && let Ok(profile) = serde_json::from_slice::<ProfileSummary>(&bytes)
        {
            snapshot.profile = Some(profile);
        }

        Ok(Self {
            snapshot: RwLock::new(snapshot),
            operation_lock: Mutex::new(()),
            connection_lock: Mutex::new(()),
            connection_epoch: AtomicU64::new(0),
            paths,
            credentials: CredentialStore::default(),
            core: CoreSupervisor::default(),
            tun: Mutex::new(None),
        })
    }

    pub fn next_connection_epoch(&self) -> u64 {
        self.connection_epoch.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn current_connection_epoch(&self) -> u64 {
        self.connection_epoch.load(Ordering::Acquire)
    }

    pub async fn active_controller(&self) -> Option<ControllerClient> {
        if let Some(controller) = self.core.controller().await {
            return Some(controller);
        }
        self.tun
            .lock()
            .await
            .as_ref()
            .map(|runtime| runtime.controller.clone())
    }

    pub async fn snapshot(&self) -> AppSnapshot {
        self.snapshot.read().await.clone()
    }

    pub async fn persist_settings(&self) -> ViaResult<()> {
        let settings = self.snapshot.read().await.settings.clone();
        atomic_write_json(self.paths.settings_file(), &settings)
    }

    pub async fn persist_profile_metadata(&self) -> ViaResult<()> {
        let profile = self.snapshot.read().await.profile.clone();
        match profile {
            Some(profile) => atomic_write_json(self.paths.profile_metadata_file(), &profile),
            None => {
                let path = self.paths.profile_metadata_file();
                if path.exists() {
                    fs::remove_file(path)?;
                }
                Ok(())
            }
        }
    }
}

pub fn atomic_write_json<T: serde::Serialize>(
    destination: std::path::PathBuf,
    value: &T,
) -> ViaResult<()> {
    let parent = destination
        .parent()
        .expect("managed files always have a parent");
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, value)?;
    temp.write_all(b"\n")?;
    temp.as_file_mut().sync_all()?;
    temp.persist(&destination).map_err(|error| error.error)?;
    Ok(())
}

pub fn atomic_write_bytes(destination: impl AsRef<std::path::Path>, bytes: &[u8]) -> ViaResult<()> {
    let destination = destination.as_ref();
    let parent = destination
        .parent()
        .expect("managed files always have a parent");
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file_mut().sync_all()?;
    temp.persist(destination).map_err(|error| error.error)?;
    Ok(())
}
