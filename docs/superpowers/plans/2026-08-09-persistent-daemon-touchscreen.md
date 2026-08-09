# Persistent Daemon Touchscreen Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep one daemon-owned Waydroid touchscreen and Android session alive across consecutive managed games while preserving deterministic cleanup and sub-5 ms acknowledged input.

**Architecture:** A dedicated `wroidd` platform thread lazily owns canonical uinput, the exact-release bridge helper, and `DesktopWaydroidSession`. Each worker receives one inherited `SOCK_SEQPACKET` descriptor and uses a synchronous `TouchInjector` client; the daemon validates, scales, injects, and acknowledges each frame, then cancels remaining contacts when the worker detaches without stopping Waydroid.

**Tech Stack:** Rust 2021, Linux `socketpair(SOCK_SEQPACKET)`, evdev/uinput, Unix peer credentials and inherited descriptors, Waydroid/LXC, typed daemon IPC, TDD.

## Global Constraints

- Normal Hub/CLI launches must not stop Waydroid when their worker exits.
- The first managed launch may perform one controlled Waydroid restart; later same-resolution launches in the same daemon lifetime must reuse the exact uinput event node, helper, and Waydroid owner.
- The virtual touchscreen uses exactly 10 slots and canonical axes `0..=65535`.
- Every gameplay frame is acknowledged only after the uinput write succeeds.
- Runtime packets are fixed binary `SOCK_SEQPACKET` messages; no JSON, subprocess, shell call, or heap allocation is allowed per gameplay frame.
- One frame contains `1..=10` unique contacts and has a fixed maximum encoded size.
- Unknown versions/opcodes/phases, truncation, oversize, replayed sequence numbers, invalid contact transitions, and out-of-range coordinates fail closed.
- Only the exact paired `/usr/lib/wroid/wroid-helper` is privileged. It never receives touch frames, profiles, packages, display properties, or host-device paths other than the independently validated Wroid event node.
- Root-only diagnostic binaries retain the existing temporary in-process bridge path.
- Worker protocol generation changes from `1` to `2`; mismatches fail before spawn.
- The final headless benchmark submits at least 20,000 acknowledged frames, releases 10/10 contacts, and reports p99 below 5 ms.
- Do not run a graphical Waydroid acceptance until all non-GUI gates pass and the user has been warned immediately before it starts.

---

### Task 1: Fixed Runtime Touch Protocol

**Files:**
- Create: `crates/wroid-inject/src/runtime_channel.rs`
- Modify: `crates/wroid-inject/src/lib.rs`

**Interfaces:**
- Consumes: `wroid_core::{Point, Resolution}` and `wroid_runtime::{TouchEngine, TouchEvent, TouchFrame, TouchInjector, TouchPhase}`.
- Produces: `RUNTIME_PROTOCOL_VERSION`, `RUNTIME_WORKER_PROTOCOL_GENERATION`, `RUNTIME_WORKER_FD`, `runtime_socket_pair()`, `RuntimeChannelClient`, `RuntimeChannelServer`, `RuntimeAttachmentReport`, and `serve_runtime_attachment()`.

- [ ] **Step 1: Add RED codec, socket, and scaling tests**

Place the tests in `runtime_channel.rs`. Cover a valid one-event packet, 10 events, empty/11-event rejection, unknown version/opcode/phase, short packet, `MSG_TRUNC`, duplicate contact ids, repeated/out-of-order sequence, coordinates outside the logical resolution, EOF, and endpoint-preserving scaling.

```rust
#[test]
fn scales_logical_endpoints_into_canonical_axes() {
    let landscape = Resolution { width: 1920, height: 1080 };
    assert_eq!(scale_point(Point { x: 0, y: 0 }, landscape).unwrap(), Point { x: 0, y: 0 });
    assert_eq!(
        scale_point(Point { x: 1919, y: 1079 }, landscape).unwrap(),
        Point { x: 65535, y: 65535 }
    );
    assert_eq!(scale_axis(960, 1920).unwrap(), 32785);
}

#[test]
fn seqpacket_receive_rejects_truncated_datagram() {
    let (client, server) = runtime_socket_pair().unwrap();
    send_raw_packet(client.as_raw_fd(), &[0x5a; MAX_PACKET_BYTES + 1]).unwrap();
    let error = RuntimeChannelServer::from_owned_fd(server)
        .unwrap()
        .receive_request(Resolution { width: 1920, height: 1080 })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("oversized runtime packet"));
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject runtime_channel -- --nocapture
```

Expected: compile failures for the missing runtime-channel types and helpers.

- [ ] **Step 3: Implement the bounded packet transport**

