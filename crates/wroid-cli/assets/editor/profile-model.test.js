"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const Model = require("./profile-model.js");

function point(x = 0.5, y = 0.5) {
  return { x, y };
}

function keyBinding(name, key, options = {}) {
  return {
    name,
    ...(options.layer ? { layer: options.layer } : {}),
    ...(options.modifier ? { modifier: options.modifier } : {}),
    input: { kind: "key", key },
    action: { kind: options.action || "tap", point: point() },
  };
}

function mouseBinding(name, button, options = {}) {
  return {
    name,
    ...(options.layer ? { layer: options.layer } : {}),
    ...(options.modifier ? { modifier: options.modifier } : {}),
    input: { kind: "mouse_button", button },
    action: { kind: options.action || "tap", point: point() },
  };
}

function clusterBinding(name, options = {}) {
  return {
    name,
    ...(options.layer ? { layer: options.layer } : {}),
    ...(options.modifier ? { modifier: options.modifier } : {}),
    input: { kind: "key_cluster", up: "w", left: "a", down: "s", right: "d" },
    action: {
      kind: "virtual_joystick",
      center: point(0.2, 0.75),
      radius: 0.1,
      dead_zone: 0.02,
      mode: "hold",
      reaffirm_ms: 50,
    },
  };
}

function layer(name, kind, key) {
  return { name, activation: { kind, key } };
}

function profile({ layers = [], bindings = [] } = {}) {
  return {
    schema_version: 2,
    name: "Test map",
    package_name: "com.example.game",
    orientation: "landscape",
    layers,
    bindings,
  };
}

function hasError(document, fragment) {
  assert.ok(
    Model.validateProfile(document).some((error) => error.includes(fragment)),
    `expected validation error containing ${JSON.stringify(fragment)}`,
  );
}

test("legacy profiles normalize to an implicit Base layer", () => {
  const legacy = { schema_version: 2, name: "Legacy", package_name: "game", bindings: [] };

  const normalized = Model.normalizeProfile(legacy);

  assert.deepEqual(normalized.layers, []);
  assert.equal(normalized.orientation, "landscape");
  assert.equal(Model.layerName(null), "base");
  assert.equal(normalized.layers.some((entry) => entry.name.toLowerCase() === "base"), false);
});

test("normalization preserves authored mouse aim routing while runtime semantics stay Base", () => {
  const document = profile({ bindings: [{
    name: "aim",
    layer: "legacy-layer",
    modifier: "shift",
    input: { kind: "mouse_move" },
    action: {
      kind: "mouse_aim",
      region: { x: 0.1, y: 0.1, w: 0.8, h: 0.8 },
      sensitivity: 1,
      recenter_threshold: 0.7,
    },
  }] });

  const normalized = Model.normalizeProfile(document);

  assert.equal(normalized.bindings[0].layer, "legacy-layer");
  assert.equal(normalized.bindings[0].modifier, "shift");
  assert.equal(Model.layerName(normalized.bindings[0]), "base");
  assert.equal(Model.clearMouseMoveRouting(normalized.bindings[0]), true);
  assert.equal(normalized.bindings[0].layer, undefined);
  assert.equal(normalized.bindings[0].modifier, undefined);
});

test("renaming a layer updates all binding references atomically", () => {
  const document = profile({
    layers: [layer("combat", "hold", "g")],
    bindings: [keyBinding("base", "r"), keyBinding("layered", "r", { layer: "combat" })],
  });

  assert.equal(Model.renameLayer(document, "combat", "grenades"), 1);
  assert.equal(document.layers[0].name, "grenades");
  assert.equal(document.bindings[1].layer, "grenades");
  assert.equal(document.bindings[0].layer, undefined);
});

test("layer rename policy rejects empty, Base, and duplicate names without mutation", () => {
  const document = profile({
    layers: [layer("combat", "hold", "g"), layer("vehicle", "toggle", "v")],
    bindings: [keyBinding("layered", "r", { layer: "combat" })],
  });

  assert.match(Model.layerRenameError(document, "combat", " "), /must not be empty/);
  assert.match(Model.layerRenameError(document, "combat", "BASE"), /reserved/);
  assert.match(Model.layerRenameError(document, "combat", "vehicle"), /already exists/);
  assert.equal(document.layers[0].name, "combat");
  assert.equal(document.bindings[0].layer, "combat");
});

