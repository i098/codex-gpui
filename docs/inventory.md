# Codex desktop app: screen and flow inventory

This document lists the screens, flows, app-server methods, IPC channels and shortcuts of the original app (bundle `com.openai.codex`, version 26.1007.30912).
It is the input for the GPUI port and for the flow parity pass.
v1 has 9 screens: Home, Sidebar, Thread view, Composer, Diff view, Settings (+ login/onboarding), Automations, Background terminal, Mobile setup (sections 4–9 and 15).
All chunk file names refer to `/tmp/cgx/asar-x/webview/assets/` unless the path starts with `.vite/build/`.
Text in backticks such as `sidebarElectron.newThread` is an i18n message id from the bundle; the English default text follows where useful.
A line marked `[INFERENCE]` is not proven by a bundle string or a reference screenshot.

## 1. Method

### 1.1 REA run

- Subset: `/tmp/cgx/e1-subset` (31 MB, 540 files): `webview/index.html`, `package.json`, `.vite/build/preload.js`, `.vite/build/main-CqEoQDOU.js`, `.vite/build/bootstrap-CooIJpZm.js`, `.vite/build/early-bootstrap.js`, and 534 renderer chunks selected by name (shell, home, composer, thread, diff, review, settings, login, onboarding, sidebar, rail destinations) and by content (`What should we build?`, `Do anything`, `Choose project`, `thread/start`, `turn/start`, `thread/list`).
- Command 1: `npx -y rea-agents@latest --help` (2.0 s).
- Command 2: `npx -y rea-agents@latest analyze-javascript-application /tmp/cgx/e1-subset --artifact-format directory --max-heap-mb 8192 --analysis-timeout-ms 1800000 --format json` (wall clock 7 min 56 s, peak RSS 3.4 GB, 538 files parsed, 8,051,419 AST nodes, 0 parse failures).
- Command 3: `npx -y rea-agents@latest inspect-analysis-view …` (three tries, about 2 s each); it needs inline or session-retained Evidence, so it gave no data for a 229 MB file.
- Raw output: `/tmp/cgx/e1-out/analyze-javascript-application.json` (229 MB) and `/tmp/cgx/e1-out/analyze.stderr` (progress log).
- REA result used here: 3 context-bridge APIs (`electronBridge`, `codexWindowType`, `__SENTRY_IPC__`), 25 preload IPC operations (20 sends, 5 listeners, 24 with dynamic channel names, 1 literal), 0 statically paired main handlers (main registers handlers through module members such as `g.wt`).
- Condensed REA facts: `/tmp/cgx/e1-out/rea-summary.json`.
- REA does not resolve the channel constants, the message `type` union, the routes or the JSON-RPC methods of this bundle, so targeted greps and a small Python extractor over the full bundle filled these gaps.
- Extractor outputs: `/tmp/cgx/e1-out/i18n-messages.json` (all non-locale `id`/`defaultMessage` pairs with chunk files), `/tmp/cgx/e1-out/route-table.js` (router source slice), `/tmp/cgx/e1-out/git-worker-cases.txt`, `/tmp/cgx/e1-out/methods-grep.txt`.
- Machine-readable copy of the tables below: `docs/inventory.json`.

### 1.2 Core chunks

- `app-initial-6350d04672d0.js` (11.4 MB): router, app shell, left rail, sidebar, command registry, keyboard handling, settings registry, most i18n strings.
- `app-primary-6f60e9ac4103.js` (2.4 MB): Home hero, composer core, model picker, submit logic, banners.
- `app-shared-8bd9d0399038.js` (8.2 MB): app-server request client, conversation store, notification reducers, thread list hydration.
- `app-main-c3673115a651.js`: entry stub.
- `.vite/build/preload.js`: `electronBridge` context bridge and all IPC channel names.
- `.vite/build/main-CqEoQDOU.js` (3.6 MB): IPC handlers, application menu, window manager, app-server bridge.
- `.vite/build/src-BJv5Mk7n.js`: main-process copy of the command registry (menu-only commands).
- `.vite/build/worker.js`: git worker (diff, status, branch, worktree methods).

## 2. App shell

### 2.1 Routes (Codex-relevant)

Router source: `app-initial-6350d04672d0.js` (offset about 8,940,000; copy in `/tmp/cgx/e1-out/route-table.js`).

| Route | Element (export) | Chunk | Screen |
|---|---|---|---|
| `/` | Home (inline in app shell) | app-primary-6f60e9ac4103.js | Home |
| `/local/:conversationId` | `LocalConversationPage`, `AppServerConversationPage` | local-conversation-page-c1365d9af374.js → local-conversation-page-a83a1af2c24b.js | Thread view |
| `/remote/:taskId` | `RemoteConversationPage` | remote-conversation-page-d5c77768f912.js | Cloud task thread |
| `/archive` | `LocalConversationPage` (archive mode) | local-conversation-page-c1365d9af374.js | Archived chats |
| `/diff` | `EditorDiffPage` | editor-diff-page-86fa0f0062f3.js | Diff view (own window) |
| `/settings/:section/*` | `SettingsPage` + section page | settings-page-50e3fab957b6.js | Settings |
| `/login` | `LoginRoute` | login-route-27ff39de87da.js | Login |
| `/welcome` | `OnboardingPage` | onboarding-page-2b170595bf4d.js | Onboarding |
| `/onboarding` | `OnboardingRoute` | route-4a18aa0cfe04.js | Onboarding |
| `/select-workspace` | `SelectWorkspacePage` | select-workspace-page-93a123d78c93.js | Onboarding |
| `/first-run` | `FirstRunPage` | first-run-597e7ed8da4e.js | Onboarding |
| `/plan-summary` | `PlanSummaryPage` | plan-summary-page-e5d72d4fa646.js | Plan window |
| `/hotkey-window/*` (`new-thread`, `thread/:conversationId`) | `HotkeyWindowHomePage`, `HotkeyWindowNewThreadPage`, `HotkeyWindowThreadPage` | hotkey-window-*-page-*.js | Popout window |
| `/space`, `/space/:pageId`, `/spaces/:spaceId` | `SpacesLandingRoute`, `SpaceDetailRoute` | landing-route-c6547c6b9cab.js, space-route-6aad6e5649b3.js | Rail: Space |
| `/scheduled`, `/automations` | `AutomationsLayoutPage` | automations-page-aa8b83be2225.js | Rail: Scheduled |
| `/plugins`, `/skills` | `ManagePluginsPage`, `SkillsPage` | skills-page-b1051baf2473.js | Rail: Plugins |
| `/pull-requests` | `PullRequestsRoute` | pull-request-route-b0c1b0c301a4.js | Rail: Code Review |
| `/sites`, `/sites/:projectId/*` | Appgen pages | appgen-*.js, page-6b177e2d195b.js | Rail: Sites |
| `/explore`, `/inbox`, `*` | `Navigate` redirect | app-initial | — |

### 2.2 Window chrome and layout (reference: `refs/home/dark@1280x832.spec.json`)

- The window has a 44 px transparent titlebar (`header`, fixed, z-30) that holds the macOS traffic lights, `Back` (`⌘[`), `Forward` (`⌘]`), `Hide sidebar` (`app.sidebar.hide` / `app.sidebar.show`), the `Chat toolbar` region (`threadWorkspace.chatTitlebar.label`) and `New tab` at the right edge.
- Titlebar buttons are 28×28 at x = 88, 122, 156 (Back, Forward, Hide sidebar) and x = 1244 (New tab).
- The left panel (`aside._LeftPanel_yl8z4_2`) is 340 px wide: the rail `nav[aria-label="App navigation"]` is 52 px, the sidebar `nav._Navigation_1uxfp_2` is 288 px.
- The main surface is `main._MainContentSurface_yl8z4_2._MainContentLeftBorder_yl8z4_2` (x 340, w 936); it has a hairline left border (`--app-shell-sidebar-divider`) and a large end radius (`--radius-xl-base`).
- A hidden `Skip to content` link (`appShell.skipToContent`) becomes visible on focus.
- The sidebar width is resizable: `dC(width, windowWidth, min = 240)` clamps to 240…min(520, window − 240), with default 340, and stores it under `sidebar-width` (`app-initial`).
- Breakpoint `(min-width: 768px)` (`KRt` in app-initial): below it the shell sets `data-app-shell-responsive-sidebar=true` and hides `._ResponsiveLeftPanel_yl8z4_2`; the sidebar then opens as an overlay (CSS in `app-initial-99e8b4b455fa.css`).
- Breakpoint `(width < 40rem)` (640 px): the composer stacks its single-line layout (`app-initial`).
- Main-process window messages: `electron-window-focus-changed`, `window-fullscreen-changed`, `electron-window-opaque-surface-changed`, `electron-window-position-changed`; renderer requests: `electron-set-window-mode`, `electron-window-zoom-changed`, `electron-window-focus-request`.
- Zoom: menu `Zoom In` `⌘+`, `Zoom Out` `⌘-`, `Actual Size` `⌘0` send `step-window-zoom` / `reset-window-zoom` to the view.

### 2.3 Shared control states

- All icon and text buttons use CSS module class `_Button_eg66f_2` (`app-shared-9a94b907ed47.css`).
- Hover (only under `@media (hover:hover)`): ghost/transparent variants fade in a `:before` background (`opacity: 1; transform: scale(1)`) and switch text to `--button-text-color-hover`; solid and soft variants change `:before` to `--button-background-color-hover`.
- Active: `:before` uses `--button-background-color-active` and scales by `--scale`.
- Focus: `:focus` has no outline; `:focus-visible:after` draws `outline: 2px solid var(--button-ring-color, var(--color-ring))` with `outline-offset: var(--button-ring-offset, 2px)`; titlebar buttons set `--button-ring-offset: 0px`, the mode switch `-2px`.
- Sidebar rows use Tailwind `focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-[-2px]`.
- Tooltips use the shared tooltip component; rail tooltips open on the right side and show the command shortcut.

## 3. Left rail

Component: rail function `D2o` and item `Y0o` in `app-initial-6350d04672d0.js` (class `group/sidebar-rail`, `aria-label` `App navigation`).
Destination ids: object `ej` in app-initial (`builtin:codex`, `builtin:automations`, `builtin:skills`, `builtin:pull-requests`, `builtin:sites`, …).
Buttons are 36×36 at x = 8, with an 8 px gap; a 24 px divider (`w-6 border-t`) separates direct items from pinned ones.
Items are reorderable by drag (dnd-kit, activation distance 6 px; screen reader strings `sidebarElectron.rail.reorder.*`).

| Order | Item | i18n id | Opens | Current when |
|---|---|---|---|---|
| 1 | Home | `sidebarElectron.rail.homeDestination` | `/` (Home in the current mode) | route is Home |
| 2 | Space | `sidebarElectron.spaceRouteNavLink` | `/space` | route starts with `/space` |
| 3 | Scheduled | `sidebarElectron.inboxRouteNavLink` | `/scheduled` (automations, `ScheduledSidebar` in sidebar-a9ef58c0cd4b.js) | route starts with `/automations` |
| 4 | Plugins | `sidebarElectron.skillsAppsRouteNavLink` | `/plugins` | route starts with `/skills` or `/plugins` |
| 5 | Explore (`…`, blue dot) | `sidebarCustomization.moreMenu` | menu with secondary destinations and `Customize sidebar` | — |
| 6 | Code Review | `sidebarElectron.codeReviewRouteNavLink` | `/pull-requests` (`CodeReviewSidebar` in sidebar-6f96a2b5f34d.js) | route starts with `/pull-requests` |
| 7 | Sites | `sidebarElectron.sitesRouteNavLink` | `/sites` | route starts with `/sites` |
| footer | Profile avatar | `codex.profileFooter.openProfileMenu` | profile menu (Profile, Settings, Usage, Log out, Show Mini, …; `codex.profileDropdown.*`, profile-dropdown-items-*.js) | — |

- Reselecting the current destination calls its `onReselect` (Home resets the view) instead of navigating.
- Rail settings button (`sidebarElectron.rail.settings`) opens settings when no footer is supplied.
- Only Home is in scope for v1 behavior; the other rail destinations need only their route switch and an empty placeholder.

## 4. Screen: Sidebar (thread list and search)

Chunks: app-initial-6350d04672d0.js (sidebar, rows, sections, dialogs), sidebar-rename-dialog-8b9e03e97b1b.js, delete-thread-dialog-a6789db079dc.js, sidebar-archive-suggestion-*.js, command-menu-dialog-16e35c7f9f05.js (search).

### 4.1 Component outline

- `nav._Navigation_1uxfp_2[aria-label="Home"]` (288 px)
  - Header row (y 54): mode switch button `Codex ⌄` (`sidebarElectron.productMode.trigger` = `Switch mode, current mode: {mode}`, 84×32), `View activity` bell (`sidebarElectron.priorityThreads.filterByPriority`, 28×28), `Search` (`sidebarElectron.search`, 28×28).
  - Nav rows (272×30): `New chat` (`sidebarElectron.newThread`), `Your dot` (`sidebarElectron.orbitNavigation.dot`).
  - Section `Projects` (`sidebarElectron.projectsNavLink`) with toggle button `group/section-toggle`; empty text `No projects`.
  - Section `Recents` (`sidebarElectron.chatGptWork.recents`); empty text `No chats` (`sidebarElectron.noTasks`).
  - Optional sections: `Pinned`, `Priority`, custom sections, connection groups (`Local`, `Cloud`, remote), worktree groups.
  - Usage alert card and update banner at the bottom (`sidebarElectron.usageAlert.*`, `internalAlphaUpdateBanner.*`).

### 4.2 Header items

- Mode switch opens a menu with `ChatGPT`, `Codex` (`Build, debug, and ship`), `Work` (`Create, learn, and explore`); shortcuts `⌃1`, `⌃2`, `⌃3` (`switchToMode1..3`).
- Activity bell toggles the activity view (`togglePriorityFilter`, `⌘⌥U`); it shows `View activity, needs attention` when unread; options menu `Activity view options` with `Show` toggles (Chat, Work, Scheduled, Pinned, Priority section), `Mark all as read`, `Clear read chats`, `Turn off activity view`.
- Search dispatches host message `chat-search-command-menu`, which opens `CommandMenuDialog` in chat-search mode (`codex.commandMenu.chatSearchPlaceholder` = `Search chats`); the tooltip shows the `openCommandMenu` keys (`⌘K`).
- Command menu groups: `Quick actions`, `Pinned chats`, `Recent chats`, `Recently viewed chats`, `Chats`, `Settings`; empty `No matches`; loading `Loading chats…`; error `Couldn't search ChatGPT chats` + `Retry`.

### 4.3 Thread rows

- Click opens `/local/:conversationId`.
- Row actions and context menu: `Pin chat` / `Unpin chat`, `Rename chat` (dialog: title `Rename chat`, placeholder `Add a title…`, `Save`, `Cancel`), `Archive chat`, `Mark as read` / `Mark as unread`, `Change emoji`, `Remove from section`, `Permanently delete` (dialog `Delete task?`).
- Hover card for projects shows counts (`{count} active`, `{count} unread`, `{count} waiting`) and `Edit project`, `Pin project`.
- Sort menu `Sort chats by`: `Created`, `Last updated`, `Manual order`; organize menu `By project`, `In one list`, `By connection`.
- Shortcuts: `⌘1`…`⌘9` (go to chat N), `⌘⌥1`…`⌘⌥6` (recent chat N), `⌘⇧[` / `⌘⇧]` (previous / next chat), `⌘⇧A` archive, `⌘⌥P` pin, `⌘⇧U` mark unread, `⌘⌥A` next chat needing attention.

### 4.4 States

- Empty: `No projects`, `No chats`, activity `Nothing needs attention`, archive `No archived tasks`.
- Loading: `Checking unread chats…`, `Loading archived tasks…`, `Loading remote connection`.
- Error: `Projects couldn't be loaded` + `Retry`, `Could not load chats`, `Failed to archive chat`, `Failed to rename chat`, `Unread chats couldn't be loaded`.
- Responsive: see 2.2; the sidebar collapses with `⌘⇧S` / `⌘B` and the titlebar toggle.

### 4.5 Data

- `thread/list` with `limit: 100`, `cursor`, `sortKey`, `modelProviders`, `sourceKinds`, `archived`, `useStateDbOnly: true`; pages through `nextCursor`.
- `thread/search` with `searchTerm`, `cursor`, `limit`, `sortKey`, `archived`, `sourceKinds`.
- Live updates: `thread/started`, `thread/name/updated`, `thread/status/changed`, `thread/archived`, `thread/unarchived`, `thread/deleted`.

## 5. Screen: Home

Chunks: app-primary-6f60e9ac4103.js (hero, composer), composer-utility-bar-7738f1184086.js, permissions-mode-dropdown-dc16e1c7f7ad.js, codex-app-home-beacon-announcement-*.js, home-announcement-slot-*.js, home-task-suggestions-e4589ce029e6.js, home-ambient-suggestions-content-ba4330f6633f.js, composer-banners-5af6d45e3584.js.

### 5.1 Component outline (dark, 1280×832)

- Main surface, content centered in the 936 px column.
  - Hero: Codex cloud-terminal icon (64 px) and title `What should we build?` (`home.hero.noProjectWhatShouldWeBuild`); with a project: `What should we build in <projectSelect>{projectName}?</projectSelect>`.
  - Announcement slot (`aside._surface_or5n7_9`, 736×74 at y 594): beacon `Try ChatGPT Voice` / `Orchestrate tasks, connect tools, and explore code`, button `Start Voice`, close `Dismiss ChatGPT beacon banner`.
  - Composer utility bar (`aria-label` `Composer utility bar`, 706×32): `Choose project` (`composer.localCwdDropdown.chooseProject`), `Work in` → `This computer` (`composer.newTask.thisComputer`).
  - Composer card: ProseMirror editor (`aria-label`/placeholder `Do anything`, `homePage.composer.placeholder.askAnything.v2`), footer row with `Add files and more` (+), `Change permissions` (`Custom`), model button `GPT-6 Astra Light` (`Light` = `composer.mode.local.reasoning.low.label.v2`), `Dictate`, `Start new voice chat` (round, `bg-composer-primary`).
  - Optional: task suggestions (`home.starterPrompts.*`, `home.taskSuggestions.*`), ambient suggestions, home banners (fast mode, rate limit reset, multi-agent).

### 5.2 Flows

- Submit: renderer builds params and calls `thread/start` (or `thread/startAeon`), then `turn/start`, then navigates to `/local/${clientThreadId}`.
- Project menu: search `Search projects`, `Add new project`, `Add local project`, `New remote project`, `Don't work in a project`; empty `No projects found`.
- Location menu (`Work in`): `This computer`, `Worktree` / `New worktree`, `Remote`, `Cloud`, environment search.
- States: empty Home (no projects, no chats) as in the reference; loading `Loading suggestions…`; error `Couldn’t load suggestions` + `Try again`; account error page `We couldn’t load your account` with `Reload` / `Sign out`.

## 6. Screen: Thread view

Chunks: local-conversation-page-a83a1af2c24b.js, local-conversation-thread-c8c5e92ea89e.js (thread, find, errors), virtualized-turn-list-6a3bab50898c.js, thread-virtualizer-e1964ca5c039.js, thread-scroll-layout-6cc539710c73.js, local-conversation-turn-632904a8790c.js, local-conversation-thread-turn-entries-289ca6f16609.js, sites-end-resource-78b98c374310.js (turn item renderers: reasoning, commands, patches, web search, agent activity), user-message-dd2539189e70.js, collapsed-turn-disclosure-5e77ff7d3908.js, active-tool-activity-label-78745b84f2e7.js, mcp-tool-activity-label-fb47c28e3f97.js, agent-activity-item-40fa48e24fdc.js, pending-request-item-panel-770855267df2.js (approvals), thread-app-shell-chrome-c6e5ce470743.js (tabs, titlebar), thread-overflow-menu-95b62ccc8390.js, thread-recovery-notice-e3e7b73ead54.js, thread-history-loading-a041c561910d.js.

