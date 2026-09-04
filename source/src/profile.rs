// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

//! Performance profiles: one setting that decides how much the look is allowed
//! to cost, and the rating that picks it on a machine that cannot keep up.
//!
//! A profile sits ON TOP of the stored settings rather than in them. The file
//! and the dialog keep the user's own values; `apply` overwrites the fields a
//! profile governs when settings go live and keeps the originals in a shadow,
//! so any code that reads the live settings and writes them back cannot leak
//! a profile's value into the file. Choosing Custom is then just a profile
//! that governs nothing.
//!
//! The rating watches how a scroll ease is paced. A display that keeps its
//! refresh rate paces one frame per refresh; one that cannot stretches every
//! frame, and a run of stretched frames steps the profile down. It never steps
//! up on the same hardware: a lighter profile renders less, so a fast-looking
//! run under it says nothing about the heavier one.

use crate::config::Settings;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Profile {
	Custom,
	Max,
	High,
	Low,
	Standard,
}

impl Profile {
	// dialog order, which is also the order they cost in
	pub const ALL: [Profile; 5] = [
		Profile::Custom,
		Profile::Max,
		Profile::High,
		Profile::Low,
		Profile::Standard,
	];

	// the spelling the config file uses
	pub fn key(self) -> &'static str {
		match self {
			Profile::Custom => "custom",
			Profile::Max => "max",
			Profile::High => "high",
			Profile::Low => "low",
			Profile::Standard => "standard",
		}
	}

	pub fn label(self) -> &'static str {
		match self {
			Profile::Custom => "Custom",
			Profile::Max => "Max silk",
			Profile::High => "High",
			Profile::Low => "Low",
			Profile::Standard => "Standard terminal",
		}
	}

	// An unknown spelling reads as the shipped default, the way every other
	// named option in the config does.
	pub fn parse(text: &str) -> Profile {
		Profile::ALL
			.into_iter()
			.find(|p| p.key().eq_ignore_ascii_case(text.trim()))
			.unwrap_or(Profile::Max)
	}

	pub fn index(self) -> usize {
		Profile::ALL.iter().position(|p| *p == self).unwrap_or(0)
	}

	pub fn from_index(index: usize) -> Profile {
		Profile::ALL.get(index).copied().unwrap_or(Profile::Max)
	}

	// The next cheaper profile, or None at the bottom. Custom has no neighbor:
	// the user's own values are not on the ladder.
	pub fn lower(self) -> Option<Profile> {
		match self {
			Profile::Max => Some(Profile::High),
			Profile::High => Some(Profile::Low),
			Profile::Low => Some(Profile::Standard),
			Profile::Standard | Profile::Custom => None,
		}
	}
}

// The user's own values of every field a profile governs, kept beside the
// live settings while a profile is in force. `put` is the whole of "choose
// Custom and everything comes back".
#[derive(Clone, PartialEq, Debug)]
pub struct Shadow {
	scroll_smooth: bool,
	scroll_ease_in_ms: f32,
	scroll_ramp_up_ms: f32,
	scroll_single_screen_tau_ms: f32,
	scroll_ramp_down_ms: f32,
	scroll_ease_out_ms: f32,
	smooth_scroll_apps: bool,
	cursor_animation: String,
	text_scrim: bool,
	text_scrim_radius: f32,
	text_scrim_strength: f32,
	text_scrim_softness: f32,
	text_scrim_function: String,
	text_outline: f32,
	wallpaper_enabled: bool,
	wallpaper_blur: f32,
	wallpaper_contrast_mask: bool,
}

impl Shadow {
	fn of(s: &Settings) -> Shadow {
		Shadow {
			scroll_smooth: s.scroll_smooth,
			scroll_ease_in_ms: s.scroll_ease_in_ms,
			scroll_ramp_up_ms: s.scroll_ramp_up_ms,
			scroll_single_screen_tau_ms: s.scroll_single_screen_tau_ms,
			scroll_ramp_down_ms: s.scroll_ramp_down_ms,
			scroll_ease_out_ms: s.scroll_ease_out_ms,
			smooth_scroll_apps: s.smooth_scroll_apps,
			cursor_animation: s.cursor_animation.clone(),
			text_scrim: s.text_scrim,
			text_scrim_radius: s.text_scrim_radius,
			text_scrim_strength: s.text_scrim_strength,
			text_scrim_softness: s.text_scrim_softness,
			text_scrim_function: s.text_scrim_function.clone(),
			text_outline: s.text_outline,
			wallpaper_enabled: s.wallpaper_enabled,
			wallpaper_blur: s.wallpaper_blur,
			wallpaper_contrast_mask: s.wallpaper_contrast_mask,
		}
	}

