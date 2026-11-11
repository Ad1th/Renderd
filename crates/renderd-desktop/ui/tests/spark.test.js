import assert from "node:assert/strict";
import { test } from "node:test";

import * as spark from "../scripts/spark.js";

test("a single sample draws nothing", () => {
  assert.equal(spark.points([]), "");
  assert.equal(spark.points([5000]), "");
});

test("the line is stepped and ends at the right edge", () => {
  const pairs = spark
    .points([1000, 2000, 2000])
    .split(" ")
    .map((pair) => pair.split(",").map(Number));
  // Two samples after the first, two points each, plus the first.
  assert.equal(pairs.length, 5);
  assert.equal(pairs.at(-1)[0], 320);
  // A step: the x moves first while the y holds the previous value.
  assert.equal(pairs[1][1], pairs[0][1]);
  assert.equal(pairs[1][0], pairs[2][0]);
});
