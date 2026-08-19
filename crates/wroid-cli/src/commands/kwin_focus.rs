use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use dbus::blocking::{Connection, SyncConnection};
use dbus::channel::MatchingReceiver;
use dbus::message::MatchRule;

const DBUS_TIMEOUT: Duration = Duration::from_secs(5);
const LOOP_INTERVAL: Duration = Duration::from_millis(20);

pub(crate) struct KwinFocusRelay {
    socket_path: PathBuf,
    stop: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl KwinFocusRelay {
    pub(crate) fn start() -> Result<Self> {
        ensure_supported_desktop()?;
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .context("XDG_RUNTIME_DIR is unavailable")?;
        let relay_dir = runtime_dir.join("wroid");
        fs::create_dir_all(&relay_dir)
            .with_context(|| format!("failed to create {}", relay_dir.display()))?;
        fs::set_permissions(&relay_dir, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("failed to secure {}", relay_dir.display()))?;

        let token = random_token()?;
        let socket_path = relay_dir.join(format!("focus-{token}.sock"));
        let script_path = relay_dir.join(format!("focus-{token}.js"));
        let plugin_name = format!("wroid-focus-{token}");
        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("failed to bind {}", socket_path.display()))?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure {}", socket_path.display()))?;
        listener
            .set_nonblocking(true)
            .context("failed to configure the focus relay socket")?;

        let (stop_tx, stop_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread_socket = socket_path.clone();
        let thread_handle = thread::spawn(move || {
            let startup_error = ready_tx.clone();
            let result = run_relay(listener, &script_path, &plugin_name, stop_rx, ready_tx);
            if let Err(error) = result {
                let _ = startup_error.send(Err(format!("{error:#}")));
                eprintln!("Wroid focus relay stopped: {error:#}");
            }
            let _ = fs::remove_file(script_path);
            let _ = fs::remove_file(thread_socket);
        });

        match ready_rx.recv_timeout(DBUS_TIMEOUT + Duration::from_secs(1)) {
            Ok(Ok(())) => Ok(Self {
                socket_path,
                stop: stop_tx,
                thread: Some(thread_handle),
            }),
            Ok(Err(message)) => {
                let _ = stop_tx.send(());
                let _ = thread_handle.join();
                Err(anyhow!(message))
            }
            Err(error) => {
                let _ = stop_tx.send(());
                let _ = thread_handle.join();
                Err(anyhow!("focus relay did not become ready: {error}"))
            }
        }
    }

    pub(crate) fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

impl Drop for KwinFocusRelay {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_relay(
    listener: UnixListener,
    script_path: &Path,
    plugin_name: &str,
    stop: mpsc::Receiver<()>,
    ready: mpsc::Sender<Result<(), String>>,
) -> Result<()> {
    let callback_connection =
        SyncConnection::new_session().context("failed to connect to the desktop D-Bus")?;
    let callback_address = callback_connection.unique_name().to_string();
    let script = focus_script(&callback_address);
    fs::write(script_path, script)
        .with_context(|| format!("failed to write {}", script_path.display()))?;
    fs::set_permissions(script_path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to secure {}", script_path.display()))?;

    let kwin_connection =
        Connection::new_session().context("failed to connect to KWin over D-Bus")?;
    let kwin_proxy = kwin_connection.with_proxy("org.kde.KWin", "/Scripting", DBUS_TIMEOUT);
    let (script_id,): (i32,) = kwin_proxy
        .method_call(
            "org.kde.kwin.Scripting",
            "loadScript",
            (script_path.to_string_lossy().as_ref(), plugin_name),
        )
        .context("KWin rejected the focus protection script")?;
    if script_id < 0 {
        bail!("KWin could not load the focus protection script");
    }

    let script_proxy = kwin_connection.with_proxy(
        "org.kde.KWin",
        format!("/Scripting/Script{script_id}"),
        DBUS_TIMEOUT,
    );
    if let Err(error) = script_proxy.method_call::<(), _, _, _>("org.kde.kwin.Script", "run", ()) {
        let _: Result<(bool,), _> =
            kwin_proxy.method_call("org.kde.kwin.Scripting", "unloadScript", (plugin_name,));
        return Err(error).context("failed to start the KWin focus protection script");
    }

    let (focus_tx, focus_rx) = mpsc::channel();
    let _receiver = callback_connection.start_receive(
        MatchRule::new_method_call(),
        Box::new(move |message, _connection| {
            if message
                .member()
                .is_some_and(|member| member == "focusChanged")
            {
                if let Some(value) = message.get1::<String>() {
                    let _ = focus_tx.send(value == "focused");
                }
            }
            true
        }),
    );

    let _ = ready.send(Ok(()));
    let mut client: Option<UnixStream> = None;
    let mut last_focus = None;
    let relay_result = (|| -> Result<()> {
        while stop.try_recv().is_err() {
            callback_connection
                .process(LOOP_INTERVAL)
                .context("desktop D-Bus focus listener failed")?;
            accept_client(&listener, &mut client, last_focus)?;
            while let Ok(focused) = focus_rx.try_recv() {
                last_focus = Some(focused);
                write_focus(&mut client, focused);
            }
        }
        Ok(())
    })();

    let _: Result<(bool,), _> =
        kwin_proxy.method_call("org.kde.kwin.Scripting", "unloadScript", (plugin_name,));
    relay_result
}

fn accept_client(
    listener: &UnixListener,
    client: &mut Option<UnixStream>,
    last_focus: Option<bool>,
) -> Result<()> {
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .context("failed to configure focus client")?;
                if let Some(focused) = last_focus {
                    stream
                        .write_all(focus_line(focused))
                        .context("failed to initialize focus client")?;
                }
                *client = Some(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error).context("failed to accept focus client"),
        }
    }
}

fn write_focus(client: &mut Option<UnixStream>, focused: bool) {
    if client
        .as_mut()
        .is_some_and(|stream| stream.write_all(focus_line(focused)).is_err())
    {
        *client = None;
    }
}

const fn focus_line(focused: bool) -> &'static [u8] {
    if focused {
        b"focused\n"
    } else {
        b"unfocused\n"
    }
}

fn focus_script(callback_address: &str) -> String {
    format!(
        r#"
function isGameSurface(window) {{
    if (window == null) {{
        return false;
    }}
    const windowClass = String(window.resourceClass || "").toLowerCase();
    const windowName = String(window.resourceName || "").toLowerCase();
    const isWaydroid = value => value === "waydroid" || value.startsWith("waydroid.");
    return isWaydroid(windowClass) || isWaydroid(windowName) ||
        windowClass === "gamescope" || windowName === "gamescope";
}}

function reportFocus(window) {{
    const state = isGameSurface(window) ? "focused" : "unfocused";
    callDBus("{callback_address}", "/", "", "focusChanged", state);
}}

workspace.windowActivated.connect(reportFocus);
reportFocus(workspace.activeWindow);
"#
    )
}

fn ensure_supported_desktop() -> Result<()> {
    if std::env::var("KDE_SESSION_VERSION").as_deref() != Ok("6") {
        bail!("automatic focus protection currently requires KDE Plasma 6");
    }
    if !std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .contains("kde")
    {
        bail!("automatic focus protection currently requires a KDE desktop session");
    }
    Ok(())
}

fn random_token() -> Result<String> {
    let mut bytes = [0_u8; 12];
    fs::File::open("/dev/urandom")
        .context("failed to open the system random source")?
        .read_exact(&mut bytes)
        .context("failed to generate a focus relay token")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    #[test]
    fn script_matches_waydroid_by_stable_window_identity() {
        let script = focus_script(":1.42");
        assert!(script.contains("window.resourceClass"));
        assert!(script.contains("window.resourceName"));
        assert!(script.contains("startsWith(\"waydroid.\")"));
        assert!(script.contains("windowClass === \"gamescope\""));
        assert!(script.contains("windowName === \"gamescope\""));
        assert!(script.contains("workspace.windowActivated.connect(reportFocus)"));
        assert!(script.contains("reportFocus(workspace.activeWindow)"));
        assert!(script.contains("callDBus(\":1.42\""));
    }

    #[test]
    fn focus_wire_protocol_is_unambiguous() {
        assert_eq!(focus_line(true), b"focused\n");
        assert_eq!(focus_line(false), b"unfocused\n");
    }

    #[test]
    #[ignore = "requires a live KDE Plasma 6 session"]
    fn live_relay_reports_initial_focus() {
        let relay = KwinFocusRelay::start().unwrap();
        let stream = UnixStream::connect(relay.socket_path()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut state = String::new();
        reader.read_line(&mut state).unwrap();
        assert!(matches!(state.trim(), "focused" | "unfocused"));
    }
}
