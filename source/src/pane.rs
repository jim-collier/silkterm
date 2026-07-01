// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use alacritty_terminal::grid::{Dimensions, Scroll as GridScroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::CursorShape;
use glyphon::{Attrs, Buffer, Color as GColor, Shaping, Style, TextArea, TextBounds, Weight};
use winit::event_loop::EventLoopProxy;

use crate::config;
use crate::gfx::RectInstance;
use crate::palette;
use crate::scroll::Scroll;
use crate::term::{PaneId, TermInstance, UserEvent};
use crate::text::{TextCtx, mono_attrs};

// Pane ids must be unique across ALL tabs (each tab is a separate PaneManager),
// not just within one: the shell-exit event carries only the id, so a collision
// closes the wrong tab and cascades. Allocate from one global counter.
static PANE_ID_SEQ: AtomicU64 = AtomicU64::new(1);
fn alloc_pane_id() -> PaneId {
	PANE_ID_SEQ.fetch_add(1, Ordering::Relaxed)
}

// Cursor animation tunables (internal).
const CURSOR_MOVE_TAU_MS: f32 = 55.0; // horizontal slide responsiveness (lower = snappier)
const CURSOR_ALPHA: f32 = 0.55; // solid block-cursor alpha
const BELL_BRIGHTEN: f32 = 0.6; // max lerp of text toward white at the bell flash peak

// The rendered cursor geometry as (width, height) fractions of the cell. An
// app-set Beam/Underline (DECSCUSR) maps to a thin bar / underline; a plain Block
// uses the configured cursor_size_* - except on the alt screen, where the app
// (vim, less, ...) owns a full block.
fn cursor_geometry(shape: CursorShape, alt_screen: bool) -> (f32, f32) {
	match shape {
		CursorShape::Beam => (0.15, 1.0),      // thin vertical bar
		CursorShape::Underline => (1.0, 0.15), // thin bottom strip
		_ if alt_screen => (1.0, 1.0),         // alt-screen app owns its block cursor
		_ => {
			let s = config::settings();
			(
				(s.cursor_size_width / 100.0).clamp(0.02, 1.0), // width, from left
				(s.cursor_size_height / 100.0).clamp(0.02, 1.0), // height, from bottom
			)
		}
	}
}

// Pulse envelope over one cycle: grow, hold full, shrink, then a brief disappear.
fn pulse_env(p: f32) -> f32 {
	let smooth = |t: f32| {
		let t = t.clamp(0.0, 1.0);
		t * t * (3.0 - 2.0 * t)
	};
	if p < 0.40 {
		smooth(p / 0.40) // grow 0 -> 1
	} else if p < 0.60 {
		1.0 // hold at full
	} else if p < 0.90 {
		1.0 - smooth((p - 0.60) / 0.30) // shrink 1 -> 0
	} else {
		0.0 // disappear momentarily
	}
}

// Lerp a text colour toward white by `t` (0..1) of the BELL_BRIGHTEN ceiling, for
// the visual-bell flash. Identity at t<=0.
fn bell_brighten(c: [u8; 3], t: f32) -> [u8; 3] {
	if t <= 0.0 {
		return c;
	}
	let t = (t * BELL_BRIGHTEN).clamp(0.0, 1.0);
	let up = |v: u8| (v as f32 + (255.0 - v as f32) * t).round() as u8;
	[up(c[0]), up(c[1]), up(c[2])]
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
	pub x: f32,
	pub y: f32,
	pub w: f32,
	pub h: f32,
}

