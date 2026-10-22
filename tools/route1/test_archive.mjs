import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";

const directory = fs.mkdtempSync(path.join(os.tmpdir(), "route1-archive-"));
const portReservation = net.createServer();
portReservation.listen(0, "127.0.0.1");
await once(portReservation, "listening");
const port = portReservation.address().port;
await new Promise(resolve => portReservation.close(resolve));
const oracle = path.join(directory, "oracle.json");
fs.copyFileSync(new URL("./oracle.json", import.meta.url), oracle);
const child = spawn(process.execPath, [new URL("./server.mjs", import.meta.url).pathname], {
  detached: true,
  env: {
    ...process.env,
    R1_OFFLINE: "1",
    R1_PORT: String(port),
    R1_ORACLE: oracle,
    R1_QUIET: "",
    MOCK_LOG: path.join(directory, "requests.log"),
    MOCK_FIXTURES: new URL("../../fixtures", import.meta.url).pathname,
    MOCK_SIGNED_OUT: "",
  },
  stdio: ["ignore", "pipe", "inherit"],
});
const exited = once(child, "exit");
const output = createInterface({ input: child.stdout });
const abort = new AbortController();
const timer = setTimeout(() => abort.abort(), 10000);
let reader;
let buffer = "";
const decoder = new TextDecoder();
try {
  for await (const line of output) {
    if (line.includes("route1 server on")) break;
  }
  const base = `http://127.0.0.1:${port}/__r1`;
  const stream = fetch(`${base}/events?sid=archive-test`, { signal: abort.signal });
  let nextId = 0;
  const notifications = new Map();
  async function call(method, params, hostId = "local") {
    const id = ++nextId;
    await fetch(`${base}/send?sid=archive-test`, {
      method: "POST",
      body: JSON.stringify({ type: "mcp-request", hostId, request: { id, method, params } }),
      signal: abort.signal,
    });
    reader ??= (await stream).body.getReader();
    let response = await nextEvent();
    while (response.type === "mcp-notification") {
      notifications.set(response.method, response);
      response = await nextEvent();
    }
    assert.equal(response.type, "mcp-response");
    assert.equal(response.hostId, hostId);
    assert.equal(response.message.id, id);
    return response.message;
  }
  async function nextEvent() {
    while (!buffer.includes("\n\n")) {
      const { value, done } = await reader.read();
      assert.equal(done, false);
      buffer += decoder.decode(value, { stream: true });
    }
    const end = buffer.indexOf("\n\n");
    const event = buffer.slice(0, end);
    buffer = buffer.slice(end + 2);
    return JSON.parse(event.slice("data: ".length));
  }
  const threadId = "01a13de9-6fff-78e1-84fc-ebb3a2dc873f";
  for (const method of ["thread/archive", "thread/unarchive"]) {
    const response = await call(method, { threadId });
    assert.equal(response.error, undefined);
    if (method === "thread/archive") assert.deepEqual(response.result, {});
    else {
      assert.equal(response.result.thread.id, threadId);
      assert.equal(response.result.thread.updatedAt, 1792540011);
    }
    const notification = notifications.get(`${method}d`) ?? await nextEvent();
    assert.equal(notification.type, "mcp-notification");
    assert.equal(notification.method, `${method}d`);
    assert.equal(notification.params.threadId, threadId);
    for (const params of [{ threadId: "unrecorded" }, {}, { threadId, extra: true }]) {
      assert.equal((await call(method, params)).error.code, -32601);
    }
    assert.equal((await call(method, { threadId }, "remote")).error.code, -32000);
  }
  for (const method of ["thread/delete", "thread/start", "turn/start"]) {
    assert.equal((await call(method, { threadId })).error.code, -32000);
  }
  console.log("archive replay: recorded results, notifications, exact parameters, host and write boundaries passed");
} finally {
  clearTimeout(timer);
  await reader?.cancel();
  abort.abort();
  output.close();
  process.kill(-child.pid, "SIGTERM");
  await exited;
  fs.rmSync(directory, { recursive: true, force: true });
}
