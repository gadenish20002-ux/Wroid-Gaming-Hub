use std::env;
use std::fs::{self, OpenOptions};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use wroid_core::profile_v2::ProfileV2;
use wroid_daemon::ipc::{
    AuthenticatedDaemonPeer, DaemonClient, DaemonRequest, DaemonResult, GameLaunchRequest,
    SessionSnapshot, SessionStateWire, StopReasonWire, PROTOCOL_VERSION,
};

use super::play_v2::PlayV2Options;

const START_ATTEMPTS: usize = 40;
const START_POLL: Duration = Duration::from_millis(50);
const RELEASE_STOP_TIMEOUT: Duration = Duration::from_secs(2);
static SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn start() -> Result<()> {
    let client = ensure_running()?;
    let (pid, sessions) = ping(&client)?;
    println!("wroidd is ready: PID {pid}, protocol {PROTOCOL_VERSION}, {sessions} session(s)");
    Ok(())
}

pub(crate) fn status() -> Result<()> {
    let client = DaemonClient::connect_default().context("wroidd is not running")?;
    let (pid, sessions) = ping(&client)?;
    println!("wroidd: running");
    println!("PID: {pid}");
    println!("Protocol: {PROTOCOL_VERSION}");
    println!("Sessions: {sessions}");
    Ok(())
}

pub(crate) fn sessions() -> Result<()> {
    let client = DaemonClient::connect_default().context("wroidd is not running")?;
    let DaemonResult::Sessions { sessions } = client.request(DaemonRequest::List)? else {
        bail!("wroidd returned an unexpected response to session listing");
    };
    if sessions.is_empty() {
        println!("No daemon-managed sessions.");
        return Ok(());
    }
    for session in sessions {
        println!(
            "{}\t{:?}\t{}\t{} control(s)",
            session.session_id, session.state, session.package_name, session.control_count
        );
    }
    Ok(())
}

pub(crate) fn launch_game(
    profile_path: &Path,
    profile: &ProfileV2,
    width: u32,
    height: u32,
    keyboard: Option<&Path>,
    mouse: Option<&Path>,
    game_mode: bool,
) -> Result<String> {
    let options = PlayV2Options {
        keyboard: keyboard.map(Path::to_path_buf),
        mouse: mouse.map(Path::to_path_buf),
        resolution: wroid_core::Resolution { width, height },
        grab: true,
        show_ui: true,
        launch_package: true,
        trace_input: false,
        exit_after: None,
        focus_socket: None,
    };
    let launch = start_managed_game(profile_path, profile, &options, game_mode)?;
    let pid = launch.process_id;
    let performance = if game_mode {
        "GameMode Auto requested"
    } else {
        "GameMode Off"
    };
    Ok(format!(
        "Started the game at {width}×{height} via wroidd PID {pid}; {performance}; Ctrl+Esc or Hub Stop ends it"
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedLaunch {
    pub(crate) session_id: String,
    pub(crate) process_id: u32,
}

pub(crate) fn start_managed_game(
    profile_path: &Path,
    profile: &ProfileV2,
    options: &PlayV2Options,
    game_mode: bool,
) -> Result<ManagedLaunch> {
    let profile_path = profile_path
        .canonicalize()
        .with_context(|| format!("failed to resolve profile {}", profile_path.display()))?;
    let session_id = next_hub_session_id();
    let request = game_launch_request(
        session_id.clone(),
        profile_path,
        profile.clone(),
        options,
        game_mode,
    );
    let client = ensure_running().context("failed to start the per-user Wroid runtime daemon")?;
    let DaemonResult::Session { session } = client
        .request(request)
        .with_context(|| format!("wroidd could not launch session {session_id}"))?
    else {
        bail!("wroidd returned an unexpected response to game launch");
    };
    let pid = session
        .process_id
        .context("wroidd launched a session without a worker PID")?;
    Ok(ManagedLaunch {
        session_id,
        process_id: pid,
    })
}

pub(crate) fn managed_session_state(
    session_id: &str,
) -> Result<wroid_daemon::ipc::SessionSnapshot> {
    let client = DaemonClient::connect_default().context("wroidd is not running")?;
    let DaemonResult::Session { session } = client.request(DaemonRequest::State {
        session_id: session_id.to_owned(),
    })?
    else {
        bail!("wroidd returned an unexpected response to session state");
    };
    Ok(session)
}

pub(crate) fn stop_game() -> Result<Option<String>> {
    let client = match DaemonClient::connect_default() {
        Ok(client) => client,
        Err(_) => return Ok(None),
    };
    let DaemonResult::Sessions { sessions } = client.request(DaemonRequest::List)? else {
        bail!("wroidd returned an unexpected response to session listing");
    };
    let mut active = sessions.into_iter().filter(|session| {
        session.process_id.is_some()
            && matches!(
                session.state,
                SessionStateWire::Running | SessionStateWire::Stopping
            )
    });
    let Some(session) = active.next() else {
        return Ok(None);
    };
    if active.next().is_some() {
        bail!("wroidd reported more than one active game session");
    }
    if session.state == SessionStateWire::Stopping {
        return Ok(Some(format!(
            "Stop is already finishing for {}",
            session.package_name
        )));
    }
    let DaemonResult::Stopped { session, .. } = client.request(DaemonRequest::Stop {
        session_id: session.session_id,
        reason: StopReasonWire::UserRequested,
    })?
    else {
        bail!("wroidd returned an unexpected response to game stop");
    };
    Ok(Some(format!(
        "Stop requested for {}; cleanup is finishing",
        session.package_name
    )))
}

fn game_launch_request(
    session_id: String,
    profile_path: PathBuf,
    profile: ProfileV2,
    options: &PlayV2Options,
    game_mode: bool,
) -> DaemonRequest {
    DaemonRequest::LaunchProfileV2 {
        launch: GameLaunchRequest {
            session_id,
            profile_path,
            profile,
            width: options.resolution.width,
            height: options.resolution.height,
            keyboard: options.keyboard.clone(),
            mouse: options.mouse.clone(),
            game_mode,
            worker_protocol_generation: wroid_inject::BRIDGE_WORKER_PROTOCOL_GENERATION,
            grab: options.grab,
            show_ui: options.show_ui,
            launch_package: options.launch_package,
            trace_input: options.trace_input,
            exit_after_millis: options
                .exit_after
                .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX)),
        },
    }
}

fn next_hub_session_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sequence = SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("hub-{}-{millis}-{sequence}", std::process::id())
}

