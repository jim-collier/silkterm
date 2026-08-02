// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

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
// backlog grows and the ease ramps faster so it keeps up, easing back to the
// slow speed once output stops. The speed change is itself smoothed (ramping up
// responsively, back down gently) so it never jumps. The ramp applies only
// while following the bottom - wheel/scrollback navigation keeps the plain
// configured ease. The ramp's top speed depends on the burst: while its first
// line is still on screen (a short listing) it tops out at the gentler
// `scroll_inview_tau_ms` and builds slowly; once that line has scrolled off the
// top, full catch-up (down to MIN_TAU_MS).
//
// The ease itself is asymmetric: motion builds from rest (a two-stage cascade -
// `visual` chases a leading `mid` stage, so the first frames are gentle instead
// of jumping straight to peak speed), and the stop is sharpened by a minimum
// closing speed inside STOP_BAND (a bare exponential would crawl the last few
// pixels in over a second).
pub const MAX_BACKLOG: f32 = 16.0; // cap on how far behind the bottom output may lag
const MIN_TAU_MS: f32 = 8.0; // fastest catch-up tau (at full ramp)
const RAMP_UP_MS: f32 = 90.0; // speeding up is responsive
const RAMP_DOWN_MS: f32 = 450.0; // returning to the smooth speed is gentle
// While a burst's own first line is still on screen (a short listing), catch-up
// tops out at the gentler configured inview tau and the ramp builds more slowly -
// brisk, but never the full-throttle chase. Once the burst has scrolled its top
// line off (advanced a screenful), the profile above takes over and keeps up.
const INVIEW_RAMP_UP_MS: f32 = 260.0;
// Ease-in: `visual` chases the leading `mid` stage over this fraction of the
// effective tau, so a fresh motion builds from zero speed instead of spiking on
// its first frame. Scales with tau, so full-throttle catch-up stays fast.
const ATTACK_FRACT: f32 = 0.35;
// Sharper stop: within this many lines of the target the exponential tail is
// replaced by a glide of at least STOP_MIN_LPS lines/s, so the last few pixels
// sweep in instead of crawling. Above the band the ease-out is untouched.
const STOP_BAND: f32 = 0.4;
const STOP_MIN_LPS: f32 = 3.0;
// Alt-screen app-scroll easing: a full-screen app (less, vim, muffer, ...) owns
// its screen and scrolls by repainting whole lines. `app_off` is a transient
// visual offset (in lines, signed: + shifts content down) set the moment such a
// repaint is detected, then eased to 0 so the new frame slides into place. The
// revealed strip is filled from the retained previous frame (see pane.rs), so the
// cap is the detector's max per-step shift, not a bg-fill budget. Kept in step
// with pane.rs APP_SCROLL_MAX; wheel notches in a mouse-tracking app repaint a
// bigger jump than line-by-line output, so the window has to be generous or the
// wheel just hard-cuts.
const APP_OFF_CAP: f32 = 24.0;
// Alt-scroll lag control: below APP_LAG_SOFT lines the slide eases at the smooth
// configured tau; from there to APP_LAG_HARD the ease ramps toward MIN_APP_TAU so
// a fast burst can't lag far enough to open a blank reveal strip (one retained
// frame fills only ~one line back).
const APP_LAG_SOFT: f32 = 1.2;
const APP_LAG_HARD: f32 = 4.0;
const MIN_APP_TAU_MS: f32 = 22.0;

pub struct Scroll {
	target: f32,
	visual: f32,
	mid: f32, // cascade stage between target and visual: gives the ease its ease-in
	max: f32,
	ramp: f32,      // 0 = initial/smooth speed, 1 = full fast catch-up (smoothed)
	app_off: f32,   // alt-screen slide offset, eased toward 0 (see APP_OFF_CAP)
	burst: f32,     // lines this output burst has advanced (resets once settled at rest)
	overflow: bool, // burst topped a screenful: its first line is off - full catch-up
}

impl Scroll {
	pub fn new() -> Self {
		Self {
			target: 0.0,
			visual: 0.0,
			mid: 0.0,
			max: 0.0,
			ramp: 0.0,
			app_off: 0.0,
			burst: 0.0,
			overflow: false,
		}
	}