impl Rect {
	pub fn contains(&self, x: f32, y: f32) -> bool {
		x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dir {
	// children laid out left | right
	Vertical,
	// children laid out top / bottom
	Horizontal,
}

enum Node {
	Leaf(PaneId),
	Split {
		dir: Dir,
		ratio: f32,
		a: Box<Node>,
		b: Box<Node>,
	},
}

// result of building one pane's frame: text lives in pane.buffer, the
// quads come back here for the shared rect renderer
#[derive(Clone)]
pub struct PaneDraw {
	pub top: f32,
	pub bg: Vec<RectInstance>,
	pub cursor: Option<RectInstance>,
}

pub struct Pane {
	pub id: PaneId,
	pub term: TermInstance,
	pub scroll: Scroll,
	pub buffer: Buffer,
	pub rect: Rect,
	pub title: String,
	pub read_only: bool, // accept no PTY input/paste; selection + copy still work
	// launch argv (None = default shell); a split inherits this so a new pane
	// runs the same shell as the one it forked off (see design.md).
	command: Option<Vec<String>>,
	last_draw: PaneDraw,
	last_history: usize,
	// On-screen row fingerprints from the last build, used only once the
	// scrollback buffer is full (history_size flatlines) to detect that output
	// still scrolled the viewport. See the output-easing note in build().
	last_rows: Vec<u64>,
	// Fallback glyphs (not in the primary mono font) pulled out of `buffer` and
	// drawn one-per-cell so their font advance can't shift the row. `glyph_bufs`
	// is a reused pool; `glyphs` holds (x, y, color, scale) for the first N of
	// them - `scale` shrinks an over-wide fallback glyph to fit its cell box.
	glyph_bufs: Vec<Buffer>,
	glyphs: Vec<(f32, f32, GColor, f32)>,
	// Cursor animation: `cursor_x` (visual column) eases toward the target column
	// so the cursor slides as you type; `blink_t` drives a smooth fade-blink while
	// it sits idle. Snaps on a row change so it doesn't slide diagonally on a newline.
	cursor_x: f32,
	cursor_col: f32,
	cursor_row: i32,
	cursor_init: bool,
	blink_t: f32,
	pub cursor_animating: bool,
	// false until the first full build (and reset on a buffer rebuild). When the
	// frame is a pure cursor animation (no content/scroll/bell change), build skips
	// the expensive text re-shape and reuses the cached buffer/bg/glyphs.
	text_built: bool,
}

impl Pane {
	pub fn build(
		&mut self,
		ctx: &mut TextCtx,
		dt: f32,
		bell: f32,
		force_rebuild: bool,
	) -> PaneDraw {
		let cell_w = ctx.cell_w;
		let cell_h = ctx.cell_h;
		let margin = ctx.margin;
		let content_x = self.rect.x + margin;
		let lines = self.term.lines;
		let s = config::settings(); // snapshot once, not per cell

		// Never block the render thread: the PTY reader thread can hold the
		// terminal lock through long bursts (e.g. a chatty shell rc). If it's
		// busy this frame, reuse the last built frame.
		let mut guard = match self.term.term.try_lock_unfair() {
			Some(g) => g,
			None => return self.last_draw.clone(),
		};

		let cols = self.term.cols;
		let history = guard.grid().history_size();

		// Output easing: nudge the smooth offset when the viewport advanced while
		// following the bottom. Pre-cap, scrollback growth IS the line-advance
		// count (and an in-place status line that uses no newline doesn't grow it,
		// so it doesn't bounce). But once the scrollback buffer fills, history_size
		// flatlines - old lines drop off the top as fast as new ones arrive - so
		// growth reads 0 even though the screen still scrolls. That silently killed
		// smooth output scroll "after a while" (sooner under fast output, which
		// fills the buffer faster). At the cap, fall back to inferring the advance
		// from how far last frame's on-screen rows reappear shifted up this frame;
		// an in-place bottom-row change shifts nothing, so it still won't nudge.
		let grew = history.saturating_sub(self.last_history);
		self.last_history = history;
		self.scroll.set_max(history as f32);
		let follow = self.scroll.following();
		let full = s.scrollback > 0 && history >= s.scrollback;
		let advanced = if grew > 0 {
			grew
		} else if follow && full {
			let mut rows: Vec<u64> = Vec::with_capacity(lines);
			let grid = guard.grid();
			for i in 0..lines as i32 {
				let row = &grid[Line(i)];
				let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a over the row's chars
				for c in 0..cols {
					h = (h ^ row[Column(c)].c as u64).wrapping_mul(0x100_0000_01b3);
				}
				rows.push(h);
			}
			let adv = scroll_shift(&rows, &self.last_rows);
			self.last_rows = rows;
			adv
		} else {
			0
		};
		if advanced > 0 && follow {
			self.scroll.nudge_output(advanced as f32);
		}

		// snap the integer grid offset to the floor of the smooth position
		let desired = self.scroll.desired_offset().min(history);
		let current = guard.grid().display_offset();
		let delta = desired as i32 - current as i32;
		if delta != 0 {
			guard.scroll_display(GridScroll::Delta(delta));
		}

		let frac = self.scroll.frac();
		let d = desired as i32;
		let hist = history as i32;
		// fractional scroll shifts content DOWN by frac of a cell; we render an
		// extra row above (screen row -1) so the revealed strip is filled.
		let y_of = |sr: i32| self.rect.y + margin + (sr as f32 + frac) * cell_h;
		let top = y_of(-1);

		// Cursor position/shape as plain values (no lasting borrow of the lock), so
		// the fast path below can drop the term lock immediately.
		let cursor_pt = guard.grid().cursor.point;
		let cursor_shape = guard.cursor_style().shape;
		// Alt-screen apps own their cursor shape; on the primary screen it's the
		// configured geometry (or the app's DECSCUSR). See cursor_geometry.
		let cgeom = cursor_geometry(cursor_shape, guard.mode().contains(TermMode::ALT_SCREEN));
		let following = desired == 0;

		// Fast path: a pure cursor-animation frame (blink/slide, no content/scroll/
		// bell change). Reuse the cached buffer + glyphs + bg from the last full
		// build and recompute only the cursor - skips set_rich_text + shaping, the
		// expensive part, so a blinking cursor doesn't re-shape text every frame.
		if !force_rebuild && self.text_built {
			drop(guard);
			let cursor = self.cursor_quad(
				cursor_pt,
				cursor_shape,
				cgeom,
				d,
				lines,
				following,
				content_x,
				cell_w,
				cell_h,
				margin,
				frac,
				dt,
				s.cursor,
			);
			let bg = std::mem::take(&mut self.last_draw.bg);
			self.last_draw = PaneDraw { top, bg, cursor };
			return self.last_draw.clone();
		}
		self.text_built = true;

		let colors = guard.colors();
		let sel_range = guard.selection.as_ref().and_then(|s| s.to_range(&*guard));
		let grid = guard.grid();

		let mut bg = Vec::new();
		// fallback glyphs to draw per-cell: (char, fg, bold, italic, col, screen-row, cells)
		let mut glyph_specs: Vec<(char, [u8; 3], bool, bool, usize, i32, u8)> = Vec::new();
		let default_attrs = mono_attrs();

		// Build attr-runs spanning the viewport (+1 overscan row). Newlines are
		// embedded into runs (never empty/standalone spans) - empty spans make
		// cosmic-text's set_rich_text loop forever.
		let mut spans: Vec<(String, Attrs)> = Vec::with_capacity(lines + 1);
		let mut run = String::new();
		let mut run_color = s.fg;
		let mut run_bold = false;
		let mut run_italic = false;

		macro_rules! flush_run {
			() => {
				if !run.is_empty() {
					let mut a = mono_attrs();
					a.color_opt = Some(GColor::rgb(run_color[0], run_color[1], run_color[2]));
					if run_bold {
						a.weight = Weight::BOLD;
					}
					if run_italic {
						a.style = Style::Italic;
					}
					spans.push((std::mem::take(&mut run), a));
				}
			};
		}

		for sr in -1..(lines as i32) {
			if sr != -1 {
				run.push('\n');
			}
			let gl = sr - d; // grid line for this screen row
			if gl < -hist || gl > (lines as i32 - 1) {
				continue; // off the top/bottom of real content: blank row
			}
			let row = &grid[Line(gl)];
			let y = y_of(sr);
			for c in 0..cols {
				let cell = &row[Column(c)];
				let flags = cell.flags;
				if flags.contains(Flags::WIDE_CHAR_SPACER) {
					continue;
				}
				let mut fg = palette::resolve(cell.fg, colors);
				let mut cell_bg = palette::resolve(cell.bg, colors);
				if flags.contains(Flags::INVERSE) {
					std::mem::swap(&mut fg, &mut cell_bg);
				}
				if flags.contains(Flags::HIDDEN) {
					fg = cell_bg;
				}
				if flags.contains(Flags::DIM) {
					fg = [
						fg[0] / 2 + fg[0] / 4,
						fg[1] / 2 + fg[1] / 4,
						fg[2] / 2 + fg[2] / 4,
					];
				}
				if bell > 0.0 {
					fg = bell_brighten(fg, bell); // visual-bell flash
				}

				let selected =
					sel_range.is_some_and(|r| r.contains(Point::new(Line(gl), Column(c))));
				let bg_color = if selected {
					Some(config::SELECTION_BG)
				} else if cell_bg != s.bg {
					Some(cell_bg)
				} else {
					None
				};
				if let Some(col) = bg_color {
					bg.push(RectInstance {
						pos: [content_x + c as f32 * cell_w, y],
						size: [cell_w, cell_h],
						color: config::srgb_f32(col),
					});
				}

				let bold = flags.contains(Flags::BOLD);
				let italic = flags.contains(Flags::ITALIC);
				// A glyph the primary mono font lacks renders via a fallback font
				// whose advance may not equal the grid width, drifting the rest of
				// the row. Pull it out, draw it per-cell, leave space placeholders.
				if !cell.c.is_ascii() && !ctx.covered(cell.c) {
					let w = if flags.contains(Flags::WIDE_CHAR) {
						2
					} else {
						1
					};
					for _ in 0..w {
						run.push(' ');
					}
					glyph_specs.push((cell.c, fg, bold, italic, c, sr, w as u8));
				} else {
					if (fg, bold, italic) != (run_color, run_bold, run_italic) {
						flush_run!();
						run_color = fg;
						run_bold = bold;
						run_italic = italic;
					}
					run.push(cell.c);
				}
			}
		}
		flush_run!();

		drop(guard);
		let cursor = self.cursor_quad(
			cursor_pt,
			cursor_shape,
			cgeom,
			d,
			lines,
			following,
			content_x,
			cell_w,
			cell_h,
			margin,
			frac,
			dt,
			s.cursor,
		);
		let span_refs = spans.iter().map(|(s, a)| (s.as_str(), a.clone()));
		// Advanced (not Basic) so missing glyphs fall back to other fonts
		// (CJK/emoji/math/RTL) instead of rendering tofu. cosmic-text 0.18.2's
		// fallback loop is bounded and keeps monospace alignment; earlier 0.18
		// could hang here (see git history) but no longer does (stress-tested).
		self.buffer.set_rich_text(
			&mut ctx.font_system,
			span_refs,
			&default_attrs,
			Shaping::Advanced,
			None,
		);
		self.buffer.shape_until_scroll(&mut ctx.font_system, false);

		// build the per-cell fallback glyphs (reusing the buffer pool)
		self.glyphs.clear();
		let rect_y = self.rect.y;
		for (i, (ch, color, bold, italic, c, sr, cells)) in glyph_specs.into_iter().enumerate() {
			let mut a = mono_attrs();
			a.color_opt = Some(GColor::rgb(color[0], color[1], color[2]));
			if bold {
				a.weight = Weight::BOLD;
			}
			if italic {
				a.style = Style::Italic;
			}
			if i >= self.glyph_bufs.len() {
				let b = ctx.new_plain_buffer();
				self.glyph_bufs.push(b);
			}
			let (ink_w, ink_off) = ctx.fill_glyph(&mut self.glyph_bufs[i], ch, &a);
			// Fit the ink inside its cell box (cells * cell_w wide), only ever
			// shrinking, and center it there - a fallback face's wider-than-a-cell
			// ink would otherwise spill over the next cell and collide with its
			// text. Back out the ink offset so centering is on the ink, not the pen.
			let target = cells as f32 * cell_w;
			let scale = if ink_w > target { target / ink_w } else { 1.0 };
			let cell_x = content_x + c as f32 * cell_w;
			let x = cell_x + (target - ink_w * scale) / 2.0 - ink_off * scale;
			let y = rect_y + margin + (sr as f32 + frac) * cell_h + cell_h * (1.0 - scale) / 2.0;
			self.glyphs
				.push((x, y, GColor::rgb(color[0], color[1], color[2]), scale));
		}

		self.last_draw = PaneDraw { top, bg, cursor };
		self.last_draw.clone()
	}

	// The cursor quad: visual column eased toward the target (slides as you type,
	// snaps on a row change), fade-blink alpha when idle, or None when hidden /
	// scrolled into history. Cheap; called every frame (incl. the cursor-only fast
	// path). Must run after the term lock is dropped (it takes &mut self).
	#[allow(clippy::too_many_arguments)]
	fn cursor_quad(
		&mut self,
		cursor_pt: Point,
		cursor_shape: CursorShape,
		geom: (f32, f32),
		d: i32,
		lines: usize,
		following: bool,
		content_x: f32,
		cell_w: f32,
		cell_h: f32,
		margin: f32,
		frac: f32,
		dt: f32,
		cursor_rgb: [u8; 3],
	) -> Option<RectInstance> {
		let cursor_sr = cursor_pt.line.0 + d;
		let shown = following
			&& cursor_shape != CursorShape::Hidden
			&& cursor_sr >= 0
			&& (cursor_sr as usize) < lines;
		self.cursor_animating = false;
		if !shown {
			return None;
		}
		let c = cursor_pt.column.0 as f32;
		let row_jump = !self.cursor_init || cursor_sr != self.cursor_row;
		let moved = row_jump || (c - self.cursor_col).abs() > 0.001;
		if row_jump {
			self.cursor_x = c; // snap on first sight / newline (no diagonal slide)
		}
		if moved {
			self.blink_t = 0.0; // solid immediately after any move
		}
		self.cursor_init = true;
		self.cursor_row = cursor_sr;
		self.cursor_col = c;
		let k = 1.0 - (-dt * 1000.0 / CURSOR_MOVE_TAU_MS).exp();
		self.cursor_x += (c - self.cursor_x) * k;
		let easing = (c - self.cursor_x).abs() > 0.01;
		if !easing {
			self.cursor_x = c;
		}
		self.blink_t += dt;
		// Animation: "none" = steady; "phase" = smooth cosine fade; "pulse_*" =
		// grow/shrink a dimension over one cycle. Always solid while sliding (it
		// starts solid right after a move). One cycle = the blink rate.
		let settings = config::settings();
		let anim = settings.cursor_animation.as_str();
		let period = (settings.cursor_blink_rate_ms / 1000.0 * 2.0).max(0.05); // full on->off->on
		let animating = !easing && anim != "none";
		let phase = (self.blink_t / period).fract();

		let (mut w_frac, mut h_frac) = geom;
		let mut alpha = CURSOR_ALPHA;
		let (pulsing_w, pulsing_h) = if animating {
			match anim {
				"phase" => {
					alpha = CURSOR_ALPHA * (0.5 + 0.5 * (phase * std::f32::consts::TAU).cos());
					(false, false)
				}
				"pulse_vertical" => {
					h_frac *= pulse_env(phase);
					(false, true)
				}
				"pulse_horizontal" => {
					w_frac *= pulse_env(phase);
					(true, false)
				}
				"pulse_both" => {
					let e = pulse_env(phase);
					w_frac *= e;
					h_frac *= e;
					(true, true)
				}
				_ => (false, false),
			}
		} else {
			(false, false)
		};
		self.cursor_animating = easing || animating;
		let mut col = config::srgb_f32(cursor_rgb);
		col[3] = alpha;
		let cell_y = self.rect.y + margin + (cursor_sr as f32 + frac) * cell_h;
		let cell_x = content_x + self.cursor_x * cell_w;
		// Width grows from the left, height from the bottom - but a *pulsing*
		// dimension grows from the cell centre (the "line in the middle") and may
		// shrink to nothing (the momentary disappear), so it skips the 2px floor.
		let w = if pulsing_w {
			cell_w * w_frac
		} else {
			(cell_w * w_frac).max(2.0)
		};
		let h = if pulsing_h {
			cell_h * h_frac
		} else {
			(cell_h * h_frac).max(2.0)
		};
		let x = if pulsing_w {
			cell_x + (cell_w - w) / 2.0
		} else {
			cell_x
		};
		let y = if pulsing_h {
			cell_y + (cell_h - h) / 2.0
		} else {
			cell_y + cell_h - h
		};
		Some(RectInstance {
			pos: [x, y],
			size: [w, h],
			color: col,
		})
	}

	pub fn text_area<'a>(&'a self, top: f32, margin: f32) -> TextArea<'a> {
		TextArea {
			buffer: &self.buffer,
			left: self.rect.x + margin,
			top,
			scale: 1.0,
			// clip to the content area (pane inset by the margin)
			bounds: TextBounds {
				left: (self.rect.x + margin) as i32,
				top: (self.rect.y + margin) as i32,
				right: (self.rect.x + self.rect.w - margin) as i32,
				bottom: (self.rect.y + self.rect.h - margin) as i32,
			},
			default_color: GColor::rgb(
				config::settings().fg[0],
				config::settings().fg[1],
				config::settings().fg[2],
			),
			custom_glyphs: &[],
		}
	}

