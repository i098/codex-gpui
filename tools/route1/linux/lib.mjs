// Recording helpers: raw CDP to the fleet chrome tier (R1_CDP, default :9522) in a fresh
// incognito browser context (tools/route1/chrome.mjs openPage), page = route1 server.
// Usage (Bun or Node): const L = await import("./lib.mjs"); await L.open(); ... L.save(name, {steps, result}).
import fs from "node:fs";
import path from "node:path";

process.env.R1_CDP ||= "http://127.0.0.1:9522";
const HERE = path.dirname(new URL(import.meta.url).pathname);
const { openPage } = await import(path.join(HERE, "../chrome.mjs"));
export const SHOT = "/tmp/inv-r1/shots";
fs.mkdirSync(SHOT, { recursive: true });
export const URL_ = process.env.R1_URL || "http://127.0.0.1:4561/?theme=dark";
export const VIEWPORT = { w: 1280, h: 832, dpr: 1, ua: "Macintosh (navigator.platform stays Linux x86_64)", theme: "dark", hoverEmulated: true };
const UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36";
export let P;
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
export const ev = (x) => P.evaluate(x);

// Headless Linux Chrome reports (hover: none) and (pointer: coarse/none); the app's hover styles
// sit in @media (hover:hover). This rewrites those media lists (and matchMedia) to match, as on a Mac.
const MEDIA_FIX = `(() => {
 const HOV = /\\((any-)?hover: ?hover\\)|\\((any-)?pointer: ?fine\\)/g, NOH = /\\((any-)?hover: ?none\\)|\\((any-)?pointer: ?coarse\\)/g;
 const fixText = (t) => t.replace(HOV, '(min-width: 0px)').replace(NOH, '(max-width: 0px)');
 const mm = window.matchMedia.bind(window); window.matchMedia = (q) => mm(fixText(q));
 const done = new WeakSet();
 const walk = (rules) => { for (const r of rules) { if (r.media && /hover|pointer/.test(r.media.mediaText)) { const t = fixText(r.media.mediaText); if (t !== r.media.mediaText) r.media.mediaText = t; } if (r.cssRules) walk(r.cssRules); } };
 window.__fixMedia = () => { for (const s of document.styleSheets) { if (done.has(s)) continue; try { walk(s.cssRules); done.add(s); } catch (e) {} } };
 setInterval(window.__fixMedia, 300);
})();`;
// From-view traffic (stub bridge): POST /__r1/send|svc|worker bodies, with page time.
const REQ_LOG = `window.__r1req=[];const __f=window.fetch.bind(window);window.fetch=(u,o)=>{try{const s=String(u);if(/\\/__r1\\/(send|svc|worker)/.test(s))window.__r1req.push({t:performance.now()|0,path:s.split('?')[0],body:JSON.parse(o.body)})}catch(e){}return __f(u,o)};`;
const FIND = `window.__find = (t) => { const vis = (e) => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
  if (t.selector) return [...document.querySelectorAll(t.selector)].find(vis) || null;
  const c = [...document.querySelectorAll('body *')].filter(e => vis(e) && ((e.getAttribute('aria-label')||'').trim() === t.text || (e.innerText||'').trim() === t.text || (t.prefix && (e.innerText||'').trim().startsWith(t.text))));
  c.sort((a,b) => { const ra=a.getBoundingClientRect(), rb=b.getBoundingClientRect(); return ra.width*ra.height - rb.width*rb.height; });
  return c[0] || null; };
 window.__bbox = (e) => { const r = e.getBoundingClientRect(); return {x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height)}; };
 window.__cs = (e) => { const s = getComputedStyle(e); return {backgroundColor: s.backgroundColor, color: s.color, opacity: s.opacity, outline: s.outline, boxShadow: s.boxShadow, cursor: s.cursor}; };
 window.__desc = (e) => e ? ({tag: e.tagName.toLowerCase(), text: (e.innerText||'').trim().slice(0,160), aria: e.getAttribute('aria-label'), role: e.getAttribute('role'), bbox: __bbox(e), computed: __cs(e)}) : null;`;