### 6.1 Component outline

- `LocalConversationPage` → thread layout with chat column and optional side panel tabs (`Changes`, `Files`, `Browser`, `Terminal`, `Side chat`; `thread.sidePanel.*`) and bottom panel (`⌘J`).
- Chat titlebar (`Chat toolbar`): chat title (fallback `Chat`), tab strip `Task tabs`, `New tab`, open-tab count.
- Virtualized turn list; each turn has: user message (copy, edit with `Send` / `Cancel`, show more/less), collapsed work disclosure `Worked for {time}`, item list, assistant message (copy response, fork `Fork chat from here`, `Branch in new chat`).
- Item renderers by `ThreadItem.type`: `userMessage`, `agentMessage`, `reasoning` (`Thinking`, `Thought for {elapsed}`), `commandExecution` (`Running {command}`, `Ran {command} in {elapsed}`, `Stopped …`; copy command/output), `fileChange` (`Edited {filename}`, `Edited # files`, `View changes`, `Undo`, `Reapply`), `mcpToolCall`, `dynamicToolCall`, `collabAgentToolCall`, `webSearch` (`Searched the web`), `imageView`, `imageGeneration`, `contextCompaction`, `plan`, `hookPrompt`.
- Live activity label above the composer: `Running command`, `Reading {target}`, `Searching for {query}`.
- Approval card (`PendingRequestItemPanel`): `Allow ChatGPT to run this command?`, `Allow similar commands`, `Allow ChatGPT to edit the following file(s)`, `Allow all edits`, network and permission prompts; decisions sent: `accept`, `acceptForSession`, `acceptWithExecpolicyAmendment`, `applyNetworkPolicyAmendment`, deny.
- Find bar (`⌘F`): `Search chat…`, `{active} / {matches} results`, `Next result` (`⌘G`), `Previous result` (`⌘⇧G`), `Close find`.
- `Scroll to bottom` floating button.

### 6.2 Streaming

- `turn/started` creates the active turn; `item/started` adds an item; `item/agentMessage/delta`, `item/reasoning/summaryTextDelta`, `item/reasoning/textDelta`, `item/commandExecution/outputDelta`, `item/plan/delta`, `item/fileChange/patchUpdated`, `item/mcpToolCall/progress` stream content; `item/completed` finalizes; `turn/completed` ends the turn.
- `turn/diff/updated` updates the aggregated diff; `turn/plan/updated` updates the plan; `thread/tokenUsage/updated` updates the context meter.
- Stop sends `turn/interrupt` with `threadId`, `turnId`.

### 6.3 States

- Loading: `Loading task…`, `Reconnecting to ChatGPT…`, `Working`, `Working for {time}`.
- Error: `Couldn't load messages` + `Retry`, `This turn couldn't render` + `Try again`, `Reconnecting {progress}`, `Server is busy, reconnecting`, `Retry in {seconds}s`, usage limit banners (`localConversation.usageLimit.*`), archived chat page (`localConversation.archived.*`, unavailable-local-conversation-page-6d83840f1b1b.js).
- Empty: a new thread shows only the first user message and the pending turn (pending-local-conversation-thread-3398c807ea7f.js).

## 7. Screen: Composer

Chunks: app-primary-6f60e9ac4103.js (editor, submit, model picker, banners), composer-controls-7c1d04d21325.js (attachments, buttons), composer-utility-bar-7738f1184086.js, permissions-mode-dropdown-dc16e1c7f7ad.js, queued-message-list-77acfdcb3f8a.js, composer-banners-5af6d45e3584.js, pending-request-item-panel-770855267df2.js (approvals above the composer).

### 7.1 Controls

- Editor: ProseMirror; placeholders `Do anything` (Home), `Work with Codex`, `Working…`, `Describe your task to generate a plan...` (plan mode), `Describe your goal, …` (goal mode).
- `Add files and more` (`+`, keyboard equivalent `@`): files, photos, remote files, @-mentions (`composer.atMentionList.*`, tabs `composer.atMentionTabs.*`).
- Slash commands `/` (`Slash commands`, search `Search`, empty `No commands`): new, model, reasoning, plan, goal, compact, fork, review, init, mcp, memories, status, usage, rename, archive, side, project, feedback, speed, personality, hotkey-window commands (`composer.*SlashCommand`).
- Permissions dropdown (`Change permissions`), title `How should ChatGPT actions be approved?`: `Ask for approval`, `Approve for me`, `Full access`, `Custom (config.toml)`, `Managed`; disabled tooltips `Disabled in config.toml`, `Disabled by requirements.toml`, `Unavailable while working in the cloud`.
- Model picker (`Select model`): model list, `Select effort` (reasoning levels; `Light` shown for low), `Fast` toggle (`{speedMultiplier}x speed, more usage`), `Reset to default`, loading `Loading model…`.
- Submit button tooltip: `Send`, `Stop`, `Queue`, `Steer`, `Resume`.
- Queued messages: `Steer` (`Submit without interrupting the model`), `Edit message`, `Delete queued message`, `Retry`, `Turn on/off queueing`, paused `Queue paused because you interrupted` + `Resume`.
- Dictation: `Dictate`, `Stop dictation`, `Transcribing`, errors `Unable to start dictation`, `Unable to transcribe audio`.
- Context meter (`composer.contextWindowUsage*`) and status (`composer.statusPlain.*`).
- Drop overlay: `Drop to attach`, `Drop to reference chat`.

### 7.2 Keys

- Enter sends by default; setting `Send shortcut` offers `Enter`, `⌘ + Enter for multiline prompts`, `⌘ + Enter always` (`settings.general.enterBehavior.*`).
- Follow-up behavior setting chooses `Queue` or `Steer`; a modifier inverts it (`settings.general.followUpQueueMode.description`).
- `Shift+Escape` focuses the main chat in the browser build (`focusMainChat`); `⌘⇧;` copies the last code block.

### 7.3 States

- Submit blocked dialog `Unable to send message` with reasons `composer.submit.*` (`Select a project to continue`, `Files uploading…`, `Remote connection is disconnected`, …).
- Upload errors `Unable to upload file`, `File is too large to upload (maximum {maximumFileSize} MB)`.
- Banners: full access warning, model upgrade, Windows sandbox, safety, usage limits.

## 8. Screen: Diff view

Chunks: thread-side-panel-tab-content-e9ce64182d53.js (`ReviewThreadSidePanelTab`, the `Changes` tab), review-file-tree-pane-e673182fde4d.js, review-file-tree-side-pane-efa4c8cabba2.js, code-diff-38f339f0a601.js, inline-file-diff-review-76525498732c.js, editor-diff-page-86fa0f0062f3.js (`/diff` window), pdf-preview-diff-*.js, use-code-diff-context-menu-*.js, codex-diff-css-a6773575cd64.js, sites-end-resource-78b98c374310.js (inline unified diff cards in the thread).

### 8.1 Component outline

- Header (`Changes controls`): source selector (`Uncommitted`, `Staged`, `Unstaged`, `Last turn`, `Branch`, `Commits`, `Cloud changes`, `All repos`), `Show files` / `Hide files`, `Refresh`, `Changes options` (`Copy git apply command`, `Expand all diffs`, `Collapse all diffs`, `Load full files`, `Word wrap`, `Hide white space`, `Word diffs`, `Rich preview`).
- File tree pane: filter `Filter files…`, `Filter changed files`, `Filter generated files`, context menu `Copy path`, `Open in {target}`, `Open with`.
- Diff list: per file header (`File actions`: `Copy path`, `Open in editor`, `Collapse file` / `Expand file`), `Mark as viewed`, hunks with `Revert`, `Stage` / `Unstage`.
- Large diff mode: `This diff is large, showing one file at a time` with `Previous file` / `Next file`.
- Diff renderer sets `data-diff-type="split"|"single"`, `data-indicators="bars"|"classic"`, `data-overflow`, `--diffs-min-number-column-width-default` (app-initial).
- `/diff` window: `{fileCount} files changed`, `Switch to split diff` / `Switch to unified diff`, `Auto` mode, `Toggle rich preview`.

### 8.2 States

- Empty: `No file changes yet`, `Changes in this project will appear here.`, `No staged changes` / `No unstaged changes`, `The last turn was reverted.`, non-git `Create a Git repository`.
- Loading: `Loading diff`, `Refreshing changes`, `Loading commits…`.
- Error: `Couldn't load changes` + `Retry`, `Diff failed to render`, `Diff failed to load after retrying`, `Diff too large to display`, `Failed to revert {path}`.

### 8.3 Data

- Turn diff arrives in `turn/diff/updated` (`threadId`, `turnId`, `diff`) and `item/fileChange/patchUpdated` (`changes`).
- Working tree data comes from the git worker over `codex_desktop:worker:git:*`: `status-summary`, `review-summary`, `review-diff`, `review-patch`, `branch-diff-stats`, `base-branch`, `current-branch`, `apply-patch`, `apply-changes`, `apply-review-section-changes`, `revert` (full list in `/tmp/cgx/e1-out/git-worker-cases.txt`, source `.vite/build/worker.js`).
- `update-diff-if-open` is posted from local-conversation-turn-632904a8790c.js to refresh an open diff window.

## 9. Screen: Settings, login and onboarding

### 9.1 Settings

Shell: `SettingsPage` in settings-page-50e3fab957b6.js (nav `Settings`, `Back to app`, search `Search settings`, headings `Personal`, `Coding`, `Integrations`, collapse toggle).
Route `/settings/:section/*`; unknown sections fall back to `general-settings`; `show-settings` from main opens `/settings/${section}`.
Section registry: `gWa` (order) and `lQa` (lazy pages) in app-initial-6350d04672d0.js.

| Slug | Nav label | Page export | Chunk file |
|---|---|---|---|
| `general-settings` | General | `GeneralSettings` | general-settings-861f7f5f7042.js |
| `notifications` | Notifications | `NotificationsSettingsPage` | notifications-settings-632d328fca3d.js |
| `import` | — | `ImportSettings` | import-settings-cdabe3e4c1d1.js |
| `profile` | Profile | `ProfileSettings` | profile-settings-6d54d51340da.js |
| `security` | Security | `ChatGptSecuritySettings` | security-settings-21b709d77af4.js |
| `account` | Account | `ChatGptAccountSettings` | account-settings-b6b8255e8212.js |
| `age` | — | `ChatGptAgeSettings` | age-settings-4b10d3d4c42c.js |
| `agreements` | — | `AgreementsSettings` | agreements-settings-bc9add37a84b.js |
| `family` | — | `GeneralSettings` | general-settings-861f7f5f7042.js |
| `trusted-contact` | — | `ChatGptTrustedContactSettings` | trusted-contact-settings-fcebc326b38b.js |
| `safety` | — | `ChatGptSafetySettings` | safety-settings-87cd943070f2.js |
| `time-management` | Time management | `ChatGptTimeManagementSettings` | time-management-settings-a1e256d15af1.js |
| `keyboard-shortcuts` | Keyboard shortcuts | `KeyboardShortcutsSettings` | keyboard-shortcuts-settings-e5d452e7d34c.js |
| `codex-micro` | Codex Micro | `CodexMicroSettings` | codex-micro-settings-a81b033f873e.js |
| `appshots` | Appshots | `AppshotsSettings` | appshots-settings-3f31c623b72d.js |
| `appearance` | Appearance | `AppearanceSettings` | appearance-settings-bf899ed18bb8.js |
| `voice` | Voice | `VoiceSettings` | voice-settings-7f1819748d32.js |
| `storage` | Storage | `DeviceStorageSettings` | device-storage-9d79d1b3554c.js |
| `pets` | — | `PetsSettingsRoute` | pets-settings-route-d66810db325d.js |
| `agent` | Configuration | `ConfigurationPanel` | configuration-panel-5d5682552c7c.js |
| `work-settings` | — | `WorkSettingsPanel` | panel-1910047edbf2.js |
| `git-settings` | Git | `GitSettings` | git-settings-cc686a50f040.js |
| `ads-controls` | Ads controls | `AdsControlsSettings` | route-e012bcfc4dbe.js |
| `archived-chats` | Archived chats | `DataControlsSettings` | data-controls-c9dc29e9444b.js |
| `data-controls` | — | `ChatGptDataControlsSettings` | data-controls-settings-807d74082bb4.js |
| `code-review` | — | `CodeReviewPluginSettings` | settings-84199bf2f2cf.js |
| `cloud-settings` | — | `CloudPreferencesSettings` | cloud-preferences-settings-41546184a793.js |
| `codex-cloud` | — | `CodexCloudSettings` | settings-ae50ba9fa78a.js |
| `cloud-environments` | — | `CloudEnvironmentsSettingsPage` | cloud-environments-settings-page-2f8899d93d38.js |
| `personalization` | Personalization | `ChatGptPersonalizationSettings` | personalization-settings-9b139bd250bc.js |
| `chronicle` | Computer history | `ComputerHistorySettingsPage` | chronicle-settings-page-585519c1acbc.js |
| `usage` | Usage & billing | `UsageSettingsPage` | page-400328cd46f4.js |
| `analytics` | Analytics | `UsageSettingsPage` | page-400328cd46f4.js |
| `usage-code-review` | — | `UsageSettingsPage` | page-400328cd46f4.js |
| `consumer-view` | — | `UsageSettingsPage` | page-400328cd46f4.js |
| `debug` | Debug | `DebugSettings` | debug-settings-299be7300ee9.js |
| `computer-use` | Computer use | `ComputerUseSettings` | computer-use-settings-672bfa843f15.js |
| `browser-use` | Browser | `BrowserUseSettings` | browser-use-settings-dd15aad57b40.js |
| `extension` | — | `BrowserExtensionSettings` | browser-extension-settings-33e5503db7da.js |
| `local-environments` | Environments | `LocalEnvironmentsSettings` | local-environments-settings-page-dd4c4cddd941.js |
| `worktrees` | — | `WorktreesSettingsPage` | worktrees-settings-page-ad5cbb37d745.js |
| `environments` | Cloud Environments | `SettingsExternalSection` | settings-external-section-b33e2f5e7f5d.js |
| `mcp-settings` | MCP servers | `McpSettings` | mcp-settings-83d0520492d3.js |
| `hooks-settings` | Hooks | `HooksSettings` | hooks-settings-cb5541ab4198.js |
| `connections` | Connections | `RemoteConnectionsSettings` | remote-connections-settings-5fcec56d1e17.js |
| `plugins-settings` | Plugins | `PluginsSettings` | plugins-settings-577ebe3677a7.js |
| `file-type-handlers` | File type handlers | `FileTypeHandlersSettings` | file-type-handlers-e21cf9f18f92.js |
| `skills-settings` | Skills | `SkillsSettings` | skills-settings-92f7be984f1d.js |
| `cloud-computer` | — | `ChatGptCloudBrowserSettings` | settings-65381b73a1a2.js |
| `passwords` | — | `PasswordsSettings` | settings-156aaa691e7d.js |
| `teams` | — | `TeamSettingsPage` | team-settings-page-9276d495b863.js |

- General settings (general-settings-65e65871cdcd.js): `Theme` `Light` / `Dark` / `System`, light and dark chrome themes (accent, background, foreground, UI/code fonts, contrast, translucent sidebar, import/copy theme), `UI font size`, `Code font size`, `Diff markers` (`Color` / `+/-`), `Use pointer cursors`, `Font smoothing`, `Reduce motion`, `Dock icon`, `Send shortcut`, `Follow-up behavior`.
- Keyboard shortcuts (keyboard-shortcuts-settings-e5d452e7d34c.js) lists and rebinds the commands of section 11; `⌘/` opens `KeyboardShortcutsDialog` (keyboard-shortcuts-dialog-a55e88bf0464.js).

### 9.2 Login and onboarding

- `/login` → `LoginRoute` (login-route-27ff39de87da.js) with content from onboarding-login-content-1e8577a45a31.js.
- Click targets: `Continue with ChatGPT`, `Continue with Google`, `Continue with Microsoft`, `Sign in another way` → `OpenAI API Key` (`sk-…`, `Continue`) or `Use Amazon Bedrock` (profile, access keys, region), `Sign up`, `Copy sign-in link`, `Cancel sign-in`, `Browser didn't open?`, `Play Snake` (while waiting).
- States: `Loading sign-in requirements…`, `Unable to load sign-in requirements` + `Retry`, `Your administrator has disabled all available sign-in methods`, `Sign-in failed: {rawMessage}`, `Continue signing in with your browser`.
- Methods: `account/login/start` (`type`, `apiKey`, AWS fields), `account/login/cancel` (`loginId`), notification `account/login/completed` (`loginId`, `success`, `error`), then `account/read` and `account/updated` (`authMode`).
- `/welcome` → `OnboardingPage` (onboarding-page-2b170595bf4d.js): welcome steps, import from other AI apps (`Import work from other AI apps`, `Not now`), `Continue`.
- `/select-workspace` → `SelectWorkspacePage`; renderer sends `electron-pick-workspace-root-option` or `electron-onboarding-skip-workspace`.

## 10. App-server JSON-RPC

### 10.1 Transport

- The renderer never talks to the app-server directly.
- Request: `electronBridge.sendMessageFromView({type: "mcp-request", request: {id, method, params}, hostId, retainResponse, savedEnvironmentConfig, threadAccess, coalesce, …})` over IPC channel `codex_desktop:message-from-view` (app-shared `enqueueRequest`).
- Main forwards to the `codex app-server` process and returns `{type: "mcp-response"}`, `{type: "mcp-notification"}`, server-initiated `{type: "mcp-request"}`, `mcp-request-delivery`, `mcp-request-abandon`, `codex-app-server-initialized`, `codex-app-server-connection-changed` over `codex_desktop:message-for-view` (main-CqEoQDOU.js).
- The client sends replies to server requests as `{id, result}`; approvals reply `{decision}`.
- Priorities: `critical` 16, `interactive` 64, `background` 128 concurrent slots; default timeout 30 s; `thread/start` to cloud 120 s (app-shared constants `gpn`, `az`, `V0t`).
- Delivery-tracked methods (outcome-unknown handling): `thread/start`, `thread/startAeon`, `thread/fork`, `thread/inject_items`, `thread/queue/add`, `turn/addUserMessage`, `turn/start`, `turn/steer`.
- Field lists come from object literals at call sites and from destructuring of `params`; builder-made requests (`thread/start`, `turn/start`, `turn/steer`, `thread/resume`) were read from their builder functions (app-shared `wDt`, `Xdn`, `buildNewConversationParams`).

### 10.2 Client requests (118)