	fn put(&self, s: &mut Settings) {
		s.scroll_smooth = self.scroll_smooth;
		s.scroll_ease_in_ms = self.scroll_ease_in_ms;
		s.scroll_ramp_up_ms = self.scroll_ramp_up_ms;
		s.scroll_single_screen_tau_ms = self.scroll_single_screen_tau_ms;
		s.scroll_ramp_down_ms = self.scroll_ramp_down_ms;
		s.scroll_ease_out_ms = self.scroll_ease_out_ms;
		s.smooth_scroll_apps = self.smooth_scroll_apps;
		s.cursor_animation.clone_from(&self.cursor_animation);
		s.text_scrim = self.text_scrim;
		s.text_scrim_radius = self.text_scrim_radius;
		s.text_scrim_strength = self.text_scrim_strength;
		s.text_scrim_softness = self.text_scrim_softness;
		s.text_scrim_function.clone_from(&self.text_scrim_function);
		s.text_outline = self.text_outline;
		s.wallpaper_enabled = self.wallpaper_enabled;
		s.wallpaper_blur = self.wallpaper_blur;
		s.wallpaper_contrast_mask = self.wallpaper_contrast_mask;
	}
}

// Put the user's own values back. Safe on settings that carry no profile.
pub fn unapply(s: &mut Settings) {
	if let Some(shadow) = s.profile_shadow.take() {
		shadow.put(s);
	}
}

// Overwrite the governed fields with the profile's, keeping the user's values
// in the shadow. Idempotent: a live copy that already carries a profile is
// unwound first, so a changed profile field is honored rather than stacked.
pub fn apply(s: &mut Settings) {
	unapply(s);
	let profile = Profile::parse(&s.performance_profile);
	if profile == Profile::Custom {
		return;
	}
	let shadow = Shadow::of(s);
	values(profile, s);
	s.profile_shadow = Some(Box::new(shadow));
}

pub fn current(s: &Settings) -> Profile {
	Profile::parse(&s.performance_profile)
}

// What each profile sets. Every profile starts from the shipped defaults, so
// Max is exactly "the defaults for everything" and the others name only what
// they change.
fn values(profile: Profile, s: &mut Settings) {
	let defaults = Settings::default();
	Shadow::of(&defaults).put(s);
	match profile {
		Profile::Custom | Profile::Max => {}
		Profile::High => quicker(s),
		Profile::Low => {
			quicker(s);
			s.cursor_animation = "none".to_string();
			// the outline is drawn by the scrim pass, so the halo stays on and
			// shrinks to the one pixel the outline covers
			s.text_scrim_radius = 1.0;
			s.text_scrim_strength = 0.0;
			s.wallpaper_enabled = false;
			s.wallpaper_blur = 0.0;
			s.wallpaper_contrast_mask = false;
		}
		Profile::Standard => {
			s.scroll_smooth = false;
			s.smooth_scroll_apps = false;
			s.cursor_animation = "none".to_string();
			s.text_scrim = false;
			s.text_outline = 0.0;
			s.wallpaper_enabled = false;
			s.wallpaper_blur = 0.0;
			s.wallpaper_contrast_mask = false;
		}
	}
}

// Shorter eases on the three stretches a slow display shows most, and a halo
// that costs fewer taps: the square metric with a smaller reach.
fn quicker(s: &mut Settings) {
	s.scroll_ease_in_ms /= 2.0;
	s.scroll_ease_out_ms /= 2.0;
	s.scroll_single_screen_tau_ms /= 2.0;
	s.text_scrim_function = "dilate".to_string();
	s.text_scrim_radius = 3.0;
}

// Names the adapter closely enough that a new card or a switch to software
// rendering reads as new hardware, and a driver update does not.
pub fn fingerprint(info: &wgpu::AdapterInfo) -> String {
	format!(
		"{} ({:?}, {:?})",
		info.name.trim(),
		info.device_type,
		info.backend
	)
}

// Where a machine starts before anything has been measured.
pub fn first_pick(info: &wgpu::AdapterInfo) -> Profile {
	match info.device_type {
		wgpu::DeviceType::Cpu => Profile::Low,
		_ => Profile::Max,
	}
}

// Frames an ease has to pace before its median says anything.
pub const WINDOW: usize = 48;

// How far past the refresh period a frame may run before it counts as a miss:
// half again, so the occasional stretched frame of a busy desktop passes and
// a display dropping every third frame does not.
pub fn budget_ms(refresh_hz: f32) -> f32 {
	1000.0 / refresh_hz.max(1.0) * 1.5
}

pub struct Rating {
	periods: Vec<f32>,
	last: Option<Instant>,
}

impl Rating {
	pub fn new() -> Rating {
		Rating {
			periods: Vec::with_capacity(WINDOW),
			last: None,
		}
	}

	// A frame just went out while an ease was running. Only the gap to the
	// previous such frame is a period; the first after a pause is a start.
	pub fn note(&mut self, now: Instant) {
		if let Some(last) = self.last {
			self.periods.push((now - last).as_secs_f32() * 1000.0);
		}
		self.last = Some(now);
	}

