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

## Current parity

Each cell shows the different pixels outside the text boxes, then the different-pixel ratio inside the text boxes.
A case passes at `0 / ≤0.500%`.
Now 0 of 36 cases pass.

| Screen | Theme | 1280x832 | 1440x900 |
|---|---|---|---|
| home | dark | 304 / 3.133% | 294 / 3.133% |
| home | light | 521 / 1.460% | 517 / 1.460% |
| sidebar | dark | 602 / 1.885% | 639 / 1.849% |
| sidebar | light | 631 / 0.938% | 642 / 0.921% |
| thread | dark | 1616 / 4.779% | 1532 / 4.724% |
| thread | light | 26040 / 2.182% | 25931 / 2.157% |
| composer | dark | 905 / 3.439% | 877 / 3.439% |
| composer | light | 69769 / 11.495% | 69952 / 11.495% |
| diff | dark | 4058 / 3.924% | 22520 / 7.015% |
| diff | light | 19432 / 1.771% | 52886 / 5.059% |
| settings | dark | 4591 / 1.661% | 4926 / 1.691% |
| settings | light | 7112 / 0.308% | 7825 / 0.315% |
| automations | dark | 3705 / 1.981% | 3706 / 1.981% |
| automations | light | 3826 / 1.688% | 3824 / 1.688% |
| terminal | dark | 6107 / 4.098% | 6457 / 5.991% |
| terminal | light | 39138 / 1.559% | 42617 / 3.987% |
| mobile | dark | 2770 / 9.062% | 2731 / 9.062% |
| mobile | light | 6059 / 9.114% | 6021 / 9.114% |
