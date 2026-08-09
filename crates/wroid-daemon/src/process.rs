use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use thiserror::Error;
use wroid_core::Resolution;
use wroid_inject::{
    runtime_socket_pair, RuntimeAttachmentReport, RuntimeChannelServer, RUNTIME_WORKER_FD,
    RUNTIME_WORKER_PROTOCOL_GENERATION,
};
use wroid_runtime::SessionId;

use crate::ipc::GameLaunchRequest;
#[cfg(test)]
use crate::platform::RuntimePlatformBackend;
use crate::platform::{PersistentPlatform, PlatformAttachment, PlatformLaunch};
use crate::production_platform::ProductionRuntimePlatform;

const GAME_LOG_FILE: &str = "game-session.log";
const GAME_MODE_WRAPPER_PATHS: [&str; 2] = ["/usr/local/bin/gamemoderun", "/usr/bin/gamemoderun"];

#[derive(Debug, Error)]
pub(crate) enum ProcessError {
    #[error("another daemon-managed game session is already active")]
    AlreadyActive,
    #[error("unsafe game launch request: {0}")]
    UnsafeRequest(String),
    #[error("game process I/O failed: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug)]
pub(crate) struct ReapedProcess {
    pub(crate) session_id: SessionId,
    pub(crate) success: bool,
    pub(crate) detail: String,
}

#[derive(Debug, PartialEq, Eq)]
struct LaunchProgram {
    executable: PathBuf,
    arguments: Vec<OsString>,
    game_mode_active: bool,
}

struct ManagedProcess {
    child: Child,
    attachment: Option<PlatformAttachment>,
    stop_requested: bool,
}

pub(crate) struct ManagedProcesses {
    children: BTreeMap<SessionId, ManagedProcess>,
    game_log_override: Option<PathBuf>,
    platform: Option<PersistentPlatform>,
}

impl ManagedProcesses {
    pub(crate) fn new() -> Self {
        Self {
            children: BTreeMap::new(),
            game_log_override: None,
            platform: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_game_log(path: PathBuf) -> Self {
        Self::with_game_log_and_platform(path, no_op_test_platform())
    }

    #[cfg(test)]
    pub(crate) fn with_platform(platform: PersistentPlatform) -> Self {
        Self {
            children: BTreeMap::new(),
            game_log_override: None,
            platform: Some(platform),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_game_log_and_platform(path: PathBuf, platform: PersistentPlatform) -> Self {
        Self {
            children: BTreeMap::new(),
            game_log_override: Some(path),
            platform: Some(platform),
        }
    }

    pub(crate) fn launch(
        &mut self,
        session_id: SessionId,
        request: &GameLaunchRequest,
        peer_pid: libc::pid_t,
        expected_uid: u32,
    ) -> Result<u32, ProcessError> {
        if !self.children.is_empty() {
            return Err(ProcessError::AlreadyActive);
        }
        if request.worker_protocol_generation != RUNTIME_WORKER_PROTOCOL_GENERATION {
            return Err(ProcessError::UnsafeRequest(format!(
                "worker protocol generation {} does not match {}",
                request.worker_protocol_generation, RUNTIME_WORKER_PROTOCOL_GENERATION
            )));
        }
        let profile_path = validated_profile_path(&request.profile_path, expected_uid)?;
        validate_input_path(request.keyboard.as_deref(), "keyboard")?;
        validate_input_path(request.mouse.as_deref(), "mouse")?;

        let executable = peer_executable(peer_pid, expected_uid)?;
        self.launch_validated_worker(
            session_id,
            request,
            &profile_path,
            &executable,
            expected_uid,
        )
    }

    #[cfg(test)]
    pub(crate) fn launch_with_worker_for_test(
        &mut self,
        session_id: SessionId,
        request: &GameLaunchRequest,
        executable: &Path,
        expected_uid: u32,
    ) -> Result<u32, ProcessError> {
        if !self.children.is_empty() {
            return Err(ProcessError::AlreadyActive);
        }
        if request.worker_protocol_generation != RUNTIME_WORKER_PROTOCOL_GENERATION {
            return Err(ProcessError::UnsafeRequest(format!(
                "worker protocol generation {} does not match {}",
                request.worker_protocol_generation, RUNTIME_WORKER_PROTOCOL_GENERATION
            )));
        }
        let profile_path = validated_profile_path(&request.profile_path, expected_uid)?;
        validate_input_path(request.keyboard.as_deref(), "keyboard")?;
        validate_input_path(request.mouse.as_deref(), "mouse")?;
        self.launch_validated_worker(session_id, request, &profile_path, executable, expected_uid)
    }

    fn launch_validated_worker(
        &mut self,
        session_id: SessionId,
        request: &GameLaunchRequest,
        profile_path: &Path,
        executable: &Path,
        expected_uid: u32,
    ) -> Result<u32, ProcessError> {
        let (worker_socket, daemon_socket) = runtime_socket_pair()?;
        let arguments = launch_arguments(profile_path, request, std::process::id());
        let program = launch_program(
            executable,
            arguments,
            request.game_mode,
            trusted_game_mode_wrapper().as_deref(),
        );
        let log_path = match &self.game_log_override {
            Some(path) => path.clone(),
            None => game_log_path()?,
        };
        let log = open_private_game_log(&log_path, expected_uid)?;
        let stderr = log.try_clone()?;
        let mut command = command_for_program(&program);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr))
            .process_group(0);
        configure_worker_child(
            &mut command,
            worker_socket.as_raw_fd(),
            std::process::id() as libc::pid_t,
        )?;
        let child = match command.spawn() {
            Ok(child) => child,
            Err(spawn_error) => return Err(ProcessError::Io(spawn_error)),
        };
        drop(worker_socket);
        self.attach_spawned_worker(session_id, request, child, daemon_socket, expected_uid)
    }

    fn attach_spawned_worker(
        &mut self,
        session_id: SessionId,
        request: &GameLaunchRequest,
        mut child: Child,
        daemon_socket: std::os::fd::OwnedFd,
        expected_uid: u32,
    ) -> Result<u32, ProcessError> {
        let channel = match RuntimeChannelServer::from_owned_fd(daemon_socket) {
            Ok(channel) => channel,
            Err(error) => {
                return Err(launch_error_after_spawn(
                    "runtime channel",
                    error,
                    &mut child,
                ))
            }
        };
        let launch = platform_launch(request);
        let attachment = match self
            .platform
            .get_or_insert_with(|| production_platform(expected_uid))
            .attach(channel, launch)
        {
            Ok(attachment) => attachment,
            Err(error) => {
                return Err(launch_error_after_spawn(
                    "runtime platform attachment",
                    error,
                    &mut child,
                ));
            }
        };
        let pid = child.id();
        self.children.insert(
            session_id,
            ManagedProcess {
                child,
                attachment: Some(attachment),
                stop_requested: false,
            },
        );
        Ok(pid)
    }

    pub(crate) fn request_stop(&mut self, session_id: &SessionId) -> Result<bool, ProcessError> {
        let Some(process) = self.children.get_mut(session_id) else {
            return Ok(false);
        };
        let pid = i32::try_from(process.child.id())
            .map_err(|_| ProcessError::UnsafeRequest("child PID is out of range".to_owned()))?;
        // SAFETY: the Child handle is still owned and unreaped, so this PID
        // cannot have been recycled for an unrelated process.
        let result = unsafe { libc::kill(pid, libc::SIGTERM) };
        if result != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error.into());
            }
        }
        process.stop_requested = true;
        Ok(true)
    }

