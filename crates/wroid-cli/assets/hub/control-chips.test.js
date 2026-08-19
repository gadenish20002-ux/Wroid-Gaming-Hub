"use strict";

const assert = require("node:assert/strict");
const { controlChipLabels, controlQuickStart, editorActionFor } = require("./control-chips.js");

assert.deepEqual(
  controlChipLabels({
    controls: { layers: 2, taps: 7, holds: 1, joysticks: 1, mouseAim: 1 },
  }),
  ["2 layers", "7 taps", "1 holds", "1 sticks", "mouse aim"],
);

assert.deepEqual(
  controlQuickStart({ kind: "standoff" }),
  {
    primary: ["WASD move", "Mouse aim", "LMB fire", "RMB aim", "R reload", "Space jump", "C crouch", "1/2 weapons", "F action"],
    safety: ["Tab — toggle mouse aim", "F12 — release input", "Ctrl+Esc — stop game"],
  },
);

assert.deepEqual(
  controlChipLabels({
    controls: { layers: 0, taps: 2, holds: 0, joysticks: 2, mouseAim: 0 },
  }),
  ["0 layers", "2 taps", "0 holds", "2 sticks", "no mouse aim"],
);

assert.equal(
  editorActionFor({ installed: null, calibration: { ready: false } }),
  "calibrate",
  "an offline runtime must not hide calibration",
);
assert.equal(editorActionFor({ installed: true, calibration: { ready: false } }), "calibrate");
assert.equal(editorActionFor({ installed: false, calibration: { ready: false } }), "edit");
assert.equal(editorActionFor({ installed: true, calibration: { ready: true } }), "edit");

console.log("hub control chip tests passed");