// Electron shows thread/row context menus natively via electronBridge.showContextMenu(items, pos);
// the stub returns null. This wraps the (frozen) bridge as it is assigned: every call is recorded
// in window.__ctx, and window.__ctxPick (an item id) is returned as the user's choice.
const CTX_HOOK = `window.__ctx=[];window.__ctxPick=null;(()=>{let real;Object.defineProperty(window,'electronBridge',{configurable:true,get(){return real},set(v){real=Object.freeze({...v,showContextMenu:async(items,pos)=>{window.__ctx.push({items,pos,t:performance.now()|0});const id=window.__ctxPick;window.__ctxPick=null;return id?{id}:null}})}})})();`;
// The model announcement modal comes back whenever get-global-state is read again (its dismissal
// is a global-state write that the server does not keep). Dismissed the way a user would.
const DISMISS = `setInterval(() => { const b = [...document.querySelectorAll('.codex-dialog button')].find((x) => x.innerText.trim() === 'Continue with current model'); if (b) { b.click(); window.__dismissed = (window.__dismissed || 0) + 1; } }, 200);`;
export const errors = [];
// Jotai store flushes throw AggregateError; log the inner errors so crashes are explainable.
const AGG = `(() => { const AE = window.AggregateError; window.AggregateError = function (errs, msg) { try { console.error('AGG ' + [...errs].map((e) => String((e && e.stack) || e).slice(0, 500)).join(' || ')); } catch (e) {} return new AE(errs, msg); }; })();`;
export async function open({ url = URL_, settle = 25000 } = {}) {
  P = await openPage();
  errors.length = 0;
  P.on((x) => {
    if (x.method === "Runtime.exceptionThrown") errors.push({ t: Date.now(), ex: x.params.exceptionDetails?.exception?.description?.slice(0, 600) });
    else if (x.method === "Runtime.consoleAPICalled" && x.params.type === "error") errors.push({ t: Date.now(), console: x.params.args.map((a) => a.value ?? a.description ?? "").join(" ").slice(0, 600) });
  });
  await P.send("Runtime.enable");
  await P.send("Page.enable");
  await P.send("Emulation.setUserAgentOverride", { userAgent: UA, platform: "MacIntel", userAgentMetadata: { brands: [{ brand: "Chromium", version: "155" }], fullVersion: "155.0.0.0", platform: "macOS", platformVersion: "26.2.0", architecture: "arm", model: "", mobile: false } });
  await P.send("Emulation.setTimezoneOverride", { timezoneId: "America/Los_Angeles" });
  await P.send("Emulation.setLocaleOverride", { locale: "en-US" });
  await P.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 832, deviceScaleFactor: 1, mobile: false });
  await P.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "dark" }] });
  await P.send("Page.addScriptToEvaluateOnNewDocument", { source: AGG + CTX_HOOK + MEDIA_FIX + REQ_LOG + FIND + DISMISS });
  await P.send("Page.navigate", { url });
  const t0 = Date.now();
  // The announcement modal appears once the model list has loaded (and can come back after it).
  while (Date.now() - t0 < settle) {
    await sleep(500);
    if (await ev(`!!document.querySelector('.ProseMirror') && !document.body.innerText.includes('Loading model')`).catch(() => false)) break;
  }
  for (let i = 0; i < 8; i++) {
    await sleep(500);
    if (await ev(`!!__find({text: 'Continue with current model'})`)) {
      await clickText("Continue with current model").catch(() => {}); // the DISMISS timer may win
      i = 4;
    }
  }
  await sleep(300);
  await moveTo(1270, 820);
  return P;
}
export const close = () => P?.close();

