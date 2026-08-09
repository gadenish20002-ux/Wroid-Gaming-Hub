# Runtime Benchmarks

Wroid's gaming hot path must be measured separately from compatibility backends.
The first benchmark tool focuses on the host-side path that is already available
without starting Waydroid:

```text
physical evdev keyboard
  -> wroid-input normalization
  -> DirectionalKeyState
  -> VirtualJoystick
  -> TouchEngine
  -> in-memory recording injector
```

This does **not** measure Android `getevent` timing yet. It is a regression
harness for the part of the pipeline that should remain extremely small before
adding daemon IPC and the privileged helper.

## Build

```sh
cargo build --release --bin wroid-bench-host
```

## Run

Find the keyboard event node:

```sh
ls -l /dev/input/by-id/
```

Run without an exclusive grab first:

```sh
sudo ./target/release/wroid-bench-host /dev/input/event7 --samples 200
```

Run with an exclusive kernel grab when you want cleaner diagnostics and are ready
for the selected keyboard to stop reaching the compositor while the tool is
active:

```sh
sudo ./target/release/wroid-bench-host /dev/input/event7 --samples 200 --grab
```

Press and release `W`, `A`, `S`, and `D` until the requested number of samples is
collected. Press `Esc` to stop early.

## Output

The tool reports:

- direction-change samples;
- evdev blocking read calls;
- ignored/repeat events;
- submitted runtime frames;
- recording-injector frame/event counts;
- min/p50/p95/p99/max for host pipeline time;
- min/p50/p95/p99/max for blocking evdev read time.

`host pipeline` is the useful regression metric. `evdev blocking read` includes
how long the process waited for the next physical event and is mostly useful for
sanity checks.

## Injection latency (`wroid-inject-latency`)

`wroid-bench-host` needs a physical keyboard and an exclusive grab, which makes
it awkward to run repeatedly on a working desktop. `wroid-inject-latency`
isolates the other end of the pipeline instead — the `TouchEngine` submit that
every gameplay input ends in — and needs no root, no Waydroid session, and no
device grab:

```text
TouchEngine submit
  -> slot/tracking-id translation
  -> real uinput virtual touchscreen
```

```sh
cargo build --release --bin wroid-inject-latency
target/release/wroid-inject-latency --samples 20000
```

It walks one contact around the surface so no frame can be skipped as a no-op,
discards a 256-frame warm-up, then reports per-frame mean/p50/p95/p99/max and
flags any frame over the 5 ms budget. Afterwards it holds all ten advertised
slots simultaneously and releases them, failing loudly if the kernel does not
accept the full advertised contact count.

### Baseline

Host: AMD Radeon RX 6600 XT (radeonsi, navi23), kernel 7.1.5-1-cachyos, KDE
Plasma 6 on Wayland, 1920x1080 @ 239.66 Hz, Waydroid MAINLINE x86_64 with
libhoudini. 20 000 frames at 1920x1080.

| Build | mean | p50 | p95 | p99 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| release | 0.8 us | 0.7 us | 0.9 us | 1.0 us | 27.5 us |
| dev | 1.3 us | 1.2 us | 1.6 us | 1.9 us | 117.6 us |

Injection is therefore not the bottleneck on this host: it consumes roughly
0.02% of the 5 ms capture-to-inject budget. The release profile matters mostly
for tail stability — its maximum is about four times lower than the debug
build's. Ten simultaneous contacts are held and released cleanly.

## Next benchmark targets

1. Add daemon IPC timing once `wroidd` exists.
2. Add privileged-helper timing once `wroid-helper` owns evdev/uinput access.
3. Add Android-visible timing by pairing injected frames with `waydroid shell -- getevent` capture.
4. Store p50/p95/p99 baselines per machine/GPU/compositor profile.
