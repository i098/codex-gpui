// Captures the two error-shell reference states from the pinned original renderer:
//
//   crash-page — boot with an empty oracle (every shared value misses): the renderer
//                lands on its full-screen error boundary (uBn at defaults).
//   loader     — SIGSTOP the harness server right after the document loads: no IPC
//                call ever answers, so the startup loader (blossom on the window
//                background) holds.
//
// Writes refs/<state>/dark@1280x832.png and .spec.json under R1_OUT, with the same
// freeze, SVG reload and memory-pressure discipline as capture.mjs.
// Run under tools/route1/lease.sh with R1_CAPTURE_CDP_URLS set.
import fs from "node:fs";
import path from "node:path";
import { openPage } from "./chrome.mjs";

const SERVER = process.env.R1_SERVER;
const OUT = process.env.R1_OUT;
const LOADER_PID = +(process.env.R1_LOADER_SERVER_PID || 0);
const UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const w = 1280, h = 832;

const FREEZE = `(() => {
  for (const a of document.getAnimations()) {
    try { if (a.effect.getComputedTiming().endTime === Infinity) { a.pause(); a.currentTime = 0; } else a.finish(); } catch {}
  }
  if (!document.getElementById("r1-freeze")) {
    const s = document.createElement("style");
    s.id = "r1-freeze";
    s.textContent = "*{caret-color:transparent!important}";
    document.head.append(s);
  }
})()`;
const RELOAD_SVG_IMAGES = `Promise.all([...document.images].filter((i) => /\\.svg([?#]|$)/.test(i.currentSrc)).map((i) => {
  i.src = i.currentSrc.replace(/#.*$/, "") + "#r1";
  return i.decode().catch(() => {});
}))`;
const spec = fs.readFileSync(new URL("../capture/spec.js", import.meta.url), "utf8");

async function shoot(p, state) {
  await p.evaluate(FREEZE);
  await p.evaluate(RELOAD_SVG_IMAGES);
  await p.send("Memory.simulatePressureNotification", { level: "critical" });
  await sleep(500);
  const shot = await p.send("Page.captureScreenshot", { format: "png" });
  const parsed = JSON.parse(await p.evaluate(spec));
  const dir = path.join(OUT, state);
  fs.mkdirSync(dir, { recursive: true });
  const name = path.join(dir, `dark@${w}x${h}`);
  fs.writeFileSync(`${name}.png`, Buffer.from(shot.data, "base64"));
  fs.writeFileSync(`${name}.spec.json`, JSON.stringify(parsed, null, 1));
  console.log("captured", state);
}

async function boot(p, server) {
  await p.send("Network.enable");
  await p.send("Emulation.setUserAgentOverride", {
    userAgent: UA, platform: "MacIntel",
    userAgentMetadata: { brands: [{ brand: "Chromium", version: "155" }], fullVersion: "155.0.0.0", platform: "macOS", platformVersion: "26.2.0", architecture: "arm", model: "", mobile: false },
  });
  await p.send("Emulation.setTimezoneOverride", { timezoneId: "America/Los_Angeles" });
  await p.send("Emulation.setLocaleOverride", { locale: "en-US" });
  await p.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "dark" }] });
  await p.send("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: 0, mobile: false });
  await p.send("Emulation.setScrollbarsHidden", { hidden: true });
  await p.send("Page.navigate", { url: `${server}/index.html?theme=dark` });
}

// Crash page: the empty-oracle server at R1_SERVER (R1_ORACLE={}), 25 s to boot into the boundary.
{
  const p = await openPage();
  try {
    await boot(p, SERVER);
    await sleep(25000);
    const text = await p.evaluate(`document.body.innerText`);
    if (!text.includes("hit a snag")) throw new Error(`crash page did not appear: ${JSON.stringify(text.slice(0, 200))}`);
    await shoot(p, "crash-page");
  } finally { await p.close(); }
}

// Loader: the full-oracle server, frozen as soon as the document exists; every later
// IPC call hangs, so the loader holds. Resumed afterwards so the lease leaves it running.
{
  const p = await openPage();
  try {
    await boot(p, process.env.R1_LOADER_SERVER || SERVER);
    // Assets and the splash take a few seconds through the tunnel; the first hold
    // experiment proved 4 s is enough for the blossom, and the freeze then holds it.
    await sleep(4000);
    // The startup CSS is transparent; the native window's backgroundColor shows
    // through in the real app (`#202020` dark, from the original main bundle).
    await p.evaluate(`document.documentElement.style.background = "#202020"`);
    if (LOADER_PID) process.kill(LOADER_PID, "SIGSTOP");
    await sleep(4000);
    const text = await p.evaluate(`document.body.innerText`);
    if (text.includes("hit a snag") || text.includes("New chat")) throw new Error(`loader did not hold: ${JSON.stringify(text.slice(0, 200))}`);
    await shoot(p, "loader");
  } finally {
    if (LOADER_PID) process.kill(LOADER_PID, "SIGCONT");
    await p.close();
  }
}
