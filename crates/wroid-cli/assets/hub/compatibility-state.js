"use strict";

(function installCompatibilityState(root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) {
    module.exports = api;
  } else {
    root.WroidHubCompatibility = api;
  }
})(typeof globalThis === "object" ? globalThis : this, function createCompatibilityState() {
  function activeRootFinding(compatibility) {
    if (compatibility?.rootAccess?.state !== "detected") return null;
    return compatibility.findings?.find(
      (finding) => finding.code === "android-root-detected",
    ) || null;
  }

  return { activeRootFinding };
});
