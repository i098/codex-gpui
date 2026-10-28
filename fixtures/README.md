# Fixtures

The original explain, subtract, greeting, and readback fixtures use the bundled `codex app-server`:
`/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex` (codex-cli 0.162.0-alpha.17.2), model gpt-6-astra, reasoning low.
Each line contains `dir` (`send` or `recv`) and the JSON-RPC `msg`; `t`, when present, records elapsed milliseconds.
`send` identifies a scripted client request, not an Electron main-process recording.
Original session clients use `codex-gpui` with `experimentalApi: true`.
The recorder replaces secrets with fixed placeholders (`user@example.com`, `<redacted>`, `00000000-0000-4000-8000-000000000000`, `Demo User`, `/Users/user`).

- `sessions/explain.jsonl`: initialize, thread/start in `/tmp/cgx-mac/proj` (workspace-write, approval never), one turn "In two sentences, what does this repository contain?" with command executions and an agent message.
- `sessions/subtract.jsonl`: same setup, one turn that adds `subtract` to `src/math.py` (fileChange item with diff, `turn/diff/updated` with a full unified diff) and runs the python3 check (commandExecution item, output `2`).
- `sessions/greeting.jsonl`: same setup, one turn that changes `hello.py` to print "Hello, Codex" (fileChange item with diff).
- `readback.jsonl`: one connection that sends every startup read the desktop UI uses (auth, account, config, models, features, rate limits, usage, skills, hooks, plugins, apps, MCP status, projects, sections), then thread/list with the desktop Recents params and, for each thread, thread/read (includeTurns), thread/turns/list, thread/items/list, thread/goal/get and thread/queue/list. Error replies stay as recorded.
- `sessions/thread-settings.jsonl` supplies session settings for the read/resume handshake. The subtract reply comes from the original desktop oracle.
  Greeting and explain replies project their recorded start settings into the default collaboration mode. These replay fixtures are not new original-image evidence.
- `sessions/archived-chats.jsonl`: real list, restore, and delete responses from CLI `0.163.0-alpha.10.2` in installed app `26.1013.40934`.
  The isolated profile copies the original three chats.
  The archive inventory changes from three chats to two, then one, then none.
  No model turn, account mutation, or credential file is involved.
  This recording does not change the pinned `26.1007.30912` renderer used for visual comparisons.

Re-record (needs `ssh mac`; recreates `/tmp/cgx-mac/proj` and runs every server with `CODEX_HOME=/tmp/cgx-mac/home` and `-c 'notify=[]'`):

```sh
node tools/fixtures/record.mjs                    # setup explain subtract greeting readback
node tools/fixtures/record.mjs readback           # one step only
```

New sessions add new threads to `/tmp/cgx-mac/home`, so `readback` lists every thread recorded so far.

## Owned Git fixture

`sessions/git-resume.jsonl` records a cold resume and read of a fresh, owned scratch thread.
The bundled CLI version is 0.163.0-alpha.10.2.
One real `gpt-6.1-sol` turn replied `CAPTURE_OK` without tools or file changes.
The thread is `01a15c3f-2215-7233-89b6-ccf332f3756c`, named `Git capture fixture`.
Its scratch project is `/tmp/cgx-capture-service/git-real-project`.
The API-key login request is not recorded, and the owned auth file was removed after recording.
No normal profile or account data was copied.

`git-readback.jsonl` records six read-only Git commands through the same owned app-server.
The commands read status, branch, diff statistics, diff text, commits, and remotes.
The fixture branch has one committed `sample.txt` addition above its local `main`.
These responses are reference data; replaying them does not execute Git commands.

## Owned SSH discovery

The route-1 oracle records `discover-remote-ssh-connections` from an isolated original-app profile.
Its `HOME`, `ZDOTDIR`, `CODEX_HOME`, and Electron user-data paths were under `/tmp/cgx-capture-service`.
The original confirmed that its home directory used this scratch path before discovery ran.
The owned SSH config contained only the `capture-fixture` alias, pointing at loopback port 1.
No connect or save request was recorded.
The temporary original process and its CDP tunnel were stopped after recording.
The transport used the installed 26.1013 main process; the capture renderer remains pinned to 26.1007.

## Recorded archive actions

`sessions/archive-actions.jsonl` contains real `thread/unarchive` and `thread/archive` replies for the owned subtract thread.
The original CLI version is `0.163.0-alpha.10.2`.
The recording used `/private/tmp/cgx-mac/cgx-fam-set-archived-profile`, an owned copy without authentication files.
No new model turn or recording was needed.

The source is `data/cgx-fam-set-archived/original-archive-mutations.jsonl` in the retained Firstmate data.
Its SHA256 is `4278165811d3196f3a3c6b17d38e46792b075cdf9fbfe07a4cdddce9ba1e24f2`.
The extracted source lines are 7, 8, 10, 9, 11, and 13.
Notifications follow their causal replies in this extraction; every message payload stays unchanged.

Route 1 replays these actions only for local offline requests with exact recorded parameters.
Replay does not write to an app-server or update the recorded thread lists.
These responses enable action captures; they do not prove archive-list transitions or pixel parity.


## Media PDF

`media/guide.pdf` is the recorded three-page `guide.pdf` of the owned media repository (SHA-256 `7b225b54…5020`).
Pillow wrote it: each page places one 180×240 4:2:0 JPEG.
`media/guide-page-1-pdfjs.png` is its first page as the original app's PDF.js canvas drew it: the 314×418 device px under the revised preview of the retained Codex 26.1007 frame at 2x (`mediaPdf/dark@1280x832`, origin 1713, 440).
Its corners show the rounded page clip over the canvas.

## Owned Space and Sites fixture

`space-sites.json` holds synthetic offline HTTP exchanges for Sites (`/websites`, `/websites/shared`) and Space (`/files/library/nodes`, `/pages/{id}`).
The shapes come from the original zod schemas in Codex 26.1007.30912. No account call was made. Replay refuses every other request, including mutations.
The clock is frozen at 2026-10-15T17:00:00-07:00. `crates/app/tests/space_sites.rs` replays each exchange.

## Conversation streams

`conversations.json` holds offline ChatGPT conversation exchanges for `ReplayHost`: the list, one conversation, stream status (found and 404), search, and the `/f/conversation` and `/f/conversation/resume` SSE bodies.
The contract comes from Codex 26.1007.30912 (`app-initial` callers and event decoders). The responses are crafted, not a live capture, and every time field derives from the frozen clock 2026-10-15T17:00:00-07:00.
`ReplayHost` matches the whole request, so every unrecorded request, including each mutation, is refused. `crates/app/tests/conversation_fixtures.rs` proves both.
