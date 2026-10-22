# codex-gpui

codex-gpui is an open-source port of the Codex desktop app to Rust and [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
The original is the Codex section of the macOS ChatGPT app.
The goal is pixel parity with the original: the same layout, text, colors and icons, device pixel for device pixel.
The app drives the real `codex app-server` over stdio, the same JSON-RPC transport that the original uses.

## Requirements

- macOS on Apple silicon (arm64).
- Rust 1.98.1. `rust-toolchain.toml` pins it, so `rustup` installs it on the first build.
- A `codex` binary. The app uses the first one that it finds:
  1. `$CODEX_BIN`.
  2. `/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex`, the binary that the ChatGPT app bundles.
  3. `codex` on `PATH`.

The workspace also builds on Linux, with GPUI's no-op text system, for CI (fmt, clippy, tests).

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

## Background terminal output

Background command output displays ANSI colors and supports text selection.
Copy preserves the selected text without ANSI control bytes.
Like the original app, the view keeps 1,000 scrollback rows plus the visible screen.
The terminal fixture in `fixtures/terminal-depth.json` contains recorded `thread/shellCommand` output.

Run `cargo test -p ui --lib terminal` for parser and command-label checks.
Run `cargo test -p app --test terminal_states background_ansi_output_supports_selection_and_copy` on macOS for the native selection check.

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

Run `cargo test -p bridge` for real loopback WebSocket exchanges, disconnect handling, and credential checks.

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

## Build and run

```sh
cargo run -p app --release
```

The app keeps its window state in `~/Library/Application Support/codex-gpui/state.json`.
Host settings use the local `settings.write(key, value)` service and persist beside that file in `host-settings.json`.
Theme and window-close preferences remain in `state.json`.
The S034 replay checks the menu-bar switch, its saved values, and Back navigation against `refs/interactions/s034.json`.

Settings > Git stores branch, merge, push, draft, diff, and pull request instruction preferences in `host-settings.json`.
Review delivery uses the existing app-server config store.
Branch prefix saves on blur or Enter; multiline instructions save after three idle seconds or when the page closes.
The focused `cargo test -p app --test git_settings -- --test-threads=1` native replay verifies saved values after reopening Git settings.
This page does not implement Git operations or active pull request watchers, and it does not complete other settings pages.

Changes > Load full files reads Git objects and working files when their rows enter the view.
Expand a context separator to show 20 more lines in either direction or the complete gap.
The view checks full content against the displayed patch and preserves the patch when a read fails.
Partial updates without a previous object ID keep their recorded hunks instead of guessing a previous revision.
Line wrapping, word differences, and full-file loading use the existing host settings store.
Run `cargo test -p app --test diff_file_reads` for file boundaries and `cargo test -p app --test diff_context` on macOS for expansion and retry.
Image and PDF previews read bytes from each selected Git version or working file.
PDF previews use macOS rendering and keep separate page controls for each side.
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

`ui::settings_kit` provides a multi-select control and an external-link row for Settings pages.
The multi-select emits `SelectionChanged`; the caller saves the value and can restore it with `set_checked` if saving fails.
`ui::confirmation::Confirmation` emits one confirm or cancel result and returns focus to the previous control.
Use `Composer::settings_field` for multiline text and `Toast` for existing information, error, and success messages.
Run `cargo test -p ui --test settings_kit` for the keyboard and locked-choice check.

`app::diff_content::read_full_file` loads typed before/after data through the existing Git object host.
It preserves line endings and the host's working-file fallback policy, with a 5 MiB object limit.
Missing or oversized objects return `FullError::Unavailable`; invalid disk-fallback paths or reply structures return `FullError::Failed`.
Run `cargo test -p app --test diff_content` for real Git object and repository-boundary checks.

Scheduled > New task accepts instructions, a daily or weekly repeat, and a local time in `HH:MM` format.
Create saves the definition through `create-automation` to `CODEX_HOME/automations/<id>/automation.toml`.
The app reloads the task list after a successful write and retains the draft if the write fails.
The existing host schedules these definitions; this app does not start another scheduler.
The row menu's `Run now` sends `automation-run-now`; the host starts one `codex app-server` thread and turn per task folder.
It runs tasks with project folders or a project target (the project's first root from `.codex-global-state.json`), in a new Git worktree for the original default environment or in the folder for the local one.
For a project target, the host saves the new thread's local project assignment before it starts the turn.
An assignment write failure stops the run; legacy folder tasks do not change project assignments.
Projectless tasks use an owned folder under `~/Documents/Codex/<local-date>/<prompt-name>/`, with separate `work` and `outputs` folders.
The `projectlessWorkspaceRoot` host setting overrides the root.
The host saves projectless thread IDs, workspace-root hints, and output paths in `.codex-global-state.json`.
Heartbeat tasks resume their existing thread with its selected model and permissions.
The app opens that thread after a successful start, unless you navigate away while the run starts.
Run `cargo test -p app --test task_create -- --test-threads=1` on macOS for the native creation and reload proof.
Run `cargo test -p app --test automation_run_now` for the `Run now` requests over a stdio app-server stub.

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

## Release bundle

```sh
tools/bundle.sh
```

The script builds the release binary on the Mac build host through `tools/mac.sh`.
It assembles `Codex GPUI.app` (bundle id `dev.codexgpui.app`, version from `crates/app/Cargo.toml`, macOS 13.0 or later).
It signs the bundle ad hoc, checks it with `plutil -lint` and `codesign --verify --strict`, and zips it with `ditto`.
The zip goes to `/tmp/cgx/release/codex-gpui-<version>-macos-arm64.zip`.
The script does not open the app.

The app icon is the original icon from `ChatGPT.app/Contents/Resources/Assets.car`.
To make `assets/app/AppIcon.icns` again, run this on the Mac:

```sh
swiftc -O assets/app/icon.swift -o /tmp/icon
/tmp/icon Assets.car /tmp/AppIcon.iconset
iconutil -c icns /tmp/AppIcon.iconset -o assets/app/AppIcon.icns
```

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
| `bridge` | JSON-RPC client for `codex app-server` over stdio or durable cloud WebSocket, protocol types, recorded sessions, and the ACP backend. |
| `ui` | The window shell (titlebar, navigation rail, sidebar, main panel), the page modules in `screens/`, the composer, the thread view, the diff view and the icons. |
| `shapes` | Box decorations that land on Chromium's pixels: rounded and squircle corners, borders, `box-shadow`, backdrop blur, SVG icon paths. It links Skia. |
| `textsys` | A GPUI text system that puts glyphs on the same pixels as Chromium on macOS. It also holds the vendored `gpui` and `gpui_apple`. |
| `theme` | Color tokens, metrics and the bundled fonts, extracted from the original renderer CSS. |
| `parity` | The pixel-parity harness: it renders each screen headless and compares it with the reference screenshot. |
| `app` | The `codex-gpui` binary: the root view, the menu bar and the app-server session. |

App routing lives in `crates/app/src/routes/`.
Home, Thread, Settings, Automations, Mobile, and Archive each own their action registration and page activation there.
`routes/mod.rs` keeps the route enum, family registration, navigation history, and shared shell updates.
`app::Route` and its saved values stay unchanged.
Shared RPC handlers and state remain on `Root` in `lib.rs`.

Opening an unloaded chat reads its history and resumes the thread before showing it.
The composer restores that thread's returned model, reasoning effort, and permissions instead of using startup defaults.
An unset effort stays unset in requests; the picker shows Medium.
The composer labels modified workspace policies as Custom.
The default pinned-summary layout hides the thread identity in a single pane.
Opening a right pane restores the header identity; the toolbar actions stay available.

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
The baseline uses window-local logical pixels from the top left; the adapter handles both view orientations.
Call it on the main thread with the unchanged selection.
Headless windows return their native-handle error; the adapter has no URL or clipboard fallback.

On macOS, `SystemNotification::reply_placeholder` adds a text-input notification action.
Register `App::on_system_notification_reply` before posting; it receives the notification identifier and reply text on the main thread.
Without a registered handler, the notification omits the reply field.
The local `vendor/gpui_macos` patch tests native request and category fields without posting a notification.

With `test-support`, `HeadlessAppContext::did_prompt_for_paths` reports a pending path dialog.
Use `simulate_path_prompt_response` to answer it with selected paths or `None` for cancellation.
These hooks use `TestPlatform` without opening an OS dialog; normal app behavior does not change.

`gpui` and `gpui_apple` come from Zed at a fixed revision and are vendored in `crates/textsys`.
The workspace `[patch]` replaces the upstream crates with these copies.
Stock GPUI cannot match Chromium's pixels, so the copies change three things:

- Blink snapping: layout in Blink `LayoutUnit`s (1/64 device px), device-pixel rounding half up, and text at fractional x positions.
- Skia blends: glyph and sprite blending as Skia's 8-bit raster does it, and `SkPaint::setAlphaf` alpha rounded to a byte.
- Layer compositing: opacity layers and composited layers (`will-change`, scrollers) that blend as Chromium's `saveLayerAlpha` does.

RGB-to-HSL conversion keeps saturated sRGB colors at saturation 1.0.
This keeps their edges on Skia's legacy raster path instead of the float path.
Comment cards clip their contents to the rounded card shape while their shadows remain outside the clip.

`Window::layout_origin_residual_y` returns the fractional vertical origin removed by layout rounding.
Use it when placing multiline field glyphs to retain Blink's baseline positions.

Inline-code backgrounds use both snapped label edges, so fractional text widths do not expand their right edge.

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

Home composer permission and model labels paint directly into their parent.
Docked composers retain the composited label path. The animated effort label keeps its filter layer.
An extra layer changes glyph edge bytes even when its opacity is one.

`shapes` links Skia binaries built with Chromium's `SkUserConfig.h` legacy defines.
`.cargo/config.toml` points `SKIA_BINARIES_URL` to these binaries.

Raster output buffers stop at the visible viewport.
Compositor tiles keep their original size, so long scrollers preserve edge rasterization.

On macOS, popup backdrops capture the current frame's scene before the popup.
The capture includes text, images, clipping, and opacity.
Nested filters use the parent layer's content, not its already-filtered backdrop.

`shapes::raster_images` decodes to the smallest mip that covers the rounded target before bilinear sampling.
Callers choose direct composition, a saved image group, or a transparent compositor tile to match the original layer.
Use `ImageComposition::Composited` with an opaque background for compositor tiles.
Rasterize these images during paint, when the layer context is available.

Wrap a sparse viewport layer's decorated subtree in `ui::recorded_bounds`.
It collects painted bounds, including shadows, without adding an opacity layer.
The Toaster uses this scope to match Chromium's shifted tile grid and local corner geometry.

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
The terminal command opens an existing background command, not an interactive shell.
Unsupported destinations remain known gaps; this change does not implement logout, git operations, cloud operations, or the project picker.
Recorded command rows, ranking, and keyboard-editing checks remain unchanged.

The native menu lives in `crates/app/src/native_menus.rs`.
Edit supports plain-text Paste and Match Style and selection-only Delete, including undo.
View contains Find; Help opens the Codex changelog, troubleshooting guide, and service status page.
View > Toggle Pinned Summary opens and closes the current thread summary.
Home, sign-in, and pending thread loads keep this command disabled.
The `native_menus` and `s013` tests check the edit actions and Find navigation on macOS.
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

```sh
tools/mac.sh -l cgx-command-dispatch 'cargo test -p app --test command_dispatch --test s025 --test s025_rank --test s026 -- --test-threads=1'
```