| Method | Screens | Params (keys seen at call sites) | Response keys read | Chunk files |
|---|---|---|---|---|
| `account/bedrock/discover` | Settings/Login | — | — | login-route-27ff39de87da.js |
| `account/bedrock/setup` | Settings/Login | `type`, `profile`, `region` | — | login-route-27ff39de87da.js |
| `account/gatewayOAuth/cancel` | Settings/Login | — | — | app-shared-8bd9d0399038.js |
| `account/gatewayOAuth/login` | Settings/Login | — | — | app-shared-8bd9d0399038.js |
| `account/gatewayOAuth/read` | Settings/Login | — | — | app-initial-6350d04672d0.js |
| `account/login/cancel` | Settings/Login | `loginId` | — | app-shared-8bd9d0399038.js |
| `account/login/start` | Settings/Login | `type`, `accessKeyId`, `secretAccessKey`, `sessionToken`, `region`, `apiKey` + spread object | — | app-shared-8bd9d0399038.js, login-route-27ff39de87da.js |
| `account/logout` | Settings/Login | — | — | app-shared-8bd9d0399038.js |
| `account/rateLimits/read` | Home, Settings/Login | — | — | execution-fde3a4937b6a.js |
| `account/read` | Home, Settings/Login | `refreshToken` | — | app-shared-8bd9d0399038.js |
| `app/installed` | Composer, Settings/Login | `forceRefresh`, `threadId` | — | app-initial-6350d04672d0.js, widget-e1674d393867.js |
| `app/list` | Home, Settings/Login | — | — | app-shared-8bd9d0399038.js |
| `app/read` | Settings/Login | `appIds` + spread object | — | app-initial-6350d04672d0.js |
| `collaborationMode/list` | Home, Composer | — | — | app-initial-6350d04672d0.js |
| `command/exec` | Diff | — | — | app-initial-6350d04672d0.js, panel-cfa1a69b08e2.js |
| `config/batchWrite` | Settings/Login | `edits`, `expectedVersion`, `filePath`, `reloadUserConfig` | — | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js, app-shared-8bd9d0399038.js, apps-availability-602b68c7254e.js … |
| `config/mcpServer/reload` | Settings/Login | — | — | app-initial-6350d04672d0.js |
| `config/read` | Home, Settings/Login | `includeLayers`, `cwd` | `config` | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js, app-shared-8bd9d0399038.js, home-ambient-suggestions-content-ba4330f6633f.js … |
| `config/value/write` | Settings/Login | `keyPath`, `value`, `mergeStrategy`, `filePath`, `expectedVersion` | — | app-shared-8bd9d0399038.js |
| `configRequirements/read` | Home, Settings/Login | — | — | app-shared-8bd9d0399038.js |
| `experimentalFeature/list` | Home, Settings/Login | `cursor`, `limit` | — | app-shared-8bd9d0399038.js |
| `externalAgentConfig/import/readHistories` | Settings/Login | — | — | app-initial-6350d04672d0.js |
| `feedback/upload` | Thread | `threadId`, `classification`, `reason`, `includeLogs`, `tags`, `extraLogFiles` | — | app-initial-6350d04672d0.js |
| `fs/copy` | Diff | `destinationPath`, `sourcePath` | — | app-shared-8bd9d0399038.js |
| `fs/createDirectory` | Diff | `path`, `recursive` | — | app-shared-8bd9d0399038.js, plan-side-panel-9997abc93cfe.js |
| `fs/getMetadata` | Diff | `path` | `modifiedAtMs` | app-shared-8bd9d0399038.js, dialog-98374c0319b8.js, text-file-storage-2de491513fbc.js |
| `fs/readDirectory` | Diff | `path` | `entries` | app-shared-8bd9d0399038.js, dialog-98374c0319b8.js |
| `fs/readFile` | Diff | `path` | `dataBase64` | app-shared-8bd9d0399038.js, save-cloud-file-copy-74b3b4a4a0c6.js, text-file-storage-2de491513fbc.js |
| `fs/remove` | Diff | `path`, `force`, `recursive` | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `fs/unwatch` | Diff | `watchId` | — | app-shared-8bd9d0399038.js |
| `fs/watch` | Diff | `path`, `watchId` | — | app-shared-8bd9d0399038.js |
| `fs/writeFile` | Diff | `dataBase64`, `path` + spread object | — | app-shared-8bd9d0399038.js, text-file-storage-2de491513fbc.js |
| `fuzzyFileSearch/sessionStart` | Composer | `sessionId`, `roots` | — | app-initial-6350d04672d0.js |
| `fuzzyFileSearch/sessionStop` | Composer | `sessionId` | — | app-initial-6350d04672d0.js |
| `fuzzyFileSearch/sessionUpdate` | Composer | `sessionId`, `query` | — | app-initial-6350d04672d0.js |
| `hooks/list` | Home, Settings/Login | `cwds` | — | app-initial-6350d04672d0.js |
| `marketplace/add` | Settings/Login | — | — | app-initial-6350d04672d0.js |
| `marketplace/remove` | Settings/Login | — | — | app-initial-6350d04672d0.js |
| `marketplace/upgrade` | Settings/Login | `marketplaceName` | — | app-initial-6350d04672d0.js |
| `mcpServer/event/stream/start` | Thread | `_meta`, `arguments`, `name`, `server`, `subscriptionId`, `threadId` | — | widget-e1674d393867.js |
| `mcpServer/event/stream/stop` | Thread | `subscriptionId` | — | widget-e1674d393867.js |
| `mcpServer/oauth/login` | Settings/Login | `name` + spread object | — | app-initial-6350d04672d0.js, plugins-settings-39f4509a79cc.js, use-external-agent-import-setup-actions-1cfc7afc46a4.js |
| `mcpServer/resource/read` | Thread | `originCallId` + spread object | — | app-initial-6350d04672d0.js |
| `mcpServer/tool/call` | Thread | `_meta` + spread object | — | app-shared-8bd9d0399038.js |
| `mcpServerStatus/list` | Home, Settings/Login | `cursor`, `limit`, `detail` | — | app-shared-8bd9d0399038.js |
| `memory/reset` | Settings/Login | — | — | personalization-settings-9b139bd250bc.js |
| `memory/status` | Settings/Login | `minConsolidatedThreads` | — | app-initial-6350d04672d0.js |
| `model/list` | Home, Composer, Settings/Login | `includeHidden`, `cursor`, `limit` + spread object | `data` | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js, app-shared-8bd9d0399038.js, content-fe0352f21c09.js … |
| `permissionProfile/list` | Home, Composer, Settings/Login | `cursor`, `limit`, `cwd` | — | app-initial-6350d04672d0.js |
| `plugin/install` | Settings/Login | — | — | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js |
| `plugin/installed` | Settings/Login | `cwds` | — | app-shared-8bd9d0399038.js, execution-fde3a4937b6a.js |
| `plugin/list` | Home, Composer, Settings/Login | `marketplaceKinds`, `forceRefetch` + spread object | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, onboarding-page-2b170595bf4d.js |
| `plugin/read` | Settings/Login | `pluginName` + spread object | `plugin` | app-shared-8bd9d0399038.js, plugin-installation-content-f2ba92649fe1.js |
| `plugin/reconcile` | Settings/Login | `reason` | — | widget-e1674d393867.js |
| `plugin/search` | Settings/Login | `searchTerm`, `scope`, `cursor`, `limit` + spread object | — | use-search-plugins-1694a9a96393.js |
| `plugin/share/list` | Settings/Login | — | — | installed-plugin-actions-1370e147e829.js |
| `plugin/share/save` | Settings/Login | — | — | installed-plugin-actions-1370e147e829.js |
| `plugin/share/updateTargets` | Settings/Login | `remotePluginId`, `discoverability`, `shareTargets` | — | installed-plugin-actions-1370e147e829.js |
| `plugin/skill/read` | Settings/Login | — | — | app-initial-6350d04672d0.js |
| `plugin/uninstall` | Settings/Login | — | — | app-initial-6350d04672d0.js |
| `project/list` | Home, Sidebar | `cursor`, `limit` | — | panel-fb651e5a5a08.js |
| `remoteControl/client/list` | Settings/Login | `environmentId`, `cursor`, `limit`, `order` | — | app-initial-6350d04672d0.js |
| `remoteControl/client/revoke` | Settings/Login | `environmentId`, `clientId` | — | app-initial-6350d04672d0.js |
| `remoteControl/pairing/start` | Settings/Login | `manualCode` | — | app-initial-6350d04672d0.js |
| `remoteControl/status/read` | Settings/Login | — | — | app-initial-6350d04672d0.js |
| `skills/config/write` | Settings/Login | `enabled`, `path` | — | app-initial-6350d04672d0.js, pets-settings-route-d66810db325d.js, skill-card-15b3e007becd.js |
| `skills/list` | Home, Composer, Settings/Login | — | — | app-initial-6350d04672d0.js |
| `thread/approveGuardianDeniedAction` | Composer | `threadId`, `event` | — | app-primary-6f60e9ac4103.js |
| `thread/archive` | Sidebar | `threadId` | — | app-shared-8bd9d0399038.js |
| `thread/attachment/add` | Composer | `threadId`, `attachmentType`, `identityKey`, `payload` | — | app-shared-8bd9d0399038.js |
| `thread/attachment/list` | Composer | `threadId`, `cursor`, `limit` | — | app-shared-8bd9d0399038.js, execution-fde3a4937b6a.js |
| `thread/attachment/remove` | Composer | `threadId`, `attachmentType`, `identityKey` | — | app-shared-8bd9d0399038.js |
| `thread/attachmentOwner/list` | Settings/Login | — | — | worktrees-settings-page-ad5cbb37d745.js |
| `thread/backgroundTerminals/clean` | Thread | `threadId` | — | app-shared-8bd9d0399038.js |
| `thread/compact/start` | Thread | `threadId` | — | app-shared-8bd9d0399038.js |
| `thread/delete` | Sidebar | `threadId` | — | app-shared-8bd9d0399038.js, delete-thread-dialog-a6789db079dc.js, device-storage-9d79d1b3554c.js |
| `thread/fork` | Thread | — | `thread` | app-shared-8bd9d0399038.js |
| `thread/goal/clear` | Thread | `threadId` | — | app-shared-8bd9d0399038.js |
| `thread/goal/get` | Thread | `threadId` | — | app-shared-8bd9d0399038.js |
| `thread/goal/set` | Thread | `threadId`, `objective`, `status` | `goal` | app-shared-8bd9d0399038.js |
| `thread/inject_items` | Thread | `threadId`, `items` | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `thread/items/list` | Thread | `threadId`, `turnId`, `cursor`, `limit`, `sortDirection` | `data`, `nextCursor` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `thread/list` | Sidebar | `archived`, `cursor`, `limit`, `modelProviders`, `sortKey`, `sourceKinds`, `useStateDbOnly`, `projectId` | `data`, `nextCursor` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, billing-analytics-a920e3f98ec7.js, data-controls-c9dc29e9444b.js … |
| `thread/memoryMode/set` | Composer | `threadId`, `mode` | — | app-primary-6f60e9ac4103.js |
| `thread/metadata/update` | Sidebar | `threadId`, `daybreakEnabled`, `gitInfo` | `thread` | app-shared-8bd9d0399038.js |
| `thread/name/set` | Sidebar | — | — | app-shared-8bd9d0399038.js |
| `thread/queue/add` | Composer | `threadId`, `input`, `clientUserMessageId` | — | app-shared-8bd9d0399038.js |
| `thread/queue/delete` | Composer | `threadId`, `queuedSubmissionId` | `deleted` | app-shared-8bd9d0399038.js |
| `thread/queue/list` | Composer | `threadId`, `cursor` | — | app-shared-8bd9d0399038.js |
| `thread/queue/reorder` | Composer | `threadId`, `queuedSubmissionIds` | — | app-shared-8bd9d0399038.js |
| `thread/queue/start` | Composer | `threadId`, `queuedSubmissionId` | `turn` | app-shared-8bd9d0399038.js |
| `thread/queue/update` | Composer | `threadId`, `queuedSubmissionId`, `input` | — | app-shared-8bd9d0399038.js |
| `thread/read` | Sidebar | `threadId`, `includeTurns` | `thread` | app-shared-8bd9d0399038.js, device-storage-9d79d1b3554c.js, worktrees-settings-page-ad5cbb37d745.js |
| `thread/realtime/appendSpeech` | Home | `threadId`, `text` | — | app-initial-6350d04672d0.js |
| `thread/realtime/appendText` | Home | `threadId`, `role`, `text` | — | app-initial-6350d04672d0.js |
| `thread/realtime/listVoices` | Home | — | — | app-initial-6350d04672d0.js |
| `thread/realtime/start` | Home | `codexResponseHandoffChannelPrefixes`, `codexResponseHandoffMode`, `codexResponseItemPrefix`, `codexResponsesAsItems`, `flushTranscriptTailOnSessionEnd`, `includeStartupContext`, `outputModality`, `realtimeEndInstructions`, `realtimeSessionId`, `realtimeStartInstructions`, `threadId`, `transport`, `version` | — | app-initial-6350d04672d0.js |
| `thread/realtime/stop` | Home | `threadId` | — | app-initial-6350d04672d0.js, execution-fde3a4937b6a.js |
| `thread/resume` | Thread | `threadId`, `history`, `path`, `model`, `modelProvider`, `cwd`, `personality`, `excludeTurns`, `initialTurnsPage` | `thread` | app-shared-8bd9d0399038.js |
| `thread/revert` | Thread, Diff | `threadId`, `beforeTurnId` | — | app-shared-8bd9d0399038.js |
| `thread/rollback` | Thread | `threadId`, `numTurns` | — | app-shared-8bd9d0399038.js |
| `thread/search` | Sidebar | `cursor`, `limit`, `sortKey`, `archived`, `sourceKinds`, `searchTerm` | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `thread/searchOccurrences` | Thread | — | — | local-conversation-thread-c8c5e92ea89e.js |
| `thread/settings/update` | Composer | `threadId`, `sandboxPolicy` + spread object | — | app-shared-8bd9d0399038.js |
| `thread/start` | Home | `cwd`, `model`, `modelProvider`, `serviceTier`, `config`, `approvalsReviewer`, `approvalPolicy`, `sandbox/permissions (from permission profile)`, `runtimeWorkspaceRoots`, `personality`, `ephemeral`, `baseInstructions`, `developerInstructions`, `threadSource`, `mockExperimentalField`, `experimentalRawEvents`, `dynamicTools`, `historyMode`, `serviceName`, `projectId`, `environments` | `thread` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `thread/startAeon` | Home | — | — | app-shared-8bd9d0399038.js |
| `thread/stop` | Thread | `threadId` | — | app-shared-8bd9d0399038.js |
| `thread/timeline/list` | Thread | `threadId`, `cursor`, `limit` | — | app-shared-8bd9d0399038.js |
| `thread/turns/list` | Thread | `threadId`, `cursor`, `limit`, `sortDirection`, `itemsView` | `data`, `nextCursor` | app-shared-8bd9d0399038.js, share-thread-857fe1d9dce4.js |
| `thread/unarchive` | Sidebar | `threadId` | — | app-shared-8bd9d0399038.js |
| `thread/unsubscribe` | Sidebar | `threadId` | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `turn/addUserMessage` | Composer | `threadId`, `input`, `clientUserMessageId`, `messageThreadId` | — | app-shared-8bd9d0399038.js |
| `turn/interrupt` | Thread | `threadId`, `turnId` | — | app-shared-8bd9d0399038.js |
| `turn/settings/update` | Composer | `threadId`, `turnId`, `approvalsReviewer` | — | app-shared-8bd9d0399038.js |
| `turn/start` | Composer | `threadId`, `input`, `clientUserMessageId`, `cwd`, `approvalPolicy`, `approvalsReviewer`, `sandboxPolicy`, `model`, `serviceTier`, `effort`, `summary`, `personality`, `outputSchema`, `collaborationMode`, `attachments`, `toolOutput`, `additionalContext`, `responsesapiClientMetadata` | `turn` | app-shared-8bd9d0399038.js |
| `turn/steer` | Composer | `threadId`, `input`, `expectedTurnId`, `additionalContext`, `responsesapiClientMetadata`, `clientUserMessageId` | — | app-shared-8bd9d0399038.js |
| `windowsSandbox/readiness` | Home, Settings/Login | — | — | app-initial-6350d04672d0.js |
| `windowsSandbox/setupStart` | Home, Settings/Login | `mode`, `cwd` | — | app-shared-8bd9d0399038.js |

### 10.3 Server notifications (58)

| Notification | Screens | Params keys destructured | Chunk files |
|---|---|---|---|
| `account/gatewayOAuth/changed` | Settings/Login | `name`, `success`, `error` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `account/login/completed` | Settings/Login | `loginId`, `success`, `error`, `onboardingEntrypoint` | app-shared-8bd9d0399038.js |
| `account/updated` | Settings/Login | `authMode` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `app/list/updated` | Settings/Login | — | app-initial-6350d04672d0.js |
| `codex/event/skills_update_available` | Home | — | app-initial-6350d04672d0.js |
| `externalAgentConfig/import/completed` | Settings/Login | — | app-initial-6350d04672d0.js |
| `fs/changed` | Diff | — | app-shared-8bd9d0399038.js |
| `fuzzyFileSearch/sessionUpdated` | Composer | — | app-initial-6350d04672d0.js |
| `hook/completed` | Thread | `threadId`, `turnId`, `run` | app-shared-8bd9d0399038.js |
| `hook/started` | Thread | `threadId`, `turnId`, `run` | app-shared-8bd9d0399038.js |
| `item/agentMessage/delta` | Thread | `itemId`, `delta`, `threadId`, `turnId` | app-shared-8bd9d0399038.js, content-fe0352f21c09.js, remote-conversation-page-d5c77768f912.js |
| `item/autoApprovalReview/completed` | Thread | — | app-shared-8bd9d0399038.js |
| `item/autoApprovalReview/started` | Thread | — | app-shared-8bd9d0399038.js |
| `item/commandExecution/outputDelta` | Thread | `itemId`, `delta`, `threadId`, `turnId`, `message` | app-shared-8bd9d0399038.js, remote-conversation-page-d5c77768f912.js |
| `item/commandExecution/terminalInteraction` | Thread | `itemId`, `stdin`, `threadId`, `turnId`, `message` | app-shared-8bd9d0399038.js |
| `item/completed` | Thread | `item`, `threadId`, `turnId`, `completedAtMs` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, content-fe0352f21c09.js, entry-points-701d4d6b2dd2.js … |
| `item/fileChange/outputDelta` | Thread, Diff | — | app-shared-8bd9d0399038.js |
| `item/fileChange/patchUpdated` | Thread, Diff | `changes`, `itemId`, `threadId`, `turnId`, `message` | app-shared-8bd9d0399038.js |
| `item/mcpToolCall/progress` | Thread | `itemId`, `message`, `threadId`, `turnId` | app-shared-8bd9d0399038.js, content-fe0352f21c09.js |
| `item/plan/delta` | Thread | `itemId`, `delta`, `threadId`, `turnId` | app-shared-8bd9d0399038.js, remote-conversation-page-d5c77768f912.js |
| `item/reasoning/summaryPartAdded` | Thread | `itemId`, `delta`, `contentIndex`, `threadId`, `turnId`, `message` | app-shared-8bd9d0399038.js |
| `item/reasoning/summaryTextDelta` | Thread | `itemId`, `summaryIndex`, `delta`, `threadId`, `turnId` | app-shared-8bd9d0399038.js, content-fe0352f21c09.js, remote-conversation-page-d5c77768f912.js |
| `item/reasoning/textDelta` | Thread | `itemId`, `delta`, `contentIndex`, `threadId`, `turnId`, `message` | app-shared-8bd9d0399038.js, remote-conversation-page-d5c77768f912.js |
| `item/started` | Thread | `item`, `threadId`, `turnId`, `startedAtMs` | app-shared-8bd9d0399038.js, content-fe0352f21c09.js, remote-conversation-page-d5c77768f912.js |
| `mcpServer/oauthLogin/completed` | Settings/Login | `name`, `success`, `error` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `mcpServer/startupStatus/updated` | Thread | `status`, `item`, `threadId` | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js, content-fe0352f21c09.js, onboarding-page-2b170595bf4d.js … |
| `model/rerouted` | Thread | `threadId`, `turnId`, `fromModel`, `toModel`, `reason` | app-shared-8bd9d0399038.js |
| `model/safetyBuffering/updated` | Thread | `threadId`, `turnId`, `model` + spread object | app-shared-8bd9d0399038.js |
| `model/verification` | Thread | — | app-initial-6350d04672d0.js |
| `serverRequest/resolved` | Thread | — | app-shared-8bd9d0399038.js |
| `skills/changed` | Settings/Login | — | app-initial-6350d04672d0.js |
| `thread/archived` | Sidebar | `threadId`, `tokenUsage` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `thread/deleted` | Sidebar | `threadId`, `tokenUsage` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `thread/environment/connected` | Thread | `threadId`, `environmentId` | app-shared-8bd9d0399038.js |
| `thread/environment/disconnected` | Thread | `threadId`, `environmentId` | app-shared-8bd9d0399038.js |
| `thread/goal/cleared` | Thread | — | app-shared-8bd9d0399038.js |
| `thread/goal/updated` | Thread | `threadId`, `goal` | app-shared-8bd9d0399038.js |
| `thread/name/updated` | Sidebar | `threadId`, `threadName` | app-shared-8bd9d0399038.js |
| `thread/realtime/closed` | Home | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `thread/realtime/error` | Home | — | app-initial-6350d04672d0.js |
| `thread/realtime/item/completed` | Home | — | app-shared-8bd9d0399038.js |
| `thread/realtime/item/started` | Home | — | app-shared-8bd9d0399038.js |
| `thread/realtime/item/transcript/delta` | Home | — | app-shared-8bd9d0399038.js |
| `thread/realtime/itemAdded` | Home | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `thread/realtime/sdp` | Home | — | app-initial-6350d04672d0.js |
| `thread/realtime/started` | Home | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `thread/realtime/transcript/delta` | Home | — | app-initial-6350d04672d0.js |
| `thread/realtime/transcript/done` | Home | — | app-initial-6350d04672d0.js |
| `thread/settings/updated` | Thread | `threadId`, `threadSettings` | app-shared-8bd9d0399038.js |
| `thread/started` | Sidebar | `thread` | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js, app-shared-8bd9d0399038.js |
| `thread/status/changed` | Sidebar | `status`, `threadId` | app-shared-8bd9d0399038.js, content-fe0352f21c09.js |
| `thread/tokenUsage/updated` | Thread | `threadId`, `tokenUsage` | app-shared-8bd9d0399038.js |
| `thread/unarchived` | Sidebar | `threadId`, `tokenUsage` | app-primary-6f60e9ac4103.js, app-shared-8bd9d0399038.js |
| `turn/completed` | Thread | `threadId`, `turn` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, content-fe0352f21c09.js, execution-fde3a4937b6a.js … |
| `turn/diff/updated` | Diff | `threadId`, `turnId`, `diff` | app-shared-8bd9d0399038.js, remote-conversation-page-d5c77768f912.js |
| `turn/plan/updated` | Thread | `threadId`, `turnId`, `plan`, `explanation` | app-shared-8bd9d0399038.js, remote-conversation-page-d5c77768f912.js |
| `turn/started` | Thread | `status`, `threadId`, `turn` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, content-fe0352f21c09.js, remote-conversation-page-d5c77768f912.js |
| `windowsSandbox/setupCompleted` | Home, Settings/Login | `mode`, `success`, `error` | app-shared-8bd9d0399038.js |

