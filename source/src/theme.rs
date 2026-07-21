// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

//! Colour themes: each theme is a (dark, light) pair of `Palette`s. The active
//! theme name + mode (Dark / Light / System) resolve to one `Palette` - the
//! terminal bg/fg/cursor/focus plus the 16 ANSI colours - which `config` folds
//! into `Settings` and `palette.rs` reads. The `[colors]` keys still override on
//! top (a per-colour tweak). Chrome/dialog theming, config-defined themes, and
//! the Settings dropdown build on this foundation.

#[derive(Clone, Copy)]
pub struct Palette {
	pub bg: [u8; 3],
	pub fg: [u8; 3],
	pub cursor: [u8; 3],
	pub focus: [u8; 3],
	// Chrome: menu bar / dropdowns (menu_*) and pop-out dialogs (dialog_*). Every
	// built-in theme uses the SAME neutral defaults below (menu identical in both
	// modes, dialog lighter in Light mode) - a theme MAY override, and the
	// [colors] menu_*/dialog_* keys tweak them per-user.
	pub menu_bg: [u8; 3],
	pub menu_fg: [u8; 3],
	pub dialog_bg: [u8; 3],
	pub dialog_fg: [u8; 3],
	pub ansi: [[u8; 3]; 16],
}

// Shared chrome defaults (same for every theme). The menu keeps one neutral gray
// in both modes (unchanged look); the dialog panel is dark-gray / light-gray by mode.
pub const MENU_BG_DEF: [u8; 3] = [0x36, 0x36, 0x3b];
pub const MENU_FG_DEF: [u8; 3] = [0xf0, 0xf0, 0xf2];
const DLG_BG_DARK: [u8; 3] = [0x20, 0x20, 0x2a];
const DLG_FG_DARK: [u8; 3] = [0xe2, 0xe2, 0xea];
const DLG_BG_LIGHT: [u8; 3] = [0xe6, 0xe6, 0xe3];
const DLG_FG_LIGHT: [u8; 3] = [0x22, 0x24, 0x2c];

#[derive(Clone, Copy)]
pub struct Theme {
	pub dark: Palette,
	pub light: Palette,
}

// The project's original palette - now the default theme's dark variant.
#[rustfmt::skip]
const SILK_DARK: Palette = Palette {
	bg: [0x00, 0x00, 0x00],
	fg: [0x88, 0xff, 0xee],
	cursor: [0xff, 0x88, 0xaa],
	focus: [0x55, 0x80, 0xc8],
	menu_bg: MENU_BG_DEF, menu_fg: MENU_FG_DEF,
	dialog_bg: DLG_BG_DARK, dialog_fg: DLG_FG_DARK,
	ansi: [
		[0x1a, 0x1a, 0x22], [0xe0, 0x6c, 0x75], [0x98, 0xc3, 0x79], [0xe5, 0xc0, 0x7b],
		[0x61, 0xaf, 0xef], [0xc6, 0x78, 0xdd], [0x56, 0xb6, 0xc2], [0xab, 0xb2, 0xbf],
		[0x5c, 0x63, 0x70], [0xef, 0x8a, 0x92], [0xb5, 0xd9, 0x9c], [0xf0, 0xd2, 0x9a],
		[0x8a, 0xc4, 0xf5], [0xd7, 0x9b, 0xeb], [0x7d, 0xcd, 0xd8], [0xe6, 0xe6, 0xee],
	],
};

#[rustfmt::skip]
const SILK_LIGHT: Palette = Palette {
	bg: [0xf6, 0xf5, 0xf0],
	fg: [0x30, 0x32, 0x38],
	cursor: [0x33, 0x55, 0x99],
	focus: [0x33, 0x66, 0xbb],
	menu_bg: MENU_BG_DEF, menu_fg: MENU_FG_DEF,
	dialog_bg: DLG_BG_LIGHT, dialog_fg: DLG_FG_LIGHT,
	ansi: [
		[0x32, 0x32, 0x3a], [0xc0, 0x3a, 0x42], [0x4f, 0x8a, 0x2f], [0xa6, 0x78, 0x12],
		[0x27, 0x65, 0xc0], [0x9a, 0x40, 0xb0], [0x1f, 0x86, 0x96], [0x55, 0x58, 0x60],
		[0x6a, 0x6e, 0x78], [0xd0, 0x4a, 0x52], [0x5f, 0x9a, 0x3f], [0xb0, 0x86, 0x20],
		[0x37, 0x75, 0xd0], [0xaa, 0x50, 0xc0], [0x2f, 0x96, 0xa6], [0x20, 0x22, 0x28],
	],
};

