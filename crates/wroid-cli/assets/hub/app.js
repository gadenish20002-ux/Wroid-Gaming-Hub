const token = new URLSearchParams(window.location.search).get("token") || "";

const elements = {
  hero: document.querySelector("#hero"),
  heroName: document.querySelector("#hero-name"),
  heroDescription: document.querySelector("#hero-description"),
  heroIndex: document.querySelector("#hero-index"),
  heroInstallState: document.querySelector("#hero-install-state"),
  heroBindingCount: document.querySelector("#hero-binding-count"),
  heroControlsState: document.querySelector("#hero-controls-state"),
  heroSessionState: document.querySelector("#hero-session-state"),
  heroMonogram: document.querySelector("#hero-monogram"),
  gameCode: document.querySelector("#game-code"),
  aimMode: document.querySelector("#aim-mode"),
  launchButton: document.querySelector("#launch-button"),
  launchIcon: document.querySelector("#launch-icon"),
  launchEyebrow: document.querySelector("#launch-eyebrow"),
  launchLabel: document.querySelector("#launch-label"),
  launchNote: document.querySelector("#launch-note"),
  sessionReport: document.querySelector("#session-report"),
  sessionReportTitle: document.querySelector("#session-report-title"),
  sessionReportTime: document.querySelector("#session-report-time"),
  sessionReportDetail: document.querySelector("#session-report-detail"),
  sessionMetrics: document.querySelector("#session-metrics"),
  sessionInputP95: document.querySelector("#session-input-p95"),
  sessionInputSamples: document.querySelector("#session-input-samples"),
  sessionKernelP95: document.querySelector("#session-kernel-p95"),
  sessionKernelSamples: document.querySelector("#session-kernel-samples"),
  sessionTouchFrames: document.querySelector("#session-touch-frames"),
  sessionPeakContacts: document.querySelector("#session-peak-contacts"),
  editButton: document.querySelector("#edit-button"),
  controlsEditButton: document.querySelector("#controls-edit-button"),
  gameGrid: document.querySelector("#game-grid"),
  runtimePill: document.querySelector("#runtime-pill"),
  runtimeLabel: document.querySelector("#runtime-label"),
  waydroidChip: document.querySelector("#waydroid-chip"),
  waydroidDetail: document.querySelector("#waydroid-detail"),
  inputSelfTestButton: document.querySelector("#input-self-test-button"),
  keyboardDot: document.querySelector("#keyboard-dot"),
  keyboardSelect: document.querySelector("#keyboard-select"),
  keyboardPath: document.querySelector("#keyboard-path"),
  mouseDot: document.querySelector("#mouse-dot"),
  mouseSelect: document.querySelector("#mouse-select"),
  mousePath: document.querySelector("#mouse-path"),
  focusProtectionTitle: document.querySelector("#focus-protection-title"),
  focusProtectionDot: document.querySelector("#focus-protection-dot"),
  focusProtectionDetail: document.querySelector("#focus-protection-detail"),
  helperTitle: document.querySelector("#helper-title"),
  helperChip: document.querySelector("#helper-chip"),
  helperDetail: document.querySelector("#helper-detail"),
  helperSetupButton: document.querySelector("#helper-setup-button"),
  graphicsTitle: document.querySelector("#graphics-title"),
  graphicsChip: document.querySelector("#graphics-chip"),
  graphicsRenderer: document.querySelector("#graphics-renderer"),
  graphicsDriver: document.querySelector("#graphics-driver"),
  androidGraphics: document.querySelector("#android-graphics"),
  androidAbi: document.querySelector("#android-abi"),
  displayMode: document.querySelector("#display-mode"),
  desktopSession: document.querySelector("#desktop-session"),
  framePacing: document.querySelector("#frame-pacing"),
  framePacingDetail: document.querySelector("#frame-pacing-detail"),
  graphicsFindings: document.querySelector("#graphics-findings"),
  graphicsSetupButton: document.querySelector("#graphics-setup-button"),
  compatibilityCard: document.querySelector("#compatibility-card"),
  compatibilityTitle: document.querySelector("#compatibility-title"),
  compatibilityChip: document.querySelector("#compatibility-chip"),
  compatibilityAndroid: document.querySelector("#compatibility-android"),
  compatibilityAbis: document.querySelector("#compatibility-abis"),
  compatibilityBridge: document.querySelector("#compatibility-bridge"),
  compatibilityStore: document.querySelector("#compatibility-store"),
  storageAvailable: document.querySelector("#storage-available"),
  storageFill: document.querySelector("#storage-fill"),
  storageDetail: document.querySelector("#storage-detail"),
  compatibilityNext: document.querySelector("#compatibility-next"),
  compatibilitySetupButton: document.querySelector("#compatibility-setup-button"),
  compatibilityGames: document.querySelector("#compatibility-games"),
  compatibilityFindings: document.querySelector("#compatibility-findings"),
  libraryPath: document.querySelector("#library-path"),
  versionLabel: document.querySelector("#version-label"),
  refreshButton: document.querySelector("#refresh-button"),
  storeButton: document.querySelector("#store-button"),
  showWaydroidButton: document.querySelector("#show-waydroid-button"),
  closeButton: document.querySelector("#close-button"),
  importButton: document.querySelector("#import-button"),
  importInput: document.querySelector("#import-input"),
  sideloadButton: document.querySelector("#sideload-button"),
  sideloadInput: document.querySelector("#sideload-input"),
  packageIntake: document.querySelector("#package-intake"),
  packageTitle: document.querySelector("#package-intake-title"),
  packageFileName: document.querySelector("#package-file-name"),
  packageState: document.querySelector("#package-state"),
  packageProgress: document.querySelector("#package-progress"),
  packageProgressFill: document.querySelector("#package-progress-fill"),
  packageProgressLabel: document.querySelector("#package-progress-label"),
  packageFormat: document.querySelector("#package-format"),
  packageSize: document.querySelector("#package-size"),
  packageAbis: document.querySelector("#package-abis"),
  packageCompatibility: document.querySelector("#package-compatibility"),
  packageDetail: document.querySelector("#package-detail"),
  packageInstallButton: document.querySelector("#package-install-button"),
  packageDiscardButton: document.querySelector("#package-discard-button"),
  presetSwitch: document.querySelector("#preset-switch"),
  gameModeToggle: document.querySelector("#game-mode-toggle"),
  gameModeState: document.querySelector("#game-mode-state"),
  toastStack: document.querySelector("#toast-stack"),
};