Use `libc::socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, ...)`, `send(..., MSG_NOSIGNAL)`, and `recvmsg(..., MSG_CMSG_CLOEXEC)` with `MSG_TRUNC` inspection. Encode integers explicitly in little-endian bytes; do not transmute Rust structs.

```rust
pub const RUNTIME_PROTOCOL_VERSION: u16 = 1;
pub const RUNTIME_WORKER_PROTOCOL_GENERATION: u32 = 2;
pub const RUNTIME_WORKER_FD: RawFd = 198;
const MAX_FRAME_EVENTS: usize = 10;
const HEADER_BYTES: usize = 20;
const EVENT_BYTES: usize = 12;
const MAX_PACKET_BYTES: usize = HEADER_BYTES + MAX_FRAME_EVENTS * EVENT_BYTES;
const STARTUP_RESPONSE_TIMEOUT: Duration = Duration::from_secs(300);
const FRAME_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);
const SERVER_IDLE_POLL: Duration = Duration::from_millis(250);

pub fn runtime_socket_pair() -> io::Result<(OwnedFd, OwnedFd)>;

pub struct RuntimeChannelClient {
    socket: OwnedFd,
    next_sequence: u64,
    ready: bool,
}

impl RuntimeChannelClient {
    pub fn from_owned_fd(fd: OwnedFd) -> io::Result<Self>;
    pub fn from_owned_fd_for_peer(
        fd: OwnedFd,
        expected_pid: libc::pid_t,
        expected_uid: u32,
    ) -> io::Result<Self>;
    pub fn wait_until_ready(&mut self) -> io::Result<()>;
    pub fn finish(&mut self) -> io::Result<()>;
}

impl TouchInjector for RuntimeChannelClient {
    fn inject(&mut self, frame: &TouchFrame) -> Result<(), TouchInjectionError>;
}

pub struct RuntimeChannelServer {
    socket: OwnedFd,
    expected_sequence: u64,
}

impl RuntimeChannelServer {
    pub fn from_owned_fd(fd: OwnedFd) -> io::Result<Self>;
    pub fn send_startup_error(&mut self, detail: &str) -> io::Result<()>;
}

pub struct RuntimeAttachmentReport {
    pub frames_submitted: u64,
    pub peak_contacts: usize,
    pub contacts_cancelled: usize,
}

pub fn serve_runtime_attachment<I: TouchInjector>(
    server: RuntimeChannelServer,
    resolution: Resolution,
    engine: &mut TouchEngine<I>,
    health_check: impl FnMut() -> io::Result<()>,
) -> io::Result<RuntimeAttachmentReport>;
```

`serve_runtime_attachment` sends `ready`, scales every decoded frame to the canonical axes, submits it through the daemon `TouchEngine`, sends the matching result, and on `finish`, EOF, or protocol failure calls `engine.cancel_all()` before returning. Initial platform readiness is bounded to 300 seconds so one boot plus a required resolution restart can finish. Gameplay acknowledgments and all writes have 2-second deadlines. The server uses a 250 ms read poll: timeout calls `health_check` and resumes waiting, so idle gameplay duration is unbounded while helper death is detected promptly.

- [ ] **Step 4: Add RED atomic-state and cleanup tests, then make them GREEN**

Use the real seqpacket pair with a recording/failing injector and a server thread.

```rust
#[test]
fn failed_uinput_write_is_not_committed_or_acknowledged_as_success() {
    let (mut client, server) = connected_runtime_pair();
    let injector = FailingInjector::once();
    let join = spawn_attachment(server, Resolution { width: 1920, height: 1080 }, injector);
    client.wait_until_ready().unwrap();
    let error = client.inject(&down_frame(1, 100, 200)).unwrap_err();
    assert!(error.to_string().contains("injected failure"));
    let outcome = join.join().unwrap().unwrap_err();
    assert!(outcome.to_string().contains("injected failure"));
}

#[test]
fn eof_cancels_ten_contacts_in_one_frame() {
    let recording = SharedRecordingInjector::default();
    let (mut client, server) = connected_runtime_pair();
    let join = spawn_attachment_with(server, recording.clone());
    client.wait_until_ready().unwrap();
    client.inject(&ten_contact_down_frame()).unwrap();
    drop(client);
    join.join().unwrap().unwrap();
    assert_eq!(recording.last_frame().events().len(), 10);
    assert!(recording.last_frame().events().iter().all(|event| event.phase == TouchPhase::Cancel));
}
```

