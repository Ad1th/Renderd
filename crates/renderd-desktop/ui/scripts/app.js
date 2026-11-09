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