const marks = {
  pubg: "P",
  freefire: "F",
  brawl: "B",
  standoff: "S",
  custom: "W",
};

let hubState = null;
let selectedId = null;
let busy = false;
let resolution = "1600x900";
let gameModeEnabled = true;
let preferenceWrite = Promise.resolve();
let stateLoadPromise = null;
let lastStateLoadAt = 0;
let packageIntake = null;
let apkUploadRequest = null;
let apkStatusTimer = null;
const focusRefreshMinimumMs = 1500;
const maximumApkBytes = 4 * 1024 * 1024 * 1024;

function apiUrl(path) {
  const separator = path.includes("?") ? "&" : "?";
  return `${path}${separator}token=${encodeURIComponent(token)}`;
}

async function request(path, options = {}) {
  const response = await fetch(apiUrl(path), {
    cache: "no-store",
    ...options,
  });
  const body = await response.json().catch(() => ({}));
  if (!response.ok) {
    const message = body.error || (body.errors || []).join("; ") || `Request failed (${response.status})`;
    throw new Error(message);
  }
  return body;
}

function loadState(options = {}) {
  if (stateLoadPromise) return stateLoadPromise;
  stateLoadPromise = loadStateNow(options).finally(() => {
    stateLoadPromise = null;
  });
  return stateLoadPromise;
}

async function loadStateNow({ announce = false } = {}) {
  elements.refreshButton.classList.add("is-loading");
  try {
    hubState = await request("/api/state");
    lastStateLoadAt = Date.now();
    if (["1280x720", "1600x900", "1920x1080"].includes(hubState.preferences?.resolution)) {
      resolution = hubState.preferences.resolution;
    }
    gameModeEnabled = hubState.preferences?.gameMode !== false;
    if (!selectedId || !hubState.games.some((game) => game.id === selectedId)) {
      selectedId = hubState.games[0]?.id || null;
    }
    render();
    if (announce) toast("System state refreshed");
    if (hubState.libraryErrors.length) {
      toast(`${hubState.libraryErrors.length} invalid profile(s) skipped`, true);
    }
    if (hubState.preferencesError) {
      toast(`Saved preferences could not be loaded: ${hubState.preferencesError}`, true);
    }
  } catch (error) {
    toast(error.message, true);
  } finally {
    elements.refreshButton.classList.remove("is-loading");
  }
}

function render() {
  if (!hubState) return;
  renderSystem();
  renderLibrary();
  renderHero();
  renderPreset();
  renderGameMode();
  elements.libraryPath.textContent = hubState.libraryPath;
  elements.versionLabel.textContent = `v${hubState.version}`;
}

function renderSystem() {
  const system = hubState.system;
  const waydroid = system.waydroid;
  const inputBridge = system.inputBridge;
  const graphics = system.graphics;
  const compatibility = system.compatibility;
  elements.runtimePill.classList.toggle("ready", waydroid.running);
  elements.runtimePill.classList.toggle("offline", !waydroid.running);
  elements.runtimePill.classList.toggle("blocked", graphics.health === "blocked");
  elements.runtimePill.classList.toggle("attention", compatibility.health === "action_required");
  elements.runtimeLabel.textContent =
    inputBridge?.busy
      ? "Game session active"
      : graphics.health === "blocked"
      ? "Performance blocked"
      : compatibility.health === "action_required"
        ? "Compatibility setup"
      : waydroid.running
        ? "Waydroid online"
        : waydroid.available
          ? "Waydroid stopped"
          : "Waydroid unavailable";

  elements.waydroidChip.textContent = waydroid.running ? "Running" : waydroid.available ? "Stopped" : "Missing";
  elements.waydroidChip.classList.toggle("ready", waydroid.running);
  elements.waydroidDetail.textContent =
    waydroid.status || waydroid.error || "Waydroid is not available on PATH.";
  const selfTestReady =
    waydroid.available
    && system.keyboard.ready
    && system.bridgeHelper?.ready
    && !inputBridge?.busy;
  elements.inputSelfTestButton.disabled = !selfTestReady;
  elements.inputSelfTestButton.title = inputBridge?.busy
    ? "Stop the active game session before running diagnostics."
    : !system.keyboard.ready
      ? "A compatible keyboard is required."
      : !system.bridgeHelper?.ready
        ? "Install the production bridge helper first."
      : waydroid.available
        ? "Starts the selected control map without launching its game."
        : "Waydroid is unavailable.";

  renderDevice(
    system.keyboard,
    elements.keyboardDot,
    elements.keyboardSelect,
    elements.keyboardPath,
    hubState.preferences?.keyboard,
  );
  renderDevice(
    system.mouse,
    elements.mouseDot,
    elements.mouseSelect,
    elements.mousePath,
    hubState.preferences?.mouse,
  );
  renderFocusProtection(system.focusProtection);
  renderBridgeHelper(system.bridgeHelper);
  renderGraphics(graphics);
  renderStorage(system.storage);
  renderCompatibility(compatibility);
}

function renderBridgeHelper(helper = {}) {
  const ready = Boolean(helper.ready);
  const installing = helper.state === "installing";
  const unsafe = helper.state === "unsafe";
  const repair = unsafe || helper.state === "outdated";
  elements.helperTitle.textContent = ready
    ? "Production helper"
    : installing
      ? "Authorizing helper"
      : repair
        ? "Repair helper"
        : "One-time setup";
  elements.helperChip.textContent = ready
    ? "Ready"
    : installing
      ? "Installing"
      : unsafe
        ? "Unsafe"
        : helper.state === "outdated"
          ? "Update"
          : "Setup";
  elements.helperChip.className = `state-chip ${ready ? "ready" : installing ? "warning" : unsafe ? "blocked" : "action_required"}`;
  elements.helperDetail.textContent =
    helper.detail || "Production bridge helper state unavailable.";
  elements.helperSetupButton.hidden = ready;
  elements.helperSetupButton.disabled = installing;
  elements.helperSetupButton.textContent = installing
    ? "Authorization open…"
    : repair
      ? "Repair secure helper ↗"
      : "Install secure helper ↗";
}