### 10.4 Server requests that need a client response (13)

| Server request | Screens | Params keys destructured | Chunk files |
|---|---|---|---|
| `account/chatgptAuthTokens/refresh` | Settings/Login | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `attestation/generate` | Thread | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `autoApprovalReview/strictReviewRequired` | Thread | — | app-shared-8bd9d0399038.js |
| `currentTime/read` | Thread | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `item/commandExecution/requestApproval` | Thread, Composer | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `item/fileChange/requestApproval` | Thread, Composer, Diff | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `item/permissions/requestApproval` | Thread, Composer | — | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js, app-shared-8bd9d0399038.js |
| `item/plan/requestImplementation` | Thread | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `item/tool/call` | Thread | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `item/tool/requestOptionPicker` | Thread, Composer | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `item/tool/requestSetupCodexContextPicker` | Thread | — | app-shared-8bd9d0399038.js |
| `item/tool/requestUserInput` | Thread, Composer | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, avatar-overlay-native-page-7ec1028a9c34.js |
| `mcpServer/elicitation/request` | Thread | — | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, sites-end-resource-78b98c374310.js |

- MCP-app widget protocol (not app-server; used inside widget iframes, chunks widget-e1674d393867.js, view-frame-7aaa3b1930f2.js): `control/clipboard`, `control/locked`, `control/release`, `notifications/events/terminated`, `notifications/resources/updated`, `openai/events/subscribe`, `openai/events/unsubscribe`, `openai/form`, `prompts/list`, `resources/list`, `resources/read`, `resources/subscribe`, `resources/templates/list`, `resources/unsubscribe`, `tools/call`, `tools/list`, `ui/message`.

## 11. Electron IPC

### 11.1 Channels (`.vite/build/preload.js`, handlers in `.vite/build/main-CqEoQDOU.js`)

| Channel | Direction | Use |
|---|---|---|
| `codex_desktop:message-from-view` | renderer → main, `invoke` | all typed messages of 11.2, including `mcp-request` |
| `codex_desktop:message-for-view` | main → renderer, `on` | all typed messages of 11.3, including `mcp-response` / `mcp-notification` |
| `codex_desktop:chunked-message-ack` | renderer → main, `send` | ack for chunked large messages (`codex-host-chunked-message-v1`) |
| `codex_desktop:get-system-theme-variant` | renderer → main, `sendSync` | returns `dark` or `light` from `nativeTheme.shouldUseDarkColors` |
| `codex_desktop:system-theme-variant-updated` | main → renderer | system theme change |
| `codex_desktop:get-shared-object-snapshot` | renderer → main, `sendSync` | initial shared-object store |
| `codex_desktop:get-initial-sidebar-bootstrap` | renderer → main, `sendSync` | initial sidebar data before first render |
| `codex_desktop:get-build-flavor` | renderer → main, `sendSync` | build flavor |
| `codex_desktop:get-sentry-init-options` | renderer → main, `sendSync` | Sentry options, app version |
| `codex_desktop:get-fast-mode-rollout-metrics` | renderer → main, `invoke` | fast mode metrics |
| `codex_desktop:show-context-menu` | renderer → main, `invoke` | native context menu; returns chosen item id |
| `codex_desktop:start-file-drag`, `codex_desktop:start-link-drag` | renderer → main | native drag out |
| `codex_desktop:worker:<id>:from-view` / `:for-view` | both | worker RPC (`git`, `computer-use`) |
| `codex_desktop:connect-app-host` | renderer → main, `postMessage` with port | MCP app host |
| `codex_desktop:mcp-app-sandbox-host-message` | main → renderer | MCP app sandbox |
| `codex_desktop:remote-hosted-pip-video-frame` | main → renderer | PiP frames |
| `codex_desktop:trigger-sentry-test` | renderer → main | debug |
| `sentry-ipc.*` | renderer → main | Sentry transport |

Bridge members (`window.electronBridge`): `windowType`, `sendMessageFromView`, `acknowledgeChunkedMessage`, `getPathForFile`, `startFileDrag`, `startLinkDrag`, `sendWorkerMessageFromView`, `subscribeToWorkerMessages`, `getPreloadStartedAtMs`, plus shared-object helpers.
The preload sets the user agent string `Codex Desktop/<version> (Mac OS; arm64)`.

### 11.2 Renderer → main message types (79)

Relevant to v1: `ready`, `view-focused`, `electron-set-window-mode`, `electron-window-zoom-changed`, `electron-window-focus-request`, `open-in-new-window`, `open-in-main-window`, `open-current-main-window`, `open-in-browser`, `open-config-toml`, `show-settings`, `electron-pick-workspace-root-option`, `electron-add-new-workspace-root-option`, `electron-onboarding-skip-workspace`, `electron-set-badge-count`, `app-shell-shortcut-state-changed`, `keyboard-layout-map-changed`, `persisted-atom-update`, `persisted-atom-sync-request`, `shared-object-set` / `-subscribe` / `-unsubscribe`, `local-thread-activity-changed`, `tray-menu-threads-changed`, `update-diff-if-open`, `codex-app-server-restart`, `fetch` / `cancel-fetch`, `log-message`, `power-save-blocker-set`, `quit-app`.

| Message `type` (renderer → main) | Chunk files |
|---|---|
| `app-shell-shortcut-state-changed` | app-initial-6350d04672d0.js |
| `avatar-overlay-close` | avatar-overlay-native-page-7ec1028a9c34.js |
| `avatar-overlay-drag-end` | avatar-overlay-native-page-7ec1028a9c34.js |
| `avatar-overlay-drag-move` | avatar-overlay-native-page-7ec1028a9c34.js |
| `avatar-overlay-drag-release` | avatar-overlay-native-page-7ec1028a9c34.js |
| `avatar-overlay-drag-start` | avatar-overlay-native-page-7ec1028a9c34.js |
| `avatar-overlay-element-size-changed` | avatar-overlay-native-page-7ec1028a9c34.js |
| `avatar-overlay-keyboard-interaction-changed` | avatar-overlay-native-page-7ec1028a9c34.js |
| `avatar-overlay-open` | app-initial-6350d04672d0.js, pets-settings-route-d66810db325d.js |
| `avatar-overlay-open-state-request` | app-initial-6350d04672d0.js |
| `avatar-overlay-pointer-interaction-changed` | avatar-overlay-native-page-7ec1028a9c34.js |
| `browser-sidebar-annotation-multi-select-enabled-changed` | app-primary-6f60e9ac4103.js |
| `browser-sidebar-attach-dragged-image` | app-primary-6f60e9ac4103.js |
| `browser-sidebar-command` | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js |
| `browser-sidebar-comment-overlay-close` | app-primary-6f60e9ac4103.js |
| `browser-sidebar-comment-overlay-submit` | app-primary-6f60e9ac4103.js |
| `browser-sidebar-owner-sync` | app-primary-6f60e9ac4103.js |
| `browser-sidebar-site-annotation-api-enabled-changed` | app-initial-6350d04672d0.js |
| `browser-sidebar-sync` | tab-content-24fe1f531bbe.js |
| `browser-sidebar-tweaks-enabled-changed` | app-initial-6350d04672d0.js |
| `browser-sidebar-webview-destroyed` | app-initial-6350d04672d0.js |
| `browser-use-cursor-arrived` | tab-content-24fe1f531bbe.js |
| `cancel-fetch` | app-shared-8bd9d0399038.js |
| `checkout-webview-presentation-changed` | webview-page-f15129326e35.js |
| `codex-app-server-restart` | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js |
| `codex-runtimes-config-changed` | app-initial-6350d04672d0.js |
| `electron-add-new-workspace-root-option` | app-initial-6350d04672d0.js |
| `electron-app-state-snapshot-response` | app-initial-6350d04672d0.js |
| `electron-avatar-overlay-feedback-diagnostics-changed` | app-initial-6350d04672d0.js |
| `electron-avatar-overlay-restore-ready` | app-initial-6350d04672d0.js |
| `electron-desktop-features-changed` | app-initial-6350d04672d0.js |
| `electron-onboarding-skip-workspace` | select-workspace-page-93a123d78c93.js |
| `electron-pick-workspace-root-option` | app-initial-6350d04672d0.js, security-route-d4f986069805.js |
| `electron-set-badge-count` | app-initial-6350d04672d0.js |
| `electron-set-window-mode` | app-initial-6350d04672d0.js |
| `electron-sparkle-gates-changed` | app-initial-6350d04672d0.js |
| `electron-window-focus-request` | app-initial-6350d04672d0.js |
| `electron-window-zoom-changed` | app-initial-6350d04672d0.js |
| `fetch` | app-shared-8bd9d0399038.js |
| `global-dictation-dismiss` | app-initial-6350d04672d0.js |
| `global-dictation-enabled-changed` | app-initial-6350d04672d0.js |
| `global-dictation-force-lock-changed` | panel-fb651e5a5a08.js |
| `global-dictation-in-app-started` | app-initial-6350d04672d0.js |
| `global-dictation-record-history-item` | app-initial-6350d04672d0.js |
| `heartbeat-automation-thread-state-changed` | app-initial-6350d04672d0.js |
| `inbox-automation-runs-mark-all-read` | app-initial-6350d04672d0.js |
| `inbox-item-set-read-state` | app-initial-6350d04672d0.js |
| `inbox-items-create` | app-shared-8bd9d0399038.js |
| `keyboard-layout-map-changed` | app-initial-6350d04672d0.js |
| `local-thread-activity-changed` | app-initial-6350d04672d0.js |
| `log-message` | app-initial-6350d04672d0.js |
| `mac-menu-bar-enabled-changed` | app-initial-6350d04672d0.js |
| `open-browser-in-main-window` | app-initial-6350d04672d0.js |
| `open-config-toml` | app-initial-6350d04672d0.js |
| `open-current-main-window` | app-initial-6350d04672d0.js, avatar-overlay-native-page-7ec1028a9c34.js |
| `open-in-browser` | app-initial-6350d04672d0.js, local-conversation-thread-c8c5e92ea89e.js |
| `open-in-main-window` | app-initial-6350d04672d0.js, avatar-overlay-native-page-7ec1028a9c34.js |
| `open-in-new-window` | app-initial-6350d04672d0.js, local-conversation-page-a83a1af2c24b.js |
| `pending-worktree-cancel` | app-initial-6350d04672d0.js |
| `pending-worktree-continue` | app-initial-6350d04672d0.js |
| `pending-worktree-create` | app-initial-6350d04672d0.js |
| `pending-worktree-dismiss` | app-initial-6350d04672d0.js |
| `pending-worktree-retry` | app-initial-6350d04672d0.js |
| `pending-worktree-update-metadata` | app-initial-6350d04672d0.js |
| `persisted-atom-sync-request` | app-initial-6350d04672d0.js |
| `persisted-atom-update` | app-initial-6350d04672d0.js |
| `power-save-blocker-set` | app-initial-6350d04672d0.js |
| `ready` | app-initial-6350d04672d0.js |
| `remote-hosted-pip-active-thread-changed` | app-initial-6350d04672d0.js |
| `set-primary-runtime-install-release` | panel-fb651e5a5a08.js |
| `set-telemetry-user` | app-initial-6350d04672d0.js |
| `shared-object-set` | app-shared-8bd9d0399038.js |
| `shared-object-subscribe` | app-shared-8bd9d0399038.js |
| `shared-object-unsubscribe` | app-shared-8bd9d0399038.js |
| `show-settings` | avatar-overlay-native-page-7ec1028a9c34.js, surface-774dbd4a6f15.js |
| `toggle-trace-recording` | app-initial-6350d04672d0.js |
| `tray-menu-threads-changed` | app-initial-6350d04672d0.js |
| `update-diff-if-open` | local-conversation-turn-632904a8790c.js |
| `view-focused` | app-initial-6350d04672d0.js |

### 11.3 Main → renderer message types (51 sent with `sendMessageToWindow`; the renderer handles 196 types in total, see section 17)

`archive-thread`, `chat-search-command-menu`, `codex-micro-device-lifecycle`, `codex-micro-hid-event`, `codex-micro-joystick-event`, `codex-micro-window-ownership-changed`, `codex-ui-render-process-gone`, `command-menu`, `connector-oauth-callback`, `copy-conversation-path`, `copy-deeplink`, `copy-working-directory`, `debug-run-eval-control-request`, `debug-window-client-analytics-history-changed`, `debug-window-origin-conversation-changed`, `electron-window-focus-changed`, `electron-window-opaque-surface-changed`, `electron-window-position-changed`, `file-search-command-menu`, `find-in-thread`, `find-next-in-thread`, `find-previous-in-thread`, `focus-composer`, `heartbeat-automation-thread-state-request`, `hotkey-window-transition`, `navigate-back`, `navigate-forward`, `navigate-to-route`, `new-projectless-task`, `open-agent-onboarding`, `open-browser-tab`, `open-pet-install-modal`, `rename-thread`, `reset-window-zoom`, `run-command`, `screenshot-selection-shortcut`, `show-chatgpt-rosalind-announcement`, `step-window-zoom`, `toggle-bottom-panel`, `toggle-browser-panel`, `toggle-debug-modal`, `toggle-diff-panel`, `toggle-query-devtools`, `toggle-ram-usage-indicator`, `toggle-react-scan`, `toggle-sidebar`, `toggle-terminal`, `toggle-thread-pin`, `visual-content-action-clicked`, `window-fullscreen-changed`, `workspace-root-option-added`.

- Menu commands without a dedicated type arrive as `run-command` with `id` and `keyboardEvent` (for example `settings`, `logOut`, `newTask`).
- Menu toggles map to: `toggle-sidebar`, `toggle-bottom-panel`, `toggle-terminal`, `toggle-diff-panel`, `toggle-thread-pin`, `rename-thread`, `archive-thread`, `find-in-thread`, `find-next-in-thread`, `find-previous-in-thread`, `navigate-back`, `navigate-forward`, `navigate-to-route`, `command-menu`, `file-search-command-menu`, `chat-search-command-menu`, `copy-working-directory`, `copy-deeplink`, `copy-conversation-path`, `focus-composer`, `new-projectless-task`, `open-browser-tab`, `step-window-zoom`, `reset-window-zoom`.

## 12. Keyboard shortcuts

Source: command registry in app-initial-6350d04672d0.js (objects `{id, titleIntlId, …, electron: {defaultKeybindings, platformDefaultKeybindings: {macOS, default}}}`), English titles in command-messages-4fe194d31bfc.js, menu-only commands in `.vite/build/src-BJv5Mk7n.js`.
Resolution on macOS: `electron.platformDefaultKeybindings.macOS` if present, else `electron.defaultKeybindings`; `browser` keys do not apply in Electron.
`CmdOrCtrl` is `⌘` on macOS.
Scope `app` means the renderer handles the key; `OS-global` keys are registered system-wide; `menu/default` keys are menu accelerators.
`thread1..9` change to tab switching when the setting “number shortcuts switch tabs” is on (`primaryNumberShortcutTarget`).

