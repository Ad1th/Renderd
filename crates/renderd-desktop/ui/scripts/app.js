// Wires the page to the backend: keeps the latest state and redraws from it.

import * as api from "./api.js";
import * as spark from "./spark.js";
import { size } from "./format.js";
import { describe, stageLabels } from "./view.js";

const $ = (id) => document.getElementById(id);

const state = {
  info: { version: "", platform: "macos", roles: ["host"] },
  settings: null,
  status: null,
  access: "unneeded",
  bitrates: [],
  typedHostError: "",
};

/** Whether a session is in progress, so options are locked. */
const running = () => !["stopped", "failed"].includes(state.status.phase);

function renderStage() {
  const { status, info } = state;
  const stage = $("stage");
  stage.dataset.phase = status.phase;
  stage.dataset.role = status.role;

  const labels = stageLabels(status.role, info.platform);
  $("near-label").textContent = labels.near;
  $("far-label").textContent = labels.far;

  // The picture lands on the other computer when sharing, on this one when viewing.
  const text = status.phase === "streaming" ? size(status.stream) : "";
  $("near-size").textContent = status.role === "viewer" ? text : "";
  $("far-size").textContent = status.role === "host" ? text : "";
}

function renderStats(stats) {
  $("stats").hidden = !stats;
  // An SVG element has no `hidden` property, only the attribute.
  $("spark").toggleAttribute("hidden", !stats);
  if (!stats) return;
  ["a", "b", "c"].forEach((slot, index) => {
    const stat = stats[index];
    $(`stat-${slot}-label`).textContent = stat.label;
    const value = $(`stat-${slot}`);
    value.textContent = stat.value;
    if (stat.unit) {
      const unit = document.createElement("small");
      unit.textContent = stat.unit;
      value.append(unit);
    }
  });
  $("spark").querySelector("polyline").setAttribute("points", spark.points(state.bitrates));
}

function renderPanel() {
  const view = describe(state.status, state.access, state.info.platform);
  document.body.dataset.phase = state.status.phase;

  $("eyebrow").textContent = view.eyebrow;
  $("title").textContent = view.title;
  $("lede").textContent = view.lede;
  $("lede").hidden = !view.lede;

  $("address-row").hidden = !view.address;
  $("address").textContent = view.address ?? "";

  renderStats(view.stats);

  $("notice").hidden = !view.notice;
  if (view.notice) {
    $("notice-text").textContent = view.notice.text;
    $("notice-action").textContent = view.notice.action ?? "";
    $("notice-action").hidden = !view.notice.action;
    $("notice-action").dataset.run = view.notice.run ?? "";
  }

  const primary = $("primary");
  const typed = view.connectForm && $("host-input").value.trim() !== "";
  primary.textContent = typed ? "Connect" : view.primary.label;
  primary.dataset.action = view.primary.action ?? "";
  primary.disabled = !view.primary.action;
  primary.classList.toggle("quiet", view.primary.quiet);

  $("connect-form").hidden = !view.connectForm;
  $("host-error").textContent = state.typedHostError;
  $("host-error").hidden = !state.typedHostError;
  if (view.connectForm) renderRecent();
}

function renderRecent() {
  const recent = $("recent");
  recent.replaceChildren(
    ...state.settings.viewer.recent_hosts.slice(0, 3).map((address) => {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "chip";
      chip.textContent = address;
      chip.addEventListener("click", () => {
        $("host-input").value = address;
        connect(address);
      });
      return chip;
    }),
  );
}

/** Reads `host.mode` style paths out of the settings. */
function readSetting(path) {
  return path.split(".").reduce((value, key) => value?.[key], state.settings);
}

function renderOptions() {
  const role = state.settings.role;
  $("options").dataset.locked = String(running());

  document.querySelectorAll(".segmented[data-setting]").forEach((group) => {
    if (group.dataset.role) group.hidden = group.dataset.role !== role;
    const current = String(readSetting(group.dataset.setting));
    group.querySelectorAll("button").forEach((button) => {
      button.setAttribute("aria-checked", String(button.dataset.value === current));
    });
  });

  document.querySelectorAll(".setting[data-role]").forEach((section) => {
    section.hidden = section.dataset.role !== role;
  });
  $("role-setting").hidden = state.info.roles.length < 2;
  $("port-input").value = state.settings.host.port;
  $("display-input").value = state.settings.host.display_id;
  $("port-input").disabled = running();
  $("display-input").disabled = running();
}