Run:

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject runtime_channel
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-runtime touch
```

Expected: all focused tests pass.

- [ ] **Step 5: Commit Task 1**

```bash
git add crates/wroid-inject/src/runtime_channel.rs crates/wroid-inject/src/lib.rs
git commit -m "Inject: add private runtime touch channel"
```

---

### Task 2: Persistent Daemon Platform Coordinator

**Files:**
- Create: `crates/wroid-daemon/src/platform.rs`
- Modify: `crates/wroid-daemon/src/lib.rs`

**Interfaces:**
- Consumes: `RuntimeChannelServer`, `RuntimeAttachmentReport`, `Resolution`, and validated launch metadata.
- Produces: `PlatformLaunch`, `RuntimePlatformBackend`, `PersistentPlatform`, and `PlatformAttachment`.

- [ ] **Step 1: Add RED ownership and reuse tests with a fake backend**

The fake backend records `prepare`, `serve`, and `shutdown` calls plus a stable fake event-node identity. Prove initialization is lazy, two sequential attachments reuse one backend, a failed preparation returns one startup error and allows a later retry, and `Drop` joins the platform thread after shutdown.

```rust
#[test]
fn two_attachments_reuse_one_lazy_backend() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let platform = PersistentPlatform::with_factory(fake_factory(calls.clone()));
    assert!(calls.lock().unwrap().is_empty());
    run_fake_attachment(&platform, platform_launch("com.example.one", 1920, 1080)).unwrap();
    run_fake_attachment(&platform, platform_launch("com.example.two", 1920, 1080)).unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        ["factory", "prepare:com.example.one", "serve", "prepare:com.example.two", "serve"]
    );
}
```

- [ ] **Step 2: Run coordinator tests and verify RED**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon platform -- --nocapture
```

Expected: compile failures for the missing platform module and types.

- [ ] **Step 3: Implement one joined platform thread**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlatformLaunch {
    pub(crate) package_name: String,
    pub(crate) resolution: Resolution,
    pub(crate) show_ui: bool,
    pub(crate) launch_package: bool,
}

pub(crate) trait RuntimePlatformBackend: Send {
    fn prepare(&mut self, launch: &PlatformLaunch) -> io::Result<()>;
    fn serve(&mut self, channel: RuntimeChannelServer, resolution: Resolution)
        -> io::Result<RuntimeAttachmentReport>;
    fn shutdown(&mut self) -> io::Result<()>;
}

pub(crate) struct PlatformAttachment {
    completion: Receiver<io::Result<RuntimeAttachmentReport>>,
}

impl PlatformAttachment {
    pub(crate) fn finish(self) -> io::Result<RuntimeAttachmentReport>;
}

