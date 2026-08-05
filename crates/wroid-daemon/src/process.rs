use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};

use thiserror::Error;
use wroid_runtime::SessionId;

use crate::ipc::GameLaunchRequest;

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

#[derive(Debug, Default)]
pub(crate) struct ManagedProcesses {
    children: BTreeMap<SessionId, Child>,
    game_log_override: Option<PathBuf>,
}

impl ManagedProcesses {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn with_game_log(path: PathBuf) -> Self {
        Self {
            children: BTreeMap::new(),
            game_log_override: Some(path),
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
        let executable = peer_executable(peer_pid, expected_uid)?;
        let profile_path = validated_profile_path(&request.profile_path, expected_uid)?;
        validate_input_path(request.keyboard.as_deref(), "keyboard")?;
        validate_input_path(request.mouse.as_deref(), "mouse")?;
        let arguments = launch_arguments(
            &profile_path,
            request.width,
            request.height,
            request.keyboard.as_deref(),
            request.mouse.as_deref(),
        );
        let program = launch_program(
            &executable,
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
        let child = command.spawn()?;
        let pid = child.id();
        self.children.insert(session_id, child);
        Ok(pid)
    }

    pub(crate) fn request_stop(&mut self, session_id: &SessionId) -> Result<bool, ProcessError> {
        let Some(child) = self.children.get(session_id) else {
            return Ok(false);
        };
        let pid = i32::try_from(child.id())
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
        Ok(true)
    }

    pub(crate) fn reap(&mut self) -> Result<Vec<ReapedProcess>, ProcessError> {
        let mut completed = Vec::new();
        for (session_id, child) in &mut self.children {
            if let Some(status) = child.try_wait()? {
                let detail = match status.signal() {
                    Some(signal) => format!("game worker terminated by signal {signal}"),
                    None => format!("game worker exited with {status}"),
                };
                completed.push(ReapedProcess {
                    session_id: session_id.clone(),
                    success: status.success(),
                    detail,
                });
            }
        }
        for process in &completed {
            self.children.remove(&process.session_id);
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
        for child in self.children.values() {
            if let Ok(pid) = i32::try_from(child.id()) {
                // SAFETY: each Child is still owned and unreaped here.
                let _ = unsafe { libc::kill(pid, libc::SIGTERM) };
            }
        }
    }
}

pub(crate) fn launch_arguments(
    profile_path: &Path,
    width: u32,
    height: u32,
    keyboard: Option<&Path>,
    mouse: Option<&Path>,
) -> Vec<OsString> {
    let mut arguments = vec![
        OsString::from("launch-v2"),
        profile_path.as_os_str().to_owned(),
        OsString::from("--width"),
        OsString::from(width.to_string()),
        OsString::from("--height"),
        OsString::from(height.to_string()),
    ];
    if let Some(path) = keyboard {
        arguments.push(OsString::from("--keyboard"));
        arguments.push(path.as_os_str().to_owned());
    }
    if let Some(path) = mouse {
        arguments.push(OsString::from("--mouse"));
        arguments.push(path.as_os_str().to_owned());
    }
    arguments
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
    use std::thread;
    use std::time::Duration;

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
        }
    }

    #[test]
    fn launch_arguments_are_fixed_and_typed() {
        assert_eq!(
            launch_arguments(
                Path::new("/profiles/pubg-v2.json"),
                1600,
                900,
                Some(Path::new("/dev/input/event3")),
                Some(Path::new("/dev/input/event5")),
            ),
            [
                "launch-v2",
                "/profiles/pubg-v2.json",
                "--width",
                "1600",
                "--height",
                "900",
                "--keyboard",
                "/dev/input/event3",
                "--mouse",
                "/dev/input/event5",
            ]
            .map(OsString::from)
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
        processes.children.insert(session_id.clone(), child);

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
        processes.children.insert(session_id, child);

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
        processes.children.insert(session_id.clone(), child);

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
        assert!(!completed[0].success);
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
