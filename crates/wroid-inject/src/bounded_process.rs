use std::io::{self, BufRead, BufReader, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const OUTPUT_CAPTURE_LIMIT: usize = 4 * 1024 * 1024;
const OUTPUT_TRUNCATED_MARKER: &[u8] = b"\n[Wroid: output truncated after 4 MiB]\n";

pub(crate) fn output_with_timeout(
    command: &mut Command,
    timeout: Duration,
    context: &str,
) -> io::Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().map_err(|error| {
        io::Error::new(error.kind(), format!("{context}: failed to spawn: {error}"))
    })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other(format!("{context}: stdout pipe is unavailable")))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other(format!("{context}: stderr pipe is unavailable")))?;
    set_nonblocking(stdout.as_raw_fd())?;
    set_nonblocking(stderr.as_raw_fd())?;

    let deadline = deadline_after(timeout)?;
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut stdout_capture = CapturedOutput::default();
    let mut stderr_capture = CapturedOutput::default();
    let mut status = None;

    loop {
        if stdout_open {
            stdout_open = drain_available(&mut stdout, &mut stdout_capture)?;
        }
        if stderr_open {
            stderr_open = drain_available(&mut stderr, &mut stderr_capture)?;
        }
        if stdout_capture.truncated || stderr_capture.truncated {
            let _ = kill_process_group_and_reap(&mut child);
            return Err(output_limit_error(
                context,
                stdout_capture.truncated,
                stderr_capture.truncated,
            ));
        }
        if status.is_none() {
            status = child.try_wait().map_err(|error| {
                kill_process_group_best_effort(&mut child);
                io::Error::new(error.kind(), format!("{context}: failed to poll: {error}"))
            })?;
        }
        if let Some(status) = status {
            if !stdout_open && !stderr_open {
                return Ok(Output {
                    status,
                    stdout: stdout_capture.bytes,
                    stderr: stderr_capture.bytes,
                });
            }
        }
        if Instant::now() >= deadline {
            let status = kill_process_group_and_reap(&mut child).ok();
            return Err(timeout_error(context, timeout, status));
        }
        poll_output_streams(
            stdout_open.then_some(stdout.as_raw_fd()),
            stderr_open.then_some(stderr.as_raw_fd()),
            deadline,
        )?;
    }
}

pub(crate) fn wait_child_with_timeout(
    child: &mut Child,
    timeout: Duration,
    context: &str,
) -> io::Result<ExitStatus> {
    let deadline = deadline_after(timeout)?;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let status = kill_process_group_and_reap(child).ok();
            return Err(timeout_error(context, timeout, status));
        }
        thread::sleep(remaining_poll_interval(deadline));
    }
}

pub(crate) fn read_child_line_with_timeout(
    child: &mut Child,
    reader: &mut BufReader<ChildStdout>,
    timeout: Duration,
    max_bytes: usize,
    context: &str,
) -> io::Result<Vec<u8>> {
    set_nonblocking(reader.get_ref().as_raw_fd())?;
    let deadline = deadline_after(timeout)?;
    let mut line = Vec::new();

    loop {
        let mut would_block = false;
        match reader.fill_buf() {
            Ok([]) => return Ok(line),
            Ok(available) => {
                let newline = available.iter().position(|byte| *byte == b'\n');
                let used = newline.map_or(available.len(), |index| index + 1);
                if let Err(error) =
                    append_line_bytes(context, &mut line, &available[..used], max_bytes)
                {
                    let _ = kill_process_group_and_reap(child);
                    return Err(error);
                }
                reader.consume(used);
                if newline.is_some() {
                    return Ok(line);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                would_block = true;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("{context}: failed to read line: {error}"),
                ));
            }
        }

        if Instant::now() >= deadline {
            let status = kill_process_group_and_reap(child).ok();
            return Err(timeout_error(context, timeout, status));
        }
        if would_block {
            poll_single_fd(reader.get_ref().as_raw_fd(), deadline)?;
        }
    }
}

#[derive(Default)]
struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

impl CapturedOutput {
    fn append(&mut self, chunk: &[u8]) {
        if self.truncated {
            return;
        }
        let marker_reservation = OUTPUT_TRUNCATED_MARKER.len().min(OUTPUT_CAPTURE_LIMIT);
        let content_limit = OUTPUT_CAPTURE_LIMIT.saturating_sub(marker_reservation);
        if self.bytes.len() + chunk.len() <= content_limit {
            self.bytes.extend_from_slice(chunk);
            return;
        }

        let remaining = content_limit.saturating_sub(self.bytes.len());
        self.bytes.extend_from_slice(&chunk[..remaining]);
        self.bytes.extend_from_slice(OUTPUT_TRUNCATED_MARKER);
        self.truncated = true;
    }
}

fn drain_available<R: Read>(reader: &mut R, output: &mut CapturedOutput) -> io::Result<bool> {
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(false),
            Ok(count) => output.append(&buffer[..count]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(true),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

fn append_line_bytes(
    context: &str,
    line: &mut Vec<u8>,
    bytes: &[u8],
    max_bytes: usize,
) -> io::Result<()> {
    if line.len().saturating_add(bytes.len()) > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{context}: protocol line exceeded {max_bytes} byte limit"),
        ));
    }
    line.extend_from_slice(bytes);
    Ok(())
}

