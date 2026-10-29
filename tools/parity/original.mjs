import fs from "node:fs";
import path from "node:path";
import { capture } from "../route1/capture.mjs";
import { openPage } from "../route1/chrome.mjs";

const [surface, state, theme, width, height, output] = process.argv.slice(2);
const w = Number(width), h = Number(height);
if (surface === "sidebar" && state === "rest") {
  // The Home route shows the thread list, search and project groups.
  await capture("home", theme, w, h, output);
} else if ((state === "rest" && ["home", "terminal", "thread"].includes(surface)) || (surface === "automations" && state === "empty")) {
  await capture(surface, theme, w, h, output);
} else if (surface === "mobile" && state === "get-started") {
  await capture("mobile", theme, w, h, output);
} else if (surface === "mobile" && state === "continuation") {
  await capture("mobile-continuation", theme, w, h, output);
} else if (surface === "settings" && state === "configuration") {
  await configuration();
} else {
  throw new Error(`No original recipe for ${surface}/${state}`);
}

// Configuration is not a state in capture.mjs. Keep its native-font capture recipe.
async function configuration() {
  const page = await openPage();
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const inflight = new Set();
  let activity = Date.now();
  page.on((event) => {
    if (event.sessionId !== page.sessionId || !event.method?.startsWith("Network.")) return;
    activity = Date.now();
    const p = event.params;
    if (event.method === "Network.requestWillBeSent" && p.type !== "EventSource" && p.loaderId && /^https?:/.test(p.request.url)) inflight.add(p.requestId);
    if (["Network.loadingFinished", "Network.loadingFailed"].includes(event.method)) inflight.delete(p.requestId);
  });
  const waitFor = async (expression) => {
    for (const end = Date.now() + 90000; Date.now() < end; await sleep(100)) {
      if (await page.evaluate(expression)) return;
    }
    throw new Error(`Timed out: ${expression}`);
  };
  const click = async (label) => {
    const element = `[...document.querySelectorAll('button,a,[role=menuitem],[role=button]')].find(e => e.offsetParent !== null && ((e.getAttribute('aria-label') || '') === ${JSON.stringify(label)} || e.innerText?.trim().split('\\n')[0] === ${JSON.stringify(label)}))`;
    await waitFor(`!!(${element})`);
    const [x, y] = await page.evaluate(`(() => { const r=(${element}).getBoundingClientRect(); return [r.x+r.width/2,r.y+r.height/2]; })()`);
    await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
    for (const type of ["mousePressed", "mouseReleased"]) await page.send("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1 });
    await sleep(300);
  };
  const settle = async (quiet) => {
    await page.evaluate(`(() => {
      if (window.__r1dom === undefined) {
        window.__r1dom = performance.now();
        new MutationObserver(() => window.__r1dom = performance.now()).observe(document, {subtree:true,childList:true,attributes:true,characterData:true});
      }
    })()`);
    for (const end = Date.now() + 60000; Date.now() < end; await sleep(100)) {
      const ready = await page.evaluate(`performance.now()-__r1dom > ${quiet} && document.fonts.status === 'loaded' && [...document.images].every(i=>i.complete)`);
      if (ready && !inflight.size && Date.now() - activity > quiet) return;
    }
    throw new Error("Configuration did not settle");
  };
  try {
    await page.send("Network.enable");
    await page.send("Emulation.setUserAgentOverride", {
      userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36",
      platform: "MacIntel",
      userAgentMetadata: { brands: [{ brand: "Chromium", version: "155" }], fullVersion: "155.0.0.0", platform: "macOS", platformVersion: "26.2.0", architecture: "arm", model: "", mobile: false },
    });
    await page.send("Emulation.setTimezoneOverride", { timezoneId: "America/Los_Angeles" });
    await page.send("Emulation.setLocaleOverride", { locale: "en-US" });
    await page.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: theme }] });
    await page.send("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: 0, mobile: false });
    await page.send("Emulation.setScrollbarsHidden", { hidden: true });
    await page.send("Page.navigate", { url: `${process.env.R1_SERVER}/index.html?theme=${theme}` });
    await waitFor(`document.querySelectorAll('nav [data-thread-title]').length >= 3 && !document.body.innerText.includes('Loading model')`);
    await settle(1000);
    if (await page.evaluate(`document.body.innerText.includes('Continue with current model')`)) await click("Continue with current model");
    await click("Open profile menu");
    await click("Settings");
    await waitFor(`document.body.innerText.includes('Default permissions')`);
    await click("Configuration");
    await waitFor(`document.body.innerText.includes('Configure permissions, web access, and agent responses for new chats') || document.body.innerText.includes('Choose how ChatGPT works on your tasks')`);
    await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: Math.round(w * 0.6), y: 6 });
    await settle(1500);
    await page.evaluate(`(() => {
      for (const animation of document.getAnimations()) {
        try { if (animation.effect.getComputedTiming().endTime === Infinity) { animation.pause(); animation.currentTime=0; } else animation.finish(); } catch {}
      }
      const style=document.createElement('style'); style.textContent='*{caret-color:transparent!important}'; document.head.append(style);
    })()`);
    await page.evaluate(`Promise.all([...document.images].filter(i=>/\\.svg([?#]|$)/.test(i.currentSrc)).map(i=>{i.src=i.currentSrc.replace(/#.*$/,'')+'#r1';return i.decode().catch(()=>{});} ))`);
    await page.send("Memory.simulatePressureNotification", { level: "critical" });
    await sleep(500);
    const shot = await page.send("Page.captureScreenshot", { format: "png" });
    const spec = JSON.parse(await page.evaluate(fs.readFileSync(new URL("../capture/spec.js", import.meta.url), "utf8")));
    // This page has no generated text. Refuse a new generated label rather than omit its mask.
    const generated = await page.evaluate(`(() => {
      const result=[];
      for (const e of document.body.querySelectorAll('*')) {
        if (!e.checkVisibility({opacityProperty:true,visibilityProperty:true})) continue;
        for (const pseudo of ['::before','::after']) {
          const s=getComputedStyle(e,pseudo);
          if (s.display!=='none' && s.visibility!=='hidden' && +s.opacity!==0 && !['none','normal','""'].includes(s.content)) result.push(s.content);
        }
      }
      return result;
    })()`);
    if (generated.length) throw new Error(`Unmeasured generated text: ${generated.join(', ')}`);
    const stem = path.join(output, surface, `${theme}@${w}x${h}`);
    fs.mkdirSync(path.dirname(stem), { recursive: true });
    fs.writeFileSync(`${stem}.png`, Buffer.from(shot.data, "base64"));
    fs.writeFileSync(`${stem}.spec.json`, JSON.stringify(spec, null, 1));
  } catch (error) {
    fs.mkdirSync(output, { recursive: true });
    const shot = await page.send("Page.captureScreenshot", { format: "png" });
    fs.writeFileSync(path.join(output, "configuration-failure.png"), Buffer.from(shot.data, "base64"));
    fs.writeFileSync(path.join(output, "configuration-failure.json"), JSON.stringify(await page.evaluate(`({url:location.href,text:document.body.innerText})`), null, 2));
    throw error;
  } finally {
    await page.close();
  }
}
