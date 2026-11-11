// Decides what the window says for a given state. No DOM in here, so it can
// be tested with plain node.

import * as format from "./format.js";

/** Labels under the two screens in the picture. */
export function stageLabels(role, platform) {
  const here = platform === "macos" ? "This Mac" : "This PC";
  const there = role === "host" ? "Your PC" : "Your Mac";
  return { near: here, far: there };
}

/** Words for a failure, by kind. */
function failure(status) {
  const problem = status.problem ?? { kind: "other", title: "Something went wrong", detail: "" };
  const hints = {
    screen_recording: "Allow Renderd under Screen Recording in System Settings, then try again.",
    port_in_use: "Pick another port in settings, or close the app that is using this one.",
    missing_binary: "Reinstall Renderd. The part that does the streaming could not be started.",
    host_unreachable: "Check that both computers are on the same network and that Renderd is sharing on the Mac.",
    crashed: "Try again. If it keeps happening, the activity log in settings says why.",
    other: problem.detail,
  };
  return {
    eyebrow: "Stopped",
    title: problem.title,
    lede: hints[problem.kind] ?? problem.detail,
    primary: { label: "Try again", action: "start", quiet: false },
  };
}
