# Daemon-Owned Bridge Helper Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `wroidd` the sole production owner of bridge-helper activation while preserving the desktop-user input hot path and deterministic crash cleanup.

**Architecture:** Each managed launch gets an inherited Unix socket pair. The worker creates uinput and speaks a small versioned startup/cleanup protocol; a daemon-owned broker starts and owns the exact release-matched helper. Public `launch-v2` becomes a daemon client, while root diagnostics retain their in-process bridge.

**Tech Stack:** Rust 2021, Unix domain sockets, Linux pidfd/prctl/fcntl, serde JSON, Clap, Waydroid/LXC, evdev/uinput.

## Global Constraints

- Keep all touch dispatch and physical input events out of IPC; the bridge channel is startup/readiness/cleanup only.
- Internal bridge protocol version is `1`; worker protocol generation is `1`; each frame is at most `4096` bytes.
- Allow `5 s` for initial `open`, `120 s` from `opened` to verification request, `3 s` for frame writes, and unbounded verified gameplay while the exact worker remains alive.
- Only `wroidd` may activate `/usr/lib/wroid/wroid-helper`; `wroid helper status` may retain the side-effect-free `--check` probe.
- Never accept a helper path, executable, command, package, property, environment entry, or shell fragment from IPC.
- Validate the event node twice: broker path shape first, existing privileged sysfs/device identity second.
- Preserve helper EOF recovery, global bridge lease, contact cancellation, evdev ungrab, Waydroid restoration, and combined failure reporting.
- A hidden worker must require a valid inherited descriptor and fail before uinput or Waydroid mutation when it is absent.
- Reuse only the exact desired daemon inode; replace a stale daemon only when it owns no active managed process and only through an authenticated pidfd.
- Preserve daemon protocol envelope v2; reject missing or mismatched worker protocol generations before spawn.
- Keep root-only diagnostic bridge paths available and do not add a hot-path dependency on `wroidd`.

---

### Task 1: Release-Matched Helper Activation Primitive

**Files:**
- Modify: `crates/wroid-inject/src/privileged_bridge.rs`
- Modify: `crates/wroid-inject/src/lib.rs`

**Interfaces:**
- Consumes: existing `validate_installed_bridge_helper(path: &Path)` and `BridgeHelperCommand`.
- Produces: `BridgeHelperCommand::production_release(staged: &Path, expected_uid: u32) -> io::Result<Self>`.

- [ ] **Step 1: Add RED tests for staged-release validation**

Add focused tests proving the staged file must be a final non-symlink regular file, owned by `expected_uid`, exactly mode `0555`, no larger than `64 MiB`, and byte-identical to the installed helper. Exercise equal, same-length-different, shorter, oversized, writable, and symlink fixtures without invoking a setuid helper.

```rust
#[test]
fn staged_release_requires_exact_protected_bytes() {
    let staged = fixture_file(b"release-a", 0o555);
    let installed = fixture_file(b"release-a", 0o4750);
    assert!(release_files_match(&installed, &staged).unwrap());
    fs::write(&installed, b"release-b").unwrap();
    assert!(!release_files_match(&installed, &staged).unwrap());
}
```

- [ ] **Step 2: Run focused tests and verify RED**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject staged_release -- --nocapture
```

Expected: compile failures for missing release-validation helpers.

- [ ] **Step 3: Implement descriptor-backed validation**

Open staged and installed files with `O_NOFOLLOW | O_CLOEXEC`, validate descriptor metadata, reject lengths above `64 * 1024 * 1024`, and compare fixed-size buffers. Probe installed root ownership/mode/effective-root only after equality succeeds.

```rust
const MAX_STAGED_HELPER_BYTES: u64 = 64 * 1024 * 1024;

