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
