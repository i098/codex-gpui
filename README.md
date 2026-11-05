# codex-gpui

codex-gpui is an open-source port of the Codex desktop app to Rust and [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).
The original is the Codex section of the macOS ChatGPT app.
The goal is pixel parity with the original: the same layout, text, colors and icons, device pixel for device pixel.
The app drives the real `codex app-server` over stdio, the same JSON-RPC transport that the original uses.

[Build and run](docs/build-and-run.md) · [Architecture](docs/architecture.md) · [Parity](docs/parity.md) · [Contributing](CONTRIBUTING.md) · [Security](.github/SECURITY.md) · [Changelog](CHANGELOG.md)

## Status

The app runs on macOS (Apple silicon and Intel). The workspace also builds on Linux for CI. Pixel parity is checked by the harness in [Parity and proof](docs/parity.md); the sections there list what is and is not proven.

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
