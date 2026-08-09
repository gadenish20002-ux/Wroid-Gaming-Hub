# Plan: profile layers and modifiers

Status: implemented and verified in automated gates; live game calibration remains a follow-up.
Implementation evidence: the post-layer release benchmark completed over
20,000 frames at mean 0.8 us, p50 0.7 us, p95 1.0 us, p99 1.3 us, and max
22.5 us; every frame stayed below 5 ms, all 10 contacts were active
simultaneously, and the final release was clean.
Author context: written against commit `eccc0cb` plus the uncommitted
latency/mouse-accumulator work.

## 1. Why

Every binding in a profile v2 document is unconditionally live. A physical key
maps to exactly one Android action for the whole session. Two consequences block
the target games:

- The keyboard runs out of comfortable keys. Standoff 2 and PUBG Mobile need
  movement, aim, fire, ADS, reload, jump, crouch, two weapon slots, grenades,
  heals, scope levels, vehicle controls, and emotes. `profiles/examples/standoff2-v2.json`
  already spends ten bindings and covers only the basics.
- There is no way to express "while I hold a key, the map means something else",
  which is how BlueStacks-class tools expose grenade wheels, scope selection,
  and vehicle modes.

This plan adds two independent mechanisms:

- **Modifier**: a binding may require a modifier key to be held. `R` reloads,
  `Shift+R` switches fire mode.
- **Layer**: a named set of bindings that is active only while its activation
  key is held, or after it is toggled on. Layer `grenades` can rebind `1..4` to
  frag/smoke/flash/molotov without touching the base map.

Both are additive schema changes. Existing profiles keep working untouched.

## 2. Design decisions

Read this section before writing code; several cheaper-looking alternatives are
rejected here for reasons that are not obvious from the call sites.

### 2.1 Additive fields, not a new `InputV2` variant

`InputV2` and `ActionV2` are `#[serde(tag = "kind")]` enums
(`crates/wroid-core/src/profile_v2.rs:110`, `:128`). Adding a variant such as
`KeyWithModifier` forces every exhaustive `match` in the workspace to change:
`profile_v2.rs:437-454`, `crates/wroid-runtime/src/profile_controls.rs:103-198`,
`crates/wroid-core/src/bin/wroid-profile-v2-validate.rs:108-160`,
`crates/wroid-cli/src/commands/session.rs:128-140`,
`crates/wroid-cli/src/commands/hub.rs:643-657`, and the dispatch arms in
`crates/wroid-inject/src/game_session.rs:996-1139`.

Instead put `modifier` on `BindingV2` and `layer` on `BindingV2`. No enum
variant is added, no exhaustive match breaks, and the change is one struct.

### 2.2 Modifier belongs on the binding, not inside `InputV2::Key`

A modifier is meaningful for `Key`, `KeyCluster`, and `MouseButton` inputs, and
meaningless for `MouseMove`. Putting it in one enum variant would force
duplicating it into three. `BindingV2.modifier` covers all three and validation
rejects it for `MouseMove`.

### 2.3 Layers are declared, not inferred

Declare layers in a top-level `layers: Vec<LayerV2>` list, and have bindings
reference a layer by name. The alternative — inferring the layer set from
whatever strings bindings mention — makes typos silently create dead layers and
gives the editor nothing to enumerate.

`layer: None` on a binding means the base layer, which is always active. This is
what every existing binding deserializes to.

### 2.4 Schema version stays at 2

`PROFILE_SCHEMA_VERSION` stays `2`. Reasons:

- `validate()` hard-rejects any other value (`profile_v2.rs:53-58`), and that
  check runs on load in `editor.rs:307`, `hub.rs:318`, `hub.rs:1845`,
  `play_v2.rs:27`, `launch_v2.rs:51`, and inside `save_to_path`
  (`profile_v2.rs:32`). Bumping to 3 makes every existing user profile invalid
  until a migration runs, and there is no migration machinery in the project.
