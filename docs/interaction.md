# Menus and native links

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

