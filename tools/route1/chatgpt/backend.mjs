import fs from "node:fs";

export const fixtures = JSON.parse(fs.readFileSync(new URL("./responses.json", import.meta.url), "utf8"));
const oracle = JSON.parse(fs.readFileSync(new URL("../oracle.json", import.meta.url), "utf8"));
export const frozenTime = Date.parse(fixtures.now);
const records = new Map(fixtures.records.map((record) => [
  `${record.method} ${record.path.replace(/\{([^}]+)\}/g, (_, name) => fixtures.parameters[name])}`, record,
]));
const gizmo = records.get(`GET /gizmos/${fixtures.ids.gizmo}`);
records.set(`GET /gizmos/${fixtures.ids.health}`, {
  ...gizmo, body: records.get("GET /gizmos/firstparty/sidebar").body[0],
});
const image = records.get("GET /files/library/recents").body.items.find((file) => file.id === fixtures.ids.image);
records.set(`GET /files/library/files/${fixtures.ids.image}`, {
  ...records.get(`GET /files/library/files/${fixtures.ids.file}`), body: image,
});
const bootstrap = new Map();
const bootstrapPaths = new Set([
  "/wham/statsig/bootstrap", "/accounts/check/v4-2023-04-27", "/accounts/optimized/check",
  "/me", "/models", "/settings/user", "/settings/is_adult", "/trusted_contact/enabled",
  "/aip/first-party/eligibility", "/beacons/home", "/wham/onboarding/context",
]);

export function requestPath(value) {
  const url = new URL(value, "https://chatgpt.com");
  if (url.origin !== "https://chatgpt.com") throw new Error("Only the recorded ChatGPT origin is allowed");
  url.pathname = url.pathname.replace(/^\/(backend-api|api)(?=\/)/, "");
  return url;
}

for (const [key, value] of Object.entries(oracle)) {
  if (!key.startsWith("svc httpFetch.fetch ")) continue;
  const [, request] = JSON.parse(key.slice("svc httpFetch.fetch ".length));
  const url = requestPath(request.url);
  if (bootstrapPaths.has(url.pathname) && value.value?.response?.__response) {
    bootstrap.set(`${request.method ?? "GET"} ${url.pathname}`, value.value.response.__response);
  }
}

export function sessionOptions(params) {
  const state = params.get("state") ?? "populated";
  const profile = params.get("profile") ?? "reference";
  if (!["populated", "empty", "error", "loading"].includes(state)) throw new Error("Unknown fixture state");
  if (!["reference", "workspace"].includes(profile)) throw new Error("Unknown fixture profile");
  const gates = { ...fixtures.gates, ...JSON.parse(params.get("gates") ?? "{}") };
  if (Object.entries(gates).some(([key, value]) => !/^\d+$/.test(key) || typeof value !== "boolean")) {
    throw new Error("Gate overrides require numeric names and boolean values");
  }
  return { state, profile, gates, theme: params.get("theme") === "dark" ? "dark" : "light" };
}

function overrideGates(payload, gates) {
  for (const [key, value] of Object.entries(gates)) {
    payload.feature_gates ??= {};
    payload.feature_gates[key] = { ...payload.feature_gates[key], name: key, value, rule_id: "owned-fixture", secondary_exposures: [] };
  }
  return payload;
}

function accountProfile(body, options) {
  if (options.profile !== "workspace") return body;
  for (const entry of Object.values(body.accounts ?? {})) {
    entry.account.structure = "workspace";
    entry.account.plan_type = "team";
    entry.features = [...new Set([...(entry.features ?? []), "desktop-app-chat.access", "chatgpt.workspace.feature.hermes.access"])];
    entry.permissions = [...new Set([...(entry.permissions ?? []), "chatgpt.workspace.feature.hermes.access", "chatgpt.workspace.feature.hermes.builder.access"])];
  }
  return body;
}

function json(status, body) {
  return { status, headers: [["content-type", "application/json"]], body: JSON.stringify(body) };
}

export function backendResponse(request, options) {
  let url;
  try { url = requestPath(request.url); } catch (error) { return json(403, { error: error.message }); }
  const method = (request.method ?? "GET").toUpperCase();
  const key = `${method} ${url.pathname}`;
  const recorded = bootstrap.get(key);
  if (recorded) {
    const body = JSON.parse(recorded.body);
    if (url.pathname === "/wham/statsig/bootstrap") overrideGates(body, options.gates);
    if (url.pathname.startsWith("/accounts/")) accountProfile(body, options);
    return json(recorded.status, body);
  }
  // Celsius is disabled locally. No WebSocket can reach an account service.
  if (url.pathname === "/celsius/ws/user" && method === "GET") return json(200, { enabled: false });
  const record = records.get(key);
  if (!record) return json(method === "GET" ? 404 : 405, { error: "No owned recording", method, path: url.pathname });
  if (options.state === "error") return json(503, { detail: "Owned fixture service unavailable" });
  let body = structuredClone(options.state === "empty" && Object.hasOwn(record, "empty") ? record.empty : record.body);
  if (record.empty && ["cursor", "after", "offset"].some((name) => url.searchParams.has(name) && !["", "0"].includes(url.searchParams.get(name)))) {
    body = structuredClone(record.empty);
  }
  const query = url.searchParams.get("query") ?? url.searchParams.get("q");
  if (query && Array.isArray(body.items)) {
    body.items = body.items.filter((item) => JSON.stringify(item).toLowerCase().includes(query.toLowerCase()));
    if ("total" in body) body.total = body.items.length;
  }
  return record.content_type === "application/json" ? json(record.status, body)
    : { status: record.status, headers: [["content-type", record.content_type]], body };
}

