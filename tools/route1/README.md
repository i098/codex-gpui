# Route 1: original renderer in headless Chromium on the Mac

The page renders in `chrome-headless-shell` on the Mac (CoreText text, the Mac's system fonts,
native 2x), driven over CDP from Linux. The server runs on Linux.

Use the existing pinned Mac browser on port 9467.
Do not restart it or use the global page selection on the shared Linux browser.
Each `chrome.mjs` connection creates an owned context and disposes it on close or disconnect.
Run the full capture command through the shared lease, including navigation and screenshots.
Set `TASK_ID`, `R1_PORT`, and `R1_CDP_PORT` in each shell.

```sh
# Choose unused local CDP and server ports before starting this tunnel.
ssh -N -o ControlMaster=no -o ControlPath=none -o ExitOnForwardFailure=yes \
  -L "$R1_CDP_PORT":127.0.0.1:9467 -R "$R1_PORT":127.0.0.1:"$R1_PORT" mac

# In another shell, keep the writable oracle and logs outside the checkout.
scratch=/sdc-disk/tmp/cgx/"$TASK_ID"
mkdir -p "$scratch"
cp tools/route1/oracle.json "$scratch/oracle.json"
env -u R1_SF_FONTS R1_OFFLINE=1 R1_ORACLE="$scratch/oracle.json" \
  MOCK_LOG="$scratch/app-server.log" node tools/route1/server.mjs

# In another shell, capture with the pinned renderer and native Mac fonts.
scratch=/sdc-disk/tmp/cgx/"$TASK_ID"
R1_CDP="http://127.0.0.1:$R1_CDP_PORT" R1_SERVER="http://127.0.0.1:$R1_PORT" \
  R1_OUT="$scratch/refs" tools/route1/lease.sh \
  node tools/route1/capture.mjs home dark 1280 832
```

The server needs the extracted 26.1007.30912 renderer at `/tmp/cgx/asar-x/webview`.
`R1_WEBVIEW` overrides that path.
Keep `R1_SF_FONTS` unset on the Mac capture route; even `R1_SF_FONTS=0` enables font aliases.
Close only owned contexts and stop only owned server and tunnel PIDs.

The lease uses `flock` on Linux, shared by all workers on that host.
Its default lock is `/sdc-disk/tmp/cgx/cgx-capture-service/capture.lock`.
`R1_CAPTURE_LOCK` overrides the path; every worker must use the same path.
The default wait is 300 seconds, and the command limit is 180 seconds.
Set `R1_CAPTURE_WAIT` or `R1_CAPTURE_TIMEOUT` to change these limits.
Exit 75 means the lease wait expired; exit 124 means the capture timed out.
The lease releases after command exit, including a timeout.
It serializes captures only, not builds.
Linux state discovery still supports `linux/start.sh`; set `R1_ORACLE` to an owned oracle copy first.
Set `R1_OUT` to an owned capture directory before importing `linux/lib.mjs`.

## Bootstrap recordings

The renderer waits for `threadReadState.open` before its initial data becomes ready.
Returning an empty value for that call leaves the startup spinner visible.
The oracle includes a ready snapshot from an owned, credential-free app-server.
The preload bridge replays its session and read-marker changes locally, per page.
No read-marker change reaches a real account.

To regenerate these entries, prepare an owned Mac `CODEX_HOME` and project under `/tmp`.
Configure a provider with `requires_openai_auth = false` and a loopback-only `base_url`.
Create scratch threads with that app-server; do not start a model turn.
Launch the original app in the background with that same `CODEX_HOME`, a fresh
`CODEX_ELECTRON_USER_DATA_PATH`, and `CODEX_ELECTRON_START_IN_BACKGROUND=1`.
Forward its CDP port to Linux, then run:

```sh
CDP_PORT=19333 R1_RPC_MODULE=./assets/rpc-a387904149fd.js \
  node tools/route1/record-bootstrap.mjs "$scratch/oracle.json"
```

The recorder rejects account-backed identities.
It records `startup.whenReady`, `threadReadState.open`, and the matching execution-host keys.
Set `R1_RPC_MODULE` to the installed app's RPC entry; its default is the pinned 26.1007 entry.
Use `R1_ORACLE` to select the writable oracle when serving these recordings.
In offline mode, the server runs the existing recorded app-server adapter directly.
It does not open the shared Mac profile or need an SSH wrapper.

The bootstrap transport was recorded with the installed 26.1013.40934 main process.
Its read-state response matches the retained 26.1007 service contract.
The renderer assets, browser, fonts, references, and pixel thresholds remain pinned.
Re-pinning the renderer from 26.1007 to 26.1013 is separate work.

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
  - App-server JSON-RPC for host `local` uses `linux/mock-app-server.mjs` when `R1_OFFLINE=1`.
    It reads `fixtures/readback.jsonl` and `fixtures/sessions`.
    Live mode instead uses the prepared scratch app-server on the Mac.
    Only reads (`*/read`, `*/list`, `*/get*`, `*/search`, `thread/resume`, ...) pass.
    `thread/start`, `turn/start`, archive, delete and other writes get a JSON-RPC error.
  - Everything else (vscode:// host fetches, persisted atoms, shared objects, cloud host
    requests, git worker, app-host service calls) comes from `oracle.json`.
    A miss is asked from the live scratch app only outside offline mode.
    Only reading calls reach that app; writes answer locally.
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
