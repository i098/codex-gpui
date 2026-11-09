# Setup

Steps to set up codex-gpui from a clean machine.
Details: [Build and run](docs/build-and-run.md) and [Contributing](CONTRIBUTING.md).

## 1. Tools

1. Install `git` and [`rustup`](https://rustup.rs). `rust-toolchain.toml` pins Rust 1.98.1 with `rustfmt` and `clippy`, so `rustup` installs them on the first build.
2. Install [blaze](https://github.com/indexable-inc/blaze). It is the build tool for Rust commands here: `blaze build|check|run|test`. Use `cargo` for `fmt` and `clippy`.
3. On Ubuntu, install the system packages that CI uses:

   ```sh
   sudo apt-get install -y --no-install-recommends \
     build-essential pkg-config clang cmake libssl-dev libzstd-dev \
     libfontconfig-dev libfreetype-dev libasound2-dev libxkbcommon-dev \
     libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libxcb1-dev \
     libvulkan-dev
   ```

4. Make a `codex` binary available. The app uses the first one that it finds:
   1. `$CODEX_BIN`.
   2. `/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex` (macOS).
   3. `codex` on `PATH`.

The native app, the native tests and the parity harness need macOS on Apple silicon.
Linux builds the workspace, which is enough for the CI checks.

## 2. Get the source

```sh
git clone https://github.com/i098/codex-gpui.git
```

## 3. Build

```sh
blaze build -p app --release
```

## 4. Run

```sh
blaze run -p app --release
```

## 5. Test and check

Run these before you open a pull request:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
blaze test --workspace
```

Run `lefthook install` once to run `cargo fmt --check` before every push.
A change to rendered pixels also needs `cargo run -p parity -- --gate` on macOS.
