# Native WebView Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Wroid Gaming Hub and Controls Studio open as native Linux windows by default while preserving their existing authenticated localhost applications and gameplay behavior.

**Architecture:** Split the loopback server lifetime from its presentation mode and return a typed `LocalWebApp` handle from Hub/editor startup. A reusable GTK 3 + WebKitGTK 4.1 shell starts the server only on first application activation, confines WebKit to the handle's exact origin, and shuts the server down when either the page or window closes; browser and headless modes reuse the same server handle.

**Tech Stack:** Rust 2021, GTK 3 (`gtk` 0.18.2), WebKitGTK 4.1 (`webkit2gtk` 2.0.2), GLib/GIO, Clap 4, existing loopback HTTP servers and Rust unit tests.

## Global Constraints

- Linux only; dynamically link the distribution `gtk3` and `webkit2gtk-4.1` packages.
- `wroid hub` and `wroid profile edit-v2` use a native window by default.
- `--browser` explicitly selects the old browser flow; `--no-open` remains headless and conflicts with `--browser`.
- Hub application ID is exactly `io.wroid.GamingHub`; Hub is single-instance and Controls Studio is non-singleton.
- Native windows default to 1280x800 and have a 1024x640 minimum.
- The WebView accepts only the exact generated `http://127.0.0.1:<port>` origin and rejects popups, other schemes, hosts, and ports.
- Keep developer extras disabled in release builds and keep the existing file chooser available for APK upload.
- Do not pass the authenticated URL through process arguments or print it in normal native-mode logs.
- Closing the native window or using the in-page close action must stop and join the loopback server; initialization/load failure must not silently open a browser.
- Do not modify the gameplay input path, Waydroid lifecycle, profile format, helper protocol, or root-detection behavior.

---

## File Structure

- Create `crates/wroid-cli/src/commands/local_web_app.rs`: presentation mode, exact-origin value, server thread handle, shutdown, wait, and bounded join.
- Create `crates/wroid-cli/src/commands/desktop_webview.rs`: GTK application/window lifecycle and WebKit policy; no Hub/editor business logic.
- Modify `crates/wroid-cli/src/commands/hub.rs`: expose Hub server startup and route the three presentation modes.
- Modify `crates/wroid-cli/src/commands/editor.rs`: expose editor server startup and route the three presentation modes.
- Modify `crates/wroid-cli/src/commands/mod.rs`: register the two focused modules and pass typed presentation modes.
- Modify `crates/wroid-cli/src/cli.rs`: native-default command copy, `--browser`, conflict validation, and parsing tests.
- Modify `crates/wroid-cli/Cargo.toml` and `Cargo.lock`: GTK/WebKit Rust bindings.
- Modify `crates/wroid-cli/src/commands/desktop.rs`: lock the launcher contract to native-default `wroid hub`.
- Modify `README.md`, `docs/roadmap.md`, and `docs/waydroid-notes.md`: native launch behavior and Arch runtime dependencies.

### Task 1: Typed local application lifetime

**Files:**
- Create: `crates/wroid-cli/src/commands/local_web_app.rs`
- Modify: `crates/wroid-cli/src/commands/mod.rs`

**Interfaces:**
- Produces: `WebUiMode::{Native, Browser, Headless}`.
- Produces: `LocalOrigin::new(SocketAddr) -> Result<LocalOrigin>`, `LocalOrigin::as_str() -> &str`, and `LocalOrigin::allows_uri(&str) -> bool`.
- Produces: `LocalWebApp::spawn(SocketAddr, String, Arc<AtomicBool>, F) -> Result<LocalWebApp> where F: FnOnce() -> Result<()> + Send + 'static`.
- Produces: `authenticated_url()`, `origin()`, `shutdown_signal()`, `is_shutdown()`, `wait()`, and `shutdown_and_join()`.

- [ ] **Step 1: Register the module and write failing value/lifecycle tests**

Add `pub(crate) mod local_web_app;` in `commands/mod.rs`. In the new file, add tests that define the intended behavior before any production types exist:

