use std::{
    fs::File,
    io::Read,
    net::{Ipv4Addr, TcpListener},
    path::PathBuf,
};

use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::{ViaError, ViaResult};

#[cfg(target_os = "windows")]
const EXPECTED_EXECUTABLE_SHA256: &str =
    "c14bda8dc4cc8910ccd2110fe2be083c51a1b66da59141a0b87aff6fe6126517";
#[cfg(target_os = "macos")]
const EXPECTED_EXECUTABLE_SHA256: &str =
    "55b7286331cb30a54b2564013b02b84a0c280e8b690bd1e5da4b9d4f4ca007ac";

pub fn core_binary_path() -> ViaResult<PathBuf> {
    #[cfg(target_os = "windows")]
    let development_name = "mihomo-x86_64-pc-windows-msvc.exe";
    #[cfg(target_os = "macos")]
    let development_name = "mihomo-aarch64-apple-darwin";

    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(development_name);
    if cfg!(debug_assertions) && development.is_file() {
        verify_binary(&development)?;
        return Ok(development);
    }

    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or_else(|| ViaError::Core("无法定位应用程序目录".to_string()))?;
    #[cfg(target_os = "windows")]
    let bundled = directory.join("mihomo.exe");
    #[cfg(target_os = "macos")]
    let bundled = directory.join("mihomo");
    if !bundled.is_file() {
        return Err(ViaError::Core(format!(
            "未找到内置 Mihomo：{}",
            bundled.display()
        )));
    }
    verify_binary(&bundled)?;
    Ok(bundled)
}

fn verify_binary(path: &std::path::Path) -> ViaResult<()> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = hex::encode(hasher.finalize());
    if actual == EXPECTED_EXECUTABLE_SHA256 {
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let bytes = std::fs::read(path)?;
        let normalized = super::macho_integrity::normalized_macho_sha256(&bytes)
            .map_err(|_| ViaError::Core("Mihomo Mach-O 完整性结构无效".to_string()))?;
        if normalized != env!("VIA_MACOS_MIHOMO_CONTENT_SHA256") {
            return Err(ViaError::Core(format!(
                "Mihomo 完整性校验失败：{}",
                path.display()
            )));
        }
        let status = std::process::Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict"])
            .arg(path)
            .status()
            .map_err(|error| ViaError::Core(format!("无法校验 Mihomo 代码签名：{error}")))?;
        if !status.success() {
            return Err(ViaError::Core("Mihomo 代码签名校验失败".to_string()));
        }
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    Err(ViaError::Core(format!(
        "Mihomo 完整性校验失败：{}",
        path.display()
    )))
}

pub fn find_free_loopback_port() -> ViaResult<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

pub fn random_controller_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_secret_has_256_bits_of_hex() {
        let first = random_controller_secret();
        let second = random_controller_secret();
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn free_port_is_loopback_bindable() {
        let port = find_free_loopback_port().unwrap();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
        assert_eq!(listener.local_addr().unwrap().port(), port);
    }
}
