# Building SilkTerm

SilkTerm is a Rust project using `wgpu` (GPU), `winit` (windowing), `glyphon` (text), and `alacritty_terminal` (VT parser + PTY). From an x86_64 Linux host it builds Linux x86_64/ARM64 and Windows x86_64/ARM64 (the latter three are cross-compiled); macOS is built natively on a Mac.

## Toolchain

Requires a Rust toolchain (edition 2024, rustc >= 1.85). `rustup` is the simplest way to manage targets:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Linux (native)

```sh
cargo build --release
./target/release/silkterm
```

Runtime needs a GPU with Vulkan or GL (X11 or Wayland). On Debian/X11 the deps are the usual Mesa/Vulkan packages already present on a desktop.

## Windows (cross-compile from Linux)

Uses the GNU ABI so it links with mingw-w64 - no MSVC or Windows host needed.

One-time setup:

```sh
rustup target add x86_64-pc-windows-gnu
sudo apt-get install -y gcc-mingw-w64-x86-64    # Debian/Ubuntu
```

Build:

```sh
cargo build --release --target x86_64-pc-windows-gnu
# -> target/x86_64-pc-windows-gnu/release/silkterm.exe
```

The linker and a static-CRT flag are wired up in `.cargo/config.toml`, so the resulting `.exe` is self-contained (depends only on stock Windows system DLLs) and is a GUI binary (no console window in release builds).

## ARM64 (Linux & Windows, cross-compile via cargo-zigbuild)

`cargo-zigbuild` uses `zig` as a universal cross-linker, so ARM64 Linux and Windows build from an x86_64 Linux host with no per-target gcc/SDK.

One-time setup:

```sh
# zig 0.13.0 (binary tarball; `pip install ziglang` also works)
curl -fsSL https://ziglang.org/download/0.13.0/zig-linux-x86_64-0.13.0.tar.xz | tar -xJ -C ~/.local
ln -sf ~/.local/zig-linux-x86_64-0.13.0/zig ~/.local/bin/zig    # ~/.local/bin on PATH
cargo install cargo-zigbuild
rustup target add aarch64-unknown-linux-gnu aarch64-pc-windows-gnullvm
```

Build:

```sh
cargo zigbuild --release --target aarch64-unknown-linux-gnu     # Linux ARM64
cargo zigbuild --release --target aarch64-pc-windows-gnullvm    # Windows ARM64
```

No link-time ARM64 system libraries are needed: X11/EGL/Wayland are loaded at runtime (dlopen), so the link succeeds with only zig's bundled libc/CRT. Verified build-clean on both ARM64 targets (Linux ELF aarch64; Windows PE32+ ARM64).

## Windows (native, MSVC)

If building on Windows with the MSVC toolchain instead:

```sh
rustup target add x86_64-pc-windows-msvc
cargo build --release            # default target on a Windows host
```

## macOS (native)

macOS is built natively on a Mac (cross-compiling Linux->macOS needs the Apple SDK and is not set up here). On a Mac:

```sh
cargo build --release
./target/release/silkterm
```

wgpu uses the Metal backend automatically. No extra system packages are needed beyond the Xcode command-line tools (`xcode-select --install`).

## Formatting

`rustfmt.toml` pins the style (`hard_tabs`). The hand-formatted data tables (the `Palette`/`Dlg` colour matrices in `theme.rs`/`settings_ui.rs`, the About table in `dialog.rs`) carry `#[rustfmt::skip]` so `cargo fmt` leaves them compact; everything else is rustfmt-canonical.

A pre-commit hook (`tools/git-hooks/pre-commit`) reformats the staged `.rs` files on every commit so they never drift. Activate it once per clone:

```sh
git config core.hooksPath tools/git-hooks
```