```rust
#[test]
fn exact_origin_allows_only_its_own_http_origin() {
    let origin = LocalOrigin::new("127.0.0.1:37613".parse().unwrap()).unwrap();

    assert!(origin.allows_uri("http://127.0.0.1:37613/"));
    assert!(origin.allows_uri("http://127.0.0.1:37613/api/state?token=secret"));
    assert!(!origin.allows_uri("http://127.0.0.1:37614/"));
    assert!(!origin.allows_uri("http://localhost:37613/"));
    assert!(!origin.allows_uri("https://127.0.0.1:37613/"));
    assert!(!origin.allows_uri("file:///tmp/profile.json"));
    assert!(!origin.allows_uri("data:text/html,blocked"));
    assert!(!origin.allows_uri("http://127.0.0.1:37613.evil.invalid/"));
}

#[test]
fn authenticated_url_keeps_token_in_the_handle() {
    let shutdown = Arc::new(AtomicBool::new(false));
    let app = LocalWebApp::spawn(
        "127.0.0.1:37613".parse().unwrap(),
        "private-token".to_owned(),
        Arc::clone(&shutdown),
        || Ok(()),
    )
    .unwrap();

    assert_eq!(app.origin().as_str(), "http://127.0.0.1:37613");
    assert_eq!(
        app.authenticated_url(),
        "http://127.0.0.1:37613/?token=private-token"
    );
    app.wait().unwrap();
}

#[test]
fn native_shutdown_stops_and_joins_the_server_thread() {
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_shutdown = Arc::clone(&shutdown);
    let app = LocalWebApp::spawn(
        "127.0.0.1:37613".parse().unwrap(),
        "token".to_owned(),
        Arc::clone(&shutdown),
        move || {
            while !worker_shutdown.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        },
    )
    .unwrap();

    app.shutdown_and_join().unwrap();
    assert!(shutdown.load(Ordering::Acquire));
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
cargo test -p wroid-cli local_web_app::tests -- --nocapture
```

Expected: compilation fails because `LocalOrigin` and `LocalWebApp` are not defined.

- [ ] **Step 3: Implement the minimal typed handle**

Implement these types in `local_web_app.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WebUiMode {
    Native,
    Browser,
    Headless,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalOrigin(String);

pub(crate) struct LocalWebApp {
    origin: LocalOrigin,
    token: String,
    shutdown: Arc<AtomicBool>,
    completion: Receiver<Result<()>>,
    thread: Option<JoinHandle<()>>,
}
```

`LocalOrigin::new` must reject non-loopback and IPv6 addresses, then store exactly `http://127.0.0.1:<port>`. `allows_uri` must accept only the exact origin followed by end-of-string, `/`, `?`, or `#`. `LocalWebApp::spawn` sends the worker result through a synchronous channel. `wait` blocks for page-driven shutdown and joins the completed thread; `shutdown_and_join` sets the signal and uses `recv_timeout(Duration::from_secs(3))`, returning context-rich errors for timeout, channel disconnect, worker failure, or panic. `Drop` sets the shutdown signal without blocking.

- [ ] **Step 4: Run the focused and CLI tests**

Run:

```bash
cargo test -p wroid-cli local_web_app::tests -- --nocapture
cargo test -p wroid-cli cli::tests -- --nocapture
```

Expected: all tests pass.

- [ ] **Step 5: Commit the lifetime primitive**

```bash
git add crates/wroid-cli/src/commands/local_web_app.rs crates/wroid-cli/src/commands/mod.rs
git commit -m "CLI: add local web app lifetime handle"
```

### Task 2: Detachable Hub and editor servers

**Files:**
- Modify: `crates/wroid-cli/src/commands/hub.rs:75-126`
- Modify: `crates/wroid-cli/src/commands/editor.rs:28-78`