pub(crate) fn ensure_running() -> Result<DaemonClient> {
    let executable = daemon_executable()?;
    validate_daemon_executable(&executable)?;
    let desired_identity = daemon_file_identity(&executable)?;
    if let Ok(client) = DaemonClient::connect_default() {
        let (_pid, _session_count, peer, running_identity) = authenticated_ping(&client)
            .context("failed to authenticate the running wroidd release")?;
        if running_identity == desired_identity {
            return Ok(client);
        }
        let (result, list_peer) = client
            .request_with_peer(DaemonRequest::List)
            .context("failed to inspect sessions owned by the stale wroidd release")?;
        if list_peer != peer {
            bail!("wroidd peer identity changed while checking the running release");
        }
        let DaemonResult::Sessions { sessions } = result else {
            bail!("wroidd returned an unexpected response to session listing");
        };
        if sessions_block_upgrade(&sessions) {
            bail!(
                "a game is still running under an older wroidd release; stop it before launching with the updated Wroid"
            );
        }
        stop_authenticated_idle_daemon(peer, running_identity)?;
    }

    let log = daemon_log()?;
    let stderr = log
        .try_clone()
        .context("failed to clone the wroidd log handle")?;
    let mut child = Command::new(&executable)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .process_group(0)
        .spawn()
        .with_context(|| format!("failed to start {}", executable.display()))?;

    for _ in 0..START_ATTEMPTS {
        if let Ok(client) = DaemonClient::connect_default() {
            if authenticated_ping(&client)
                .is_ok_and(|(_, _, _, identity)| identity == desired_identity)
            {
                return Ok(client);
            }
        }
        if let Some(status) = child
            .try_wait()
            .context("failed to inspect the new wroidd process")?
        {
            bail!(
                "wroidd exited during startup with {status}; inspect {}",
                daemon_log_path()?.display()
            );
        }
        thread::sleep(START_POLL);
    }
    bail!(
        "wroidd did not publish its private socket within 2 seconds; inspect {}",
        daemon_log_path()?.display()
    )
}

