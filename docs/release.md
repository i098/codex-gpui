# Updates, releases and live QA

## Managed update policy

General settings shows the read-only `In-app updates` row with `Managed` when the app-server disables `in_app_updates`.
The row uses `experimentalFeature/list`, not a local preference.
The app hides the row when that policy is absent or enabled.

### Legacy Codex Cloud threads

Open an existing task with `cargo run -p app -- /remote/<task-id>`.
The `/hotkey-window/remote/<task-id>` path selects the compact content variant.
The page uses the app-server's ChatGPT identity for the original `/wham/tasks` API.
It reads task turns, streams setup logs and thread events, and supports image previews and follow-up image uploads.
A log download failure does not hide the task's setup error.
Applying changes selects a configured local project with the same Git remote.
Apply and Revert use a temporary Git index, so they do not change the user's staged files.
The result dialog lists applied, conflicted, and skipped paths.
`Root::open_remote_task` and `Root::remote_page` provide the same page to native window hosts.
This route is separate from the durable `codex_cloud` thread service.
Run `cargo test -p app -p ui --lib cloud` and `cargo test -p app --test host` for the protocol and native state checks.
Live-account and strict pixel validation remain pending for this page.

## Release bundle

```sh
tools/bundle.sh
```

The script builds the release binary on the Mac build host through `tools/mac.sh`.
It assembles `Codex GPUI.app` (bundle id `dev.codexgpui.app`, version from `tools/version.sh`, macOS 13.0 or later).
By default, it signs the bundle ad hoc, checks it with `plutil -lint` and `codesign --verify --strict`, and zips it with `ditto`.
The zip goes to `/tmp/cgx/release/codex-gpui-<version>-macos-arm64.zip`.
The script does not open the app.
Bundle assembly requires Python 3.
It generates `THIRD_PARTY_NOTICES.txt` from production Rust dependencies and their available license files, then includes the file in Resources.

### In-app updates

Development bundles have no update feed, so update checks and the update menu stay off.
For a release, set `CODEX_GPUI_UPDATE_FEED`, `CODEX_GPUI_UPDATE_PUBLIC_KEY`, and `CODEX_GPUI_SPARKLE_FRAMEWORK` before running `tools/bundle.sh`.
The framework path refers to the Mac build host and must contain the verified Sparkle 2.9 framework.
Set `CODEX_GPUI_SIGN_IDENTITY` to the release signing identity; without it the bundle is signed ad hoc and not notarized.
Sparkle checks the Ed25519 signature of the zip, so an ad-hoc bundle still updates.
The script embeds the feed and Ed25519 public key, then signs Sparkle's helpers, framework, and app with that identity.
The release workflow does this when the repository secrets `SPARKLE_ED_PRIVATE_KEY` and `SPARKLE_ED_PUBLIC_KEY` exist.
It then runs `tools/appcast.py` and uploads `appcast.xml` and `<zip>.sha256` beside the zip; run `python3 tools/test_appcast.py` for its checks.
General settings has an `Automatically check for updates` switch that sets Sparkle's `automaticallyChecksForUpdates`.
The release owner must supply the production feed and complete signing and notarization before distribution.

For manual local installation, run `tools/install-mac.sh` on the Mac through `tools/mac.sh -l install "nice -n 19 bash tools/install-mac.sh $(git rev-parse HEAD)"`.
It builds the normal bundle, replaces `~/Applications/Codex GPUI.app`, and keeps the old bundle as `~/Applications/.Codex GPUI.previous.app`.
It never opens, activates, or closes the app; a running copy changes at its next launch.

Sparkle checks the signed appcast and finds a newer release.
The rail shows its update banner while the embedded installer downloads and installs the release in the background.
The script downloads the release archive and checksum, verifies the checksum and app signature, keeps the previous bundle, and opens the new app without taking focus.
The app reads the `in_app_updates` policy through `experimentalFeature/list`.
A disabled policy removes the update action and shows the read-only `Managed` row in General settings.
Sparkle dialogs show check progress, no-update results, and update errors.
Run `cargo test -p app --lib updater` on macOS for the feed and managed-policy checks.
Run `cargo test -p app --test updater -- --test-threads=1` for native rail visibility and input checks.

The app icon is the original icon from `ChatGPT.app/Contents/Resources/Assets.car`.
To make `assets/app/AppIcon.icns` again, run this on the Mac:

```sh
swiftc -O assets/app/icon.swift -o /tmp/icon
/tmp/icon Assets.car /tmp/AppIcon.iconset
iconutil -c icns /tmp/AppIcon.iconset -o assets/app/AppIcon.icns
```

### Versions and releases

The workspace `version` in the root `Cargo.toml` is the app version.
Every crate under `crates/` except `skia-hairline-sys` inherits it.
`tools/version.sh` prints it.
`tools/version.sh 0.2.0` bumps it in `Cargo.toml` and `Cargo.lock` and adds a `## [0.2.0]` section under `## [Unreleased]` in `CHANGELOG.md`.
Merge the bump, then push the tag `v0.2.0`.
The `release` workflow builds the arm64 bundle with `tools/bundle.sh` on the self-hosted runner (labels `self-hosted,macOS,ARM64,macmini-arm64`), signs it ad hoc unless the Developer ID and notary secrets exist, and stops if the tag is not `v` plus the workspace version.
It publishes `codex-gpui-0.2.0-macos-arm64.zip` and `codex-gpui-0.2.0-macos-arm64.dmg` as a GitHub release with that version's changelog section as notes.
About Codex shows the version.
`Codex` > `Check for Updates…` uses the signed update feed when the bundle has one and organization policy permits updates.

## Live QA build

QA uses `~/Applications/Codex GPUI QA.app` with bundle id `dev.codexgpui.qa`.
It rebuilds merged `origin/main`; it does not hot-reload a running Rust process.
Build a staged QA bundle from Linux with `tools/bundle.sh --qa`.
That command does not install or open it.

Run this once on the Mac from a clean checkout of the latest merged main:

```sh
python3 tools/qa-sync.py install
```

The owned launchd job `dev.codexgpui.qa-sync` checks main every 30 seconds.
Its script, source cache, build logs, artifact records, and agent plist stay in `~/Library/Application Support/codex-gpui-qa/updater/`.
Temporary builds and isolated startup probes stay under `/tmp`.
The updater fetches real Git objects, rejects dirty or unlanded source, and checks main again before activation.
It coalesces newer revisions during a build and retains a pending candidate when QA cannot close safely.
It verifies the signed bundle, compiled source revision, artifact digest, executable path, and running PID.
It keeps the previous bundle at `updater/last-working.app`.
It rebuilds missing temporary candidates and removes completed or superseded temporary artifacts.
It requests a cooperative close before removing a superseded idle probe.
It keeps the pending ownership record when a staged process cannot close safely.
It verifies an unfinished activation or restores the backup after the new process exits.

QA data stays in `~/Library/Application Support/codex-gpui-qa/profile/`.
Its UI state and host settings live under `profile/home/Library/Application Support/codex-gpui/`.
Its app-server uses `profile/codex` as `CODEX_HOME` and `profile/workspace` as the default project.
The app-server receives a clean environment and uses file-only credential storage.
QA starts signed out; it never copies normal credentials, settings, history, or workspace data.
It can read the existing ChatGPT app-server executable without changing that app.
The normal Codex GPUI profile and launch behavior stay unchanged.

Programmatic startup uses `open -g -j` and keeps QA hidden.
Open QA from its installed bundle or Dock icon when you want its window.
If the installed revision already matches main but its process has stopped, the updater starts the installed bundle again.
After QA gains focus, starts a turn, changes model settings, or starts sign-in, close QA when ready to update.
The updater keeps its candidate pending until that close.
It also refuses to close during a draft, attachment work, an approval, a running turn, or unverified readiness.
Native foreground activity also prevents closing QA, even when its last heartbeat or window reports inactive.
An observed application activation keeps the existing manual-close protection.
A shared profile lock prevents a reopened QA process from racing with bundle replacement.
It does not promise to preserve drafts, open views, navigation history, or running sessions.
It never force-quits an app.
If a new process cannot prove readiness, the updater retains the backup rather than forcing that process to stop.

Read the current source, artifact, and runtime identities with:

```sh
python3 "$HOME/Library/Application Support/codex-gpui-qa/updater/qa-sync.py" status
```

Run the updater regression checks with `python3 -B tools/test_qa_sync.py`.

Stop only the owned timer with `launchctl bootout "gui/$(id -u)/dev.codexgpui.qa-sync"`.
That command does not close QA or any normal app.


