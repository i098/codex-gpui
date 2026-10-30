# Flow parity: original vs port

This document gives the flow status of the port for each screen of `docs/inventory.md` §14 and §15, plus the app-level shortcuts and menu bar of §12 and §13.
The harness is `crates/app/tests/flows.rs`.
It builds the real `app::Root` on a replayed app-server (`Bridge::replay` on `fixtures/readback.jsonl`, plus `fixtures/sessions/greeting.jsonl` for turns) in a headless window with the `textsys` text system and the Metal headless renderer.
It clicks at the original's element bounds (`refs/<screen>/dark@1280x832.spec.json`), types keys, dispatches the menu bar's actions, and then checks the route, rail selection, sidebar, open thread, composer text and focus, popovers (repainted pixels) and theme.
The fixtures do not record some methods (`thread/archive`, `thread/fork`, `thread/resume`, `thread/revert`, `config/batchWrite`, `config/value/write`) or any approval request. For these, a flow adds crafted lines to the replay script (`exchange`): the replay answers such a request by its method, and the flow checks the state the answer leads to. The headless window also takes live resizes (`HeadlessAppContext::simulate_window_resize`).

Run it on the Mac: `tools/mac.sh -l flows 'cargo test -p app'`.
To run some flows only, add a name filter: `cargo test -p app --test flows -- settings`.
On shared Linux hosts, run `RUST_TEST_THREADS=1 cargo test --workspace` to prevent parallel fixture processes from exhausting host resources.

The shared macOS test setup installs an exit-on-panic hook before GPUI starts.
A panic prints its message and exits with code 101 instead of opening a system crash dialog.
Check `~/Library/Logs/DiagnosticReports` before and after each Mac run.
Stop and fix the cause if a new report appears.

## Status values

- **PASS (flow)**: the named flow or slice test in `crates/app/tests/` passes.
- **FAIL (why)**: the port behaves differently, or no flow can reach the row yet (the reason tells which).
- **NOT WIRED (owner)**: the port has no handler for the row; the owner file must add it.
- **BOUNDARY (original)**: the accepted original fixture does not expose the action or screen.

The "Original" column gives the behavior from the inventory.
Section 14.7 also uses owned, offline captures of the original renderer.
These captures do not use a live account or change a normal profile.

## Historical totals before the Settings action work

This table preserves the earlier audit totals; section 14.7 records the later Settings work.

| Screen | Rows | PASS | FAIL | NOT WIRED |
|---|---|---|---|---|
| App shell: shortcuts, menu bar, window | 20 | 13 | 3 | 4 |
| 14.1 Left rail | 12 | 4 | 1 | 7 |
| 14.2 Sidebar | 23 | 12 | 4 | 7 |
| 14.3 Home | 21 | 10 | 3 | 8 |
| 14.4 Thread view | 20 | 15 | 2 | 3 |
| 14.5 Composer | 20 | 6 | 5 | 9 |
| 14.6 Diff view | 20 | 14 | 5 | 1 |
| 14.7 Settings, login and onboarding | 20 | 7 | 8 | 5 |
| 15.1 Automations | 16 | 2 | 2 | 12 |
| 15.2 Background terminal | 10 | 10 | 0 | 0 |
| 15.3 Mobile setup | 10 | 10 | 0 | 0 |
| **All** | **192** | **99** | **33** | **60** |

Before round 5 (F2Flows): 57 PASS, 33 FAIL, 102 NOT WIRED. Before round 4 (F1Flows): 42 PASS, 25 FAIL, 125 NOT WIRED.

The historical round 5 run reported 76 flows passed.
The P7 strict-gate run checks 73 flows: 66 pass and seven fail.
Six failures are strict Root parity failures; `worked_for_expands` also reports zero pixels repainted.

The `scroll_*` flows send wheel events (pixel deltas, a trackpad flick with its momentum events, and wheel lines) to the sidebar sections, the thread, the Changes panel, the Settings nav and page, and the Scheduled page.
At 1280x320, the largest offset of the sidebar sections is 82 px, the same as in the original.
This value was recorded through `tools/route1` with a copy of the fixture `CODEX_HOME`.
The other panes must repaint on the first wheel, stay still past the end, and show the start again.
Div scroll limits use raw layout sizes rounded to device pixels; painting keeps device-snapped bounds.

## Parity of the states the flows reach

Each `parity_*` flow enters the real Root with mouse or key input, then checks its route and selected side panel.
The Sidebar flow also checks that the command menu opens; Composer checks its typed draft and open menu.
Only then does the flow compare its captured image with the original reference through `parity::diff_ref`.
The gate requires zero differing device pixels outside text boxes and at most 0.5% inside text boxes.
`Fx::parity` returns an error on a mismatch.
Each screen still runs both themes and sizes before it returns its combined failure.
The flow executable exits 1 if any flow fails.
There is no report-only or optional mode.

`Fx::reference_shot` parks the pointer at `(round(width * 0.6), 6)` and makes the captured composer and search input carets transparent.
It keeps keyboard focus, selection, and editor input intact, then restores normal caret drawing.
The fixture clock uses the original `2026-10-15T17:00:00Z` base and `-07:00` offset, plus monotonic elapsed time from test startup.
The original preload also advances from its base; the exact historical screenshot instant is not retained.
Normal Root clocks remain live, and the separate static screen clocks keep their fixed semantics.

Run all nine Root checks with `tools/mac.sh -l root-gate 'cargo test -p app --test flows -- parity_'`.
Keep `cargo run -p parity -- --gate` as the separate static screen gate.
Its 36 passing comparisons do not prove Root parity.

