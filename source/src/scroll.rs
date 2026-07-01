// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

use crate::config;

// Fractional scroll overlay. The crate's grid only knows integer line
// offsets; everything sub-line lives here.
//
// `target`/`visual` are measured in lines of scrollback from the bottom
// (0.0 == following new output). Each frame the grid is snapped to
// `visual.floor()` and the renderer translates by the fractional part.
//
// Dynamic-speed output scroll: `scroll_tau_ms` ("Initial scroll speed") is the
// slow, smooth ease used for sporadic output. When output bursts, the visual
// backlog grows; the ease then ramps faster (down to MIN_TAU_MS) so it keeps up,
// and eases back to the slow speed once output stops. The speed change is itself
// smoothed (ramping up responsively, back down gently) so it never jumps. The
// ramp applies only while following the bottom - wheel/scrollback navigation
// keeps the plain configured ease.
pub const MAX_BACKLOG: f32 = 16.0; // cap on how far behind the bottom output may lag
const MIN_TAU_MS: f32 = 8.0; // fastest catch-up tau (at full ramp)
const RAMP_UP_MS: f32 = 90.0; // speeding up is responsive
const RAMP_DOWN_MS: f32 = 450.0; // returning to the smooth speed is gentle

pub struct Scroll {
	target: f32,
	visual: f32,
	max: f32,
	ramp: f32, // 0 = initial/smooth speed, 1 = full fast catch-up (smoothed)
}

impl Scroll {
	pub fn new() -> Self {
		Self {
			target: 0.0,
			visual: 0.0,
			max: 0.0,
			ramp: 0.0,
		}
	}

	pub fn set_max(&mut self, history_lines: f32) {
		self.max = history_lines.max(0.0);
		self.target = self.target.clamp(0.0, self.max);
		let over = config::settings().output_ease_lines.max(MAX_BACKLOG);
		self.visual = self.visual.clamp(0.0, self.max + over);
	}

	pub fn following(&self) -> bool {
		self.target <= config::SETTLE_EPS
	}

	pub fn wheel(&mut self, lines: f32) {
		self.target = (self.target + lines).clamp(0.0, self.max);
	}

	pub fn jump_bottom(&mut self) {
		self.target = 0.0;
	}

	// New output grew the scrollback by `grown` lines while following the bottom:
	// accumulate it into the visual backlog (capped) so a fast burst lags and the
	// ramp scrolls through it. Sporadic output stays at ~output_ease_lines and
	// eases in at the slow speed.
	pub fn nudge_output(&mut self, grown: f32) {
		if self.following() {
			let floor = config::settings().output_ease_lines.max(0.0);
			self.visual = (self.visual + grown).clamp(floor, MAX_BACKLOG);
		}
	}

	pub fn advance(&mut self, dt_s: f32) {
		let init = config::settings().scroll_tau_ms;
		// ramp target from the output backlog (only while following); 0 below the
		// normal slide distance, 1 at the cap. Wheel/scrollback uses the plain ease.
		let raw = if self.following() {
			let lo = config::settings().output_ease_lines.max(0.5);
			((self.visual - lo) / (MAX_BACKLOG - lo)).clamp(0.0, 1.0)
		} else {
			0.0
		};
		let ramp_ms = if raw > self.ramp {
			RAMP_UP_MS
		} else {
			RAMP_DOWN_MS
		};
		self.ramp += (raw - self.ramp) * (1.0 - (-dt_s * 1000.0 / ramp_ms).exp());

		// effective tau: the configured "initial" speed at ramp 0, MIN_TAU at ramp 1
		let tau = (init + (MIN_TAU_MS - init) * self.ramp).max(1.0);
		let k = 1.0 - (-dt_s * 1000.0 / tau).exp();
		self.visual += (self.target - self.visual) * k;
		if (self.target - self.visual).abs() < config::SETTLE_EPS {
			self.visual = self.target;
			self.ramp = 0.0;
		}
	}

	// whole-line scrollback position the grid should snap to
	pub fn desired_offset(&self) -> usize {
		self.visual.floor().max(0.0) as usize
	}

	// sub-line remainder in [0,1)
	pub fn frac(&self) -> f32 {
		self.visual - self.visual.floor()
	}