    pub(crate) fn reap(&mut self) -> Result<Vec<ReapedProcess>, ProcessError> {
        let mut completed = Vec::new();
        let mut exited = Vec::new();
        for (session_id, process) in &mut self.children {
            if let Some(status) = process.child.try_wait()? {
                exited.push((session_id.clone(), status));
            }
        }
        for (session_id, status) in exited {
            let mut process = self
                .children
                .remove(&session_id)
                .expect("exited managed process remains owned");
            let attachment_result = finish_attachment(process.attachment.take());
            let (success, detail) =
                combine_reaped_detail(status, attachment_result, process.stop_requested);
            completed.push(ReapedProcess {
                session_id,
                success,
                detail,
            });
        }
        Ok(completed)
    }
}

fn launch_program(
    worker: &Path,
    arguments: Vec<OsString>,
    game_mode_requested: bool,
    game_mode_wrapper: Option<&Path>,
) -> LaunchProgram {
    if game_mode_requested {
        if let Some(wrapper) = game_mode_wrapper {
            let mut wrapped_arguments = Vec::with_capacity(arguments.len() + 1);
            wrapped_arguments.push(worker.as_os_str().to_owned());
            wrapped_arguments.extend(arguments);
            return LaunchProgram {
                executable: wrapper.to_path_buf(),
                arguments: wrapped_arguments,
                game_mode_active: true,
            };
        }
    }
    LaunchProgram {
        executable: worker.to_path_buf(),
        arguments,
        game_mode_active: false,
    }
}

fn command_for_program(program: &LaunchProgram) -> Command {
    let mut command = Command::new(&program.executable);
    command
        .args(&program.arguments)
        .env_remove("GAMEMODERUNEXEC")
        .env_remove("LD_PRELOAD");
    command
}

fn trusted_game_mode_wrapper() -> Option<PathBuf> {
    GAME_MODE_WRAPPER_PATHS
        .iter()
        .map(Path::new)
        .find(|path| validate_game_mode_wrapper(path, 0, Path::new("/")).is_ok())
        .map(Path::to_path_buf)
}

