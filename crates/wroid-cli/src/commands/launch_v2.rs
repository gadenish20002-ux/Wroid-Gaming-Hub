use std::env;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
#[cfg(test)]
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::process::ExitStatus;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use wroid_core::profile_v2::ProfileV2;
use wroid_daemon::ipc::{
    DaemonClient, DaemonRequest, DaemonResult, SessionSnapshot, SessionStateWire, StopReasonWire,
};
use wroid_inject::{BridgeBrokerClient, GameSessionReport, LatencyMetrics, BRIDGE_WORKER_FD};

use super::compatibility::CompatibilityReport;
use super::graphics::GraphicsReport;
use super::kwin_focus::KwinFocusRelay;
use super::play_v2::{self, PlayV2Options};

const RESTORE_TICKET_BYTES: usize = 16;
const RESTORE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const RESTORE_READY_ATTEMPTS: usize = 120;
const ACTIVE_SESSION_VERSION: u32 = 1;
const ACTIVE_SESSION_FILE: &str = "active-session.json";
const LAST_SESSION_VERSION: u32 = 1;
const LAST_SESSION_FILE: &str = "last-game-session.json";
const LAST_SESSION_MAX_BYTES: u64 = 64 * 1024;
const LAST_SESSION_DETAIL_CHARS: usize = 4096;
const STOP_WAIT_ATTEMPTS: usize = 30;
const SIGTERM: i32 = 15;
const MANAGED_POLL_INTERVAL: Duration = Duration::from_millis(50);
const GAME_SESSION_LOG_FILE: &str = "game-session.log";
static FOREGROUND_INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DaemonWorkerInvocation {
    pub(crate) bridge_fd: i32,
    pub(crate) daemon_parent_pid: u32,
}

pub(crate) fn launch_v2(
    profile_path: PathBuf,
    options: PlayV2Options,
    daemon_worker: Option<DaemonWorkerInvocation>,
) -> Result<()> {
    let Some(invocation) = daemon_worker else {
        return launch_v2_managed(profile_path, options);
    };
    let result = launch_v2_worker(profile_path.clone(), options, invocation);
    let outcome = session_outcome_from_result(&profile_path, std::process::id(), &result);
    if let Err(error) = write_last_game_session(&outcome) {
        eprintln!("Warning: could not save the last game session report: {error:#}");
    }
    result.map(|_| ())
}

fn launch_v2_managed(profile_path: PathBuf, options: PlayV2Options) -> Result<()> {
    let profile_path = profile_path
        .canonicalize()
        .with_context(|| format!("failed to resolve profile {}", profile_path.display()))?;
    let profile = load_validated_profile(&profile_path)?;
    ensure_input_bridge_available()?;
    print_launch_preflight(&profile, options.launch_package)?;
    let _interrupt_handler = ForegroundInterruptHandler::install()?;
    let launch =
        super::runtime_daemon::start_managed_game(&profile_path, &profile, &options, false)?;
    let mut cleanup = ManagedSessionCleanup::new(&launch.session_id);
    println!(
        "Managed session {} started via wroidd (worker PID {}). Ctrl+Esc or Ctrl+C stops it.",
        launch.session_id, launch.process_id
    );
    let result = wait_for_managed_session(&launch.session_id);
    if result.is_ok() {
        cleanup.disarm();
    }
    result
}

fn launch_v2_worker(
    profile_path: PathBuf,
    options: PlayV2Options,
    invocation: DaemonWorkerInvocation,
) -> Result<GameSessionReport> {
    let actual_parent = u32::try_from(unsafe { libc::getppid() }).unwrap_or(0);
    validate_daemon_worker_parent(invocation.daemon_parent_pid, actual_parent)?;
    super::runtime_daemon::validate_daemon_worker_parent_executable(actual_parent)?;
    if invocation.bridge_fd != BRIDGE_WORKER_FD {
        bail!(
            "daemon worker bridge descriptor must be {BRIDGE_WORKER_FD}, got {}",
            invocation.bridge_fd
        );
    }
    // SAFETY: clap accepted a non-standard descriptor, the daemon contract
    // assigns its sole ownership to this worker, and this is the only adoption.
    let owned_fd = unsafe { OwnedFd::from_raw_fd(invocation.bridge_fd) };
    let bridge_broker = BridgeBrokerClient::from_owned_fd_for_peer(
        owned_fd,
        i32::try_from(actual_parent).context("daemon parent PID is out of range")?,
        effective_uid_from_proc().unwrap_or(u32::MAX),
    )
    .context("failed to adopt the daemon-owned bridge channel")?;
    launch_v2_worker_inner(profile_path, options, bridge_broker)
}

fn launch_v2_worker_inner(
    profile_path: PathBuf,
    mut options: PlayV2Options,
    bridge_broker: BridgeBrokerClient,
) -> Result<GameSessionReport> {
    let profile_path = profile_path
        .canonicalize()
        .with_context(|| format!("failed to resolve profile {}", profile_path.display()))?;
    let profile = load_validated_profile(&profile_path)?;
    let is_root = effective_uid_from_proc().unwrap_or(u32::MAX) == 0;
    let _launch_lease = if is_root {
        None
    } else {
        Some(acquire_launch_lease(&profile.name)?)
    };
    ensure_input_bridge_available()?;
    print_launch_preflight(&profile, options.launch_package)?;
    let _active_session = if is_root {
        None
    } else {
        Some(ActiveSessionGuard::register(
            &profile.name,
            &profile.package_name,
        )?)
    };

    if is_root {
        return play_v2::play_v2_with_broker(profile_path, options, Some(bridge_broker));
    }

    let focus_relay = if options.grab {
        match KwinFocusRelay::start() {
            Ok(relay) => {
                println!("Focus protection: KDE window tracking is active.");
                options.focus_socket = Some(relay.socket_path().to_path_buf());
                Some(relay)
            }
            Err(error) => {
                eprintln!("Focus protection unavailable: {error:#}");
                eprintln!("Input will remain captured until Ctrl+Esc or session shutdown.");
                None
            }
        }
    } else {
        None
    };
    let mut desktop = SystemDesktopSession;
    let result = run_with_desktop_restoration(&mut desktop, || {
        println!(
            "Starting {} as the desktop user; the verified helper owns only the temporary input bridge…",
            profile.name
        );
        play_v2::play_v2_with_broker(profile_path, options, Some(bridge_broker))
    });
    drop(focus_relay);
    result
}

