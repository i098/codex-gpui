// Opens a page in a fresh isolated context of the renderer at R1_CDP (default 127.0.0.1:19444: the
// Mac's chrome-headless-shell through an ssh tunnel, see README) and returns {send, evaluate, close};
// close() disposes the context.
const CDP = process.env.R1_CDP || "http://127.0.0.1:19444";

export async function openPage() {
  const { webSocketDebuggerUrl } = await (await fetch(`${CDP}/json/version`)).json();
  // The browser reports its own port; through a tunnel the local one differs.
  const ws = new WebSocket(webSocketDebuggerUrl.replace(/^ws:\/\/[^/]+/, CDP.replace(/^http/, "ws")));
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
  const raw = (method, params = {}, sessionId) =>
    new Promise((res, rej) => {
      const i = ++id;
      pending.set(i, (x) => (x.error ? rej(new Error(`${method}: ${x.error.message}`)) : res(x.result)));
      ws.send(JSON.stringify({ id: i, method, params, sessionId }));
    });
  const { browserContextId } = await raw("Target.createBrowserContext", { disposeOnDetach: true });
  const { targetId } = await raw("Target.createTarget", { url: "about:blank", browserContextId });
  const { sessionId } = await raw("Target.attachToTarget", { targetId, flatten: true });
  const send = (method, params) => raw(method, params, sessionId);
  const evaluate = async (expression) => {
    const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 800));
    return r.result.value;
  };
  const on = (f) => (listeners.add(f), () => listeners.delete(f));
  const close = async () => {
    await raw("Target.disposeBrowserContext", { browserContextId }).catch(() => {});
    ws.close();
  };
  return { send, evaluate, on, close, sessionId };
}
