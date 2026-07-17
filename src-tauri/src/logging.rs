use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    time::{Duration, SystemTime},
};

use chrono::Utc;

use crate::error::{ViaError, ViaResult};

const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

pub fn initialize(log_dir: &Path) -> ViaResult<()> {
    cleanup_expired(log_dir)?;
    event(log_dir, "app_started")
}

/// Append a controlled event identifier only. Untrusted strings and core
/// output never enter this log, which makes it redacted by construction.
pub fn event(log_dir: &Path, event_name: &'static str) -> ViaResult<()> {
    if !event_name
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(ViaError::Other("拒绝写入不安全的日志事件名".to_string()));
    }
    fs::create_dir_all(log_dir)?;
    let path = log_dir.join(format!("via-{}.log", Utc::now().format("%Y-%m-%d")));
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    serde_json::to_writer(
        &mut file,
        &serde_json::json!({
            "timestamp": Utc::now().to_rfc3339(),
            "event": event_name,
        }),
    )?;
    file.write_all(b"\n")?;
    Ok(())
}

fn cleanup_expired(log_dir: &Path) -> ViaResult<()> {
    let now = SystemTime::now();
    for entry in match fs::read_dir(log_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    } {
        let entry = entry?;
        let path = entry.path();
        let managed_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("via-") && name.ends_with(".log"));
        if !managed_name || !entry.file_type()?.is_file() {
            continue;
        }
        let expired = entry
            .metadata()?
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > RETENTION);
        if expired {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_only_controlled_event_data() {
        let directory = tempfile::tempdir().unwrap();
        event(directory.path(), "core_started").unwrap();
        let entry = fs::read_dir(directory.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let content = fs::read_to_string(entry.path()).unwrap();
        assert!(content.contains("core_started"));
        assert!(!content.contains("://"));
    }
}
