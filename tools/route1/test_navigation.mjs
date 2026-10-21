import assert from "node:assert/strict";
import vm from "node:vm";
import { navigate } from "./chrome.mjs";

let evaluations = 0;
const unusedPage = { evaluate() { evaluations++; } };
for (const path of ["https://example.com", "//example.com", "/\\example.com", "settings"])
  await assert.rejects(navigate(unusedPage, path), TypeError);
await assert.rejects(navigate(unusedPage, "/pull-requests", { timeoutMs: -1 }), RangeError);
assert.equal(evaluations, 0);

const uninitializedPage = {
  evaluate: expression => new vm.Script(expression).runInNewContext({ window: {}, performance, setTimeout }),
};
await assert.rejects(
  navigate(uninitializedPage, "/pull-requests", { timeoutMs: 0 }),
  /Original route handler did not become ready/,
);
console.log("navigation: local routes and missing-bootstrap deadline passed");
