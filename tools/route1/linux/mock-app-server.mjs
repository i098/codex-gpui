// Mock `codex app-server`: JSON-RPC over stdio, one JSON per line. Answers from the recorded
// fixtures (fixtures/readback.jsonl, fixtures/sessions/*.jsonl; lines {t, dir: send|recv, msg}).
// A request matches a recorded request by method; among those, exact params win, then the one
// with the same threadId, then the most equal top-level params. The recorded result is re-keyed to
// the live id and the recorded notifications that followed it are emitted. Unknown methods answer
// a JSON-RPC error. Log: $MOCK_LOG (default /tmp/inv-r1/mock-app-server.log).
// MOCK_SIGNED_OUT=1: the account reads report no account (the renderer shows the login route)
// until account/login/start, which answers a ChatGPT login; the recorder pushes account/updated
// and account/login/completed itself. One login per mock start.
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
const oracle = JSON.parse(fs.readFileSync(process.env.R1_ORACLE || new URL("../oracle.json", import.meta.url), "utf8"));
for (const [key, value] of Object.entries(oracle)) {
  const prefix = "mcp local thread/resume ";
  if (!key.startsWith(prefix) || !value?.message?.result?.thread) continue;
  if (!byMethod.has("thread/resume")) byMethod.set("thread/resume", []);
  byMethod.get("thread/resume").push({ params: JSON.parse(key.slice(prefix.length)), reply: value.message, notes: [] });
}
// MOCK_BACKEND_ORIGIN=<url>: a fake signed-in ChatGPT account for the original desktop app. Token reads
// return an unsigned JWT with placeholder ids, and account/read routes the backend to that origin
// (tools/route1/backend-stub.mjs). No real credential exists anywhere in this flow.
const BACKEND = process.env.MOCK_BACKEND_ORIGIN;
const b64 = (o) => Buffer.from(JSON.stringify(o)).toString("base64url");
const FAKE_TOKEN = [b64({ alg: "none", typ: "JWT" }), b64({
  exp: 4102444800, iat: 1760000000, iss: "https://auth.openai.com", sub: "user-000000000000000000000000",
  "https://api.openai.com/auth": { chatgpt_account_id: "00000000-0000-4000-8000-000000000000", chatgpt_user_id: "user-000000000000000000000000", user_id: "user-000000000000000000000000", chatgpt_plan_type: "promax", chatgpt_compute_residency: "no_constraint" },
  "https://api.openai.com/profile": { email: "user@example.com", email_verified: true },
}), "fake"].join(".");
// MOCK_THEME=light|dark: the desktop reads its settings from the `desktop` table of config/read (the theme, and the
// scrubbed home folder that Settings shows as the projectless chat folder).
const THEME = process.env.MOCK_THEME;
function fakeAccount(method, params, reply) {
  const res = reply.result;
  if (THEME && method === "config/read" && res?.config) return { ...reply, result: { ...res, config: { ...res.config, desktop: { ...res.config.desktop, appearanceTheme: THEME, projectlessWorkspaceRoot: "/Users/user/Documents/Codex" } } } };
  if (!BACKEND || !res) return reply;
  if (method === "getAuthStatus" && params?.includeToken) return { ...reply, result: { ...res, authToken: FAKE_TOKEN } };
  if (method === "account/read" && res.workspaceRouting) return { ...reply, result: { ...res, workspaceRouting: { ...res.workspaceRouting, backendOrigin: BACKEND } } };
  return reply;
}
let signedOut = !!process.env.MOCK_SIGNED_OUT;
const SIGNED_OUT = {
  getAuthStatus: { authMethod: null, authToken: null, requiresOpenaiAuth: true },
  "account/read": { account: null, requiresOpenaiAuth: true },
};
if (signedOut) {
  const loginId = "11111111-1111-4111-8111-111111111111";
  const authUrl = "https://auth.openai.com/oauth/authorize?client_id=app_EMoamEEZ73f0CkXaXp7hrann&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback&response_type=code&scope=openid+profile+email+offline_access&state=s034b-state";
  byMethod.set("account/login/start", [{ params: { type: "chatgpt" }, reply: { result: { type: "chatgpt", loginId, authUrl } }, notes: [] }]);
}

const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
function pick(method, params) {
  const c = byMethod.get(method);
  if (!c) return null;
  if (/^thread\/(archive|unarchive)$/.test(method)) return c.find(r => same(r.params, params)) ?? null;
  const score = (r) => {
    if (same(r.params, params)) return 1e6;
    const p = params ?? {};
    const q = r.params ?? {};
    if (p.threadId && q.threadId && p.threadId !== q.threadId) return -1;
    return (p.threadId && p.threadId === q.threadId ? 1e3 : 0) + Object.keys(p).filter((k) => same(p[k], q[k])).length;
  };
  const best = c.reduce((best, r) => (score(r) > score(best) ? r : best));
  return score(best) < 0 ? null : best;
}