impl BridgeHelperCommand {
    pub fn production_release(staged: &Path, expected_uid: u32) -> io::Result<Self> {
        validate_staged_helper_release(staged, expected_uid)?;
        let installed = Path::new(DEFAULT_PRIVILEGED_BRIDGE_HELPER);
        if !release_files_match(installed, staged)? {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied,
                "installed Wroid helper differs from the paired staged release"));
        }
        validate_installed_bridge_helper(installed)?;
        Ok(Self { executable: installed.to_path_buf() })
    }
}
```

- [ ] **Step 4: Run GREEN and regressions**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject staged_release
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject privileged_bridge
```

- [ ] **Step 5: Commit Task 1**

```bash
git add crates/wroid-inject/src/privileged_bridge.rs crates/wroid-inject/src/lib.rs
git commit -m "Inject: validate daemon helper release pairing"
```

---

### Task 2: Versioned Private Bridge Broker Protocol

**Files:**
- Create: `crates/wroid-inject/src/bridge_broker.rs`
- Modify: `crates/wroid-inject/Cargo.toml`
- Modify: `crates/wroid-inject/src/lib.rs`
- Modify: `Cargo.lock`

**Interfaces:**
- Consumes: `BridgeHelperCommand` and `PrivilegedBridgeHelper`.
- Produces: `BRIDGE_PROTOCOL_VERSION`, `BRIDGE_WORKER_PROTOCOL_GENERATION`, `BRIDGE_WORKER_FD`, `BridgeBrokerClient`, `BridgeHelperFactory`, `BridgeHelperSession`, `ProductionBridgeHelperFactory`, and `serve_bridge_broker`.

- [ ] **Step 1: Add RED wire/state tests**

Use `UnixStream::pair()` and a fake helper. Cover open/verify/finish, version mismatch, `4097` bytes, missing newline, invalid event path, duplicate/reordered operations, EOF before/after open, false graceful flag, and helper failures.

```rust
#[test]
fn broker_accepts_only_open_verify_finish() {
    let (client_stream, server_stream) = UnixStream::pair().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let server = spawn_fake_broker(server_stream, calls.clone());
    let mut client = BridgeBrokerClient::from_stream(client_stream).unwrap();
    client.open(Path::new("/dev/input/event42")).unwrap();
    client.verify_android_input().unwrap();
    client.finish(true).unwrap();
    server.join().unwrap().unwrap();
    assert_eq!(*calls.lock().unwrap(), ["open", "verify", "finish:true"]);
}
```

- [ ] **Step 2: Run protocol tests and verify RED**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject bridge_broker -- --nocapture
```

- [ ] **Step 3: Implement the closed protocol**

```rust
pub const BRIDGE_PROTOCOL_VERSION: u32 = 1;
pub const BRIDGE_WORKER_PROTOCOL_GENERATION: u32 = 1;
pub const BRIDGE_WORKER_FD: RawFd = 198;
const MAX_BRIDGE_FRAME_BYTES: usize = 4096;

pub trait BridgeHelperSession: Send {
    fn verify_android_input(&mut self) -> io::Result<()>;
    fn finish(self: Box<Self>, waydroid_stopped: bool) -> io::Result<()>;
}
pub trait BridgeHelperFactory: Send + Sync + 'static {
    fn start(&self, event_node: &Path) -> io::Result<Box<dyn BridgeHelperSession>>;
}
pub struct BridgeBrokerClient { stream: UnixStream, state: ClientState }
impl BridgeBrokerClient {
    pub fn from_owned_fd(fd: OwnedFd) -> io::Result<Self>;
    pub fn open(&mut self, event_node: &Path) -> io::Result<()>;
    pub fn verify_android_input(&mut self) -> io::Result<()>;
    pub fn finish(self, waydroid_stopped: bool) -> io::Result<()>;
}
pub fn serve_bridge_broker(stream: UnixStream,
    factory: Arc<dyn BridgeHelperFactory>) -> io::Result<()>;
```

`from_owned_fd` verifies `S_IFSOCK`, rejects descriptors `0..=2`, and restores `FD_CLOEXEC`. Wire enums remain private and accept only `open -> verify_android_input -> finish`.

- [ ] **Step 4: Run GREEN and helper regressions**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject bridge_broker
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject privileged_bridge
```

