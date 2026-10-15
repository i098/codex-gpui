# Fixtures

Real `codex app-server` sessions, recorded with the binary that the desktop app bundles:
`/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex` (codex-cli 0.162.0-alpha.17.2), model gpt-6-astra, reasoning low.
Each line is `{"t": <ms since start>, "dir": "send"|"recv", "msg": <JSON-RPC message>}`. `send` lines come from the scripted client in `tools/fixtures/record.mjs` (clientInfo `codex-gpui`, `experimentalApi: true`), not from the desktop app.
The recorder replaces secrets with fixed placeholders (`user@example.com`, `<redacted>`, `00000000-0000-4000-8000-000000000000`, `Demo User`, `/Users/user`).

- `sessions/explain.jsonl`: initialize, thread/start in `/tmp/cgx-mac/proj` (workspace-write, approval never), one turn "In two sentences, what does this repository contain?" with command executions and an agent message.
- `sessions/subtract.jsonl`: same setup, one turn that adds `subtract` to `src/math.py` (fileChange item with diff, `turn/diff/updated` with a full unified diff) and runs the python3 check (commandExecution item, output `2`).
- `sessions/greeting.jsonl`: same setup, one turn that changes `hello.py` to print "Hello, Codex" (fileChange item with diff).
- `readback.jsonl`: one connection that sends every startup read the desktop UI uses (auth, account, config, models, features, rate limits, usage, skills, hooks, plugins, apps, MCP status, projects, sections), then thread/list with the desktop Recents params and, for each thread, thread/read (includeTurns), thread/turns/list, thread/items/list, thread/goal/get and thread/queue/list. Error replies stay as recorded.

Re-record (needs `ssh mac`; recreates `/tmp/cgx-mac/proj` and runs every server with `CODEX_HOME=/tmp/cgx-mac/home` and `-c 'notify=[]'`):

```sh
node tools/fixtures/record.mjs                    # setup explain subtract greeting readback
node tools/fixtures/record.mjs readback           # one step only
```

New sessions add new threads to `/tmp/cgx-mac/home`, so `readback` lists every thread recorded so far.
