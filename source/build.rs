// Embed the app icon into the Windows PE, so Explorer, taskbar pins and the
// installer show the real icon instead of a generic exe. embed_resource::compile
// finds the resource compiler via the cc crate's toolchain logic (rc.exe for
// msvc, windres for gnu/gnullvm) - the same mechanism rustc uses to find the
// linker - so it works both natively and on the Linux->windows cross-build.
// It no-ops on non-windows targets. Non-fatal: if the compiler is missing or
// can't target this arch (e.g. aarch64 windres), warn and build on iconless.
fn main() {
	println!("cargo:rerun-if-changed=assets/silkterm.rc");
	println!("cargo:rerun-if-changed=assets/icon.ico");
	let result = embed_resource::compile("assets/silkterm.rc", embed_resource::NONE);
	if let Err(err) = result.manifest_optional() {
		println!("cargo:warning=icon resource not embedded: {err}");
	}
}
