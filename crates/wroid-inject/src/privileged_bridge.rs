use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::thread::{self, sleep};
use std::time::{Duration, Instant};

use crate::{
    ensure_root, remove_default_bridge, validate_wroid_touchscreen_node, InputDeviceNode,
    InstalledWaydroidBridge, WaydroidBridgeLease, WROID_TOUCHSCREEN_NAME,
};

const READY_LINE: &str = "WROID_BRIDGE_READY 1";
const CHECK_LINE: &str = "WROID_HELPER_CHECK 1";
const CLEANUP_COMMAND: &[u8] = b"CLEANUP\n";
const VERIFY_ANDROID_INPUT_COMMAND: &[u8] = b"VERIFY_ANDROID_INPUT\n";
const ANDROID_INPUT_READY_LINE: &str = "WROID_ANDROID_INPUT_READY 1";
const MAX_HELPER_COMMAND_BYTES: u64 = 64;
const MAX_ANDROID_SETTINGS_ERROR_CHARS: usize = 4 * 1024;
const ANDROID_SETTINGS_TIMEOUT: Duration = Duration::from_secs(5);
const ANDROID_SETTINGS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const ANDROID_INPUT_ATTEMPTS: usize = 60;
const ANDROID_INPUT_INTERVAL: Duration = Duration::from_millis(500);
// The worker runs `waydroid session stop` right before opening the bridge,
// so the container is usually still tearing Android down when the helper
// starts. Wait out that window instead of failing on the first probe.
const CONTAINER_STOP_ATTEMPTS: usize = 30;
const CONTAINER_STOP_INTERVAL: Duration = Duration::from_secs(1);
// Android AID_INPUT. The Waydroid system_server belongs to this group and
// InputReader cannot open a host input node that retains the host input GID.
const ANDROID_INPUT_GID: u32 = 1004;
const ANDROID_INPUT_MODE: u32 = 0o660;
const UDEV_SETTLE_TIMEOUT_ARG: &str = "--timeout=5";
const LXC_PATH: &str = "/var/lib/waydroid/lxc";
const WAYDROID_CONTAINER_NAME: &str = "waydroid";
const SAFE_SYSTEM_PATH: &str = "/usr/sbin:/usr/bin";
const MAX_STAGED_HELPER_BYTES: u64 = 64 * 1024 * 1024;
pub const DEFAULT_PRIVILEGED_BRIDGE_HELPER: &str = "/usr/lib/wroid/wroid-helper";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeHelperCommand {
    executable: PathBuf,
}

impl BridgeHelperCommand {
    pub fn production() -> io::Result<Self> {
        let helper = PathBuf::from(DEFAULT_PRIVILEGED_BRIDGE_HELPER);
        validate_installed_bridge_helper(&helper)?;
        Ok(Self { executable: helper })
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn production_release(staged: &Path, expected_uid: u32) -> io::Result<Self> {
        validate_staged_helper_release(staged, expected_uid)?;
        let installed = Path::new(DEFAULT_PRIVILEGED_BRIDGE_HELPER);
        if !release_files_match(installed, staged)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "installed Wroid helper differs from the paired staged release",
            ));
        }
        validate_installed_bridge_helper(installed)?;
        Ok(Self {
            executable: installed.to_path_buf(),
        })
    }
}

fn open_release_file(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

fn validate_staged_helper_release(path: &Path, expected_uid: u32) -> io::Result<()> {
    let file = open_release_file(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot open paired staged Wroid helper {}: {error}",
                path.display()
            ),
        )
    })?;
    let metadata = file.metadata()?;
    let valid = metadata.is_file()
        && metadata.uid() == expected_uid
        && metadata.permissions().mode() & 0o7777 == 0o555
        && metadata.nlink() == 1
        && metadata.len() <= MAX_STAGED_HELPER_BYTES;
    if !valid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "paired staged Wroid helper must be a current-user-owned, single-link mode 0555 regular file no larger than 64 MiB: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn release_files_match(installed: &Path, staged: &Path) -> io::Result<bool> {
    let mut installed = open_release_file(installed)?;
    let mut staged = open_release_file(staged)?;
    let installed_len = installed.metadata()?.len();
    let staged_len = staged.metadata()?.len();
    if installed_len > MAX_STAGED_HELPER_BYTES || staged_len > MAX_STAGED_HELPER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Wroid helper release exceeds the 64 MiB safety limit",
        ));
    }
    if installed_len != staged_len {
        return Ok(false);
    }

    let mut installed_bytes = [0_u8; 8192];
    let mut staged_bytes = [0_u8; 8192];
    loop {
        let installed_read = installed.read(&mut installed_bytes)?;
        let staged_read = staged.read(&mut staged_bytes)?;
        if installed_read != staged_read
            || installed_bytes[..installed_read] != staged_bytes[..staged_read]
        {
            return Ok(false);
        }
        if installed_read == 0 {
            return Ok(true);
        }
    }
}