	// An alt-screen app repainted shifted by `lines` (signed). Sets (not stacks)
	// the slide offset: the retained previous frame is exactly one repaint back, so
	// each detected step is its own slide - a fast run just replaces the offset each
	// frame (content lags ~one step, then settles), always fillable from that frame.
	pub fn app_scroll(&mut self, lines: f32) {
		self.app_off = lines.clamp(-APP_OFF_CAP, APP_OFF_CAP);
	}

	// Hard-cut any in-flight alt-screen slide. An alt-screen enter/exit is an
	// instant full-screen swap, not a scroll, so a slide left easing across it
	// would drag the wrong screen's content.
	pub fn cancel_app_scroll(&mut self) {
		self.app_off = 0.0;
	}

	// Freeze catch-up (hidden tab shown, minimized window restored): land at rest
	// instantly. Anything left easing was invisible while frozen, and the pending
	// backlog must not ease in - that is the bounce class.
	pub fn snap(&mut self) {
		self.visual = self.target;
		self.mid = self.target;
		self.ramp = 0.0;
		self.app_off = 0.0;
		self.burst = 0.0;
		self.overflow = false;
	}

	// Current alt-screen slide offset in lines (added to the render's vertical
	// offset; 0 except briefly after an app repaint-scroll).
	pub fn app_offset(&self) -> f32 {
		self.app_off
	}

	pub fn set_max(&mut self, history_lines: f32) {
		self.max = history_lines.max(0.0);
		self.target = self.target.clamp(0.0, self.max);
		let overscan = config::settings().output_ease_lines.max(MAX_BACKLOG);
		self.visual = self.visual.clamp(0.0, self.max + overscan);
		self.mid = self.mid.clamp(0.0, self.max + overscan);
	}

	pub fn following(&self) -> bool {
		self.target <= config::SETTLE_EPS
	}

	pub fn wheel(&mut self, lines: f32) {
		self.target = (self.target + lines).clamp(0.0, self.max);
	}

	// Scrollback extent in lines (0 = nothing to scroll).
	pub fn max_lines(&self) -> f32 {
		self.max
	}

	// Where the view actually sits right now, mid-ease. The scrollbar thumb rides
	// this so it tracks the content rather than the pending destination.
	pub fn visual_lines(&self) -> f32 {
		self.visual
	}

	// Where the view is headed. A thumb being DRAGGED rides this instead: the
	// handle must follow the pointer exactly (direct manipulation), while the
	// content eases in behind it.
	pub fn target_lines(&self) -> f32 {
		self.target
	}

	// Scroll to an absolute position (lines from the bottom) - scrollbar drags
	// and track clicks. Eases like every other scroll.
	pub fn scroll_to(&mut self, lines: f32) {
		self.target = lines.clamp(0.0, self.max);
	}

	pub fn jump_bottom(&mut self) {
		self.target = 0.0;
	}

	// New output grew the scrollback by `grown` lines while following the bottom:
	// accumulate it into the visual backlog (capped) so a fast burst lags and the
	// ramp scrolls through it. Sporadic output stays at ~output_ease_lines and
	// eases in at the slow speed. `view_rows` is the pane's screen height: once a
	// burst has advanced that far, its first line has provably scrolled off the
	// top (it printed at most a screen above the bottom), which switches the ease
	// from the gentle in-view profile to full catch-up.
	pub fn nudge_output(&mut self, grown: f32, view_rows: f32) {
		if self.following() {
			self.burst += grown;
			if view_rows > 0.0 && self.burst >= view_rows {
				self.overflow = true;
			}
			// resolve() clamps the config value, but stay self-defensive: a floor
			// above MAX_BACKLOG makes this clamp panic (min > max = abort in release)
			let floor = config::settings().output_ease_lines.clamp(0.0, MAX_BACKLOG);
			let before = self.visual;
			self.visual = (self.visual + grown).clamp(floor, MAX_BACKLOG);
			// the nudge is a coordinate shift (content moved under the view), so the
			// cascade stage rides along - shifting it keeps its lead over `visual`
			// intact, which is what preserves the eased speed across a burst
			self.mid = (self.mid + (self.visual - before)).clamp(0.0, self.visual);
		}
	}