	// Per-cell fallback glyphs, already positioned (see Pane::build). Drawn in
	// the same text pass as `text_area`, on top of their space placeholders.
	pub fn glyph_areas(&self) -> Vec<TextArea<'_>> {
		self.glyphs
			.iter()
			.zip(&self.glyph_bufs)
			.map(|(&(x, y, color, scale), buf)| TextArea {
				buffer: buf,
				left: x,
				top: y,
				scale,
				bounds: TextBounds {
					left: self.rect.x as i32,
					top: self.rect.y as i32,
					right: (self.rect.x + self.rect.w) as i32,
					bottom: (self.rect.y + self.rect.h) as i32,
				},
				default_color: color,
				custom_glyphs: &[],
			})
			.collect()
	}

	// Map a window pixel to a grid point + which half of the cell, for selection.
	// Returns None if the pixel is outside this pane.
	pub fn point_at(&self, x: f32, y: f32, ctx: &TextCtx) -> Option<(Point, Side)> {
		if !self.rect.contains(x, y) {
			return None;
		}
		let cols = self.term.cols as i32;
		let lines = self.term.lines as i32;
		let rel_x = (x - self.rect.x - ctx.margin).max(0.0);
		let colf = (rel_x / ctx.cell_w).floor();
		let col = (colf as i32).clamp(0, cols - 1);
		let side = if rel_x - colf * ctx.cell_w < ctx.cell_w / 2.0 {
			Side::Left
		} else {
			Side::Right
		};
		let sr = ((y - self.rect.y - ctx.margin) / ctx.cell_h)
			.floor()
			.clamp(0.0, (lines - 1) as f32) as i32;
		let d = self.term.term.lock().grid().display_offset() as i32;
		Some((Point::new(Line(sr - d), Column(col as usize)), side))
	}

	// If a double-click `point` sits inside a matched pair on its line, return
	// the inside span (start..=end, same line) of the highest-precedence
	// enclosing non-empty pair. Single line only (multi-line pairs aren't
	// handled). `pairs` is (open, close) in precedence order.
	pub fn pair_span(&self, point: Point, pairs: &[(char, char)]) -> Option<(Point, Point)> {
		let cols = self.term.cols;
		let col = point.column.0;
		if col >= cols {
			return None;
		}
		let row: Vec<char> = {
			let t = self.term.term.lock();
			let grid = t.grid();
			(0..cols).map(|c| grid[point.line][Column(c)].c).collect()
		};
		let (start, end) = pair_inside(&row, col, pairs)?;
		Some((
			Point::new(point.line, Column(start)),
			Point::new(point.line, Column(end)),
		))
	}

	pub fn begin_selection(&self, point: Point, side: Side, ty: SelectionType) {
		self.term.term.lock().selection = Some(Selection::new(ty, point, side));
	}

	pub fn update_selection(&self, point: Point, side: Side) {
		let mut t = self.term.term.lock();
		if let Some(sel) = t.selection.as_mut() {
			sel.update(point, side);
		}
	}

	pub fn clear_selection(&self) {
		self.term.term.lock().selection = None;
	}

	pub fn selection_text(&self) -> Option<String> {
		self.term
			.term
			.lock()
			.selection_to_string()
			.filter(|s| !s.is_empty())
	}

	// Write pasted text to the PTY (wrapped in bracketed paste when the app
	// enabled it). No-op when the pane is read-only.
	pub fn paste(&self, text: &str) {
		if self.read_only || text.is_empty() {
			return;
		}
		let bracket = self.term.mode().contains(TermMode::BRACKETED_PASTE);
		let mut bytes = Vec::with_capacity(text.len() + 12);
		if bracket {
			bytes.extend_from_slice(b"\x1b[200~");
		}
		bytes.extend_from_slice(text.as_bytes());
		if bracket {
			bytes.extend_from_slice(b"\x1b[201~");
		}
		self.term.write(bytes);
	}
}