fn load_validated_profile(profile_path: &Path) -> Result<ProfileV2> {
    let profile = ProfileV2::load_from_path(profile_path)
        .with_context(|| format!("failed to load profile v2 {}", profile_path.display()))?;
    profile
        .validate()
        .map_err(|error| anyhow::anyhow!("invalid profile v2: {}", error.errors.join("; ")))?;
    Ok(profile)
}

fn print_launch_preflight(profile: &ProfileV2, launch_package: bool) -> Result<()> {
    let graphics = GraphicsReport::probe();
    graphics.ensure_launch_ready()?;
    println!(
        "Performance preflight: {} — {}",
        graphics.health().to_ascii_uppercase(),
        graphics
            .host
            .renderer
            .as_deref()
            .unwrap_or("renderer unknown")
    );
    let compatibility = CompatibilityReport::probe();
    if launch_package {
        compatibility.ensure_package_installed_if_known(&profile.package_name)?;
    }
    if let Some(game) = compatibility.game(&profile.package_name) {
        println!(
            "Game compatibility: {} — {}",
            compatibility.health().to_ascii_uppercase(),
            game.detail
        );
    }
    Ok(())
}

fn validate_daemon_worker_parent(expected: u32, actual: u32) -> Result<()> {
    if expected == 0 || actual != expected {
        bail!("daemon worker parent changed: expected PID {expected}, got {actual}");
    }
    Ok(())
}

fn managed_session_finished(session: &SessionSnapshot) -> bool {
    matches!(
        session.state,
        SessionStateWire::Stopped | SessionStateWire::Failed
    )
}

fn managed_stop_request(session_id: &str) -> DaemonRequest {
    DaemonRequest::Stop {
        session_id: session_id.to_owned(),
        reason: StopReasonWire::UserRequested,
    }
}

fn managed_terminal_result(session: &SessionSnapshot) -> Result<()> {
    match session.state {
        SessionStateWire::Stopped => Ok(()),
        SessionStateWire::Failed => bail!(
            "managed game session failed: {}",
            session.detail.as_deref().unwrap_or("worker failure")
        ),
        _ => bail!("managed game session has not reached a terminal state"),
    }
}

fn wait_for_managed_session(session_id: &str) -> Result<()> {
    let mut log = open_private_game_session_log()?;
    let mut stop_sent = false;
    loop {
        copy_available_log(&mut log)?;
        if FOREGROUND_INTERRUPTED.load(Ordering::Relaxed) && !stop_sent {
            let client = DaemonClient::connect_default().context("wroidd is not running")?;
            let DaemonResult::Stopped { .. } = client
                .request(managed_stop_request(session_id))
                .context("failed to stop the interrupted managed session")?
            else {
                bail!("wroidd returned an unexpected response to managed session stop");
            };
            stop_sent = true;
            eprintln!("Stop requested; waiting for cleanup…");
        }
        let session = super::runtime_daemon::managed_session_state(session_id)?;
        if managed_session_finished(&session) {
            copy_available_log(&mut log)?;
            return managed_terminal_result(&session);
        }
        thread::sleep(MANAGED_POLL_INTERVAL);
    }
}

struct ManagedSessionCleanup {
    session_id: String,
    armed: bool,
}

impl ManagedSessionCleanup {
    fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_owned(),
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ManagedSessionCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(client) = DaemonClient::connect_default() {
            let _ = client.request(managed_stop_request(&self.session_id));
        }
    }
}

fn game_session_log_path() -> Result<PathBuf> {
    let state_home = env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("state"))
        })
        .context("HOME and XDG_STATE_HOME are unavailable for the game log")?;
    Ok(state_home.join("wroid").join(GAME_SESSION_LOG_FILE))
}

fn open_private_game_session_log() -> Result<fs::File> {
    let path = game_session_log_path()?;
    open_private_game_session_log_at(&path, effective_uid_from_proc().unwrap_or(u32::MAX))
}

fn open_private_game_session_log_at(path: &Path, uid: u32) -> Result<fs::File> {
    let directory = path.parent().context("game log path has no parent")?;
    let directory_metadata = fs::symlink_metadata(directory)
        .with_context(|| format!("failed to inspect {}", directory.display()))?;
    if !directory_metadata.is_dir()
        || directory_metadata.uid() != uid
        || directory_metadata.permissions().mode() & 0o077 != 0
    {
        bail!("game log directory is not private and current-user-owned");
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o077 != 0
    {
        bail!("game log is not a private current-user file");
    }
    Ok(file)
}

fn copy_available_log(log: &mut fs::File) -> Result<()> {
    let mut buffer = [0_u8; 8192];
    let mut stdout = std::io::stdout().lock();
    loop {
        match log.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => stdout.write_all(&buffer[..count])?,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error).context("failed to read the managed game log"),
        }
    }
    stdout.flush()?;
    Ok(())
}

struct ForegroundInterruptHandler {
    previous: libc::sigaction,
}

impl ForegroundInterruptHandler {
    fn install() -> Result<Self> {
        FOREGROUND_INTERRUPTED.store(false, Ordering::Relaxed);
        // SAFETY: zeroed sigaction is initialized below before the syscall.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = foreground_interrupt as *const () as usize;
        // SAFETY: sigemptyset initializes the embedded mask.
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        action.sa_flags = 0;
        // SAFETY: storage for the previous handler is valid and both action
        // structures live through sigaction.
        let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigaction(libc::SIGINT, &action, &mut previous) } != 0 {
            return Err(std::io::Error::last_os_error())
                .context("failed to install Ctrl+C handler");
        }
        Ok(Self { previous })
    }
}

impl Drop for ForegroundInterruptHandler {
    fn drop(&mut self) {
        // SAFETY: previous was populated by sigaction and remains valid.
        unsafe { libc::sigaction(libc::SIGINT, &self.previous, std::ptr::null_mut()) };
        FOREGROUND_INTERRUPTED.store(false, Ordering::Relaxed);
    }
}

extern "C" fn foreground_interrupt(_signal: libc::c_int) {
    FOREGROUND_INTERRUPTED.store(true, Ordering::Relaxed);
}

