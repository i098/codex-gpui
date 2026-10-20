import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { randomUUID } from "node:crypto";
import { fixtures, frozenTime, sessionOptions, backendResponse, callService, recordedValue, replayMessage } from "./backend.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const mime = { ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".html": "text/html", ".json": "application/json", ".svg": "image/svg+xml", ".png": "image/png", ".jpg": "image/jpeg", ".webp": "image/webp", ".woff2": "font/woff2", ".woff": "font/woff", ".ttf": "font/ttf", ".wasm": "application/wasm" };
const policy = "default-src 'self' data: blob:; script-src 'self' 'unsafe-inline' 'unsafe-eval' blob:; style-src 'self' 'unsafe-inline'; connect-src 'self'; form-action 'none'; base-uri 'self'";

function send(res, status, body, type = "application/json") {
  res.writeHead(status, { "content-type": type, "cache-control": "no-store", "content-security-policy": policy });
  res.end(typeof body === "object" && !Buffer.isBuffer(body) ? JSON.stringify(body) : body);
}

async function body(req) {
  const chunks = [];
  let bytes = 0;
  for await (const chunk of req) {
    bytes += chunk.length;
    if (bytes > 1024 * 1024) throw new Error("Request exceeds 1 MiB");
    chunks.push(chunk);
  }
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

export function createFixtureServer({ webview = process.env.R1_WEBVIEW } = {}) {
  const root = webview && fs.realpathSync(webview);
  const sessions = new Map();
  function getSession(url) {
    const id = url.searchParams.get("sid") ?? randomUUID();
    if (!sessions.has(id)) {
      const options = sessionOptions(url.searchParams);
      sessions.set(id, { id, options, atoms: recordedValue("persisted-atoms", options)?.state ?? {}, events: null, queue: [] });
    }
    return sessions.get(id);
  }
  function emit(session, message) {
    const data = `data: ${JSON.stringify(message)}\n\n`;
    if (session.events) session.events.write(data); else session.queue.push(data);
  }
  function serveIndex(res, session) {
    if (!root) return send(res, 503, { error: "Set R1_WEBVIEW to the pinned original webview" });
    let html = fs.readFileSync(path.join(root, "index.html"), "utf8");
    const appShared = html.match(/assets\/app-shared-[0-9a-f]+\.js/)?.[0];
    if (!appShared) throw new Error("Pinned original app-shared chunk is missing");
    const config = { sid: session.id, theme: session.options.theme, now: frozenTime, flavor: "prod", appShared: `/${appShared}`, sentry: recordedValue("sentry", session.options) };
    html = html.replace(/<meta http-equiv="Content-Security-Policy"[^>]*>/, "");
    html = html.replace("<head>", `<head><script>window.__R1__=${JSON.stringify(config)}</script><script src="/__r1/preload-stub.js"></script>`);
    send(res, 200, html, "text/html");
  }
  const server = http.createServer(async (req, res) => {
    try {
      if (!/^127\.0\.0\.1(?::\d+)?$|^localhost(?::\d+)?$/.test(req.headers.host ?? "")) return send(res, 403, { error: "Loopback Host required" });
      const url = new URL(req.url, "http://127.0.0.1");
      const p = url.pathname;
      if (!["GET", "POST"].includes(req.method)) return send(res, 405, { error: "Method not allowed" });
      if (p === "/__r1/manifest") return send(res, 200, fixtures);
      if (p === "/__r1/preload-stub.js") {
        const script = fs.readFileSync(path.join(here, "../preload-stub.js"), "utf8");
        return send(res, 200, script.replace("const now = () => Math.floor(cfg.now + performance.now() - t0);", "const now = () => cfg.now;"), "text/javascript");
      }
      if (p === "/__r1/media/walking-notes.txt") return send(res, 200, "Bring water for the afternoon walk.\n", "text/plain");
      if (p === "/__r1/media/walking-route.svg") return send(res, 200, fs.readFileSync(path.join(here, "walking-route.svg")), "image/svg+xml");
      if (p.startsWith("/assets/") || p.startsWith("/images/") && path.extname(p)) {
        if (!root) return send(res, 404, { error: "No webview" });
        const file = fs.realpathSync(path.resolve(root, `.${decodeURIComponent(p)}`));
        if (!file.startsWith(`${root}${path.sep}`) || !fs.statSync(file).isFile()) return send(res, 404, { error: "Not an asset" });
        return send(res, 200, fs.readFileSync(file), mime[path.extname(file)] ?? "application/octet-stream");
      }
      const session = getSession(url);
      if (p.startsWith("/backend-api/")) {
        const response = backendResponse({ url: p + url.search, method: req.method }, session.options);
        if (session.options.state === "loading" && response.status === 200) return;
        return send(res, response.status, response.body, new Map(response.headers).get("content-type"));
      }
      if (p === "/__r1/events" && req.method === "GET") {
        res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-store" });
        res.flushHeaders();
        session.events?.end();
        session.events = res;
        for (const event of session.queue.splice(0)) res.write(event);
        res.on("close", () => { if (session.events === res) sessions.delete(session.id); });
        return;
      }
      if (p === "/__r1/svc" && req.method === "POST") {
        const request = await body(req);
        const result = callService(request, session.options);
        if (session.options.state === "loading" && request.svc === "httpFetch" && result.value?.response?.__response?.status === 200) {
          const pathname = new URL(request.args[1].url, "https://chatgpt.com").pathname;
          if (fixtures.records.some((record) => pathname.endsWith(record.path.replace(/\{([^}]+)\}/g, (_, name) => fixtures.parameters[name])))) return;
        }
        return send(res, 200, result);
      }
      if (p === "/__r1/send" && req.method === "POST") {
        const message = await body(req);
        for (const reply of replayMessage(message, session.options, session.atoms)) emit(session, reply);
        return send(res, 200, {});
      }
      if (p === "/__r1/services") return send(res, 200, recordedValue("app-services", session.options));
      if (p === "/__r1/shared") return send(res, 200, recordedValue(`shared ${url.searchParams.get("key")}`, session.options));
      if (p === "/__r1/sidebar") return send(res, 200, recordedValue("sidebar-bootstrap", session.options));
      if (p.startsWith("/__r1/")) return send(res, 404, { error: "Unknown route1 request" });
      if (req.method !== "GET") return send(res, 405, { error: "No owned mutation" });
      return serveIndex(res, session);
    } catch (error) {
      if (!res.headersSent) send(res, error.code === "ENOENT" ? 404 : 400, { error: error.message });
      else res.end();
    }
  });
  return server;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const server = createFixtureServer();
  server.listen(Number(process.env.R1_PORT ?? 0), "127.0.0.1", () => {
    console.log(`Owned ChatGPT route1: http://127.0.0.1:${server.address().port}/`);
  });
}
