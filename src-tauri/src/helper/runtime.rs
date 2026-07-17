use std::{
    fs,
    io::Write,
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    HELPER_PROTOCOL_VERSION, HelperError, HelperLayout, HelperOperation, HelperRequest,
    HelperResponse, HelperResponseBody, HelperResult, HelperStatus,
    config::{digest, install_config, load_and_validate_config},
    layout::PINNED_CORE_SHA256,
};

const HELPER_LEASE_VERSION: u32 = 1;

pub(crate) struct HelperRuntime {
    layout: HelperLayout,
    auth_token: String,
    inner: Mutex<RuntimeState>,
}

struct RuntimeState {
    child: Option<Child>,
    active: Option<ActiveSession>,
    last_error: Option<String>,
}

struct ActiveSession {
    session_id: Uuid,
    pid: u32,
    config_sha256: String,
    started_at: DateTime<Utc>,
    last_heartbeat_at: DateTime<Utc>,
    last_heartbeat_monotonic: Instant,
    heartbeat_timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HelperLease {
    version: u32,
    dirty: bool,
    session_id: Uuid,
    pid: u32,
    core_sha256: String,
    config_sha256: String,
    started_at: DateTime<Utc>,
    last_heartbeat_at: DateTime<Utc>,
    heartbeat_timeout_seconds: u16,
    checksum: String,
}

#[derive(Serialize)]
struct HelperLeaseIntegrity<'a> {
    version: u32,
    dirty: bool,
    session_id: Uuid,
    pid: u32,
    core_sha256: &'a str,
    config_sha256: &'a str,
    started_at: DateTime<Utc>,
    last_heartbeat_at: DateTime<Utc>,
    heartbeat_timeout_seconds: u16,
}

impl HelperLease {
    fn from_active(active: &ActiveSession) -> HelperResult<Self> {
        let mut lease = Self {
            version: HELPER_LEASE_VERSION,
            dirty: true,
            session_id: active.session_id,
            pid: active.pid,
            core_sha256: PINNED_CORE_SHA256.to_string(),
            config_sha256: active.config_sha256.clone(),
            started_at: active.started_at,
            last_heartbeat_at: active.last_heartbeat_at,
            heartbeat_timeout_seconds: active.heartbeat_timeout.as_secs() as u16,
            checksum: String::new(),
        };
        lease.checksum = lease.calculate_checksum()?;
        Ok(lease)
    }

    fn verify(&self) -> HelperResult<()> {
        if self.version != HELPER_LEASE_VERSION || !self.dirty {
            return Err(HelperError::Core(
                "invalid helper lease version or state".to_string(),
            ));
        }
        if self.core_sha256 != PINNED_CORE_SHA256
            || self.heartbeat_timeout_seconds < 5
            || self.heartbeat_timeout_seconds > 60
        {
            return Err(HelperError::Core(
                "helper lease failed policy validation".to_string(),
            ));
        }
        if self.calculate_checksum()? != self.checksum {
            return Err(HelperError::Core(
                "helper lease checksum mismatch".to_string(),
            ));
        }
        Ok(())
    }

    fn calculate_checksum(&self) -> HelperResult<String> {
        let view = HelperLeaseIntegrity {
            version: self.version,
            dirty: self.dirty,
            session_id: self.session_id,
            pid: self.pid,
            core_sha256: &self.core_sha256,
            config_sha256: &self.config_sha256,
            started_at: self.started_at,
            last_heartbeat_at: self.last_heartbeat_at,
            heartbeat_timeout_seconds: self.heartbeat_timeout_seconds,
        };
        let bytes = serde_json::to_vec(&view)?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }
}

impl HelperRuntime {
    pub(crate) fn from_installed() -> HelperResult<Self> {
        let layout = HelperLayout::installed();
        let auth_token = layout.read_auth_token()?;
        let runtime = Self {
            layout,
            auth_token,
            inner: Mutex::new(RuntimeState {
                child: None,
                active: None,
                last_error: None,
            }),
        };
        runtime.recover_stale_process()?;
        Ok(runtime)
    }

    pub(crate) fn dispatch(&self, request: HelperRequest) -> HelperResponse {
        let request_id = request.request_id;
        let result = request
            .validate_shape()
            .and_then(|()| self.authenticate(&request.auth_token))
            .and_then(|()| self.execute(request.operation));
        match result {
            Ok(body) => HelperResponse::success(request_id, body),
            Err(error) => {
                if let Ok(mut state) = self.inner.lock() {
                    state.last_error = Some(redact_error(&error));
                }
                HelperResponse::error(request_id, &error)
            }
        }
    }