fn ensure_input_bridge_available() -> Result<()> {
    if let Some(owner) = wroid_inject::active_default_bridge_lease_owner()
        .context("failed to inspect the Wroid input bridge lease")?
    {
        bail!(
            "another Wroid game session is already active ({owner}); stop it with Ctrl+Esc before launching another game"
        );
    }
    Ok(())
}

#[cfg(test)]
mod daemon_worker_contract_tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use wroid_daemon::ipc::{DaemonRequest, SessionSnapshot, SessionStateWire, StopReasonWire};

    #[test]
    fn worker_parent_must_still_be_the_authenticated_daemon() {
        assert!(validate_daemon_worker_parent(42, 42).is_ok());
        assert!(validate_daemon_worker_parent(42, 7).is_err());
        assert!(validate_daemon_worker_parent(0, 0).is_err());
    }

    #[test]
    fn foreground_wait_recognizes_only_terminal_states() {
        for state in [
            SessionStateWire::Preparing,
            SessionStateWire::Running,
            SessionStateWire::Stopping,
        ] {
            assert!(!managed_session_finished(&snapshot(state)));
        }
        assert!(managed_session_finished(&snapshot(
            SessionStateWire::Stopped
        )));
        assert!(managed_session_finished(&snapshot(
            SessionStateWire::Failed
        )));
        assert!(managed_terminal_result(&snapshot(SessionStateWire::Failed)).is_err());
    }

    #[test]
    fn foreground_interrupt_targets_only_its_managed_session() {
        assert_eq!(
            managed_stop_request("launch-42"),
            DaemonRequest::Stop {
                session_id: "launch-42".to_owned(),
                reason: StopReasonWire::UserRequested,
            }
        );
    }

    #[test]
    fn foreground_log_reader_rejects_links_and_public_files() {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let log = directory.path().join("game-session.log");
        fs::write(&log, b"worker output\n").unwrap();
        fs::set_permissions(&log, fs::Permissions::from_mode(0o600)).unwrap();
        let uid = effective_uid_from_proc().unwrap();
        assert!(open_private_game_session_log_at(&log, uid).is_ok());

        fs::set_permissions(&log, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(open_private_game_session_log_at(&log, uid).is_err());
        fs::set_permissions(&log, fs::Permissions::from_mode(0o600)).unwrap();
        let link = directory.path().join("linked.log");
        symlink(&log, &link).unwrap();
        assert!(open_private_game_session_log_at(&link, uid).is_err());
    }

    fn snapshot(state: SessionStateWire) -> SessionSnapshot {
        SessionSnapshot {
            session_id: "launch-42".to_owned(),
            state,
            package_name: "com.example.game".to_owned(),
            launch_package: true,
            control_count: 0,
            process_id: Some(42),
            detail: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveGameSessionState {
    pub(crate) owner: Option<String>,
    pub(crate) can_stop: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LastGameSessionState {
    version: u32,
    pub(crate) pid: u32,
    pub(crate) profile_path: PathBuf,
    pub(crate) profile_name: String,
    pub(crate) package_name: String,
    pub(crate) state: String,
    pub(crate) detail: String,
    pub(crate) finished_unix_millis: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) performance: Option<LastSessionPerformance>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LastSessionPerformance {
    pub(crate) frames_submitted: u64,
    pub(crate) peak_simultaneous_contacts: u64,
    pub(crate) mouse_aim_recenters: u64,
    pub(crate) reader_to_inject: LastSessionLatency,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) kernel_to_inject: Option<LastSessionLatency>,
    pub(crate) rejected_kernel_timestamps: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LastSessionLatency {
    pub(crate) samples: u64,
    pub(crate) p50_micros: u64,
    pub(crate) p95_micros: u64,
    pub(crate) p99_micros: u64,
    pub(crate) max_micros: u64,
}

impl From<GameSessionReport> for LastSessionPerformance {
    fn from(report: GameSessionReport) -> Self {
        Self {
            frames_submitted: report.frames_submitted,
            peak_simultaneous_contacts: report.peak_simultaneous_contacts,
            mouse_aim_recenters: report.mouse_aim_recenters,
            reader_to_inject: report.reader_to_inject.into(),
            kernel_to_inject: report.kernel_to_inject.map(Into::into),
            rejected_kernel_timestamps: report.rejected_kernel_timestamps,
        }
    }
}

impl From<LatencyMetrics> for LastSessionLatency {
    fn from(metrics: LatencyMetrics) -> Self {
        Self {
            samples: metrics.samples,
            p50_micros: metrics.p50_micros,
            p95_micros: metrics.p95_micros,
            p99_micros: metrics.p99_micros,
            max_micros: metrics.max_micros,
        }
    }
}

pub(crate) fn last_game_session_state() -> Result<Option<LastGameSessionState>> {
    let path = last_game_session_path()?;
    read_last_game_session_at(&path)
}

#[cfg(test)]
fn report_covers_launch(
    current: &LastGameSessionState,
    pid: u32,
    launched_unix_millis: u64,
) -> bool {
    current.pid == pid || current.finished_unix_millis >= launched_unix_millis
}

#[cfg(test)]
fn background_exit_description(status: &ExitStatus) -> (&'static str, String) {
    if status.success() {
        ("clean", format!("Session process exited with {status}"))
    } else if matches!(
        status.signal(),
        Some(libc::SIGHUP) | Some(libc::SIGINT) | Some(libc::SIGTERM)
    ) {
        (
            "stopped",
            format!("Session process stopped by signal {status}"),
        )
    } else {
        (
            "failed",
            format!("Session process exited unexpectedly with {status}"),
        )
    }
}

pub(crate) fn active_game_session_state() -> Result<ActiveGameSessionState> {
    if let Some(record) = read_active_session()? {
        return Ok(ActiveGameSessionState {
            owner: Some(format!("PID {} · {}", record.pid, record.profile_name)),
            can_stop: true,
        });
    }
    if let Some(owner) = wroid_inject::active_default_bridge_lease_owner()
        .context("failed to inspect the Wroid input bridge lease")?
    {
        return Ok(ActiveGameSessionState {
            owner: Some(owner),
            can_stop: false,
        });
    }
    Ok(ActiveGameSessionState {
        owner: active_launch_lease_owner()?,
        can_stop: false,
    })
}

pub(crate) fn active_game_session_owner() -> Result<Option<String>> {
    Ok(active_game_session_state()?.owner)
}

pub(crate) fn stop_active_game_session() -> Result<String> {
    let record = read_active_session()?.context("no background Wroid game session is active")?;
    let process_fd = match open_validated_process_fd(&record) {
        Ok(process_fd) => process_fd,
        Err(error) if error_is_esrch(&error) => {
            remove_active_session_if_matches(&record);
            return Ok(format!("{} was already stopped", record.profile_name));
        }
        Err(error) => return Err(error),
    };
    // SAFETY: pidfd_send_signal addresses the already-open process identity,
    // not a recyclable numeric PID. SIGTERM is handled by the game runtime.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            process_fd.as_raw_fd(),
            SIGTERM,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error).context("failed to request game session stop");
        }
    }
    for _ in 0..STOP_WAIT_ATTEMPTS {
        if !process_matches_record(&record) {
            remove_active_session_if_matches(&record);
            return Ok(format!("Stopped {}", record.profile_name));
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(format!(
        "Stop requested for {}; cleanup is still finishing",
        record.profile_name
    ))
}

fn open_validated_process_fd(record: &ActiveSessionRecord) -> Result<OwnedFd> {
    let pid = i32::try_from(record.pid).context("active game PID is out of range")?;
    // SAFETY: pidfd_open returns a new owned descriptor on success.
    let raw_fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if raw_fd < 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to open the active game process");
    }
    // SAFETY: raw_fd was returned as a fresh descriptor by pidfd_open.
    let process_fd = unsafe { OwnedFd::from_raw_fd(raw_fd as i32) };
    if !process_matches_record(record) {
        bail!("the active game process changed before it could be stopped");
    }
    Ok(process_fd)
}

fn error_is_esrch(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.raw_os_error() == Some(libc::ESRCH))
}

fn acquire_launch_lease(profile_name: &str) -> Result<wroid_inject::WaydroidBridgeLease> {
    acquire_user_lifecycle_lease(&format!("launching {profile_name}"))
}

pub(crate) fn acquire_desktop_action_lease(
    action: &str,
) -> Result<wroid_inject::WaydroidBridgeLease> {
    acquire_user_lifecycle_lease(action)
}

fn acquire_user_lifecycle_lease(owner: &str) -> Result<wroid_inject::WaydroidBridgeLease> {
    let path = launch_lease_path()?;
    let directory = path
        .parent()
        .context("game launcher lease path has no parent")?;
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create runtime directory {}", directory.display()))?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure runtime directory {}", directory.display()))?;
    wroid_inject::WaydroidBridgeLease::acquire_named(&path, "the desktop game launcher", owner)
        .with_context(|| "another Wroid desktop action or game launch is already active")
}

