#!/usr/bin/env node
// Records real `codex app-server` JSON-RPC sessions on the Mac as JSONL fixtures.
// Usage: node tools/fixtures/record.mjs [setup] [explain] [subtract] [greeting] [readback]
// No arguments runs all steps in that order. `setup` recreates /tmp/cgx-mac/proj.
import { spawn, execFileSync } from "node:child_process";
import { createInterface } from "node:readline";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const OUT = join(dirname(fileURLToPath(import.meta.url)), "../../fixtures");
const PROJ = "/tmp/cgx-mac/proj";
const CODEX = "/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex";
// notify=[] keeps the turn-complete hook (an app bundle binary) from running.
const SERVER = `cd ${PROJ} && CODEX_HOME=/tmp/cgx-mac/home ${CODEX} app-server -c 'notify=[]'`;

const SETUP = `set -e; rm -rf ${PROJ}; mkdir -p ${PROJ}/src; cd ${PROJ}
printf '# demo\\nA tiny Python demo used by codex-gpui fixtures.\\n' > README.md
printf 'print("Hello, world")\\n' > hello.py
: > src/__init__.py
printf 'def add(a, b):\\n    return a + b\\n' > src/math.py
git init -q -b main && git add -A
GIT_AUTHOR_DATE=2026-01-01T00:00:00Z GIT_COMMITTER_DATE=2026-01-01T00:00:00Z \\
  git -c user.name=demo -c user.email=demo@example.com commit -qm 'Initial commit'
git log --oneline`;

const PROMPTS = {
  explain: "In two sentences, what does this repository contain?",
  subtract:
    "Add a subtract(a, b) function to src/math.py next to add, then run python3 -c 'from src.math import subtract; print(subtract(5, 3))' to check it.",
  greeting: 'Change the greeting in hello.py to "Hello, Codex".',
};