pub(crate) struct PersistentPlatform {
    commands: Sender<PlatformCommand>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

type PlatformFactory = Arc<
    dyn Fn() -> io::Result<Box<dyn RuntimePlatformBackend>> + Send + Sync + 'static,
>;

impl PersistentPlatform {
    pub(crate) fn with_factory(factory: PlatformFactory) -> Self;
    pub(crate) fn attach(
        &self,
        channel: RuntimeChannelServer,
        launch: PlatformLaunch,
    ) -> io::Result<PlatformAttachment>;
}
```

The thread creates its backend only when it receives the first attachment. It processes one attachment at a time, sends a bounded startup error through `RuntimeChannelServer` when `prepare` fails, returns that failure through `PlatformAttachment`, and retains a healthy backend for later attachments. `Drop` sends `Shutdown`, waits for the current attachment to end, calls backend shutdown once, and joins the thread.

- [ ] **Step 4: Run GREEN and thread-leak regressions**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon platform
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon --lib
```

Expected: all daemon library tests pass and every test joins its platform thread.

- [ ] **Step 5: Commit Task 2**

```bash
git add crates/wroid-daemon/src/platform.rs crates/wroid-daemon/src/lib.rs
git commit -m "Daemon: add persistent platform coordinator"
```

---

### Task 3: Production uinput, Helper, and Waydroid Backend

**Files:**
- Create: `crates/wroid-daemon/src/production_platform.rs`
- Modify: `crates/wroid-daemon/src/lib.rs`
- Modify: `crates/wroid-inject/src/privileged_bridge.rs`
- Modify: `crates/wroid-inject/src/waydroid_session.rs`
- Modify: `crates/wroid-inject/src/lib.rs`
- Modify: `crates/wroid-inject/src/bridge_broker.rs`

**Interfaces:**
- Consumes: Task 2 `RuntimePlatformBackend`; existing `BridgeHelperCommand`, `PrivilegedBridgeHelper`, `UinputTouchInjector`, `DeviceConfig`, `DesktopUser`, and `DesktopWaydroidSession`.
- Produces: `ProductionRuntimePlatform::new(expected_uid)`, reusable bridge-helper factory traits in `privileged_bridge.rs`, and `stop_existing_waydroid_session()`.

- [ ] **Step 1: Move helper traits without behavior changes and run baseline tests**

Move these exact interfaces from `bridge_broker.rs` to `privileged_bridge.rs`, then re-export them from `lib.rs`:

```rust
pub trait BridgeHelperSession: Send {
    fn verify_android_input(&mut self) -> io::Result<()>;
    fn check_health(&mut self) -> io::Result<()>;
    fn finish(self: Box<Self>, waydroid_stopped: bool) -> io::Result<()>;
}

pub trait BridgeHelperFactory: Send + Sync + 'static {
    fn start(&self, event_node: &Path) -> io::Result<Box<dyn BridgeHelperSession>>;
}

pub struct ProductionBridgeHelperFactory {
    command: BridgeHelperCommand,
}
```

Run:

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject bridge_broker
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject privileged_bridge
```

Expected: existing broker/helper tests pass unchanged.

- [ ] **Step 2: Add RED lifecycle-model tests**

Put lifecycle effects behind this exact driver boundary in `production_platform.rs`; `LinuxPlatformDriver` owns the real resources and tests use `RecordingPlatformDriver`. Prove first prepare creates one touchscreen/helper/session; second same-resolution prepare only launches the second package; resolution change calls one Waydroid restart without recreating touchscreen/helper; shutdown orders `cancel -> waydroid stop -> helper finish -> uinput drop`; and a failed first initialization rolls back and retries cleanly.

```rust
trait PlatformDriver: Send {
    fn initialize(&mut self, resolution: Resolution) -> io::Result<()>;
    fn change_resolution(&mut self, resolution: Resolution) -> io::Result<()>;
    fn verify_health(&mut self) -> io::Result<()>;
    fn show_ui(&mut self) -> io::Result<()>;
    fn launch_package(&mut self, package: &str) -> io::Result<()>;
    fn serve(
        &mut self,
        channel: RuntimeChannelServer,
        resolution: Resolution,
    ) -> io::Result<RuntimeAttachmentReport>;
    fn shutdown(&mut self) -> io::Result<()>;
}
```

```rust
#[test]
fn same_resolution_reuses_touchscreen_helper_and_waydroid() {
    let calls = shared_calls();
    let mut backend = fixture_backend(calls.clone());
    backend.prepare(&platform_launch("com.example.one", 1920, 1080)).unwrap();
    backend.prepare(&platform_launch("com.example.two", 1920, 1080)).unwrap();
    assert_eq!(count(&calls, "uinput:create"), 1);
    assert_eq!(count(&calls, "helper:start"), 1);
    assert_eq!(count(&calls, "waydroid:start"), 1);
    assert_eq!(packages(&calls), ["com.example.one", "com.example.two"]);
}
```

- [ ] **Step 3: Run production-platform tests and verify RED**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon production_platform -- --nocapture
```

Expected: compile failures for the missing backend and dependency interfaces.

- [ ] **Step 4: Implement stop-existing and production lifecycle**

```rust
pub fn stop_existing_waydroid_session(user: &DesktopUser) -> io::Result<()>;

pub(crate) struct ProductionRuntimePlatform {
    driver: Box<dyn PlatformDriver>,
    ready: bool,
    resolution: Option<Resolution>,
}

impl ProductionRuntimePlatform {
    pub(crate) fn new(expected_uid: u32) -> Self;
    #[cfg(test)]
    fn with_driver(driver: Box<dyn PlatformDriver>) -> Self;
}
```

`LinuxPlatformDriver` owns `TouchEngine<UinputTouchInjector>`, the event path, helper, and Waydroid session. Initialization uses `DeviceConfig::with_slots(65_536, 65_536, 10)`, selects only a direct `/dev/input/eventN` node, constructs the exact paired helper from the running daemon release directory, stops an existing session when Waydroid status reports an active session/container, starts the helper and `DesktopWaydroidSession`, applies and confirms resolution, verifies Android input through the held helper, then shows UI/launches the validated package according to `PlatformLaunch`.

`serve` delegates to `serve_runtime_attachment(channel, resolution, engine, || helper.check_health())`. `shutdown` cancels engine contacts, stops Waydroid, calls `helper.finish(stop_succeeded)`, combines all failures, and drops uinput last. `PrivilegedBridgeHelper::check_health` uses `Child::try_wait` only; it sends no new privileged command.

- [ ] **Step 5: Run GREEN and existing lifecycle regressions**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon production_platform
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject privileged_bridge
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject waydroid_session
```

Expected: all focused tests pass without starting real Waydroid.

- [ ] **Step 6: Commit Task 3**

```bash
git add crates/wroid-daemon/src/production_platform.rs crates/wroid-daemon/src/lib.rs crates/wroid-inject/src/privileged_bridge.rs crates/wroid-inject/src/waydroid_session.rs crates/wroid-inject/src/lib.rs crates/wroid-inject/src/bridge_broker.rs
git commit -m "Daemon: own persistent touchscreen platform"
```

---

### Task 4: Remote Production Game Worker

**Files:**
- Modify: `crates/wroid-inject/src/game_session.rs`
- Modify: `crates/wroid-cli/src/commands/play_v2.rs`
- Modify: `crates/wroid-cli/src/commands/launch_v2.rs`
- Modify: `crates/wroid-cli/src/cli.rs`

**Interfaces:**
- Consumes: Task 1 `RuntimeChannelClient`, `RUNTIME_WORKER_FD`, and generic `TouchInjector`.
- Produces: daemon-worker `GameSessionOptions::runtime_channel`, `play_v2_with_runtime_channel()`, and hidden `--runtime-fd 198` launch mode.

- [ ] **Step 1: Add RED worker-boundary tests**

Prove an unprivileged production worker requires the inherited runtime channel before uinput or Waydroid mutation; a root diagnostic still selects the local bridge; daemon mode waits for `ready` before grabbing evdev; remote cleanup sends `finish`; runtime failure is preserved; and the generic metrics report works with both local and remote injectors.

```rust
#[test]
fn rootless_session_selects_remote_runtime_before_platform_mutation() {
    let error = select_session_backend(false, None).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("daemon runtime channel"));
}