fn active_launch_lease_owner() -> Result<Option<String>> {
    let Some(path) = optional_launch_lease_path() else {
        return Ok(None);
    };
    wroid_inject::active_bridge_lease_owner(path)
        .context("failed to inspect the desktop game launcher lease")
}

fn launch_lease_path() -> Result<PathBuf> {
    optional_launch_lease_path().context("XDG_RUNTIME_DIR is unavailable for the game launcher")
}

fn optional_launch_lease_path() -> Option<PathBuf> {
    env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|directory| directory.join("wroid").join("game-launch.lock"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ActiveSessionRecord {
    version: u32,
    pid: u32,
    process_start_ticks: u64,
    executable: PathBuf,
    profile_name: String,
    package_name: String,
    started_unix_seconds: u64,
}

struct ActiveSessionGuard {
    record: ActiveSessionRecord,
    path: PathBuf,
}

impl ActiveSessionGuard {
    fn register(profile_name: &str, package_name: &str) -> Result<Self> {
        let pid = std::process::id();
        let record = ActiveSessionRecord {
            version: ACTIVE_SESSION_VERSION,
            pid,
            process_start_ticks: read_process_start_ticks(pid)
                .context("failed to identify the game session process")?,
            executable: env::current_exe()
                .context("failed to locate the game session executable")?
                .canonicalize()
                .context("failed to resolve the game session executable")?,
            profile_name: profile_name.chars().take(160).collect(),
            package_name: package_name.chars().take(255).collect(),
            started_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        let path = active_session_path()?;
        write_active_session(&path, &record)?;
        Ok(Self { record, path })
    }
}

impl Drop for ActiveSessionGuard {
    fn drop(&mut self) {
        remove_active_session_if_matches_at(&self.path, &self.record);
    }
}

fn active_session_path() -> Result<PathBuf> {
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .context("XDG_RUNTIME_DIR is unavailable for active game session state")?;
    Ok(runtime_dir.join("wroid").join(ACTIVE_SESSION_FILE))
}

fn write_active_session(path: &Path, record: &ActiveSessionRecord) -> Result<()> {
    let directory = path.parent().context("active session path has no parent")?;
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure {}", directory.display()))?;
    let temporary = directory.join(format!(
        ".{ACTIVE_SESSION_FILE}.{}-{}.tmp",
        record.pid, record.process_start_ticks
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .with_context(|| format!("failed to create {}", temporary.display()))?;
        serde_json::to_writer(&mut file, record).context("failed to encode active session")?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)
            .with_context(|| format!("failed to publish {}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_active_session() -> Result<Option<ActiveSessionRecord>> {
    let path = active_session_path()?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()))
        }
    };
    let record: ActiveSessionRecord = match serde_json::from_slice(&bytes) {
        Ok(record) => record,
        Err(_) => {
            let _ = fs::remove_file(&path);
            return Ok(None);
        }
    };
    if record.version != ACTIVE_SESSION_VERSION || !process_matches_record(&record) {
        remove_active_session_if_matches_at(&path, &record);
        return Ok(None);
    }
    Ok(Some(record))
}

fn process_matches_record(record: &ActiveSessionRecord) -> bool {
    if record.pid == 0 || record.pid > i32::MAX as u32 {
        return false;
    }
    let process = PathBuf::from(format!("/proc/{}", record.pid));
    let current_uid = effective_uid_from_proc().unwrap_or(u32::MAX);
    if fs::metadata(&process).map(|metadata| metadata.uid()).ok() != Some(current_uid) {
        return false;
    }
    if read_process_start_ticks(record.pid).ok() != Some(record.process_start_ticks) {
        return false;
    }
    if fs::read_link(process.join("exe")).ok().as_deref() != Some(record.executable.as_path()) {
        return false;
    }
    process_is_launch_v2(&process.join("cmdline"))
}

fn process_is_launch_v2(cmdline_path: &Path) -> bool {
    fs::read(cmdline_path).is_ok_and(|bytes| {
        bytes
            .split(|byte| *byte == 0)
            .nth(1)
            .is_some_and(|argument| argument == b"launch-v2")
    })
}

fn read_process_start_ticks(pid: u32) -> Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))
        .with_context(|| format!("failed to read process {pid} start time"))?;
    parse_process_start_ticks(&stat).context("malformed Linux process stat")
}

