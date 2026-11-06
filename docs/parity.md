# Parity and proof

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

The `parity-mac` workflow runs the same gate on the Mac mini runner (`macmini-arm64`) for a PR head: dispatch it with the PR number or add the `parity-gate` label. It uploads the report as an artifact and prints the `Parity gate:` numbers in its summary. It is not a required check.

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
| Hairline scanner package | `tools/shapes/build-skia-hairline.py` | Builds `skia-hairline-sys` against the installed Skia archive twice and writes the versioned package and receipt. |

The labs `tools/shapes/run.sh`, `tools/shapes/blur.sh` and `tools/text/measure.sh` compare shapes, blur and text with Chromium case by case.

## Live preview

```sh
scripts/live-preview.sh 18500
```

The script watches `crates`, `assets`, `fixtures` and the Cargo files. On a change it runs `cargo build -p ui` for the `shot`, `usage` and `settings_kit` examples under `flock /tmp/cargo-slot.lock`. Then it renders Home, Usage and Settings headless and serves `$LIVE_OUT` (default `/sdc-disk/tmp/cgx/live-preview`) on `127.0.0.1`.
The page polls `status.json` every second. It shows the build state and the last good build time, swaps only the images that changed, and shows the compiler output instead of the images when the build fails.
The Cargo target is `/sdc-disk/tmp/cgx/cargo-targets/<checkout path with / as _>`; set `CARGO_TARGET_DIR` to change it.

The host runs it as the `systemd --user` unit `codex-gpui-live-preview` (`ExecStart` is this script in the checkout), published on the tailnet:

```sh
systemctl --user enable --now codex-gpui-live-preview
sudo -n tailscale serve --bg --https=18500 http://127.0.0.1:18500
```

The link is `https://agents.tailc4c9b.ts.net:18500`. The unit watches the checkout that its `ExecStart` names, so point it at a checkout that stays.

## Live proof

```sh
tools/mac.sh 'cargo run -p app --example live_turn'
```

The example runs the real root view, headless, on a real `codex app-server`.
With `CODEX_GPUI_ACP` set, it runs on that ACP agent instead and also checks the ACP toast.
It types a prompt into the Home composer with key events and pushes Enter.
It waits for `turn/completed` and checks that the thread view shows `pong`.
It prints the transcript and saves `live-1-home.png`, `live-2-streaming.png` and `live-3-done.png`.
`$LIVE_ROOT` (default `/tmp/cgx-mac`) holds the project, the Codex home and the screenshots.

Thread model, reasoning effort, and permission changes use `thread/settings/update`.
Reviewer changes also use `turn/settings/update` during an active turn.
The composer applies `thread/settings/updated` without changing the draft.
Older servers keep the next-turn settings path.
Run `cargo test -p app --test live_thread_settings` to check these requests and the fallback.

## Frame benchmark

```sh
tools/mac.sh 'cargo run -p app --release --example bench'
```

The example runs the real root view, headless at 1280x832 @2x, on a replayed app-server with 60 Recents and a 40-turn chat.
It sends key, mouse and scroll-wheel events and prints, for each scenario, the p50, p95 and max milliseconds from an input to the end of its frame.
The animation scenarios (sidebar and changes-panel toggles in a chat) also count every frame of the animation, at simulated 120 Hz ticks.
The `fps @120Hz` column gives the frames per second that a 120 Hz display shows when each frame takes its CPU time.
The bench also prints the time from launch to the first ready frame.
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