#[test]
fn root_diagnostic_keeps_local_uinput_backend() {
    assert!(matches!(select_session_backend(true, None).unwrap(), SessionBackend::LocalDiagnostic));
}
```

In CLI tests, reject `--daemon-worker` without all of `--runtime-fd 198` and `--daemon-parent-pid`, reject the old `--bridge-fd`, and verify peer PID/UID validation occurs before `play_v2`.

- [ ] **Step 2: Run worker tests and verify RED**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject game_session -- --nocapture
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli launch_v2 -- --nocapture
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli play_v2 -- --nocapture
```

Expected: missing runtime-channel fields/functions and old bridge assertions fail.

- [ ] **Step 3: Split local diagnostics from daemon-worker lifecycle**

Keep the existing root path in `run_local_game_session`: it creates uinput, installs the temporary bridge, starts/stops Waydroid, and cleans up as before. Add `run_remote_game_session`: it loads/validates the profile, opens evdev, waits for daemon readiness, builds `UnifiedRuntime<SessionMetricsInjector<RuntimeChannelClient>>`, handles focus/capture, stops controls, sends `finish`, and never calls `ensure_container_stopped`, `UinputTouchInjector::open`, `DesktopWaydroidSession::start/stop`, or helper methods.

```rust
enum SessionBackend {
    LocalDiagnostic,
    Remote(RuntimeChannelClient),
}

fn select_session_backend(
    is_root: bool,
    runtime_channel: Option<RuntimeChannelClient>,
) -> io::Result<SessionBackend>;

impl<I: TouchInjector> UnifiedRuntime<SessionMetricsInjector<I>> {
    fn report(&self) -> GameSessionReport;
}
```

Give `SessionMetricsInjector<I>` an `inner_mut()` accessor so the specialized remote cleanup can call `RuntimeChannelClient::finish()` after `runtime.stop()` and include finish failure in the session outcome.

- [ ] **Step 4: Replace hidden broker adoption with runtime-channel adoption**

Rename the hidden fixed option to `--runtime-fd`; require descriptor 198 and the authenticated daemon parent. Construct the client with:

```rust
let runtime_channel = RuntimeChannelClient::from_owned_fd_for_peer(
    inherited_runtime_fd(invocation.runtime_fd)?,
    invocation.daemon_parent_pid,
    unsafe { libc::geteuid() },
)?;
play_v2::play_v2_with_runtime_channel(profile_path, options, Some(runtime_channel))
```

The public `play-v2` remains root-only diagnostics; ordinary `launch-v2` still routes through `wroidd`.

