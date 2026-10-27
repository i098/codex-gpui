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

### Fixed

- Apply the shared feature and account access checks to activity entry points, and use the configured activity shortcut.
- Keep the diff scroll position while typing a Find query that already has a match on screen, and when expanding context, instead of re-centring the first match.
- Preserve titled-thread identity in the titlebar and derive compositor tiles from painted bounds.
- Replay Skia's pinned scanner operations for legacy hairline icon strokes through a new versioned `skia-hairline-sys` package.
- Preserve page-frame occlusion under opaque scrolling content at rounded surface corners.
- Draw the Code Review open-request status dot as the original sub-pixel SVG circle.
- Group the diff find bar in one composited layer and paint its icons under the bar's rounded overflow clip, as the original does.
- Clip the diff line-number gutters to the scroller so scrolled-out rows no longer paint over the panel header.
