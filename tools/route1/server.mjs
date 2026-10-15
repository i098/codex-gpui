// Route 1: serves the original Codex renderer (/tmp/cgx/asar-x/webview) to a plain Chromium and
// plays the Electron main process for it. Usage: `node tools/route1/server.mjs` (port R1_PORT, 4517).
//
// - App-server JSON-RPC (`mcp-request` for host `local`) goes to a real `codex app-server` on the
//   Mac (CODEX_HOME=/tmp/cgx-mac/home); methods that would change that state are refused.
// - Everything else the main process answers (HTTP/vscode:// fetches, persisted atoms, shared
//   objects, cloud host requests, git worker) comes from oracle.json. A miss is asked once from the
//   live scratch app over CDP (tools/capture/cdp.mjs), scrubbed and stored, so replays are offline.
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const HERE = path.dirname(new URL(import.meta.url).pathname);
const WEBVIEW = process.env.R1_WEBVIEW || "/tmp/cgx/asar-x/webview";
const FONTS = process.env.R1_FONTS || "/tmp/cgx/route1-fonts";
const ORACLE = path.join(HERE, "oracle.json");
const PORT = +(process.env.R1_PORT || 4517);
// 2026-10-15 10:00 PDT: one day after the fixture threads were recorded.
const NOW = Date.UTC(2026, 9, 15, 17, 0, 0);
const APP_SERVER = `cd /tmp/cgx-mac/proj && CODEX_HOME=/tmp/cgx-mac/home /Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex app-server -c 'notify=[]'`;
const log = (...a) => process.env.R1_QUIET || console.log(new Date().toISOString().slice(11, 23), ...a);

