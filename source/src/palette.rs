// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor};

use crate::config::Settings;

// The 16 ANSI colours come from the active theme (config::settings().ansi),
// resolved in config from the theme name + mode. See theme.rs. Callers pass
// their per-frame Settings snapshot: this runs ~2x per cell per rebuilt frame,
// and settings() is an RwLock read + Arc clone - too hot to take per colour.

pub fn resolve(c: Color, colors: &Colors, s: &Settings) -> [u8; 3] {
	match c {
		Color::Spec(rgb) => [rgb.r, rgb.g, rgb.b],
		Color::Indexed(i) => indexed(i, colors, s),
		Color::Named(n) => named(n, colors, s),
	}
}

fn indexed(i: u8, colors: &Colors, s: &Settings) -> [u8; 3] {
	if let Some(rgb) = colors[i as usize] {
		return [rgb.r, rgb.g, rgb.b];
	}
	default_indexed(i, s)
}

fn default_indexed(i: u8, s: &Settings) -> [u8; 3] {
	match i {
		0..=15 => s.ansi[i as usize],
		16..=231 => {
			// 6x6x6 cube
			let v = i - 16;
			let r = v / 36;
			let g = (v % 36) / 6;
			let b = v % 6;
			let step = |n: u8| if n == 0 { 0u8 } else { 55 + n * 40 };
			[step(r), step(g), step(b)]
		}
		_ => {
			// grayscale ramp 232..=255
			let l = 8 + (i - 232) * 10;
			[l, l, l]
		}
	}
}

fn named(n: NamedColor, colors: &Colors, s: &Settings) -> [u8; 3] {
	if let Some(rgb) = colors[n] {
		return [rgb.r, rgb.g, rgb.b];
	}
	use NamedColor::*;
	match n {
		Foreground | DimForeground | BrightForeground => s.fg,
		Background => s.bg,
		Cursor => s.cursor,
		Black | DimBlack => s.ansi[0],
		Red | DimRed => s.ansi[1],
		Green | DimGreen => s.ansi[2],
		Yellow | DimYellow => s.ansi[3],
		Blue | DimBlue => s.ansi[4],
		Magenta | DimMagenta => s.ansi[5],
		Cyan | DimCyan => s.ansi[6],
		White | DimWhite => s.ansi[7],
		BrightBlack => s.ansi[8],
		BrightRed => s.ansi[9],
		BrightGreen => s.ansi[10],
		BrightYellow => s.ansi[11],
		BrightBlue => s.ansi[12],
		BrightMagenta => s.ansi[13],
		BrightCyan => s.ansi[14],
		BrightWhite => s.ansi[15],
	}
}