**Interfaces:**
- Consumes: `LocalWebApp::spawn`, `WebUiMode`.
- Produces: `hub::start_hub(port: u16, profiles_dir: Option<PathBuf>) -> Result<LocalWebApp>`.
- Produces: `editor::start_editor(path: PathBuf, port: u16) -> Result<LocalWebApp>`.
- Produces: blocking `run_hub(port, mode, profiles_dir)` and `edit_v2(path, port, mode)` browser/headless behavior; native routing is completed in Task 4.

- [ ] **Step 1: Write failing real-server lifecycle tests**

In the existing `#[cfg(test)]` modules, add tests using temporary profile paths and an ephemeral port:

```rust
#[test]
fn started_hub_serves_until_handle_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let app = start_hub(0, Some(temp.path().join("profiles"))).unwrap();
    let response = http_get(app.authenticated_url());
    assert!(response.starts_with("HTTP/1.1 200"));
    app.shutdown_and_join().unwrap();
}

#[test]
fn editor_page_close_stops_the_server_handle() {
    let temp = tempfile::tempdir().unwrap();
    let profile_path = temp.path().join("profile.json");
    write_valid_profile(&profile_path);
    let app = start_editor(profile_path, 0).unwrap();
    let close_url = format!(
        "{}/api/close?token={}",
        app.origin().as_str(),
        app.token()
    );
    let response = http_post(&close_url, b"{}");
    assert!(response.starts_with("HTTP/1.1 200"));
    app.wait().unwrap();
}
```

Keep token extraction test-only by adding `#[cfg(test)] pub(crate) fn token(&self) -> &str` on `LocalWebApp`; production callers use only `authenticated_url()`.

- [ ] **Step 2: Run and verify RED**

Run:

```bash
cargo test -p wroid-cli commands::hub::tests::started_hub_serves_until_handle_shutdown -- --exact --nocapture
cargo test -p wroid-cli commands::editor::tests::editor_page_close_stops_the_server_handle -- --exact --nocapture
```

Expected: compilation fails because `start_hub` and `start_editor` do not exist.

- [ ] **Step 3: Extract server startup and accept loops without changing handlers**

For each application:

1. Keep all validation, bootstrap, listener binding, token creation, HTTP parsing, authorization, response headers, upload limits, and route handlers unchanged.
2. Move listener preparation into `start_hub`/`start_editor`.
3. Move the existing nonblocking accept loop into the closure passed to `LocalWebApp::spawn`.
4. Clone the same `Arc<AtomicBool>` into connection threads so `/api/close` triggers `LocalWebApp::wait`.
5. Return the handle without printing or opening its URL.

Make `run_hub` and `edit_v2` use one shared pattern for the non-native modes:

```rust
let app = start_hub(port, profiles_dir)?;
match mode {
    WebUiMode::Browser => open_url(app.authenticated_url()),
    WebUiMode::Headless => println!("Hub: {}", app.authenticated_url()),
    WebUiMode::Native => unreachable!("wired by the native-shell task"),
}
app.wait()
```

The editor keeps its existing profile/library status lines, but only browser/headless mode may expose the tokenized URL.

- [ ] **Step 4: Run lifecycle and existing HTTP tests**

Run:

```bash
cargo test -p wroid-cli commands::hub::tests -- --nocapture
cargo test -p wroid-cli commands::editor::tests -- --nocapture
```

Expected: all existing request, security, upload, profile, and new lifecycle tests pass.

- [ ] **Step 5: Commit the server extraction**

```bash
git add crates/wroid-cli/src/commands/hub.rs crates/wroid-cli/src/commands/editor.rs crates/wroid-cli/src/commands/local_web_app.rs
git commit -m "CLI: detach Hub and editor server lifetimes"
```

### Task 3: Native-default command contract

**Files:**
- Modify: `crates/wroid-cli/src/cli.rs:68-78,258-265,954-1010`
- Modify: `crates/wroid-cli/src/commands/mod.rs:45-65`
- Modify: `crates/wroid-cli/src/commands/desktop.rs:380-415`

