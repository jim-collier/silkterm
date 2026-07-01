// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor};

use crate::config;

// The 16 ANSI colours come from the active theme (config::settings().ansi),
// resolved in config from the theme name + mode. See theme.rs.

pub fn resolve(c: Color, colors: &Colors) -> [u8; 3] {
	match c {
		Color::Spec(rgb) => [rgb.r, rgb.g, rgb.b],
		Color::Indexed(i) => indexed(i, colors),
		Color::Named(n) => named(n, colors),
	}
}

fn indexed(i: u8, colors: &Colors) -> [u8; 3] {
	if let Some(rgb) = colors[i as usize] {
		return [rgb.r, rgb.g, rgb.b];
	}
	default_indexed(i)
}

fn default_indexed(i: u8) -> [u8; 3] {
	match i {
		0..=15 => config::settings().ansi[i as usize],
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

fn named(n: NamedColor, colors: &Colors) -> [u8; 3] {
	if let Some(rgb) = colors[n] {
		return [rgb.r, rgb.g, rgb.b];
	}
	use NamedColor::*;
	match n {
		Foreground | DimForeground | BrightForeground => config::settings().fg,
		Background => config::settings().bg,
		Cursor => config::settings().cursor,
		Black | DimBlack => config::settings().ansi[0],
		Red | DimRed => config::settings().ansi[1],
		Green | DimGreen => config::settings().ansi[2],
		Yellow | DimYellow => config::settings().ansi[3],
		Blue | DimBlue => config::settings().ansi[4],
		Magenta | DimMagenta => config::settings().ansi[5],
		Cyan | DimCyan => config::settings().ansi[6],
		White | DimWhite => config::settings().ansi[7],
		BrightBlack => config::settings().ansi[8],
		BrightRed => config::settings().ansi[9],
		BrightGreen => config::settings().ansi[10],
		BrightYellow => config::settings().ansi[11],
		BrightBlue => config::settings().ansi[12],
		BrightMagenta => config::settings().ansi[13],
		BrightCyan => config::settings().ansi[14],
		BrightWhite => config::settings().ansi[15],
	}
}
