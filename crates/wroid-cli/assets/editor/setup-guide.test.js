"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { steps } = require("./setup-guide.js");

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