| Command id | Title | macOS default keys | Scope | Group | In command menu | Source |
|---|---|---|---|---|---|---|
| `spotlightSearch` | Spotlight search | `Alt+K` | OS-global | — | no | app-initial-6350d04672d0.js |
| `toggleNotch` | Toggle Notch | `Alt+C` | OS-global | — | no | app-initial-6350d04672d0.js |
| `notchVoice` | Voice in Mini | `Alt+V` | OS-global | — | no | app-initial-6350d04672d0.js |
| `screenshotToCurrentChat` | Screenshot selection to current chat | `Control+Shift+4` | OS-global | — | no | app-initial-6350d04672d0.js |
| `screenshotToNewChat` | Screenshot selection to new chat | `Control+Alt+Shift+4` | OS-global | — | no | app-initial-6350d04672d0.js |
| `hotkeyWindow` | Popout Window hotkey | — | OS-global | — | no | app-initial-6350d04672d0.js |
| `globalDictationHold` | Dictation shortcut | — | OS-global | — | no | app-initial-6350d04672d0.js |
| `globalDictationSingleTap` | Single-tap dictation | — | OS-global | — | no | app-initial-6350d04672d0.js |
| `realtimeVoice` | codex.command.realtimeVoice | — | OS-global | — | no | app-initial-6350d04672d0.js |
| `newTask` | New chat | `CmdOrCtrl+N`, `CmdOrCtrl+Shift+O` | menu/default | thread | yes | app-initial-6350d04672d0.js |
| `newProjectlessTask` | New standalone chat | `CmdOrCtrl+Alt+O` | menu/default | thread | yes | app-initial-6350d04672d0.js |
| `temporaryChat` | New Temporary Chat | `CmdOrCtrl+Shift+N` | app | thread | yes | app-initial-6350d04672d0.js |
| `openThreadInNewWindow` | Open in new window | — | menu/default | thread | yes | app-initial-6350d04672d0.js |
| `archiveThread` | Archive chat | `CmdOrCtrl+Shift+A` | menu/default | thread | yes | app-initial-6350d04672d0.js |
| `deleteChat` | Delete chat | — | menu/default | thread | no | app-initial-6350d04672d0.js |
| `markThreadUnread` | Mark as unread | `CmdOrCtrl+Shift+U` | app | thread | yes | app-initial-6350d04672d0.js |
| `toggleThreadPin` | Toggle pin | `CmdOrCtrl+Alt+P` | menu/default | thread | yes | app-initial-6350d04672d0.js |
| `copyConversationMarkdown` | Copy as Markdown | — | menu/default | — | no | app-initial-6350d04672d0.js |
| `copyLastCodeBlock` | Copy last code block | `CmdOrCtrl+Shift+;` | app | thread | yes | app-initial-6350d04672d0.js |
| `openSideChat` | Open side chat | `CmdOrCtrl+Alt+S` | app | thread | yes | app-initial-6350d04672d0.js |
| `openControlWindow` | Open control window | — | menu/default | app | yes | app-initial-6350d04672d0.js |
| `toggleDebugModal` | Toggle debug panel | `Ctrl+D` | app | — | no | app-initial-6350d04672d0.js |
| `undoAppAction` | Undo last action | `CmdOrCtrl+Z` | app | app | yes | app-initial-6350d04672d0.js |
| `redoAppAction` | Redo last action | `Command+Shift+Z` | app | app | yes | app-initial-6350d04672d0.js |
| `reopenClosedTab` | Reopen closed tab | `CmdOrCtrl+Shift+T` | app | panels | yes | app-initial-6350d04672d0.js |
| `forkThread` | Fork chat | — | app | — | no | app-initial-6350d04672d0.js |
| `openAvatarOverlay` | Show Mini | `Alt+Space` | OS-global | app | yes | app-initial-6350d04672d0.js |
| `searchChats` | Switch chat… | — | menu/default | navigation | yes | app-initial-6350d04672d0.js |
| `togglePriorityFilter` | Toggle activity view | `CmdOrCtrl+Alt+U` | app | navigation | yes | app-initial-6350d04672d0.js |
| `focusMainChat` | Focus main chat | — | app | navigation | yes | app-initial-6350d04672d0.js |
| `focusSideChat` | Focus side chat | — | app | navigation | yes | app-initial-6350d04672d0.js |
| `copyPageLink` | Copy Page link | `CmdOrCtrl+L` | app | navigation | yes | app-initial-6350d04672d0.js |
| `focusPageBlockControls` | codex.command.focusPageBlockControls | `Shift+F10` | app | — | no | app-initial-6350d04672d0.js |
| `openOrbit` | codex.command.openOrbit.dot | `CmdOrCtrl+.` | app | navigation | yes | app-initial-6350d04672d0.js |
| `nextThreadNeedingAttention` | Next chat needing attention | `CmdOrCtrl+Alt+A` | app | navigation | yes | app-initial-6350d04672d0.js |
| `previousTab` | Previous tab | `Ctrl+Shift+Tab`, `Command+Shift+[`, `Command+Alt+Left` | app | navigation | no | app-initial-6350d04672d0.js |
| `previousThread` | Previous chat | `Command+Shift+[`, `Command+Alt+Left` | menu/default | navigation | yes | app-initial-6350d04672d0.js |
| `previousRecentThread` | Previous recently viewed chat | `Ctrl+Shift+Tab` | app | navigation | no | app-initial-6350d04672d0.js |
| `nextTab` | Next tab | `Ctrl+Tab`, `Command+Shift+]`, `Command+Alt+Right` | app | navigation | no | app-initial-6350d04672d0.js |
| `nextThread` | Next chat | `Command+Shift+]`, `Command+Alt+Right` | menu/default | navigation | yes | app-initial-6350d04672d0.js |
| `nextRecentThread` | Next recently viewed chat | `Ctrl+Tab` | app | navigation | no | app-initial-6350d04672d0.js |
| `recentThread1` | Go to recent chat 1 | `CmdOrCtrl+Alt+1` | app | navigation | yes | app-initial-6350d04672d0.js |
| `recentThread2` | Go to recent chat 2 | `CmdOrCtrl+Alt+2` | app | navigation | yes | app-initial-6350d04672d0.js |
| `recentThread3` | Go to recent chat 3 | `CmdOrCtrl+Alt+3` | app | navigation | yes | app-initial-6350d04672d0.js |
| `recentThread4` | Go to recent chat 4 | `CmdOrCtrl+Alt+4`, `CmdOrCtrl+Shift+9` | app | navigation | yes | app-initial-6350d04672d0.js |
| `recentThread5` | Go to recent chat 5 | `CmdOrCtrl+Alt+5`, `CmdOrCtrl+Shift+8` | app | navigation | yes | app-initial-6350d04672d0.js |
| `recentThread6` | Go to recent chat 6 | `CmdOrCtrl+Alt+6` | app | navigation | yes | app-initial-6350d04672d0.js |
| `switchToMode1` | Switch to Chat | `Ctrl+1` | app | navigation | yes | app-initial-6350d04672d0.js |
| `switchToMode2` | Switch to Work | `Ctrl+2` | app | navigation | yes | app-initial-6350d04672d0.js |
| `switchToMode3` | Switch to Codex | `Ctrl+3` | app | navigation | yes | app-initial-6350d04672d0.js |
| `settings` | Settings | `CmdOrCtrl+,` | menu/default | app | yes | app-initial-6350d04672d0.js |
| `codexMicroSettings` | Codex Micro setup | — | menu/default | configure | no | app-initial-6350d04672d0.js |
| `mcpSettings` | MCP | — | menu/default | configure | yes | app-initial-6350d04672d0.js |
| `personalitySettings` | codex.command.personalitySettings | — | menu/default | configure | yes | app-initial-6350d04672d0.js |
| `importExternalAgent` | codex.command.importExternalAgent | — | menu/default | configure | yes | app-initial-6350d04672d0.js |
| `keyboardShortcuts` | Keyboard shortcuts | — | menu/default | configure | yes | app-initial-6350d04672d0.js |
| `showKeyboardShortcuts` | Show keyboard shortcuts | `CmdOrCtrl+/` | app | — | no | app-initial-6350d04672d0.js |
| `clearAllUnreads` | Clear all unreads | `Shift+Escape` | app | app | yes | app-initial-6350d04672d0.js |
| `manageTasks` | Manage scheduled tasks | — | menu/default | app | yes | app-initial-6350d04672d0.js |
| `forceReloadSkills` | Force reload skills | — | menu/default | skills | yes | app-initial-6350d04672d0.js |
| `openSkills` | Go to skills | — | menu/default | skills | yes | app-initial-6350d04672d0.js |
| `newProject` | New project | — | menu/default | workspace | yes | app-initial-6350d04672d0.js |
| `openFolder` | Open folder | `CmdOrCtrl+O` | menu/default | workspace | yes | app-initial-6350d04672d0.js |
| `toggleSidebar` | Toggle sidebar | `CmdOrCtrl+Shift+S`, `CmdOrCtrl+B` | menu/default | panels | yes | app-initial-6350d04672d0.js |
| `toggleFileTreePanel` | Toggle navigation panel | `CmdOrCtrl+Shift+E` | app | panels | yes | app-initial-6350d04672d0.js |
| `toggleBottomPanel` | Toggle bottom panel | `CmdOrCtrl+J` | menu/default | panels | yes | app-initial-6350d04672d0.js |
| `togglePinnedSummary` | Toggle pinned summary | — | menu/default | panels | yes | app-initial-6350d04672d0.js |
| `toggleTerminal` | Open terminal | — | menu/default | panels | yes | app-initial-6350d04672d0.js |
| `openBrowserTab` | Open browser tab | `CmdOrCtrl+T` | menu/default | panels | yes | app-initial-6350d04672d0.js |
| `closeOtherTabs` | Close other tabs | `CmdOrCtrl+Alt+W` | app | — | no | app-initial-6350d04672d0.js |
| `showWorkspaceTabView` | Full view | `CmdOrCtrl+Shift+F` | app | panels | yes | app-initial-6350d04672d0.js |
| `stepWorkspaceLayout` | Cycle workspace layout | `CmdOrCtrl+Shift+B` | app | panels | yes | app-initial-6350d04672d0.js |
| `openReviewTab` | Open Changes tab | `Ctrl+Shift+G` | app | panels | yes | app-initial-6350d04672d0.js |
| `toggleReviewTab` | Toggle Changes | — | app | panels | no | app-initial-6350d04672d0.js |
| `toggleSidePanel` | Toggle Changes panel | `CmdOrCtrl+Alt+B` | menu/default | panels | yes | app-initial-6350d04672d0.js |
| `toggleMaximizeSidePanel` | Toggle maximize side panel | — | app | — | no | app-initial-6350d04672d0.js |
| `findInThread` | Find | `Command+F` | menu/default | navigation | yes | app-initial-6350d04672d0.js |
| `goToLine` | Go to line | `CmdOrCtrl+L` | app | navigation | yes | app-initial-6350d04672d0.js |
| `focusBrowserAddressBar` | Focus browser address bar | `CmdOrCtrl+L` | menu/default | navigation | yes | app-initial-6350d04672d0.js |
| `navigateBrowserBack` | Browser back | `Command+Left` | app | — | no | app-initial-6350d04672d0.js |
| `navigateBrowserForward` | Browser forward | `Command+Right` | app | — | no | app-initial-6350d04672d0.js |
| `navigateBack` | Back | `CmdOrCtrl+[`, `MouseBack` | menu/default | navigation | yes | app-initial-6350d04672d0.js |
| `navigateForward` | Forward | `CmdOrCtrl+]`, `MouseForward` | menu/default | navigation | yes | app-initial-6350d04672d0.js |
| `logOut` | Log out | — | menu/default | app | yes | app-initial-6350d04672d0.js |
| `feedback` | Feedback | — | menu/default | app | yes | app-initial-6350d04672d0.js |
| `environmentAction1` | Environment action 1 | `Command+Shift+D` | app | workspace | no | app-initial-6350d04672d0.js |
| `environmentAction2` | Environment action 2 | — | app | workspace | no | app-initial-6350d04672d0.js |
| `environmentAction3` | Environment action 3 | — | app | workspace | no | app-initial-6350d04672d0.js |
| `environmentAction4` | Environment action 4 | — | app | workspace | no | app-initial-6350d04672d0.js |
| `environmentAction5` | Environment action 5 | — | app | workspace | no | app-initial-6350d04672d0.js |
| `environmentAction6` | Environment action 6 | — | app | workspace | no | app-initial-6350d04672d0.js |
| `environmentAction7` | Environment action 7 | — | app | workspace | no | app-initial-6350d04672d0.js |
| `environmentAction8` | Environment action 8 | — | app | workspace | no | app-initial-6350d04672d0.js |
| `environmentAction9` | Environment action 9 | — | app | workspace | no | app-initial-6350d04672d0.js |
| `thread1` | Go to chat 1 | `CmdOrCtrl+1` | menu/default | — | no | app-initial-6350d04672d0.js |
| `thread2` | Go to chat 2 | `CmdOrCtrl+2` | menu/default | — | no | app-initial-6350d04672d0.js |
| `thread3` | Go to chat 3 | `CmdOrCtrl+3` | menu/default | — | no | app-initial-6350d04672d0.js |
| `thread4` | Go to chat 4 | `CmdOrCtrl+4` | menu/default | — | no | app-initial-6350d04672d0.js |
| `thread5` | Go to chat 5 | `CmdOrCtrl+5` | menu/default | — | no | app-initial-6350d04672d0.js |
| `thread6` | Go to chat 6 | `CmdOrCtrl+6` | menu/default | — | no | app-initial-6350d04672d0.js |
| `thread7` | Go to chat 7 | `CmdOrCtrl+7` | menu/default | — | no | app-initial-6350d04672d0.js |
| `thread8` | Go to chat 8 | `CmdOrCtrl+8` | menu/default | — | no | app-initial-6350d04672d0.js |
| `thread9` | Go to chat 9 | `CmdOrCtrl+9` | menu/default | — | no | app-initial-6350d04672d0.js |
| `openCommandMenu` | Open command menu | `CmdOrCtrl+K`, `CmdOrCtrl+Shift+P` | menu | — | no | .vite/build/src-BJv5Mk7n.js |
| `searchFiles` | Search Files… | `CmdOrCtrl+P` | menu | — | no | .vite/build/src-BJv5Mk7n.js |
| `renameThread` | Rename chat | `CmdOrCtrl+Alt+R` | menu | — | no | .vite/build/src-BJv5Mk7n.js |
| `copyWorkingDirectory` | Copy working directory | `CmdOrCtrl+Shift+C` | menu | — | no | .vite/build/src-BJv5Mk7n.js |
| `copyDeeplink` | Copy deeplink | `CmdOrCtrl+Alt+L` | menu | — | no | .vite/build/src-BJv5Mk7n.js |
| `copyConversationPath` | Copy conversation path | `CmdOrCtrl+Alt+Shift+C` | menu | — | no | .vite/build/src-BJv5Mk7n.js |
| `closeTab` | Close Tab | `CmdOrCtrl+W` | menu | — | no | .vite/build/src-BJv5Mk7n.js |
| `closeWindow` | Close | `CmdOrCtrl+W` | menu | — | no | .vite/build/src-BJv5Mk7n.js |
| `newWindow` | New Window | — | menu | — | no | .vite/build/src-BJv5Mk7n.js |

Fixed menu accelerators (main): `Command+G` find next, `Command+Shift+G` find previous, `CmdOrCtrl+Plus` / `CmdOrCtrl+-` / `CmdOrCtrl+0` zoom, `Command+Control+F` full screen, `Command+Control+R` reload window, `Command+Option+Shift+V` paste and match style, `CmdOrCtrl+Alt+Y` query devtools (internal).

## 13. Menu bar (macOS)

Built in `.vite/build/main-CqEoQDOU.js` (template `Ut`, rebuilt on feature or keymap change).

- **Codex** (app menu): `About {appName}`, separator, `Settings…` (`⌘,`), `Check for Updates…` (when updater exists), `Log Out`, separator, `Services`, separator, `Hide {appName}`, `Hide Others`, `Show All`, separator, `Quit {appName}`.
- **File**: `New Window` (multi-window only), `New Chat` (`⌘N`), `New Temporary Chat` (`⌘⇧N`, enabled when allowed), `New Project…`, `New standalone chat` (`⌘⌥O`), separator, `Open Folder…` (`⌘O`), separator, `Close` (`⌘W`).
- **Edit**: `Undo`, `Redo`, `Cut`, `Copy`, `Paste`, `Paste and Match Style` (`⌘⇧V`), `Delete`, `Select All`, `Substitutions` ▸, `Speech` ▸.
- **View**: `Toggle Sidebar` (`⌘⇧S`, also `⌘B`), `Toggle Bottom Panel` (`⌘J`), `Toggle Pinned Summary`, `Open Terminal`, `Toggle navigation panel` (`⌘⇧E`), `Full view` (`⌘⇧F`, unified tab strip only), `Toggle Changes Panel` (`⌘⌥B`; label `Switch between Chat and tabs` with unified tab strip), `Browser` ▸ (browser pane only), separator, `Find` (`⌘F`), separator, `Previous Chat` (`⌘⇧[`), `Next Chat` (`⌘⇧]`), `Back` (`⌘[`), `Forward` (`⌘]`), `Reload Window` (dev), separator, `Zoom In`, `Zoom Out`, `Actual Size`, `Developer` ▸ (dev), separator, hidden full screen item; hidden accelerators: command menu (`⌘K`, `⌘⇧P`), `Search Chats…`, `Search Files…` (`⌘P`), pin (`⌘⌥P`), rename (`⌘⌥R`), archive (`⌘⇧A`), copy working directory (`⌘⇧C`), copy deeplink (`⌘⌥L`), copy conversation path (`⌘⌥⇧C`), `Go to Chat 1…9`.
- **Window**: `Minimize`, `Zoom`, `Bring All to Front`.
- **Help**: `Documentation` (opens developers.openai.com/codex/app), `Keyboard Shortcuts` (`⌘/`), `What's New`, separator, `Troubleshooting`, `System Status`, `Send Feedback`, `Task Manager`, trace items.

## 14. Flow checklists

Use one row per check during the flow parity pass.
The three resize widths are 1280 px (reference), 767 px (below the 768 px sidebar breakpoint) and 600 px (below the 40rem composer breakpoint).

### 14.1 Left rail

| # | Item | Kind | Expected behavior | Source | Pass |
|---|---|---|---|---|---|
| 1 | Home | click | Navigates to `/`; reselect resets Home. | `sidebarElectron.rail.homeDestination` | [ ] |
| 2 | Space | click | Navigates to `/space`. | `sidebarElectron.spaceRouteNavLink` | [ ] |
| 3 | Scheduled | click | Navigates to `/scheduled` (Automations); unread dot. | `sidebarElectron.inboxRouteNavLink` | [ ] |
| 4 | Plugins | click | Navigates to `/plugins` (Skills/Plugins page). | `sidebarElectron.skillsAppsRouteNavLink` | [ ] |
| 5 | Explore `…` | click | Opens the Explore menu with secondary destinations; blue dot for new items. | `sidebarCustomization.moreMenu` | [ ] |
| 6 | Code Review | click | Navigates to `/pull-requests`. | `sidebarElectron.codeReviewRouteNavLink` | [ ] |
| 7 | Sites | click | Navigates to `/sites`. | `sidebarElectron.sitesRouteNavLink` | [ ] |
| 8 | Profile avatar | click | Opens profile menu. | `codex.profileFooter.openProfileMenu` | [ ] |
| 9 | Rail item hover | hover | Tooltip on the right side with label and shortcut. | tooltip `side:right` | [ ] |
| 10 | Drag a rail item | click | Reorders pinned items (dnd-kit, 6 px activation). | `sidebarElectron.rail.reorder.*` | [ ] |
| 11 | Focus ring | focus ring | 2 px ring on the 36×36 button. | `_Button_eg66f_2` | [ ] |
| 12 | Resize | resize | Check at 1280, 767 and 600 px window width; rail stays 52 px, items scroll with fade mask. | `vertical-scroll-fade-mask` | [ ] |

### 14.2 Sidebar

