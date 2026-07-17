use std::{sync::Arc, time::Duration};

use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::{
    HELPER_PROTOCOL_VERSION, HelperError, HelperLayout, HelperOperation, HelperRequest,
    HelperResponse, HelperResult, PIPE_NAME, config::MAX_TUN_CONFIG_BYTES, runtime::HelperRuntime,
};

const MAX_FRAME_BYTES: usize = MAX_TUN_CONFIG_BYTES + 128 * 1024;

#[derive(Debug, Clone)]
pub struct HelperClient {
    auth_token: String,
}

impl HelperClient {
    pub fn from_installed() -> HelperResult<Self> {
        let auth_token = HelperLayout::installed().read_auth_token()?;
        Ok(Self { auth_token })
    }

    pub async fn request(&self, operation: HelperOperation) -> HelperResult<HelperResponse> {
        let request = HelperRequest::new(self.auth_token.clone(), operation);
        request.validate_shape()?;
        let request_id = request.request_id;

        #[cfg(windows)]
        let mut stream = connect_windows().await?;
        #[cfg(target_os = "macos")]
        let mut stream = {
            let stream = tokio::net::UnixStream::connect(super::SOCKET_PATH)
                .await
                .map_err(|source| HelperError::io("connect to helper socket", source))?;
            let credentials = stream
                .peer_cred()
                .map_err(|source| HelperError::io("authenticate helper socket peer", source))?;
            if credentials.uid() != 0 {
                return Err(HelperError::Unauthorized);
            }
            stream
        };

        write_frame(&mut stream, &request).await?;
        let response: HelperResponse = read_frame(&mut stream).await?;
        if response.version != HELPER_PROTOCOL_VERSION || response.request_id != request_id {
            return Err(HelperError::MismatchedResponse);
        }
        Ok(response)
    }
}

pub async fn run_server() -> HelperResult<()> {
    let runtime = Arc::new(HelperRuntime::from_installed()?);
    let watchdog = Arc::clone(&runtime);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            let _ = watchdog.watchdog_tick();
        }
    });

    #[cfg(windows)]
    return run_windows_server(runtime).await;
    #[cfg(target_os = "macos")]
    {
        let shutdown_runtime = Arc::clone(&runtime);
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .map_err(|source| HelperError::io("listen for helper termination", source))?;
        tokio::select! {
            result = run_macos_server(runtime) => result,
            _ = terminate.recv() => shutdown_runtime.shutdown(),
        }
    }
}

async fn serve_stream<S>(mut stream: S, runtime: Arc<HelperRuntime>) -> HelperResult<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request: HelperRequest = read_frame(&mut stream).await?;
    let response = runtime.dispatch(request);
    write_frame(&mut stream, &response).await?;
    stream
        .shutdown()
        .await
        .map_err(|source| HelperError::io("close helper connection", source))?;
    Ok(())
}

async fn read_frame<R, T>(reader: &mut R) -> HelperResult<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut length = [0u8; 4];
    reader
        .read_exact(&mut length)
        .await
        .map_err(|source| HelperError::io("read helper frame length", source))?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(HelperError::FrameTooLarge);
    }
    let mut payload = vec![0u8; length];
    reader
        .read_exact(&mut payload)
        .await
        .map_err(|source| HelperError::io("read helper frame payload", source))?;
    Ok(serde_json::from_slice(&payload)?)
}

async fn write_frame<W, T>(writer: &mut W, value: &T) -> HelperResult<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(value)?;
    if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
        return Err(HelperError::FrameTooLarge);
    }
    writer
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .map_err(|source| HelperError::io("write helper frame length", source))?;
    writer
        .write_all(&payload)
        .await
        .map_err(|source| HelperError::io("write helper frame payload", source))?;
    writer
        .flush()
        .await
        .map_err(|source| HelperError::io("flush helper frame", source))?;
    Ok(())
}

