"use strict";

const assert = require("node:assert/strict");
const { controlChipLabels } = require("./control-chips.js");

assert.deepEqual(
  controlChipLabels({
    controls: { layers: 2, taps: 7, holds: 1, joysticks: 1, mouseAim: 1 },
  }),
  ["2 layers", "7 taps", "1 holds", "1 sticks", "mouse aim"],
);

assert.deepEqual(
  controlChipLabels({
    controls: { layers: 0, taps: 2, holds: 0, joysticks: 2, mouseAim: 0 },
  }),
  ["0 layers", "2 taps", "0 holds", "2 sticks", "no mouse aim"],
);

console.log("hub control chip tests passed");