export const loc = (t) => ev(`__desc(__find(${JSON.stringify(t)}))`);
export const center = (b) => ({ x: b.x + b.w / 2, y: b.y + b.h / 2 });
const mouse = (type, x, y, extra = {}) => P.send("Input.dispatchMouseEvent", { type, x, y, ...extra });
export const moveTo = (x, y, buttons = 0) => mouse("mouseMoved", x, y, buttons ? { buttons, button: "left" } : {});
export const down = (x, y) => mouse("mousePressed", x, y, { button: "left", buttons: 1, clickCount: 1 });
export const up = (x, y) => mouse("mouseReleased", x, y, { button: "left", buttons: 0, clickCount: 1 });
export async function clickAt(x, y, button = "left") {
  await mouse("mouseMoved", x, y);
  await mouse("mousePressed", x, y, { button, buttons: button === "left" ? 1 : 2, clickCount: 1 });
  await mouse("mouseReleased", x, y, { button, buttons: 0, clickCount: 1 });
}
export async function clickText(text, button = "left", t = { text }) {
  const d = await loc(t);
  if (!d) throw new Error(`not found: ${JSON.stringify(t)}`);
  const c = center(d.bbox);
  await clickAt(c.x, c.y, button);
  return d;
}
export async function hoverT(t) {
  const d = await loc(t);
  if (!d) throw new Error(`not found: ${JSON.stringify(t)}`);
  const c = center(d.bbox);
  await moveTo(c.x, c.y);
  return d;
}
export const wheelAt = (x, y, deltaY) => mouse("mouseWheel", x, y, { deltaX: 0, deltaY });
export async function drag(x0, y0, x1, y1, n = 10) {
  await moveTo(x0, y0);
  await down(x0, y0);
  for (let i = 1; i <= n; i++) await moveTo(x0 + ((x1 - x0) * i) / n, y0 + ((y1 - y0) * i) / n, 1);
  await up(x1, y1);
}
const MOD = { alt: 1, ctrl: 2, meta: 4, shift: 8 };
const NAMED = { Enter: [13, "Enter", "\r"], Escape: [27, "Escape"], Tab: [9, "Tab"], ArrowDown: [40, "ArrowDown"], ArrowUp: [38, "ArrowUp"], ArrowLeft: [37, "ArrowLeft"], ArrowRight: [39, "ArrowRight"], Backspace: [8, "Backspace"], Home: [36, "Home"], End: [35, "End"], PageDown: [34, "PageDown"] };
const PUNCT = { "/": [191, "Slash"], ",": [188, "Comma"], ".": [190, "Period"], "[": [219, "BracketLeft"], "]": [221, "BracketRight"], "=": [187, "Equal"], "-": [189, "Minus"], "\\": [220, "Backslash"], "'": [222, "Quote"], ";": [186, "Semicolon"], "`": [192, "Backquote"] };
export async function key(k, mods = []) {
  const m = mods.reduce((a, x) => a | MOD[x], 0);
  const cmd = mods.some((x) => x === "meta" || x === "ctrl");
  let kc, code, text;
  if (NAMED[k]) [kc, code, text] = NAMED[k];
  else if (PUNCT[k]) [kc, code] = PUNCT[k];
  else if (/^[a-z]$/i.test(k)) [kc, code] = [k.toUpperCase().charCodeAt(0), `Key${k.toUpperCase()}`];
  else if (/^\d$/.test(k)) [kc, code] = [48 + +k, `Digit${k}`];
  if (!NAMED[k] && !cmd) text = mods.includes("shift") ? k.toUpperCase() : k;
  const keyName = mods.includes("shift") && /^[a-z]$/.test(k) ? k.toUpperCase() : k;
  await P.send("Input.dispatchKeyEvent", { type: text ? "keyDown" : "rawKeyDown", key: keyName, code, windowsVirtualKeyCode: kc, nativeVirtualKeyCode: kc, modifiers: m, text, unmodifiedText: text });
  await P.send("Input.dispatchKeyEvent", { type: "keyUp", key: keyName, code, windowsVirtualKeyCode: kc, nativeVirtualKeyCode: kc, modifiers: m });
}
export const typeText = (t) => P.send("Input.insertText", { text: t });
export async function shot(name) {
  const { data } = await P.send("Page.captureScreenshot", { format: "png" });
  const p = `${SHOT}/${name}.png`;
  fs.writeFileSync(p, Buffer.from(data, "base64"));
  return p;
}
export const pnow = () => ev("performance.now()|0");
// Stub-bridge messages since page time t, minus noise (log/telemetry).
export const reqs = (t = 0) =>
  ev(`(window.__r1req||[]).filter(r => r.t >= ${t}).filter(r => !/log-message|performanceTelemetry|analytics/.test(JSON.stringify(r.body).slice(0,200))).map(r => r.path.endsWith('svc') ? {svc: r.body.svc + '.' + r.body.method, args: JSON.stringify(r.body.args).slice(0,200)} : r.path.endsWith('worker') ? {worker: r.body.request?.method} : {type: r.body.type, method: r.body.request?.method, url: r.body.url, key: r.body.key, params: r.body.request?.params ? JSON.stringify(r.body.request.params).slice(0,200) : undefined})`);
export const save = (name, obj) => ({ name, viewport: VIEWPORT, ...obj });

// --- Determinism run: `node lib.mjs record <out.json>` records 01 and 09 twice (needs start.sh and
// the chrome tier) into {name, runs: [[rec01, rec09], [rec01, rec09]]}; the runs must match.
const ROW = '[data-app-action-sidebar-thread-row][aria-label^="Change the greeting"]';
const HANDLE = "[role=separator][aria-orientation=vertical]";
const probe = (sel) => ev(`(() => { const e = __find({selector: ${JSON.stringify(sel)}}); return e && {bbox: __bbox(e), computed: __cs(e)}; })()`);
const tips = () => ev(`[...document.querySelectorAll('[role=tooltip]')].filter((e) => e.getBoundingClientRect().width > 0).map((e) => ({text: e.innerText.trim().replace(/\\s*\\n\\s*/g, ' | '), bbox: __bbox(e)}))`);
// Background bridge traffic (httpFetch, swipe state, attachment lists) varies per run; keep persisted writes.
const keep = (r) => r.type === "persisted-atom-update";
const only = (c) => ({ backgroundColor: c.backgroundColor, color: c.color, opacity: c.opacity, cursor: c.cursor });