pub fn validate_installed_bridge_helper(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "cannot inspect installed Wroid bridge helper {}: {error}",
                path.display()
            ),
        )
    })?;
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "installed bridge helper path has no parent",
        )
    })?;
    let parent_metadata = fs::symlink_metadata(parent)?;
    if !helper_metadata_is_safe(
        metadata.file_type().is_file(),
        metadata.uid(),
        metadata.permissions().mode(),
        parent_metadata.file_type().is_dir(),
        parent_metadata.uid(),
        parent_metadata.permissions().mode(),
    ) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "refusing unsafe Wroid bridge helper {}; it must be root-owned mode 4750 and {} must be root-owned and non-writable",
                path.display(),
                parent.display(),
            ),
        ));
    }
    probe_privileged_bridge_helper(path)
}

fn helper_metadata_is_safe(
    file_is_regular: bool,
    file_uid: u32,
    file_mode: u32,
    parent_is_directory: bool,
    parent_uid: u32,
    parent_mode: u32,
) -> bool {
    file_is_regular
        && file_uid == 0
        && file_mode & 0o7777 == 0o4750
        && parent_is_directory
        && parent_uid == 0
        && parent_mode & 0o022 == 0
}

/// Hands the Wroid touchscreen node to Android's AID_INPUT group while a
/// session runs, restoring the original ownership on cleanup.
///
/// Shared by the privileged helper and the in-process root session path:
/// without this handoff Android's InputReader cannot open a host node that
/// still carries the host input GID, and touches silently never arrive.
pub(crate) struct AndroidInputAccess {
    file: fs::File,
    original_uid: u32,
    original_gid: u32,
    original_mode: u32,
    active: bool,
}

impl AndroidInputAccess {
    /// Validate that the node still is the Wroid touchscreen and hand its
    /// ownership to Android. Callers must have let udev settle first so the
    /// device identity is stable across the handoff.
    pub(crate) fn prepare(node: &InputDeviceNode) -> io::Result<Self> {
        validate_wroid_touchscreen_node(node)?;
        Self::prepare_unvalidated(node)
    }

    fn prepare_unvalidated(node: &InputDeviceNode) -> io::Result<Self> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(node.path())?;
        let metadata = file.metadata()?;
        let access = Self {
            file,
            original_uid: metadata.uid(),
            original_gid: metadata.gid(),
            original_mode: metadata.permissions().mode() & 0o7777,
            active: true,
        };
        access.set(access.original_uid, ANDROID_INPUT_GID, ANDROID_INPUT_MODE)?;
        Ok(access)
    }

    fn set(&self, uid: u32, gid: u32, mode: u32) -> io::Result<()> {
        let fd = self.file.as_raw_fd();
        // SAFETY: fd is owned by this guard and uid/gid/mode are fixed or
        // captured from the validated Wroid virtual touchscreen inode.
        unsafe {
            if libc::fchown(fd, uid, gid) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::fchmod(fd, mode as libc::mode_t) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    fn restore(&self) -> io::Result<()> {
        self.set(self.original_uid, self.original_gid, self.original_mode)
    }

    pub(crate) fn cleanup(mut self) -> io::Result<()> {
        self.restore()?;
        self.active = false;
        Ok(())
    }
}

impl Drop for AndroidInputAccess {
    fn drop(&mut self) {
        if self.active {
            let _ = self.restore();
        }
    }
}

pub fn run_privileged_bridge_helper(event_node: PathBuf) -> io::Result<()> {
    ensure_root("Wroid privileged input bridge helper")?;
    assume_full_root_identity()?;
    // SAFETY: changing this process-local umask has no memory-safety
    // preconditions and keeps privileged bridge artifacts deterministic.
    unsafe {
        libc::umask(0o022);
    }
    let _lease = WaydroidBridgeLease::acquire_default("privileged bridge helper")?;
    wait_for_container_stopped_privileged()?;
    remove_default_bridge()?;

    let node = InputDeviceNode::from_path(event_node)?;
    validate_wroid_touchscreen_node(&node)?;
    settle_wroid_input_udev()?;
    // udev may have completed device initialization while we waited. Recheck
    // the identity immediately before the privileged permission handoff.
    validate_wroid_touchscreen_node(&node)?;
    let input_access = AndroidInputAccess::prepare(&node)?;
    let bridge = InstalledWaydroidBridge::install_default(&node)?;

    println!("{READY_LINE}");
    io::stdout().flush()?;

    let mut commands = io::stdin().lock();
    let mut replies = io::stdout().lock();
    let protocol_result = serve_helper_protocol(
        &mut commands,
        &mut replies,
        wait_for_android_input_privileged,
    );
    let stop_result = if matches!(protocol_result, Ok(true)) {
        Ok(())
    } else {
        force_stop_waydroid_container()
    };
    let bridge_result = bridge.cleanup();
    let input_access_result = input_access.cleanup();
    let bridge_result = combine_bridge_cleanup_results(bridge_result, input_access_result);
    let cleanup_result = combine_helper_cleanup_results(stop_result, bridge_result);
    match (protocol_result, cleanup_result) {
        (Ok(_), cleanup_result) => cleanup_result,
        (Err(protocol_error), Ok(())) => Err(protocol_error),
        (Err(protocol_error), Err(cleanup_error)) => Err(io::Error::other(format!(
            "helper protocol failed: {protocol_error}; cleanup also failed: {cleanup_error}"
        ))),
    }
}

pub fn run_privileged_bridge_helper_check() -> io::Result<()> {
    ensure_root("Wroid privileged input bridge helper")?;
    assume_full_root_identity()?;
    println!("{CHECK_LINE}");
    Ok(())
}

fn assume_full_root_identity() -> io::Result<()> {
    // A setuid launch starts with effective UID 0 but retains the caller as
    // its real UID. LXC's command-line tools reject that mixed identity even
    // though the helper itself has already proved effective root.
    // SAFETY: the helper is already effective root; these calls affect only
    // this short-lived process and use fixed root IDs with no user input.
    unsafe {
        if libc::setgroups(0, std::ptr::null()) != 0
            || libc::setresgid(0, 0, 0) != 0
            || libc::setresuid(0, 0, 0) != 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelperProtocolCommand {
    VerifyAndroidInput,
    Cleanup,
}

fn read_helper_command<R: BufRead>(reader: &mut R) -> io::Result<Option<HelperProtocolCommand>> {
    let mut command = Vec::new();
    reader
        .take(MAX_HELPER_COMMAND_BYTES + 1)
        .read_until(b'\n', &mut command)?;
    if command.is_empty() {
        return Ok(None);
    }
    if command.len() as u64 > MAX_HELPER_COMMAND_BYTES || !command.ends_with(b"\n") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid Wroid helper protocol command",
        ));
    }
    match command.as_slice() {
        VERIFY_ANDROID_INPUT_COMMAND => Ok(Some(HelperProtocolCommand::VerifyAndroidInput)),
        CLEANUP_COMMAND => Ok(Some(HelperProtocolCommand::Cleanup)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid Wroid helper protocol command",
        )),
    }
}

