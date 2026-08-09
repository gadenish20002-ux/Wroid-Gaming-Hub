"use strict";

(function installControlChips(root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) {
    module.exports = api;
  } else {
    root.WroidHubControlChips = api;
  }
})(typeof globalThis === "object" ? globalThis : this, function createControlChips() {
  function controlChipLabels(game) {
    return [
      `${game.controls.layers} layers`,
      `${game.controls.taps} taps`,
      `${game.controls.holds} holds`,
      `${game.controls.joysticks} sticks`,
      game.controls.mouseAim ? "mouse aim" : "no mouse aim",
    ];
  }

  return { controlChipLabels };
});