test("deleting a layer safely moves its bindings to Base", () => {
  const document = profile({
    layers: [layer("combat", "hold", "g"), layer("vehicle", "toggle", "v")],
    bindings: [keyBinding("layered", "r", { layer: "combat" })],
  });

  assert.equal(Model.deleteLayer(document, "combat"), 1);
  assert.deepEqual(document.layers.map((entry) => entry.name), ["vehicle"]);
  assert.equal(document.bindings[0].layer, undefined);
});

test("chord labels cover keyboard, mouse, and clusters", () => {
  assert.equal(Model.controlKey(keyBinding("reload", "r", { modifier: "shift" })), "SHIFT+R");
  assert.equal(Model.controlKey(mouseBinding("ads", "right", { modifier: "ctrl" })), "CTRL+M2");
  assert.equal(Model.controlKey({ ...clusterBinding("move"), modifier: "alt" }), "ALT+WASD");
});

test("validation enforces layer count, names, and activation keys", () => {
  hasError(profile({ layers: Array.from({ length: 65 }, (_, index) => layer(`l${index}`, "hold", "g")) }), "at most 64 layers");
  hasError(profile({ layers: [layer(" ", "hold", "g")] }), "layer name must not be empty");
  hasError(profile({ layers: [layer("combat", "hold", "g"), layer("combat", "toggle", "v")] }), "duplicate layer name: combat");
  hasError(profile({ layers: [layer("BASE", "hold", "g")] }), "layer name base is reserved");
  hasError(profile({ layers: [layer("combat", "hold", "f12")] }), "unsupported activation key: f12");
  hasError(profile({ layers: [layer("combat", "hold", "g"), layer("vehicle", "toggle", "G")] }), "duplicate layer activation key: G");
});

test("validation rejects activation collisions with plain Base key and clusters", () => {
  hasError(
    profile({ layers: [layer("combat", "hold", "g")], bindings: [keyBinding("base", "g")] }),
    "layer activation key g cannot be used by a base-layer binding",
  );
  const cluster = clusterBinding("base_move");
  cluster.input.up = "g";
  hasError(
    profile({ layers: [layer("combat", "hold", "g")], bindings: [cluster] }),
    "layer activation key g cannot be used by a base-layer binding",
  );
  assert.deepEqual(
    Model.validateProfile(profile({
      layers: [layer("combat", "hold", "g")],
      bindings: [keyBinding("chord", "g", { modifier: "shift" })],
    })),
    [],
  );
});

test("validation enforces known layers and modifiers", () => {
  hasError(profile({ bindings: [keyBinding("unknown", "r", { layer: "combat" })] }), "references unknown layer: combat");
  hasError(profile({ bindings: [keyBinding("unknown", "r", { modifier: "f12" })] }), "uses unsupported modifier: f12");
  hasError(profile({ bindings: [keyBinding("same", "r", { modifier: "r" })] }), "modifier must differ from input key: r");
  hasError(profile({ bindings: [{ ...clusterBinding("move"), modifier: "w" }] }), "modifier must differ from key_cluster key: w");
  hasError(profile({ bindings: [{
    name: "aim",
    modifier: "shift",
    input: { kind: "mouse_move" },
    action: {
      kind: "mouse_aim",
      region: { x: 0.1, y: 0.1, w: 0.8, h: 0.8 },
      sensitivity: 1,
      recenter_threshold: 0.7,
      reaffirm_ms: 50,
    },
  }] }), "cannot use a modifier with mouse_move input");
});

test("validation rejects reserved ctrl session chords including cluster constituents", () => {
  hasError(profile({ bindings: [keyBinding("exit", "esc", { modifier: "ctrl" })] }), "uses ctrl+esc, which is reserved");
  const cluster = clusterBinding("reserved");
  cluster.modifier = "ctrl";
  cluster.input.right = "c";
  hasError(profile({ bindings: [cluster] }), "uses ctrl+c, which is reserved");
});