- [ ] **Step 5: Commit Task 2**

```bash
git add Cargo.lock crates/wroid-inject/Cargo.toml crates/wroid-inject/src/bridge_broker.rs crates/wroid-inject/src/lib.rs docs/superpowers/plans/2026-08-09-daemon-owned-bridge-helper.md
git commit -m "Inject: add private bridge broker protocol"
```

---

### Task 3: Route Game Sessions Through the Broker Client

**Files:**
- Modify: `crates/wroid-inject/src/game_session.rs`
- Modify: `crates/wroid-cli/src/commands/play_v2.rs`

**Interfaces:**
- Consumes: `BridgeBrokerClient`.
- Produces: `GameSessionOptions::bridge_broker: Option<BridgeBrokerClient>` and broker-backed `SessionBridge`.

- [ ] **Step 1: Add RED adapter tests**

Prove a rootless session requires the broker before device/Waydroid mutation, opens it after event-node discovery, delegates verification, and passes the real Waydroid-stop result to finish.

```rust
#[test]
fn rootless_bridge_selection_requires_daemon_broker() {
    let error = select_session_bridge(false, None, Path::new("/dev/input/event9"))
        .unwrap_err();
    assert!(error.to_string().contains("daemon-owned bridge channel"));
}
```

- [ ] **Step 2: Run tests and verify RED**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject production_bridge -- --nocapture
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli play_v2 -- --nocapture
```

- [ ] **Step 3: Replace production direct-helper selection**

Remove `GameSessionOptions::bridge_helper`, add `bridge_broker`, and use:

```rust
enum SessionBridge {
    InProcess(InstalledWaydroidBridge),
    Broker(BridgeBrokerClient),
}
```

Root uses the existing in-process bridge. Desktop-user execution requires the broker, calls `open`, `verify_android_input`, and `finish(waydroid_stopped)`. Unprivileged direct `play-v2` fails with `use wroid launch-v2 for production sessions`.

- [ ] **Step 4: Run GREEN and runtime regressions**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject production_bridge
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject game_session
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli play_v2
```

- [ ] **Step 5: Commit Task 3**

```bash
git add crates/wroid-inject/src/game_session.rs crates/wroid-cli/src/commands/play_v2.rs
git commit -m "Inject: consume daemon-owned bridge sessions"
```

---

### Task 4: Make `wroidd` Own Broker, Helper, and Worker Descriptors

**Files:**
- Modify: `crates/wroid-daemon/Cargo.toml`
- Modify: `crates/wroid-daemon/src/ipc.rs`
- Modify: `crates/wroid-daemon/src/process.rs`
- Modify: `Cargo.lock`

**Interfaces:**
- Consumes: Tasks 1-2 APIs and current `ManagedProcesses`/`GameLaunchRequest`.
- Produces: `ManagedProcess { child, broker }`, fixed inherited worker command, generation validation, and combined reap outcomes.

- [ ] **Step 1: Add RED ownership tests**

Cover request default generation zero, mismatch rejection, bounded launch options, no helper path in worker args, fixed FD/parent PID, one inherited socket, spawn failure before activation, EOF cleanup, combined broker+worker failures, SIGTERM, and daemon drop.

```rust
#[test]
fn worker_arguments_carry_only_broker_capability() {
    let args = launch_arguments(&request(), Path::new("/profiles/game.json"), 4242);
    assert!(args.windows(2).any(|v| v == ["--bridge-fd", "198"]));
    assert!(args.windows(2).any(|v| v == ["--daemon-parent-pid", "4242"]));
    assert!(args.iter().any(|v| v == "--daemon-worker"));
    assert!(!args.iter().any(|v| v.to_string_lossy().contains("wroid-helper")));
}
```

