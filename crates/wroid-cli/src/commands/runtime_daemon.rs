use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
#[cfg(not(test))]
use std::process::Child;
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
const RESUME_WATCHDOG_PIDFD: RawFd = 199;
const RESUME_WATCHDOG_CONTROL_FD: RawFd = 200;
const WATCHDOG_SOURCE_FD_MIN: RawFd = 201;
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

pub(crate) fn validate_daemon_worker_parent_executable(parent_pid: u32) -> Result<()> {
    let desired = daemon_executable()?;
    let parent = PathBuf::from(format!("/proc/{parent_pid}/exe"));
    validate_daemon_worker_parent_identity(&desired, &parent)
}

fn validate_daemon_worker_parent_identity(desired: &Path, parent: &Path) -> Result<()> {
    if daemon_file_identity(desired)? != daemon_file_identity(parent)? {
        bail!("daemon worker parent executable does not match the selected wroidd release");
    }
    Ok(())
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
    match daemon_file_identity(&executable) {
        Ok(identity) if identity == expected_identity => {}
        Ok(_) => bail!("authenticated wroidd executable changed before replacement"),
        Err(_error) if process_is_gone(peer.pid) => return Ok(()),
        Err(error) => return Err(error),
    }
    if pidfd_has_exited(peer.pidfd())? {
        return Ok(());
    }

    let mut resume = ContinueGuard::new(peer.pidfd())?;
    pidfd_send_signal(peer.pidfd(), libc::SIGSTOP)
        .context("failed to freeze the authenticated stale wroidd")?;
    if !wait_for_process_stopped(peer.pid, RELEASE_STOP_TIMEOUT)? {
        return Ok(());
    }
    if process_has_children(peer.pid)? {
        bail!("stale wroidd acquired a child process while upgrade safety was being checked");
    }
    pidfd_send_signal(peer.pidfd(), libc::SIGTERM)
        .context("failed to stop the authenticated stale wroidd")?;
    resume.resume()?;
    wait_for_pidfd_exit(peer.pidfd(), RELEASE_STOP_TIMEOUT)
}

fn pidfd_send_signal(pidfd: RawFd, signal: libc::c_int) -> Result<()> {
    // SAFETY: pidfd_send_signal targets the task referenced by pidfd; null
    // siginfo and flags zero are valid for standard signals.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd,
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(error.into())
}

struct ContinueGuard {
    pidfd: RawFd,
    armed: bool,
    control: Option<UnixStream>,
    watchdog: Option<ResumeWatchdog>,
}

impl ContinueGuard {
    fn new(pidfd: RawFd) -> Result<Self> {
        let (control, watchdog) = spawn_resume_watchdog(pidfd)?;
        Ok(Self {
            pidfd,
            armed: true,
            control: Some(control),
            watchdog: Some(watchdog),
        })
    }

    fn resume(&mut self) -> Result<()> {
        pidfd_send_signal(self.pidfd, libc::SIGCONT)?;
        self.armed = false;
        self.disarm_watchdog()
    }

