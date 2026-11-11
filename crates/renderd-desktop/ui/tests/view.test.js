import assert from "node:assert/strict";
import { test } from "node:test";

import { describe, stageLabels } from "../scripts/view.js";

const METRICS = { fps: null, bitrate_kbps: null, rtt_ms: null, send_queue_kb: null, decode_ms: null, lost_frames: null };

/** A status with everything empty except what the test sets. */
function status(role, phase, extra = {}) {
  return { role, phase, address: null, peer: null, stream: null, metrics: METRICS, problem: null, ...extra };
}

test("a stopped host offers to start sharing", () => {
  const view = describe(status("host", "stopped"));
  assert.equal(view.title, "Share this Mac");
  assert.deepEqual(view.primary, { label: "Start sharing", action: "start", quiet: false });
  assert.equal(view.notice, undefined);
});

test("a waiting host shows the address without the default port", () => {
  const view = describe(status("host", "waiting", { address: "192.168.1.20:4433" }));
  assert.equal(view.address, "192.168.1.20");
  assert.equal(view.primary.action, "stop");
});

test("a live host shows three numbers and describes the stream", () => {
  const view = describe(
    status("host", "streaming", {
      peer: "192.168.1.34:53561",
      stream: { codec: "hevc", width: 2560, height: 1440, fps: 60 },
      metrics: { ...METRICS, fps: 59.7, bitrate_kbps: 8240, rtt_ms: 1.42 },
    }),
  );
  assert.equal(view.title, "Sharing to 192.168.1.34");
  assert.equal(view.lede, "2560×1440 at 60 fps, HEVC.");
  assert.deepEqual(
    view.stats.map((stat) => stat.value),
    ["60", "8.2", "1.4"],
  );
});

test("a missing screen recording permission adds a notice with an action", () => {
  const view = describe(status("host", "stopped"), "denied");
  assert.equal(view.notice.run, "grant");
  assert.equal(view.notice.action, "Allow");
});

test("the permission notice is not shown to a viewer", () => {
  const view = describe(status("viewer", "stopped"), "denied");
  assert.equal(view.notice, undefined);
});

test("a failed session explains itself and offers a retry", () => {
  const problem = { kind: "port_in_use", title: "Another app is already using this port", detail: "os error 48" };
  const view = describe(status("host", "failed", { problem }));
  assert.equal(view.title, problem.title);
  assert.match(view.lede, /another port/);
  assert.equal(view.primary.label, "Try again");
  assert.equal(view.notice, undefined);
});

test("an unknown failure falls back to the raw detail", () => {
  const problem = { kind: "other", title: "Something went wrong", detail: "decoder exploded" };
  assert.equal(describe(status("viewer", "failed", { problem })).lede, "decoder exploded");
});

test("an extend failure during a session tells the user to mirror", () => {
  const problem = { kind: "extend_failed", title: "macOS did not bring up the extra display", detail: "" };
  const view = describe(status("host", "waiting", { problem }));
  assert.match(view.notice.text, /Mirror/);
});

test("the viewer names this computer by platform", () => {
  assert.equal(describe(status("viewer", "stopped"), "unneeded", "windows").title, "Use this PC as a display");
  assert.equal(describe(status("viewer", "stopped"), "unneeded", "macos").title, "Use this Mac as a display");
  assert.equal(describe(status("viewer", "stopped")).connectForm, true);
});

test("the viewer says whether it is searching or dialling", () => {
  assert.equal(describe(status("viewer", "connecting")).eyebrow, "Looking for your Mac");
  const dialling = describe(status("viewer", "connecting", { address: "10.0.0.7:5000" }));
  assert.equal(dialling.title, "Connecting to 10.0.0.7:5000");
});