- [ ] **Step 2: Run daemon tests and verify RED**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon process -- --nocapture
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon protocol_launch -- --nocapture
```

- [ ] **Step 3: Extend and validate the launch request**

```rust
pub struct GameLaunchRequest {
    // existing fields
    #[serde(default)] pub worker_protocol_generation: u32,
    #[serde(default = "default_true")] pub grab: bool,
    #[serde(default = "default_true")] pub show_ui: bool,
    #[serde(default = "default_true")] pub launch_package: bool,
    #[serde(default)] pub trace_input: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_after_millis: Option<u64>,
}
```

Require generation `1`; accept timeout only in `1..=3_600_000`; use `launch_package` in daemon metadata.

- [ ] **Step 4: Implement managed broker and FD inheritance**

Use `BTreeMap<SessionId, ManagedProcess>`, derive paired helper as `current_exe()?.parent()?.join("wroid-helper")`, create `UnixStream::pair`, and spawn `serve_bridge_broker` in a named thread.

```rust
struct ManagedProcess {
    child: Child,
    broker: Option<JoinHandle<io::Result<()>>>,
}
fn configure_worker_child(command: &mut Command, source_fd: RawFd,
    daemon_pid: libc::pid_t) -> io::Result<()>;
```

In `pre_exec`, publish only FD `198`, set `PR_SET_PDEATHSIG(SIGTERM)`, and fail if `getppid()` changed. Reap/drop waits for the exact child, joins the broker, and combines both errors.

- [ ] **Step 5: Run GREEN**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon process
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon ipc
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon
```

- [ ] **Step 6: Commit Task 4**

```bash
git add Cargo.lock crates/wroid-daemon/Cargo.toml crates/wroid-daemon/src/ipc.rs crates/wroid-daemon/src/process.rs
git commit -m "Daemon: own privileged bridge lifecycle"
```

---

### Task 5: Route Public `launch-v2` Through the Managed Worker

**Files:**
- Modify: `crates/wroid-cli/src/cli.rs`
- Modify: `crates/wroid-cli/src/commands/mod.rs`
- Modify: `crates/wroid-cli/src/commands/launch_v2.rs`
- Modify: `crates/wroid-cli/src/commands/play_v2.rs`
- Modify: `crates/wroid-cli/src/commands/runtime_daemon.rs`
- Modify: `crates/wroid-cli/src/commands/hub.rs`

**Interfaces:**
- Consumes: `BridgeBrokerClient::from_owned_fd` and Task 4 request fields.
- Produces: public managed dispatch, `ManagedLaunch`, foreground wait/log relay, and hidden `DaemonWorkerInvocation`.

- [ ] **Step 1: Add RED CLI/request tests**

Cover hidden flag pairing/ranges, manual failure without a socket, public option mapping, no-launch self-test preservation, worker-only outcome writes, no recursion, Hub defaults, foreground log relay, exact-session Stop on `Ctrl+C`, and unprivileged `play-v2` refusal.

```rust
#[test]
fn daemon_worker_requires_broker_fd_and_parent_pid() {
    assert!(Cli::try_parse_from(["wroid", "launch-v2", "p.json", "--daemon-worker"]).is_err());
    assert!(Cli::try_parse_from(["wroid", "launch-v2", "p.json", "--daemon-worker",
        "--bridge-fd", "198", "--daemon-parent-pid", "42"]).is_ok());
}
```