**Interfaces:**
- Consumes: `WebUiMode`.
- Produces: `WebUiMode::from_flags(browser: bool, no_open: bool) -> WebUiMode`.
- Produces: `Commands::Hub { browser, no_open, ... }` and `ProfileCommand::EditV2 { browser, no_open, ... }`.

- [ ] **Step 1: Write failing Clap contract tests**

Add tests for native defaults, explicit browser mode, invalid mixed mode, and unchanged desktop launcher:

```rust
#[test]
fn hub_defaults_to_native_window_mode() {
    let cli = Cli::try_parse_from(["wroid", "hub"]).unwrap();
    let Commands::Hub { browser, no_open, .. } = cli.command else { panic!() };
    assert!(!browser);
    assert!(!no_open);
}

#[test]
fn hub_browser_and_headless_modes_conflict() {
    let error = Cli::try_parse_from(["wroid", "hub", "--browser", "--no-open"])
        .unwrap_err();
    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
}

#[test]
fn editor_accepts_explicit_browser_mode() {
    let cli = Cli::try_parse_from(["wroid", "profile", "edit-v2", "p.json", "--browser"])
        .unwrap();
    let Commands::Profile { command: ProfileCommand::EditV2 { browser, no_open, .. } } = cli.command else { panic!() };
    assert!(browser);
    assert!(!no_open);
}
```

Extend the desktop entry test with:

```rust
assert!(entry.contains("Exec=/opt/wroid hub\n"));
assert!(!entry.contains("--browser"));
```

- [ ] **Step 2: Run and verify RED**

Run:

```bash
cargo test -p wroid-cli cli::tests::hub_defaults_to_native_window_mode -- --exact
cargo test -p wroid-cli cli::tests::hub_browser_and_headless_modes_conflict -- --exact
cargo test -p wroid-cli cli::tests::editor_accepts_explicit_browser_mode -- --exact
```

Expected: compile failures because the command variants have no `browser` field.

- [ ] **Step 3: Add flags and typed routing**

Update command documentation to say “Open ... in a native desktop window.” Add this field to both graphical commands:

```rust
/// Open the local application in the default browser instead of its native window.
#[arg(long, conflicts_with = "no_open")]
browser: bool,
```

Implement:

```rust
impl WebUiMode {
    pub(crate) fn from_flags(browser: bool, no_open: bool) -> Self {
        match (browser, no_open) {
            (true, false) => Self::Browser,
            (false, true) => Self::Headless,
            (false, false) => Self::Native,
            (true, true) => unreachable!("clap rejects conflicting UI modes"),
        }
    }
}
```

Pass the resulting mode through `commands::run`. Keep the generated desktop entry exactly `Exec=<installed-wroid> hub`, which now means native mode.

- [ ] **Step 4: Run all CLI and desktop tests**

Run:

```bash
cargo test -p wroid-cli cli::tests -- --nocapture
cargo test -p wroid-cli commands::desktop::tests -- --nocapture
```

Expected: all tests pass, including `--browser`/`--no-open` conflict validation.

- [ ] **Step 5: Commit the public command contract**

```bash
git add crates/wroid-cli/src/cli.rs crates/wroid-cli/src/commands/mod.rs crates/wroid-cli/src/commands/desktop.rs crates/wroid-cli/src/commands/local_web_app.rs
git commit -m "CLI: make native Hub mode the default"
```

### Task 4: Reusable GTK/WebKitGTK shell

**Files:**
- Create: `crates/wroid-cli/src/commands/desktop_webview.rs`
- Modify: `crates/wroid-cli/src/commands/mod.rs`
- Modify: `crates/wroid-cli/src/commands/hub.rs`
- Modify: `crates/wroid-cli/src/commands/editor.rs`
- Modify: `crates/wroid-cli/Cargo.toml`
- Modify: `Cargo.lock`

**Interfaces:**
- Consumes: `LocalWebApp`, `LocalOrigin`, `start_hub`, and `start_editor`.
- Produces: `NativeAppSpec`, constants `HUB_WINDOW` and `CONTROLS_WINDOW`, and `run_native_app<F>(spec, start_server) -> Result<()> where F: FnOnce() -> Result<LocalWebApp> + 'static`.

