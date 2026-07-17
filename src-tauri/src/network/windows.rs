use std::io;

use ::windows::Win32::Networking::WinInet::{
    INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED, InternetSetOptionW,
};
use serde::{Deserialize, Serialize};
use winreg::{
    RegKey, RegValue,
    enums::{
        HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_BINARY, REG_DWORD, REG_DWORD_BIG_ENDIAN,
        REG_EXPAND_SZ, REG_FULL_RESOURCE_DESCRIPTOR, REG_LINK, REG_MULTI_SZ, REG_NONE, REG_QWORD,
        REG_RESOURCE_LIST, REG_RESOURCE_REQUIREMENTS_LIST, REG_SZ,
    },
    types::ToRegValue,
};

use super::{LoopbackProxyTarget, NetworkError, NetworkResult, ProxyAdapter};

const INTERNET_SETTINGS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
const PROXY_ENABLE: &str = "ProxyEnable";
const PROXY_SERVER: &str = "ProxyServer";
const PROXY_OVERRIDE: &str = "ProxyOverride";
const AUTO_CONFIG_URL: &str = "AutoConfigURL";
const VIA_PROXY_OVERRIDE: &str = "<local>;localhost;127.*;[::1]";

/// Serializable mirror of every registry value type accepted by winreg.
/// Keeping the raw bytes allows restoration of the precise pre-VIA value,
/// including uncommon value types, instead of coercing it through a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryValueKind {
    None,
    String,
    ExpandString,
    Binary,
    Dword,
    DwordBigEndian,
    Link,
    MultiString,
    ResourceList,
    FullResourceDescriptor,
    ResourceRequirementsList,
    Qword,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryValueSnapshot {
    pub kind: RegistryValueKind,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsProxySnapshot {
    pub proxy_enable: Option<RegistryValueSnapshot>,
    pub proxy_server: Option<RegistryValueSnapshot>,
    pub proxy_override: Option<RegistryValueSnapshot>,
    pub auto_config_url: Option<RegistryValueSnapshot>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsProxyAdapter;

impl WindowsProxyAdapter {
    pub fn new() -> Self {
        Self
    }

    fn open_read(&self) -> NetworkResult<RegKey> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(INTERNET_SETTINGS_KEY, KEY_READ)
            .map_err(|error| registry_error("open Internet Settings for reading", error))
    }

    fn open_write(&self) -> NetworkResult<RegKey> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(INTERNET_SETTINGS_KEY, KEY_READ | KEY_WRITE)
            .map_err(|error| registry_error("open Internet Settings for writing", error))
    }

    fn write_snapshot(&self, snapshot: &WindowsProxySnapshot) -> NetworkResult<()> {
        let key = self.open_write()?;

        // Keep the switch off while the other fields are replaced. This makes
        // incomplete writes fail open, and ProxyEnable is committed last.
        let write_result = (|| -> NetworkResult<()> {
            key.set_raw_value(PROXY_ENABLE, &0_u32.to_reg_value())
                .map_err(|error| registry_error("temporarily disable WinINET proxy", error))?;
            write_optional_raw(&key, PROXY_SERVER, &snapshot.proxy_server)?;
            write_optional_raw(&key, PROXY_OVERRIDE, &snapshot.proxy_override)?;
            write_optional_raw(&key, AUTO_CONFIG_URL, &snapshot.auto_config_url)?;
            write_optional_raw(&key, PROXY_ENABLE, &snapshot.proxy_enable)?;
            Ok(())
        })();

        // Notify WinINET after every write attempt, including a failed partial
        // attempt, so consumers do not retain an obsolete in-memory value.
        let refresh_result = refresh_wininet();
        match (write_result, refresh_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(write), Ok(())) => Err(write),
            (Ok(()), Err(refresh)) => Err(refresh),
            (Err(write), Err(refresh)) => Err(NetworkError::adapter(
                "write and refresh Windows proxy",
                format!("{write}; additionally {refresh}"),
            )),
        }
    }

    fn owned_snapshot(target: LoopbackProxyTarget) -> WindowsProxySnapshot {
        WindowsProxySnapshot {
            proxy_enable: Some(snapshot_raw(1_u32.to_reg_value())),
            proxy_server: Some(snapshot_raw(target.proxy_server().to_reg_value())),
            proxy_override: Some(snapshot_raw(VIA_PROXY_OVERRIDE.to_reg_value())),
            // A PAC URL can supersede a manually configured proxy. VIA removes
            // it only while owning the lease and restores its raw baseline.
            auto_config_url: None,
        }
    }
}

