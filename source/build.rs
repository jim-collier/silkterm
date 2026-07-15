// Embed the app icon + version info into the Windows PE, so Explorer, taskbar
// pins and the installer show the real icon, and Properties > Details shows the
// version/product strings. The .rc is generated from assets/silkterm.rc.in with
// the version + description filled in from Cargo metadata (so they never drift
// from Cargo.toml), then compiled by embed-resource - which finds the resource
// compiler via the cc crate (rc.exe for msvc, windres for gnu/gnullvm), the same
// way rustc finds the linker, so it works natively and cross from Linux. It
// no-ops on non-windows targets. Non-fatal: if the compiler is missing or can't
// target this arch (e.g. aarch64 windres), warn and build on iconless.
use std::{env, fs, path::Path};

fn main() {
	println!("cargo:rerun-if-changed=assets/silkterm.rc.in");
	println!("cargo:rerun-if-changed=assets/icon.ico");
	println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
	println!("cargo:rerun-if-env-changed=CARGO_PKG_DESCRIPTION");

	let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
	let out = env::var("OUT_DIR").unwrap();

	let major = env::var("CARGO_PKG_VERSION_MAJOR").unwrap();
	let minor = env::var("CARGO_PKG_VERSION_MINOR").unwrap();
	let patch = env::var("CARGO_PKG_VERSION_PATCH").unwrap();
	let ver_str = env::var("CARGO_PKG_VERSION").unwrap();
	let desc = env::var("CARGO_PKG_DESCRIPTION").unwrap_or_default();

	// forward slashes so the absolute path needs no backslash escaping, and works
	// under both rc.exe and windres (incl. windres running on the Linux cross-build)
	let icon = Path::new(&manifest).join("assets/icon.ico").to_string_lossy().replace('\\', "/");

	let template = fs::read_to_string(Path::new(&manifest).join("assets/silkterm.rc.in")).unwrap();
	let rc = template
		.replace("@ICON@", &icon)
		.replace("@VER_CSV@", &format!("{major},{minor},{patch},0"))
		.replace("@VER_STR@", &ver_str)
		.replace("@DESC@", &desc);

	let rc_path = Path::new(&out).join("silkterm.rc");
	fs::write(&rc_path, rc).unwrap();

	let result = embed_resource::compile(&rc_path, embed_resource::NONE);
	if let Err(err) = result.manifest_optional() {
		println!("cargo:warning=windows resources not embedded: {err}");
	}
}