pub struct PaneManager {
	pub panes: HashMap<PaneId, Pane>,
	root: Node,
	pub focused: PaneId,
	// CLI `--title` for this tab; overrides the computed "<shell> [program]".
	pub title_override: Option<String>,
}

impl PaneManager {
	pub fn new(
		ctx: &mut TextCtx,
		proxy: &EventLoopProxy<UserEvent>,
		area: Rect,
		command: Option<Vec<String>>,
	) -> anyhow::Result<Self> {
		let id = alloc_pane_id();
		let pane = spawn_pane(ctx, proxy, id, area, command)?;
		let mut panes = HashMap::new();
		panes.insert(id, pane);
		Ok(Self {
			panes,
			root: Node::Leaf(id),
			focused: id,
			title_override: None,
		})
	}

	// Interactive split (menu/keyboard): even ratio, new pane after; inherits the
	// source pane's command so the new pane runs the same shell it forked off.
	pub fn split(
		&mut self,
		ctx: &mut TextCtx,
		proxy: &EventLoopProxy<UserEvent>,
		id: PaneId,
		dir: Dir,
		area: Rect,
	) -> anyhow::Result<()> {
		let cmd = self.panes.get(&id).and_then(|p| p.command.clone());
		self.split_at(ctx, proxy, id, dir, false, 0.5, cmd, area);
		Ok(())
	}

