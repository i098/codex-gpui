// Fake ChatGPT backend for the original desktop app: `node backend-stub.mjs <port>`.
// The account routing in the fake account (MOCK_BACKEND_ORIGIN, see linux/mock-app-server.mjs)
// points the desktop main process here. Answers come from the scrubbed oracle's `httpFetch.fetch`
// recordings; the owner's chats answer an empty last page, any other unrecorded GET 404 `{}` and any other
// unrecorded write 200 `{}` (the original shows its own empty or error state), as server.mjs does.
// The desktop accepts only an https origin for workspace routing: TLS_KEY/TLS_CERT name a self-signed pair
// and the app starts with --ignore-certificate-errors. Log (one line per request): $STUB_LOG.
import fs from "node:fs";
import https from "node:https";

const PORT = +process.argv[2] || 4750;
const LOG = process.env.STUB_LOG;
const oracle = JSON.parse(fs.readFileSync(process.env.R1_ORACLE || new URL("./oracle.json", import.meta.url), "utf8"));
const rec = new Map();
for (const [key, entry] of Object.entries(oracle)) {
  if (!key.startsWith("svc httpFetch.fetch ")) continue;
  const r = entry.value?.response?.__response;
  const req = JSON.parse(key.slice("svc httpFetch.fetch ".length))[1];
  if (!r) continue;
  rec.set(`${req.method} ${req.url.replace(/^https:\/\/chatgpt\.com\/backend-api/, "")}`, r);
}
// The 26.1029 app reads the account list from this path; the 26.1007 recording holds the same answer under the old one.
rec.set("GET /wham/accounts/check", rec.get("GET /accounts/check/v4-2023-04-27"));
const EMPTY_CHATS = JSON.stringify({ items: [], total: 0, limit: 100, offset: 0, has_missing_conversations: false });
const SKIP = new Set(["content-encoding", "content-length", "transfer-encoding", "connection"]);

https.createServer({ key: fs.readFileSync(process.env.TLS_KEY), cert: fs.readFileSync(process.env.TLS_CERT) }, (req, res) => {
  const chunks = [];
  req.on("data", (c) => chunks.push(c));
  req.on("end", () => {
    const rel = req.url.replace(/^\/(?:backend-api|api)(?=\/)/, "");
    // Recorded urls may carry a different query order or ids; fall back to the bare path.
    const r = rec.get(`${req.method} ${rel}`) ?? rec.get(`${req.method} ${rel.split("?")[0]}`) ?? [...rec].find(([k]) => k.startsWith(`${req.method} ${rel.split("?")[0]}`))?.[1];
    const own = !r && (req.method === "GET" ? (rel.startsWith("/conversations?") ? { status: 200, body: EMPTY_CHATS } : { status: 404, body: "{}" }) : { status: 200, body: "{}" });
    if (LOG) fs.appendFileSync(LOG, `${new Date().toISOString().slice(11, 23)} ${req.method} ${req.url} -> ${(r ?? own).status}\n`);
    const headers = Object.fromEntries((r?.headers ?? [["content-type", "application/json"]]).filter(([k]) => !SKIP.has(k.toLowerCase())));
    res.writeHead((r ?? own).status, headers);
    res.end(!r ? own.body : r.base64 ? Buffer.from(r.base64, "base64") : typeof r.body === "string" ? r.body : JSON.stringify(r.body ?? null));
  });
}).listen(PORT, "127.0.0.1");
