import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";

const rpc = await import(`data:text/javascript,${encodeURIComponent(`
  export class K0t {}
  export function Y0t(port, main) { port.main = main; }
`)}`);
let onMessage;
const window = {
  __R1__: { sid: "test", now: 0, theme: "dark", appShared: "test-rpc" },
  addEventListener(type, listener) { if (type === "message") onMessage = listener; },
};
const snapshot = {
  status: "ready",
  identity: { kind: "execution-storage", authMode: "none" },
  executionHostKeysByHostId: { local: "local:test" },
  unreadThreadIdsByHostId: { local: ["unread-thread"] },
};
const context = {
  window,
  document: { documentElement: { dataset: {} } },
  performance: { now: () => 0 },
  EventSource: class {},
  setTimeout,
  async fetch(url) {
    const value = url.startsWith("/__r1/services")
      ? { threadReadState: "function" }
      : { value: structuredClone(snapshot), fn: false, calls: [] };
    return { json: async () => value };
  },
};
new vm.Script(fs.readFileSync(new URL("./preload-stub.js", import.meta.url), "utf8"), {
  importModuleDynamically: async () => rpc,
}).runInNewContext(context);
const port = {};
await onMessage({ source: window, data: { type: "connect-app-host", port } });
const events = [];
const opened = await port.main.services.threadReadState.open(event => events.push(event), null);
const change = { hostId: "local", threadId: "unread-thread", hasUnreadTurn: false };
assert.equal(opened.session.set(change).status, "ok");
assert.equal(opened.unreadThreadIdsByHostId.local.length, 0);
assert.equal(events.length, 1);
assert.equal(events[0].type, "changed");
assert.equal(events[0].origin, "self");
assert.equal(events[0].threadId, "unread-thread");
assert.equal(events[0].hasUnreadTurn, false);
assert.equal(opened.session.set(change).status, "ok");
assert.equal(events.length, 1);
opened.session.unsubscribe();
assert.equal(opened.session.set({ ...change, hasUnreadTurn: true }).status, "retired");
assert.equal(opened.unreadThreadIdsByHostId.local.length, 0);
console.log("read markers: clearing, notification, and session retirement passed");

let wroteOracle = false;
let closedRecorder = false;
const browser = vm.createContext({});
const services = new vm.SyntheticModule(["appServices"], function () {
  this.setExport("appServices", {
    threadReadState: {
      async open() {
        return { status: "ready", identity: { kind: "execution-storage", authMode: "apikey" } };
      },
    },
  });
}, { context: browser });
await services.link(() => {});
await services.evaluate();
const recorderContext = vm.createContext({
  process: { argv: ["node", "record-bootstrap.mjs", "oracle.json"], env: {} },
  console: { log() {} },
});
const recorder = new vm.SourceTextModule(
  fs.readFileSync(new URL("./record-bootstrap.mjs", import.meta.url), "utf8"),
  { context: recorderContext },
);
await recorder.link(specifier => {
  const exports = specifier === "node:fs"
    ? { default: { readFileSync: () => "{}", writeFileSync: () => { wroteOracle = true; } } }
    : { connect: async () => ({
        evaluate: expression => new vm.Script(expression, {
          importModuleDynamically: async () => services,
        }).runInContext(browser),
        close: () => { closedRecorder = true; },
      }) };
  return new vm.SyntheticModule(Object.keys(exports), function () {
    for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
  }, { context: recorderContext });
});
await assert.rejects(recorder.evaluate(), /credential-free/);
assert.equal(wroteOracle, false);
assert.equal(closedRecorder, true);
console.log("bootstrap recorder: API-key profiles are rejected before writing");
