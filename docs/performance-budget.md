# Performance Budget

Performance work is accepted against measured budgets rather than subjective
"fast" behaviour. Measurements must identify hardware, kernel, compositor,
Waydroid image, renderer, Android resolution, and profile.

## Input

| Metric | Initial acceptance target |
| --- | ---: |
| Process creation in gaming input hot path | 0 |
| Capture-to-inject latency, p95 | < 5 ms |
| Capture-to-inject jitter, p95 | < 2 ms |
| Simultaneous logical touch contacts | >= 10 |
| Lost release/cancel transitions | 0 |
| Cleanup after focus loss or normal shutdown | 100% |
| Runtime state commit after failed injection | 0 contacts changed |

The first benchmark compares the existing shell backend with the persistent
backend on the same host. Results must include median, p95, p99, and maximum.

## Rendering

| Metric | Initial acceptance target |
| --- | ---: |
| Software renderer detection | mandatory |
| Additional local video encode/decode stages | 0 |
| Frame-time telemetry overhead | < 0.5 ms per frame |
| Stable frame pacing | no recurring Wroid-induced spikes > 2 ms |

FPS alone is insufficient. Reports must include a frame-time distribution and
1% low behaviour.

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