// Deterministic secret scrub: values under secret-looking keys and inside linked
// account profiles become fixed placeholders, and every occurrence of the same
// raw value anywhere in the file is replaced too (see Session.close).
const SECRET_KEY = /token|secret|password|api_?key|authorization|cookie/i;
const ID_KEY = /^(chatgpt_?)?(account|user|org|organization|workspace|installation)_?id$/i;
const PROFILE = { id: "0", name: "Demo User", nickname: "demo", picture: "https://example.com/avatar.png" };
const EMAIL = /[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g;
const JWT = /eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+/g;
const seen = new Map();
const MAC_USER = new RegExp(`\\b${execFileSync("ssh", ["mac", "whoami"], { encoding: "utf8" }).trim()}\\b`, "g");
function placeholder(key, profile) {
  if (SECRET_KEY.test(key)) return "<redacted>";
  if (ID_KEY.test(key)) return "00000000-0000-4000-8000-000000000000";
  if (/email/i.test(key)) return "user@example.com";
  return profile ? PROFILE[key] : undefined;
}
function scrub(v, key = "", profile = false) {
  if (Array.isArray(v)) return v.map((x) => scrub(x, key, profile));
  if (v && typeof v === "object") {
    const inner = profile || /profile$/i.test(key);
    return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, scrub(x, k, inner)]));
  }
  if (typeof v !== "string" || v === "") return v;
  const ph = placeholder(key, profile);
  if (ph !== undefined && v.length > 3) seen.set(v, ph);
  let s = v;
  for (const [raw, rep] of seen) s = s.split(raw).join(rep);
  return s
    .replace(JWT, "<redacted-token>")
    .replace(EMAIL, (m) => (m === "demo@example.com" ? m : "user@example.com"))
    .replace(/\/Users\/[^/\s"']+/g, "/Users/user")
    .replace(MAC_USER, "user");
}

function answer(msg) {
  const p = msg.params ?? {};
  switch (msg.method) {
    case "item/commandExecution/requestApproval":
    case "item/fileChange/requestApproval":
      return { decision: "accept" };
    case "applyPatchApproval":
    case "execCommandApproval":
      return { decision: "approved" };
    case "item/permissions/requestApproval":
      return { permissions: p.permissions ?? {}, scope: "turn" };
    case "mcpServer/elicitation/request":
      return { action: "accept", content: {}, _meta: null };
    case "item/tool/requestUserInput":
      return { answers: {} };
    case "currentTime/read":
      return { currentTimeAt: Math.floor(Date.now() / 1000) };
  }
}

class Session {
  constructor(name) {
    this.name = name;
    this.start = Date.now();
    this.log = [];
    this.nextId = 1;
    this.pending = new Map();
    this.waiters = [];
    this.lastRecv = Date.now();
    this.proc = spawn("ssh", ["mac", SERVER], { stdio: ["pipe", "pipe", "inherit"] });
    this.exited = new Promise((r) => this.proc.on("exit", r));
    createInterface({ input: this.proc.stdout }).on("line", (line) => this.recv(line));
  }
  write(dir, msg) {
    this.log.push({ t: Date.now() - this.start, dir, msg });
    if (dir === "send") this.proc.stdin.write(`${JSON.stringify(msg)}\n`);
  }
  recv(line) {
    if (!line.trim()) return;
    const msg = JSON.parse(line);
    this.lastRecv = Date.now();
    this.write("recv", msg);
    if (msg.id !== undefined && msg.method) {
      const result = answer(msg);
      this.write(
        "send",
        result
          ? { id: msg.id, result }
          : { id: msg.id, error: { code: -32601, message: `unsupported: ${msg.method}` } },
      );
    } else if (msg.id !== undefined) {
      this.pending.get(msg.id)?.(msg);
      this.pending.delete(msg.id);
    } else {
      this.waiters = this.waiters.filter((w) => !w(msg));
    }
  }
  request(method, params) {
    const id = this.nextId++;
    const msg = params === undefined ? { id, method } : { id, method, params };
    const reply = new Promise((r) => this.pending.set(id, r));
    this.write("send", msg);
    return reply.then((m) => {
      console.error(`[${this.name}] ${method}: ${m.error ? `error ${m.error.message}` : "ok"}`);
      return m;
    });
  }
  notify(method) {
    this.write("send", { method });
  }
  waitFor(method, pred = () => true) {
    return new Promise((r) =>
      this.waiters.push((m) => m.method === method && pred(m.params) && (r(m), true)),
    );
  }
  async quiet(ms, max = 60000) {
    const until = Date.now() + max;
    while (Date.now() - this.lastRecv < ms && Date.now() < until)
      await new Promise((r) => setTimeout(r, 250));
  }
  async init() {
    await this.request("initialize", {
      clientInfo: { name: "codex-gpui", title: "Codex GPUI", version: "0.1.0" },
      capabilities: { experimentalApi: true, requestAttestation: false },
    });
    this.notify("initialized");
  }
  async close(file) {
    this.proc.stdin.end();
    const timer = setTimeout(() => this.proc.kill(), 10000);
    await this.exited;
    clearTimeout(timer);
    const path = join(OUT, file);
    mkdirSync(dirname(path), { recursive: true });
    this.log.forEach((l) => scrub(l)); // first pass only collects raw secrets
    writeFileSync(path, this.log.map((l) => `${JSON.stringify(scrub(l))}\n`).join(""));
    console.error(`[${this.name}] wrote ${path} (${this.log.length} lines)`);
  }
}

async function recordTurn(name) {
  const s = new Session(name);
  await s.init();
  const { result, error } = await s.request("thread/start", {
    cwd: PROJ,
    sandbox: "workspace-write",
    approvalPolicy: "never",
  });
  if (error) throw new Error(`thread/start: ${error.message}`);
  const threadId = result.thread.id;
  const done = s.waitFor("turn/completed", (p) => p.threadId === threadId);
  await s.request("turn/start", {
    threadId,
    input: [{ type: "text", text: PROMPTS[name], text_elements: [] }],
  });
  const { params } = await done;
  console.error(`[${name}] thread ${threadId} turn ${params.turn.status}`);
  await s.quiet(3000); // trailing notifications such as thread/name/updated
  await s.close(`sessions/${name}.jsonl`);
}

async function recordReadback() {
  const s = new Session("readback");
  await s.init();
  const cwds = [PROJ];
  const reads = [
    ["getAuthStatus", { includeToken: false, refreshToken: false }],
    ["account/read", { refreshToken: false }],
    ["account/gatewayOAuth/read"],
    ["configRequirements/read"],
    ["config/read", { includeLayers: false, cwd: PROJ }],
    ["experimentalFeature/list", {}],
    ["model/list", {}],
    ["collaborationMode/list", {}],
    ["permissionProfile/list", { cwd: PROJ }],
    ["modelProvider/capabilities/read", {}],
    ["account/rateLimits/read"],
    ["account/usage/read"],
    ["account/workspaceMessages/read"],
    ["skills/list", { cwds }],
    ["hooks/list", { cwds }],
    ["plugin/list", { cwds, marketplaceKinds: ["local"] }],
    ["plugin/installed", { cwds }],
    ["app/installed", {}],
    ["mcpServerStatus/list", { limit: 100, detail: "toolsAndAuthOnly" }],
    ["project/list", {}],
    ["threadSection/list", {}],
    ["thread/loaded/list", {}],
    ["remoteControl/status/read"],
    ["memory/status", {}],
    ["gitDiffToRemote", { cwd: PROJ }],
  ];
  for (const [method, params] of reads) await s.request(method, params);
  // Same params as the desktop Recents query.
  const list = await s.request("thread/list", {
    limit: 50,
    cursor: null,
    sortKey: "recency_at",
    modelProviders: null,
    archived: false,
    sourceKinds: [],
    useStateDbOnly: true,
  });
  for (const t of list.result?.data ?? []) {
    const threadId = t.id;
    await s.request("thread/read", { threadId, includeTurns: true });
    // Paginated reads as the desktop thread view sends them.
    await s.request("thread/turns/list", { threadId, cursor: null, limit: 5, sortDirection: "asc", itemsView: "full" });
    await s.request("thread/items/list", { threadId, cursor: null, limit: 500, sortDirection: "asc" });
    await s.request("thread/goal/get", { threadId });
    await s.request("thread/queue/list", { threadId });
    console.error(`[readback] thread ${threadId} name=${JSON.stringify(t.name)} preview=${JSON.stringify(t.preview)}`);
  }
  await s.quiet(2000);
  await s.close("readback.jsonl");
}

const steps = process.argv.slice(2);
for (const step of steps.length ? steps : ["setup", "explain", "subtract", "greeting", "readback"]) {
  if (step === "setup") console.error(execFileSync("ssh", ["mac", SETUP], { encoding: "utf8" }));
  else if (step === "readback") await recordReadback();
  else if (PROMPTS[step]) await recordTurn(step);
  else throw new Error(`unknown step: ${step}`);
}