	// The ease stopped, so the next frame's gap means nothing.
	pub fn pause(&mut self) {
		self.last = None;
	}

	// Start over, with nothing measured - the profile changed, so what was
	// measured was measured under another workload.
	pub fn reset(&mut self) {
		self.periods.clear();
		self.last = None;
	}

	// Once a window is full: did the display miss its budget? Empties the
	// window either way, so a verdict is one window's worth of evidence.
	pub fn verdict(&mut self, budget_ms: f32) -> Option<bool> {
		if self.periods.len() < WINDOW {
			return None;
		}
		let over = median(&mut self.periods) > budget_ms;
		self.periods.clear();
		Some(over)
	}
}

fn median(values: &mut [f32]) -> f32 {
	values.sort_by(f32::total_cmp);
	values[values.len() / 2]
}

#[cfg(test)]
mod tests {
	use super::{Profile, Rating, WINDOW, apply, budget_ms, unapply};
	use crate::config::Settings;
	use std::time::{Duration, Instant};

	fn tuned() -> Settings {
		Settings {
			scroll_ease_in_ms: 300.0,
			scroll_smooth: false,
			cursor_animation: "phase".to_string(),
			text_scrim_radius: 9.0,
			wallpaper_enabled: false,
			..Settings::default()
		}
	}

	#[test]
	fn a_profile_masks_the_stored_values_and_custom_puts_them_back() {
		let mut s = tuned();
		s.performance_profile = "max".to_string();
		apply(&mut s);
		assert!(s.scroll_smooth, "Max is the shipped default");
		assert_eq!(s.scroll_ease_in_ms, Settings::default().scroll_ease_in_ms);
		assert_eq!(s.cursor_animation, "pulse_vertical");
		assert!(s.wallpaper_enabled);

		s.performance_profile = "custom".to_string();
		apply(&mut s);
		assert!(!s.scroll_smooth);
		assert_eq!(s.scroll_ease_in_ms, 300.0);
		assert_eq!(s.cursor_animation, "phase");
		assert_eq!(s.text_scrim_radius, 9.0);
		assert!(!s.wallpaper_enabled);
		assert!(s.profile_shadow.is_none());
	}

	#[test]
	fn applying_twice_does_not_stack() {
		let mut s = tuned();
		s.performance_profile = "low".to_string();
		apply(&mut s);
		s.performance_profile = "high".to_string();
		apply(&mut s);
		assert!(s.wallpaper_enabled, "High keeps the wallpaper");
		unapply(&mut s);
		assert_eq!(s.scroll_ease_in_ms, 300.0, "the user's value, not Low's");
		assert!(!s.wallpaper_enabled, "the user's value, not High's");
	}

	#[test]
	fn each_profile_costs_less_than_the_one_above() {
		let mut s = Settings::default();
		let mut radius = f32::MAX;
		for profile in [Profile::Max, Profile::High, Profile::Low] {
			s.performance_profile = profile.key().to_string();
			apply(&mut s);
			assert!(s.scroll_smooth);
			assert!(s.text_scrim);
			assert!(s.text_scrim_radius <= radius);
			radius = s.text_scrim_radius;
		}
		assert_eq!(s.cursor_animation, "none");
		assert!(!s.wallpaper_enabled);
		assert!(s.text_outline > 0.0, "Low keeps the outline");
		s.performance_profile = "standard".to_string();
		apply(&mut s);
		assert!(!s.scroll_smooth);
		assert!(!s.smooth_scroll_apps);
		assert!(!s.text_scrim);
		assert_eq!(s.text_outline, 0.0);
	}

	#[test]
	fn the_ladder_ends_at_standard_and_custom_is_off_it() {
		assert_eq!(Profile::Max.lower(), Some(Profile::High));
		assert_eq!(Profile::Standard.lower(), None);
		assert_eq!(Profile::Custom.lower(), None);
		assert_eq!(Profile::parse("LOW"), Profile::Low);
		assert_eq!(
			Profile::parse("silky"),
			Profile::Max,
			"unknown reads as the default"
		);
		for p in Profile::ALL {
			assert_eq!(Profile::from_index(p.index()), p);
		}
	}

	#[test]
	fn a_window_of_stretched_frames_is_a_miss_and_a_pause_breaks_the_chain() {
		let budget = budget_ms(60.0);
		let mut r = Rating::new();
		let mut t = Instant::now();
		for _ in 0..=WINDOW {
			r.note(t);
			t += Duration::from_millis(16);
		}
		assert_eq!(r.verdict(budget), Some(false));
		assert_eq!(r.verdict(budget), None, "the window was spent");
		// a long gap while paused is not a period
		r.pause();
		t += Duration::from_secs(5);
		for _ in 0..=WINDOW {
			r.note(t);
			t += Duration::from_millis(30);
		}
		assert_eq!(r.verdict(budget), Some(true));
	}
}
