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
//! - **Wallpaper visibility.** At 10% a picture is plainly there over black and
//!   all but gone over white. The slider keeps its number and light mode raises
//!   the alpha behind it. How far is a judgement, because the two ways of
//!   reading "as much picture" disagree: how far the composite sits from the
//!   background, and how much of the picture's own texture survives. Matching
//!   the first alone puts the picture there and leaves it flat; matching the
//!   second brings the detail back and takes the background to a mid gray.
//!   `PRESENCE` picks the point between.
//! - **Text scrim.** The halo is the background color laid over whatever the
//!   picture put there, so in light mode it is a pale plate on a darkened
//!   field, which is the same move in the direction the eye notices most. Its
//!   alpha is scaled down until it covers the same ground dark mode's does.
//!
//! The measure throughout is the sRGB transfer curve taken on Rec.709 luma. Luma
//! because it is affine under the alpha composite, so one number stands in for a
//! whole blend; the transfer curve because it tracks CIE L* closely enough here
//! (the two disagree by two points of alpha on the shipped defaults) and it is
//! already the program's color language.

use crate::config::{self, Settings};

// A stand-in for the picture, as a linear luma. Measured over the shipped pack
// of 104: median 0.129, mean 0.141, quartiles 0.062 and 0.193. One number for
// every image rather than each image's own, so a rotation folder does not change
// the alpha under the user every few minutes - and a picture far from this one
// is off by a few points, not by a factor.
const WALLPAPER_LUMA: f32 = 0.13;

// How the two readings of "as much picture" are weighed: 0 matches how far the
// composite sits from the background, 1 matches how much of the picture's own
// texture survives. Measured on the rig at the shipped 10% over the built-in
// theme, against dark mode at the same setting: 0 leaves a picture that is there
// but flat, 1 gives back dark mode's detail over a mid-gray background.
const PRESENCE: f32 = 0.5;

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

// How much of the picture's own texture survives, at this alpha over this
// background. The alpha scales the picture's range in linear light, and the
// transfer curve's slope where the composite sits decides how much of that the
// eye gets back - which is why a picture laid over white goes flat while the
// same picture over black keeps its detail.
fn texture(alpha: f32, bg: f32) -> f32 {
	let at = alpha * WALLPAPER_LUMA + (1.0 - alpha) * bg;
	let slope = if at <= 0.003_130_8 {
		12.92
	} else {
		(1.055 / 2.4) * at.powf(1.0 / 2.4 - 1.0)
	};
	alpha * slope
}

// The alpha at which the picture keeps `want` of its texture over `bg`. Rises
// with the alpha either way round, so it bisects.
fn alpha_for_texture(bg: f32, want: f32) -> f32 {
	if texture(1.0, bg) <= want {
		return 1.0;
	}
	let (mut lo, mut hi) = (0.0f32, 1.0f32);
	for _ in 0..20 {
		let mid = 0.5 * (lo + hi);
		if texture(mid, bg) < want {
			lo = mid;
		} else {
			hi = mid;
		}
	}
	0.5 * (lo + hi)
}

