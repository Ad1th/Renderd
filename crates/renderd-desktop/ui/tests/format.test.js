import assert from "node:assert/strict";
import { test } from "node:test";

import * as format from "../scripts/format.js";

test("fps rounds to a whole number", () => {
  assert.equal(format.fps(59.6), "60");
  assert.equal(format.fps(null), "--");
});

test("bitrates show one decimal until they reach 100 Mbit/s", () => {
  assert.equal(format.mbps(8240), "8.2");
  assert.equal(format.mbps(400), "0.4");
  assert.equal(format.mbps(120000), "120");
  assert.equal(format.mbps(undefined), "--");
});

test("milliseconds keep a decimal only while small", () => {
  assert.equal(format.ms(0.66), "0.7");
  assert.equal(format.ms(9.94), "9.9");
  assert.equal(format.ms(23.4), "23");
  assert.equal(format.ms(NaN), "--");
});

test("picture size uses a multiplication sign", () => {
  assert.equal(format.size({ width: 2560, height: 1440 }), "2560×1440");
  assert.equal(format.size(null), "");
});

test("codec names are written the usual way", () => {
  assert.equal(format.codec("hevc"), "HEVC");
  assert.equal(format.codec("h264"), "H.264");
  assert.equal(format.codec("av1"), "AV1");
  assert.equal(format.codec(""), "");
});