    fn disarm_watchdog(&mut self) -> Result<()> {
        if let Some(mut control) = self.control.take() {
            control
                .write_all(b"D")
                .context("failed to disarm the stale-daemon resume watchdog")?;
        }
        let Some(watchdog) = self.watchdog.take() else {
            return Ok(());
        };
        match watchdog {
            #[cfg(not(test))]
            ResumeWatchdog::Process(mut watchdog) => {
                let deadline = Instant::now() + Duration::from_secs(1);
                loop {
                    if let Some(status) = watchdog
                        .try_wait()
                        .context("failed to inspect the stale-daemon resume watchdog")?
                    {
                        if status.success() {
                            return Ok(());
                        }
                        bail!("stale-daemon resume watchdog exited with {status}");
                    }
                    if Instant::now() >= deadline {
                        let _ = watchdog.kill();
                        let _ = watchdog.wait();
                        bail!("stale-daemon resume watchdog did not exit after disarm");
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            }
            #[cfg(test)]
            ResumeWatchdog::Thread(watchdog) => watchdog
                .join()
                .map_err(|_| anyhow::anyhow!("stale-daemon resume watchdog thread panicked"))?,
        }
    }
}

enum ResumeWatchdog {
    #[cfg(not(test))]
    Process(Child),
    #[cfg(test)]
    Thread(thread::JoinHandle<Result<()>>),
}

#[cfg(test)]
fn spawn_resume_watchdog(pidfd: RawFd) -> Result<(UnixStream, ResumeWatchdog)> {
    let pidfd = duplicate_owned_fd(pidfd)?;
    let (control, watchdog_control) =
        UnixStream::pair().context("failed to create the stale-daemon watchdog control channel")?;
    let watchdog = thread::spawn(move || watch_stopped_daemon(pidfd, watchdog_control));
    Ok((control, ResumeWatchdog::Thread(watchdog)))
}

#[cfg(not(test))]
fn spawn_resume_watchdog(pidfd: RawFd) -> Result<(UnixStream, ResumeWatchdog)> {
    spawn_resume_watchdog_process(pidfd)
        .map(|(control, child)| (control, ResumeWatchdog::Process(child)))
}

#[cfg(not(test))]
fn spawn_resume_watchdog_process(pidfd: RawFd) -> Result<(UnixStream, Child)> {
    let inherited_pidfd = duplicate_owned_fd(pidfd)?;
    let (control, child_control) =
        UnixStream::pair().context("failed to create the stale-daemon watchdog control channel")?;
    let inherited_control = duplicate_owned_fd(child_control.as_raw_fd())?;
    drop(child_control);
    let pidfd_source = inherited_pidfd.as_raw_fd();
    let control_source = inherited_control.as_raw_fd();
    let executable = env::current_exe().context("failed to locate the Wroid executable")?;
    let mut command = Command::new(executable);
    command
        .arg("resume-stale-daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    // SAFETY: the closure uses only async-signal-safe descriptor syscalls
    // between fork and exec. Source descriptors stay owned until spawn returns.
    unsafe {
        command.pre_exec(move || {
            if libc::dup3(pidfd_source, RESUME_WATCHDOG_PIDFD, 0) < 0
                || libc::dup3(control_source, RESUME_WATCHDOG_CONTROL_FD, 0) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if libc::syscall(
                libc::SYS_close_range,
                libc::STDERR_FILENO + 1,
                RESUME_WATCHDOG_PIDFD - 1,
                libc::CLOSE_RANGE_CLOEXEC,
            ) != 0
                || libc::syscall(
                    libc::SYS_close_range,
                    RESUME_WATCHDOG_CONTROL_FD + 1,
                    u32::MAX,
                    libc::CLOSE_RANGE_CLOEXEC,
                ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let watchdog = command
        .spawn()
        .context("failed to start the stale-daemon resume watchdog")?;
    Ok((control, watchdog))
}

impl Drop for ContinueGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = pidfd_send_signal(self.pidfd, libc::SIGCONT);
            self.armed = false;
        }
        let _ = self.disarm_watchdog();
    }
}

fn duplicate_owned_fd(source: RawFd) -> Result<OwnedFd> {
    // SAFETY: F_DUPFD_CLOEXEC duplicates a valid borrowed descriptor into a
    // fresh descriptor at or above the requested minimum.
    let duplicate = unsafe { libc::fcntl(source, libc::F_DUPFD_CLOEXEC, WATCHDOG_SOURCE_FD_MIN) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to duplicate a stale-daemon watchdog descriptor");
    }
    // SAFETY: fcntl returned a fresh descriptor transferred exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
}

pub(crate) fn resume_stale_daemon() -> Result<()> {
    for fd in [RESUME_WATCHDOG_PIDFD, RESUME_WATCHDOG_CONTROL_FD] {
        // SAFETY: F_GETFD validates the inherited descriptor without changing it.
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
            bail!("stale-daemon resume watchdog is missing inherited descriptor {fd}");
        }
    }
    // SAFETY: this hidden command is the sole adopter of both descriptors
    // assigned by spawn_resume_watchdog.
    let pidfd = unsafe { OwnedFd::from_raw_fd(RESUME_WATCHDOG_PIDFD) };
    // SAFETY: the inherited control descriptor is one endpoint of a Unix pair.
    let control_fd = unsafe { OwnedFd::from_raw_fd(RESUME_WATCHDOG_CONTROL_FD) };
    watch_stopped_daemon(pidfd, UnixStream::from(control_fd))
}

fn watch_stopped_daemon(pidfd: OwnedFd, mut control: UnixStream) -> Result<()> {
    let mut command = [0_u8; 1];
    match control.read(&mut command) {
        Ok(1) if command[0] == b'D' => Ok(()),
        Ok(1) => {
            pidfd_send_signal(pidfd.as_raw_fd(), libc::SIGCONT)?;
            bail!("stale-daemon watchdog received an invalid control command")
        }
        Ok(0) => pidfd_send_signal(pidfd.as_raw_fd(), libc::SIGCONT)
            .context("failed to resume stale wroidd after upgrader exit"),
        Ok(_) => unreachable!("one-byte read returned an oversized result"),
        Err(error) => {
            let resume = pidfd_send_signal(pidfd.as_raw_fd(), libc::SIGCONT);
            match resume {
                Ok(()) => Err(error).context("stale-daemon watchdog control failed"),
                Err(resume_error) => Err(anyhow::anyhow!(
                    "stale-daemon watchdog control failed: {error}; resume failed: {resume_error}"
                )),
            }
        }
    }
}

fn wait_for_process_stopped(pid: libc::pid_t, timeout: Duration) -> Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        match fs::read_to_string(format!("/proc/{pid}/status")) {
            Ok(status) => {
                if status.lines().any(|line| {
                    line.strip_prefix("State:")
                        .is_some_and(|value| value.trim_start().starts_with(['T', 't']))
                }) {
                    return Ok(true);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error).context("failed to inspect frozen stale wroidd"),
        }
        if Instant::now() >= deadline {
            bail!("stale wroidd did not stop for atomic upgrade inspection");
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn process_has_children(pid: libc::pid_t) -> Result<bool> {
    let children = fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .context("failed to inspect stale wroidd children")?;
    Ok(!children.trim().is_empty())
}

fn pidfd_has_exited(pidfd: RawFd) -> Result<bool> {
    let mut descriptor = libc::pollfd {
        fd: pidfd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: descriptor points to one initialized pollfd and zero performs a
    // non-blocking readiness check.
    let result = unsafe { libc::poll(&mut descriptor, 1, 0) };
    if result < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to inspect wroidd pidfd");
    }
    Ok(result > 0 && descriptor.revents & (libc::POLLIN | libc::POLLHUP) != 0)
}

fn wait_for_pidfd_exit(pidfd: RawFd, timeout: Duration) -> Result<()> {
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
            fd: pidfd,
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
    fn daemon_worker_parent_executable_must_match_selected_release() {
        let directory = tempfile::tempdir().unwrap();
        let desired = directory.path().join("wroidd");
        let same = directory.path().join("same-wroidd");
        let wrapper = directory.path().join("wrapper");
        fs::write(&desired, b"daemon").unwrap();
        fs::hard_link(&desired, &same).unwrap();
        fs::write(&wrapper, b"wrapper").unwrap();

        validate_daemon_worker_parent_identity(&desired, &same).unwrap();
        assert!(validate_daemon_worker_parent_identity(&desired, &wrapper).is_err());
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
        let peer =
            AuthenticatedDaemonPeer::bind_process(child.id() as libc::pid_t, effective_uid())
                .unwrap();
        let unrelated = daemon_file_identity(&env::current_exe().unwrap()).unwrap();
        let child_identity =
            daemon_file_identity(Path::new(&format!("/proc/{}/exe", child.id()))).unwrap();
        assert_ne!(child_identity, unrelated);
        assert!(child.try_wait().unwrap().is_none());

        let result = stop_authenticated_idle_daemon(peer, unrelated);
        assert!(result.is_err(), "mismatched identity returned {result:?}");
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn daemon_release_signals_the_authenticated_pidfd() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let peer =
            AuthenticatedDaemonPeer::bind_process(child.id() as libc::pid_t, effective_uid())
                .unwrap();
        let identity =
            daemon_file_identity(Path::new(&format!("/proc/{}/exe", child.id()))).unwrap();

        stop_authenticated_idle_daemon(peer, identity).unwrap();
        let status = child.wait().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGTERM));
    }

    #[test]
    fn daemon_release_resumes_and_refuses_a_peer_with_a_new_child() {
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30 & wait"])
            .process_group(0)
            .spawn()
            .unwrap();
        let pid = child.id() as libc::pid_t;
        let deadline = Instant::now() + Duration::from_secs(1);
        while !process_has_children(pid).unwrap() {
            assert!(Instant::now() < deadline, "shell child was not published");
            thread::sleep(Duration::from_millis(10));
        }
        let peer = AuthenticatedDaemonPeer::bind_process(pid, effective_uid()).unwrap();
        let identity = daemon_file_identity(Path::new(&format!("/proc/{pid}/exe"))).unwrap();

        assert!(stop_authenticated_idle_daemon(peer, identity).is_err());
        assert!(child.try_wait().unwrap().is_none());
        let state = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
        assert!(!state.lines().any(|line| {
            line.strip_prefix("State:")
                .is_some_and(|value| value.trim_start().starts_with(['T', 't']))
        }));

        // SAFETY: the test created an isolated process group whose id is pid.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
        child.wait().unwrap();
    }

    #[test]
    fn resume_watchdog_continues_a_daemon_when_its_control_peer_disappears() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let peer =
            AuthenticatedDaemonPeer::bind_process(child.id() as libc::pid_t, effective_uid())
                .unwrap();
        let (control, watchdog_control) = std::os::unix::net::UnixStream::pair().unwrap();
        let pidfd = duplicate_owned_fd(peer.pidfd()).unwrap();
        pidfd_send_signal(peer.pidfd(), libc::SIGSTOP).unwrap();
        assert!(
            wait_for_process_stopped(child.id() as libc::pid_t, Duration::from_secs(1)).unwrap()
        );
        let watchdog = thread::spawn(move || watch_stopped_daemon(pidfd, watchdog_control));

        drop(control);
        watchdog.join().unwrap().unwrap();
        let state = fs::read_to_string(format!("/proc/{}/status", child.id())).unwrap();
        assert!(!state.lines().any(|line| {
            line.strip_prefix("State:")
                .is_some_and(|value| value.trim_start().starts_with(['T', 't']))
        }));

        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn daemon_release_pidfd_wait_is_bounded() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let peer =
            AuthenticatedDaemonPeer::bind_process(child.id() as libc::pid_t, effective_uid())
                .unwrap();

        assert!(wait_for_pidfd_exit(peer.pidfd(), Duration::from_millis(10)).is_err());

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
