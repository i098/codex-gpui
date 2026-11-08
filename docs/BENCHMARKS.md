# Benchmarks

## Reproduce

Hardware and software of the run above: Mac mini M4 (10 cores), macOS 26.6.2, console locked. The original is ChatGPT/Codex 26.1029.61003 for start, memory and CPU; the app on that host now reports 26.1103.41825, the version of the size rows. codex-gpui is commit `9e6ef934` (release build with `CODEX_GPUI_QA=1`, plus `tools/bench/probe.diff`).

The two states are `plain` (no thread open) and `thread` (the app opens `codex://threads/<id>` of the `explain` fixture). Both apps use the same recorded-fixture app-server (`tools/route1/linux/mock-app-server.mjs`).

```sh
git apply tools/bench/probe.diff
CODEX_GPUI_QA=1 cargo build --locked --release -p app --bin codex-gpui
export BENCH_ORIG=/path/to/ChatGPT.app BENCH_BIN=$PWD/target/release/codex-gpui
tools/bench/run.sh plain    # cold and warm start, idle RSS and CPU, no thread link
tools/bench/run.sh thread   # cold start, idle RSS and CPU, with the thread link
```

Size: `tools/bench/size.sh /path/to/ChatGPT.app /path/to/Codex\ GPUI.app` prints `du`, the file-size sum and the `ditto -c -k --keepParent` zip size. Its raw output on amelia, for the v0.1.1 `.dmg` app and the original, is in `tools/bench/results/size-amelia.txt`. The codex-gpui `.app` is arm64 only, built with `CODEX_GPUI_DMG=1 bash tools/bundle.sh --assemble /tmp/cgx-bsize/release normal "$(git rev-parse HEAD)" archive`, ad-hoc signed (commit `ebc3fd8a`, v0.1.1 tree). The `.dmg` is 60.8 MB. A universal build is not measured: the x86_64 build fails in `skia-hairline-sys`.

`run.sh` needs passwordless `sudo` (for `purge` and `launchctl asuser`), starts the apps in the background, runs 5 interleaved rounds, and prints median (min-max) per metric. `probe.diff` adds the first-frame log and a switch that paints a window the locked console hides. Do not ship it.

## Method and notes

An earlier table is replaced by this 5-run median check: that one mixed the thread-link state into the memory and CPU rows, and its "about 4.1 s" original start was arithmetic, not a measurement, so it is gone.

Method: amelia Mac mini (Apple M4), release build, same account-free fixture for both apps. Every row is the median of n=5 runs, interleaved between the apps. The "no thread link" rows launch with no thread open; the "with thread link" rows open a `codex://threads` link.

- The original could not open a thread without a signed-in account, so its long-thread memory was not measured.
