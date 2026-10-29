# Changelog

All notable changes to this project are in this file.
The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).
Each release has a section below; `tools/version.sh MAJOR.MINOR.PATCH` adds it.

## [Unreleased]

### Added
- Add the advanced Appearance settings: theme presets, colors, contrast, fonts, translucent sidebar, theme import and copy, separate light and dark modes, font sizes, diff markers, reduced motion, pointer cursors, font smoothing, the macOS Dock icon, and Reset to default.
- Add optional keyboard-accessible documentation links to confirmation dialogs without accepting or cancelling the pending action.
- Add saved, revision-scoped Mark as viewed controls for Branch diffs.
- Route native Codex links with thread host and access state, project selection, SSH setup, source-specific task editing, and window-owned OAuth callbacks.
- Notify completed automation runs and show the Scheduled unread dot from saved run state.
- Add working Notifications settings: system permission, turn mode, alert switches, sound with import and preview, and ChatGPT channels.
- Show the crash page with Try again when the app-server cannot start, and keep unsent composer drafts when Try again restarts it.
- Show scheduled previous runs with saved read state, mark-all-as-read, and confirmed bulk archive with partial-failure reporting.
- Add the native `File` > `New Project…` form with a name, marker, ordered source folders, and a device picker.
- A contributing guide, issue and pull request templates, a security policy, and this changelog.
- Add one workspace version, a version bump script, and a tag release workflow that publishes the macOS arm64 zip.
- Add the native `/first-run` tour with the IDE, cloud, code, and autonomy pages and the original noise background.
- Add native Code Review search, filters, and pull request details.
- Add Storage usage measurement and safe review before deleting old local chats.
- Add Import settings for Claude Code and Cursor, with saved sync choices and preserved imported workspaces.
- Add the native Security workbench for scans, repository history, finding triage, remediation, and source viewing.
- Add Settings > Connections with saved SSH hosts, SSH discovery, remote-control setup, and `codex://settings/connections/<tab>` deep links.
- Add the searchable General `Language` menu, which saves `desktop.localeOverride` and clears it with `Auto detect`.
- Add the Git settings page controls and the Worktrees settings page with General and Environments tabs, saved worktree preferences, and local environment editing.
- Show the Plugins and Skills directories in the original `Customize` sidebar, with the original header, scope pills, and two-column skill rows.
- Add the Cloud environment UI with repository selection and saved setup flows
- Bind `⇧⌘C` and `⌥⇧⌘C` to copy the open chat's newest managed worktree root, else its working directory, and route `⇧⌘F` and `⇧⌘B` through the editable keymap.
- Add signed native updates with download progress, installation, relaunch, and organization policy controls.
- Restore `View` > `Toggle Bottom Panel` and `View` > `Open Terminal` in the macOS menu bar, in the original order.

### Fixed
- Keep context-only commands out of the live Keyboard shortcuts dialog while retaining them in Settings.
- Show Login when the account read fails, and keep Coding Home available when access-inputs fails.
- Share Mark as unread state between the chat command, its shortcut, and the sidebar menu.
- Open Home on launch, as the original does, instead of the last saved page; window size, sidebar, theme and an unfinished onboarding page stay.
- Show saved hourly and minute intervals in scheduled task rows without changing daily or weekly labels.
- Raster a shown Code Review pull request in the page's tile grid, as the original does, so its status glyph and the main panel's corners match the original, and paint the author avatar's white fill as its box background.
- Paint the Code Review tabs, search toolbar and pull request list in the original's composited layers, so their icon and pill edges match the original pixel for pixel.
- Place the Git review file tree's `+N`/`-N` line counts on the original's SVG text baseline.
- Paint a short Plugins or Hooks settings page without a composited layer, as the original does, so the empty Hooks page matches its text, card corners and disabled reload button.
- Let the Git review header's capsule shadows reach the large-diff notice area, as in the original.
- Show `Enter full view` and the tab-count `Show tabs` in the titlebar when a narrow window hides the side panel, and match the original's code span pills and scroll limits.
- Show a screenshot or pasted image in the composer as the original's attachment card: squircle clip, hover ring, hover remove button, and Chromium's image filtering.
- Match the original Home logo, zoom banner and titlebar buttons at 90% and 110% zoom.
- Draw the bottom terminal panel's tab close glyph once, and snap a selected tab at a fractional position to the device grid as the original does.
- Paint the bottom terminal panel's tab strip, `Open new terminal` and `Reload` pills, and the workspace warning tint as the original rasters them.
- Paint the Code Review detail section in its own opaque composited layer, as the original does, so the `Summary` and `Chat` pill edges match.
- Show the original Steer, Delete, and menu icons on queued messages, and place the queued-message menu where the original opens it.
- Match the original Keyboard shortcuts dialog list, search field, keycaps and close icon, and the original Keyboard settings row, menu check and search text offsets.
- Use one local environment parser for Settings and worktree setup, reject stale saves without losing external edits, and keep a disabled environment selection.
- Use each window's current account for dictation and stop stalled transcription requests after 60 seconds.
- Reuse shared bootstrap data for Security startup without extra policy requests or navigation catalog reads.
- Apply the shared feature and account access checks to activity entry points, and use the configured activity shortcut.
- Keep the diff scroll position while typing a Find query that already has a match on screen, and when expanding context, instead of re-centring the first match.
- Preserve titled-thread identity in the titlebar and derive compositor tiles from painted bounds.
- Replay Skia's pinned scanner operations for legacy hairline icon strokes through a new versioned `skia-hairline-sys` package.
- Preserve page-frame occlusion under opaque scrolling content at rounded surface corners.
- Draw the Code Review open-request status dot as the original sub-pixel SVG circle.
- Group the diff find bar in one composited layer and paint its icons under the bar's rounded overflow clip, as the original does.
- Clip the diff line-number gutters to the scroller so scrolled-out rows no longer paint over the panel header.
- Measure text x-height at the requested optical size on macOS so CoreText metrics match the rendered font size