- [ ] **Step 2: Run tests and verify RED**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli launch_v2 -- --nocapture
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli daemon_worker -- --nocapture
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli input_self_test -- --nocapture
```

- [ ] **Step 3: Add hidden worker adoption**

Add mutually required hidden flags and adopt the FD once:

```rust
pub(crate) struct DaemonWorkerInvocation {
    pub(crate) bridge_fd: RawFd,
    pub(crate) daemon_parent_pid: u32,
}
fn adopt_bridge(invocation: DaemonWorkerInvocation) -> Result<BridgeBrokerClient> {
    if unsafe { libc::getppid() } != invocation.daemon_parent_pid as libc::pid_t {
        bail!("daemon worker parent identity changed");
    }
    // SAFETY: the fixed daemon command transfers sole ownership of this fd.
    let fd = unsafe { OwnedFd::from_raw_fd(invocation.bridge_fd) };
    BridgeBrokerClient::from_owned_fd(fd).context("invalid daemon bridge channel")
}
```

Only hidden worker mode enters desktop restoration and writes last-session outcomes.

- [ ] **Step 4: Route public launch and self-test through `wroidd`**

Keep profile/graphics/compatibility/helper readiness preflight, then map every option and generation into `GameLaunchRequest`:

```rust
pub(crate) fn launch_game(profile_path: &Path, profile: &ProfileV2,
    options: &PlayV2Options, game_mode: bool) -> Result<String>;
```

Return a typed start result internally:

```rust
pub(crate) struct ManagedLaunch {
    pub(crate) session_id: String,
    pub(crate) process_id: u32,
}
```

Hub normal launches return after managed spawn. Direct CLI `launch-v2` waits for
that session id, opens only the existing validated current-user `0600`
`game-session.log` with `O_NOFOLLOW`, prints appended output, and polls typed
session state. Its signal handler converts `Ctrl+C` into `DaemonRequest::Stop`
for the exact session. The no-APK terminal self-test therefore keeps live trace,
`launch_package=false`, and its bounded timeout through IPC.

- [ ] **Step 5: Run GREEN and Hub regressions**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli launch_v2
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli runtime_daemon
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli hub
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli
```

- [ ] **Step 6: Commit Task 5**

```bash
git add crates/wroid-cli/src/cli.rs crates/wroid-cli/src/commands/mod.rs crates/wroid-cli/src/commands/launch_v2.rs crates/wroid-cli/src/commands/play_v2.rs crates/wroid-cli/src/commands/runtime_daemon.rs crates/wroid-cli/src/commands/hub.rs
git commit -m "CLI: route production launch through bridge broker"
```

---

### Task 6: Safe Running-Daemon Release Handoff

**Files:**
- Modify: `crates/wroid-daemon/src/ipc.rs`
- Modify: `crates/wroid-cli/src/commands/runtime_daemon.rs`

**Interfaces:**
- Consumes: private socket peer credentials, `DaemonRequest::List`, desired daemon executable, Linux pidfd.
- Produces: `AuthenticatedDaemonPeer`, `DaemonClient::request_with_peer`, exact file identity checks, safe idle replacement.

- [ ] **Step 1: Add RED handoff tests**

Cover peer PID bound to the response, equal/different inode, idle/live state classification, PID reuse protection, pidfd SIGTERM, timeout, and active-session refusal.

```rust
#[test]
fn only_process_bearing_live_states_block_upgrade() {
    assert!(!sessions_block_upgrade(&[snapshot(SessionStateWire::Stopped, None)]));
    assert!(!sessions_block_upgrade(&[snapshot(SessionStateWire::Preparing, None)]));
    assert!(sessions_block_upgrade(&[snapshot(SessionStateWire::Running, Some(99))]));
    assert!(sessions_block_upgrade(&[snapshot(SessionStateWire::Stopping, Some(99))]));
}
```

- [ ] **Step 2: Run tests and verify RED**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon request_with_peer -- --nocapture
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli daemon_release -- --nocapture
```

- [ ] **Step 3: Bind peer identity to one request**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthenticatedDaemonPeer { pub pid: libc::pid_t, pub uid: u32 }
impl DaemonClient {
    pub fn request_with_peer(&self, request: DaemonRequest)
        -> Result<(DaemonResult, AuthenticatedDaemonPeer), IpcError>;
}
```

Keep `request` as a wrapper that discards the peer value.

- [ ] **Step 4: Implement inode comparison and pidfd replacement**

Compare followed `/proc/<pid>/exe` and desired daemon metadata by `(st_dev, st_ino)`. On mismatch, use the same peer-aware `List` response, refuse active sessions, `pidfd_open`, revalidate, `pidfd_send_signal(SIGTERM)`, and wait at most two seconds for socket/lease removal.