fn parse_process_start_ticks(stat: &str) -> Option<u64> {
    let after_name = stat.rsplit_once(") ")?.1;
    after_name.split_whitespace().nth(19)?.parse().ok()
}

fn remove_active_session_if_matches(record: &ActiveSessionRecord) {
    if let Ok(path) = active_session_path() {
        remove_active_session_if_matches_at(&path, record);
    }
}

fn remove_active_session_if_matches_at(path: &Path, record: &ActiveSessionRecord) {
    let matches = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ActiveSessionRecord>(&bytes).ok())
        .is_some_and(|current| {
            current.pid == record.pid && current.process_start_ticks == record.process_start_ticks
        });
    if matches {
        let _ = fs::remove_file(path);
    }
}

fn session_outcome_from_result(
    profile_path: &Path,
    pid: u32,
    result: &Result<GameSessionReport>,
) -> LastGameSessionState {
    match result {
        Ok(report) => session_outcome(
            profile_path,
            pid,
            "clean",
            "Session ended cleanly; input and Waydroid lifecycle cleanup completed.",
            unix_time_millis(),
            Some((*report).into()),
        ),
        Err(error) => session_outcome(
            profile_path,
            pid,
            "failed",
            &format!("{error:#}"),
            unix_time_millis(),
            None,
        ),
    }
}

fn session_outcome(
    profile_path: &Path,
    pid: u32,
    state: &str,
    detail: &str,
    finished_unix_millis: u64,
    performance: Option<LastSessionPerformance>,
) -> LastGameSessionState {
    let resolved_path = profile_path
        .canonicalize()
        .unwrap_or_else(|_| profile_path.to_path_buf());
    let profile = ProfileV2::load_from_path(&resolved_path).ok();
    let fallback_name = resolved_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Wroid game")
        .to_owned();
    LastGameSessionState {
        version: LAST_SESSION_VERSION,
        pid,
        profile_path: resolved_path,
        profile_name: profile
            .as_ref()
            .map(|profile| profile.name.chars().take(160).collect())
            .unwrap_or(fallback_name),
        package_name: profile
            .map(|profile| profile.package_name.chars().take(255).collect())
            .unwrap_or_default(),
        state: state.to_owned(),
        detail: bounded_session_detail(detail),
        finished_unix_millis,
        performance,
    }
}

fn bounded_session_detail(detail: &str) -> String {
    detail
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .take(LAST_SESSION_DETAIL_CHARS)
        .collect()
}

fn last_game_session_path() -> Result<PathBuf> {
    let state_directory = env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("state"))
        })
        .context("HOME and XDG_STATE_HOME are unavailable for the game session report")?;
    Ok(state_directory.join("wroid").join(LAST_SESSION_FILE))
}

fn write_last_game_session(outcome: &LastGameSessionState) -> Result<()> {
    write_last_game_session_at(&last_game_session_path()?, outcome)
}

fn write_last_game_session_at(path: &Path, outcome: &LastGameSessionState) -> Result<()> {
    let directory = path.parent().context("last session path has no parent")?;
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure {}", directory.display()))?;
    let ticket = random_ticket().context("failed to name the session report update")?;
    let temporary = directory.join(format!(".{LAST_SESSION_FILE}.{ticket}.tmp"));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)
            .with_context(|| format!("failed to create {}", temporary.display()))?;
        serde_json::to_writer(&mut file, outcome).context("failed to encode session report")?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)
            .with_context(|| format!("failed to publish {}", path.display()))?;
        fs::File::open(directory)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_last_game_session_at(path: &Path) -> Result<Option<LastGameSessionState>> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to open {}", path.display()))
        }
    };
    if file.metadata()?.len() > LAST_SESSION_MAX_BYTES {
        drop(file);
        let _ = fs::remove_file(path);
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let outcome: LastGameSessionState = match serde_json::from_slice(&bytes) {
        Ok(outcome) => outcome,
        Err(_) => {
            let _ = fs::remove_file(path);
            return Ok(None);
        }
    };
    if !valid_last_game_session(&outcome) {
        let _ = fs::remove_file(path);
        return Ok(None);
    }
    Ok(Some(outcome))
}

fn valid_last_game_session(outcome: &LastGameSessionState) -> bool {
    outcome.version == LAST_SESSION_VERSION
        && outcome.pid > 0
        && matches!(outcome.state.as_str(), "clean" | "stopped" | "failed")
        && outcome.profile_name.chars().count() <= 160
        && outcome.package_name.chars().count() <= 255
        && outcome.detail.chars().count() <= LAST_SESSION_DETAIL_CHARS
        && outcome.profile_path.as_os_str().as_encoded_bytes().len() <= 4096
        && outcome
            .performance
            .as_ref()
            .is_none_or(valid_session_performance)
}

fn valid_session_performance(performance: &LastSessionPerformance) -> bool {
    const MAX_SAMPLES: u64 = 100_000;
    const MAX_LATENCY_MICROS: u64 = 60_000_000;
    const MAX_COUNTER: u64 = 1_000_000_000_000;

    fn valid_latency(latency: &LastSessionLatency) -> bool {
        latency.samples <= MAX_SAMPLES
            && (latency.samples > 0 || latency.max_micros == 0)
            && latency.p50_micros <= latency.p95_micros
            && latency.p95_micros <= latency.p99_micros
            && latency.p99_micros <= latency.max_micros
            && latency.max_micros <= MAX_LATENCY_MICROS
    }

    valid_latency(&performance.reader_to_inject)
        && performance
            .kernel_to_inject
            .as_ref()
            .is_none_or(valid_latency)
        && performance.frames_submitted <= MAX_COUNTER
        && performance.mouse_aim_recenters <= MAX_COUNTER
        && performance.rejected_kernel_timestamps <= MAX_COUNTER
        && performance.peak_simultaneous_contacts <= u16::MAX.into()
}

