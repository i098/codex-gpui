# Fixtures

The bundled desktop binary recorded the original `codex app-server` session set:
`/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex` (codex-cli 0.162.0-alpha.17.2), model gpt-6-astra, reasoning low.
Each line contains `dir` (`send` or `recv`) and the JSON-RPC `msg`; `t`, when present, records elapsed milliseconds.
`send` identifies a scripted client request, not an Electron main-process recording.
Original session clients use `codex-gpui` with `experimentalApi: true`.
The recorder replaces secrets with fixed placeholders (`user@example.com`, `<redacted>`, `00000000-0000-4000-8000-000000000000`, `Demo User`, `/Users/user`).

- `sessions/explain.jsonl`: initialize, thread/start in `/tmp/cgx-mac/proj` (workspace-write, approval never), one turn "In two sentences, what does this repository contain?" with command executions and an agent message.
- `sessions/subtract.jsonl`: same setup, one turn that adds `subtract` to `src/math.py` (fileChange item with diff, `turn/diff/updated` with a full unified diff) and runs the python3 check (commandExecution item, output `2`).
- `sessions/greeting.jsonl`: same setup, one turn that changes `hello.py` to print "Hello, Codex" (fileChange item with diff).
- `readback.jsonl`: one connection that sends every startup read the desktop UI uses (auth, account, config, models, features, rate limits, usage, skills, hooks, plugins, apps, MCP status, projects, sections), then thread/list with the desktop Recents params and, for each thread, thread/read (includeTurns), thread/turns/list, thread/items/list, thread/goal/get and thread/queue/list. Error replies stay as recorded.
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