	pub fn animating(&self) -> bool {
		(self.target - self.visual).abs() > config::SETTLE_EPS
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// settings() falls back to Settings::default() when no config file is
	// loaded, so these run against the shipped defaults.
	fn ease_lines() -> f32 {
		config::settings().output_ease_lines.max(0.0)
	}

	#[test]
	fn starts_following() {
		let s = Scroll::new();
		assert!(s.following());
		assert!(!s.animating());
		assert_eq!(s.desired_offset(), 0);
	}

	#[test]
	fn wheel_clamps_to_history() {
		let mut s = Scroll::new();
		s.set_max(10.0);
		s.wheel(25.0);
		assert!(!s.following());
		// target is private; observe via advance converging onto max
		for _ in 0..2000 {
			s.advance(0.016);
		}
		assert_eq!(s.desired_offset(), 10);
		s.jump_bottom();
		for _ in 0..2000 {
			s.advance(0.016);
		}
		assert!(s.following());
		assert_eq!(s.desired_offset(), 0);
		assert!(s.frac().abs() < 1e-3);
	}

	#[test]
	fn nudge_accumulates_and_caps() {
		let mut s = Scroll::new();
		s.set_max(1000.0);
		s.nudge_output(1.0);
		let after_one = s.frac() + s.desired_offset() as f32;
		assert!(after_one >= ease_lines().min(1.0) - 1e-3);
		// a burst may lag at most MAX_BACKLOG lines
		for _ in 0..100 {
			s.nudge_output(5.0);
		}
		assert!(s.desired_offset() as f32 + s.frac() <= MAX_BACKLOG + 1e-3);
	}

	#[test]
	fn nudge_ignored_when_scrolled_back() {
		let mut s = Scroll::new();
		s.set_max(100.0);
		s.wheel(50.0);
		let before = s.desired_offset() as f32 + s.frac();
		s.nudge_output(10.0);
		let after = s.desired_offset() as f32 + s.frac();
		assert_eq!(before, after); // no-snap rule: output must not move a reader
	}

	#[test]
	fn output_backlog_settles_to_bottom() {
		let mut s = Scroll::new();
		s.set_max(1000.0);
		for _ in 0..10 {
			s.nudge_output(3.0);
		}
		assert!(s.animating());
		for _ in 0..2000 {
			s.advance(0.016);
		}
		// eased all the way back down to following the live bottom
		assert!(s.following());
		assert_eq!(s.desired_offset(), 0);
		assert!(s.frac().abs() < 1e-3);
	}

	#[test]
	fn burst_ramps_faster_than_trickle() {
		// a deep backlog must converge measurably faster than the plain ease
		// (the dynamic-speed ramp) - compare lines cleared in the same time
		let mut burst = Scroll::new();
		burst.set_max(1000.0);
		for _ in 0..10 {
			burst.nudge_output(5.0); // deep backlog -> full ramp
		}
		let start_b = burst.desired_offset() as f32 + burst.frac();
		let mut trickle = Scroll::new();
		trickle.set_max(1000.0);
		trickle.nudge_output(0.9); // below the ramp threshold
		let start_t = trickle.desired_offset() as f32 + trickle.frac();
		for _ in 0..12 {
			burst.advance(0.016);
			trickle.advance(0.016);
		}
		let cleared_b = (start_b - (burst.desired_offset() as f32 + burst.frac())) / start_b;
		let cleared_t = (start_t - (trickle.desired_offset() as f32 + trickle.frac())) / start_t;
		assert!(
			cleared_b > cleared_t,
			"burst {cleared_b} should clear proportionally faster than trickle {cleared_t}"
		);
	}

	#[test]
	fn set_max_clamps_positions() {
		let mut s = Scroll::new();
		s.set_max(100.0);
		s.wheel(80.0);
		for _ in 0..2000 {
			s.advance(0.016);
		}
		assert_eq!(s.desired_offset(), 80);
		// history shrank (e.g. clear/reset): both target and visual clamp
		s.set_max(5.0);
		for _ in 0..2000 {
			s.advance(0.016);
		}
		assert!(s.desired_offset() <= 5);
	}
}