fn serve_helper_protocol<R, W, F>(
    reader: &mut R,
    writer: &mut W,
    mut verify_android_input: F,
) -> io::Result<bool>
where
    R: BufRead,
    W: Write,
    F: FnMut() -> io::Result<()>,
{
    let mut verified = false;
    loop {
        match read_helper_command(reader)? {
            Some(HelperProtocolCommand::VerifyAndroidInput) if !verified => {
                verify_android_input()?;
                writeln!(writer, "{ANDROID_INPUT_READY_LINE}")?;
                writer.flush()?;
                verified = true;
            }
            Some(HelperProtocolCommand::VerifyAndroidInput) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Android input verification was already requested",
                ));
            }
            Some(HelperProtocolCommand::Cleanup) => return Ok(true),
            None => return Ok(false),
        }
    }
}

fn force_stop_waydroid_container() -> io::Result<()> {
    if ensure_container_stopped_privileged().is_ok() {
        return Ok(());
    }
    let output = lxc_stop_command().output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "privileged Waydroid recovery stop failed\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    ensure_container_stopped_privileged()
}

fn ensure_container_stopped_privileged() -> io::Result<()> {
    let output = lxc_status_command().output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "privileged Waydroid LXC status failed\n{}",
            combined_output(&output)
        )));
    }
    if combined_output(&output).trim() == "STOPPED" {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Waydroid container is running. Stop it first with: waydroid session stop",
    ))
}

/// Wait for the Waydroid container to finish stopping, bounded by
/// [`CONTAINER_STOP_ATTEMPTS`]. Failing on the very first status probe killed
/// the helper during the worker's session teardown and surfaced as a bare
/// EPIPE in the game session; only a container that stays up is an error.
fn wait_for_container_stopped_privileged() -> io::Result<()> {
    let mut last_status = String::new();
    for _ in 0..CONTAINER_STOP_ATTEMPTS {
        let output = lxc_status_command().output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "privileged Waydroid LXC status failed\n{}",
                combined_output(&output)
            )));
        }
        last_status = combined_output(&output);
        if last_status.trim() == "STOPPED" {
            return Ok(());
        }
        sleep(CONTAINER_STOP_INTERVAL);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "Waydroid container is still running after waiting {} seconds. \
             Stop it first with: waydroid session stop\n{last_status}",
            CONTAINER_STOP_ATTEMPTS
        ),
    ))
}

