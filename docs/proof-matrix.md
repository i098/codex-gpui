# Census proof command

Run one original/native comparison without changing references, masks, or thresholds.
The command renders both frames on the Mac and calls the existing `parity::diff_ref` gate.
It does not open a native app window or change an account.

```sh
python3 tools/parity/proof.py home rest \
  --theme dark --size 1280x832 \
  --warm-target /tmp/cgx-mac/target/content-typing \
  --out /sdc-disk/tmp/cgx/my-proof/home-dark

python3 tools/parity/proof.py terminal rest \
  --theme dark --size 1280x832 \
  --warm-target /tmp/cgx-mac/target/content-typing \
  --out /sdc-disk/tmp/cgx/my-proof/terminal-dark

python3 tools/parity/proof.py settings configuration \
  --theme dark --size 1280x832 \
  --warm-target /tmp/cgx-mac/target/content-typing \
  --out /sdc-disk/tmp/cgx/my-proof/configuration-dark
```

## Requirements

- Obtain a handoff for the warm Mac target before use.
  The path above is an example, not a cache grant.
- Keep SSH access through the `mac` alias, or set `--mac`.
- Keep the pinned original webview at `/tmp/cgx/asar-x/webview`, or set `--webview` to its Linux path.
- Keep the validated shared Mac headless shell available on port 9467.
  The command uses a fresh isolated context through an owned SSH tunnel.
  It never starts, stops, or restarts that browser.
- Install no dependencies for this command.
  It uses Python 3, Node, rsync, the installed Mac Rust toolchain, and cached Cargo dependencies.
- Choose a new output directory outside the checkout.

The browser must report `HeadlessChrome/155.0.8059.79` and WebKit commit `a49e2e3d4addfd84e765b52afe230bedb81096db`.
The shared shell must use the pinned flags from `tools/route1/README.md`.
The command checks the seven retained entry-script and style hashes before capture.
It uses native Mac fonts with `R1_SF_FONTS` unset.
It preserves the existing route1 and Root capture clocks and fixtures.

## State adapters

| Surface | State | Original path | Native path |
|---|---|---|---|
| `home` | `rest` | Existing route1 Home capture | `common::Fx` startup |
| `terminal` | `rest` | Existing route1 recorded command replay | The same recorded events and input sequence as `flows::open_terminal` |
| `settings` | `configuration` | Profile menu, Settings, Configuration | Settings shortcut, Configuration navigation item |

These are the initial executable adapters, not all 175 census surfaces.
An unknown surface or state fails before a capture starts.
Add matching original and Root state adapters before running another census cell.
Do not substitute a static screen or a prior native screenshot for a Root frame.

Both themes support the normal census sizes, `1280x832` and `1440x900`.
Home rest also accepts `767x832` and `600x832`.
Terminal and Configuration reject narrow sizes until their navigation adapters support them.
Each frame uses the same CSS size and native device scale 2.
The narrow sizes have no separate waiver for a failing gate.

## Results and failure behavior

Each output directory contains:

- `original.png`, `native.png`, and `diff.png`.
- `differences.md`, with the gate result and every connected difference region.
- `metrics.json`, from the unchanged parity gate.
- `provenance.json`, the browser identity and pinned original assets.
- `source-head.txt`, the checked-out commit before the run.
- The fresh original `refs/` image and DOM text specification.
- The build, fixture server, original capture, and native capture logs.

Region coordinates use device pixels.
Each region joins differing pixels that share an edge or corner.
Yellow pixels are inside the existing text mask; magenta pixels are outside it.
The report counts both classes per region and does not guess a source-code cause.

The gate permits zero differing pixels outside text and at most 0.5% inside text.
A failing gate returns nonzero after it writes the images and report.
A capture or build failure also returns nonzero; inspect the logs if no report exists.
Never treat missing evidence as a passing cell.

The command creates a private Mac directory under `/tmp/cgx-proof.*`.
It copies the native source there and makes an APFS clone of the handed-off target.
The fixture server and CDP client run in owned Linux scratch while the browser renders on the Mac.
The native build runs at the same time as the original capture.
The command transfers the fresh original and its text specification, then captures Root and applies the gate on the Mac.
Only its own fixture server, browser context, and SSH tunnel stop at completion.
The Linux caller retrieves the proof and removes the private Mac directory.
If retrieval fails, the command retains the Mac directory and prints its path.
Retrieve that evidence before removing the directory.
The command does not modify the handed-off target, tracked references, or normal profiles.

## Report regression check

```sh
CARGO_TARGET_DIR=<handed-off-Linux-target> \
  cargo test -p app --test proof_matrix regions_keep_diagonal_text_and_geometry_counts
```

This test checks difference-region bounds and text/geometry counts.
It does not claim Linux rendering parity.
The ignored `capture_cell` test runs only through the Mac command above.

## Recorded smoke result

The Mac run at native source `28b50bc` produced all three images and a region report for each cell.
All cells used the dark theme and `1280x832` CSS size.

| Cell | Outside text pixels | Inside text difference | Gate |
|---|---:|---:|---|
| Home rest | 0 | 0.376963% | PASS |
| Terminal rest | 3078 | 0.850551% | FAIL |
| Settings Configuration | 971375 | 49.345921% | FAIL |

The failed cells demonstrate the nonzero gate result; they do not grant native appearance approval.
The Configuration native frame shows its heading without the original controls.
The tool preserves those differences rather than changing the reference or text mask.