use std::{fs, path::PathBuf};

use tauri::{AppHandle, Manager};

use crate::error::{ViaError, ViaResult};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub log_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub mihomo_home_dir: PathBuf,
    pub profiles_dir: PathBuf,
    pub providers_dir: PathBuf,
}

impl AppPaths {
    pub fn discover(app: &AppHandle) -> ViaResult<Self> {
        let resolver = app.path();
        let config_dir = resolver
            .app_config_dir()
            .map_err(|error| ViaError::Other(format!("无法定位配置目录：{error}")))?;
        let data_dir = resolver
            .app_data_dir()
            .map_err(|error| ViaError::Other(format!("无法定位数据目录：{error}")))?;
        let cache_dir = resolver
            .app_cache_dir()
            .map_err(|error| ViaError::Other(format!("无法定位缓存目录：{error}")))?;
        let log_dir = resolver
            .app_log_dir()
            .map_err(|error| ViaError::Other(format!("无法定位日志目录：{error}")))?;
        let runtime_dir = data_dir.join("runtime");
        let mihomo_home_dir = cache_dir.join("mihomo");
        let profiles_dir = data_dir.join("profiles");
        let providers_dir = mihomo_home_dir.join("providers");

        let paths = Self {
            config_dir,
            data_dir,
            cache_dir,
            log_dir,
            runtime_dir,
            mihomo_home_dir,
            profiles_dir,
            providers_dir,
        };
        paths.create_private_directories()?;
        Ok(paths)
    }

    fn create_private_directories(&self) -> ViaResult<()> {
        for path in [
            &self.config_dir,
            &self.data_dir,
            &self.cache_dir,
            &self.log_dir,
            &self.runtime_dir,
            &self.mihomo_home_dir,
            &self.profiles_dir,
            &self.providers_dir,
        ] {
            fs::create_dir_all(path)?;
            set_owner_only(path)?;
        }
        Ok(())
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    pub fn profile_metadata_file(&self) -> PathBuf {
        self.data_dir.join("profile.json")
    }

    pub fn effective_profile_file(&self) -> PathBuf {
        self.profiles_dir.join("active.yaml")
    }

    /// The original, untrusted profile bytes. They are never passed directly
    /// to Mihomo; every connection re-runs the sanitizer from this source so
    /// ports, DNS, and transport settings cannot become stale.
    pub fn source_profile_file(&self) -> PathBuf {
        self.profiles_dir.join("source.yaml")
    }

    pub fn last_valid_profile_file(&self) -> PathBuf {
        self.profiles_dir.join("last-valid.yaml")
    }

    pub fn network_lease_file(&self) -> PathBuf {
        self.data_dir.join("network-lease.json")
    }

    pub fn runtime_profile_file(&self) -> PathBuf {
        self.mihomo_home_dir.join("runtime.yaml")
    }
}

#[cfg(unix)]
fn set_owner_only(path: &std::path::Path) -> ViaResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only(_path: &std::path::Path) -> ViaResult<()> {
    // Windows directories inherit the current user's profile ACL. The installer
    // applies an explicit ACL to privileged helper files, which live elsewhere.
    Ok(())
}