```rust
fn daemon_file_identity(path: &Path) -> Result<(u64, u64)>;
fn sessions_block_upgrade(sessions: &[SessionSnapshot]) -> bool;
fn stop_authenticated_idle_daemon(peer: AuthenticatedDaemonPeer,
    expected: (u64, u64)) -> Result<()>;
```

- [ ] **Step 5: Run GREEN**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli runtime_daemon
```

- [ ] **Step 6: Commit Task 6**

```bash
git add crates/wroid-daemon/src/ipc.rs crates/wroid-cli/src/commands/runtime_daemon.rs
git commit -m "Daemon: replace stale idle releases safely"
```

---

### Task 7: Documentation, Full Gates, and Live Acceptance

**Files:**
- Modify: `README.md`
- Modify: `SPEC.md`
- Modify: `docs/architecture-v2.md`
- Modify: `docs/roadmap.md`
- Modify: `docs/waydroid-input-bridge.md`
- Modify: `docs/performance-budget.md`
- Modify: `docs/superpowers/specs/2026-08-09-daemon-owned-bridge-helper-design.md`

**Interfaces:**
- Consumes: Tasks 1-6 behavior and installed release paths.
- Produces: accurate product claims and live evidence.

- [ ] **Step 1: Update docs precisely**

State that `wroidd` owns helper activation while the worker retains profile/input dispatch. Mark only “Replace direct helper activation with versioned daemon/helper IPC” complete. Keep daemon-native capture, stable bridge discovery/reconciliation, game calibration, gamepad, bundles, and cross-vendor testing open.

- [ ] **Step 2: Run focused and full gates**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test --workspace --all-features
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
for js in $(git ls-files '*.js'); do node --check "$js"; done
node --test crates/wroid-cli/assets/editor/profile-model.test.js crates/wroid-cli/assets/hub/control-chips.test.js
cargo build -p wroid-core --bin wroid-profile-v2-validate
for profile in profiles/examples/*-v2.json; do target/debug/wroid-profile-v2-validate "$profile"; done
git diff --check
```

- [ ] **Step 3: Build and install exact release**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo build --release --workspace --all-features
target/release/wroid desktop install
target/release/wroid helper install
~/.local/bin/wroid desktop status
~/.local/bin/wroid helper status
```

- [ ] **Step 4: Run live no-APK acceptance**

Use selected devices from `~/.config/wroid/preferences.json` and run:

```bash
~/.local/bin/wroid launch-v2 ~/.config/wroid/profiles-v2/standoff-2.json --no-launch --no-grab --trace-input --exit-after-seconds 20
ps -eo pid,ppid,stat,cmd | rg 'wroidd|launch-v2|wroid-helper'
~/.local/bin/wroid daemon sessions
```

Required: helper is owned by `wroidd`, worker shows hidden daemon mode/FD, and fixed Android verification finds `Wroid Gaming Touchscreen`.

- [ ] **Step 5: Verify cleanup and ten contacts**

```bash
waydroid status
ps -eo pid,ppid,stat,cmd | rg 'wroid-helper|launch-v2' || true
rg -n 'wroid-input-bridge' /var/lib/waydroid/lxc/waydroid/config /var/lib/waydroid/lxc/waydroid/config_nodes 2>/dev/null || true
target/release/wroid-inject-latency --samples 20000
```

Required: previous Waydroid state restored, no worker/helper/include remains, ten contacts release cleanly, and p99 remains below `5 ms`.

- [ ] **Step 6: Commit docs and final evidence**

```bash
git add README.md SPEC.md docs/architecture-v2.md docs/roadmap.md docs/waydroid-input-bridge.md docs/performance-budget.md docs/superpowers/specs/2026-08-09-daemon-owned-bridge-helper-design.md
git commit -m "Docs: record daemon-owned bridge lifecycle"
git status --short
```

Expected: clean worktree.