fn authenticated_ping(
    client: &DaemonClient,
) -> Result<(u32, usize, AuthenticatedDaemonPeer, (u64, u64))> {
    let (result, peer) = client.request_with_peer(DaemonRequest::Ping)?;
    let DaemonResult::Pong { pid, session_count } = result else {
        bail!("wroidd returned an unexpected response to ping");
    };
    if i64::from(pid) != i64::from(peer.pid) {
        bail!(
            "wroidd response PID {pid} does not match authenticated peer PID {}",
            peer.pid
        );
    }
    let running_path = PathBuf::from(format!("/proc/{}/exe", peer.pid));
    let identity = daemon_file_identity(&running_path)
        .context("failed to identify the authenticated running wroidd")?;
    Ok((pid, session_count, peer, identity))
}

fn daemon_file_identity(path: &Path) -> Result<(u64, u64)> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect daemon identity {}", path.display()))?;
    if !metadata.is_file() {
        bail!("daemon identity is not a regular file: {}", path.display());
    }
    Ok((metadata.dev(), metadata.ino()))
}

fn sessions_block_upgrade(sessions: &[SessionSnapshot]) -> bool {
    sessions.iter().any(|session| {
        session.process_id.is_some()
            && matches!(
                session.state,
                SessionStateWire::Preparing
                    | SessionStateWire::Running
                    | SessionStateWire::Stopping
            )
    })
}

fn stop_authenticated_idle_daemon(
    peer: AuthenticatedDaemonPeer,
    expected_identity: (u64, u64),
) -> Result<()> {
    if peer.pid <= 0 || peer.uid != effective_uid() {
        bail!("refusing to stop an unauthenticated wroidd peer");
    }
    let executable = PathBuf::from(format!("/proc/{}/exe", peer.pid));
    if daemon_file_identity(&executable)? != expected_identity {
        bail!("authenticated wroidd PID changed before pidfd acquisition");
    }
    let pidfd = open_pidfd(peer.pid)?;
    match daemon_file_identity(&executable) {
        Ok(identity) if identity == expected_identity => {}
        Ok(_) => bail!("authenticated wroidd PID was reused before replacement"),
        Err(_error) if process_is_gone(peer.pid) => return Ok(()),
        Err(error) => return Err(error),
    }
    // SAFETY: pidfd_send_signal targets the task referenced by the owned
    // pidfd; null siginfo and flags zero are valid for SIGTERM.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            std::os::fd::AsRawFd::as_raw_fd(&pidfd),
            libc::SIGTERM,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error).context("failed to stop the authenticated stale wroidd");
        }
    }
    wait_for_pidfd_exit(&pidfd, RELEASE_STOP_TIMEOUT)
}

fn open_pidfd(pid: libc::pid_t) -> Result<OwnedFd> {
    if pid <= 0 {
        bail!("cannot open a pidfd for an invalid PID");
    }
    // SAFETY: pidfd_open takes a positive PID and flags zero.
    let raw_fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if raw_fd < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to open wroidd pidfd");
    }
    let raw_fd = i32::try_from(raw_fd).context("wroidd pidfd is out of range")?;
    // SAFETY: pidfd_open returned a new descriptor whose sole ownership is
    // transferred into OwnedFd exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(raw_fd) })
}

fn wait_for_pidfd_exit(pidfd: &OwnedFd, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!(
                "stale wroidd did not exit within {} ms",
                timeout.as_millis()
            );
        }
        let timeout_ms = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
        let mut descriptor = libc::pollfd {
            fd: std::os::fd::AsRawFd::as_raw_fd(pidfd),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: descriptor points to one initialized pollfd for the duration
        // of the call and timeout_ms is non-negative.
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if result > 0 && descriptor.revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            return Ok(());
        }
        if result > 0 {
            bail!(
                "unexpected poll state while waiting for stale wroidd: {}",
                descriptor.revents
            );
        }
        if result == 0 {
            bail!(
                "stale wroidd did not exit within {} ms",
                timeout.as_millis()
            );
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error).context("failed while waiting for stale wroidd exit");
        }
    }
}