// Matrix: monochrome green. Dark = bright green on near-black; light = dark green
// on a light gray.
#[rustfmt::skip]
const MATRIX_DARK: Palette = Palette {
	bg: [0x00, 0x08, 0x02],
	fg: [0x33, 0xff, 0x66],
	cursor: [0x33, 0xff, 0x66],
	focus: [0x1f, 0xaa, 0x44],
	menu_bg: MENU_BG_DEF, menu_fg: MENU_FG_DEF,
	dialog_bg: DLG_BG_DARK, dialog_fg: DLG_FG_DARK,
	ansi: [
		[0x05, 0x18, 0x0a], [0x2a, 0xcc, 0x44], [0x33, 0xff, 0x66], [0x7a, 0xff, 0x8a],
		[0x1f, 0xaa, 0x3a], [0x44, 0xdd, 0x77], [0x55, 0xee, 0x88], [0x9a, 0xff, 0xaa],
		[0x1a, 0x55, 0x2a], [0x3a, 0xee, 0x55], [0x55, 0xff, 0x77], [0x99, 0xff, 0x99],
		[0x33, 0xcc, 0x55], [0x66, 0xff, 0x99], [0x77, 0xff, 0xaa], [0xcc, 0xff, 0xcc],
	],
};

#[rustfmt::skip]
const MATRIX_LIGHT: Palette = Palette {
	bg: [0xe9, 0xee, 0xe9],
	fg: [0x0a, 0x55, 0x1f],
	cursor: [0x0a, 0x66, 0x22],
	focus: [0x0a, 0x77, 0x2a],
	menu_bg: MENU_BG_DEF, menu_fg: MENU_FG_DEF,
	dialog_bg: DLG_BG_LIGHT, dialog_fg: DLG_FG_LIGHT,
	ansi: [
		[0x14, 0x2a, 0x18], [0x18, 0x6a, 0x2a], [0x0a, 0x55, 0x1f], [0x2a, 0x70, 0x38],
		[0x1a, 0x60, 0x2a], [0x22, 0x6a, 0x34], [0x16, 0x66, 0x2c], [0x2c, 0x52, 0x36],
		[0x3a, 0x5a, 0x40], [0x1f, 0x7a, 0x32], [0x12, 0x66, 0x26], [0x32, 0x80, 0x42],
		[0x22, 0x70, 0x34], [0x2a, 0x7a, 0x3e], [0x1e, 0x76, 0x36], [0x10, 0x30, 0x18],
	],
};

// Retro amber: monochrome amber/orange. Dark = amber on near-black; light = dark
// amber on a warm light gray.
#[rustfmt::skip]
const AMBER_DARK: Palette = Palette {
	bg: [0x10, 0x0a, 0x00],
	fg: [0xff, 0xb0, 0x00],
	cursor: [0xff, 0xb0, 0x00],
	focus: [0xcc, 0x80, 0x00],
	menu_bg: MENU_BG_DEF, menu_fg: MENU_FG_DEF,
	dialog_bg: DLG_BG_DARK, dialog_fg: DLG_FG_DARK,
	ansi: [
		[0x2a, 0x1c, 0x06], [0xff, 0x8c, 0x1a], [0xff, 0xb0, 0x00], [0xff, 0xc8, 0x4a],
		[0xd0, 0x86, 0x10], [0xff, 0xa0, 0x33], [0xff, 0xc0, 0x55], [0xff, 0xd8, 0x9a],
		[0x6a, 0x46, 0x10], [0xff, 0x9a, 0x33], [0xff, 0xbe, 0x33], [0xff, 0xd4, 0x77],
		[0xe0, 0x96, 0x22], [0xff, 0xb0, 0x55], [0xff, 0xcc, 0x77], [0xff, 0xe8, 0xc0],
	],
};