impl ProxyAdapter for WindowsProxyAdapter {
    type Snapshot = WindowsProxySnapshot;

    fn capture(&self) -> NetworkResult<Self::Snapshot> {
        let key = self.open_read()?;
        Ok(WindowsProxySnapshot {
            proxy_enable: read_optional_raw(&key, PROXY_ENABLE)?,
            proxy_server: read_optional_raw(&key, PROXY_SERVER)?,
            proxy_override: read_optional_raw(&key, PROXY_OVERRIDE)?,
            auto_config_url: read_optional_raw(&key, AUTO_CONFIG_URL)?,
        })
    }

    fn build_via_owned_state(
        &self,
        _baseline: &Self::Snapshot,
        target: LoopbackProxyTarget,
    ) -> NetworkResult<Self::Snapshot> {
        target.validate()?;
        Ok(Self::owned_snapshot(target))
    }

    fn apply_snapshot(&self, via_owned_state: &Self::Snapshot) -> NetworkResult<()> {
        // The transaction layer performs rollback only after proving that the
        // complete current state equals this VIA-owned snapshot. A partial
        // state retains its dirty lease and is never mistaken for a user value.
        self.write_snapshot(via_owned_state)
    }

    fn restore_snapshot(&self, baseline: &Self::Snapshot) -> NetworkResult<()> {
        self.write_snapshot(baseline)
    }

    fn merge_recovery_state(
        &self,
        current: &Self::Snapshot,
        baseline: &Self::Snapshot,
        via_owned_state: &Self::Snapshot,
    ) -> NetworkResult<Self::Snapshot> {
        Ok(WindowsProxySnapshot {
            proxy_enable: restore_if_owned(
                &current.proxy_enable,
                &baseline.proxy_enable,
                &via_owned_state.proxy_enable,
            ),
            proxy_server: restore_if_owned(
                &current.proxy_server,
                &baseline.proxy_server,
                &via_owned_state.proxy_server,
            ),
            proxy_override: restore_if_owned(
                &current.proxy_override,
                &baseline.proxy_override,
                &via_owned_state.proxy_override,
            ),
            auto_config_url: restore_if_owned(
                &current.auto_config_url,
                &baseline.auto_config_url,
                &via_owned_state.auto_config_url,
            ),
        })
    }
}

fn restore_if_owned<T: Clone + PartialEq>(current: &T, baseline: &T, owned: &T) -> T {
    if current == owned {
        baseline.clone()
    } else {
        current.clone()
    }
}

fn read_optional_raw(
    key: &RegKey,
    value_name: &'static str,
) -> NetworkResult<Option<RegistryValueSnapshot>> {
    match key.get_raw_value(value_name) {
        Ok(value) => Ok(Some(snapshot_raw(value))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(registry_error("read Windows proxy registry value", error)),
    }
}

fn write_optional_raw(
    key: &RegKey,
    value_name: &'static str,
    value: &Option<RegistryValueSnapshot>,
) -> NetworkResult<()> {
    match value {
        Some(value) => key
            .set_raw_value(value_name, &value.clone().into_raw())
            .map_err(|error| registry_error("write Windows proxy registry value", error)),
        None => match key.delete_value(value_name) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(registry_error("delete Windows proxy registry value", error)),
        },
    }
}

fn snapshot_raw(value: RegValue) -> RegistryValueSnapshot {
    RegistryValueSnapshot {
        kind: RegistryValueKind::from_raw(value.vtype),
        bytes: value.bytes,
    }
}

impl RegistryValueSnapshot {
    fn into_raw(self) -> RegValue {
        RegValue {
            bytes: self.bytes,
            vtype: self.kind.into_raw(),
        }
    }
}

