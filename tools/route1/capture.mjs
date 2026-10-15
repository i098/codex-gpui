// Captures one reference from the original renderer served by server.mjs:
// `node tools/route1/capture.mjs <screen> <theme> <width> <height>` writes
// refs/<screen>/<theme>@<w>x<h>.png and .spec.json. The page runs in a fresh isolated context
// of the Mac's headless shell (Mac UA, America/Los_Angeles, en-US, native 2x), and reaches the
// state by clicking.
import fs from "node:fs";
import path from "node:path";
import { openPage } from "./chrome.mjs";

const SERVER = process.env.R1_SERVER || "http://127.0.0.1:4517";
const UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36";
const read = (f) => fs.readFileSync(new URL(f, import.meta.url), "utf8");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// JS expression for the first visible element whose aria-label or first text line is `t`
// (menu items carry their shortcut on a second line).
const byText = (t, sel = "button,a,[role=button],[role=menuitem],[role=menuitemradio],[role=option],[role=tab],[role=link]") =>
  `[...document.querySelectorAll(${JSON.stringify(sel)})].find((e) => e.offsetParent !== null && ((e.getAttribute("aria-label") || "").trim() === ${JSON.stringify(t)} || e.innerText?.trim().split("\\n")[0].trim() === ${JSON.stringify(t)}))`;
const row = (prefix) => `[...document.querySelectorAll("nav [data-thread-title]")].find((e) => e.textContent.trim().startsWith(${JSON.stringify(prefix)}))`;

function driver(p, w, h) {
  const waitFor = async (expr, what = expr, ms = 30000) => {
    for (const end = Date.now() + ms; Date.now() < end; await sleep(200)) if (await p.evaluate(`!!(${expr})`)) return;
    throw new Error(`timeout waiting for ${what}`);
  };
  const mouse = (type, x, y, extra = {}) => p.send("Input.dispatchMouseEvent", { type, x, y, ...extra });
  const click = async (expr, what = expr) => {
    await waitFor(expr, what);
    const [x, y] = await p.evaluate(`(() => { const r = (${expr}).getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; })()`);
    await mouse("mouseMoved", x, y);
    await mouse("mousePressed", x, y, { button: "left", clickCount: 1 });
    await mouse("mouseReleased", x, y, { button: "left", clickCount: 1 });
    await sleep(300);
  };
  const key = async (key, code = key, keyCode = 0) => {
    await p.send("Input.dispatchKeyEvent", { type: "keyDown", key, code, windowsVirtualKeyCode: keyCode });
    await p.send("Input.dispatchKeyEvent", { type: "keyUp", key, code, windowsVirtualKeyCode: keyCode });
  };
  const type = (text) => p.send("Input.insertText", { text });
  // The pointer rests on the empty titlebar centre, where nothing reacts to hover.
  const park = () => mouse("mouseMoved", Math.round(w * 0.6), 6);
  return { waitFor, click, key, type, park, sleep };
}

// Each state starts on a freshly loaded Home route.
const STATES = {
  home: async () => {},
  sidebar: async (d) => {
    await d.click(byText("Search"));
    // The Home composer keeps focus; the search field must have it before typing.
    await d.click(`document.querySelector("[cmdk-input]")`, "search field");
    await d.type("math");
    await d.waitFor(`document.querySelector("[cmdk-input]")?.value === "math"`, "search query");
    await d.waitFor(`(() => { const t = document.querySelector("[cmdk-list]")?.innerText ?? ""; return t.includes("subtract") && !t.includes("greeting"); })()`, "filtered results");
  },
  thread: async (d) => {
    await d.click(row("Add a subtract"));
    await d.waitFor(`document.body.innerText.includes("Worked for") && document.body.innerText.includes("View changes")`, "subtract thread");
  },
  composer: async (d) => {
    await d.click(`document.querySelector(".ProseMirror")`, "composer");
    await d.type("Refactor src/math.py");
    await d.click(`[...document.querySelectorAll("button")].find((b) => b.offsetParent !== null && b.innerText.trim().startsWith("GPT-6 Astra"))`, "model picker");
    await d.waitFor(`document.querySelector("[role=menu]")`, "model menu");
  },
  diff: async (d) => {
    await STATES.thread(d);
    await d.click(byText("View changes"));
    // The hunk body does not render here (header row only); README explains why.
    await d.waitFor(`/Last [Tt]urn/.test(document.body.innerText)`, "diff panel");
  },
  settings: async (d) => {
    await d.click(byText("Open profile menu"));
    await d.click(byText("Settings"));
    await d.waitFor(`document.body.innerText.includes("Default permissions")`, "settings general");
  },
  automations: async (d) => {
    await d.click(byText("Scheduled"));
    await d.waitFor(`document.body.innerText.includes("Scheduled") && !document.body.innerText.includes("Loading scheduled")`, "automations page");
  },
  terminal: async (d) => {
    await STATES.thread(d);
    // server.mjs replays the subtract session's python3 check as still running (README).
    await d.waitFor(`fetch("/__r1/replay-running-command?sid=" + __R1__.sid).then(async (r) => (await r.text(), r.ok))`, "replay");
    await d.waitFor(`[...document.querySelectorAll("button")].some((b) => b.getAttribute("aria-label") === "Stop all background terminals")`, "background terminal list");
    // The list lives in the thread summary panel (header `Toggle summary`); its row opens the
    // `Background terminal` side tab. The panel slides in, so the row is clicked once it is hit.
    await d.click(byText("Toggle summary"));
    const entry = `[...document.querySelectorAll("[data-slot=thread-summary-panel-item-group] button")].find((b) => b.innerText.trim().startsWith("python3"))`;
    await d.waitFor(`(() => { const b = ${entry}; if (!b) return false; const r = b.getBoundingClientRect(); return b.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2)); })()`, "summary panel row");
    await d.click(entry, "background terminal row");
    await d.waitFor(`document.querySelector(".xterm") && [...document.querySelectorAll("[role=tab]")].some((t) => t.innerText.trim().startsWith("python3"))`, "background terminal tab");
  },
  mobile: async (d) => {
    await d.click(byText("Open profile menu"));
    await d.click(byText("Help"));
    await d.click(byText("Set up mobile"));
    await d.waitFor(`document.body.innerText.includes("Connect your phone to this Mac")`, "mobile setup");
  },
};