	// General split used by the CLI: split `id` along `dir`, the new pane on the
	// `before` side (a) or after (b), taking `new_ratio` of the split; runs
	// `command`. Returns the new pane id (None if `id` wasn't a leaf).
	pub fn split_at(
		&mut self,
		ctx: &mut TextCtx,
		proxy: &EventLoopProxy<UserEvent>,
		id: PaneId,
		dir: Dir,
		before: bool,
		new_ratio: f32,
		command: Option<Vec<String>>,
		area: Rect,
	) -> Option<PaneId> {
		// leaves mirror `panes`, so this is also "is id a leaf" - checked up
		// front so a doomed insert can't spawn (then kill) a shell
		if !self.panes.contains_key(&id) {
			return None;
		}
		let new_id = alloc_pane_id();
		// spawn BEFORE touching the tree: a failed spawn must not leave a
		// phantom leaf that reserves layout space with no pane behind it
		let pane = match spawn_pane(ctx, proxy, new_id, area, command) {
			Ok(p) => p,
			Err(e) => {
				eprintln!("split: failed to spawn shell: {e}");
				return None;
			}
		};
		// child-a's ratio: if the new pane is 'a' (before) it takes new_ratio,
		// else 'a' is the old pane and keeps the remainder.
		let ratio_a = if before { new_ratio } else { 1.0 - new_ratio };
		if !insert_split_at(
			&mut self.root,
			id,
			dir,
			new_id,
			before,
			ratio_a.clamp(0.05, 0.95),
		) {
			return None;
		}
		self.panes.insert(new_id, pane);
		self.focused = new_id;
		self.relayout(ctx, area);
		Some(new_id)
	}

	// returns true when the last pane closed (caller should exit)
	pub fn close(&mut self, ctx: &mut TextCtx, id: PaneId, area: Rect) -> bool {
		match prune(std::mem::replace(&mut self.root, Node::Leaf(0)), id) {
			Some(n) => {
				self.root = n;
				self.panes.remove(&id);
				if self.focused == id {
					self.focused = first_leaf(&self.root);
				}
				self.relayout(ctx, area);
				false
			}
			None => {
				self.panes.remove(&id);
				true
			}
		}
	}

	// Recreate each pane's text buffer from `ctx`'s font system. Needed after a
	// TextCtx rebuild (font size / line height change) since buffers are tied to
	// the FontSystem they were made with. Follow with `relayout`.
	pub fn rebuild_buffers(&mut self, ctx: &mut TextCtx) {
		for pane in self.panes.values_mut() {
			pane.buffer = ctx.new_buffer(pane.rect.w.max(1.0), pane.rect.h.max(1.0));
			pane.text_built = false; // fresh empty buffer: force a full rebuild next frame
		}
	}

	pub fn relayout(&mut self, ctx: &mut TextCtx, area: Rect) {
		let mut out = Vec::new();
		layout(&self.root, area, &mut out);
		for (id, rect) in out {
			if let Some(pane) = self.panes.get_mut(&id) {
				pane.rect = rect;
				let (cw, ch, cols, lines) = content_dims(rect, ctx);
				pane.term
					.resize(cols, lines, ctx.cell_w as u16, ctx.cell_h as u16);
				// `build` lays out lines+1 rows (the -1 overscan row above the
				// viewport plus rows 0..lines-1) into this buffer; the last row
				// sits at y=lines*cell_h. When `ch` is an exact multiple of
				// cell_h (the default window size hits this), that's right at the
				// buffer's height and cosmic-text drops the row - the bottom line
				// goes invisible until you scroll/resize. Give it overscan slack;
				// TextArea bounds still clip drawing to the pane.
				ctx.resize_buffer(&mut pane.buffer, cw, ch + 2.0 * ctx.cell_h);
			}
		}
	}

