# Task 2 report: runtime control-plan materialization

## Status

Implemented the runtime layer/key control plan in `wroid-runtime` without adding a
dependency on `wroid-input` and without changing action geometry/contact
materialization.

## Delivered behavior

- Added `LayerId(u16)` with `LayerId::BASE == 0`; declared layers receive ids
  `1..=N` in profile order.
- Added resolved `RuntimeLayer` records with `LayerMode::{Hold, Toggle}` and
  resolved activation keys. The implicit base layer is not represented by a
  fake declared layer or activation key.
- Added `HostKeyName`, a `repr(u8)`, copyable runtime key enum covering all 46
  currently profile-visible keyboard keys, excluding reserved F12. Parsing
  trims and matches ASCII case-insensitively. Every value exposes stable
  `index()` and `bit()` values for `u64` masks.
- Added `ModifierMask`, resolved binding modifiers, and a plan-wide
  `modifier_keys` mask for allocation-free modifier tracking.
- Added `HostMouseButton` and `RuntimePhysicalInput` so keyboard keys and mouse
  buttons have distinct compact identities.
- Added fixed-capacity sibling suppression metadata to every runtime binding.
  `RuntimeControlBinding::is_suppressed(physical_input, held_modifiers)` uses
  only fixed-array lookup and bit intersection; it neither allocates nor scans
  other controls per event.
- Suppression is scoped by resolved layer and individual physical input.
  Consequently, `Shift+W` suppresses only W in an unmodified WASD cluster,
  cross-layer siblings do not suppress, and mouse-button siblings work.
- Binding layer resolution runs before `ProfileV2::validate()`, ensuring an
  unknown layer is observable as `RuntimeControlPlanError::UnknownLayer`.
- Exported the new runtime types from `wroid-runtime` for the dispatch task.

## TDD evidence

### Initial baseline observation

Before adding Task 2 tests, the focused package did not compile because Task 1
had added `ProfileV2.layers` and `BindingV2.{layer,modifier}` while the existing
runtime test fixtures had not yet supplied those fields:

```text
$ CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-runtime --all-features --no-run
error[E0063]: missing fields `layer` and `modifier` in initializer of `BindingV2`
error[E0063]: missing field `layers` in initializer of `ProfileV2`
error: could not compile `wroid-runtime` (lib test) due to 5 previous errors
```

The test-only fixtures were updated to the Task 1 schema before establishing
Task 2 RED.

### RED

Behavioral tests were written first for stable layer ids/modes, base binding
resolution, resolved activation/modifier keys, dedicated unknown-layer errors,
known-key bit identity/F12 exclusion, multi-modifier sibling suppression,
per-key cluster suppression, cross-layer isolation, and mouse-button
suppression.

```text
$ CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-runtime --all-features profile_controls::tests
error[E0422]: cannot find struct, variant or union type `RuntimeLayer` in this scope
error[E0609]: no field `layers` on type `RuntimeControlPlan`
error[E0609]: no field `layer` on type `&RuntimeControlBinding`
error[E0599]: no variant named `UnknownLayer` found for enum `RuntimeControlPlanError`
error[E0599]: no method named `is_suppressed` found for reference `&RuntimeControlBinding`
error: could not compile `wroid-runtime` (lib test) due to 56 previous errors
```

This was the expected failure: the requested runtime API and behavior did not
exist.

### Focused GREEN

After the minimal implementation:

```text
$ CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-runtime --all-features profile_controls::tests
running 10 tests
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 33 filtered out
```

### Final package and formatting gates

Fresh verification after formatting and self-review assertions:

```text
$ CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-runtime --all-features
running 43 tests
test result: ok. 43 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

Doc-tests wroid_runtime
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo fmt --all -- --check
exit 0

$ git diff --check
exit 0
```

## Self-review

- Confirmed key/button identities are separate, so arrow `left` cannot collide
  with mouse button `left`.
- Confirmed sibling masks are built only for the same `LayerId` and only from
  modifier-specific siblings.
- Confirmed modified bindings cannot themselves be suppressed by this API.
- Confirmed each binding has at most four distinct physical constituents, so
  the fixed four-entry suppression table covers every current `InputV2` shape.
- Confirmed the parser list matches `wroid-core`'s current known-key list and
  intentionally omits F12.
- Confirmed no changes were made to `materialize_action`.
- Confirmed unrelated dirty latency/helper work was not staged or modified by
  this task.

## Files

- `crates/wroid-runtime/src/profile_controls.rs`
- `crates/wroid-runtime/src/lib.rs`
- `.superpowers/sdd/План_реазизации-profile-layers-and-modifiers/task-2-report.md`

## Concerns

None for Task 2. The next dispatch task must convert captured input enums to
`HostKeyName`/`HostMouseButton` once per event and use the resolved masks/API;
it should not reintroduce profile-string matching for layer/modifier checks.
