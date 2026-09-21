// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier [ID: 2უNაɘ«҂թȹɤξπ๙¿ձϖ]

//! What light mode has to do differently to read the way dark mode does.
//!
//! Both settings here are linear-light alphas, and linear light is not what the
//! eye reads. sRGB's curve is steep at the bottom and flat at the top, so the
//! same alpha covers a lot of visible ground against a near-black background and
//! almost none against a near-white one. Dark mode is the reference and is never
//! touched: every function here returns the value it was handed when the active
//! mode is dark.
//!
//! - **Wallpaper visibility.** The slider means one thing: how much of the
//!   picture's own contrast reaches the screen. A linear-light blend delivers
//!   that over a near-black background and almost none of it over a light one,
//!   so light mode mixes the background and the picture in a power curve
//!   instead, at the amount dark mode's blend would have delivered. Nothing is
//!   calibrated by eye and nothing is solved: both halves are closed form.
//! - **Text scrim.** The halo is the background color laid over whatever the
//!   picture put there, so in light mode it is a pale plate on a darkened
//!   field, which is the same move in the direction the eye notices most. That
//!   composite blends against the destination through the pipeline's blend
//!   state and cannot read it, so the halo is still a calibration: its alpha is
//!   scaled down until it covers the same ground dark mode's does.
//!
//! The measure throughout is the sRGB transfer curve taken on Rec.709 luma. Luma
//! because it is affine under the alpha composite, so one number stands in for a
//! whole blend; the transfer curve because it tracks CIE L* closely enough here
//! and it is already the program's color language.

use crate::config::{self, Settings};

// Where the transfer curve's slope is read when the two modes are compared, as
// a linear luma. Measured over the shipped pack of 104: median 0.129, mean
// 0.141, quartiles 0.062 and 0.193. A picture far from this one is out by a few
// points, not by a factor.
const WALLPAPER_LUMA: f32 = 0.13;

// The transfer curve the mix happens in. A pure power rather than sRGB's own,
// because sRGB's `- 0.055` term does not cancel: over a black background the
// power curve makes the mix exactly the linear blend it replaces, and sRGB's
// would lift the black by eight levels.
const MIX_GAMMA: f32 = 2.4;

// Where the two halos are compared. The gain cannot be right across the whole
// falloff - the curves meet at both ends whatever it is - so it is matched at
// half the halo, which is the widest the gap gets.
const HALO_REF: f32 = 0.5;

// The halo never drops below a quarter of what was asked for. Past that the
// plate stops doing the job it is there for, and legibility over a busy picture
// matters more than the plate being tidy.
const MIN_HALO_GAIN: f32 = 0.25;

// Does this settings copy resolve to the dark variant? `Settings` rather than
// the live store, so everything here stays a function of what it is handed.
fn dark(s: &Settings) -> bool {
	crate::theme::is_dark_mode(&s.theme_mode, config::os_dark())
}

// The theme's own dark background - what "as prominent as dark mode" is measured
// against. An overridden `bg` in light mode is still compared with the theme the
// user picked, since that is the dark mode they would see.
fn paired_dark_luma(s: &Settings) -> f32 {
	config::luma(crate::theme::resolve_in(&s.user_themes, &s.theme, "dark", true).bg)
}

// How far a linear-light mix of `from` toward `to` travels in sRGB-encoded luma.
// Signed, in the direction of the mix.
fn shift(from: f32, to: f32, alpha: f32) -> f32 {
	config::from_linear(from + (to - from) * alpha) - config::from_linear(from)
}

// The inverse: the alpha at which that mix covers `want` of sRGB distance. A mix
// that cannot get that far is pinned at 1.
fn alpha_for(from: f32, to: f32, want: f32) -> f32 {
	let span = to - from;
	if span.abs() < 1e-4 {
		return 1.0;
	}
	let target = (config::from_linear(from) + span.signum() * want).clamp(0.0, 1.0);
	((config::to_linear_f32(target) - from) / span).clamp(0.0, 1.0)
}

// The sRGB transfer curve's slope at a linear value. How much of a change in the
// picture the eye gets back, where the composite happens to sit.
fn slope(at: f32) -> f32 {
	if at <= 0.003_130_8 {
		12.92
	} else {
		(1.055 / 2.4) * at.powf(1.0 / 2.4 - 1.0)
	}
}

// How much of the picture's own contrast a linear-light blend at `alpha` puts on
// screen, over a background of `bg`. This is what the visibility slider has
// always meant, whether or not anyone said so.
//
// Over pure black it works out at exactly `alpha^(1/2.4)`, which is why dark
// mode has never needed any of this: black leaves the blend a pure scale of the
// encoded picture, and a scale cannot touch contrast. A dark theme whose
// background is not black delivers less, and this says how much less.
fn encoded_scale(alpha: f32, bg: f32) -> f32 {
	alpha * slope(alpha * WALLPAPER_LUMA + (1.0 - alpha) * bg) / slope(WALLPAPER_LUMA)
}