function render() {
  if (!state.status || !state.settings) return;
  renderStage();
  renderPanel();
  renderOptions();
}

/** Takes a new status from the backend. */
function onStatus(status) {
  const wasStreaming = state.status?.phase === "streaming";
  state.status = status;
  if (status.phase === "streaming") {
    state.bitrates = spark.push(state.bitrates, status.metrics.bitrate_kbps);
  } else if (wasStreaming) {
    state.bitrates = [];
  }
  render();
}

function appendLog(line) {
  const logs = $("logs");
  const pinned = logs.scrollTop + logs.clientHeight >= logs.scrollHeight - 4;
  const row = document.createElement("span");
  row.className = line.level;
  row.textContent = `${line.message}\n`;
  logs.append(row);
  while (logs.childElementCount > 400) logs.firstElementChild.remove();
  if (pinned) logs.scrollTop = logs.scrollHeight;
}

/** Writes a changed copy of the settings and redraws with what was saved. */
async function updateSettings(change) {
  const next = structuredClone(state.settings);
  change(next);
  try {
    state.settings = await api.saveSettings(next);
    state.status = await api.getStatus();
  } catch (error) {
    console.warn(error);
  }
  render();
}

function writeSetting(path, raw) {
  const keys = path.split(".");
  const value = raw === "true" ? true : raw === "false" ? false : raw;
  return updateSettings((settings) => {
    const last = keys.pop();
    const target = keys.reduce((object, key) => object[key], settings);
    target[last] = value;
  });
}

async function connect(host) {
  state.typedHostError = "";
  try {
    onStatus(await api.start(host));
    state.settings = await api.getSettings();
  } catch (error) {
    state.typedHostError = String(error);
  }
  render();
}

async function refreshAccess() {
  state.access = await api.screenAccess();
  render();
}

async function grantAccess() {
  state.access = await api.requestScreenAccess();
  if (state.access === "denied") await api.openScreenSettings();
  render();
}

function bind() {
  // One button does both: an empty field means "find it", a filled one "dial it".
  const go = () => {
    const typed = $("connect-form").hidden ? "" : $("host-input").value.trim();
    connect(typed || null);
  };
  $("primary").addEventListener("click", (event) => {
    const action = event.currentTarget.dataset.action;
    if (action === "start") go();
    if (action === "stop") api.stop();
  });
  $("connect-form").addEventListener("submit", (event) => {
    event.preventDefault();
    go();
  });
  $("host-input").addEventListener("input", () => {
    state.typedHostError = "";
    render();
  });

  $("copy-address").addEventListener("click", (event) => copy($("address").textContent, event.currentTarget));
  $("copy-logs").addEventListener("click", (event) => copy($("logs").textContent, event.currentTarget));
  $("notice-action").addEventListener("click", (event) => {
    if (event.currentTarget.dataset.run === "grant") grantAccess();
  });

  document.querySelectorAll(".segmented[data-setting]").forEach((group) => {
    group.addEventListener("click", (event) => {
      const button = event.target.closest("button");
      if (!button || running()) return;
      writeSetting(group.dataset.setting, button.dataset.value);
    });
  });

  $("port-input").addEventListener("change", (event) => {
    const port = Number.parseInt(event.target.value, 10);
    updateSettings((settings) => {
      settings.host.port = port >= 1 && port <= 65535 ? port : 4433;
    });
  });
  $("display-input").addEventListener("change", (event) => {
    const display = Number.parseInt(event.target.value, 10);
    updateSettings((settings) => {
      settings.host.display_id = display >= 0 ? display : 0;
    });
  });

  $("open-settings").addEventListener("click", () => toggleSheet(true));
  $("close-settings").addEventListener("click", () => toggleSheet(false));
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") toggleSheet(false);
  });

  // The permission can be granted in System Settings while the app is open.
  window.addEventListener("focus", refreshAccess);
}

function toggleSheet(open) {
  $("sheet").hidden = !open;
  if (open) {
    const logs = $("logs");
    logs.scrollTop = logs.scrollHeight;
    $("close-settings").focus();
  } else {
    $("open-settings").focus();
  }
}