function renderFocusProtection(focusProtection) {
  const supported = Boolean(focusProtection?.supported);
  elements.focusProtectionTitle.textContent = supported ? "Automatic focus guard" : "Fallback focus guard";
  elements.focusProtectionDot.classList.toggle("ready", supported);
  elements.focusProtectionDetail.textContent =
    focusProtection?.detail || "Focus protection state unavailable.";
}

function renderGraphics(graphics) {
  const healthLabels = { ready: "Ready", warning: "Review", blocked: "Blocked" };
  elements.graphicsChip.textContent = healthLabels[graphics.health] || "Unknown";
  elements.graphicsChip.className = `state-chip ${graphics.health}`;
  elements.graphicsTitle.textContent =
    graphics.health === "blocked" ? "Launch blocked" : graphics.health === "warning" ? "Review graphics" : "Graphics ready";
  elements.graphicsRenderer.textContent = graphics.host.renderer || "Renderer unknown";
  const drivers = graphics.drmDevices
    .map((device) => [device.vendor, device.driver].filter(Boolean).join(" / "))
    .filter((value, index, values) => value && values.indexOf(value) === index);
  elements.graphicsDriver.textContent = drivers.length
    ? `DRM ${drivers.join(" · ")}`
    : `Probe ${graphics.host.source || "unavailable"}`;
  elements.androidGraphics.textContent = [
    graphics.android.egl && `EGL ${graphics.android.egl}`,
    graphics.android.vulkan && `VK ${graphics.android.vulkan}`,
  ].filter(Boolean).join(" · ") || "Waydroid graphics unknown";
  elements.androidAbi.textContent = [
    graphics.android.gralloc && `gralloc ${graphics.android.gralloc}`,
    graphics.android.abi && `ABI ${graphics.android.abi}`,
  ].filter(Boolean).join(" · ") || "Android ABI unknown";
  elements.displayMode.textContent = graphics.display
    ? `${graphics.display.resolution} @ ${graphics.display.refreshHz.toFixed(2)} Hz`
    : "Display mode unknown";
  elements.desktopSession.textContent = [graphics.desktop, graphics.sessionType]
    .filter(Boolean)
    .join(" / ") || "Desktop session unknown";
  const pacing = graphics.framePacing || {};
  elements.framePacing.textContent = pacing.source
    ? pacing.targetHz
      ? `${pacing.targetHz.toFixed(2)} Hz host-driven`
      : "Host-driven timing"
    : "Starts with Waydroid";
  elements.framePacingDetail.textContent = pacing.presentationFeedback === true
    ? "wp_presentation · phase locked"
    : pacing.presentationFeedback === false
      ? "Presentation feedback disabled"
      : "Effective target unavailable offline";
  elements.graphicsFindings.replaceChildren(
    ...graphics.findings.map((finding) => {
      const item = document.createElement("span");
      item.className = `finding ${finding.severity}`;
      item.textContent = finding.message;
      item.title = finding.code;
      return item;
    }),
  );
  elements.graphicsSetupButton.hidden = !graphics.gpuSetup?.needed;
  elements.graphicsSetupButton.textContent = `${graphics.gpuSetup?.label || "Use active GPU"} ↗`;
  elements.graphicsSetupButton.title = graphics.gpuSetup?.detail || "";
}

function renderCompatibility(compatibility) {
  const healthLabels = {
    ready: "Ready",
    warning: "Check runtime",
    action_required: "Action needed",
  };
  elements.compatibilityChip.textContent = healthLabels[compatibility.health] || "Unknown";
  elements.compatibilityChip.className = `state-chip ${compatibility.health}`;
  elements.compatibilityTitle.textContent =
    compatibility.health === "ready"
      ? "Games ready to install"
      : compatibility.health === "action_required"
        ? "Compatibility setup needed"
        : "Runtime check needed";
  elements.compatibilityAndroid.textContent = [
    compatibility.androidVersion && `Android ${compatibility.androidVersion}`,
    compatibility.primaryAbi && compatibility.primaryAbi,
  ].filter(Boolean).join(" · ") || "Android ABI unknown";
  elements.compatibilityAbis.textContent = compatibility.abiList.length
    ? `ABIs ${compatibility.abiList.join(" · ")}`
    : "Supported ABIs unknown";
  elements.compatibilityBridge.textContent =
    compatibility.armTranslation === true
      ? compatibility.nativeBridge || "ARM application support available"
      : compatibility.armTranslation === false
        ? "ARM translation missing"
        : "ARM translation status unknown";
  elements.compatibilityStore.textContent =
    compatibility.playStore === true
      ? "Google Play available"
      : compatibility.playStore === false
        ? "Google Play missing"
        : "Google Play status unknown";
  elements.compatibilityNext.textContent =
    compatibility.health === "ready"
      ? "Install games in Google Play"
      : compatibility.armTranslation === true
        ? "Start Waydroid and refresh"
        : compatibility.armTranslation === false
          ? "Enable libndk or libhoudini"
          : "Start Waydroid to verify";
  elements.compatibilitySetupButton.textContent = `${compatibility.setup.label} ↗`;
  elements.compatibilitySetupButton.title = compatibility.setup.detail;

  elements.compatibilityGames.replaceChildren(
    ...compatibility.games.map((game) => {
      const item = document.createElement("div");
      item.className = `compatibility-game state-${game.state}`;
      const title = document.createElement("strong");
      title.textContent = game.name;
      const state = document.createElement("span");
      state.textContent = {
        installed: "Installed",
        ready_to_install: "Store ready",
        arm_translation_needed: "ARM setup",
        store_missing: "GAPPS needed",
        compatibility_unknown: "Runtime check",
        unknown: "Pending",
      }[game.state] || game.state;
      const detail = document.createElement("small");
      detail.textContent = game.detail;
      item.append(title, state, detail);
      return item;
    }),
  );

  elements.compatibilityFindings.replaceChildren(
    ...compatibility.findings.map((finding) => {
      const item = document.createElement("span");
      item.className = `finding compatibility-${finding.severity}`;
      item.textContent = finding.message;
      item.title = finding.code;
      return item;
    }),
  );
}

