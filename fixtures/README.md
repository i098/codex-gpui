# Fixtures

The original explain, subtract, greeting, and readback fixtures use the bundled `codex app-server`:
`/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex` (codex-cli 0.162.0-alpha.17.2), model gpt-6-astra, reasoning low.
Each line is `{"t": <ms since start>, "dir": "send"|"recv", "msg": <JSON-RPC message>}`. `send` lines come from the scripted client in `tools/fixtures/record.mjs` (clientInfo `codex-gpui`, `experimentalApi: true`), not from the desktop app.
The recorder replaces secrets with fixed placeholders (`user@example.com`, `<redacted>`, `00000000-0000-4000-8000-000000000000`, `Demo User`, `/Users/user`).

- `sessions/explain.jsonl`: initialize, thread/start in `/tmp/cgx-mac/proj` (workspace-write, approval never), one turn "In two sentences, what does this repository contain?" with command executions and an agent message.
- `sessions/subtract.jsonl`: same setup, one turn that adds `subtract` to `src/math.py` (fileChange item with diff, `turn/diff/updated` with a full unified diff) and runs the python3 check (commandExecution item, output `2`).
- `sessions/greeting.jsonl`: same setup, one turn that changes `hello.py` to print "Hello, Codex" (fileChange item with diff).
- `readback.jsonl`: one connection that sends every startup read the desktop UI uses (auth, account, config, models, features, rate limits, usage, skills, hooks, plugins, apps, MCP status, projects, sections), then thread/list with the desktop Recents params and, for each thread, thread/read (includeTurns), thread/turns/list, thread/items/list, thread/goal/get and thread/queue/list. Error replies stay as recorded.
- `sessions/thread-settings.jsonl` supplies session settings for the read/resume handshake. The subtract reply comes from the original desktop oracle.
  Greeting and explain replies project their recorded start settings into the default collaboration mode. These replay fixtures are not new original-image evidence.

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

