import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import { spawn } from "node:child_process";

const prefix = "mcp local thread/resume ";
const oracle = JSON.parse(fs.readFileSync(new URL("../route1/oracle.json", import.meta.url), "utf8"));
const resumes = Object.keys(oracle).filter(key => key.startsWith(prefix)).map(key => JSON.parse(key.slice(prefix.length)));

// Select the retained request for this thread; its response remains owned by route1.
function recordedResume(message) {
  if (message.type !== "mcp-request" || message.hostId !== "local" || message.request?.method !== "thread/resume") return message;
  const incoming = message.request.params;
  const matches = resumes.filter(params => ["threadId", "path", "excludeTurns"].every(key => params[key] === incoming[key]));
  if (matches.length !== 1) throw new Error("No unique recorded resume for this thread, path, and history mode");
  return { ...message, request: { ...message.request, params: matches[0] } };
}

async function serve() {
  const reservation = net.createServer();
  await new Promise(resolve => reservation.listen(0, "127.0.0.1", resolve));
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  const child = spawn(process.execPath, [new URL("../route1/server.mjs", import.meta.url).pathname], {
    env: { ...process.env, R1_PORT: String(port) }, stdio: "inherit",
  });
  child.on("exit", code => process.exit(code || 1));
  const server = http.createServer(async (request, response) => {
    try {
      let body;
      if (request.method === "POST" && new URL(request.url, "http://local").pathname === "/__r1/send") {
        const chunks = [];
        for await (const chunk of request) chunks.push(chunk);
        body = Buffer.from(JSON.stringify(recordedResume(JSON.parse(Buffer.concat(chunks).toString()))));
      }
      const headers = { ...request.headers };
      if (body) headers["content-length"] = body.length;
      const upstream = http.request({ host: "127.0.0.1", port, path: request.url, method: request.method, headers }, result => {
        response.writeHead(result.statusCode, result.headers);
        result.pipe(response);
      });
      upstream.on("error", error => { response.writeHead(502); response.end(String(error)); });
      response.on("close", () => upstream.destroy());
      if (body) upstream.end(body); else request.pipe(upstream);
    } catch (error) {
      console.error(error);
      response.writeHead(500);
      response.end(String(error));
    }
  });
  server.listen(Number(process.env.R1_PORT), "127.0.0.1");
}

if (process.argv[2] === "--self-test") {
  const params = resumes[0];
  const message = { type: "mcp-request", hostId: "local", request: { id: 91, method: "thread/resume", params: { ...params, developerInstructions: "changed desktop context" } } };
  assert.deepEqual(recordedResume(message).request, { id: 91, method: "thread/resume", params });
  assert.throws(() => recordedResume({ ...message, request: { ...message.request, params: { ...params, threadId: "unrecorded" } } }), /No unique recorded resume/);
  console.log("Recorded resume selection passed");
} else {
  await serve();
}