function renderStorage(storage = {}) {
  const available = storage.availableBytes == null ? Number.NaN : Number(storage.availableBytes);
  const ratio = storage.usedRatio == null ? Number.NaN : Number(storage.usedRatio);
  elements.storageAvailable.textContent = Number.isFinite(available)
    ? `${formatGib(available)} free`
    : "Storage unavailable";
  elements.storageAvailable.className = `storage-${storage.health || "unknown"}`;
  elements.storageAvailable.title = storage.path || "";
  elements.storageDetail.textContent =
    storage.message || "Waydroid game storage could not be inspected.";
  const usedPercent = Number.isFinite(ratio)
    ? Math.max(0, Math.min(100, ratio * 100))
    : 0;
  elements.storageFill.style.width = `${usedPercent.toFixed(1)}%`;
  elements.storageFill.className = `storage-${storage.health || "unknown"}`;
}

function formatGib(bytes) {
  const gib = bytes / (1024 ** 3);
  return `${gib >= 100 ? gib.toFixed(0) : gib.toFixed(1)} GiB`;
}

function renderDevice(device, dot, select, path, saved) {
  dot.classList.toggle("ready", device.ready);
  const devices = device.devices || [];
  const selected = devices.some((candidate) => candidate.path === saved)
    ? saved
    : device.value;
  select.replaceChildren(...devices.map((candidate) => {
    const option = document.createElement("option");
    option.value = candidate.path;
    option.textContent = `${candidate.name}${candidate.preferred ? " · AUTO" : ""}`;
    option.title = candidate.path;
    return option;
  }));
  if (!devices.length) {
    const option = document.createElement("option");
    option.textContent = "No compatible device";
    select.append(option);
  }
  select.disabled = !device.ready;
  if (selected) select.value = selected;
  updateDevicePath(select, path, device.error);
}

function updateDevicePath(select, path, error) {
  const value = select.disabled ? "" : select.value;
  path.textContent = value ? compactPath(value) : error || "No compatible evdev device";
  path.title = value || error || "";
}

function compactPath(path) {
  const marker = "/dev/input/by-id/";
  return path.startsWith(marker) ? path.slice(marker.length) : path;
}

function renderLibrary() {
  elements.gameGrid.replaceChildren(
    ...hubState.games.map((game, index) => {
      const card = document.createElement("article");
      card.className = `game-card kind-${game.kind}${game.id === selectedId ? " selected" : ""}`;
      card.dataset.mark = marks[game.kind] || "W";
      card.tabIndex = 0;
      card.setAttribute("role", "button");
      card.setAttribute("aria-label", `Select ${game.name}`);

      const top = document.createElement("div");
      top.className = "card-top";
      const cardId = document.createElement("span");
      cardId.className = "card-id";
      cardId.textContent = `GAME / ${String(index + 1).padStart(2, "0")}`;
      const installState = document.createElement("span");
      const compatibility = compatibilityFor(game);
      const calibrationNeeded = game.installed === true && !game.calibration?.ready;
      installState.className = `install-state${game.installed && !calibrationNeeded ? " installed" : ""}${
        compatibility?.state === "arm_translation_needed" || calibrationNeeded ? " attention" : ""
      }`;
      installState.textContent = game.installed === true
        ? game.calibration?.ready ? "Map prepared" : "Calibrate controls"
        : compatibility?.state === "arm_translation_needed"
          ? "ARM setup needed"
          : game.installed === false
            ? "Not installed"
            : "Status offline";
      installState.title = game.installed === true && game.calibration?.detail
        ? game.calibration.detail
        : "";
      top.append(cardId, installState);

      const bottom = document.createElement("div");
      bottom.className = "card-bottom";
      const title = document.createElement("h3");
      title.textContent = game.name;
      const description = document.createElement("p");
      description.textContent = game.description;
      const controls = document.createElement("div");
      controls.className = "card-controls";
      controls.append(
        label(`${game.controls.taps} taps`),
        label(`${game.controls.holds} holds`),
        label(`${game.controls.joysticks} sticks`),
        label(game.controls.mouseAim ? "mouse aim" : "no mouse aim"),
      );
      bottom.append(title, description, controls);
      card.append(top, bottom);
      card.addEventListener("click", () => selectGame(game.id));
      card.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          selectGame(game.id);
        }
      });
      return card;
    }),
  );
}

function label(text) {
  const span = document.createElement("span");
  span.textContent = text;
  return span;
}

function selectedGame() {
  return hubState?.games.find((game) => game.id === selectedId) || null;
}

function compatibilityFor(game) {
  return hubState?.system.compatibility.games.find(
    (candidate) => candidate.package === game.package,
  ) || null;
}

function editorActionFor(game) {
  return game.installed === true && !game.calibration?.ready ? "calibrate" : "edit";
}

function primaryActionFor(game) {
  const compatibility = compatibilityFor(game);
  if (
    game.installed !== true
    && (
      hubState.system.compatibility.armTranslation === false
      || hubState.system.compatibility.playStore === false
    )
  ) {
    return "compatibility";
  }
  if (!hubState.system.waydroid.running && game.installed !== true) {
    return "runtime";
  }
  if (game.installed === false) {
    if (["arm_translation_needed", "store_missing", "compatibility_unknown"].includes(compatibility?.state)) {
      return "compatibility";
    }
    return "store";
  }
  if (game.installed === true && !hubState.system.bridgeHelper?.ready) {
    return "helper";
  }
  return "launch";
}

function compatibilityActionDetail(game) {
  const compatibility = compatibilityFor(game);
  if (compatibility?.state && compatibility.state !== "unknown") {
    return compatibility.detail;
  }
  return hubState.system.compatibility.findings.find((finding) => finding.severity === "action")?.message
    || "Complete Android compatibility setup, then refresh the Hub.";
}

function selectGame(id) {
  selectedId = id;
  renderLibrary();
  renderHero();
  document.querySelector("#hero").scrollIntoView({ behavior: "smooth", block: "start" });
}

