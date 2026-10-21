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
This change does not trigger those permissions or implement those separate pages.

The existing command-menu entry `Open in new window` now opens the selected local thread in another primary window.
Each root retains its own composer and backend connection.
Closing one primary does not quit other windows.
Closing the final Mac primary hides that window and preserves its draft.
If the retained window is fullscreen, the manager waits for fullscreen exit before hiding it.
A dock reopen displays the retained primary before creating another one.
The manager stores primary geometry through the existing host settings store.
Page and auxiliary windows do not overwrite that geometry.

## Plan summary

`Route::PlanSummary(Plan)` renders the `/plan-summary` content without the main navigation shell.
Missing conversation, host, or content fields show the original loading card.
The original host ignores `show-plan-summary`, so this change adds no menu entry or automatic window launch.

Thread plan cards use `ThreadItem::Plan` and `item/plan/delta`.
The last work item shows `Writing plan` while its turn runs.
A later work item or a finished turn changes the title to `Plan`.
The completed card opens and closes the shared side panel.
Download writes the unchanged markdown through the native save dialog with the suggested name `PLAN.md`.
The standalone card supports collapse and expansion.

Run `cargo test -p app --test plan_summary -- --test-threads=1`.
These tests check protocol scoping, completion timing, card controls, file bytes, and cancellation.
Synthetic regression data does not prove original pixel parity.
The original capture gate remains required before release.

## Verification

Run the geometry regression with `cargo test -p app --lib windows::tests`.
Run `native_windows` on macOS with `CODEX_GPUI_QA=1` and an owned recorded `CODEX_BIN` executable.
Pass `--qa-smoke-profile /tmp/cgx-qa-smoke-<owned-name>` to isolate all profile state.
The smoke opens two actual native primary windows, checks independent drafts, closes one, hides the last, and restores it without activation.
The smoke reads AppKit visibility, not screen occlusion, and waits for deferred window actions.
The example installs a panic hook before initialization so failures exit with code 101 without a crash dialog.
It does not require microphone, accessibility, screen recording, account access, or a live turn.
No page painting code changes in this slice.
The separate page owners still owe their original-versus-native appearance checks.
