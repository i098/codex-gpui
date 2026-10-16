// Mock `codex app-server`: JSON-RPC over stdio, one JSON per line. Answers from the recorded
// fixtures (fixtures/readback.jsonl, fixtures/sessions/*.jsonl; lines {t, dir: send|recv, msg}).
// A request matches a recorded request by method; among those, exact params win, then the one
// with the same threadId, then the most equal top-level params. The recorded result is re-keyed to
// the live id and the recorded notifications that followed it are emitted. Unknown methods answer
// a JSON-RPC error. Log: $MOCK_LOG (default /tmp/inv-r1/mock-app-server.log).
import fs from "node:fs";
import path from "node:path";
import { createInterface } from "node:readline";

const ROOT = process.env.MOCK_FIXTURES || path.resolve(path.dirname(new URL(import.meta.url).pathname), "../../../fixtures");
const LOG = process.env.MOCK_LOG || "/tmp/inv-r1/mock-app-server.log";
const log = (...a) => fs.appendFileSync(LOG, `${new Date().toISOString().slice(11, 23)} ${a.join(" ")}\n`);
const files = [path.join(ROOT, "readback.jsonl"), ...fs.readdirSync(path.join(ROOT, "sessions")).map((f) => path.join(ROOT, "sessions", f))];

// Recorded exchanges: {method, params, reply, notes[]}.
const byMethod = new Map();
for (const f of files) {
  const lines = fs.readFileSync(f, "utf8").trim().split("\n").map((l) => JSON.parse(l));
  for (let i = 0; i < lines.length; i++) {
    const { dir, msg } = lines[i];
    if (dir !== "send" || msg.id == null) continue;
    let reply;
    const notes = [];
    for (let j = i + 1; j < lines.length; j++) {
      const l = lines[j];
      if (l.dir === "send" && l.msg.id != null && reply) break;
      if (l.dir !== "recv") continue;
      if (l.msg.id === msg.id) reply = l.msg;
      else if (reply && l.msg.method && l.msg.id == null) notes.push(l.msg);
    }
    if (!reply) continue;
    if (!byMethod.has(msg.method)) byMethod.set(msg.method, []);
    byMethod.get(msg.method).push({ params: msg.params ?? null, reply, notes });
  }
}

const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
function pick(method, params) {
  const c = byMethod.get(method);
  if (!c) return null;
  const score = (r) => {
    if (same(r.params, params)) return 1e6;
    const p = params ?? {};
    const q = r.params ?? {};
    if (p.threadId && q.threadId && p.threadId !== q.threadId) return -1;
    return (p.threadId && p.threadId === q.threadId ? 1e3 : 0) + Object.keys(p).filter((k) => same(p[k], q[k])).length;
  };
  return c.reduce((best, r) => (score(r) > score(best) ? r : best));
}

const out = (m) => process.stdout.write(`${JSON.stringify(m)}\n`);
createInterface({ input: process.stdin }).on("line", (line) => {
  let m;
  try {
    m = JSON.parse(line);
  } catch {
    return;
  }
  if (m.id == null) return; // notifications (initialized) and responses need no answer
  const r = pick(m.method, m.params);
  if (!r) {
    log("MISS", m.method, JSON.stringify(m.params ?? null).slice(0, 200));
    return out({ id: m.id, error: { code: -32601, message: `mock: no recording for ${m.method}` } });
  }
  log("hit ", m.method, JSON.stringify(m.params ?? null).slice(0, 120));
  out({ ...adapt(m.params ?? {}, r), id: m.id });
  for (const n of r.notes) out(n);
});

// The fixtures were recorded by a scripted client whose list params differ from the desktop's:
// a page in the other sort direction is reversed, and sub-agent listings (ancestorThreadId) are
// empty because no fixture thread spawned one.
function adapt(p, r) {
  const res = r.reply.result;
  if (!res) return r.reply;
  if (p.ancestorThreadId) return { ...r.reply, result: { ...res, data: [], nextCursor: null } };
  const q = r.params ?? {};
  if (Array.isArray(res.data) && p.sortDirection && q.sortDirection && p.sortDirection !== q.sortDirection) return { ...r.reply, result: { ...res, data: [...res.data].reverse() } };
  return r.reply;
}