function renderHero() {
  const game = selectedGame();
  if (!game) {
    elements.heroName.textContent = "No valid profiles";
    elements.heroDescription.textContent = "Import a Profile V2 JSON file to start building your library.";
    elements.launchButton.disabled = true;
    elements.editButton.disabled = true;
    elements.controlsEditButton.disabled = true;
    elements.heroSessionState.hidden = true;
    elements.sessionReport.hidden = true;
    return;
  }

  const index = hubState.games.findIndex((item) => item.id === game.id);
  elements.hero.className = `hero game-${game.kind}`;
  elements.heroName.textContent = game.name;
  elements.heroDescription.textContent = game.description;
  elements.heroIndex.textContent = `${String(index + 1).padStart(2, "0")} / ${String(hubState.games.length).padStart(2, "0")}`;
  elements.heroInstallState.textContent =
    game.installed === true ? "Package installed" : game.installed === false ? "Install from Play Store" : "Package status offline";
  elements.heroBindingCount.textContent = `${game.bindings} controls`;
  elements.heroControlsState.textContent = game.calibration?.ready
    ? "Calibration reference saved"
    : game.calibration?.state === "invalid"
      ? "Calibration needs repair"
      : "Control map not calibrated";
  elements.heroControlsState.className = game.calibration?.ready
    ? "calibration-ready"
    : "calibration-needed";
  elements.heroControlsState.title = game.calibration?.detail || "";
  elements.heroMonogram.textContent = marks[game.kind] || "W";
  elements.gameCode.textContent = `${game.kind.toUpperCase().slice(0, 3)} / ${String(index + 1).padStart(2, "0")}`;
  elements.aimMode.textContent = game.controls.mouseAim ? "relative mouse" : game.controls.joysticks > 1 ? "dual joystick" : "profile-defined";
  const graphicsBlocker = hubState.system.graphics.findings.find(
    (finding) => finding.severity === "blocking",
  );
  const bridge = hubState.system.inputBridge;
  renderLastGameSession(game, bridge);
  const primaryAction = primaryActionFor(game);
  elements.launchButton.dataset.action = primaryAction;
  elements.launchButton.disabled = false;
  if (bridge?.busy) {
    elements.launchButton.dataset.action = bridge.canStop ? "stop" : "active";
    elements.launchIcon.textContent = bridge.canStop ? "■" : "●";
    elements.launchEyebrow.textContent = "SESSION ACTIVE";
    elements.launchLabel.textContent = bridge.canStop ? "Stop game" : "Game already running";
    elements.launchNote.textContent = bridge.canStop
      ? `${bridge.owner || "Wroid game session"}. Running in the background; Ctrl+Esc or this button stops it safely.`
      : `${bridge.owner || "Wroid owns the input bridge"}. Stop it with Ctrl+Esc before launching another game.`;
    elements.launchButton.disabled = !bridge.canStop;
  } else if (primaryAction === "runtime") {
    elements.launchIcon.textContent = "↻";
    elements.launchEyebrow.textContent = "RUNTIME OFFLINE";
    elements.launchLabel.textContent = "Start Waydroid & scan";
    elements.launchNote.textContent = "Starts Android without sudo, then detects Play Store and installed games.";
  } else if (primaryAction === "compatibility") {
    const needsRuntimeCheck = compatibilityFor(game)?.state === "compatibility_unknown";
    elements.launchIcon.textContent = "!";
    elements.launchEyebrow.textContent = needsRuntimeCheck ? "RUNTIME CHECK" : "REQUIRED STEP";
    elements.launchLabel.textContent = needsRuntimeCheck
      ? "Review runtime compatibility"
      : "Complete compatibility setup";
    elements.launchNote.textContent = compatibilityActionDetail(game);
  } else if (primaryAction === "store") {
    elements.launchIcon.textContent = "↗";
    elements.launchEyebrow.textContent = "INSTALL PACKAGE";
    elements.launchLabel.textContent = "Open Play Store";
    elements.launchNote.textContent = "Install the game and refresh the library when it is ready.";
  } else if (primaryAction === "helper") {
    elements.launchIcon.textContent = "◇";
    elements.launchEyebrow.textContent = "ONE-TIME SECURITY SETUP";
    elements.launchLabel.textContent = "Install bridge helper";
    const helperDetail =
      hubState.system.bridgeHelper?.detail || "Install the minimal root-owned helper before production play.";
    elements.launchNote.textContent =
      `${helperDetail}. Uses one desktop authorization dialog; no game launch password.`;
  } else if (graphicsBlocker) {
    elements.launchButton.dataset.action = "blocked";
    elements.launchIcon.textContent = "×";
    elements.launchEyebrow.textContent = "PRECHECK FAILED";
    elements.launchLabel.textContent = "Performance blocked";
    elements.launchNote.textContent = graphicsBlocker.message;
    elements.launchButton.disabled = true;
  } else {
    elements.launchIcon.textContent = "▶";
    elements.launchEyebrow.textContent = game.installed === true ? "START SESSION" : "START + VERIFY";
    elements.launchLabel.textContent = game.installed === true ? "Launch game" : "Launch and verify";
    elements.launchNote.textContent = game.installed === true
      ? "Gameplay stays unprivileged; the verified helper manages only the temporary input bridge."
      : "Waydroid is stopped; Wroid will start it and verify the package before gameplay.";
  }
  const editorAction = editorActionFor(game);
  elements.editButton.disabled = false;
  elements.controlsEditButton.disabled = false;
  elements.editButton.dataset.action = editorAction;
  elements.editButton.innerHTML = editorAction === "calibrate"
    ? '<span class="crosshair">◉</span> Open & calibrate'
    : '<span class="crosshair">⌖</span> Edit controls';
  elements.controlsEditButton.dataset.action = editorAction;
  elements.controlsEditButton.innerHTML = editorAction === "calibrate"
    ? "◉ Open game & calibrate"
    : "⌖ Edit selected profile";
}