fn process_is_gone(pid: libc::pid_t) -> bool {
    // SAFETY: signal zero performs existence/permission checking only.
    let result = unsafe { libc::kill(pid, 0) };
    result != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

fn ping(client: &DaemonClient) -> Result<(u32, usize)> {
    match client.request(DaemonRequest::Ping)? {
        DaemonResult::Pong { pid, session_count } => Ok((pid, session_count)),
        _ => bail!("wroidd returned an unexpected response to ping"),
    }
}

fn daemon_executable() -> Result<PathBuf> {
    let current = env::current_exe().context("failed to locate the current Wroid executable")?;
    let adjacent = current
        .parent()
        .context("Wroid executable has no parent directory")?
        .join("wroidd");
    if adjacent.is_file() {
        return Ok(adjacent);
    }
    let staged = data_home()?.join("libexec").join("wroid").join("wroidd");
    if staged.is_file() {
        return Ok(staged);
    }
    bail!(
        "wroidd is missing beside {} and from {}; rebuild and run `wroid desktop install`",
        current.display(),
        staged.display()
    )
}

fn validate_daemon_executable(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect daemon executable {}", path.display()))?;
    let effective_uid = effective_uid();
    if !metadata.is_file()
        || metadata.uid() != effective_uid
        || metadata.permissions().mode() & 0o111 == 0
        || metadata.permissions().mode() & 0o022 != 0
    {
        bail!(
            "wroidd must be a current-user-owned executable that is not group/other writable: {}",
            path.display()
        );
    }
    Ok(())
}

fn daemon_log() -> Result<fs::File> {
    let path = daemon_log_path()?;
    let directory = path.parent().context("wroidd log path has no parent")?;
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure {}", directory.display()))?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != effective_uid()
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o077 != 0
    {
        bail!("wroidd log is not a private current-user file");
    }
    Ok(file)
}

fn daemon_log_path() -> Result<PathBuf> {
    let state_home = env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("state"))
        })
        .context("HOME and XDG_STATE_HOME are unavailable for the wroidd log")?;
    Ok(state_home.join("wroid").join("wroidd.log"))
}