fn unix_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

trait DesktopSessionControl {
    fn is_running(&mut self) -> Result<bool>;
    fn stop(&mut self) -> Result<()>;
    fn start(&mut self) -> Result<()>;
    fn arm_watchdog(&mut self) -> Result<Option<RestoreWatchdog>>;
}

struct SystemDesktopSession;

impl DesktopSessionControl for SystemDesktopSession {
    fn is_running(&mut self) -> Result<bool> {
        let status = wroid_waydroid::status().context("failed to inspect the Waydroid session")?;
        Ok(session_is_running(&status))
    }

    fn stop(&mut self) -> Result<()> {
        stop_desktop_waydroid_session()
    }

    fn start(&mut self) -> Result<()> {
        start_desktop_waydroid_session()
    }

    fn arm_watchdog(&mut self) -> Result<Option<RestoreWatchdog>> {
        RestoreWatchdog::arm().map(Some)
    }
}

fn run_with_desktop_restoration<C, F, T>(desktop: &mut C, run: F) -> Result<T>
where
    C: DesktopSessionControl,
    F: FnOnce() -> Result<T>,
{
    let restore = desktop.is_running()?;
    let watchdog = if restore {
        println!("Lifecycle guard: desktop Waydroid will be restored after gameplay.");
        desktop.arm_watchdog()?
    } else {
        println!("Lifecycle guard: desktop Waydroid was already stopped.");
        None
    };

    if let Err(stop_error) = desktop.stop() {
        if restore {
            mark_watchdog_restoring(watchdog.as_ref());
        }
        let restore_error = if restore { desktop.start().err() } else { None };
        let restored = restore_error.is_none();
        let lifecycle_error =
            combine_lifecycle_errors("failed to stop desktop Waydroid", stop_error, restore_error);
        if restored {
            if let Some(watchdog) = watchdog {
                if let Err(disarm_error) = watchdog.disarm() {
                    return Err(anyhow!(
                        "{lifecycle_error:#}\nAdditionally, the lifecycle watchdog could not be disarmed: {disarm_error:#}"
                    ));
                }
            }
        }
        return Err(lifecycle_error);
    }

    let run_result = run();
    let restore_result = if restore {
        println!("Restoring desktop Waydroid session…");
        mark_watchdog_restoring(watchdog.as_ref());
        desktop.start()
    } else {
        Ok(())
    };

    let disarm_result = if restore_result.is_ok() {
        watchdog.map(RestoreWatchdog::disarm).unwrap_or(Ok(()))
    } else {
        Ok(())
    };
    if restore_result.is_ok() {
        if restore {
            println!("Desktop Waydroid session restored.");
        }
    } else {
        eprintln!(
            "Warning: immediate Waydroid restore failed; the detached lifecycle watchdog will retry."
        );
    }

    let outcome = match (run_result, restore_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(game_error), Ok(())) => Err(game_error),
        (Ok(_), Err(restore_error)) => Err(restore_error)
            .context("game session ended, but desktop Waydroid could not be restored"),
        (Err(game_error), Err(restore_error)) => Err(anyhow!(
            "{game_error:#}\nAdditionally, desktop Waydroid restore failed: {restore_error:#}"
        )),
    };
    match (outcome, disarm_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(disarm_error)) => Err(disarm_error)
            .context("desktop Waydroid was restored, but its recovery ticket remains armed"),
        (Err(error), Err(disarm_error)) => Err(anyhow!(
            "{error:#}\nAdditionally, the lifecycle watchdog could not be disarmed: {disarm_error:#}"
        )),
    }
}

fn mark_watchdog_restoring(watchdog: Option<&RestoreWatchdog>) {
    if let Some(watchdog) = watchdog {
        if let Err(error) = watchdog.begin_restore() {
            eprintln!("Warning: could not update the lifecycle watchdog phase: {error:#}");
        }
    }
}

fn combine_lifecycle_errors(
    context: &str,
    primary: anyhow::Error,
    secondary: Option<anyhow::Error>,
) -> anyhow::Error {
    match secondary {
        Some(secondary) => anyhow!("{context}: {primary:#}\nRestore also failed: {secondary:#}"),
        None => primary.context(context.to_owned()),
    }
}

fn stop_desktop_waydroid_session() -> Result<()> {
    let status = Command::new("waydroid")
        .args(["session", "stop"])
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to stop the current Waydroid session")?;
    if !status.success() {
        bail!("waydroid session stop exited with {status}");
    }
    Ok(())
}

fn start_desktop_waydroid_session() -> Result<()> {
    if wroid_waydroid::status()
        .as_deref()
        .is_ok_and(session_is_running)
    {
        return Ok(());
    }

    let mut command = Command::new("waydroid");
    command
        .args(["session", "start"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.process_group(0);
    let mut child = command
        .spawn()
        .context("failed to start the desktop Waydroid session")?;

    let mut last_status = String::new();
    for _ in 0..RESTORE_READY_ATTEMPTS {
        match wroid_waydroid::status() {
            Ok(status) => {
                last_status = status;
                if session_is_running(&last_status) {
                    return Ok(());
                }
            }
            Err(error) => last_status = error.to_string(),
        }
        if let Some(status) = child
            .try_wait()
            .context("failed to monitor the restored Waydroid session")?
        {
            bail!("waydroid session start exited with {status}\n{last_status}");
        }
        thread::sleep(RESTORE_POLL_INTERVAL);
    }
    bail!("desktop Waydroid did not return to RUNNING state\n{last_status}")
}

fn session_is_running(status: &str) -> bool {
    status.lines().any(|line| {
        line.split_once(':')
            .is_some_and(|(name, value)| name.trim() == "Session" && value.trim() == "RUNNING")
    })
}

struct RestoreWatchdog {
    ticket_path: PathBuf,
}

impl RestoreWatchdog {
    fn arm() -> Result<Self> {
        let parent_pid = std::process::id();
        let ticket = random_ticket()?;
        let ticket_path = restore_ticket_path(parent_pid, &ticket)?;
        if let Some(directory) = ticket_path.parent() {
            fs::create_dir_all(directory).with_context(|| {
                format!(
                    "failed to create lifecycle directory {}",
                    directory.display()
                )
            })?;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).with_context(
                || {
                    format!(
                        "failed to secure lifecycle directory {}",
                        directory.display()
                    )
                },
            )?;
        }
        let mut ticket_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&ticket_path)
            .with_context(|| {
                format!(
                    "failed to create lifecycle ticket {}",
                    ticket_path.display()
                )
            })?;
        if let Err(error) = ticket_file.write_all(b"armed\n") {
            let _ = fs::remove_file(&ticket_path);
            return Err(error).context("failed to initialize the lifecycle ticket");
        }

        let executable = env::current_exe().context("failed to locate the wroid executable")?;
        let mut command = Command::new(executable);
        command
            .args([
                "restore-desktop-session",
                "--parent-pid",
                &parent_pid.to_string(),
                "--ticket",
                &ticket,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.process_group(0);
        if let Err(error) = command.spawn() {
            let _ = fs::remove_file(&ticket_path);
            return Err(error).context("failed to start the Waydroid lifecycle watchdog");
        }
        Ok(Self { ticket_path })
    }

    fn begin_restore(&self) -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.ticket_path)
            .with_context(|| {
                format!(
                    "failed to update lifecycle ticket {}",
                    self.ticket_path.display()
                )
            })?;
        file.write_all(b"restoring\n")
            .context("failed to persist the lifecycle restore phase")
    }

    fn disarm(self) -> Result<()> {
        match fs::remove_file(&self.ticket_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to disarm lifecycle ticket {}",
                    self.ticket_path.display()
                )
            }),
        }
    }
}

