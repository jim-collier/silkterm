// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

//! Flyover help: the parts every tip in the program shares.
//!
//! There are four places a tip comes up - a Settings row, a link in the About
//! box, a tab in the strip, and a menu item - drawn by two different renderers
//! in two different fonts. What they have in common is not the drawing: it is
//! how long the pointer has to rest before a tip appears, how the text is
//! broken to fit, and where the box goes relative to what it describes. Those
//! three live here, and each caller draws the result its own way.

use std::time::{Duration, Instant};

use crate::pane::Rect;

// How long the pointer rests on something before its tip comes up. One value
// for every tip in the program - a menu that answered faster than the tab strip
// would read as a different kind of thing.
pub const DELAY: Duration = Duration::from_millis(600);

// Greedy word wrap, measured in whatever font the caller draws in. A single
// word wider than the budget still gets its own line rather than being split -
// breaking mid-word would be worse than a tip that overhangs by one long word.
pub fn wrap(text: &str, max_w: f32, mut measure: impl FnMut(&str) -> f32) -> Vec<String> {
	let mut lines: Vec<String> = Vec::new();
	let mut line = String::new();
	for word in text.split_whitespace() {
		let candidate = if line.is_empty() {
			word.to_string()
		} else {
			format!("{line} {word}")
		};
		if !line.is_empty() && measure(&candidate) > max_w {
			lines.push(std::mem::take(&mut line));
			line = word.to_string();
		} else {
			line = candidate;
		}
	}
	if !line.is_empty() {
		lines.push(line);
	}
	if lines.is_empty() {
		lines.push(String::new());
	}
	lines
}

// Where a tip box goes: centered under what it describes, or above it when
// there is no room below, and never off an edge. Clamping into the bottom edge
// instead of flipping would sit a footer button's own tip on the buttons it is
// describing, which is the case that made the flip necessary.
pub fn place(anchor: Rect, size: (f32, f32), win: (f32, f32), gap: f32, edge: f32) -> (f32, f32) {
	let (box_w, box_h) = size;
	let (win_w, win_h) = win;
	let x = (anchor.x + anchor.w * 0.5 - box_w * 0.5).clamp(edge, (win_w - box_w - edge).max(edge));
	let below = anchor.y + anchor.h + gap;
	let y = if below + box_h + edge <= win_h {
		below
	} else {
		(anchor.y - gap - box_h).max(edge)
	};
	(x, y)
}

// Where a tip goes when it must not cover what it describes: clear of the
// anchor's right edge, flipped to its left when there is no room there, and top
// aligned with it. A menu tip needs this - a box centered under the row would
// sit on the rows below it, which are exactly what the reader is choosing
// between.
pub fn beside(anchor: Rect, size: (f32, f32), win: (f32, f32), gap: f32, edge: f32) -> (f32, f32) {
	let (box_w, box_h) = size;
	let (win_w, win_h) = win;
	let right = anchor.x + anchor.w + gap;
	let x = if right + box_w + edge <= win_w {
		right
	} else {
		(anchor.x - gap - box_w).max(edge)
	};
	let y = anchor.y.clamp(edge, (win_h - box_h - edge).max(edge));
	(x, y)
}

// What the pointer is resting on, and since when. `T` names the thing in
// whatever terms the caller thinks in - a tab index, a menu row - so the timing
// rule is written once and the identity stays the caller's business.
#[derive(Default)]
pub struct Dwell<T> {
	over: Option<(T, Instant)>,
}

impl<T: Copy + PartialEq> Dwell<T> {
	// Point at something, or at nothing. The clock runs on while the target is
	// unchanged, and restarts when it is not. True means the caller has to
	// redraw: a tip that was up is now pointing somewhere else, or at nothing.
	pub fn point_at(&mut self, target: Option<T>) -> bool {
		match (target, &self.over) {
			(Some(want), Some((have, _))) if *have == want => false,
			(Some(want), _) => {
				let was_ripe = self.ripe().is_some();
				self.over = Some((want, Instant::now()));
				was_ripe
			}
			(None, None) => false,
			(None, Some(_)) => {
				let was_ripe = self.ripe().is_some();
				self.over = None;
				was_ripe
			}
		}
	}

	// What the pointer has rested on long enough to deserve a tip, if anything.
	pub fn ripe(&self) -> Option<T> {
		let (target, since) = self.over.as_ref()?;
		(Instant::now().duration_since(*since) >= DELAY).then_some(*target)
	}

