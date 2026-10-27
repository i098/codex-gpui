# Native windows

The reference is Codex `26.1007.30912`, build `20156`, production flavor.
The host source is `main-CqEoQDOU.js`.
The renderer source is `app-initial-6350d04672d0.js`.
All sizes below use logical pixels.

## Original window census

| Kind or route | Entry | Size and placement | Lifetime |
|---|---|---|---|
| Primary | Launch, dock, File New Window, `open-in-new-window` | Default 1280×820; minimum 480×600; restore `electron-main-window-bounds` | Independent windows; keep the final Mac window hidden on close; dock restores it |
| Page primary | Host `createPrimaryWindow`, `/space/:id?window=page` | At most 800×720, the owner size, and 80% of the display work area; minimum 480×600 | Independent page; do not replace stored main bounds |
| Detached browser | Detached host `open()` / `createService()`; `detached-window.html`; content kind `browser` | At most 960×720, the owner size, and 80% of its display work area; minimum 400×400 | Transfer browser ownership; destruction releases its session; the owner can outlive it |
| Secondary | Host `createWindow`; debug uses 920×840 | Factory default 1280×820; minimum 400×400; standard titlebar | Close destroys the window; the production build disables the debug entry |
| HUD | Host `createWindow` appearance `hud` | Factory default 1280×820; minimum 400×400; floating; traffic lights at 10,10 on Mac | No minimize, maximize, or fullscreen |
| Hotkey home | `toggleHotkey()` / `showHome()`; `/hotkey-window` | 470×290; minimum 320×140; centered horizontally; bottom aligned with the thread window | Prewarm hidden; toggle reuses it; blur hides; store dragged composer placement |
| Hotkey thread | `showThread()`; hotkey routes for new, local, or remote threads | Initial 470×640; minimum 400×400; 52 below the work-area top; clamp to its display | Prewarm hidden; retain size; switch between home and thread; blur hides |
| Spotlight | `toggle()` from its configured global shortcut; `/spotlight` | 760×640; transparent, fixed, above normal windows, on all spaces | Prewarm hidden; blur and close hide; dispose destroys |
| Avatar overlay | `open()` / `toggleFromHotkey()` / `ensureWindow()`; `/avatar-overlay` | Initial 356×320; fixed, transparent, above normal windows | Non-activating; initial pointer pass-through; controller owns layout and visibility |
| Screen overlay | Host overlay appearance | Display-sized; transparent, fixed, above normal windows, on all spaces | Non-activating; controller owns visibility and pointer interaction |
| Global dictation | `showAndStart()` / `confirmStart()`; `/global-dictation` | 720×180; centered on the cursor display; 16 above its work-area bottom | Prewarm hidden; show without activation; keep pointer pass-through until recovery interaction |
| Browser comment popup | Validated `window.open` frame name from a primary browser view | 294×208; fixed, transparent; attached to its owner | Start hidden; cannot outlive its owner |
| `/diff` | Renderer route with `unifiedDiff`, `conversationId`, optional `cwd` state | No separate active host sizing call was found | `show-diff` opens the inline panel in this host; `update-diff-if-open` is a no-op |
| `/plan-summary` | Renderer route with plan content, conversation, and host state | No separate active host sizing call was found | `show-plan-summary` is a no-op in this host |
| `/extension/panel/new`, `/browser/handoff` | Renderer routes used by the extension integration | Their route does not establish another native appearance | Keep extension pairing and page behavior with the page owner |
| Quick chat chunks | Bundled renderer chunks | No mount path established by the census | Do not invent a native entry |

There is no default shortcut for `openThreadInNewWindow` or `newWindow`.
The hotkey and spotlight controllers register configured shortcuts separately.
The diff and plan route presence does not prove that their old host messages open windows.

## Native contract

`app::windows::open(Spec, cx, builder)` creates a typed GPUI window.
The builder owns its real page state; the manager does not create placeholder pages.
`Kind` selects native chrome, minimum size, initial placement, and retained-window policy.
`set_bounds` returns the frame accepted by the native window, including platform constraints.
`Spec::owner` selects the placement display and records ownership.
Only browser comment popups close with their owner.
Other child windows remain independent.

Use `Presentation::Background` for non-activating display and `Presentation::Hidden` for prewarming.
QA builds never request foreground presentation.
The AppKit adapter supplies work-area bounds, background presentation, hiding, and overlay policies.
Fullscreen moves the history and sidebar controls into the space left by the window controls.
Leaving fullscreen restores their windowed positions.
Unsupported background presentation returns an error instead of claiming success on other platforms.
The page controllers retain responsibility for hotkey registration, browser transfer, overlay input transitions, and permissions.
The window manager does not trigger permissions.