const keyOf = (value) => JSON.stringify(value)
  .replace(/[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/g, "<id>")
  .replace(/\b1\d{12}\b/g, "<t>");

const localRequests = new Map();
const localReplies = new Map();
for (const line of fs.readFileSync(new URL("../../../fixtures/readback.jsonl", import.meta.url), "utf8").trim().split("\n")) {
  const { dir, msg } = JSON.parse(line);
  if (dir === "send" && msg.id != null) localRequests.set(msg.id, msg);
  const request = localRequests.get(msg.id);
  if (dir === "recv" && request && ["account/read", "getAuthStatus"].includes(request.method)) {
    localReplies.set(request.method, msg);
  }
}

export function recordedValue(key, options) {
  const value = structuredClone(oracle[key] ?? null);
  if (key === "shared statsig_evaluations" && value?.payload) {
    value.payload = JSON.stringify(overrideGates(JSON.parse(value.payload), options.gates));
  }
  if (key === "sentry" && value) value.dsn = "";
  if (key === "app-services" && value) {
    // The renderer has local fallbacks for these optional live host services.
    delete value.startup;
    delete value.threadReadState;
  }
  return value;
}

export function callService(message, options) {
  const { svc, method, args = [] } = message;
  if (svc === "httpFetch" && method === "fetch") {
    return { value: { response: { __response: backendResponse(args[1] ?? {}, options) } } };
  }
  if (svc === "localThreadCatalog" && method === "readPage" && args[0]?.hostId?.startsWith("chatgpt:")) {
    return { value: { entries: [], nextCursor: null }, calls: [], fn: false };
  }
  if (!/^(get|list|read|is|has|fetch|load|query|lookup|resolve|check|find|search|current|snapshot|subscribe|watch|observe|on[A-Z])/.test(method)) {
    return { error: `Owned fixture refuses ${svc}.${method}` };
  }
  const result = recordedValue(`svc ${svc}.${method} ${keyOf(args)}`, options);
  if (!result) return { error: `No owned recording for ${svc}.${method}` };
  if (svc === "settings" && method === "readAll" && result.value?.values) result.value.values.appearanceTheme = options.theme;
  if (svc === "settings" && args[0] === "appearanceTheme") result.calls = result.calls?.map(([index]) => [index, [{ configured: options.theme, effective: options.theme }]]);
  return result;
}

export function replayMessage(message, options, atoms) {
  switch (message.type) {
    case "ready":
      return [...(recordedValue("ready", options) ?? []), { type: "electron-window-opaque-surface-changed", opaqueWindowSurfaceEnabled: true },
        ...(message.initializationOnly ? [] : [{ type: "persisted-atom-sync", state: atoms, canWritePrimaryWindowTabPersistence: true }])];
    case "persisted-atom-sync-request": return [{ type: "persisted-atom-sync", state: atoms, canWritePrimaryWindowTabPersistence: true }];
    case "persisted-atom-update":
      if (message.deleted) delete atoms[message.key]; else atoms[message.key] = message.value;
      return [];
    case "shared-object-subscribe": return [{ type: "shared-object-updated", key: message.key, value: recordedValue(`shared ${message.key}`, options) }];
    case "mcp-request": {
      const request = message.request;
      if (!/\/(read|list|get|search|status)[A-Za-z]*$|^(getAuthStatus|thread\/resume)$/.test(request.method)) {
        return [{ type: "mcp-response", hostId: message.hostId, message: { id: request.id, error: { code: -32601, message: "Owned fixture refuses RPC mutation" } } }];
      }
      const reply = recordedValue(`mcp ${message.hostId} ${request.method} ${keyOf(request.params ?? null)}`, options)?.message
        ?? (message.hostId === "local" ? localReplies.get(request.method) : null);
      return [{ type: "mcp-response", hostId: message.hostId, message: { ...(reply ?? { error: { code: -32601, message: "No owned RPC recording" } }), id: request.id } }];
    }
    case "fetch": {
      const reply = recordedValue(`fetch ${message.method ?? "GET"} ${message.url} ${keyOf(message.body ?? "")}`, options);
      return [{ ...(reply ?? { type: "fetch-response", responseType: "error", status: 404, error: "No owned host recording" }), requestId: message.requestId }];
    }
    default: return [];
  }
}