// The alpha the wallpaper quad is drawn at, for a slider reading `slider`.
pub fn wallpaper_alpha(s: &Settings, slider: f32) -> f32 {
	if dark(s) {
		return slider;
	}
	opacity_for(slider, config::luma(s.bg), paired_dark_luma(s))
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

// Light mode's alpha for a slider reading `slider`, given the two backgrounds.
//
// Two things decide how much picture is there, and they disagree. Matching how
// far the composite sits from the background leaves light mode flat: the picture
// is present and its detail is gone. Matching how much of the picture's own
// texture survives brings the detail back and takes the background to a mid
// gray, which stops it being light mode. `PRESENCE` picks the point between.
//
// Never below the slider itself: the correction is there to show more picture,
// and near the top of the slider it has nothing left to add - at 100% the
// picture has replaced the background in both modes and there is nothing to
// match.
fn opacity_for(slider: f32, light: f32, dark: f32) -> f32 {
	let by_offset = alpha_for(
		light,
		WALLPAPER_LUMA,
		shift(dark, WALLPAPER_LUMA, slider).abs(),
	);
	let by_texture = alpha_for_texture(light, texture(slider, dark));
	(by_offset + (by_texture - by_offset) * PRESENCE).max(slider)
}

// Light mode's share of the halo alpha, given the two backgrounds. Both modes
// are measured at their own worst case: the halo is the background color, and
// the field under it is the picture at whatever alpha that mode draws it.
fn gain_for(slider: f32, light: f32, dark: f32) -> f32 {
	let shown = opacity_for(slider, light, dark);
	let field_dark = dark + (WALLPAPER_LUMA - dark) * slider;
	let field_light = light + (WALLPAPER_LUMA - light) * shown;
	let want = shift(field_dark, dark, HALO_REF).abs();
	(alpha_for(field_light, light, want) / HALO_REF).clamp(MIN_HALO_GAIN, 1.0)
}

#[cfg(test)]
mod tests {
	use super::{
		HALO_REF, MIN_HALO_GAIN, PRESENCE, WALLPAPER_LUMA, alpha_for, alpha_for_texture, gain_for,
		halo_gain, opacity_for, shift, texture, wallpaper_alpha,
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

	const SLIDERS: [f32; 7] = [0.0, 0.05, 0.1, 0.25, 0.5, 0.8, 1.0];

	#[test]
	fn dark_mode_gets_back_exactly_what_it_handed_over() {
		for name in crate::theme::names() {
			// "system" resolves through the OS bit, which the tests leave dark
			for mode in ["dark", "system"] {
				let s = themed(name, mode);
				for v in SLIDERS {
					assert_eq!(wallpaper_alpha(&s, v), v, "{name} {mode} {v}");
					assert_eq!(halo_gain(&s, v), 1.0, "{name} {mode} {v}");
				}
			}
		}
	}

	#[test]
	fn light_mode_sits_between_the_two_readings_of_as_much_picture() {
		for name in crate::theme::names() {
			let dark = config::luma(crate::theme::resolve(name, "dark", true).bg);
			let light = config::luma(crate::theme::resolve(name, "light", true).bg);
			for v in SLIDERS {
				let by_offset =
					alpha_for(light, WALLPAPER_LUMA, shift(dark, WALLPAPER_LUMA, v).abs());
				let by_texture = alpha_for_texture(light, texture(v, dark));
				// texture always asks for more, which is what makes PRESENCE a choice
				// rather than a rounding
				assert!(by_texture >= by_offset - 1e-6, "{name} {v}");
				let want = by_offset + (by_texture - by_offset) * PRESENCE;
				let got = opacity_for(v, light, dark);
				assert!(
					(got - want.max(v)).abs() < 1e-5,
					"{name} {v}: wanted {want}, got {got}"
				);
			}
		}
	}

	// Each invariant on its own, so a change to either is caught where it happens
	// rather than only through the blend.
	#[test]
	fn each_reading_hits_what_it_aims_at() {
		let dark = config::luma(crate::theme::resolve("SilkTerm", "dark", true).bg);
		let light = config::luma(crate::theme::resolve("SilkTerm", "light", true).bg);
		for v in [0.05f32, 0.1, 0.25, 0.5] {
			let by_offset = alpha_for(light, WALLPAPER_LUMA, shift(dark, WALLPAPER_LUMA, v).abs());
			assert!(
				(shift(light, WALLPAPER_LUMA, by_offset).abs()
					- shift(dark, WALLPAPER_LUMA, v).abs())
				.abs() < 0.005,
				"offset {v}"
			);
			let by_texture = alpha_for_texture(light, texture(v, dark));
			assert!(
				(texture(by_texture, light) - texture(v, dark)).abs() < 0.01,
				"texture {v}"
			);
		}
	}

	#[test]
	fn the_shipped_default_is_plainly_there_in_light_mode() {
		let s = themed("SilkTerm", "light");
		let a = wallpaper_alpha(&s, 0.10);
		// half, near enough - the number is measured rather than chosen, so the
		// band is what a change of constant is allowed to move
		assert!((0.45..0.55).contains(&a), "{a}");
	}

	#[test]
	fn light_mode_never_shows_less_than_the_slider_asked() {
		for name in crate::theme::names() {
			let s = themed(name, "light");
			let mut last = 0.0;
			for i in 0..=100 {
				let v = i as f32 / 100.0;
				let a = wallpaper_alpha(&s, v);
				assert!(a >= v - 1e-6, "{name} {v}: {a}");
				assert!(a >= last - 1e-6, "{name} {v}: {a} after {last}");
				assert!(a <= 1.0, "{name} {v}: {a}");
				last = a;
			}
		}
	}

	#[test]
	fn the_halo_is_quietened_wherever_a_picture_is_up() {
		let s = themed("SilkTerm", "light");
		for v in [0.05f32, 0.1, 0.35, 0.75] {
			let g = halo_gain(&s, v);
			// about a third of the asked-for alpha, which is a doubling and a half
			assert!((0.30..0.42).contains(&g), "{v}: {g}");
		}
		// with the picture at full strength the background is gone and the plate
		// has the furthest to travel, so the floor is what holds it
		assert_eq!(halo_gain(&s, 1.0), MIN_HALO_GAIN);
	}

	#[test]
	fn the_halo_covers_the_same_ground_in_both_modes() {
		let name = "SilkTerm";
		let dark = config::luma(crate::theme::resolve(name, "dark", true).bg);
		let light = config::luma(crate::theme::resolve(name, "light", true).bg);
		for v in [0.05f32, 0.1, 0.25, 0.5] {
			let gain = gain_for(v, light, dark);
			let shown = opacity_for(v, light, dark);
			let field_dark = dark + (WALLPAPER_LUMA - dark) * v;
			let field_light = light + (WALLPAPER_LUMA - light) * shown;
			let want = shift(field_dark, dark, HALO_REF).abs();
			let got = shift(field_light, light, HALO_REF * gain).abs();
			assert!((got - want).abs() < 0.01, "{v}: wanted {want}, got {got}");
		}
	}

	#[test]
	fn no_picture_leaves_the_halo_alone() {
		let s = themed("SilkTerm", "light");
		assert_eq!(halo_gain(&s, 0.0), 1.0);
	}
}