fn fixed_privileged_command(executable: &str, arguments: &[&str]) -> Command {
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .env_clear()
        .env("PATH", SAFE_SYSTEM_PATH)
        .env("HOME", "/root")
        .env("LANG", "C")
        .current_dir("/")
        .stdin(Stdio::null());
    command
}

fn udev_settle_command() -> Command {
    fixed_privileged_command("/usr/bin/udevadm", &["settle", UDEV_SETTLE_TIMEOUT_ARG])
}

pub(crate) fn settle_wroid_input_udev() -> io::Result<()> {
    let output = udev_settle_command().output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "udev did not settle the Wroid touchscreen before Android input permission handoff
{}",
        combined_output(&output).trim()
    )))
}

fn android_input_probe_command() -> Command {
    fixed_privileged_command(
        "/usr/bin/lxc-attach",
        &[
            "-P",
            LXC_PATH,
            "-n",
            WAYDROID_CONTAINER_NAME,
            "--clear-env",
            "--",
            "/system/bin/getevent",
            "-pl",
        ],
    )
}

fn android_dumpsys_input_command() -> Command {
    fixed_privileged_command(
        "/usr/bin/lxc-attach",
        &[
            "-P",
            LXC_PATH,
            "-n",
            WAYDROID_CONTAINER_NAME,
            "--clear-env",
            "--",
            "/system/bin/dumpsys",
            "input",
        ],
    )
}

fn android_show_touches_off_command() -> Command {
    fixed_privileged_command(
        "/usr/bin/lxc-attach",
        &[
            "-P",
            LXC_PATH,
            "-n",
            WAYDROID_CONTAINER_NAME,
            "--clear-env",
            "--",
            "/system/bin/settings",
            "put",
            "system",
            "show_touches",
            "0",
        ],
    )
}

fn android_pointer_location_off_command() -> Command {
    fixed_privileged_command(
        "/usr/bin/lxc-attach",
        &[
            "-P",
            LXC_PATH,
            "-n",
            WAYDROID_CONTAINER_NAME,
            "--clear-env",
            "--",
            "/system/bin/settings",
            "put",
            "system",
            "pointer_location",
            "0",
        ],
    )
}

fn android_input_unfreeze_command() -> Command {
    fixed_privileged_command(
        "/usr/bin/lxc-unfreeze",
        &["-P", LXC_PATH, "-n", WAYDROID_CONTAINER_NAME],
    )
}

fn lxc_status_command() -> Command {
    fixed_privileged_command(
        "/usr/bin/lxc-info",
        &["-P", LXC_PATH, "-n", WAYDROID_CONTAINER_NAME, "-sH"],
    )
}

fn lxc_stop_command() -> Command {
    fixed_privileged_command(
        "/usr/bin/lxc-stop",
        &["-P", LXC_PATH, "-n", WAYDROID_CONTAINER_NAME, "-k"],
    )
}

fn waydroid_status_is_frozen(status: &str) -> bool {
    status.trim() == "FROZEN"
}

fn unfreeze_waydroid_for_input_probe() -> io::Result<()> {
    let status = lxc_status_command().output()?;
    if !status.status.success() {
        return Err(io::Error::other(format!(
            "privileged Waydroid status failed before Android input probe\n{}",
            combined_output(&status)
        )));
    }
    if !waydroid_status_is_frozen(&combined_output(&status)) {
        return Ok(());
    }

    let output = android_input_unfreeze_command().output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "privileged Waydroid unfreeze failed before Android input probe\n{}",
        combined_output(&output)
    )))
}

fn wait_for_android_input_privileged() -> io::Result<()> {
    let mut last_output = String::new();
    for _ in 0..ANDROID_INPUT_ATTEMPTS {
        unfreeze_waydroid_for_input_probe()?;
        let output = android_input_probe_command().output()?;
        last_output = combined_output(&output);
        if output.status.success() && last_output.contains(WROID_TOUCHSCREEN_NAME) {
            wait_for_android_input_reader_privileged()?;
            disable_android_pointer_diagnostics_privileged()?;
            return Ok(());
        }
        sleep(ANDROID_INPUT_INTERVAL);
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "Android getevent did not list {WROID_TOUCHSCREEN_NAME}; device bridge is not active\n{}",
            tail_for_error(&last_output)
        ),
    ))
}