	pub fn pane_at(&self, x: f32, y: f32) -> Option<PaneId> {
		self.panes
			.iter()
			.find(|(_, p)| p.rect.contains(x, y))
			.map(|(id, _)| *id)
	}

	// A grabbable divider under the cursor: its path in the split-tree and
	// orientation (for the resize cursor).
	pub fn divider_at(&self, x: f32, y: f32, area: Rect) -> Option<(Vec<bool>, Dir)> {
		let mut path = Vec::new();
		divider_at(&self.root, area, x, y, &mut path).map(|dir| (path, dir))
	}

	// Drag a divider (identified by `path`) to the cursor and relayout.
	pub fn drag_divider(&mut self, ctx: &mut TextCtx, path: &[bool], area: Rect, x: f32, y: f32) {
		set_ratio(&mut self.root, area, path, x, y);
		self.relayout(ctx, area);
	}

	// Swap two panes' positions in the split-tree (drag-and-drop reorder).
	pub fn swap_panes(&mut self, ctx: &mut TextCtx, a: PaneId, b: PaneId, area: Rect) {
		if a == b {
			return;
		}
		swap_leaves(&mut self.root, a, b);
		self.relayout(ctx, area);
	}
}

fn swap_leaves(node: &mut Node, a: PaneId, b: PaneId) {
	match node {
		Node::Leaf(id) => {
			if *id == a {
				*id = b;
			} else if *id == b {
				*id = a;
			}
		}
		Node::Split { a: l, b: r, .. } => {
			swap_leaves(l, a, b);
			swap_leaves(r, a, b);
		}
	}
}

// content area (pane inset by the margin) in pixels and in cells
fn content_dims(rect: Rect, ctx: &TextCtx) -> (f32, f32, usize, usize) {
	let cw = (rect.w - 2.0 * ctx.margin).max(ctx.cell_w);
	let ch = (rect.h - 2.0 * ctx.margin).max(ctx.cell_h);
	let cols = (cw / ctx.cell_w).floor().max(1.0) as usize;
	let lines = (ch / ctx.cell_h).floor().max(1.0) as usize;
	(cw, ch, cols, lines)
}

fn spawn_pane(
	ctx: &mut TextCtx,
	proxy: &EventLoopProxy<UserEvent>,
	id: PaneId,
	rect: Rect,
	command: Option<Vec<String>>,
) -> anyhow::Result<Pane> {
	let (cw, ch, cols, lines) = content_dims(rect, ctx);
	let term = TermInstance::spawn(
		id,
		cols,
		lines,
		ctx.cell_w as u16,
		ctx.cell_h as u16,
		proxy.clone(),
		command.clone(),
	)?;
	// +2 cells of height for the overscan rows build() renders (see relayout).
	let buffer = ctx.new_buffer(cw, ch + 2.0 * ctx.cell_h);
	Ok(Pane {
		id,
		term,
		scroll: Scroll::new(),
		buffer,
		rect,
		title: config::APP_NAME.into(),
		read_only: false,
		command,
		last_draw: PaneDraw {
			top: rect.y,
			bg: Vec::new(),
			cursor: None,
		},
		last_history: 0,
		last_rows: Vec::new(),
		glyph_bufs: Vec::new(),
		glyphs: Vec::new(),
		cursor_x: 0.0,
		cursor_col: 0.0,
		cursor_row: i32::MIN,
		cursor_init: false,
		blink_t: 0.0,
		cursor_animating: false,
		text_built: false,
	})
}

// Inside span (start..=end columns) of the highest-precedence matched pair that
// encloses `col` on `row`. `pairs` is (open, close) in precedence order; the
// first enclosing non-empty pair wins (so e.g. inside `()` selects the `()`
// contents even if a lower-precedence `[]` is nested within). None -> no pair.
fn pair_inside(row: &[char], col: usize, pairs: &[(char, char)]) -> Option<(usize, usize)> {
	for &(open, close) in pairs {
		let found = if open == close {
			same_char_pair(row, col, open)
		} else {
			distinct_pair(row, col, open, close)
		};
		if let Some((o, c)) = found {
			if c > o + 1 {
				// Exclude runs of spaces directly against the delimiters (keep any
				// interior spaces): `" Now is the time. "` selects `Now is the time.`.
				let (mut s, mut e) = (o + 1, c - 1);
				while s < e && row[s] == ' ' {
					s += 1;
				}
				while e > s && row[e] == ' ' {
					e -= 1;
				}
				// all-spaces inside: fall back to the full inside span
				return Some(if row[s] == ' ' {
					(o + 1, c - 1)
				} else {
					(s, e)
				});
			}
		}
	}
	None
}

// Innermost matched (open,close) pair enclosing `col` on `row`, for distinct
// open/close chars. The char at `col` itself isn't treated as an endpoint.
fn distinct_pair(row: &[char], col: usize, open: char, close: char) -> Option<(usize, usize)> {
	let mut depth = 0i32;
	let mut o = None;
	for i in (0..col).rev() {
		if row[i] == close {
			depth += 1;
		} else if row[i] == open {
			if depth == 0 {
				o = Some(i);
				break;
			}
			depth -= 1;
		}
	}
	let o = o?;
	let mut depth = 0i32;
	for (i, &ch) in row.iter().enumerate().skip(col + 1) {
		if ch == open {
			depth += 1;
		} else if ch == close {
			if depth == 0 {
				return Some((o, i));
			}
			depth -= 1;
		}
	}
	None
}

// Pair of identical chars (quotes) enclosing `col`: occurrences pair off
// left-to-right; `col` is inside the pair strictly between two of them.
fn same_char_pair(row: &[char], col: usize, ch: char) -> Option<(usize, usize)> {
	let pos: Vec<usize> = row
		.iter()
		.enumerate()
		.filter(|&(_, &c)| c == ch)
		.map(|(i, _)| i)
		.collect();
	let mut i = 0;
	while i + 1 < pos.len() {
		if pos[i] < col && col < pos[i + 1] {
			return Some((pos[i], pos[i + 1]));
		}
		i += 2;
	}
	None
}

