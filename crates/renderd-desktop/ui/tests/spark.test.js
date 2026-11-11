import assert from "node:assert/strict";
import { test } from "node:test";

import * as spark from "../scripts/spark.js";

test("a single sample draws nothing", () => {
  assert.equal(spark.points([]), "");
  assert.equal(spark.points([5000]), "");
});