test("duplicate inputs are scoped by layer, modifier, and physical namespace", () => {
  hasError(profile({ bindings: [keyBinding("one", "r"), keyBinding("two", "R")] }), "key R drives multiple bindings in base layer without a modifier");
  hasError(profile({ bindings: [mouseBinding("one", "left"), mouseBinding("two", "LEFT")] }), "mouse_button LEFT drives multiple bindings in base layer without a modifier");
  assert.deepEqual(Model.validateProfile(profile({
    layers: [layer("combat", "hold", "g")],
    bindings: [
      keyBinding("base", "r"),
      keyBinding("layered", "r", { layer: "combat" }),
      keyBinding("modified", "r", { modifier: "shift" }),
      mouseBinding("mouse", "right"),
      keyBinding("keyboard_namespace", "right"),
    ],
  })), []);
});

test("activation key cannot be bound inside its own layer", () => {
  hasError(
    profile({
      layers: [layer("combat", "hold", "g")],
      bindings: [keyBinding("bad", "g", { layer: "combat", modifier: "shift" })],
    }),
    "layer activation key g cannot be used by binding bad inside layer combat",
  );
});

test("preview tracks hold and toggle layers on press edges", () => {
  const document = profile({
    layers: [layer("combat", "hold", "g"), layer("vehicle", "toggle", "v")],
    bindings: [keyBinding("base", "r"), keyBinding("combat", "r", { layer: "combat" }), keyBinding("vehicle", "f", { layer: "vehicle" })],
  });
  const preview = Model.createPreviewState();

  Model.setPreviewKey(preview, document, "g", true);
  assert.deepEqual([...preview.activeLayers], ["combat"]);
  Model.setPreviewKey(preview, document, "g", false);
  assert.deepEqual([...preview.activeLayers], []);

  Model.setPreviewKey(preview, document, "v", true);
  Model.setPreviewKey(preview, document, "v", true);
  Model.setPreviewKey(preview, document, "v", false);
  assert.deepEqual([...preview.activeLayers], ["vehicle"]);
  Model.setPreviewKey(preview, document, "v", true);
  assert.deepEqual([...preview.activeLayers], []);
});

test("preview applies layer precedence, modifier siblings, and release state", () => {
  const document = profile({
    layers: [layer("combat", "hold", "g")],
    bindings: [
      keyBinding("base", "r", { action: "hold" }),
      keyBinding("layer_plain", "r", { layer: "combat", action: "hold" }),
      keyBinding("layer_chord", "r", { layer: "combat", modifier: "shift", action: "hold" }),
    ],
  });
  const preview = Model.createPreviewState();

  Model.setPreviewKey(preview, document, "r", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [0]);
  Model.setPreviewKey(preview, document, "g", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [1]);
  Model.setPreviewKey(preview, document, "shift", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [2]);
  Model.setPreviewKey(preview, document, "shift", false);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [1]);
  Model.setPreviewKey(preview, document, "g", false);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [0]);
  Model.setPreviewKey(preview, document, "r", false);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), []);
});

test("preview applies the same layer and modifier precedence to mouse buttons", () => {
  const document = profile({
    layers: [layer("combat", "hold", "g")],
    bindings: [
      mouseBinding("base", "left", { action: "hold" }),
      mouseBinding("layer_plain", "left", { layer: "combat", action: "hold" }),
      mouseBinding("layer_chord", "left", { layer: "combat", modifier: "shift", action: "hold" }),
    ],
  });
  const preview = Model.createPreviewState();

  Model.setPreviewButton(preview, document, "left", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [0]);
  Model.setPreviewKey(preview, document, "g", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [1]);
  Model.setPreviewKey(preview, document, "shift", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [2]);
  Model.setPreviewButton(preview, document, "left", false);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), []);
});