	pub fn advance(&mut self, dt_s: f32) {
		// one settings() snapshot per call - this runs per pane per frame
		let cfg = config::settings();
		let init_tau_ms = cfg.scroll_tau_ms;
		// ramp target from the output backlog (only while following); 0 below the
		// normal slide distance, 1 at the cap. Wheel/scrollback uses the plain ease.
		let ramp_target = if self.following() {
			// upper bound keeps the divisor positive (at the cap it would be 0 -> NaN
			// propagating into the ramp and the visual position)
			let ease_floor = cfg.output_ease_lines.clamp(0.5, MAX_BACKLOG - 1.0);
			((self.visual - ease_floor) / (MAX_BACKLOG - ease_floor)).clamp(0.0, 1.0)
		} else {
			0.0
		};
		// Profile: an overflowed burst chases at full throttle; one still wholly on
		// screen ramps more slowly toward the gentler configured in-view tau (never
		// slower than the initial speed itself, if the user inverts the two).
		let (top_tau, ramp_up_ms) = if self.overflow {
			(MIN_TAU_MS, RAMP_UP_MS)
		} else {
			let inview = cfg
				.scroll_inview_tau_ms
				.clamp(MIN_TAU_MS, init_tau_ms.max(MIN_TAU_MS));
			(inview, INVIEW_RAMP_UP_MS)
		};
		let ramp_ms = if ramp_target > self.ramp {
			ramp_up_ms
		} else {
			RAMP_DOWN_MS
		};
		self.ramp += (ramp_target - self.ramp) * (1.0 - (-dt_s * 1000.0 / ramp_ms).exp());

		// effective tau: the configured "initial" speed at ramp 0, the profile's
		// top speed at ramp 1
		let tau = (init_tau_ms + (top_tau - init_tau_ms) * self.ramp).max(1.0);
		// Two-stage ease: `mid` chases the target at the effective tau and `visual`
		// chases `mid` over ATTACK_FRACT of it, so motion builds from rest instead
		// of spiking on the first frame. Neither stage can pass its input, so the
		// cascade cannot overshoot.
		let smoothing = 1.0 - (-dt_s * 1000.0 / tau).exp();
		self.mid += (self.target - self.mid) * smoothing;
		let attack_tau = (tau * ATTACK_FRACT).max(1.0);
		let attack = 1.0 - (-dt_s * 1000.0 / attack_tau).exp();
		let before = self.visual;
		self.visual += (self.mid - self.visual) * attack;
		// Sharper stop: inside the band, close on the target by at least
		// STOP_MIN_LPS (never past it) so the tail sweeps in instead of crawling.
		let gap = self.target - before;
		if gap.abs() < STOP_BAND {
			let floored = if gap >= 0.0 {
				(before + STOP_MIN_LPS * dt_s).min(self.target)
			} else {
				(before - STOP_MIN_LPS * dt_s).max(self.target)
			};
			if (self.target - floored).abs() < (self.target - self.visual).abs() {
				self.visual = floored;
			}
			// keep the leading stage at least as close to the target as `visual`,
			// or the cascade would pull the glide back
			if (self.target - self.mid).abs() > (self.target - self.visual).abs() {
				self.mid = self.visual;
			}
		}
		if (self.target - self.visual).abs() < config::SETTLE_EPS {
			// Rest on a whole line: a pixel-delta wheel (touchpad, hi-res wheel)
			// accumulates a fractional target, and parking there renders every row
			// shifted by a sub-cell fraction - the top scanlines of the first
			// clipped row peek out at the pane's content bottom, which reads as
			// garbage hugging the divider. Glide to the nearest line instead.
			let detent = self.target.round().clamp(0.0, self.max);
			if (self.visual - detent).abs() < config::SETTLE_EPS {
				self.target = detent;
				self.visual = detent;
				self.mid = detent;
				self.ramp = 0.0;
				// at rest the burst is over; the next output starts a fresh one
				self.burst = 0.0;
				self.overflow = false;
			} else {
				self.target = detent; // still a fraction away: keep easing to the detent
			}
		}

		// Ease the alt-screen slide offset back to rest. The reveal strip is filled
		// from ONE retained frame (one step back), so the offset must not lag more
		// than ~a line or the strip under-fills (shows background). Ease at the smooth
		// configured speed while the lag is small (gentle scroll stays buttery), but
		// ramp the ease faster as the lag grows past a line, so a fast burst can't
		// glide far behind and open a blank band. The rate change is smooth (a shorter
		// tau, not a snap), so the motion never jumps.
		if self.app_off != 0.0 {
			let lag = self.app_off.abs();
			let lag_ramp = ((lag - APP_LAG_SOFT) / (APP_LAG_HARD - APP_LAG_SOFT)).clamp(0.0, 1.0);
			let app_tau = (init_tau_ms + (MIN_APP_TAU_MS - init_tau_ms) * lag_ramp).max(1.0);
			let app_smoothing = 1.0 - (-dt_s * 1000.0 / app_tau).exp();
			self.app_off -= self.app_off * app_smoothing;
			if self.app_off.abs() < config::SETTLE_EPS {
				self.app_off = 0.0;
			}
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
		(self.target - self.visual).abs() > config::SETTLE_EPS || self.app_off != 0.0
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
	fn fractional_wheel_rests_on_a_whole_line() {
		// pixel-delta wheels (touchpad, hi-res) send fractional line amounts; the
		// rest position must still be a whole line or every row renders sub-cell
		// shifted and the first clipped row's top peeks out at the content bottom
		let mut s = Scroll::new();
		s.set_max(100.0);
		s.wheel(2.6);
		for _ in 0..2000 {
			s.advance(0.016);
		}
		assert!(!s.animating());
		assert_eq!(s.desired_offset(), 3); // detent at round(2.6)
		assert!(s.frac().abs() < 1e-6, "frac {} at rest", s.frac());
		// accumulating many small fractional notches also lands on a line
		let mut t = Scroll::new();
		t.set_max(100.0);
		for _ in 0..7 {
			t.wheel(0.3);
		}
		for _ in 0..2000 {
			t.advance(0.016);
		}
		assert_eq!(t.desired_offset(), 2); // round(2.1)
		assert!(t.frac().abs() < 1e-6);
	}

	#[test]
	fn nudge_accumulates_and_caps() {
		let mut s = Scroll::new();
		s.set_max(1000.0);
		s.nudge_output(1.0, 40.0);
		let after_one = s.frac() + s.desired_offset() as f32;
		assert!(after_one >= ease_lines().min(1.0) - 1e-3);
		// a burst may lag at most MAX_BACKLOG lines
		for _ in 0..100 {
			s.nudge_output(5.0, 40.0);
		}
		assert!(s.desired_offset() as f32 + s.frac() <= MAX_BACKLOG + 1e-3);
	}

	#[test]
	fn nudge_ignored_when_scrolled_back() {
		let mut s = Scroll::new();
		s.set_max(100.0);
		s.wheel(50.0);
		let before = s.desired_offset() as f32 + s.frac();
		s.nudge_output(10.0, 40.0);
		let after = s.desired_offset() as f32 + s.frac();
		assert_eq!(before, after); // no-snap rule: output must not move a reader
	}

	#[test]
	fn output_backlog_settles_to_bottom() {
		let mut s = Scroll::new();
		s.set_max(1000.0);
		for _ in 0..10 {
			s.nudge_output(3.0, 40.0);
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
			burst.nudge_output(5.0, 24.0); // deep backlog, overflows -> full ramp
		}
		let start_b = burst.desired_offset() as f32 + burst.frac();
		let mut trickle = Scroll::new();
		trickle.set_max(1000.0);
		trickle.nudge_output(0.9, 24.0); // below the ramp threshold
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
	fn app_scroll_sets_caps_and_eases_to_rest() {
		let mut s = Scroll::new();
		s.app_scroll(3.0);
		assert_eq!(s.app_offset(), 3.0);
		assert!(s.animating());
		// sets (does not stack) the per-step offset; over the cap is clamped
		s.app_scroll(3.0);
		assert_eq!(s.app_offset(), 3.0);
		s.app_scroll(99.0);
		assert_eq!(s.app_offset(), APP_OFF_CAP);
		// negative direction too
		let mut b = Scroll::new();
		b.app_scroll(-2.0);
		assert_eq!(b.app_offset(), -2.0);
		// eases back to 0 and stops animating (following the bottom, no output)
		for _ in 0..2000 {
			b.advance(0.016);
		}
		assert_eq!(b.app_offset(), 0.0);
		assert!(!b.animating());
	}

	#[test]
	fn app_off_lag_ramp_bounds_a_fast_burst() {
		// The caller accumulates the slide offset (app_off += step) for smooth content;
		// the ease ramps faster as the lag grows so a fast burst can't glide far behind
		// and open a blank reveal strip. Simulate a fast continuous scroll and check the
		// lag stays bounded well under the hard cap.
		let mut s = Scroll::new();
		let mut max_lag = 0.0f32;
		for _ in 0..80 {
			s.app_scroll(s.app_offset() + 1.0); // one detected line-step
			max_lag = max_lag.max(s.app_offset().abs());
			s.advance(0.016);
			s.advance(0.016);
			max_lag = max_lag.max(s.app_offset().abs());
		}
		assert!(max_lag < APP_LAG_HARD + 3.0, "lag {max_lag} ran away");
		assert!(max_lag < APP_OFF_CAP, "lag {max_lag} hit the hard cap");
	}

	#[test]
	fn output_ease_descends_monotonically() {
		// After output stops, the visual position must ease straight down to the live
		// bottom - never rise again. A rise mid-ease is the "page jumps around" /
		// "scrolls bottom-up" artifact. Assert the position is non-increasing every
		// frame and reaches the bottom - in BOTH speed profiles (in-view and
		// overflowed).
		for rows in [40.0f32, 8.0] {
			let mut s = Scroll::new();
			s.set_max(1000.0);
			for _ in 0..4 {
				s.nudge_output(4.0, rows); // build a backlog to ease down from
			}
			let mut prev = s.desired_offset() as f32 + s.frac();
			assert!(prev > 0.0, "backlog should lift the view off the bottom");
			for _ in 0..3000 {
				s.advance(0.016);
				let pos = s.desired_offset() as f32 + s.frac();
				assert!(pos <= prev + 1e-4, "position rose {prev} -> {pos} (bounce)");
				prev = pos;
			}
			assert!(s.following());
			assert_eq!(s.desired_offset(), 0);
		}
	}

	#[test]
	fn an_inview_burst_eases_gentler_than_an_overflowed_one() {
		// Same backlog, two screen heights: the burst that stays wholly on screen
		// (a short listing) must clear measurably slower than one whose first line
		// has scrolled off the top - that is the whole two-profile point.
		let mut inview = Scroll::new();
		inview.set_max(1000.0);
		let mut over = Scroll::new();
		over.set_max(1000.0);
		for _ in 0..6 {
			inview.nudge_output(2.0, 40.0); // 12 lines on a 40-row pane: in view
			over.nudge_output(2.0, 10.0); // same lines on 10 rows: top scrolled off
		}
		let start = inview.desired_offset() as f32 + inview.frac();
		assert_eq!(start, over.desired_offset() as f32 + over.frac());
		for _ in 0..30 {
			inview.advance(0.016);
			over.advance(0.016);
		}
		let left_i = inview.desired_offset() as f32 + inview.frac();
		let left_o = over.desired_offset() as f32 + over.frac();
		assert!(
			left_o < left_i,
			"overflowed burst ({left_o} left) should outrun the in-view one ({left_i} left)"
		);
	}

	#[test]
	fn a_burst_ends_at_rest_and_the_next_starts_in_view() {
		// Overflow must not outlive its burst: once the ease has settled at the
		// bottom, fresh output is a new burst and gets the gentle profile again.
		// A settled-then-nudged scroll must track a never-overflowed twin exactly.
		let mut s = Scroll::new();
		s.set_max(1000.0);
		s.nudge_output(12.0, 5.0); // overflows a 5-row pane at once
		for _ in 0..3000 {
			s.advance(0.016);
		}
		assert!(
			!s.animating(),
			"must be fully settled before the second burst"
		);
		let mut fresh = Scroll::new();
		fresh.set_max(1000.0);
		for _ in 0..4 {
			s.nudge_output(2.0, 40.0);
			fresh.nudge_output(2.0, 40.0);
			for _ in 0..3 {
				s.advance(0.016);
				fresh.advance(0.016);
			}
		}
		let pos_s = s.desired_offset() as f32 + s.frac();
		let pos_f = fresh.desired_offset() as f32 + fresh.frac();
		assert!(
			(pos_s - pos_f).abs() < 1e-4,
			"stale overflow leaked into the next burst: {pos_s} vs fresh {pos_f}"
		);
	}

	#[test]
	fn app_slide_eases_monotonically_without_overshoot() {
		// A single detected app-scroll step must glide to rest in one direction: the
		// offset magnitude only shrinks and never flips sign (a sign flip = the content
		// bounces back the other way). Guards the alt-screen slide feel.
		let mut s = Scroll::new();
		s.app_scroll(4.0);
		let mut prev = s.app_offset();
		for _ in 0..3000 {
			s.advance(0.016);
			let off = s.app_offset();
			assert!(off >= -1e-4, "offset flipped negative: {off} (bounce)");
			assert!(off <= prev + 1e-4, "offset grew {prev} -> {off}");
			prev = off;
			if off == 0.0 {
				break;
			}
		}
		assert_eq!(s.app_offset(), 0.0);
	}

	#[test]
	fn cancel_app_scroll_hard_cuts_the_slide() {
		// an alt-screen enter/exit must drop any in-flight slide at once (no ease)
		let mut s = Scroll::new();
		s.app_scroll(7.0);
		assert!(s.animating());
		s.cancel_app_scroll();
		assert_eq!(s.app_offset(), 0.0);
		assert!(!s.animating());
	}

	#[test]
	fn snap_lands_at_rest_instantly() {
		// unfreeze catch-up: pending output backlog and any slide drop at once
		let mut s = Scroll::new();
		s.set_max(50.0);
		s.nudge_output(12.0, 40.0);
		s.app_scroll(4.0);
		assert!(s.animating());
		s.snap();
		assert!(!s.animating());
		assert_eq!(s.app_offset(), 0.0);
		assert_eq!(s.desired_offset(), 0);
		// scrolled back: snap holds the position, kills only the motion
		s.wheel(20.0);
		s.snap();
		assert!(!s.animating());
		assert_eq!(s.desired_offset(), 20);
	}

	#[test]
	fn a_fresh_motion_builds_speed_instead_of_spiking() {
		// The first frame of a new scroll must be gentler than the motion a few
		// frames in - the ease-in half of the asymmetric curve. A bare exponential
		// fails this: its very first frame is the fastest of the whole ease.
		let mut s = Scroll::new();
		s.set_max(100.0);
		s.wheel(8.0);
		let dt = 0.016;
		let mut prev = 0.0f32;
		let mut steps = Vec::new();
		for _ in 0..8 {
			s.advance(dt);
			let pos = s.desired_offset() as f32 + s.frac();
			steps.push(pos - prev);
			prev = pos;
		}
		let first = steps[0];
		let peak = steps.iter().copied().fold(0.0f32, f32::max);
		assert!(
			first < peak * 0.6,
			"first frame moved {first} of peak {peak}: no ease-in"
		);
	}

	#[test]
	fn the_tail_sweeps_in_instead_of_crawling() {
		// Once within STOP_BAND of the target the remainder must land in well
		// under half a second - the sharpened stop. The bare exponential took
		// over a second to close the same distance at the default tau.
		let mut s = Scroll::new();
		s.set_max(100.0);
		s.wheel(6.0);
		let dt = 0.016;
		let mut in_band = 0;
		for _ in 0..3000 {
			s.advance(dt);
			if !s.animating() {
				break;
			}
			let pos = s.desired_offset() as f32 + s.frac();
			if (6.0 - pos).abs() < STOP_BAND {
				in_band += 1;
			}
		}
		assert!(!s.animating(), "never settled");
		let band_s = in_band as f32 * dt;
		assert!(
			band_s < 0.35,
			"tail took {band_s}s inside the stop band (crawl)"
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
