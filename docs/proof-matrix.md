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
- Use the existing Mac browser on port 9467, or set `--cdp` to its Mac-local HTTP endpoint.
  Do not start or restart a shared browser.
- Install no dependencies for this command.
  It uses Python 3, rsync, the existing Mac Node executable, the installed Rust toolchain, and cached Cargo dependencies.
- Choose a new output directory outside the checkout.

The browser must report `HeadlessChrome/155.0.8059.79` and WebKit commit `a49e2e3d4addfd84e765b52afe230bedb81096db`.
Start that shared browser with the flags in `tools/route1/README.md` through its owner, not this command.
The command checks the seven retained entry-script and style hashes before capture.
It uses native Mac fonts with `R1_SF_FONTS` unset.
It preserves the existing route1 and Root capture clocks and fixtures.

The fixture adapter selects the retained `thread/resume` request by thread, rollout path, and history mode.
It reuses the recorded desktop context and returns the unchanged original response through route1.
It rejects an unknown thread instead of opening a real app-server session.

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
It copies the source and original assets there and makes an APFS clone of the handed-off target.
It builds while the original capture runs, then renders Root and applies the gate.
Only its own fixture server and browser context stop at completion.
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


Run the retained-request check with `node tools/parity/fixture-server.mjs --self-test`.