/// Verify Android's InputReader opened the bridged touchscreen.
///
/// `getevent -pl` only proves the node is visible inside the container;
/// `dumpsys input` proves system_server could actually open it. A device the
/// InputReader rejects (typically because the node still carries the host
/// input GID instead of AID_INPUT) stays EventHub-listed but never delivers
/// a single touch. Runs privileged because `waydroid shell` requires root
/// and the daemon worker is unprivileged.
fn wait_for_android_input_reader_privileged() -> io::Result<()> {
    let mut last_output = String::new();
    for _ in 0..ANDROID_INPUT_ATTEMPTS {
        unfreeze_waydroid_for_input_probe()?;
        let output = android_dumpsys_input_command().output()?;
        last_output = combined_output(&output);
        if output.status.success()
            && crate::waydroid_session::dumpsys_lists_input_reader_device(
                &last_output,
                WROID_TOUCHSCREEN_NAME,
            )
        {
            return Ok(());
        }
        sleep(ANDROID_INPUT_INTERVAL);
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "Android InputReader did not register {WROID_TOUCHSCREEN_NAME}; the event node is \
             visible but unreadable for system_server (expected host group 1004/AID_INPUT, mode \
             0660)\n{}",
            tail_for_error(&last_output)
        ),
    ))
}

/// Keep error output bounded: `dumpsys input` can exceed tens of kilobytes
/// while only its tail matters for triage.
fn tail_for_error(output: &str) -> String {
    const MAX_ERROR_CHARS: usize = 4 * 1024;
    let char_count = output.chars().count();
    if char_count <= MAX_ERROR_CHARS {
        return output.to_owned();
    }
    output
        .chars()
        .skip(char_count - MAX_ERROR_CHARS)
        .collect::<String>()
}

fn disable_android_pointer_diagnostics_privileged() -> io::Result<()> {
    disable_android_pointer_diagnostics_with(
        || run_fixed_android_setting_command("show_touches", android_show_touches_off_command()),
        || {
            run_fixed_android_setting_command(
                "pointer_location",
                android_pointer_location_off_command(),
            )
        },
    )
}

fn disable_android_pointer_diagnostics_with(
    disable_show_touches: impl FnOnce() -> io::Result<()>,
    disable_pointer_location: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    disable_show_touches()?;
    disable_pointer_location()
}

fn run_fixed_android_setting_command(name: &str, command: Command) -> io::Result<()> {
    run_fixed_android_setting_command_with_timeout(name, command, ANDROID_SETTINGS_TIMEOUT)
}

fn run_fixed_android_setting_command_with_timeout(
    name: &str,
    mut command: Command,
    timeout: Duration,
) -> io::Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("failed to start fixed Android pointer diagnostic cleanup for {name}: {error}"),
        )
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        io::Error::other(format!(
            "stdout unavailable while disabling Android setting {name}"
        ))
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        io::Error::other(format!(
            "stderr unavailable while disabling Android setting {name}"
        ))
    })?;
    let stdout_reader = thread::spawn(move || read_android_setting_output(stdout));
    let stderr_reader = thread::spawn(move || read_android_setting_output(stderr));
    let started = Instant::now();

    let status = loop {
        if stdout_reader.is_finished() && stderr_reader.is_finished() {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => {
                    kill_android_setting_process_group(&mut child);
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(io::Error::new(
                        error.kind(),
                        format!("failed to poll Android setting cleanup for {name}: {error}"),
                    ));
                }
            }
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            kill_android_setting_process_group(&mut child);
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "timed out after {:.2} seconds while disabling Android setting {name}",
                    timeout.as_secs_f64()
                ),
            ));
        }
        sleep(ANDROID_SETTINGS_POLL_INTERVAL.min(timeout - elapsed));
    };

    let stdout = join_android_setting_output(stdout_reader, name, "stdout")?;
    let stderr = join_android_setting_output(stderr_reader, name, "stderr")?;
    let output = Output {
        status,
        stdout,
        stderr,
    };
    if output.status.success() {
        return Ok(());
    }
    let detail = combined_output(&output)
        .chars()
        .take(MAX_ANDROID_SETTINGS_ERROR_CHARS)
        .collect::<String>();
    Err(io::Error::other(format!(
        "failed to disable fixed Android pointer diagnostic {name} ({})\n{}",
        output.status,
        detail.trim()
    )))
}

fn read_android_setting_output(mut stream: impl Read) -> io::Result<Vec<u8>> {
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Ok(retained);
        }
        let remaining = MAX_ANDROID_SETTINGS_ERROR_CHARS.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..remaining.min(count)]);
    }
}

fn join_android_setting_output(
    reader: thread::JoinHandle<io::Result<Vec<u8>>>,
    name: &str,
    label: &str,
) -> io::Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| {
            io::Error::other(format!(
                "{label} reader panicked for Android setting {name}"
            ))
        })?
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to read {label} while disabling Android setting {name}: {error}"),
            )
        })
}

