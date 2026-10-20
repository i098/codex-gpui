import assert from "node:assert/strict";
import { once } from "node:events";
import vm from "node:vm";
import { createFixtureServer } from "./server.mjs";
import { fixtures, frozenTime } from "./backend.mjs";

const server = createFixtureServer();
server.listen(0, "127.0.0.1");
await once(server, "listening");
const base = `http://127.0.0.1:${server.address().port}`;
async function service(url, { state = "populated", method = "GET", gates, sid = state, profile } = {}) {
  const params = new URLSearchParams({ sid, state });
  if (gates) params.set("gates", JSON.stringify(gates));
  if (profile) params.set("profile", profile);
  const response = await fetch(`${base}/__r1/svc?${params}`, {
    method: "POST", body: JSON.stringify({ svc: "httpFetch", method: "fetch", args: ["request", { url, method }, null] }),
  });
  return (await response.json()).value.response.__response;
}
const decode = (response) => JSON.parse(response.body);
try {
  const conversations = decode(await service("/conversations?limit=20"));
  const conversation = decode(await service(`/conversation/${conversations.items[0].id}`));
  const answer = conversation.mapping[conversation.current_node];
  assert.equal(answer.message.author.role, "assistant");
  assert.equal(conversation.mapping[answer.parent].message.author.role, "user");
  assert.equal(conversation.mapping[answer.parent].children[0], conversation.current_node);
  assert.equal(conversation.mapping[conversation.mapping[answer.parent].parent].message, null);
  assert.equal(conversation.create_time * 1000, frozenTime);
  assert.equal(decode(await service("/conversations?offset=20")).total, 0);
  assert.equal(decode(await service("/conversations?q=does-not-exist")).total, 0);
  assert.equal(decode(await service("/conversations", { state: "empty" })).total, 0);
  assert.equal((await service("/conversations", { state: "error" })).status, 503);
  assert.equal(decode(await service("/conversations")).total, 1);

  const space = decode(await service("/spaces/v2")).items[0];
  const pages = decode(await service(`/spaces/${space.id}/pages`));
  assert.equal(pages.items[0].page_id, space.root_page_id);
  assert.equal(pages.items[0].drive_space_id, space.id);
  const nodes = decode(await service("/files/library/nodes")).items;
  assert.equal(nodes[1].parent_directory_id, nodes[0].id);
  const site = decode(await service("/websites")).items[0];
  assert.equal(decode(await service(`/websites/${site.id}`)).title, site.title);
  assert.equal(decode(await service("/websites/access")).enabled, true);
  const dot = decode(await service("/tbo/primary"));
  assert.equal(dot.selection.thread_id, conversation.conversation_id);
  assert.equal(dot.profile.messaging_room_id, conversation.conversation_id);
  assert.equal(decode(await service("/gizmos/firstparty/sidebar"))[0].gizmo.gizmo_snorlax_type, "potion");
  assert.equal(decode(await service("/aip/olympic/chats")).items[0].id, conversation.conversation_id);
  assert.equal(decode(await service("/images/init")).groups[0].items[0].id, "walk-prompt");
  assert.equal(decode(await service("/profiles/me")).profile_details.username, "demo");
  const task = decode(await service("/wham/tasks/list")).items[0].task;
  assert.equal(decode(await service(`/wham/tasks/${task.id}`)).task.status, "completed");
  const stream = await service("/f/conversation", { method: "POST" });
  assert.equal(new Map(stream.headers).get("content-type"), "text/event-stream");
  assert.equal(JSON.parse(stream.body.split("\n")[0].slice(6)).message.id, answer.message.id);
  assert.ok(stream.body.endsWith("data: [DONE]\n\n"));

  const gates = { "2200939124": false, "1598376": false };
  const flags = decode(await service("/wham/statsig/bootstrap", { method: "POST", sid: "gated", gates }));
  assert.equal(flags.feature_gates["2200939124"].value, false);
  const shared = await (await fetch(`${base}/__r1/shared?sid=gated&key=statsig_evaluations`)).json();
  assert.equal(JSON.parse(shared.payload).feature_gates["1598376"].value, false);
  const workspace = decode(await service("/accounts/check/v4-2023-04-27", { sid: "workspace", profile: "workspace" }));
  assert.equal(workspace.accounts[fixtures.ids.account].account.structure, "workspace");
  assert.equal((await service("https://example.com/conversations")).status, 403);
  assert.equal((await service("/conversation/not-owned")).status, 404);
  assert.equal((await service(`/conversation/${fixtures.ids.conversation}`, { method: "DELETE" })).status, 405);
  assert.equal((await service("/accounts/mfa_info")).status, 404);
  assert.equal((await service("/auth/login", { method: "POST" })).status, 405);
  assert.equal((await fetch(`${base}/__r1/shared?state=unknown&sid=invalid`)).status, 400);
  assert.equal((await fetch(`${base}/__r1/shared?sid=bad-gate&gates=%7B%22x%22%3Atrue%7D`)).status, 400);
  const image = await fetch(`${base}/__r1/media/walking-route.svg`);
  assert.equal(image.headers.get("content-type"), "image/svg+xml");
  assert.ok(image.headers.get("content-security-policy").includes("connect-src 'self'"));

  const pending = fetch(`${base}/__r1/svc?state=loading&sid=loading`, {
    method: "POST", signal: AbortSignal.timeout(150),
    body: JSON.stringify({ svc: "httpFetch", method: "fetch", args: ["request", { url: "/conversations" }] }),
  });
  await assert.rejects(pending, { name: "TimeoutError" });
  const preload = await (await fetch(`${base}/__r1/preload-stub.js`)).text();
  const clock = preload.slice(preload.indexOf("const RealDate"), preload.indexOf("// Seeded"));
  const context = vm.createContext({ window: {}, cfg: { now: frozenTime }, performance: { now: () => 42 } });
  vm.runInContext(clock, context);
  assert.equal(context.window.Date.now(), frozenTime);
  assert.equal(new context.window.Date().getTime(), frozenTime);
  console.log("PASS: owned HTTP fixtures, linked families, SSE, isolated states, gates, clock, and denied account writes");
} finally {
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
}