// Blink phase and running animations depend on wall time, so they are fixed before the shot:
// finite animations jump to their end, infinite ones (spinners, shimmers) rest at t=0, and the
// caret is transparent (its blink phase is arbitrary in any still; the port draws its own).
const FREEZE = `(() => {
  for (const a of document.getAnimations()) {
    try {
      if (a.effect.getComputedTiming().endTime === Infinity) { a.pause(); a.currentTime = 0; } else a.finish();
    } catch {}
  }
  if (!document.getElementById("r1-freeze")) {
    const s = document.createElement("style");
    s.id = "r1-freeze";
    s.textContent = "*{caret-color:transparent!important}";
    document.head.append(s);
  }
})()`;
// An SVG <img> keeps the raster size its document got while the page was still laying out, and
// that differs between loads by a few levels at edges. Loading each one again (new URL fragment)
// on the still page renders it at its final size.
const RELOAD_SVG_IMAGES = `Promise.all([...document.images].filter((i) => /\\.svg([?#]|$)/.test(i.currentSrc)).map((i) => {
  i.src = i.currentSrc.replace(/#.*$/, "") + "#r1";
  return i.decode().catch(() => {});
}))`;
// Marks DOM mutations in page time; settle() reads how long the DOM has been quiet.
const WATCH_DOM = `(() => {
  if (window.__r1dom === undefined) {
    window.__r1dom = performance.now();
    new MutationObserver(() => (window.__r1dom = performance.now())).observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  }
})()`;
// Text that CSS generates (::before/::after `content`, e.g. the composer placeholder) has no text
// node, so spec.js misses it. Each one is measured as a real span carrying the pseudo element's
// computed style (pseudo switched off meanwhile), after the screenshot; DOM state is restored.
const PSEUDO_TEXTS = `(() => {
  const r4 = (r) => [r.x, r.y, r.width, r.height].map((v) => Math.round(v * 100) / 100);
  const off = document.createElement("style");
  off.textContent = ".r1-nopseudo::before,.r1-nopseudo::after{content:none!important}";
  document.head.append(off);
  const out = [];
  for (const el of document.body.querySelectorAll("*")) {
    if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) continue;
    for (const which of ["before", "after"]) {
      const ps = getComputedStyle(el, "::" + which);
      const attr = /^attr\\(([^)]+)\\)$/.exec(ps.content);
      const text = attr ? el.getAttribute(attr[1].trim()) : /^"(.*)"$/s.exec(ps.content)?.[1];
      if (!text?.trim() || ps.display === "none" || ps.visibility === "hidden" || +ps.opacity === 0) continue;
      const span = document.createElement("span");
      for (const k of ps) span.style.setProperty(k, ps.getPropertyValue(k));
      span.style.content = "normal";
      span.textContent = text;
      el.classList.add("r1-nopseudo");
      which === "before" ? el.prepend(span) : el.append(span);
      const range = document.createRange();
      range.selectNodeContents(span.firstChild);
      for (const tr of range.getClientRects())
        if (tr.width && tr.height)
          out.push({ owner: [el.tagName.toLowerCase(), ...r4(el.getBoundingClientRect())], pseudo: which, text: text.trim().slice(0, 200), rect: r4(tr), font: ps.fontWeight + " " + ps.fontSize + "/" + ps.lineHeight + " " + ps.fontFamily, color: ps.color });
      span.remove();
      el.classList.remove("r1-nopseudo");
    }
  }
  off.remove();
  return out;
})()`;