fn validate_game_mode_wrapper(
    path: &Path,
    expected_uid: u32,
    trust_anchor: &Path,
) -> Result<(), ProcessError> {
    let canonical = path.canonicalize()?;
    let metadata = fs::symlink_metadata(path)?;
    let anchor = trust_anchor.canonicalize()?;
    if canonical != path
        || !path.starts_with(&anchor)
        || !metadata.is_file()
        || metadata.uid() != expected_uid
        || metadata.permissions().mode() & 0o111 == 0
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(ProcessError::UnsafeRequest(
            "GameMode wrapper is not a protected canonical executable".to_owned(),
        ));
    }
    let mut directory = path.parent();
    while let Some(parent) = directory {
        let metadata = fs::symlink_metadata(parent)?;
        if !metadata.is_dir()
            || metadata.uid() != expected_uid
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(ProcessError::UnsafeRequest(
                "GameMode wrapper directory chain is not protected".to_owned(),
            ));
        }
        if parent == anchor {
            return Ok(());
        }
        directory = parent.parent();
    }
    Err(ProcessError::UnsafeRequest(
        "GameMode wrapper is outside its trust anchor".to_owned(),
    ))
}

impl Drop for ManagedProcesses {
    fn drop(&mut self) {
        for process in self.children.values() {
            if let Ok(pid) = i32::try_from(process.child.id()) {
                // SAFETY: each Child is still owned and unreaped here.
                let _ = unsafe { libc::kill(pid, libc::SIGTERM) };
            }
        }
        let children = std::mem::take(&mut self.children);
        for (_, mut process) in children {
            let _ = wait_for_terminated_child(&mut process.child);
            let _ = finish_attachment(process.attachment.take());
        }
        drop(self.platform.take());
    }
}

fn production_platform(expected_uid: u32) -> PersistentPlatform {
    PersistentPlatform::with_factory(Arc::new(move || {
        Ok(Box::new(ProductionRuntimePlatform::new(expected_uid)))
    }))
}

#[cfg(test)]
fn no_op_test_platform() -> PersistentPlatform {
    PersistentPlatform::with_factory(Arc::new(|| Ok(Box::new(NoopTestPlatformBackend))))
}

#[cfg(test)]
struct NoopTestPlatformBackend;

#[cfg(test)]
impl RuntimePlatformBackend for NoopTestPlatformBackend {
    fn prepare(&mut self, _launch: &PlatformLaunch) -> io::Result<()> {
        Ok(())
    }

    fn serve(
        &mut self,
        _channel: RuntimeChannelServer,
        _resolution: Resolution,
    ) -> io::Result<RuntimeAttachmentReport> {
        Ok(empty_attachment_report())
    }

    fn shutdown(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn finish_attachment(
    attachment: Option<PlatformAttachment>,
) -> io::Result<RuntimeAttachmentReport> {
    match attachment {
        Some(attachment) => attachment.finish(),
        None => Ok(empty_attachment_report()),
    }
}

const fn empty_attachment_report() -> RuntimeAttachmentReport {
    RuntimeAttachmentReport {
        frames_submitted: 0,
        peak_contacts: 0,
        contacts_cancelled: 0,
    }
}

fn launch_error_after_spawn(context: &str, error: io::Error, child: &mut Child) -> ProcessError {
    let kind = error.kind();
    let detail = match terminate_and_reap_child(child) {
        Ok(status) => format!("{context} failed: {error}; worker reaped with {status}"),
        Err(reap_error) => {
            format!("{context} failed: {error}; worker reap also failed: {reap_error}")
        }
    };
    ProcessError::Io(io::Error::new(kind, detail))
}

fn terminate_and_reap_child(child: &mut Child) -> io::Result<std::process::ExitStatus> {
    if child.try_wait()?.is_none() {
        if let Ok(pid) = i32::try_from(child.id()) {
            // SAFETY: the exact unreaped Child is still owned here.
            let result = unsafe { libc::kill(pid, libc::SIGTERM) };
            if result != 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(error);
                }
            }
        }
    }
    wait_for_terminated_child(child)
}

fn wait_for_terminated_child(child: &mut Child) -> io::Result<std::process::ExitStatus> {
    for _ in 0..100 {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => return Err(error),
        }
    }
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: the exact unreaped Child is still owned here.
        let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    child.wait()
}

pub(crate) fn launch_arguments(
    profile_path: &Path,
    request: &GameLaunchRequest,
    daemon_pid: u32,
) -> Vec<OsString> {
    let mut arguments = vec![
        OsString::from("launch-v2"),
        profile_path.as_os_str().to_owned(),
        OsString::from("--daemon-worker"),
        OsString::from("--runtime-fd"),
        OsString::from(RUNTIME_WORKER_FD.to_string()),
        OsString::from("--daemon-parent-pid"),
        OsString::from(daemon_pid.to_string()),
        OsString::from("--width"),
        OsString::from(request.width.to_string()),
        OsString::from("--height"),
        OsString::from(request.height.to_string()),
    ];
    if let Some(path) = request.keyboard.as_deref() {
        arguments.push(OsString::from("--keyboard"));
        arguments.push(path.as_os_str().to_owned());
    }
    if let Some(path) = request.mouse.as_deref() {
        arguments.push(OsString::from("--mouse"));
        arguments.push(path.as_os_str().to_owned());
    }
    if !request.grab {
        arguments.push(OsString::from("--no-grab"));
    }
    if !request.show_ui {
        arguments.push(OsString::from("--no-ui"));
    }
    if !request.launch_package {
        arguments.push(OsString::from("--no-launch"));
    }
    if request.trace_input {
        arguments.push(OsString::from("--trace-input"));
    }
    if let Some(milliseconds) = request.exit_after_millis {
        arguments.push(OsString::from("--exit-after-ms"));
        arguments.push(OsString::from(milliseconds.to_string()));
    }
    arguments
}

