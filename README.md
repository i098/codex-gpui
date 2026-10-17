# codex-gpui

codex-gpui is an open-source port of the Codex desktop app to Rust and [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
The original is the Codex section of the macOS ChatGPT app.
The goal is pixel parity with the original: the same layout, text, colors and icons, device pixel for device pixel.
The app drives the real `codex app-server` over stdio, the same JSON-RPC transport that the original uses.

## Requirements

- macOS on Apple silicon (arm64).
- Rust 1.98.1. `rust-toolchain.toml` pins it, so `rustup` installs it on the first build.
- A `codex` binary. The app uses the first one that it finds:
  1. `$CODEX_BIN`.
  2. `/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex`, the binary that the ChatGPT app bundles.
  3. `codex` on `PATH`.

The workspace also builds on Linux, with GPUI's no-op text system, for CI (fmt, clippy, tests).

## Build and run

```sh
cargo run -p app --release
```

The app keeps its window state in `~/Library/Application Support/codex-gpui/state.json`.
Host settings use the local `settings.write(key, value)` service and persist beside that file in `host-settings.json`.
Theme and window-close preferences remain in `state.json`.
The S034 replay checks the menu-bar switch, its saved values, and Back navigation against `refs/interactions/s034.json`.

## Release bundle

```sh
tools/bundle.sh
```

The script builds the release binary on the Mac build host through `tools/mac.sh`.
It assembles `Codex GPUI.app` (bundle id `dev.codexgpui.app`, version from `crates/app/Cargo.toml`, macOS 13.0 or later).
It signs the bundle ad hoc, checks it with `plutil -lint` and `codesign --verify --strict`, and zips it with `ditto`.
The zip goes to `/tmp/cgx/release/codex-gpui-<version>-macos-arm64.zip`.
The script does not open the app.

The app icon is the original icon from `ChatGPT.app/Contents/Resources/Assets.car`.
To make `assets/app/AppIcon.icns` again, run this on the Mac:

```sh
swiftc -O assets/app/icon.swift -o /tmp/icon
/tmp/icon Assets.car /tmp/AppIcon.iconset
iconutil -c icns /tmp/AppIcon.iconset -o assets/app/AppIcon.icns
```

## Architecture

| Crate | Role |
|---|---|
| `bridge` | Client for `codex app-server`: newline-delimited JSON-RPC over stdio, hand-written protocol types, replay of recorded sessions. |
| `ui` | The window shell (titlebar, navigation rail, sidebar, main panel), the 9 screens in `screens/`, the composer, the thread view, the diff view and the icons. |
| `shapes` | Box decorations that land on Chromium's pixels: rounded and squircle corners, borders, `box-shadow`, backdrop blur, SVG icon paths. It links Skia. |
| `textsys` | A GPUI text system that puts glyphs on the same pixels as Chromium on macOS. It also holds the vendored `gpui` and `gpui_apple`. |
| `theme` | Color tokens, metrics and the bundled fonts, extracted from the original renderer CSS. |
| `parity` | The pixel-parity harness: it renders each screen headless and compares it with the reference screenshot. |
| `app` | The `codex-gpui` binary: the root view, the menu bar and the app-server session. |

On macOS, `textsys::show_definition(window, selection, baseline)` opens AppKit's native Look Up popover.
The baseline uses window-local logical pixels from the top left; the adapter handles both view orientations.
Call it on the main thread with the unchanged selection.
Headless windows return their native-handle error; the adapter has no URL or clipboard fallback.

`gpui` and `gpui_apple` come from Zed at a fixed revision and are vendored in `crates/textsys`.
The workspace `[patch]` replaces the upstream crates with these copies.
Stock GPUI cannot match Chromium's pixels, so the copies change three things:

- Blink snapping: layout in Blink `LayoutUnit`s (1/64 device px), device-pixel rounding half up, and text at fractional x positions.
- Skia blends: glyph and sprite blending as Skia's 8-bit raster does it, and `SkPaint::setAlphaf` alpha rounded to a byte.
- Layer compositing: opacity layers and composited layers (`will-change`, scrollers) that blend as Chromium's `saveLayerAlpha` does.

`shapes` links Skia binaries built with Chromium's `SkUserConfig.h` legacy defines.
`.cargo/config.toml` points `SKIA_BINARIES_URL` to these binaries.

## Parity method

The references come from the original renderer itself.
`tools/route1` serves the extracted renderer with a stub Electron main process.
`chrome-headless-shell` on the Mac renders it at 2x, with the Mac's fonts.
The captures are in `refs/<screen>/<theme>@<W>x<H>.png`, with a `spec.json` that holds the DOM boxes, styles and text boxes.
There are 36 references: 9 screens, dark and light, 1280x832 and 1440x900.

The harness renders the same states with GPUI on the Mac and compares exact RGBA values:

```sh
tools/mac.sh 'cargo run -p parity -- [--screen home] [--gate]'
```

- All pixels outside the text boxes must be equal.
- Inside the text boxes (inflated 1 device px), at most 0.5% of the pixels can be different.
- `--gate` exits with code 1 if one case fails.
- The report is in `target/parity/` (`report.html`, `report.json`, and the ref, port and diff PNGs).

`tools/parity/publish.sh <out_dir>` uploads a report to the public R2 evidence bucket.
It prints a Markdown table for a PR body.

## Regenerate inputs

| Input | Command | Notes |
|---|---|---|
| References | `node tools/route1/capture-all.mjs [screen...]` | Start the renderer, the tunnels and the server first, as `tools/route1/README.md` tells. |
| Fixtures | `node tools/fixtures/record.mjs [step...]` | Records real app-server sessions on the Mac; see `fixtures/README.md`. |
| Theme tokens | `uv run --with playwright python tools/extract/theme.py` | Writes `crates/theme/src/tokens.rs`. |
| Fonts | `uv run --with fonttools --with brotli python tools/extract/fonts.py` | Converts the bundled woff2 files to TTF. |
| Icon SVGs | `python3 tools/extract/icons.py` | Writes `assets/icons/<name>.svg` from the references. |
| Icon images | `uv run -p /usr/bin/python3 --with numpy --with pillow tools/icons/build.py` | Fits the per-pixel icon images in `assets/icons/px/` and `tools/icons/check.json`. |
| Skia binaries | `tools/shapes/skia.sh` | Builds Skia with Chromium's defines and uploads it for `SKIA_BINARIES_URL`. |

The labs `tools/shapes/run.sh`, `tools/shapes/blur.sh` and `tools/text/measure.sh` compare shapes, blur and text with Chromium case by case.

## Live proof

```sh
tools/mac.sh 'cargo run -p app --example live_turn'
```

The example runs the real root view, headless, on a real `codex app-server`.
It types a prompt into the Home composer with key events and pushes Enter.
It waits for `turn/completed` and checks that the thread view shows `pong`.
It prints the transcript and saves `live-1-home.png`, `live-2-streaming.png` and `live-3-done.png`.

## Frame benchmark

```sh
tools/mac.sh 'cargo run -p app --release --example bench'
```

The example runs the real root view, headless at 1280x832 @2x, on a replayed app-server with 60 Recents and a 40-turn chat.
It sends key, mouse and scroll-wheel events and prints, for each scenario, the p50, p95 and max milliseconds from an input to the end of its frame.
The 120 Hz frame budget is 8.3 ms.
With `BENCH_SHOT=<dir>`, it saves every 20th frame, so that you can compare the pixels of two builds.

## Current parity

Each cell shows the different pixels outside the text boxes, then the different-pixel ratio inside the text boxes.
A case passes at `0 / ≤0.500%`.
Now 36 of 36 cases pass.

| Screen | Theme | 1280x832 | 1440x900 |
|---|---|---|---|
| home | dark | 0 / 0.377% | 0 / 0.377% |
| home | light | 0 / 0.098% | 0 / 0.098% |
| sidebar | dark | 0 / 0.320% | 0 / 0.316% |
| sidebar | light | 0 / 0.153% | 0 / 0.152% |
| thread | dark | 0 / 0.443% | 0 / 0.438% |
| thread | light | 0 / 0.444% | 0 / 0.439% |
| composer | dark | 0 / 0.332% | 0 / 0.332% |
| composer | light | 0 / 0.250% | 0 / 0.250% |
| diff | dark | 0 / 0.360% | 0 / 0.467% |
| diff | light | 0 / 0.281% | 0 / 0.388% |
| settings | dark | 0 / 0.088% | 0 / 0.080% |
| settings | light | 0 / 0.117% | 0 / 0.107% |
| automations | dark | 0 / 0.279% | 0 / 0.279% |
| automations | light | 0 / 0.148% | 0 / 0.148% |
| terminal | dark | 0 / 0.179% | 0 / 0.261% |
| terminal | light | 0 / 0.171% | 0 / 0.249% |
| mobile | dark | 0 / 0.468% | 0 / 0.468% |
| mobile | light | 0 / 0.467% | 0 / 0.467% |

## Permission-menu interaction replay

`refs/interactions/f2b.json` records 16 steps from the original app through route1 on Linux.
The Mac replay checks hover focus, Home/End, typeahead with a one-second reset, Tab containment, and keyboard opening.
It compares focus, highlight pixels, and outgoing requests.
The existing port menu has different dimensions, so the replay maps hover targets to the same item and heading.
Submenu arrows and model-trigger recording remain outside this replay.

```sh
tools/mac.sh -l f2b 'cargo test -p app --test f2b -- --test-threads=1'
```
