# Contributing to codex-gpui

codex-gpui ports the Codex desktop app to Rust and GPUI.
Every change keeps the goal in the [README](README.md): the same layout, text, colors, icons and behavior as the original app.

## Set up

Read [Requirements](docs/build-and-run.md#requirements) first.
The native app, the native tests and the parity harness need macOS on Apple silicon.
The workspace also builds on Linux, which is enough for the CI checks.
On Ubuntu, CI installs these packages first:

```sh
sudo apt-get install -y --no-install-recommends \
  build-essential pkg-config clang cmake libssl-dev libzstd-dev \
  libfontconfig-dev libfreetype-dev libasound2-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libxcb1-dev \
  libvulkan-dev
```

`rust-toolchain.toml` pins the Rust version, so `rustup` installs it on the first build.

`tools/mac.sh` runs a command on a Mac from a Linux checkout.
It needs an SSH host named `mac`; on a Mac, run the command directly.

## Checks

CI runs these three commands on every pull request.
Run them before you open one:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`lefthook.yml` runs `cargo fmt --check` before every push. Run `lefthook install` once to enable it.

The docs name a focused test for most features; run the tests for the area that you change.
Many native tests run only on macOS.

A change to rendered pixels also needs the parity harness on macOS (see [Parity method](docs/parity.md#parity-method)):

```sh
cargo run -p parity -- --gate
```

The references in `refs/` come from the original renderer.
Do not edit them by hand; regenerate them as [Regenerate inputs](docs/parity.md#regenerate-inputs) tells.

## Commits

Commit subjects use [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/): `type(scope): subject`.
Use `feat`, `fix`, `test`, `docs`, `refactor`, `ci` or `chore`; the scope is optional.
Write the subject in the imperative, with no period at the end.
Use the body to tell why the change is necessary.

```text
fix(diff): preserve decoded Changes file paths
```

## Issues

Use the [bug report](.github/ISSUE_TEMPLATE/bug_report.yml) or [feature request](.github/ISSUE_TEMPLATE/feature_request.yml) form.
Write a plain sentence-case title, without a commit prefix such as `fix:`.
For a visual difference, attach screenshots of the original app and of codex-gpui in the same state and at the same window size.
Report a vulnerability in private, as the [security policy](.github/SECURITY.md) tells; never in an issue.

## Pull requests

- Open an issue first, then one pull request for one concern.
- Use the commit format for the title.
- Start the body with `Closes #N` on its own line, then fill each section of the [pull request template](.github/PULL_REQUEST_TEMPLATE.md).
- Under Test evidence, name the commands that you ran and their results; do not claim checks that you did not run.
- Leave an inapplicable checklist item unchecked and tell why beside it.
- Add a user-visible change to [CHANGELOG.md](CHANGELOG.md) (see [Changelog](#changelog)).

## Changelog

[CHANGELOG.md](CHANGELOG.md) follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).
Add one line for a user-visible change under `## [Unreleased]`, in the section that fits: `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed` or `Security`.
A change with nothing for a user to note, such as a test-only change, needs no line.
