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
// Dynamic-speed output scroll: an output burst is chased at an explicit speed
// (`chase`, lines/s) that traverses a chain of named segments, each handing its
// end point to the next - the settings are the segments, and each one has no
// other influence on its neighbours:
//
//   Ease-in    a linear lift from rest to KNEE_LPS, covering that delta in
//              `scroll_ease_in_ms`. This is the only segment that can leave
//              zero, so the first moments of any burst read as motion, not a
//              jump. When the single-screen cap lifts mid-burst it runs once
//              more from the speed it finds itself at (same slope, same delta).
//   Ramp-up    from the knee the speed doubles every `scroll_ramp_up_ms`,
//              toward whichever cap currently applies. Its end height belongs
//              to the cap, not to it - so it controls neither its own duration
//              nor its length, only its steepness.
//   Max speed  a flat ceiling: 1000/`scroll_single_screen_tau_ms` while the
//              burst's own first line is still on screen ("Single-screen
//              speed"); unbounded once that line has scrolled off the top.
//   Ramp-down  the braking curve, applied as a cap the whole time: the speed
//              may never exceed what can decay (halving per
//              `scroll_ramp_down_ms`) down to the ease-out handoff within the
//              backlog still to render. While output pours in the backlog is
//              deep and the cap is moot - but it is also what keeps the view
//              a braking distance behind the live bottom (the reserve), so the
//              moment output ceases the speed rides this curve down instead of
//              stopping dead. Traced backwards from Ease-out: it ends exactly
//              where the landing begins.
//   Ease-out   the landing: the last STOP_BAND of a line closes at the speed
//              that covers the band in `scroll_ease_out_ms`, never past the
//              target. Y ends at 0 by construction.
//
// The backlog itself is NOT capped in lines (a hard cap forces the view to ride
// the output rate the instant it fills, which is exactly the "jumps immediately
// to blazing speed" bug) - the ramp-up growth and ramp-down reserve bound the
// lag in time instead of lines. The chase applies only to output while
// following the bottom - wheel/scrollback navigation keeps the plain NAV_TAU_MS
// ease, and a user jump back to the bottom sweeps at full ease speed (`sweep`).
//
// The ease itself is asymmetric, and the two ends are the "Ease-in"/"Ease-out"
// settings: motion builds from rest (a two-stage cascade - `visual` chases a
// leading `mid` stage over `scroll_ease_in_ms`, so the first frames are gentle
// instead of jumping straight to peak speed), and the stop is sharpened by a
// minimum closing speed inside STOP_BAND (a bare exponential would crawl the
// last few pixels in over a second). The two are presented as one pair, so
// BOTH read "higher = gentler" in the dialog - which is why ease-out is stored
// as the tail's DURATION and the closing speed is derived from it, rather than
// stored as the speed itself (that would invert against its own partner).
pub const MAX_BACKLOG: f32 = 16.0; // reference depth for output_ease_lines' clamp + set_max overscan
const CHASE_GROW_MIN: f32 = 2.0; // gap (lines) under which a user sweep counts as caught up
// Where Ease-in hands off to Ramp-up (lines/s). An exponential ramp cannot
// leave zero, so the first KNEE_LPS of speed is a linear lift; past it the
// doubling takes over. The overflow re-invocation re-arms the knee this far
// above the speed the cap lift found.
const KNEE_LPS: f32 = 6.0;
// User-navigation ease time constant (wheel, scrollbar, jumps) and the
// alt-screen slide's base tau. Fixed: the one "Initial scroll speed" knob that
// used to feed this also fed the chase start and the ease-in attack, which is
// how every slider ended up influencing every other.
const NAV_TAU_MS: f32 = 230.0;
// Sharper stop: within this many lines of the target the exponential tail is
// replaced by a glide that covers the band in `scroll_ease_out_ms`, so the last
// few pixels sweep in instead of crawling. Above the band the ease-out is
// untouched.
const STOP_BAND: f32 = 0.4;
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
	chase: f32, // output catch-up speed, lines/s (the segment the curve is in falls out of it)
	knee: f32,  // speed where the current Ease-in segment hands off to Ramp-up
	app_off: f32, // alt-screen slide offset, eased toward 0 (see APP_OFF_CAP)
	burst: f32, // lines this output burst has advanced (resets once settled at rest)
	overflow: bool, // burst topped a screenful: its first line is off - uncapped chase
	sweep: bool, // user jumped back to the bottom: full ease speed until caught up
}