The capture-preparation Mac run checks dark and light at 1280x832 and 1440x900.
Each cell below gives `outside count / text percent`.
Composer passes all four comparisons; five failing screens remain tracked in [the Root mismatch issue](https://github.com/i098/codex-gpui/issues/151).

| Screen and input entry | Dark 1280x832 | Dark 1440x900 | Light 1280x832 | Light 1440x900 |
|---|---|---|---|---|
| Home: launch | 0 / 0.377% | 0 / 0.377% | 0 / 0.098% | 0 / 0.098% |
| Sidebar: Search, type `math` | 95831 / 12.250% | 102522 / 9.995% | 128199 / 13.491% | 137557 / 10.381% |
| Composer: type `Refactor src/math.py`, click model pill | 0 / 0.332% | 0 / 0.332% | 0 / 0.250% | 0 / 0.250% |
| Thread: click the subtract row | 3695 / 0.795% | 3695 / 0.786% | 3698 / 0.797% | 3698 / 0.788% |
| Diff: subtract, `⌥⌘B`, collapse file | 3695 / 0.694% | 3695 / 0.795% | 3698 / 0.617% | 3698 / 0.716% |
| Settings: `⌘,` | 0 / 0.088% | 0 / 0.080% | 0 / 0.117% | 0 / 0.107% |
| Automations: click Scheduled | 40039 / 27.119% | 40037 / 27.119% | 40077 / 27.176% | 40053 / 27.176% |
| Terminal: subtract, recorded command events, summary command row | 3830 / 0.727% | 3830 / 0.801% | 3833 / 0.720% | 3833 / 0.791% |
| Mobile: avatar, Set up mobile | 0 / 0.468% | 0 / 0.468% | 0 / 0.467% | 0 / 0.467% |

The old profile helper opens Keyboard shortcuts over Home instead of Mobile.
The corrected helper uses the proved Mobile entry; the old failed captures remain retained in the acceptance evidence.
The large-window Composer click uses the original reference's centered-column and bottom offsets.
The prior large-window clicks miss the menu; the corrected clicks and reference capture preparation pass the strict Composer gate.

Pointer, clock, and composer-caret preparation now follow the reference; other fixture, host-data, and drawing differences remain unresolved.
None of these remaining causes waives a failure.
Keep S026's recorded command groups and settled local search behavior; the independent ChatGPT search can still be pending.
Keep S009's File viewer behavior; its recorded test replaces the obsolete file-link-to-Changes assertion.
The S009, S026, F3b, and S034b recorded tests pass with the strict shared helper.
Formatting and `cargo clippy --workspace --all-targets -- -D warnings` pass after integrating current main.
The initial broad clippy run failed at `ms6a.rs:168`; the retained log preserves that result.

Before this repair, an isolated smoke check sends a retained Composer capture through `parity_image` and returns exit 1 with 11016 outside pixels.
All 36 unchanged original images pass the same helper with zero differences.
Replaying the old Mobile click returns exit 1 for all four route checks before any pixel comparison.
These checks prove the guard; they do not replace the failing Root captures.

### Home rail selection on thread routes

Home stays selected on its Thread, Changes, and Background terminal routes.
`rail_thread_family` enters those states through Root input and compares the Home rail with the original pixels.
It checks both themes and sizes without changing focus, pointer position, clocks, or fixtures.
Thread and Changes check the full Home button at `[8,52,36,36]`.
Terminal checks `[8,66,36,22]`, below the reference's separate unread dot.
This selection check does not waive any full-Root pixel difference.
The strict full-screen gate still checks the original Terminal unread state and every outside-text pixel.

Run the selection regression with `cargo test -p app --test flows -- rail_thread_family`.

The new regression fails all 12 cases before the rail correction and matches all 12 afterward.
The separate static gate remains 36/36.
At integrated source `60283d8` on main `9079aa0`, the unchanged full-Root gate returns exit 1: 32 comparisons pass and four fail.
Every screen except Sidebar passes all four variants.
Sidebar still differs from its older pending-state reference; correct S026 settled search behavior remains unchanged.
This result includes the upstream repairs; the consumer change remains limited to Home rail selection.
Cells give `outside count / text percent`; the owned empty `CODEX_HOME` excludes ambient automation rows.

| Remaining full-Root failure | Dark 1280x832 | Dark 1440x900 | Light 1280x832 | Light 1440x900 |
|---|---|---|---|---|
| Sidebar | 95831 / 12.250% | 102522 / 9.995% | 128199 / 13.491% | 137557 / 10.381% |

### Sidebar multi-source search

The fixed Sidebar reference shows a completed local content search while the separate ChatGPT search remains pending.
The Root now consumes both sources through the existing app-server and `CloudHost`.
The spinner remains while either source is pending.
Cloud failures retain local results and expose Retry by pointer or keyboard.
The cloud source uses the original three retries, after one, two, and four seconds.
Query generations reject stale results, and a closed menu sends no later retry.

Local results open the native thread route.
Cloud conversations and dot messages open their encoded canonical ChatGPT pages through the existing platform URL action.
This repair does not add a separate ChatGPT application mode.

`sidebar-math-search.jsonl` retains the real local answer from the recovered original rollout files and history database.
`sidebar-cloud-search.json` retains the original owned fixture server's HTTP response.
That response triggers the original pending retry interval without fabricated search results or production pending overrides.
The search input uses the same transparent-caret capture setting as the composer.
The references, text masks, and strict assertions remain unchanged.

The focused native run passes all four Sidebar comparisons:

| Case | Outside-text pixels | Text difference |
|---|---:|---:|
| Dark 1280x832 | 0 | 0.154% |
| Dark 1440x900 | 0 | 0.152% |
| Light 1280x832 | 0 | 0.153% |
| Light 1440x900 | 0 | 0.152% |

Run `cargo test -p app --test flows -- parity_sidebar` on the Mac for the unchanged pixel gate.
Run `cargo test -p app --test sidebar_search` for source completion, failure, retry, selection, and canonical navigation checks.
The S026 control creates its own Git repository; it no longer depends on the shared temporary project surviving system cleanup.
The repair is tracked in [the Sidebar issue](https://github.com/i098/codex-gpui/issues/273).


## App shell: shortcuts, menu bar, window (§2.2, §12, §13)

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | `⌘,` | shortcut | Opens Settings (`settings`). | Route Settings, settings shell. | PASS (`cmd_comma_opens_settings`) |
| 2 | `⌘N`, `⌘⇧O` | shortcut | New chat (`newTask`). | Home, draft cleared, composer focused. | PASS (`cmd_n_new_chat`) |
| 3 | `⌘⇧S`, `⌘B` | shortcut | Toggle sidebar. | Shell sidebar toggles. | PASS (`sidebar_toggle_keys`) |
| 4 | `⌘[`, `⌘]` | shortcut | Back, Forward (`navigateBack`, `navigateForward`). | Route history over Home, threads, Settings. | PASS (`back_forward_keys`) |
| 5 | `⌘⇧[`, `⌘⇧]`, `⌘⌥←`, `⌘⌥→` | shortcut | Previous / next chat. | Previous / next Recents row; no wrap. | PASS (`previous_next_chat`) |
| 6 | Number shortcuts | shortcut | Swap primary-number shortcuts between chats and tabs. | The saved number target selects Command-number or Control-number for chats; the other chord selects panel tabs. | PASS (`keyboard_shortcuts`; Linux) |
| 7 | `⌘W` | shortcut | Close tab, else close window. | Closes the thread route's selected side panel tab, else the window. | PASS (`cmd_w_closes_window`, `s029::panel_tabs_replay`) |
| 8 | Menu bar: Codex ▸ Settings…, File ▸ New Chat, View ▸ Toggle Sidebar, Toggle Changes Panel, Previous Chat, Next Chat, Back, Forward | click | Same commands as the keys. | `app::menus()` items dispatch their actions. | PASS (`menu_bar_actions`, `changes_panel_keys`) |
| 9 | System appearance and Settings ▸ Appearance ▸ Theme | click | Light / Dark / System applies at once. | `State.theme` (`ThemeSetting`) is applied on each appearance change and by `Root::set_theme`. | PASS (`theme_switch`) |
| 10 | `⌘M`, Window ▸ Minimize, Zoom | shortcut | Minimize, zoom. | The real AppKit window minimizes, restores without activation, and toggles zoom; the check reads its native state. | PASS (`native_lifecycle`, macOS key and menu runs) |
| 11 | `⌘Q`, `⌘H`, `⌥⌘H`, Show All | shortcut | Quit, hide, hide others, show all. | Hide and Quit run live on the owned app. Hide Others and Show All reach their AppKit selectors through a process-local test double; other applications remain untouched. | PASS (`native_lifecycle`; Hide Others/Show All are call-level checks only) |
| 12 | Window minimum size | resize | `getPrimaryMinimumSize()` 480x600. | `windows::Kind::Primary` sets `window_min_size`; the native check reads `contentMinSize` as 480×600 and verifies a constrained resize. | PASS (`native_lifecycle`, macOS) |
| 13 | Live window resize across 768 px | resize | The sidebar hides below 768 px and comes back above it. | `Root::fit` on `observe_window_bounds`: live resizes to 767, 1280, 600 and 1440 px hide and show the sidebar; a sidebar the user closed stays closed. | PASS (`resize_live`, `resize_three_widths`) |
| 14 | `⌘K`, `⌘⇧P` | shortcut | Command menu. | `CommandMenu` over live Recents, focused; Escape or a click outside closes it; Enter or a click runs the row (chat, `New chat`, `Manage scheduled tasks`, settings pages). | PASS (`command_menu_keys`, `command_menu_settings_page`) |
| 15 | `⌘F` | shortcut | Find in chat. | `FindInThread` opens the thread find bar (`Thread::open_find`) in the split layout; in the full layout the Changes panel takes it. | PASS (`shortcut_chat_actions::pin_rename_and_find_shortcuts_act_on_the_open_chat`, `workspace_layout`) |
| 16 | `⌥⌘B` | shortcut | Toggle Changes panel. | The thread route's side panel (`ui::screens::diff::panel::with_side_tabs`) shows the last turn's diff in a `Changes` tab beside the other open tabs; the tab's close button closes it. | PASS (`changes_panel_keys`) |
| 17 | `⌘J` | shortcut | Toggle bottom panel. | Opens or hides a real PTY terminal without discarding its process or buffer. | PASS (`shortcut_actions`; native) |
| 18 | `⌘⇧N`, `⌘⌥O` | shortcut | Temporary chat, standalone chat. | Temporary chats use ephemeral app-server threads. Standalone chats allocate and record a projectless workspace. | PASS (`shortcut_chat_modes`; Linux) |
| 19 | `⌘⌥1`…`⌘⌥6` | shortcut | Go to recent chat N (`recentThread1..6`: the chats by last update, newest first). | `GoToRecentChat`: the Nth Recents thread by `updatedAt`; past the end does nothing. | PASS (`recent_chat_keys`) |
| 20 | `⌘+`, `⌘-`, `⌘0` | shortcut | Zoom in, zoom out, actual size. | Scales layout, paint, pointer input, and IME bounds. Saves the zoom and shows its timed control banner. | PASS (`keyboard_shortcuts`; Linux); native pixels pending |
| 21 | Startup loader | loading | 56 px blossom centered on the window background until `initialize` answers. | Shows until the first app-server answer; then the app's own screens take over. | PASS (`error_shell::loader_shows_until_the_app_server_answers`) |
| 22 | App-server death | error | Crash page: `ChatGPT hit a snag`, `Something went wrong. Try again to continue`, Try again, the fallen-pet illustration. | `Event::Closed`, or an app-server that cannot start at launch, shows the recovery page; Try again reconnects through `windows::connect` and keeps the Home and thread composer drafts. | PASS (`error_shell::crash_page_shows_when_the_app_server_dies`, `error_shell::crash_page_shows_when_the_app_server_cannot_start`, `error_shell::try_again_respawns_the_app_server`) |

F1d covers SH-020 and SH-135 with `refs/interactions/f1d.json` and `cargo test -p app --test f1d`.
The original Mac recording proves a native 1280×832 viewport at scale 2.
Shift-Tab from the Home rail button reveals `Skip to content`.
Enter or a click moves focus to the main content.
The next Tab reaches `Choose project`.
The shared focus handles retain the main container outside the normal Tab stops.
Command titles open their full-label tooltip after 2000 ms only when the measured title exceeds its viewport.
The tooltip wraps within 320 px and closes when the pointer leaves.
Fitting command titles and the profile avatar have no tooltip.
The original sidebar hover card also opens for fitting titles, so F1d does not replace that card.
Functional acceptance runs `skip_link_keyboard_and_pointer_paths_reach_same_content` and `full_label_tooltip_requires_overflow` in `f1d`.
The shared `keyboard_tooltip_ownership_and_dismissal_follow_focus` check remains in `f1a`, with its stronger native paint and blur coverage.
These checks pass on the real headless Root and native tooltip surface.
Keyboard and pointer activation reach the same main region and the same next content control without changing the route or sending app-server requests.
Every original visual assertion remains in the separate recorded replay.
The initial 15-input shared replay, F1a/F1b/F1c checks, and all 36 strict parity cases passed with unchanged metrics.
The extended record also checks the already-recorded project tooltip, its shortcut, and its bounds after both skip-link activation methods.
`Tooltip::focus_handle` reuses the control's native focus handle and the shared provider.
Keyboard focus opens immediately; blur and Escape close the tooltip without moving focus.
The lifecycle check rejects pointer-focus opening, delayed reopening after Escape, and multiple visible tooltip owners.
F1d retains the original `(455.5, 695)` probes at steps 3 and 7.
Functional readiness is reported independently from the remaining Home visual parity.
The original Home-mask, border-box, and keyboard-tooltip caller differences remain recorded; their checks are not removed or relaxed.
The combined native check on `96995c3` passes S033 and both functional F1d paths with all 880 source hashes verified.
The shared guard allows main-content ancestors and the exact Thread transcript focus while keeping popup input separate.
This includes blank-Home typing, Thread insertion and caret edits, and popup isolation.
This result does not clear the separate appearance gate or the retained Home visual failures.

### Account logout (RS-036, SH-088)

The command menu, profile menu, and Codex ▸ Log Out open the same confirmation dialog.
Cancel and Escape send no request.
Confirm sends `account/logout`.
Success shows Login and clears the open chat, Recents, panels, and navigation history.
Failure shows an error and keeps the chat and draft usable.
Signed-out shortcuts cannot reopen the old account view.
Logout also clears attachments from both composers and ignores unfinished attachment work from the previous session.
After sign-in, Recents can load the remaining pages.

Run the focused native replay with `cargo test -p app --test account_logout -- --test-threads=1`.
It uses replayed success and failure responses and an isolated local host.
This proves headless native behavior, not live installed-window acceptance.

## 14.1 Left rail

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Home | click | Navigates to `/`; reselect resets Home. | `NewChat`: route Home, draft cleared, composer focused. | PASS (`rail_home_from_thread`, `rail_home_reselect_resets`) |
| 2 | Space | click | Navigates to `/space`. | No handler. | NOT WIRED (outside v1 scope (no screen)) |
| 3 | Scheduled | click | Navigates to `/scheduled` (Automations); unread dot. | `OpenScheduled`: `Route::Scheduled`, rail Scheduled selected. Settled local unread runs set the dot; opening the run chat clears its read state. | PASS (`rail_scheduled`, `parity_automations`; native unread proof: `automation_run_now::persisted_run_unread_survives_navigation_and_clears_on_open`) |
| 4 | Plugins | click | Navigates to `/plugins`. | `OpenPlugins`: `Route::Plugins("/plugins")`, rail Plugins selected, the directory from `plugin/list`; a plugin opens `/plugins/:id` with `plugin/read`; Install and Uninstall send `plugin/install` and `plugin/uninstall` with the original's remote params (`remotePluginId`) and reload the catalog. | PASS (`plugins_rail::rail_plugins_lists_reads_installs_and_uninstalls`; live: the same requests against a signed-in macOS `codex app-server` 0.164.0-alpha.11.1) |
| 5 | Explore `…` | click | Opens the Explore menu; blue dot. | Dot shows. A click opens a menu right of the rail item with the destinations the port has (`Security` when available, else `No more destinations`); a second click or a click outside closes it. | PASS (`rail_explore_menu`); `Customize sidebar` and MCP destinations: BOUNDARY (no sidebar-customization store; Explore row 5) |
| 6 | Code Review | click | Navigates to `/pull-requests`. | `OpenPullRequests`: `Route::PullRequests(None)` with the Code Review page; no rail item is selected, as in `refs/pull-requests/open-all`; Back returns Home. The page reads GitHub through the original's `vscode://codex/gh-cli-status`, `gh-current-user`, `gh-pr-search`, `gh-pr-detail`, `gh-pr-patch` and `gh-pr-media` handlers, which run `gh` through app-server `process/spawn`. | PASS (`pull_requests::rail_code_review_opens_the_page_and_back_returns_home`; page states: the other `crates/app/tests/pull_requests.rs` tests) |
| 7 | Sites | click | Navigates to `/sites`. | `OpenSites`: `Route::Sites`, rail Sites selected. The page reads the owned list from the ChatGPT backend `GET /websites?limit=20` (the original's `appgen-sites-query`) and shows `Loading sites`, `Unable to load sites` with `Retry`, `No sites yet`, or one row per site (title, last edit, `Not published`). | PASS (`rail_sites::rail_sites`; `sites::tests`); tabs, search, pins, paging, create, publish, share and site detail: BOUNDARY (not built) |
| 8 | Profile avatar | click | Opens the profile menu. | Menu above the avatar: `Settings`, `Log out`, `Help` ▸ `Set up mobile`; a click outside or on the avatar closes it. | PASS (`profile_menu_settings`, `profile_menu_dismiss`, `profile_menu_mobile`; logout: `account_logout`) |
| 9 | Rail item hover | hover | Tooltip on the right side with label and shortcut. | Hover fill shows (4716 px repainted); a tooltip with the item's label opens 200 ms after the pointer enters (at once when another was open), 2 px right of the item; `Explore` and the avatar have none; it closes on leave, press or Escape. | PASS (`rail_hover_fill`; tooltip: `crates/app/tests/f1a.rs` against `refs/interactions/f1a.json`) |
| 10 | Drag a rail item | click | Reorders pinned items. | Dragging a pinned destination (`Code Review`, `Sites`) onto another moves it to that place (the original's `arrayMove` on drop), and the drag does not open it; the order lasts for the session. | PASS (`rail_drag_reorders_pinned`; the persisted `sidebar-customization-snapshots-v2` `order` write stays separate work) |
| 11 | Focus ring | focus ring | 2 px ring on the 36×36 button. | Every rail item and the avatar have a tab stop and draw the shared 2 px keyboard ring (`focus::ring` in `Shell::rail`). | PASS (`crates/ui/src/focus.rs` ring; `rail_focus_ring`) |
| 12 | Resize | resize | At 1280, 767 and 600 px the rail stays 52 px. | The rail stays and rail Home answers at all 3 widths. | PASS (`resize_three_widths`) |

Home reselect clears the draft and keeps the current focus only when the main view is `HomeMain`.
Home rail selection can identify the Thread route family; it does not identify the current Home view.
Thread-to-Home navigation keeps the Home composer focus set by `Root::new_chat`.

## 14.2 Sidebar

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Sidebar visible | entry | Rail 52 px plus panel 288 px at 1280 px. | Open at launch; parity: see `parity_home`. | PASS (`launch_home`, `parity_home`) |
| 2 | `Codex ⌄` mode switch | click | Menu: ChatGPT, Codex, Work. | Opens the menu under the trigger (`ChatGPT`, `Codex` `Build, debug, and ship`, `Work` `Create, learn, and explore`, with a check on the current mode); a pick, Enter, or Escape closes it, a click elsewhere closes it. A pick records the mode (default Codex); the ChatGPT and Work surfaces do not exist, so the sidebar and wordmark stay Codex. | PASS (`mode_switch_menu_picks_with_keys_and_click_outside_closes`; the original popup cannot be captured) |
| 3 | `⌃1` `⌃2` `⌃3` | shortcut | Switch mode. | Commands `switchToMode1..3` of the recorded keyboard table: `⌃1` ChatGPT, `⌃2` Work, `⌃3` Codex, through the keymap module; the menu rows show these keys. | FAIL (`control_digits_switch_mode` passes, but the ChatGPT and Work surfaces do not exist, so a switch changes no screen) |
| 4 | Bell `View activity` | click | Toggles the activity view. | `ToggleActivity` selects the existing Home sidebar's priority view when shared access permits it. | PASS (shared access tests and native main-control comparison) |
| 5 | `⌘⌥U` | shortcut | Toggles the activity view. | Uses the configured activity shortcut and shared access state; keeps a Thread route and opens a hidden sidebar. | PASS (`priority_activity`, native main-control comparison) |
| 6 | Search | click | Opens the command menu in chat-search mode. | `OpenCommandMenu`: the command menu over live Recents. | PASS (`command_menu_search_click`, `parity_sidebar`) |
| 7 | `⌘K` | shortcut | Opens the command menu. | Same as shell row 14. | PASS (`command_menu_keys`) |
| 8 | `Your dot` row | click | Opens the dot (orbit) thread. | Opens `https://chatgpt.com/dots/home` through the platform URL action; the original navigates in-app to the same `/dots/home` route, and the port has no dot screen. | PASS (`your_dot_row`) |
| 9 | `Projects` section toggle | click | Collapses or expands; chevron rotates. | A header click collapses or expands the section; the chevron shows on hover or while collapsed and turns -90° when collapsed (`crates/ui/src/screens/sidebar.rs`). | PASS (`sidebar_section_collapse`) |
| 10 | `Recents` section toggle | click | Collapses or expands. | Same; a collapsed Recents row no longer opens. | PASS (`sidebar_section_collapse`) |
| 11 | Thread row | click | Opens `/local/:conversationId`. | `OpenThread` → route Thread, `thread/read`, row selected. | PASS (`thread_row_opens_thread`) |
| 12 | Thread row hover | hover | Row background; Archive / Pin actions appear. | The row reveals its actions. Buttons use the shared 150 ms hover transition. | PASS (`thread_row_hover`, `crates/app/tests/s001.rs`) |
| 13 | Thread row context menu | click | Pin, Rename, Archive, Mark as unread, Open in new window, Delete. | Archive removes the row without opening the chat. Pin and Unpin move the chat into or out of `Pinned` through `thread/section/move`, with no toast; a refused move puts the row back silently. The Unpin icon hides at rest like Pin and Archive. | PASS (`thread_row_menu`, `s002`; Archive and Unpin: `crates/app/tests/s001.rs`; Pin: `crates/app/tests/s001b.rs`; Share closes the menu only: the port has no shared-snapshot client) |
| 14 | `⌘1`…`⌘9` | shortcut | Go to chat N. | Opens Recents row N; `⌘9` past the end does nothing. | PASS (`cmd_digit_goes_to_chat`) |
| 15 | `⌘⇧[` / `⌘⇧]` | shortcut | Previous / next chat. | Previous / next Recents row. | PASS (`previous_next_chat`, `menu_bar_actions`) |
| 16 | `⌘⇧S` / `⌘B` | shortcut | Toggle sidebar. | Toggles. | PASS (`sidebar_toggle_keys`) |
| 17 | Titlebar `Hide sidebar` | click | Collapses the left panel over a 300 ms spring (main surface slides, sidebar rows fade and shift 16 px); label changes to `Show sidebar`; instant under reduced motion. | The main surface and the sidebar content follow `motion::spring_300` from the toggle's progress; the recorded tooltip stays `Toggle sidebar ⇧⌘S` in both states. | PASS (`sidebar_toggle_titlebar`; `crates/app/tests/s004.rs`; spring frames: `crates/app/tests/ms2.rs` against `refs/interactions/ms2.json`) |
| 18 | Row focus | focus ring | 2 px ring, offset -2 px. | Tab moves through rows and their Pin and Archive actions; focus reveals the actions. | PASS (`crates/app/tests/f1b.rs`) |
| 19 | Empty lists | empty | `No projects`, `No chats`; activity `Nothing needs attention`. | Keeps the normal empty labels and renders the priority empty state. | PASS (S003 and native main-control comparison) |
| 20 | Lists loading | loading | Activity: `Checking unread chats…`. | Shared `ListState` renders the priority loading message. | PASS (native loading-state captures in both themes and sizes) |
| 21 | Load error | error | `Projects couldn't be loaded` + `Retry`; `Could not load chats`. | Keeps Home available after a failed initial list read; Retry restores the list. | PASS (`startup_chat_failure_keeps_home_available_and_retry_restores_chats`, `project_load_failure_retries_the_owned_state_file`) |
| 22 | Sidebar drag-resize | resize | The recorded edge clamps to 290…520 px; dragging below 145 px collapses it. | The 16 px handle resizes the sidebar and main surface, retains the width on toggle, and saves it on release. | PASS (`crates/app/tests/s004.rs`, `refs/interactions/s004.json`) |
| 23 | Resize | resize | Below 768 px the panel is hidden; it opens as an overlay. | Hidden at 767 and 600 px, shown at 1280 px; the user's choice is kept. `⌘B` below the breakpoint opens it as a shadowed overlay over the main surface, which does not move (`Shell::overlay`). | PASS (`resize_three_widths`) |

The original `isPriorityFilterEnabled` requires feature `4039078146` and access status `allowed` or `loading`.
The native provider reads that feature from Statsig and checks the current account's plan and workspace settings.
The bell, command menu, shortcut settings, and action handler share the resulting availability.
Denied access, failed access reads, account changes, and logout close an active activity session.
Same-account refreshes keep an active session.
Transient host failures use the original three-retry policy.
ACP keeps its existing local activity view without account-backed requests.
The native comparison covers 35 full-window states, including both themes at 1280×832 and 1440×900.
All captured states match main `394f93f261e0fc750fb950a600f9353e5015544e` with zero changed pixels.
The four retained original comparisons preserve main's existing outside-text differences, with no differences inside text rectangles.
No fresh original capture service was available; popup captures use the main control and the original item order, labels, and shortcuts.


S001 replays `refs/interactions/s001.json` with `cargo test -p app --test s001` on macOS.
The recording uses the original renderer with isolated host and app-server fixtures.
The replay checks Unpin, Archive, the Home route, outgoing requests, button timing, and the recorded hover blend.
The native host translates `pinnedThreads.set(false)` to `thread/section/move` with a null section.
The original shows no toast for this Unpin action.
S001b replays `refs/interactions/s001b.json` with `cargo test -p app --test s001b` on macOS: the Pin click moves the row under `Pinned` at once, sends `thread/section/move` with the fixed Pinned section, shows no toast, hides the Unpin icon at rest, and a refused move puts the row back without a toast.
Running-chat and scheduled-task archive confirmation still need a dialog extension outside this six-file slice.

S031 replays `refs/interactions/s031.json` with `cargo test -p app --test s031` on macOS.
The original recording uses real archive and unarchive replies from an isolated fixture home.
View opens the standalone `/archive` route without the Settings sidebar.
The original uses a memory router, so its document pathname stays `/` while its route context records `/archive`.
The port opens `Route::Archive` and reads the local archived chat list.
The replay checks Undo, View, the exact requests, the hidden sidebar, Back navigation, and the unchanged Settings entry.
The shared replay runs the same dedicated adapter, so it retains these state and request assertions.
The destination loads every cursor page before showing the list.
Successful Undo also removes the restored row from an open archive page.
Two review regressions check cursor traversal and Undo after another toast opens View.
Both regressions use real app-server replies from a fresh isolated fixture home.
F3a still checks the toast layout, lifetime, hover pause, swipe, and Undo.
The shared Toaster paints its Skia decoration, glyphs, and shadow into one viewport-origin compositor layer.
This preserves low-alpha shadow pixels when the layer blends into the page.
Archive search, filters, and management actions are separate inventory rows outside AM-101.

The S004 recording uses the original renderer through the Linux route1 stack in an isolated Chrome context.
Its twelve actions check resize limits, collapse, restored width, titlebar toggles, tooltips, boundary pixels, and no app-server requests.
The current original tooltip is `Toggle sidebar ⇧⌘S`, not the inventory's `⌘B` label.
The static parity gate retains all 36 baseline counts.
The repair records the original 300 ms settled-toggle delay and requests native presentation after advancing the virtual clock.
Clock advancement alone leaves the step-6 surface closed.
The repaired S004 replay passes all twelve actions with the original pixel, width, tooltip, and request assertions.
Native frame requests bypass the inactive window's wall-clock throttle while preserving the recorded virtual clock.
The unchanged MS2 replay passes all spring, row-shift, opacity, and reduced-motion assertions.
The scoped issue is [S004 acceptance samples a stale sidebar frame](https://github.com/i098/codex-gpui/issues/149).

F1b replays all 30 original steps with `cargo test -p app --test f1b` on macOS.
Clicking a section header keeps pointer focus; Enter enables its keyboard ring without moving focus.
The section header retains its original 202×25 px target inside the 272×30 px layout row at the recorded width.
The S003 header replacement enlarged that target, so the ring moved away from the recorded border pixels.
The repair restores the shared target and tab group for Projects, Recents, and Pinned without changing the ring painter or input handling.
The existing replay checks the original colors, row actions, tooltips, canceled row click, Home route, and absence of app-server writes.

## 14.3 Home

The retained native Home input capture reaches `Choose project` and `This computer` with Tab and Shift-Tab.
Pointer movement keeps the keyboard ring; a pointer press clears it.
Tab and Shift-Tab scroll an overflowing utility bar to the focused control.
`Choose project` keeps the existing native folder picker; `This computer` still has no location-menu handler.
The original `⌥⇧⌘O` shortcut and `⌘O` open that same picker.
Home derives live fades from the scroll range and captures the finished-animation mask for static references.
Both utility icons use the runtime path so the scroll masks affect their pixels.
The capture regression checks live-state restoration, another window, a closed-window error, and the reduced-motion preference.
The focused project-control crop now matches the original, including the tooltip shadow over the ring.
The original recordings, masks and strict gates stay unchanged; remaining tooltip and rail differences are tracked separately.

New chat adds a Back entry even when Home is already open.

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Launch to `/` | entry | Hero, announcement slot and composer. | Route Home, rail Home selected, composer focused, Recents loaded. | PASS (`launch_home`, `parity_home`) |
| 2 | `New chat` row | click | Navigates to `/` and clears the draft. | Same. | PASS (`new_chat_row`) |
| 3 | `⌘N` / `⌘⇧O` | shortcut | New chat on Home, composer focused. | Same. | PASS (`cmd_n_new_chat`) |
| 4 | Hero project selector | click | Opens the project menu. | Shows the selected folder and opens the native folder picker. | Native local-folder flow; no project dropdown. |
| 5 | Beacon `Start Voice` | click | Starts a realtime voice chat. | `HomeMain::beacon` draws `Try ChatGPT Voice` with `Start Voice`; the button raises `StartVoiceChat`, which shows a notice. | PASS (`home_beacon`); the `/beacons/home` feed that turns the beacon on and the realtime audio session: BOUNDARY (Home row 5) |
| 6 | Beacon dismiss `×` | click | Hides the banner. | Hides the banner for the session; persistence needs the `/beacons/home` feed. | PASS (`home_beacon`); persistence BOUNDARY (Home row 6) |
| 7 | `Choose project` | click | Project dropdown. | Opens the native folder picker and selects a local project for new chats. | Native local-folder flow (`project_picker`); no project dropdown. |
| 8 | `Work in` / `This computer` | click | Location menu. | The chip opens a menu above it: `This computer` (checked) and `New worktree`, `Remote`, `Cloud` listed disabled; Enter or Space on the chip toggles it, Escape, a pick, or a click elsewhere closes it. Choosing another location is not wired. | PASS (`work_in_menu_toggles_and_dismisses`; the original popup cannot be captured) |
| 9 | Editor `Do anything` | focus | Caret shows. | Click focuses the editor. | PASS (`editor_click_focuses`) |
| 10 | `+` Add files and more | click | Add menu (`composer.atMentionList.contextActions`): `Files and folders`, `Attach appshot` (disabled), `Work in a project`, `Goal`, `Plan mode`, `Sketch`; tooltip `Add files and more` with the `@` hint. | Opens the `@` list without inserting `@`, with the recorded rows, order, disabled appshot row, selection and tooltip; a second click closes it. `Files and folders` opens the file picker. `Plan mode` turns plan mode on or off, as `/plan-mode` does: the footer then shows the `Plan` chip (tooltip `Create a plan`; a click turns it off), the row reads `Turn plan mode off`, and every `turn/start` sends `collaborationMode` `plan` or `default` with the model and effort. A resumed chat keeps the mode that `thread/resume` reports. `Work in a project`, `Goal` and `Sketch` do nothing, as their `/` commands do: the port has no project submenu from the list, goal mode or sketch canvas. | PARTIAL (`s018::add_menu_replay`, `s018::add_tooltip_replay`, `plan_mode`); `Work in a project`, `Goal` and `Sketch` are NOT WIRED (`composer/mention.rs` `activate_mention`) |
| 11 | `Custom` permissions | click | Permissions dropdown. | Opens; Escape closes. | PASS (`permissions_popover`) |
| 12 | Model picker | click | Model and effort menu. | Opens; Escape closes. | PASS (`model_picker_popover`) |
| 13 | `Dictate` | click | Starts dictation. | Uses native macOS capture and authenticated `/transcribe`; inserts at the initiating Composer selection. | WIRED; controlled-input proof passes. Live microphone acceptance remains open. |
| 14 | `Start new voice chat` | click | Starts realtime voice. | With an empty draft the button is labelled `Start new voice chat` and raises `StartVoiceChat`, which shows a notice; no turn starts. | PASS (`voice_button_notice`); realtime audio session: BOUNDARY (Home row 14) |
| 15 | Submit with Enter | shortcut | `thread/start`, `turn/start`, navigates to the thread. | Same; the turn streams in; the draft clears; also by clicking Send. | PASS (`submit_enter_starts_thread`, `submit_click_send`) |
| 16 | Hover on every button | hover | Ghost background fades in. | Hover fill on composer buttons (checked on `Dictate`). | PASS (`composer_button_hover`) |
| 17 | Tab through controls | focus ring | 2 px ring. | `Choose project` and `This computer` have tab stops; the native capture proves focus traversal, not menu activation. | NOT WIRED (remaining composer controls) |
| 18 | No projects / no chats | empty | `No projects`, `No chats`, `What should we build?`. | Same with an empty `thread/list`. | PASS (`sidebar_empty_lists`) |
| 19 | Model list loading | loading | `Loading model…`. | Empty `model/list` shows `Loading model…` (the pill repaints). | PASS (`model_list_loading`) |
| 20 | Account load failure | error | An `account/read` error shows Login. A separate `accessInputs` error keeps Coding Home available. | Use the existing Login view for account failures. Keep Home and its composer available for access-inputs failures. | PASS (`account_availability`: 2 native tests; original Login pixel comparison) |
| 21 | Resize | resize | Below 768 px the sidebar is an overlay; below 40rem the composer stacks. | Sidebar hides (row 14.2/23); composer does not stack. | FAIL (`crates/ui/src/composer.rs`: only the 475 px compact footer) |

The shared entry point is `Composer::dictate(window, cx)`.
`cancel_dictation(cx)` cancels capture and pending insertion; `dictation_label()` and `dictation_error()` expose the state.
`finish_dictation(window, cx)` requests transcription without cancelling the session.
`Completion` reports success after transcript processing, failure, or cancellation.
Request text fields use the same dispatcher; secret and numeric fields do not expose Dictate.
The comment editor owner calls this API on its own Composer entity.

Run `cargo test -p app --features test-support --test dictation` and `cargo test -p ui --lib composer::dictation`.
The controlled test sends generated WAV bytes through the production worker, `CloudHost`, and a loopback HTTP fixture into the rendered Composer.
The fixture returns a known transcript; this checks transport and editing, not speech recognition.
The macOS run uses the native renderer without opening a microphone, requesting permission, or activating the app.
Live microphone capture, permission approval, and production transcription require separate acceptance.

A closed event stream reports failure instead of leaving the session active.

`Dispatcher::new` receives the initiating `Window` and `App`; native capture resolves that window's current host.
Changing a window's host cancels only that window's session.
Transcription HTTP requests use a 60-second timeout.
Requests without `timeoutMs` keep their existing timeout behavior.
Typing, IME edits, mentions, and slash commands cancel dictation and emit `Completion::Cancelled`.
Comment owners call `finish_dictation` and wait for `Completion::Succeeded` before saving the inserted transcript.

## 14.4 Thread view

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Open `/local/:conversationId` | entry | `thread/resume`, turns render, scroll to bottom. | `thread/read` with turns (resume on the first send); parity: see `parity_thread`. | PASS (`thread_row_opens_thread`, `parity_thread`) |
| 2 | History loading | loading | Spinner; `Loading task…` for assistive technology. | The loading view remains visible during `thread/read`. The spinner uses the original SVG in its own raster layer and retains its rotation timing. The composer stays unmounted, so hidden bindings cannot send or approve in another chat. | PASS (`thread_recovery`: hidden-composer regression and zero differences over the 3600-pixel spinner region; `native_icon_rotation_preserves_its_raster`) |
| 3 | History failure | error | `Couldn't load messages` + `Retry`. | The squircle warning shares its opaque footer's compositor layer above the original fade. Its Retry tooltip and accessibility label retain the source error. Retry reads the same thread again. Late responses cannot replace another route. | PASS (`thread_recovery`: button-bound retry and route change; complete 123984-pixel warning comparison: zero differences) |
| 4 | Turn render failure | error | `This turn couldn't render`. | A failed turn leaves other turns visible. Try again rebuilds that turn from current data. Its tooltip and accessibility label retain the source error. The squircle card and Retry fill use the original viewport layer; Retry keeps its hover behavior and border-box size. | PASS (`thread_recovery`: Linux render failure and retry without a graphics adapter; original-context component: zero differences over 233208 pixels, hover and restoration checked without a Mac panic) |
| 5 | Streaming agent message | loading | Text grows by deltas; new segments fade over 700 ms; `Working` shimmer. | New glyphs use the recorded adaptive fade. Later deltas do not restart older glyphs. Reduced motion shows the end state immediately. Shimmer belongs to S5. | PASS (`submit_enter_starts_thread`, `stream_fade`) |
| 6 | Reasoning item | click | Toggles `Thought for {elapsed}`. | A reasoning item seen starting and completing in the open thread shows a `Thought for {elapsed}` row; a click shows its summary under it. The protocol carries no item duration, so a loaded history item has no row, as in the original recording (`refs/interactions/s014.json`). | PASS (`live_reasoning_gets_a_toggleable_thought_row`; `s014` keeps history reasoning hidden) |
| 6b | Streaming reasoning, context status and compaction | loading | `Thinking` with the streamed summary; `Compacting context`, `Context compacted`, `Context automatically compacted`, `Compaction interrupted`; the context window donut and its tooltip. | `item/reasoning/summaryPartAdded`, `summaryTextDelta` and `textDelta` grow the live item under `Thinking` (heading stripped) until `Thought for {elapsed}`. Configuration `Hide thinking` (`hide_agent_reasoning`) hides reasoning rows; `Show raw thinking` (`show_raw_agent_reasoning`) shows raw content first. `contextCompaction` items show progress, then the completed or interrupted label; `/compact` marks the next one manual. `thread/tokenUsage/updated` (`last.totalTokens` of `modelContextWindow`) feeds the composer donut while `Show context window usage` is on. | PASS (`context_status_reasoning_stream_and_compaction`; live 0.164 app-server proof in the PR) |
| 7 | Command card | click | Expands output; copy buttons. | The `Ran` row opens the `Shell` card with output and exit footer. The card header has `Copy command` and `Copy output` buttons that write the clipboard. | PASS (card expansion: `grouped_activity_matches_the_original`; copy buttons: `crates/ui/src/thread/items.rs` `copy_control`, no recorded pixel reference) |
| 8 | File change card | click | `View changes` opens Changes; `Undo` / `Reapply`. | `View changes` opens the Changes side panel; `Undo` sends `thread/revert` (`beforeTurnId`, after `thread/resume` if needed) and shows the reverted thread. No `Reapply`. | FAIL (`Reapply`: the original re-applies through the host git worker; `Undo` checked by `undo_reverts_turn`, `View changes` by `view_changes_opens_panel`) |
| 9 | Collapsed turn `Worked for {time}` | click | Expands hidden items. | A click shows the turn's work above the answer (agent messages, `Ran commands`) and turns the chevron down; a second click folds it. | PASS (`worked_for_expands`) |
| 10 | User message hover | hover | Copy and Edit appear. | Copy message and Edit message appear on hover or focus without moving the transcript; Copy writes the message and shows a checkmark for 1500 ms. | PASS (`s005`, `s006`) |
| 11 | Assistant message hover | hover | Copy, Fork chat from here. | Shown under the last turn, on hover under earlier turns. `Copy` puts the answer on the clipboard; `Fork chat from here` sends `thread/fork` (then `thread/rollback` of the later turns) and opens the fork at the top of Recents. `Read aloud` and `Rate response` have no handler. | PASS (`copy_response`, `fork_chat`, `earlier_turn_actions_hover`) |
| 12 | `Scroll to bottom` | click | Jumps to the newest turn. | A 32 px round button floats above the composer band while the transcript is scrolled up; a click scrolls to the end and the button hides. No original recording exists, so its pixels are not compared. | PASS (behavior only; no pixel reference) |
| 13 | `⌘F` | shortcut | Find bar. | The bar opens on `cmd-f` with the query selected; Enter, Shift+Enter, `⌘G` and `⌘⇧G` step matches; Escape closes it. | PASS (`shortcut_chat_actions::pin_rename_and_find_shortcuts_act_on_the_open_chat`, `s028`) |
| 14 | `⌘⇧A` | shortcut | Archives the chat. | `ArchiveChat`: `thread/archive`, the row leaves Recents, the route goes to Home. | PASS (`archive_chat`) |
| 15 | `⌘⌥R` | shortcut | Rename dialog. | `RenameOpenChat` opens the `Rename chat` dialog (`Dialog::rename`) for the open chat. | PASS (`shortcut_chat_actions::pin_rename_and_find_shortcuts_act_on_the_open_chat`) |
| 16 | Approval card | click | Approve, Allow similar, Allow all edits, Deny. | `Root::on_event` keeps each pending request by its request id and thread. The open thread shows its oldest request. Command, file-change, permission, user-input, standard MCP elicitation, and supported dynamic-tool forms use this queue. Responses retain the displayed card's identity. Explicit resolution removes requests without answering them. Turn completion removes only turn-bound requests. | PASS (`approval_queue.rs`: command/file-change queue order, withdrawal, thread isolation and turn end; non-command release gates remain open below) |
| 17 | Stream error | error | `Reconnecting {progress}`. | Thread-bound notices show reconnect progress or busy status for `serverOverloaded` and HTTP 429 stream disconnects. Each digit uses the original character width and a separate layer. The suffix keeps its fractional position. Progress updates replace the current retry count. Later activity and completion retain source notices with their turn. | PASS (`thread_recovery`: thread and turn isolation, progress, busy status, retained notices, and zero pixel differences in both counter regions) |
| 18 | Usage limit | error | Usage limit banner. | `usageLimitExceeded` shows the divider and the advice for the plan reported by `account/read`. Unknown plans use the retry-later fallback. Source details remain available, and completion retains terminal errors. | PASS (`thread_recovery`: account-plan advice and source-error retention) |
| 19 | Focus ring on cards | focus ring | 2 px ring (`focus-visible:ring-2 ring-offset-0`) on the diff card's `Undo` and `View changes`. | Both buttons are tab stops and draw the shared 2 px `#3a83f7` ring only under keyboard focus; a pointer move keeps it. No original capture of the focused card exists, so its pixels are not compared. | PASS (`thread_card_focus_ring`; no pixel reference) |
| 20 | Resize | resize | The side panel yields width before the chat column. | `panel_width` keeps 320 px for the chat; no flow checks the 3 widths with the panel open. | FAIL (no flow; `crates/ui/src/diff.rs` `panel_width` assumes the sidebar open) |

TH-123/124/125 add permission, user-input, standard MCP elicitation, and supported dynamic-tool forms to the existing request queue.
The request editor retains drafts by thread and request while switching chats or recovering from a failed cancellation.
MCP elicitations and nonblocking questions remain pending until explicit resolution.
Unsupported question and permission requests receive empty-denial responses.
Skipping an optional final choice submits the answers from earlier fields.
Enter retains draft submission for command and file-change approvals.
The reference frames use the pinned original renderer on macOS, with native fonts, at 1280x832 and scale 2.
The native request replay covers numeric bounds, pointer steps, and keyboard stepping after a pointer step.
Plain input fields keep a stable keyboard context across request and focus changes.
The appearance packet remains held until the remaining visible differences are resolved.

Native Computer Use app approvals use this queue and retain the original request, thread, and turn IDs.
Only the registered `lcu` server can use the native card.
The card shows the app name, elevated risk, and first-use disclosure.
The card uses the shared rounded clip for content and paints the risk shield outside that clip.
The icon rasterizes during paint in the active compositor layer; the footer records in window space.
`Allow this conversation` selects the offered session scope; without that scope, `Allow once` permits one use.
`Always allow` appears only when the server offers that scope in the persistence array.
MCP responses use `action`, optional empty `content`, and the selected `_meta.persist`; they never use a command-approval `decision`.
The host retains the disclosure choice only after an accepted response enters the bridge.
Run `cargo test -p app --test computer_use_approvals`; the native replay and captures require macOS.

The native recovery replay runs with `cargo test -p app --test thread_recovery -- --test-threads=1`.
It exercises Root, ThreadView, the replayed JSON-RPC bridge and the Metal renderer on macOS.
The deliberate render-panic case runs only on Linux. The Mac suite exits on unexpected panics to prevent system crash dialogs.
Set `CGX_THREAD_RECOVERY_PROOF` to an output directory to retain native captures.

MO-36 uses `refs/interactions/ms4.json`, recorded with the original CSS classes in an isolated route1 Chrome context.
The recording appends synthetic markdown spans and seeks their animations; it does not measure live model segmentation.
Run `tools/mac.sh -l ms4 'cargo test -p app --test stream_fade'` for native glyph-alpha checks.
The test samples each delta from 0 to 700 ms and checks that earlier glyphs remain unchanged.
Linux uses a no-op text system, so these glyph checks require macOS.
MO-37 and MO-39 remain with the image and block-renderer slices, as approved for MS4.

S010 uses the original native menu template in `refs/interactions/s010.json` and the retained S011 paragraph and clipboard contract.
The selected-text menu enables `Look Up`, `Search with Google`, and `Copy`, with the original separators and Copy shortcut.
The menu measures its row widths once when it opens, so the original Look Up label fits without clipping.
`Look Up` calls `textsys::show_definition` at the menu's opening point; it does not open Dictionary.app or change the clipboard.
`crates/app/tests/s010.rs` checks the rows, selection, clipboard, search URL, dismissal, reverse-selection copy, and absence of app-server requests.
It checks Copy by mouse and `Cmd+C` while the menu holds focus, with old clipboard text seeded before each action.
The shared menu restores transcript focus before running an action, so `Cmd+C` still works after dismissal.
S011 remains unchanged and continues to check paragraph selection and highlight pixels.
The headless S010 replay has no AppKit view; it checks the missing-handle path without an external-app fallback.

## 14.5 Composer

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Composer in thread | entry | `Work with Codex` (idle) or `Working…`. | Docked thread composer, focused on open; `Working…` not checked. | PASS (`shift_enter_newline`, `parity_thread`) |
| 2 | Type text | focus | Editor grows to a max height, then scrolls. | The card's editor grows with its lines up to 25% of the window height (`max-h-[min(25dvh,…)]`), then scrolls and keeps the caret in view. | PASS (`composer::tests::editor_grows_to_a_quarter_of_the_window_then_scrolls`) |
| 3 | Enter | shortcut | Sends; `⌘ + Enter` modes per setting. | Reads `desktop.composerEnterBehavior` at startup and after successful writes. `cmdAlways` requires Command+Enter; `cmdIfMultiline` requires it for multiline drafts. Shift+Enter inserts a line break. | PASS (`submit_enter_starts_thread`, `send_shortcut_controls_enter_without_changing_shift_enter`) |
| 4 | Shift+Enter | shortcut | New line. | New line in the Home and thread composers. | PASS (`shift_enter_newline`) |
| 5 | `@` | shortcut | Mention list. | `@` opens the list; typing searches the workspace files with `fuzzyFileSearch`; Enter inserts a chip. | PASS (`s019::mention_list_replay`, `s020::mention_keys_chips_and_prompt_replay`) |
| 6 | `/` | shortcut | Slash command menu. | Matches the recorded menu, filtering, selected row, editor focus, arrows, Escape, Enter, and Tab. | PASS (`s021::slash_menu_keyboard_replay`) |
| 6a | `/` then Enter on `Compact`, `Rename`, `Pin chat`, `Fork chat`, `Model`, `Reasoning`, `Work in a project` | shortcut | Runs the chat action (CP-058 to CP-063, CP-068, CP-069). | `Compact`: `thread/compact/start` (resumes the chat first), toast `Compact is disabled while a chat is in progress` while a turn runs, `Failed to compact chat` on error. `Rename`: the `Rename chat` dialog; closing it focuses the composer. `Pin chat`: `thread/section/move`, toggles. `Fork chat`, `Model`, `Reasoning`, `Work in a project`: Enter turns the token into `/id ` and opens a submenu filtered by the text after it; Backspace over the space returns to the list, Escape leaves the text, a pick removes token and query, keeps the rest of the draft, and acts (`thread/fork` in the same workspace with `Failed to fork chat` on error, `config/batchWrite`, the existing-project selection for new chats). Not wired: `Fork chat in new worktree`, the submenu's `Search` input row, the usage text on `Compact`, and the other commands. | PASS (`slash_commands`) |
| 6b | `/fast` then Enter; Settings > Configuration `Available reasoning efforts`, `Open config.toml` | shortcut | `/fast` toggles Fast; the efforts menu lists the `model/list` catalog's optional efforts; Open config.toml opens the user layer. | `/fast` writes `service_tier = "priority"`, a second `/fast` removes it (`config/batchWrite`). The menu shows `Max`, `Ultra`, and `Persistent` only when a catalog model supports them, saves `enabled-reasoning-efforts`, and the composer's effort choices follow it (`low` to `xhigh` always stay). The page reads `config/read` with `includeLayers` on the window's host: a row that another layer sets names that layer, a disabled layer shows its reason, and Open config.toml opens the user layer's file. A remote host's path or a missing user layer shows a toast; a failed read shows `Unable to read config.toml`. | PASS (`settings_actions::fast_toggles_the_priority_service_tier`, `settings_actions::configuration_efforts_and_open_config_follow_the_host`) |
| 7 | Paste large text | click | `Pasted text.txt` chip. | Clipboard text of 5,000 UTF-16 units or more is saved as `Pasted text.txt` and shown as a chip. | PASS (`attach::tests::pasted_text_keeps_its_host_after_another_window_becomes_active`) |
| 8 | Drag file over composer | hover | `Drop to attach`. | A file dragged over the page column shows the `Drop to attach` overlay and the drag-active card; it clears when the file leaves, and a drop attaches the files. | PASS (`s017b::file_drag_over_the_composer_matches_the_original`, macOS) |
| 8a | Selected-host image attachment | action | CLI image input uses a path on the selected host. | Read image previews through that window's `fs/readFile`. Save pasted images with `fs/createDirectory` and `fs/writeFile` under its reported Codex home. Send host paths as `turn/start` `localImage` inputs. Other active windows cannot redirect these requests. A missing image blocks Send and keeps the draft. | PASS (`host_images`, native live app-server 0.164.0-alpha.11.1: `Red`) |
| 9 | Attachment chip remove | click | Removes the attachment. | The chip's `×` removes that attachment only. | PASS (`attach::tests::upload_states_and_chip_remove`) |
| 10 | Model picker | click | `Select model`, `Select effort`, Fast. | Opens; a model or effort choice writes `model` and `model_reasoning_effort` with `config/batchWrite` (the original's `writeModel`); parity: see `parity_composer`. | PASS (`model_picker_popover`, `effort_pick_writes_config`, `parity_composer`) |
| 11 | Permissions dropdown | click | Ask / Approve for me / Full access / Custom. | Opens. | PASS (`permissions_popover`) |
| 12 | Submit while running | click | Stop; with text Queue or Steer. | Stop interrupts the active turn. Queue adds a server-owned submission; Steer sends input into the active turn. | PASS (`s023`, `s023b`) |
| 13 | Stop | click | `turn/interrupt`. | Sends the active thread and turn IDs; completion removes Stop. Errors show `Failed to stop chat`. | PASS (`s023::stop_interrupts_only_the_active_turn`) |
| 14 | Queued message row | click | Steer, Edit, Delete, Retry. | Steer sends the selected submission once. Delete removes it; Cmd+Z restores its text, identity, order, and chat location. The `…` menu (`Queued message actions`) holds `Edit message`, `Open in side chat` and `Turn off queueing` / `Turn on queueing`. `Edit message` sends `thread/queue/delete`, then moves the exact text and local images back to the composer (a draft already there stays below it; remote images, audio, skills and mentions wait in the app and go out with the next send). Sending queues it again (`thread/queue/add`) and `thread/queue/reorder` puts it before the row that followed it, else after the one before it; the restore reads every `thread/queue/list` page first, so a long queue keeps all its rows. If the server already took the submission, only the tray refreshes. `Open in side chat` sends `thread/queue/delete`, then `thread/fork` (`ephemeral`, `excludeTurns`, the original's side-conversation developer instructions) and `thread/inject_items` (the boundary message), and starts the fork's first turn with the queued input (`turn/start`, `turnTrigger: queued_side_chat`, same `clientUserMessageId`). The fork opens as a selected side panel tab, titled with the message, with its own transcript and focused composer; the chat and its draft stay. Each conversation takes only its own thread's notifications, queue, approvals and `Stop`; a side chat's Delete has no Cmd+Z undo. Closing the tab interrupts a running side turn and sends `thread/unsubscribe` for the fork only. A failed fork, injection or first turn closes the tab, queues the message again in its old place (`thread/queue/add`, `thread/queue/reorder`) and toasts `Failed to open side chat`. The item is hidden for a chat archived in this session. The queueing item writes `desktop.followUpQueueMode`, so the next submit queues or steers; a refused write toasts `Couldn’t save your changes. Try again` and keeps the label of the behavior that applies. A refused follow-up stays in its chat's tray with Retry. Retry preserves the exact input and client message ID, leaves the current draft alone, and uses the current queueing mode or starts a new turn if the old turn ended. Local failed rows support Edit, Delete, undo and side chat without deleting a server row. | PASS (`s023b::failed_follow_up_retries_without_changing_the_draft`, `s023b::failed_follow_up_retries_as_a_new_turn_after_completion`, `s023b::failed_follow_up_delete_undo_and_edit_stay_local`; CP-113/114 in `s023b`; CP-115 and retry input preservation in `queue_options`; a queue of several pages in `queue_options::edit_restores_the_place_in_a_queue_of_several_pages`) |
| 15 | Submit blocked | error | `Unable to send message`. | A pending or failed upload opens `Unable to send message` with its reason and one `OK` button, and keeps the draft. `OK` or Escape closes it and focuses the composer. The dialog stays open if the upload ends first. | PASS (`composer::tests::submit_blocked_by_uploads_shows_unable_to_send_message`) |
| 16 | Upload in progress | loading | `Files uploading…`. | The line shows above the editor while a paste saves; a submit opens the row 15 dialog. | PASS (`attach::tests::upload_states_and_chip_remove`) |
| 17 | Upload failure | error | `Unable to upload file`. | The line shows when a paste fails to save; the next paste clears it. | PASS (`attach::tests::upload_states_and_chip_remove`) |
| 18 | Button hover | hover | Ghost hover; tooltip after a delay. | Hover fill. The `+` tooltip includes the `@` hint and matches its original bounds. | PARTIAL (`+`: `s018::add_tooltip_replay`; other composer tooltips remain absent) |
| 19 | Focus ring | focus ring | Send button ring. | `send` has a tab stop and a 2 px `focus_visible` ring in the text color (`composer/footer.rs`); no native test drives keyboard focus onto it. | WIRED (`composer/footer.rs`, `send`) |
| 20 | Resize | resize | Below 40rem the editor and footer stack. | The editor sits above the footer at every width, and the footer collapses by its container width (440, 300 and 509 px). `refs/composer` has 1280 and 1440 px only, so no narrow layout is recorded and none is invented. | BOUNDARY (`crates/ui/src/composer/footer.rs`; `refs/composer` has no narrow capture) |
| 21 | `+` Add menu | click | Opens the mention list without inserting `@`; Escape keeps the draft. | Reuses the S019 query state; keys skip disabled actions, and a second click closes the list. | PASS (`s018::add_menu_replay`) |

The `host_images` replay waits for attachment completion before it sends or retries.
Its proxy records requests before replies finish; advancing the test clock does not wait for those replies.

Queued side chats restore the fork response's model, effort, and permission settings without changing the primary composer.
Their tab shows activity while a turn runs.
Turns without a server start time measure elapsed time from their first notification without adding a message timestamp.
The queue replay matches the retained running state at one second.
The queue dropdown sizes itself to its icon labels and opens 2 px below the measured button, 1 px inside its right edge, including keyboard activation.
Queued rows use the original Steer and Delete icons, action spacing, and 13 px Steer label.
Editor context menus keep their existing minimum width.
The original/native pixel comparison still fails for transcript geometry and surface colors.

S018 records CP-039 and CP-040 in `refs/interactions/s018.json`.
Its Add replay checks local actions, selection, query text, focus, Escape, reopen, and the second trigger click.
Its draft check covers a Unicode prefix, replacement before the query anchor, and typed `@` after dismissal.
The tooltip matches the original trigger bounds and y=742..772 box, including the unchanged bottom probe.
The bordered trigger uses `side_offset(px(5.))` for its recorded 4 px visible gap; other tooltips keep the shared default.
Host-provided Plugins, Agents, Pages, and Spaces remain outside the local-action scope, as in S019 and S020.
The isolated original host returns HTTP 200 `{}` for Pages search; repeated queries can fail at `N.items.flatMap`.
The recording retains that limit and does not replace the host response.

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

S023b covers CP-113 and CP-114 with the existing headless native Metal/CoreText application and the real Root/bridge path.
The isolated local protocol fixture retains server queue IDs and the original `clientUserMessageId` and input.
Native mouse and key input queues three messages, steers one row, deletes another, changes chats, and restores with Cmd+Z.
The checks preserve remaining rows, paginated order, Unicode text, editor history, and the `Queued message restored` feedback.
The Steer and undo checks retain the exact pixels of the remaining queued message.
An idle Steer starts the existing queued submission through `thread/queue/start`.
A refused queue deletion does not send an accepted steer again.
A failed restore reorder retains the undo entry and does not add the message twice.
The original S023b recording, pixel probes, and references remain unchanged.
Captured images are headless native application frames, not installed-window desktop captures.
Run `tools/mac.sh -l queue-actions 'cargo test -p app --test s023b -- --test-threads=1'`.

Failed follow-ups stay in a per-chat local queue until Retry succeeds, Edit takes them back, or Delete removes them.
Retry keeps the client message ID, exact input, model, effort and permissions when it starts a new turn.
`queue_options::retry_keeps_edited_images_skills_and_queue_position` checks retained images, skills and queue order.
`queue_options::failed_follow_up_can_open_a_side_chat_without_deleting_a_server_row` checks local transfers and fork failures.
The original S023b JSON records the failed row and Retry button, but this state has no retained original screenshot.
The native before/after screenshots show the editor-restoration behavior and the completed Retry row at the same capture size.

When the model catalog is unavailable, the composer blocks idle submission from both Send and Enter.
`git_revert_ui::missing_model_blocks_new_submission_but_preserves_running_actions` checks the retained draft and running-turn actions.
Inline code wraps within a narrow thread while its background stays continuous on each line.
`git_revert_ui::inline_code_wraps_across_lines` checks both visible line backgrounds.

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
Search chat and Search diffs keep the query while switching the active find surface.
Enter, Shift+Enter, and the Find Next commands move the active diff highlight and scroll it into view.
The find bar paints its shadow, border, and content in one composited layer.
The scope icons bypass fitted images, and the next arrow uses the original path with a half-turn SVG matrix.
Its rows sit inside the bar's anti-aliased rounded clip (padding box, 19.5 px radius), which sets the icons' edge pixels.
The diff matcher reuses the chat's Unicode matcher and retains the total count beyond the 250 highlighted matches.
Local repository sources keep a recursive filesystem watch while the Changes tab is active.
The app drops the watch when the repository or active route changes.
Changes that arrive during a read cause one more refresh after that read.
Watch failures show the original Refresh notice.
`item/fileChange/patchUpdated` replaces the streamed file changes until an aggregate turn diff takes precedence.

`refs/interactions/s027.json` records the original diff menus, text selection, clipboard writes, and line hover.
`s027::diff_selection_and_context_actions_replay` checks forward and reverse text selection and `Cmd+C` without line numbers.
The original does not change a line's background on hover in this state, despite the DT-014 inventory claim.
The test compares the port's pixels before and after the same hover.
The header menu copies the displayed path and collapses or expands the file.
The body menu copies selected text, absolute paths, or displayed relative paths.
The body menu also wraps long code lines without changing the copied text.
The `Open in editor` menu item sends the file's absolute path to the desktop host's `open-in` endpoint with the thread `cwd`.
The header's `Open in` button opens the existing native file tab.
Streamed diff paths resolve against the thread's working directory, not the selected workspace.
The body menu reads available targets from `open-in-targets`.
`Open with` includes a native file tab and each available host target.
The shared `open-in` launcher handles app paths, project preferences, and launch failures.
A failed `Open in editor` action shows `Unable to open in editor` and preserves the displayed changes.
`s027::failed_editor_launch_keeps_the_diff_and_reports_the_error` runs the real host with a controlled opener on an owned repository.
The gutter's round button opens a comment editor for one line or a selected range.
Comments support edit, cancel, delete, and send actions.
The draft keeps its selected lines visible and uses the original author header and line reference.
Its text layout and input hit testing share the comment editor's line height.
Saved local comments keep the draft card's author header and text metrics.
Editing preserves the stored comment until Save.
The composer groups comments in one count attachment.
Its preview opens the selected source file; `Remove comments attachment` clears the stored comments.
Closing the keyboard preview restores focus to its trigger.
The comment's Dictate control targets its own Composer through the shared dispatcher.
The microphone keeps its original shape against the comment card background.
Comment and Save wait for a successful completion before persisting the edited text.
A failed or cancelled transcription keeps the comment editable and preserves its stored value.
Clicking the comment card returns focus to its editor, including after an error.
Controlled native input checks transcript placement, finish/cancel commands, and rejection of late results after closing.
The test does not open a microphone or request permission.
Dismissed model comments persist after confirmation.
`diff_comments` checks those actions through native pointer and keyboard input.

Local Changes uses the existing desktop host for `git-changes`, `git-stage`, and `git-unstage`.
Each request uses the open thread's absolute `cwd`; file actions also send one repository-relative `path`.
The response body contains `root`, `unstaged`, and `staged`; each file contains `path` and its unified `diff`.
The selector offers Last Turn, Uncommitted, Unstaged, Staged, Committed, and Branch.
Committed opens the commit submenu; the selected source has a checkmark.
The toolbar retains Refresh. Re-selecting a repository source also refreshes a narrow panel.
Uncommitted compares the working tree with HEAD and includes untracked files.
Branch compares HEAD with the selected branch's merge base; Committed compares a selected commit with its first parent.
An initial commit compares with the empty tree.
Each local file menu stages or unstages the whole file; unstage preserves the working tree.
Reads and mutations run off the UI thread, with loading and error states in the panel.
Tracked files use one bulk Git diff per source; untracked files use separate comparisons.
The `git_stage` native proof checks the real index, visible diff text, partial staging, deletion, lock recovery, and unborn repositories.
It also checks literal and newline filenames, multi-file patches, and rejects paths outside the repository.
Hunk Stage and Unstage apply only the reviewed hunk to the index and preserve the working tree.
Stage all, Unstage all, and Revert all send the reviewed section through the existing Git endpoints.
The host checks every file snapshot before it applies one combined patch.
Patch writes use the captured bytes; only the displayed diff replaces invalid UTF-8.
Revert all uses the same confirmation and saved preference as file and hunk reverts.
Uncommitted permits Stage all but hides and rejects Revert all; that action is available only in Unstaged.
Source and revision menus remain open while repository refresh replaces the displayed files.
For a folder outside Git, Changes shows Create a Git repository and the Create git repository button.
The button calls `git-init-repo` in the thread's folder; initialization preserves existing files and rejects an existing repository.
The portable `git_review` proof checks real source selection, repository creation, file/hunk/section effects, stale snapshots, locks, and newer-work preservation.
The file menu, file revert button, and per-hunk `Revert` controls call the existing `git-revert` host endpoint.
It takes `cwd`, a repository-relative `path`, `source` (`unstaged` or `staged`), the `diff` text that Changes shows for that file, and an optional 0-based `hunk`.
The host captures that file's `--binary` diff once, compares it with the request up to the binary data (the text form says `Binary files … differ` there; the full `index` line pins that data), and refuses a stale request (`file changed since its diff was shown`); the same capture is the patch it applies, so an edit made while the request runs can only fail the check, never enter the patch.
An unstaged revert reverse-applies the patch to the working tree, which also deletes an untracked file and restores a deleted one.
A staged revert reverse-applies it to the index and then to the working tree, as the original does.
The response `status` is `partial-success` when only the index was reverted: the working-tree step failed, or the working file already differed from the index, because Git could then move the patch onto a look-alike block and erase newer edits.
Otherwise `status` is `success` and the body is the usual `{root, unstaged, staged}`; a Git failure is an error and changes nothing.
A hunk revert leaves the file mode alone; a whole-file revert restores binary files and modes.
The `git_revert` native-host proof checks the real index and files for file and hunk reverts, partial results, look-alike blocks, modes, binary files, stale input, an edit made during the request, an index lock, and rejected input.
File and hunk actions show `Revert changes?`, `This action removes all of these changes.`, `Don't ask again`, `Cancel`, and danger `Confirm`.
Confirm starts focused; Escape, Cancel, and an outside click dismiss without a Git request.
Only Confirm saves `skip-revert-confirmation`; the host preference survives reload.
The request retains the displayed patch and source while the dialog is open.
Success shows `Reverted {path}` or `Reverted hunk {n} in {path}`.
Failure shows `Failed to revert` with the target and keeps the displayed changes beside the host error.
A partial result shows `Partially reverted` with the target and refreshes both sources from the host response.
The `git_revert_ui` native proof drives pointer and keyboard input on disposable local Git repositories.
It checks cancellation, preference reload, file and hunk effects on disk and index, stale and locked failures, and index-only partial results.
Remote Git remains separate work.
Commit and push are in DT-139 below.

Diff controls reuse `DiffPanel` for the thread panel and the standalone `/diff <patch-file>` entry.
The standalone entry reads a Git-style unified patch from disk without starting an app-server.
Auto uses split layout only when both sides change and the file pane is at least 800 px wide.
The file tree filter changes the tree, not the displayed code.
The Word diffs option toggles separate changed-word spans without changing the patch or line counts.
Review menu icons retain the original SVG or compositor opacity, including disabled rows and nested popup layers.
Find highlights use rounded inline boxes and keep the current match visible.
The tokenizer follows the original word-alt joining rule and keeps UTF-8 boundaries.
It skips word spans above 1,000 UTF-16 units per line or 2,000 changed lines per file.
Diffs and file tabs cache syntax tokens from the installed Syntect grammars.
The old and new sides keep separate multiline parser state.
Python function declarations use the original declaration color.
Binary, empty, and unchanged rename entries stay in the file list with their original placeholder labels.
Git-quoted paths retain spaces, newlines, and UTF-8 bytes.
The header and context menu share the same word-wrap state.
Unmodified-line controls use the original 32-pixel context row.
Jump to file filters with the existing path matcher and keeps files with the same name as separate results.
Selecting a result opens that file and scrolls its header into view.
File headers stay at the top of the viewport while their diff scrolls.
`DiffPanel::cloud` accepts the original cloud patch without adding a second transport.
Its Cloud changes source has no local source selector or repository mutation controls.
The file viewer handles `⌘L`, clamps line numbers, and counts negative numbers from the end.
The line input stays closed until file contents are available.
The `git-file-url` host read uses the current branch for a GitHub origin, as in the original.
It encodes path segments and can include a line anchor.
Other origins return no link.
A detached HEAD returns no link.
A file stops rendering above 15,000 changed lines, 3,145,728 changed UTF-16 units, or 1,048,576 units in one changed line.
The count includes retained newlines and excludes unchanged context.
Oversized files keep their change counts and large-diff navigation.
Invalid hunk numbers show `Diff failed to render` instead of overflowing a line counter.
File boundaries must start a line; a `diff --git` string inside changed code does not split the file.
The view shows one file at a time above 128 files, 9,000 changed lines, or 12 MiB of diff input.
Previous file, Next file, the tree, and Jump to file share that active file.
Above 256 untracked files, repository reads omit all untracked patches and report the actual omitted count.
The warning offers the original interactive cleanup command without running it.
Known commit and revert results clear the last-turn diff and offer View branch diff.
Refresh retains the displayed changes.
A failed read or mutation adds a status row with Retry.
`cargo test -p app --test diff_controls -- --nocapture --test-threads=1` runs the native input proof on macOS.
`CGX_DIFF_EVIDENCE` selects its screenshot folder; the default is the system temporary folder under `cgx-diff-controls`.
The proof covers header wrap, file filtering, layout changes, empty and loading states, real read failure, Retry, and line navigation.
The `diff_review` native proof also checks visible find navigation, scope changes, and Escape against the recorded subtract patch.
`cargo test -p app --test diff_review -- --nocapture --test-threads=1` checks the Word diffs toggle and four real Git file states.
`CGX_DIFF_REVIEW_EVIDENCE` retains that proof's screenshots and source patch.
`cargo test -p app --test diff_states --test git_review -- --nocapture` checks the omission limit and preserves real Git data.
On macOS, `diff_states` also drives nested file updates, streamed patches, and large-diff page selection through the native view.
Original captures for the file tree and file filter remain open capture items.
The release comparison omits both pairs and identifies its exact matching component crops.
Whole-window pixel parity is not claimed by those component comparisons.

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Open Changes panel | entry | `⌥⌘B` or `View changes`. | Both open the last turn's diff (`Last Turn` source). A file link opens the File viewer, as S009 records. | PASS (`changes_panel_keys`, `view_changes_opens_panel`, `s009`); strict pixels FAIL (`parity_diff`) |
| 2 | `/diff` window | entry | Single file diff. | `/diff <patch-file>` opens the shared renderer in its own window without a chat backend. | PASS (`diff_controls`; standalone executable accepts the route and reports unreadable input) |
| 3 | Source selector | click | Uncommitted / Staged / … | Last Turn, Uncommitted, Unstaged, Staged, Committed, and Branch select the corresponding local comparison. | PASS (`git_review`, `git_revert_ui`) |
| 4 | Show / Hide files | click | Toggles the file tree. | The header toggles the file tree without losing its filter. | PASS (`diff_controls` native frames) |
| 5 | File tree filter | focus | `Filter files…`. | Filters paths and shows `No matching files`; typing and paste retain keyboard ownership. | PASS (`diff_controls` native frames) |
| 6 | File header actions | click | Copy path, Open, Collapse/Expand. | Copy path and Collapse/Expand work. The header opens a native file tab; Open in editor uses the context menu. Local sources add Stage file, Unstage file, and confirmed Revert file. | PARTIAL (`s027::diff_selection_and_context_actions_replay`, `s027::failed_editor_launch_keeps_the_diff_and_reports_the_error`, `git_stage`, `git_revert_ui`) |
| 7 | Hunk revert / stage | click | Revert, Stage / Unstage hunk. | Local staged and unstaged hunks have confirmed Revert and Stage/Unstage controls. | PASS (`git_review`, `git_revert_ui`) |
| 8 | Revert all | click | `Revert changes?` dialog. | Unstaged provides section-wide Revert all with confirmation and the saved skip preference. Uncommitted does not expose this action. | PASS (`git_revert_ui`) |
| 9 | Split / unified toggle | click | Switch diff layout. | Unified, Split, and Auto share the existing line renderer. The narrow header exposes layout through Changes options. | PASS (`diff_controls`; split pairing regression check) |
| 10 | Word wrap | click | Toggles wrap. | The header, Changes options, and body menu toggle the same wrap state. | PASS (`diff_controls`; `s027::diff_selection_and_context_actions_replay`) |
| 11 | Mark as viewed | click | Branch diffs collapse and retain the reviewed revision. | Save marks by repository, comparison, path, and file revision. Unmarking expands the file. A failed save leaves its state unchanged. | PASS (`diff_viewed`: native repository, saved state, revision change, failed save; original control pixel comparison) |
| 12 | `⌘F` in diff | shortcut | `Search diff…`. | Focus the panel, open find, and type a query; matches highlight and update with the diff. | PASS (`s028::streamed_diff_and_find_replay`) |
| 13 | `⌘L` | shortcut | Go to line in the file viewer. | Available files expose the line input; Enter navigates, Escape closes, and negative numbers count from the end. Unavailable files keep the input closed. | PASS (`diff_controls`; whole-line validation regression check) |
| 14 | Line hover | hover | No background change in the recorded Last Turn state; no `data-hovered` attribute. | No background change. | PASS (`s027::diff_selection_and_context_actions_replay`; corrects DT-014) |
| 15 | No changes | empty | `No file changes yet`. | Last Turn shows `No file changes yet`; local sources show `No staged changes` or `No unstaged changes`. | PASS (`diff_controls` native frames) |
| 16 | Not a git repo | empty | `Create a Git repository`. | The action initializes the thread's folder and preserves existing files. | PASS (`git_review`, `git_revert_ui`) |
| 17 | Loading | loading | `Loading diff`. | Initial local reads show `Loading diff`. Refresh retains the file list and disables mutations until completion. | PASS (`diff_controls` native frame) |
| 18 | Load failure | error | `Couldn't load changes`. | Shows the host error and Retry without hiding existing changes. Invalid hunk data shows `Diff failed to render`. | PASS (`diff_controls` real read failure and recovery; parser regression checks) |
| 19 | Too large | error | `This file is too large to display here.` | Oversized input keeps its header and offers Open in editor. | PASS (`diff_controls` native frame; parser limit checks) |
| 20 | Resize | resize | Split → inline for narrow panes. | Auto uses inline at 767 and 600 px and split for a wide, two-sided diff. | PASS (`diff_controls`; original 1280, 767, and 600 px captures) |

### DT-139: Commit or push

The summary Changes row exposes `Git actions` on pointer hover or keyboard focus.
Commit and Push open the existing form; Push does not run before the user selects its form row.
The shared command menu offers `Commit or push` only when Commit or Push is available and no workflow runs.
Git availability includes staged, unstaged, and untracked changes, independent of the form's saved checkbox.
Disabled Git rows show the original loading or unavailable reason.
Arrow keys skip disabled rows, Enter selects a row, and Escape returns focus to the trigger.
Opening the form or command list closes Summary.
Closing the form returns focus to the current composer.
The menu shows the active workflow phase and cannot start a second operation.
Opening either menu reads status through the existing host; stage, revert, commit, and push results refresh it.
Responses remain bound to the current thread entity and host.
The dirty form uses the retained original's 420 × 289 px surface and 20 px corners.
It uses 4 px field and action gaps, 14 px text, and the original commit/cloud-upload icons.
The native capture also renders the original's blank-message and new-branch states with its `+5 -1` selection at 1280 × 832.
`Commit to` lists the current branch and `New branch`; a detached HEAD starts on `New branch`.
The new name is prefilled with `git-branch-prefix` (default `codex/`) and the first five words of the chat title.
`Include unstaged changes` (default on, kept for the session) shows `+N -M` of the selection or `No changes`.
`Commit`, `Commit and push` and `Push` carry the original's tooltips: `No changes to commit`, `No new commits to push`, `Branch information unavailable`.
When no local changes exist, the form hides the message and checkbox and focuses `Push`.
The form retains Push actions without a remote; the existing host reports a missing remote as a failure.
An empty or taken name disables all three rows.
`Cmd+Enter` runs the focused row, else `Commit`.
The host explicitly routes `git-commit-status`, `git-commit-diff`, `git-commit`, `git-push`, and `git-checkout` with the existing Git actions.
It does not accept a generic `git-*` route.
`git-commit` creates the branch, stages everything when unstaged changes are included, then commits; `git-push` sets the upstream of a branch that has none, as the original's push does.
A blank message is written as the original's `generate-commit-message` does: an ephemeral read-only app-server thread (`thread/start` with `ephemeral`, then `turn/start` with a structured `message` schema) answers the original prompt, and the thread is unsubscribed.
The generation model is `gpt-6-luna` unless the model list lacks it.
Failures toast `Failed to create branch`, `Failed to commit changes`, `Failed to push changes` or `Failed to generate commit message: …` with `Copy error`; success toasts `Committed to {branch}` or `Pushed {branch}`.
The `git_commit` native proof drives all of this with keys and a mouse click against owned repositories and a local bare remote.
Its message generation answers from a crafted replay exchange, because the fixtures hold no recording of it.
The queued-state check uses native input and captures the rendered disabled UI before the background executor drains.
It then drains real host work and checks the owned commit and remote refs.
It does not park Git or fake a host response.
Matched capture fixtures use the retained Subtract thread and an owned `+5 -1` checkout.
The `codex_git_commit` attribution trailer and `Create PR` remain separate work.

### Native Create branch and Switch branch

`LocalHost` accepts `git-checkout` with `cwd` and exactly one of `branch` or `newBranch`.
`branch` resolves a local branch, a unique remote branch with tracking, or a commit reference for detached checkout.
Ambiguous remote names fail without changing the current branch.
`newBranch` validates the name and rejects existing names and repositories without commits before creating and checking out the branch.
Success reports the actual current branch, or null for a detached HEAD.
The host reads full symbolic references and removes only `refs/heads/`; the branch list uses `refname:lstrip=2`.
Disambiguated display names never replace branch identities, and a real branch named `heads/main` keeps its name.
Git carries safe local changes and refuses checkout when it would overwrite tracked, staged, or untracked files.
The failure reports `blocked-by-working-tree-changes`, the affected paths, and the Git error.
The operation never forces checkout, stashes changes, or contacts a remote.
`git_commit::checkout_dispatch_preserves_changes_and_resolves_branches` exercises the actual runtime dispatcher against disposable repositories and a local bare remote.
It checks the resulting refs, file contents, index state, detached checkout, invalid names, and remote ambiguity.
The Git actions menu enables Create branch on a detached HEAD and Switch branch when repository information is available.
Switch branch opens a searchable native list with the current branch marked and its uncommitted file count.
Keyboard navigation and pointer selection call the same checkout operation.
Case-insensitive search retains a separate `Use {ref}` row for an exact, case-sensitive Git reference.
Selecting the current branch closes the list without changing Git state.
The list also offers Create and checkout new branch, except in a repository without commits.
The create form uses the configured branch prefix and chat title.
It removes the original input's excluded characters, rejects empty names, trailing slashes, and existing names, and shows Git validation errors.
Failed creation keeps the name draft and current branch.
A blocked checkout shows the conflicting paths and offers Commit and switch branch.
That action opens the existing commit form and retries checkout only after the chosen operation succeeds.
A failed commit keeps the message draft; closing a Git surface preserves the unsent composer draft.
`git_commit::native_branch_controls_preserve_drafts_and_protect_changes` proves these controls with native keys and pointer input against disposable repositories.
The scenario checks real branch refs, retained file contents, commit recovery, and an unchanged local remote.
It also checks an exact tag checkout followed by native reattachment, a real `heads/main` branch, and committing dirty detached work.
Its thread data uses replay fixtures, not a live installed-account session.

### Native worktree ownership and GitHub process transport

`LocalHost` accepts the native `worktree-set-owner-thread` request and the Git worker's `set-worktree-owner-thread` request.
Both persist the original `codex-thread.json` format in a linked worktree's Git directory.
The main checkout is not a valid ownership target.
`automation_worktree::creates_the_automation_worktree_at_the_remote_default_without_touching_the_source` checks both request names against the same fixture.

`host::git_pr::execute` runs an argv vector through the connected app-server's `process/spawn` method.
It subscribes before spawning, matches `process/exited` by handle, and returns the exit code, stdout, stderr, and capture-limit flags.
The caller supplies an absolute working directory and a timeout in milliseconds.
One deadline bounds the spawn reply and process exit.
Timeout cleanup sends `process/kill` without waiting for its reply.
Dropped requests release their pending reply slots.
The deadline regression delays spawn and cleanup replies; neither delay extends the caller's timeout.
The transport disables Git and GitHub prompts and does not run a local shell.
These trusted UI commands run without a Codex sandbox.
Connection failures and process-spawn failures remain errors.
The original bundled app-server smoke checked concurrent handles, separate stdout and stderr, and a nonzero exit code.

## 14.7 Settings, login and onboarding

A settings row reports its change to `Root`.
`desktop.*` and `features.*` rows persist with `config/value/write`; `Root` keeps the value after the server accepts it.
The menu-bar switch persists through the local `settings.write` service.
Session rows keep their values for the session.
Leaving and reopening Settings shows the kept value (`settings_value_round_trip`).

The shared single-select menu searches labels and aliases, skips disabled choices, and returns focus to its trigger.
`settings_kit` checks these interactions and the existing checkbox-menu, cancellation, and external-link behavior.

S034 covers ST-003 and ST-026 with the unchanged off/on samples in `refs/interactions/s034.json`.
The replay advances the virtual clock by the original 150 ms transition before checking settled pixels.
Enabled switches use the existing GPUI native click path for Enter and Space, without duplicate key handlers.
The replay checks pointer focus without a ring, the blue keyboard ring, saved values, reload, and Back navigation.
S034b checks the recorded sign-in request, browser URL, pending view, and Home after completion.
Managed sign-in requirements finish before the app reports startup readiness.
The unmanaged-policy fallback uses the already loaded configuration instead of reading it again.

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Open settings | entry | `⌘,`, rail Settings or profile menu; `/settings/general-settings`. | `⌘,`, Codex ▸ Settings… and profile menu ▸ `Settings` open the General page; parity: see `parity_settings`. | PASS (`cmd_comma_opens_settings`, `menu_bar_actions`, `profile_menu_settings`, `parity_settings`) |
| 2 | `Back to app` | click | Returns to the previous route. | The nav has no `Back to app` row; titlebar Back returns. | PASS (`settings_back`) |
| 3 | Nav item | click | Navigates to `/settings/:section`. | The page changes (1488938 px repainted). | PASS (`settings_nav_click`) |
| 4 | Search settings | focus | Filters; `No results found`. | Typing filters the nav to `No results found`; Escape clears. | PASS (`settings_search`) |
| 5 | Theme Light/Dark/System | click | Applies the theme at once. | Appearance ▸ Theme (`Light`, `Dark`, `System`) dispatches `SetTheme`; `Root::set_theme` applies and persists it. | PASS (`appearance_theme_control`) |
| 6 | Collapse nav | click | The captured Settings route passes `canCollapse:false`. | No collapse control is added. | BOUNDARY (`settings-page-50e3fab957b6.js`) |
| 7 | Row hover | hover | Row highlight. | Highlight shows. | PASS (`settings_nav_hover`) |
| 8 | Keyboard nav | focus ring | Ring on rows and controls. | Search, nav rows, selects, segments, and switches use native focus and complete key presses. | PASS (`settings_search_and_theme_work_from_the_keyboard`, S034) |
| 9 | Section loading | loading | Status row with a spinner and text. | Personal instructions show loading, load failure, and Retry states. | PASS (`instruction_load_failure_can_retry_without_losing_the_page`) |
| 10 | Save failure | error | Toast. | Failed config writes show an error toast and keep the saved config unchanged. | PASS (`a_failed_setting_write_keeps_the_saved_value_and_shows_an_error`) |
| 11 | Login `/login` | entry | Sign-in screen. | A signed-out `account/read` shows the sign-in view. | PASS (`signed_out_shows_login`) |
| 12 | `Continue with ChatGPT` | click | `account/login/start`, waits for `account/login/completed`. | The recorded request and pending view work; only the active login can complete. | PASS (S034b, `foreign_and_cancelled_browser_logins_cannot_authenticate`) |
| 13 | `Cancel sign-in` | click | `account/login/cancel`. | Cancel sends the active login ID and ignores its later completion. | PASS (`foreign_and_cancelled_browser_logins_cannot_authenticate`) |
| 14 | `Copy sign-in link` | click | The original gates this control with `2502927686`; it is absent in the accepted fixture. | The control remains absent. | BOUNDARY (`login-route-27ff39de87da.js`, owned pending-login capture) |
| 15 | API key form | focus | `sk-…`; `Continue` when non-empty. | A permitted, non-empty key submits; a successful API reply enters Home without opening a browser. | PASS (`a_successful_api_key_reply_enters_the_app_without_a_browser`) |
| 16 | Login error | error | `Sign-in failed: {rawMessage}`. | The active login failure keeps the raw message and allows a new attempt. | PASS (`a_failed_browser_login_preserves_the_error_and_allows_retry`) |
| 17 | Sign-in requirements loading | loading | `Loading sign-in requirements…`. | The app gates methods on managed requirements; failed or empty policies cannot start authentication. | PASS (`a_policy_with_no_login_methods_never_starts_authentication`, S034b) |
| 18 | Welcome `/welcome` | entry | The owned original override renders an empty body. | No welcome screen is invented for this state. | BOUNDARY (owned original capture; instruction004) |
| 19 | Select workspace | click | Pick a folder or Skip. | A new profile opens workspace selection. Picker cancel stays there; selection or a successful Skip enters Home. | PASS (`workspace_picker_cancel_stays_then_selection_enters_home`, `workspace_skip_creates_a_distinct_project_before_entering_home`) |
| 20 | Resize | resize | No narrow-width capture exists, and row 6 records `canCollapse:false` for the Settings route. | The settings nav stays at all widths; no collapse is invented. | BOUNDARY (`settings-page-50e3fab957b6.js`; `refs/settings` has 1280 and 1440 px only) |

### Personal instructions and workspace storage

ST-093 opens the personal instructions editor from Personalization.
The host reads `AGENTS.md` under `CODEX_HOME`, or `~/.codex` by default, through `codex-agents-md`.
The editor shows `Loading instructions…`, or `Unable to load instructions` with Retry.
Late replies from an earlier instruction load cannot replace a reopened editor or its current draft.
The editor provides a keyboard-accessible Learn more link to the original instructions guide.
Its textarea grip resizes the field vertically, down to the original's 96 px `min-h-24` box.
ST-094 saves through `codex-agents-md-save`; the host replaces the file atomically and preserves its permissions.
Success shows `Instructions updated`; failure shows `Unable to save instructions` and retains the draft.
ST-095 reads the original `2826489636` feature gate from the authenticated Statsig bootstrap.
The gate keeps the legacy editor unchanged when it is off or unavailable.
When the gate is on, the AGENTS.md editor switches between Markdown preview and source without replacing its draft.
The document editor uses LF internally and restores the first loaded line separator on save.
It preserves CRLF, CR, and LF, including trailing newlines.
This does not change the legacy instructions field.
The editor preserves selection and Undo/Redo history when the feature flag arrives during an edit.
ST-096 reads the sibling `AGENTS.override.md` file and shows the original override notice when that file contains non-whitespace text.
Missing or empty override files do not show the notice.
Other override read failures use the existing instruction load error instead of reporting a false absence.
Cancel discards the draft.
Navigation, Back, window close, and Quit ask `Discard changes?` while edits remain.
Application-wide Quit checks every open Root window before it exits.
After discard, native close uses the existing window lifecycle and retains the final primary window on macOS.
The multiline field resizes vertically and keeps its document while its viewport changes.
The field keeps its border outside the scrolling content.
Typing scrolls the caret into view; the field keeps a minimum height of 96 pixels.
The instructions tests use owned files and cover save, cancel, discard, load failure, and save retry.
The coordinate-based instructions cases run on macOS because Linux uses the no-op text system.
The remaining action and host tests also run on Linux.

A launch opens Home, like the original; saved window size, sidebar, theme, and preferences stay.
An unfinished onboarding page is the only saved route that a launch keeps.
A missing or unreadable window state starts at `SelectWorkspace`.
Skip creates an unused `New project` folder under `~/Documents/ChatGPT`, registers it, then enters Home.
Existing names get a numeric suffix; the host does not overwrite an existing project.
The workspace tests use an owned host root and the native path-prompt test hook.
They do not create folders in the normal profile.

### Memory and Writing

Memory reads the selected computer's configuration and its paginated experimental feature list.
Enabling memory updates `features.memories`, `memories.generate_memories`, and `memories.use_memories`.
Disabling also turns off Chronicle on the local computer.
The tool-assisted switch writes `memories.disable_on_external_context` and removes the legacy alias.
Reads and writes use the user configuration layer when it supplies a file path.
After a partial failure, the page shows the values that the server actually saved.
Delete requires confirmation before `memory/reset`; failure keeps the confirmation available for retry.

Writing is available to a ChatGPT account through its existing authenticated host.
The page reads the canonical Writing plugin and the installed plugin list.
Enable installs or enables that plugin; disable uninstalls it.
A failed operation retries the requested value, not its inverse.
Account identity checks prevent a later response or write from switching accounts.
Enabling Writing replaces Settings with its previous route before it opens setup.
Setup uses the existing discard guard before it replaces an edited instruction document.
Cancel keeps the document and its unsaved draft.
Setup keeps the same account snapshot until it closes.
The setup dialog supports optional app connections through their supplied browser URLs.
A dialog that cannot open leaves Writing retryable instead of waiting for authorization.
Show me opens a prepared draft; it does not start a turn.
### MCP servers, Hooks, and the extension manager

Settings owns the Plugins, Apps, MCPs, Skills, and Marketplace management tabs.
Each Root owns its manager state and dispatch callback; another window cannot replace them.
The manager reads the app-server and keeps confirmed state after each completed operation.
MCP edits preserve fields outside the form, exact argument values, and empty arguments.
Project-owned server definitions stay read-only, and new names cannot replace hidden servers.
`OpenMcpSettings(query)` opens the MCP management tab and applies the supplied search, including when Settings is already open.
Search ignores leading and trailing whitespace.
An unmatched search does not offer an Add server action.
Configured servers use their configuration key before their display name when matching authentication status.
Successful removal hides both names from the stale runtime list until restart.
Form writes use the captured user-file version and retain the draft when that version is stale.
OAuth links must use HTTP or HTTPS.
The `mcpServer/oauthLogin/completed` notification refreshes server status; a refresh queues while another operation is pending.
Hooks stay off until their current definition is trusted.
Managed hooks stay on and cannot be edited.
Marketplace removal uninstalls its remaining plugins before removing the configured source.
Built-in marketplaces cannot be removed.
The shared host reads the original Statsig bootstrap and caches its parsed evaluations for the current Root.
The manager page uses a composited layer only while its content scrolls, as the original does; a short page such as empty Hooks paints directly.

`cargo test -p ui --lib screens::settings::integrations` checks key collisions, form preservation, skill precedence, read-only origins, and hook trust.
`cargo test -p app --lib settings_integrations` checks OAuth boundaries, notification handling, queued status refreshes, stale writes, and window isolation.
An earlier owned-profile application smoke checked Root dispatch against the real app-server on Linux and macOS.
It covered MCP toggling, saving, trust, invalid-save preservation, deletion, marketplace add/remove, and directory navigation.
The native renderer produced a manager screenshot without a new crash report.
Those earlier checks do not validate per-Root state ownership or appearance parity.
Strict form comparisons use the original renderer at the same state and size.
ST-155 remains gated while the native MCP app sandbox is absent; the manager never reports a synthetic data-clear success.


## 15.1 Automations ("Scheduled")

`Root` opens the page from rail `Scheduled` and the command menu's `Manage scheduled tasks`.
It reads `list-automations` and `/suggested_automations` independently through the existing desktop host, off the UI thread.
`CloudHost` reads local tasks from `CODEX_HOME` and uses the app-server's existing sign-in for the read-only suggestion request.
The page shows landing examples while suggestions load or when the suggestion read fails.
The response replaces the cards with server suggestions, followed by distinct landing examples, up to six cards.
Route changes and host replacement cancel the pending suggestion update.
The pending state follows the selected prompt across a late response.
Cards use their emoji when the icon is not bundled.

`parity_automations` replays the genuine `refs/interactions/host.json` response through the actual Root.
All four unchanged strict Automations comparisons pass with an owned `CODEX_HOME` and temporary directory.
The `host` test `scheduled_suggestions_follow_late_reads_and_host_replacement` checks navigation before response handling and prevents old-host cards after replacement.
The new host read does not load oracle or replay files.
This read-only repair does not create tasks, prepare conversations, schedule work, or change accounts.

Slice S035 proves suggestion hover (AM-037) with `refs/interactions/s035.json` and `crates/app/tests/s035.rs`.
The test replays pointer entry and exit on the existing fixture screen with the original server suggestions.
It checks the card fill, the adjacent card, and the icon colors against the Linux route1 recording.
The existing hover implementation already matches; no production code changes are needed.
Slice S035b covers the suggestion click (AM-036) with `refs/interactions/s035b.json` and `crates/app/tests/s035b.rs`.
The original recording shows a failed models read, not a successful guided start.
The native implementation follows the extracted `conversation-actions` start order and the `recommendations` prompts.
It reads models, prepares a local thread, starts its first turn, and opens that thread with the Scheduled sidebar.
`LocalHost` serves `start-suggested-task`; app-server notifications and the normal thread reader supply the transcript.
The card shows `Starting…` and rejects duplicate clicks until the host answers.
A failed start shows `Couldn’t start this suggested task` and leaves the cards and route available.
The start uses read-only permissions and asks for schedule confirmation before creating an automation.
The native path does not call the original cloud `/f/conversation` API.
The original HTTP 499 recording remains unchanged; it does not prove a successful original start.
A headless native check started the selected meal-plan suggestion on a real app-server and completed its guided questions.
It retained the Scheduled sidebar and created no automation.
No successful original screenshot exists for this state, so its pixel comparison is not comparable.
Set `S035B_SHOTS` to retain native failure screenshots outside the Cargo target.
Ordinary local task creation and the existing draft remain unchanged.

`Run now` (AM-021) is the second item of the row `Scheduled task actions` menu, after `Pause` / `Resume`. The original sidebar menu is `q` in `menu-items-19a7bd2f5b31.js`, called with `statusFirst: true` from `actions-b3b1b77c618b.js`; `run-now-11e55df28c95.js` holds only the run action.
Choosing it sends `automation-run-now` with `{id, collaborationMode, permissions}` and disables duplicate starts until the host answers.
The information toast and row show `Starting run…` until the host answers.
`LocalHost` starts one app-server thread per task folder, titles it with the task name, and sends the original's run preamble and the prompt as the first turn.
The new thread then appears in Recents through `thread/started`.
Before each thread the host reads the user's saved config (`config/read` for the folder) and the config requirements, and picks the model from `model/list` like the original (`Pr`): the task's model, else `gpt-5.3-codex`, else the default.
The sandbox and approvals follow the original's agent-mode rules (`Al`): the saved `sandbox_mode` and `approval_policy`, or `auto`; an unattended run sets `never` when the requirements allow it, and the turn's sandbox adds the folder, `$CODEX_HOME/automations/<id>` and the git roots.
The mode the user last picked is read from `agent-mode-by-host-id` in the global state [INFERENCE: the persisted-atom table is assumed].
When the original database exists, a `pending:` row in `automation_runs` opens before the thread, takes the thread id, and is archived (`auto`) if the thread or turn fails to start.
`turn/completed` finishes the matching initial turn through `finish_automation_run(threadId, turn)`.
The bridge keeps the run's initial `notificationPolicy` and `autoArchive` values until completion.
A `failed_runs_only` success keeps its existing read time or becomes read; failure and interruption clear the read time.
A successful cron run with `autoArchive` sends `thread/archive` unless a heartbeat targets the thread.
Only an accepted archive request changes the row to `ARCHIVED` with reason `auto`.
An archive refusal keeps `PENDING_REVIEW` and the completed run's read state.
User start, steer, add-message and queue requests cancel automatic archive before they reach the server.
If archive is in flight or already complete, the bridge restores the thread first and sets the run to `ACCEPTED`.
The per-thread archive lock also prevents a user request from racing with that restore.
No completion path removes a worktree or starts a cleanup service.
A missing store or run row releases only the matching completed tracker, without creating an archive row.
A refusal shows the danger toast `Could not start scheduled task`, with the host error on a separate description line.
A `success: false` result shows the title without a description; a successful start adds no result toast.
Event-triggered tasks (`## When this happens` prompts) have the item disabled with the tooltip `Automations triggered by events can’t be run manually`; the host refuses them too.
The project folders or project target (its first root from `.codex-global-state.json`) run, in a new worktree for the original default environment (`create_worktree` and `set_owner_thread`, removed again when no thread starts) or in the folder for `local`; outside a Git repository the run happens in the folder.
For a project target, the host saves `{projectKind: "local", projectId}` after `thread/start` and worktree ownership.
It saves the assignment before it moves the pending run row or starts the turn.
This follows the original `Vl` and `U0.setAssignment`; it preserves existing projectless membership and unrelated global state.
All native project-state writers use the same locked, atomic `.codex-global-state.json` update.
An assignment failure archives the pending row and sends no turn; the started thread keeps its worktree.
Legacy folder targets do not receive a project assignment, even when a saved project contains the folder.
Projectless tasks use the original `nl`/`Ct` allocation under `projectlessWorkspaceRoot`, or `~/Documents/Codex` when that setting is absent.
The host reads `desktop.projectlessWorkspaceRoot` through `config/read`, matching the General page's configuration write.
Each run gets a local-date folder and a unique prompt-derived folder with `work` and `outputs` subfolders.
The host rejects relative roots and symlinks at the root or local-date folder.
It saves `projectless-thread-ids`, `thread-workspace-root-hints`, and `thread-projectless-output-directories` through the shared atomic global-state writer.
The thread receives the original projectless output instructions and permissions; projectless runs do not create Git worktrees.
A failed start retains the owned folder and archives the pending run; a failed turn retains the thread metadata and archives its run.
Successful projectless runs use the same automatic archive completion path and retain their saved folders and metadata.
The host reads config requirements before it creates a projectless workspace, as in the original `Ll`.
A failed metadata write retains the started thread and its folder without starting a turn.
Heartbeat tasks use the original `Wl` path: read the target thread, check its active flags and recent turns, then resume it.
The renderer supplies the target thread's selected collaboration mode and permissions, not another thread's composer settings.
The heartbeat retains saved approvals when no renderer permission override exists.
Permission resolution fails if the managed requirements cannot be read.
The heartbeat uses the thread's saved effort when no renderer snapshot exists.
The host starts one turn with `automation_heartbeat_run_now` and the original heartbeat XML prompt, without creating or renaming a thread.
Success waits for a matching `turn/started` or `turn/completed` notification.
It opens the existing thread with the Scheduled sidebar only if the original navigation location still matches.
Missing, busy, loading, resume, and turn failures clear `Starting run…`, keep the location, and show the existing failure toast.
An interval heartbeat recalculates its next run from the current run time and does not create a cron run row.
`Root` reads `inbox-items` on startup, after completion, after archive changes, and every minute.
The global unread-run list drives the Scheduled dot on each route's shell.
An explicit chat open marks that local run read; completion alone does not clear its dot.
Archiving a completed run changes its local run status to `ARCHIVED`, without changing its saved read time.
The completion result supplies the `failed_runs_only` policy used for both the saved read state and the desktop notice.
`DONT_NOTIFY` suppresses a notice unless the failed-runs-only policy overrides it; a successful `SKIP_COMPLETION` stays quiet.
Delivery preserves background focus; a notification click opens the exact run chat and dismisses its notice.
The realtime headless `Root` proof keeps real Bridge I/O and checks the actual click target `owned`.
It does not send an alert to the operating system or change account permissions.
The native `automation_run_now::persisted_run_unread_survives_navigation_and_clears_on_open` proof checks persisted unread state, read-state clearing, and archive clearing.
`automation_run_now` checks projectless metadata, project assignment, saved policy, early completion, user cancellation, and archive failures against a stdio peer.
It also checks account isolation, recoverable restore writes, and retained worktrees.
Its native window proof also checks pending state, duplicate suppression, success navigation, failure, Back/Forward, and navigation away.
On macOS, its native row-menu replay starts a task, then displays the real app-server refusal through the same toast component.

`Share` (AM-024) follows `Edit` and precedes `Delete` in the same task menu, after `Pause` / `Resume` and `Run now`, as in the original.
It uses the original `link-light-20` icon and copies `https://chatgpt.com/scheduled?automationId=<id>&automationSource=local`.
The URL encoder protects reserved characters and Unicode in the task ID.
The app shows `Link copied` only when the clipboard reads back the link.
Sharing closes the menu and returns focus to its button without opening the task or sending a network request.
Arrow keys move through the menu; Home and End select its first and last items.
The `automation_share` check uses the headless platform's private clipboard for mouse and keyboard input.
Suggested tasks retain their existing unsupported failure result and never create a task through this action.

`LocalHost` implements the original `automation-update` status change for local tasks (issue https://github.com/i098/codex-gpui/issues/226).
It accepts `{id, status}` or the original full cron or heartbeat body, including edits to `name`, `prompt`, and `rrule`.
Like the original `ugi`, a task without a target sends its first cwd as `projectId`; the host compares that value with the stored cwd.
`PAUSED` clears `next_run_at` and `next_run_nominal_at`.
`ACTIVE` recalculates the original nominal and actual next run with `GF`/`KF`/`JF`/`XF` and the persisted `.run-jitter-salt`; it never reuses a cached time.
A rule without a future run returns the original `Automation schedule has no future runs…` error.
The update keeps the task identity, the definition and the unrelated TOML and database fields, and restores the TOML when the database commit fails.
It returns the original `{item}` shape.
The recurrence uses the vendored `rrule` 0.14.0 with the original limits: `INTERVAL` and `COUNT` up to the JavaScript safe integers, iteration through year 9999, and `BYSECOND=60` moved into the next minute.
The `host` checks match the retained original oracle.

AM-022, AM-023 and AM-028 use the existing sidebar task list.

Task delete (AM-026, AM-027) is the last item of the row menu: the red trash `Delete` after `Share`.
The dialog `Delete {name}?` names the task the host lists, starts on `Cancel`, and sends nothing until `Delete scheduled task`.
Escape and `Cancel` close it without a request and return focus to the row's button; both are ignored while the delete runs.
Confirm sends `automation-delete` through `LocalHost`, which removes the definition directory, its cached `automations` row and its `automation_runs` like the original `aI`.
The answer is `{item, success, status}`; `deleted` and `not_found` succeed, and account-owned rows are never touched.
Success drops the row at once, reads the list again, and closes the deleted task's page to `Scheduled`.
Failure closes the dialog like the original, shows the toast `Could not delete scheduled task` with `Copy error` for the host status, and keeps the task for another try.
The failure toast shows `Try again` for an unsuccessful host response and the error message for a failed host request.
The page-list row's inline delete icon and tooltip (AM-029) belong to the original page list, which the native route does not draw.
`crates/app/tests/task_delete.rs` proves the host contract in disposable stores and, on macOS, the native Escape, Cancel, confirm, failed removal and retry path.
The `Scheduled task actions` menu opens under the row button like the original (right edge 1 px inside the button, 220 px wide); its first item is `Pause` or `Resume`.
The row status control shows `Pause` or `Resume` on hover or keyboard focus, with a named accessible action.
Both send `automation-update` with the original cron or heartbeat request, block duplicate writes and refetch the list.
Success shows `Successfully paused automation` or `Successfully resumed automation`; the row shows `Paused` only after the host reports PAUSED.
An error keeps the stored state and shows `Could not update scheduled task` with the host error description.
`s037::task_status_controls_persist_through_the_real_local_host` pauses through the menu with keyboard events, restarts the app on a fresh `LocalHost`, resumes with the pointer and fails a real definition write.
It checks the TOML, the database row, the recalculated next run and the unchanged task fields.
It uses an isolated headless native Root and a task-owned `CODEX_HOME`, not an installed window.

`LocalHost` also creates the worktree that the default Run now path needs (issue https://github.com/i098/codex-gpui/issues/254).
`create-worktree` and `set-worktree-owner-thread` follow the original git worker (`D2`, `e4`) and the automation start `Ol`.
A cwd outside a Git repository makes no worktree, and the original then runs in the cwd.
The start is the remote default branch, else `HEAD`: the local branch, or its upstream when only the upstream moved, with a local tracking branch for a remote-only start.
The worktree is `<worktrees root>/<4 hex>/<repo>` with the `.codex-worktree-name` marker and a 4-digit suffix when the name is taken; the default root is `$CODEX_HOME/worktrees`.
`git worktree add --detach` runs without clean or smudge filters, and a start branch that is not the default branch is recorded in `codex-synced-branch.json`.
Ignored `AGENTS.override.md` files and the ignored files that `.worktreeinclude` names are copied only as regular files, never through a symlinked directory or over an existing file.
The selected local environment is stored in `codex.localEnvironmentConfigPath`, and its setup script runs in the user's login environment with `CODEX_SOURCE_TREE_PATH` and `CODEX_WORKTREE_PATH`; the exported variables go to `codex-shell-environment.json`.
Like the original, Git's `extensions.worktreeConfig` is enabled in the repository when Git requires it for that store.
A failure removes only the worktree that the call made, after its cleanup script, without `--force`; a worktree with changes is kept and the error says so.
`set-worktree-owner-thread` writes `codex-thread.json` for a linked worktree only.
`crates/app/tests/automation_worktree.rs` proves these effects against disposable repositories and a local bare remote, and the `host::git` unit tests prove the cleanup refusals and the name collision.
Run now calls `create_worktree` and `set_owner_thread` directly for worktree tasks.

The sidebar New task options menu offers `Create with Codex`, `Create with ChatGPT`, and `Set up manually`.
The chat actions open a new native chat with the original setup prompt.
They do not send a turn until the user submits the composer.
The manual editor uses the existing Composer text fields for Name and multiline Prompt.
It supports Interval, Daily, Weekdays, Weekly, Monthly, and Custom schedules.
Custom supports Hourly, Daily, Weekly, Monthly, and Yearly, with the matching time, interval, weekday, month-day, month, and minute controls.
Morning, Afternoon, Evening, and Night select 09:00, 14:00, 18:00, and 22:00.
Empty names, empty prompts, invalid numbers, invalid times, and empty weekly day selections disable Save.
The existing recurrence parser validates the host request.
The editor retains the draft on a write error and reloads the saved task after success.
Row Edit loads the stored Name, Prompt, and recurrence into the same editor.
Text-only edits retain an existing advanced recurrence without rewriting it.
`automation-update` writes Name, Prompt, and recurrence to the original TOML and database stores while preserving unrelated fields and rollback.
A profile without the original database saves the definition without creating a parallel database.
`task_create::editor` exercises the native fields, menu, schedule controls, failed write, create, edit, and fresh-host read.
`host` checks persistence across the accepted recurrence variants and rejects invalid changes before writing.


| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Rail `Scheduled` | entry | Automations page, `ScheduledSidebar`. | `Route::Scheduled` with `ScheduledSidebar` and the page. | PASS (`rail_scheduled`, `parity_automations`) |
| 2 | Deep link `/scheduled?automationId=…` | entry | Page with the task selected. | `codex://scheduled?automationId=…` parses to `Route::Automation`; a cold start rebuilds the page when `list-automations` answers, so the task is selected. | PASS (`deep_link::settings_automations_and_launch_links_route`, `scheduled_list::cold_link_selects_its_task_and_its_runs_open_their_chat`) |
| 3 | `New task` | click | Create flow. | The recorded compact schedule dialog saves through `automation-create` and retains the draft on a write error. | PASS (`task_create`, `s036`) |
| 4 | `Create` menu | click | Create with Codex / ChatGPT / manually. | The sidebar split control opens both chat actions with the original setup prompt, or the full manual editor. | PASS (`task_create::editor`) |
| 5 | Search | focus | Filters; `No matching tasks`. | `Search scheduled tasks` opens a field under the header; the name filter is case-insensitive, `No matching tasks` shows when nothing matches, and Escape closes the field. | PASS (`scheduled_list::search_and_filter_narrow_the_list`) |
| 6 | Status filter | click | `Filter by`: Active / Paused / Completed; a single inactive choice renames `Upcoming`. | The same checkbox menu with arrow, Home, End, Enter and Escape keys; a selected paused task stays listed. | PASS (`scheduled_list`) |
| 7 | Task row | click | Detail panel. | Selects the task and opens its thread. | PASS (`s037::task_row_hover_and_click_match_original`) |
| 8 | Row actions | click | Edit, Run now, Pause, Share, Delete. | Edit opens the stored definition in the full editor; Run now and Pause / Resume retain their existing host paths; Share copies the task link. Delete has a separate owner. | PARTIAL (`task_create::editor`, `automation_run_now`, `s037`, `automation_share`) |
| 9 | Editor fields | focus | Name, Prompt, schedule. | Editable Name and multiline Prompt, six schedule modes, custom frequencies, time presets, and day fields persist through the existing host. | PASS (`task_create::editor`, `host`) |
| 10 | Previous runs | click | Run chat, mark read, archive. | The selected cron task lists its `inbox-items` runs (three, then `Show older`); a run opens its chat with the task kept in the list. Mark read and archive have a separate owner. | PARTIAL (`scheduled_list::cold_link_selects_its_task_and_its_runs_open_their_chat`) |
| 11 | Row hover | hover | Row highlight. | The recorded row fill and actions match; the status icon changes to Pause on hover or keyboard focus. | PASS (`s037::task_row_hover_and_click_match_original`; suggestion hover: `s035::suggestion_hover_matches_original`) |
| 12 | Focus ring | focus ring | 2 px ring. | Task rows, search and filter buttons draw the shared keyboard ring. | PASS (`crates/ui/src/screens/automations.rs`) |
| 13 | No tasks | empty | `No scheduled tasks`, suggestions. | `No scheduled tasks`, server suggestions and distinct landing examples, up to six cards. | PASS (`rail_scheduled`, `parity_automations`, `host::scheduled_suggestions_follow_late_reads_and_host_replacement`) |
| 14 | Loading | loading | `Loading scheduled tasks…`. | Shown until the first `list-automations` answer. | PASS (`scheduled_list`) |
| 15 | Error | error | `Scheduled tasks could not be loaded`. | Shown with `Try again`, which reads the list again. | PASS (`scheduled_list::unreadable_list_fails_and_try_again_loads_it`) |
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

`cargo test -p app --test terminal_states -- --test-threads=1` exercises the native terminal states on macOS.
The replay captures the empty view, streamed output, retained failed output, preview, focus ring, stop failure and successful cleanup.
It resizes the window to 1280, 767 and 600 px.
`cargo test -p ui --lib` checks Unicode refit and retained failed output without a native window.
Reference captures keep composer focus and hide the caret.
The change card clips overflowing stats to the original subtitle width.
Stop sends one thread-scoped request and waits for command completion notifications before removing commands from the running list.
A refused request keeps both commands and permits another attempt.
The failure toast paints above the open summary.
These checks use isolated replay data and do not stop real processes.
They do not replace original-versus-native appearance approval or the strict pixel gate.

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Thread with a running background command | entry | `Background terminal` entry in the thread. | `Toggle summary` lists it under `Background processes`, as refs/terminal shows. | PASS (`background_terminal_tab`, `parity_terminal`) |
| 2 | Click the entry | click | Opens the side panel tab. | `OpenBackgroundTerminal`: the command's tab (`Xterm`) in the side panel. | PASS (`background_terminal_tab`, `parity_terminal`) |
| 3 | Output streaming | loading | Appends `outputDelta`; `No output yet`. | Shows the empty placeholder, then the selected command's streamed output. | PASS (`terminal_states`) |
| 4 | `Stop all background terminals` | click | `thread/backgroundTerminals/clean`. | The summary action sends the thread ID; pending requests cannot send a duplicate. | PASS (`terminal_states`) |
| 5 | Stop failure | error | Toast. | Shows `Unable to stop background terminals` above the summary and retains the running commands. | PASS (`terminal_states`) |
| 6 | Tab close | click | Closes the tab; `⌘W`. | The tab's `×` or `⌘W` closes the selected tab; the next tab, else the previous, is selected; the last tab closes the panel. | PASS (`s029::panel_tabs_replay`, `background_terminal_tab`, `changes_panel_keys`) |
| 7 | Tab thumbnail | hover | Thumbnail preview. | Shows the command title and the last eight rendered output rows. | PASS (`terminal_states`, native capture) |
| 8 | Command finishes | empty | Leaves the running list; buffer kept. | Keeps the selected item and its streamed output, including a failure without a final output snapshot. | PASS (`terminal_states`) |
| 9 | Focus ring | focus ring | 2 px ring. | Uses the shared 2 px overlay ring for tabs and close buttons; keyboard activation and close work. | PASS (`terminal_states`) |
| 10 | Resize | resize | xterm refits columns. | Refits read-only output to the panel width without changing its stored buffer. | PASS (`terminal_states`, Unicode refit check) |

## 15.3 Mobile setup

`Root` opens the page from the profile menu's `Help` ▸ `Set up mobile` (`Route::Mobile`).

| # | Item | Kind | Original | Port | Status |
|---|---|---|---|---|---|
| 1 | Profile menu → Help → `Set up mobile` | entry | `/codex-mobile`. | `Route::Mobile`: sidebar closed, no rail item selected. | PASS (`profile_menu_mobile`, `parity_mobile`) |
| 2 | Initial step | click | `Get started` reads `/wham/remote/control/mfa_requirement`, then on `required` `/accounts/mfa_info`; `Later` (dialog only). | `GetStarted`: loading button; `required` with `mfa_enabled_v2` off opens the MFA step, otherwise the allow-host step; a failed read or a body without the expected field shows `Couldn’t check security requirements. Try again`. | PASS (`mobile_get_started_follows_the_recorded_mfa_status`) |
| 3 | Allow host step | click | `Allow`: `remoteControl/enable`. | `Allow` sends `remoteControl/enable` (loading step while it runs), waits for `connected`, then starts pairing. | PASS (`allow_enables_remote_control_and_a_claimed_code_connects` in `mobile_pairing`) |
| 4 | MFA required | error | `Turn on Multi-Factor Authentication`, `Continue on chatgpt.com`. | Step view; the button opens `chatgpt.com/#settings/Security`. | BOUNDARY (the recorded account has MFA on, and no MFA-off response is recorded, so no fixture reaches it) |
| 5 | Waiting / pairing step | loading | QR code, pairing code. | Waiting step after `Allow`: `remoteControl/pairing/start` fills the QR code and manual code; `pairing/status` is polled each second. | PASS (same flow) |
| 6 | Pairing unavailable | error | `Pairing code unavailable`. | A failed `pairing/start` keeps the waiting step and shows `Pairing code unavailable: <error>` with `Refresh pairing code`. | PASS (`a_failed_pairing_start_keeps_the_waiting_step_with_its_error` in `mobile_pairing`) |
| 7 | Connected step | click | Toggles, `Done`, `Manage connections`. | A claimed code with a new client opens `You’re connected` with the toggles, `Done` (back to Home) and `Manage connections` (Settings > Connections). | PASS (same flow as row 3; toggle clicks and `Manage connections` are not driven) |
| 8 | Button hover | hover | Ghost/solid hover. | `Get started` hover fill (55250 px repainted). | PASS (`mobile_button_hover`) |
| 9 | Focus ring | focus ring | 2 px ring (`focus-visible:ring-2 ring-offset-0`). | Every step button (`Get started`, `Later`, `Allow`, `Refresh pairing code`, `Continue on chatgpt.com`, `Done`, `Manage connections`) is a tab stop and draws the shared 2 px `#3a83f7` ring only under keyboard focus; a pointer press clears it. The connected step's switches have no ring yet. | PASS (`mobile_focus_ring`; no original capture of the focused button) |
| 10 | Resize | resize | Content stays centered; the artwork shows only at 1024 px and wider (`hidden lg:block`). | At 1280 px the column centers in its half beside the artwork; at 767 and 600 px the artwork is hidden and the column centers in the whole main surface. | PASS (`mobile_resize_three_widths`) |

## Changes needed outside crates/app

A row above that names a `crates/ui` owner needs that change first. The changes that unblock the most rows:

1. `crates/ui/src/thread.rs`: click handlers on the reasoning and command cards, `Reapply`, the user message's hover actions, a find bar, and loading and error views.
2. `crates/ui/src/diff.rs`: the Changes header controls (source selector, file tree, layout, wrap, refresh), empty, loading and error states.
3. Focus-visible rings on the shared buttons (`icon_button`, `pill`, rail items), and tooltips on the composer and sidebar row buttons.

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

Activity uses `HomeSidebar` for selection, unread state, running state, rows, and paging.
Its session keeps priority items after reading or completion and retains the original recent timestamps until the next activation.
The options change Priority, Pinned, and Scheduled visibility, mark items read, and confirm archive requests.
The native S003 checks cover those transitions and the initial-list recovery path.
`priority_activity` checks keyboard navigation from a hidden sidebar and Settings without requiring a GPU.
`ui`'s `screens::sidebar::priority::tests` checks the session snapshot and the seven-day boundary.
The active bell preserves the Oklab color conversion's blue-channel overshoot and 13% alpha for the float rendering path.
Its edges match the original in both themes; the shared sidebar corner still differs by seven dark and three light pixels.


The original hides timestamps in every Home sidebar list, including project groups.
It shows an unread dot after a failed background turn, rather than an error icon.
`refs/interactions/s003_sections.json` records a non-final custom section with fifteen local chats.
`Show more` changes ten visible rows to fifteen; `Show less` restores ten rows.
A header click hides the section; reopening keeps its expanded list.
Default Recents retains infinite paging and has no paging button.
The recorded input check verifies all five clicks, unchanged Recents, and no app-server requests.
The dark paging label uses the original generated theme alpha, `0.498`; the light label stays unchanged.
Each paging label gets an independent transparent layer; the default sidebar drawing stays unchanged.
Both paging-label image checks pass with zero differing pixels in every recorded More/Less state.
The references, text rectangles, and limits remain unchanged.
Section creation and new storage remain outside RS-066.

### Custom section header actions

`crates/app/tests/section_actions.rs` drives section actions and section-targeted chat creation.
The header shows Options and New chat when the pointer or keyboard focus enters it.
Options contains Edit, Mark all as read, Archive chats, New section, and Remove section.
On macOS, Options opens an AppKit `NSMenu` with 16-pixel template icons and the original button-bottom-left anchor.
`HomeSidebar::native_menu_snapshot` exposes the actual menu readback, including its open state, native items, and popup anchor.
Mark all as read requires unread members; Archive chats requires at least one member.
Remove section preserves its chats.
The app loads sections through `threadSection/list` and their ordered members through `thread/list`.
Edit, Create, and Remove use `threadSection/update`, `threadSection/create`, and `threadSection/delete`.
New chat selects an available section project, opens an empty composer, and assigns the resulting chat through `thread/section/move` before `turn/start`.
Ordinary New chat clears the section target.
The native check covers section edits, default naming, removal, read state, archive cancellation, bulk archive, and chat assignment order.


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

## S025 ranking: Command-search score and pointer selection

`crates/app/tests/s025_rank.rs` replays `refs/interactions/s025_rank.json` on the background Mac lane.
The original capture records twelve ambiguous queries before the ordering change.
The matcher uses the original `_4o` and `k2` score rules over UTF-16 units, including emoji title ordering.
Settings comparisons retain original scores without fixing an incidental order for equal-score rows.
Empty-query groups retain the S026 record.
Nonempty queries put suggested commands in their original command groups.

The replay checks arrow selection across groups, hover fill, input focus, and selection reset after typing.
The destination checks compare rendered heading glyphs with the original Worktrees, General, and Configuration records.
Enter opens Worktrees.
Row clicks open General and the Configuration destination for Work preferences.
The original `chat` query selects the greeting chat.
That recorded state replaces the obsolete, unrecorded S026 assumption that New chat stays selected.
The original S026 group, request, and failed-search checks remain.
The S025 caret and selection replay remains unchanged.

The test reuses the unchanged original search response selected by the fixture server.
The capture changes no renderer code or startup response.
The first query outcomes precede the port change; a second record corrects native entry and pointer transport.
The trusted capture uses native ref clicks, native keyboard input, and atomic owned-target CDP pointer events.
The original also shows an unrelated ChatGPT search error and a later Create page provider.
This slice does not add either provider or settings-page content.
The record includes the earlier raw New chat attempt.
Atomic native clicks prove General and Configuration activation.

All 36 strict static cases pass with the original thresholds.
Their metrics and port PNGs match the baseline exactly.
The [hosted strict report](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/6a2e81ed2b8a20f663537c3e11dde75b1fc541b2604d0a89/report.html) retains those thresholds.

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
The replay owns its Git repository and waits for its recorded commit availability.
It does not depend on the shared Mac temporary project.

## Native file search and detached chats

`crates/app/tests/command_destinations.rs` drives the current Root through keyboard and command-menu input.
File search ignores responses for another session or an old query.
The file index starts when a fuzzy query needs it.
A refused index start can be retried without losing the query.
Closing the menu releases its index, including when a dialog replaces it.
Selecting a match opens the existing file viewer.
Absolute path lookup rejects directories.
Only successfully loaded files enter the shared recent-file history.
The history stays available across windows and trailing directory separators.
The detached chat test closes the first window, then receives another backend turn in the remaining window.
The bridge regression also checks both event receivers and connection closure.
These checks do not complete the other command products.

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

## F1d projects: Worktree and missing-folder markers

`crates/app/tests/f1d_projects.rs` replays `refs/interactions/f1d_projects.json` through the F4 host.
The original fixture contains a normal project, a managed worktree, and a missing folder.
Its collapsed-state values come from actual original row clicks.

The local and cloud hosts read the original `.codex-global-state.json` project and atom keys without rewriting the file.
Rows keep trimmed names and the original first-root marker rules.
Roots absent from `project-order` appear first; listed roots follow the saved order.
A missing path response does not mark every project as deleted.
The tests check folder deletion and restoration, malformed entries, ordering, and the `worktrees-extra` prefix boundary.
A failed filesystem probe marks only its own root as missing, without hiding healthy projects.

The worktree icon reuses `thread-fork.svg` inside the original 12.5px superellipse overflow clip.
That clip is required for the unchanged native glyph pixels to match exactly.
The missing row uses the original 0.7 opacity and disabled-action tooltip.
Both tooltip labels, bounds, dismissal, and `-0.15px` tracking follow the original recording.
The worktree tooltip retains the original zero-size `contents` wrapper placement at the window origin.
Long project names stay clear of markers and hover actions.
The label's stable element ID makes GPUI apply hover dimensions during layout.
An image check protects the control regions for 100-character names at rest and on hover.

Default Recents, pins, status indicators, and paging retain their existing checks.
Project menus, hover-card actions, and project thread paging are outside RS-081.

RS-069 and RS-070 select an existing project in the sidebar.
A click on a row, or on its hover `new-chat` icon, dispatches `SelectProject(root)` with the project's first stored root path.
`Root::select_project` sets `State.project_root` (persisted by the existing state save), the Home chip and the file-search root, then opens Home.
The icon also runs `NewChat`, so the composer is empty and focused.
`thread/start` takes its `cwd` from `State.project_root`.
Open chats keep the `cwd` the server stored with them, and selecting a project does not change them.
A missing folder, or a project with no root path, ignores both clicks.
`project_picker` clicks two owned projects and a deleted one, and checks the logged `thread/start` working directories.
The row click does not expand or collapse the folder, because the port lists no per-project chats.
The shared replay selects the dedicated `f1d_projects` adapter.

Run `tools/mac.sh -l f1d-projects 'cargo test -p app --test f1d_projects -- --test-threads=1'`.


| Recorded region | Original | Port |
|---|---|---|
| Project rows | [Image](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/ff8c28f83516f4fb8545d34d04a67ab08498485936cb345b/project-markers/original-rows.png) | [Image](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/ff8c28f83516f4fb8545d34d04a67ab08498485936cb345b/project-markers/port-rows.png) |
| Worktree tooltip | [Image](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/ff8c28f83516f4fb8545d34d04a67ab08498485936cb345b/project-markers/original-worktree-tooltip.png) | [Image](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/ff8c28f83516f4fb8545d34d04a67ab08498485936cb345b/project-markers/port-worktree-tooltip.png) |
| Missing-folder tooltip | [Image](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/ff8c28f83516f4fb8545d34d04a67ab08498485936cb345b/project-markers/original-missing-tooltip.png) | [Image](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/ff8c28f83516f4fb8545d34d04a67ab08498485936cb345b/project-markers/port-missing-tooltip.png) |

The [strict static report](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/ff8c28f83516f4fb8545d34d04a67ab08498485936cb345b/report.html) passes all 36 cases with zero outside-text differences.


## Native project selection and chat directories

`Choose project`, the selected Home title, `⌘O`, `⌥⇧⌘O`, File > Open Folder, and the `Open folder` command use GPUI's folder picker.
The picker accepts one folder and does not change the selection or the current chat when canceled.
The local host adds the folder to the existing `local-projects` and `project-order` records.
It retains other projects and unrelated values in `.codex-global-state.json`.
Selecting an existing root does not add a duplicate project.
The sidebar uses the same host project rows as the marker flow.

The app saves the selected new-chat root in `state.json`.
The Home title, project chip, and file search use that selection.
Changing roots stops the old file-search session, removes its results, and restarts the current query without clearing the draft.
Home and thread composers accept only search replies for their own session.
`thread/start` captures the selected root before its asynchronous request.
Existing chats retain the directory from their thread data.
`thread/resume` sends only the thread ID, without a selected-folder override.
Changing the Home selection does not change another chat's directory.

`crates/app/tests/project_picker.rs` checks two temporary repositories, cancellation, reopening an existing chat, new-chat requests, and saved selection.
It also checks project reuse, host-state preservation, and invalid-folder errors.
The Mac flow uses native GPUI mouse and key events with the supported headless picker response API.
It captures the selected roots, the new chat, and the reopened app.
The physical macOS folder dialog is not exercised by this headless check.
Project dropdowns, worktrees, remote environments, discovery, indexing, and Git actions remain outside this native-folder slice.

Run `tools/mac.sh -l cgx-project-picker 'cargo test -p app --test project_picker -- --test-threads=1'`.

A refused `thread/start` from Home (CP-120) shows the `Error starting chat` toast and stays on Home.
The Home composer takes back the submitted text and the submission's attachments, ahead of anything typed since.
The model, effort, permission choices and the selected project did not change, so a second send starts the chat in the same project with the same attachment.
The toast carries only that title; the app-server's error text goes to the process log.
Text with mention chips comes back as the plain prompt text that was sent, as in the thread composer's refused-send path.
A refused `turn/start` after a successful `thread/start` is a separate case and is not handled here.

`crates/app/tests/start_failure.rs` selects a temporary project, attaches a file to the Home composer and sends a prompt that the replayed app-server refuses.
It then checks the toast, the route, the restored text, and that a second send starts in the same project with the same attachment.

Run `tools/mac.sh -l cgx-asap-workable 'cargo test -p app --test start_failure -- --test-threads=1'`.

### New Project (SH-093)

File > New Project… and the `New project` command open the native `Create project` form.
The form has a name field, a marker button, ordered source folders, a device picker, `Cancel` and `Create project`.
The marker picker offers the 30 schema icons and 8 colors of `PROJECT_APPEARANCES`; `Clear` restores the folder default.
The original emoji and icon browser is not recorded, so the native picker is a grid.
`Add` and `Add folder` open GPUI's multi-folder picker; new folders go after the current ones and duplicates are skipped.
The first folder is primary; `Make primary` moves a folder first, and the close button removes it.
The device menu lists this computer and connected remotes; `Add remote` opens Settings > Connections at SSH.
Remote rows are disabled because the native app has no remote-project store yet.

`Create project` asks `thread/list` for chats in the chosen folders, then calls `Host::create_project`.
The local host first pins each such chat that has no assignment to its current single owner project.
It then reuses a project with the same roots or adds one, saves the appearance, puts the project first in `project-order` and sets `selected-project`.
Without folders, it makes a `name` folder (or `name 2`, …) under `~/Documents/ChatGPT`.
A failure shows `Failed to create project`, keeps the form open with its input, and removes a folder it made.
`crates/app/tests/project_picker.rs` checks root order, appearance, ownership pins, reuse, default folders and failure.
`crates/ui/src/screens/new_project.rs` checks source order and the stored appearance.


## F3c: Model announcement (SH-142)

`crates/app/tests/f3c.rs` checks the recorded GPT-6.1 Sol announcement on the Mac headless app.
The recording is `refs/interactions/f3c.json`.
The dialog keeps the original image, title, message, button labels, focus order, and dismissal controls.
The tests cover both primary responses, the secondary button, Escape, Close, outside clicks, and the stored once-only history.

The primary action writes the model and reasoning effort with `config/batchWrite`.
A successful write updates only the two model fields in each composer.
Permissions, queue mode, Reset defaults and drafts remain unchanged.
A rejected write keeps the current model and shows the original error message.
`Root` stores the model ID for both outcomes.
`seen-model-upgrade-list` uses the existing global host-settings store.
Each dismissal test reads the resulting file and starts a new app context without changing that file.
The new context does not show the same announcement.

The shared replay uses the recording's empty initial history.
Ordinary replay data keeps the original baseline history from `tools/route1/oracle.json`.
The ordinary checks use the separate fixture-preparation prerequisite.
No original profile, sign-in flow, or legacy dialog assertion changes.

[Original announcement](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/ddae05ee78165eadfc0a754e40daeb9a03ee71e3bb4b0d16/original-announcement.png).
[Port announcement](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/ddae05ee78165eadfc0a754e40daeb9a03ee71e3bb4b0d16/announcement-open.png).
The retained port image uses the same dialog source as the integrated feature.
The [strict static report](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/ddae05ee78165eadfc0a754e40daeb9a03ee71e3bb4b0d16/report.html) passes all 36 cases with zero outside-text differences.
The scoped issue is [SH-142](https://github.com/i098/codex-gpui/issues/184).

## Shared tooltip side offset

`Tooltip::side_offset` sets the gap from one trigger's absolute-positioning edge.
The default remains 2 px; other callers keep their positions.

The native original uses a 1280×832 viewport at scale 2.
Its Add trigger has bounds `[448.25, 776, 28, 28]`, zero padding, and a 1 px border.
Its tooltip has vertical bounds `742..772`, with a 4 px gap above the trigger.
The same capture places Search's tooltip 2 px below its trigger.

The shared bordered pill positions its absolute child at y777.
The S018 caller can use `.side_offset(px(5.))` to match the original Add tooltip without moving the trigger.
Add rendering and query changes stay in S018.

`crates/app/tests/f1a.rs` checks the bordered pill at both offsets.
The selected offset gives vertical bounds `742..772`; the unchanged default gives `745..775`.
The existing recording still checks all 34 actions and 13 default tooltips.
The F1a fixture loads `common::recorded_host_settings()` before `Root` starts, matching the recorded model-upgrade history.

The [native original capture](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/b3cf204103d640697ec41c58541aab67d5f63b6dde0c7fba/original-add.png) retains the original Add tooltip.
The [port capture](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/b3cf204103d640697ec41c58541aab67d5f63b6dde0c7fba/s018-add-after.png) uses the S018 smoke snapshot with the offset setter.
Both captures use the same Mac viewport and scale.

## Shared tooltip keyboard focus API

`Tooltip::focus_handle(&gpui::FocusHandle)` uses the trigger's existing focus handle.
Set it when the trigger is first rendered.
Keyboard focus opens the shared tooltip immediately.
Blur and Escape close it without moving focus.
Pointer focus does not reopen it.
Hover replaces the visible provider owner, and Escape closes that active owner.

`crates/app/tests/f1a.rs` checks this lifecycle in an isolated native fixture.
The fixture uses real pointer entry before Tab and changes only the simulated TestWindow activity.
It renders the focus event and the notified tooltip frame.
It checks the painted tooltip fill after keyboard focus.
It checks that the previous pill no longer paints after hover takes ownership.
The default hover delay and `side_offset` geometry remain unchanged.

This API does not complete Home focus behavior or add the separate F1d overflow predicate.
The original Home inputs and edge probes remain unchanged.
The scoped issue is [Add an opt-in focus API to the shared tooltip](https://github.com/i098/codex-gpui/issues/186).

## Shortcut navigation (SH-044, SH-058, SH-069)

Control-Tab and Control-Shift-Tab open the command menu in `Recently viewed` mode.
The picker omits the search field and number labels.
Repeated shortcuts move the selection without changing the open chat.
Releasing Control opens the selected chat; Escape cancels.
The picker keeps viewed order separate from the updated order used by Command-Option-number.

Mouse Back and Forward use the same route history as Command-[ and Command-].
Control-Shift-G opens the existing Changes tab.
`crates/app/tests/shortcut_navigation.rs` checks these paths on recorded threads.
The [original/native picker pair](https://pub-c548cd7bc64c41ae8ce0d860578f3b6f.r2.dev/9a2941014013a3237d6c7389b13d9c4a3a00e15a0eba9abb/report.html) uses a 520×134 CSS-pixel crop at scale 2.
It has zero outside-text differences and 45 differing pixels inside 147631 text pixels (0.0305%).

## Storage settings (ST-159, ST-160, ST-161)

`Settings > Storage` measures the app-server's Codex home without following file symlinks.
The usage bar separates chats, subagents, archived chats, unclassified chats, worktrees, and other files.
Refresh reports completed categories separately from a failed or active scan.
It keeps cached category values until each replacement measurement completes.
The page uses actual file lengths and app-server thread metadata.

`Clean up old chats` requires descendant discovery support.
The review starts with archived chats last updated more than 90 days ago.
The age filter also offers 30, 180, and 365 days; the scope filter also offers all chats.
Only inactive, non-ephemeral roots and complete, inactive descendant trees can enter the selection.
The final confirmation warns that deletion is permanent.
Before deleting, the app reloads the inventory, discovers descendants, and checks each thread's update time and active state.
It skips changed trees and reports deleted, skipped, and failed root counts.
The deletion request goes through `thread/delete`; the UI never removes rollout files directly.
The cleanup lock and result toast survive navigation away from Storage.
Keyboard activation runs once per completed Enter or Space press.
Deleting a chat clears its cached view, so Back cannot reuse the deleted chat.

The management controls route through `Root::open_settings_section` for Worktrees and Archived chats.
Their destination pages remain owned by their separate settings families.
The focused checks run with `cargo test -p bridge -p ui --lib storage -- --test-threads=1`.
They cover measured categories, partial metadata, symlink exclusion, descendant safety, version eligibility, cached refresh state, and keyboard cancellation.
The `storage_capture` test also checks cached-view invalidation after a deletion notification.
The UI unit captures use synthetic interaction data and do not establish pixel parity.
The ignored `storage_capture` integration test captures the real Root and can apply the unchanged parity gate.
Set `CGX_STORAGE_HOME` to an owned fixture, `CGX_STORAGE_CAPTURE` to its output directory, and `CGX_STORAGE_REFS` to the original references.
`CGX_STORAGE_REPLAY` supplies additional recorded read responses; `CGX_STORAGE_REVIEW_Y` selects the cleanup-dialog capture.
`CGX_STORAGE_ACTIONS` reads an ordered JSON array of `[name, [x, y]]` clicks for additional full-window captures.
Compare those extra images with their matching original states; this option does not replace the default state's gate.

## Keyboard preferences and app actions

The Keyboard shortcuts page uses the original Codex command registry and flat-card layout.
Text search includes commands and the browser-link preference. Keystroke search filters effective bindings.
The existing host settings store saves bindings, the number target, and the browser-link modifier.
Conflicts and failed writes leave the previous bindings active.
Reset restores default bindings and the link modifier without changing the number target.
App bindings update immediately, including menu and sidebar labels.
Unmodified app shortcuts do not intercept editable text.
Focused binding capture blocks normal key dispatch and publishes `KeyboardState.capture_owner` for the global-shortcut owner.

Standalone chats use the existing projectless workspace allocator and app-server requests.
Temporary chats send `ephemeral: true` and stay out of recents and saved route state.
Undo and redo apply recorded rename, section, and archive actions through their app-server methods.
A failed undo retains its entry for another attempt.
Clear all unreads uses a confirmation dialog and saves the optional confirmation preference.

The interactive terminal keeps its real PTY process when the bottom panel hides.
Chat drafts that begin with `!` run through `thread/shellCommand` after a native warning confirmation.
This method runs outside the sandbox, so only Full access chats enable it.
Other permission modes show the reason and keep the draft.
Home, cloud-task, and message-edit composers keep `!` as ordinary text.
Cancel keeps the draft; an RPC failure restores it.
Shell output and exit status use the existing command-execution transcript, not the native PTY panel.
The `chat_shell` flow covers permission denial, cancellation, shell syntax, and RPC failure.
UI zoom keeps layout, painting, pointer input, and IME bounds in the same coordinate system.
The zoom banner waits two seconds and remains visible while the pointer is over it.
Reopening a saved window restores its zoom.
Original-versus-native visual approval remains separate from the Linux functional checks.


Native checks cover reset confirmation, number/link menus, and modifier-only capture in `keyboard_shortcuts`.
The `project_picker` scenario checks folder selection, cancellation, and the directory used by new chats.
The `shortcut_actions` checks cover attention navigation, undo/redo, unread confirmation, and live terminal retention.

## Feedback, transcript export, and recap (S13)

`/feedback` opens a native form without sending anything.
`Send feedback` calls `feedback/upload`; `Save ZIP` calls `feedback/save`.
Both use the connected app-server and the current thread ID, when available.
Diagnostic logs require explicit selection.
Saving requires an unused absolute `.zip` path on the app-server host, not the client host.
A server refusal keeps the form and its input.
Cancel closes the form during a pending request without canceling the server request.
Late responses do not close a newer form or move keyboard focus.

`/export` reads the complete thread with `thread/read` and `includeTurns: true`.
The native save picker selects a local Markdown file.
The export preserves messages, attachments, activity items, and errors.
It refuses to overwrite an existing file.
`/recap` starts a normal `turn/start` on the current thread after any required resume.
It keeps the existing history, model permissions, and composer draft.
It refuses to run while another turn is active and does not call compaction.
The recap prompt asks the model not to use tools; the existing permissions remain in force.

`crates/app/tests/feedback.rs` covers the native actions, consent, server paths, and file safety.
Live protocol evidence uses Codex CLI `0.164.0-alpha.11.1`.
The synthetic conversation retained both turns after the recap.
The real server accepted `feedback/save` and `feedback/upload` with logs excluded.
