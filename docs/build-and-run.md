# Build and run

Build and run the native Codex Desktop in Rust, codex-gpui, on macOS and Linux.

## Requirements

- macOS on Apple silicon (arm64) and Intel (x86_64); the release zip and .dmg are arm64 only.
- Rust 1.98.1. `rust-toolchain.toml` pins it, so `rustup` installs it on the first build.
- A `codex` binary. The app uses the first one that it finds:
  1. `$CODEX_BIN`.
  2. `/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex`, the binary that the ChatGPT app bundles.
  3. `codex` on `PATH`.

The workspace also builds on Linux, with GPUI's no-op text system, for CI (fmt, clippy, tests).
The self-hosted CI job disables dev/test debug information and incremental compilation.
It uses only its own workspace's `target` directory and removes that directory after success or failure.
CI skips draft pull requests by default. Marking a pull request ready for review starts the normal run; maintainers can also use Actions > ci > Run workflow to test a draft branch. A newer push to a pull request cancels its older run; pushes to `main` always run. A pull request that changes only the listed documentation files (`paths-ignore` in `.github/workflows/ci.yml`) starts no run.


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
Worktree starts show creating and attaching notices, then a thread status after creation.
Setup failures show a separate error notice.
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

File-change activity displays `item/fileChange/outputDelta` while a patch runs and keeps that output after completion.
Run `cargo test -p ui --lib live_patch_output_keeps_item_boundaries` to check streaming and item isolation.

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