| # | Item | Kind | Expected behavior | Source | Pass |
|---|---|---|---|---|---|
| 1 | Sidebar visible | entry | Rail 52 px plus panel 288 px at 1280 px. | refs/home spec `nav` rects | [ ] |
| 2 | `Codex ⌄` mode switch | click | Menu: ChatGPT, Codex, Work with descriptions. | `sidebarElectron.productMode.*` | [ ] |
| 3 | `⌃1` `⌃2` `⌃3` | shortcut | Switch to Chat / Work / Codex mode. | commands `switchToMode1..3` | [ ] |
| 4 | Bell `View activity` | click | Toggles the activity (priority) view of the thread list. | `sidebarElectron.priorityThreads.filterByPriority` | [ ] |
| 5 | `⌘⌥U` | shortcut | Toggles the activity view. | command `togglePriorityFilter` | [ ] |
| 6 | Search | click | Opens the command menu in chat-search mode. | host message `chat-search-command-menu` | [ ] |
| 7 | `⌘K` | shortcut | Opens the command menu (tooltip of Search shows this key). | command `openCommandMenu` | [ ] |
| 8 | `Your dot` row | click | Opens the dot (orbit) thread. | `sidebarElectron.orbitNavigation.dot` | [ ] |
| 9 | `Projects` section toggle | click | Collapses or expands the section; chevron rotates. | button `group/section-toggle` | [ ] |
| 10 | `Recents` section toggle | click | Collapses or expands the list. | `sidebarElectron.chatGptWork.recents` | [ ] |
| 11 | Thread row | click | Opens `/local/:conversationId`. | app-initial-6350d04672d0.js | [ ] |
| 12 | Thread row hover | hover | Row background highlights; [INFERENCE] trailing Archive / Pin short actions appear. | `sidebarElectron.archiveThreadShort`, `pinThreadShort` | [ ] |
| 13 | Thread row context menu | click | Pin, Rename, Archive, Mark as unread, Open in new window, Permanently delete. | `sidebarElectron.*Thread*` | [ ] |
| 14 | `⌘1`…`⌘9` | shortcut | Go to chat N. | commands `thread1..9` | [ ] |
| 15 | `⌘⇧[` / `⌘⇧]` | shortcut | Previous / next chat. | commands `previousThread`, `nextThread` | [ ] |
| 16 | `⌘⇧S` / `⌘B` | shortcut | Toggle sidebar. | command `toggleSidebar` | [ ] |
| 17 | Titlebar `Hide sidebar` | click | Collapses the left panel; label changes to `Show sidebar`. | `app.sidebar.hide` | [ ] |
| 18 | Row focus | focus ring | 2 px ring outline, offset -2 px. | spec row classes `focus-visible:outline-ring` | [ ] |
| 19 | Empty lists | empty | `No projects`, `No chats`, activity: `Nothing needs attention`. | `sidebarElectron.*empty*` | [ ] |
| 20 | Lists loading | loading | Activity: `Checking unread chats…`. | `sidebarElectron.activity.loading` | [ ] |
| 21 | Load error | error | `Projects couldn't be loaded` + `Retry`; `Could not load chats`. | `sidebarElectron.projectsError`, `tasksSection.error` | [ ] |
| 22 | Sidebar drag-resize | resize | Width clamps to 240…min(520, window−240); default 340; stored as `sidebar-width`. | app-initial `dC()`, `qRt` | [ ] |
| 23 | Resize | resize | Check at 1280, 767 and 600 px window width; below 768 px the panel is hidden (`data-app-shell-responsive-sidebar`). | app-initial-99e8b4b455fa.css | [ ] |

### 14.3 Home

| # | Item | Kind | Expected behavior | Source | Pass |
|---|---|---|---|---|---|
| 1 | Launch to `/` | entry | Home renders with hero, announcement slot and composer. | app-primary-6f60e9ac4103.js | [ ] |
| 2 | `New chat` row (sidebar) | click | Navigates to `/` and clears the composer draft. | app-initial-6350d04672d0.js `sidebarElectron.newThread` | [ ] |
| 3 | `⌘N` / `⌘⇧O` | shortcut | Opens a new chat on Home and focuses the composer. | command `newTask` | [ ] |
| 4 | Hero project selector `{projectName}?` | click | Opens the project menu (only when a project is set). | `home.hero.whatShouldWeBuildInSelectedProject` | [ ] |
| 5 | Beacon `Start Voice` | click | Starts a realtime voice chat. | codex-app-home-beacon-announcement-*.js | [ ] |
| 6 | Beacon dismiss `×` | click | Hides the banner; [INFERENCE] dismissal persists. | aria `Dismiss ChatGPT beacon banner` | [ ] |
| 7 | `Choose project` | click | Opens the project dropdown with search, `Add new project`, `Don't work in a project`. | `composer.localCwdDropdown.*` app-primary | [ ] |
| 8 | `Work in` / `This computer` | click | Opens the location menu: This computer, Worktree, Remote, Cloud. | composer-utility-bar-7738f1184086.js | [ ] |
| 9 | Editor `Do anything` | focus | Caret shows, placeholder stays until first character. | `homePage.composer.placeholder.askAnything.v2` | [ ] |
| 10 | `+` Add files and more | click | Opens the context menu (files, photos, @-mentions). | `composer.contextButton.ariaLabel` | [ ] |
| 11 | `Custom` permissions | click | Opens the permissions dropdown. | permissions-mode-dropdown-dc16e1c7f7ad.js | [ ] |
| 12 | Model picker `GPT-6 Astra Light` | click | Opens model and effort menu. | `composer.intelligenceDropdown.tooltip` | [ ] |
| 13 | `Dictate` | click | Starts dictation; button changes to stop. | `composer.dictation.aria` | [ ] |
| 14 | `Start new voice chat` | click | Starts realtime voice. | `composer.realtime.startNewVoiceChat` | [ ] |
| 15 | Submit with Enter | shortcut | Sends `thread/start` then `turn/start` and navigates to `/local/:conversationId`. | app-shared-8bd9d0399038.js | [ ] |
| 16 | Hover on every button | hover | Ghost background fades in (`:hover:before` opacity 1, scale 1). | app-shared-9a94b907ed47.css `_Button_eg66f_2` | [ ] |
| 17 | Tab through controls | focus ring | 2 px `--color-ring` outline on `:focus-visible:after`. | app-shared-9a94b907ed47.css | [ ] |
| 18 | No projects / no chats | empty | Sidebar shows `No projects` and `No chats`; hero shows `What should we build?`. | `sidebarElectron.flatProjectSidebar.noProjects`, `sidebarElectron.noTasks` | [ ] |
| 19 | Model list loading | loading | Model picker shows `Loading model…`. | `composer.intelligenceDropdown.loadingModel` | [ ] |
| 20 | Account load failure | error | Full page `We couldn’t load your account` with `Reload` and `Sign out`. | `home.accountAccessError.*` | [ ] |
| 21 | Resize | resize | Check at 1280, 767 and 600 px window width; below 768 px the sidebar becomes an overlay; below 40rem the composer stacks. | `KRt=(min-width: 768px)`, `(width < 40rem)` | [ ] |

### 14.4 Thread view

| # | Item | Kind | Expected behavior | Source | Pass |
|---|---|---|---|---|---|
| 1 | Open `/local/:conversationId` | entry | `thread/resume` then turns render; scroll to bottom. | local-conversation-page-*.js | [ ] |
| 2 | History loading | loading | `Loading task…` / skeleton; reconnect notice `Reconnecting to ChatGPT…`. | thread-recovery-notice-e3e7b73ead54.js | [ ] |
| 3 | History failure | error | `Couldn't load messages` + `Retry`. | `localConversation.historyLoadFailed` | [ ] |
| 4 | Turn render failure | error | `This turn couldn't render` + `Try again`. | `localConversation.turnRenderError.*` | [ ] |
| 5 | Streaming agent message | loading | Text grows by `item/agentMessage/delta`; `Working` / `Thinking` shimmer. | sites-end-resource-78b98c374310.js | [ ] |
| 6 | Reasoning item | click | Toggles `Thought for {elapsed}` details. | `reasoningItem.*` | [ ] |
| 7 | Command card | click | Expands command output; copy command/output buttons. | `toolSummaryForCmd.*`, `codex.shell.*` | [ ] |
| 8 | File change card | click | `View changes` opens Changes panel; `Undo` / `Reapply`. | `codex.unifiedDiff.*` | [ ] |
| 9 | Collapsed turn `Worked for {time}` | click | Expands hidden work items. | collapsed-turn-disclosure-5e77ff7d3908.js | [ ] |
| 10 | User message hover | hover | Copy and Edit buttons appear. | `codex.userMessage.copyAriaLabel`, `editAriaLabel` | [ ] |
| 11 | Assistant message hover | hover | Copy response, Fork chat from here. | `assistantMessageContent.*` | [ ] |
| 12 | `Scroll to bottom` | click | Jumps to the newest turn; hidden when at bottom. | `localConversation.scrollToBottomButton` | [ ] |
| 13 | `⌘F` | shortcut | Opens find bar `Search chat…`; `⌘G` / `⌘⇧G` next/previous. | `codex.threadFindBar.*` | [ ] |
| 14 | `⌘⇧A` | shortcut | Archives the chat. | command `archiveThread` | [ ] |
| 15 | `⌘⌥R` | shortcut | Rename chat dialog. | command `renameThread` | [ ] |
| 16 | Approval card | click | Approve, `Allow similar commands`, `Allow all edits`, Deny. | pending-request-item-panel-770855267df2.js | [ ] |
| 17 | Stream error | error | `Reconnecting {progress}` / `Server is busy, reconnecting`. | `localConversation.streamError.*` | [ ] |
| 18 | Usage limit | error | `You've hit your usage limit…` banner. | `localConversation.usageLimit.*` | [ ] |
| 19 | Focus ring on cards | focus ring | 2 px ring. | app-shared-9a94b907ed47.css | [ ] |
| 20 | Resize | resize | Check at 1280, 767 and 600 px window width; [INFERENCE] side panel yields width before the chat column. | thread-app-shell-chrome-c6e5ce470743.js | [ ] |

### 14.5 Composer

| # | Item | Kind | Expected behavior | Source | Pass |
|---|---|---|---|---|---|
| 1 | Composer in thread | entry | Placeholder `Work with Codex` (idle) or `Working…` (turn running); footer controls as on Home. | app-primary-6f60e9ac4103.js | [ ] |
| 2 | Type text | focus | [INFERENCE] ProseMirror editor grows to a max height, then scrolls. | app-primary | [ ] |
| 3 | Enter | shortcut | Sends (default); `⌘ + Enter` modes per setting `Send shortcut`. | `settings.general.enterBehavior.*` | [ ] |
| 4 | Shift+Enter | shortcut | Inserts a new line. | app-primary | [ ] |
| 5 | `@` | shortcut | Opens file/app mention list. | `composer.contextButton.keyboardEquivalent`, `composer.atMentionList.*` | [ ] |
| 6 | `/` | shortcut | Opens slash command menu. | `composer.slashCommands.*` | [ ] |
| 7 | Paste large text | click | Creates `Pasted text.txt` attachment chip. | `composer.fileAttachment.pastedTextLabel` | [ ] |
| 8 | Drag file over composer | hover | Overlay `Drop to attach`. | `composer.dropOverlay.dropToAttach` | [ ] |
| 9 | Attachment chip remove | click | Removes the attachment. | composer-controls-7c1d04d21325.js | [ ] |
| 10 | Model picker | click | List `Select model`, effort row `Select effort`, Fast toggle. | `composer.modelPicker.*`, `composer.intelligencePicker.*` | [ ] |
| 11 | Permissions dropdown | click | Ask for approval / Approve for me / Full access / Custom (config.toml). | permissions-mode-dropdown-dc16e1c7f7ad.js | [ ] |
| 12 | Submit while running | click | Button shows Stop; with text shows Queue or Steer. | `composer.submitButtonTooltip.*` | [ ] |
| 13 | Stop | click | Sends `turn/interrupt`. | app-shared | [ ] |
| 14 | Queued message row | click | Steer, Edit, Delete, Retry. | queued-message-list-77acfdcb3f8a.js | [ ] |
| 15 | Submit blocked | error | Dialog `Unable to send message` with reason. | `composer.submit.*` | [ ] |
| 16 | Upload in progress | loading | Send disabled with `Files uploading…`. | `composer.submit.waitForFileUploads` | [ ] |
| 17 | Upload failure | error | `Unable to upload file`. | `composer.fileAttachment.uploadError` | [ ] |
| 18 | Button hover | hover | Ghost hover background; tooltip after delay. | app-shared-9a94b907ed47.css | [ ] |
| 19 | Focus ring | focus ring | Send button: `focus-visible:outline-2 outline-background-composer-primary`. | spec class list | [ ] |
| 20 | Resize | resize | Check at 1280, 767 and 600 px window width; below 40rem the editor and footer stack. | `(width < 40rem)` in app-initial | [ ] |

### 14.6 Diff view

| # | Item | Kind | Expected behavior | Source | Pass |
|---|---|---|---|---|---|
| 1 | Open Changes panel | entry | `⌥⌘B` or card `View changes`; tab `Changes` in side panel. | thread-side-panel-tab-content-e9ce64182d53.js | [ ] |
| 2 | `/diff` window | entry | EditorDiffPage renders a single file diff. | editor-diff-page-86fa0f0062f3.js | [ ] |
| 3 | Source selector | click | Uncommitted / Staged / Unstaged / Last turn / Branch / Commits. | `codex.review.source.*`, `stageFilter.*` | [ ] |
| 4 | Show / Hide files | click | Toggles file tree pane. | `codex.review.header.showFiles` | [ ] |
| 5 | File tree filter | focus | `Filter files…` input; `No matching files`. | review-file-tree-pane-e673182fde4d.js | [ ] |
| 6 | File header actions | click | Copy path, Open in editor, Collapse/Expand. | `codex.diff.fileHeader.*` | [ ] |
| 7 | Hunk revert / stage | click | Revert hunk, Stage / Unstage hunk. | `codex.review.revert.hunk.*`, `codex.hunk.*` | [ ] |
| 8 | Revert all | click | Confirm dialog `Revert changes?` with `Don't ask again`. | `codex.review.revertDialog.*` | [ ] |
| 9 | Split / unified toggle | click | `Switch to split diff` / `Switch to unified diff`. | `codex.diffView.switchTo*` | [ ] |
| 10 | Word wrap | click | Toggles wrap. | `codex.review.wrap.*` | [ ] |
| 11 | Mark as viewed | click | File collapses and checkbox state persists. | `codex.review.fileDiff.markAsViewed` | [ ] |
| 12 | `⌘F` in diff | shortcut | `Search diff…` find bar. | `codex.threadFindBar.placeholder.review` | [ ] |
| 13 | `⌘L` | shortcut | Go to line. | `codex.goToLine.*` | [ ] |
| 14 | Line hover | hover | [INFERENCE] line hover background; comment card in diff-comment-card-*.js. | code-diff-38f339f0a601.js | [ ] |
| 15 | No changes | empty | `No file changes yet` / `No unstaged changes`. | `codex.review.noDiff*` | [ ] |
| 16 | Not a git repo | empty | `Create a Git repository` + `Create git repository`. | `codex.review.noDiff.gitRepoRequired.*` | [ ] |
| 17 | Loading | loading | `Loading diff`, `Refreshing changes`. | `codex.review.diff.loading` | [ ] |
| 18 | Load failure | error | `Couldn't load changes` + `Retry`; `Diff failed to render`. | `codex.review.loadFailed.*` | [ ] |
| 19 | Too large | error | `Diff too large to display`. | `codex.review.diffTooLarge.*` | [ ] |
| 20 | Resize | resize | Check at 1280, 767 and 600 px window width; auto mode switches split → inline for narrow panes. | `codex.diffView.autoDescription` | [ ] |

### 14.7 Settings, login and onboarding

| # | Item | Kind | Expected behavior | Source | Pass |
|---|---|---|---|---|---|
| 1 | Open settings | entry | `⌘,`, rail Settings, or profile menu → Settings; route `/settings/general-settings`. | settings-page-50e3fab957b6.js | [ ] |
| 2 | `Back to app` | click | Returns to the previous route. | `settings.nav.back` | [ ] |
| 3 | Nav item | click | Navigates to `/settings/:section`. | `settings.nav.*` | [ ] |
| 4 | Search settings | focus | Filters results; `No results found`. | `settings.search.*` | [ ] |
| 5 | Theme Light/Dark/System | click | [INFERENCE] applies theme at once (root `data-theme`). | `settings.general.appearance.theme.*` general-settings-65e65871cdcd.js | [ ] |
| 6 | Collapse nav | click | `Collapse settings navigation`. | `settings.nav.collapseSidebar` | [ ] |
| 7 | Row hover | hover | Row highlight on nav items. | settings-page | [ ] |
| 8 | Keyboard nav | focus ring | Ring on nav rows and controls. | app-shared CSS | [ ] |
| 9 | Section loading | loading | Skeleton row (`settings-loading-row`). | settings-loading-row-*.js | [ ] |
| 10 | Save failure | error | Toast e.g. `Could not save follow-up behavior`. | `settings.general.*.saveError` | [ ] |
| 11 | Login `/login` | entry | Sign-in screen with ChatGPT, API key, Bedrock options. | login-route-27ff39de87da.js | [ ] |
| 12 | `Continue with ChatGPT` | click | Calls `account/login/start` (type chatgpt) and opens browser; waits for `account/login/completed`. | `electron.onboarding.login.*` | [ ] |
| 13 | `Cancel sign-in` | click | Calls `account/login/cancel`. | `electron.onboarding.login.chatgpt.cancel` | [ ] |
| 14 | `Copy sign-in link` | click | Copies URL; toast `Sign-in link copied`. | `electron.onboarding.login.copyLink` | [ ] |
| 15 | API key form | focus | `sk-…` placeholder; `Continue` enabled when non-empty. | onboarding-login-content-1e8577a45a31.js | [ ] |
| 16 | Login error | error | `Sign-in failed: {rawMessage}`. | `electron.onboarding.login.error` | [ ] |
| 17 | Sign-in requirements loading | loading | `Loading sign-in requirements…`. | `electron.loginMethods.loading` | [ ] |
| 18 | Welcome `/welcome` | entry | Onboarding steps then Home. | onboarding-page-2b170595bf4d.js | [ ] |
| 19 | Select workspace `/select-workspace` | click | Pick folder or skip (`electron-onboarding-skip-workspace`). | select-workspace-page-93a123d78c93.js | [ ] |
| 20 | Resize | resize | Check at 1280, 767 and 600 px window width; [INFERENCE] settings nav collapses on narrow widths. | settings-page | [ ] |

## 15. Scope change: v1 has 9 screens

The integration owner extended v1 to 9 screens: the 6 screens above plus Automations (15.1), Background terminal (15.2) and Mobile setup (15.3).
Section 17 gives, for all 9 screens, the UI path from Home, the fixture state and the exact app-server methods, host endpoints and bridge message types each screen needs.

### 15.1 Screen: Automations ("Scheduled")

Chunks: automations-page-aa8b83be2225.js (`AutomationsLayoutPage`, lazy stub), automations-page-58e351b49373.js (page body), sidebar-a9ef58c0cd4b.js (`ScheduledSidebar`), automation-row-83d4b4688b9c.js, automation-detail-panel-dffc218d9516.js, automation-dialog-d8fddc103f1e.js, automation-frequency-section-92c333d79c0a.js, schedule-fields-6388c2794c61.js, automation-submit-tooltip-7b1f9c8aa18f.js, automation-notification-policy-dropdown-b64aee6fb057.js, automation-team-row-95c319fa231c.js, automation-delete-confirmation-dialog-9610b52e088e.js, automation-side-panel-tab-b008a380ce69.js (thread side panel variant).

- Route: rail `Scheduled` selects destination `builtin:automations`; the current-route check is `/automations` (`Lzt()`); deep links use `/scheduled?automationId=<id>&automationSource=<source>` (`Rzt()`); `/automations?automationMode=create` opens the create dialog.
- Component outline: `ScheduledSidebar` (header `Scheduled`, `New task`, `Search scheduled tasks`, `Filter tasks` with ownership `Personal` / `Team`, sections `Upcoming`, `Inactive`, run list with `Show older`) → main column with title `Scheduled`, subtitle `Ask ChatGPT to schedule tasks, set reminders, or monitor for updates`, `Create` split button, status filter, task rows, detail panel (`Details`, `Project`, `Model`, `Reasoning`, `Runs in`, `Notifications`, `Frequency`, `Repeat`, `On`, `At`), `Previous runs` history.
- Data source: host fetch, not app-server: `list-automations` (returns `{items, habitatMigrationComplete?}`, refetch every 60 s), `inbox-items` (`{limit, threadId}` → `{items, unreadRunCounts}`), `automation-create`, `automation-update`, `automation-delete`, `automation-run-archive`; main pushes `automation-runs-updated` and the renderer then invalidates `list-automations` and `inbox-items`.
- App-server use: `model/list` (model and reasoning pickers), `thread/read` / `thread/list` for run chats, `config/read`; renderer → main `inbox-automation-runs-mark-all-read`, `inbox-item-set-read-state`, `inbox-items-create`, `heartbeat-automation-thread-state-changed`.
- Keyboard: `manageTasks` command (`Manage scheduled tasks`, no default key) opens this screen from the command menu.