async function rec01() {
  await open();
  const rest = await probe(ROW);
  const at = { x: 150, y: rest.bbox.y + 15 };
  await moveTo(at.x, at.y);
  await sleep(800);
  const hover = await probe(ROW);
  const group = await probe(`${ROW} .opacity-0.group-hover\\:opacity-100`);
  const pin = await loc({ selector: `${ROW} [aria-label="Pin chat"]` });
  const archive = await loc({ selector: `${ROW} [aria-label="Archive chat"]` });
  const marquee = await ev(`(() => { const t = document.querySelector(${JSON.stringify(ROW + " ._track_a3okg_28")}); const s = t && getComputedStyle(t); return s && {animationName: s.animationName, duration: s.animationDuration}; })()`);
  const rowTip = await tips();
  const t0 = await pnow();
  const c = center(pin.bbox);
  await moveTo(c.x, c.y);
  await sleep(600);
  const pinTip = await tips();
  const requests = (await reqs(t0)).filter((r) => /^thread\/(archive|pin|name)/.test(r.method || "")); // hover must write nothing
  const out = {
    steps: [
      { type: "hover", target: { selector: ROW, bbox: rest.bbox }, at },
      { type: "hover", target: { selector: `${ROW} [aria-label="Pin chat"]`, bbox: pin.bbox } },
    ],
    result: {
      dom: [
        { selector: ROW, state: "rest", bbox: rest.bbox, computed: only(rest.computed) },
        { selector: ROW, state: "hover", bbox: hover.bbox, computed: only(hover.computed) },
        { selector: "row action group", state: "hover", opacity: group.computed.opacity, children: [{ aria: "Pin chat", bbox: pin.bbox }, { aria: "Archive chat", bbox: archive.bbox }] },
        { selector: "row title track", state: "hover", marquee },
        { selector: "[role=tooltip] (row hover card)", tooltips: rowTip },
        { selector: "[role=tooltip] (Pin chat)", tooltips: pinTip },
      ],
      requests,
    },
  };
  await close();
  return save("01-sidebar-row-hover", out);
}

async function rec09() {
  await open();
  const sample = async (x, ms = 120) => {
    await moveTo(x, 400, 1);
    await sleep(ms);
    const h = await probe(HANDLE);
    const row = await probe(ROW);
    return { pointerX: x, handleX: h ? h.bbox.x : null, rowW: row ? row.bbox.w : 0 };
  };
  const handle = await probe(HANDLE);
  const line = () => probe(".sidebar-resize-handle-line").then((p) => p && { opacity: p.computed.opacity, bg: p.computed.backgroundColor, bbox: p.bbox });
  const lineRest = await line();
  await moveTo(340, 400);
  await sleep(800);
  const lineHover = await line();
  const t0 = await pnow();
  await down(340, 400);
  const grow = [];
  for (const x of [360, 380, 400]) grow.push(await sample(x));
  const lineDrag = await line();
  await up(400, 400);
  await sleep(200);
  await down(400, 400);
  const shrink = [];
  for (const x of [350, 300, 260, 200, 110]) shrink.push(await sample(x, x < 150 ? 800 : 120)); // the collapse animates
  await up(110, 400);
  await sleep(200);
  const collapsed = !(await probe(HANDLE));
  await clickAt(170, 22);
  await sleep(500);
  const reopened = await probe(HANDLE);
  await down(reopened.bbox.x + 8, 400);
  const grow2 = [];
  for (const x of [500, 600, 900]) grow2.push(await sample(x));
  await up(900, 400);
  await sleep(300);
  const requests = (await reqs(t0)).filter((r) => keep(r) && r.key === "sidebar-width");
  const out = {
    steps: [
      { type: "hover", target: { selector: HANDLE, bbox: handle.bbox }, at: { x: 340, y: 400 } },
      { type: "drag", from: { x: 340, y: 400 }, to: { x: 400, y: 400 } },
      { type: "drag", from: { x: 400, y: 400 }, to: { x: 110, y: 400 } },
      { type: "click", target: { text: "Toggle sidebar (titlebar)", at: { x: 170, y: 22 } } },
      { type: "drag", from: { x: reopened.bbox.x + 8, y: 400 }, to: { x: 900, y: 400 } },
    ],
    result: {
      dom: [
        { selector: HANDLE, bbox: handle.bbox, cursor: handle.computed.cursor },
        { selector: ".sidebar-resize-handle-line", rest: lineRest, hover: lineHover, dragging: lineDrag },
        { selector: "drag +60", samples: grow },
        { selector: "min / collapse", samples: shrink, collapsed },
        { selector: "max", reopenedHandleX: reopened.bbox.x, samples: grow2 },
      ],
      requests,
    },
  };
  await close();
  return save("09-sidebar-resize-drag", out);
}

if (process.argv[2] === "record" && process.argv[3]) {
  const runs = [];
  for (let i = 0; i < 2; i++) runs.push([await rec01(), await rec09()]);
  fs.writeFileSync(process.argv[3], `${JSON.stringify({ name: "f0a", runs }, null, 1)}\n`);
  process.exit(0);
}