The existing command-menu entry `Open in new window` now opens the selected local thread in another primary window.
Each root retains its own composer and backend connection.
Closing one primary does not quit other windows.
Closing the final Mac primary hides that window and preserves its draft.
If the retained window is fullscreen, the manager waits for fullscreen exit before hiding it.
A dock reopen displays the retained primary before creating another one.
The manager stores primary geometry through the existing host settings store.
Page and auxiliary windows do not overwrite that geometry.
Shared `Bridge` clients serialize initialization and reuse the first successful handshake.
The Quit command checks active local chats and scheduled tasks before it exits.
The native warning offers Quit and Cancel when either check finds work.
Cancel keeps the windows open.
The check starts after the current window action returns so it can read every primary window.
The warning uses a remaining primary window if another window closes during the check.
If the task store cannot be read, the warning includes the error and still offers Quit and Cancel.
`Host::prepare_projectless` and `Host::record_projectless` expose the existing standalone workspace allocation and metadata writes.

## Verification

Run the geometry regression with `cargo test -p app --lib windows::tests`.
Run `cargo test -p bridge concurrent_windows_initialize_the_shared_server_once` for concurrent initialization.
Run `cargo test -p app --test quit_guard` for a Quit action from a primary window with an active scheduled task.
Run `cargo test -p app --test fullscreen` on macOS for fullscreen control placement and history navigation.
Run `native_windows` on macOS with `CODEX_GPUI_QA=1` and an owned recorded `CODEX_BIN` executable.
Pass `--qa-smoke-profile /tmp/cgx-qa-smoke-<owned-name>` to isolate all profile state.
The smoke opens two actual native primary windows, checks independent drafts, closes one, hides the last, and restores it without activation.
The smoke reads AppKit visibility, not screen occlusion, and waits for deferred window actions.
The example installs a panic hook before initialization so failures exit with code 101 without a crash dialog.
It does not require microphone, accessibility, screen recording, account access, or a live turn.
No page painting code changes in this slice.
The separate page owners still owe their original-versus-native appearance checks.

## Popout and Spotlight controller

`app::hotkey::open(owner, target, cx)` opens the local popout or Spotlight without changing the primary window's route or draft.
The controller accepts `/hotkey-window`, `/hotkey-window/new-thread`, `/hotkey-window/thread/:id`, `/hotkey-window/remote/:id`, and `/spotlight`.
The popout uses the existing composer, thread renderer, app-server connection, and window manager.
Its settings menu selects a project, local, worktree or cloud execution, a branch, an environment, and permissions.
Worktree execution uses the existing `create-worktree` and `set-worktree-owner-thread` host calls.
The window expands for a thread and collapses for a new chat.
Open in Main Window transfers navigation to the owning primary window.

On macOS, the controller registers Spotlight `Alt+K` and screenshot selection `Control+Shift+4` and `Control+Alt+Shift+4`.
`replace_global_bindings` applies the saved command map; an empty override disables its shortcut.
`suspend_global_bindings` removes native registrations while the shortcut editor captures input.
No popout shortcut is set by default.
Screenshot selection attaches the captured image without sending the chat.
Shortcut registration failures remain errors.
Other platforms reject native shortcut registration.

Global dictation uses the saved shortcut map and the existing voice settings store.
It requires the account rollout and workspace permission checks before registration.
The controller records through the selected native microphone, retains recoverable audio, and calls the existing transcription request.
It restores the clipboard only if another application has not changed it.
Recovery exposes Copy, the Accessibility settings link, and saved recordings.

Run `cargo test -p app --lib hotkey -- --nocapture --test-threads=1` for the local regression checks.
The checks exercise real temporary worktree creation, environment parsing, route boundaries, and separate primary and popout drafts.
Set `CGX_HOTKEY_EVIDENCE` to save native home, settings and Spotlight frames from the recorded backend.
These checks do not activate a microphone or screen capture.
Strict original-versus-native appearance checks remain separate from these behavior checks.

## Mini

`Show Mini` and `openAvatarOverlay` use one retained `/avatar-overlay` window.
The profile menu and command menu use the same action.
The macOS global shortcut defaults to `Alt+Space` and follows the saved command map.
The menu toggles visibility; the global shortcut opens the compact chat input.
Mini keeps its draft separate from the primary window and uses the existing standalone chat pipeline.
The native pointer monitor leaves transparent areas inactive.
Opening Mini does not request microphone access or start audio capture.

The nine built-in pet sprites come from the pinned renderer.
Unknown pet IDs report an error instead of selecting a different pet.
`notchVoice` and `toggleNotch` stay inactive because the pinned host has no active Notch controller.

Run `native_mini` on macOS with `CODEX_GPUI_QA=1` and an owned recorded `CODEX_BIN`.
Pass an isolated `--qa-smoke-profile`, as for `native_windows`.
Set `CGX_MINI_EVIDENCE` to retain the pet and compact-input frames.
The smoke checks native visibility, background presentation, draft separation, dismissal, and window reuse.
It does not start a model turn or record audio.
Original-versus-native appearance approval still requires a matching original capture.