| # | Item | Kind | Expected behavior | Source | Pass |
|---|---|---|---|---|---|
| 1 | Rail `Scheduled` | entry | Navigates to the automations page; left sidebar becomes `ScheduledSidebar`. | `sidebarElectron.inboxRouteNavLink`, sidebar-a9ef58c0cd4b.js | [ ] |
| 2 | Deep link `/scheduled?automationId=…&automationSource=…` | entry | Opens the page with the task selected. | app-initial `Rzt()` | [ ] |
| 3 | `New task` (sidebar) | click | Opens the create flow. | `scheduled.sidebar.create` | [ ] |
| 4 | `Create` menu | click | `Create with Codex`, `Create with ChatGPT`, `Set up manually`. | `inbox.automations.create*` | [ ] |
| 5 | Search `Search scheduled tasks` | focus | Filters list; empty `No matching tasks`. | `scheduled.sidebar.searchTasks` | [ ] |
| 6 | Status filter | click | `All` / `Active` / `Paused`. | `inbox.automations.statusFilter.*` | [ ] |
| 7 | Task row | click | Opens the detail panel (Details, Project, Model, Reasoning, Runs in, Notifications, Frequency). | automation-detail-panel-dffc218d9516.js | [ ] |
| 8 | Row actions | click | `Edit`, `Run now`, `Pause` / `Resume`, `Delete` (confirm `Delete {name}?`). | automation-submit-tooltip-7b1f9c8aa18f.js, automation-delete-confirmation-dialog-9610b52e088e.js | [ ] |
| 9 | Editor fields | focus | `Name`, `Prompt`, schedule `Interval/Daily/Weekdays/Weekly/Monthly/Custom`, `Time`, `Days of the week`. | automation-dialog-d8fddc103f1e.js, automation-frequency-section-92c333d79c0a.js, schedule-fields-6388c2794c61.js | [ ] |
| 10 | Previous runs | click | Open a run chat; `Mark all as read`; `Archive all` (confirm). | `inbox.automations.history.*` | [ ] |
| 11 | Row hover | hover | Row highlight; [INFERENCE] action buttons show. | automation-row-83d4b4688b9c.js | [ ] |
| 12 | Focus ring | focus ring | 2 px ring. | app-shared CSS | [ ] |
| 13 | No tasks | empty | `No scheduled tasks`; suggestions `Daily brief`, `Weekly review`, `Follow-up monitor`; detail `Select a scheduled task to view`. | `scheduled.sidebar.noScheduledTasks`, `inbox.automations.emptySuggestion.*` | [ ] |
| 14 | Loading | loading | `Loading scheduled tasks…`, `Loading runs…`, `Loading scheduled task details`. | `scheduled.sidebar.loading` | [ ] |
| 15 | Error | error | `Scheduled tasks could not be loaded`, `Couldn’t load runs`, `Could not save scheduled task`, `Could not delete scheduled task`. | `scheduled.sidebar.loadError`, `inbox.automations.*Error` | [ ] |
| 16 | Resize | resize | Check at 1280, 767 and 600 px window width; list and detail panel share the main column. | automations-page-58e351b49373.js | [ ] |

### 15.2 Screen: Background terminal

Chunks: local-conversation-background-terminal-tab-f87ad9d5555c.js (`LocalConversationBackgroundTerminalTab`), local-conversation-background-terminal-signals-5c82aa902f38.js (running-command selector), open-local-conversation-background-terminal-tab-12ec570f232a.js (`openLocalConversationBackgroundTerminalTab`), background-terminal-3dda7c677015.js (`BackgroundTerminalThumbnail`), terminal-panel-c0ca9a862516.js (xterm renderer), xterm-display-helpers-*.js, local-conversation-thread-c8c5e92ea89e.js (entry list and stop action).

- It is a side panel tab of the thread view, not a route; title `Background terminal` (`codex.localConversation.backgroundTerminalTab.title`).
- A command counts as a background terminal when its `commandExecution` item has `status: "inProgress"` and is not the active foreground item; the selector reads `id`, `command`, `cwd`, `processId` from the item.
- Output comes from `item/commandExecution/outputDelta` (`threadId`, `turnId`, `itemId`, `delta`) and `item/commandExecution/terminalInteraction` (`stdin`); `item/completed` ends it.
- `Stop all background terminals` sends `thread/backgroundTerminals/clean` (`threadId`); failure toast `Unable to stop background terminals`.
- Main → renderer `close-terminal-session` closes terminal sessions of a conversation.
- The interactive `Terminal` tab (`toggleTerminal`, terminal-tab-1947951ef320.js, `cloudTerminal.*` states) is a separate panel and is not part of this screen.

| # | Item | Kind | Expected behavior | Source | Pass |
|---|---|---|---|---|---|
| 1 | Thread with a running background command | entry | Thread lists the running command as `Background terminal` entry. | `codex.localConversation.backgroundTerminals.defaultLabel`, local-conversation-thread-c8c5e92ea89e.js | [ ] |
| 2 | Click the entry | click | Opens side panel tab `Background terminal` (`LocalConversationBackgroundTerminalTab`). | local-conversation-background-terminal-tab-f87ad9d5555c.js | [ ] |
| 3 | Output streaming | loading | xterm view appends `item/commandExecution/outputDelta`; before output `No output yet`. | `codex.localConversation.backgroundTerminalTab.noOutput`, terminal-panel-c0ca9a862516.js | [ ] |
| 4 | `Stop all background terminals` | click | Calls `thread/backgroundTerminals/clean` with `threadId`. | `codex.localConversation.backgroundTerminals.stop` | [ ] |
| 5 | Stop failure | error | Toast `Unable to stop background terminals`. | `codex.localConversation.backgroundTerminals.cleanError` | [ ] |
| 6 | Tab close | click | Closes the tab (`Close {title} tab`); `⌘W` closes the active tab. | `codex.tabs.closeNamed`, command `closeTab` | [ ] |
| 7 | Tab thumbnail | hover | `BackgroundTerminalThumbnail` preview in tab switcher. | background-terminal-3dda7c677015.js | [ ] |
| 8 | Command finishes | empty | `item/completed` removes it from the running list; the tab keeps the buffer. | local-conversation-background-terminal-signals-5c82aa902f38.js | [ ] |
| 9 | Focus ring | focus ring | Tab and buttons: 2 px ring. | app-shared CSS | [ ] |
| 10 | Resize | resize | Check at 1280, 767 and 600 px window width; xterm refits columns on panel resize. | xterm-display-helpers-*.js | [ ] |

### 15.3 Screen: Mobile setup

Chunks: codex-mobile-page-aa98000e29a7.js (`CodexMobilePage`), codex-mobile-setup-flow-964d39beb2ad.js, codex-mobile-setup-dialog-b2025d279701.js (95 KB, all steps), remote-connections-settings-5fcec56d1e17.js (`Manage connections` target).

- Route: `/codex-mobile` (router in app-initial); entry from the profile menu `Help` submenu (`sidebarHelp.submenu`) items `Set up mobile` (`sidebarHelp.setUpMobile`) or `Set up remote` (`sidebarHelp.setUpRemote`).
- Steps: initial (`Connect your phone to this Mac`, features `Pick up where you left off`, `Start something new`, `Stay in the loop`, buttons `Get started`, `Later`), allow host (`Allow devices to control this computer?`, `Allow`), MFA required, waiting/pairing (QR code, pairing code, device type), connected (`You’re connected`, toggles, `Done`).
- App-server: `remoteControl/status/read`, `remoteControl/enable` / `remoteControl/disable` (params `null`), `remoteControl/pairing/start` (`manualCode`), `remoteControl/client/list` (`environmentId`, `cursor`, `limit`, `order`), `remoteControl/client/revoke` (`environmentId`, `clientId`), notification `remoteControl/status/changed`; `account/read` for the MFA check.
- Host fetch: `set-local-remote-control-enabled`, `refresh-remote-control-connections`, `authorize-remote-control-connections`; renderer → main `power-save-blocker-set` for `Keep this Mac awake`.

| # | Item | Kind | Expected behavior | Source | Pass |
|---|---|---|---|---|---|
| 1 | Profile menu → `Help` → `Set up mobile` | entry | Navigates to `/codex-mobile` (`CodexMobilePage`). | `sidebarHelp.setUpMobile`, profile-dropdown-items-d842ad78cd70.js | [ ] |
| 2 | Initial step | click | `Get started` / `Later`; heading `Connect your phone to this Mac`. | `codexMobile.setupDialog.initial.*` | [ ] |
| 3 | Allow host step | click | `Allow` enables remote control (`remoteControl/enable`, host `set-local-remote-control-enabled`). | `codexMobile.setupDialog.allowHost.*`, codex-mobile-setup-flow-964d39beb2ad.js | [ ] |
| 4 | MFA required | error | `Turn on Multi-Factor Authentication` + `Continue on chatgpt.com`. | `codexMobile.setupDialog.mfaRequired.*` | [ ] |
| 5 | Waiting / pairing step | loading | QR code, `Loading pairing code`, `Copy pairing code`, `Refresh pairing code`, `Show QR code fullscreen`, device type iPhone / Android. | `codexMobile.setupPage.waiting.*` | [ ] |
| 6 | Pairing unavailable | error | `Pairing code unavailable: {error}`; `Couldn’t enable remote control. Try again`. | `codexMobile.setupPage.waiting.pairing.unavailable*` | [ ] |
| 7 | Connected step | click | Toggles `Keep this Mac awake`, `Enable computer use`, `Set up Chrome extension`, `Use your Mac apps while locked`; `Done`; `Manage connections`. | `codexMobile.setupDialog.connected.*` | [ ] |
| 8 | Button hover | hover | Ghost/solid hover per 2.3. | app-shared CSS | [ ] |
| 9 | Focus ring | focus ring | 2 px ring. | app-shared CSS | [ ] |
| 10 | Resize | resize | Check at 1280, 767 and 600 px window width; dialog content stays centered. | codex-mobile-setup-dialog-b2025d279701.js | [ ] |

## 16. Host fetch endpoints (`vscode://codex/*`)

- Besides JSON-RPC, the renderer calls main-process handlers through a fetch-like bridge.
- Request: `sendMessageFromView({type: "fetch", requestId, method: "POST", url: "vscode://codex/<endpoint>", headers, body: JSON string, responsePriority})`; cancel with `{type: "cancel-fetch", requestId}` (app-shared `sendRequest`).
- Response: for-view `{type: "fetch-response", requestId, responseType: "success" | "error", status, body | error}` (main-CqEoQDOU.js).
- Handler map: `handlers={…}` in `.vite/build/main-CqEoQDOU.js` (offset about 1,399,000), 128 endpoints; list also in `/tmp/cgx/e1-out/host-fetch-endpoints.txt`.

| Endpoint (`vscode://codex/<name>`) | Renderer chunks |
|---|---|
| `workspace-root-options` | annotation-comment-editor-card-2a63d60663c7.js, app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, panel-fb651e5a5a08.js |
| `electron-clone-workspace-repo` | — (no literal reference) |
| `worktree-create-managed` | app-initial-6350d04672d0.js |
| `worktree-delete` | app-initial-6350d04672d0.js, worktrees-settings-page-ad5cbb37d745.js |
| `worktree-set-owner-thread` | app-initial-6350d04672d0.js |
| `codex-home` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `projectless-thread-cwd` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, panel-fb651e5a5a08.js |
| `projectless-workspace-root` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `home-directory` | app-shared-8bd9d0399038.js |
| `auto-deny-some-permissions` | pending-request-item-panel-770855267df2.js |
| `download-internal-plugins` | app-initial-6350d04672d0.js |
| `app-connect-oauth-callback-url` | app-shared-8bd9d0399038.js |
| `fast-mode-rollout-metrics` | app-primary-6f60e9ac4103.js |
| `refresh-remote-connections` | remote-connections-settings-5fcec56d1e17.js |
| `discover-remote-ssh-connections` | remote-connections-settings-5fcec56d1e17.js |
| `refresh-remote-control-connections` | remote-connections-settings-5fcec56d1e17.js |
| `rename-remote-control-environment` | remote-connections-settings-5fcec56d1e17.js |
| `delete-remote-control-environment` | remote-connections-settings-5fcec56d1e17.js |
| `authorize-remote-control-connections` | remote-connections-settings-5fcec56d1e17.js |
| `set-remote-control-connections-enabled` | app-initial-6350d04672d0.js |
| `set-remote-wsl-connections-enabled` | app-initial-6350d04672d0.js |
| `set-local-remote-control-enabled` | codex-mobile-setup-flow-964d39beb2ad.js |
| `save-codex-managed-remote-ssh-connections` | remote-connection-editor-modal-94d8639e9683.js, remote-connections-settings-5fcec56d1e17.js |
| `set-remote-connection-auto-connect` | remote-connections-settings-5fcec56d1e17.js |
| `install-remote-codex` | — (no literal reference) |
| `start-remote-chatgpt-login-port-forward` | app-shared-8bd9d0399038.js |
| `stop-remote-chatgpt-login-port-forward` | app-shared-8bd9d0399038.js |
| `app-server-connection-state` | app-initial-6350d04672d0.js |
| `remote-workspace-directory-entries` | app-initial-6350d04672d0.js, create-project-dialog-1b5add5efff1.js |
| `workspace-directory-entries` | app-initial-6350d04672d0.js, workspace-directory-entries-query-70873766e7c6.js |
| `has-custom-cli-executable` | composer-utility-bar-7738f1184086.js |
| `account-info` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, cloud-project-unavailable-banner-f12357e7e38e.js, login-route-27ff39de87da.js |
| `email-domain-mail-provider` | onboarding-mail-provider-b16a75ffdd1c.js, panel-fb651e5a5a08.js |
| `third-party-notices` | open-source-licenses-page-aa8c2e98f58d.js |
| `locale-info` | app-initial-6350d04672d0.js |
| `os-info` | app-shared-8bd9d0399038.js |
| `wsl-bash-availability` | app-initial-6350d04672d0.js, general-settings-65e65871cdcd.js |
| `browser-browsing-data-clear` | browser-use-origin-state-queries-5b0748745b24.js |
| `browser-use-origin-state-read` | browser-use-origin-state-queries-5b0748745b24.js |
| `browser-use-approval-mode-write` | browser-use-origin-state-queries-5b0748745b24.js |
| `browser-use-history-approval-mode-write` | browser-use-origin-state-queries-5b0748745b24.js |
| `browser-use-file-transfer-approval-mode-write` | browser-use-origin-state-queries-5b0748745b24.js |
| `browser-use-full-cdp-access-enabled-write` | browser-use-origin-state-queries-5b0748745b24.js |
| `node-repl-active-execs-kill` | app-shared-8bd9d0399038.js |
| `is-packaged` | app-shared-8bd9d0399038.js, panel-fb651e5a5a08.js |
| `global-dictation-hotkey-state` | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js, keyboard-shortcuts-settings-bae6922f5c0c.js, voice-settings-7f1819748d32.js |
| `global-dictation-set-hotkey` | app-primary-6f60e9ac4103.js, voice-settings-7f1819748d32.js |
| `global-dictation-set-toggle-hotkey` | voice-settings-7f1819748d32.js |
| `codex-command-keymap-state` | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js, general-settings-65e65871cdcd.js, keyboard-shortcuts-settings-bae6922f5c0c.js |
| `set-codex-command-keybinding` | keyboard-shortcuts-settings-bae6922f5c0c.js, pets-settings-route-d66810db325d.js, voice-settings-7f1819748d32.js |
| `reset-codex-command-keybindings` | keyboard-shortcuts-settings-bae6922f5c0c.js |
| `feedback-desktop-log-archive` | app-initial-6350d04672d0.js |
| `delete-feedback-desktop-log-archive` | app-initial-6350d04672d0.js |
| `openai-api-key` | — (no literal reference) |
| `recommended-skills` | skill-card-15b3e007becd.js, use-skills-page-data-0b2ad312fd4c.js |
| `local-custom-agents` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `install-recommended-skill` | pets-settings-route-d66810db325d.js, use-skills-page-data-0b2ad312fd4c.js |
| `remove-skill` | app-initial-6350d04672d0.js, skill-card-15b3e007becd.js |
| `read-file` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, configuration-panel-5d5682552c7c.js, local-conversation-turn-632904a8790c.js |
| `read-file-metadata` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, artifact-tab-content.electron-68a541ab97f2.js, local-conversation-turn-632904a8790c.js |
| `write-file` | text-file-storage-2de491513fbc.js |
| `ensure-directory` | app-shared-8bd9d0399038.js |
| `read-file-binary` | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js, app-shared-8bd9d0399038.js, artifact-tab-content.electron-68a541ab97f2.js |
| `upload-local-file-over-sftp` | app-shared-8bd9d0399038.js |
| `compile-latex-artifact` | app-shared-8bd9d0399038.js, artifact-tab-content.electron-68a541ab97f2.js |
| `read-git-file-binary` | use-binary-preview-source-cf245fdbbb64.js |
| `codex-agents-md` | app-initial-6350d04672d0.js, personalization-settings-9b139bd250bc.js, project-import-prompt-host-13e8db734f83.js |
| `codex-agents-md-save` | personalization-settings-9b139bd250bc.js |
| `generate-commit-message` | git-action-new-branch-field-279c01ccef88.js |
| `generate-commit-pull-request-message` | git-action-new-branch-field-279c01ccef88.js |
| `generate-pull-request-message` | git-action-new-branch-field-279c01ccef88.js |
| `inbox-items` | app-initial-6350d04672d0.js, automations-page-58e351b49373.js, data-controls-c9dc29e9444b.js, local-conversation-thread-c8c5e92ea89e.js |
| `list-automations` | actions-b3b1b77c618b.js, app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, automation-side-panel-tab-b008a380ce69.js |
| `ambient-suggestions` | first-app-connect-ambient-suggestions-1f7444f42d68.js, home-ambient-suggestions-content-ba4330f6633f.js, new-chat-suggestion-source-debug-state-e0b9a4575f0a.js, onboarding-page-2b170595bf4d.js |
| `ambient-suggestions-refresh` | first-app-connect-ambient-suggestions-1f7444f42d68.js, home-ambient-suggestions-content-ba4330f6633f.js, new-chat-suggestion-source-debug-state-e0b9a4575f0a.js, onboarding-page-2b170595bf4d.js |
| `ambient-suggestion-set-status` | home-ambient-suggestions-content-ba4330f6633f.js |
| `developer-instructions` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `automation-run-archive` | app-shared-8bd9d0399038.js |
| `automation-run-delete` | — (no literal reference) |
| `delete-archived-thread` | — (no literal reference) |
| `delete-all-archived-threads` | — (no literal reference) |
| `list-pinned-threads` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js |
| `set-thread-pinned` | app-initial-6350d04672d0.js |
| `set-pinned-threads-order` | app-initial-6350d04672d0.js, execution-fde3a4937b6a.js |
| `automation-create` | app-initial-6350d04672d0.js, automation-side-panel-tab-b008a380ce69.js, automations-page-58e351b49373.js, pull-request-fix-automation-1473b35ad8d0.js |
| `automation-update` | actions-b3b1b77c618b.js, agent-activity-item-40fa48e24fdc.js, app-shared-8bd9d0399038.js, automation-side-panel-tab-b008a380ce69.js |
| `automation-delete` | actions-b3b1b77c618b.js, automation-side-panel-tab-b008a380ce69.js, automations-page-58e351b49373.js, local-conversation-heartbeat-automation-eb50da46f57b.js |
| `automation-run-now` | — (no literal reference) |
| `find-files` | — (no literal reference) |
| `paths-exist` | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js, review-file-tree-pane-e673182fde4d.js, select-workspace-page-93a123d78c93.js |
| `claude-code-import-status` | — (no literal reference) |
| `external-agent-imported-connectors` | app-initial-6350d04672d0.js |
| `local-environment-config` | app-initial-6350d04672d0.js, local-conversation-thread-c8c5e92ea89e.js, local-environments-settings-content-399f96d641e1.js |
| `local-environment-config-save` | configuration-panel-5d5682552c7c.js |
| `is-copilot-api-available` | app-shared-8bd9d0399038.js |
| `get-copilot-api-proxy-info` | app-shared-8bd9d0399038.js |
| `get-global-state` | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js, app-shared-8bd9d0399038.js |
| `set-global-state` | app-shared-8bd9d0399038.js |
| `queued-follow-up-send-lock-acquire` | app-shared-8bd9d0399038.js |
| `queued-follow-up-send-lock-release` | app-shared-8bd9d0399038.js |
| `get-configuration` | — (no literal reference) |
| `set-configuration` | — (no literal reference) |
| `get-setting` | app-shared-8bd9d0399038.js |
| `get-settings` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, execution-fde3a4937b6a.js, register-app-actions-4b8b92c62fa1.js |
| `set-setting` | app-shared-8bd9d0399038.js |
| `settings-read` | execution-fde3a4937b6a.js |
| `settings-write` | execution-fde3a4937b6a.js |
| `apply-patch` | app-shared-8bd9d0399038.js, remote-conversation-page-d5c77768f912.js, sites-end-resource-78b98c374310.js, thread-side-panel-tab-content-e9ce64182d53.js |
| `mcp-codex-config` | app-shared-8bd9d0399038.js |
| `worktree-shell-environment-config` | app-shared-8bd9d0399038.js |
| `pick-file` | — (no literal reference) |
| `pick-files` | app-initial-6350d04672d0.js |
| `save-file` | app-initial-6350d04672d0.js |
| `add-context-file` | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js, base-file-tree-651d700763e3.js |
| `git-origins` | app-initial-6350d04672d0.js, app-primary-6f60e9ac4103.js, app-shared-8bd9d0399038.js, use-create-git-repository-68b40fad1718.js |
| `git-push` | app-shared-8bd9d0399038.js, git-action-new-branch-field-279c01ccef88.js |
| `git-create-branch` | app-initial-6350d04672d0.js, app-shared-8bd9d0399038.js, git-action-new-branch-field-279c01ccef88.js, git-branch-switcher-4b041e029b15.js |
| `git-checkout-branch` | app-shared-8bd9d0399038.js, git-action-new-branch-field-279c01ccef88.js, git-branch-switcher-4b041e029b15.js |
| `git-merge-base` | app-shared-8bd9d0399038.js, code-review-3b4934e8f7f0.js |
| `prepare-worktree-snapshot` | app-primary-6f60e9ac4103.js, app-shared-8bd9d0399038.js |
| `upload-worktree-snapshot` | app-primary-6f60e9ac4103.js, app-shared-8bd9d0399038.js |
| `set-vs-context` | — (no literal reference) |
| `native-desktop-apps` | app-initial-6350d04672d0.js, computer-use-native-app-metadata-db64f29da0ce.js |
| `native-desktop-app-by-bundle-id` | computer-use-native-app-metadata-db64f29da0ce.js, user-message-attachments-178fc2e1502f.js |
| `computer-use-frontmost-window` | app-initial-6350d04672d0.js |
| `computer-use-start-capture` | app-initial-6350d04672d0.js |
| `native-desktop-app-icon` | — (no literal reference) |
| `computer-use-native-desktop-app-icon` | app-initial-6350d04672d0.js |

