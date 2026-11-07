# codex-gpui

codex-gpui is an open-source port of the Codex desktop app to Rust and [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
The original is the Codex section of the macOS ChatGPT app.
The goal is pixel parity with the original: the same layout, text, colors and icons, device pixel for device pixel.
The app drives the real `codex app-server` over stdio, the same JSON-RPC transport that the original uses.

[Build and run](docs/build-and-run.md) · [Architecture](docs/architecture.md) · [Parity](docs/parity.md) · [Contributing](CONTRIBUTING.md) · [Security](.github/SECURITY.md) · [Changelog](CHANGELOG.md)

## Status

The app runs on macOS (Apple silicon and Intel). The workspace also builds on Linux for CI. Pixel parity is checked by the harness in [Parity and proof](docs/parity.md); the sections there list what is and is not proven.

## Performance

Measured against the original Codex app (26.1029; 26.1103 for size) on the same account-free fixture:

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
| App size, `.app` (`du -sh`) | 1.9 GB (1 964 200 KB) | 107 MB (109 204 KB), arm64 |
| Download size (`ditto` zip) | 815 MB | 56.2 MB |
| Download size, macOS `.dmg` | not measured | 60.8 MB |

How to reproduce: [docs/BENCHMARKS.md](docs/BENCHMARKS.md)

An earlier table is replaced by this 5-run median check: that one mixed the thread-link state into the memory and CPU rows, and its "about 4.1 s" original start was arithmetic, not a measurement, so it is gone.

Method: amelia Mac mini (Apple M4), release build, same account-free fixture for both apps. Every row is the median of n=5 runs, interleaved between the apps. The "no thread link" rows launch with no thread open; the "with thread link" rows open a `codex://threads` link.

- The original could not open a thread without a signed-in account, so its long-thread memory was not measured.

### Download

- Latest release, with the macOS Apple silicon `.dmg`, Linux `AppImage` and Linux `deb`: [github.com/i098/codex-gpui/releases/latest](https://github.com/i098/codex-gpui/releases/latest)

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
