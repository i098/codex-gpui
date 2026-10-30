# codex-gpui

codex-gpui is an open-source port of the Codex desktop app to Rust and [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
The original is the Codex section of the macOS ChatGPT app.
The goal is pixel parity with the original: the same layout, text, colors and icons, device pixel for device pixel.
The app drives the real `codex app-server` over stdio, the same JSON-RPC transport that the original uses.

[Build and run](#build-and-run) · [Architecture](#architecture) · [Parity method](#parity-method) · [Contributing](CONTRIBUTING.md) · [Security](.github/SECURITY.md) · [Changelog](CHANGELOG.md)

## Requirements

- macOS on Apple silicon (arm64).
- Rust 1.98.1. `rust-toolchain.toml` pins it, so `rustup` installs it on the first build.
- A `codex` binary. The app uses the first one that it finds:
  1. `$CODEX_BIN`.
  2. `/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex`, the binary that the ChatGPT app bundles.
  3. `codex` on `PATH`.

The workspace also builds on Linux, with GPUI's no-op text system, for CI (fmt, clippy, tests).
The self-hosted CI job disables dev/test debug information and incremental compilation.
It uses only its own workspace's `target` directory and removes that directory after success or failure.
CI skips draft pull requests by default. Marking a pull request ready for review starts the normal run; maintainers can also use Actions > ci > Run workflow to test a draft branch. A newer push to a pull request cancels its older run; pushes to `main` always run. A pull request that changes only the listed documentation files (`paths-ignore` in `.github/workflows/ci.yml`) starts no run.

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

## Build and run

```sh
cargo run -p app --release
```

The app keeps its window state in `~/Library/Application Support/codex-gpui/state.json`.
Host settings use the local `settings.write(key, value)` service and persist beside that file in `host-settings.json`.
Theme and window-close preferences remain in `state.json`.
The S034 replay checks the menu-bar switch, its saved values, and Back navigation against `refs/interactions/s034.json`.

The local `get-global-state` host request reads a named value from `.codex-global-state.json` without changing the profile.
Missing files and keys return `{"value": null}`; malformed files and invalid requests return errors.
Thread replies retain the optional rollout `path` for archived-chat search.
Run `cargo test -p app --test host global_state` for the filesystem and request checks.

`Confirmation::settings_dialog` selects the settings dialog style without changing confirmation or focus behavior.

Settings > Archived chats lists local archives and account-authorized cloud tasks.
Search, project filters, and sorting keep the archive on one page.
Unarchive offers View; local deletion asks for confirmation.
Delete all uses the current local archived inventory, regardless of the active search or type filter.
Run `cargo test -p app --test archived_chats -- --test-threads=1` for restore, View, Retry, and individual, project, and bulk deletion.

Settings > Git stores branch, merge, push, draft, and pull request instruction preferences in `host-settings.json`.
Review delivery uses the existing app-server config store.
Branch prefix saves on blur or Enter; multiline instructions save after three idle seconds or when the page closes.
Save and Command-S also write pending instructions.
The native `cargo test -p app --test git_settings -- --test-threads=1` replay checks autosave, write recovery, shutdown flush, and confirmed worktree preferences.
PR watch preference changes update active PR heartbeat instructions; paused automations remain unchanged.
Settings > Worktrees provides General and Environments tabs, root and retention preferences, live worktree rows, and row actions.
The tabs stay accessible while the content scrolls.
Pinned conversations prevent worktree deletion.
The Environments tab lists project and inherited files and edits setup scripts, cleanup scripts, platform overrides, and named actions.

Settings > Voice keeps microphone, sound, screen-context, and local dictionary preferences in the same host settings store.
Its gate adapter accepts trusted original Statsig and capability values; missing inputs never enable Voice.
Saved recording controls read `$CODEX_HOME/dictation-history` and the legacy `transcription-history.jsonl` file.
Downloads stream audio bytes and never replace an existing file.
Retry transcription buffers at most 128 MiB; larger saved recordings remain available for download.
Account dictionaries, voice selection, language selection, and retry transcription use the existing authenticated host.
The shared keyboard service owns global shortcut storage and registration.
Run `cargo test -p app --test voice_settings` for recording, preference, gate, and binary HTTP checks.
Changes > Load full files reads Git objects and working files when their rows enter the view.
Expand a context separator to show 20 more lines in either direction or the complete gap.
The view checks full content against the displayed patch and preserves the patch when a read fails.
Partial updates without a previous object ID keep their recorded hunks instead of guessing a previous revision.
Line wrapping, word differences, and full-file loading use the existing host settings store.
Run `cargo test -p app --test diff_file_reads` for file boundaries and `cargo test -p app --test diff_context` on macOS for expansion and retry.
Image and PDF previews read bytes from each selected Git version or working file.
PDF previews use macOS rendering and keep separate page controls for each side.
The controls appear when the pointer or keyboard focus enters the PDF preview.
The file tree omits zero change counts and groups thousands in large totals.
Oversized text retains its change counts and Open in editor action instead of rendering a partial file.
Large-diff notices clip their fill and controls together while painting the outer shadow separately.
The header capsule shadows continue into the notice area, as in the original.
Stage and Unstage actions apply only to the separate Unstaged and Staged sources, not the combined Uncommitted view.
Use the Rich preview option to switch SVG files between rendered previews and text.
Run `cargo test -p app --test diff_preview` on macOS for image version and PDF page checks.

The desktop host exposes `git-review-patch` for complete, binary-capable patches from the selected review source.
The response uses `contentsBase64` to preserve patch bytes, including non-UTF-8 text.
`git-changes` accepts `hideWhitespace`; raw patch export never applies that display filter.
Stage, Unstage, and Revert accept the same filter while retaining hidden whitespace edits and the stale-snapshot check.
Revision exports use `beforeRef` and `afterRef`; staged and working-tree exports retain their separate Git states.
Raw export omits untracked files above the original 256-file limit but keeps tracked changes. This does not limit the existing Changes list.
Run `cargo test -p app --test diff_file_reads` for real Git apply round trips and source-boundary checks.

`ui::screens::diff::panel::word::changes` returns removed and added UTF-8 byte ranges for the original `word-alt` comparison.
It ignores a final LF or CRLF and returns no highlights above 1,000 UTF-16 units per line.
The API preserves the original edit-path order and final-span boundaries. It does not change rendered controls.
Run `cargo test -p ui --lib screens::diff::panel::word` for its boundary checks.

`ui::screens::diff::panel::patch::file_path` reads the target path from one Git file patch.
It decodes quoted names and handles rename, copy, binary, empty-file, and deleted-file metadata.
Complete file headers preserve filenames under no-prefix and mnemonic-prefix settings. Hunk contents never supply a path.
The parser does not access or authorize filesystem paths.
Changes uses it for aggregate patches, preserving quoted names and metadata-only file entries through the file headers and Copy path action.
File headers collapse display whitespace and show rename sources. Copy path keeps the decoded path unchanged.
Metadata-only files show notices instead of raw Git headers.
Unified deletions use source-sized striped markers. Split deletions keep solid markers.
View changes selects Last Turn and opens only the matching requested path.
File expansion follows exact response paths; collapsed rows retain their file contents.
Expand all resets the path overrides.
Run `cargo test -p ui --lib screens::diff::panel::patch` for its boundary checks.

`Host::review_repositories(thread, cwd)` reads the assigned project roots and worktrees owned by that thread.
It returns each repository root, working directory, and display name without changing a rendered control.
Run `cargo test -p app --test diff_repositories` for the real-Git ownership and path checks.

The local host lists managed worktrees and saves tracked and untracked files in `refs/codex/snapshots` before targeted deletion.
It rejects deletion of initialized nested Git repositories.
Worktree creation uses the saved root, upstream refresh preference, and selected local environment.
Local environment files support inherited listing, validated atomic writes, and revision conflict checks.
All environment callers share one parser and the `sha256:<hex>` revision format.
Identical saves succeed without replacing the file; stale saves preserve external edits, symlinks, and file permissions.
Workspace selection preserves unrelated state and distinguishes a disabled selection from an unset selection.
These trusted native operations accept explicit absolute config paths; they are not model-callable tools.
The `git-pr-watch-update` host operation changes only active PR heartbeat instructions.
Run `cargo test -p app --test settings_git_family` for these file and Git operations.
Settings > Import uses the original feature gates and the app-server's external-agent import methods.
It shows detected providers and recorded import history.
Automatic sync keeps the source selection and runs at the original 12-hour interval.
Customize changes future imports only; disabling sync keeps saved selections.
Cursor projectless imports keep their workspace and output-folder records without adding a named project.
Settings > Connections saves SSH definitions under `CODEX_HOME` and keeps a separate app-server bridge for each connected host.
WSL rows appear only when Windows discovers a distribution.
The editor keeps the entered values when a save fails.
The SSH chooser lists aliases that are not saved yet.
Manual entry reports all validation errors before it writes a connection.
Connection colors use the original per-host setting and survive a restart.
Mobile setup uses the app-server remote-control status, pairing, and authorized-device APIs.
It does not bypass account authorization or MFA.
The existing native policy controls remote-device visibility.
Capture tests require separate, owned `HOME` and `CODEX_HOME` directories.

`ui::settings_kit` provides a multi-select control and an external-link row for Settings pages.
The multi-select emits `SelectionChanged`; the caller saves the value and can restore it with `set_checked` if saving fails.
`ui::confirmation::Confirmation` emits one confirm or cancel result and returns focus to the previous control.
Use `.with_documentation(label, url, cx)` to add a link that opens without confirming or cancelling the action.
Tab and Shift-Tab cycle through the buttons, optional checkbox, and documentation link.
Enter or Space opens the focused link; Escape cancels and returns focus to the previous control.
Run `cargo run -p ui --example settings_kit -- dark confirmation-link /tmp/confirmation-link.png` for a native focus-ring capture.
Use `Composer::settings_field` for text and `.monospace_field()` for scripts and commands.
Textareas scroll inside a fixed initial height, resize to a 78-pixel minimum content height, and keep the caret visible after resizing.
Use `Toast` for existing information, error, and success messages.
Use `Composer::settings_form_field` for compact form inputs and `settings_form_secret` for their masked values.
Subscribe to `SettingsFieldSubmit` to handle Enter without dropping focus.
Run `cargo test -p ui --test settings_kit enter_respects_single_line_and_multiline_form_fields` for the real keyboard check.
Run `cargo test -p ui --test settings_kit` for the keyboard and locked-choice check.

`app::diff_content::read_full_file` loads typed before/after data through the existing Git object host.
It preserves line endings and the host's working-file fallback policy, with a 5 MiB object limit.
Missing or oversized objects return `FullError::Unavailable`; invalid disk-fallback paths or reply structures return `FullError::Failed`.
Run `cargo test -p app --test diff_content` for real Git object and repository-boundary checks.
Thread and standalone Changes views load visible full files according to the saved `load-full-files` preference.
They validate full text against the displayed patch and discard replies from an older content generation.
Restored patches without object IDs use the HEAD file for the before version and apply the same patch validation.
Select an unmodified-lines gap to expand it, or use its controls to add 20 lines from either edge.
Expanding context preserves the reviewed line position, including wrapped rows, and enforces the normal diff display limits.
File headers remain visible during scrolling.
Diff code and context rows paint in their own opaque scroll layer, as in the original renderer.
Failed reads retain the patch and provide Retry.
Only Staged and Unstaged sources show staging actions. Uncommitted combines both states and does not expose those actions.

The Plugins rail opens the native plugin directory and the Skills page.
Both pages load app-server data and support search.
Plugin installation, removal, enablement, and sharing wait for server responses.
Failed operations keep the last confirmed catalog state.
Share creates a remote link only after Copy link; only personal-marketplace plugins can publish a shared version.
Plugins with install-time authentication require confirmed app connections before Try now.
New skill prepares an editable draft without sending a turn.
App connections open the original browser setup flow; Check connection reads the app list again.
Manage opens Settings > Plugins, which owns the management tabs and marketplace controls.
The existing 36 reference frames do not cover these routes.
Run `cargo test -p app --test plugins -- --test-threads=1` on macOS for the native failure and draft checks.

Settings > Usage & billing reads plan limits and reset credits from the app-server.
It keeps the original account gates for credit controls, enterprise usage requests, analytics, and balance history.
Reset retries keep the same idempotency key; billing changes keep their confirmed state when a request fails.
An uncertain reset keeps its original credit list until the server confirms the result.
Account changes clear drafts and reset confirmations before the next control action.
Analytics refreshes keep the selected period; credit speed groups use the recorded speed.
The input fields reuse `Composer::settings_field`, including selection, clipboard access, and input methods.
The renderer paints each limit card within the scroll layer and paints the sticky tabs in a separate layer.
The sticky fade uses the original compositor layer's pixel origin to preserve gradient dithering.
Disabled Analytics arrows use their painted bounds for the opacity layer.
Card contents keep the original rounded overflow clip.
Thin SVG strokes use Skia's original hairline alpha adjustment before rasterization.
Run `cargo test -p ui -p app --lib usage` for the protocol and validation checks.
Run `cargo run -p ui --example usage -- /path/to/usage.png` for the recorded native screen.
Add `confirm-reset` after the output path to capture the confirmation state without redeeming a reset.
Add `analytics` or `reset-error` to capture the recorded error states.
Add `links` to check plan, credit, and billing destinations without opening a browser.
Add `account-switch` to check that an account change discards the previous reset confirmation.

Cloud environment routes use `Root::open_cloud_environment` and the existing authenticated host transport.
The legacy editor preserves unchanged secret values and keeps package version choices in the environment draft.
Cloud access follows the original account policy and feature gates; Cloud computer stays hidden on the Codex surface.
Each Cloud page keeps the account and user identity used by its first access request.
An identity change stops later environment operations before the host sends them.
Container-image errors remain inside the editor row; name checks can complete independently.
Repository selection loads GitHub connectors and visible account links before requesting repositories.
Connection retries preserve the saved-environment list state and keep keyboard dismissal available while loading.
The create dialog uses the original setup artwork and a full-window native overlay.
The `375130565` feature gate also hides legacy environments when the account otherwise permits cloud access.
Run `cargo test -p app --lib cloud::tests` for the account and feature-gate checks.
Run `cargo test -p ui --lib screens::cloud::model::tests` for secret updates and `.env` parsing.
Creating an environment opens an account-bound cloud connection and starts the recorded setup skill.
Existing setup conversations resume without starting another model turn.
The repository picker keeps selections across searches, pages, retries, and provider changes.
Run `cargo test -p ui --lib screens::cloud::tests` for repository and modal state checks.
Run `cargo test -p app --lib reopening_completed_setup_resumes_without_another_model_turn` for setup resumption.
Run `cargo run -p ui --example cloud_capture -- /tmp/cloud.png current-repositories` for the native repository picker.
Selecting another settings section clears the Cloud view.
Run `cargo test -p ui --lib screens::settings::tests::leaving_cloud_restores_the_selected_settings_page` to check this transition.
Use `settings-return` with `cloud_capture` to capture the return to General.
Original-versus-native pixel validation for these pages remains pending.

Scheduled > New task accepts instructions, a daily or weekly repeat, and a local time in `HH:MM` format.
Create saves the definition through `automation-create` to `CODEX_HOME/automations/<id>/automation.toml`.
The app reloads the task list after a successful write and retains the draft if the write fails.
The existing host schedules these definitions; this app does not start another scheduler.
Saved hourly and minute schedules show their interval in the task list.
`Previous runs` shows local stored history and read state for the selected task.
Its actions mark completed runs as read or archive them after confirmation.
Bulk actions skip running, archived, and account-owned runs and report partial archive failures.
Run `cargo test -p app --test scheduled_history -- --test-threads=1` on macOS to check these actions and the existing View route.
The row menu's `Run now` sends `automation-run-now`; the host starts one `codex app-server` thread and turn per task folder.
It runs tasks with project folders or a project target (the project's first root from `.codex-global-state.json`), in a new Git worktree for the original default environment or in the folder for the local one.
For a project target, the host saves the new thread's local project assignment before it starts the turn.
An assignment write failure stops the run; legacy folder tasks do not change project assignments.
Cron threads receive the original memory rules, inbox response rules, and desktop developer context through `thread/start`.
The app-server retains saved base instructions.
On macOS, automation threads use executable local app tools through a private native pipe.
The tools need the `codex-app-tools` adapter and Node runtime installed with `ChatGPT.app`.
If the adapter or pipe cannot start, the task keeps the app-server's original configuration.
The pipe stops safely if its temporary directory is removed.
The catalog exposes automation creation, full updates, deletion, local project and thread lists, history, pin, archive, and title changes.
Unsupported delegation, remote hosts, plugin management, and terminal operations stay out of the catalog.
Proposal cards open the shared task editor. Cancel changes nothing; saving preserves the reviewed definition.
Completed task tools use only this card, not a second MCP activity row.
The card clips its foreground to the original 20 px squircle; its background and shadow remain outside the clip.
Projectless tasks use an owned folder under `~/Documents/Codex/<local-date>/<prompt-name>/`, with separate `work` and `outputs` folders.
The `projectlessWorkspaceRoot` host setting overrides the root.
The host saves projectless thread IDs, workspace-root hints, and output paths in `.codex-global-state.json`.
Heartbeat tasks resume their existing thread with its selected model and permissions.
The app opens that thread after a successful start, unless you navigate away while the run starts.
Completed automation runs update the Scheduled unread dot from saved run state.
Opening a run clears its unread mark; opening Scheduled does not.
Desktop notifications follow the saved turn mode and task policy without moving focus.
Run `cargo test -p app --lib automation_notifications -- --test-threads=1` on macOS for completion, duplicate, focus, and notification-click checks.
Run `cargo test -p app --test task_create -- --test-threads=1` on macOS for the native creation and reload proof.
The New task options menu also opens the full manual editor or a new chat with the original task setup prompt.
The manual editor saves Name, multiline Prompt, Interval, Daily, Weekdays, Weekly, Monthly, and Custom schedules.
Name and time controls use single-line inputs; Prompt accepts multiple lines.
Custom schedules support Hourly, Daily, Weekly, Monthly, and Yearly repeats, with interval, weekday, month-day, month, minute, and local time fields.
The time presets use 09:00, 14:00, 18:00, and 22:00.
The row menu's Edit action updates the task through `automation-update`, without changing its identity or unrelated definition fields.
The editor keeps an existing advanced rule unchanged when only Name or Prompt changes.
Prefilled drafts stay in Create mode until their data includes an existing task ID.
Escape closes an open schedule picker first; otherwise, it discards the editor without saving.
Command-Enter saves the editor, including when Name or Prompt has keyboard focus.
The local Trigger menu keeps Schedule active and explains why cloud-only Date and time is unavailable.
Advanced controls update notifications, new-chat or existing-chat execution, automatic archive, project, model, and effort.
Switching an existing-chat task back to new-chat execution supplies its current local workspace.
Model and effort choices come from app-server `model/list` and `config/read`; existing chats come from `thread/list`.
Inherited model and effort values follow the original automation catalog precedence, not ordinary chat defaults.
The editor saves those choices atomically through `automation-create` or `automation-update` and retains unrelated definition fields.
Run `cargo test -p app --test task_create editor:: -- --test-threads=1` on macOS for the native editor, schedule, and Create menu checks.
Run `cargo test -p app --test automation_run_now` for the `Run now` requests over a stdio app-server stub.
Use `-- --include-ignored` on macOS to check the installed adapter.

Cloud automation storage uses `cloud-list-automations`, `cloud-read-automation`, and the `cloud-automation-*` mutation endpoints.
These endpoints use the signed-in account's original `/automations` and `/automation` APIs.
Mutation requests carry the loaded task's `expectedIdentity`; account changes reject the write.
Cloud edits keep schedule, timezone, notification, and model fields that the editor does not change.
Local endpoints reject `automationSource: "cloud"` instead of reading or changing a matching local task.
Run `cargo test -p app --test host cloud_automation_` for source isolation and account-change checks.

### Legacy Codex Cloud threads

Open an existing task with `cargo run -p app -- /remote/<task-id>`.
The `/hotkey-window/remote/<task-id>` path selects the compact content variant.
The page uses the app-server's ChatGPT identity for the original `/wham/tasks` API.
It reads task turns, streams setup logs and thread events, and supports image previews and follow-up image uploads.
A log download failure does not hide the task's setup error.
Applying changes selects a configured local project with the same Git remote.
Apply and Revert use a temporary Git index, so they do not change the user's staged files.
The result dialog lists applied, conflicted, and skipped paths.
`Root::open_remote_task` and `Root::remote_page` provide the same page to native window hosts.
This route is separate from the durable `codex_cloud` thread service.
Run `cargo test -p app -p ui --lib cloud` and `cargo test -p app --test host` for the protocol and native state checks.
Run `cargo test -p app --test cloud_task_native -- --test-threads=1` on macOS for the native state and action checks.
Those checks use constructed responses from the original schema and exercise real Git apply and revert operations.
They do not certify live Cloud access or original pixel parity.

## Managed update policy

General settings shows the read-only `In-app updates` row with `Managed` when the app-server disables `in_app_updates`.
The row uses `experimentalFeature/list`, not a local preference.
The app hides the row when that policy is absent or enabled.

### Legacy Codex Cloud threads

Open an existing task with `cargo run -p app -- /remote/<task-id>`.
The `/hotkey-window/remote/<task-id>` path selects the compact content variant.
The page uses the app-server's ChatGPT identity for the original `/wham/tasks` API.
It reads task turns, streams setup logs and thread events, and supports image previews and follow-up image uploads.
A log download failure does not hide the task's setup error.
Applying changes selects a configured local project with the same Git remote.
Apply and Revert use a temporary Git index, so they do not change the user's staged files.
The result dialog lists applied, conflicted, and skipped paths.
`Root::open_remote_task` and `Root::remote_page` provide the same page to native window hosts.
This route is separate from the durable `codex_cloud` thread service.
Run `cargo test -p app -p ui --lib cloud` and `cargo test -p app --test host` for the protocol and native state checks.
Live-account and strict pixel validation remain pending for this page.

## Release bundle

```sh
tools/bundle.sh
```

The script builds the release binary on the Mac build host through `tools/mac.sh`.
It assembles `Codex GPUI.app` (bundle id `dev.codexgpui.app`, version from `tools/version.sh`, macOS 13.0 or later).
By default, it signs the bundle ad hoc, checks it with `plutil -lint` and `codesign --verify --strict`, and zips it with `ditto`.
The zip goes to `/tmp/cgx/release/codex-gpui-<version>-macos-arm64.zip`.
The script does not open the app.
Bundle assembly requires Python 3.
It generates `THIRD_PARTY_NOTICES.txt` from production Rust dependencies and their available license files, then includes the file in Resources.

### In-app updates

Development bundles have no update feed, so update checks and the update menu stay off.
For a release, set `CODEX_GPUI_UPDATE_FEED`, `CODEX_GPUI_UPDATE_PUBLIC_KEY`, and `CODEX_GPUI_SPARKLE_FRAMEWORK` before running `tools/bundle.sh`.
The framework path refers to the Mac build host and must contain the verified Sparkle 2.9 framework.
Set `CODEX_GPUI_SIGN_IDENTITY` to the release signing identity.
The script rejects an update feed with an ad-hoc signing identity.
The script embeds the feed and Ed25519 public key, then signs Sparkle's helpers, framework, and app with that identity.
The release workflow does this when the repository secrets `SPARKLE_ED_PRIVATE_KEY`, `SPARKLE_ED_PUBLIC_KEY` and `CODEX_GPUI_SIGN_IDENTITY` exist.
It then runs `tools/appcast.py` and uploads `appcast.xml` and `<zip>.sha256` beside the zip; run `python3 tools/test_appcast.py` for its checks.
General settings has an `Automatically check for updates` switch that sets Sparkle's `automaticallyChecksForUpdates`.
The release owner must supply the production feed and complete signing and notarization before distribution.

Until a production feed exists, `tools/install-mac.sh` updates an installed copy in place.
Run it on the Mac through `tools/mac.sh -l install "nice -n 19 bash tools/install-mac.sh $(git rev-parse HEAD)"`.
It builds the normal bundle, replaces `~/Applications/Codex GPUI.app`, and keeps the old bundle as `~/Applications/.Codex GPUI.previous.app`.
It never opens, activates, or closes the app; a running copy changes at its next launch.

Sparkle checks, downloads, verifies, installs, and relaunches the app.
The rail shows download progress and enables `Update available` only when installation is ready.
The app reads the `in_app_updates` policy through `experimentalFeature/list`.
A disabled policy removes the update action and shows the read-only `Managed` row in General settings.
The native Sparkle dialogs show check progress, no-update results, and update errors.
Run `cargo test -p app --lib updater` on macOS for the feed and managed-policy checks.
Run `cargo test -p app --test updater -- --test-threads=1` for native rail visibility and input checks.

The app icon is the original icon from `ChatGPT.app/Contents/Resources/Assets.car`.
To make `assets/app/AppIcon.icns` again, run this on the Mac:

```sh
swiftc -O assets/app/icon.swift -o /tmp/icon
/tmp/icon Assets.car /tmp/AppIcon.iconset
iconutil -c icns /tmp/AppIcon.iconset -o assets/app/AppIcon.icns
```

### Versions and releases

The workspace `version` in the root `Cargo.toml` is the app version.
Every crate under `crates/` except `skia-hairline-sys` inherits it.
`tools/version.sh` prints it.
`tools/version.sh 0.2.0` bumps it in `Cargo.toml` and `Cargo.lock` and adds a `## [0.2.0]` section under `## [Unreleased]` in `CHANGELOG.md`.
Merge the bump, then push the tag `v0.2.0`.
The `release` workflow builds the bundle on a GitHub macOS arm64 runner and stops if the tag is not `v` plus the workspace version.
It publishes `codex-gpui-0.2.0-macos-arm64.zip` as a GitHub release with that version's changelog section as notes.
About Codex shows the version.
`Codex` > `Check for Updates…` uses the signed update feed when the bundle has one and organization policy permits updates.

## Live QA build

QA uses `~/Applications/Codex GPUI QA.app` with bundle id `dev.codexgpui.qa`.
It rebuilds merged `origin/main`; it does not hot-reload a running Rust process.
Build a staged QA bundle from Linux with `tools/bundle.sh --qa`.
That command does not install or open it.

Run this once on the Mac from a clean checkout of the latest merged main:

```sh
python3 tools/qa-sync.py install
```

The owned launchd job `dev.codexgpui.qa-sync` checks main every 30 seconds.
Its script, source cache, build logs, artifact records, and agent plist stay in `~/Library/Application Support/codex-gpui-qa/updater/`.
Temporary builds and isolated startup probes stay under `/tmp`.
The updater fetches real Git objects, rejects dirty or unlanded source, and checks main again before activation.
It coalesces newer revisions during a build and retains a pending candidate when QA cannot close safely.
It verifies the signed bundle, compiled source revision, artifact digest, executable path, and running PID.
It keeps the previous bundle at `updater/last-working.app`.
It rebuilds missing temporary candidates and removes completed or superseded temporary artifacts.
It requests a cooperative close before removing a superseded idle probe.
It keeps the pending ownership record when a staged process cannot close safely.
It verifies an unfinished activation or restores the backup after the new process exits.

QA data stays in `~/Library/Application Support/codex-gpui-qa/profile/`.
Its UI state and host settings live under `profile/home/Library/Application Support/codex-gpui/`.
Its app-server uses `profile/codex` as `CODEX_HOME` and `profile/workspace` as the default project.
The app-server receives a clean environment and uses file-only credential storage.
QA starts signed out; it never copies normal credentials, settings, history, or workspace data.
It can read the existing ChatGPT app-server executable without changing that app.
The normal Codex GPUI profile and launch behavior stay unchanged.

Programmatic startup uses `open -g -j` and keeps QA hidden.
Open QA from its installed bundle or Dock icon when you want its window.
If the installed revision already matches main but its process has stopped, the updater starts the installed bundle again.
After QA gains focus, starts a turn, changes model settings, or starts sign-in, close QA when ready to update.
The updater keeps its candidate pending until that close.
It also refuses to close during a draft, attachment work, an approval, a running turn, or unverified readiness.
Native foreground activity also prevents closing QA, even when its last heartbeat or window reports inactive.
An observed application activation keeps the existing manual-close protection.
A shared profile lock prevents a reopened QA process from racing with bundle replacement.
It does not promise to preserve drafts, open views, navigation history, or running sessions.
It never force-quits an app.
If a new process cannot prove readiness, the updater retains the backup rather than forcing that process to stop.

Read the current source, artifact, and runtime identities with:

```sh
python3 "$HOME/Library/Application Support/codex-gpui-qa/updater/qa-sync.py" status
```

Run the updater regression checks with `python3 -B tools/test_qa_sync.py`.

Stop only the owned timer with `launchctl bootout "gui/$(id -u)/dev.codexgpui.qa-sync"`.
That command does not close QA or any normal app.

## Architecture

| Crate | Role |
|---|---|
| `bridge` | Client for `codex app-server`: newline-delimited JSON-RPC over stdio, hand-written protocol types, replay of recorded sessions, and the ACP backend. |
| `ui` | The window shell (titlebar, navigation rail, sidebar, main panel), the page modules in `screens/`, the composer, the thread view, the diff view and the icons. |
| `shapes` | Box decorations that land on Chromium's pixels: rounded and squircle corners, borders, `box-shadow`, backdrop blur, SVG icon paths. It links Skia. |
| `bridge` | JSON-RPC client for `codex app-server` over stdio or durable cloud WebSocket, protocol types, recorded sessions, and the ACP backend. |
| `ui` | The window shell (titlebar, navigation rail, sidebar, main panel), the page modules in `screens/`, the composer, the thread view, the diff view and the icons. |
| `shapes` | Box decorations that land on Chromium's pixels: rounded and squircle corners, borders, `box-shadow`, backdrop blur, SVG icon paths and circles. It links Skia. |
| `textsys` | A GPUI text system that puts glyphs on the same pixels as Chromium on macOS. It also holds the vendored `gpui` and `gpui_apple`. |
| `theme` | Color tokens, metrics and the bundled fonts, extracted from the original renderer CSS. |
| `parity` | The pixel-parity harness: it renders each screen headless and compares it with the reference screenshot. |
| `app` | The `codex-gpui` binary: the root view, the menu bar and the app-server session. |

App routing lives in `crates/app/src/routes/`.
Home, Thread, Settings, Automations, Mobile, and Archive each own their action registration and page activation there.
`routes/mod.rs` keeps the route enum, family registration, navigation history, and shared shell updates.
`app::Route` and its saved values stay unchanged.
Shared RPC handlers and state remain on `Root` in `lib.rs`.
`Root::first_app_connected` lets app-list refresh callers pass the previous list and the new list.
It refreshes default and science ambient suggestions only when a known list gains its first enabled, accessible app.
The host requests run off the UI thread; a failed request does not stop the other domain.
The plugins slice owns the caller; this hook does not change the plugins page.
Run `cargo test -p app --test first_app_connected -- --test-threads=1` on macOS for the native hook checks.

Opening an unloaded chat reads its history and resumes the thread before showing it.
The composer restores that thread's returned model, reasoning effort, and permissions instead of using startup defaults.
An unset effort stays unset in requests; the picker shows Medium.
The composer labels modified workspace policies as Custom.
A titled thread shows its header identity, also in a single pane.
An untitled thread shows it only when a right pane is open; the toolbar actions stay available.

In a loaded chat with an open content tab, `Cmd+Shift+F` switches between full and split view.
`Cmd+Shift+B` changes full view to split view, hides the tabs, or restores the retained tabs beside Chat.
Hiding tabs keeps their selected content; opening the same file, terminal, or Changes tab shows it again.
Full view focuses the content pane and does not send typed text to the hidden composer.
Find in full view searches the visible Changes panel, not the hidden chat.
These commands do not create a browser tab when no content tab is open.
Run `cargo test -p app --test workspace_layout -- --test-threads=1` for the native interaction checks.

`LocalHost` serves `native-desktop-apps` and `native-desktop-app-by-bundle-id` through the desktop fetch boundary.
On macOS, the adapter reads Spotlight metadata, applies the original app exclusions, and supports usage or display-name ordering.
It returns each app's path, bundle ID, and display name without launching the app.
Linux returns an empty native app list, as the original host does.

On macOS and Linux, `workspace-directory-entries` lists workspace children with relative paths and directories first.
`includeHidden` and `directoriesOnly` filter the reply.
Directory requests reject lexical and symlink paths outside the workspace.
Enumeration uses the validated directory handle, so replacing a symlink cannot redirect a listing.

`add-context-file` sends a file reference to the host's bound view.
The composer shows a file card without changing the draft text or selection.
Repeated context files stay attached once; replacing the host disconnects its previous context channel.
Submission lists these paths in the original `Files mentioned by the user` section, separate from image uploads.
Editing a queued message restores its file cards and request text.
Failed thread submissions restore the request and file cards.

`open-in-targets` returns the original editor and terminal catalog, installed targets, and the preferred target.
It reads `open-in-target-preferences` from the same shared host settings store as the UI.
Saving one project's preference preserves entries for other projects.
It reads `custom_file_handlers` from enabled raw app-server configuration layers because the typed configuration omits this desktop setting.
Set `deferEnrichment` without a path to read the catalog without application detection.
Path queries include native file handlers and their icons; discovery does not launch an application.
Alacritty is a Linux fallback for the `terminal` target, not a separate target ID in this original build.
Cloned `host_settings::Settings` handles share updates; release `read_all()` guards before writing.

`open-in` launches a selected target and returns its success state.
Local paths must resolve inside the project, including symlink targets.
Existing relative paths take precedence over Git diff prefixes.
Editor locations and terminal commands follow the original platform rules.
Custom handlers receive a path argument, a JSON argument, or JSON on standard input.
The native UI reports failed launches and keeps the current diff.
Command text sent to an interactive terminal rejects control characters.
ACP connections use built-in targets without requesting Codex-only configuration.

On macOS, `textsys::show_definition(window, selection, baseline)` opens AppKit's native Look Up popover.
The baseline uses window-local logical pixels from the top left.
The adapter applies window magnification and handles both view orientations.
Call it on the main thread with the unchanged selection.
Headless windows return their native-handle error; the adapter has no URL or clipboard fallback.

On macOS, `TextInputConfiguration::text_substitutions` enables native text substitutions for an editable field.
The composer enables this option only for non-secret text.
Edit > Substitutions controls smart quotes, smart dashes, and text replacement for the window.
`NSSpellChecker` supplies the edits; the adapter preserves UTF-16 ranges and the caret.
Each check requests up to 512 UTF-16 units of context on either side of the accepted edit.
`cargo run -p app --example native_substitutions` checks the input path in a hidden native window.
The check uses process-local replacement options without changing shared OS settings or activating the app.

On macOS, `SystemNotification::reply_placeholder` adds a text-input notification action.
Register `App::on_system_notification_reply` before posting; it receives the notification identifier and reply text on the main thread.
Without a registered handler, the notification omits the reply field.
The local `vendor/gpui_macos` patch tests native request and category fields without posting a notification.

With `test-support`, `HeadlessAppContext::did_prompt_for_paths` reports a pending path dialog.
Use `simulate_path_prompt_response` to answer it with selected paths or `None` for cancellation.
These hooks use `TestPlatform` without opening an OS dialog; normal app behavior does not change.

`gpui` and `gpui_apple` come from Zed at a fixed revision and are vendored in `crates/textsys`.
The workspace `[patch]` replaces the upstream crates with these copies.
Stock GPUI cannot match Chromium's pixels. The local copies include these changes:

- Blink snapping: layout in Blink `LayoutUnit`s (1/64 device px), device-pixel rounding half up, and text at fractional x positions.
- Skia blends: glyph and sprite blending as Skia's 8-bit raster does it, and `SkPaint::setAlphaf` alpha rounded to a byte.
- Layer compositing: opacity layers and composited layers (`will-change`, scrollers) that blend as Chromium's `saveLayerAlpha` does.

RGB-to-HSL conversion keeps saturated sRGB colors at saturation 1.0.
This keeps their edges on Skia's legacy raster path instead of the float path.
Comment cards clip their contents to the rounded card shape while their shadows remain outside the clip.
Clipped deferred draws retain their ancestor's raster recording origin without creating a new opacity layer.
Comment attachments use Skia for the rounded border and fill, including the hover state.

`Window::layout_origin_residual_y` returns the fractional vertical origin removed by layout rounding.
Use it when placing multiline field glyphs to retain Blink's baseline positions.

`Window::set_ui_scale` scales layout, painting, pointer input, and IME geometry without resizing the native window.
Text layouts use each window's effective device scale and discard stale cached metrics when that scale changes.

Inline-code backgrounds use both snapped label edges, so fractional text widths do not expand their right edge.
Their height and position follow the active code font's metrics.
The thread's top fade preserves content that scrolls below it.

Monochrome glyphs and SVGs keep their paint alpha.
Element opacity is applied once when the enclosing layer is restored.

Native sprite state retains four nested layer devices within Metal's eight color targets.
Each device restores into its parent before a parent opacity mask is applied.
Deeper alpha is combined in the innermost device.

Character-unit widths use the raw glyph advance at the requested font size.
They exclude tracking; text layout still includes it.
Use `TextSystem::ch_advance` for CSS `ch` columns, not glyph ink bounds.

Canvas callbacks honor their `opacity`, `layer`, and `top_layer` styles.
These styles use the same compositor boundary as other elements.

Linear gradients interpolate premultiplied colors in the selected color space.
Their ordered RGB dither leaves alpha unchanged, so transparent fades do not add opacity noise.
Gradients inside compositor layers apply dithering after blending and retain element opacity and the unsnapped gradient origin.

Home composer permission and model labels paint directly into their parent.
Docked composers retain the composited label path. The animated effort label keeps its filter layer.
An extra layer changes glyph edge bytes even when its opacity is one.

`shapes` links Skia binaries built with Chromium's `SkUserConfig.h` legacy defines.
`.cargo/config.toml` points `SKIA_BINARIES_URL` to these binaries.
The rasterizer reuses identical inverse-blend probe results across icon positions without changing their pixels.
`skia-hairline-sys` links a narrow C scanner wrapper for Skia revision `61e7ca4e99062cdd0ab69445d5963fb3365778f6`.
It records the ordered legacy hairline operations that the sprite renderer replays.
`.cargo/config.toml` points `SKIA_HAIRLINE_BINARIES_URL` and `SKIA_HAIRLINE_PACKAGE_SHA256` to its versioned package.

Raster output buffers stop at the visible viewport.
Compositor tiles keep their original size, so long scrollers preserve edge rasterization.

On macOS, popup backdrops capture the current frame's scene before the popup.
The capture includes text, images, clipping, and opacity.
Nested filters use the parent layer's content, not its already-filtered backdrop.

`shapes::raster_images` decodes to the smallest mip that covers the rounded target before bilinear sampling.
Callers choose direct composition, a saved image group, or a transparent compositor tile to match the original layer.
An image group keeps its save-layer even when it contains one image, so clipped sampling retains the original interpolation.
Transparent image groups retain straight-alpha pixels for GPUI, so the background and shadow remain visible through rounded corners.
Use `ImageComposition::Composited` with an opaque background for compositor tiles.
Rasterize these images during paint, when the layer context is available.

Wrap a sparse viewport layer's decorated subtree in `ui::recorded_bounds`.
It collects painted bounds, including shadows, without adding an opacity layer.
The Toaster uses this scope to match Chromium's shifted tile grid and local corner geometry.
The thread titlebar reports its icon and divider bounds so the tile grid follows painted content, including untitled states.

Wrap a tightly bounded compositor layer in `ui::composited_bounds`.
It keeps the exact shadow bounds and uses Chromium's dense tile sizes.
The tooltip uses this scope around its own `top_layer`.
`focus::shadow_ring` paints outer spread shadows. `focus::ring` retains the outline path.
Use `focus::inset_outline` for CSS outlines with negative offsets.
Pass the CSS offset directly; `px(-2.)` keeps a 2px outline inside the control.
The helper keeps a bounded local recording while keyboard focus is visible.

For a composited layer recorded in window coordinates, add `.window_space_recording()` to its container.
Keep `.cc_tiles()` when the original layer rectangle sets the tile size.
Paths retain their window coordinates; the tile grid remains relative to the layer.
Nested composited layers keep their own recording space. Saved layers inherit it; sparse recordings retain their local origin.

## Parity method

The references come from the original renderer itself.
`tools/route1` serves the extracted renderer with a stub Electron main process.
`chrome-headless-shell` on the Mac renders it at 2x, with the Mac's fonts.
The captures are in `refs/<screen>/<theme>@<W>x<H>.png`, with a `spec.json` that holds the DOM boxes, styles and text boxes.
There are 36 references: 9 screens, dark and light, 1280x832 and 1440x900.

The harness renders the same states with GPUI on the Mac and compares exact RGBA values:

```sh
tools/mac.sh 'cargo run -p parity -- [--screen home] [--gate]'
```

- All pixels outside the text boxes must be equal.
- Inside the text boxes (inflated 1 device px), at most 0.5% of the pixels can be different.
- `--gate` exits with code 1 if one case fails.
- The report is in `target/parity/` (`report.html`, `report.json`, and the ref, port and diff PNGs).

`tools/parity/publish.sh <out_dir>` uploads a report to the public R2 evidence bucket.
It prints a Markdown table for a PR body.

## Regenerate inputs

| Input | Command | Notes |
|---|---|---|
| References | `node tools/route1/capture-all.mjs [screen...]` | Start the renderer, the tunnels and the server first, as `tools/route1/README.md` tells. |
| Fixtures | `node tools/fixtures/record.mjs [step...]` | Records real app-server sessions on the Mac; see `fixtures/README.md`. |
| Theme tokens | `uv run --with playwright python tools/extract/theme.py` | Writes `crates/theme/src/tokens.rs`. |
| Fonts | `uv run --with fonttools --with brotli python tools/extract/fonts.py` | Converts the bundled woff2 files to TTF. |
| Icon SVGs | `python3 tools/extract/icons.py` | Writes `assets/icons/<name>.svg` from the references. |
| Icon images | `uv run -p /usr/bin/python3 --with numpy --with pillow tools/icons/build.py` | Fits the per-pixel icon images in `assets/icons/px/` and `tools/icons/check.json`. |
| Skia binaries | `tools/shapes/skia.sh` | Builds Skia with Chromium's defines and uploads it for `SKIA_BINARIES_URL`. |
| Hairline scanner package | `tools/shapes/build-skia-hairline.py` | Builds `skia-hairline-sys` against the installed Skia archive twice and writes the versioned package and receipt. |

The labs `tools/shapes/run.sh`, `tools/shapes/blur.sh` and `tools/text/measure.sh` compare shapes, blur and text with Chromium case by case.

## Live proof

```sh
tools/mac.sh 'cargo run -p app --example live_turn'
```

The example runs the real root view, headless, on a real `codex app-server`.
With `CODEX_GPUI_ACP` set, it runs on that ACP agent instead and also checks the ACP toast.
It types a prompt into the Home composer with key events and pushes Enter.
It waits for `turn/completed` and checks that the thread view shows `pong`.
It prints the transcript and saves `live-1-home.png`, `live-2-streaming.png` and `live-3-done.png`.
`$LIVE_ROOT` (default `/tmp/cgx-mac`) holds the project, the Codex home and the screenshots.

## Frame benchmark

```sh
tools/mac.sh 'cargo run -p app --release --example bench'
```

The example runs the real root view, headless at 1280x832 @2x, on a replayed app-server with 60 Recents and a 40-turn chat.
It sends key, mouse and scroll-wheel events and prints, for each scenario, the p50, p95 and max milliseconds from an input to the end of its frame.
The animation scenarios (sidebar and changes-panel toggles in a chat) also count every frame of the animation, at simulated 120 Hz ticks.
The `fps @120Hz` column gives the frames per second that a 120 Hz display shows when each frame takes its CPU time.
The bench also prints the time from launch to the first ready frame.
The 120 Hz frame budget is 8.3 ms.
With `BENCH_SHOT=<dir>`, it saves every 20th frame, so that you can compare the pixels of two builds.

## Current parity

Each cell shows the different pixels outside the text boxes, then the different-pixel ratio inside the text boxes.
A case passes at `0 / ≤0.500%`.
Now 36 of 36 cases pass.

| Screen | Theme | 1280x832 | 1440x900 |
|---|---|---|---|
| home | dark | 0 / 0.377% | 0 / 0.377% |
| home | light | 0 / 0.098% | 0 / 0.098% |
| sidebar | dark | 0 / 0.320% | 0 / 0.316% |
| sidebar | light | 0 / 0.153% | 0 / 0.152% |
| thread | dark | 0 / 0.443% | 0 / 0.438% |
| thread | light | 0 / 0.444% | 0 / 0.439% |
| composer | dark | 0 / 0.332% | 0 / 0.332% |
| composer | light | 0 / 0.250% | 0 / 0.250% |
| diff | dark | 0 / 0.360% | 0 / 0.467% |
| diff | light | 0 / 0.281% | 0 / 0.388% |
| settings | dark | 0 / 0.088% | 0 / 0.080% |
| settings | light | 0 / 0.117% | 0 / 0.107% |
| automations | dark | 0 / 0.279% | 0 / 0.279% |
| automations | light | 0 / 0.148% | 0 / 0.148% |
| terminal | dark | 0 / 0.179% | 0 / 0.261% |
| terminal | light | 0 / 0.171% | 0 / 0.249% |
| mobile | dark | 0 / 0.468% | 0 / 0.468% |
| mobile | light | 0 / 0.467% | 0 / 0.467% |

## Permission-menu interaction replay

`refs/interactions/f2b.json` records 16 steps from the original app through route1 on Linux.
The Mac replay checks hover focus, Home/End, typeahead with a one-second reset, Tab containment, and keyboard opening.
It compares focus, highlight pixels, and outgoing requests.
The existing port menu has different dimensions, so the replay maps hover targets to the same item and heading.
Submenu arrows and model-trigger recording remain outside this replay.

```sh
tools/mac.sh -l f2b 'cargo test -p app --test f2b -- --test-threads=1'
```

## Command-menu actions

The command menu uses the existing app actions for new chats, archive, Find, navigation, six recent-chat slots, sidebar visibility, panels, and scheduled tasks.
Find keeps focus in its search field after the menu closes.
Panel commands keep the selected chat and the existing tabs.
The terminal command opens or hides the interactive bottom panel without restarting its shell.
Unsupported destinations remain known gaps; this change does not implement logout, git operations, cloud operations, or the project picker.
Recorded command rows, ranking, and keyboard-editing checks remain unchanged.

A hidden native menu item dispatches the shared `NewStandaloneChat` action and keeps its keyboard shortcut.
Its first message creates a separate projectless workspace.
File > New Temporary Chat uses the existing ephemeral mode and does not add a Recents entry.
The temporary action is unavailable for ACP backends.
Settings hides Toggle Sidebar and disables its action.
Leaving Settings restores that item without changing the sidebar.
Keyboard shortcut edits preserve the active window's menu visibility.
The native menu lives in `crates/app/src/native_menus.rs`.
Edit supports plain-text Paste and Match Style and selection-only Delete, including undo.
View contains Find; Help opens the Codex changelog, troubleshooting guide, and service status page.
View > Toggle Pinned Summary opens and closes the current thread summary.
Home, sign-in, and pending thread loads keep this command disabled.
The `native_menus` and `s013` tests check the edit actions and Find navigation on macOS.
View > Toggle navigation panel opens or closes the active Changes tree or workspace file tree.
The workspace tree reads real directories and files, keeps its selection, and filters through the app-server file search.
File navigation does not modify workspace files.
The File viewer uses the Changes panel's main-surface layer for its divider and rounded window corners.
Command+Shift+E (`toggleFileTreePanel`) dispatches `native_menus::ToggleNavigationPanel`, the same action as the menu item.
About shows this build's version in a separate window; Escape closes it.
The Window menu has Bring All to Front, and Control+Command+F toggles the active window's fullscreen state.
The fullscreen replay checks command dispatch and window state without bringing a native window to the front.
The spelling menu replaces the selected misspelling and sends Learn Spelling to the native service.
Headless tests check these actions without relying on the system dictionary's suggestions.
The Speech submenu uses the native speech synthesizer; its service test sets the volume to zero.
Images in the composer and tool output support Copy Image and Save Image As.
Images with a local source also support Reveal in Finder.
Saving writes the original bytes and replaces the destination only after the write succeeds.
The headless Save-dialog hook allows tests to select a destination without opening a native window.
On macOS, Control+Shift+4 attaches a selected screen region to the primary window's current chat.
Control+Option+Shift+4 opens a new chat before selection.
Starting a new chat clears the previous Home draft, including its attachments.
Both shortcuts use the native selection tool and leave the screenshot unsent.
Enlarged image previews retain their edge colors instead of sampling adjacent texture data.
Escape cancels selection without an error; changing the chat or account discards an unfinished capture.
The shortcuts use `codex-command-keybindings` overrides and never change Screen Recording permissions.
Run `cargo test -p app --lib screenshot:: -- --test-threads=1` for isolated capture, attachment, and macOS image-edge checks.

```sh
tools/mac.sh -l cgx-command-dispatch 'cargo test -p app --test command_dispatch --test s025 --test s025_rank --test s026 -- --test-threads=1'
```
`⌥⌘P` toggles the open chat's pin, `⌥⌘R` renames it and `⌘F` finds in it, all through the keymap. `⌥⌘L` copies the open chat's `codex://threads/{id}` link.
`Next tab`/`Previous tab` and `⌥⌘W` (close other tabs) apply when focus is inside a side tab. `⌘W` closes the open side tab first; a `closeWindow`-only key closes the window.
`⌘↩` on Home sends the message in the background: the chat starts and shows in Recents, Home stays open and the draft clears. The `⌘/` dialog lists it on Home only.
The `⌘/` dialog searches command IDs, titles and descriptions with the same matching rules as the Settings page.
`⇧⌘C` and `⌥⇧⌘C` copy the open chat's newest attached worktree root (else its working directory); a failed attachment refresh copies nothing. `⇧⌘F` and `⇧⌘B` follow the editable keymap.
The Keyboard shortcuts page search controls use their recorded SVG variants; the Settings sidebar keeps its own keyboard glyph.

## Native links

The app handles `codex://` and `codex-dev://` links from launch arguments and native URL events.
Thread links keep `hostId` and `threadAccess=shared` on the selected connection.
A private link to the same thread clears shared access.
New-chat links accept `prompt`, `projectId`, `path`, and `originUrl`.
Project selection uses a saved project first, then an existing folder, then a saved repository with a matching origin.
The prompt fills and focuses the composer without sending it.

`codex://settings/connections/<tab>` opens the Computer, Devices, or SSH tab.
An SSH `add` link accepts `name` or `alias`, `projectPath`, and `enabled=true`.
The app checks a remote project folder through the connected host before selecting it.
`codex://automations/<id>` uses cloud storage unless `automationSource=local` selects local storage.
Add `edit=true` to open the selected task's editor.
Local and cloud tasks can have the same ID without sharing their stored data.
The Scheduled button and new tasks use local storage.
`codex://connector/oauth_callback` returns to the window that owns the pending OAuth state.
Excluded product links do not open a native page.

Run `cargo test -p app --lib deep_link` and `cargo test -p app --test deep_link --test thread_hosts --test oauth --test automation_share`.
