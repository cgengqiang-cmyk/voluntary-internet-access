use std::{
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use super::{HelperError, HelperResult};

pub const PIPE_NAME: &str = r"\\.\pipe\via-helper-v1";
pub const SOCKET_PATH: &str = "/var/run/voluntary-internet-access/helper-v1.sock";

#[cfg(windows)]
pub const PINNED_CORE_SHA256: &str =
    "c14bda8dc4cc8910ccd2110fe2be083c51a1b66da59141a0b87aff6fe6126517";
#[cfg(target_os = "macos")]
pub const PINNED_CORE_SHA256: &str =
    "55b7286331cb30a54b2564013b02b84a0c280e8b690bd1e5da4b9d4f4ca007ac";

#[derive(Debug, Clone)]
pub struct HelperLayout {
    pub root: PathBuf,
    pub helper: PathBuf,
    pub core: PathBuf,
    pub runtime_dir: PathBuf,
    pub config: PathBuf,
    pub auth_token: PathBuf,
    pub lease: PathBuf,
}

impl HelperLayout {
    pub fn installed() -> Self {
        #[cfg(windows)]
        let root = PathBuf::from(r"C:\ProgramData\VoluntaryInternetAccess");
        #[cfg(target_os = "macos")]
        let root = PathBuf::from("/Library/Application Support/VoluntaryInternetAccess");

        #[cfg(windows)]
        let core = root.join("core").join("mihomo.exe");
        #[cfg(target_os = "macos")]
        let core = root.join("core").join("mihomo");

        let runtime_dir = root.join("runtime");
        #[cfg(windows)]
        let helper = root.join("via-helper.exe");
        #[cfg(target_os = "macos")]
        let helper = root.join("via-helper");
        Self {
            helper,
            core,
            config: runtime_dir.join("tun.yaml"),
            auth_token: root.join("helper.auth"),
            lease: runtime_dir.join("helper-lease.json"),
            runtime_dir,
            root,
        }
    }

    pub fn verify_core(&self) -> HelperResult<()> {
        self.verify_exact_fixed_file(&self.core)?;
        let bytes =
            fs::read(&self.core).map_err(|source| HelperError::io("read pinned core", source))?;
        let digest = hex::encode(Sha256::digest(bytes));
        if digest != PINNED_CORE_SHA256 {
            return Err(HelperError::CoreDigestMismatch);
        }
        Ok(())
    }

    pub fn verify_config(&self) -> HelperResult<()> {
        self.verify_exact_fixed_file(&self.config)
    }

    pub fn read_auth_token(&self) -> HelperResult<String> {
        self.verify_exact_fixed_file(&self.auth_token)?;
        let token = fs::read_to_string(&self.auth_token)
            .map_err(|source| HelperError::io("read helper authentication token", source))?;
        let token = token.trim().to_string();
        if token.len() != 64
            || !token
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(HelperError::NotInstalled(
                "invalid helper authentication token",
            ));
        }
        Ok(token)
    }

    /// Verify one of this layout's exact fixed files without following a
    /// symlink or accepting a caller-supplied sibling/traversal path.
    pub(crate) fn verify_exact_fixed_file(&self, path: &Path) -> HelperResult<()> {
        if path != self.helper
            && path != self.core
            && path != self.config
            && path != self.auth_token
        {
            return Err(HelperError::UnsafeFixedPath(path.to_path_buf()));
        }
        if !path.starts_with(&self.root) {
            return Err(HelperError::UnsafeFixedPath(path.to_path_buf()));
        }

        let mut cursor = Some(path);
        while let Some(candidate) = cursor {
            let metadata = fs::symlink_metadata(candidate)
                .map_err(|source| HelperError::io("inspect fixed privileged path", source))?;
            if metadata.file_type().is_symlink() {
                return Err(HelperError::UnsafeFixedPath(path.to_path_buf()));
            }
            if candidate == self.root {
                break;
            }
            cursor = candidate.parent();
        }

        let canonical = fs::canonicalize(path)
            .map_err(|source| HelperError::io("canonicalize fixed privileged path", source))?;
        let canonical_root = fs::canonicalize(&self.root)
            .map_err(|source| HelperError::io("canonicalize privileged root", source))?;
        if !canonical.starts_with(canonical_root) || !canonical.is_file() {
            return Err(HelperError::UnsafeFixedPath(path.to_path_buf()));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn for_test(root: PathBuf) -> Self {
        #[cfg(windows)]
        let core = root.join("core").join("mihomo.exe");
        #[cfg(target_os = "macos")]
        let core = root.join("core").join("mihomo");
        let runtime_dir = root.join("runtime");
        #[cfg(windows)]
        let helper = root.join("via-helper.exe");
        #[cfg(target_os = "macos")]
        let helper = root.join("via-helper");
        Self {
            helper,
            core,
            config: runtime_dir.join("tun.yaml"),
            auth_token: root.join("helper.auth"),
            lease: runtime_dir.join("helper-lease.json"),
            runtime_dir,
            root,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_allowlisted_sibling_path() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = HelperLayout::for_test(temporary.path().to_path_buf());
        let attacker = temporary.path().join("attacker.exe");
        fs::write(&attacker, b"not a core").unwrap();

        assert!(matches!(
            layout.verify_exact_fixed_file(&attacker),
            Err(HelperError::UnsafeFixedPath(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_at_the_fixed_config_location() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().unwrap();
        let layout = HelperLayout::for_test(temporary.path().join("root"));
        fs::create_dir_all(&layout.runtime_dir).unwrap();
        fs::create_dir_all(layout.core.parent().unwrap()).unwrap();
        fs::write(
            temporary.path().join("outside.yaml"),
            b"tun: {enable: true}",
        )
        .unwrap();
        symlink(temporary.path().join("outside.yaml"), &layout.config).unwrap();

        assert!(matches!(
            layout.verify_config(),
            Err(HelperError::UnsafeFixedPath(_))
        ));
    }
}
