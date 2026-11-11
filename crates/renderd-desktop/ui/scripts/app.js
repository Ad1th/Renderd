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