- The new fields are `#[serde(default)]`, and none of the profile structs use
  `deny_unknown_fields`, so both directions already work: an old profile loads
  in a new binary, and a new profile loads in an old binary (dropping the new
  fields on rewrite).

Write this reasoning into `docs/profile-v2.md` so the next person does not
"fix" it by bumping the version.

### 2.5 Modifier state has action-specific edge semantics

The runtime tracks modifier keys as state, but the effect depends on the action:

- Continuous `Hold` and hold-mode `VirtualJoystick` bindings reconcile whenever
  the action key, modifier, or layer state changes. The modifier may therefore
  arrive after the action key is already held; ownership transitions without a
  leaked or overlapping contact.
- Zero-latency `Tap` samples layer/modifier scope only on the physical action-key
  press edge. A Tap chord requires the modifier to be down before that edge.
  The runtime does not delay a Base tap waiting for a possible modifier and
  never refires or switches a Tap on modifier/layer-only changes.

Release remains correct independently of current availability: see §2.7.

### 2.6 Reserved keys

`game_session.rs:800-822` swallows `F12` before profile dispatch, and
`:827-838` returns on `Ctrl+Esc` and `Ctrl+C` before `handle_keyboard`. So:

- `f12` is already absent from `known_key_name` (`profile_v2.rs:496-545`) and
  from the editor's `supportedKeys` (`assets/editor/app.js:6-11`). Nothing to do.
- `ctrl` + `esc` and `ctrl` + `c` must be **rejected by validation**. Today
  nothing stops a user from creating that binding, and it would silently never
  fire because the event loop returns first. Add the check in both
  `profile_v2.rs` validation and `app.js` `validateProfile`.
- A binding whose modifier is `ctrl` still works for other keys. Note that
  `control_pressed` (`game_session.rs:824`) is tracked separately and the `ctrl`
  event is *also* forwarded to `handle_keyboard` (`:839`). The new modifier
  tracking must live inside `handle_keyboard` and must not depend on
  `control_pressed`, or the two will drift.

### 2.7 Release correctness is the hard part

This is where a naive implementation breaks. Three scenarios must be handled,
and each needs a test:

1. **Modifier released while the action key is still held.** `Shift+W` holds a
   point; the user releases `Shift` but keeps `W`. The held contact must be
   released, because the binding's condition no longer holds. Without this the
   contact leaks until session end.
2. **Layer deactivated while a layer binding is held.** Same problem, one level
   up. Deactivating a layer must release every contact owned by bindings in
   that layer.
3. **Action key released after the modifier already went up.** The `Released`
   event arrives when the binding is no longer "matching". A naive
   `if matches_now` guard skips the release and leaks the contact.

The rule that solves all three: **press is gated by the current condition;
release is never gated.** Track which bindings are actively held, and release a
held binding whenever it is held and its condition stops being satisfied —
whether that is because the action key, the modifier, or the layer went away.

`UnifiedRuntime` already owns exactly the state needed for this:
`point_contacts` (`game_session.rs:890`), `directions` (`:889`), and
`aim_controllers` (`:891`). `suspend()` (`:1178-1184`) is the existing template
for "release everything cleanly" — reuse its shape for partial release.

### 2.8 Hot-path cost

`handle_keyboard` already walks all controls per event
(`game_session.rs:994`). Layer and modifier checks add two comparisons per
control and no allocation, so the existing budget in
`docs/performance-budget.md` holds. Constraints:

- Store active modifiers and active layers in fixed-size or preallocated
  containers. Do **not** allocate a `String` per event. The existing code takes
  care to avoid this (see the roadmap entry about removing control-plan clones);
  do not regress it.
- Compare interned indices, not strings, in the hot path. Resolve layer names
  to indices once in `RuntimeControlPlan::from_profile_v2`, the same way contact
  ids are allocated there (`profile_controls.rs:31`, `:200-204`).