- [ ] **Step 1: Write failing pure policy and spec tests**

In `desktop_webview.rs`, write tests before the production definitions:

```rust
#[test]
fn hub_is_single_instance_and_editor_is_not() {
    assert_eq!(HUB_WINDOW.application_id, Some("io.wroid.GamingHub"));
    assert!(!HUB_WINDOW.non_unique);
    assert_eq!(CONTROLS_WINDOW.application_id, None);
    assert!(CONTROLS_WINDOW.non_unique);
}

#[test]
fn window_geometry_matches_the_desktop_contract() {
    assert_eq!(HUB_WINDOW.default_size, (1280, 800));
    assert_eq!(HUB_WINDOW.minimum_size, (1024, 640));
    assert_eq!(CONTROLS_WINDOW.default_size, (1280, 800));
    assert_eq!(CONTROLS_WINDOW.minimum_size, (1024, 640));
}

#[test]
fn navigation_policy_rejects_popups_and_foreign_origins() {
    let origin = LocalOrigin::new("127.0.0.1:37613".parse().unwrap()).unwrap();
    assert_eq!(navigation_action(&origin, "http://127.0.0.1:37613/", false), NavigationAction::Allow);
    assert_eq!(navigation_action(&origin, "http://127.0.0.1:37614/", false), NavigationAction::Block);
    assert_eq!(navigation_action(&origin, "http://127.0.0.1:37613/", true), NavigationAction::Block);
}
```

Add a lifecycle test around a small non-GTK finalizer:

```rust
#[test]
fn shell_initialization_failure_still_stops_server() {
    let (app, stopped) = running_test_app();
    let error = finish_shell_session(app, Err(anyhow!("WebView initialization failed")))
        .unwrap_err();
    assert!(error.to_string().contains("WebView initialization failed"));
    assert!(stopped.load(Ordering::Acquire));
}
```

- [ ] **Step 2: Run and verify RED**

Run:

```bash
cargo test -p wroid-cli desktop_webview::tests -- --nocapture
```

Expected: compilation fails because the module's specs, policy, and finalizer do not exist.

- [ ] **Step 3: Add the system bindings**

Add exact direct dependencies:

```toml
gtk = { version = "0.18.2", features = ["v3_24"] }
webkit2gtk = { version = "2.0.2", features = ["v2_40"] }
```

Run:

```bash
cargo check -p wroid-cli
```

Expected: GTK 3 and WebKitGTK 4.1 are found through `pkg-config`; the new test module still needs its implementation.

- [ ] **Step 4: Implement app registration and window lifecycle**

Implement these exact specs:

```rust
pub(crate) const HUB_WINDOW: NativeAppSpec = NativeAppSpec {
    application_id: Some("io.wroid.GamingHub"),
    title: "Wroid Gaming Hub",
    non_unique: false,
    default_size: (1280, 800),
    minimum_size: (1024, 640),
};

pub(crate) const CONTROLS_WINDOW: NativeAppSpec = NativeAppSpec {
    application_id: None,
    title: "Wroid Controls Studio",
    non_unique: true,
    default_size: (1280, 800),
    minimum_size: (1024, 640),
};
```

`run_native_app` must:

1. Construct `gtk::Application` with empty flags for Hub and `gio::ApplicationFlags::NON_UNIQUE` for Controls Studio, then call `run_with_args(&["wroid"])` so GTK never reparses the already-consumed Wroid CLI arguments.
2. Store the `FnOnce` server factory and current session in `Rc<RefCell<Option<_>>>` values.
3. On first `activate`, start the server and construct `gtk::ApplicationWindow`; on later Hub activation, call `present()` on the existing window without creating a listener or WebView.
4. Set title, default size, and WebView minimum size; add the WebView as the sole child and call `show_all()`.
5. On GTK delete/close, set the server shutdown signal, then let post-`Application::run()` cleanup call `shutdown_and_join()`.
6. Poll `LocalWebApp::is_shutdown()` with `glib::timeout_add_local(Duration::from_millis(50), ...)`; when `/api/close` fires, close the window and return `glib::ControlFlow::Break`.
7. Store activation/load errors, show a `gtk::MessageDialog`, quit after acknowledgement, and return the original error after server cleanup.

