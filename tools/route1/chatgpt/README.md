# Owned ChatGPT route1 fixtures

Run this mode instead of `tools/route1/server.mjs` for ChatGPT page work.
It never starts an app-server, opens an account, or forwards a request.
It reuses the route1 preload, service envelope, event stream, and scrubbed oracle.
The browser receives a content security policy that blocks external connections.

```sh
R1_WEBVIEW=/sdc-disk/tmp/cgx/reference-26.1007.30912/asar-x/webview \
  node tools/route1/chatgpt/server.mjs
```

The server prints its loopback URL and uses a free port by default.
Set `R1_PORT` when an existing capture tool needs a specific port.
Use an owned browser context on the existing shared Chrome endpoint.
Do not use an account profile or start another shared browser.

## Data and routes

`responses.json` contains owned example data derived from the pinned `26.1007.30912` request consumers.
Each record names its source chunk, source offset, method, family, and response content type.
These are synthetic fixtures, not recordings of private chats or proof of live backend behavior.
The existing scrubbed `../oracle.json` supplies account bootstrap, models, settings, and host read responses.
The server strips the recorded HTTP headers and never stores incoming authorization headers.

The data links a conversation, GPT, Health project, Space page, Library folder, file, image, Site, Dot, and remote task.
Read the stable IDs in `/__r1/manifest` or `responses.json`.
The responses cover conversation history and SSE, GPT discovery, images, Health chats, Space, Library, Sites, profiles, and Dot rooms.
The remote task has a completed turn and a finite event stream.
Image and text downloads use local owned assets.

Open these paths after the printed server URL:

| Family | Path |
| --- | --- |
| Conversation | `/c/10000000-0000-4000-8000-000000000001` |
| GPTs | `/gpts` or `/g/g-owned-demo` |
| Images | `/images` |
| Health | `/health` |
| Space | `/space` or `/spaces/10000000-0000-4000-8000-000000000005` |
| Library | `/library` or `/library/file/file-owned-notes` |
| Sites | `/space/sites` |
| Dots | `/dots/home` |
| Profile | `/profile` |
| Remote task | `/remote/10000000-0000-4000-8000-000000000009` |

## Capture controls

Each page gets a separate route1 session.
Its options and persisted atom changes stay in memory for that session.
The clock stays at `2026-10-15T17:00:00-07:00`, including `Date.now()`.
Add query parameters when opening the page:

- `theme=light` or `theme=dark` selects the existing appearance setting.
- `state=populated` selects the linked examples.
- `state=empty` clears collection responses but keeps directly addressed detail objects.
- `state=error` returns 503 for owned page responses and preserves account bootstrap.
- `state=loading` leaves page requests pending until the client disconnects.
- `profile=workspace` selects an owned workspace account for the Agents access predicate.
- `gates={"2200939124":false}` overrides numeric Statsig gates in both bootstrap and the shared snapshot.

The default overrides enable ChatGPT, Space, Sites, Health, Maps, security, and the Agents rollout.
The Agents predicate also needs the workspace profile.
These overrides describe owned test accounts, not the permissions of a real account.
Unknown reads return 404.
Unknown writes, sign-in, MFA, and account changes return 405 or an explicit RPC error.
No miss falls back to the live app.

## Checks

```sh
node tools/route1/chatgpt/server.test.mjs
```

The check exercises the HTTP service envelope, linked data, SSE, pagination, search, state isolation, gate overrides, clock, and refused account writes.
The check uses a free loopback port and closes its server.
It needs Node.js only, not Cargo, Electron, or account credentials.

Page owners must still capture their actual original page states and compare them with the native implementation.
This fixture mode does not claim pixel parity or complete every page interaction.
Add a source-backed response here when a page needs another read contract.
Keep unknown mutations refused.