- [ ] **Step 5: Run GREEN and input-state regressions**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject game_session
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli launch_v2
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli play_v2
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-runtime
```

Expected: all remote/local worker tests pass; no live device is opened.

- [ ] **Step 6: Commit Task 4**

```bash
git add crates/wroid-inject/src/game_session.rs crates/wroid-cli/src/commands/play_v2.rs crates/wroid-cli/src/commands/launch_v2.rs crates/wroid-cli/src/cli.rs
git commit -m "Runtime: route game touch through daemon"
```

---

### Task 5: Daemon Process and Attachment Wiring

**Files:**
- Modify: `crates/wroid-daemon/src/process.rs`
- Modify: `crates/wroid-daemon/src/ipc.rs`
- Modify: `crates/wroid-daemon/src/lib.rs`
- Modify: `crates/wroid-cli/src/commands/runtime_daemon.rs`

**Interfaces:**
- Consumes: `PersistentPlatform`, `PlatformLaunch`, `runtime_socket_pair()`, `RuntimeChannelServer`, `RUNTIME_WORKER_FD`, and generation `2`.
- Produces: `ManagedProcess { child, attachment, stop_requested }` and one lazy persistent platform per `ManagedProcesses` owner.

- [ ] **Step 1: Add RED fixed-command and ownership tests**

Update request fixtures to generation 2. Prove the daemon passes only `--runtime-fd 198`, contains no helper path or `--bridge-fd`, inherits only that descriptor, enqueues the attachment only after successful spawn, rejects generation 0/1 before spawn, combines worker and attachment failures, and keeps the same platform across two reaped workers.

```rust
#[test]
fn worker_arguments_carry_only_runtime_capability() {
    let args = launch_arguments(&profile_path(), &launch_request(), 4242);
    assert!(args.windows(2).any(|pair| pair == ["--runtime-fd", "198"]));
    assert!(!args.iter().any(|arg| arg == "--bridge-fd"));
    assert!(!args.iter().any(|arg| arg.to_string_lossy().contains("wroid-helper")));
}

#[test]
fn spawn_failure_does_not_initialize_platform() {
    let calls = shared_platform_calls();
    let mut processes = ManagedProcesses::with_platform(fake_platform(calls.clone()));
    assert!(processes.launch_with_missing_worker(launch_request()).is_err());
    assert!(calls.lock().unwrap().is_empty());
}
```

- [ ] **Step 2: Run process/IPC tests and verify RED**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon process -- --nocapture
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon ipc -- --nocapture
```

Expected: old bridge broker assertions fail and missing platform wiring does not compile.

- [ ] **Step 3: Replace per-worker broker with the persistent platform**

`ManagedProcesses` owns `Option<PersistentPlatform>` and creates the production instance lazily with the already authenticated current-user UID. Launch order is validate request, create seqpacket pair, configure/spawn worker, convert the daemon endpoint to `RuntimeChannelServer`, then call `platform.attach(...)`. If attach fails, terminate and reap the exact child before returning the launch error.

```rust
struct ManagedProcess {
    child: Child,
    attachment: Option<PlatformAttachment>,
    stop_requested: bool,
}

fn combine_reaped_detail(
    status: ExitStatus,
    attachment_result: io::Result<RuntimeAttachmentReport>,
    stop_requested: bool,
) -> (bool, String);
```

On reap, wait for `PlatformAttachment::finish()` after the worker descriptor closes and include cleanup/protocol errors in the session detail. On daemon drop, terminate/reap workers first, finish their attachments, then drop the persistent platform so its shutdown order remains deterministic.

- [ ] **Step 4: Update generation and stale-daemon compatibility**

Use `RUNTIME_WORKER_PROTOCOL_GENERATION` in daemon IPC validation and client launch requests. A daemon reporting protocol v2 but old worker generation must still be replaced through the existing authenticated pidfd handoff only when it has no active session. Keep all current stale-daemon security tests.

- [ ] **Step 5: Run GREEN and daemon handoff regressions**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli runtime_daemon
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli launch_v2
```

Expected: all daemon ownership, stop, reap, pidfd handoff, and CLI launch tests pass.

- [ ] **Step 6: Commit Task 5**

```bash
git add crates/wroid-daemon/src/process.rs crates/wroid-daemon/src/ipc.rs crates/wroid-daemon/src/lib.rs crates/wroid-cli/src/commands/runtime_daemon.rs
git commit -m "Daemon: persist runtime across game workers"
```

---

### Task 6: Remove the Superseded Per-Launch Broker and Harden Failures

**Files:**
- Delete: `crates/wroid-inject/src/bridge_broker.rs`
- Modify: `crates/wroid-inject/src/lib.rs`
- Modify: `crates/wroid-daemon/src/process.rs`
- Modify: `crates/wroid-daemon/src/platform.rs`
- Modify: `crates/wroid-daemon/src/production_platform.rs`
- Modify: `crates/wroid-inject/src/game_session.rs`

**Interfaces:**
- Consumes: Tasks 1-5 production runtime path.
- Produces: one closed production ownership path with no `BridgeBrokerClient` or `serve_bridge_broker` references.

- [ ] **Step 1: Add RED failure-matrix tests**

Cover worker SIGTERM with successful cancellation, EOF with 10 active contacts, invalid packet followed by cleanup failure, helper death after readiness, Waydroid resolution restart failure, attachment error plus non-zero worker exit, daemon drop during active input, and clean retry after failed initialization.

```rust
#[test]
fn worker_and_attachment_failures_are_both_visible() {
    let (success, detail) = combine_reaped_detail(
        exit_status(7),
        Err(io::Error::other("contact cleanup failed")),
        false,
    );
    assert!(!success);
    assert!(detail.contains("exit status: 7"));
    assert!(detail.contains("contact cleanup failed"));
}

