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

function hostView(status) {
  const peer = format.hostOnly(status.peer);
  switch (status.phase) {
    case "stopped":
      return {
        eyebrow: "Ready",
        title: "Share this Mac",
        lede: "Your PC becomes a second display. No cables, no accounts, nothing leaves your network.",
        primary: { label: "Start sharing", action: "start", quiet: false },
      };
    case "starting":
      return {
        eyebrow: "Starting",
        title: "Getting ready",
        lede: "This takes a moment.",
        primary: { label: "Cancel", action: "stop", quiet: true },
      };
    case "waiting":
      return {
        eyebrow: "Waiting for your PC",
        title: "Open Renderd on the PC",
        lede: "It finds this Mac by itself. If it does not, type this address there.",
        address: format.shortAddress(status.address),
        primary: { label: "Stop sharing", action: "stop", quiet: true },
      };
    case "connecting":
      return {
        eyebrow: "Connecting",
        title: peer ? `Saying hello to ${peer}` : "Saying hello",
        lede: "Agreeing on a picture format.",
        primary: { label: "Stop sharing", action: "stop", quiet: true },
      };
    case "streaming":
      return {
        eyebrow: "Live",
        title: peer ? `Sharing to ${peer}` : "Sharing",
        lede: streamLine(status),
        stats: [
          { label: "FPS", value: format.fps(status.metrics.fps) },
          { label: "Mbps", value: format.mbps(status.metrics.bitrate_kbps) },
          { label: "Ping", value: format.ms(status.metrics.rtt_ms), unit: "ms" },
        ],
        primary: { label: "Stop sharing", action: "stop", quiet: true },
      };
    case "stopping":
      return {
        eyebrow: "Stopping",
        title: "Wrapping up",
        lede: "",
        primary: { label: "Stopping", action: null, quiet: true },
      };
    default:
      return failure(status);
  }
}

function viewerView(status, platform) {
  const here = platform === "macos" ? "Mac" : "PC";
  const host = format.shortAddress(status.address);
  const stop = { label: "Cancel", action: "stop", quiet: true };
  switch (status.phase) {
    case "stopped":
      return {
        eyebrow: "Ready",
        title: `Use this ${here} as a display`,
        lede: `Start sharing on your Mac first. This ${here} finds it on the network.`,
        primary: { label: "Find my Mac", action: "start", quiet: false },
        connectForm: true,
      };
    case "starting":
      return { eyebrow: "Starting", title: "Getting ready", lede: "This takes a moment.", primary: stop };
    case "connecting":
      return {
        eyebrow: host ? "Connecting" : "Looking for your Mac",
        title: host ? `Connecting to ${host}` : "Looking around the network",
        lede: "Not found after a few seconds? Cancel and type the address shown on the Mac.",
        primary: stop,
      };
    case "reconnecting":
      return {
        eyebrow: "Reconnecting",
        title: "Lost the Mac, trying again",
        lede: "The picture comes back by itself once the Mac is reachable.",
        primary: { label: "Disconnect", action: "stop", quiet: true },
      };
    case "streaming":
      return {
        eyebrow: "Live",
        title: host ? `Showing ${format.hostOnly(status.address)}` : "Showing your Mac",
        lede: streamLine(status),
        stats: [
          { label: "FPS", value: format.fps(status.metrics.fps) },
          { label: "Mbps", value: format.mbps(status.metrics.bitrate_kbps) },
          { label: "Decode", value: format.ms(status.metrics.decode_ms), unit: "ms" },
        ],
        primary: { label: "Disconnect", action: "stop", quiet: true },
      };
    case "stopping":
      return {
        eyebrow: "Stopping",
        title: "Wrapping up",
        lede: "",
        primary: { label: "Stopping", action: null, quiet: true },
      };
    default:
      return failure(status);
  }
}

/** One line describing the stream, like "2560x1440 at 60 fps, HEVC". */
function streamLine(status) {
  const stream = status.stream;
  if (!stream) return "";
  return `${format.size(stream)} at ${Math.round(stream.fps)} fps, ${format.codec(stream.codec)}.`;
}

/**
 * Everything the panel shows for `status`.
 *
 * `access` is the screen recording state on macOS: sharing cannot work
 * without it, so a notice is added while it is missing. `platform` only
 * changes what this computer is called.
 */
export function describe(status, access = "unneeded", platform = "windows") {
  const view = status.role === "host" ? hostView(status) : viewerView(status, platform);

  if (status.role === "host" && access === "denied" && status.phase !== "streaming") {
    view.notice = {
      text: "macOS has not allowed Renderd to record the screen yet.",
      action: "Allow",
      run: "grant",
    };
  } else if (status.problem && status.phase !== "failed" && status.phase !== "stopped") {
    // A problem that did not end the session, like a dial that will be retried.
    view.notice = { text: status.problem.title, action: null, run: null };
  }
  return view;
}