fn kill_android_setting_process_group(child: &mut Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: the child owns a new process group whose ID is its PID.
        if unsafe { libc::kill(-pid, libc::SIGKILL) } != 0 {
            let _ = child.kill();
        }
    } else {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn request_android_input_verification<W: Write, R: BufRead>(
    writer: &mut W,
    reader: &mut R,
) -> io::Result<()> {
    writer.write_all(VERIFY_ANDROID_INPUT_COMMAND)?;
    writer.flush()?;
    let mut reply = Vec::new();
    reader
        .take(MAX_HELPER_COMMAND_BYTES + 1)
        .read_until(b'\n', &mut reply)?;
    if reply.as_slice() == format!("{ANDROID_INPUT_READY_LINE}\n").as_bytes() {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "privileged bridge helper did not verify the Android input device",
    ))
}

fn combined_output(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

fn probe_privileged_bridge_helper(path: &Path) -> io::Result<()> {
    let output = Command::new(path)
        .arg("--check")
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to execute Wroid bridge helper check: {error}"),
            )
        })?;
    if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == CHECK_LINE {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "Wroid bridge helper setuid check failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    ))
}

fn combine_bridge_cleanup_results(
    bridge_result: io::Result<()>,
    input_access_result: io::Result<()>,
) -> io::Result<()> {
    match (bridge_result, input_access_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(bridge_error), Err(input_error)) => Err(io::Error::other(format!(
            "bridge cleanup failed: {bridge_error}; input node access restore also failed: {input_error}"
        ))),
    }
}

fn combine_helper_cleanup_results(
    stop_result: io::Result<()>,
    bridge_result: io::Result<()>,
) -> io::Result<()> {
    match (stop_result, bridge_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(stop_error), Err(bridge_error)) => Err(io::Error::other(format!(
            "Waydroid recovery stop failed: {stop_error}; bridge cleanup also failed: {bridge_error}"
        ))),
    }
}

pub struct PrivilegedBridgeHelper {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    active: bool,
}

impl PrivilegedBridgeHelper {
    pub fn start(helper: &BridgeHelperCommand, event_node: &Path) -> io::Result<Self> {
        let arguments = helper_arguments(event_node);
        let mut command = Command::new(helper.executable());
        command
            .args(arguments)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        command.process_group(0);
        let mut child = command.spawn().map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to start the Wroid privileged bridge helper: {error}"),
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("bridge helper stdin pipe is unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("bridge helper stdout pipe is unavailable"))?;
        let mut reader = BufReader::new(stdout);
        let mut ready = String::new();
        let read_result = reader.read_line(&mut ready);
        if let Err(error) = read_result {
            drop(stdin);
            let _ = child.wait();
            return Err(error);
        }
        if ready.trim_end() != READY_LINE {
            drop(stdin);
            let status = child.wait()?;
            return Err(io::Error::other(format!(
                "privileged bridge helper did not become ready (exit {status})"
            )));
        }
        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout: reader,
            active: true,
        })
    }

    pub fn verify_android_input(&mut self) -> io::Result<()> {
        let stdin = self.stdin.as_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "bridge helper stdin pipe is unavailable",
            )
        })?;
        request_android_input_verification(stdin, &mut self.stdout)
    }

    pub fn finish(mut self, waydroid_stopped: bool) -> io::Result<()> {
        if waydroid_stopped {
            if let Some(stdin) = self.stdin.as_mut() {
                stdin.write_all(CLEANUP_COMMAND)?;
                stdin.flush()?;
            }
        }
        self.stdin.take();
        let status = self.child.wait()?;
        self.active = false;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "privileged bridge helper exited with {status}"
            )))
        }
    }
}

impl Drop for PrivilegedBridgeHelper {
    fn drop(&mut self) {
        if self.active {
            self.stdin.take();
            let _ = self.child.wait();
        }
    }
}

