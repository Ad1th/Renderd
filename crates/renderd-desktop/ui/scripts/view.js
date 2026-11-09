// Decides what the window says for a given state. No DOM in here, so it can
// be tested with plain node.

import * as format from "./format.js";

/** Labels under the two screens in the picture. */
export function stageLabels(role, platform) {
  const here = platform === "macos" ? "This Mac" : "This PC";
  const there = role === "host" ? "Your PC" : "Your Mac";
  return { near: here, far: there };
}