// What the wallpaper pass does this frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Mix {
	// The alpha of the linear blend, or the share of the picture in the power
	// curve. Which one is decided by `perceptual`.
	pub amount: f32,
	// False is the linear-light blend the program has always drawn, and is what
	// dark mode gets. True mixes the background and the picture in a power curve,
	// which needs the background color and so cannot be a hardware blend.
	pub perceptual: bool,
}

impl Mix {
	// The field a glyph sits on, as a linear luma, for a picture of `picture`
	// over a background of `bg`. The renderer's own blend in one number, so the
	// derived text colors are placed against what will really be there.
	pub fn field(self, picture: f32, bg: f32) -> f32 {
		if !self.perceptual {
			return bg + (picture - bg) * self.amount;
		}
		let p = |x: f32| x.max(0.0).powf(1.0 / MIX_GAMMA);
		let mixed = p(bg) + (p(picture) - p(bg)) * self.amount;
		mixed.max(0.0).powf(MIX_GAMMA)
	}
}

// How the wallpaper is drawn, for a slider reading `slider`.
pub fn wallpaper_mix(s: &Settings, slider: f32) -> Mix {
	if dark(s) {
		return Mix {
			amount: slider,
			perceptual: false,
		};
	}
	Mix {
		amount: encoded_scale(slider, paired_dark_luma(s)).clamp(0.0, 1.0),
		perceptual: true,
	}
}

// How much of the asked-for halo alpha is actually drawn, for a wallpaper whose
// slider reads `slider`. 0 means no picture, which leaves the halo alone - it is
// then sitting on the background color it is made of, and invisible either way.
pub fn halo_gain(s: &Settings, slider: f32) -> f32 {
	if dark(s) || slider <= 0.0 {
		return 1.0;
	}
	gain_for(slider, config::luma(s.bg), paired_dark_luma(s))
}

// Light mode's share of the halo alpha, given the two backgrounds. Both modes
// are measured at their own worst case: the halo is the background color, and
// the field under it is the picture as that mode draws it.
fn gain_for(slider: f32, light: f32, dark: f32) -> f32 {
	let shown = Mix {
		amount: encoded_scale(slider, dark).clamp(0.0, 1.0),
		perceptual: true,
	};
	let drawn = Mix {
		amount: slider,
		perceptual: false,
	};
	let field_dark = drawn.field(WALLPAPER_LUMA, dark);
	let field_light = shown.field(WALLPAPER_LUMA, light);
	let want = shift(field_dark, dark, HALO_REF).abs();
	(alpha_for(field_light, light, want) / HALO_REF).clamp(MIN_HALO_GAIN, 1.0)
}

#[cfg(test)]
mod tests {
	use super::{
		HALO_REF, MIN_HALO_GAIN, MIX_GAMMA, Mix, WALLPAPER_LUMA, encoded_scale, gain_for,
		halo_gain, shift, wallpaper_mix,
	};
	use crate::config::{self, Settings};

	fn themed(name: &str, mode: &str) -> Settings {
		let pal = crate::theme::resolve(name, mode, true);
		Settings {
			theme: name.to_string(),
			theme_mode: mode.to_string(),
			bg: pal.bg,
			fg: pal.fg,
			cursor: pal.cursor,
			..Settings::default()
		}
	}

	fn bg_luma(name: &str, mode: &str) -> f32 {
		config::luma(crate::theme::resolve(name, mode, true).bg)
	}

	const SLIDERS: [f32; 7] = [0.0, 0.05, 0.1, 0.25, 0.5, 0.8, 1.0];
	// a picture's dark and bright ends, as linear luma
	const LO: f32 = 0.02;
	const HI: f32 = 0.45;

	#[test]
	fn dark_mode_gets_back_exactly_what_it_handed_over() {
		for name in crate::theme::names() {
			// "system" resolves through the OS bit, which the tests leave dark
			for mode in ["dark", "system"] {
				let s = themed(name, mode);
				for v in SLIDERS {
					let mix = wallpaper_mix(&s, v);
					assert_eq!(mix.amount, v, "{name} {mode} {v}");
					assert!(!mix.perceptual, "{name} {mode} {v}");
					assert_eq!(halo_gain(&s, v), 1.0, "{name} {mode} {v}");
				}
			}
		}
	}