`finish_shell_session` must always call `shutdown_and_join`; if both shell and shutdown fail, preserve the shell error and attach cleanup context.

- [ ] **Step 5: Implement WebKit confinement**

Create `webkit2gtk::WebView`, load only `LocalWebApp::authenticated_url()`, and configure:

```rust
if let Some(settings) = webview.settings() {
    settings.set_enable_developer_extras(cfg!(debug_assertions));
}
webview.connect_create(|_, _| None);
webview.connect_context_menu(|_, _, _, _| true);
webview.connect_permission_request(|_, request| {
    request.deny();
    true
});
```

In `connect_decide_policy`, downcast navigation decisions to `NavigationPolicyDecision`, read `request().and_then(|request| request.uri())`, and call `decision.use_()` only when `LocalOrigin::allows_uri` succeeds and the policy type is not `NewWindowAction`; otherwise call `decision.ignore()`. Keep the existing CSP (`default-src 'self'`) as the subresource boundary and do not add TLS exception handling. `connect_load_failed` records the URI/error, signals shutdown, and opens the native error dialog. Do not disable the default file chooser.

- [ ] **Step 6: Route native Hub and editor startup through the shell**

Replace the temporary native `unreachable!` branches:

```rust
WebUiMode::Native => desktop_webview::run_native_app(HUB_WINDOW, move || {
    start_hub(port, profiles_dir)
}),
```

and:

```rust
WebUiMode::Native => desktop_webview::run_native_app(CONTROLS_WINDOW, move || {
    start_editor(path, port)
}),
```

Browser and headless modes must still call the exact same `start_*` functions.

- [ ] **Step 7: Run shell, Hub, editor, and CLI tests**

Run:

```bash
cargo test -p wroid-cli desktop_webview::tests -- --nocapture
cargo test -p wroid-cli commands::hub::tests -- --nocapture
cargo test -p wroid-cli commands::editor::tests -- --nocapture
cargo test -p wroid-cli cli::tests -- --nocapture
```

Expected: all pass without opening a graphical window during tests.

- [ ] **Step 8: Commit the native shell**

```bash
git add Cargo.lock crates/wroid-cli/Cargo.toml crates/wroid-cli/src/commands/desktop_webview.rs crates/wroid-cli/src/commands/mod.rs crates/wroid-cli/src/commands/hub.rs crates/wroid-cli/src/commands/editor.rs
git commit -m "Desktop: embed Hub in native WebKitGTK windows"
```

### Task 5: Packaging and operator documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/roadmap.md:118-125,198-246`
- Modify: `docs/waydroid-notes.md:25-34`

**Interfaces:**
- Consumes: native-default command behavior and exact Arch package names.
- Produces: install/runtime guidance matching the built binary.

- [ ] **Step 1: Write the documentation assertions as a failing shell check**

Run:

```bash
rg -n "webkit2gtk-4\.1|wroid hub --browser|native desktop window" README.md docs/waydroid-notes.md docs/roadmap.md
```

Expected: at least one required phrase is absent.

- [ ] **Step 2: Update installation and behavior documentation**

Document all of the following literal contracts:

- Arch/CachyOS build/runtime packages include `gtk3` and `webkit2gtk-4.1`.
- The application-menu entry and `wroid hub` open the native window.
- `wroid hub --browser` and `wroid profile edit-v2 <path> --browser` are explicit diagnostics.
- `--no-open` is headless and cannot be combined with `--browser`.
- The native shell still serves only authenticated loopback traffic and does not alter the game input hot path.
- Mark native desktop packaging complete in the roadmap while leaving tray/overlay/release automation out of scope.

- [ ] **Step 3: Verify the documentation and formatting**

Run:

```bash
rg -n "webkit2gtk-4\.1|wroid hub --browser|native desktop window" README.md docs/waydroid-notes.md docs/roadmap.md
git diff --check
```

Expected: every contract appears and `git diff --check` is clean.

- [ ] **Step 4: Commit documentation**

```bash
git add README.md docs/roadmap.md docs/waydroid-notes.md
git commit -m "Docs: document native Wroid desktop runtime"
```

### Task 6: Full verification, installed smoke test, and rollout

**Files:**
- Modify only if verification exposes a tested defect in files already listed above.

**Interfaces:**
- Consumes: complete native shell feature.
- Produces: release binaries and an installed desktop launcher proven on KDE Wayland.

- [ ] **Step 1: Run the full automated quality gate**

Run:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
node --check crates/wroid-cli/assets/hub/app.js
node --check crates/wroid-cli/assets/hub/control-chips.js
node --check crates/wroid-cli/assets/hub/compatibility-state.js
node --check crates/wroid-cli/assets/editor/app.js
node --check crates/wroid-cli/assets/editor/profile-model.js
git diff --check
```

Expected: every command exits 0 with no warnings.

- [ ] **Step 2: Build and install the release desktop application**

Run:

```bash
cargo build --release -p wroid-cli --bin wroid
cargo build --release -p wroid-daemon --bin wroidd
cargo build --release -p wroid-inject --bin wroid-helper
target/release/wroid desktop install
target/release/wroid desktop status
```

Expected: release builds succeed; desktop status reports the installed binary, daemon, helper staging, icon, and application entry ready.

- [ ] **Step 3: Verify installed native Hub behavior on KDE Wayland**

Launch **Wroid Gaming Hub** from the application menu and verify:

1. A window titled `Wroid Gaming Hub` appears and no new default-browser process/tab is opened.
2. A second launcher activation presents the same Hub process/window.
3. Library, System, Play Store, APK picker, refresh, and close controls respond.
4. The Hub process command line contains no `token=` value.
5. Closing the window removes its localhost listener and leaves no orphan Hub process.

Use `ss -ltnp`, `ps`, the compositor window list, and a screenshot for evidence; do not expose the private token in captured output.

- [ ] **Step 4: Verify native Controls Studio**

Open **Edit controls** for Standoff 2 and verify a separate `Wroid Controls Studio` window appears. Save and close it, then confirm its listener/process exits while the Hub remains active. Open two editor actions and verify they can coexist as separate non-singleton processes.

- [ ] **Step 5: Run the existing Standoff 2 managed-session smoke test**

From the installed Hub, launch the already installed Standoff 2 profile, enter a match, verify keyboard movement and relative mouse aiming, stop the session, and confirm Waydroid desktop restoration. Re-run the existing compatibility status and confirm active root remains `not_detected`.

- [ ] **Step 6: Audit cleanup and commit any verification-only fixes**

Confirm there are no test Hub/editor listeners, orphan Wroid processes, stale managed-session leases, or unexpected browser tabs. If a defect required a code fix, first add a focused failing regression test, verify RED, make the minimum fix, repeat the full gate, then commit only that fix:

```bash
git add Cargo.lock crates/wroid-cli/Cargo.toml crates/wroid-cli/src/cli.rs \
  crates/wroid-cli/src/commands/desktop.rs \
  crates/wroid-cli/src/commands/desktop_webview.rs \
  crates/wroid-cli/src/commands/editor.rs \
  crates/wroid-cli/src/commands/hub.rs \
  crates/wroid-cli/src/commands/local_web_app.rs \
  crates/wroid-cli/src/commands/mod.rs README.md docs/roadmap.md \
  docs/waydroid-notes.md
git commit -m "Desktop: fix native shell smoke-test regression"
```

- [ ] **Step 7: Push the completed main branch**

After the finishing-development workflow integrates the feature branch into `main`, run:

```bash
git status --short --branch
git log --oneline --decorate -8
git push origin main
```

Expected: the worktree is clean and `origin/main` advances through the native-shell commits.