// MOCK_REAL_TMP=1: on a Mac /tmp is a link to /private/tmp, and the desktop's git worker reports the real path;
// the fixtures' project path (/tmp/cgx-mac/proj) must name the same directory.
const REAL_TMP = process.env.MOCK_REAL_TMP ? (text) => text.replace(/(?<![\w/])\/tmp\/cgx-mac/g, "/private/tmp/cgx-mac") : (text) => text;
const out = (m) => process.stdout.write(`${REAL_TMP(JSON.stringify(m))}\n`);
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
  if (signedOut && SIGNED_OUT[m.method]) return out({ id: m.id, result: SIGNED_OUT[m.method] });
  if (m.method === "account/login/start") signedOut = false;
  const turn = m.method === "turn/start" && HOLD ? freshTurn(r, m.params) : r;
  out({ ...fakeAccount(m.method, m.params, adapt(m.params ?? {}, turn)), id: m.id });
  // A fresh turn streams: its notifications arrive a little after the reply, as from a running model.
  const notes = turn.notes.filter((n) => !held(n));
  if (turn === r) for (const n of notes) out(n);
  else {
    // The turn ends 17 s after it started, as the recorded one did ("Worked for 17s").
    const at = (i) => 300 + 15 * i;
    const started = at(notes.findIndex((n) => n.method === "turn/started"));
    notes.forEach((n, i) => setTimeout(() => out(n), n.method === "turn/completed" || n.method === "thread/status/changed" && n.params.status.type === "idle" ? started + 17000 : at(i)));
  }
});

// A replayed turn gets new turn and item ids, so the desktop treats it as a new turn of the thread instead of
// an update of the recorded one (only with MOCK_HOLD_COMMAND).
function freshTurn(r, { threadId, clientUserMessageId, input }) {
  const ids = new Set([r.reply.result.turn.id]);
  for (const n of r.notes) if (n.params?.item?.id) ids.add(n.params.item.id);
  const rekey = (v) => {
    let text = JSON.stringify(v);
    for (const id of ids) text = text.replaceAll(id, `${id}-again`);
    return JSON.parse(text.replaceAll(/"threadId":"[^"]*"/g, `"threadId":"${threadId}"`));
  };
  // The desktop confirms delivery when the user message item carries the id of the message it sent.
  const own = (n) => {
    const item = n.params?.item;
    if (item?.type === "userMessage") Object.assign(item, { clientId: clientUserMessageId, content: input });
    return n;
  };
  // The held command prints its result and keeps running.
  const output = (n) => {
    const { item, threadId, turnId } = n.params ?? {};
    return n.method === "item/started" && item.type === "commandExecution" && item.command.includes(HOLD)
      ? [n, { method: "item/commandExecution/outputDelta", params: { threadId, turnId, itemId: item.id, delta: "2\n" } }]
      : [n];
  };
  // The recorded times move to now, so the turn lasts as long as the recorded one (17 s) from the desktop's view.
  const shift = Date.now() - r.notes.find((n) => n.method === "turn/started").emittedAtMs;
  const retime = (v) => {
    if (v && typeof v === "object")
      for (const [k, x] of Object.entries(v)) {
        if (/^(startedAt|completedAt)$/.test(k) && typeof x === "number") v[k] = x + Math.round(shift / 1000);
        else if (/^(startedAtMs|completedAtMs|emittedAtMs)$/.test(k) && typeof x === "number") v[k] = x + shift;
        else retime(x);
      }
    return v;
  };
  return { ...r, reply: retime(rekey(r.reply)), notes: r.notes.flatMap((n) => output(own(retime(rekey(n))))) };
}

// MOCK_HOLD_COMMAND=<text>: a replayed command whose text contains <text> never completes, so the thread keeps
// a running background command (the terminal screen) while its turn still finishes.
const HOLD = process.env.MOCK_HOLD_COMMAND;
// account/updated is dropped from replays: the desktop reads it as an account change and retires its app-server manager.
const held = (n) => n.method === "account/updated" || HOLD && n.method === "item/completed" && n.params.item.type === "commandExecution" && n.params.item.command.includes(HOLD);

// The fixtures were recorded by a scripted client whose list params differ from the desktop's:
// a page in the other sort direction is reversed, and sub-agent listings (ancestorThreadId) are
// empty because no fixture thread spawned one; no thread is pinned, so section listings are empty too.
function adapt(p, r) {
  const res = r.reply.result;
  if (!res) return r.reply;
  if (p.ancestorThreadId || p.sectionId) return { ...r.reply, result: { ...res, data: [], nextCursor: null } };
  const q = r.params ?? {};
  if (Array.isArray(res.data) && p.sortDirection && q.sortDirection && p.sortDirection !== q.sortDirection) return { ...r.reply, result: { ...res, data: [...res.data].reverse() } };
  return r.reply;
}