impl RegistryValueKind {
    fn from_raw(kind: winreg::enums::RegType) -> Self {
        match kind {
            REG_NONE => Self::None,
            REG_SZ => Self::String,
            REG_EXPAND_SZ => Self::ExpandString,
            REG_BINARY => Self::Binary,
            REG_DWORD => Self::Dword,
            REG_DWORD_BIG_ENDIAN => Self::DwordBigEndian,
            REG_LINK => Self::Link,
            REG_MULTI_SZ => Self::MultiString,
            REG_RESOURCE_LIST => Self::ResourceList,
            REG_FULL_RESOURCE_DESCRIPTOR => Self::FullResourceDescriptor,
            REG_RESOURCE_REQUIREMENTS_LIST => Self::ResourceRequirementsList,
            REG_QWORD => Self::Qword,
        }
    }

    fn into_raw(self) -> winreg::enums::RegType {
        match self {
            Self::None => REG_NONE,
            Self::String => REG_SZ,
            Self::ExpandString => REG_EXPAND_SZ,
            Self::Binary => REG_BINARY,
            Self::Dword => REG_DWORD,
            Self::DwordBigEndian => REG_DWORD_BIG_ENDIAN,
            Self::Link => REG_LINK,
            Self::MultiString => REG_MULTI_SZ,
            Self::ResourceList => REG_RESOURCE_LIST,
            Self::FullResourceDescriptor => REG_FULL_RESOURCE_DESCRIPTOR,
            Self::ResourceRequirementsList => REG_RESOURCE_REQUIREMENTS_LIST,
            Self::Qword => REG_QWORD,
        }
    }
}

fn registry_error(operation: &'static str, error: io::Error) -> NetworkError {
    NetworkError::adapter(operation, error.to_string())
}

fn refresh_wininet() -> NetworkResult<()> {
    // SAFETY: both options explicitly require a null internet handle and no
    // option buffer. No borrowed pointer crosses the FFI boundary.
    let settings = unsafe { InternetSetOptionW(None, INTERNET_OPTION_SETTINGS_CHANGED, None, 0) };
    // Attempt refresh even when notification failed so callers receive the
    // strongest best-effort cache invalidation possible.
    let refresh = unsafe { InternetSetOptionW(None, INTERNET_OPTION_REFRESH, None, 0) };

    match (settings, refresh) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(settings), Ok(())) => Err(NetworkError::adapter(
            "notify WinINET settings changed",
            settings.to_string(),
        )),
        (Ok(()), Err(refresh)) => Err(NetworkError::adapter(
            "refresh WinINET proxy cache",
            refresh.to_string(),
        )),
        (Err(settings), Err(refresh)) => Err(NetworkError::adapter(
            "refresh WinINET proxy cache",
            format!("settings notification failed: {settings}; refresh failed: {refresh}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_state_is_loopback_only_and_disables_pac() {
        let target = LoopbackProxyTarget::new(17890).unwrap();
        let state = WindowsProxyAdapter::owned_snapshot(target);

        assert_eq!(state.auto_config_url, None);
        assert_eq!(state.proxy_enable, Some(snapshot_raw(1_u32.to_reg_value())));
        assert_eq!(
            state.proxy_server,
            Some(snapshot_raw("127.0.0.1:17890".to_reg_value()))
        );
    }

    #[test]
    fn recovery_merge_preserves_external_fields_and_removes_owned_fields() {
        let baseline = WindowsProxySnapshot {
            proxy_enable: Some(snapshot_raw(0_u32.to_reg_value())),
            proxy_server: Some(snapshot_raw("baseline:8080".to_reg_value())),
            proxy_override: None,
            auto_config_url: Some(snapshot_raw("https://baseline/pac".to_reg_value())),
        };
        let owned = WindowsProxyAdapter::owned_snapshot(LoopbackProxyTarget::new(17890).unwrap());
        let mut current = owned.clone();
        current.proxy_server = Some(snapshot_raw("external:9090".to_reg_value()));

        let merged = WindowsProxyAdapter
            .merge_recovery_state(&current, &baseline, &owned)
            .unwrap();

        assert_eq!(merged.proxy_enable, baseline.proxy_enable);
        assert_eq!(merged.proxy_server, current.proxy_server);
        assert_eq!(merged.proxy_override, baseline.proxy_override);
        assert_eq!(merged.auto_config_url, baseline.auto_config_url);
    }
}
