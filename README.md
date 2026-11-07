# codex-gpui

codex-gpui is an open-source port of the Codex desktop app to Rust and [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
The original is the Codex section of the macOS ChatGPT app.
The goal is pixel parity with the original: the same layout, text, colors and icons, device pixel for device pixel.
The app drives the real `codex app-server` over stdio, the same JSON-RPC transport that the original uses.

[Build and run](docs/build-and-run.md) · [Architecture](docs/architecture.md) · [Parity](docs/parity.md) · [Contributing](CONTRIBUTING.md) · [Security](.github/SECURITY.md) · [Changelog](CHANGELOG.md)

## Status

The app runs on macOS (Apple silicon and Intel). The workspace also builds on Linux for CI. Pixel parity is checked by the harness in [Parity and proof](docs/parity.md); the sections there list what is and is not proven.

## Performance

Measured against the original Codex app (26.1029) on the same account-free fixture:

| Metric | Original | codex-gpui |
| --- | --- | --- |
| Cold start to sidebar rows painted (median) | 11 079 ms | 1 378 ms |
| Warm start to sidebar rows painted (median) | 9 295 ms | 204 ms |
| Idle memory, RSS at +30 s (all processes), no thread link | 2 034 MB | 155 MB |
| Idle memory, RSS at +30 s, with thread link | 2 118 MB | 181 MB |
| Idle memory, `phys_footprint` at +90 s, no thread link | 798 MB | 93 MB |
| Idle memory, `phys_footprint` at +90 s, with thread link | 892 MB | 123 MB |
| Idle CPU, window visible, no thread link | 2.2 % of one core | 0.7 % |
| Idle CPU, window visible, with thread link | 31.9 % | 0.7 % |
| Idle CPU, window hidden, no thread link | 0.7 % | 0.2 % |
| Size: original app (`du`) / codex-gpui release binary | 1 918 MiB | 100.5 MiB |
| Scroll a long thread, frame interval p50 / p95 | not measurable | 16.67 / 18.6 ms |

Method: amelia Mac mini (Apple M4), release build, same account-free fixture for both apps. Every row is the median of n=5 runs, interleaved between the apps. The "no thread link" rows launch with no thread open; the "with thread link" rows open a `codex://threads` link.

- The codex-gpui size is the binary only; the app bundle is not assembled yet.
- The original could not open a thread without a signed-in account, so its long-thread memory and scroll were not measured.
- Scroll is the frame interval of a forced 60 Hz redraw, not frame work.

### Reproduce

Hardware and software of the run above: Mac mini M4 (10 cores), macOS 26.6.2, console locked. The original is ChatGPT/Codex 26.1029.61003. codex-gpui is commit `9e6ef934` (release build with `CODEX_GPUI_QA=1`, plus `tools/bench/probe.diff`).

The two states are `plain` (no thread open) and `thread` (the app opens `codex://threads/<id>` of the `explain` fixture). Both apps use the same recorded-fixture app-server (`tools/route1/linux/mock-app-server.mjs`).

```sh
git apply tools/bench/probe.diff
CODEX_GPUI_QA=1 cargo build --locked --release -p app --bin codex-gpui
export BENCH_ORIG=/path/to/ChatGPT.app BENCH_BIN=$PWD/target/release/codex-gpui
tools/bench/run.sh plain    # cold and warm start, idle RSS and CPU, no thread link
tools/bench/run.sh thread   # cold start, idle RSS and CPU, with the thread link
```

`run.sh` needs passwordless `sudo` (for `purge` and `launchctl asuser`), starts the apps in the background, runs 5 interleaved rounds, and prints median (min-max) per metric. `probe.diff` adds the first-frame log and a switch that paints a window the locked console hides. Do not ship it.

## Quick start

1. Install Rust 1.98.1 (`rust-toolchain.toml` pins it, so `rustup` does it on the first build).
2. Make a `codex` binary available: `$CODEX_BIN`, the ChatGPT app's bundled one, or `codex` on `PATH`.
3. Run the app:

   ```sh
   cargo run -p app --release
   ```

Full requirements, flags, environment variables and tests: [Build and run](docs/build-and-run.md).

## Documentation

| Doc | Content |
| --- | --- |
| [Build and run](docs/build-and-run.md) | Requirements, CI notes, build flags, run modes, tests |
| [Features and runtime setup](docs/features.md) | Computer use, SSH, terminals, skills, connectors, ACP, cloud transport, sign-in, onboarding, Security workbench |
| [Updates, releases and live QA](docs/release.md) | Update policy, release bundle, in-app updates, versions, live QA build |
| [Architecture](docs/architecture.md) | Crates, data flow, design |
| [Parity and proof](docs/parity.md) | Parity method, regenerate inputs, live preview and proof, frame benchmark, current parity |
| [Menus and native links](docs/interaction.md) | Permission-menu replay, command-menu actions, deep links |
| [Computer use contract](docs/computer-use-contract.md) | Computer Use integration contract |
| [Flows](docs/flows.md) | Flow parity: original vs port |
| [Inventory](docs/inventory.md) | Screen and flow inventory (data in `inventory.json`) |
| [Proof matrix](docs/proof-matrix.md) | Census proof command |
| [Windows](docs/windows.md) | Native windows |