- Re-run `target/release/wroid-inject-latency --samples 20000` after the change
  and compare against the baseline in `docs/runtime-benchmarks.md`
  (release p99 ≈ 1 µs). A regression above ~2 µs p99 means something allocates.

## 3. Schema

Target shape in `crates/wroid-core/src/profile_v2.rs`.

```rust
pub struct ProfileV2 {
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    pub name: String,
    pub package_name: String,
    #[serde(default)]
    pub orientation: Orientation,
    #[serde(default)]
    pub layers: Vec<LayerV2>,      // new
    #[serde(default)]
    pub bindings: Vec<BindingV2>,
}

pub struct BindingV2 {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,     // new; None = base layer
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifier: Option<String>,  // new; a key name from known_key_name
    pub input: InputV2,
    pub action: ActionV2,
}

pub struct LayerV2 {
    pub name: String,
    pub activation: LayerActivation,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayerActivation {
    /// Active only while this key is held.
    Hold { key: String },
    /// Flips on press, stays until pressed again.
    Toggle { key: String },
}
```

`skip_serializing_if` keeps existing profiles byte-identical when the editor
rewrites them, which matters because `hub.rs:277-300` `starter_predecessors`
compares whole `ProfileV2` values with `PartialEq` to decide whether a starter
profile is untouched. Without `skip_serializing_if`, every saved profile grows
`"layer": null` noise and that comparison starts misfiring.

JSON example — a grenade layer:

```json
{
  "schema_version": 2,
  "name": "Standoff 2 — layered",
  "package_name": "com.axlebolt.standoff2",
  "orientation": "landscape",
  "layers": [
    { "name": "grenades", "activation": { "kind": "hold", "key": "g" } }
  ],
  "bindings": [
    {
      "name": "primary_weapon",
      "input": { "kind": "key", "key": "1" },
      "action": { "kind": "tap", "point": { "x": 0.89, "y": 0.18 } }
    },
    {
      "name": "frag",
      "layer": "grenades",
      "input": { "kind": "key", "key": "1" },
      "action": { "kind": "tap", "point": { "x": 0.70, "y": 0.30 } }
    },
    {
      "name": "fire_mode",
      "modifier": "shift",
      "input": { "kind": "key", "key": "r" },
      "action": { "kind": "tap", "point": { "x": 0.93, "y": 0.60 } }
    }
  ]
}
```

## 4. Validation rules

Add to `validate()` (`profile_v2.rs:50-90`) and the helpers below it.

Layer list:

1. Layer name non-empty after trim; no duplicates. Mirror the binding-name
   check at `profile_v2.rs:66-73`.
2. `activation.key` passes `known_key_name` (`:496`).
3. Reject the reserved layer name `base` (case-insensitive) so it cannot shadow
   the implicit base layer.
4. No two layers share an activation key.
5. A layer activation key must not also be used as a plain binding key in the
   base layer — holding it would fire both. Report it as an error, not a
   warning; a silent double-fire is worse than a rejected save.

Per binding:

6. `layer`, when present, must name a declared layer. Unknown layer is an
   error, not silently ignored.
7. `modifier`, when present, passes `known_key_name`.
8. `modifier` must differ from the binding's own key, and must not be one of
   the keys inside a `KeyCluster`.
9. `modifier` is rejected for `InputV2::MouseMove` — mouse aim is continuous
   and has its own `toggle_key`. Put this in
   `validate_binding_compatibility` (`:405-435`), which already owns
   input/action pairing rules.
10. Reject `modifier: "ctrl"` combined with key `esc` or `c` — unreachable per
    §2.6. Message must say why, e.g. `binding X uses ctrl+esc, which is
    reserved for the session exit hotkey`.

Uniqueness, replacing today's implicit "any key may repeat" behaviour:

11. Within the same `(layer, modifier)` scope, the same key must not drive two
    bindings. Cross-scope duplicates are the entire point of the feature and
    must stay legal. The tuple to deduplicate on is
    `(layer.unwrap_or(base), modifier, key)`.
12. A layer activation key must not be used by any binding *inside that same
    layer* — the layer could never be released cleanly.

Every rule above gets one unit test in the `mod tests` block at
`profile_v2.rs:547`, following the existing style: build a profile, mutate one
field, assert the error message substring.

Mirror rules 1-12 in `assets/editor/app.js` `validateProfile`
(`app.js:883-922`) so the browser refuses to save the same documents the Rust
validator would reject. Keep the two lists in the same order and use the same
wording; they drift otherwise.

## 5. Runtime materialization

`crates/wroid-runtime/src/profile_controls.rs`.

1. Extend `RuntimeControlBinding` (`:56-61`) with resolved, hot-path-cheap
   fields:

```rust
pub struct RuntimeControlBinding {
    pub name: String,
    pub input: InputV2,
    pub action: RuntimeControlAction,
    pub layer: LayerId,           // new: interned index; LayerId::BASE for None
    pub modifier: Option<HostKeyName>, // new: resolved once, not re-parsed
}
```

   `LayerId` is a newtype over `u16` in `wroid-runtime`, with an associated
   `BASE` constant. Interning happens in `from_profile_v2` (`:25-48`), where
   contact ids are already allocated (`:31`, `:200-204`).

2. Add to `RuntimeControlPlan` (`:17-22`) the resolved layer table:

```rust
pub layers: Vec<RuntimeLayer>,  // index == LayerId
pub struct RuntimeLayer {
    pub name: String,
    pub activation_key: String,
    pub mode: LayerMode,   // Hold | Toggle
}
```

3. `from_profile_v2` resolves each binding's `layer` name to a `LayerId`.
   Unknown names cannot occur because `validate()` already ran at `:29`, but
   return a new `RuntimeControlPlanError::UnknownLayer { binding, layer }`
   rather than panicking — the plan is also built from daemon-supplied
   documents.

4. Do not touch `materialize_action` (`:103-198`). Layers do not change how an
   action materializes, only when it fires.

Tests in `profile_controls.rs` `mod tests` (`:224-380`): a profile with two
layers materializes distinct `LayerId`s; a base-layer binding gets
`LayerId::BASE`; an unknown layer returns `UnknownLayer`.

## 6. Runtime dispatch

`crates/wroid-inject/src/game_session.rs` — the substantive part.

### 6.1 New state on `UnifiedRuntime`

Extend the struct at `:886-898`:

```rust
active_layers: LayerMask,        // bitmask over LayerId
held_modifiers: ModifierMask,    // bitmask over the small modifier key set
held_bindings: BTreeSet<usize>,  // indices into plan.controls, currently held
```

`LayerMask` and `ModifierMask` are `u64` newtypes. This caps layers at 64,
which is far beyond any sane control map, and keeps the hot path allocation-free
per §2.8. Reject profiles with more than 64 layers in validation (rule 1's
neighbourhood) so the cap is a documented limit rather than a silent truncation.

Initialize in `new()` (`:901-949`): base layer always active, no modifiers, no
held bindings.

### 6.2 Condition check

One private helper, used by both press and release paths:

```rust
fn binding_is_available(&self, control: &RuntimeControlBinding) -> bool {
    self.active_layers.contains(control.layer)
        && match &control.modifier {
            None => true,
            Some(modifier) => self.held_modifiers.contains(modifier),
        }
}
```

A binding with no modifier must remain available when a modifier *is* held,
unless a more specific binding for the same key exists in the same layer. Decide
this explicitly: **the modifier binding wins, and the unmodified binding is
suppressed while its modifier-carrying sibling is available.** Otherwise
`Shift+R` would fire both "reload" and "fire mode". Precompute the sibling
relationship in `RuntimeControlPlan` — do not scan for siblings per event.

### 6.3 `handle_keyboard` changes