export async function capture(screen, theme, w, h, outDir = new URL("../../refs/", import.meta.url).pathname) {
  if (!STATES[screen]) throw new Error(`unknown screen ${screen}; one of ${Object.keys(STATES).join(", ")}`);
  const p = await openPage();
  // In-flight http requests of the document and the time of the last network or server event;
  // settle() needs DOM and network quiet. Not tracked: EventSource (never ends), blob:/data: URLs
  // and worker scripts (empty loaderId), whose end is reported to other targets or not at all.
  const net = { inflight: new Map(), last: Date.now() };
  p.on((x) => {
    if (x.sessionId !== p.sessionId || !x.method?.startsWith("Network.")) return;
    net.last = Date.now();
    if (x.method === "Network.requestWillBeSent" && x.params.type !== "EventSource" && x.params.loaderId && /^https?:/.test(x.params.request.url)) net.inflight.set(x.params.requestId, `${x.params.type} ${x.params.request.url}`);
    if (x.method === "Network.loadingFinished" || x.method === "Network.loadingFailed") net.inflight.delete(x.params.requestId);
  });
  // Resolves once DOM and network have been quiet for `quiet` ms with fonts and images loaded.
  // A slow page (the Mac renders through ssh tunnels) pauses on lazy chunks and app-server
  // answers; a DOM-only check would shoot in such a pause.
  const settle = async (quiet) => {
    await p.evaluate(WATCH_DOM);
    let state;
    for (const t0 = Date.now(); Date.now() - t0 < 60000; await sleep(100)) {
      const [domIdle, loaded] = await p.evaluate(`[performance.now() - __r1dom, document.fonts.status === "loaded" && [...document.images].every((i) => i.complete)]`);
      state = { domIdle, loaded, netIdle: Date.now() - net.last, inflight: [...net.inflight.values()].map((u) => u.slice(0, 120)) };
      if (loaded && domIdle > quiet && !net.inflight.size && state.netIdle > quiet) return;
    }
    throw new Error(`page did not settle in 60 s: ${JSON.stringify(state)}`);
  };
  try {
    await p.send("Network.enable");
    await p.send("Emulation.setUserAgentOverride", {
      userAgent: UA,
      platform: "MacIntel",
      userAgentMetadata: { brands: [{ brand: "Chromium", version: "155" }], fullVersion: "155.0.0.0", platform: "macOS", platformVersion: "26.2.0", architecture: "arm", model: "", mobile: false },
    });
    await p.send("Emulation.setTimezoneOverride", { timezoneId: "America/Los_Angeles" });
    await p.send("Emulation.setLocaleOverride", { locale: "en-US" });
    await p.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: theme }] });
    // Scale 2 comes from the shell's --force-device-scale-factor=2 (layout in device px, as in the
    // app); emulating the scale factor would lay out in CSS px and snap boxes differently.
    await p.send("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: 0, mobile: false });
    // macOS overlay scrollbars are invisible at rest.
    await p.send("Emulation.setScrollbarsHidden", { hidden: true });
    await p.send("Page.navigate", { url: `${SERVER}/index.html?theme=${theme}` });
    const d = driver(p, w, h);
    await d.waitFor(`document.querySelector("nav[aria-label='App navigation']") && document.querySelectorAll("nav [data-thread-title]").length >= 3 && !document.body.innerText.includes("Loading model")`, "home", 90000);
    await settle(1000);
    // Announcement modals and beacon banners are dismissed the way a user would.
    for (const t of ["Continue with current model", "Dismiss ChatGPT beacon banner"])
      if (await p.evaluate(`!!(${byText(t)})`)) await d.click(byText(t));
    await STATES[screen](d).catch(async (e) => {
      const shot = await p.send("Page.captureScreenshot", { format: "png" });
      fs.writeFileSync(`/tmp/r1-fail-${screen}-${theme}.png`, Buffer.from(shot.data, "base64"));
      throw e;
    });
    // The announcement modal can come back after navigation; close it again.
    if (await p.evaluate(`!!(${byText("Continue with current model")})`)) await d.click(byText("Continue with current model"));
    await d.park();
    await settle(1500);
    await p.evaluate(FREEZE);
    await p.evaluate(RELOAD_SVG_IMAGES);
    // Tiles and decoded images can keep a raster from while the page was still moving. A critical
    // memory-pressure signal drops them; the shot then rasterizes the still page from scratch.
    await p.send("Memory.simulatePressureNotification", { level: "critical" });
    await sleep(500);
    const shot = await p.send("Page.captureScreenshot", { format: "png" });
    const spec = JSON.parse(await p.evaluate(read("../capture/spec.js")));
    for (const { owner, ...t } of await p.evaluate(PSEUDO_TEXTS)) {
      const node = spec.nodes.find((n) => n.tag === owner[0] && n.rect.every((v, i) => v === owner[i + 1]));
      spec.texts.push({ node: node?.id ?? -1, ...t });
    }
    fs.mkdirSync(path.join(outDir, screen), { recursive: true });
    const name = path.join(outDir, screen, `${theme}@${w}x${h}`);
    fs.writeFileSync(`${name}.png`, Buffer.from(shot.data, "base64"));
    fs.writeFileSync(`${name}.spec.json`, JSON.stringify(spec, null, 1));
    return `${name}.png`;
  } finally {
    await p.close();
  }
}

if (process.argv[1] === new URL(import.meta.url).pathname) {
  const [screen, theme, w, h] = process.argv.slice(2);
  console.log(await capture(screen, theme, +w, +h, process.env.R1_OUT));
}