#[rustfmt::skip]
const AMBER_LIGHT: Palette = Palette {
	bg: [0xf2, 0xee, 0xe6],
	fg: [0x7a, 0x42, 0x00],
	cursor: [0x8a, 0x4a, 0x00],
	focus: [0x9a, 0x52, 0x00],
	menu_bg: MENU_BG_DEF, menu_fg: MENU_FG_DEF,
	dialog_bg: DLG_BG_LIGHT, dialog_fg: DLG_FG_LIGHT,
	ansi: [
		[0x33, 0x24, 0x10], [0xa0, 0x4e, 0x08], [0x7a, 0x42, 0x00], [0x90, 0x5a, 0x0c],
		[0x86, 0x46, 0x06], [0x96, 0x52, 0x10], [0x80, 0x4a, 0x08], [0x52, 0x40, 0x2a],
		[0x60, 0x4a, 0x30], [0xb0, 0x58, 0x0e], [0x8a, 0x4c, 0x06], [0xa0, 0x66, 0x12],
		[0x92, 0x50, 0x0a], [0xa6, 0x5c, 0x18], [0x8e, 0x52, 0x0c], [0x28, 0x1c, 0x0c],
	],
};

#[rustfmt::skip]
pub const THEMES: &[(&str, Theme)] = &[
	("SilkTerm", Theme { dark: SILK_DARK, light: SILK_LIGHT }),
	("Matrix", Theme { dark: MATRIX_DARK, light: MATRIX_LIGHT }),
	("Retro Amber", Theme { dark: AMBER_DARK, light: AMBER_LIGHT }),
];

#[allow(dead_code)] // used by the Settings theme dropdown (next increment)
pub fn names() -> impl Iterator<Item = &'static str> {
	THEMES.iter().map(|(n, _)| *n)
}

// Resolve the active palette from a theme name + mode. Unknown name falls back to
// the first theme; `system_dark` chooses the variant when mode is "system".
pub fn resolve(name: &str, mode: &str, system_dark: bool) -> Palette {
	let theme = THEMES
		.iter()
		.find(|(n, _)| n.eq_ignore_ascii_case(name.trim()))
		.map_or(&THEMES[0].1, |(_, t)| t);
	let dark = match mode.trim().to_ascii_lowercase().as_str() {
		"light" => false,
		"system" => system_dark,
		_ => true, // "dark" / unknown
	};
	if dark { theme.dark } else { theme.light }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn resolve_picks_theme_and_mode() {
		// unknown name falls back to the first theme (SilkTerm)
		assert_eq!(resolve("nope", "dark", true).bg, THEMES[0].1.dark.bg);
		// mode selects the variant; "system" honors system_dark
		assert_eq!(resolve("Matrix", "light", true).bg, find("Matrix").light.bg);
		assert_eq!(resolve("Matrix", "system", true).bg, find("Matrix").dark.bg);
		assert_eq!(
			resolve("Matrix", "system", false).bg,
			find("Matrix").light.bg
		);
		// case/space tolerant
		assert_eq!(resolve(" matrix ", "DARK", true).fg, find("Matrix").dark.fg);
	}

	#[test]
	fn chrome_defaults_shared_across_themes() {
		// every built-in theme uses the same neutral menu colours (both modes)
		for (_, t) in THEMES {
			assert_eq!(t.dark.menu_bg, MENU_BG_DEF);
			assert_eq!(t.light.menu_bg, MENU_BG_DEF);
			assert_eq!(t.dark.menu_fg, MENU_FG_DEF);
			// the dialog panel is darker in dark mode than in light mode
			assert!(t.dark.dialog_bg[0] < t.light.dialog_bg[0]);
		}
	}

	fn find(name: &str) -> &'static Theme {
		THEMES
			.iter()
			.find(|(n, _)| *n == name)
			.map(|(_, t)| t)
			.unwrap()
	}
}
