use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::thread::sleep;
use std::time::Duration;

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
const ANDROID_INPUT_ATTEMPTS: usize = 60;
const ANDROID_INPUT_INTERVAL: Duration = Duration::from_millis(500);
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

pub fn run_privileged_bridge_helper(event_node: PathBuf) -> io::Result<()> {
    ensure_root("Wroid privileged input bridge helper")?;
    assume_full_root_identity()?;
    // SAFETY: changing this process-local umask has no memory-safety
    // preconditions and keeps privileged bridge artifacts deterministic.
    unsafe {
        libc::umask(0o022);
    }
    let _lease = WaydroidBridgeLease::acquire_default("privileged bridge helper")?;
    ensure_container_stopped_privileged()?;
    remove_default_bridge()?;

    let node = InputDeviceNode::from_path(event_node)?;
    validate_wroid_touchscreen_node(&node)?;
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
            return Ok(());
        }
        sleep(ANDROID_INPUT_INTERVAL);
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "Android getevent did not list {WROID_TOUCHSCREEN_NAME}; device bridge is not active\n{last_output}"
        ),
    ))
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