// Split the leaf `id` into a `dir` Split. `before` puts the new pane on the
// a-side (left/top); `ratio_a` is child-a's fraction of the split.
fn insert_split_at(
	node: &mut Node,
	id: PaneId,
	dir: Dir,
	new_id: PaneId,
	before: bool,
	ratio_a: f32,
) -> bool {
	match node {
		Node::Leaf(i) if *i == id => {
			let old = *i;
			let (a, b) = if before { (new_id, old) } else { (old, new_id) };
			*node = Node::Split {
				dir,
				ratio: ratio_a,
				a: Box::new(Node::Leaf(a)),
				b: Box::new(Node::Leaf(b)),
			};
			true
		}
		Node::Leaf(_) => false,
		Node::Split { a, b, .. } => {
			insert_split_at(a, id, dir, new_id, before, ratio_a)
				|| insert_split_at(b, id, dir, new_id, before, ratio_a)
		}
	}
}

fn prune(node: Node, id: PaneId) -> Option<Node> {
	match node {
		Node::Leaf(i) if i == id => None,
		Node::Leaf(i) => Some(Node::Leaf(i)),
		Node::Split { dir, ratio, a, b } => {
			let a2 = prune(*a, id);
			let b2 = prune(*b, id);
			match (a2, b2) {
				(Some(a), Some(b)) => Some(Node::Split {
					dir,
					ratio,
					a: Box::new(a),
					b: Box::new(b),
				}),
				(Some(n), None) | (None, Some(n)) => Some(n),
				(None, None) => None,
			}
		}
	}
}

fn first_leaf(node: &Node) -> PaneId {
	match node {
		Node::Leaf(id) => *id,
		Node::Split { a, .. } => first_leaf(a),
	}
}

fn layout(node: &Node, area: Rect, out: &mut Vec<(PaneId, Rect)>) {
	match node {
		Node::Leaf(id) => out.push((*id, area)),
		Node::Split { dir, ratio, a, b } => {
			let (a_area, b_area) = child_areas(area, *dir, *ratio);
			layout(a, a_area, out);
			layout(b, b_area, out);
		}
	}
}

// The two child rects of a split, with the gap strip between them.
fn child_areas(area: Rect, dir: Dir, ratio: f32) -> (Rect, Rect) {
	let gap = config::PANE_GAP_PX;
	match dir {
		Dir::Vertical => {
			let wa = ((area.w - gap) * ratio).floor();
			(
				Rect {
					x: area.x,
					y: area.y,
					w: wa,
					h: area.h,
				},
				Rect {
					x: area.x + wa + gap,
					y: area.y,
					w: area.w - gap - wa,
					h: area.h,
				},
			)
		}
		Dir::Horizontal => {
			let ha = ((area.h - gap) * ratio).floor();
			(
				Rect {
					x: area.x,
					y: area.y,
					w: area.w,
					h: ha,
				},
				Rect {
					x: area.x,
					y: area.y + ha + gap,
					w: area.w,
					h: area.h - gap - ha,
				},
			)
		}
	}
}

// Find the split whose divider is under (x, y), within a grab tolerance.
// Returns a path of child choices (false = a, true = b) from the root to that
// split, plus its orientation (for the resize cursor).
fn divider_at(node: &Node, area: Rect, x: f32, y: f32, path: &mut Vec<bool>) -> Option<Dir> {
	let Node::Split { dir, ratio, a, b } = node else {
		return None;
	};
	let (a_area, b_area) = child_areas(area, *dir, *ratio);
	let tol = config::DIVIDER_GRAB_PX;
	let on_divider =
		match dir {
			Dir::Vertical => {
				x >= a_area.x + a_area.w - tol
					&& x <= b_area.x + tol
					&& y >= area.y && y <= area.y + area.h
			}
			Dir::Horizontal => {
				y >= a_area.y + a_area.h - tol
					&& y <= b_area.y + tol
					&& x >= area.x && x <= area.x + area.w
			}
		};
	if on_divider {
		return Some(*dir);
	}
	if a_area.contains(x, y) {
		path.push(false);
		if let Some(d) = divider_at(a, a_area, x, y, path) {
			return Some(d);
		}
		path.pop();
	}
	if b_area.contains(x, y) {
		path.push(true);
		if let Some(d) = divider_at(b, b_area, x, y, path) {
			return Some(d);
		}
		path.pop();
	}
	None
}

// Walk `path` to a split node and set its ratio from the mouse position.
fn set_ratio(node: &mut Node, area: Rect, path: &[bool], x: f32, y: f32) {
	let Node::Split { dir, ratio, a, b } = node else {
		return;
	};
	if let [first, rest @ ..] = path {
		let (a_area, b_area) = child_areas(area, *dir, *ratio);
		if *first {
			set_ratio(b, b_area, rest, x, y);
		} else {
			set_ratio(a, a_area, rest, x, y);
		}
		return;
	}
	let gap = config::PANE_GAP_PX;
	let r = match dir {
		Dir::Vertical => (x - area.x) / (area.w - gap),
		Dir::Horizontal => (y - area.y) / (area.h - gap),
	};
	*ratio = r.clamp(0.05, 0.95);
}

// Lines the on-screen content scrolled up between frames, inferred from row
// fingerprints when scrollback growth can't tell us (the buffer is full). It's
// the smallest shift k where this frame's top (rows-k) lines equal last frame's
// bottom (rows-k) lines.
fn scroll_shift(cur: &[u64], last: &[u64]) -> usize {
	let n = cur.len();
	if n == 0 || last.len() != n {
		return 0;
	}
	for k in 1..n {
		if cur[..n - k] == last[k..] {
			return k;
		}
	}
	// No clean vertical shift matched. Either nothing scrolled - an in-place
	// change, e.g. a status line redrawn with no newline (don't nudge: that was
	// the apt-bounce hazard) - or the screen turned over completely in one fast
	// burst. The top line is the tell: unchanged => nothing scrolled; changed =>
	// a full-screen burst, so report the backlog cap to ramp to full catch-up.
	if cur[0] == last[0] {
		0
	} else {
		crate::scroll::MAX_BACKLOG as usize
	}
}

