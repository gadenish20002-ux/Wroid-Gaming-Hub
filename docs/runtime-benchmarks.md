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

## Next benchmark targets

1. Add daemon IPC timing once `wroidd` exists.
2. Add privileged-helper timing once `wroid-helper` owns evdev/uinput access.
3. Add Android-visible timing by pairing injected frames with `waydroid shell -- getevent` capture.
4. Store p50/p95/p99 baselines per machine/GPU/compositor profile.
