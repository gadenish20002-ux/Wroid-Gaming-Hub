"use strict";

(function installSetupGuide(root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) {
    module.exports = api;
  } else {
    root.WroidSetupGuide = api;
  }
})(typeof globalThis === "object" ? globalThis : this, function createSetupGuide() {
  const definitions = Object.freeze([
    Object.freeze({
      action: "capture",
      label: "Capture game",
      detail: "Select the running Waydroid game and align its HUD.",
    }),
    Object.freeze({
      action: "place",
      label: "Place & bind",
      detail: "Drag a marker onto the HUD, then press its keyboard or mouse input.",
    }),
    Object.freeze({
      action: "test",
      label: "Test bindings",
      detail: "Preview WASD, aim and buttons locally before playing.",
    }),
    Object.freeze({
      action: "save",
      label: "Save & play",
      detail: "Validate the map, save it and return to Wroid Hub.",
    }),
  ]);

  function steps({ backgroundSaved, selected, testing, dirty }) {
    const hasSurface = Boolean(backgroundSaved);
    const hasSelection = hasSurface && Number(selected) >= 0;
    const states = [
      hasSurface ? "done" : "active",
      !hasSurface ? "pending" : hasSelection ? "done" : "active",
      !hasSelection ? "pending" : testing ? "active" : dirty ? "done" : "active",
      !hasSurface || testing ? "pending" : dirty ? "active" : "done",
    ];
    return definitions.map((definition, index) => ({ ...definition, state: states[index] }));
  }

  function selectionForPlace(bindings, selected, isVisible) {
    if (Number.isInteger(selected) && selected >= 0 && bindings[selected] && isVisible(bindings[selected])) {
      return selected;
    }
    return bindings.findIndex(isVisible);
  }

  return { selectionForPlace, steps };
});