#[cfg(windows)]
async fn connect_windows() -> HelperResult<tokio::net::windows::named_pipe::NamedPipeClient> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let mut last_error = None;
    for _ in 0..20 {
        match ClientOptions::new().open(PIPE_NAME) {
            Ok(client) => {
                verify_windows_pipe_server(&client)?;
                return Ok(client);
            }
            Err(source) => last_error = Some(source),
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(HelperError::io(
        "connect to helper named pipe",
        last_error.unwrap_or_else(|| std::io::Error::other("helper pipe unavailable")),
    ))
}

#[cfg(windows)]
fn verify_windows_pipe_server(
    client: &tokio::net::windows::named_pipe::NamedPipeClient,
) -> HelperResult<()> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, os::windows::io::AsRawHandle};

    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::{
            Pipes::GetNamedPipeServerProcessId,
            Threading::{
                OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
    };
    use windows::core::PWSTR;

    let pipe = HANDLE(client.as_raw_handle());
    let mut pid = 0_u32;
    // SAFETY: `pipe` is borrowed from a live Tokio named-pipe client and the
    // output pointer remains valid for the duration of the call.
    unsafe { GetNamedPipeServerProcessId(pipe, &mut pid) }
        .map_err(|_| HelperError::Unauthorized)?;
    // SAFETY: access is query-only and the returned handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|_| HelperError::Unauthorized)?;

    let verification = (|| -> HelperResult<()> {
        let mut path_buffer = vec![0_u16; 32_768];
        let mut path_length = path_buffer.len() as u32;
        // SAFETY: the process handle is live and `path_buffer` is writable for
        // exactly the length advertised through `path_length`.
        unsafe {
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(path_buffer.as_mut_ptr()),
                &mut path_length,
            )
        }
        .map_err(|_| HelperError::Unauthorized)?;
        path_buffer.truncate(path_length as usize);
        let actual = std::fs::canonicalize(OsString::from_wide(&path_buffer))
            .map_err(|_| HelperError::Unauthorized)?;
        let layout = HelperLayout::installed();
        layout.verify_exact_fixed_file(&layout.helper)?;
        let expected =
            std::fs::canonicalize(&layout.helper).map_err(|_| HelperError::Unauthorized)?;
        if actual != expected {
            return Err(HelperError::Unauthorized);
        }

        let mut token = HANDLE::default();
        // SAFETY: the process handle is live; `token` is an initialized output
        // slot and is closed before this scope returns.
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }
            .map_err(|_| HelperError::Unauthorized)?;
        let token_result = (|| -> HelperResult<()> {
            let mut elevation = TOKEN_ELEVATION::default();
            let mut returned = 0_u32;
            // SAFETY: `elevation` is a correctly sized writable buffer for the
            // requested TOKEN_ELEVATION information class.
            unsafe {
                GetTokenInformation(
                    token,
                    TokenElevation,
                    Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
                    std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                    &mut returned,
                )
            }
            .map_err(|_| HelperError::Unauthorized)?;
            if returned < std::mem::size_of::<TOKEN_ELEVATION>() as u32
                || elevation.TokenIsElevated == 0
            {
                return Err(HelperError::Unauthorized);
            }
            Ok(())
        })();
        // SAFETY: `token` was returned by OpenProcessToken exactly once.
        let _ = unsafe { CloseHandle(token) };
        token_result
    })();

    // SAFETY: `process` was returned by OpenProcess exactly once.
    let _ = unsafe { CloseHandle(process) };
    verification
}

#[cfg(windows)]
async fn run_windows_server(runtime: Arc<HelperRuntime>) -> HelperResult<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let mut first = true;
    loop {
        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(first)
            .reject_remote_clients(true);
        let server = options
            .create(PIPE_NAME)
            .map_err(|source| HelperError::io("create helper named pipe", source))?;
        first = false;
        server
            .connect()
            .await
            .map_err(|source| HelperError::io("accept helper named pipe connection", source))?;
        let runtime = Arc::clone(&runtime);
        tokio::spawn(async move {
            let _ = serve_stream(server, runtime).await;
        });
    }
}

#[cfg(target_os = "macos")]
async fn run_macos_server(runtime: Arc<HelperRuntime>) -> HelperResult<()> {
    use std::{
        fs,
        os::unix::fs::{FileTypeExt, PermissionsExt},
        path::Path,
        process::Command,
    };

    let socket = Path::new(super::SOCKET_PATH);
    let parent = socket
        .parent()
        .ok_or_else(|| HelperError::InvalidRequest("socket path has no parent".to_string()))?;
    fs::create_dir_all(parent)
        .map_err(|source| HelperError::io("create helper socket directory", source))?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o750))
        .map_err(|source| HelperError::io("restrict helper socket directory", source))?;
    chown_via_group(parent)?;

    match fs::symlink_metadata(socket) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(socket)
            .map_err(|source| HelperError::io("remove stale helper socket", source))?,
        Ok(_) => return Err(HelperError::UnsafeFixedPath(socket.to_path_buf())),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(HelperError::io("inspect helper socket", source)),
    }
    let listener = tokio::net::UnixListener::bind(socket)
        .map_err(|source| HelperError::io("bind helper socket", source))?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o660))
        .map_err(|source| HelperError::io("restrict helper socket", source))?;
    chown_via_group(socket)?;

    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|source| HelperError::io("accept helper socket connection", source))?;
        let credentials = stream
            .peer_cred()
            .map_err(|source| HelperError::io("authenticate helper client peer", source))?;
        if credentials.uid() == 0 || credentials.pid().is_none() {
            // Desktop clients must be real, non-root local processes. The
            // socket's root:_via 0660 ACL then limits which user can connect.
            continue;
        }
        let runtime = Arc::clone(&runtime);
        tokio::spawn(async move {
            let _ = serve_stream(stream, runtime).await;
        });
    }

    fn chown_via_group(path: &Path) -> HelperResult<()> {
        let status = Command::new("/usr/sbin/chown")
            .arg("root:_via")
            .arg(path)
            .status()
            .map_err(|source| HelperError::io("set helper IPC group", source))?;
        if !status.success() {
            return Err(HelperError::Core(format!("chown failed with {status}")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn frame_round_trip_is_length_delimited() {
        let (mut left, mut right) = duplex(4096);
        let request = HelperRequest::new("a".repeat(64), HelperOperation::Status);
        let expected_id = request.request_id;
        let writer = tokio::spawn(async move { write_frame(&mut left, &request).await });
        let decoded: HelperRequest = read_frame(&mut right).await.unwrap();
        writer.await.unwrap().unwrap();
        assert_eq!(decoded.request_id, expected_id);
    }

    #[tokio::test]
    async fn oversized_frame_is_rejected_before_allocation() {
        let (mut left, mut right) = duplex(16);
        left.write_all(&((MAX_FRAME_BYTES as u32) + 1).to_be_bytes())
            .await
            .unwrap();
        let result = read_frame::<_, HelperRequest>(&mut right).await;
        assert!(matches!(result, Err(HelperError::FrameTooLarge)));
    }
}