function renderLastGameSession(game, bridge) {
  const session = hubState.system.lastGameSession;
  const matchesGame = session
    && session.state !== "unavailable"
    && (session.packageName === game.package || session.profilePath === game.path);
  const visible = Boolean(matchesGame && !bridge?.busy);
  const performance = session?.performance;
  const hasPerformance = Boolean(performance);

  elements.heroSessionState.hidden = !visible;
  elements.sessionReport.hidden = !visible || (session.state !== "failed" && !hasPerformance);
  if (!visible) return;

  const labels = {
    clean: "Last session clean",
    stopped: "Last session stopped",
    failed: "Last session failed",
  };
  const label = labels[session.state] || "Last session recorded";
  const inputP95 = performance?.readerToInject;
  const inputSummary = inputP95?.samples > 0
    ? ` · ${formatLatencyMillis(inputP95.p95Micros)} p95`
    : "";
  elements.heroSessionState.textContent = `${label}${inputSummary}`;
  elements.heroSessionState.className = `session-${session.state}`;
  const overBudget = inputP95?.samples > 0 && inputP95.p95Micros > 5_000;
  elements.sessionReport.className = [
    "session-report",
    `session-${session.state}`,
    overBudget ? "performance-warning" : "",
  ].filter(Boolean).join(" ");
  elements.sessionReport.open = session.state === "failed" || hasPerformance;
  elements.sessionReportTitle.textContent =
    session.state === "failed" ? label : "Session performance";
  elements.sessionMetrics.hidden = !hasPerformance;
  elements.sessionReportDetail.hidden = session.state !== "failed";
  elements.sessionReportDetail.textContent =
    session.detail || "No diagnostic detail was recorded.";

  if (hasPerformance) {
    const kernel = performance.kernelToInject;
    elements.sessionInputP95.textContent = inputP95?.samples > 0
      ? formatLatency(inputP95.p95Micros)
      : "—";
    elements.sessionInputSamples.textContent = formatSamples(inputP95?.samples);
    elements.sessionKernelP95.textContent = kernel?.samples > 0
      ? formatLatency(kernel.p95Micros)
      : "—";
    elements.sessionKernelSamples.textContent = formatSamples(kernel?.samples);
    elements.sessionTouchFrames.textContent =
      Number(performance.framesSubmitted || 0).toLocaleString();
    elements.sessionPeakContacts.textContent =
      Number(performance.peakSimultaneousContacts || 0).toLocaleString();
  }

  const finished = new Date(Number(session.finishedUnixMillis));
  const validTime = Number.isFinite(finished.getTime());
  elements.sessionReportTime.textContent = validTime
    ? finished.toLocaleString([], { dateStyle: "medium", timeStyle: "short" })
    : "";
  elements.sessionReportTime.dateTime = validTime ? finished.toISOString() : "";
}

function formatLatency(micros) {
  const value = Number(micros);
  if (!Number.isFinite(value) || value < 0) return "—";
  return value < 1_000
    ? `${Math.round(value)} µs`
    : `${(value / 1_000).toFixed(value < 10_000 ? 2 : 1)} ms`;
}

function formatLatencyMillis(micros) {
  const value = Number(micros);
  return Number.isFinite(value) && value >= 0
    ? `${(value / 1_000).toFixed(2)} ms`
    : "—";
}

function formatSamples(samples) {
  const value = Number(samples);
  return Number.isFinite(value) && value > 0
    ? `${value.toLocaleString()} SAMPLES`
    : "NO SAMPLES";
}

function renderPreset() {
  for (const button of elements.presetSwitch.querySelectorAll("button")) {
    button.setAttribute("aria-checked", String(button.dataset.resolution === resolution));
  }
}

function renderGameMode() {
  elements.gameModeToggle.setAttribute("aria-checked", String(gameModeEnabled));
  elements.gameModeState.textContent = gameModeEnabled ? "AUTO" : "OFF";
}

function savePreferences(patch) {
  preferenceWrite = preferenceWrite
    .then(async () => {
      const result = await request("/api/preferences", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(patch),
      });
      hubState.preferences = result.preferences;
      hubState.preferencesError = null;
    })
    .catch((error) => toast(`Could not save preferences: ${error.message}`, true));
}

function refreshAfterExternalAction() {
  if (busy || document.visibilityState === "hidden") return;
  if (Date.now() - lastStateLoadAt < focusRefreshMinimumMs) return;
  loadState();
}

async function performAction(action, extras = {}) {
  if (busy) return;
  busy = true;
  document.body.classList.add("is-busy");
  try {
    const result = await request("/api/action", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ action, ...extras }),
    });
    toast(result.message);
    if (action === "store" || action === "show-waydroid") {
      await loadState();
    }
    if (["launch", "stop", "calibrate", "input-test", "graphics-setup", "compatibility-setup"].includes(action)) {
      window.setTimeout(() => loadState(), 1000);
    }
    if (action === "helper-setup") {
      for (const delay of [600, 1800, 4500, 9000, 16000]) {
        window.setTimeout(() => loadState(), delay);
      }
    }
  } catch (error) {
    toast(error.message, true);
    if (action === "launch") {
      window.setTimeout(() => loadState(), 250);
    }
  } finally {
    busy = false;
    document.body.classList.remove("is-busy");
  }
}

function editSelected() {
  const game = selectedGame();
  if (game) performAction(editorActionFor(game), { id: game.id });
}

function launchSelected() {
  if (elements.launchButton.dataset.action === "stop") {
    performAction("stop");
    return;
  }
  const game = selectedGame();
  if (!game) return;
  const primaryAction = primaryActionFor(game);
  if (primaryAction === "compatibility") {
    elements.compatibilityCard.scrollIntoView({ behavior: "smooth", block: "center" });
    return;
  }
  if (primaryAction === "store") {
    performAction("store", { id: game.id });
    return;
  }
  if (primaryAction === "helper") {
    performAction("helper-setup");
    return;
  }
  if (primaryAction === "runtime") {
    performAction("show-waydroid");
    return;
  }
  const [width, height] = resolution.split("x").map(Number);
  performAction("launch", {
    id: game.id,
    width,
    height,
    keyboard: elements.keyboardSelect.disabled ? undefined : elements.keyboardSelect.value,
    mouse: elements.mouseSelect.disabled ? undefined : elements.mouseSelect.value,
    gameMode: gameModeEnabled,
  });
}

function runInputSelfTest() {
  const game = selectedGame();
  if (!game || elements.inputSelfTestButton.disabled) return;
  const [width, height] = resolution.split("x").map(Number);
  performAction("input-test", {
    id: game.id,
    width,
    height,
    keyboard: elements.keyboardSelect.disabled ? undefined : elements.keyboardSelect.value,
    mouse: elements.mouseSelect.disabled ? undefined : elements.mouseSelect.value,
  });
}

function formatPackageBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  const kib = 1024;
  const mib = kib * 1024;
  const gib = mib * 1024;
  if (bytes >= gib) return `${(bytes / gib).toFixed(2)} GiB`;
  if (bytes >= mib) return `${(bytes / mib).toFixed(1)} MiB`;
  return `${Math.max(bytes / kib, 0.1).toFixed(1)} KiB`;
}

function packageStateLabel(state) {
  return {
    uploading: "RECEIVING",
    inspected: "INSPECTED",
    queued: "QUEUED",
    installing: "INSTALLING",
    installed: "INSTALLED",
    failed: "REJECTED",
  }[state] || "STANDBY";
}

function packageCompatibilityLabel(state) {
  return {
    universal: "UNIVERSAL",
    native: "NATIVE",
    native_translation: "ARM BRIDGE",
    unknown: "UNCONFIRMED",
    arm_translation_missing: "BRIDGE MISSING",
    incompatible: "INCOMPATIBLE",
  }[state] || "SCANNING";
}

function renderPackageIntake() {
  const intake = packageIntake;
  elements.packageIntake.hidden = !intake;
  if (!intake) {
    elements.sideloadButton.disabled = false;
    return;
  }

  const state = intake.state || "uploading";
  const progress = Math.max(0, Math.min(100, Number(intake.progress) || 0));
  const artifact = intake.artifact || {};
  const compatibility = intake.compatibility || {};
  const working = ["uploading", "queued", "installing"].includes(state);
  elements.packageIntake.dataset.state = state;
  elements.packageTitle.textContent = state === "failed" ? "Package rejected" : "Android package";
  elements.packageFileName.textContent = intake.fileName || "Local APK";
  elements.packageState.textContent = packageStateLabel(state);
  elements.packageProgressFill.style.width = `${working && state !== "uploading" ? 100 : progress}%`;
  elements.packageProgressLabel.textContent = state === "installing"
    ? "WORKER"
    : state === "queued"
      ? "WAIT"
      : `${Math.round(progress)}%`;
  if (state === "queued" || state === "installing") {
    elements.packageProgress.removeAttribute("aria-valuenow");
    elements.packageProgress.setAttribute("aria-valuetext", packageStateLabel(state));
  } else {
    elements.packageProgress.setAttribute("aria-valuenow", String(Math.round(progress)));
    elements.packageProgress.setAttribute("aria-valuetext", `${Math.round(progress)} percent`);
  }
  elements.packageFormat.textContent = artifact.formatLabel || "SCANNING";
  elements.packageSize.textContent = formatPackageBytes(artifact.fileSize ?? intake.fileSize);
  elements.packageAbis.textContent = artifact.nativeAbis?.length
    ? artifact.nativeAbis.join(" / ").toUpperCase()
    : artifact.format
      ? "UNIVERSAL"
      : "SCANNING";
  elements.packageCompatibility.textContent = packageCompatibilityLabel(compatibility.state);
  elements.packageDetail.textContent = intake.detail || "Inspecting archive structure and native code…";
  elements.packageInstallButton.disabled = state !== "inspected";
  elements.packageDiscardButton.disabled = state === "queued" || state === "installing";
  elements.packageDiscardButton.textContent = ["failed", "installed"].includes(state) ? "Clear" : "Discard";
  elements.sideloadButton.disabled = !["failed", "installed"].includes(state);
}

function clearPackageTimer() {
  if (apkStatusTimer) window.clearTimeout(apkStatusTimer);
  apkStatusTimer = null;
}

function resetPackageIntake({ abortUpload = true } = {}) {
  clearPackageTimer();
  const upload = apkUploadRequest;
  apkUploadRequest = null;
  if (abortUpload && upload) upload.abort();
  packageIntake = null;
  elements.sideloadInput.value = "";
  renderPackageIntake();
}

function rejectPackageIntake(message, xhr) {
  if (xhr && apkUploadRequest !== xhr) return;
  apkUploadRequest = null;
  packageIntake = {
    ...(packageIntake || {}),
    state: "failed",
    progress: 0,
    detail: message,
  };
  renderPackageIntake();
  toast(message, true);
}

function uploadApk(file) {
  if (!file) return;
  if (file.size === 0) {
    rejectPackageIntake("The selected APK is empty");
    return;
  }
  if (file.size > maximumApkBytes) {
    rejectPackageIntake("APK exceeds the 4 GiB Hub upload limit");
    return;
  }

  clearPackageTimer();
  packageIntake = {
    state: "uploading",
    progress: 0,
    fileName: file.name,
    fileSize: file.size,
    detail: "Streaming into private Wroid storage…",
  };
  renderPackageIntake();

  const xhr = new XMLHttpRequest();
  apkUploadRequest = xhr;
  xhr.open("POST", apiUrl("/api/apk/upload"));
  xhr.setRequestHeader("Content-Type", "application/vnd.android.package-archive");
  xhr.upload.addEventListener("progress", (event) => {
    if (apkUploadRequest !== xhr || !event.lengthComputable) return;
    packageIntake.progress = Math.min(99, (event.loaded / event.total) * 100);
    packageIntake.detail = `Receiving ${formatPackageBytes(event.loaded)} of ${formatPackageBytes(event.total)}`;
    renderPackageIntake();
  });
  xhr.addEventListener("load", () => {
    if (apkUploadRequest !== xhr) return;
    let body = {};
    try {
      body = JSON.parse(xhr.responseText || "{}");
    } catch (_) {
      rejectPackageIntake("Hub returned an invalid package inspection response", xhr);
      return;
    }
    if (xhr.status < 200 || xhr.status >= 300) {
      rejectPackageIntake(body.error || `Package inspection failed (${xhr.status})`, xhr);
      return;
    }
    apkUploadRequest = null;
    packageIntake = {
      ...packageIntake,
      ...body,
      state: "inspected",
      progress: 100,
      detail: `Static inspection passed · ${body.artifact?.archiveEntries || 0} archive entries`,
    };
    elements.sideloadInput.value = "";
    renderPackageIntake();
    toast("APK inspected and ready to install");
  });
  xhr.addEventListener("error", () => rejectPackageIntake("APK upload connection failed", xhr));
  xhr.addEventListener("abort", () => {
    if (apkUploadRequest === xhr) resetPackageIntake({ abortUpload: false });
  });
  xhr.send(file);
}