fn configure_worker_child(
    command: &mut Command,
    source_fd: libc::c_int,
    daemon_pid: libc::pid_t,
) -> io::Result<()> {
    // SAFETY: the closure uses only async-signal-safe Linux syscalls between
    // fork and exec and captures plain integer descriptors/PIDs.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::getppid() != daemon_pid {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Wroid daemon parent changed before worker exec",
                ));
            }
            if source_fd == RUNTIME_WORKER_FD {
                let flags = libc::fcntl(source_fd, libc::F_GETFD);
                if flags < 0
                    || libc::fcntl(source_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) != 0
                {
                    return Err(io::Error::last_os_error());
                }
            } else {
                if libc::dup3(source_fd, RUNTIME_WORKER_FD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                libc::close(source_fd);
            }
            if libc::syscall(
                libc::SYS_close_range,
                libc::STDERR_FILENO + 1,
                RUNTIME_WORKER_FD - 1,
                libc::CLOSE_RANGE_CLOEXEC,
            ) != 0
                || libc::syscall(
                    libc::SYS_close_range,
                    RUNTIME_WORKER_FD + 1,
                    u32::MAX,
                    libc::CLOSE_RANGE_CLOEXEC,
                ) != 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

fn platform_launch(request: &GameLaunchRequest) -> PlatformLaunch {
    PlatformLaunch {
        package_name: request.profile.package_name.clone(),
        resolution: Resolution {
            width: request.width,
            height: request.height,
        },
        show_ui: request.show_ui,
        launch_package: request.launch_package,
    }
}

fn combine_reaped_detail(
    status: std::process::ExitStatus,
    attachment_result: io::Result<RuntimeAttachmentReport>,
    stop_requested: bool,
) -> (bool, String) {
    let detail = match status.signal() {
        Some(signal) => format!("game worker terminated by signal {signal}"),
        None => format!("game worker exited with {status}"),
    };
    let expected_stop = stop_requested && status.signal() == Some(libc::SIGTERM);
    match attachment_result {
        Ok(report) => (
            status.success() || expected_stop,
            format!(
                "{detail}; runtime attachment finished: {} frame(s), peak {} contact(s), cancelled {} contact(s)",
                report.frames_submitted, report.peak_contacts, report.contacts_cancelled
            ),
        ),
        Err(error) => (
            false,
            format!("{detail}; runtime attachment failed: {error}"),
        ),
    }
}

fn peer_executable(peer_pid: libc::pid_t, expected_uid: u32) -> Result<PathBuf, ProcessError> {
    if peer_pid <= 0 {
        return Err(ProcessError::UnsafeRequest(
            "peer PID is unavailable".to_owned(),
        ));
    }
    let proc_executable = PathBuf::from(format!("/proc/{peer_pid}/exe"));
    let executable = fs::read_link(&proc_executable)?;
    let metadata = fs::metadata(&proc_executable)?;
    if !metadata.is_file()
        || metadata.uid() != expected_uid
        || metadata.permissions().mode() & 0o111 == 0
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(ProcessError::UnsafeRequest(format!(
            "peer executable is not a protected current-user executable: {}",
            executable.display()
        )));
    }
    Ok(executable)
}

fn validated_profile_path(path: &Path, expected_uid: u32) -> Result<PathBuf, ProcessError> {
    if !path.is_absolute() {
        return Err(ProcessError::UnsafeRequest(
            "profile path must be absolute".to_owned(),
        ));
    }
    let canonical = path.canonicalize()?;
    let metadata = fs::metadata(&canonical)?;
    if canonical != path || !metadata.is_file() || metadata.uid() != expected_uid {
        return Err(ProcessError::UnsafeRequest(
            "profile must be a canonical current-user regular file".to_owned(),
        ));
    }
    Ok(canonical)
}

fn validate_input_path(path: Option<&Path>, label: &str) -> Result<(), ProcessError> {
    let Some(path) = path else {
        return Ok(());
    };
    let clean = path.is_absolute()
        && path.starts_with("/dev/input")
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir));
    if !clean {
        return Err(ProcessError::UnsafeRequest(format!(
            "{label} path must be an absolute /dev/input path"
        )));
    }
    Ok(())
}

