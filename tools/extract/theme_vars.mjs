// Runs the original app's theme code (the part of app-initial-*.js that builds the inline
// :root custom properties from the Codex base theme) and prints {dark:{...}, light:{...}}.
// Usage: node tools/extract/theme_vars.mjs /tmp/cgx/asar-x/webview/assets
import fs from "node:fs";
import path from "node:path";

const dir = process.argv[2] ?? "/tmp/cgx/asar-x/webview/assets";
const file = fs.readdirSync(dir).find((f) => /^app-initial-.*\.js$/.test(f));
const src = fs.readFileSync(path.join(dir, file), "utf8");
const start = src.lastIndexOf("function ", src.indexOf("pya={light"));
const end = src.indexOf("})))()}", src.indexOf("function ixa(){")) + 7;
const region = src.slice(start, end).replaceAll("import.meta", "({env:{}})");
// Names declared in the region stay local; every other free name resolves to a no-op.
const local = new Set([
  ...[...region.matchAll(/function ([\w$]+)\(/g)].map((m) => m[1]),
  ...[...region.matchAll(/var ([\w$,]+);/g)].flatMap((m) => m[1].split(",")),
]);
const lazy = (fn, res) => () => (fn && (res = fn((fn = 0))), res);
const scope = new Proxy(
  { e: lazy },
  {
    has: (t, k) => typeof k === "string" && !local.has(k) && !(k in globalThis && k !== "e"),
    get: (t, k) => (k in t ? t[k] : () => {}),
  },
);
const run = new Function("scope", `with (scope) { ${region}; return { KJ, Aba, ixa }; }`);
const { KJ, Aba, ixa } = run(scope);
ixa();

const out = {};
for (const variant of ["dark", "light"]) {
  // The reference captures use the ChatGPT accent "black" (selection #78787866).
  out[variant] = Aba(KJ(undefined, variant, "black"), variant, "black");
}
process.stdout.write(JSON.stringify(out, null, 1));