## 17. Per-screen render contract (9 screens)

Common to every screen (app shell boot):
- `sendSync`: `codex_desktop:get-sentry-init-options`, `codex_desktop:get-build-flavor`, `codex_desktop:get-shared-object-snapshot`, `codex_desktop:get-system-theme-variant`, `codex_desktop:get-initial-sidebar-bootstrap`.
- `sendMessageFromView` types: `ready`, `view-focused`, `persisted-atom-sync-request`, `persisted-atom-update`, `shared-object-subscribe`, `shared-object-set`, `keyboard-layout-map-changed`, `app-shell-shortcut-state-changed`, `electron-desktop-features-changed`, `electron-sparkle-gates-changed`, `log-message`, `fetch`, `cancel-fetch`, `mcp-request`.
- For-view types the shell needs: `codex-app-server-initialized`, `codex-app-server-connection-changed`, `codex-app-server-fatal-error`, `mcp-response`, `mcp-notification`, `mcp-request`, `mcp-request-delivery`, `fetch-response`, `persisted-atom-sync`, `persisted-atom-updated`, `shared-object-updated`, `global-state-updated`, `electron-window-focus-changed`, `window-fullscreen-changed`, `run-command`, `navigate-to-route`, `toggle-sidebar`, plus `codex_desktop:system-theme-variant-updated`.
- App-server at boot: `account/read`, `config/read`, `configRequirements/read`, `model/list`, `experimentalFeature/list`, `collaborationMode/list`, `thread/list` (sidebar), `skills/list`, `plugin/list`, `app/list`, `mcpServerStatus/list`.
- Host fetch at boot (referenced from app-initial/app-shared): `get-global-state`, `get-settings`, `account-info`, `codex-home`, `os-info`, `locale-info`, `workspace-root-options`, `list-pinned-threads`, `codex-command-keymap-state`, `app-server-connection-state`, `local-custom-agents`, `developer-instructions`, `inbox-items`, `list-automations`.
- The renderer handles 196 for-view `type` values in one switch in app-initial (label `bb460`): `add-context-file`, `ambient-suggestion-generation`, `appshot-shortcut`, `archive-thread`, `artifact-direct-comment`, `automation-runs-updated`, `avatar-overlay-action`, `avatar-overlay-computer-use-cursor-changed`, `avatar-overlay-explicit-pet-opened`, `avatar-overlay-keyboard-interaction-ready`, `avatar-overlay-layout-changed`, `avatar-overlay-open-state-changed`, `browser-sidebar-browser-use-capture-surface`, `browser-sidebar-browser-use-cursor-state`, `browser-sidebar-browser-use-page-released`, `browser-sidebar-browser-use-state`, `browser-sidebar-browser-use-viewport`, `browser-sidebar-clear-pending-panel-open`, `browser-sidebar-command`, `browser-sidebar-comment-overlay-insert-text`, `browser-sidebar-comment-overlay-session`, `browser-sidebar-context-menu-action`, `browser-sidebar-destroy-webview`, `browser-sidebar-direct-comment`, `browser-sidebar-find-state`, `browser-sidebar-floating-composer-reveal`, `browser-sidebar-image-attachment`, `browser-sidebar-image-drag-state`, `browser-sidebar-open-panel-without-animation`, `browser-sidebar-page-loaded`, `browser-sidebar-runtime-annotation-permission-result`, `browser-sidebar-runtime-annotation-selection-hover-state`, `browser-sidebar-runtime-annotation-selection-modifier-state`, `browser-sidebar-runtime-clear-comment-screenshot`, `browser-sidebar-runtime-close-editor`, `browser-sidebar-runtime-create-comment-at-point`, `browser-sidebar-runtime-open-design-editor-at-point`, `browser-sidebar-runtime-prepare-comment-screenshot`, `browser-sidebar-runtime-remove-annotation-selection`, `browser-sidebar-runtime-restore-editor`, `browser-sidebar-runtime-select-comment`, `browser-sidebar-runtime-sync`, `browser-sidebar-screenshot-copied`, `browser-sidebar-screenshot-copy-failed`, `browser-sidebar-state`, `browser-sidebar-tab-capture-state`, `browser-sidebar-tab-lifecycle`, `browser-sidebar-tab-render-process-gone`, `browser-sidebar-tab-state`, `browser-sidebar-url-copied`, `browser-sidebar-usage`, `browser-sidebar-web-contents-pointer-down`, `browser-sidebar-webview-attached`, `bundled-plugin-install-state-observed`, `business-checkout-complete`, `capture-screenshot-selection`, `chat-font-settings`, `chat-search-command-menu`, `chatgpt-auth-token-unavailable`, `clear-active-terminal`, `close-active-app-shell-tab`, `close-browser-tab`, `close-terminal-session`, `codex-app-server-connection-changed`, `codex-app-server-fatal-error`, `codex-app-server-initialized`, `codex-micro-device-lifecycle`, `codex-micro-device-state-changed`, `codex-micro-hid-event`, `codex-micro-insert-composer-text`, `codex-micro-insert-skill-mention`, `codex-micro-joystick-event`, `codex-micro-push-to-talk-start`, `codex-micro-push-to-talk-stop`, `codex-micro-window-ownership-changed`, `codex-ui-render-process-gone`, `command-menu`, `computer-use-analytics-event`, `computer-use-capture-updated`, `connector-oauth-callback`, `copy-conversation-path`, `copy-deeplink`, `copy-working-directory`, `credit-purchase-checkout-complete`, `custom-prompts-updated`, `debug-run-eval-control-request`, `debug-run-eval-control-response`, `debug-window-client-analytics-history-changed`, `debug-window-origin-conversation-changed`, `desktop-app-entry-received`, `dynamic-tool-call-requested`, `dynamic-tools-for-thread-start-requested`, `electron-app-state-snapshot-request`, `electron-onboarding-skip-workspace-result`, `electron-window-focus-changed`, `electron-window-opaque-surface-changed`, `electron-window-position-changed`, `ephemeral-generation-token-usage`, `external-browser-open-finished`, `external-browser-open-started`, `fetch-response`, `file-search-command-menu`, `find-in-files`, `find-in-thread`, `find-next-in-thread`, `find-previous-in-thread`, `focus-composer`, `global-dictation-idle`, `global-dictation-in-app-start`, `global-dictation-in-app-stop`, `global-dictation-paste-failed`, `global-dictation-start`, `global-dictation-start-confirmed`, `global-dictation-stop`, `global-state-updated`, `heartbeat-automation-thread-state-request`, `hotkey-window-home-composer-position-changed`, `hotkey-window-transition`, `image-preview-zoom-command`, `implement-todo`, `invalidate-thread-search`, `is-copilot-api-available-updated`, `keyboard-modifier-state-changed`, `mcp-event-notification`, `mcp-request`, `mcp-request-delivery`, `mcp-resource-updated`, `mcp-response`, `navigate-back`, `navigate-forward`, `navigate-to-route`, `new-chat`, `new-chat-with-appshot`, `new-projectless-task`, `notch-overlay-collapse`, `notch-overlay-expand`, `notch-overlay-keyboard-interaction-ready`, `notch-overlay-layout-changed`, `notch-overlay-toggle`, `open-agent-onboarding`, `open-browser-tab`, `open-create-remote-project-modal`, `open-file-changed`, `open-pet-install-modal`, `pdf-direct-comment`, `persisted-atom-sync`, `persisted-atom-updated`, `personal-checkout-complete`, `personal-workspace-access-blocked`, `pinned-threads-updated`, `primary-runtime-install-progress`, `prompt-link-destination`, `rate-limit-reset-checkout-complete`, `refetch-review-file-source`, `remote-hosted-pip-browser-frame-state-changed`, `remote-hosted-pip-content-layout-state-changed`, `remote-hosted-pip-task-state-changed`, `remote-workspace-root-requested`, `rename-thread`, `request-user-input-auto-resolution-changed`, `request-user-input-auto-resolution-snapshot`, `reset-window-zoom`, `review-file-source-changed`, `run-command`, `screenshot-selection-shortcut`, `set-primary-runtime-install-release`, `shared-object-updated`, `show-chatgpt-rosalind-announcement`, `step-font-size`, `step-window-zoom`, `tasks-reload-requested`, `thread-project-memberships-updated`, `thread-title-updated`, `toggle-bottom-panel`, `toggle-browser-panel`, `toggle-debug-modal`, `toggle-diff-panel`, `toggle-file-tree-panel`, `toggle-performance-tools`, `toggle-query-devtools`, `toggle-ram-usage-indicator`, `toggle-react-scan`, `toggle-sidebar`, `toggle-terminal`, `toggle-thread-pin`, `trace-recording-state-changed`, `trace-recording-uploaded`, `visual-content-action-clicked`, `window-fullscreen-changed`, `worker-event`, `worker-response`, `workspace-root-option-add-canceled`, `workspace-root-option-added`, `workspace-root-option-picked`, `workspace-root-options-updated`, `worktrees-reload-requested`.

| # | Screen | UI path from Home | Fixture state | App-server (requests / notifications) | Host fetch | `sendMessageFromView` types | For-view types |
|---|---|---|---|---|---|---|---|
| 1 | Home | Launch app (route `/`) | Signed in (`account/read` returns ChatGPT auth), 0 projects, 0 threads, one model in `model/list` (`GPT-6 Astra`, effort `low`), permissions profile `Custom` from config | `thread/start`, `turn/start` (on submit), `model/list`, `permissionProfile/list`, `config/read`, `account/rateLimits/read` / `thread/started` | `workspace-root-options`, `ambient-suggestions`, `fast-mode-rollout-metrics`, `has-custom-cli-executable`, `paths-exist` | `electron-pick-workspace-root-option`, `electron-add-new-workspace-root-option` (project menu) | `focus-composer`, `new-chat`, `workspace-root-option-picked`, `workspace-root-option-added`, `workspace-root-options-updated` |
| 2 | Sidebar | Visible on Home; Search: click `Search` or `⌘K`; Activity: click bell | 3 projects, 10 threads (2 pinned, 1 unread, 1 running), one archived | `thread/list`, `thread/search`, `thread/archive`, `thread/unarchive`, `thread/delete`, `thread/name/set`, `thread/metadata/update` / `thread/started`, `thread/status/changed`, `thread/name/updated`, `thread/archived`, `thread/deleted` | `list-pinned-threads`, `set-thread-pinned`, `set-pinned-threads-order`, `workspace-root-options` | `open-in-new-window`, `local-thread-activity-changed`, `tray-menu-threads-changed`, `electron-set-badge-count` | `chat-search-command-menu`, `command-menu`, `pinned-threads-updated`, `thread-title-updated`, `invalidate-thread-search`, `toggle-sidebar`, `archive-thread`, `rename-thread`, `toggle-thread-pin` |
| 3 | Thread view | Click a thread row in `Recents` (route `/local/:conversationId`) | Thread with 3 completed turns: user message, reasoning, `commandExecution` (exit 0), `fileChange` (2 files), agent message; one pending command approval for the approval card | `thread/resume`, `thread/turns/list`, `thread/items/list`, `turn/interrupt`, `thread/fork`, `thread/rollback` / `turn/started`, `item/started`, `item/agentMessage/delta`, `item/reasoning/summaryTextDelta`, `item/commandExecution/outputDelta`, `item/completed`, `turn/completed`, `turn/diff/updated`, `thread/tokenUsage/updated`; server requests `item/commandExecution/requestApproval`, `item/fileChange/requestApproval` | `read-file`, `read-file-metadata`, `auto-deny-some-permissions`, `local-environment-config` | `open-in-browser`, `open-in-new-window`, `update-diff-if-open` | `find-in-thread`, `find-next-in-thread`, `find-previous-in-thread`, `focus-composer`, `toggle-diff-panel`, `toggle-bottom-panel`, `toggle-terminal`, `copy-working-directory`, `copy-deeplink`, `copy-conversation-path` |
| 4 | Composer | Home composer, or thread composer after step 3; open each menu by click | Same as 1 and 3; two models with efforts `low`/`medium`/`high`; one file in project for `@` search; one queued follow-up | `turn/start`, `turn/steer`, `thread/queue/add`, `thread/queue/list`, `thread/queue/delete`, `thread/settings/update`, `model/list`, `permissionProfile/list`, `fuzzyFileSearch/sessionStart`, `fuzzyFileSearch/sessionUpdate`, `fuzzyFileSearch/sessionStop`, `thread/attachment/add` / `fuzzyFileSearch/sessionUpdated`, `thread/settings/updated` | `add-context-file`, `pick-files`, `queued-follow-up-send-lock-acquire`, `queued-follow-up-send-lock-release`, `global-dictation-hotkey-state` | `global-dictation-in-app-started` (dictation) | `add-context-file`, `focus-composer`, `global-dictation-in-app-start`, `global-dictation-in-app-stop` |
| 5 | Diff view | Thread view → `⌥⌘B` (Toggle Changes Panel) or card `View changes`; `/diff` window via file `Open` | Thread from 3 in a git repo with 2 modified files and 1 untracked file, 3 hunks | `turn/diff/updated`, `item/fileChange/patchUpdated`; `thread/revert` | `apply-patch`, `read-file`, `read-file-binary`, `read-git-file-binary`, `git-merge-base`, `paths-exist` | `update-diff-if-open`; worker `codex_desktop:worker:git:from-view` (`status-summary`, `review-summary`, `review-diff`, `review-patch`, `branch-diff-stats`, `apply-changes`, `revert`) | `toggle-diff-panel`, `toggle-file-tree-panel`, `review-file-source-changed`, `refetch-review-file-source`, `open-file-changed`, `worker-response`, `worker-event` |
| 6 | Settings + login | `⌘,` or profile avatar → `Settings`; login: signed-out start (route `/login`) | Settings: default config; login: `account/read` returns no account | `config/read`, `config/value/write`, `config/batchWrite`, `account/read`, `account/logout`, `account/login/start`, `account/login/cancel` / `account/login/completed`, `account/updated` | `get-settings`, `set-setting`, `settings-read`, `settings-write`, `codex-command-keymap-state`, `set-codex-command-keybinding`, `reset-codex-command-keybindings`, `account-info` | `show-settings`, `open-config-toml`, `electron-pick-workspace-root-option`, `electron-onboarding-skip-workspace` | `chat-font-settings`, `step-font-size`, `global-state-updated`, `electron-onboarding-skip-workspace-result`, `navigate-to-route` |
| 7 | Automations | Click rail `Scheduled` | 2 local automations (1 active daily at 09:00, 1 paused weekly), 3 past runs (1 unread) | `model/list`, `thread/read`, `config/read` | `list-automations`, `inbox-items`, `automation-create`, `automation-update`, `automation-delete`, `automation-run-archive` | `inbox-automation-runs-mark-all-read`, `inbox-item-set-read-state`, `heartbeat-automation-thread-state-changed` | `automation-runs-updated`, `tasks-reload-requested`, `heartbeat-automation-thread-state-request`, `navigate-to-route` |
| 8 | Background terminal | Thread view (3) with a running background command → click its `Background terminal` entry | Thread with one `commandExecution` item `status: inProgress`, `processId` set, 40 lines of output, plus a running turn | `thread/backgroundTerminals/clean` / `item/started`, `item/commandExecution/outputDelta`, `item/commandExecution/terminalInteraction`, `item/completed` | — | — | `close-terminal-session`, `clear-active-terminal`, `close-active-app-shell-tab` |
| 9 | Mobile setup | Profile avatar → `Help` → `Set up mobile` (route `/codex-mobile`) | Signed in with MFA on; remote control status `disabled`; pairing code available | `remoteControl/status/read`, `remoteControl/enable`, `remoteControl/pairing/start`, `remoteControl/client/list`, `remoteControl/client/revoke` / `remoteControl/status/changed` | `set-local-remote-control-enabled`, `refresh-remote-control-connections`, `authorize-remote-control-connections` | `power-save-blocker-set` | `navigate-to-route` |

- The `[INFERENCE]` rule applies to the fixture column: fixture values are the minimum state that makes every element of the screen render, chosen from the bundle data shapes, not from a captured session.
