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