pub(crate) fn restore_desktop_session(parent_pid: u32, ticket: &str) -> Result<()> {
    validate_ticket(ticket)?;
    let ticket_path = restore_ticket_path(parent_pid, ticket)?;
    while ticket_path.exists() && PathBuf::from(format!("/proc/{parent_pid}")).exists() {
        thread::sleep(RESTORE_POLL_INTERVAL);
    }
    if !ticket_path.exists() {
        return Ok(());
    }
    loop {
        let phase = fs::read_to_string(&ticket_path).unwrap_or_else(|_| "armed".to_owned());
        if let Ok(status) = wroid_waydroid::status() {
            match watchdog_action(phase.trim(), session_is_running(&status)) {
                WatchdogAction::Wait => {}
                WatchdogAction::Complete => return remove_restore_ticket(&ticket_path),
                WatchdogAction::Restore => {
                    if start_desktop_waydroid_session().is_ok() {
                        return remove_restore_ticket(&ticket_path);
                    }
                }
            }
        }
        if !ticket_path.exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatchdogAction {
    Wait,
    Complete,
    Restore,
}

fn watchdog_action(phase: &str, session_running: bool) -> WatchdogAction {
    if !session_running {
        WatchdogAction::Restore
    } else if phase == "restoring" {
        WatchdogAction::Complete
    } else {
        WatchdogAction::Wait
    }
}

fn remove_restore_ticket(ticket_path: &Path) -> Result<()> {
    match fs::remove_file(ticket_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to remove lifecycle ticket {}",
                ticket_path.display()
            )
        }),
    }
}

fn validate_ticket(ticket: &str) -> Result<()> {
    if ticket.len() != RESTORE_TICKET_BYTES * 2
        || !ticket
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("invalid lifecycle ticket");
    }
    Ok(())
}