	// The whole reason the mix is a pure power curve rather than sRGB's own. Over
	// a black background the two are the same arithmetic, so a dark theme could
	// take either path and draw the same pixels.
	#[test]
	fn over_black_the_mix_is_the_blend_it_replaces() {
		for v in SLIDERS {
			let blend = Mix {
				amount: v,
				perceptual: false,
			};
			let curve = Mix {
				amount: v.powf(1.0 / MIX_GAMMA),
				perceptual: true,
			};
			for p in [0.0f32, 0.01, 0.13, 0.5, 1.0] {
				let (a, b) = (blend.field(p, 0.0), curve.field(p, 0.0));
				assert!((a - b).abs() < 1e-5, "v {v}, picture {p}: {a} against {b}");
			}
		}
	}

	// What the slider means, in both modes: this much of the picture's own
	// contrast reaches the screen.
	fn contrast_on_screen(mix: Mix, bg: f32) -> f32 {
		config::from_linear(mix.field(HI, bg)) - config::from_linear(mix.field(LO, bg))
	}

	#[test]
	fn light_mode_shows_the_contrast_dark_mode_shows() {
		for name in crate::theme::names() {
			let (dark, light) = (bg_luma(name, "dark"), bg_luma(name, "light"));
			for v in SLIDERS {
				let in_dark = contrast_on_screen(
					Mix {
						amount: v,
						perceptual: false,
					},
					dark,
				);
				let in_light = contrast_on_screen(wallpaper_mix(&themed(name, "light"), v), light);
				// the stand-in picture is one luma and a real one is a spread, so
				// this is close rather than exact
				assert!(
					(in_dark - in_light).abs() < 0.02,
					"{name} {v}: dark {in_dark}, light {in_light}"
				);
			}
		}
	}

	#[test]
	fn the_shipped_default_asks_for_the_scale_black_would_have_given() {
		let s = themed("SilkTerm", "light");
		let mix = wallpaper_mix(&s, 0.10);
		assert!(mix.perceptual);
		// SilkTerm's dark background is black, so the closed form is exact
		assert!(
			(mix.amount - 0.10f32.powf(1.0 / MIX_GAMMA)).abs() < 1e-4,
			"{mix:?}"
		);
	}

	// A dark theme whose background is not black already shows less picture, so
	// its light mode shows less too. Self-consistent rather than uniform.
	#[test]
	fn a_theme_with_a_lifted_dark_background_asks_for_less() {
		let silk = wallpaper_mix(&themed("SilkTerm", "light"), 0.10).amount;
		let pastel = wallpaper_mix(&themed("Pastel", "light"), 0.10).amount;
		assert!(bg_luma("Pastel", "dark") > bg_luma("SilkTerm", "dark"));
		assert!(pastel < silk - 0.05, "silk {silk}, pastel {pastel}");
	}

	#[test]
	fn the_scale_runs_end_to_end_and_only_upward() {
		for name in crate::theme::names() {
			let dark = bg_luma(name, "dark");
			assert_eq!(encoded_scale(0.0, dark), 0.0, "{name}");
			assert!((encoded_scale(1.0, dark) - 1.0).abs() < 1e-5, "{name}");
			let mut last = 0.0;
			for i in 0..=100 {
				let g = encoded_scale(i as f32 / 100.0, dark);
				assert!(g >= last - 1e-6, "{name} at {i}: {g} after {last}");
				last = g;
			}
		}
	}

	#[test]
	fn the_halo_is_quietened_wherever_a_picture_is_up() {
		let s = themed("SilkTerm", "light");
		for v in [0.05f32, 0.1, 0.35, 0.75, 1.0] {
			let g = halo_gain(&s, v);
			assert!((MIN_HALO_GAIN..1.0).contains(&g), "{v}: {g}");
		}
	}

	#[test]
	fn the_halo_covers_the_same_ground_in_both_modes() {
		let name = "SilkTerm";
		let (dark, light) = (bg_luma(name, "dark"), bg_luma(name, "light"));
		for v in [0.05f32, 0.1, 0.25, 0.5] {
			let gain = gain_for(v, light, dark);
			let field_dark = Mix {
				amount: v,
				perceptual: false,
			}
			.field(WALLPAPER_LUMA, dark);
			let field_light = wallpaper_mix(&themed(name, "light"), v).field(WALLPAPER_LUMA, light);
			let want = shift(field_dark, dark, HALO_REF).abs();
			let got = shift(field_light, light, HALO_REF * gain).abs();
			assert!(
				(got - want).abs() < 0.01 || gain <= MIN_HALO_GAIN,
				"{v}: wanted {want}, got {got}"
			);
		}
	}

	#[test]
	fn no_picture_leaves_the_halo_alone() {
		assert_eq!(halo_gain(&themed("SilkTerm", "light"), 0.0), 1.0);
	}
}
