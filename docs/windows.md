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
`Host::prepare_projectless` and `Host::record_projectless` expose the existing standalone workspace allocation and metadata writes.

The geometry check uses the display work area, not the full screen bounds.
Fullscreen does not replace the saved windowed bounds.
Each new process starts on Home; saved geometry does not restore the last route.

## Shell controls

Below 768 logical pixels, the sidebar starts collapsed and opens as a docked panel.
The narrow panel does not change the saved wide-window preference.
When the wide sidebar is hidden, a rail hover opens a preview after 100 milliseconds.
Switching the preview waits 300 milliseconds; leaving it waits 100 milliseconds.
Escape closes the preview before the composer handles its dismiss action.

The resize separators support arrow keys in 10-pixel steps and Home/End for their limits.
They expose their current size and limits to accessibility tools.
The close shortcut closes a side tab only when the pane or its tab strip has focus.
Otherwise, the window close preference controls the confirmation.
Quit warns about running local chats and enabled scheduled tasks.

The titlebar menu supports pointer and keyboard entry.
Content-tab creation remains gated while in-app browser pages are excluded.

## Loading and recovery

The loading view remains until initialization and the startup reads finish.
A disconnected app-server keeps the window open and shows recovery controls.
Try again starts a new connection and reloads the route without clearing the composer draft.
Invalid route and tab content have separate retry controls.
Feedback checks the account and feedback policy before opening its form.
Submission uses `feedback/upload`; a failed upload keeps the entered details.


## Deep links

Launch arguments and macOS URL events use the same validated dispatcher.
Thread and project links prefill the composer; they do not submit a turn.
An unavailable foreign host never falls back to a local thread.
Unknown settings sections open General, as in the original parser.
The normal bundle registers `codex`; the QA bundle uses `codex-gpui-qa`.
Each page family retains its own availability and permission gates.

## Verification

Run the geometry regression with `cargo test -p app --lib windows::tests`.
Run `cargo test -p bridge concurrent_windows_initialize_the_shared_server_once` for concurrent initialization.
Run `native_windows` on macOS with `CODEX_GPUI_QA=1` and an owned recorded `CODEX_BIN` executable.
Create an owned `/tmp/cgx-qa-smoke-<name>` directory, then pass it with `--qa-smoke-profile` to isolate all profile state.
The smoke opens two actual native primary windows, checks independent drafts, closes one, hides the last, and restores it without activation.
The smoke reads AppKit visibility, not screen occlusion, and waits for deferred window actions.
The example installs a panic hook before initialization so failures exit with code 101 without a crash dialog.
It does not require microphone, accessibility, screen recording, account access, or a live turn.
Run `cargo test -p app --test window_shell` for fullscreen, preview, floating-panel, and close-focus behavior.
Run `cargo test -p app --test recovery` for draft preservation across connection recovery.
Original-versus-native pixel checks remain separate from these interaction checks.
