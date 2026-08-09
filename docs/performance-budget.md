# Performance Budget

Performance work is accepted against measured budgets rather than subjective
"fast" behaviour. Measurements must identify hardware, kernel, compositor,
Waydroid image, renderer, Android resolution, and profile.

## Input

| Metric | Initial acceptance target |
| --- | ---: |
| Process creation in gaming input hot path | 0 |
| Profile/control-plan clones in input hot path | 0 |
| Heap allocation per steady-state mouse-motion runtime dispatch | 0 |
| Capture-to-inject latency, p95 | < 5 ms |
| Capture-to-inject jitter, p95 | < 2 ms |
| Simultaneous logical touch contacts | >= 10 |
| Lost release/cancel transitions | 0 |
| Cleanup after focus loss or normal shutdown | 100% |
| Runtime state commit after failed injection | 0 contacts changed |

The first benchmark compares the existing shell backend with the persistent
backend on the same host. Results must include median, p95, p99, and maximum.
Production game-session reports include both reader-to-inject and evdev kernel
timestamp-to-inject p50/p95/p99/max over batches that actually submit a touch
frame. Timestamps with a future clock or an implausible age are rejected and
reported instead of contaminating percentiles. Wroid persists these metrics,
touch-frame count, and peak contact count in the bounded private last-session
record; the Hub highlights reader-to-inject p95 above the 5 ms budget. Hardware
acceptance still needs a recorded live session on each supported host class.

The Hub's bounded input self-test runs this same production session with
package launch disabled and tracing enabled. Its 20-second live interval makes
hardware latency and cleanup validation available before game installation;
sudo authorization and Android boot occur before the timed interval begins.

The daemon/helper bridge protocol runs only during bridge open, Android
readiness verification, and cleanup. Steady-state keyboard/mouse dispatch and
touch injection remain in the desktop-user worker and do not cross daemon IPC,
so daemon ownership adds no request/response work to the measured input hot
path. The foreground CLI log relay is also outside the worker and polls only
daemon session state.

Hub does not poll system probes periodically while a game is running. It
performs one deduplicated state refresh after a launch handoff and refreshes
again only when its browser regains focus or becomes visible, preventing the
launcher UI from adding recurring gameplay process or graphics-probe load.

The unified runtime borrows its immutable control plan in place. Mouse motion
iterates the already materialized aim controllers directly; keyboard, mouse
button, and reaffirm paths no longer clone the plan or binding names per event.
Normal one- and two-event touch frames use fixed inline storage, and
`TouchEngine` validates before injection then commits in place instead of
cloning its contact map. Consequently, an already-active mouse-aim MOVE reaches
the preallocated uinput buffers without a runtime heap allocation. Joystick
bookkeeping allocates a binding key only on its first direction event after
startup or suspension; large cleanup frames retain a heap fallback.

Input readers park in `poll` on the evdev descriptor instead of waking on a
fixed timer, so a keystroke or mouse report is picked up as soon as the kernel
queues it rather than after a scheduling tick. The wait stays bounded because
capture toggles and shutdown arrive over a channel that `poll` cannot observe.

Scaled mouse motion carries its sub-pixel remainder across events. Integer
division alone discards every delta smaller than the scale denominator, so any
sensitivity below 1.0 previously dropped slow aim movement entirely; the
accumulator keeps slow tracking proportional and makes total travel match the
configured sensitivity. It resets on deactivation and on ADS transitions so a
remainder captured under one scale cannot leak into another.

Release builds are tuned for steady-state latency: `opt-level = 3`, fat LTO, a
single codegen unit, and `panic = "abort"` allow inlining across the reader,
runtime, and injector crates and remove unwinding machinery from the injection
path.

`wroid-inject-latency` measures the injection hot path alone — no evdev capture,
no Android, no root, no device grab. It walks one contact around the virtual
touchscreen and reports mean/p50/p95/p99/max per touch frame, flagging any frame
over the 5 ms budget. Measured baseline on the AMD RX 6600 XT / 7.1.5-cachyos /
KDE Wayland development host, 20 000 frames:

| Build | mean | p50 | p95 | p99 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| release | 0.8 us | 0.7 us | 1.1 us | 1.3 us | 20.9 us |
| dev | 1.3 us | 1.2 us | 1.6 us | 1.9 us | 117.6 us |

Injection therefore consumes a negligible share of the 5 ms capture-to-inject
budget on this host, and the release profile matters most for tail stability.

## Rendering

| Metric | Initial acceptance target |
| --- | ---: |
| Software renderer detection | mandatory |
| Additional local video encode/decode stages | 0 |
| Frame-time telemetry overhead | < 0.5 ms per frame |
| Stable frame pacing | no recurring Wroid-induced spikes > 2 ms |

FPS alone is insufficient. Reports must include a frame-time distribution and
1% low behaviour.

The implemented `wroid performance` preflight records the active host renderer,
direct/accelerated flags, DRM devices and drivers, Waydroid EGL/gralloc/Vulkan
properties, desktop session, active resolution, and refresh rate. `launch-v2`
and the Hub refuse to start a game when this probe identifies a software
renderer. Unknown fields remain visible warnings because some hosts do not ship
the optional `glxinfo`, `eglinfo`, or `xrandr` utilities.

On hosts with more than one DRM GPU, the report maps each card to its render
node and compares the active host renderer with Waydroid's
`gralloc.gbm.device`. `wroid performance --setup-gpu` applies Waydroid's native
`drm_device` configuration through a visible authorization terminal. The write
is atomic, creates a first-use backup, uses `waydroid upgrade -o`, and restores
the previous configuration if regeneration fails. When Waydroid is stopped or
its DRM property is unavailable, the report keeps Android graphics unknown and
never guesses that a GPU switch is needed. The interactive setup preserves the
desktop session state: it stops a running session before regeneration, restores
it on success or cancelled authorization, and checks the restarted Android
`gralloc.gbm.device` value before claiming that the switch is active.
The Hub and Controls Studio presets configure Waydroid's persistent Android
width and height, verify both property readbacks transactionally, and restart
the session once only after an actual change. The launch then confirms the
effective Android `wm size` before enabling the virtual touchscreen or starting
the package, so the selected performance target and touch coordinate surface
cannot silently diverge.

Waydroid's hardware composer derives Android's vsync period from the maximum
active refresh advertised by the Wayland compositor, with a 60 Hz fallback.
Wroid therefore reports that compositor target and the effective
`persist.waydroid.no_presentation` state instead of inventing an unsupported
FPS property. The normal default keeps `wp_presentation` feedback enabled for
accurate phase timing; explicitly disabling it is a visible warning. Offline
reports leave the Android pacing target unknown rather than equating it with
the host display probe.

Normal Hub launches request Feral GameMode by default when a protected system
`gamemoderun` is installed. This can apply host CPU, scheduler, I/O, and GPU
policy configured by the machine without adding work to Wroid's gameplay hot
path. The user can persistently select Off. The daemon accepts only a boolean,
resolves the wrapper from fixed absolute paths, requires a canonical root-owned
executable that is not group/other writable, and clears `GAMEMODERUNEXEC` and
`LD_PRELOAD` before spawn. A missing optional wrapper falls back to the direct
worker and is never a launch blocker.

## Lifecycle

| Metric | Initial acceptance target |
| --- | ---: |
| Warm launch regression | blocks merge when > 10% |
| Input capture release after daemon exit | < 1 s |
| Active touch cleanup after controlled stop | immediate final frame |
| Configuration restoration after stop | deterministic and tested |

## CI and regression policy

Unit tests verify state-machine invariants on every pull request. Hardware and
Waydroid integration benchmarks run on dedicated Linux hosts once that runner is
available. Benchmark output is stored as an artifact and compared with the last
accepted baseline.
