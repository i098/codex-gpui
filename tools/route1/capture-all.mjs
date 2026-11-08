// Captures all 36 references (9 screens x dark/light x 1280x832/1440x900) one after another.
// Needs `node tools/route1/server.mjs` running, or R1_APP_CDP (app-capture.sh). R1_THEMES=dark,light picks themes.
// Usage: `node tools/route1/capture-all.mjs [screen...]`.
import { capture } from "./capture.mjs";
import { pixelR } from "./ring-probe.mjs";

// The page frame's 0.5 px ring is rastered one of two ways per capture, with identical DOM: onto the page
// ("direct": 47 dark / 229 light) or as a translucent tile ("tile": 48 / 230). Most captures are tiles and the
// native port draws a tile, so a capture whose ring pixel is not the tile value is taken again.
const TILE = { dark: { sidebar: 42, other: 48 }, light: { sidebar: 199, other: 230 } };
const ATTEMPTS = 6;
async function captureTile(screen, theme, w, h, out) {
  for (let i = 1; ; i++) {
    const file = await capture(screen, theme, w, h, out);
    const { w: pw, h: ph } = pixelR(file, 0, 0);
    const got = pixelR(file, pw >> 1, ph - 8).r;
    if (got === TILE[theme][screen === "sidebar" ? "sidebar" : "other"]) return file;
    if (i === ATTEMPTS) throw new Error(`the page frame ring stayed ${got} after ${ATTEMPTS} captures`);
    console.error(`retry ${screen} ${theme} ${w}x${h}: ring ${got}`);
  }
}

const screens = process.argv.slice(2).length ? process.argv.slice(2) : ["home", "sidebar", "thread", "composer", "diff", "settings", "automations", "terminal", "mobile"];
let failed = 0;
for (const screen of screens)
  for (const theme of (process.env.R1_THEMES || "dark,light").split(","))
    for (const [w, h] of [[1280, 832], [1440, 900]])
      try {
        console.log(await captureTile(screen, theme, w, h, process.env.R1_OUT));
      } catch (e) {
        failed++;
        console.error(`FAIL ${screen} ${theme} ${w}x${h}: ${e.message.slice(0, 300)}`);
      }
process.exit(failed ? 1 : 0);
