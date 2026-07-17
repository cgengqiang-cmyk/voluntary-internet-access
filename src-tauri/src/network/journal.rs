use std::{
    fs,
    io::Write,
    marker::PhantomData,
    path::{Path, PathBuf},
};

use serde::{Serialize, de::DeserializeOwned};

use super::{NetworkError, NetworkLease, NetworkResult};

pub trait LeaseJournal<S>: Send + Sync {
    fn load(&self) -> NetworkResult<Option<NetworkLease<S>>>;
    fn persist_dirty(&self, lease: &NetworkLease<S>) -> NetworkResult<()>;
    fn clear(&self) -> NetworkResult<()>;
}

/// An atomic JSON lease journal. The file is durable before an adapter is
/// allowed to mutate operating-system settings.
#[derive(Debug, Clone)]
pub struct FileLeaseJournal<S> {
    path: PathBuf,
    snapshot: PhantomData<fn() -> S>,
}

impl<S> FileLeaseJournal<S> {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            snapshot: PhantomData,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl<S> LeaseJournal<S> for FileLeaseJournal<S>
where
    S: Serialize + DeserializeOwned + Send + Sync,
{
    fn load(&self) -> NetworkResult<Option<NetworkLease<S>>> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(NetworkError::io("read network lease", &self.path, source)),
        };
        let lease: NetworkLease<S> =
            serde_json::from_slice(&bytes).map_err(|source| NetworkError::JournalDecode {
                path: self.path.clone(),
                source,
            })?;
        lease.verify()?;
        Ok(Some(lease))
    }

    fn persist_dirty(&self, lease: &NetworkLease<S>) -> NetworkResult<()> {
        lease.verify()?;
        let parent = self.path.parent().ok_or_else(|| {
            NetworkError::adapter(
                "persist network lease",
                "lease path must have a parent directory",
            )
        })?;
        fs::create_dir_all(parent)
            .map_err(|source| NetworkError::io("create lease directory", parent, source))?;

        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|source| NetworkError::io("create temporary lease", parent, source))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temp.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|source| {
                    NetworkError::io("restrict temporary lease permissions", temp.path(), source)
                })?;
        }

        serde_json::to_writer_pretty(&mut temp, lease).map_err(|source| {
            NetworkError::adapter("serialize network lease", source.to_string())
        })?;
        temp.write_all(b"\n")
            .map_err(|source| NetworkError::io("write network lease", temp.path(), source))?;
        temp.as_file_mut()
            .sync_all()
            .map_err(|source| NetworkError::io("flush network lease", temp.path(), source))?;
        temp.persist(&self.path).map_err(|error| {
            NetworkError::io("atomically replace network lease", &self.path, error.error)
        })?;
        sync_parent_directory(parent)?;
        Ok(())
    }

    fn clear(&self) -> NetworkResult<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => {
                if let Some(parent) = self.path.parent() {
                    sync_parent_directory(parent)?;
                }
                Ok(())
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(NetworkError::io("remove network lease", &self.path, source)),
        }
    }
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> NetworkResult<()> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| NetworkError::io("flush lease directory", parent, source))
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> NetworkResult<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    use super::*;
    use crate::network::{LeaseMode, LoopbackProxyTarget};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Snapshot {
        enabled: bool,
        server: Option<String>,
    }

    fn lease() -> NetworkLease<Snapshot> {
        NetworkLease::new_dirty(
            Uuid::new_v4(),
            LeaseMode::SystemProxy,
            Utc::now(),
            LoopbackProxyTarget::new(17890).unwrap(),
            Snapshot {
                enabled: false,
                server: Some("original.example.invalid:8080".to_string()),
            },
            Snapshot {
                enabled: true,
                server: Some("127.0.0.1:17890".to_string()),
            },
        )
        .unwrap()
    }

    #[test]
    fn file_journal_round_trips_a_verified_lease() {
        let directory = tempfile::tempdir().unwrap();
        let journal = FileLeaseJournal::new(directory.path().join("network-lease.json"));
        let expected = lease();

        journal.persist_dirty(&expected).unwrap();
        assert_eq!(journal.load().unwrap(), Some(expected));
    }

    #[test]
    fn file_journal_rejects_checksum_corruption_without_deleting_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("network-lease.json");
        let journal = FileLeaseJournal::new(path.clone());
        let expected = lease();
        journal.persist_dirty(&expected).unwrap();

        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["checksum"] = serde_json::Value::String("corrupt".to_string());
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        assert!(matches!(
            journal.load().unwrap_err(),
            NetworkError::ChecksumMismatch { .. }
        ));
        assert!(path.exists());
    }
}
