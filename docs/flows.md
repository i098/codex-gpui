# Flow parity: original vs port

This document gives the flow status of the port for each screen of `docs/inventory.md` §14 and §15, plus the app-level shortcuts and menu bar of §12 and §13.
The harness is `crates/app/tests/flows.rs`.
It builds the real `app::Root` on a replayed app-server (`Bridge::replay` on `fixtures/readback.jsonl`, plus `fixtures/sessions/greeting.jsonl` for turns) in a headless window with the `textsys` text system and the Metal headless renderer.
It clicks at the original's element bounds (`refs/<screen>/dark@1280x832.spec.json`), types keys, dispatches the menu bar's actions, and then checks the route, rail selection, sidebar, open thread, composer text and focus, popovers (repainted pixels) and theme.
The fixtures do not record some methods (`thread/archive`, `thread/fork`, `thread/resume`, `thread/revert`, `config/batchWrite`, `config/value/write`) or any approval request. For these, a flow adds crafted lines to the replay script (`exchange`): the replay answers such a request by its method, and the flow checks the state the answer leads to. The headless window also takes live resizes (`HeadlessAppContext::simulate_window_resize`).

Run it on the Mac: `tools/mac.sh -l flows 'cargo test -p app'`.
To run some flows only, add a name filter: `cargo test -p app --test flows -- settings`.

## Status values

- **PASS (flow)**: the named flow or slice test in `crates/app/tests/` passes.
- **FAIL (why)**: the port behaves differently, or no flow can reach the row yet (the reason tells which).
- **NOT WIRED (owner)**: the port has no handler for the row; the owner file must add it.

The "Original" column gives the behavior from the inventory.
The rows were not clicked again on the original through `tools/route1`: its `server.mjs` starts a live `codex app-server` with `CODEX_HOME=/tmp/cgx-mac/home`, and this lane must not touch that directory.

## Totals

| Screen | Rows | PASS | FAIL | NOT WIRED |
|---|---|---|---|---|
| App shell: shortcuts, menu bar, window | 20 | 13 | 3 | 4 |
| 14.1 Left rail | 12 | 4 | 1 | 7 |
| 14.2 Sidebar | 23 | 12 | 4 | 7 |
| 14.3 Home | 21 | 10 | 3 | 8 |
| 14.4 Thread view | 20 | 6 | 3 | 11 |
| 14.5 Composer | 20 | 6 | 5 | 9 |
| 14.6 Diff view | 20 | 2 | 1 | 17 |
| 14.7 Settings, login and onboarding | 20 | 7 | 8 | 5 |
| 15.1 Automations | 16 | 2 | 2 | 12 |
| 15.2 Background terminal | 10 | 2 | 2 | 6 |
| 15.3 Mobile setup | 10 | 2 | 1 | 7 |
| **All** | **192** | **66** | **33** | **93** |

Before round 5 (F2Flows): 57 PASS, 33 FAIL, 102 NOT WIRED. Before round 4 (F1Flows): 42 PASS, 25 FAIL, 125 NOT WIRED.

The historical round 5 run reported 76 flows passed.
The P7 strict-gate run checks 73 flows: 66 pass and seven fail.
Six failures are strict Root parity failures; `worked_for_expands` also reports zero pixels repainted.

The `scroll_*` flows send wheel events (pixel deltas, a trackpad flick with its momentum events, and wheel lines) to the sidebar sections, the thread, the Changes panel, the Settings nav and page, and the Scheduled page.
At 1280x320, the largest offset of the sidebar sections is 82 px, the same as in the original.
This value was recorded through `tools/route1` with a copy of the fixture `CODEX_HOME`.
The other panes must repaint on the first wheel, stay still past the end, and show the start again.

## Parity of the states the flows reach

Each `parity_*` flow enters the real Root with mouse or key input, then checks its route and selected side panel.
The Sidebar flow also checks that the command menu opens; Composer checks its typed draft and open menu.
Only then does the flow compare its captured image with the original reference through `parity::diff_ref`.
The gate requires zero differing device pixels outside text boxes and at most 0.5% inside text boxes.
`Fx::parity` returns an error on a mismatch.
Each screen still runs both themes and sizes before it returns its combined failure.
The flow executable exits 1 if any flow fails.
There is no report-only or optional mode.

Run all nine Root checks with `tools/mac.sh -l root-gate 'cargo test -p app --test flows -- parity_'`.
Keep `cargo run -p parity -- --gate` as the separate static screen gate.
Its 36 passing comparisons do not prove Root parity.

