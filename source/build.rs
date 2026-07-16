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
	let icon = Path::new(&manifest)
		.join("assets/icon.ico")
		.to_string_lossy()
		.replace('\\', "/");

	let template = fs::read_to_string(Path::new(&manifest).join("assets/silkterm.rc.in")).unwrap();
	let rc = template
		.replace("@ICON@", &icon)
		.replace("@VER_CSV@", &format!("{major},{minor},{patch},0"))
		.replace("@VER_STR@", &ver_str)
		.replace("@DESC@", &desc);

	let rc_path = Path::new(&out).join("silkterm.rc");
	fs::write(&rc_path, rc).unwrap();

	// embed-resource picks its compiler from the build HOST toolchain, not the
	// cargo target: on an msvc host it always runs rc.exe, whose .res mingw's ld
	// can't link. So cross-building a gnu target from an msvc host, drive windres
	// ourselves for a real COFF object. Every other path (Linux cross, gnu host)
	// already uses windres via embed-resource, so leave it be.
	let target = env::var("TARGET").unwrap_or_default();
	let host = env::var("HOST").unwrap_or_default();
	let gnu_target = target.ends_with("-windows-gnu") || target.ends_with("-windows-gnullvm");
	if gnu_target && host.ends_with("-windows-msvc") {
		if let Err(err) = windres_compile(&out, &rc_path) {
			println!("cargo:warning=windows resources not embedded: {err}");
		}
		return;
	}

	let result = embed_resource::compile(&rc_path, embed_resource::NONE);
	if let Err(err) = result.manifest_optional() {
		println!("cargo:warning=windows resources not embedded: {err}");
	}
}

// Compile the .rc to a COFF object with mingw windres and hand it to the linker.
// Non-fatal by contract (see the caller): a windres miss - not on PATH, or no PE
// support for the arch (aarch64) - just warns and the exe builds iconless.
fn windres_compile(out: &str, rc_path: &Path) -> Result<(), String> {
	let bfd = match env::var("CARGO_CFG_TARGET_ARCH")
		.unwrap_or_default()
		.as_str()
	{
		"x86_64" => "pe-x86-64",
		"aarch64" => "pe-aarch64-little",
		"x86" => "pe-i386",
		other => return Err(format!("no windres bfd target for arch {other}")),
	};
	let obj = Path::new(out).join("silkterm-res.o");
	// -c 65001: the .rc is UTF-8 (the © in the copyright string). -O coff: a
	// linkable object, not a raw .res.
	let ok = std::process::Command::new("windres")
		.args(["-c", "65001", "-O", "coff", "--target", bfd, "-I"])
		.arg(out)
		.arg("-i")
		.arg(rc_path)
		.arg("-o")
		.arg(&obj)
		.status()
		.map_err(|err| format!("windres not runnable: {err}"))?
		.success();
	if !ok {
		return Err("windres failed to compile the resource".into());
	}
	println!("cargo:rustc-link-arg-bins={}", obj.display());
	Ok(())
}