fn helper_arguments(event_node: &Path) -> Vec<OsString> {
    vec![
        OsString::from("--event-node"),
        event_node.as_os_str().to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::os::unix::fs::symlink;

    #[test]
    fn helper_protocol_parses_only_fixed_line_commands() {
        let mut commands = Cursor::new(b"VERIFY_ANDROID_INPUT\nCLEANUP\n");

        assert_eq!(
            read_helper_command(&mut commands).unwrap(),
            Some(HelperProtocolCommand::VerifyAndroidInput)
        );
        assert_eq!(
            read_helper_command(&mut commands).unwrap(),
            Some(HelperProtocolCommand::Cleanup)
        );
        assert_eq!(read_helper_command(&mut commands).unwrap(), None);

        let mut unknown = Cursor::new(b"CLEANUP NOW\n");
        assert_eq!(
            read_helper_command(&mut unknown).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );

        let mut oversized = Cursor::new(vec![b'x'; 65]);
        assert_eq!(
            read_helper_command(&mut oversized).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn helper_protocol_android_probe_has_fixed_arguments() {
        let command = android_input_probe_command();

        assert_eq!(command.get_program(), "/usr/bin/lxc-attach");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "-P",
                "/var/lib/waydroid/lxc",
                "-n",
                "waydroid",
                "--clear-env",
                "--",
                "/system/bin/getevent",
                "-pl",
            ]
            .map(std::ffi::OsStr::new)
        );

        let command = android_input_unfreeze_command();
        assert_eq!(command.get_program(), "/usr/bin/lxc-unfreeze");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["-P", "/var/lib/waydroid/lxc", "-n", "waydroid"].map(std::ffi::OsStr::new)
        );

        assert!(waydroid_status_is_frozen("FROZEN\n"));
        assert!(!waydroid_status_is_frozen("RUNNING\n"));

        let show_touches = android_show_touches_off_command();
        assert_eq!(show_touches.get_program(), "/usr/bin/lxc-attach");
        assert_eq!(
            show_touches.get_args().collect::<Vec<_>>(),
            [
                "-P",
                "/var/lib/waydroid/lxc",
                "-n",
                "waydroid",
                "--clear-env",
                "--",
                "/system/bin/settings",
                "put",
                "system",
                "show_touches",
                "0",
            ]
            .map(std::ffi::OsStr::new)
        );

        let pointer_location = android_pointer_location_off_command();
        assert_eq!(pointer_location.get_program(), "/usr/bin/lxc-attach");
        assert_eq!(
            pointer_location.get_args().collect::<Vec<_>>(),
            [
                "-P",
                "/var/lib/waydroid/lxc",
                "-n",
                "waydroid",
                "--clear-env",
                "--",
                "/system/bin/settings",
                "put",
                "system",
                "pointer_location",
                "0",
            ]
            .map(std::ffi::OsStr::new)
        );
    }

    #[test]
    fn helper_protocol_verifies_android_before_graceful_cleanup() {
        let mut commands = Cursor::new(b"VERIFY_ANDROID_INPUT\nCLEANUP\n");
        let mut replies = Vec::new();
        let mut probes = 0;

        let graceful = serve_helper_protocol(&mut commands, &mut replies, || {
            probes += 1;
            Ok(())
        })
        .unwrap();

        assert!(graceful);
        assert_eq!(probes, 1);
        assert_eq!(replies, b"WROID_ANDROID_INPUT_READY 1\n");
    }

    #[test]
    fn helper_protocol_withholds_ready_when_android_cleanup_fails() {
        let mut commands = Cursor::new(b"VERIFY_ANDROID_INPUT\n");
        let mut replies = Vec::new();

        let error = serve_helper_protocol(&mut commands, &mut replies, || {
            Err(io::Error::other("pointer cleanup failed"))
        })
        .unwrap_err();

        assert_eq!(error.to_string(), "pointer cleanup failed");
        assert!(replies.is_empty());
    }

    #[test]
    fn pointer_diagnostic_cleanup_runs_both_fixed_operations() {
        let calls = std::cell::RefCell::new(Vec::new());

        disable_android_pointer_diagnostics_with(
            || {
                calls.borrow_mut().push("show_touches");
                Ok(())
            },
            || {
                calls.borrow_mut().push("pointer_location");
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(*calls.borrow(), ["show_touches", "pointer_location"]);
    }

    #[test]
    fn pointer_setting_timeout_is_bounded_and_names_the_setting() {
        let mut command = Command::new("/usr/bin/sh");
        command.args(["-c", "sleep 30"]);
        let started = std::time::Instant::now();

        let error = run_fixed_android_setting_command_with_timeout(
            "pointer_location",
            command,
            Duration::from_millis(50),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("pointer_location"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn pointer_setting_timeout_kills_descendants_holding_output_pipes() {
        let mut command = Command::new("/usr/bin/sh");
        command.args(["-c", "sleep 30 &"]);
        let started = std::time::Instant::now();

        let error = run_fixed_android_setting_command_with_timeout(
            "show_touches",
            command,
            Duration::from_millis(50),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn pointer_setting_spawn_error_names_the_setting() {
        let command = Command::new("/definitely/missing/wroid-setting-command");

        let error = run_fixed_android_setting_command_with_timeout(
            "show_touches",
            command,
            Duration::from_secs(1),
        )
        .unwrap_err();

        assert!(error.to_string().contains("show_touches"));
    }

    #[test]
    fn helper_protocol_rejects_duplicate_android_verification() {
        let mut commands = Cursor::new(b"VERIFY_ANDROID_INPUT\nVERIFY_ANDROID_INPUT\n");
        let mut replies = Vec::new();

        assert_eq!(
            serve_helper_protocol(&mut commands, &mut replies, || Ok(()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn helper_protocol_client_requires_exact_android_ready_reply() {
        let mut request = Vec::new();
        let mut ready = Cursor::new(b"WROID_ANDROID_INPUT_READY 1\n");
        request_android_input_verification(&mut request, &mut ready).unwrap();
        assert_eq!(request, b"VERIFY_ANDROID_INPUT\n");

        let mut rejected_request = Vec::new();
        let mut malformed = Cursor::new(b"READY MAYBE\n");
        assert_eq!(
            request_android_input_verification(&mut rejected_request, &mut malformed)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn helper_arguments_are_typed_and_do_not_contain_shell_fragments() {
        assert_eq!(
            helper_arguments(Path::new("/dev/input/event42")),
            ["--event-node", "/dev/input/event42"].map(OsString::from)
        );
    }

    #[test]
    fn helper_cleanup_preserves_both_failures() {
        let error = combine_helper_cleanup_results(
            Err(io::Error::other("stop failed")),
            Err(io::Error::other("bridge failed")),
        )
        .unwrap_err();
        assert!(error.to_string().contains("stop failed"));
        assert!(error.to_string().contains("bridge failed"));
    }

    #[test]
    fn production_helper_requires_a_root_owned_non_writable_chain() {
        assert!(helper_metadata_is_safe(
            true, 0, 0o104750, true, 0, 0o040755
        ));
        assert!(!helper_metadata_is_safe(
            true, 1000, 0o104750, true, 0, 0o040755
        ));
        assert!(!helper_metadata_is_safe(
            true, 0, 0o104770, true, 0, 0o040755
        ));
        assert!(!helper_metadata_is_safe(
            true, 0, 0o104750, true, 1000, 0o040755
        ));
        assert!(!helper_metadata_is_safe(
            true, 0, 0o104750, true, 0, 0o040777
        ));
        assert!(!helper_metadata_is_safe(
            true, 0, 0o100750, true, 0, 0o040755
        ));
        assert!(!helper_metadata_is_safe(
            true, 0, 0o104755, true, 0, 0o040755
        ));
    }

    #[test]
    fn staged_release_requires_a_user_owned_non_writable_regular_file() {
        let directory = tempfile::tempdir().unwrap();
        let staged = directory.path().join("wroid-helper");
        fs::write(&staged, b"paired helper release").unwrap();
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o555)).unwrap();
        // SAFETY: geteuid takes no arguments and has no preconditions.
        let uid = unsafe { libc::geteuid() };

        validate_staged_helper_release(&staged, uid).unwrap();

        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            validate_staged_helper_release(&staged, uid)
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );

        fs::set_permissions(&staged, fs::Permissions::from_mode(0o555)).unwrap();
        let link = directory.path().join("wroid-helper-link");
        symlink(&staged, &link).unwrap();
        assert!(validate_staged_helper_release(&link, uid).is_err());
        assert!(validate_staged_helper_release(&staged, uid.saturating_add(1)).is_err());
    }

    #[test]
    fn staged_release_match_is_bounded_and_byte_exact() {
        let directory = tempfile::tempdir().unwrap();
        let installed = directory.path().join("installed");
        let staged = directory.path().join("staged");
        fs::write(&installed, b"same helper bytes").unwrap();
        fs::write(&staged, b"same helper bytes").unwrap();

        assert!(release_files_match(&installed, &staged).unwrap());

        fs::write(&installed, b"same helper byte!").unwrap();
        assert!(!release_files_match(&installed, &staged).unwrap());
        fs::write(&installed, b"short").unwrap();
        assert!(!release_files_match(&installed, &staged).unwrap());

        let oversized = fs::File::create(&staged).unwrap();
        oversized.set_len(MAX_STAGED_HELPER_BYTES + 1).unwrap();
        assert_eq!(
            release_files_match(&installed, &staged).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn privileged_lxc_commands_use_absolute_binaries_and_fixed_arguments() {
        let udev = udev_settle_command();
        assert_eq!(udev.get_program(), "/usr/bin/udevadm");
        assert_eq!(
            udev.get_args().collect::<Vec<_>>(),
            ["settle", "--timeout=5"].map(std::ffi::OsStr::new)
        );

        let command = lxc_status_command();
        assert_eq!(command.get_program(), "/usr/bin/lxc-info");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["-P", "/var/lib/waydroid/lxc", "-n", "waydroid", "-sH"].map(std::ffi::OsStr::new)
        );

        let command = lxc_stop_command();
        assert_eq!(command.get_program(), "/usr/bin/lxc-stop");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["-P", "/var/lib/waydroid/lxc", "-n", "waydroid", "-k"].map(std::ffi::OsStr::new)
        );
        let environment = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            environment.get("PATH").and_then(Option::as_deref),
            Some(SAFE_SYSTEM_PATH)
        );
        assert_eq!(
            environment.get("HOME").and_then(Option::as_deref),
            Some("/root")
        );
        assert_eq!(command.get_current_dir(), Some(Path::new("/")));
    }
}