impl Scroll {
	pub fn new() -> Self {
		Self {
			target: 0.0,
			visual: 0.0,
			mid: 0.0,
			max: 0.0,
			chase: 0.0,
			knee: KNEE_LPS,
			app_off: 0.0,
			burst: 0.0,
			overflow: false,
			sweep: false,
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
		self.chase = 0.0;
		self.knee = KNEE_LPS;
		self.app_off = 0.0;
		self.burst = 0.0;
		self.overflow = false;
		self.sweep = false;
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
		// a wheel that lands back on the bottom is still the user driving: the
		// remaining gap sweeps at full ease speed, not the output chase
		self.sweep = true;
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
		self.sweep = true;
	}

	pub fn jump_bottom(&mut self) {
		self.target = 0.0;
		self.sweep = true;
	}

	// New output grew the scrollback by `grown` lines while following the bottom:
	// accumulate it into the visual backlog so a fast burst lags and the chase
	// scrolls through it. The backlog is deliberately uncapped - the chase's
	// exponential growth bounds the lag in time, and a line cap would force the
	// view straight to the output rate the instant it filled. Sporadic output
	// stays at ~output_ease_lines and eases in at the initial speed. `view_rows`
	// is the pane's screen height: once a burst has advanced that far, its first
	// line has provably scrolled off the top (it printed at most a screen above
	// the bottom), which lifts the in-view speed ceiling off the chase.
	pub fn nudge_output(&mut self, grown: f32, view_rows: f32) {
		if self.following() {
			let cfg = config::settings();
			if !cfg.scroll_smooth {
				return; // master off: the grid already sits at the bottom, no lag to ease
			}
			if self.burst <= 0.0 {
				// fresh burst: the curve starts at Y=0 - Ease-in owns the first moments
				self.chase = 0.0;
				self.knee = KNEE_LPS;
			}
			self.burst += grown;
			if view_rows > 0.0 && self.burst >= view_rows && !self.overflow {
				self.overflow = true;
				// the cap lifts: Ease-in runs once more from the speed it found
				// itself at (same slope, same delta), then Ramp-up resumes
				self.knee = self.chase + KNEE_LPS;
			}
			let floor = cfg.output_ease_lines.clamp(0.0, MAX_BACKLOG);
			let before = self.visual;
			self.visual = (self.visual + grown).max(floor);
			// the nudge is a coordinate shift (content moved under the view), so the
			// cascade stage rides along - shifting it keeps its lead over `visual`
			// intact, which is what preserves the eased speed across a burst
			self.mid = (self.mid + (self.visual - before)).clamp(0.0, self.visual);
		}
	}

	pub fn advance(&mut self, dt_s: f32) {
		// one settings() snapshot per call - this runs per pane per frame
		let cfg = config::settings();
		if !cfg.scroll_smooth {
			// master off: every scroll lands instantly, on a whole line
			self.target = self.target.round().clamp(0.0, self.max);
			self.snap();
			return;
		}
		// The output chase, one segment at a time. Ease-in lifts the speed
		// linearly to the knee; Ramp-up doubles it toward the current cap; the
		// braking curve ("Ramp-down", traced backwards from "Ease-out") caps it
		// the whole way at what can still be wound down within the remaining
		// backlog - which both holds the reserve behind a live burst and owns
		// the deceleration once output ceases.
		let chasing = self.burst > 0.0 && self.following() && !self.sweep;
		if chasing {
			if self.chase < self.knee {
				let ease_in_s = cfg.scroll_ease_in_ms.max(1.0) / 1000.0;
				self.chase = (self.chase + KNEE_LPS * dt_s / ease_in_s).min(self.knee);
			} else {
				let double_s = cfg.scroll_ramp_up_ms.max(1.0) / 1000.0;
				self.chase *= (dt_s / double_s).exp2();
			}
			if !self.overflow {
				let onscreen_v = 1000.0 / cfg.scroll_single_screen_tau_ms.max(1.0);
				self.chase = self.chase.min(onscreen_v);
			}
			// braking: halving per "Ramp-down" from here must land on the
			// ease-out handoff (the STOP_BAND edge, at its closing speed) with
			// no backlog left over
			let v_land = STOP_BAND * 1000.0 / cfg.scroll_ease_out_ms.max(1.0);
			let brake_tau_s = cfg.scroll_ramp_down_ms.max(1.0) / 1000.0 / std::f32::consts::LN_2;
			let backlog = (self.visual - self.target).max(0.0);
			self.chase = self
				.chase
				.min(v_land + (backlog - STOP_BAND).max(0.0) / brake_tau_s);
		}

		// Two-stage ease: `mid` chases the target at NAV_TAU_MS and `visual`
		// chases `mid` over the ease-in attack, so motion builds from rest
		// instead of spiking on the first frame. Neither stage can pass its input,
		// so the cascade cannot overshoot.
		let smoothing = 1.0 - (-dt_s * 1000.0 / NAV_TAU_MS).exp();
		self.mid += (self.target - self.mid) * smoothing;
		let attack_tau = cfg.scroll_ease_in_ms.max(1.0);
		let attack = 1.0 - (-dt_s * 1000.0 / attack_tau).exp();
		let before = self.visual;
		self.visual += (self.mid - self.visual) * attack;
		if chasing {
			// the chase is a speed LIMIT on the ease, not a motor: the ease's own
			// arrival dynamics (decel, stop band, detent) take over once the gap is
			// small enough that the unclamped ease is the slower of the two
			self.visual = self.visual.max(before - self.chase * dt_s);
		}
		if self.sweep && self.visual - self.target < CHASE_GROW_MIN {
			self.sweep = false; // caught up: output easing owns the view again
		}
		// Sharper stop: inside the band, close on the target fast enough to cover
		// the whole band within "Ease-out" (never past it) so the tail sweeps in
		// instead of crawling. Storing the duration rather than the speed is what
		// lets the setting read the same direction as its Ease-in partner.
		let stop_min_lps = STOP_BAND * 1000.0 / cfg.scroll_ease_out_ms.max(1.0);
		let gap = self.target - before;
		if gap.abs() < STOP_BAND {
			let floored = if gap >= 0.0 {
				(before + stop_min_lps * dt_s).min(self.target)
			} else {
				(before - stop_min_lps * dt_s).max(self.target)
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
				self.chase = 0.0;
				self.knee = KNEE_LPS;
				// at rest the burst is over; the next output starts a fresh one
				self.burst = 0.0;
				self.overflow = false;
				self.sweep = false;
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
			let app_tau = (NAV_TAU_MS + (MIN_APP_TAU_MS - NAV_TAU_MS) * lag_ramp).max(1.0);
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
	use std::sync::{Mutex, MutexGuard};

	// The settings store initializes from the LIVE user config, not the shipped
	// defaults - a box whose config carries tuned scroll speeds would otherwise
	// steer every assertion here (it did: a fast tau made the whole module fail).
	// Each test pins the defaults first; the guard serializes the module so a
	// test that pins something else cannot race the rest.
	static PIN: Mutex<()> = Mutex::new(());
	fn pin() -> MutexGuard<'static, ()> {
		let guard = PIN
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		config::update(config::Settings::default());
		guard
	}

	fn ease_lines() -> f32 {
		config::settings().output_ease_lines.max(0.0)
	}

	#[test]
	fn starts_following() {
		let _g = pin();
		let s = Scroll::new();
		assert!(s.following());
		assert!(!s.animating());
		assert_eq!(s.desired_offset(), 0);
	}

	#[test]
	fn wheel_clamps_to_history() {
		let _g = pin();
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
		let _g = pin();
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
	fn nudge_accumulates_without_a_line_cap() {
		let _g = pin();
		let mut s = Scroll::new();
		s.set_max(1000.0);
		s.nudge_output(1.0, 40.0);
		let after_one = s.frac() + s.desired_offset() as f32;
		assert!(after_one >= ease_lines().min(1.0) - 1e-3);
		// the backlog is deliberately uncapped: a line cap forces the view onto
		// the raw output rate the moment it fills, which defeats the slow start
		for _ in 0..100 {
			s.nudge_output(5.0, 40.0);
		}
		let lag = s.desired_offset() as f32 + s.frac();
		assert!(lag > MAX_BACKLOG, "backlog capped at {lag}");
		assert!(lag <= 501.0 + 1e-3);
	}

	#[test]
	fn nudge_ignored_when_scrolled_back() {
		let _g = pin();
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
		let _g = pin();
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
	fn a_burst_leaves_rest_slowly_with_the_ramp_still_ahead() {
		let _g = pin();
		// THE core complaint this model answers: a dump used to hit full speed
		// within ~100ms. The first quarter second of even a deep, overflowed
		// burst must move - but only through Ease-in's lift and the first
		// moments of the ramp, nowhere near the speed it will reach.
		let mut s = Scroll::new();
		s.set_max(10_000.0);
		s.nudge_output(200.0, 24.0); // a dump: deep and instantly overflowed
		let start = s.desired_offset() as f32 + s.frac();
		for _ in 0..15 {
			s.advance(0.016);
		}
		let covered = start - (s.desired_offset() as f32 + s.frac());
		assert!(covered > 0.1, "never started moving ({covered} lines)");
		// generous ceiling: the knee speed for the whole window, plus slack for
		// the ramp's first doublings
		assert!(
			covered < KNEE_LPS * 0.24 * 2.0,
			"first 0.24s covered {covered} lines - that is a jump, not a slow start"
		);
	}

	#[test]
	fn burst_ramps_faster_than_trickle() {
		let _g = pin();
		// a sustained deep backlog must be moving measurably faster later in the
		// burst than the initial speed a trickle eases at - the exponential ramp
		let mut burst = Scroll::new();
		burst.set_max(10_000.0);
		burst.nudge_output(300.0, 24.0); // deep, overflowed: the chase is uncapped
		for _ in 0..62 {
			burst.advance(0.016); // ~1s: the chase has doubled a few times
		}
		let at_1s = burst.desired_offset() as f32 + burst.frac();
		for _ in 0..15 {
			burst.advance(0.016); // measure a 0.24s window at speed
		}
		let burst_window = at_1s - (burst.desired_offset() as f32 + burst.frac());
		assert!(
			burst_window > KNEE_LPS * 0.24 * 3.0,
			"one second in, a deep burst should far outrun the knee speed \
			 (covered {burst_window} lines in 0.24s; the knee would cover {})",
			KNEE_LPS * 0.24
		);
	}

	#[test]
	fn app_scroll_sets_caps_and_eases_to_rest() {
		let _g = pin();
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
		let _g = pin();
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
		let _g = pin();
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
	fn a_single_screen_burst_eases_gentler_than_an_overflowed_one() {
		let _g = pin();
		// Same backlog, two screen heights: the burst that stays wholly on screen
		// (a short listing) must clear measurably slower than one whose first line
		// has scrolled off the top - that is the whole two-profile point.
		let mut inview = Scroll::new();
		inview.set_max(1000.0);
		let mut over = Scroll::new();
		over.set_max(1000.0);
		for _ in 0..20 {
			inview.nudge_output(2.0, 60.0); // 40 lines on a 60-row pane: in view
			over.nudge_output(2.0, 10.0); // same lines on 10 rows: top scrolled off
		}
		let start = inview.desired_offset() as f32 + inview.frac();
		assert_eq!(start, over.desired_offset() as f32 + over.frac());
		for _ in 0..75 {
			// ~1.2s: past where the uncapped chase overtakes the in-view ceiling
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
		let _g = pin();
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
		let _g = pin();
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
		let _g = pin();
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
		let _g = pin();
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
	fn smooth_off_lands_every_scroll_instantly() {
		let _g = pin();
		// the master switch: no eased wheel, no output lag, no app slide
		config::update(config::Settings {
			scroll_smooth: false,
			..config::Settings::default()
		});
		let mut s = Scroll::new();
		s.set_max(100.0);
		s.wheel(30.0);
		s.advance(0.016);
		assert_eq!(s.desired_offset(), 30, "wheel must land instantly");
		assert!(!s.animating());
		s.jump_bottom();
		s.advance(0.016);
		assert_eq!(s.desired_offset(), 0);
		s.nudge_output(10.0, 40.0); // output produces no visual lag at all
		assert_eq!(s.desired_offset(), 0);
		assert!(s.frac().abs() < 1e-6);
		s.app_scroll(3.0); // a slide request is dropped on the next frame
		s.advance(0.016);
		assert_eq!(s.app_offset(), 0.0);
		assert!(!s.animating());
		// leave the shared store on defaults for tests outside this module
		config::update(config::Settings::default());
	}

	#[test]
	fn a_user_jump_to_bottom_is_not_chase_limited() {
		let _g = pin();
		// Returning from deep scrollback is the user driving: the gap closes at
		// the full ease speed, never throttled to the output chase's slow start.
		let mut s = Scroll::new();
		s.set_max(1000.0);
		s.wheel(200.0);
		for _ in 0..2000 {
			s.advance(0.016);
		}
		assert_eq!(s.desired_offset(), 200);
		s.jump_bottom();
		// output keeps arriving while the jump eases home
		for _ in 0..125 {
			s.nudge_output(0.2, 40.0);
			s.advance(0.016);
		}
		// 200 lines through the chase's opening segments would take most of a
		// minute; the configured ease does it in about a second
		let left = s.desired_offset() as f32 + s.frac();
		assert!(left < 30.0, "jump home crawled: {left} lines left after 2s");
	}

	#[test]
	fn a_fresh_motion_builds_speed_instead_of_spiking() {
		let _g = pin();
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
		let _g = pin();
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

	// The four feel knobs each have to MOVE something: this whole settings line
	// exists because two speed sliders shipped as no-ops, and a knob that reads
	// as inert is worse than no knob. Each test drives the two extremes of one
	// setting and asserts they diverge.
	fn with(cfg: config::Settings) {
		config::update(cfg);
	}

	#[test]
	fn ease_in_sets_how_gently_motion_leaves_rest() {
		let _g = pin();
		let travelled = |ease_in_ms: f32| {
			with(config::Settings {
				scroll_ease_in_ms: ease_in_ms,
				..config::Settings::default()
			});
			let mut s = Scroll::new();
			s.set_max(100.0);
			s.wheel(20.0);
			for _ in 0..4 {
				s.advance(0.016);
			}
			s.visual
		};
		let abrupt = travelled(8.0);
		let gentle = travelled(800.0);
		with(config::Settings::default()); // other modules read this same store
		assert!(
			abrupt > gentle * 1.5,
			"leaving rest at ease-in 8ms ({abrupt}) should far outpace 800ms ({gentle})"
		);
	}

	#[test]
	fn ease_in_also_paces_a_bursts_first_moments() {
		let _g = pin();
		// The same knob shapes the output curve's leave-from-rest: a longer
		// Ease-in is a shallower lift to the knee, so the burst's opening
		// covers less ground. Everything past the knee is Ramp-up's business.
		let covered = |ease_in_ms: f32| {
			with(config::Settings {
				scroll_ease_in_ms: ease_in_ms,
				..config::Settings::default()
			});
			let mut s = Scroll::new();
			s.set_max(10_000.0);
			s.nudge_output(200.0, 24.0);
			let start = s.desired_offset() as f32 + s.frac();
			for _ in 0..9 {
				s.advance(0.016); // ~0.14s: inside the gentle arm's lift
			}
			start - (s.desired_offset() as f32 + s.frac())
		};
		let crisp = covered(8.0);
		let soft = covered(800.0);
		with(config::Settings::default());
		assert!(
			crisp > soft * 3.0,
			"a crisp ease-in ({crisp} lines) should far outpace a soft one ({soft}) leaving rest"
		);
	}

	#[test]
	fn ramp_up_sets_how_fast_catch_up_accelerates() {
		let _g = pin();
		let left = |ramp_up_ms: f32| {
			with(config::Settings {
				scroll_ramp_up_ms: ramp_up_ms,
				..config::Settings::default()
			});
			let mut s = Scroll::new();
			s.set_max(1000.0);
			s.nudge_output(300.0, 24.0); // deep and instantly overflowed: uncapped
			for _ in 0..60 {
				s.advance(0.016); // ~1s of ramping
			}
			s.visual
		};
		let hard = left(50.0);
		let soft = left(1500.0);
		with(config::Settings::default());
		assert!(
			hard < soft * 0.5,
			"a hard ramp ({hard} lines left) should outrun a soft one ({soft} left)"
		);
	}

	#[test]
	fn ramp_down_sets_how_gradually_speed_winds_down() {
		let _g = pin();
		// Ramp-down is the braking curve, traced backwards from the ease-out
		// handoff: a gentler (longer) setting holds the view a longer braking
		// distance behind the live bottom and takes correspondingly longer to
		// wind the same backlog down once output has ceased. Frames from
		// "output stopped" to rest is the measurement.
		let frames_to_rest = |ramp_down_ms: f32| {
			with(config::Settings {
				scroll_ramp_down_ms: ramp_down_ms,
				..config::Settings::default()
			});
			let mut s = Scroll::new();
			s.set_max(5000.0);
			s.nudge_output(400.0, 24.0); // deep, instantly overflowed
			for _ in 0..80 {
				s.advance(0.016); // ~1.3s: well up to speed
			}
			let mut frames = 0;
			while s.animating() && frames < 20_000 {
				s.advance(0.016);
				frames += 1;
			}
			assert!(!s.animating(), "never settled at ramp-down {ramp_down_ms}");
			frames
		};
		let quick = frames_to_rest(60.0);
		let slow = frames_to_rest(3000.0);
		with(config::Settings::default());
		assert!(
			slow > quick * 2,
			"a gentle ramp-down should wind down far longer ({slow} frames) than a hard one ({quick})"
		);
	}

	#[test]
	fn a_stopped_burst_decelerates_instead_of_stopping_dead() {
		let _g = pin();
		// The reserve: while output pours in at a steady rate, the braking cap
		// keeps the view a wind-down's distance behind the bottom - so the
		// moment output ceases, the speed rides the ramp-down curve to the
		// landing instead of falling off a cliff. Reach the steady state
		// first, then stop feeding and watch the descent.
		let mut s = Scroll::new();
		s.set_max(50_000.0);
		for _ in 0..250 {
			s.nudge_output(1.0, 24.0); // ~62 lines/s, long enough to equalize
			s.advance(0.016);
		}
		let reserve = s.desired_offset() as f32 + s.frac();
		assert!(
			reserve > 5.0,
			"at speed the view should trail by a braking distance, not {reserve} lines"
		);
		let mut prev_pos = reserve;
		let mut prev_v = f32::MAX;
		let mut frames_moving = 0;
		for _ in 0..3000 {
			s.advance(0.016);
			let pos = s.desired_offset() as f32 + s.frac();
			let v = (prev_pos - pos) / 0.016;
			if v <= 0.5 {
				break;
			}
			assert!(
				v < prev_v * 1.05 + 0.1,
				"speed rose mid wind-down: {prev_v} -> {v} lines/s"
			);
			frames_moving += 1;
			prev_v = v;
			prev_pos = pos;
		}
		assert!(
			frames_moving > 20,
			"the wind-down lasted only {frames_moving} frames - a cliff, not a ramp-down"
		);
	}

	#[test]
	fn ease_out_sets_how_gently_the_tail_lands() {
		let _g = pin();
		// Frames spent inside the stop band - the tail, and nothing else: the
		// approach above the band is identical either way.
		let band_frames = |ease_out_ms: f32| {
			with(config::Settings {
				scroll_ease_out_ms: ease_out_ms,
				..config::Settings::default()
			});
			let mut s = Scroll::new();
			s.set_max(100.0);
			s.wheel(6.0);
			let mut in_band = 0;
			for _ in 0..3000 {
				s.advance(0.016);
				if !s.animating() {
					break;
				}
				if (6.0 - s.visual).abs() < STOP_BAND {
					in_band += 1;
				}
			}
			assert!(!s.animating(), "never settled at ease-out {ease_out_ms}");
			in_band
		};
		let crisp = band_frames(20.0);
		let soft = band_frames(800.0);
		with(config::Settings::default());
		assert!(
			soft > crisp,
			"a soft ease-out should linger in the tail ({soft} frames) longer than a crisp one ({crisp})"
		);
	}

	#[test]
	fn set_max_clamps_positions() {
		let _g = pin();
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