fn set_nonblocking(fd: libc::c_int) -> io::Result<()> {
    // SAFETY: fcntl operates on a live file descriptor borrowed from a Child
    // pipe and does not take ownership of it.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fcntl updates only the status flags for this live descriptor.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn poll_output_streams(
    stdout_fd: Option<libc::c_int>,
    stderr_fd: Option<libc::c_int>,
    deadline: Instant,
) -> io::Result<()> {
    let mut fds = Vec::with_capacity(2);
    if let Some(fd) = stdout_fd {
        fds.push(libc::pollfd {
            fd,
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        });
    }
    if let Some(fd) = stderr_fd {
        fds.push(libc::pollfd {
            fd,
            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
            revents: 0,
        });
    }
    if fds.is_empty() {
        thread::sleep(remaining_poll_interval(deadline));
        return Ok(());
    }
    poll_fds(&mut fds, deadline)
}

fn poll_single_fd(fd: libc::c_int, deadline: Instant) -> io::Result<()> {
    let mut fds = [libc::pollfd {
        fd,
        events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
        revents: 0,
    }];
    poll_fds(&mut fds, deadline)
}

fn poll_fds(fds: &mut [libc::pollfd], deadline: Instant) -> io::Result<()> {
    let timeout = poll_timeout_millis(deadline);
    // SAFETY: poll receives a valid pointer/len pair for the stack/Vec-backed
    // pollfd slice and does not retain it after returning.
    let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}

fn poll_timeout_millis(deadline: Instant) -> libc::c_int {
    let remaining = deadline.saturating_duration_since(Instant::now());
    let interval = remaining.min(PROCESS_POLL_INTERVAL);
    i32::try_from(interval.as_millis())
        .unwrap_or(i32::MAX)
        .max(1)
}

fn remaining_poll_interval(deadline: Instant) -> Duration {
    deadline
        .saturating_duration_since(Instant::now())
        .min(PROCESS_POLL_INTERVAL)
}

fn deadline_after(timeout: Duration) -> io::Result<Instant> {
    Instant::now().checked_add(timeout).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "process timeout is too large for monotonic deadline",
        )
    })
}

fn kill_process_group_and_reap(child: &mut Child) -> io::Result<ExitStatus> {
    kill_process_group_best_effort(child);
    child.wait()
}

fn kill_process_group_best_effort(child: &mut Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: children using this module are spawned into their own
        // process group; negative PID targets that group. ESRCH is harmless
        // when the child/group already exited.
        let result = unsafe { libc::kill(-pid, libc::SIGKILL) };
        if result == 0 {
            return;
        }
    }
    let _ = child.kill();
}

fn timeout_error(context: &str, timeout: Duration, status: Option<ExitStatus>) -> io::Error {
    let status_detail = status
        .map(|status| format!("; reaped with {status}"))
        .unwrap_or_else(|| "; reap status unavailable".to_owned());
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "{context}: timed out after {:.3} seconds; killed process group{status_detail}",
            timeout.as_secs_f64()
        ),
    )
}

fn output_limit_error(context: &str, stdout: bool, stderr: bool) -> io::Error {
    let stream = match (stdout, stderr) {
        (true, true) => "stdout and stderr",
        (true, false) => "stdout",
        (false, true) => "stderr",
        (false, false) => "output",
    };
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{context}: {stream} exceeded the 4 MiB capture limit; output was truncated"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    #[test]
    fn command_output_timeout_bounds_a_hung_process_boundary() {
        let mut command = Command::new("sleep");
        command.arg("0.2");
        let started = Instant::now();

        let error = output_with_timeout(&mut command, Duration::from_millis(20), "test command")
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn child_wait_timeout_bounds_a_hung_shutdown_boundary() {
        let mut child = Command::new("sleep")
            .arg("0.2")
            .process_group(0)
            .spawn()
            .unwrap();
        let started = Instant::now();

        let error = wait_child_with_timeout(&mut child, Duration::from_millis(20), "test child")
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn line_read_timeout_bounds_a_silent_helper_boundary() {
        let mut child = Command::new("sleep")
            .arg("0.2")
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let started = Instant::now();

        let error = read_child_line_with_timeout(
            &mut child,
            &mut reader,
            Duration::from_millis(20),
            64,
            "READY",
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn line_read_limit_reaps_a_protocol_overrun_child() {
        let mut child = Command::new("python3")
            .args([
                "-c",
                "import sys, time; sys.stdout.write('x' * 128); sys.stdout.flush(); time.sleep(30)",
            ])
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let started = Instant::now();

        let error = read_child_line_with_timeout(
            &mut child,
            &mut reader,
            Duration::from_secs(1),
            64,
            "READY",
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn command_output_preserves_status_stdout_and_stderr() {
        let mut command = Command::new("python3");
        command.args([
            "-c",
            "import sys; sys.stdout.write('out'); sys.stderr.write('err'); sys.exit(7)",
        ]);

        let output = output_with_timeout(&mut command, Duration::from_secs(1), "capture").unwrap();

        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout, b"out");
        assert_eq!(output.stderr, b"err");
    }
}
