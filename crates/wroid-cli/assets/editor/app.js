(() => {
  "use strict";

  const token = new URLSearchParams(window.location.search).get("token") || "";
  const api = (path) => `${path}?token=${encodeURIComponent(token)}`;
  const supportedKeys = new Set([
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m",
    "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z",
    "space", "tab", "shift", "ctrl", "alt", "up", "left", "down", "right", "esc",
  ]);
  const supportedButtons = new Set(["left", "right", "middle", "side", "extra"]);
  const resolutionPresets = Object.freeze({
    "1280x720": Object.freeze({ width: 1280, height: 720, label: "720" }),
    "1600x900": Object.freeze({ width: 1600, height: 900, label: "900" }),
    "1920x1080": Object.freeze({ width: 1920, height: 1080, label: "1080" }),
  });
  let preferenceWrite = Promise.resolve();

  const elements = {
    boot: document.querySelector("#bootScreen"),
    shell: document.querySelector("#appShell"),
    profileName: document.querySelector("#profileName"),
    packageName: document.querySelector("#packageName"),
    orientation: document.querySelector("#orientation"),
    bindingCount: document.querySelector("#bindingCount"),
    bindingSearch: document.querySelector("#bindingSearch"),
    bindingList: document.querySelector("#bindingList"),
    inspectorBody: document.querySelector("#inspectorBody"),
    inspectorEmpty: document.querySelector("#inspectorEmpty"),
    deleteButton: document.querySelector("#deleteButton"),
    duplicateButton: document.querySelector("#duplicateButton"),
    liveTestButton: document.querySelector("#liveTestButton"),
    resolutionSwitch: document.querySelector("#resolutionSwitch"),
    restoreButton: document.querySelector("#restoreButton"),
    saveButton: document.querySelector("#saveButton"),
    closeButton: document.querySelector("#closeButton"),
    undoButton: document.querySelector("#undoButton"),
    redoButton: document.querySelector("#redoButton"),
    saveState: document.querySelector("#saveState"),
    validationState: document.querySelector("#validationState"),
    selectedStatus: document.querySelector("#selectedStatus"),
    viewport: document.querySelector("#viewport"),
    viewportReadout: document.querySelector("#viewportReadout"),
    controlLayer: document.querySelector("#controlLayer"),
    screenshot: document.querySelector("#screenshot"),
    screenshotInput: document.querySelector("#screenshotInput"),
    screenshotButton: document.querySelector("#screenshotButton"),
    captureButton: document.querySelector("#captureButton"),
    clearBackgroundButton: document.querySelector("#clearBackgroundButton"),
    backgroundState: document.querySelector("#backgroundState"),
    calibrationDock: document.querySelector("#calibrationDock"),
    calibrationSource: document.querySelector("#calibrationSource"),
    calibrationZoom: document.querySelector("#calibrationZoom"),
    calibrationZoomValue: document.querySelector("#calibrationZoomValue"),
    calibrationX: document.querySelector("#calibrationX"),
    calibrationXValue: document.querySelector("#calibrationXValue"),
    calibrationY: document.querySelector("#calibrationY"),
    calibrationYValue: document.querySelector("#calibrationYValue"),
    calibrationResetButton: document.querySelector("#calibrationResetButton"),
    calibrationSaveButton: document.querySelector("#calibrationSaveButton"),
    calibrationStopButton: document.querySelector("#calibrationStopButton"),
    liveCalibrationCanvas: document.querySelector("#liveCalibrationCanvas"),
    emptySurface: document.querySelector("#emptySurface"),
    testButton: document.querySelector("#testButton"),
    inputTestHud: document.querySelector("#inputTestHud"),
    inputTestReadout: document.querySelector("#inputTestReadout"),
    inputTestMatches: document.querySelector("#inputTestMatches"),
    gridButton: document.querySelector("#gridButton"),
    labelsButton: document.querySelector("#labelsButton"),
    cursorReadout: document.querySelector("#cursorReadout"),
    toastStack: document.querySelector("#toastStack"),
  };

  const state = {
    profile: null,
    selected: -1,
    history: [],
    future: [],
    dirty: false,
    saving: false,
    snap: true,
    labels: true,
    screenshotUrl: null,
    backgroundSaved: false,
    backgroundSaving: false,
    drag: null,
    testing: false,
    launchingLiveTest: false,
    backupAvailable: false,
    resolutionKey: "1600x900",
    pressedKeys: new Set(),
    pressedButtons: new Set(),
    mouseMoving: false,
    mouseMoveTimer: null,
    keyCapture: null,
    calibration: null,
  };

  const clone = (value) => JSON.parse(JSON.stringify(value));
  const clamp = (value, min, max) => Math.min(max, Math.max(min, value));
  const rounded = (value, precision = 4) => Number(value.toFixed(precision));
  const normalized = (value) => rounded(clamp(value, 0, 1));
  const snapped = (value) => {
    const next = state.snap ? Math.round(value / 0.005) * 0.005 : value;
    return normalized(next);
  };

  function escapeHtml(value) {
    return String(value)
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#039;");
  }

  function actionKind(binding) {
    return binding?.action?.kind || "unknown";
  }

  function inputSummary(input) {
    if (!input) return "NO INPUT";
    if (input.kind === "key") return `KEY / ${input.key || "—"}`;
    if (input.kind === "key_cluster") {
      return `CLUSTER / ${[input.up, input.left, input.down, input.right].join(" ")}`;
    }
    if (input.kind === "mouse_button") return `MOUSE / ${input.button || "—"}`;
    if (input.kind === "mouse_move") return "MOUSE / RELATIVE";
    return input.kind;
  }

  function controlKey(binding) {
    const input = binding.input;
    if (input.kind === "key") return input.key;
    if (input.kind === "key_cluster") {
      if ([input.up, input.left, input.down, input.right].join("") === "wasd") return "WASD";
      if ([input.up, input.left, input.down, input.right].join(",") === "up,left,down,right") return "ARROWS";
      return "4 KEY";
    }
    if (input.kind === "mouse_button") {
      return {
        left: "M1",
        right: "M2",
        middle: "M3",
        side: "M4",
        extra: "M5",
      }[input.button] || "M?";
    }
    return "MOUSE";
  }

  function actionLabel(kind) {
    return {
      tap: "Tap",
      hold: "Hold",
      virtual_joystick: "Virtual joystick",
      mouse_aim: "Mouse aim",
    }[kind] || kind;
  }

  function browserKeyName(key) {
    const value = key.toLowerCase();
    return {
      " ": "space",
      arrowup: "up",
      arrowleft: "left",
      arrowdown: "down",
      arrowright: "right",
      escape: "esc",
      control: "ctrl",
    }[value] || value;
  }

  function browserButtonName(button) {
    return ["left", "middle", "right", "side", "extra"][button] || null;
  }

  function isTextEditor(target) {
    return target instanceof HTMLInputElement
      || target instanceof HTMLSelectElement
      || target instanceof HTMLTextAreaElement
      || target?.isContentEditable;
  }

  function bindingIsTestActive(binding) {
    const input = binding.input;
    if (input.kind === "key") return state.pressedKeys.has(input.key);
    if (input.kind === "key_cluster") {
      return [input.up, input.left, input.down, input.right]
        .some((key) => state.pressedKeys.has(key));
    }
    if (input.kind === "mouse_button") return state.pressedButtons.has(input.button);
    if (input.kind === "mouse_move") {
      return state.mouseMoving
        || Boolean(binding.action.toggle_key && state.pressedKeys.has(binding.action.toggle_key));
    }
    return false;
  }

  function activeTestBindings() {
    if (!state.testing) return [];
    return state.profile.bindings
      .map((binding, index) => ({ binding, index }))
      .filter(({ binding }) => bindingIsTestActive(binding));
  }

  function updateTestPreview() {
    elements.viewport.classList.toggle("is-testing", state.testing);
    elements.testButton.classList.toggle("is-active", state.testing);
    elements.testButton.setAttribute("aria-pressed", String(state.testing));
    elements.testButton.innerHTML = state.testing
      ? "<span>●</span> Testing live"
      : "<span>⌁</span> Test inputs";
    elements.inputTestHud.hidden = !state.testing;

    const active = activeTestBindings();
    const activeIndexes = new Set(active.map(({ index }) => String(index)));
    document.querySelectorAll("[data-binding-index]").forEach((node) => {
      node.classList.toggle("is-test-active", activeIndexes.has(node.dataset.bindingIndex));
    });

    if (!state.testing) return;
    const sources = [
      ...state.pressedKeys,
      ...state.pressedButtons,
      ...(state.mouseMoving ? ["mouse move"] : []),
    ].map((value) => value.toUpperCase());
    elements.inputTestReadout.textContent = sources.length
      ? sources.join(" + ")
      : "Press a mapped key or mouse button";
    elements.inputTestMatches.textContent = active.length
      ? `Matched: ${active.map(({ binding }) => binding.name).join(" · ")}`
      : sources.length
        ? "No binding matches this input."
        : "Browser preview only · no Android events are sent.";
  }

  function setTesting(enabled) {
    state.testing = enabled;
    state.pressedKeys.clear();
    state.pressedButtons.clear();
    state.mouseMoving = false;
    window.clearTimeout(state.mouseMoveTimer);
    state.mouseMoveTimer = null;
    updateTestPreview();
    if (enabled) toast("Input preview armed. Press keys and mouse buttons over the game surface.");
  }

  function pulseMouseMovement() {
    if (!state.testing) return;
    state.mouseMoving = true;
    window.clearTimeout(state.mouseMoveTimer);
    state.mouseMoveTimer = window.setTimeout(() => {
      state.mouseMoving = false;
      updateTestPreview();
    }, 180);
    updateTestPreview();
  }

  function handleTestKey(event, pressed) {
    if (!state.testing || isTextEditor(event.target)) return false;
    const key = browserKeyName(event.key);
    if (!supportedKeys.has(key)) return false;
    if (pressed) state.pressedKeys.add(key);
    else state.pressedKeys.delete(key);
    updateTestPreview();
    event.preventDefault();
    return true;
  }

  function handleTestPointer(event, pressed) {
    if (!state.testing) return;
    const button = browserButtonName(event.button);
    if (!button) return;
    if (!pressed && !state.pressedButtons.has(button)) return;
    if (pressed) state.pressedButtons.add(button);
    else state.pressedButtons.delete(button);
    updateTestPreview();
    event.preventDefault();
    event.stopImmediatePropagation();
  }

  function mutate(operation, render = true) {
    state.history.push(clone(state.profile));
    if (state.history.length > 80) state.history.shift();
    state.future.length = 0;
    operation();
    state.dirty = true;
    if (render) renderAll();
    updateStatus();
  }

  function undo() {
    const previous = state.history.pop();
    if (!previous) return;
    state.future.push(clone(state.profile));
    state.profile = previous;
    state.selected = Math.min(state.selected, state.profile.bindings.length - 1);
    state.dirty = true;
    renderAll();
  }

  function redo() {
    const next = state.future.pop();
    if (!next) return;
    state.history.push(clone(state.profile));
    state.profile = next;
    state.selected = Math.min(state.selected, state.profile.bindings.length - 1);
    state.dirty = true;
    renderAll();
  }

  function selectBinding(index) {
    state.selected = index;
    renderBindingList();
    renderOverlay();
    renderInspector();
    updateStatus();
  }

  function renderAll() {
    renderMeta();
    renderBindingList();
    renderOverlay();
    renderInspector();
    renderViewportMode();
    updateStatus();
  }

  function renderMeta() {
    elements.profileName.value = state.profile.name || "";
    elements.packageName.value = state.profile.package_name || "";
    elements.orientation.value = state.profile.orientation || "landscape";
    elements.bindingCount.textContent = String(state.profile.bindings.length).padStart(2, "0");
  }

  function renderBindingList() {
    const query = elements.bindingSearch.value.trim().toLowerCase();
    elements.bindingList.replaceChildren();
    state.profile.bindings.forEach((binding, index) => {
      if (
        query &&
        !binding.name.toLowerCase().includes(query) &&
        !inputSummary(binding.input).toLowerCase().includes(query)
      ) {
        return;
      }
      const kind = actionKind(binding);
      const button = document.createElement("button");
      button.type = "button";
      button.dataset.bindingIndex = String(index);
      button.className = `binding-item${index === state.selected ? " is-selected" : ""}`;
      button.innerHTML = `
        <span class="binding-symbol ${escapeHtml(kind)}">${escapeHtml(controlKey(binding))}</span>
        <span class="binding-copy">
          <strong>${escapeHtml(binding.name || "Untitled binding")}</strong>
          <small>${escapeHtml(actionLabel(kind))} · ${escapeHtml(inputSummary(binding.input))}</small>
        </span>
        <span class="binding-index">${String(index + 1).padStart(2, "0")}</span>
      `;
      button.addEventListener("click", () => selectBinding(index));
      elements.bindingList.append(button);
    });
    updateTestPreview();
  }

  function renderOverlay() {
    elements.controlLayer.replaceChildren();
    state.profile.bindings.forEach((binding, index) => {
      const node = createControlNode(binding, index);
      if (node) elements.controlLayer.append(node);
    });
    updateTestPreview();
  }

  function createControlNode(binding, index) {
    const action = binding.action;
    if (!["tap", "hold", "virtual_joystick", "mouse_aim"].includes(action.kind)) return null;
    const node = document.createElement("div");
    node.dataset.index = String(index);
    node.dataset.bindingIndex = String(index);
    node.dataset.kind = action.kind;
    const pointAction = action.kind === "tap" || action.kind === "hold";
    node.className = `control-node ${action.kind === "tap" ? "tap-node" : action.kind === "hold" ? "hold-node" : action.kind === "virtual_joystick" ? "joystick-node" : "aim-node"}${index === state.selected ? " is-selected" : ""}`;
    node.innerHTML = `
      ${pointAction ? `<span class="node-key">${escapeHtml(controlKey(binding))}</span>` : ""}
      <span class="control-label">${escapeHtml(binding.name)} / ${escapeHtml(controlKey(binding))}</span>
      ${!pointAction && index === state.selected ? '<span class="resize-handle" data-resize="true"></span>' : ""}
    `;
    applyNodeGeometry(node, action);
    node.addEventListener("pointerdown", beginDrag);
    return node;
  }

  function applyNodeGeometry(node, action) {
    if (action.kind === "tap" || action.kind === "hold") {
      node.style.left = `${action.point.x * 100}%`;
      node.style.top = `${action.point.y * 100}%`;
      return;
    }
    if (action.kind === "virtual_joystick") {
      const rect = elements.viewport.getBoundingClientRect();
      const diameter = Math.max(34, action.radius * Math.min(rect.width, rect.height) * 2);
      node.style.left = `${action.center.x * 100}%`;
      node.style.top = `${action.center.y * 100}%`;
      node.style.width = `${diameter}px`;
      node.style.height = `${diameter}px`;
      return;
    }
    node.style.left = `${action.region.x * 100}%`;
    node.style.top = `${action.region.y * 100}%`;
    node.style.width = `${action.region.w * 100}%`;
    node.style.height = `${action.region.h * 100}%`;
  }

  function pointerPosition(event) {
    const rect = elements.viewport.getBoundingClientRect();
    return {
      x: clamp((event.clientX - rect.left) / rect.width, 0, 1),
      y: clamp((event.clientY - rect.top) / rect.height, 0, 1),
      rect,
    };
  }

  function beginDrag(event) {
    if (state.testing) {
      event.preventDefault();
      return;
    }
    const node = event.currentTarget;
    const index = Number(node.dataset.index);
    if (state.selected !== index) {
      state.selected = index;
      node.classList.add("is-selected");
      renderBindingList();
      renderInspector();
      updateStatus();
    }
    const action = state.profile.bindings[index].action;
    const pointer = pointerPosition(event);
    state.drag = {
      index,
      mode: event.target.dataset.resize ? "resize" : "move",
      start: pointer,
      before: clone(state.profile),
      action: clone(action),
      node,
    };
    node.setPointerCapture(event.pointerId);
    node.addEventListener("pointermove", moveDrag);
    node.addEventListener("pointerup", endDrag, { once: true });
    node.addEventListener("pointercancel", endDrag, { once: true });
    event.preventDefault();
  }

  function moveDrag(event) {
    if (!state.drag) return;
    const { index, mode, start, action: original, node } = state.drag;
    const action = state.profile.bindings[index].action;
    const pointer = pointerPosition(event);
    const dx = pointer.x - start.x;
    const dy = pointer.y - start.y;

    if (mode === "move") {
      if (action.kind === "tap" || action.kind === "hold") {
        action.point.x = snapped(original.point.x + dx);
        action.point.y = snapped(original.point.y + dy);
      } else if (action.kind === "virtual_joystick") {
        action.center.x = snapped(original.center.x + dx);
        action.center.y = snapped(original.center.y + dy);
      } else if (action.kind === "mouse_aim") {
        action.region.x = snapped(clamp(original.region.x + dx, 0, 1 - action.region.w));
        action.region.y = snapped(clamp(original.region.y + dy, 0, 1 - action.region.h));
      }
    } else if (action.kind === "virtual_joystick") {
      const centerX = action.center.x * pointer.rect.width;
      const centerY = action.center.y * pointer.rect.height;
      const pointerX = pointer.x * pointer.rect.width;
      const pointerY = pointer.y * pointer.rect.height;
      action.radius = rounded(
        clamp(Math.hypot(pointerX - centerX, pointerY - centerY) / Math.min(pointer.rect.width, pointer.rect.height), 0.01, 0.45),
        4,
      );
      action.dead_zone = Math.min(action.dead_zone, rounded(action.radius - 0.001, 4));
    } else if (action.kind === "mouse_aim") {
      action.region.w = snapped(clamp(pointer.x - action.region.x, 0.05, 1 - action.region.x));
      action.region.h = snapped(clamp(pointer.y - action.region.y, 0.05, 1 - action.region.y));
    }

    state.dirty = true;
    applyNodeGeometry(node, action);
    renderInspector();
    updateStatus();
    event.preventDefault();
  }

  function endDrag(event) {
    if (!state.drag) return;
    state.history.push(state.drag.before);
    if (state.history.length > 80) state.history.shift();
    state.future.length = 0;
    state.drag.node.removeEventListener("pointermove", moveDrag);
    state.drag = null;
    renderAll();
    event.preventDefault();
  }

  function section(title, body) {
    return `<section class="inspector-section"><div class="inspector-section-title">${title}</div>${body}</section>`;
  }

  function textField(id, label, value, hint = "") {
    return `<label class="field-label" for="${id}">${label}</label>
      <input class="text-input mono-input" id="${id}" value="${escapeHtml(value ?? "")}" autocomplete="off" spellcheck="false">
      ${hint ? `<p class="hint">${hint}</p>` : ""}`;
  }

  function keyCaptureField(id, label, value, hint = "") {
    const display = value ? String(value).toUpperCase() : "UNBOUND";
    return `<label class="field-label" for="${id}">${label}</label>
      <button class="key-capture-button" id="${id}" type="button" aria-label="${escapeHtml(label)}: ${escapeHtml(display)}" aria-pressed="false">
        <strong>${escapeHtml(display)}</strong><small>PRESS TO BIND</small>
      </button>
      ${hint ? `<p class="hint">${hint}</p>` : ""}`;
  }

  function clusterKeyButton(id, area, label, value) {
    return `<button class="key-capture-button key-${area}" id="${id}" type="button" aria-label="${label}: ${escapeHtml(value)}" aria-pressed="false">
      <strong>${escapeHtml(String(value).toUpperCase())}</strong><small>${label}</small>
    </button>`;
  }

  function numberField(id, label, value, min = 0, max = 1, step = 0.001) {
    return `<label class="field-label" for="${id}">${label}</label>
      <input class="number-input" id="${id}" type="number" value="${escapeHtml(value)}" min="${min}" max="${max}" step="${step}">`;
  }

  function selectField(id, label, value, options) {
    return `<label class="field-label" for="${id}">${label}</label>
      <select class="select-input" id="${id}">
        ${options.map(([key, title]) => `<option value="${key}"${key === value ? " selected" : ""}>${title}</option>`).join("")}
      </select>`;
  }

  function renderInspector() {
    disarmKeyCapture();
    const binding = state.profile.bindings[state.selected];
    const active = Boolean(binding);
    elements.inspectorBody.hidden = !active;
    elements.inspectorEmpty.hidden = active;
    elements.deleteButton.disabled = !active;
    elements.duplicateButton.disabled = !active;
    if (!binding) {
      elements.inspectorBody.replaceChildren();
      return;
    }

    const kind = actionKind(binding);
    let html = section(
      "Identity",
      `<div class="binding-type-banner"><strong>${escapeHtml(actionLabel(kind))}</strong><span>${escapeHtml(kind)}</span></div>
       ${textField("bindingName", "Binding name", binding.name, "Names must be unique inside this profile.")}`,
    );
    html += section("Host input", renderInputEditor(binding.input, binding.action.kind));
    html += section("Android action", renderActionEditor(binding.action));
    elements.inspectorBody.innerHTML = html;
    wireInspector(binding);
  }

  function inputOptionsForAction(kind) {
    if (kind === "tap" || kind === "hold") {
      return [
        ["key", "Keyboard key"],
        ["mouse_button", "Mouse button"],
      ];
    }
    if (kind === "virtual_joystick") return [["key_cluster", "Four-key cluster"]];
    if (kind === "mouse_aim") return [["mouse_move", "Relative mouse"]];
    return [
      ["key", "Keyboard key"],
      ["key_cluster", "Four-key cluster"],
      ["mouse_button", "Mouse button"],
      ["mouse_move", "Relative mouse"],
    ];
  }

  function renderInputEditor(input, actionKind) {
    const options = inputOptionsForAction(actionKind);
    let html = options.length === 1
      ? `<label class="field-label">Input source</label>
        <div class="readout locked-source">${escapeHtml(options[0][1])}<span>RUNTIME LOCK</span></div>`
      : selectField("inputKind", "Input source", input.kind, options);
    if (input.kind === "key") {
      html += keyCaptureField(
        "inputKey",
        "Key",
        input.key,
        "Activate the cell, then press the physical key. F12 remains reserved for input release.",
      );
    } else if (input.kind === "key_cluster") {
      html += `<label class="field-label">Directional keys</label>
        <div class="key-cluster-grid">
          ${clusterKeyButton("clusterUp", "up", "UP", input.up)}
          ${clusterKeyButton("clusterLeft", "left", "LEFT", input.left)}
          ${clusterKeyButton("clusterDown", "down", "DOWN", input.down)}
          ${clusterKeyButton("clusterRight", "right", "RIGHT", input.right)}
        </div>
        <p class="hint">Activate a direction cell, then press its physical key.</p>`;
    } else if (input.kind === "mouse_button") {
      html += selectField("mouseButton", "Mouse button", input.button, [
        ["left", "Left / M1"],
        ["right", "Right / M2"],
        ["middle", "Middle / M3"],
        ["side", "Side / M4"],
        ["extra", "Extra / M5"],
      ]);
    } else {
      html += '<p class="hint">Relative X/Y motion drives a persistent Android touch contact while mouse aim is active.</p>';
    }
    return html;
  }

  function renderActionEditor(action) {
    if (action.kind === "tap" || action.kind === "hold") {
      const verb = action.kind === "hold"
        ? "The Android contact stays down until the key or mouse button is released."
        : "The Android contact is pressed and released immediately.";
      return `<div class="inspector-row">
        <div>${numberField("tapX", "X position", action.point.x)}</div>
        <div>${numberField("tapY", "Y position", action.point.y)}</div>
      </div><p class="hint">${verb} Drag the marker to the matching button in the game HUD.</p>`;
    }
    if (action.kind === "virtual_joystick") {
      return `<div class="inspector-row">
          <div>${numberField("stickX", "Center X", action.center.x)}</div>
          <div>${numberField("stickY", "Center Y", action.center.y)}</div>
        </div>
        <div class="inspector-row">
          <div>${numberField("stickRadius", "Radius", action.radius, 0.001, 1)}</div>
          <div>${numberField("stickDeadZone", "Dead zone", action.dead_zone ?? 0, 0, 0.999)}</div>
        </div>
        ${selectField("stickMode", "Activation mode", action.mode || "hold", [["hold", "Hold"], ["toggle", "Toggle"]])}
        ${numberField("stickReaffirm", "Reaffirm interval (ms)", action.reaffirm_ms ?? 50, 1, 5000, 1)}
        <p class="hint">The radius is relative to the shorter surface edge. Drag the ring handle to resize.</p>`;
    }
    return `<div class="inspector-row">
        <div>${numberField("aimX", "Region X", action.region.x)}</div>
        <div>${numberField("aimY", "Region Y", action.region.y)}</div>
      </div>
      <div class="inspector-row">
        <div>${numberField("aimW", "Region width", action.region.w, 0.001, 1)}</div>
        <div>${numberField("aimH", "Region height", action.region.h, 0.001, 1)}</div>
      </div>
      <div class="inspector-row">
        <div>${numberField("aimSensitivity", "Sensitivity", action.sensitivity ?? 1, 0.01, 20, 0.01)}</div>
        <div>${keyCaptureField("aimToggle", "Toggle key", action.toggle_key ?? "", "Backspace clears the toggle.")}</div>
      </div>
      <div class="inspector-row">
        <div>${numberField("aimThreshold", "Recenter threshold", action.recenter_threshold ?? 0.7, 0.1, 1, 0.01)}</div>
        <div>${numberField("aimGap", "Recenter gap (ms)", action.recenter_gap_ms ?? 0, 0, 5000, 1)}</div>
      </div>
      <div class="inspector-row">
        <div>${numberField("aimAds", "ADS multiplier", action.ads_multiplier ?? 0.6, 0.1, 1, 0.01)}</div>
        <div>${numberField("aimReaffirm", "Reaffirm (ms)", action.reaffirm_ms ?? 50, 1, 5000, 1)}</div>
      </div>
      <p class="hint">Tab is recommended for capture toggle. Leave the toggle key empty for always-on aim.</p>`;
  }

  function wireChange(id, setter, parser = (value) => value) {
    const input = document.querySelector(`#${id}`);
    if (!input) return;
    input.addEventListener("change", () => {
      const value = parser(input.value);
      mutate(() => setter(value));
    });
  }

  function disarmKeyCapture() {
    const capture = state.keyCapture;
    if (!capture) return;
    window.removeEventListener("keydown", capture.onKeyDown, true);
    window.removeEventListener("pointerdown", capture.onPointerDown, true);
    if (capture.button.isConnected) {
      capture.button.innerHTML = capture.originalHtml;
      capture.button.classList.remove("is-listening");
      capture.button.setAttribute("aria-pressed", "false");
    }
    state.keyCapture = null;
  }

  function wireKeyCapture(id, setter, { allowEmpty = false } = {}) {
    const button = document.querySelector(`#${id}`);
    if (!button) return;
    button.addEventListener("click", (event) => {
      event.preventDefault();
      disarmKeyCapture();
      const originalHtml = button.innerHTML;
      const onKeyDown = (keyEvent) => {
        keyEvent.preventDefault();
        keyEvent.stopImmediatePropagation();
        if (keyEvent.repeat) return;
        const clearsBinding =
          allowEmpty && (keyEvent.key === "Backspace" || keyEvent.key === "Delete");
        const key = clearsBinding ? null : browserKeyName(keyEvent.key);
        if (!clearsBinding && !supportedKeys.has(key)) {
          toast(`'${keyEvent.key}' is reserved or unsupported by the production runtime.`, true);
          return;
        }
        disarmKeyCapture();
        mutate(() => setter(key));
      };
      const onPointerDown = (pointerEvent) => {
        if (!button.contains(pointerEvent.target)) disarmKeyCapture();
      };
      state.keyCapture = { button, originalHtml, onKeyDown, onPointerDown };
      button.classList.add("is-listening");
      button.setAttribute("aria-pressed", "true");
      button.innerHTML = "<strong>PRESS KEY</strong><small>LISTENING</small>";
      window.addEventListener("keydown", onKeyDown, true);
      window.addEventListener("pointerdown", onPointerDown, true);
    });
  }

  function wireInspector(binding) {
    wireChange("bindingName", (value) => { binding.name = value.trim(); });
    wireChange("inputKind", (value) => {
      binding.input = defaultInput(value);
    });
    if (binding.input.kind === "key") {
      wireKeyCapture("inputKey", (value) => { binding.input.key = value; });
    } else if (binding.input.kind === "key_cluster") {
      wireKeyCapture("clusterUp", (value) => { binding.input.up = value; });
      wireKeyCapture("clusterLeft", (value) => { binding.input.left = value; });
      wireKeyCapture("clusterDown", (value) => { binding.input.down = value; });
      wireKeyCapture("clusterRight", (value) => { binding.input.right = value; });
    } else if (binding.input.kind === "mouse_button") {
      wireChange("mouseButton", (value) => { binding.input.button = value; });
    }

    const action = binding.action;
    const number = (value) => Number(value);
    if (action.kind === "tap" || action.kind === "hold") {
      wireChange("tapX", (value) => { action.point.x = normalized(value); }, number);
      wireChange("tapY", (value) => { action.point.y = normalized(value); }, number);
    } else if (action.kind === "virtual_joystick") {
      wireChange("stickX", (value) => { action.center.x = normalized(value); }, number);
      wireChange("stickY", (value) => { action.center.y = normalized(value); }, number);
      wireChange("stickRadius", (value) => {
        action.radius = rounded(clamp(value, 0.001, 1));
        action.dead_zone = Math.min(action.dead_zone, rounded(action.radius - 0.001));
      }, number);
      wireChange("stickDeadZone", (value) => {
        action.dead_zone = rounded(clamp(value, 0, Math.max(0, action.radius - 0.001)));
      }, number);
      wireChange("stickMode", (value) => { action.mode = value; });
      wireChange("stickReaffirm", (value) => { action.reaffirm_ms = Math.max(1, Math.round(value)); }, number);
    } else if (action.kind === "mouse_aim") {
      wireChange("aimX", (value) => {
        action.region.x = rounded(clamp(value, 0, 1 - action.region.w));
      }, number);
      wireChange("aimY", (value) => {
        action.region.y = rounded(clamp(value, 0, 1 - action.region.h));
      }, number);
      wireChange("aimW", (value) => {
        action.region.w = rounded(clamp(value, 0.001, 1 - action.region.x));
      }, number);
      wireChange("aimH", (value) => {
        action.region.h = rounded(clamp(value, 0.001, 1 - action.region.y));
      }, number);
      wireChange("aimSensitivity", (value) => { action.sensitivity = Math.max(0.01, value); }, number);
      wireKeyCapture("aimToggle", (value) => { action.toggle_key = value; }, { allowEmpty: true });
      wireChange("aimThreshold", (value) => {
        action.recenter_threshold = rounded(clamp(value, 0.1, 1));
      }, number);
      wireChange("aimGap", (value) => {
        action.recenter_gap_ms = Math.max(0, Math.round(value));
      }, number);
      wireChange("aimAds", (value) => {
        action.ads_multiplier = rounded(clamp(value, 0.1, 1));
      }, number);
      wireChange("aimReaffirm", (value) => {
        action.reaffirm_ms = Math.max(1, Math.round(value));
      }, number);
    }
  }

  function defaultInput(kind) {
    if (kind === "key") return { kind: "key", key: "f" };
    if (kind === "key_cluster") {
      return { kind: "key_cluster", up: "w", left: "a", down: "s", right: "d" };
    }
    if (kind === "mouse_button") return { kind: "mouse_button", button: "left" };
    return { kind: "mouse_move" };
  }

  function uniqueName(base) {
    const names = new Set(state.profile.bindings.map((binding) => binding.name));
    if (!names.has(base)) return base;
    let suffix = 2;
    while (names.has(`${base}_${suffix}`)) suffix += 1;
    return `${base}_${suffix}`;
  }

  function addControl(kind) {
    mutate(() => {
      let binding;
      if (kind === "tap") {
        binding = {
          name: uniqueName("new_tap"),
          input: defaultInput("key"),
          action: { kind: "tap", point: { x: 0.5, y: 0.5 } },
        };
      } else if (kind === "hold") {
        binding = {
          name: uniqueName("new_hold"),
          input: defaultInput("mouse_button"),
          action: { kind: "hold", point: { x: 0.5, y: 0.5 } },
        };
      } else if (kind === "virtual_joystick") {
        binding = {
          name: uniqueName("new_joystick"),
          input: defaultInput("key_cluster"),
          action: {
            kind: "virtual_joystick",
            center: { x: 0.2, y: 0.75 },
            radius: 0.1,
            dead_zone: 0.02,
            mode: "hold",
            reaffirm_ms: 50,
          },
        };
      } else {
        binding = {
          name: uniqueName("mouse_aim"),
          input: defaultInput("mouse_move"),
          action: {
            kind: "mouse_aim",
            region: { x: 0.35, y: 0.08, w: 0.58, h: 0.7 },
            sensitivity: 1,
            toggle_key: "tab",
            recenter_threshold: 0.7,
            recenter_gap_ms: 0,
            ads_multiplier: 0.6,
            reaffirm_ms: 50,
          },
        };
      }
      state.profile.bindings.push(binding);
      state.selected = state.profile.bindings.length - 1;
    });
  }

  function deleteSelected() {
    if (state.selected < 0) return;
    mutate(() => {
      state.profile.bindings.splice(state.selected, 1);
      state.selected = Math.min(state.selected, state.profile.bindings.length - 1);
    });
  }

  function duplicateSelected() {
    const source = state.profile.bindings[state.selected];
    if (!source) return;
    mutate(() => {
      const copy = clone(source);
      copy.name = uniqueName(`${source.name}_copy`);
      if (copy.action.kind === "tap" || copy.action.kind === "hold") {
        copy.action.point.x = normalized(copy.action.point.x + 0.025);
        copy.action.point.y = normalized(copy.action.point.y + 0.025);
      } else if (copy.action.kind === "virtual_joystick") {
        copy.action.center.x = normalized(copy.action.center.x + 0.025);
        copy.action.center.y = normalized(copy.action.center.y + 0.025);
      } else if (copy.action.kind === "mouse_aim") {
        copy.action.region.x = clamp(copy.action.region.x + 0.02, 0, 1 - copy.action.region.w);
        copy.action.region.y = clamp(copy.action.region.y + 0.02, 0, 1 - copy.action.region.h);
      }
      state.profile.bindings.splice(state.selected + 1, 0, copy);
      state.selected += 1;
    });
  }

  function validateProfile() {
    const errors = [];
    if (state.profile.schema_version !== 2) errors.push("Schema version must be 2.");
    if (!state.profile.name.trim()) errors.push("Profile name is required.");
    if (!state.profile.package_name.trim()) errors.push("Android package is required.");
    const names = new Set();
    state.profile.bindings.forEach((binding, index) => {
      const label = binding.name || `Binding ${index + 1}`;
      if (!binding.name.trim()) errors.push(`Binding ${index + 1} needs a name.`);
      if (names.has(binding.name)) errors.push(`Duplicate binding name: ${binding.name}.`);
      names.add(binding.name);
      const input = binding.input;
      if (input.kind === "key" && !supportedKeys.has(input.key)) errors.push(`${label}: unsupported key '${input.key}'.`);
      if (input.kind === "key_cluster") {
        [input.up, input.left, input.down, input.right].forEach((key) => {
          if (!supportedKeys.has(key)) errors.push(`${label}: unsupported cluster key '${key}'.`);
        });
      }
      if (input.kind === "mouse_button" && !supportedButtons.has(input.button)) {
        errors.push(`${label}: unsupported mouse button.`);
      }
      const action = binding.action;
      const allowedInputs = inputOptionsForAction(action.kind).map(([kind]) => kind);
      if (!allowedInputs.includes(input.kind)) {
        errors.push(`${label}: ${actionLabel(action.kind)} cannot use ${input.kind}.`);
      }
      if (action.kind === "virtual_joystick" && action.dead_zone >= action.radius) {
        errors.push(`${label}: dead zone must be smaller than radius.`);
      }
      if (action.kind === "mouse_aim") {
        if (action.region.x + action.region.w > 1.00001 || action.region.y + action.region.h > 1.00001) {
          errors.push(`${label}: aim region leaves the viewport.`);
        }
        if (action.toggle_key && !supportedKeys.has(action.toggle_key)) {
          errors.push(`${label}: unsupported toggle key '${action.toggle_key}'.`);
        }
      }
    });
    return errors;
  }

  function updateStatus() {
    const errors = state.profile ? validateProfile() : ["Profile is not loaded."];
    elements.validationState.innerHTML = errors.length
      ? `<span class="status-led error"></span> ${errors.length} validation issue${errors.length === 1 ? "" : "s"}`
      : '<span class="status-led ok"></span> Profile valid';
    elements.validationState.title = errors.join("\n");
    elements.saveState.textContent = state.saving ? "WRITING…" : state.dirty ? "UNSAVED" : "SYNCED";
    elements.saveState.style.color = state.dirty ? "var(--orange)" : "";
    elements.undoButton.disabled = state.history.length === 0;
    elements.redoButton.disabled = state.future.length === 0;
    elements.restoreButton.disabled =
      state.saving || state.launchingLiveTest || !state.backupAvailable;
    elements.saveButton.disabled = state.saving || errors.length > 0;
    elements.closeButton.disabled = state.saving || errors.length > 0;
    elements.liveTestButton.disabled =
      state.saving || state.launchingLiveTest || errors.length > 0 || state.profile.bindings.length === 0;
    elements.liveTestButton.innerHTML = state.launchingLiveTest
      ? '<span class="live-test-mark is-busy">◇</span> Opening game session…'
      : '<span class="live-test-mark">▶</span> Run map in Waydroid';
    const binding = state.profile?.bindings[state.selected];
    elements.selectedStatus.innerHTML = binding
      ? `<span>SELECTED</span> ${escapeHtml(binding.name).toUpperCase()}`
      : "<span>SELECTED</span> NONE";
  }

  async function saveProfile(closeAfter = false) {
    const errors = validateProfile();
    if (errors.length) {
      toast(errors[0], true);
      return false;
    }
    state.saving = true;
    updateStatus();
    try {
      const response = await fetch(api("/api/profile"), {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(state.profile),
      });
      const result = await response.json();
      if (!response.ok) {
        const message = result.errors?.join("; ") || result.error || "Profile save failed.";
        throw new Error(message);
      }
      state.dirty = false;
      state.backupAvailable = Boolean(result.backupAvailable);
      toast(result.changed === false ? "Profile already matches the saved map." : "Profile saved. Previous map retained.");
      if (closeAfter) {
        await fetch(api("/api/close"), { method: "POST" });
        window.close();
        document.body.innerHTML = '<div class="boot-screen"><div class="boot-copy"><span>WROID CONTROL SYSTEM</span><strong>Profile saved. You can close this tab.</strong></div></div>';
      }
      return true;
    } catch (error) {
      toast(error.message, true);
      return false;
    } finally {
      state.saving = false;
      updateStatus();
    }
  }

  async function launchLiveTest() {
    if (state.launchingLiveTest) return;
    if (!(await saveProfile(false))) return;
    state.launchingLiveTest = true;
    updateStatus();
    try {
      const { width, height } = resolutionPresets[state.resolutionKey];
      const response = await fetch(api("/api/live-test"), {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ width, height }),
      });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || "Could not open live profile test.");
      toast(result.message);
    } catch (error) {
      toast(error.message, true);
    } finally {
      state.launchingLiveTest = false;
      updateStatus();
    }
  }

  async function loadPreviousSave() {
    if (!state.backupAvailable || state.saving || state.launchingLiveTest) return;
    try {
      const response = await fetch(api("/api/profile-backup"), { cache: "no-store" });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || "Previous profile save is unavailable.");
      state.history.push(clone(state.profile));
      if (state.history.length > 80) state.history.shift();
      state.future.length = 0;
      state.profile = result.profile;
      state.profile.orientation ||= "landscape";
      state.profile.bindings ||= [];
      state.selected = state.profile.bindings.length
        ? Math.min(Math.max(state.selected, 0), state.profile.bindings.length - 1)
        : -1;
      state.dirty = true;
      renderAll();
      toast("Previous save loaded for review. Save to make it active, or Undo to return.");
    } catch (error) {
      toast(error.message, true);
    }
  }

  async function detectProfileBackup() {
    const response = await fetch(api("/api/profile-backup"), { cache: "no-store" });
    if (response.status === 404) {
      state.backupAvailable = false;
      return;
    }
    if (!response.ok) {
      const result = await response.json().catch(() => ({}));
      throw new Error(result.error || "Could not inspect the previous profile save.");
    }
    state.backupAvailable = true;
  }

  function savePreferences(patch) {
    preferenceWrite = preferenceWrite
      .then(async () => {
        const response = await fetch(api("/api/preferences"), {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(patch),
        });
        const result = await response.json();
        if (!response.ok) throw new Error(result.error || "Could not save Wroid preferences.");
      })
      .catch((error) => toast(`Preference save failed: ${error.message}`, true));
  }

  async function loadPreferences() {
    const response = await fetch(api("/api/preferences"), { cache: "no-store" });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || "Could not load Wroid preferences.");
    const saved = result.preferences?.resolution;
    if (resolutionPresets[saved]) state.resolutionKey = saved;
  }

  function toast(message, error = false) {
    const item = document.createElement("div");
    item.className = `toast${error ? " error" : ""}`;
    item.textContent = message;
    elements.toastStack.append(item);
    window.setTimeout(() => item.remove(), 4200);
  }

  function renderViewportMode() {
    const portrait = state.profile.orientation === "portrait";
    const resolution = resolutionPresets[state.resolutionKey];
    elements.viewport.classList.toggle("is-portrait", portrait);
    elements.viewport.classList.toggle("is-grid", state.snap);
    elements.viewport.classList.toggle("hide-labels", !state.labels);
    elements.viewportReadout.textContent = portrait
      ? `${resolution.height} × ${resolution.width} / PORTRAIT`
      : `${resolution.width} × ${resolution.height} / ${(state.profile.orientation || "landscape").toUpperCase()}`;
    elements.resolutionSwitch.querySelectorAll("[data-resolution]").forEach((button) => {
      const selected = button.dataset.resolution === state.resolutionKey;
      button.classList.toggle("is-active", selected);
      button.setAttribute("aria-checked", String(selected));
    });
    window.requestAnimationFrame(renderOverlay);
  }

  function showBackground(blob, saved) {
    if (state.screenshotUrl) URL.revokeObjectURL(state.screenshotUrl);
    state.screenshotUrl = URL.createObjectURL(blob);
    state.backgroundSaved = saved;
    elements.screenshot.src = state.screenshotUrl;
    elements.screenshot.hidden = false;
    elements.emptySurface.hidden = true;
    elements.clearBackgroundButton.hidden = false;
    elements.backgroundState.textContent = saved ? "BACKGROUND SAVED" : "LOCAL PREVIEW";
    elements.backgroundState.classList.toggle("is-saved", saved);
  }

  function calibrationTargetSize(preview = false) {
    const preset = resolutionPresets[state.resolutionKey];
    const width = state.profile.orientation === "portrait" ? preset.height : preset.width;
    const height = state.profile.orientation === "portrait" ? preset.width : preset.height;
    if (!preview) return { width, height };
    const scale = Math.min(1, 960 / width, 540 / height);
    return {
      width: Math.max(1, Math.round(width * scale)),
      height: Math.max(1, Math.round(height * scale)),
    };
  }

  function calibrationSourceRect(video, targetWidth, targetHeight) {
    const sourceWidth = video.videoWidth;
    const sourceHeight = video.videoHeight;
    const targetAspect = targetWidth / targetHeight;
    const sourceAspect = sourceWidth / sourceHeight;
    let baseWidth;
    let baseHeight;
    if (sourceAspect > targetAspect) {
      baseHeight = sourceHeight;
      baseWidth = baseHeight * targetAspect;
    } else {
      baseWidth = sourceWidth;
      baseHeight = baseWidth / targetAspect;
    }

    const zoom = state.calibration?.zoom || 1;
    const width = baseWidth / zoom;
    const height = baseHeight / zoom;
    const maxShiftX = Math.max(0, (sourceWidth - width) / 2);
    const maxShiftY = Math.max(0, (sourceHeight - height) / 2);
    const centerX = sourceWidth / 2 + (state.calibration?.offsetX || 0) * maxShiftX;
    const centerY = sourceHeight / 2 + (state.calibration?.offsetY || 0) * maxShiftY;
    return {
      x: clamp(centerX - width / 2, 0, sourceWidth - width),
      y: clamp(centerY - height / 2, 0, sourceHeight - height),
      width,
      height,
    };
  }

  function drawCalibrationFrame(canvas, width, height) {
    const calibration = state.calibration;
    if (!calibration || calibration.video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) return false;
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
    const source = calibrationSourceRect(calibration.video, width, height);
    const context = canvas.getContext("2d", { alpha: false });
    context.drawImage(
      calibration.video,
      source.x,
      source.y,
      source.width,
      source.height,
      0,
      0,
      width,
      height,
    );
    return true;
  }

  function renderLiveCalibration() {
    if (!state.calibration) return;
    const target = calibrationTargetSize(true);
    drawCalibrationFrame(elements.liveCalibrationCanvas, target.width, target.height);
    state.calibration.animationFrame = requestAnimationFrame(renderLiveCalibration);
  }

  function renderCalibrationControls() {
    const calibration = state.calibration;
    if (!calibration) return;
    elements.calibrationZoom.value = String(Math.round(calibration.zoom * 100));
    elements.calibrationX.value = String(Math.round(calibration.offsetX * 100));
    elements.calibrationY.value = String(Math.round(calibration.offsetY * 100));
    elements.calibrationZoomValue.textContent = `${Math.round(calibration.zoom * 100)}%`;
    elements.calibrationXValue.textContent = `${Math.round(calibration.offsetX * 100)}`;
    elements.calibrationYValue.textContent = `${Math.round(calibration.offsetY * 100)}`;
  }

  function liveCalibrationUi(active) {
    document.querySelector(".workspace").classList.toggle("is-calibrating", active);
    elements.calibrationDock.hidden = !active;
    elements.liveCalibrationCanvas.hidden = !active;
    elements.captureButton.classList.toggle("is-active", active);
    elements.captureButton.innerHTML = active
      ? "<span>■</span> End live align"
      : "<span>◉</span> Live align";
    elements.screenshotButton.disabled = active;
    elements.clearBackgroundButton.disabled = active;
    if (active) {
      elements.emptySurface.hidden = true;
      elements.backgroundState.textContent = "LIVE WINDOW";
      elements.backgroundState.classList.add("is-live");
    } else {
      elements.backgroundState.classList.remove("is-live");
      elements.screenshot.hidden = !state.screenshotUrl;
      elements.emptySurface.hidden = Boolean(state.screenshotUrl);
      elements.backgroundState.textContent = state.screenshotUrl
        ? state.backgroundSaved ? "BACKGROUND SAVED" : "LOCAL PREVIEW"
        : "NO BACKGROUND";
      elements.backgroundState.classList.toggle("is-saved", state.backgroundSaved);
    }
  }

  function stopLiveCalibration({ notify = false } = {}) {
    const calibration = state.calibration;
    if (!calibration) return;
    state.calibration = null;
    cancelAnimationFrame(calibration.animationFrame);
    calibration.stream.getTracks().forEach((track) => {
      track.onended = null;
      track.stop();
    });
    calibration.video.pause();
    calibration.video.srcObject = null;
    liveCalibrationUi(false);
    if (notify) toast("Live window calibration ended.");
  }

  async function persistBackground(blob, successMessage = "Calibration background saved with this profile.") {
    state.backgroundSaving = true;
    elements.backgroundState.textContent = "WRITING IMAGE…";
    elements.captureButton.disabled = true;
    elements.screenshotButton.disabled = true;
    try {
      const response = await fetch(api("/api/background"), {
        method: "PUT",
        headers: { "Content-Type": blob.type || "application/octet-stream" },
        body: blob,
      });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || "Background save failed.");
      elements.backgroundState.textContent = "BACKGROUND SAVED";
      elements.backgroundState.classList.add("is-saved");
      state.backgroundSaved = true;
      toast(successMessage);
      return true;
    } catch (error) {
      elements.backgroundState.textContent = "LOCAL PREVIEW";
      elements.backgroundState.classList.remove("is-saved");
      state.backgroundSaved = false;
      toast(error.message, true);
      return false;
    } finally {
      state.backgroundSaving = false;
      elements.captureButton.disabled = false;
      elements.screenshotButton.disabled = false;
    }
  }

  async function loadScreenshot(file) {
    if (!file || !file.type.startsWith("image/")) {
      toast("Choose an image file.", true);
      return;
    }
    showBackground(file, false);
    await persistBackground(file);
  }

  async function captureWindow() {
    if (state.calibration) {
      stopLiveCalibration({ notify: true });
      return;
    }
    if (!navigator.mediaDevices?.getDisplayMedia) {
      toast("Window capture is not supported by this browser.", true);
      return;
    }
    let stream;
    try {
      toast("Choose the Waydroid game window. Its live surface will stay behind the control map.");
      stream = await navigator.mediaDevices.getDisplayMedia({
        video: { frameRate: { ideal: 30, max: 60 } },
        audio: false,
      });
      const video = document.createElement("video");
      video.muted = true;
      video.playsInline = true;
      video.srcObject = stream;
      await video.play();
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      if (!video.videoWidth || !video.videoHeight) throw new Error("The selected window did not provide video frames.");

      const track = stream.getVideoTracks()[0];
      const settings = track.getSettings();
      state.calibration = {
        stream,
        video,
        animationFrame: 0,
        zoom: 1,
        offsetX: 0,
        offsetY: 0,
      };
      stream = null;
      const surface = settings.displaySurface ? ` · ${String(settings.displaySurface).toUpperCase()}` : "";
      elements.calibrationSource.textContent =
        `${track.label || "Selected source"} · ${video.videoWidth}×${video.videoHeight}${surface}`;
      track.onended = () => {
        if (!state.calibration) return;
        stopLiveCalibration();
        toast("The selected window stopped sharing.", true);
      };
      renderCalibrationControls();
      liveCalibrationUi(true);
      renderLiveCalibration();
      if (settings.displaySurface && settings.displaySurface !== "window") {
        toast("A full screen was selected. Use pan and zoom, or restart Live align and choose the Waydroid window.", true);
      } else {
        toast("Live alignment active. Move controls over the HUD, then save an aligned frame.");
      }
    } catch (error) {
      if (state.calibration) stopLiveCalibration();
      else stream?.getTracks().forEach((track) => track.stop());
      if (error.name !== "NotAllowedError") toast(error.message, true);
    }
  }

  async function saveCalibrationFrame() {
    if (!state.calibration || state.backgroundSaving) return;
    const target = calibrationTargetSize();
    const canvas = document.createElement("canvas");
    if (!drawCalibrationFrame(canvas, target.width, target.height)) {
      toast("The live window has no current frame.", true);
      return;
    }
    elements.calibrationSaveButton.disabled = true;
    try {
      const blob = await new Promise((resolve, reject) => {
        canvas.toBlob(
          (result) => result ? resolve(result) : reject(new Error("Could not encode the aligned frame.")),
          "image/webp",
          0.92,
        );
      });
      showBackground(blob, false);
      await persistBackground(
        blob,
        `Aligned ${target.width}×${target.height} frame saved with this profile.`,
      );
      liveCalibrationUi(true);
    } catch (error) {
      toast(error.message, true);
    } finally {
      elements.calibrationSaveButton.disabled = false;
    }
  }

  function updateCalibrationFromControls() {
    if (!state.calibration) return;
    state.calibration.zoom = Number(elements.calibrationZoom.value) / 100;
    state.calibration.offsetX = Number(elements.calibrationX.value) / 100;
    state.calibration.offsetY = Number(elements.calibrationY.value) / 100;
    renderCalibrationControls();
  }

  function resetCalibration() {
    if (!state.calibration) return;
    state.calibration.zoom = 1;
    state.calibration.offsetX = 0;
    state.calibration.offsetY = 0;
    renderCalibrationControls();
  }

  async function clearBackground() {
    try {
      const response = await fetch(api("/api/background"), { method: "DELETE" });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || "Could not remove background.");
      if (state.screenshotUrl) URL.revokeObjectURL(state.screenshotUrl);
      state.screenshotUrl = null;
      state.backgroundSaved = false;
      elements.screenshot.removeAttribute("src");
      elements.screenshot.hidden = true;
      elements.emptySurface.hidden = false;
      elements.clearBackgroundButton.hidden = true;
      elements.backgroundState.textContent = "NO BACKGROUND";
      elements.backgroundState.classList.remove("is-saved");
      toast(result.message);
    } catch (error) {
      toast(error.message, true);
    }
  }

  async function loadSavedBackground() {
    const response = await fetch(api("/api/background"), { cache: "no-store" });
    if (response.status === 404) return;
    if (!response.ok) throw new Error(`Saved background could not be loaded (${response.status}).`);
    showBackground(await response.blob(), true);
  }

  function wireStaticEvents() {
    elements.profileName.addEventListener("change", () => mutate(() => {
      state.profile.name = elements.profileName.value.trim();
    }));
    elements.packageName.addEventListener("change", () => mutate(() => {
      state.profile.package_name = elements.packageName.value.trim();
    }));
    elements.orientation.addEventListener("change", () => mutate(() => {
      state.profile.orientation = elements.orientation.value;
    }));
    elements.bindingSearch.addEventListener("input", renderBindingList);
    document.querySelectorAll("[data-add]").forEach((button) => {
      button.addEventListener("click", () => addControl(button.dataset.add));
    });
    elements.deleteButton.addEventListener("click", deleteSelected);
    elements.duplicateButton.addEventListener("click", duplicateSelected);
    elements.undoButton.addEventListener("click", undo);
    elements.redoButton.addEventListener("click", redo);
    elements.restoreButton.addEventListener("click", loadPreviousSave);
    elements.saveButton.addEventListener("click", () => saveProfile(false));
    elements.closeButton.addEventListener("click", () => saveProfile(true));
    elements.liveTestButton.addEventListener("click", launchLiveTest);
    elements.resolutionSwitch.addEventListener("click", (event) => {
      const button = event.target.closest("[data-resolution]");
      if (!button || !resolutionPresets[button.dataset.resolution]) return;
      state.resolutionKey = button.dataset.resolution;
      savePreferences({ resolution: state.resolutionKey });
      renderViewportMode();
      toast(`${resolutionPresets[state.resolutionKey].label}p selected for the next Waydroid session.`);
    });
    elements.testButton.addEventListener("click", () => setTesting(!state.testing));
    elements.gridButton.addEventListener("click", () => {
      state.snap = !state.snap;
      elements.gridButton.classList.toggle("is-active", state.snap);
      elements.gridButton.setAttribute("aria-pressed", String(state.snap));
      renderViewportMode();
    });
    elements.labelsButton.addEventListener("click", () => {
      state.labels = !state.labels;
      elements.labelsButton.classList.toggle("is-active", state.labels);
      elements.labelsButton.setAttribute("aria-pressed", String(state.labels));
      renderViewportMode();
    });
    elements.screenshotButton.addEventListener("click", () => elements.screenshotInput.click());
    elements.screenshotInput.addEventListener("change", () => loadScreenshot(elements.screenshotInput.files[0]));
    elements.captureButton.addEventListener("click", captureWindow);
    elements.calibrationStopButton.addEventListener("click", () => stopLiveCalibration({ notify: true }));
    elements.calibrationSaveButton.addEventListener("click", saveCalibrationFrame);
    elements.calibrationResetButton.addEventListener("click", resetCalibration);
    elements.calibrationZoom.addEventListener("input", updateCalibrationFromControls);
    elements.calibrationX.addEventListener("input", updateCalibrationFromControls);
    elements.calibrationY.addEventListener("input", updateCalibrationFromControls);
    elements.clearBackgroundButton.addEventListener("click", clearBackground);
    elements.viewport.addEventListener("pointerdown", (event) => handleTestPointer(event, true), true);
    window.addEventListener("pointerup", (event) => handleTestPointer(event, false), true);
    elements.viewport.addEventListener("contextmenu", (event) => {
      if (state.testing) event.preventDefault();
    });
    elements.viewport.addEventListener("dragover", (event) => {
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
    });
    elements.viewport.addEventListener("drop", (event) => {
      event.preventDefault();
      loadScreenshot(event.dataTransfer.files[0]);
    });
    elements.viewport.addEventListener("pointermove", (event) => {
      if (state.drag) return;
      const pointer = pointerPosition(event);
      elements.cursorReadout.textContent = `X ${pointer.x.toFixed(3)} / Y ${pointer.y.toFixed(3)}`;
      if (event.pointerType === "mouse" && (event.movementX || event.movementY)) {
        pulseMouseMovement();
      }
    });
    elements.viewport.addEventListener("pointerleave", () => {
      if (!state.drag) elements.cursorReadout.textContent = "X ——— / Y ———";
    });
    window.addEventListener("keydown", (event) => {
      const command = event.ctrlKey || event.metaKey;
      if (command && event.key.toLowerCase() === "s") {
        event.preventDefault();
        saveProfile(false);
      } else if (command && event.key.toLowerCase() === "z" && event.shiftKey) {
        event.preventDefault();
        redo();
      } else if (command && event.key.toLowerCase() === "z") {
        event.preventDefault();
        undo();
      } else {
        handleTestKey(event, true);
      }
    });
    window.addEventListener("keyup", (event) => handleTestKey(event, false));
    window.addEventListener("blur", () => {
      disarmKeyCapture();
      if (!state.testing) return;
      state.pressedKeys.clear();
      state.pressedButtons.clear();
      state.mouseMoving = false;
      updateTestPreview();
    });
    window.addEventListener("beforeunload", (event) => {
      if (state.dirty) {
        event.preventDefault();
        event.returnValue = "";
      }
    });
    window.addEventListener("pagehide", () => stopLiveCalibration());
    window.addEventListener("resize", () => window.requestAnimationFrame(renderOverlay));
  }

  async function boot() {
    try {
      const response = await fetch(api("/api/profile"), { cache: "no-store" });
      if (!response.ok) throw new Error(`Editor authorization failed (${response.status}).`);
      state.profile = await response.json();
      state.profile.orientation ||= "landscape";
      state.profile.bindings ||= [];
      state.selected = state.profile.bindings.length ? 0 : -1;
      const startupWarnings = [];
      try {
        await loadPreferences();
      } catch (error) {
        startupWarnings.push(`preferences: ${error.message}`);
      }
      try {
        await detectProfileBackup();
      } catch (error) {
        startupWarnings.push(`previous save: ${error.message}`);
      }
      await loadSavedBackground();
      wireStaticEvents();
      renderAll();
      elements.shell.hidden = false;
      elements.boot.remove();
      if (startupWarnings.length) toast(`Startup warning — ${startupWarnings.join(" · ")}`, true);
    } catch (error) {
      elements.boot.querySelector("strong").textContent = error.message;
      elements.boot.querySelector("span").textContent = "STARTUP ERROR";
    }
  }

  boot();
})();