#[test]
fn daemon_drop_cancels_before_platform_shutdown() {
    let calls = shared_calls();
    drop(active_managed_process_fixture(calls.clone(), ten_contact_down_frame()));
    assert_order(&calls, &["worker:term", "touch:cancel-all", "waydroid:stop", "helper:finish", "uinput:drop"]);
}
```

- [ ] **Step 2: Run the failure matrix and verify RED**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon failure -- --nocapture
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject cleanup -- --nocapture
```

Expected: at least the combined-error, helper-death, and daemon-drop ordering assertions fail before hardening.

- [ ] **Step 3: Implement deterministic poison/retry and combined cleanup**

Mark the production backend unready after helper/runtime-channel/uinput failure, take every owned component, and run cleanup in fixed order while accumulating bounded details. Preserve a ready backend after ordinary worker EOF or typed finish. Ensure `PlatformAttachment::finish()` cannot block forever after the exact worker has exited; the runtime socket read deadline must terminate it.

```rust
fn poison_and_cleanup(&mut self, primary: io::Error) -> io::Error {
    self.ready = false;
    self.resolution = None;
    combine_platform_errors(primary, self.driver.shutdown().err())
}

fn attachment_failure_requires_reinitialize(error: &io::Error) -> bool {
    !matches!(error.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset)
}
```

Treat typed `finish` and plain worker EOF as attachment completion after successful `cancel_all`; keep the backend ready. Treat helper health failure, injection failure, invalid protocol, or failed contact cancellation as poison and fully clean the driver before the next retry.

- [ ] **Step 4: Delete the old broker and prove there is one production path**

Remove `bridge_broker.rs`, its module/re-exports, JSON protocol dependencies used only by it, and old broker fixtures. Keep helper traits in `privileged_bridge.rs` and root diagnostic bridge code.

Run:

```bash
! rg -n 'BridgeBrokerClient|serve_bridge_broker|BRIDGE_WORKER_FD|BRIDGE_WORKER_PROTOCOL_GENERATION|--bridge-fd' crates
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli
```

Expected: `rg` finds nothing and all three package suites pass.

- [ ] **Step 5: Commit Task 6**

```bash
git add -A crates/wroid-inject/src/bridge_broker.rs crates/wroid-inject/src crates/wroid-daemon/src crates/wroid-cli/src
git commit -m "Runtime: retire per-launch bridge lifecycle"
```

---

### Task 7: Headless Performance Gate and Product Documentation

**Files:**
- Create: `crates/wroid-inject/src/bin/wroid-runtime-channel-bench.rs`
- Modify: `crates/wroid-inject/Cargo.toml`
- Modify: `docs/architecture-v2.md`
- Modify: `docs/waydroid-input-bridge.md`
- Modify: `docs/roadmap.md`
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-08-09-persistent-daemon-touchscreen-design.md`
- Modify: `docs/superpowers/plans/2026-08-09-persistent-daemon-touchscreen.md`

**Interfaces:**
- Consumes: the final runtime channel and persistent uinput implementation.
- Produces: a no-Waydroid benchmark executable, current operator docs, and completed plan/status evidence.

- [ ] **Step 1: Add the deterministic headless benchmark**

The binary creates a real `UinputTouchInjector` at canonical range, a real seqpacket client/server pair, and a server thread. It submits 10 Down contacts, distributes at least 20,000 acknowledged Move frames across them, sends 10 Up contacts, finishes cleanly, and prints machine-readable totals and p50/p95/p99/max microseconds.

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = DeviceConfig::with_slots(65_536, 65_536, 10)?;
    let engine = TouchEngine::new(UinputTouchInjector::open(config)?);
    let (client_fd, server_fd) = runtime_socket_pair()?;
    let server = std::thread::spawn(move || run_benchmark_server(server_fd, engine));
    let summary = run_20k_acknowledged_frames(client_fd)?;
    let report = server.join().map_err(|_| "benchmark server panicked")??;
    print_summary(summary, report);
    validate_gate(summary, report)?;
    Ok(())
}
```

```text
runtime_channel_frames=20002
runtime_channel_peak_contacts=10
runtime_channel_released_contacts=10
runtime_channel_p99_micros=...
runtime_channel_result=PASS
```

