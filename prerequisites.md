# Development prerequisites

What to install before you can develop and build SilkTerm, per OS. Once these are in place, [build.md](build.md) has the actual build commands (native + cross), and the CI/CD pipelines (`cicd/cicd.bash` on Linux, `cicd/cicd-win.ps1` on Windows) drive the full lint/test/build/package flow.

SilkTerm is a Rust project (edition 2024, rustc >= 1.89). The toolchain channel is pinned in `rust-toolchain.toml` (1.96.0 + rustfmt/clippy + the cross targets); `rustup`-routed cargo picks it up automatically on first build.

## Windows (native)

The Windows box builds and dogfoods natively. Ship target is the **gnu** ABI (self-contained, matches the Linux cross-build); **msvc** is handy for local debugging. The pipeline builds both, so set up both.

### PowerShell 7

The Windows pipeline (`cicd/cicd-win.ps1`) and the dogfood launcher (`n8runterm.ps1`) require **PowerShell 7+** (`pwsh`), not Windows PowerShell 5.1.

```powershell
choco install powershell-core
```

Scripts here are unsigned local files, so clear the execution policy once (CurrentUser scope, no admin; PS7 and 5.1 keep separate policies - set it from `pwsh`):

```powershell
Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
```

Or bypass per-run without changing anything: `pwsh -ExecutionPolicy Bypass -File cicd\cicd-win.ps1`.

### Rust (rustup)

Install `rustup` with the gnu host and the pinned toolchain (the repo's `rust-toolchain.toml` re-pins it anyway):

```powershell
choco install rustup.install
rustup toolchain install 1.96.0
rustup target add x86_64-pc-windows-gnu x86_64-pc-windows-msvc
```

`rustup`'s shims live in `%USERPROFILE%\.cargo\bin` (on PATH after a new shell).

### mingw-w64 (the gnu linker)

The gnu target links with mingw-w64:

```powershell
choco install mingw
```

- Lands at `C:\ProgramData\mingw64\mingw64\bin` - add it to PATH for a gnu build.
- Gotcha: `.cargo/config.toml` names `x86_64-w64-mingw32-ar`, but mingw ships only plain `ar`. Copy it once (redo if mingw is reinstalled), or override with `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_AR=ar`:

```powershell
Copy-Item "C:\ProgramData\mingw64\mingw64\bin\ar.exe" "C:\ProgramData\mingw64\mingw64\bin\x86_64-w64-mingw32-ar.exe"
```

### Visual Studio Build Tools (the msvc linker)

The msvc target needs the VC++ build tools; rustc finds `link.exe` automatically (no dev prompt):

```powershell
choco install visualstudio2022-workload-vctools
```

### NSIS (optional - installers)

Only needed if you want the pipeline to build the Windows installer `.exe` (stage 5):

```powershell
choco install nsis
```

`makensis` installs to `C:\Program Files (x86)\NSIS`; the pipeline probes there, so it needs no PATH change.

### ARM64 (optional - either path yields an ARM64 build)

Skip unless you need Windows-on-ARM binaries. Pick one:

- **gnullvm via zig** (recommended - self-contained, small footprint; the target is preinstalled):

	```powershell
	choco install zig
	cargo install cargo-zigbuild --version 0.23.0 --locked
	```

	The Linux box uses zig 0.13; if a newer choco zig errors on a version mismatch, pin `zig --version 0.13.0`.

- **msvc via the VS ARM64 workload** (larger; links the MSVC runtime):

	```powershell
	choco install visualstudio2022-workload-vctools --package-parameters "--add Microsoft.VisualStudio.Component.VC.Tools.ARM64"
	rustup target add aarch64-pc-windows-msvc
	```

	If the VC tools are already installed, choco may no-op the add - then use the Visual Studio Installer (Modify -> "MSVC v143 ARM64 build tools") instead.

The pipeline auto-detects each ARM path and warn-skips it when its toolchain is absent, so ARM is never a hard requirement.

### Build

```powershell
pwsh cicd\cicd-win.ps1 -Quick       # fmt, build, test, lint, release (msvc + gnu)
pwsh cicd\cicd-win.ps1              # + ARM64 (if set up), installers, dogfood, publish
```

Or a bare build - see [build.md](build.md#windows-native-msvc).

## Linux (native, and cross-build host)

Linux x86_64 is the primary dev target and the host that cross-builds Windows + ARM64.

### Rust + native runtime

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Running needs a Vulkan/GL-capable GPU under X11 or Wayland - on a Debian desktop the usual Mesa/Vulkan packages are already present.

### Cross toolchains

- Windows x86_64 (gnu): `sudo apt-get install -y gcc-mingw-w64-x86-64` + `rustup target add x86_64-pc-windows-gnu`.
- ARM64 (Linux + Windows) via `cargo-zigbuild`: install zig 0.13 + `cargo install cargo-zigbuild`, then `rustup target add aarch64-unknown-linux-gnu aarch64-pc-windows-gnullvm`.

Full commands in [build.md](build.md).

### Pipeline helpers (optional)

For the full `cicd/cicd.bash` run (packages, deps check): `cargo install cargo-deny cargo-deb cargo-generate-rpm cargo-zigbuild` and `makensis` (for the Windows installer). Versions are pinned in `cicd/config.bash` (`TOOL_PINS`).

## macOS (native)

macOS builds natively on a Mac (there's no Linux->macOS cross set up here - it needs the Apple SDK).

```sh
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo build --release
```

wgpu uses the Metal backend automatically; no extra system packages beyond the Xcode command-line tools.