Current flow at `:961-1072`. Insert, in this order:

1. **Layer activation, before binding dispatch.** If the key matches a layer's
   `activation_key`:
   - `Hold`: set the bit on press, clear on release.
   - `Toggle`: flip on press only.
   When a layer turns off, call the new `release_layer(layer_id)` (§6.5).
   Then `continue` — an activation key is not also a binding key (validation
   rules 5 and 12 guarantee it).

2. **Modifier tracking, before binding dispatch.** If the key is used as a
   modifier by any binding, update `held_modifiers`. Unlike layer keys, a
   modifier key must **not** `continue`: `shift` can legitimately also be a
   plain binding elsewhere. When a modifier goes up, call
   `release_modifier(modifier)` (§6.5).

3. **Use action-specific press semantics; never gate releases.** A `Tap` checks
   availability only on the physical action-key press edge. Do not delay the
   Base tap and do not replay/switch it when only a modifier or layer changes.
   Continuous `Hold` and hold-mode `VirtualJoystick` actions recompute desired
   ownership on action-key, modifier, and layer changes, including when the
   modifier arrives after the action key. Release paths never consult current
   availability before cleaning up an active owner.

4. **Track held bindings.** On a successful hold press, insert the control
   index into `held_bindings`; on release, remove it. `point_contacts`
   (`:890`) already maps names to contacts; `held_bindings` records *which* are
   currently down, which `point_contacts` alone does not tell you.

### 6.4 `handle_mouse` changes

At `:1074-1144`. Mouse buttons participate in layers and modifiers exactly like
keys: add `binding_is_available` to the press arms at `:1116` and `:1126`, and
keep the release path ungated. Mouse *motion* (`:1078`) and `mouse_aim` are
unaffected — validation rule 9 forbids modifiers there, and aim controllers are
not layer-scoped in this iteration. State that explicitly in the docs: mouse aim
is always live regardless of layer.

### 6.5 New release paths

Two methods, both modelled on `suspend()` (`:1178-1184`):

```rust
fn release_layer(&mut self, layer: LayerId) -> GameSessionResult<bool>;
fn release_modifier(&mut self, modifier: &HostKeyName) -> GameSessionResult<bool>;
```

Each walks `held_bindings`, and for every binding that is now unavailable:

- `Hold` action: end the contact via the same path `set_hold_binding` uses.
- `VirtualJoystick`: clear the entry in `directions` (`:889`) and apply the
  neutral `DirectionalInput`, matching what `suspend()` does at `:1180`.
- `Tap`: nothing to release, taps are instantaneous.

Both return whether a frame was submitted, so the caller can feed
`record_pipeline_latency` the way `:859-861` does.

### 6.6 Suspend and focus loss

`suspend()` (`:1178`) must also clear `active_layers` to base-only,
`held_modifiers`, and `held_bindings`. Otherwise F12 release followed by
recapture resumes with a stale layer active and a phantom modifier held. Verify
against the F12 path at `:800-822`, which calls `suspend()` then `start()`.

## 7. Tests

Rust unit tests, alongside the existing ones in each file:

- `profile_v2.rs` (`mod tests`, `:547`): one test per validation rule in §4.
- `profile_controls.rs` (`:224`): layer interning, base default, `UnknownLayer`.
- `game_session.rs` (`:1600+`, existing test profiles at `:1743`, `:2005`,
  `:2077` are the templates):
  1. base binding fires without any layer;
  2. layer binding does not fire while its layer is inactive;
  3. layer binding fires while the hold key is down;
  4. toggle layer stays active after release and flips off on second press;
  5. same key drives different actions in base vs layer;
  6. modifier binding fires only with the modifier held, with separate edge
     coverage proving that Tap requires modifier-before-action while continuous
     actions reconcile if the modifier arrives after the action is already held;
  7. unmodified sibling is suppressed while the modifier is held, and Tap does
     not delay Base or refire on modifier/layer-only changes;
  8. **releasing the modifier while the action key is held releases the
     contact** (§2.7 case 1);
  9. **deactivating a layer releases contacts held by that layer** (case 2);
  10. **releasing the action key after the modifier already went up does not
      leak a contact** (case 3);
  11. `suspend()` clears layers, modifiers, and held bindings;
  12. joystick held in a layer goes neutral when the layer deactivates.