The P7 Mac run checks dark and light at 1280x832 and 1440x900.
Each cell below gives `outside count / text percent`.
The six failing screens remain tracked in [the Root mismatch issue](https://github.com/i098/codex-gpui/issues/151).

| Screen and input entry | Dark 1280x832 | Dark 1440x900 | Light 1280x832 | Light 1440x900 |
|---|---|---|---|---|
| Home: launch | 0 / 0.411% | 0 / 0.411% | 0 / 0.132% | 0 / 0.132% |
| Sidebar: Search, type `math` | 95803 / 12.237% | 102494 / 9.982% | 128171 / 13.479% | 137529 / 10.368% |
| Composer: type `Refactor src/math.py`, click model pill | 11016 / 2.402% | 11016 / 2.402% | 11016 / 2.320% | 11016 / 2.320% |
| Thread: click the subtract row | 3695 / 1.669% | 3695 / 1.649% | 3698 / 1.670% | 3698 / 1.651% |
| Diff: subtract, `⌥⌘B`, collapse file | 3695 / 1.379% | 3695 / 1.492% | 3698 / 1.303% | 3698 / 1.412% |
| Settings: `⌘,` | 0 / 0.088% | 0 / 0.080% | 0 / 0.117% | 0 / 0.107% |
| Automations: click Scheduled | 45742 / 27.970% | 45740 / 27.970% | 45788 / 28.031% | 45764 / 28.031% |
| Terminal: subtract, recorded command events, summary command row | 3830 / 0.745% | 3830 / 0.819% | 3833 / 0.738% | 3833 / 0.808% |
| Mobile: avatar, Set up mobile | 0 / 0.468% | 0 / 0.468% | 0 / 0.467% | 0 / 0.467% |

The old profile helper opens Keyboard shortcuts over Home instead of Mobile.
The corrected helper uses the proved Mobile entry; the old failed captures remain retained in the acceptance evidence.
The large-window Composer click uses the original reference's centered-column and bottom offsets.
The prior large-window clicks miss the menu; the corrected clicks reach it and still fail the strict pixel gate.

Fixture, clock, focus, host-data, and drawing differences remain unresolved.
None of these possible causes waives a failure.
Keep S026's recorded command groups and settled search behavior instead of restoring an older pending-search screenshot state.
Keep S009's File viewer behavior; its recorded test replaces the obsolete file-link-to-Changes assertion.
The S009, S026, F3b, and S034b recorded tests pass with the strict shared helper.
Formatting and `cargo clippy --workspace --all-targets -- -D warnings` pass after integrating current main.
The initial broad clippy run failed at `ms6a.rs:168`; the retained log preserves that result.

An isolated smoke check sends a retained input-driven Composer capture through `parity_image`; it returns exit 1 with 11016 outside pixels.
All 36 unchanged original images pass the same helper with zero differences.
Replaying the old Mobile click returns exit 1 for all four route checks before any pixel comparison.
These checks prove the guard; they do not replace the failing Root captures.

## App shell: shortcuts, menu bar, window (§2.2, §12, §13)

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | `⌘,` | shortcut | Opens Settings (`settings`). | Route Settings, settings shell. | PASS (`cmd_comma_opens_settings`) |
| 2 | `⌘N`, `⌘⇧O` | shortcut | New chat (`newTask`). | Home, draft cleared, composer focused. | PASS (`cmd_n_new_chat`) |
| 3 | `⌘⇧S`, `⌘B` | shortcut | Toggle sidebar. | Shell sidebar toggles. | PASS (`sidebar_toggle_keys`) |
| 4 | `⌘[`, `⌘]` | shortcut | Back, Forward (`navigateBack`, `navigateForward`). | Route history over Home, threads, Settings. | PASS (`back_forward_keys`) |
| 5 | `⌘⇧[`, `⌘⇧]`, `⌘⌥←`, `⌘⌥→` | shortcut | Previous / next chat. | Previous / next Recents row; no wrap. | PASS (`previous_next_chat`) |
| 6 | `⌘1`…`⌘9` | shortcut | Go to chat N (`thread1..9`). | Opens Recents row N. | PASS (`cmd_digit_goes_to_chat`) |
| 7 | `⌘W` | shortcut | Close tab, else close window. | Closes the thread route's selected side panel tab, else the window. | PASS (`cmd_w_closes_window`, `s029::panel_tabs_replay`) |
| 8 | Menu bar: Codex ▸ Settings…, File ▸ New Chat, View ▸ Toggle Sidebar, Toggle Changes Panel, Previous Chat, Next Chat, Back, Forward | click | Same commands as the keys. | `app::menus()` items dispatch their actions. | PASS (`menu_bar_actions`, `changes_panel_keys`) |
| 9 | System appearance and Settings ▸ Appearance ▸ Theme | click | Light / Dark / System applies at once. | `State.theme` (`ThemeSetting`) is applied on each appearance change and by `Root::set_theme`. | PASS (`theme_switch`) |
| 10 | `⌘M`, Window ▸ Minimize, Zoom | shortcut | Minimize, zoom. | Bound; `window.minimize_window()`, `zoom_window()`. | FAIL (no headless check: the test window ignores minimize and zoom) |
| 11 | `⌘Q`, `⌘H`, `⌥⌘H`, Show All | shortcut | Quit, hide, hide others, show all. | Bound; app-level handlers in `app::init`. | FAIL (no headless check: quitting or hiding ends the test app) |
| 12 | Window minimum size | resize | `getPrimaryMinimumSize()` 480x600. | `window_min_size` in `main.rs`. | FAIL (no headless check: the test window does not apply a minimum size) |
| 13 | Live window resize across 768 px | resize | The sidebar hides below 768 px and comes back above it. | `Root::fit` on `observe_window_bounds`: live resizes to 767, 1280, 600 and 1440 px hide and show the sidebar; a sidebar the user closed stays closed. | PASS (`resize_live`, `resize_three_widths`) |
| 14 | `⌘K`, `⌘⇧P` | shortcut | Command menu. | `CommandMenu` over live Recents, focused; Escape or a click outside closes it; Enter or a click runs the row (chat, `New chat`, `Manage scheduled tasks`, settings pages). | PASS (`command_menu_keys`, `command_menu_settings_page`) |
| 15 | `⌘F` | shortcut | Find in chat. | — | NOT WIRED (`crates/ui/src/thread.rs`: no find bar) |
| 16 | `⌥⌘B` | shortcut | Toggle Changes panel. | The thread route's side panel (`ui::screens::diff::panel::with_side_tabs`) shows the last turn's diff in a `Changes` tab beside the other open tabs; the tab's close button closes it. | PASS (`changes_panel_keys`) |
| 17 | `⌘J` | shortcut | Toggle bottom panel. | — | NOT WIRED (`crates/ui/src/lib.rs` `Shell`: no bottom panel) |
| 18 | `⌘⇧N`, `⌘⌥O` | shortcut | Temporary chat, standalone chat. | — | NOT WIRED (`crates/app/src/lib.rs`: no projects or temporary chats) |
| 19 | `⌘⌥1`…`⌘⌥6` | shortcut | Go to recent chat N (`recentThread1..6`: the chats by last update, newest first). | `GoToRecentChat`: the Nth Recents thread by `updatedAt`; past the end does nothing. | PASS (`recent_chat_keys`) |
| 20 | `⌘+`, `⌘-`, `⌘0` | shortcut | Zoom in, zoom out, actual size. | — | NOT WIRED (`crates/ui/src/lib.rs`: no UI scale) |

## 14.1 Left rail

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Home | click | Navigates to `/`; reselect resets Home. | `NewChat`: route Home, draft cleared, composer focused. | PASS (`rail_home_from_thread`, `rail_home_reselect_resets`) |
| 2 | Space | click | Navigates to `/space`. | No handler. | NOT WIRED (outside v1 scope (no screen)) |
| 3 | Scheduled | click | Navigates to `/scheduled` (Automations); unread dot. | `OpenScheduled`: `Route::Scheduled`, rail Scheduled selected. No unread dot (no automation runs). | PASS (`rail_scheduled`, `parity_automations`) |
| 4 | Plugins | click | Navigates to `/plugins`. | No handler. | NOT WIRED (outside v1 scope (no screen)) |
| 5 | Explore `…` | click | Opens the Explore menu; blue dot. | Dot shows; no menu. | NOT WIRED (outside v1 scope (no screen): the menu holds the secondary destinations) |
| 6 | Code Review | click | Navigates to `/pull-requests`. | No handler. | NOT WIRED (outside v1 scope (no screen)) |
| 7 | Sites | click | Navigates to `/sites`. | No handler. | NOT WIRED (outside v1 scope (no screen)) |
| 8 | Profile avatar | click | Opens the profile menu. | Menu above the avatar: `Settings`, `Help` ▸ `Set up mobile`; a click outside or on the avatar closes it. | PASS (`profile_menu_settings`, `profile_menu_dismiss`, `profile_menu_mobile`) |
| 9 | Rail item hover | hover | Tooltip on the right side with label and shortcut. | Hover fill shows (4716 px repainted); a tooltip with the item's label opens 200 ms after the pointer enters (at once when another was open), 2 px right of the item; `Explore` and the avatar have none; it closes on leave, press or Escape. | PASS (`rail_hover_fill`; tooltip: `crates/app/tests/f1a.rs` against `refs/interactions/f1a.json`) |
| 10 | Drag a rail item | click | Reorders pinned items. | — | NOT WIRED (`crates/ui/src/lib.rs` `Shell::rail`) |
| 11 | Focus ring | focus ring | 2 px ring on the 36×36 button. | No tab stops. | NOT WIRED (`crates/ui/src/lib.rs` `Shell::rail`: no focus handles or `focus_visible` ring) |
| 12 | Resize | resize | At 1280, 767 and 600 px the rail stays 52 px. | The rail stays and rail Home answers at all 3 widths. | PASS (`resize_three_widths`) |

## 14.2 Sidebar

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Sidebar visible | entry | Rail 52 px plus panel 288 px at 1280 px. | Open at launch; parity: see `parity_home`. | PASS (`launch_home`, `parity_home`) |
| 2 | `Codex ⌄` mode switch | click | Menu: ChatGPT, Codex, Work. | No handler. | NOT WIRED (`crates/ui/src/lib.rs` `home_sidebar_header`, `mode-switch`) |
| 3 | `⌃1` `⌃2` `⌃3` | shortcut | Switch mode. | — | NOT WIRED (`crates/ui/src/lib.rs`: no modes; then bind in `crates/app/src/lib.rs` `init`) |
| 4 | Bell `View activity` | click | Toggles the activity view. | No handler. | NOT WIRED (`crates/ui/src/lib.rs` `home_sidebar_header`, `activity`) |
| 5 | `⌘⌥U` | shortcut | Toggles the activity view. | — | NOT WIRED (`crates/ui/src/screens/sidebar.rs`: no activity view) |
| 6 | Search | click | Opens the command menu in chat-search mode. | `OpenCommandMenu`: the command menu over live Recents. | PASS (`command_menu_search_click`, `parity_sidebar`) |
| 7 | `⌘K` | shortcut | Opens the command menu. | Same as shell row 14. | PASS (`command_menu_keys`) |
| 8 | `Your dot` row | click | Opens the dot (orbit) thread. | No handler. | NOT WIRED (`crates/ui/src/lib.rs` `home_sidebar_header`, `your-dot`) |
| 9 | `Projects` section toggle | click | Collapses or expands; chevron rotates. | A header click collapses or expands the section; no chevron. | FAIL (no chevron: `crates/ui/src/lib.rs` `sidebar_section`; collapse checked by `sidebar_section_collapse`) |
| 10 | `Recents` section toggle | click | Collapses or expands. | Same; a collapsed Recents row no longer opens. | PASS (`sidebar_section_collapse`) |
| 11 | Thread row | click | Opens `/local/:conversationId`. | `OpenThread` → route Thread, `thread/read`, row selected. | PASS (`thread_row_opens_thread`) |
| 12 | Thread row hover | hover | Row background; Archive / Pin actions appear. | The row reveals its actions. Buttons use the shared 150 ms hover transition. | PASS (`thread_row_hover`, `crates/app/tests/s001.rs`) |
| 13 | Thread row context menu | click | Pin, Rename, Archive, Mark as unread, Open in new window, Delete. | Archive removes the row without opening the chat. Pin and Unpin move the chat into or out of `Pinned` through `thread/section/move`, with no toast; a refused move puts the row back silently. The Unpin icon hides at rest like Pin and Archive. | FAIL (the context menu remains unwired; Archive and Unpin: `crates/app/tests/s001.rs`; Pin: `crates/app/tests/s001b.rs`) |
| 14 | `⌘1`…`⌘9` | shortcut | Go to chat N. | Opens Recents row N; `⌘9` past the end does nothing. | PASS (`cmd_digit_goes_to_chat`) |
| 15 | `⌘⇧[` / `⌘⇧]` | shortcut | Previous / next chat. | Previous / next Recents row. | PASS (`previous_next_chat`, `menu_bar_actions`) |
| 16 | `⌘⇧S` / `⌘B` | shortcut | Toggle sidebar. | Toggles. | PASS (`sidebar_toggle_keys`) |
| 17 | Titlebar `Hide sidebar` | click | Collapses the left panel over a 300 ms spring (main surface slides, sidebar rows fade and shift 16 px); label changes to `Show sidebar`; instant under reduced motion. | The main surface and the sidebar content follow `motion::spring_300` from the toggle's progress; the recorded tooltip stays `Toggle sidebar ⇧⌘S` in both states. | PASS (`sidebar_toggle_titlebar`; `crates/app/tests/s004.rs`; spring frames: `crates/app/tests/ms2.rs` against `refs/interactions/ms2.json`) |
| 18 | Row focus | focus ring | 2 px ring, offset -2 px. | Tab moves through rows and their Pin and Archive actions; focus reveals the actions. | PASS (`crates/app/tests/f1b.rs`) |
| 19 | Empty lists | empty | `No projects`, `No chats`; activity `Nothing needs attention`. | Empty `thread/list` gives no Recents (`No chats` branch); `No projects` always. No activity view. | PASS (`sidebar_empty_lists`) |
| 20 | Lists loading | loading | Activity: `Checking unread chats…`. | — | NOT WIRED (`crates/ui/src/screens/sidebar.rs`: no activity view or loading state) |
| 21 | Load error | error | `Projects couldn't be loaded` + `Retry`; `Could not load chats`. | A `thread/list` error stops `Root::connect`, so no route shows. | FAIL (`crates/app/src/lib.rs` `connect` treats it as fatal; `crates/ui/src/screens/sidebar.rs` has no error state) |
| 22 | Sidebar drag-resize | resize | The recorded edge clamps to 290…520 px; dragging below 145 px collapses it. | The 16 px handle resizes the sidebar and main surface, retains the width on toggle, and saves it on release. | PASS (`crates/app/tests/s004.rs`, `refs/interactions/s004.json`) |
| 23 | Resize | resize | Below 768 px the panel is hidden; it opens as an overlay. | Hidden at 767 and 600 px, shown at 1280 px; the user's choice is kept. `⌘B` opens it docked, not as an overlay. | FAIL (overlay: `crates/ui/src/lib.rs` `Shell`; hide/show checked by `resize_three_widths`) |

S001 replays `refs/interactions/s001.json` with `cargo test -p app --test s001` on macOS.
The recording uses the original renderer with isolated host and app-server fixtures.
The replay checks Unpin, Archive, the Home route, outgoing requests, button timing, and the recorded hover blend.
The native host translates `pinnedThreads.set(false)` to `thread/section/move` with a null section.
The original shows no toast for this Unpin action.
S001b replays `refs/interactions/s001b.json` with `cargo test -p app --test s001b` on macOS: the Pin click moves the row under `Pinned` at once, sends `thread/section/move` with the fixed Pinned section, shows no toast, hides the Unpin icon at rest, and a refused move puts the row back without a toast.
Running-chat and scheduled-task archive confirmation still need a dialog extension outside this six-file slice.

The S004 recording uses the original renderer through the Linux route1 stack in an isolated Chrome context.
Its twelve actions check resize limits, collapse, restored width, titlebar toggles, tooltips, boundary pixels, and no app-server requests.
The current original tooltip is `Toggle sidebar ⇧⌘S`, not the inventory's `⌘B` label.
The static parity gate retains all 36 baseline counts.

F1b replays all 30 original steps with `cargo test -p app --test f1b` on macOS.
Clicking a section header keeps pointer focus; Enter enables its keyboard ring without moving focus.
The section header retains its original 202×25 px target inside the 272×30 px layout row at the recorded width.
The S003 header replacement enlarged that target, so the ring moved away from the recorded border pixels.
The repair restores the shared target and tab group for Projects, Recents, and Pinned without changing the ring painter or input handling.
The existing replay checks the original colors, row actions, tooltips, canceled row click, Home route, and absence of app-server writes.

## 14.3 Home

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Launch to `/` | entry | Hero, announcement slot and composer. | Route Home, rail Home selected, composer focused, Recents loaded. | PASS (`launch_home`, `parity_home`) |
| 2 | `New chat` row | click | Navigates to `/` and clears the draft. | Same. | PASS (`new_chat_row`) |
| 3 | `⌘N` / `⌘⇧O` | shortcut | New chat on Home, composer focused. | Same. | PASS (`cmd_n_new_chat`) |
| 4 | Hero project selector | click | Opens the project menu. | No projects. | NOT WIRED (`crates/ui/src/screens/home.rs` hero) |
| 5 | Beacon `Start Voice` | click | Starts a realtime voice chat. | No beacon. | NOT WIRED (`crates/ui/src/screens/home.rs`) |
| 6 | Beacon dismiss `×` | click | Hides the banner. | No beacon. | NOT WIRED (`crates/ui/src/screens/home.rs`) |
| 7 | `Choose project` | click | Project dropdown. | Chip has no handler. | NOT WIRED (`crates/ui/src/screens/home.rs` `utility_bar`) |
| 8 | `Work in` / `This computer` | click | Location menu. | Chip has no handler. | NOT WIRED (`crates/ui/src/screens/home.rs` `utility_bar`) |
| 9 | Editor `Do anything` | focus | Caret shows. | Click focuses the editor. | PASS (`editor_click_focuses`) |
| 10 | `+` Add files and more | click | Context menu (files, photos, @-mentions). | Opens the system file picker at once. | FAIL (`crates/ui/src/composer.rs` `add_files`: no context menu) |
| 11 | `Custom` permissions | click | Permissions dropdown. | Opens; Escape closes. | PASS (`permissions_popover`) |
| 12 | Model picker | click | Model and effort menu. | Opens; Escape closes. | PASS (`model_picker_popover`) |
| 13 | `Dictate` | click | Starts dictation. | No handler. | NOT WIRED (`crates/ui/src/composer.rs`, `dictate`) |
| 14 | `Start new voice chat` | click | Starts realtime voice. | With an empty draft the button calls `submit`, which does nothing. | NOT WIRED (`crates/ui/src/composer.rs`, `send`) |
| 15 | Submit with Enter | shortcut | `thread/start`, `turn/start`, navigates to the thread. | Same; the turn streams in; the draft clears; also by clicking Send. | PASS (`submit_enter_starts_thread`, `submit_click_send`) |
| 16 | Hover on every button | hover | Ghost background fades in. | Hover fill on composer buttons (checked on `Dictate`). | PASS (`composer_button_hover`) |
| 17 | Tab through controls | focus ring | 2 px ring. | No tab stops. | NOT WIRED (`crates/ui/src/composer.rs`, `crates/ui/src/screens/home.rs`) |
| 18 | No projects / no chats | empty | `No projects`, `No chats`, `What should we build?`. | Same with an empty `thread/list`. | PASS (`sidebar_empty_lists`) |
| 19 | Model list loading | loading | `Loading model…`. | Empty `model/list` shows `Loading model…` (the pill repaints). | PASS (`model_list_loading`) |
| 20 | Account load failure | error | `We couldn’t load your account`, `Reload`, `Sign out`. | An `account/read` error stops `Root::connect`; the window stays empty. | FAIL (`crates/app/src/lib.rs` `connect`; no error page in `crates/ui`) |
| 21 | Resize | resize | Below 768 px the sidebar is an overlay; below 40rem the composer stacks. | Sidebar hides (row 14.2/23); composer does not stack. | FAIL (`crates/ui/src/composer.rs`: only the 475 px compact footer) |

## 14.4 Thread view

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Open `/local/:conversationId` | entry | `thread/resume`, turns render, scroll to bottom. | `thread/read` with turns (resume on the first send); parity: see `parity_thread`. | PASS (`thread_row_opens_thread`, `parity_thread`) |
| 2 | History loading | loading | `Loading task…`. | The main panel is empty while `thread/read` runs. | NOT WIRED (`crates/ui/src/thread.rs`: no loading view; `crates/app/src/lib.rs` `sync` shows `EmptyMain`) |
| 3 | History failure | error | `Couldn't load messages` + `Retry`. | The error is logged; the panel stays empty. | FAIL (`crates/app/src/lib.rs` `load_thread` only logs; no error view in `crates/ui/src/thread.rs`) |
| 4 | Turn render failure | error | `This turn couldn't render`. | — | NOT WIRED (`crates/ui/src/thread.rs`) |
| 5 | Streaming agent message | loading | Text grows by deltas; new segments fade over 700 ms; `Working` shimmer. | New glyphs use the recorded adaptive fade. Later deltas do not restart older glyphs. Reduced motion shows the end state immediately. Shimmer belongs to S5. | PASS (`submit_enter_starts_thread`, `stream_fade`) |
| 6 | Reasoning item | click | Toggles `Thought for {elapsed}`. | No handler. | NOT WIRED (`crates/ui/src/thread.rs`) |
| 7 | Command card | click | Expands output; copy buttons. | No handler. | NOT WIRED (`crates/ui/src/thread.rs`) |
| 8 | File change card | click | `View changes` opens Changes; `Undo` / `Reapply`. | `View changes` opens the Changes side panel; `Undo` sends `thread/revert` (`beforeTurnId`, after `thread/resume` if needed) and shows the reverted thread. No `Reapply`. | FAIL (`Reapply`: the original re-applies through the host git worker; `Undo` checked by `undo_reverts_turn`, `View changes` by `view_changes_opens_panel`) |
| 9 | Collapsed turn `Worked for {time}` | click | Expands hidden items. | A click shows the turn's work above the answer (agent messages, `Ran commands`) and turns the chevron down; a second click folds it. | PASS (`worked_for_expands`) |
| 10 | User message hover | hover | Copy and Edit appear. | — | NOT WIRED (`crates/ui/src/thread.rs`) |
| 11 | Assistant message hover | hover | Copy, Fork chat from here. | Shown under the last turn, on hover under earlier turns. `Copy` puts the answer on the clipboard; `Fork chat from here` sends `thread/fork` (then `thread/rollback` of the later turns) and opens the fork at the top of Recents. `Read aloud` and `Rate response` have no handler. | PASS (`copy_response`, `fork_chat`, `earlier_turn_actions_hover`) |
| 12 | `Scroll to bottom` | click | Jumps to the newest turn. | — | NOT WIRED (`crates/ui/src/thread.rs`) |
| 13 | `⌘F` | shortcut | Find bar. | — | NOT WIRED (`crates/ui/src/thread.rs`) |
| 14 | `⌘⇧A` | shortcut | Archives the chat. | `ArchiveChat`: `thread/archive`, the row leaves Recents, the route goes to Home. | PASS (`archive_chat`) |
| 15 | `⌘⌥R` | shortcut | Rename dialog. | — | NOT WIRED (`crates/ui`: no rename dialog) |
| 16 | Approval card | click | Approve, Allow similar, Allow all edits, Deny. | `Root::on_event` shows the card; `Allow once`, the scoped allow and `Deny` answer the server request by its id. | PASS (`approval_round_trip`: a command approval request crafted from `CommandApprovalRequest`, because the sessions ran with `approvalPolicy: never`) |
| 17 | Stream error | error | `Reconnecting {progress}`. | — | NOT WIRED (`crates/ui/src/thread.rs`) |
| 18 | Usage limit | error | Usage limit banner. | — | NOT WIRED (`crates/ui/src/thread.rs`) |
| 19 | Focus ring on cards | focus ring | 2 px ring. | — | NOT WIRED (`crates/ui/src/thread.rs`) |
| 20 | Resize | resize | The side panel yields width before the chat column. | `panel_width` keeps 320 px for the chat; no flow checks the 3 widths with the panel open. | FAIL (no flow; `crates/ui/src/diff.rs` `panel_width` assumes the sidebar open) |

MO-36 uses `refs/interactions/ms4.json`, recorded with the original CSS classes in an isolated route1 Chrome context.
The recording appends synthetic markdown spans and seeks their animations; it does not measure live model segmentation.
Run `tools/mac.sh -l ms4 'cargo test -p app --test stream_fade'` for native glyph-alpha checks.
The test samples each delta from 0 to 700 ms and checks that earlier glyphs remain unchanged.
Linux uses a no-op text system, so these glyph checks require macOS.
MO-37 and MO-39 remain with the image and block-renderer slices, as approved for MS4.

## 14.5 Composer

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Composer in thread | entry | `Work with Codex` (idle) or `Working…`. | Docked thread composer, focused on open; `Working…` not checked. | PASS (`shift_enter_newline`, `parity_thread`) |
| 2 | Type text | focus | Editor grows to a max height, then scrolls. | Lines add; height growth not measured. | FAIL (no flow checks the max height) |
| 3 | Enter | shortcut | Sends; `⌘ + Enter` modes per setting. | Enter sends; no `Send shortcut` setting. | PASS (`submit_enter_starts_thread`) |
| 4 | Shift+Enter | shortcut | New line. | New line in the Home and thread composers. | PASS (`shift_enter_newline`) |
| 5 | `@` | shortcut | Mention list. | — | NOT WIRED (`crates/ui/src/composer.rs`) |
| 6 | `/` | shortcut | Slash command menu. | Matches the recorded menu, filtering, selected row, editor focus, arrows, Escape, Enter, and Tab. | PASS (`s021::slash_menu_keyboard_replay`) |
| 7 | Paste large text | click | `Pasted text.txt` chip. | Paste inserts the text. | NOT WIRED (`crates/ui/src/composer.rs` `paste`) |
| 8 | Drag file over composer | hover | `Drop to attach`. | — | NOT WIRED (`crates/ui/src/composer.rs`) |
| 9 | Attachment chip remove | click | Removes the attachment. | Handler exists (`remove-attachment`). | FAIL (no flow: attachments come only from the system file picker, which the headless window does not answer) |
| 10 | Model picker | click | `Select model`, `Select effort`, Fast. | Opens; a model or effort choice writes `model` and `model_reasoning_effort` with `config/batchWrite` (the original's `writeModel`); parity: see `parity_composer`. | PASS (`model_picker_popover`, `effort_pick_writes_config`, `parity_composer`) |
| 11 | Permissions dropdown | click | Ask / Approve for me / Full access / Custom. | Opens. | PASS (`permissions_popover`) |
| 12 | Submit while running | click | Stop; with text Queue or Steer. | Stop appears when the composer is empty. Queue and Steer remain unimplemented. | FAIL (CP-108 follow-up) |
| 13 | Stop | click | `turn/interrupt`. | Sends the active thread and turn IDs; completion removes Stop. Errors show `Failed to stop chat`. | PASS (`s023::stop_interrupts_only_the_active_turn`) |
| 14 | Queued message row | click | Steer, Edit, Delete, Retry. | — | NOT WIRED (`crates/ui/src/composer.rs`) |
| 15 | Submit blocked | error | `Unable to send message`. | — | NOT WIRED (`crates/ui/src/composer.rs`) |
| 16 | Upload in progress | loading | `Files uploading…`. | — | NOT WIRED (`crates/ui/src/composer.rs`) |
| 17 | Upload failure | error | `Unable to upload file`. | — | NOT WIRED (`crates/ui/src/composer.rs`) |
| 18 | Button hover | hover | Ghost hover; tooltip after a delay. | Hover fill; no tooltip. | FAIL (no tooltip: `crates/ui/src/composer.rs`; fill checked by `composer_button_hover`) |
| 19 | Focus ring | focus ring | Send button ring. | — | NOT WIRED (`crates/ui/src/composer.rs`: no `focus_visible` ring) |
| 20 | Resize | resize | Below 40rem the editor and footer stack. | Only the 475 px compact footer. | FAIL (`crates/ui/src/composer.rs`) |

S021 replays all 16 original steps from `refs/interactions/s021.json` for CP-054 and CP-055.
The surface check covers the menu's top and bottom padding across its width, outside the rounded corners.
Filtered selected rows cover the former scrollbar gutter, so that gutter cannot identify the menu background.
The original ArrowDown counterfactual changes the old sample without changing the menu bounds, draft, or focus.
The replay retains the selected-row color, command order, focus, and exact request checks.
Run `tools/mac.sh -l s021-repair 'cargo test -p app --test s021 --test s019 --test s020 --test s022 -- --test-threads=1'`.
The neighboring checks retain command draft rules, mention navigation, atomic chips, undo, and editor input.
Other command bodies and the plus Add menu remain outside S021.

The S023 test replays `refs/interactions/s023.json`, recorded from the original renderer with the isolated Linux route1 stub.
It checks the Stop pixels, request, error toast, focus, completion, and inactive click.
Firstmate authorized a separate CP-108 follow-up after the corrected recording stub still emitted no `turn/steer` request.

## 14.6 Diff view

`Root` opens the Changes panel beside the open thread (`⌥⌘B`, `View changes`, the summary's `Changes` row) through `ui::screens::diff::panel::with_side_panel`, which the `diff` screen builder also uses.

Slice S028 records the original with the Linux route1 recorder in an isolated browser context.
`refs/interactions/s028.json` records file changes, aggregate diff updates, and the `def` search.
The native `find-in-thread` message substitutes for the Electron shortcut in the browser.
The port test checks `Cmd+F`, the native Find action, both result counts, and the recorded highlight colors.
The app routes Find to the focused Changes panel before thread find.
The replay checks that diff find never opens thread find.
It resolves `View changes` from the static reference because Linux text layout places that button below the Mac reference.
Run `tools/mac.sh -l s028-repair 'cargo test -p app --test s028 -- --nocapture --test-threads=1'`.
The open panel keeps its state across streamed updates and uses `turn/diff/updated` as the authoritative diff.
Search scope switching and repository-watch sources remain separate inventory slices.

`refs/interactions/s027.json` records the original diff menus, text selection, clipboard writes, and line hover.
`s027::diff_selection_and_hover_replay` checks forward and reverse text selection and `Cmd+C` without line numbers.
The original does not change a line's background on hover in this state, despite the DT-014 inventory claim.
The test compares the port's pixels before and after the same hover.
The header menu copies the displayed path and collapses or expands the file.
The body menu copies selected text, absolute paths, or displayed relative paths.
The body menu also wraps long code lines without changing the copied text.
The editor entry stays disabled until the native `openIn` service exists.
Comment actions (S075/S082), file tabs (S186), and external targets remain outside this slice, as firstmate approved.


| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Open Changes panel | entry | `⌥⌘B` or `View changes`. | Both open the last turn's diff (`Last Turn` source). A file link opens the File viewer, as S009 records. | PASS (`changes_panel_keys`, `view_changes_opens_panel`, `s009`); strict pixels FAIL (`parity_diff`) |
| 2 | `/diff` window | entry | Single file diff. | — | NOT WIRED (`crates/app/src/main.rs`: one window only) |
| 3 | Source selector | click | Uncommitted / Staged / … | — | NOT WIRED (`crates/ui/src/diff.rs`) |
| 4 | Show / Hide files | click | Toggles the file tree. | — | NOT WIRED (`crates/ui/src/diff.rs`) |
| 5 | File tree filter | focus | `Filter files…`. | — | NOT WIRED (`crates/ui/src/diff.rs`) |
| 6 | File header actions | click | Copy path, Open, Collapse/Expand. | Copy path and Collapse/Expand work. Open in editor remains disabled pending host services. | PARTIAL (`s027::diff_selection_and_hover_replay`) |
| 7 | Hunk revert / stage | click | Revert, Stage / Unstage hunk. | — | NOT WIRED (`crates/ui/src/diff.rs`) |
| 8 | Revert all | click | `Revert changes?` dialog. | — | NOT WIRED (`crates/ui/src/diff.rs`) |
| 9 | Split / unified toggle | click | Switch diff layout. | — | NOT WIRED (`crates/ui/src/diff.rs`) |
| 10 | Word wrap | click | Toggles wrap. | The body context menu toggles wrap; the header control remains separate work. | PARTIAL (`s027::diff_selection_and_hover_replay`) |
| 11 | Mark as viewed | click | Collapses, persists. | — | NOT WIRED (`crates/ui/src/diff.rs`) |
| 12 | `⌘F` in diff | shortcut | `Search diff…`. | Focus the panel, open find, and type a query; matches highlight and update with the diff. | PASS (`s028::streamed_diff_and_find_replay`) |
| 13 | `⌘L` | shortcut | Go to line. | — | NOT WIRED (`crates/ui/src/diff.rs`) |
| 14 | Line hover | hover | No background change in the recorded Last Turn state; no `data-hovered` attribute. | No background change. | PASS (`s027::diff_selection_and_hover_replay`; corrects DT-014) |
| 15 | No changes | empty | `No file changes yet`. | — | NOT WIRED (`crates/ui/src/diff.rs`) |
| 16 | Not a git repo | empty | `Create a Git repository`. | — | NOT WIRED (`crates/ui/src/diff.rs`) |
| 17 | Loading | loading | `Loading diff`. | — | NOT WIRED (`crates/ui/src/diff.rs`) |
| 18 | Load failure | error | `Couldn't load changes`. | — | NOT WIRED (`crates/ui/src/diff.rs`) |
| 19 | Too large | error | `Diff too large to display`. | — | NOT WIRED (`crates/ui/src/diff.rs`) |
| 20 | Resize | resize | Split → inline for narrow panes. | — | NOT WIRED (`crates/ui/src/diff.rs`) |

## 14.7 Settings, login and onboarding

A settings row reports its change to `Root`.
`desktop.*` and `features.*` rows persist with `config/value/write`; `Root` keeps the value after the server accepts it.
The menu-bar switch persists through the local `settings.write` service.
Session rows keep their values for the session.
Leaving and reopening Settings shows the kept value (`settings_value_round_trip`).

S034 covers ST-003 and ST-026 with the unchanged off/on samples in `refs/interactions/s034.json`.
The replay advances the virtual clock by the original 150 ms transition before checking settled pixels.
Enabled switches use the existing GPUI native click path for Enter and Space, without duplicate key handlers.
The replay checks pointer focus without a ring, the blue keyboard ring, saved values, reload, and Back navigation.
S034b still checks the recorded sign-in path; no account or authentication changes are part of this repair.

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Open settings | entry | `⌘,`, rail Settings or profile menu; `/settings/general-settings`. | `⌘,`, Codex ▸ Settings… and profile menu ▸ `Settings` open the General page; parity: see `parity_settings`. | PASS (`cmd_comma_opens_settings`, `menu_bar_actions`, `profile_menu_settings`, `parity_settings`) |
| 2 | `Back to app` | click | Returns to the previous route. | The nav has no `Back to app` row; titlebar Back returns. | PASS (`settings_back`) |
| 3 | Nav item | click | Navigates to `/settings/:section`. | The page changes (1488938 px repainted). | PASS (`settings_nav_click`) |
| 4 | Search settings | focus | Filters; `No results found`. | Typing filters the nav to `No results found`; Escape clears. | PASS (`settings_search`) |
| 5 | Theme Light/Dark/System | click | Applies the theme at once. | Appearance ▸ Theme (`Light`, `Dark`, `System`) dispatches `SetTheme`; `Root::set_theme` applies and persists it. | PASS (`appearance_theme_control`) |
| 6 | Collapse nav | click | `Collapse settings navigation`. | — | NOT WIRED (`crates/ui/src/screens/settings.rs` `SettingsNav`) |
| 7 | Row hover | hover | Row highlight. | Highlight shows. | PASS (`settings_nav_hover`) |
| 8 | Keyboard nav | focus ring | Ring on rows and controls. | S034 proves pointer focus, Enter/Space activation, and the keyboard ring on switches. Other rows remain unwired. | NOT WIRED (remaining rows and controls) |
| 9 | Section loading | loading | Skeleton row. | — | NOT WIRED (`crates/ui/src/screens/settings.rs`) |
| 10 | Save failure | error | Toast. | The error is logged. | FAIL (`crates/ui/src/screens/settings.rs` `send` only logs; no toast) |
| 11 | Login `/login` | entry | Sign-in screen. | A signed-out `account/read` shows the sign-in view. | PASS (`signed_out_shows_login`) |
| 12 | `Continue with ChatGPT` | click | `account/login/start`, waits for `account/login/completed`. | Handler in `Login`. | FAIL (no flow: the fixtures hold no `account/login/*` exchange) |
| 13 | `Cancel sign-in` | click | `account/login/cancel`. | Handler in `Login`. | FAIL (no flow: no recorded exchange) |
| 14 | `Copy sign-in link` | click | Copies the URL; toast. | Handler in `Login`. | FAIL (no flow: needs a recorded login URL) |
| 15 | API key form | focus | `sk-…`; `Continue` when non-empty. | Handler in `Login`. | FAIL (no flow: no recorded `account/login/start` with an API key) |
| 16 | Login error | error | `Sign-in failed: {rawMessage}`. | `Login::apply` shows it. | FAIL (no flow: no recorded failure) |
| 17 | Sign-in requirements loading | loading | `Loading sign-in requirements…`. | — | NOT WIRED (`crates/ui/src/screens/settings.rs` `Login`) |
| 18 | Welcome `/welcome` | entry | Onboarding then Home. | — | NOT WIRED (`crates/app/src/lib.rs`: no onboarding route; no onboarding view in `crates/ui`) |
| 19 | Select workspace | click | Pick a folder or skip. | — | FAIL (the port takes its project from `$CODEX_GPUI_CWD` or the launch directory, `crates/app/src/main.rs`) |
| 20 | Resize | resize | [INFERENCE] The nav collapses on narrow widths. | The settings nav stays at all widths. | FAIL (`crates/app/src/lib.rs` `fit` covers the main shell only; the original behavior is an inference) |

## 15.1 Automations ("Scheduled")

`Root` opens the page on rail `Scheduled` and on the command menu's `Manage scheduled tasks`. The page data comes from host fetches (`list-automations`, `inbox-items`, `automation-*`); `CloudHost` (`crates/app/src/host.rs`) serves the local ones from `CODEX_HOME` and the ChatGPT backend ones (`/wham/usage`, `/beacons/home`, `/automations?filter=scheduled`, `/suggested_automations`, the suggestion card's conversation start) with the sign-in that `codex app-server` holds. The page shows the empty state with the landing examples until the UI reads those answers.

Slice S035 proves suggestion hover (AM-037) with `refs/interactions/s035.json` and `crates/app/tests/s035.rs`.
The test replays pointer entry and exit on the existing fixture screen with the original server suggestions.
It checks the card fill, the adjacent card, and the icon colors against the Linux route1 recording.
The existing hover implementation already matches; no production code changes are needed.
Slice S035b proves suggestion click (AM-036) with `refs/interactions/s035b.json` and `crates/app/tests/s035b.rs`.
The click puts the card in `Starting…` (rays spinner, status text) and disables every card at opacity 0.4.
`Root` then reads the TPP models through the desktop host like the original's guided start (`conversation-actions` `J`).
The oracle answers that read with the host's 499, so the recorded outcome is the toast `Couldn’t start this suggested task` and the cards enabled again.
The conversation prepare, its stream, and the navigation into the started conversation are not recorded and not wired.

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Rail `Scheduled` | entry | Automations page, `ScheduledSidebar`. | `Route::Scheduled` with `ScheduledSidebar` and the page. | PASS (`rail_scheduled`, `parity_automations`) |
| 2 | Deep link `/scheduled?automationId=…` | entry | Page with the task selected. | — | NOT WIRED (`crates/app/src/lib.rs`: no URL routing; no tasks without the host) |
| 3 | `New task` | click | Create flow. | Dispatches `NewTask`; no handler. | NOT WIRED (no host: `automation-create` is a host fetch) |
| 4 | `Create` menu | click | Create with Codex / ChatGPT / manually. | — | NOT WIRED (no host: `automation-create`) |
| 5 | Search | focus | Filters; `No matching tasks`. | Dispatches `SearchTasks`; no handler. | NOT WIRED (no host: `list-automations`) |
| 6 | Status filter | click | All / Active / Paused. | — | NOT WIRED (no host: `list-automations`) |
| 7 | Task row | click | Detail panel. | — | NOT WIRED (no host: `list-automations`) |
| 8 | Row actions | click | Edit, Run now, Pause, Delete. | — | NOT WIRED (no host: `automation-update`, `automation-delete`) |
| 9 | Editor fields | focus | Name, Prompt, schedule. | — | NOT WIRED (no host: `automation-create`) |
| 10 | Previous runs | click | Run chat, mark read, archive. | — | NOT WIRED (no host: `inbox-items`, `automation-run-archive`) |
| 11 | Row hover | hover | Row highlight. | No task rows; suggestion hover matches the original recording. | FAIL (no task rows; suggestion hover passes `s035::suggestion_hover_matches_original`) |
| 12 | Focus ring | focus ring | 2 px ring. | — | NOT WIRED (`crates/ui/src/screens/automations.rs`) |
| 13 | No tasks | empty | `No scheduled tasks`, suggestions. | `No scheduled tasks` and the landing examples (no `/suggested_automations` backend). | PASS (`rail_scheduled`, `parity_automations`) |
| 14 | Loading | loading | `Loading scheduled tasks…`. | — | NOT WIRED (no host: `list-automations`) |
| 15 | Error | error | `Scheduled tasks could not be loaded`. | — | NOT WIRED (no host: `list-automations`) |
| 16 | Resize | resize | List and detail share the main column. | No list or detail panel. | FAIL (no task list without the host) |

## 15.2 Background terminal

`Root` opens the terminal tab beside the open thread from the thread summary's `Background processes` row, through the same side panel as Changes.

The Terminal helper sends all six recorded notifications through `Root::on_event` in the original order.
The sequence starts with active status and ends with idle status after the completed turn.
Completion marks the selected thread unread and shows Home attention, as the original renderer does.
Opening or closing a side tab preserves both indicators.
Opening the unread thread clears both indicators.
`background_terminal_tab` checks these transitions, the open summary, the running command, and output `2\n`.
The complete strict Terminal pixel gate still fails; the separate rail and capture repairs remain required.
References, text boxes, route checks, and thresholds stay unchanged.
See [the scoped unread issue](https://github.com/i098/codex-gpui/issues/169).

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Thread with a running background command | entry | `Background terminal` entry in the thread. | `Toggle summary` lists it under `Background processes`, as refs/terminal shows. | PASS (`background_terminal_tab`, `parity_terminal`) |
| 2 | Click the entry | click | Opens the side panel tab. | `OpenBackgroundTerminal`: the command's tab (`Xterm`) in the side panel. | PASS (`background_terminal_tab`, `parity_terminal`) |
| 3 | Output streaming | loading | Appends `outputDelta`; `No output yet`. | The tab shows the replayed `outputDelta`; no `No output yet`. | FAIL (no `No output yet`: `crates/ui/src/screens/terminal.rs` `Xterm`; output checked by `parity_terminal`) |
| 4 | `Stop all background terminals` | click | `thread/backgroundTerminals/clean`. | — | NOT WIRED (`crates/ui/src/thread.rs`: no stop button; no recorded `thread/backgroundTerminals/clean`) |
| 5 | Stop failure | error | Toast. | — | NOT WIRED (`crates/ui`) |
| 6 | Tab close | click | Closes the tab; `⌘W`. | The tab's `×` or `⌘W` closes the selected tab; the next tab, else the previous, is selected; the last tab closes the panel. | PASS (`s029::panel_tabs_replay`, `background_terminal_tab`, `changes_panel_keys`) |
| 7 | Tab thumbnail | hover | Thumbnail preview. | — | NOT WIRED (`crates/ui/src/diff.rs` `tab_strip`) |
| 8 | Command finishes | empty | Leaves the running list; buffer kept. | The tab follows the running list only, so its buffer goes with the command. | NOT WIRED (`crates/ui/src/screens/terminal.rs` `Xterm`: no kept buffer) |
| 9 | Focus ring | focus ring | 2 px ring. | — | NOT WIRED (`crates/ui/src/diff.rs` `tab_strip`) |
| 10 | Resize | resize | xterm refits columns. | — | NOT WIRED (`crates/ui/src/screens/terminal.rs`) |

## 15.3 Mobile setup

`Root` opens the page from the profile menu's `Help` ▸ `Set up mobile` (`Route::Mobile`).

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Profile menu → Help → `Set up mobile` | entry | `/codex-mobile`. | `Route::Mobile`: sidebar closed, no rail item selected. | PASS (`profile_menu_mobile`, `parity_mobile`) |
| 2 | Initial step | click | `Get started` / `Later`. | Dispatches `GetStarted`; no handler, no `Later`. | NOT WIRED (`crates/ui/src/screens/mobile.rs`: no allow-host step view) |
| 3 | Allow host step | click | `remoteControl/enable`. | — | NOT WIRED (`crates/ui/src/screens/mobile.rs`, then `crates/app/src/lib.rs`) |
| 4 | MFA required | error | `Turn on Multi-Factor Authentication`. | — | NOT WIRED (`crates/ui/src/screens/mobile.rs`) |
| 5 | Waiting / pairing step | loading | QR code, pairing code. | — | NOT WIRED (`crates/ui/src/screens/mobile.rs`) |
| 6 | Pairing unavailable | error | `Pairing code unavailable`. | — | NOT WIRED (`crates/ui/src/screens/mobile.rs`) |
| 7 | Connected step | click | Toggles, `Done`, `Manage connections`. | — | NOT WIRED (`crates/ui/src/screens/mobile.rs`) |
| 8 | Button hover | hover | Ghost/solid hover. | `Get started` hover fill (55250 px repainted). | PASS (`mobile_button_hover`) |
| 9 | Focus ring | focus ring | 2 px ring. | — | NOT WIRED (`crates/ui/src/screens/mobile.rs`) |
| 10 | Resize | resize | Content stays centered. | — | FAIL (no flow checks the 3 widths on this route) |

## Changes needed outside crates/app

A row above that names a `crates/ui` owner needs that change first. The changes that unblock the most rows:

1. `crates/ui/src/thread.rs`: click handlers on the reasoning and command cards, `Reapply`, the user message's hover actions, a find bar, and loading and error views.
2. `crates/ui/src/diff.rs`: the Changes header controls (source selector, file tree, layout, wrap, refresh), empty, loading and error states.
3. `crates/ui/src/screens/mobile.rs`: the allow-host, pairing and connected steps (`remoteControl/*`).
4. Focus-visible rings on the shared buttons (`icon_button`, `pill`, rail items), and tooltips on the composer and sidebar row buttons.

## S016: Thread header

`crates/app/tests/s016.rs` replays `refs/interactions/s016.json` on the Mac headless app.
The test opens a chat, opens and dismisses `Chat actions`, and checks the header tooltips.
It checks the title, menu labels, menu pixels, tooltip position, and outgoing thread requests.
The menu uses the original labels, icons, separators, and shortcuts.
The `Toggle summary` tooltip stays inside the window.
The recorded `Chat actions` button has no tooltip.

This slice covers TH-011, TH-013, and TH-036.
Menu command activation, submenus, and arrow-key navigation belong to the later header slices.
The recording also retains the delayed `Add scheduled task…` item as an observation of host-dependent state.

## S003: Sidebar state

`crates/app/tests/s003.rs` replays `refs/interactions/s003.json` on the Mac headless app.
Projects collapses and expands its child list.
Active threads show a spinner; completed background turns show a blue unread dot.
Hover replaces either indicator with row actions.
Opening a thread clears its unread dot.
A persistent title-clip ID keeps the hover layout state, so long titles do not cover the actions.

The original's default Recents section uses infinite paging, with ten more rows when its end enters the view.
The recording shows 20 rows after initial layout and 40 after scrolling.
The port loads cursor pages and shows `Could not load more chats` after a failed page; a click retries that cursor.
The test checks both the paging requests and the retry transition.

The original hides timestamps in every Home sidebar list, including project groups.
It shows an unread dot after a failed background turn, rather than an error icon.
`Show more` belongs to a non-final Recents section; `Show less` belongs to custom sections.
Those section arrangements remain outside this port's default layout.

## S025: Command-query caret and selection

`crates/app/tests/s025.rs` checks 45 native editing states from `refs/interactions/s025.json`.
The original recording uses the background Mac renderer and native editing commands for Command shortcuts.
The test checks query text, UTF-16 selection, selection direction, and input focus after every recorded step.
It also checks copied text and the original selection color.
The recording covers caret insertion, selection replacement, word and line deletion, select-all, copy, cut, paste, and pointer selection.
Clipboard line endings become spaces in the single-line query.
The test also checks focus restoration and result activation after caret movement.
Static reference scenes hide the caret, as the original capture does.
Ranking, command providers, and the S026 search contract remain unchanged.

## S026: Command menu groups and chat results

`crates/app/tests/s026.rs` replays `refs/interactions/s026.json` with the shared headless app helpers.
The original recording uses an isolated route1 home and a fleet Chrome context.
The test checks every group and row on Home and an open chat, including commands that only the chat route registers.
The two queries check title and content matches, substring highlights, project labels, snippets, and exact `thread/search` parameters.
The result layout keeps the project column at 96 px.
The test also checks the unread dot in the port's pixels.
The unread state is injected in the port because the original fixture contains no unread chat.
Command activation belongs to the later command-menu slices.
The test also checks selection after a delayed search reply and spinner removal after a failed search.

## F0b: Recording contract dispatch

`crates/app/tests/replay.rs` checks every JSON recording in `refs/interactions/`.
Each slice uses its dedicated test target, which keeps its state, pixel, and request assertions.
The motion recording `ms4.json` uses `stream_fade`; `selftest.json` uses the generic step checks.
The recorder determinism bundle runs through `f0a` instead of being skipped.
S017 also compares its first recorded filesystem writes through the shared request comparison.
The proxy log preserves paths, directory flags, and decoded attachment contents while the comparison normalizes random IDs.
Later restoration writes cannot hide an incorrect initial paste.
Unknown contracts, an empty recording selection, and failed adapters fail the combined result.
The driver checks the remaining recordings after an adapter fails and reports every failure.
It does not treat known acceptance failures as success.

Run the complete replay on the Mac:

```sh
tools/mac.sh -l accept-parity 'cargo test -p app --test replay -- --test-threads=1 --nocapture'
```

Set `REPLAY=<stem>` to select one exact recording, such as `REPLAY=selftest`.
An incomplete `portReference` returns a field error instead of panicking.
The regression check passes the same incomplete object that panicked before the repair.
No original recording, product behavior, or pixel threshold changes.

## S014: Grouped activity and completed reasoning

`crates/app/tests/s014.rs` replays `refs/interactions/s014.json` on the Mac headless app.
The replay advances the test clock by the recorded 800 ms after the `Worked for` click.
Without that wait, the disclosure remains at animation progress zero when the screenshot runs.
The legacy `worked_for_expands` check uses the same recorded wait after each disclosure click.
Both checks keep their original pixel assertions.

The replay checks the summary box, hover text, chevron, expanded row icons, row text, collapse, and request count.
The original hides the completed reasoning fixture and draws no `Thought` header.
The port retains that state.
No product code, probe coordinates, fixture responses, or pixel gates change.

The fresh original input replay matches the recorded disclosure and activity row boxes exactly.
It also checks the original hover color, expanded rows, collapse, and hidden completed reasoning.
[Original screenshot](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/003c2454a49370802469ed64320fe26f160b4da61215578a/original-group-expanded.png).
[Port screenshot](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/003c2454a49370802469ed64320fe26f160b4da61215578a/s014-group-expanded.png).
