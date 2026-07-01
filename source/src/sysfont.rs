// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

//! Best-effort detection of the OS's default monospace/fixed-pitch font (family
//! and point size). Each platform uses its native mechanism; everything is
//! optional and falls back gracefully. Detected once and cached, since it's read
//! a couple of times at startup (config size + font-family resolution).

use std::sync::OnceLock;

#[derive(Default, Clone)]
pub struct Monospace {
	pub family: Option<String>, // e.g. "Monaspace Argon" (style/size stripped)
	pub size_pt: Option<f32>,   // points
}

/// Cached system monospace description.
pub fn monospace() -> &'static Monospace {
	static M: OnceLock<Monospace> = OnceLock::new();
	M.get_or_init(platform::monospace)
}

/// Cached OS sans-serif (proportional UI) family, best-effort. Used to pin the
/// chrome font - the generic `Family::SansSerif` is unreliable (fontdb defaults
/// it to "Arial" and falls through to a serif when that's absent).
pub fn sans_serif() -> Option<&'static str> {
	static S: OnceLock<Option<String>> = OnceLock::new();
	S.get_or_init(sans_serif_detect).as_deref()
}

#[cfg(target_os = "linux")]
fn sans_serif_detect() -> Option<String> {
	// fontconfig's `sans-serif` alias resolves to a real sans face regardless of
	// the user's (possibly serif) document font.
	let out = std::process::Command::new("fc-match")
		.args(["--format=%{family}", "sans-serif"])
		.output()
		.ok()?;
	out.status.success().then_some(())?;
	let s = String::from_utf8(out.stdout).ok()?;
	let fam = s.trim().split(',').next().unwrap_or("").trim(); // may be a list
	(!fam.is_empty()).then(|| fam.to_string())
}

#[cfg(not(target_os = "linux"))]
fn sans_serif_detect() -> Option<String> {
	None // other platforms fall back to the curated list in text::resolve_sans_family
}

#[cfg(target_os = "linux")]
mod platform {
	use super::Monospace;
	use std::process::Command;

	pub fn monospace() -> Monospace {
		if let Some(desc) = gsettings_desc() {
			let m = parse_pango(&desc);
			if m.family.is_some() || m.size_pt.is_some() {
				return m;
			}
		}
		// No GNOME settings: fontconfig gives a size, no specific family.
		Monospace {
			family: None,
			size_pt: fontconfig_size(),
		}
	}

	fn gsettings_desc() -> Option<String> {
		let out = Command::new("gsettings")
			.args(["get", "org.gnome.desktop.interface", "monospace-font-name"])
			.output()
			.ok()?;
		out.status.success().then_some(())?;
		String::from_utf8(out.stdout).ok()
	}

	fn fontconfig_size() -> Option<f32> {
		let out = Command::new("fc-match")
			.args(["--format=%{size}", "monospace"])
			.output()
			.ok()?;
		out.status.success().then_some(())?;
		String::from_utf8(out.stdout).ok()?.trim().parse().ok()
	}

	// Parse a Pango font description "Family Style... Size", e.g.
	// "Monaspace Argon Semi-Bold 13" -> family "Monaspace Argon", size 13.
	fn parse_pango(desc: &str) -> Monospace {
		let desc = desc.trim().trim_matches(['\'', '"']);
		let mut tokens: Vec<&str> = desc.split_whitespace().collect();

		let mut size_pt = None;
		if let Some(last) = tokens.last() {
			if let Ok(n) = last.parse::<f32>() {
				size_pt = Some(n);
				tokens.pop();
			}
		}
		// Drop trailing weight/style/stretch words so only the family remains.
		while tokens.last().is_some_and(|t| is_style_word(t)) {
			tokens.pop();
		}
		let family = (!tokens.is_empty()).then(|| tokens.join(" "));
		Monospace { family, size_pt }
	}

	fn is_style_word(w: &str) -> bool {
		const STYLES: &[&str] = &[
			"thin",
			"hairline",
			"extralight",
			"extra-light",
			"ultralight",
			"ultra-light",
			"light",
			"semilight",
			"semi-light",
			"demilight",
			"demi-light",
			"book",
			"regular",
			"normal",
			"medium",
			"semibold",
			"semi-bold",
			"demibold",
			"demi-bold",
			"bold",
			"extrabold",
			"extra-bold",
			"ultrabold",
			"ultra-bold",
			"black",
			"heavy",
			"italic",
			"oblique",
			"condensed",
			"semicondensed",
			"semi-condensed",
			"expanded",
			"semiexpanded",
			"semi-expanded",
			"roman",
		];
		STYLES.iter().any(|s| s.eq_ignore_ascii_case(w))
	}
}

#[cfg(target_os = "macos")]
mod platform {
	use super::Monospace;
	use std::process::Command;

	pub fn monospace() -> Monospace {
		Monospace {
			family: family(),
			size_pt: size(),
		}
	}

	fn defaults_global(key: &str) -> Option<String> {
		let out = Command::new("defaults")
			.args(["read", "-g", key])
			.output()
			.ok()?;
		out.status.success().then_some(())?;
		Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
	}

	fn size() -> Option<f32> {
		defaults_global("NSFixedPitchFontSize")?.parse().ok()
	}

	fn family() -> Option<String> {
		// Stored as a PostScript name, e.g. "Menlo-Regular"; take the family part.
		let ps = defaults_global("NSFixedPitchFont")?;
		let fam = ps.split('-').next().unwrap_or(&ps).trim();
		(!fam.is_empty()).then(|| fam.to_string())
	}
}

#[cfg(windows)]
mod platform {
	use super::Monospace;
	use windows_sys::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, LOGPIXELSY, ReleaseDC};
	use windows_sys::Win32::UI::WindowsAndMessaging::{
		NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SystemParametersInfoW,
	};

	// Windows has no dedicated monospace setting; report the message-box font
	// size (the conventional system size). No reliable system *monospace* family,
	// so leave family None and let the generic monospace resolution pick one.
	pub fn monospace() -> Monospace {
		Monospace {
			family: None,
			size_pt: message_font_pt(),
		}
	}

	fn message_font_pt() -> Option<f32> {
		unsafe {
			let mut ncm: NONCLIENTMETRICSW = core::mem::zeroed();
			ncm.cbSize = core::mem::size_of::<NONCLIENTMETRICSW>() as u32;
			let ok = SystemParametersInfoW(
				SPI_GETNONCLIENTMETRICS,
				ncm.cbSize,
				core::ptr::addr_of_mut!(ncm).cast(),
				0,
			);
			if ok == 0 {
				return None;
			}
			let h = ncm.lfMessageFont.lfHeight;
			if h == 0 {
				return None;
			}
			let dc = GetDC(core::ptr::null_mut());
			let dpi = if dc.is_null() {
				96
			} else {
				GetDeviceCaps(dc, LOGPIXELSY as i32)
			};
			if !dc.is_null() {
				ReleaseDC(core::ptr::null_mut(), dc);
			}
			let dpi = if dpi <= 0 { 96 } else { dpi };
			Some(h.unsigned_abs() as f32 * 72.0 / dpi as f32)
		}
	}
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod platform {
	use super::Monospace;
	pub fn monospace() -> Monospace {
		Monospace::default()
	}
}
