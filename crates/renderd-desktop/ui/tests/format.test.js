import assert from "node:assert/strict";
import { test } from "node:test";

import * as format from "../scripts/format.js";

test("fps rounds to a whole number", () => {
  assert.equal(format.fps(59.6), "60");
  assert.equal(format.fps(null), "--");
});