Exit non-zero unless frames are at least 20,000, peak/released contacts are exactly 10, no contact remains, and p99 is below 5,000 microseconds.

- [ ] **Step 2: Run the benchmark and focused release tests**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo run --release -p wroid-inject --bin wroid-runtime-channel-bench
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test --release -p wroid-inject runtime_channel
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test --release -p wroid-daemon platform
```

Expected: benchmark `PASS`, 20,000+ acknowledged frames, 10/10 released, p99 below 5 ms.

- [ ] **Step 3: Update architecture, bridge guide, roadmap, and README**

Document daemon ownership, first-use controlled restart, no per-game shutdown, runtime protocol generation 2, root diagnostic exception, crash cleanup, benchmark command/result, and the deferred live-hotplug reconcile. Check off roadmap “Productionize bridge lifecycle, reconciliation, and stable device discovery” only for persistence within a daemon lifetime; leave live hot-plug after abrupt replacement explicitly open.

- [ ] **Step 4: Run the full non-GUI release gate**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test --workspace --all-targets --all-features
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 RUSTFLAGS='-D warnings' cargo clippy --workspace --all-targets --all-features
cargo fmt --all -- --check
find apps -type f -name '*.js' -print0 | xargs -0 -n1 node --check
node --test apps/controls-studio/web/tests/*.test.js apps/hub/web/tests/*.test.js
for profile in examples/profiles/v2/*.json; do cargo run --quiet -p wroid-cli -- profile validate "$profile"; done
git diff --check
git status --short
```

Expected: every test/check passes; only intended task files are changed.

- [ ] **Step 5: Commit Task 7**

```bash
git add crates/wroid-inject/Cargo.toml crates/wroid-inject/src/bin/wroid-runtime-channel-bench.rs README.md docs/architecture-v2.md docs/waydroid-input-bridge.md docs/roadmap.md docs/superpowers/specs/2026-08-09-persistent-daemon-touchscreen-design.md docs/superpowers/plans/2026-08-09-persistent-daemon-touchscreen.md
git commit -m "Docs: ship persistent daemon touchscreen"
```

---

### Task 8: Review, Release Install, and Announced Live Acceptance

**Files:**
- Modify only when review finds a concrete defect; keep every fix in a focused commit.

**Interfaces:**
- Consumes: Tasks 1-7 and the installed release workflow.
- Produces: reviewed exact release binaries and one live two-session reuse result.

- [ ] **Step 1: Run specification and security review**

Review every design invariant against the diff, especially packet bounds, ACK/state atomicity, helper privilege isolation, daemon-drop ordering, same-resolution reuse, generation rejection, inherited-FD closure, and combined error reporting. Fix every Critical/Important finding with a RED regression and rerun its package gates.

- [ ] **Step 2: Re-run fresh full non-GUI verification after review fixes**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test --workspace --all-targets --all-features
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 RUSTFLAGS='-D warnings' cargo clippy --workspace --all-targets --all-features
cargo fmt --all -- --check
git diff --check
```

Expected: zero failures and no warnings.

- [ ] **Step 3: Build and install the exact release**

```bash
cargo build --release -p wroid-cli -p wroid-daemon -p wroid-inject --bin wroid-helper
install -m 0555 target/release/wroid "$HOME/.local/bin/wroid"
install -m 0555 target/release/wroidd "$HOME/.local/bin/wroidd"
install -m 0555 target/release/wroid-helper "$HOME/.local/share/libexec/wroid/wroid-helper"
wroid helper install
```

Verify staged/installed helper equality, root ownership/mode `4750`, current daemon identity, and helper `--check` before live use.

- [ ] **Step 4: Warn the user and run one bounded two-session acceptance**

Immediately before this step, send a commentary update that Waydroid may visibly open once. Then run two bounded no-APK sessions through the same daemon. Record daemon PID, helper PID, uinput event node, Waydroid state, and bridge include before/after each worker. Required assertions:

- first worker reaches ready and releases every contact/grab;
- after first worker exit, Waydroid, helper, event node, and bridge remain ready;
- second worker uses the same daemon/helper/event node without a Waydroid stop/start cycle;
- stopping the daemon releases all contacts, stops Waydroid, exits helper/worker, removes the managed bridge include, and destroys the uinput node.

- [ ] **Step 5: Commit review fixes/evidence and hand off the result**

```bash
git status --short --branch
git log --oneline --decorate -12
```

Report the non-GUI gate counts, benchmark p99, live reuse evidence, installed paths, any remaining deferred live-hotplug limitation, and the next roadmap block. Do not claim completion if any required assertion failed.
