# Route 1: original renderer in headless Chromium on the Mac

The page renders in `chrome-headless-shell` on the Mac (CoreText text, the Mac's system fonts,
native 2x), driven over CDP from Linux. The server runs on Linux.

Regenerate all references (each long-lived command in its own shell; `$$` prints the shell pid so
that only this process is stopped afterwards):

```
# 1. Mac renderer (no window; it makes its own temporary profile)
ssh mac 'echo PID $$; exec /tmp/cgx-mac/chrome-headless-shell-mac-arm64/chrome-headless-shell --headless --remote-debugging-port=9444 --hide-scrollbars --force-device-scale-factor=2 --num-raster-threads=1 --disable-checker-imaging --disable-partial-raster about:blank'
# 2. Tunnels: CDP to Linux :19444, the server to the Mac's :4517
ssh -N -o ExitOnForwardFailure=yes -L 19444:127.0.0.1:9444 -R 4517:127.0.0.1:4517 mac
# 3. Server, then the captures (R1_OUT=<dir> writes elsewhere than refs/)
R1_OFFLINE=1 node tools/route1/server.mjs
node tools/route1/capture-all.mjs
# 4. Afterwards: ssh mac 'kill <PID from step 1>'
```

Prerequisites: the extracted app at `/tmp/cgx/asar-x/webview`, `ssh mac`, and the headless shell
in `/tmp/cgx-mac/chrome-headless-shell-mac-arm64`. `R1_CDP` overrides the CDP endpoint
(default `http://127.0.0.1:19444`).

For a renderer without the Mac fonts (Linux Chrome), start the server with `R1_SF_FONTS=1`:
`@font-face` aliases then map `-apple-system`, `BlinkMacSystemFont`, `system-ui`, `SF Pro*`,
`ui-monospace`, `SF Mono`, `Menlo` to the variable SF fonts in `/tmp/cgx/route1-fonts`
(`SFNS.ttf`, `SFNSItalic.ttf`, `SFNSMono.ttf` from the Mac's `/System/Library/Fonts`).

## What the stub fakes

- `preload-stub.js` replaces Electron's `preload.js`: `window.electronBridge` (all 22 members),
  `codexWindowType`, `__SENTRY_IPC__`, `data-theme`. From-view messages go to the server in order
  (POST `/__r1/send`; `log-message` is dropped); for-view messages come back over an EventSource
  and are dispatched as `message` events. `Date` runs from a fixed instant (2026-10-15 10:00 PDT).
  The headless shell on the Mac reports `(hover: hover)` and `(pointer: fine)`, so no media rewrite.
- App-host services (Cap'n Web over the `connect-app-host` MessagePort, about 130 services) are
  hosted in the page with the bundle's own Cap'n Web library; every call goes to the server.
- `server.mjs` plays the main process:
  - App-server JSON-RPC for host `local` goes to a real `codex app-server` on the Mac
    (`CODEX_HOME=/tmp/cgx-mac/home`). Only reads (`*/read`, `*/list`, `*/get*`, `*/search`,
    `thread/resume`, ...) pass; `thread/start`, `turn/start`, archive, delete and other writes
    get a JSON-RPC error.
  - Everything else (vscode:// host fetches, persisted atoms, shared objects, cloud host
    requests, git worker, app-host service calls) comes from `oracle.json`. A miss is asked once
    from the live scratch app over CDP (`tools/capture/cdp.mjs`) and stored, so later runs are
    offline (`R1_OFFLINE=1`). Only reading calls reach the live app; writes answer locally.
  - `thread/resume` is answered by the live app (recorded): its app-server holds the fixture
    threads' writer locks, so a second app-server would show "This is open in another app".
  - Background terminal (`terminal` screen): no fixture thread has a running command and starting
    one is a write, so `/__r1/replay-running-command` replays the subtract session's recorded
    `python3` check (fixtures/sessions/subtract.jsonl) for a new turn that completes while the
    command stays `inProgress`. The thread then lists it in the summary panel (`Toggle summary`,
    `Background processes`); `capture.mjs` opens the panel and clicks the row, which opens the
    `Background terminal` side tab (xterm with the output `2`).
  - ChatGPT backend data that is private (billing, chats, spaces, pins, MFA factors, notifications)
    is never recorded: GETs answer 404, analytics POSTs an empty 200. ChatGPT-host thread catalog
    pages answer an empty last page, the sidebar bootstrap keeps only local threads,
    `external-agent-import-detect` (Claude/Cursor sessions) answers `null`, and the persisted
    atoms drop the import scan and pinned projects.
  - Every entry is scrubbed before it is stored, and the whole oracle again when the server
    starts (it rewrites `oracle.json` if a rule changed): emails, `/Users/<name>`, user/org/account
    ids, phone numbers, IP addresses, GitHub avatar ids and the name fields of person objects get
    the fixture placeholders (`user@example.com`, `Demo User`, `/Users/user`,
    `00000000-0000-4000-8000-000000000000`), also inside nested JSON bodies and base64 JSON tokens
    (decoded, scrubbed, encoded again). The account avatar image stays: the `httpFetch.fetch`
    entry for `backend-api/estuary/public_content/enc/...` holds its bytes.
  - The theme is the app's own `appearanceTheme` setting, per page (`?theme=dark|light`).
  - On `ready` it sends `electron-window-opaque-surface-changed` (opaque window, as the Mac app),
    so `<html>` gets `electron-opaque` and the body its opaque background.
  - The lodash `shuffle` chunk (`assets/shuffle-<hash>.js`, automations suggestion cards) is
    served as identity: how many `Math.random` draws come before it varies per load.
  - Diff screen: `View changes` shows the file header row but no hunk body, even 30 s later
    (offline run). The body does render when the turn diff comes in as a notification (the
    `terminal` replay opens Changes with lines 2-6). So the body probably comes from a
    git-worker or diff-worker answer that `oracle.json` does not hold [INFERENCE]. Capture with
    the live scratch app (no `R1_OFFLINE`) to record it.
- `capture.mjs` uses a Mac user agent, America/Los_Angeles, en-US, the shell's native scale 2 (no
  scale emulation: layout in device px as in the app), hidden scrollbars. It dismisses the
  announcement modal, reaches the state by clicking, and waits until the DOM and the network (CDP
  Network events; server messages on the EventSource count as activity) have been quiet for
  1.5 s with no request in flight; the Mac page loads slowly through the tunnels and pauses on
  lazy chunks and app-server answers. It then ends finite animations, rests infinite ones at t=0,
  makes the caret transparent, loads every SVG `<img>` again on the still page (its raster size
  otherwise depends on load timing) and sends a critical memory-pressure signal so that tiles
  and decoded images are rasterized again from the still page. The spec gets extra `texts[]`
  entries (`pseudo: "before"|"after"`) for CSS generated text such as the composer placeholder,
  measured as a span carrying the pseudo element's computed style.
- Determinism: the shell runs with one raster thread, without checker-imaging and without partial
  raster. Two full runs give 36/36 byte-identical PNGs. Without `--disable-partial-raster`, cc
  rasters only the invalidated rect of some tiles again, so a few antialiased px at rounded-clip
  corners depend on the load timing.
- The sidebar refs keep an earlier server state: the search panel shows the "Chats" spinner and a
  snippet line. The current server answers the search POST locally, so a new capture shows
  "Couldn't search ChatGPT chats" and no snippet.