#[cfg(test)]
mod tests {
	use super::{bell_brighten, distinct_pair, pair_inside, same_char_pair, scroll_shift};

	// default pairs in precedence order: backtick, ", ', {}, (), [], <>
	const PAIRS: &[(char, char)] = &[
		('`', '`'),
		('"', '"'),
		('\'', '\''),
		('{', '}'),
		('(', ')'),
		('[', ']'),
		('<', '>'),
	];

	fn row(s: &str) -> Vec<char> {
		s.chars().collect()
	}

	#[test]
	fn distinct_innermost() {
		let r = row("a (b [c] d) e");
		// click on 'c' (index 6): [] is inner, () is the outer
		assert_eq!(distinct_pair(&r, 6, '[', ']'), Some((5, 7)));
		assert_eq!(distinct_pair(&r, 6, '(', ')'), Some((2, 10)));
	}

	#[test]
	fn precedence_paren_over_bracket() {
		let r = row("a (b [c] d) e");
		// inside both () and []; () has higher precedence -> select () contents
		// contents columns are 3..=9 ("b [c] d")
		assert_eq!(pair_inside(&r, 6, PAIRS), Some((3, 9)));
		assert_eq!(r[3..=9].iter().collect::<String>(), "b [c] d");
	}

	#[test]
	fn bracket_only() {
		let r = row("x [y] z");
		assert_eq!(pair_inside(&r, 3, PAIRS), Some((3, 3))); // just "y"
	}

	#[test]
	fn quotes_pair_left_to_right() {
		let r = row(r#"say "hello world" now"#);
		// click inside the quotes (e.g. index 8)
		assert_eq!(same_char_pair(&r, 8, '"'), Some((4, 16)));
		let (s, e) = pair_inside(&r, 8, PAIRS).unwrap();
		assert_eq!(r[s..=e].iter().collect::<String>(), "hello world");
	}

	#[test]
	fn quote_beats_paren() {
		let r = row(r#"(a "b" c)"#);
		// inside both () and ""; "" higher precedence -> "b"
		let (s, e) = pair_inside(&r, 4, PAIRS).unwrap();
		assert_eq!(r[s..=e].iter().collect::<String>(), "b");
	}

	#[test]
	fn outside_any_pair() {
		let r = row("just words here");
		assert_eq!(pair_inside(&r, 5, PAIRS), None);
	}

	#[test]
	fn empty_pair_skipped() {
		// click between empty () - nothing inside, so no pair selection
		let r = row("a () b");
		assert_eq!(pair_inside(&r, 2, PAIRS), None);
	}

	#[test]
	fn pair_trims_adjacent_spaces() {
		// spaces directly inside the delimiters are excluded; interior spaces kept
		let r = row(r#" " Now is the time. " "#);
		let (s, e) = pair_inside(&r, 6, PAIRS).unwrap();
		assert_eq!(r[s..=e].iter().collect::<String>(), "Now is the time.");
		// brackets too
		let r2 = row("a [   hi   ] b");
		let (s, e) = pair_inside(&r2, 6, PAIRS).unwrap();
		assert_eq!(r2[s..=e].iter().collect::<String>(), "hi");
		// all-spaces inside: nothing to trim to, keep the full inside span
		let r3 = row("a (   ) b");
		let (s, e) = pair_inside(&r3, 4, PAIRS).unwrap();
		assert_eq!(r3[s..=e].iter().collect::<String>(), "   ");
	}

	#[test]
	fn on_open_char_uses_outer() {
		let r = row("(a [b] c)");
		// click exactly on '[' (index 3): not inside [], but inside () -> () contents
		let (s, e) = pair_inside(&r, 3, PAIRS).unwrap();
		assert_eq!(r[s..=e].iter().collect::<String>(), "a [b] c");
	}

	// scroll_shift: row fingerprints are arbitrary u64s; a shift up by k means the
	// new top (n-k) rows equal the old bottom (n-k) rows.
	const CAP: usize = crate::scroll::MAX_BACKLOG as usize;

	#[test]
	fn shift_none_when_unchanged() {
		let f = [10, 20, 30, 40, 50];
		assert_eq!(scroll_shift(&f, &f), 0);
	}

	#[test]
	fn shift_in_place_bottom_change_does_not_count() {
		// only the last row changed (an in-place status line) - no scroll
		let last = [10, 20, 30, 40, 50];
		let cur = [10, 20, 30, 40, 99];
		assert_eq!(scroll_shift(&cur, &last), 0);
	}

	#[test]
	fn shift_by_one() {
		let last = [10, 20, 30, 40, 50];
		let cur = [20, 30, 40, 50, 60]; // scrolled up one, new line 60 at bottom
		assert_eq!(scroll_shift(&cur, &last), 1);
	}

	#[test]
	fn shift_by_three() {
		let last = [10, 20, 30, 40, 50];
		let cur = [40, 50, 60, 70, 80];
		assert_eq!(scroll_shift(&cur, &last), 3);
	}

	#[test]
	fn shift_full_turnover_reports_cap() {
		// no overlap at all (a fast burst replaced the whole screen)
		let last = [10, 20, 30, 40, 50];
		let cur = [60, 70, 80, 90, 100];
		assert_eq!(scroll_shift(&cur, &last), CAP);
	}

	#[test]
	fn shift_empty_or_mismatched_is_zero() {
		assert_eq!(scroll_shift(&[], &[]), 0);
		assert_eq!(scroll_shift(&[1, 2, 3], &[1, 2]), 0);
	}

	#[test]
	fn bell_brighten_lightens_and_is_identity_at_zero() {
		let c = [100, 120, 140];
		assert_eq!(bell_brighten(c, 0.0), c); // no flash -> unchanged
		let b = bell_brighten(c, 1.0);
		assert!(b[0] > c[0] && b[1] > c[1] && b[2] > c[2]); // peak flash brightens
		assert!(b.iter().zip(&c).all(|(&n, &o)| n >= o)); // never darkens
	}
}
