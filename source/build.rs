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

// The build number generator, shared with the crate so the number baked in here
// and the tests over there can't be two different implementations.
include!("src/buildnum.rs");

fn main() {
	println!("cargo:rerun-if-changed=assets/silkterm.rc.in");
	println!("cargo:rerun-if-changed=assets/icon.ico");
	println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
	println!("cargo:rerun-if-env-changed=CARGO_PKG_DESCRIPTION");

	emit_build_number();

	// Nothing here belongs in a non-Windows binary, and the no-op has to be
	// explicit: on a WINDOWS host, embed-resource picks its compiler from the
	// host rather than the target, so it happily ran rc.exe and handed the
	// resulting COFF .lib to whatever linker was in play - a Linux cross-build
	// from this box then died with "invalid token in LD script" on it. On a Linux
	// host the same call already came to nothing, so this changes no behavior
	// anywhere; it just stops the one host that got it wrong.
	let target = env::var("TARGET").unwrap_or_default();
	if !target.contains("windows") {
		return;
	}

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

// A version alone can't tell two builds apart - every dogfood build of a release
// shares it - so bake in a number that can. Watching src/ is what keeps it honest:
// without it cargo would only re-run this script when the icon or the .rc changed,
// and the number would sit frozen at whatever it was the first time. Unchanged
// sources produce the same binary and keep the same number, which is the point.
//
// SILK_BUILD_MINUTES pins the value. cicd sets it once per run so all four target
// builds report one build instead of one per link, minutes apart.
fn emit_build_number() {
	println!("cargo:rerun-if-changed=src");
	println!("cargo:rerun-if-env-changed=SILK_BUILD_MINUTES");

	let pinned = env::var("SILK_BUILD_MINUTES").unwrap_or_default();
	let pinned = pinned.trim();
	let minutes = if pinned.is_empty() {
		minutes_since_2000(unix_now())
	} else {
		pinned.parse::<u64>().unwrap_or_else(|_| {
			println!(
				"cargo:warning=SILK_BUILD_MINUTES is not a number ({pinned}); using the clock"
			);
			minutes_since_2000(unix_now())
		})
	};
	println!("cargo:rustc-env=SILK_BUILD={}", crockford32(minutes));
}

// Seconds since the unix epoch. A clock set before 1970 reads as 0, which comes
// out the far end as build number "0" rather than as a failed build.
fn unix_now() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |since| since.as_secs())
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
