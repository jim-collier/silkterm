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

#[derive(Default, Clone)]
pub struct UiFont {
	pub family: Option<String>, // desktop interface font family, e.g. "GentiumAlt"
	pub size_pt: Option<f32>,   // points
	pub bold: bool,             // desktop asks for a bold UI face
	pub italic: bool,
}

/// Cached desktop *interface* (UI/chrome) font: whatever the user picked for
/// menus and dialogs - serif or not. This is the first choice for chrome text;
/// sans_serif() below is only the fallback when no desktop setting is readable.
pub fn interface() -> &'static UiFont {
	static U: OnceLock<UiFont> = OnceLock::new();
	U.get_or_init(platform::interface)
}

/// Cached OS sans-serif (proportional UI) family, best-effort. Fallback for
/// chrome when no desktop interface font is readable - the generic
/// `Family::SansSerif` is unreliable (fontdb defaults it to "Arial" and falls
/// through to a serif when that's absent).
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
	let family_list = String::from_utf8(out.stdout).ok()?;
	let family = family_list.trim().split(',').next().unwrap_or("").trim(); // may be a list
	(!family.is_empty()).then(|| family.to_string())
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
		if let Some(desc) = gsettings_desc("monospace-font-name") {
			let parsed = parse_pango(&desc);
			if parsed.family.is_some() || parsed.size_pt.is_some() {
				return Monospace {
					family: parsed.family,
					size_pt: parsed.size_pt,
				};
			}
		}
		// No GNOME settings: fontconfig gives a size, no specific family.
		Monospace {
			family: None,
			size_pt: fontconfig_size(),
		}
	}

	// The desktop interface (UI) font. GNOME/MATE/Cinnamon expose it through
	// gsettings; Xfce through xfconf. Either may be a serif - that's the point:
	// chrome follows whatever the user picked, not a sans assumption.
	pub fn interface() -> super::UiFont {
		gsettings_desc("font-name")
			.or_else(xfconf_ui_desc)
			.map(|desc| parse_pango(&desc))
			.filter(|parsed| parsed.family.is_some() || parsed.size_pt.is_some())
			.unwrap_or_default()
	}

	fn gsettings_desc(key: &str) -> Option<String> {
		let out = Command::new("gsettings")
			.args(["get", "org.gnome.desktop.interface", key])
			.output()
			.ok()?;
		out.status.success().then_some(())?;
		String::from_utf8(out.stdout).ok()
	}

	fn xfconf_ui_desc() -> Option<String> {
		let out = Command::new("xfconf-query")
			.args(["-c", "xsettings", "-p", "/Gtk/FontName"])
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
	// "GentiumAlt Bold 13" -> family "GentiumAlt", size 13, bold. The style
	// words are captured (bold/italic), not just stripped, so chrome can honour
	// the desktop's weight/slant.
	fn parse_pango(desc: &str) -> super::UiFont {
		let desc = desc.trim().trim_matches(['\'', '"']);
		let mut tokens: Vec<&str> = desc.split_whitespace().collect();

		let mut size_pt = None;
		if let Some(last) = tokens.last() {
			if let Ok(size) = last.parse::<f32>() {
				size_pt = Some(size);
				tokens.pop();
			}
		}
		// Peel trailing weight/style/stretch words so only the family remains.
		let (mut bold, mut italic) = (false, false);
		while tokens.last().is_some_and(|t| is_style_word(t)) {
			let word = tokens.pop().unwrap().to_ascii_lowercase();
			match word.as_str() {
				"bold" | "semibold" | "semi-bold" | "demibold" | "demi-bold" | "extrabold"
				| "extra-bold" | "ultrabold" | "ultra-bold" | "black" | "heavy" => bold = true,
				"italic" | "oblique" => italic = true,
				_ => {}
			}
		}
		let family = (!tokens.is_empty()).then(|| tokens.join(" "));
		super::UiFont {
			family,
			size_pt,
			bold,
			italic,
		}
	}

	fn is_style_word(word: &str) -> bool {
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
		STYLES.iter().any(|style| style.eq_ignore_ascii_case(word))
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
		let postscript_name = defaults_global("NSFixedPitchFont")?;
		let family = postscript_name
			.split('-')
			.next()
			.unwrap_or(&postscript_name)
			.trim();
		(!family.is_empty()).then(|| family.to_string())
	}

	// macOS has no user-set UI font, and the actual one (San Francisco) hides
	// behind a private name fontdb can't query. Report the conventional AppKit
	// system size and let the family fall back (curated list has Helvetica Neue).
	pub fn interface() -> super::UiFont {
		super::UiFont {
			family: None,
			size_pt: Some(13.0),
			bold: false,
			italic: false,
		}
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

	// The menu font is what native chrome (menus/dialog labels) uses; family,
	// size, weight and slant all honour the user's "Menu" font setting.
	pub fn interface() -> super::UiFont {
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
				return super::UiFont::default();
			}
			let lf = &ncm.lfMenuFont;
			let end = lf.lfFaceName.iter().position(|&c| c == 0).unwrap_or(32);
			let family = String::from_utf16(&lf.lfFaceName[..end])
				.ok()
				.map(|f| f.trim().to_string())
				.filter(|f| !f.is_empty());
			let size_pt = (lf.lfHeight != 0).then(|| {
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
				lf.lfHeight.unsigned_abs() as f32 * 72.0 / dpi as f32
			});
			super::UiFont {
				family,
				size_pt,
				bold: lf.lfWeight >= 600,
				italic: lf.lfItalic != 0,
			}
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
	use super::{Monospace, UiFont};
	pub fn monospace() -> Monospace {
		Monospace::default()
	}
	pub fn interface() -> UiFont {
		UiFont::default()
	}
}
