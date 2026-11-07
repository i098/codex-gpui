// One measured launch on macOS. All times are Date.now() on this machine's clock. run.sh calls this once per run.
// Usage: node perf.mjs <orig|ours> <outdir> [--cold] [--vis] [--bin <path>] [--thread] [--mock <dir>] [--port <n>]
// Env: BENCH_WORK (scratch dir, default /tmp/cgx-bench; holds the codex wrapper), BENCH_ORIG (the original ChatGPT.app).
// Needs passwordless sudo for `purge` (--cold) and `launchctl asuser`, so the app starts in the background, never in front.
import { execSync, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
const a = process.argv.slice(2);
const app = a[0], out = a[1];
const has = (f) => a.includes(f);
const arg = (f, d) => (a.includes(f) ? a[a.indexOf(f) + 1] : d);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sh = (c) => execSync(c, { encoding: "utf8", maxBuffer: 1 << 26 });
const vis = has("--vis"), cold = has("--cold"), thread = has("--thread");
const PORT = +arg("--port", "9493");
const W = process.env.BENCH_WORK ?? "/tmp/cgx-bench";
const ORIG_APP = process.env.BENCH_ORIG ? fs.realpathSync(process.env.BENCH_ORIG) : "";
const USER = os.userInfo().username, UID = os.userInfo().uid;
const REPO = new URL("../..", import.meta.url).pathname;
const EX = "01a13de9-35ed-7c72-a36a-68eeb2f4e1ee";
fs.mkdirSync(out, { recursive: true });
if (cold) { sh("sync; sudo -n purge"); await sleep(3000); }
fs.writeFileSync(`${out}/meta.json`, JSON.stringify({ app, args: a, load: sh("uptime").trim(), locked: sh("ioreg -n Root -d1 -a | grep -A1 IOConsoleLocked | tr -d '\\n\\t '"), start: Date.now() }));
const env = ["PATH=/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin", `USER=${USER}`, `LOGNAME=${USER}`, "TMPDIR=/tmp"];
const prof = `${fs.realpathSync(W)}/profiles/${app}-${arg("--bin", "x").replace(/\W/g, "_")}`;
const exeOf = (p) => fs.realpathSync(p);
let exe, argv;
if (app === "orig") {
  exe = `${ORIG_APP}/Contents/MacOS/ChatGPT`;
  env.push(`HOME=${prof}/home`, `CODEX_HOME=${prof}/codex-home`, `CODEX_ELECTRON_USER_DATA_PATH=${prof}/userdata`, "CODEX_ELECTRON_START_IN_BACKGROUND=1", `CODEX_CLI_PATH=${W}/codex`);
  argv = [`--remote-debugging-port=${PORT}`, "--remote-debugging-address=127.0.0.1", ...(vis ? ["--disable-backgrounding-occluded-windows", "--disable-renderer-backgrounding", "--disable-background-timer-throttling"] : []), ...(thread ? [`codex://threads/${EX}`] : [])];
} else {
  exe = exeOf(arg("--bin"));
  env.push(`HOME=${prof}/home`, `CODEX_BIN=${W}/codex`, ...(vis ? ["CGX_PERF=1", "CGX_PERF_VISIBLE=1"] : []));
  argv = thread ? [`codex://threads/${EX}`] : [];
}
for (const d of ["home", "codex-home", "userdata", "home/Library/Application Support"]) fs.mkdirSync(`${prof}/${d}`, { recursive: true });
// The QA build clears the environment of its app-server, so the codex wrapper reads these files instead.
fs.writeFileSync(`${W}/current-repo`, arg("--mock", REPO)); fs.writeFileSync(`${W}/current-out`, out); fs.writeFileSync(`${W}/current-orig`, ORIG_APP);
const log = `${out}/app.log`;
fs.writeFileSync(log, "");
const fd = fs.openSync(log, "a");
const t0 = Date.now();
spawn("sudo", ["-n", "launchctl", "asuser", String(UID), "sudo", "-n", "-u", USER, "env", "-i", ...env, exe, ...argv], { stdio: ["ignore", fd, fd], detached: true }).unref();
const marks = { t0 };
const secsOf = (t) => t.split(":").reduce((x, y) => x * 60 + parseFloat(y), 0);
function allProcs() {
  return sh("ps -axo pid=,ppid=,rss=,time=,pcpu=,command=").split("\n").filter(Boolean).map((l) => {
    const m = l.trim().match(/^(\d+)\s+(\d+)\s+(\d+)\s+(\S+)\s+(\S+)\s+(.*)$/);
    return { pid: +m[1], ppid: +m[2], rss: +m[3], cpu: secsOf(m[4]), pcpu: +m[5], cmd: m[6] };
  });
}
// The app's own process tree: the main process (ours: the binary; original: ChatGPT with our CDP port) and all descendants,
// minus the mock app-server and the bundled CLI. Other workers' instances on this host are never counted.
function procs() {
  const all = allProcs();
  const roots = all.filter((r) => app === "orig" ? r.cmd.startsWith(exe + " ") && r.cmd.includes(`--remote-debugging-port=${PORT}`) : r.cmd === exe || r.cmd.startsWith(exe + " "));
  const keep = new Map(roots.map((r) => [r.pid, r]));
  for (let grew = true; grew;) { grew = false; for (const r of all) if (!keep.has(r.pid) && keep.has(r.ppid) && !/mock-app-server|codex-cli\//.test(r.cmd)) { keep.set(r.pid, r); grew = true; } }
  return [...keep.values()];
}
function foreign() {
  const mine = new Set(procs().map((p) => p.pid));
  return allProcs().filter((r) => !mine.has(r.pid) && /ChatGPT|Codex Framework|codex-gpui|cargo|rustc/.test(r.cmd) && !/perf\.mjs|sudo|launchctl/.test(r.cmd) && (r.pcpu > 1 || /MacOS\/ChatGPT|codex-gpui/.test(r.cmd))).map((r) => `${r.pid} ${r.pcpu} ${r.cmd.slice(0, 90)}`);
}
async function cdp(url) {
  const ws = new WebSocket(url); await new Promise((r) => (ws.onopen = r));
  let id = 0; const pend = new Map();
  ws.onmessage = (e) => { const m = JSON.parse(e.data); pend.get(m.id)?.(m); };
  return { ev: (expr) => new Promise((res) => { const i = ++id; pend.set(i, res); ws.send(JSON.stringify({ id: i, method: "Runtime.evaluate", params: { expression: expr, awaitPromise: true, returnByValue: true } })); }).then((m) => m.result?.result?.value), close: () => ws.close() };
}
const raf = "new Promise((r)=>requestAnimationFrame(()=>requestAnimationFrame(()=>r(Date.now()))))";
if (app === "orig") {
  let page;
  while (!page && Date.now() - t0 < 60000) {
    try { const l = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json(); page = l.find((t) => t.type === "page"); } catch {}
    if (!page) await sleep(5);
  }
  marks.cdp_target = Date.now();
  const c = await cdp(page.webSocketDebuggerUrl);
  while (Date.now() - t0 < 60000) {
    const text = (await c.ev(`document.body?.innerText ?? ""`).catch(() => "")) ?? "";
    if (!marks.shell_dom && text.includes("New chat")) { marks.shell_dom = Date.now(); if (vis) marks.shell_frame = await c.ev(raf); }
    if (!vis && marks.shell_dom) break;
    if (text.includes("Change the greeting in hello.py")) { marks.sidebar_dom = Date.now(); if (vis) marks.sidebar_frame = await c.ev(raf); break; }
    await sleep(5);
  }
  global.CDP = c;
} else if (vis) {
  while (Date.now() - t0 < 60000) {
    const s = fs.readFileSync(log, "utf8");
    const f = s.match(/CGX_PERF first_frame (\d+)/), m = s.match(/CGX_PERF sidebar_frame (\d+)/);
    if (f) marks.first_frame = +f[1];
    if (m) { marks.sidebar_frame = +m[1]; break; }
    await sleep(5);
  }
}
// Mock-server clock: first request and the includeToken getAuthStatus retries.
function mockMarks() {
  try {
    const L = fs.readFileSync(`${out}/mock.log`, "utf8").trim().split("\n").map((l) => [+l.slice(0, 13), l.slice(14)]);
    const first = (re) => L.find(([, t]) => re.test(t))?.[0];
    return { mock_first: L[0]?.[0], mock_thread_list: first(/^hit\s+thread\/list/), mock_auth_calls_ms: L.filter(([, t]) => /getAuthStatus \{"includeToken":true/.test(t)).map(([t]) => t - t0) };
  } catch { return {}; }
}
await sleep(Math.max(0, t0 + 30000 - Date.now()));
const foreign30 = foreign();
let page30 = null;
if (global.CDP) { page30 = await global.CDP.ev(`JSON.stringify({vis: document.visibilityState, anims: document.getAnimations().length, text: document.body.innerText.slice(0, 300)})`).catch(() => null); global.CDP.close(); }
const A = procs(); const tA = Date.now();
fs.writeFileSync(`${out}/ps30.txt`, A.map((p) => `${p.pid} ${p.rss} ${p.cpu} ${p.cmd}`).join("\n"));
await sleep(Math.max(0, t0 + 90000 - Date.now()));
const B = procs(); const tB = Date.now();
fs.writeFileSync(`${out}/ps90.txt`, B.map((p) => `${p.pid} ${p.rss} ${p.cpu} ${p.cmd}`).join("\n"));
let fp = "";
try { fp = sh(`footprint --noCategories ${B.map((p) => `-p ${p.pid}`).join(" ")} 2>&1`); } catch (e) { fp = String(e.stdout ?? e); }
fs.writeFileSync(`${out}/footprint.txt`, fp);
const fpMB = (() => { let t = 0; for (const l of fp.split("\n")) { const m = l.match(/Footprint:\s+([\d.]+)\s+(KB|MB|GB)/); if (m && /\[\d+\]/.test(l)) t += +m[1] * ({ KB: 1 / 1024, MB: 1, GB: 1024 }[m[2]]); } return +t.toFixed(1); })();
const sum = (P, k) => P.reduce((x, p) => x + p[k], 0);
const mm = mockMarks();
const rel = Object.fromEntries(Object.entries(marks).filter(([k, v]) => k !== "t0" && typeof v === "number").map(([k, v]) => [k, v - t0]));
const res = { page30, app, args: a.slice(2), t0, foreign_at_30s: foreign30, foreign_at_90s: foreign(), ...rel, mock_first_ms: mm.mock_first && mm.mock_first - t0, mock_thread_list_ms: mm.mock_thread_list && mm.mock_thread_list - t0, mock_auth_calls_ms: mm.mock_auth_calls_ms,
  rss_mb_30s: +(sum(A, "rss") / 1024).toFixed(1), nproc: A.length, rss_mb_90s: +(sum(B, "rss") / 1024).toFixed(1),
  footprint_mb_90s: fpMB, cpu_pct_30_90: +(((sum(B, "cpu") - sum(A, "cpu")) / ((tB - tA) / 1000)) * 100).toFixed(2), window_s: (tB - tA) / 1000,
  per_proc_cpu: B.map((p) => ({ cmd: p.cmd.replace(/^.*\/(Codex \([A-Za-z]+\)|ChatGPT|[^/ ]+)( .*?--type=([a-z-]+))?.*$/, "$1 $3").trim(), pct: +(((p.cpu - (A.find((q) => q.pid === p.pid)?.cpu ?? p.cpu)) / ((tB - tA) / 1000)) * 100).toFixed(2) })) };
fs.writeFileSync(`${out}/result.json`, JSON.stringify(res, null, 1));
console.log(JSON.stringify({ out, ...rel, mock_thread_list_ms: res.mock_thread_list_ms, rss30: res.rss_mb_30s, n: res.nproc, fp: res.footprint_mb_90s, cpu: res.cpu_pct_30_90 }));