function scheduleApkStatus(ticket, delay = 500) {
  clearPackageTimer();
  apkStatusTimer = window.setTimeout(() => pollApkStatus(ticket), delay);
}

async function pollApkStatus(ticket) {
  if (!packageIntake || packageIntake.ticket !== ticket) return;
  try {
    const status = await request(`/api/apk/status?ticket=${encodeURIComponent(ticket)}`);
    if (!packageIntake || packageIntake.ticket !== ticket) return;
    packageIntake = {
      ...packageIntake,
      state: status.state,
      detail: status.detail,
      progress: 100,
    };
    renderPackageIntake();
    if (status.state === "installed") {
      toast("APK installed into Waydroid");
      await loadState();
      return;
    }
    if (status.state === "failed") {
      toast(status.detail || "APK installation failed", true);
      return;
    }
    scheduleApkStatus(ticket, 750);
  } catch (error) {
    if (!packageIntake || packageIntake.ticket !== ticket) return;
    packageIntake.detail = `Status link interrupted · ${error.message}`;
    renderPackageIntake();
    scheduleApkStatus(ticket, 1500);
  }
}

async function installInspectedApk() {
  if (!packageIntake?.ticket || packageIntake.state !== "inspected") return;
  const ticket = packageIntake.ticket;
  packageIntake.state = "queued";
  packageIntake.detail = "Starting isolated install worker…";
  renderPackageIntake();
  try {
    const result = await request("/api/apk/install", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ ticket }),
    });
    packageIntake.detail = result.message;
    renderPackageIntake();
    scheduleApkStatus(ticket);
  } catch (error) {
    packageIntake.state = "inspected";
    packageIntake.detail = error.message;
    renderPackageIntake();
    toast(error.message, true);
  }
}

async function discardPackageIntake() {
  if (!packageIntake) return;
  if (packageIntake.state === "uploading") {
    resetPackageIntake();
    toast("APK upload cancelled");
    return;
  }
  if (packageIntake.state !== "inspected" || !packageIntake.ticket) {
    resetPackageIntake();
    return;
  }
  const ticket = packageIntake.ticket;
  elements.packageDiscardButton.disabled = true;
  try {
    const result = await request("/api/apk/discard", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ ticket }),
    });
    resetPackageIntake({ abortUpload: false });
    toast(result.message);
  } catch (error) {
    elements.packageDiscardButton.disabled = false;
    toast(error.message, true);
  }
}

async function importProfile(file) {
  if (!file) return;
  try {
    const text = await file.text();
    JSON.parse(text);
    const result = await request("/api/import", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: text,
    });
    toast(result.message);
    selectedId = result.id;
    await loadState();
  } catch (error) {
    toast(error.message, true);
  } finally {
    elements.importInput.value = "";
  }
}

function toast(message, error = false) {
  const item = document.createElement("div");
  item.className = `toast${error ? " error" : ""}`;
  item.textContent = message;
  elements.toastStack.append(item);
  window.setTimeout(() => item.remove(), 4500);
}

elements.launchButton.addEventListener("click", launchSelected);
elements.editButton.addEventListener("click", editSelected);
elements.controlsEditButton.addEventListener("click", editSelected);
elements.refreshButton.addEventListener("click", () => loadState({ announce: true }));
elements.storeButton.addEventListener("click", () => {
  const game = selectedGame();
  performAction("store", game ? { id: game.id } : {});
});
elements.compatibilitySetupButton.addEventListener("click", () => performAction("compatibility-setup"));
elements.graphicsSetupButton.addEventListener("click", () => performAction("graphics-setup"));
elements.helperSetupButton.addEventListener("click", () => performAction("helper-setup"));
elements.showWaydroidButton.addEventListener("click", () => performAction("show-waydroid"));
elements.inputSelfTestButton.addEventListener("click", runInputSelfTest);
elements.keyboardSelect.addEventListener("change", () => {
  savePreferences({ keyboard: elements.keyboardSelect.value });
  updateDevicePath(elements.keyboardSelect, elements.keyboardPath);
});
elements.mouseSelect.addEventListener("change", () => {
  savePreferences({ mouse: elements.mouseSelect.value });
  updateDevicePath(elements.mouseSelect, elements.mousePath);
});
elements.importButton.addEventListener("click", () => elements.importInput.click());
elements.importInput.addEventListener("change", () => importProfile(elements.importInput.files[0]));
elements.sideloadButton.addEventListener("click", () => {
  if (packageIntake && ["failed", "installed"].includes(packageIntake.state)) {
    resetPackageIntake({ abortUpload: false });
  }
  elements.sideloadInput.click();
});
elements.sideloadInput.addEventListener("change", () => uploadApk(elements.sideloadInput.files[0]));
elements.packageInstallButton.addEventListener("click", installInspectedApk);
elements.packageDiscardButton.addEventListener("click", discardPackageIntake);

elements.closeButton.addEventListener("click", async () => {
  try {
    await request("/api/close", { method: "POST" });
  } finally {
    document.body.innerHTML = '<main class="closed"><h1>Wroid Hub closed.</h1><p>You can close this tab.</p></main>';
  }
});

elements.presetSwitch.addEventListener("click", (event) => {
  const button = event.target.closest("button[data-resolution]");
  if (!button) return;
  resolution = button.dataset.resolution;
  savePreferences({ resolution });
  renderPreset();
});

elements.gameModeToggle.addEventListener("click", () => {
  gameModeEnabled = !gameModeEnabled;
  savePreferences({ gameMode: gameModeEnabled });
  renderGameMode();
});

window.addEventListener("focus", refreshAfterExternalAction);
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") refreshAfterExternalAction();
});

for (const navItem of document.querySelectorAll("[data-scroll]")) {
  navItem.addEventListener("click", () => {
    document.querySelector(`#${navItem.dataset.scroll}`)?.scrollIntoView({ behavior: "smooth" });
  });
}

loadState();