    pub(crate) fn watchdog_tick(&self) -> HelperResult<()> {
        let mut state = self.inner.lock().map_err(|_| HelperError::LockPoisoned)?;
        if let Some(child) = state.child.as_mut()
            && child
                .try_wait()
                .map_err(|source| HelperError::io("poll privileged core", source))?
                .is_some()
        {
            state.child = None;
            state.active = None;
            self.clear_lease()?;
            state.last_error = Some("TUN core exited; routes were released".to_string());
            return Ok(());
        }

        let expired = state.active.as_ref().is_some_and(|active| {
            active.last_heartbeat_monotonic.elapsed() > active.heartbeat_timeout
        });
        if expired {
            self.stop_locked(&mut state)?;
            state.last_error =
                Some("desktop heartbeat expired; TUN was stopped fail-open".to_string());
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn shutdown(&self) -> HelperResult<()> {
        let mut state = self.inner.lock().map_err(|_| HelperError::LockPoisoned)?;
        self.stop_locked(&mut state)
    }

    fn authenticate(&self, supplied: &str) -> HelperResult<()> {
        let expected = self.auth_token.as_bytes();
        let supplied = supplied.as_bytes();
        let mut different = expected.len() ^ supplied.len();
        for index in 0..expected.len().max(supplied.len()) {
            different |= usize::from(
                expected.get(index).copied().unwrap_or_default()
                    ^ supplied.get(index).copied().unwrap_or_default(),
            );
        }
        if different != 0 {
            return Err(HelperError::Unauthorized);
        }
        Ok(())
    }

    fn execute(&self, operation: HelperOperation) -> HelperResult<HelperResponseBody> {
        match operation {
            HelperOperation::Status => Ok(HelperResponseBody::Status {
                status: self.status()?,
            }),
            HelperOperation::Install {
                config_yaml,
                config_sha256,
            } => {
                let state = self.inner.lock().map_err(|_| HelperError::LockPoisoned)?;
                if state.active.is_some() {
                    return Err(HelperError::SessionConflict(
                        "configuration cannot change while TUN is active".to_string(),
                    ));
                }
                drop(state);
                self.layout.verify_core()?;
                install_config(&self.layout, &config_yaml, &config_sha256)?;
                Ok(HelperResponseBody::Installed { config_sha256 })
            }
            HelperOperation::Remove { confirm: _ } => {
                let mut state = self.inner.lock().map_err(|_| HelperError::LockPoisoned)?;
                self.stop_locked(&mut state)?;
                match fs::remove_file(&self.layout.config) {
                    Ok(()) => {}
                    Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                    Err(source) => {
                        return Err(HelperError::io("remove privileged configuration", source));
                    }
                }
                Ok(HelperResponseBody::Removed)
            }
            HelperOperation::StartTun {
                session_id,
                config_sha256,
                controller_secret,
                heartbeat_timeout_seconds,
            } => self.start_tun(
                session_id,
                config_sha256,
                controller_secret,
                heartbeat_timeout_seconds,
            ),
            HelperOperation::Heartbeat { session_id } => {
                self.heartbeat(session_id)?;
                Ok(HelperResponseBody::HeartbeatAccepted)
            }
            HelperOperation::StopTun { session_id } => {
                let mut state = self.inner.lock().map_err(|_| HelperError::LockPoisoned)?;
                let active = state.active.as_ref().ok_or_else(|| {
                    HelperError::SessionConflict("there is no active TUN session".to_string())
                })?;
                if active.session_id != session_id {
                    return Err(HelperError::SessionConflict(
                        "stop request does not own the active TUN session".to_string(),
                    ));
                }
                self.stop_locked(&mut state)?;
                Ok(HelperResponseBody::Stopped)
            }
            HelperOperation::Restore { session_id } => {
                let mut state = self.inner.lock().map_err(|_| HelperError::LockPoisoned)?;
                if let (Some(requested), Some(active)) = (session_id, state.active.as_ref())
                    && requested != active.session_id
                {
                    return Err(HelperError::SessionConflict(
                        "restore request does not own the active TUN session".to_string(),
                    ));
                }
                self.stop_locked(&mut state)?;
                Ok(HelperResponseBody::Restored)
            }
        }
    }

    fn status(&self) -> HelperResult<HelperStatus> {
        self.watchdog_tick()?;
        let state = self.inner.lock().map_err(|_| HelperError::LockPoisoned)?;
        Ok(HelperStatus {
            protocol_version: HELPER_PROTOCOL_VERSION,
            core_installed: self.layout.verify_core().is_ok(),
            config_installed: load_and_validate_config(&self.layout).is_ok(),
            running: state.active.is_some(),
            session_id: state.active.as_ref().map(|active| active.session_id),
            pid: state.active.as_ref().map(|active| active.pid),
            last_heartbeat_at: state.active.as_ref().map(|active| active.last_heartbeat_at),
            last_error: state.last_error.clone(),
        })
    }

    fn start_tun(
        &self,
        session_id: Uuid,
        config_sha256: String,
        controller_secret: String,
        heartbeat_timeout_seconds: u16,
    ) -> HelperResult<HelperResponseBody> {
        self.layout.verify_core()?;
        let (config, actual_sha256) = load_and_validate_config(&self.layout)?;
        if config_sha256 != actual_sha256 || digest(&config) != config_sha256 {
            return Err(HelperError::InvalidRequest(
                "start request does not match installed configuration".to_string(),
            ));
        }
        let configured_secret = serde_yaml::from_slice::<serde_yaml::Value>(&config)
            .ok()
            .and_then(|document| document.as_mapping().cloned())
            .and_then(|root| {
                root.get(serde_yaml::Value::String("secret".to_string()))
                    .cloned()
            })
            .and_then(|value| value.as_str().map(str::to_string))
            .ok_or_else(|| {
                HelperError::UnsafeConfiguration("controller secret is missing".to_string())
            })?;
        if configured_secret != controller_secret {
            return Err(HelperError::InvalidRequest(
                "start request does not match the installed controller secret".to_string(),
            ));
        }

        let mut state = self.inner.lock().map_err(|_| HelperError::LockPoisoned)?;
        if state.active.is_some() {
            return Err(HelperError::SessionConflict(
                "a TUN session is already active".to_string(),
            ));
        }
        validate_with_core(&self.layout)?;

        let mut command = core_command(&self.layout);
        command
            .arg("-d")
            .arg(&self.layout.root)
            .arg("-f")
            .arg(&self.layout.config)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|source| HelperError::io("start pinned privileged core", source))?;
        let pid = child.id();
        let now = Utc::now();
        let active = ActiveSession {
            session_id,
            pid,
            config_sha256,
            started_at: now,
            last_heartbeat_at: now,
            last_heartbeat_monotonic: Instant::now(),
            heartbeat_timeout: Duration::from_secs(u64::from(heartbeat_timeout_seconds)),
        };
        if let Err(error) = self.persist_lease(&active) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        state.child = Some(child);
        state.active = Some(active);
        state.last_error = None;
        Ok(HelperResponseBody::Started { pid })
    }

    fn heartbeat(&self, session_id: Uuid) -> HelperResult<()> {
        let mut state = self.inner.lock().map_err(|_| HelperError::LockPoisoned)?;
        let active = state.active.as_mut().ok_or_else(|| {
            HelperError::SessionConflict("there is no active TUN session".to_string())
        })?;
        if active.session_id != session_id {
            return Err(HelperError::SessionConflict(
                "heartbeat does not own the active TUN session".to_string(),
            ));
        }
        active.last_heartbeat_at = Utc::now();
        self.persist_lease(active)?;
        active.last_heartbeat_monotonic = Instant::now();
        Ok(())
    }

    fn stop_locked(&self, state: &mut RuntimeState) -> HelperResult<()> {
        if let Some(child) = state.child.as_mut() {
            match child
                .try_wait()
                .map_err(|source| HelperError::io("poll privileged TUN core", source))?
            {
                Some(_) => {}
                None => {
                    if let Err(kill_error) = child.kill() {
                        let exited = child
                            .try_wait()
                            .map_err(|source| {
                                HelperError::io(
                                    "poll privileged TUN core after stop failure",
                                    source,
                                )
                            })?
                            .is_some();
                        if !exited {
                            return Err(HelperError::io("stop privileged TUN core", kill_error));
                        }
                    }
                    child
                        .wait()
                        .map_err(|source| HelperError::io("reap privileged TUN core", source))?;
                }
            }
        } else if let Some(active) = state.active.as_ref() {
            // An earlier partial stop must remain recoverable from its durable
            // PID lease instead of silently clearing ownership of an orphan.
            terminate_verified_process(active.pid, &self.layout.core)?;
        }

        // Clear in-memory and durable ownership only after the process is
        // confirmed gone. Any kill/wait error leaves all three retry handles.
        state.child = None;
        state.active = None;
        self.clear_lease()?;
        Ok(())
    }

    fn persist_lease(&self, active: &ActiveSession) -> HelperResult<()> {
        fs::create_dir_all(&self.layout.runtime_dir)
            .map_err(|source| HelperError::io("create helper runtime directory", source))?;
        let lease = HelperLease::from_active(active)?;
        let mut temporary = tempfile::NamedTempFile::new_in(&self.layout.runtime_dir)
            .map_err(|source| HelperError::io("create temporary helper lease", source))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|source| HelperError::io("restrict helper lease", source))?;
        }
        serde_json::to_writer_pretty(&mut temporary, &lease)?;
        temporary
            .write_all(b"\n")
            .map_err(|source| HelperError::io("write helper lease", source))?;
        temporary
            .as_file_mut()
            .sync_all()
            .map_err(|source| HelperError::io("flush helper lease", source))?;
        temporary
            .persist(&self.layout.lease)
            .map_err(|error| HelperError::io("replace helper lease", error.error))?;
        Ok(())
    }

    fn clear_lease(&self) -> HelperResult<()> {
        match fs::remove_file(&self.layout.lease) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(HelperError::io("remove helper lease", source)),
        }
    }

    fn recover_stale_process(&self) -> HelperResult<()> {
        let bytes = match fs::read(&self.layout.lease) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(HelperError::io("read stale helper lease", source)),
        };
        let lease: HelperLease = serde_json::from_slice(&bytes)?;
        lease.verify()?;
        terminate_verified_process(lease.pid, &self.layout.core)?;
        self.clear_lease()
    }
}