test("Tap preview latches only on the action key press edge", () => {
  const document = profile({
    layers: [layer("combat", "hold", "g")],
    bindings: [
      keyBinding("base", "r"),
      keyBinding("base_modified", "r", { modifier: "shift" }),
      keyBinding("layered", "r", { layer: "combat" }),
      keyBinding("layered_modified", "r", { layer: "combat", modifier: "shift" }),
    ],
  });
  const preview = Model.createPreviewState();

  Model.setPreviewKey(preview, document, "r", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [0]);
  Model.setPreviewKey(preview, document, "shift", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), []);
  Model.setPreviewKey(preview, document, "shift", false);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), []);
  Model.setPreviewKey(preview, document, "g", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), []);
  Model.setPreviewKey(preview, document, "g", false);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), []);
  Model.setPreviewKey(preview, document, "r", false);

  Model.setPreviewKey(preview, document, "shift", true);
  Model.setPreviewKey(preview, document, "r", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [1]);
  Model.setPreviewKey(preview, document, "shift", false);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), []);
  Model.setPreviewKey(preview, document, "r", false);

  Model.setPreviewKey(preview, document, "g", true);
  Model.setPreviewKey(preview, document, "r", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [2]);
  Model.setPreviewKey(preview, document, "g", false);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), []);
  Model.setPreviewKey(preview, document, "r", false);
});

test("mouse-button Tap preview has the same press-edge latch semantics", () => {
  const document = profile({
    bindings: [
      mouseBinding("base", "left"),
      mouseBinding("modified", "left", { modifier: "shift" }),
    ],
  });
  const preview = Model.createPreviewState();

  Model.setPreviewButton(preview, document, "left", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [0]);
  Model.setPreviewKey(preview, document, "shift", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), []);
  Model.setPreviewButton(preview, document, "left", false);

  Model.setPreviewButton(preview, document, "left", true);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [1]);
  Model.setPreviewKey(preview, document, "shift", false);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), []);
  Model.setPreviewButton(preview, document, "left", false);
});

test("preview consumes declared activation keys before binding dispatch", () => {
  const document = profile({
    layers: [layer("combat", "hold", "g"), layer("vehicle", "toggle", "v")],
    bindings: [keyBinding("other_layer_g", "g", { layer: "vehicle" })],
  });
  const preview = Model.createPreviewState();
  Model.setPreviewKey(preview, document, "v", true);
  Model.setPreviewKey(preview, document, "v", false);
  Model.setPreviewKey(preview, document, "g", true);

  assert.deepEqual([...preview.activeLayers].sort(), ["combat", "vehicle"]);
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), []);
});

test("mouse aim preview remains always live outside layer selection", () => {
  const document = profile({ bindings: [{
    name: "aim",
    layer: "unknown-from-legacy-editor",
    input: { kind: "mouse_move" },
    action: {
      kind: "mouse_aim",
      region: { x: 0.1, y: 0.1, w: 0.8, h: 0.8 },
      sensitivity: 1,
      recenter_threshold: 0.7,
      reaffirm_ms: 50,
    },
  }] });
  const preview = Model.createPreviewState();
  preview.mouseMoving = true;
  assert.deepEqual(Model.activePreviewBindingIndexes(document, preview), [0]);
});

test("selection reconciliation keeps the inspector inside the editing layer", () => {
  const document = profile({
    layers: [layer("combat", "hold", "g")],
    bindings: [
      keyBinding("base", "r"),
      keyBinding("combat_one", "1", { layer: "combat" }),
      keyBinding("combat_two", "2", { layer: "combat" }),
      {
        name: "aim",
        input: { kind: "mouse_move" },
        action: {
          kind: "mouse_aim",
          region: { x: 0.1, y: 0.1, w: 0.8, h: 0.8 },
          sensitivity: 1,
          recenter_threshold: 0.7,
        },
      },
    ],
  });

  assert.equal(Model.reconcileSelectedBinding(document, "combat", 2), 2);
  assert.equal(Model.reconcileSelectedBinding(document, "combat", 0), 1);
  assert.equal(Model.reconcileSelectedBinding(document, null, 3), 3);
  document.bindings.splice(1, 2);
  assert.equal(Model.reconcileSelectedBinding(document, "combat", 1), -1);
});
