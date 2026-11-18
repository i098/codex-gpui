// Opens a page and returns {send, evaluate, on, close}.
// Default: a fresh isolated context of the renderer at R1_CDP (default 127.0.0.1:19444: the Mac's
// chrome-headless-shell through an ssh tunnel, see README); close() disposes the context.
// R1_APP_CDP=<url>: attach to the page of the original desktop app instead (app.mjs); close() only
// detaches.
const APP_CDP = process.env.R1_APP_CDP;
const CDP = process.env.R1_CDP || "http://127.0.0.1:19444";
const HIDDEN_RUNNER = /^https:\/\/cdn\.platform\.openai\.com\/deployments\/dil\/v\d+\/runner\.html(?:[?#]|$)/;

export async function openPage() {
  const base = APP_CDP ?? CDP;
  // The browser reports its own port; through a tunnel the local one differs.
  const local = (url) => url.replace(/^ws:\/\/[^/]+/, base.replace(/^http/, "ws"));
  const target = APP_CDP && (await (await fetch(`${APP_CDP}/json`)).json()).find((t) => t.type === "page" && t.url === "app://-/index.html");
  if (APP_CDP && !target) throw new Error("the original app has no page target");
  const ws = new WebSocket(local(target?.webSocketDebuggerUrl ?? (await (await fetch(`${CDP}/json/version`)).json()).webSocketDebuggerUrl));
  await new Promise((r, e) => ((ws.onopen = r), (ws.onerror = e)));
  let id = 0;
  const pending = new Map();
  const listeners = new Set();
  ws.onmessage = (m) => {
    const x = JSON.parse(m.data);
    if (x.id && pending.has(x.id)) {
      pending.get(x.id)(x);
      pending.delete(x.id);
    } else listeners.forEach((f) => f(x));
  };
  let closed = false;
  ws.onclose = () => {
    closed = true;
    pending.forEach((f) => f({ error: { message: "CDP connection closed" } }));
  };
  const raw = (method, params = {}, sessionId) =>
    new Promise((res, rej) => {
      if (closed) return rej(new Error(`${method}: CDP connection closed`));
      const i = ++id;
      pending.set(i, (x) => (x.error ? rej(new Error(`${method}: ${x.error.message}`)) : res(x.result)));
      ws.send(JSON.stringify({ id: i, method, params, sessionId }));
    });
  let sessionId;
  const send = (method, params) => raw(method, params, sessionId);
  const evaluate = async (expression) => {
    const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 800));
    return r.result.value;
  };
  const on = (f) => (listeners.add(f), () => listeners.delete(f));
  if (target) {
    // Attached straight to the app's page: no session, and no context to dispose.
    const close = async () => {
      await send("Emulation.clearDeviceMetricsOverride").catch(() => {});
      ws.close();
    };
    return { send, evaluate, on, close, sessionId };
  }
  const { browserContextId } = await raw("Target.createBrowserContext", { disposeOnDetach: true });
  const { targetId } = await raw("Target.createTarget", { url: "about:blank", browserContextId });
  ({ sessionId } = await raw("Target.attachToTarget", { targetId, flatten: true }));
  const close = async () => {
    await raw("Target.disposeBrowserContext", { browserContextId }).catch(() => {});
    ws.close();
  };
  return { send, evaluate, on, close, sessionId };
}

export async function navigate(page, path, { state, replace = false, timeoutMs = 90000 } = {}) {
  if (typeof path !== "string" || !path.startsWith("/") || new URL(path, "http://route1.invalid").origin !== "http://route1.invalid")
    throw new TypeError("Use an application route starting with /");
  if (!Number.isFinite(timeoutMs) || timeoutMs < 0) throw new RangeError("Invalid navigation timeout");
  // The original uses a memory router, not window.location or browser history.
  await page.evaluate(`(async () => {
    const deadline = performance.now() + ${JSON.stringify(timeoutMs)};
    const wait = async (ready) => {
      while (!ready()) {
        if (performance.now() >= deadline) throw new Error("Original route handler did not become ready");
        await new Promise(resolve => setTimeout(resolve, 50));
      }
    };
    await wait(() => window.__R1__?.appShared);
    let importTimer;
    const shared = await Promise.race([
      import(window.__R1__.appShared),
      new Promise((_, reject) => {
        importTimer = setTimeout(() => reject(new Error("Original module exceeded the navigation timeout")), Math.max(0, deadline - performance.now()));
      }),
    ]).finally(() => clearTimeout(importTimer));
    if (!("Nvn" in shared)) throw new Error("Pinned original host-message bus is unavailable");
    await wait(() => shared.Nvn?.handlers.get("navigate-to-route")?.size);
    shared.Nvn.dispatchHostMessage(${JSON.stringify({ type: "navigate-to-route", path, state, replace })});
  })()`);
}

export function isCaptureRequest({ type, loaderId, request }) {
  return type !== "EventSource" && !!loaderId && /^https?:/.test(request.url)
    && !(type === "Document" && HIDDEN_RUNNER.test(request.url));
}
