# Features and runtime setup

## Computer use runtime

Install the pinned [LCU 0.11.6 release](https://github.com/amontlabs/lcu/releases/tag/v0.11.6) on Apple Silicon macOS:

```sh
cargo run -p app --release -- --setup-computer-use
# Or, from the release bundle:
"/Applications/Codex GPUI.app/Contents/MacOS/codex-gpui" --setup-computer-use
```

The setup checks the archive's pinned SHA-256 and installs under `$CODEX_HOME/computer-use/lcu` (default: `~/.codex/computer-use/lcu`).
It keeps the signed ChatGPT app and its native helper in place.
Set `CODEX_GPUI_LCU_APP` to select another installed ChatGPT app.
Set `CODEX_BIN` to select the app-server executable, as for normal startup.
Setup uses that exact executable's config writer to register the `lcu` Codex relay and trust the original Stop, Interrupt, and SubagentStop hooks.
It preserves unrelated MCP servers, hooks, and LCU fields; the original provider policy replaces conflicting LCU fields.
It disables an existing `cua_repl` server and installed `unified-computer-use` plugins to prevent two active providers.
Setup updates older LCU installations and fails if the signed runtime is missing, the manifest differs, or the CLI cannot trust the hooks.
Normal startup reads the saved configuration; Rust does not run a second MCP client or send duplicate turn cleanup.

Restart the app after command-line setup.
Settings > Computer use runs setup off the UI thread and reloads the app-server configuration after success.
Registration does not grant macOS permissions, approve applications, enable Chrome or audio, or prove native input works.
The original runtime requests its permissions and approvals when you use it.
This setup applies only to the Codex app-server backend, not ACP.

The Settings page reads the helper's allowed apps and sound setting through the native host service.
The Any App switch changes the provider and its existing lifecycle hooks together.
Stop active chats before changing that switch or installing the runtime.
Failed writes keep the last confirmed Settings values.

Thread items retain MCP arguments, text, images, structured results, errors, progress, and unknown metadata.
Open a tool row to inspect its result.
Repeated Stop actions send one interrupt while the turn stops.
LCU and the trusted app-server hooks own cleanup, including reset; the native app does not send another `turn_ended`.
An in-flight JavaScript call can delay Interrupt cleanup; the original hook can report a timeout before cleanup completes.

Run `cargo test -p app --test computer_use_lifecycle` on macOS for native Stop and thread-projection regression checks.

## SSH connection discovery

The host service supports `discover-remote-ssh-connections` and returns an `sshAliases` list.
It reads `$HOME/.ssh/config` and its top-level `Include` files without running SSH or changing connection settings.
Discovery follows the original literal-host, quote, and include rules.
Settings > Connections uses the same discovery for its SSH chooser.

Run `cargo test -p app --lib --test host ssh_discovery` for the parser and host-service checks.

`Root::on_host` binds a window to an existing host connection.
Thread reads, changes, and events use that connection, with separate thread caches for each window.
Child widgets keep their connection when another window becomes active.
Remote windows reject local desktop storage requests.
Run `cargo test -p app --test thread_hosts` for the two-host native check.

## Terminals

Open a chat, then choose **View > Open Terminal**.
Use `Cmd+J` or `Ctrl+\`` to show or hide the bottom panel without restarting its shell.
The panel sends PTY input, output, resize, and termination through the app-server `command/exec` methods.
Before typing, use `Option+number` on macOS or `Alt+number` to choose a source directory in a multi-root project.
The terminal warns when its shell belongs outside the chat's current workspaces.

Background command tabs retain their output after completion.
Both terminal views support ANSI colors, text selection, copy, scrollback, HTTP(S) links, and font/theme changes.
Like the original app, the views keep 1,000 scrollback rows plus the visible screen.
Output follows the bottom only while the view is already at the bottom.
The recorded PTY stream in `fixtures/sessions/terminal-shell.jsonl` comes from the bundled macOS app-server.
Run `cargo test -p app --test terminal_states` on macOS for native terminal checks.

Command details include agent input from `item/commandExecution/terminalInteraction`.
The input remains in the output after the command completes.
Run `cargo test -p ui --lib command_completion_keeps_streamed_output_and_failure` to check retention.


## Recommended skills

The local host supports `recommended-skills` and `install-recommended-skill`.
The catalog uses `CODEX_HOME/vendor_imports/skills` and a ten-minute cache in `skills-curated-cache.json`.
Installation writes to `CODEX_HOME/skills`, or `<installRoot>/.codex/skills` for a selected workspace.
Existing skills remain unchanged unless the request explicitly replaces them or supplies a Markdown override.
The host rejects remote host IDs and paths that leave the source or installation root.
This service does not enable the Plugin or Skill directory UI.

Run `cargo test -p app --lib host::skills::tests` for the catalog and installation checks.

## Connector OAuth callbacks

Use `Root::start_app_connection` with `oauth::ConnectRequest` to start a connector connection.
The existing authenticated `CloudHost` sends the original start or reauthentication request.
The provider returns through `codex://connector/oauth_callback`; pass that URL to `Root::receive_oauth_redirect`.
The root checks the pending state, account, connector, and optional reauthentication link before it reports success.
Duplicate callbacks do not send a second request, and failed callbacks can retry.
Provider URLs, authorization codes, and pending OAuth state never enter saved window settings.

`ConnectionEvent` reports provider opening, connection completion, failure, replacement, and cancellation to the originating control.
That control owns its plugin resume action and connected-app refresh.
Call `Root::cancel_app_connection` when the originating connection is cancelled.
Health, Plaid, and mobile app-specific-password callback URLs remain gated as excluded ChatGPT or mobile products.
OS URL delivery and scheme registration belong to the window-shell family; this module provides the in-process callback lifecycle.

Run `cargo test -p app --lib oauth::model::tests` for URL and identity boundaries.
Run `cargo test -p app --test oauth -- --test-threads=1` for native root lifecycle checks with schema-level responses.
These checks do not sign in or change a real provider account.

## ACP backend

Set `CODEX_GPUI_ACP` to the command line of an [Agent Client Protocol](https://agentclientprotocol.com) agent to use that agent instead of `codex app-server`:

```sh
CODEX_GPUI_ACP="omp acp --model anthropic/claude-sonnet-5-5" cargo run -p app --release
```

The app then speaks ACP v1 over stdio to the agent (`crates/bridge/src/acp.rs`); without the variable it uses `codex app-server` as before.
Set the model and other agent settings on the agent's command line; the app does not select them.
The app shows a toast after it connects: `<agent> over ACP: only chats, streaming, Stop and approvals work`.
Arbitrary interactive CLIs are not compatible; the agent must speak ACP.
The backend uses only the ACP methods that every agent must have.
It does not use optional agent capabilities (`loadSession`, session list, image prompts, model and mode config options), also when the agent advertises them.

| Feature | Codex app-server | ACP agent |
|---|---|---|
| New chat (`thread/start`) | Yes | `session/new` in the project folder, no MCP servers |
| Send a message (`turn/start`) | Yes | `session/prompt`, text only; one turn at a time per chat |
| Streamed reply | Yes | `agent_message_chunk` updates become `item/agentMessage/delta` |
| Tool calls | Yes | Not shown; a `tool_call` update ends the current reply message, so the text after it is a new message |
| Stop (`turn/interrupt`) | Yes | `session/cancel`; open permission requests are answered `cancelled` |
| Turn end and errors | Yes | `end_turn` is completed, `cancelled` is interrupted, other stop reasons and errors are failed |
| Approvals | Yes | `session/request_permission` keeps its request id, and the prompt lists the agent's option labels; Allow and Allow similar choose the `allow_once` option, Decline chooses `reject_once`; `allow_always` is never chosen, because it can outlive the session; without the matching option the answer is `cancelled` |
| Recents and reopen a chat | Yes | Chats of this run only |
| Model, effort, Fast mode, permission mode | Yes | No; the agent's command line and settings apply |
| Attachments, file mentions, skills | Yes | No |
| Steer, queue, edit, revert, fork, rename, pin, archive | Yes | No |
| Account, sign-in, usage, config | Yes | No; the app does not ask for them |
| Commit message generation | Yes | No |

An unsupported method gets the JSON-RPC error `<method> is not supported by the ACP backend`, and the app shows its usual failure state.
The client offers no file system or terminal capability, so the agent uses its own tools.

## Durable cloud transport

`Bridge::connect_cloud(endpoint, token, account_id)` connects to the durable cloud app-server.
Use `bridge::CLOUD_ENDPOINT` for the original service and call `initialize` before sending requests.
Run the blocking connection step off the UI thread.
The bridge sends the token through the original WebSocket subprotocol and preserves the existing request, response, and event APIs.
Remote endpoints require TLS; plain WebSocket connections are limited to loopback.
The caller owns authentication changes and reconnects.
Closing the socket fails pending requests, and dropping the bridge closes its connection.
TLS uses an explicit Ring provider with standard and system trust roots.
The vendored Tungstenite 0.28 package omits raw handshake-request trace output because it contains the authentication token.

`Bridge::start_cloud_environment_onboarding(account_id, config_id, params)` starts a thread bound to a saved cloud environment.
It rejects local connections and a different account before sending the request.
It replaces local paths with the original deferred onboarding environment, service-tier rules, and plugin metadata.
Cloud thread start/resume and turn start/add requests follow the original tier and personality rules.
The onboarding API returns the server JSON without requiring local-thread fields.
This API does not send the first model turn or finish setup.
The environment UI and host-aware thread routing remain separate work.

Run `cargo test -p bridge` for real loopback WebSocket exchanges, disconnect handling, and credential checks.

## Sign-in

The sign-in screen reads `configRequirements/read` before it shows account methods.
Missing, invalid, and failed requirement reads keep those methods hidden.
Null requirements fall back to `config/read` and its `forced_login_method`.

The screen uses the original feature gates for Google, Microsoft, Amazon Bedrock, and browser account lookup.
Browser account lookup uses the installed app's native eligibility helper and a bounded localhost callback.
It does not read browser profiles or cookies.
Credential fields support the native clipboard and keyboard editor.
The OpenAI API-key field shows its text, as the original does.
The Bedrock secret fields mask both displayed text and operating-system text reads.
The copied sign-in link matches the browser URL, including provider and account selection.
Bedrock sign-in discovers AWS profiles, accepts explicit credentials, restarts the app-server, and checks GovCloud requirements.
The GovCloud warning remains open after restart until the user selects Continue.
No test signs into a real account or changes AWS credentials.

## Onboarding

Onboarding supports `/welcome`, `/onboarding`, `/select-workspace`, and `/first-run`.
Launch `codex-gpui /select-workspace` to open the workspace route directly.
The same startup argument supports `/welcome`, `/onboarding`, and `/first-run`.
A new ChatGPT account opens setup when the server reports incomplete onboarding and no project is available.
Role choices retain the original coding and non-coding groups.
The import prompt saves the choices before opening Settings > Import.
The workspace picker uses real project folders; Skip creates a new `Documents/ChatGPT/New project` folder.
Failed saves keep the current page open.
The first-run tour keeps the app shell and shows the IDE, cloud, code, and autonomy pages.
Its background uses the original noise algorithm; a pinned fixture clock holds the first frame.
Finishing the tour saves `viewed2025-09-15-nux` and opens Home.

## Native Computer Use service

The host keeps one managed native helper at `$CODEX_HOME/computer-use/Codex Computer Use.app`.
Setup copies the signed OpenAI helper without changing its signature.
The host exposes installation, explicit helper startup, Locked use, and Appshots through `Host::computer_use_native`.
Construction and status reads do not start the helper or request macOS permissions.
Locked use requires the original feature, remote-control, and account-policy gates.
`LockedUseState::visible` applies those gates to confirmed host state.
The Computer use page shows Locked use only when the native policy and confirmed state are supplied.
The control requires a trusted callback and stays disabled while a change is pending.
The helper's nested installer handles `status`, `install`, and `uninstall`; errors remain visible.
Appshots use the original Apple Event protocol and retain request IDs through completion or cancellation.
Capture files must be PNG or JPEG files inside the helper's temporary folder and at most 25 MiB.
Accessibility and Screen Recording remain separate macOS permissions.
Model-driven computer use stays on the LCU relay and original native sockets.
Do not grant SSH Automation access to test the separate Appshots Apple Event path.
These native operations require macOS; Linux checks cover policy, capture-file boundaries, and request state only.

## Native Security workbench

`bridge::security::Client` uses the installed `codex-security` MCP provider through the existing app-server connection.
It opens an ephemeral, read-only host thread and preserves its identity on tool calls.
The bridge does not embed the Security SDK or install the provider.

The API reads scans, repositories, findings, and target metadata.
It also handles scan setup, continuation delivery, cancellation, and finding remediation.
Failed or unconfirmed delivery keeps the ownership claim when another turn might still be active.
Rejected or mismatched handoff claims never cancel a scan owned by another client.
Gate resolution requires the rollout gate and an enabled, available `codex-security@` plugin.

Run `cargo test -p bridge --lib security::tests` for gate, response-validation, pagination, and delivery-ownership checks.
These tests use a local protocol peer and do not start a model turn.
The Explore entry opens the native Security workbench when the rollout gate and provider are available.
The workbench supports scan setup and progress, repository history, finding filters, triage, remediation, and source viewing.
Access changes stay within their own window; unavailable or remote hosts cannot start scans.

Run `cargo test -p ui --lib screens::security::tests` for workbench access and scan-setup checks.
The native workbench does not yet have current visual-parity proof against the original app.


