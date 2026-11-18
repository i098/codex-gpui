// Seeds the app's persisted UI state (sidebar caches, MCP destinations, seen flags) from the scrubbed oracle, so a
// fresh profile starts in the state the references were recorded in: `node seed.mjs <oracle.json> <codex-home>`.
import fs from "node:fs";
import path from "node:path";

const [oracle, home] = process.argv.slice(2);
const all = JSON.parse(fs.readFileSync(oracle, "utf8"))["persisted-atoms"].state;
// Restored thread tabs would open panels that a capture opens itself.
const atoms = Object.fromEntries(Object.entries(all).filter(([key]) => !key.startsWith("thread-tab-routes-v1:")));
fs.writeFileSync(path.join(home, ".codex-global-state.json"), JSON.stringify({ "electron-persisted-atom-state": atoms }));