// ---- scrub: personal values become the placeholders the repo fixtures use. Every entry (key and
// value, nested JSON bodies and base64 JSON tokens included) is scrubbed before it is stored, and
// the whole oracle again on load. `unscrub` maps learnt ids back for the live app.
const ZERO_ID = "00000000-0000-4000-8000-000000000000";
const SCRUB = [
  [/[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g, (m) => (/^demo@example\.com$|@openai-bundled\./.test(m) ? m : "user@example.com")],
  [/\/Users\/(?!user\b)[^/\s"'\\]+/g, "/Users/user"],
  [/-Users-(?!user-)[^-/\s"'\\]+/g, "-Users-user"],
  [/\b(user|org)-(?!0{24}(?![A-Za-z0-9]))[A-Za-z0-9]{24}(?![A-Za-z0-9])/g, (m, p) => `${p}-${"0".repeat(24)}`],
  [/\bchatgpt:[0-9a-f-]{36}:/g, `chatgpt:${ZERO_ID}:`],
  [/\+\d{1,3}[ -]?\d{3}[ -]?\d{3}[ -]?\d{4}\b/g, "+10000000000"],
  [/avatars\.githubusercontent\.com\/u\/\d+/g, "avatars.githubusercontent.com/u/0"],
  [/([?&]verify=)[^"&\\\s]+/g, (m, p) => `${p}0`],
  [/AccountUserId_[0-9a-f]{32}/g, `AccountUserId_${"0".repeat(32)}`],
  // Cursors and signed file URLs: base64 JSON, decoded, scrubbed and encoded again.
  [/eyJ[A-Za-z0-9_+/-]+={0,2}/g, (m) => {
    const enc = /[-_]/.test(m) ? "base64url" : "base64";
    const s = Buffer.from(m, enc).toString();
    if (Buffer.from(s).toString(enc) !== m) return m;
    const c = scrub(s);
    return c === s ? m : Buffer.from(c).toString(enc);
  }],
];
// Fields of an object that describes a person (it has one of the HINT keys); a numeric `id` there
// is an account id elsewhere (GitHub).
const HINT = /^(email|user_email|email_domain|nickname|first_name|phone_number|picture|profile_picture_url|ip)$/;
const PERSON = { name: "Demo User", display_name: "Demo User", first_name: "Demo", last_name: "User", nickname: "demo", username: "demo", email_domain: "example.com", ip: "0.0.0.0" };
const isPerson = (o) => Object.keys(o).some((k) => HINT.test(k));
const personValue = (k, x) => (typeof x !== "string" ? undefined : k === "id" ? (/^\d+$/.test(x) ? "0" : undefined) : Object.hasOwn(PERSON, k) ? PERSON[k] : undefined);
const ids = new Map(); // real id -> placeholder, learnt from id-like keys
const names = new Map(); // real person name -> placeholder, replaced wherever it is a whole string
const ID_KEY = /^(chatgpt_?)?(account|user)_?id$/i;
const parsed = (v) => {
  if (typeof v === "string" && /^\s*[[{]/.test(v)) try { return JSON.parse(v); } catch {}
  return undefined;
};
function learn(v, key = "") {
  v = parsed(v) ?? v;
  if (Array.isArray(v)) v.forEach((x) => learn(x, key));
  else if (v && typeof v === "object") {
    const person = isPerson(v);
    for (const [k, x] of Object.entries(v)) {
      if (person && Object.hasOwn(PERSON, k) && typeof x === "string" && /[A-Za-z]{3}/.test(x) && x !== PERSON[k]) names.set(x, PERSON[k]);
      learn(x, k);
    }
  } else if (typeof v === "string" && ID_KEY.test(key) && v.length > 8 && !ids.has(v) && !v.startsWith("0000"))
    ids.set(v, /user/i.test(key) ? `user-${"0".repeat(24)}` : ZERO_ID);
}
function scrub(v) {
  if (typeof v === "string") {
    const j = parsed(v);
    if (j !== undefined) {
      const s = JSON.stringify(scrub(j));
      return s === JSON.stringify(j) ? v : s;
    }
    if (names.has(v)) return names.get(v);
    for (const [re, rep] of SCRUB) v = v.replace(re, rep);
    for (const [real, rep] of ids) v = v.split(real).join(rep);
    return v;
  }
  if (Array.isArray(v)) return v.map(scrub);
  if (!v || typeof v !== "object") return v;
  const person = isPerson(v);
  return Object.fromEntries(Object.entries(v).map(([k, x]) => [scrub(k), (person ? personValue(k, x) : undefined) ?? scrub(x)]));
}
function unscrub(json) {
  for (const [real, rep] of ids) json = json.split(rep).join(real);
  return json;
}

// ---- oracle: recorded main-process answers, filled from the live scratch app on a miss.
const oracle = fs.existsSync(ORACLE) ? JSON.parse(fs.readFileSync(ORACLE, "utf8")) : {};
let live;
async function liveEval(expr) {
  if (process.env.R1_OFFLINE) throw new Error("offline");
  live ??= import("../capture/cdp.mjs").then((m) => m.connect());
  return (await live).evaluate(expr);
}
// Sends `msg` to the live main process and resolves with the first for-view message that `match`
// (a JS predicate source over `d`) accepts, or null after `ms`.
const askLive = (msg, match, ms = 15000) =>
  liveEval(`new Promise((res) => {
    const h = (e) => { const d = e.data; if (${match}) { removeEventListener("message", h); clearTimeout(t); res(d); } };
    const t = setTimeout(() => { removeEventListener("message", h); res(null); }, ${ms});
    addEventListener("message", h);
    electronBridge.sendMessageFromView(${unscrub(JSON.stringify(msg))});
  })`);
const save = () => fs.writeFileSync(ORACLE, `${JSON.stringify(oracle, null, 1)}\n`);
let saveTimer;
async function recall(key, ask) {
  key = scrub(key);
  if (key in oracle) return oracle[key];
  let value = null;
  try {
    const raw = await ask();
    learn(raw);
    value = keep(key, scrub(raw));
    oracle[key] = value;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(save, 500);
    log("oracle +", key.slice(0, 160));
  } catch (e) {
    log("oracle miss", key.slice(0, 160), String(e.message).slice(0, 80));
  }
  return value;
}
// The owner's private data is not kept: ChatGPT-host chats in the sidebar bootstrap, and in the
// persisted atoms the Claude/Cursor import scan and the pinned ChatGPT projects.
const PRIVATE_ATOM = /^(external-agent-import-discovery|chatgpt-sidebar-state)/;
function keep(key, v) {
  if (key === "sidebar-bootstrap" && v) return { ...v, catalogEntries: v.catalogEntries.filter((e) => e.hostId === "local"), catalogHostIds: ["local"] };
  if (key === "persisted-atoms" && v?.state) return { ...v, state: Object.fromEntries(Object.entries(v.state).filter(([k]) => !PRIVATE_ATOM.test(k))) };
  return v;
}
// Entries that the rules above (or privateAnswer, VSCODE_READ) would not record are dropped.
const privateKey = (key) => {
  const fetch = /^fetch \S+ vscode:\/\/codex\/(\S+) /.exec(key);
  if (fetch) return !VSCODE_READ.test(fetch[1]);
  const svc = /^svc (\w+)\.(\w+) (.*)$/s.exec(key);
  return !!svc && !!privateAnswer(svc[1], svc[2], JSON.parse(svc[3]));
};
learn(oracle);

// ---- app-server on the Mac, shared by all page sessions.
// thread/resume only loads a thread into the server; nothing is written until a turn starts.
const READ_ONLY = /\/(read|list|get|search|status)[A-Za-z]*$|\/(installed|loaded\/list|capabilities\/read)$|^(getAuthStatus|gitDiffToRemote|fuzzyFileSearch|getConversationSummary|thread\/resume)$/;
// vscode://codex/<handler> fetches that only read (handler names from main-CqEoQDOU.js). Not
// external-agent-import-detect: it lists the owner's Claude/Cursor sessions.
const VSCODE_READ = /^(ambient-suggestions|app-server-connection-state|browser-use-origin-state-read|claude-code-import-status|codex-agents-md|codex-command-keymap-state|codex-home|developer-instructions|external-agent-import-status|external-agent-imported-connectors|fast-mode-rollout-metrics|find-files|get-.*|git-merge-base|git-origins|global-dictation-hotkey-state|has-custom-cli-executable|home-directory|inbox-items|is-.*|list-.*|local-custom-agents|locale-info|local-environment-config|mcp-codex-config|native-desktop-apps?|native-desktop-app-(icon|by-bundle-id)|os-info|paths-exist|projectless-.*|read-.*|recommended-skills|.*directory-entries|settings-read|third-party-notices|workspace-root-options|worktree-shell-environment-config|wsl-bash-availability|statsig-.*)$/;
let appServer;
const rpcPending = new Map();
let rpcId = 0;
function startAppServer() {
  const p = spawn("ssh", ["mac", APP_SERVER], { stdio: ["pipe", "pipe", "inherit"] });
  createInterface({ input: p.stdout }).on("line", (line) => {
    let m;
    try {
      m = JSON.parse(line);
    } catch {
      return;
    }
    learn(m);
    if (m.id != null && rpcPending.has(m.id)) {
      const f = rpcPending.get(m.id);
      rpcPending.delete(m.id);
      f(m);
    } else if (m.method && m.id == null) {
      for (const s of sessions.values()) s.emit({ type: "mcp-notification", hostId: "local", method: m.method, params: m.params });
    }
  });
  const call = (method, params) =>
    new Promise((res) => {
      const id = ++rpcId;
      rpcPending.set(id, res);
      p.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
    });
  const ready = call("initialize", {
    clientInfo: { name: "codex_desktop", title: "Codex Desktop", version: "26.1007.30912" },
    capabilities: { experimentalApi: true, requestAttestation: false },
  }).then((r) => {
    p.stdin.write(`${JSON.stringify({ method: "initialized" })}\n`);
    log("app-server ready", r.result?.userAgent);
    return r.result;
  });
  return { call, ready, proc: p };
}

// ---- page sessions (one per page load).
const sessions = new Map();
function session(sid) {
  let s = sessions.get(sid);
  if (s) return s;
  s = { sid, queue: [], res: null, atoms: null, unknown: new Set() };
  s.emit = (msg) => {
    const line = `data: ${scrub(JSON.stringify(msg))}\n\n`;
    if (s.res) s.res.write(line);
    else s.queue.push(line);
  };
  sessions.set(sid, s);
  return s;
}

async function fromView(s, msg) {
  if (process.env.R1_TRACE) log("<-", msg.type, msg.request?.method ?? msg.url ?? msg.key ?? "");
  switch (msg.type) {
    case "ready": {
      for (const m of (await recall("ready", () => readySnapshot())) ?? []) s.emit(m);
      // The Mac window is opaque: Electron main sends this on ready, and the renderer then sets
      // `electron-opaque` on <html> (body gets the opaque background).
      s.emit({ type: "electron-window-opaque-surface-changed", opaqueWindowSurfaceEnabled: true });
      if (msg.initializationOnly) break;
      const prompts = await recall("custom-prompts", () => null);
      if (prompts) s.emit(prompts);
      await sendAtoms(s);
      break;
    }
    case "persisted-atom-sync-request":
      await sendAtoms(s);
      break;
    case "persisted-atom-update":
      s.atoms ??= structuredClone(await atoms());
      if (msg.deleted) delete s.atoms[msg.key];
      else s.atoms[msg.key] = msg.value;
      break;
    case "shared-object-subscribe":
      s.emit({ type: "shared-object-updated", key: msg.key, value: await sharedValue(msg.key) });
      break;
    case "mcp-request":
      await mcpRequest(s, msg);
      break;
    case "mcp-response":
      if (msg.hostId === "local") appServer.proc.stdin.write(`${unscrub(JSON.stringify(msg.response))}\n`);
      break;
    case "fetch": {
      // vscode://codex/<handler> runs a main-process handler; only reading handlers reach the live app.
      const handler = msg.url.replace(/^vscode:\/\/codex\//, "");
      if (!VSCODE_READ.test(handler)) {
        log("fetch (local null)", handler);
        s.emit({ type: "fetch-response", responseType: "success", requestId: msg.requestId, status: 200, headers: { "content-type": "application/json" }, bodyJsonString: "null" });
        break;
      }
      const key = `fetch ${msg.method ?? "GET"} ${msg.url} ${keyOf(msg.body ?? "")}`;
      const res = await recall(key, () => {
        const rid = `r1-${crypto.randomUUID()}`;
        return askLive({ ...msg, requestId: rid }, `d?.type === "fetch-response" && d.requestId === "${rid}"`);
      });
      s.emit(res ? { ...res, requestId: msg.requestId } : { type: "fetch-response", responseType: "error", requestId: msg.requestId, status: 500, error: "route1: no recorded response", responseStatus: null });
      break;
    }
    case "shared-object-set":
    case "shared-object-unsubscribe":
    case "cancel-fetch":
    case "mcp-notification":
    case "mcp-request-abandon":
    case "log-message":
      break;
    default:
      if (!s.unknown.has(msg.type)) {
        s.unknown.add(msg.type);
        log("from-view (ignored)", msg.type);
      }
  }
}

async function mcpRequest(s, msg) {
  const { method, id } = msg.request;
  if (!READ_ONLY.test(method)) {
    log("refused", msg.hostId, method);
    s.emit({ type: "mcp-response", hostId: msg.hostId, message: { id, error: { code: -32000, message: `route1: ${method} is not allowed` } } });
    return;
  }
  // thread/resume goes to the live app too: its app-server holds the fixture threads' writer
  // locks, so a second server answers "This is open in another app".
  if (msg.hostId !== "local" || method === "thread/resume") {
    const key = `mcp ${msg.hostId} ${method} ${keyOf(msg.request.params ?? null)}`;
    const res = await recall(key, () => {
      const rid = `r1-${crypto.randomUUID()}`;
      return askLive({ ...msg, request: { ...msg.request, id: rid } }, `d?.type === "mcp-response" && d.message?.id === "${rid}"`);
    });
    s.emit({ type: "mcp-response", hostId: msg.hostId, message: { ...(res?.message ?? { error: { code: -32000, message: "route1: no recorded response" } }), id } });
    return;
  }
  await appServer.ready;
  const r = await appServer.call(method, JSON.parse(unscrub(JSON.stringify(msg.request.params ?? null))));
  s.emit({ type: "mcp-response", hostId: "local", message: { ...r, id } });
}

async function readySnapshot() {
  const out = await liveEval(`new Promise((res) => {
    const out = []; const h = (e) => out.push(e.data); addEventListener("message", h);
    electronBridge.sendMessageFromView({ type: "ready", initializationOnly: true });
    setTimeout(() => { removeEventListener("message", h); res(out); }, 2000);
  })`);
  return out.filter((d) => ["codex-app-server-connection-changed", "codex-app-server-initialized", "request-user-input-auto-resolution-snapshot"].includes(d?.type));
}
const atoms = async () =>
  (await recall("persisted-atoms", () => askLive({ type: "persisted-atom-sync-request" }, `d?.type === "persisted-atom-sync"`))) ?? { type: "persisted-atom-sync", state: {}, canWritePrimaryWindowTabPersistence: true };
async function sendAtoms(s) {
  const a = await atoms();
  s.emit({ ...a, state: s.atoms ?? a.state });
}
const sharedValue = (key) => recall(`shared ${key}`, () => liveEval(`electronBridge.getSharedObjectSnapshotValue(${JSON.stringify(key)}) ?? null`));

// App-host services (see preload-stub.js). Only calls that read are asked from the live app;
// anything else answers undefined so the scratch app's state never changes.
const LIVE_RPC = `(await import("./assets/rpc-f6f57972be0d.js")).appServices`;
const services = () =>
  recall("app-services", () =>
    liveEval(`(async () => { const s = ${LIVE_RPC}; return Object.fromEntries(Object.keys(s).map((k) => [k, typeof s[k] === "function" ? "function" : s[k] ?? null])); })()`),
  );
const READ_METHOD = /^(get|list|read|is|has|fetch|load|query|lookup|resolve|check|find|search|current|snapshot|subscribe|watch|observe|on[A-Z])/;
// Request ids (random v4 UUIDs) and millisecond timestamps differ on every call; keys omit them.
const keyOf = (x) =>
  JSON.stringify(x)
    .replace(/[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/g, "<id>")
    .replace(/\b1\d{12}\b/g, "<t>");
// ChatGPT backend calls through the httpFetch service: GETs are reads; POSTs reach the live app only
// for the known read endpoints (feature flags). Billing, chats, pinned projects, MFA factors and
// other private data stay out of the oracle: GETs answer 404, other POSTs (analytics, cache
// flushes) an empty 200. ChatGPT-host thread catalog pages (the owner's chats) answer an empty
// last page.
const POST_READ_URL = /statsig\/bootstrap|ab\.chatgpt\.com\/v1\/initialize|has_installations/;
const PRIVATE_URL = /payment|subscriptions|checkout|gift-credits|referrals|\/spaces|\/pins\b|gizmos|amphora\/notifications|\/tbo\b|conversation|files\/library|\/websites|mfa_info/;
const jsonResponse = (status, body) => ({ value: { response: { __response: { status, statusText: "", headers: [["content-type", "application/json"]], body } } } });
function privateAnswer(svc, method, args) {
  if (svc === "localThreadCatalog" && method === "readPage" && args[0]?.hostId?.startsWith("chatgpt:")) return { value: { entries: [], nextCursor: null }, fn: false, calls: [] };
  if (svc !== "httpFetch" || method !== "fetch") return;
  const { url = "", method: verb = "GET" } = args[1] ?? {};
  const get = verb.toUpperCase() === "GET";
  if (PRIVATE_URL.test(url) || (!get && !POST_READ_URL.test(url))) return jsonResponse(get ? 404 : 200, "{}");
}
async function callService(s, { svc, method, args }) {
  if (!READ_METHOD.test(method)) {
    log("svc (local no-op)", `${svc}.${method}`);
    return {};
  }
  const own = privateAnswer(svc, method, args);
  if (own) {
    if (process.env.R1_TRACE) log("private (local answer)", svc, method, JSON.stringify(args).slice(0, 120));
    return own;
  }
  const r =
    (await recall(`svc ${svc}.${method} ${keyOf(args)}`, () =>
      liveEval(`(async () => {
        const s = ${LIVE_RPC};
        const calls = [];
        const args = ${unscrub(JSON.stringify(args))}.map((a, i) => a === "__fn" ? (...x) => { try { calls.push([i, structuredClone(x)]); } catch {} } : a);
        let value;
        try { value = await s[${JSON.stringify(svc)}][${JSON.stringify(method)}](...args); } catch (e) { return { error: String(e?.message ?? e) }; }
        const fn = typeof value === "function";
        if (args.some((a) => typeof a === "function")) await new Promise((r) => setTimeout(r, 1500));
        if (fn) { try { value[Symbol.dispose]?.(); } catch {} value = null; }
        const enc = async (v) => {
          if (v instanceof Response) {
            const buf = new Uint8Array(await v.arrayBuffer());
            const text = /json|text|xml|javascript/.test(v.headers.get("content-type") ?? "text");
            let b = ""; if (!text) for (const c of buf) b += String.fromCharCode(c);
            return { __response: { status: v.status, statusText: v.statusText, headers: [...v.headers], ...(text ? { body: new TextDecoder().decode(buf) } : { base64: btoa(b) }) } };
          }
          return v && typeof v === "object" && Object.getPrototypeOf(v) === Object.prototype ? Object.fromEntries(await Promise.all(Object.entries(v).map(async ([k, x]) => [k, await enc(x)]))) : v;
        };
        try { value = structuredClone(await enc(value ?? null)); } catch { value = null; }
        return { value, fn, calls };
      })()`),
    )) ?? {};
  // The app follows its own appearance setting; each page gets the theme it was opened with.
  if (svc === "settings" && method === "readAll" && r.value?.values) return { ...r, value: { ...r.value, values: { ...r.value.values, appearanceTheme: s.theme } } };
  if (svc === "settings" && args[0] === "appearanceTheme") return { ...r, calls: r.calls?.map(([i]) => [i, [{ configured: s.theme, effective: s.theme }]]) };
  return r;
}

// Background terminal: no fixture thread has a running command, and starting one is a write.
// The subtract session's recorded notifications are replayed for a new turn whose `python3`
// check starts (inProgress), prints "2", and is still running when the turn completes: that is
// what makes a command a background terminal. The command never gets item/completed. Fixed ids
// keep it deterministic; turn and item ids differ from the finished turn's.
function replayRunningCommand(s) {
  const lines = fs.readFileSync(path.join(HERE, "../../fixtures/sessions/subtract.jsonl"), "utf8").trim().split("\n").map((l) => JSON.parse(l).msg);
  const all = (method) => lines.filter((m) => m.method === method).map((m) => m.params);
  const isCheck = (p) => p.item?.type === "commandExecution" && p.item.command.includes("python3");
  const turnId = "01a13de9-70bc-7de1-8ae6-000000000001";
  const command = all("item/started").find(isCheck);
  const [active, idle] = all("thread/status/changed");
  const [started] = all("turn/started");
  const [completed] = all("turn/completed");
  const itemId = `${command.item.id}-running`;
  const notes = [
    { method: "thread/status/changed", params: active },
    { method: "turn/started", params: { ...started, turn: { ...started.turn, id: turnId, rootTurnId: turnId } } },
    { method: "item/started", params: { ...command, turnId, item: { ...command.item, id: itemId } } },
    { method: "item/commandExecution/outputDelta", params: { threadId: command.threadId, turnId, itemId, delta: all("item/completed").find(isCheck).item.aggregatedOutput } },
    { method: "turn/completed", params: { ...completed, turn: { ...completed.turn, id: turnId, rootTurnId: turnId, items: [{ ...command.item, id: itemId }] } } },
    { method: "thread/status/changed", params: idle },
  ];
  for (const n of notes) s.emit({ type: "mcp-notification", hostId: "local", ...n });
  return notes.length;
}

// ---- HTTP
const MIME = { ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".html": "text/html", ".json": "application/json", ".svg": "image/svg+xml", ".png": "image/png", ".jpg": "image/jpeg", ".webp": "image/webp", ".gif": "image/gif", ".woff2": "font/woff2", ".woff": "font/woff", ".ttf": "font/ttf", ".otf": "font/otf", ".wasm": "application/wasm", ".mp4": "video/mp4", ".webm": "video/webm", ".mp3": "audio/mpeg", ".wav": "audio/wav", ".txt": "text/plain" };

// Only for a renderer without the Mac system fonts (R1_SF_FONTS=1, e.g. Linux Chrome): SF Pro
// stands in for every system-font alias the app's stacks name first. The fonts are variable, so
// CSS weights such as 430 and 510 land on the wght axis. On the Mac the real system fonts render.
const SF_FONTS = !!process.env.R1_SF_FONTS;
const FONT_ALIASES = {
  "SFNS.ttf": ["-apple-system", "-apple-system-body", "BlinkMacSystemFont", "system-ui", "ui-sans-serif", "SF Pro", "SF Pro Text", "SF Pro Display"],
  "SFNSMono.ttf": ["ui-monospace", "SFMono-Regular", "SF Mono", "Menlo", "menlo"],
};
const fontCss = !SF_FONTS
  ? ""
  : Object.entries(FONT_ALIASES)
      .flatMap(([file, names]) =>
        names.flatMap((n) => [
          `@font-face{font-family:"${n}";src:url(/__r1/fonts/${file}) format("truetype");font-weight:1 1000;font-style:normal}`,
          file === "SFNS.ttf" ? `@font-face{font-family:"${n}";src:url(/__r1/fonts/SFNSItalic.ttf) format("truetype");font-weight:1 1000;font-style:italic}` : "",
        ]),
      )
      .join("\n");
// `system-ui` is a generic keyword that @font-face cannot claim, so with R1_SF_FONTS served CSS
// names a quoted family instead.
const rewriteCss = (css) => (SF_FONTS ? css.replace(/(^|[\s,:(])system-ui(?=[\s,;)!}]|$)/g, '$1"system-ui"') : css);

const sidebarBootstrap = () => recall("sidebar-bootstrap", () => liveEval("electronBridge.getInitialSidebarBootstrap()"));

function indexHtml(url) {
  const sid = crypto.randomUUID();
  let html = fs.readFileSync(path.join(WEBVIEW, "index.html"), "utf8");
  html = html.replace(/<meta http-equiv="Content-Security-Policy"[^>]*>/, "");
  const cfg = {
    sid,
    theme: url.searchParams.get("theme") === "dark" ? "dark" : "light",
    now: +(url.searchParams.get("now") || NOW),
    sentry: oracle.sentry,
    flavor: "prod",
    appShared: `/${html.match(/assets\/app-shared-[0-9a-f]+\.js/)[0]}`,
  };
  session(sid).theme = cfg.theme;
  return html.replace(
    "<head>",
    `<head>\n<style>${fontCss}</style>\n<script>window.__R1__=${JSON.stringify(cfg)}</script>\n<script src="/__r1/preload-stub.js"></script>`,
  );
}

function send(res, code, body, type = "application/json") {
  res.writeHead(code, { "content-type": type, "cache-control": "no-store" });
  res.end(body);
}
const readBody = (req) =>
  new Promise((r) => {
    const c = [];
    req.on("data", (d) => c.push(d));
    req.on("end", () => r(Buffer.concat(c).toString("utf8")));
  });

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://${req.headers.host}`);
  const p = url.pathname;
  const s = url.searchParams.get("sid") && session(url.searchParams.get("sid"));
  try {
    if (p === "/" || p === "/index.html") return send(res, 200, indexHtml(url), "text/html");
    if (p === "/__r1/preload-stub.js") return send(res, 200, fs.readFileSync(path.join(HERE, "preload-stub.js")), MIME[".js"]);
    if (p.startsWith("/__r1/fonts/")) return send(res, 200, fs.readFileSync(path.join(FONTS, path.basename(p))), MIME[".ttf"]);
    if (p === "/__r1/events") {
      res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-store" });
      s.res = res;
      for (const l of s.queue.splice(0)) res.write(l);
      req.on("close", () => sessions.delete(s.sid));
      return;
    }
    if (p === "/__r1/send" || p === "/__r1/worker") {
      const msg = JSON.parse(await readBody(req));
      send(res, 200, "{}");
      if (p === "/__r1/send") return fromView(s, JSON.parse(unscrub(JSON.stringify(msg))));
      return workerRequest(s, url.searchParams.get("id"), msg);
    }
    if (p === "/__r1/shared") return send(res, 200, JSON.stringify((await sharedValue(url.searchParams.get("key"))) ?? null));
    if (p === "/__r1/services") return send(res, 200, JSON.stringify((await services()) ?? {}));
    if (p === "/__r1/svc") return send(res, 200, JSON.stringify(await callService(s, JSON.parse(await readBody(req)))));
    if (p === "/__r1/sidebar") return send(res, 200, JSON.stringify(await sidebarBootstrap()));
    if (p === "/__r1/replay-running-command") return send(res, 200, JSON.stringify(replayRunningCommand(s)));
    const file = path.join(WEBVIEW, path.normalize(decodeURIComponent(p)));
    if (!file.startsWith(WEBVIEW) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) return send(res, 404, "not found", "text/plain");
    const ext = path.extname(file);
    const body = fs.readFileSync(file);
    if (ext === ".css") return send(res, 200, rewriteCss(body.toString("utf8")), MIME[ext]);
    // lodash `shuffle` (automations suggestion cards) draws from Math.random, and how many draws
    // come before it varies per load; served as identity, every capture shows the bundle order.
    if (/^shuffle-[0-9a-f]+\.js$/.test(path.basename(file))) return send(res, 200, body.toString("utf8").replace("function c(e){return(s(e)?r:i)(e)}", "function c(e){return s(e)?[...e]:Object.values(Object(e))}"), MIME[ext]);
    return send(res, 200, body, MIME[ext] ?? "application/octet-stream");
  } catch (e) {
    log("error", p, e.stack);
    if (!res.headersSent) send(res, 500, String(e), "text/plain");
  }
});

// Git worker: reads go to the live app's worker; only the replies to this request (and events of
// the live query it subscribes) are kept, then re-addressed to the page's request id.
const WORKER_WRITE = /apply|revert|checkout|create|delete|remove|commit|push|pull|fetch|stage|stash|reset|merge|rebase|clone|cleanup|abort|write|set-|rename|move|init|restore|discard|^add$/;
async function workerRequest(s, id, msg) {
  const req = msg.request;
  if (msg.type !== "worker-request" || WORKER_WRITE.test(req.method)) {
    if (req) log("worker (no reply)", req.method);
    return;
  }
  const sub = req.params?.subscriptionId;
  const r = await recall(`worker ${id} ${req.method} ${keyOf(req.params ?? null)}`, () =>
    liveEval(`new Promise((res) => {
      const out = [];
      const off = electronBridge.subscribeToWorkerMessages(${JSON.stringify(id)}, (m) => {
        if (m.response?.id === ${JSON.stringify(req.id)} || (${JSON.stringify(sub ?? null)} && m.event?.subscriptionId === ${JSON.stringify(sub ?? null)})) out.push(m);
      });
      electronBridge.sendWorkerMessageFromView(${JSON.stringify(id)}, ${unscrub(JSON.stringify(msg))});
      setTimeout(() => { off(); res(out); }, 3000);
    })`),
  );
  for (const m of r ?? []) {
    const message = m.response ? { ...m, response: { ...m.response, id: req.id } } : { ...m, event: { ...m.event, subscriptionId: sub } };
    s.emit({ __worker: id, message });
  }
}

// The stored oracle is scrubbed again with the current rules; entries now private are dropped.
{
  const before = JSON.stringify(oracle);
  const entries = Object.entries(oracle);
  for (const [k] of entries) delete oracle[k];
  for (const [k, v] of entries) {
    const key = scrub(k);
    if (privateKey(key) || (key !== k && key in oracle)) continue; // a clean key wins over a scrubbed duplicate
    oracle[key] = keep(key, scrub(v));
  }
  if (JSON.stringify(oracle) !== before) save(), log("oracle scrubbed");
}
// The renderer's Sentry would post to the network from a browser; an empty DSN keeps it off.
const sentry = await recall("sentry", () => liveEval("electronBridge.getSentryInitOptions()"));
if (sentry) sentry.dsn = "";
appServer = startAppServer();
server.listen(PORT, "127.0.0.1", () => log(`route1 server on http://127.0.0.1:${PORT}/`));