fn random_ticket() -> Result<String> {
    let mut bytes = [0_u8; RESTORE_TICKET_BYTES];
    fs::File::open("/dev/urandom")
        .context("failed to open the system random source")?
        .read_exact(&mut bytes)
        .context("failed to generate a lifecycle ticket")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn restore_ticket_path(parent_pid: u32, ticket: &str) -> Result<PathBuf> {
    validate_ticket(ticket)?;
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .with_context(|| "XDG_RUNTIME_DIR is unavailable for the lifecycle watchdog")?;
    Ok(runtime_dir
        .join("wroid")
        .join(format!("restore-{parent_pid}-{ticket}")))
}

fn effective_uid_from_proc() -> Option<u32> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let uid_line = status.lines().find(|line| line.starts_with("Uid:"))?;
    uid_line.split_whitespace().nth(2)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeDesktopSession {
        running: bool,
        stop_fails: bool,
        start_fails: bool,
        stops: usize,
        starts: usize,
    }

    impl DesktopSessionControl for FakeDesktopSession {
        fn is_running(&mut self) -> Result<bool> {
            Ok(self.running)
        }

        fn stop(&mut self) -> Result<()> {
            self.stops += 1;
            if self.stop_fails {
                bail!("synthetic stop failure");
            }
            self.running = false;
            Ok(())
        }

        fn start(&mut self) -> Result<()> {
            self.starts += 1;
            if self.start_fails {
                bail!("synthetic start failure");
            }
            self.running = true;
            Ok(())
        }

        fn arm_watchdog(&mut self) -> Result<Option<RestoreWatchdog>> {
            Ok(None)
        }
    }

    #[test]
    fn parses_effective_uid_from_linux_process_status() {
        assert!(effective_uid_from_proc().is_some());
    }

    #[test]
    fn parses_linux_start_ticks_when_process_name_contains_spaces() {
        let mut fields = vec!["S".to_owned()];
        fields.extend((4_u64..=21).map(|field| field.to_string()));
        fields.push("424242".to_owned());
        fields.push("23".to_owned());
        let stat = format!("42 (Wroid game session) {}", fields.join(" "));

        assert_eq!(parse_process_start_ticks(&stat), Some(424242));
        assert_eq!(parse_process_start_ticks("malformed"), None);
    }

    #[test]
    fn accepts_only_a_typed_launch_v2_process_command() {
        let directory = tempfile::tempdir().unwrap();
        let cmdline = directory.path().join("cmdline");

        fs::write(&cmdline, b"/opt/wroid\0launch-v2\0/profile.json\0").unwrap();
        assert!(process_is_launch_v2(&cmdline));

        fs::write(&cmdline, b"/opt/wroid\0helper\0install\0").unwrap();
        assert!(!process_is_launch_v2(&cmdline));
    }

    #[test]
    fn last_session_report_round_trips_with_private_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state/wroid/last-game-session.json");
        let outcome = session_outcome(
            Path::new("/profiles/pubg-v2.json"),
            42,
            "failed",
            "startup failed\u{1b}[31m\nretry",
            123_456,
            Some(LastSessionPerformance {
                frames_submitted: 8_192,
                peak_simultaneous_contacts: 4,
                mouse_aim_recenters: 12,
                reader_to_inject: LastSessionLatency {
                    samples: 1_024,
                    p50_micros: 280,
                    p95_micros: 840,
                    p99_micros: 1_200,
                    max_micros: 2_400,
                },
                kernel_to_inject: Some(LastSessionLatency {
                    samples: 1_000,
                    p50_micros: 410,
                    p95_micros: 1_100,
                    p99_micros: 1_800,
                    max_micros: 3_100,
                }),
                rejected_kernel_timestamps: 2,
            }),
        );

        write_last_game_session_at(&path, &outcome).unwrap();

        assert_eq!(read_last_game_session_at(&path).unwrap(), Some(outcome));
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!fs::read_to_string(path).unwrap().contains('\u{1b}'));
    }

    #[test]
    fn malformed_last_session_report_is_removed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("last-game-session.json");
        fs::write(&path, "{broken").unwrap();

        assert_eq!(read_last_game_session_at(&path).unwrap(), None);
        assert!(!path.exists());
    }

    #[test]
    fn rejects_impossible_last_session_latency_metrics() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("last-game-session.json");
        let outcome = session_outcome(
            Path::new("/profile.json"),
            42,
            "clean",
            "done",
            123,
            Some(LastSessionPerformance {
                frames_submitted: 1,
                peak_simultaneous_contacts: 1,
                mouse_aim_recenters: 0,
                reader_to_inject: LastSessionLatency {
                    samples: 2,
                    p50_micros: 900,
                    p95_micros: 800,
                    p99_micros: 1_000,
                    max_micros: 1_100,
                },
                kernel_to_inject: None,
                rejected_kernel_timestamps: 0,
            }),
        );
        write_last_game_session_at(&path, &outcome).unwrap();

        assert_eq!(read_last_game_session_at(&path).unwrap(), None);
        assert!(!path.exists());
    }

    #[test]
    fn last_session_reader_does_not_follow_symbolic_links() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.json");
        let link = directory.path().join("last-game-session.json");
        fs::write(&target, "{}").unwrap();
        symlink(&target, &link).unwrap();

        assert!(read_last_game_session_at(&link).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "{}");
    }

    #[test]
    fn background_exit_classifies_signals_and_preserves_newer_reports() {
        let clean = ExitStatus::from_raw(0);
        let stopped = ExitStatus::from_raw(libc::SIGTERM);
        let failed = ExitStatus::from_raw(libc::SIGKILL);

        assert_eq!(background_exit_description(&clean).0, "clean");
        assert_eq!(background_exit_description(&stopped).0, "stopped");
        assert_eq!(background_exit_description(&failed).0, "failed");

        let report = session_outcome(Path::new("/profile.json"), 77, "clean", "done", 500, None);
        assert!(report_covers_launch(&report, 77, 900));
        assert!(report_covers_launch(&report, 78, 400));
        assert!(!report_covers_launch(&report, 78, 600));
    }

    #[test]
    fn restores_previously_running_desktop_after_success() {
        let mut desktop = FakeDesktopSession {
            running: true,
            ..Default::default()
        };
        run_with_desktop_restoration(&mut desktop, || Ok(())).unwrap();
        assert_eq!(desktop.stops, 1);
        assert_eq!(desktop.starts, 1);
        assert!(desktop.running);
    }

    #[test]
    fn desktop_restoration_preserves_the_game_report() {
        let mut desktop = FakeDesktopSession::default();
        let report = run_with_desktop_restoration(&mut desktop, || Ok("metrics")).unwrap();

        assert_eq!(report, "metrics");
    }

    #[test]
    fn restores_desktop_after_game_failure_and_preserves_error() {
        let mut desktop = FakeDesktopSession {
            running: true,
            ..Default::default()
        };
        let error = run_with_desktop_restoration(&mut desktop, || -> Result<()> {
            bail!("synthetic game failure")
        })
        .unwrap_err();
        assert!(error.to_string().contains("synthetic game failure"));
        assert_eq!(desktop.starts, 1);
        assert!(desktop.running);
    }

    #[test]
    fn leaves_previously_stopped_desktop_stopped() {
        let mut desktop = FakeDesktopSession::default();
        run_with_desktop_restoration(&mut desktop, || Ok(())).unwrap();
        assert_eq!(desktop.stops, 1);
        assert_eq!(desktop.starts, 0);
        assert!(!desktop.running);
    }

    #[test]
    fn stop_failure_attempts_to_restore_original_session() {
        let mut desktop = FakeDesktopSession {
            running: true,
            stop_fails: true,
            ..Default::default()
        };
        let error = run_with_desktop_restoration(&mut desktop, || Ok(())).unwrap_err();
        assert!(error
            .to_string()
            .contains("failed to stop desktop Waydroid"));
        assert_eq!(desktop.starts, 1);
    }

    #[test]
    fn reports_game_and_restore_failures_together() {
        let mut desktop = FakeDesktopSession {
            running: true,
            start_fails: true,
            ..Default::default()
        };
        let error = run_with_desktop_restoration(&mut desktop, || -> Result<()> {
            bail!("synthetic game failure")
        })
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("synthetic game failure"));
        assert!(message.contains("desktop Waydroid restore failed"));
    }

    #[test]
    fn parses_session_state_without_matching_container_state() {
        assert!(session_is_running(
            "Session:\tRUNNING\nContainer:\tFROZEN\n"
        ));
        assert!(!session_is_running(
            "Session:\tSTOPPED\nContainer:\tRUNNING\n"
        ));
    }

    #[test]
    fn lifecycle_ticket_accepts_only_fixed_lowercase_hex() {
        assert!(validate_ticket("0123456789abcdef0123456789abcdef").is_ok());
        for invalid in [
            "",
            "abc",
            "0123456789abcdef0123456789abcdeF",
            "0123456789abcdef0123456789abcdeg",
        ] {
            assert!(validate_ticket(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn crash_watchdog_waits_for_game_session_before_restore() {
        assert_eq!(
            watchdog_action("armed", true),
            WatchdogAction::Wait,
            "a surviving privileged game session still owns Waydroid"
        );
        assert_eq!(watchdog_action("armed", false), WatchdogAction::Restore);
        assert_eq!(
            watchdog_action("restoring", true),
            WatchdogAction::Complete,
            "the parent had already started desktop restoration"
        );
    }
}
