import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";

const directory = fs.mkdtempSync(path.join(os.tmpdir(), "route1-resume-"));
const child = spawn(process.execPath, [new URL("./linux/mock-app-server.mjs", import.meta.url).pathname], {
  env: {
    ...process.env,
    MOCK_LOG: path.join(directory, "requests.log"),
    MOCK_FIXTURES: new URL("../../fixtures", import.meta.url).pathname,
    R1_ORACLE: new URL("./oracle.json", import.meta.url).pathname,
    MOCK_SIGNED_OUT: "",
  },
  stdio: ["pipe", "pipe", "inherit"],
});
const exited = once(child, "exit");
const pending = new Map();
const lines = createInterface({ input: child.stdout });
lines.on("line", line => {
  const message = JSON.parse(line);
  pending.get(message.id)?.(message);
});
let nextId = 0;
function call(method, params) {
  return new Promise((resolve, reject) => {
    const id = ++nextId;
    const timeout = setTimeout(() => reject(new Error(`No reply for ${method}`)), 5000);
    pending.set(id, message => {
      clearTimeout(timeout);
      pending.delete(id);
      resolve(message);
    });
    child.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
  });
}
try {
  const legacyId = "01a13de9-6fff-78e1-84fc-ebb3a2dc873f";
  const legacy = await call("thread/resume", { threadId: legacyId, excludeTurns: true });
  assert.equal(legacy.result.thread.id, legacyId);
  const freshId = "01a15c3f-2215-7233-89b6-ccf332f3756c";
  const fresh = await call("thread/resume", { threadId: freshId, model: null });
  assert.equal(fresh.result.thread.id, freshId);
  assert.equal(fresh.result.thread.name, "Git capture fixture");
  const read = await call("thread/read", { threadId: freshId, includeTurns: true });
  assert.equal(read.result.thread.turns[0].items.find(item => item.type === "agentMessage").text, "CAPTURE_OK");
  const missing = await call("thread/resume", { threadId: "unrecorded-thread" });
  assert.equal(missing.error.code, -32601);
  assert.equal(missing.result, undefined);
  console.log("resume fixtures: legacy, fresh, and unknown-thread boundaries passed");
} finally {
  child.stdin.end();
  await exited;
  lines.close();
  fs.rmSync(directory, { recursive: true, force: true });
}
