"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const {
  captureErrorMessage,
  capturePrompt,
  selectionForPlace,
  steps,
} = require("./setup-guide.js");

test("setup starts by asking for the game window", () => {
  assert.deepEqual(
    steps({ backgroundSaved: false, selected: -1, testing: false, dirty: false })
      .map((step) => step.state),
    ["active", "pending", "pending", "pending"],
  );
});

test("setup advances through placement into local testing", () => {
  assert.deepEqual(
    steps({ backgroundSaved: true, selected: 2, testing: true, dirty: true })
      .map((step) => step.state),
    ["done", "done", "active", "pending"],
  );
});

test("changed tested map makes save and play the next action", () => {
  assert.equal(
    steps({ backgroundSaved: true, selected: 2, testing: false, dirty: true })[3].state,
    "active",
  );
});

test("guide exposes concise action labels", () => {
  assert.deepEqual(
    steps({ backgroundSaved: false, selected: -1, testing: false, dirty: false })
      .map((step) => step.action),
    ["capture", "place", "test", "save"],
  );
});

test("place keeps a visible selection and otherwise chooses the first visible binding", () => {
  const bindings = [{ layer: "base" }, { layer: "combat" }, { layer: "combat" }];
  const visible = (binding) => binding.layer === "combat";

  assert.equal(selectionForPlace(bindings, 2, visible), 2);
  assert.equal(selectionForPlace(bindings, 0, visible), 1);
  assert.equal(selectionForPlace([], -1, visible), -1);
});

test("window capture guidance names the required confirmation", () => {
  assert.match(capturePrompt(), /Standoff 2 \/ Waydroid/);
  assert.match(capturePrompt(), /Share/);
  assert.match(captureErrorMessage({ name: "NotAllowedError" }), /cancelled or denied/);
  assert.equal(captureErrorMessage({ message: "portal failed" }), "portal failed");
});
