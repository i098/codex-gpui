// Stand-in for .vite/build/preload.js when the original renderer runs in a plain browser.
// server.mjs prepends `window.__R1__ = {sid, theme, now, sentry, flavor}` and injects this file
// before any app script. Main-process traffic goes to server.mjs: POST /__r1/send (from-view)
// and the /__r1/events stream (for-view), so ordering matches Electron's IPC.
(() => {
  const cfg = window.__R1__;
  const q = (path) => `/__r1/${path}${path.includes("?") ? "&" : "?"}sid=${cfg.sid}`;

  // Fixed clock: relative timestamps ("2h ago") must not change between captures.
  const RealDate = Date;
  const t0 = performance.now();
  const now = () => Math.floor(cfg.now + performance.now() - t0);
  class FixedDate extends RealDate {
    constructor(...a) {
      super(...(a.length ? a : [now()]));
    }
    static now() {
      return now();
    }
  }
  window.Date = FixedDate;
  // Seeded Math.random: suggestion cards and similar picks must be the same on every capture.
  let seed = 42;
  Math.random = () => ((seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0) / 2 ** 32);

  document.documentElement.dataset.theme = cfg.theme;

  // from-view messages are posted strictly in order, one at a time. The (empty) response body is
  // read so that the request finishes; capture.mjs waits for the network to be idle.
  let chain = Promise.resolve();
  const post = (path, msg) =>
    (chain = chain.then(() =>
      fetch(q(path), { method: "POST", body: JSON.stringify(msg) }).then((r) => r.text()).then(() => undefined, () => undefined),
    ));
  const syncGet = (path) => {
    const x = new XMLHttpRequest();
    x.open("GET", q(path), false);
    x.send();
    return x.responseText ? JSON.parse(x.responseText) : undefined;
  };

  const shared = {};
  const workers = new Map();
  const themeListeners = new Set();
  const events = new EventSource(q("events"));
  events.onmessage = (e) => {
    const data = JSON.parse(e.data);
    if (data.__worker) {
      workers.get(data.__worker)?.forEach((f) => f(data.message));
      return;
    }
    if (data.type === "shared-object-updated") shared[data.key] = { v: data.value };
    window.dispatchEvent(new MessageEvent("message", { data }));
  };

  // App-host services: Electron's main serves ~130 of them over Cap'n Web on a MessagePort that
  // the page sends with {type: "connect-app-host"}. Here the page's own copy of the Cap'n Web
  // library hosts them; every method call goes to the server, which answers from its oracle.
  // Function arguments (listeners) get the calls the oracle recorded for them.
  // Listener stubs are disposed when the call returns unless dup()ed (Cap'n Web rule).
  const dec = (v) => {
    const r = v?.__response;
    if (r) {
      const body = r.base64 ? Uint8Array.from(atob(r.base64), (c) => c.charCodeAt(0)) : r.body;
      return new Response([101, 204, 205, 304].includes(r.status) ? null : body, r);
    }
    return v && typeof v === "object" && !Array.isArray(v) ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, dec(x)])) : v;
  };
  const callService = async (svc, method, args) => {
    const fns = new Map();
    const wire = args.map((a, i) => (typeof a === "function" ? (fns.set(i, a.dup ? a.dup() : a), "__fn") : a));
    const res = await fetch(q("svc"), { method: "POST", body: JSON.stringify({ svc, method, args: wire }) });
    const { value, fn, calls = [], error } = await res.json();
    if (error) throw new Error(error);
    for (const [i, x] of calls) setTimeout(() => fns.get(i)?.(...x));
    return fn ? () => {} : dec(value);
  };
  window.addEventListener("message", async (e) => {
    if (e.source !== window || e.data?.type !== "connect-app-host") return;
    const { K0t: RpcTarget, Y0t: newMessagePortRpcSession } = await import(cfg.appShared);
    const shape = await (await fetch(q("services"))).json();
    const services = {};
    for (const [name, kind] of Object.entries(shape)) {
      if (kind !== "function") {
        services[name] = kind ?? undefined;
        continue;
      }
      class Service extends RpcTarget {}
      const methods = new Proxy(Object.create(RpcTarget.prototype), {
        get: (t, m) =>
          typeof m === "string" && !["then", "constructor", "toJSON"].includes(m)
            ? (...args) => callService(name, m, args)
            : Reflect.get(t, m),
      });
      Object.setPrototypeOf(Service.prototype, methods);
      services[name] = new Service();
    }
    class Main extends RpcTarget {
      get services() {
        return services;
      }
    }
    newMessagePortRpcSession(e.data.port, new Main());
  });

  let sidebar;
  window.codexWindowType = "electron";
  window.__SENTRY_IPC__ = {
    "sentry-ipc": {
      sendRendererStart() {},
      sendScope() {},
      sendEnvelope() {},
      sendStatus() {},
      sendStructuredLog() {},
      sendMetric() {},
    },
  };
  window.electronBridge = Object.freeze({
    windowType: "electron",
    acknowledgeChunkedMessage() {},
    getPreloadStartedAtMs: () => performance.timeOrigin,
    sendMessageFromView: async (msg) => {
      if (msg.type === "shared-object-set") shared[msg.key] = { v: msg.value };
      // The server drops log messages; posting them only queues the ordered chain behind them.
      if (msg.type !== "log-message") await post("send", msg);
    },
    getPathForFile: () => null,
    startFileDrag: () => false,
    startLinkDrag() {},
    sendWorkerMessageFromView: async (id, msg) => {
      await post(`worker?id=${encodeURIComponent(id)}`, msg);
    },
    subscribeToWorkerMessages: (id, f) => {
      if (!workers.has(id)) workers.set(id, new Set());
      workers.get(id).add(f);
      return () => workers.get(id)?.delete(f);
    },
    showContextMenu: async () => null,
    getFastModeRolloutMetrics: async () => null,
    getSharedObjectSnapshotValue: (key) => {
      if (!(key in shared)) shared[key] = { v: syncGet(`shared?key=${encodeURIComponent(key)}`) };
      return shared[key].v;
    },
    getInitialSidebarBootstrap: () => (sidebar ??= syncGet("sidebar")),
    getSystemThemeVariant: () => cfg.theme,
    subscribeToSystemThemeVariant: (f) => {
      themeListeners.add(f);
      return () => themeListeners.delete(f);
    },
    triggerSentryTestError: async () => {},
    getSentryInitOptions: () => cfg.sentry,
    getDesktopUserAgent: () => `Codex Desktop/${cfg.sentry.appVersion} (Mac OS; arm64)`,
    getAppSessionId: () => cfg.sentry.codexAppSessionId,
    getBuildFlavor: () => cfg.flavor,
    isDeviceCheckSupported: () => true,
    isIntelMacBuild: () => false,
  });
})();
