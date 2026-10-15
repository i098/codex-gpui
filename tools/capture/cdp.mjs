// Minimal CDP client for the main page of the scratch Codex app instance.
// The instance listens on the Mac at 127.0.0.1:9333; reach it with
// `ssh -N -L 19333:localhost:9333 mac` (CDP_PORT overrides 19333).
const port = process.env.CDP_PORT || 19333;

export async function connect() {
  const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const page = list.find((t) => t.type === "page" && t.url === "app://-/index.html");
  if (!page) throw new Error("main app page not found");
  const ws = new WebSocket(page.webSocketDebuggerUrl.replace(/localhost:\d+/, `127.0.0.1:${port}`));
  await new Promise((r, e) => ((ws.onopen = r), (ws.onerror = e)));
  let id = 0;
  const pending = new Map();
  ws.onmessage = (m) => {
    const x = JSON.parse(m.data);
    if (x.id && pending.has(x.id)) pending.get(x.id)(x);
  };
  const send = (method, params = {}) => {
    const i = ++id;
    ws.send(JSON.stringify({ id: i, method, params }));
    return new Promise((r) => pending.set(i, r));
  };
  const evaluate = async (expression) => {
    const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails).slice(0, 1000));
    return r.result.result.value;
  };
  return { send, evaluate, close: () => ws.close() };
}
