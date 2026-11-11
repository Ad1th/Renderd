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
