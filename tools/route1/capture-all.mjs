// Captures all 36 references (9 screens x dark/light x 1280x832/1440x900) one after another.
// Needs `node tools/route1/server.mjs` running, or R1_APP_CDP (app-capture.sh). R1_THEMES=dark,light picks themes.
// Usage: `node tools/route1/capture-all.mjs [screen...]`.
import { capture } from "./capture.mjs";

const screens = process.argv.slice(2).length ? process.argv.slice(2) : ["home", "sidebar", "thread", "composer", "diff", "settings", "automations", "terminal", "mobile"];
let failed = 0;
for (const screen of screens)
  for (const theme of (process.env.R1_THEMES || "dark,light").split(","))
    for (const [w, h] of [[1280, 832], [1440, 900]])
      try {
        console.log(await capture(screen, theme, w, h, process.env.R1_OUT));
      } catch (e) {
        failed++;
        console.error(`FAIL ${screen} ${theme} ${w}x${h}: ${e.message.slice(0, 300)}`);
      }
process.exit(failed ? 1 : 0);