Assert on `engine.state().active_contact_count()` reaching 0, the way
`crates/wroid-runtime/src/mouse_aim.rs` tests do. A leaked contact is the
failure mode that matters, and it is invisible unless asserted.

Acceptance gates from `SPEC.md:113-119` all apply: `cargo fmt`,
`cargo clippy --workspace --all-targets` clean, `cargo test --workspace` green,
example profiles validate.

Plus the performance gate from §2.8: re-run
`target/release/wroid-inject-latency --samples 20000`, confirm p99 has not
regressed, and record the number in `docs/runtime-benchmarks.md` if it moved.
**Completed implementation evidence:** mean 0.8 us, p50 0.7 us, p95 1.0 us,
p99 1.3 us, max 22.5 us; 20,000/20,000 frames stayed below 5 ms, 10/10
simultaneous contacts were accepted, and clean release was verified.

## 8. Editor (Controls Studio)

`editor.rs` needs almost nothing: `GET /api/profile` returns the raw file text
(`editor.rs:202-205`) and `PUT /api/profile` deserializes into `ProfileV2` and
validates (`:291-331`), so new fields flow through once the schema knows them.
Only the hardcoded JSON in the test at `editor.rs:651-694` needs updating.

The work is in `crates/wroid-cli/assets/editor/`:

`app.js`:
1. `validateProfile` (`:883-922`) — mirror all rules from §4.
2. `renderInspector` (`:546-569`) — add a "Layer" section: a select listing
   declared layers plus "Base", and a modifier key-capture field.
3. `renderInputEditor` (`:588-622`) — place the modifier capture next to the
   key/cluster fields. `wireKeyCapture` (`:692-723`) is reusable as-is with
   `allowEmpty: true` for "no modifier"; `Backspace`/`Delete` already clears.
4. `wireInspector` (`:725-786`) — wire the two new setters.
5. `defaultInput` / `addControl` (`:788-852`) — new bindings default to the
   currently selected layer, not always base. This is the difference between
   "layers are usable" and "layers are technically supported".
6. Layer management UI — add/rename/delete a layer, and pick its activation key
   and hold/toggle mode. Panel "02 / LAYERS" in `index.html:69-82` is currently
   just the binding-list heading; it is the natural home.
7. `renderBindingList` (`:338-366`) — filter by active layer, and show a layer
   and modifier chip per row. Without a filter, a layered map becomes an
   unreadable flat list.
8. `renderOverlay` (`:368-375`) — show only the selected layer's controls over
   the calibration background, otherwise overlapping layers look like a bug.
9. `inputSummary` (`:122-131`), `controlKey` (`:133-151`) — include modifier in
   the label, e.g. `Shift+R`.
10. `bindingIsTestActive` (`:186-199`), `handleTestKey` (`:261-270`),
    `updateTestPreview` (`:208-237`) — the in-browser preview must respect
    layers and modifiers, or it will contradict the real session.
11. `supportedKeys` (`:6-11`) — unchanged; `f12` stays absent.

`index.html`: layer switcher in panel 02, layer/modifier fields in the
inspector (`:200-233`).

`styles.css`: chip styles for layer and modifier badges.

## 9. Hub

`crates/wroid-cli/src/commands/hub.rs`:

1. `control_counts` (`:643-657`) — add a layer count. The `ActionV2::Macro`
   arm at `:653` stays as-is.
2. `build_state` (`:468-493`) — expose `layers` next to `bindings` (`:480`) and
   the `controls` object (`:481-486`).