fn open_private_game_log(path: &Path, expected_uid: u32) -> Result<File, ProcessError> {
    let directory = path.parent().ok_or_else(|| {
        ProcessError::UnsafeRequest("game log path has no parent directory".to_owned())
    })?;
    fs::create_dir_all(directory)?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    let directory_metadata = fs::symlink_metadata(directory)?;
    if !directory_metadata.is_dir()
        || directory_metadata.uid() != expected_uid
        || directory_metadata.permissions().mode() & 0o077 != 0
    {
        return Err(ProcessError::UnsafeRequest(
            "game log directory is not private".to_owned(),
        ));
    }
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != expected_uid || metadata.nlink() != 1 {
        return Err(ProcessError::UnsafeRequest(
            "game log is not a private current-user file".to_owned(),
        ));
    }
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.set_len(0)?;
    Ok(file)
}

fn game_log_path() -> Result<PathBuf, ProcessError> {
    let state_home = env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("state"))
        })
        .ok_or_else(|| {
            ProcessError::UnsafeRequest(
                "HOME and XDG_STATE_HOME are unavailable for the game log".to_owned(),
            )
        })?;
    Ok(state_home.join("wroid").join(GAME_LOG_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::fd::AsRawFd;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use wroid_core::Resolution;
    use wroid_inject::{
        runtime_socket_pair, RuntimeAttachmentReport, RuntimeChannelServer,
        RUNTIME_WORKER_PROTOCOL_GENERATION,
    };

    use crate::platform::{PersistentPlatform, PlatformLaunch, RuntimePlatformBackend};

    type PlatformCalls = Arc<Mutex<Vec<String>>>;
    static PANIC_HOOK_LOCK: Mutex<()> = Mutex::new(());

    struct RecordingBackend {
        calls: PlatformCalls,
        serve_error: Option<&'static str>,
    }

    impl RuntimePlatformBackend for RecordingBackend {
        fn prepare(&mut self, launch: &PlatformLaunch) -> io::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("prepare:{}", launch.package_name));
            Ok(())
        }

        fn serve(
            &mut self,
            _channel: RuntimeChannelServer,
            _resolution: Resolution,
        ) -> io::Result<RuntimeAttachmentReport> {
            self.calls.lock().unwrap().push("serve".to_owned());
            if let Some(error) = self.serve_error.take() {
                return Err(io::Error::other(error));
            }
            Ok(RuntimeAttachmentReport {
                frames_submitted: 7,
                peak_contacts: 3,
                contacts_cancelled: 2,
            })
        }

        fn shutdown(&mut self) -> io::Result<()> {
            self.calls.lock().unwrap().push("shutdown".to_owned());
            Ok(())
        }
    }

    fn shared_platform_calls() -> PlatformCalls {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn fake_platform(calls: PlatformCalls) -> PersistentPlatform {
        fake_platform_with_serve_error(calls, None)
    }

    fn fake_platform_with_serve_error(
        calls: PlatformCalls,
        serve_error: Option<&'static str>,
    ) -> PersistentPlatform {
        let factory: Arc<
            dyn Fn() -> io::Result<Box<dyn RuntimePlatformBackend>> + Send + Sync + 'static,
        > = Arc::new(move || {
            calls.lock().unwrap().push("factory".to_owned());
            Ok(Box::new(RecordingBackend {
                calls: calls.clone(),
                serve_error,
            }))
        });
        PersistentPlatform::with_factory(factory)
    }

    fn dead_platform() -> PersistentPlatform {
        let _guard = PANIC_HOOK_LOCK.lock().unwrap();
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let factory: Arc<
            dyn Fn() -> io::Result<Box<dyn RuntimePlatformBackend>> + Send + Sync + 'static,
        > = Arc::new(|| panic!("test platform thread stopped"));
        let platform = PersistentPlatform::with_factory(factory);
        let attachment = platform
            .attach(runtime_server(), platform_launch("com.example.dead"))
            .unwrap();
        let _ = attachment.finish();
        std::panic::set_hook(previous_hook);
        platform
    }

    fn platform_launch(package_name: &str) -> PlatformLaunch {
        PlatformLaunch {
            package_name: package_name.to_owned(),
            resolution: Resolution {
                width: 1600,
                height: 900,
            },
            show_ui: true,
            launch_package: true,
        }
    }

    fn runtime_server() -> RuntimeChannelServer {
        let (_client, server) = runtime_socket_pair().unwrap();
        RuntimeChannelServer::from_owned_fd(server).unwrap()
    }

    fn launch_request() -> GameLaunchRequest {
        GameLaunchRequest {
            session_id: "process-test".to_owned(),
            profile_path: PathBuf::from("/unused/profile.json"),
            profile: serde_json::from_str(
                r#"{
                    "schema_version": 2,
                    "name": "Process Test",
                    "package_name": "com.example.process",
                    "bindings": []
                }"#,
            )
            .unwrap(),
            width: 1600,
            height: 900,
            keyboard: None,
            mouse: None,
            game_mode: false,
            worker_protocol_generation: RUNTIME_WORKER_PROTOCOL_GENERATION,
            grab: true,
            show_ui: true,
            launch_package: true,
            trace_input: false,
            exit_after_millis: None,
        }
    }

    fn test_process(child: Child) -> ManagedProcess {
        ManagedProcess {
            child,
            attachment: None,
            stop_requested: false,
        }
    }

    fn write_profile(directory: &Path) -> PathBuf {
        let profile_path = directory.join("profile.json");
        fs::write(
            &profile_path,
            serde_json::to_vec(&launch_request().profile).unwrap(),
        )
        .unwrap();
        profile_path
    }

    fn request_for_profile(profile_path: &Path, session_id: &str) -> GameLaunchRequest {
        let mut request = launch_request();
        request.session_id = session_id.to_owned();
        request.profile_path = profile_path.to_path_buf();
        request
    }

    fn sleeping_worker(directory: &Path) -> PathBuf {
        let worker = directory.join("sleep-worker.sh");
        fs::write(&worker, b"#!/bin/sh\nexec sleep 30\n").unwrap();
        fs::set_permissions(&worker, fs::Permissions::from_mode(0o755)).unwrap();
        worker
    }

    #[test]
    fn worker_arguments_carry_only_runtime_capability() {
        let mut request = launch_request();
        request.keyboard = Some(PathBuf::from("/dev/input/event3"));
        request.mouse = Some(PathBuf::from("/dev/input/event5"));
        request.grab = false;
        request.show_ui = false;
        request.launch_package = false;
        request.trace_input = true;
        request.exit_after_millis = Some(20_000);
        assert_eq!(
            launch_arguments(Path::new("/profiles/pubg-v2.json"), &request, 4242,),
            [
                "launch-v2",
                "/profiles/pubg-v2.json",
                "--daemon-worker",
                "--runtime-fd",
                "198",
                "--daemon-parent-pid",
                "4242",
                "--width",
                "1600",
                "--height",
                "900",
                "--keyboard",
                "/dev/input/event3",
                "--mouse",
                "/dev/input/event5",
                "--no-grab",
                "--no-ui",
                "--no-launch",
                "--trace-input",
                "--exit-after-ms",
                "20000",
            ]
            .map(OsString::from)
        );
        assert!(
            launch_arguments(Path::new("/profiles/pubg-v2.json"), &request, 4242)
                .windows(2)
                .any(|pair| pair == ["--runtime-fd", "198"].map(OsString::from))
        );
        assert!(
            !launch_arguments(Path::new("/profiles/pubg-v2.json"), &request, 4242)
                .iter()
                .any(|arg| arg == "--bridge-fd")
        );
    }

    #[test]
    fn worker_arguments_never_contain_a_helper_path() {
        let arguments = launch_arguments(Path::new("/profiles/game.json"), &launch_request(), 7);
        assert!(!arguments
            .iter()
            .any(|argument| argument.to_string_lossy().contains("wroid-helper")));
    }

    #[test]
    fn configured_worker_inherits_only_the_fixed_runtime_socket() {
        let (worker_socket, _daemon_socket) = runtime_socket_pair().unwrap();
        let (leaked_socket, leaked_peer) = runtime_socket_pair().unwrap();
        let leaked_fd = leaked_socket.as_raw_fd();
        // SAFETY: the valid test descriptor is deliberately made inheritable
        // to prove worker setup closes capabilities it did not select.
        let flags = unsafe { libc::fcntl(leaked_fd, libc::F_GETFD) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(leaked_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) },
            0
        );
        let mut command = Command::new("/usr/bin/sh");
        command.args([
            "-c",
            &format!("test -S /proc/self/fd/198 && test ! -e /proc/self/fd/{leaked_fd}"),
        ]);
        configure_worker_child(
            &mut command,
            worker_socket.as_raw_fd(),
            std::process::id() as libc::pid_t,
        )
        .unwrap();

        let status = command.status().unwrap();

        assert!(status.success());
        drop(worker_socket);
        drop(leaked_socket);
        drop(leaked_peer);
    }

    #[test]
    fn reaped_detail_preserves_worker_and_attachment_failures() {
        let status = Command::new("/usr/bin/sh")
            .args(["-c", "exit 7"])
            .status()
            .unwrap();
        let (success, detail) = combine_reaped_detail(
            status,
            Err(io::Error::other("runtime attachment cleanup failed")),
            false,
        );

        assert!(!success);
        assert!(detail.contains("exit status: 7"));
        assert!(detail.contains("runtime attachment cleanup failed"));
    }

    #[test]
    fn managed_reaper_finishes_the_owned_runtime_attachment() {
        let session_id = SessionId::new("attachment-reap").unwrap();
        let child = Command::new("/usr/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        let calls = shared_platform_calls();
        let platform = fake_platform_with_serve_error(calls, Some("owned attachment failed"));
        let attachment = platform
            .attach(runtime_server(), platform_launch("com.example.reap"))
            .unwrap();
        let mut processes = ManagedProcesses::new();
        processes.children.insert(
            session_id.clone(),
            ManagedProcess {
                child,
                attachment: Some(attachment),
                stop_requested: false,
            },
        );

        let completed = (0..100)
            .find_map(|_| {
                let completed = processes.reap().unwrap();
                if completed.is_empty() {
                    thread::sleep(Duration::from_millis(5));
                    None
                } else {
                    Some(completed)
                }
            })
            .expect("short-lived managed worker was not reaped");

        assert_eq!(completed[0].session_id, session_id);
        assert!(!completed[0].success);
        assert!(completed[0].detail.contains("owned attachment failed"));
        drop(platform);
    }

    #[test]
    fn stale_worker_generations_are_rejected_before_spawn_or_platform_attach() {
        let calls = shared_platform_calls();
        let mut processes = ManagedProcesses::with_platform(fake_platform(calls.clone()));

        for generation in [0, 1] {
            let mut request = launch_request();
            request.worker_protocol_generation = generation;
            let error = processes
                .launch(
                    SessionId::new(format!("stale-generation-{generation}")).unwrap(),
                    &request,
                    std::process::id() as libc::pid_t,
                    // SAFETY: geteuid takes no arguments and has no preconditions.
                    unsafe { libc::geteuid() },
                )
                .unwrap_err();

            assert!(matches!(error, ProcessError::UnsafeRequest(_)));
        }

        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn spawn_failure_does_not_initialize_platform() {
        let directory = tempfile::tempdir().unwrap();
        let profile_path = write_profile(directory.path());
        let calls = shared_platform_calls();
        let mut processes = ManagedProcesses::with_game_log_and_platform(
            directory.path().join("state/wroid/game-session.log"),
            fake_platform(calls.clone()),
        );

        let error = processes
            .launch_with_worker_for_test(
                SessionId::new("missing-worker").unwrap(),
                &request_for_profile(&profile_path, "missing-worker"),
                &directory.path().join("missing-worker"),
                // SAFETY: geteuid takes no arguments and has no preconditions.
                unsafe { libc::geteuid() },
            )
            .unwrap_err();

        assert!(matches!(error, ProcessError::Io(_)));
        assert!(calls.lock().unwrap().is_empty());
        assert!(processes.children.is_empty());
    }

    #[test]
    fn attach_failure_terminates_and_reaps_spawned_worker() {
        let directory = tempfile::tempdir().unwrap();
        let profile_path = write_profile(directory.path());
        let worker = sleeping_worker(directory.path());
        let mut processes = ManagedProcesses::with_game_log_and_platform(
            directory.path().join("state/wroid/game-session.log"),
            dead_platform(),
        );

        let error = processes
            .launch_with_worker_for_test(
                SessionId::new("attach-fails").unwrap(),
                &request_for_profile(&profile_path, "attach-fails"),
                &worker,
                // SAFETY: geteuid takes no arguments and has no preconditions.
                unsafe { libc::geteuid() },
            )
            .unwrap_err();

        assert!(matches!(error, ProcessError::Io(_)));
        assert!(processes.children.is_empty());
    }

    #[test]
    fn same_persistent_platform_is_reused_across_reaped_workers() {
        let directory = tempfile::tempdir().unwrap();
        let profile_path = write_profile(directory.path());
        let calls = shared_platform_calls();
        let mut processes = ManagedProcesses::with_game_log_and_platform(
            directory.path().join("state/wroid/game-session.log"),
            fake_platform(calls.clone()),
        );
        let uid = {
            // SAFETY: geteuid takes no arguments and has no preconditions.
            unsafe { libc::geteuid() }
        };

        for session in ["first-worker", "second-worker"] {
            processes
                .launch_with_worker_for_test(
                    SessionId::new(session).unwrap(),
                    &request_for_profile(&profile_path, session),
                    &env::current_exe().unwrap(),
                    uid,
                )
                .unwrap();
            (0..100)
                .find_map(|_| {
                    let completed = processes.reap().unwrap();
                    if completed.is_empty() {
                        std::thread::sleep(Duration::from_millis(5));
                        None
                    } else {
                        Some(completed)
                    }
                })
                .expect("managed worker was not reaped");
        }

        assert_eq!(
            *calls.lock().unwrap(),
            [
                "factory",
                "prepare:com.example.process",
                "serve",
                "prepare:com.example.process",
                "serve"
            ]
        );
    }

    #[test]
    fn requested_game_mode_wraps_the_authenticated_worker() {
        let worker = Path::new("/opt/wroid/bin/wroid");
        let wrapper = Path::new("/usr/bin/gamemoderun");
        let arguments = vec![OsString::from("launch-v2"), OsString::from("profile.json")];

        let plan = launch_program(worker, arguments.clone(), true, Some(wrapper));

        assert_eq!(plan.executable, wrapper);
        assert_eq!(
            plan.arguments,
            [
                worker.as_os_str().to_owned(),
                arguments[0].clone(),
                arguments[1].clone()
            ]
        );
        assert!(plan.game_mode_active);
    }

    #[test]
    fn disabled_or_unavailable_game_mode_launches_worker_directly() {
        let worker = Path::new("/opt/wroid/bin/wroid");
        let arguments = vec![OsString::from("launch-v2")];

        for (requested, wrapper) in [
            (false, Some(Path::new("/usr/bin/gamemoderun"))),
            (true, None),
        ] {
            let plan = launch_program(worker, arguments.clone(), requested, wrapper);
            assert_eq!(plan.executable, worker);
            assert_eq!(plan.arguments, arguments);
            assert!(!plan.game_mode_active);
        }
    }

    #[test]
    fn game_process_command_removes_loader_override_environment() {
        let plan = launch_program(
            Path::new("/opt/wroid/bin/wroid"),
            vec![OsString::from("launch-v2")],
            true,
            Some(Path::new("/usr/bin/gamemoderun")),
        );

        let command = command_for_program(&plan);
        let environment: BTreeMap<_, _> = command.get_envs().collect();

        assert_eq!(environment.get(OsStr::new("GAMEMODERUNEXEC")), Some(&None));
        assert_eq!(environment.get(OsStr::new("LD_PRELOAD")), Some(&None));
    }

    #[test]
    fn game_mode_wrapper_must_be_canonical_owned_and_protected() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let wrapper = directory.path().join("gamemoderun");
        fs::write(&wrapper, b"#!/bin/sh\nexec \"$@\"\n").unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        // SAFETY: geteuid takes no arguments and has no preconditions.
        let uid = unsafe { libc::geteuid() };

        assert!(validate_game_mode_wrapper(&wrapper, uid, directory.path()).is_ok());

        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o775)).unwrap();
        assert!(validate_game_mode_wrapper(&wrapper, uid, directory.path()).is_err());

        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
        let link = directory.path().join("gamemoderun-link");
        symlink(&wrapper, &link).unwrap();
        assert!(validate_game_mode_wrapper(&link, uid, directory.path()).is_err());

        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o777)).unwrap();
        assert!(validate_game_mode_wrapper(&wrapper, uid, directory.path()).is_err());
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn input_paths_cannot_escape_the_device_tree() {
        validate_input_path(Some(Path::new("/dev/input/event3")), "keyboard").unwrap();
        assert!(validate_input_path(Some(Path::new("/tmp/event3")), "keyboard").is_err());
        assert!(
            validate_input_path(Some(Path::new("/dev/input/../../tmp/event3")), "keyboard")
                .is_err()
        );
    }

    #[test]
    fn owned_child_is_reaped_with_exit_detail() {
        let session_id = SessionId::new("reap-clean").unwrap();
        let child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let mut processes = ManagedProcesses::new();
        processes
            .children
            .insert(session_id.clone(), test_process(child));

        let completed = (0..100)
            .find_map(|_| {
                let completed = processes.reap().unwrap();
                if completed.is_empty() {
                    thread::sleep(Duration::from_millis(5));
                    None
                } else {
                    Some(completed)
                }
            })
            .expect("short-lived child was not reaped");

        assert_eq!(completed[0].session_id, session_id);
        assert!(completed[0].success);
        assert!(completed[0].detail.contains("exit status: 0"));
        assert!(processes.children.is_empty());
    }

    #[test]
    fn active_child_blocks_a_second_launch_before_request_paths_are_used() {
        let session_id = SessionId::new("active-one").unwrap();
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let mut processes = ManagedProcesses::new();
        processes.children.insert(session_id, test_process(child));

        let error = processes
            .launch(
                SessionId::new("active-two").unwrap(),
                &launch_request(),
                std::process::id() as libc::pid_t,
                // SAFETY: geteuid takes no arguments and has no preconditions.
                unsafe { libc::geteuid() },
            )
            .unwrap_err();

        assert!(matches!(error, ProcessError::AlreadyActive));
    }

    #[test]
    fn stop_signals_the_exact_owned_child_for_later_reaping() {
        let session_id = SessionId::new("stop-owned").unwrap();
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let mut processes = ManagedProcesses::new();
        processes
            .children
            .insert(session_id.clone(), test_process(child));

        assert!(processes.request_stop(&session_id).unwrap());
        let completed = (0..100)
            .find_map(|_| {
                let completed = processes.reap().unwrap();
                if completed.is_empty() {
                    thread::sleep(Duration::from_millis(5));
                    None
                } else {
                    Some(completed)
                }
            })
            .expect("signalled child was not reaped");

        assert_eq!(completed[0].session_id, session_id);
        assert!(completed[0].success);
        assert!(completed[0].detail.contains("signal 15"));
    }

    #[test]
    fn private_game_log_rejects_hardlink_without_truncating_target() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let target = directory.path().join("keep.txt");
        fs::write(&target, b"preserve this content").unwrap();
        let log = state.join("game-session.log");
        fs::hard_link(&target, &log).unwrap();

        // SAFETY: geteuid takes no arguments and has no preconditions.
        let uid = unsafe { libc::geteuid() };
        assert!(open_private_game_log(&log, uid).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"preserve this content");
    }
}
