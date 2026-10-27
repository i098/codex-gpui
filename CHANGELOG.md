# Changelog

All notable changes to this project are in this file.
The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).
There is no release yet.

## [Unreleased]

### Added
- A contributing guide, issue and pull request templates, a security policy, and this changelog.
- Add Storage usage measurement and safe review before deleting old local chats.
- Add Import settings for Claude Code and Cursor, with saved sync choices and preserved imported workspaces.
- Add the native Security workbench for scans, repository history, finding triage, remediation, and source viewing.
- Add Settings > Connections with saved SSH hosts, SSH discovery, remote-control setup, and `codex://settings/connections/<tab>` deep links.
- Add the searchable General `Language` menu, which saves `desktop.localeOverride` and clears it with `Auto detect`.
- Add the Git settings page controls and the Worktrees settings page with General and Environments tabs, saved worktree preferences, and local environment editing.
- Show the Plugins and Skills directories in the original `Customize` sidebar, with the original header, scope pills, and two-column skill rows.
- Add the Cloud environment UI with repository selection and saved setup flows

### Fixed
- Show the original Steer, Delete, and menu icons on queued messages, and place the queued-message menu where the original opens it.
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