3. `assets/hub/app.js:524-531` — render a layer chip beside taps/holds/sticks.
4. `profile_needs_mouse` (`:1733-1741`) — unchanged; a modifier never makes a
   keyboard binding need a mouse.
5. `starter_predecessors` (`:277-300`) — verify the `PartialEq` comparison still
   behaves once `layers` exists. This is why §3 mandates
   `skip_serializing_if`; add a test that an untouched starter profile still
   compares equal after a load/save round trip.

## 10. Other call sites

- `crates/wroid-core/src/bin/wroid-profile-v2-validate.rs`:
  `print_materialized_bindings` (`:101-107`) should print layer and modifier.
  No exhaustive match breaks, since no enum gained a variant.
- `crates/wroid-cli/src/commands/session.rs`: `describe_input` (`:128-140`)
  should include the modifier.
- `crates/wroid-inject/src/bin/wroid-waydroid-profile-smoke.rs`:
  `find_wasd_joystick` (`:112-159`) and `find_key_taps` (`:162-176`) should
  restrict themselves to base-layer bindings, or the smoke tool will pick a
  layered binding it cannot activate.
- `crates/wroid-cli/src/commands/profile.rs` and `commands/input.rs` operate on
  the **legacy** schema (`crate::Binding`) and need no changes.

## 11. Starter profiles

Do not add layers to the four shipped starters in the same change. Ship the
mechanism first, verified by tests, then add layered starters in a follow-up so
a regression in layer handling cannot break the default experience for all four
games at once.

When that follow-up happens, `profiles/examples/standoff2-v2.json` is the best
first candidate: a `grenades` hold-layer over `1..4` is the clearest win, and
the file already has a full base map to build on.

## 12. Documentation

- `docs/profile-v2.md:27-49` — document `layers`, `layer`, `modifier`, the
  `LayerActivation` kinds, and the 64-layer cap.
- `docs/profile-v2.md:78-95` — the validation rules from §4.
- `docs/profile-v2.md` — a short section stating that `schema_version` stays 2
  and why (§2.4).
- `docs/input-model.md` — the dispatch rule from §2.7: presses are gated,
  releases are not; modifier siblings suppress the unmodified binding.
- `docs/roadmap.md:91` — mark schema layers/modifiers done, leaving migrations
  and daemon wiring open.
- `SPEC.md` — add layers/modifiers to the capability list.
- `README.md:519+` (Profile Format) — the JSON example from §3.

## 13. Suggested commit sequence

Each step must leave the workspace green (`fmt`, `clippy`, `test`).

1. Schema: `LayerV2`, `LayerActivation`, `BindingV2.layer`, `BindingV2.modifier`,
   with `serde` defaults and `skip_serializing_if`. No validation yet, no
   behaviour change. Round-trip test proving existing profiles serialize
   byte-identically.
2. Validation: all rules from §4 plus one test each.
3. Runtime plan: `LayerId`, `RuntimeLayer`, interning, `UnknownLayer`.
4. Dispatch — press/reconciliation semantics: layer activation, modifier
   tracking, Tap press-edge scope, and continuous modifier-order reconciliation.
   Tests 1-7 from §7.
5. Dispatch — releases: `release_layer`, `release_modifier`, `suspend()`
   clearing, sibling suppression. Tests 8-12 from §7. **Do not merge step 4
   without step 5**; between them the runtime leaks contacts.
6. Editor: validation mirror, inspector fields, layer management, list filter,
   overlay filter, preview.
7. Hub: counts and chips.
8. Peripheral call sites (§10) and docs (§12).
9. Re-run the latency benchmark and record the result. **Complete:** mean
   0.8 us, p99 1.3 us over 20,000 frames; all frames met 5 ms, 10/10 contacts
   were simultaneous, and release was clean.

Steps 1-5 are the feature. Steps 6-7 are what makes it usable by someone who
does not edit JSON by hand.
