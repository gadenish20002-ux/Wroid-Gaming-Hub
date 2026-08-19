"use strict";

const assert = require("node:assert/strict");
const { activeRootFinding } = require("./compatibility-state.js");

const finding = activeRootFinding({
  rootAccess: { state: "detected" },
  findings: [
    {
      code: "android-root-detected",
      severity: "action",
      message: "Remove Magisk",
    },
  ],
});

assert.equal(finding.message, "Remove Magisk");
assert.equal(
  activeRootFinding({ rootAccess: { state: "not_detected" }, findings: [] }),
  null,
);
assert.equal(
  activeRootFinding({ rootAccess: { state: "unknown" }, findings: [] }),
  null,
);
assert.equal(
  activeRootFinding({
    rootAccess: { state: "detected" },
    findings: [{ code: "different-action", severity: "action", message: "Other" }],
  }),
  null,
);