fn data_home() -> Result<PathBuf> {
    env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("share"))
        })
        .context("HOME and XDG_DATA_HOME are unavailable")
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid takes no arguments and has no preconditions.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use wroid_core::profile_v2::ProfileV2;
    use wroid_daemon::ipc::{AuthenticatedDaemonPeer, SessionSnapshot};

    #[test]
    fn rejects_group_writable_daemon_executable() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wroidd");
        fs::write(&path, b"test").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        validate_daemon_executable(&path).unwrap();

        fs::set_permissions(&path, fs::Permissions::from_mode(0o775)).unwrap();
        assert!(validate_daemon_executable(&path).is_err());

        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        let link = directory.path().join("wroidd-link");
        symlink(&path, &link).unwrap();
        assert!(validate_daemon_executable(&link).is_err());
    }

    #[test]
    fn hub_launch_builds_a_typed_daemon_request() {
        let profile: ProfileV2 = serde_json::from_str(
            r#"{
                "schema_version": 2,
                "name": "PUBG Mobile",
                "package_name": "com.tencent.ig",
                "bindings": []
            }"#,
        )
        .unwrap();

        let request = game_launch_request(
            "hub-42-7".to_owned(),
            PathBuf::from("/profiles/pubg-v2.json"),
            profile,
            &PlayV2Options {
                keyboard: Some(PathBuf::from("/dev/input/event3")),
                mouse: Some(PathBuf::from("/dev/input/event5")),
                resolution: wroid_core::Resolution {
                    width: 1600,
                    height: 900,
                },
                grab: true,
                show_ui: true,
                launch_package: true,
                trace_input: false,
                exit_after: None,
                focus_socket: None,
            },
            true,
        );

        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["method"], "launch_profile_v2");
        assert_eq!(
            value["params"]["launch"]["profilePath"],
            "/profiles/pubg-v2.json"
        );
        assert_eq!(
            value["params"]["launch"]["profile"]["package_name"],
            "com.tencent.ig"
        );
        assert_eq!(value["params"]["launch"]["gameMode"], true);
        assert!(value["params"]["launch"].get("arguments").is_none());
    }

    #[test]
    fn managed_launch_maps_every_worker_option() {
        let profile: ProfileV2 = serde_json::from_str(
            r#"{
                "schema_version": 2,
                "name": "Input test",
                "package_name": "com.example.input",
                "bindings": []
            }"#,
        )
        .unwrap();
        let options = super::super::play_v2::PlayV2Options {
            keyboard: Some(PathBuf::from("/dev/input/event3")),
            mouse: Some(PathBuf::from("/dev/input/event5")),
            resolution: wroid_core::Resolution {
                width: 1280,
                height: 720,
            },
            grab: false,
            show_ui: false,
            launch_package: false,
            trace_input: true,
            exit_after: Some(Duration::from_millis(25)),
            focus_socket: None,
        };

        let request = game_launch_request(
            "managed-42".to_owned(),
            PathBuf::from("/profiles/input-v2.json"),
            profile,
            &options,
            false,
        );
        let DaemonRequest::LaunchProfileV2 { launch } = request else {
            panic!("expected managed launch");
        };
        assert_eq!((launch.width, launch.height), (1280, 720));
        assert_eq!(launch.keyboard, options.keyboard);
        assert_eq!(launch.mouse, options.mouse);
        assert!(!launch.grab);
        assert!(!launch.show_ui);
        assert!(!launch.launch_package);
        assert!(launch.trace_input);
        assert_eq!(launch.exit_after_millis, Some(25));
        assert_eq!(
            launch.worker_protocol_generation,
            wroid_inject::BRIDGE_WORKER_PROTOCOL_GENERATION
        );
    }

    #[test]
    fn daemon_release_identity_uses_followed_device_and_inode() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first");
        let hardlink = directory.path().join("hardlink");
        let different = directory.path().join("different");
        fs::write(&first, b"one").unwrap();
        fs::hard_link(&first, &hardlink).unwrap();
        fs::write(&different, b"two").unwrap();

        assert_eq!(
            daemon_file_identity(&first).unwrap(),
            daemon_file_identity(&hardlink).unwrap()
        );
        assert_ne!(
            daemon_file_identity(&first).unwrap(),
            daemon_file_identity(&different).unwrap()
        );
    }

    #[test]
    fn only_process_bearing_live_states_block_daemon_release() {
        assert!(!sessions_block_upgrade(&[snapshot(
            SessionStateWire::Stopped,
            None
        )]));
        assert!(!sessions_block_upgrade(&[snapshot(
            SessionStateWire::Preparing,
            None
        )]));
        assert!(sessions_block_upgrade(&[snapshot(
            SessionStateWire::Preparing,
            Some(98)
        )]));
        assert!(sessions_block_upgrade(&[snapshot(
            SessionStateWire::Running,
            Some(99)
        )]));
        assert!(sessions_block_upgrade(&[snapshot(
            SessionStateWire::Stopping,
            Some(99)
        )]));
        assert!(!sessions_block_upgrade(&[snapshot(
            SessionStateWire::Failed,
            None
        )]));
    }

    #[test]
    fn daemon_release_rejects_changed_pid_identity_without_signalling() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let peer = AuthenticatedDaemonPeer {
            pid: child.id() as libc::pid_t,
            uid: effective_uid(),
        };
        let unrelated = daemon_file_identity(&env::current_exe().unwrap()).unwrap();

        assert!(stop_authenticated_idle_daemon(peer, unrelated).is_err());
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn daemon_release_signals_the_authenticated_pidfd() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let peer = AuthenticatedDaemonPeer {
            pid: child.id() as libc::pid_t,
            uid: effective_uid(),
        };
        let identity =
            daemon_file_identity(Path::new(&format!("/proc/{}/exe", child.id()))).unwrap();

        stop_authenticated_idle_daemon(peer, identity).unwrap();
        let status = child.wait().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGTERM));
    }

    #[test]
    fn daemon_release_pidfd_wait_is_bounded() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let pidfd = open_pidfd(child.id() as libc::pid_t).unwrap();

        assert!(wait_for_pidfd_exit(&pidfd, Duration::from_millis(10)).is_err());

        child.kill().unwrap();
        child.wait().unwrap();
    }

    fn snapshot(state: SessionStateWire, process_id: Option<u32>) -> SessionSnapshot {
        SessionSnapshot {
            session_id: "release-test".to_owned(),
            state,
            package_name: "com.example.game".to_owned(),
            launch_package: true,
            control_count: 0,
            process_id,
            detail: None,
        }
    }
}