fn validate_with_core(layout: &HelperLayout) -> HelperResult<()> {
    let status = core_command(layout)
        .arg("-t")
        .arg("-d")
        .arg(&layout.root)
        .arg("-f")
        .arg(&layout.config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|source| HelperError::io("validate privileged configuration with core", source))?;
    if !status.success() {
        return Err(HelperError::Core(format!(
            "pinned core rejected configuration with {status}"
        )));
    }
    Ok(())
}

fn core_command(layout: &HelperLayout) -> Command {
    let mut command = Command::new(&layout.core);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(windows)]
fn terminate_verified_process(pid: u32, expected_core: &std::path::Path) -> HelperResult<()> {
    let powershell = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
    let query = format!("$ErrorActionPreference='Stop'; (Get-Process -Id {pid}).Path");
    let output = Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command", &query])
        .output()
        .map_err(|source| HelperError::io("inspect stale privileged core", source))?;
    if !output.status.success() {
        // A process that is already gone needs no cleanup.
        return Ok(());
    }
    let actual = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let actual = fs::canonicalize(actual)
        .map_err(|source| HelperError::io("canonicalize stale process image", source))?;
    let expected = fs::canonicalize(expected_core)
        .map_err(|source| HelperError::io("canonicalize fixed core image", source))?;
    if actual != expected {
        return Err(HelperError::Core(
            "stale lease PID belongs to a different executable; it was preserved".to_string(),
        ));
    }
    let taskkill = r"C:\Windows\System32\taskkill.exe";
    let status = Command::new(taskkill)
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .map_err(|source| HelperError::io("terminate stale privileged core", source))?;
    if !status.success() {
        return Err(HelperError::Core(format!("taskkill failed with {status}")));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn terminate_verified_process(pid: u32, expected_core: &std::path::Path) -> HelperResult<()> {
    let output = Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .map_err(|source| HelperError::io("inspect stale privileged core", source))?;
    if !output.status.success() || output.stdout.is_empty() {
        return Ok(());
    }
    let actual = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let actual = fs::canonicalize(actual)
        .map_err(|source| HelperError::io("canonicalize stale process image", source))?;
    let expected = fs::canonicalize(expected_core)
        .map_err(|source| HelperError::io("canonicalize fixed core image", source))?;
    if actual != expected {
        return Err(HelperError::Core(
            "stale lease PID belongs to a different executable; it was preserved".to_string(),
        ));
    }
    let status = Command::new("/bin/kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .map_err(|source| HelperError::io("terminate stale privileged core", source))?;
    if !status.success() {
        return Err(HelperError::Core(format!("kill failed with {status}")));
    }
    Ok(())
}

fn redact_error(error: &HelperError) -> String {
    match error {
        HelperError::Unauthorized => "helper authentication failed".to_string(),
        HelperError::UnsafeConfiguration(_) => "TUN configuration was rejected".to_string(),
        HelperError::InvalidRequest(_) => "helper request was rejected".to_string(),
        _ => error.to_string(),
    }
}
