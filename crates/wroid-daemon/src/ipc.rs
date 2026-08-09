//! Private, versioned IPC for the per-user Wroid runtime daemon.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use wroid_core::profile_v2::ProfileV2;
use wroid_core::Resolution;
use wroid_inject::BRIDGE_WORKER_PROTOCOL_GENERATION;
use wroid_runtime::{DisplayInfo, SessionId, SessionLifecycle, SessionState, StopReason};

use crate::process::ManagedProcesses;
use crate::{DaemonSessionManager, RuntimeSession};

pub const PROTOCOL_VERSION: u32 = 2;
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const SOCKET_FILE: &str = "wroidd.sock";
const LOCK_FILE: &str = "wroidd.lock";
const IO_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolRequest {
    pub protocol_version: u32,
    #[serde(flatten)]
    pub request: DaemonRequest,
}

impl ProtocolRequest {
    pub const fn new(request: DaemonRequest) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            request,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum DaemonRequest {
    Ping,
    PrepareProfileV2 {
        session_id: String,
        profile: ProfileV2,
        width: u32,
        height: u32,
        launch_package: bool,
    },
    LaunchProfileV2 {
        launch: GameLaunchRequest,
    },
    Start {
        session_id: String,
    },
    State {
        session_id: String,
    },
    Stop {
        session_id: String,
        reason: StopReasonWire,
    },
    List,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameLaunchRequest {
    pub session_id: String,
    pub profile_path: PathBuf,
    pub profile: ProfileV2,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mouse: Option<PathBuf>,
    #[serde(default)]
    pub game_mode: bool,
    #[serde(default)]
    pub worker_protocol_generation: u32,
    #[serde(default = "default_true")]
    pub grab: bool,
    #[serde(default = "default_true")]
    pub show_ui: bool,
    #[serde(default = "default_true")]
    pub launch_package: bool,
    #[serde(default)]
    pub trace_input: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_after_millis: Option<u64>,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReasonWire {
    UserRequested,
    FocusLost,
    BackendFailed,
    ClientDisconnected,
    RuntimeShutdown,
}

impl From<StopReasonWire> for StopReason {
    fn from(reason: StopReasonWire) -> Self {
        match reason {
            StopReasonWire::UserRequested => Self::UserRequested,
            StopReasonWire::FocusLost => Self::FocusLost,
            StopReasonWire::BackendFailed => Self::BackendFailed,
            StopReasonWire::ClientDisconnected => Self::ClientDisconnected,
            StopReasonWire::RuntimeShutdown => Self::RuntimeShutdown,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolResponse {
    pub protocol_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<DaemonResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

impl ProtocolResponse {
    fn success(result: DaemonResult) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            result: Some(result),
            error: None,
        }
    }

    fn failure(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            result: None,
            error: Some(ProtocolError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonResult {
    Pong {
        pid: u32,
        session_count: usize,
    },
    Session {
        session: SessionSnapshot,
    },
    Sessions {
        sessions: Vec<SessionSnapshot>,
    },
    Stopped {
        session: SessionSnapshot,
        contacts_cancelled: usize,
        leases_released: usize,
        settings_restored: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub session_id: String,
    pub state: SessionStateWire,
    pub package_name: String,
    pub launch_package: bool,
    pub control_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl SessionSnapshot {
    fn from_session(session: &RuntimeSession) -> Self {
        Self {
            session_id: session.session_id().as_str().to_owned(),
            state: session.state().into(),
            package_name: session.active_package().to_owned(),
            launch_package: session.launch_package(),
            control_count: session.control_plan().map_or(0, |plan| plan.controls.len()),
            process_id: session.process_id(),
            detail: session.detail().map(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStateWire {
    Stopped,
    Preparing,
    Running,
    Stopping,
    Failed,
}

impl From<SessionState> for SessionStateWire {
    fn from(state: SessionState) -> Self {
        match state {
            SessionState::Stopped => Self::Stopped,
            SessionState::Preparing => Self::Preparing,
            SessionState::Running => Self::Running,
            SessionState::Stopping => Self::Stopping,
            SessionState::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtocolError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("XDG_RUNTIME_DIR is unavailable")]
    RuntimeDirectoryUnavailable,
    #[error("Wroid daemon runtime path is unsafe: {0}")]
    UnsafeRuntimePath(String),
    #[error("another wroidd process already owns {0}")]
    AlreadyRunning(String),
    #[error("Wroid daemon I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("Wroid daemon protocol failed: {0}")]
    Protocol(String),
    #[error("Wroid daemon rejected the request ({code}): {message}")]
    Rejected { code: String, message: String },
}

#[derive(Debug, Clone)]
pub struct DaemonPaths {
    pub directory: PathBuf,
    pub socket: PathBuf,
    pub lock: PathBuf,
}

impl DaemonPaths {
    pub fn from_environment() -> Result<Self, IpcError> {
        let runtime = env::var_os("XDG_RUNTIME_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or(IpcError::RuntimeDirectoryUnavailable)?;
        Ok(Self::under(runtime.join("wroid")))
    }

    pub fn under(directory: PathBuf) -> Self {
        Self {
            socket: directory.join(SOCKET_FILE),
            lock: directory.join(LOCK_FILE),
            directory,
        }
    }
}

pub struct DaemonServer {
    listener: UnixListener,
    manager: DaemonSessionManager,
    processes: ManagedProcesses,
    socket_identity: FileIdentity,
    paths: DaemonPaths,
    _lease: File,
    expected_uid: u32,
}

impl DaemonServer {
    pub fn bind_default() -> Result<Self, IpcError> {
        Self::bind(DaemonPaths::from_environment()?)
    }

    pub fn bind(paths: DaemonPaths) -> Result<Self, IpcError> {
        Self::bind_with_processes(paths, ManagedProcesses::new())
    }

    fn bind_with_processes(
        paths: DaemonPaths,
        processes: ManagedProcesses,
    ) -> Result<Self, IpcError> {
        let expected_uid = effective_uid();
        prepare_runtime_directory(&paths.directory, expected_uid)?;
        let lease = acquire_process_lease(&paths.lock)?;
        remove_stale_socket(&paths.socket, expected_uid)?;
        let listener = UnixListener::bind(&paths.socket)?;
        fs::set_permissions(&paths.socket, fs::Permissions::from_mode(0o600))?;
        let socket_identity = FileIdentity::from_metadata(&fs::symlink_metadata(&paths.socket)?);
        Ok(Self {
            listener,
            manager: DaemonSessionManager::new(),
            processes,
            socket_identity,
            paths,
            _lease: lease,
            expected_uid,
        })
    }

    #[cfg(test)]
    fn bind_with_game_log(paths: DaemonPaths, game_log: PathBuf) -> Result<Self, IpcError> {
        Self::bind_with_processes(paths, ManagedProcesses::with_game_log(game_log))
    }

    pub fn serve_once(&mut self) -> Result<(), IpcError> {
        self.reap_managed_processes()?;
        let (mut stream, _) = self.listener.accept()?;
        let response = match peer_credentials(&stream, self.expected_uid) {
            Ok(credentials) => self.read_and_dispatch(&mut stream, credentials),
            Err(error) => ProtocolResponse::failure("unauthorized", error.to_string()),
        };
        write_response(&mut stream, &response)?;
        Ok(())
    }

    pub fn serve_until(&mut self, stop: &AtomicBool) -> Result<(), IpcError> {
        self.listener.set_nonblocking(true)?;
        while !stop.load(Ordering::Relaxed) {
            match self.serve_once() {
                Ok(()) => {}
                Err(IpcError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn read_and_dispatch(
        &mut self,
        stream: &mut UnixStream,
        credentials: PeerCredentials,
    ) -> ProtocolResponse {
        if let Err(error) = stream.set_read_timeout(Some(IO_TIMEOUT)) {
            return ProtocolResponse::failure("io_error", error.to_string());
        }
        let bytes = match read_bounded_message(stream) {
            Ok(bytes) => bytes,
            Err(error) => return ProtocolResponse::failure("invalid_request", error.to_string()),
        };
        let request: ProtocolRequest = match serde_json::from_slice(&bytes) {
            Ok(request) => request,
            Err(error) => {
                return ProtocolResponse::failure(
                    "invalid_request",
                    format!("request is not valid JSON: {error}"),
                )
            }
        };
        if request.protocol_version != PROTOCOL_VERSION {
            return ProtocolResponse::failure(
                "version_mismatch",
                format!(
                    "client protocol {} is incompatible with server protocol {}",
                    request.protocol_version, PROTOCOL_VERSION
                ),
            );
        }
        match dispatch(
            &mut self.manager,
            &mut self.processes,
            request.request,
            credentials,
            self.expected_uid,
        ) {
            Ok(result) => ProtocolResponse::success(result),
            Err(error) => ProtocolResponse::failure("session_error", error),
        }
    }

    fn reap_managed_processes(&mut self) -> Result<(), IpcError> {
        let completed = self
            .processes
            .reap()
            .map_err(|error| IpcError::Protocol(error.to_string()))?;
        for process in completed {
            self.manager
                .mark_exited(&process.session_id, process.success, &process.detail)
                .map_err(|error| IpcError::Protocol(error.to_string()))?;
        }
        Ok(())
    }
}

impl Drop for DaemonServer {
    fn drop(&mut self) {
        let matches = fs::symlink_metadata(&self.paths.socket)
            .ok()
            .is_some_and(|metadata| {
                metadata.file_type().is_socket()
                    && FileIdentity::from_metadata(&metadata) == self.socket_identity
            });
        if matches {
            let _ = fs::remove_file(&self.paths.socket);
        }
    }
}

#[derive(Debug, Clone)]
pub struct DaemonClient {
    socket: PathBuf,
    expected_uid: u32,
}

impl DaemonClient {
    pub fn connect_default() -> Result<Self, IpcError> {
        let paths = DaemonPaths::from_environment()?;
        validate_runtime_directory(&paths.directory, effective_uid())?;
        validate_socket(&paths.socket, effective_uid())?;
        Ok(Self {
            socket: paths.socket,
            expected_uid: effective_uid(),
        })
    }

    pub fn at(socket: PathBuf) -> Self {
        Self {
            socket,
            expected_uid: effective_uid(),
        }
    }

    pub fn request(&self, request: DaemonRequest) -> Result<DaemonResult, IpcError> {
        let mut stream = UnixStream::connect(&self.socket)?;
        peer_credentials(&stream, self.expected_uid)?;
        stream.set_read_timeout(Some(IO_TIMEOUT))?;
        stream.set_write_timeout(Some(IO_TIMEOUT))?;
        serde_json::to_writer(&mut stream, &ProtocolRequest::new(request))
            .map_err(|error| IpcError::Protocol(error.to_string()))?;
        stream.write_all(b"\n")?;
        stream.flush()?;
        let bytes = read_bounded_message(&mut stream)?;
        let response: ProtocolResponse = serde_json::from_slice(&bytes)
            .map_err(|error| IpcError::Protocol(format!("invalid server response: {error}")))?;
        if response.protocol_version != PROTOCOL_VERSION {
            return Err(IpcError::Protocol(format!(
                "server protocol {} is incompatible with client protocol {}",
                response.protocol_version, PROTOCOL_VERSION
            )));
        }
        match (response.result, response.error) {
            (Some(result), None) => Ok(result),
            (None, Some(error)) => Err(IpcError::Rejected {
                code: error.code,
                message: error.message,
            }),
            _ => Err(IpcError::Protocol(
                "server response must contain exactly one result or error".to_owned(),
            )),
        }
    }
}

fn dispatch(
    manager: &mut DaemonSessionManager,
    processes: &mut ManagedProcesses,
    request: DaemonRequest,
    credentials: PeerCredentials,
    expected_uid: u32,
) -> Result<DaemonResult, String> {
    match request {
        DaemonRequest::Ping => Ok(DaemonResult::Pong {
            pid: std::process::id(),
            session_count: manager.session_count(),
        }),
        DaemonRequest::PrepareProfileV2 {
            session_id,
            profile,
            width,
            height,
            launch_package,
        } => {
            let session_id = validated_session_id(session_id)?;
            if !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
                return Err("resolution dimensions must each be between 1 and 8192".to_owned());
            }
            let resolution = Resolution { width, height };
            manager
                .prepare_profile_v2(
                    session_id.clone(),
                    profile,
                    DisplayInfo::new(resolution),
                    launch_package,
                )
                .map_err(|error| error.to_string())?;
            let session = manager
                .session(&session_id)
                .ok_or_else(|| "prepared session disappeared".to_owned())?;
            Ok(DaemonResult::Session {
                session: SessionSnapshot::from_session(session),
            })
        }
        DaemonRequest::LaunchProfileV2 { launch } => {
            let session_id = validated_session_id(launch.session_id.clone())?;
            if launch.worker_protocol_generation != BRIDGE_WORKER_PROTOCOL_GENERATION {
                return Err(format!(
                    "worker protocol generation {} is incompatible with daemon generation {}",
                    launch.worker_protocol_generation, BRIDGE_WORKER_PROTOCOL_GENERATION
                ));
            }
            if !(1..=8192).contains(&launch.width) || !(1..=8192).contains(&launch.height) {
                return Err("resolution dimensions must each be between 1 and 8192".to_owned());
            }
            if launch
                .exit_after_millis
                .is_some_and(|value| !(1..=3_600_000).contains(&value))
            {
                return Err(
                    "diagnostic timeout must be between 1 and 3600000 milliseconds".to_owned(),
                );
            }
            let resolution = Resolution {
                width: launch.width,
                height: launch.height,
            };
            manager
                .prepare_profile_v2(
                    session_id.clone(),
                    launch.profile.clone(),
                    DisplayInfo::new(resolution),
                    launch.launch_package,
                )
                .map_err(|error| error.to_string())?;
            let pid = match processes.launch(
                session_id.clone(),
                &launch,
                credentials.pid,
                expected_uid,
            ) {
                Ok(pid) => pid,
                Err(error) => {
                    let detail = error.to_string();
                    manager
                        .mark_failed(&session_id, &detail)
                        .map_err(|state_error| state_error.to_string())?;
                    return Err(detail);
                }
            };
            manager
                .mark_running(&session_id, pid)
                .map_err(|error| error.to_string())?;
            let session = manager
                .session(&session_id)
                .ok_or_else(|| "launched session disappeared".to_owned())?;
            Ok(DaemonResult::Session {
                session: SessionSnapshot::from_session(session),
            })
        }
        DaemonRequest::Start { session_id } => {
            let session_id = validated_session_id(session_id)?;
            manager
                .start(&session_id)
                .map_err(|error| error.to_string())?;
            let session = manager
                .session(&session_id)
                .ok_or_else(|| "started session disappeared".to_owned())?;
            Ok(DaemonResult::Session {
                session: SessionSnapshot::from_session(session),
            })
        }
        DaemonRequest::State { session_id } => {
            let session_id = validated_session_id(session_id)?;
            manager
                .state(&session_id)
                .map_err(|error| error.to_string())?;
            let session = manager
                .session(&session_id)
                .ok_or_else(|| "session disappeared".to_owned())?;
            Ok(DaemonResult::Session {
                session: SessionSnapshot::from_session(session),
            })
        }
        DaemonRequest::Stop { session_id, reason } => {
            let session_id = validated_session_id(session_id)?;
            if processes
                .request_stop(&session_id)
                .map_err(|error| error.to_string())?
            {
                manager
                    .mark_stopping(&session_id)
                    .map_err(|error| error.to_string())?;
                let session = manager
                    .session(&session_id)
                    .ok_or_else(|| "stopping session disappeared".to_owned())?;
                return Ok(DaemonResult::Stopped {
                    session: SessionSnapshot::from_session(session),
                    contacts_cancelled: 0,
                    leases_released: 0,
                    settings_restored: false,
                });
            }
            let report = manager
                .stop(&session_id, reason.into())
                .map_err(|error| error.to_string())?;
            let session = manager
                .session(&session_id)
                .ok_or_else(|| "stopped session disappeared".to_owned())?;
            Ok(DaemonResult::Stopped {
                session: SessionSnapshot::from_session(session),
                contacts_cancelled: report.contacts_cancelled,
                leases_released: report.leases_released,
                settings_restored: report.settings_restored,
            })
        }
        DaemonRequest::List => Ok(DaemonResult::Sessions {
            sessions: manager
                .sessions()
                .map(SessionSnapshot::from_session)
                .collect(),
        }),
    }
}

fn validated_session_id(value: String) -> Result<SessionId, String> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if !valid {
        return Err(
            "session id must be 1-128 ASCII letters, digits, dots, dashes, or underscores"
                .to_owned(),
        );
    }
    SessionId::new(value).map_err(|error| error.to_string())
}

fn read_bounded_message(stream: &mut UnixStream) -> io::Result<Vec<u8>> {
    let mut reader = BufReader::new(stream);
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take((MAX_MESSAGE_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC message exceeds the 1 MiB limit",
        ));
    }
    if !bytes.ends_with(b"\n") {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "IPC message must end with a newline",
        ));
    }
    bytes.pop();
    Ok(bytes)
}

fn write_response(stream: &mut UnixStream, response: &ProtocolResponse) -> Result<(), IpcError> {
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    serde_json::to_writer(&mut *stream, response)
        .map_err(|error| IpcError::Protocol(error.to_string()))?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

fn prepare_runtime_directory(path: &Path, expected_uid: u32) -> Result<(), IpcError> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != expected_uid
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(IpcError::UnsafeRuntimePath(path.display().to_string()));
    }
    Ok(())
}

fn validate_runtime_directory(path: &Path, expected_uid: u32) -> Result<(), IpcError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != expected_uid
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(IpcError::UnsafeRuntimePath(path.display().to_string()));
    }
    Ok(())
}

fn acquire_process_lease(path: &Path) -> Result<File, IpcError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != effective_uid() || metadata.nlink() != 1 {
        return Err(IpcError::UnsafeRuntimePath(path.display().to_string()));
    }
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    // SAFETY: flock only reads the valid file descriptor and does not retain it.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::WouldBlock {
            return Err(IpcError::AlreadyRunning(path.display().to_string()));
        }
        return Err(IpcError::Io(error));
    }
    file.set_len(0)?;
    writeln!(&file, "pid={}", std::process::id())?;
    file.sync_all()?;
    Ok(file)
}

fn remove_stale_socket(path: &Path, expected_uid: u32) -> Result<(), IpcError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_socket() || metadata.uid() != expected_uid {
        return Err(IpcError::UnsafeRuntimePath(path.display().to_string()));
    }
    fs::remove_file(path)?;
    Ok(())
}

fn validate_socket(path: &Path, expected_uid: u32) -> Result<(), IpcError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != expected_uid
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(IpcError::UnsafeRuntimePath(path.display().to_string()));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct PeerCredentials {
    pid: libc::pid_t,
}

fn peer_credentials(stream: &UnixStream, expected_uid: u32) -> Result<PeerCredentials, IpcError> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: credentials and length are valid writable buffers for SO_PEERCRED.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::addr_of_mut!(credentials).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error().into());
    }
    if length as usize != std::mem::size_of::<libc::ucred>() || credentials.uid != expected_uid {
        return Err(IpcError::Protocol(format!(
            "peer UID {} does not match current UID {expected_uid}",
            credentials.uid
        )));
    }
    if credentials.pid <= 0 {
        return Err(IpcError::Protocol(
            "peer credentials did not include a valid PID".to_owned(),
        ));
    }
    Ok(PeerCredentials {
        pid: credentials.pid,
    })
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid takes no arguments and has no preconditions.
    unsafe { libc::geteuid() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn profile() -> ProfileV2 {
        serde_json::from_str(
            r#"{
                "schema_version": 2,
                "name": "IPC Game",
                "package_name": "com.example.ipc",
                "bindings": [{
                    "name": "fire",
                    "input": {"kind": "key", "key": "f"},
                    "action": {"kind": "tap", "point": {"x": 0.8, "y": 0.5}}
                }]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn protocol_round_trip_manages_one_typed_session() {
        let directory = tempfile::tempdir().unwrap();
        let paths = DaemonPaths::under(directory.path().join("runtime"));
        let socket = paths.socket.clone();
        let mut server = DaemonServer::bind(paths).unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            for _ in 0..5 {
                server.serve_once().unwrap();
            }
        });
        ready_rx.recv().unwrap();
        let client = DaemonClient::at(socket);

        assert!(matches!(
            client.request(DaemonRequest::Ping).unwrap(),
            DaemonResult::Pong {
                session_count: 0,
                ..
            }
        ));
        let prepared = client
            .request(DaemonRequest::PrepareProfileV2 {
                session_id: "ipc-test".to_owned(),
                profile: profile(),
                width: 1600,
                height: 900,
                launch_package: true,
            })
            .unwrap();
        assert!(matches!(
            prepared,
            DaemonResult::Session {
                session: SessionSnapshot {
                    state: SessionStateWire::Preparing,
                    control_count: 1,
                    ..
                }
            }
        ));
        assert!(matches!(
            client
                .request(DaemonRequest::Start {
                    session_id: "ipc-test".to_owned()
                })
                .unwrap(),
            DaemonResult::Session {
                session: SessionSnapshot {
                    state: SessionStateWire::Running,
                    ..
                }
            }
        ));
        assert!(matches!(
            client.request(DaemonRequest::List).unwrap(),
            DaemonResult::Sessions { sessions } if sessions.len() == 1
        ));
        assert!(matches!(
            client
                .request(DaemonRequest::Stop {
                    session_id: "ipc-test".to_owned(),
                    reason: StopReasonWire::UserRequested,
                })
                .unwrap(),
            DaemonResult::Stopped {
                session: SessionSnapshot {
                    state: SessionStateWire::Stopped,
                    ..
                },
                settings_restored: true,
                ..
            }
        ));
        thread.join().unwrap();
    }

    #[test]
    fn rejects_protocol_mismatch_without_mutating_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let paths = DaemonPaths::under(directory.path().join("runtime"));
        let socket = paths.socket.clone();
        let mut server = DaemonServer::bind(paths).unwrap();
        let thread = thread::spawn(move || {
            server.serve_once().unwrap();
            server.serve_once().unwrap();
        });
        let request = ProtocolRequest {
            protocol_version: PROTOCOL_VERSION + 1,
            request: DaemonRequest::Ping,
        };
        let mut stream = UnixStream::connect(&socket).unwrap();
        serde_json::to_writer(&mut stream, &request).unwrap();
        stream.write_all(b"\n").unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        let response: ProtocolResponse = serde_json::from_str(&response).unwrap();

        assert_eq!(
            response.error.unwrap().code,
            "version_mismatch",
            "server must explicitly reject incompatible clients"
        );
        assert!(matches!(
            DaemonClient::at(socket)
                .request(DaemonRequest::Ping)
                .unwrap(),
            DaemonResult::Pong {
                session_count: 0,
                ..
            }
        ));
        thread.join().unwrap();
    }

    #[test]
    fn second_daemon_cannot_replace_the_live_socket() {
        let directory = tempfile::tempdir().unwrap();
        let paths = DaemonPaths::under(directory.path().join("runtime"));
        let server = DaemonServer::bind(paths.clone()).unwrap();

        assert!(matches!(
            DaemonServer::bind(paths),
            Err(IpcError::AlreadyRunning(_))
        ));
        assert!(server.paths.socket.exists());
    }

    #[test]
    fn daemon_lock_rejects_hardlinks() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = directory.path().join("runtime");
        fs::create_dir(&runtime).unwrap();
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        let target = directory.path().join("unrelated");
        fs::write(&target, b"keep").unwrap();
        let paths = DaemonPaths::under(runtime);
        fs::hard_link(&target, &paths.lock).unwrap();

        assert!(matches!(
            DaemonServer::bind(paths),
            Err(IpcError::UnsafeRuntimePath(_))
        ));
        assert_eq!(fs::read(target).unwrap(), b"keep");
    }

    #[test]
    fn session_ids_are_bounded_and_path_independent() {
        assert!(validated_session_id("game-01_test.profile".to_owned()).is_ok());
        assert!(validated_session_id("../escape".to_owned()).is_err());
        assert!(validated_session_id("x".repeat(129)).is_err());
    }

    #[test]
    fn game_launch_request_serializes_only_typed_fields() {
        let request = ProtocolRequest::new(DaemonRequest::LaunchProfileV2 {
            launch: GameLaunchRequest {
                session_id: "hub-42-1".to_owned(),
                profile_path: PathBuf::from("/profiles/pubg-v2.json"),
                profile: profile(),
                width: 1600,
                height: 900,
                keyboard: Some(PathBuf::from("/dev/input/event3")),
                mouse: Some(PathBuf::from("/dev/input/event5")),
                game_mode: true,
                worker_protocol_generation: 1,
                grab: false,
                show_ui: false,
                launch_package: false,
                trace_input: true,
                exit_after_millis: Some(20_000),
            },
        });

        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["method"], "launch_profile_v2");
        assert_eq!(value["params"]["launch"]["width"], 1600);
        assert_eq!(value["params"]["launch"]["gameMode"], true);
        assert_eq!(value["params"]["launch"]["workerProtocolGeneration"], 1);
        assert_eq!(value["params"]["launch"]["grab"], false);
        assert_eq!(value["params"]["launch"]["showUi"], false);
        assert_eq!(value["params"]["launch"]["launchPackage"], false);
        assert_eq!(value["params"]["launch"]["traceInput"], true);
        assert_eq!(value["params"]["launch"]["exitAfterMillis"], 20_000);
        assert!(value["params"]["launch"].get("executable").is_none());
        assert!(value["params"]["launch"].get("arguments").is_none());

        let mut legacy_value = value;
        legacy_value["params"]["launch"]
            .as_object_mut()
            .unwrap()
            .remove("gameMode");
        legacy_value["params"]["launch"]
            .as_object_mut()
            .unwrap()
            .remove("workerProtocolGeneration");
        let legacy: ProtocolRequest = serde_json::from_value(legacy_value).unwrap();
        let DaemonRequest::LaunchProfileV2 { launch } = legacy.request else {
            panic!("unexpected legacy request method");
        };
        assert!(!launch.game_mode);
        assert_eq!(launch.worker_protocol_generation, 0);
    }

    #[test]
    fn launch_rejects_worker_generation_and_timeout_before_session_mutation() {
        let request = |generation, timeout| GameLaunchRequest {
            session_id: "invalid-worker".to_owned(),
            profile_path: PathBuf::from("/path/is/not/used"),
            profile: profile(),
            width: 1600,
            height: 900,
            keyboard: None,
            mouse: None,
            game_mode: false,
            worker_protocol_generation: generation,
            grab: true,
            show_ui: true,
            launch_package: false,
            trace_input: true,
            exit_after_millis: timeout,
        };
        let credentials = PeerCredentials {
            pid: std::process::id() as libc::pid_t,
        };

        for (launch, expected) in [
            (request(0, Some(20_000)), "worker protocol generation"),
            (
                request(BRIDGE_WORKER_PROTOCOL_GENERATION, Some(0)),
                "diagnostic timeout",
            ),
            (
                request(BRIDGE_WORKER_PROTOCOL_GENERATION, Some(3_600_001)),
                "diagnostic timeout",
            ),
        ] {
            let mut manager = DaemonSessionManager::new();
            let mut processes = ManagedProcesses::new();
            let error = dispatch(
                &mut manager,
                &mut processes,
                DaemonRequest::LaunchProfileV2 { launch },
                credentials,
                effective_uid(),
            )
            .unwrap_err();
            assert!(error.contains(expected));
            assert_eq!(manager.session_count(), 0);
        }
    }

    #[test]
    fn protocol_launch_is_owned_and_reaped_by_the_daemon() {
        let directory = tempfile::tempdir().unwrap();
        let profile_path = directory.path().join("profile.json");
        let selected_profile = profile();
        fs::write(
            &profile_path,
            serde_json::to_vec(&selected_profile).unwrap(),
        )
        .unwrap();
        let paths = DaemonPaths::under(directory.path().join("runtime"));
        let socket = paths.socket.clone();
        let game_log = directory.path().join("state/wroid/game-session.log");
        let mut server = DaemonServer::bind_with_game_log(paths, game_log.clone()).unwrap();
        let thread = thread::spawn(move || {
            for _ in 0..21 {
                server.serve_once().unwrap();
            }
        });
        let client = DaemonClient::at(socket);

        let launched = client
            .request(DaemonRequest::LaunchProfileV2 {
                launch: GameLaunchRequest {
                    session_id: "owned-launch".to_owned(),
                    profile_path,
                    profile: selected_profile,
                    width: 1600,
                    height: 900,
                    keyboard: None,
                    mouse: None,
                    game_mode: false,
                    worker_protocol_generation: BRIDGE_WORKER_PROTOCOL_GENERATION,
                    grab: true,
                    show_ui: true,
                    launch_package: true,
                    trace_input: false,
                    exit_after_millis: None,
                },
            })
            .unwrap();
        assert!(matches!(
            launched,
            DaemonResult::Session {
                session: SessionSnapshot {
                    state: SessionStateWire::Running,
                    process_id: Some(_),
                    ..
                }
            }
        ));

        let mut last_state = SessionStateWire::Running;
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(10));
            let DaemonResult::Sessions { sessions } = client.request(DaemonRequest::List).unwrap()
            else {
                panic!("unexpected list response");
            };
            last_state = sessions[0].state;
        }
        thread.join().unwrap();

        assert_eq!(last_state, SessionStateWire::Failed);
        assert!(game_log.exists());
    }
}