	// When the loop next has to wake to raise a tip. None while nothing is being
	// pointed at, and while one is already up.
	pub fn wake(&self) -> Option<Instant> {
		let (_, since) = self.over.as_ref()?;
		let due = *since + DELAY;
		(due > Instant::now()).then_some(due)
	}
}

#[cfg(test)]
mod tests {
	use super::{Dwell, beside, place, wrap};
	use crate::pane::Rect;

	fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
		Rect { x, y, w, h }
	}

	// Six pixels a character is enough to make the arithmetic obvious.
	fn measure(s: &str) -> f32 {
		s.chars().count() as f32 * 6.0
	}

	// A tip has to fit whatever it hangs off, whatever the font does to the text
	// - one clamped to a window edge simply runs off it.
	#[test]
	fn a_tip_wraps_on_words_and_never_splits_one() {
		let lines = wrap(
			"Apply changes now, without closing Settings.",
			120.0,
			measure,
		);
		assert!(lines.len() > 1);
		assert!(lines.iter().all(|line| measure(line) <= 120.0));
		assert_eq!(
			lines.join(" "),
			"Apply changes now, without closing Settings.",
			"wrapping lost or added text"
		);
		// a word too long for the budget still gets a line of its own
		assert_eq!(
			wrap("a supercalifragilistic word", 48.0, measure),
			vec!["a", "supercalifragilistic", "word"]
		);
		// text that already fits stays on one line, and empty text is one empty
		// line rather than none
		assert_eq!(wrap("short", 240.0, measure), vec!["short"]);
		assert_eq!(wrap("", 100.0, measure), vec![""]);
	}

	// A tip that cannot fit below what it describes flips above it, rather than
	// clamping into the bottom edge and covering it.
	#[test]
	fn a_tip_with_no_room_below_goes_above() {
		let anchor = rect(100.0, 40.0, 60.0, 20.0);
		let (x, y) = place(anchor, (80.0, 30.0), (400.0, 300.0), 8.0, 4.0);
		assert_eq!((x, y), (90.0, 68.0));
		// the same anchor near the bottom of a short window
		let low = rect(100.0, 250.0, 60.0, 20.0);
		let (_, up) = place(low, (80.0, 30.0), (400.0, 300.0), 8.0, 4.0);
		assert_eq!(up, 212.0);
	}

	#[test]
	fn a_tip_stays_inside_both_side_edges() {
		let win = (400.0, 300.0);
		let (left, _) = place(rect(0.0, 10.0, 10.0, 10.0), (80.0, 30.0), win, 8.0, 4.0);
		assert_eq!(left, 4.0);
		let (right, _) = place(rect(390.0, 10.0, 10.0, 10.0), (80.0, 30.0), win, 8.0, 4.0);
		assert_eq!(right, 316.0);
	}

	// A menu tip stands clear of the menu, and swaps to the other side rather
	// than covering the rows it is describing.
	#[test]
	fn a_menu_tip_never_lies_over_its_own_menu() {
		let row = rect(20.0, 60.0, 180.0, 24.0);
		let win = (400.0, 300.0);
		let (x, y) = beside(row, (150.0, 40.0), win, 6.0, 4.0);
		assert_eq!((x, y), (206.0, 60.0));
		// the same row on a menu that has been opened against the right edge
		let far = rect(230.0, 60.0, 160.0, 24.0);
		let (left, _) = beside(far, (150.0, 40.0), win, 6.0, 4.0);
		assert_eq!(left, 74.0);
	}

	// The clock runs on while the pointer stays put, and restarts when it moves
	// to something else - otherwise dragging across a strip would flash a tip
	// over every tab on the way.
	#[test]
	fn moving_to_something_else_restarts_the_clock() {
		let mut dwell: Dwell<usize> = Dwell::default();
		assert!(!dwell.point_at(Some(1)));
		let first = dwell.wake().expect("a wake-up while nothing is up yet");
		assert!(!dwell.point_at(Some(1)));
		assert_eq!(dwell.wake(), Some(first), "same target: the clock runs on");
		assert!(!dwell.point_at(Some(2)));
		assert!(dwell.wake().expect("a fresh wake-up") > first);
		assert_eq!(dwell.ripe(), None);
		// pointing at nothing is how a tip is put away
		assert!(!dwell.point_at(None));
		assert_eq!(dwell.wake(), None);
	}
}
