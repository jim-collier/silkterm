// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use alacritty_terminal::grid::{Dimensions, Scroll as GridScroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::Term;
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
		// true once the user has dragged this divider: auto even-distribution stops
		// for its same-direction run (successive splits there stay 50/50).
		manual: bool,
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
	// Retained-frame app-scroll slide (None = common case: whole pane at `top`).
	// While a full-screen app's scroll eases, the current frame draws shifted at
	// `top` and the previous frame (pane.prev_buffer) fills the revealed strip.
	pub slide: Option<Slide>,
}

// One frame of an easing app-scroll slide. The current frame renders at
// `PaneDraw.top` (the scroll region, clipped above `split_y`); the previous
// frame renders at `prev_top` clipped to `[prev_clip_t, prev_clip_b]` (just the
// revealed strip, so its static band can't ghost into the scroll region); and a
// fixed bottom band (status/input line), when `has_band`, redraws unshifted at
// `band_top` below `split_y`.
#[derive(Clone)]
pub struct Slide {
	pub prev_top: f32,
	pub prev_clip_t: f32,
	pub prev_clip_b: f32,
	pub split_y: f32,
	pub band_top: f32,
	pub has_band: bool,
}

pub struct Pane {
	pub id: PaneId,
	pub term: TermInstance,
	pub scroll: Scroll,
	pub buffer: Buffer,
	// Previous frame's shaped text, kept one frame back by swapping with `buffer`
	// on each detected app-scroll step. Drawn (translated) to fill the strip a
	// slide reveals - the scrolled-off alt-screen lines are gone from the grid, so
	// the retained shaped frame is the only source of real content for that strip.
	prev_buffer: Buffer,
	pub rect: Rect,
	pub title: String,
	pub read_only: bool, // accept no PTY input/paste; selection + copy still work
	// launch argv (None = default shell); a split inherits this so a new pane
	// runs the same shell as the one it forked off (see design.md).
	command: Option<Vec<String>>,
	last_draw: PaneDraw,
	last_history: usize,
	// On-screen row fingerprints from the last build, used to detect a scrolled
	// viewport once the scrollback buffer is full (output easing) and to detect an
	// alt-screen app's repaint-scroll (app-scroll easing). See build().
	last_rows: Vec<u64>,
	// Rows of static bottom band (status/input line) that must NOT slide during the
	// current alt-screen app-scroll ease. Captured when a scroll is detected.
	slide_static: usize,
	// Shift (signed lines) of the retained prev_buffer relative to the current
	// frame, set when a scroll step is detected. Positions prev_buffer for the
	// slide (prev is at rest when app_off == slide_sh, slid fully out at app_off 0).
	slide_sh: f32,
	// Fallback glyphs (not in the primary mono font) pulled out of `buffer` and
	// drawn one-per-cell so their font advance can't shift the row. `glyph_bufs`
	// is a reused pool; `glyphs` holds (x, y, color, scale) for the first N of
	// them - `scale` shrinks an over-wide fallback glyph to fit its cell box.
	glyph_bufs: Vec<Buffer>,
	glyphs: Vec<(f32, f32, GColor, f32)>,
	// Glow source with bold stripped (text_glow_regular_weight): shaped alongside
	// the main buffer only on rebuild frames that actually contain bold runs.
	// `glow_debold` says the buffer is valid for the current content.
	glow_buf: Option<Buffer>,
	glow_debold: bool,
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
	// TermMode snapshot from the last build, so per-keystroke/wheel input paths
	// read it lock-free (at worst one frame stale) instead of taking the term
	// lock the PTY reader may hold across a whole read cycle.
	pub mode: TermMode,
	// This pane's PTY produced output since the last successful build. Set by
	// the Wakeup(id) event, cleared in build() once the term lock is acquired
	// (a busy-term frame keeps it, so the rebuild retries next frame). Scopes
	// re-shaping to panes that changed: one busy pane no longer forces its
	// idle siblings through set_rich_text every frame.
	pub content_dirty: bool,
	// Copy-output-on-command-finish (see arm_capture / poll_capture). `auto_copy` is
	// the per-pane opt-in. On Enter at the shell prompt we arm and record `cmd_start`
	// (the line after the prompt); when the terminal then settles (no new output for
	// a debounce) back at the prompt, the lines since are copied. `last_output` is
	// refreshed on every Wakeup so the settle timer measures true idle. This catches
	// both instant (ls) and long commands without racing the fg-pgid transition.
	pub auto_copy: bool,
	capture_armed: bool,
	cmd_start: usize,
	last_output: std::time::Instant,
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
		self.mode = *guard.mode();
		self.content_dirty = false;

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

		// Alt-screen app-scroll easing: a full-screen app owns its screen and scrolls
		// by repainting whole lines. Detect a clean vertical translate between this
		// repaint and the last (same row-fingerprints as the output-scroll probe) and
		// nudge a slide offset so the frame eases into place instead of snapping. The
		// revealed strip fills from the retained previous frame (swapped in below).
		// Only clean line-scrolls (up to APP_SCROLL_MAX rows) match - in-place redraws
		// and big page-jumps don't, so they hard-cut. Opt-in (experimental).
		let mut capture_prev = false;
		if s.smooth_scroll_apps && self.mode.contains(TermMode::ALT_SCREEN) {
			const APP_SCROLL_MAX: usize = 8;
			let grid = guard.grid();
			let mut rows: Vec<u64> = Vec::with_capacity(lines);
			for i in 0..lines as i32 {
				let row = &grid[Line(i)];
				let mut h: u64 = 0xcbf2_9ce4_8422_2325;
				for c in 0..cols {
					h = (h ^ row[Column(c)].c as u64).wrapping_mul(0x100_0000_01b3);
				}
				rows.push(h);
			}
			let sh = scroll_shift_signed(&rows, &self.last_rows, APP_SCROLL_MAX);
			if sh != 0 {
				// count the unchanged rows at the bottom - the fixed status/input
				// band that must not slide with the scrolling region above it
				let mut sb = 0;
				while sb < lines && rows[lines - 1 - sb] == self.last_rows[lines - 1 - sb] {
					sb += 1;
				}
				self.slide_static = sb;
				self.slide_sh = sh as f32;
				self.scroll.app_scroll(sh as f32);
				capture_prev = true; // this frame's outgoing content becomes prev_buffer
			}
			self.last_rows = rows;
		}

		// snap the integer grid offset to the floor of the smooth position
		let desired = self.scroll.desired_offset().min(history);
		let current = guard.grid().display_offset();
		let delta = desired as i32 - current as i32;
		if delta != 0 {
			guard.scroll_display(GridScroll::Delta(delta));
		}

		let frac = self.scroll.frac();
		// alt-screen slide rides on top of the fractional scrollback offset (which
		// is 0 on the alt screen); + shifts content down, revealing bg at the top.
		let app_off = self.scroll.app_offset();
		let voff = frac + app_off;
		// Region-aware slide: the scrolling region (top) shifts by voff while a
		// static bottom band (status/input line) stays at its fractional-only
		// position. `split_row` is the first screen row of that band; rows at/below
		// it get `frac` not `voff`. No band (or no active slide) => whole pane at voff.
		let static_rows = if app_off != 0.0 {
			self.slide_static.min(lines)
		} else {
			0
		};
		let split_row = (lines - static_rows) as i32;
		let voff_of = |sr: i32| {
			if static_rows > 0 && sr >= split_row {
				frac
			} else {
				voff
			}
		};
		let d = desired as i32;
		let hist = history as i32;
		// fractional scroll shifts content DOWN by frac of a cell; we render an
		// extra row above (screen row -1) so the revealed strip is filled.
		let y_of = |sr: i32| self.rect.y + margin + (sr as f32 + voff_of(sr)) * cell_h;
		let top = y_of(-1);
		// Retained-frame slide geometry (only while app_off is easing). The current
		// frame draws at `top` (scroll region, clipped above split_y); prev_buffer
		// fills the strip the shift reveals - above the content when sliding down
		// (app_off > 0), below it when sliding up. Clip prev to just that strip so
		// its own static band can't ghost into the scrolling region.
		let slide = if app_off != 0.0 {
			// split_y bounds the scroll region; a static bottom band (if any) is below
			let split_y = if static_rows > 0 {
				self.rect.y + margin + (split_row as f32 + frac) * cell_h
			} else {
				self.rect.y + self.rect.h
			};
			let band_top = self.rect.y + margin + (-1.0 + frac) * cell_h;
			// prev sits `slide_sh` behind the current frame; at app_off == slide_sh
			// it's at rest, sliding fully out as app_off -> 0
			let voff_prev = frac + app_off - self.slide_sh;
			let prev_top = self.rect.y + margin + (-1.0 + voff_prev) * cell_h;
			let (prev_clip_t, prev_clip_b) = if app_off > 0.0 {
				// down-slide: strip above the current content's first row (screen 0)
				(f32::MIN, self.rect.y + margin + voff * cell_h)
			} else {
				// up-slide: strip below the current scroll region's last row
				(
					self.rect.y + margin + (split_row as f32 + voff) * cell_h,
					split_y,
				)
			};
			Some(Slide {
				prev_top,
				prev_clip_t,
				prev_clip_b,
				split_y,
				band_top,
				has_band: static_rows > 0,
			})
		} else {
			None
		};

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
				voff,
				dt,
				s.cursor,
			);
			let bg = std::mem::take(&mut self.last_draw.bg);
			// the fast path is a pure cursor frame - never taken while a slide eases
			// (that forces a rebuild), so there is never a slide here
			self.last_draw = PaneDraw {
				top,
				bg,
				cursor,
				slide: None,
			};
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
		let mut saw_bold = false;

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
				let mut fg = palette::resolve(cell.fg, colors, &s);
				let mut cell_bg = palette::resolve(cell.bg, colors, &s);
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
				saw_bold |= bold;
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
			voff_of(cursor_pt.line.0 + d),
			dt,
			s.cursor,
		);
		// A scroll step was detected: the buffer still holds the outgoing frame, so
		// swap it into prev_buffer before set_rich_text overwrites it. prev_buffer
		// then fills the strip this slide reveals (see the slide geometry above).
		if capture_prev {
			std::mem::swap(&mut self.buffer, &mut self.prev_buffer);
		}
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

		// Glow source with uniform weight: bold ink is wider, so its halo reads
		// heavier than the neighbours'. When text_glow_regular_weight is on and
		// bold is on screen, shape a parallel buffer with bold stripped for the
		// glow pass (crisp text on top keeps its real weight). Costs a second
		// shape only on rebuild frames that contain bold. Per-cell fallback
		// glyphs keep their weight - rare, and not worth a second glyph pool.
		self.glow_debold =
			s.text_glow && s.text_glow_radius > 0.0 && s.text_glow_regular_weight && saw_bold;
		if self.glow_debold {
			let (bw, bh) = self.buffer.size();
			let gb = self.glow_buf.get_or_insert_with(|| {
				let mut b = Buffer::new(&mut ctx.font_system, ctx.metrics);
				b.set_wrap(&mut ctx.font_system, glyphon::Wrap::None);
				b.set_monospace_width(&mut ctx.font_system, Some(cell_w));
				b
			});
			gb.set_metrics(&mut ctx.font_system, ctx.metrics);
			gb.set_size(&mut ctx.font_system, bw, bh);
			let despan = spans.iter().map(|(t, a)| {
				let mut a2 = a.clone();
				a2.weight = default_attrs.weight;
				(t.as_str(), a2)
			});
			gb.set_rich_text(
				&mut ctx.font_system,
				despan,
				&default_attrs,
				Shaping::Advanced,
				None,
			);
			gb.shape_until_scroll(&mut ctx.font_system, false);
		}

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
			let y =
				rect_y + margin + (sr as f32 + voff_of(sr)) * cell_h + cell_h * (1.0 - scale) / 2.0;
			self.glyphs
				.push((x, y, GColor::rgb(color[0], color[1], color[2]), scale));
		}

		self.last_draw = PaneDraw {
			top,
			bg,
			cursor,
			slide,
		};
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
		voff: f32,
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
		let cell_y = self.rect.y + margin + (cursor_sr as f32 + voff) * cell_h;
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

	// Same as `text_area` but for the glow source pass: uses the de-bolded buffer
	// when it was built this frame (text_glow_regular_weight + bold on screen), so
	// the halo weight matches non-bold text while the crisp text keeps its weight.
	pub fn glow_text_area<'a>(&'a self, top: f32, margin: f32) -> TextArea<'a> {
		let mut area = self.text_area(top, margin);
		if self.glow_debold {
			if let Some(gb) = &self.glow_buf {
				area.buffer = gb;
			}
		}
		area
	}

	// glow_text_area with the band clip of text_area_band (see there).
	pub fn glow_text_area_band<'a>(
		&'a self,
		top: f32,
		margin: f32,
		clip_top: f32,
		clip_bottom: f32,
	) -> TextArea<'a> {
		let mut area = self.glow_text_area(top, margin);
		area.bounds.top = area.bounds.top.max(clip_top as i32);
		area.bounds.bottom = area.bounds.bottom.min(clip_bottom as i32);
		area
	}

	fn buf_area<'a>(&'a self, buf: &'a Buffer, top: f32, margin: f32) -> TextArea<'a> {
		TextArea {
			buffer: buf,
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

	pub fn text_area<'a>(&'a self, top: f32, margin: f32) -> TextArea<'a> {
		self.buf_area(&self.buffer, top, margin)
	}

	// Same buffer as text_area, positioned at `top`, but with its vertical clip
	// narrowed to [clip_top, clip_bottom]. Used by the app-scroll slide to draw the
	// current buffer clipped to the scroll region and the static band separately.
	pub fn text_area_band<'a>(
		&'a self,
		top: f32,
		margin: f32,
		clip_top: f32,
		clip_bottom: f32,
	) -> TextArea<'a> {
		let mut a = self.text_area(top, margin);
		a.bounds.top = a.bounds.top.max(clip_top as i32);
		a.bounds.bottom = a.bounds.bottom.min(clip_bottom as i32);
		a
	}

	// The retained previous frame (prev_buffer) at `top`, clipped to the strip the
	// slide reveals. Fills that strip with real outgoing content instead of bg.
	pub fn prev_text_area_band<'a>(
		&'a self,
		top: f32,
		margin: f32,
		clip_top: f32,
		clip_bottom: f32,
	) -> TextArea<'a> {
		let mut a = self.buf_area(&self.prev_buffer, top, margin);
		a.bounds.top = a.bounds.top.max(clip_top as i32);
		a.bounds.bottom = a.bounds.bottom.min(clip_bottom as i32);
		a
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

	// Copy-output: Enter was pressed at the shell prompt, so a command is (maybe)
	// about to run. Record where its output will begin (the line after the prompt/
	// echoed command) and arm the settle-based capture. Only arms at the shell
	// prompt, so an Enter inside a foreground app (vim, a REPL) doesn't arm.
	pub fn arm_capture(&mut self) {
		if !self.term.at_shell_prompt() {
			return;
		}
		if let Some(g) = self.term.term.try_lock_unfair() {
			let grid = g.grid();
			self.cmd_start = grid.history_size() + grid.cursor.point.line.0.max(0) as usize + 1;
			self.capture_armed = true;
			self.last_output = std::time::Instant::now();
		}
	}

	// New PTY output arrived: push the settle deadline out so capture waits for the
	// command (and its prompt) to finish before copying.
	pub fn note_output(&mut self) {
		self.last_output = std::time::Instant::now();
	}

	// While armed, the instant the settle timer would fire (so the loop can wake to
	// check) - None when nothing is pending.
	pub fn capture_deadline(&self, settle: std::time::Duration) -> Option<std::time::Instant> {
		self.capture_armed.then(|| self.last_output + settle)
	}

	// If armed and the terminal has settled (no output for `settle`) back at the
	// shell prompt, return the command's output as plain Unicode text (control/
	// colour codes are already gone - it's read from the parsed grid) and disarm.
	// Returns None otherwise, and skips empty output (e.g. a bare Enter or `cd`).
	pub fn poll_capture(&mut self, settle: std::time::Duration) -> Option<String> {
		if !self.capture_armed || self.last_output.elapsed() < settle {
			return None;
		}
		if !self.term.at_shell_prompt() {
			return None; // a foreground app is still running; wait for it to exit
		}
		let g = self.term.term.try_lock_unfair()?;
		self.capture_armed = false;
		let end = {
			let grid = g.grid();
			grid.history_size() + grid.cursor.point.line.0.max(0) as usize
		};
		let text = capture_grid_text(&g, self.cmd_start, end);
		(!text.trim().is_empty()).then_some(text)
	}

	// Map a window pixel to a 0-based on-screen cell (col, row) within this pane's
	// viewport, for mouse reporting. Clamped to the grid; None if outside the pane.
	pub fn screen_cell_at(&self, x: f32, y: f32, ctx: &TextCtx) -> Option<(usize, usize)> {
		if !self.rect.contains(x, y) {
			return None;
		}
		let cols = self.term.cols as i32;
		let lines = self.term.lines as i32;
		let rel_x = (x - self.rect.x - ctx.margin).max(0.0);
		let col = ((rel_x / ctx.cell_w).floor() as i32).clamp(0, cols - 1);
		let row = ((y - self.rect.y - ctx.margin) / ctx.cell_h)
			.floor()
			.clamp(0.0, (lines - 1) as f32) as i32;
		Some((col as usize, row as usize))
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
		let d = self.term.term.lock_unfair().grid().display_offset() as i32;
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
			let t = self.term.term.lock_unfair();
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
		self.term.term.lock_unfair().selection = Some(Selection::new(ty, point, side));
	}

	pub fn update_selection(&self, point: Point, side: Side) {
		let mut t = self.term.term.lock_unfair();
		if let Some(sel) = t.selection.as_mut() {
			sel.update(point, side);
		}
	}

	pub fn clear_selection(&self) {
		self.term.term.lock_unfair().selection = None;
	}

	pub fn selection_text(&self) -> Option<String> {
		self.term
			.term
			.lock_unfair()
			.selection_to_string()
			.filter(|s| !s.is_empty())
	}

	// Write pasted text to the PTY (wrapped in bracketed paste when the app
	// enabled it). No-op when the pane is read-only.
	pub fn paste(&self, text: &str) {
		if self.read_only || text.is_empty() {
			return;
		}
		let bracket = self.mode.contains(TermMode::BRACKETED_PASTE);
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
		// interactive splits even-distribute the same-direction run (unless a divider
		// in it was hand-dragged); the CLI drives its own sizing, so it passes false
		self.split_at(ctx, proxy, id, dir, false, 0.5, cmd, area, true);
		Ok(())
	}

	// General split used by the CLI: split `id` along `dir`, the new pane on the
	// `before` side (a) or after (b), taking `new_ratio` of the split; runs
	// `command`. Returns the new pane id (None if `id` wasn't a leaf). `equalize`
	// re-distributes the same-direction run to equal fractions after inserting
	// (interactive default); the CLI passes false and sizes explicitly.
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
		equalize: bool,
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
		// even-distribute the same-direction run the new pane joined, unless a
		// divider in it was hand-dragged (then successive splits stay 50/50)
		if equalize {
			equalize_dir_run(&mut self.root, new_id, dir);
		}
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
			pane.prev_buffer = ctx.new_buffer(pane.rect.w.max(1.0), pane.rect.h.max(1.0));
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
				ctx.resize_buffer(&mut pane.prev_buffer, cw, ch + 2.0 * ctx.cell_h);
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
	let prev_buffer = ctx.new_buffer(cw, ch + 2.0 * ctx.cell_h);
	Ok(Pane {
		id,
		term,
		scroll: Scroll::new(),
		buffer,
		prev_buffer,
		rect,
		title: config::APP_NAME.into(),
		read_only: false,
		command,
		last_draw: PaneDraw {
			top: rect.y,
			bg: Vec::new(),
			cursor: None,
			slide: None,
		},
		last_history: 0,
		last_rows: Vec::new(),
		slide_static: 0,
		slide_sh: 0.0,
		glyph_bufs: Vec::new(),
		glyphs: Vec::new(),
		glow_buf: None,
		glow_debold: false,
		cursor_x: 0.0,
		cursor_col: 0.0,
		cursor_row: i32::MIN,
		cursor_init: false,
		blink_t: 0.0,
		cursor_animating: false,
		text_built: false,
		mode: TermMode::empty(),
		content_dirty: true,
		auto_copy: false,
		capture_armed: false,
		cmd_start: 0,
		last_output: std::time::Instant::now(),
	})
}

// Extract the grid text for absolute line range [start_abs, end_abs) as plain
// Unicode. Absolute index 0 is the oldest line currently in the buffer; screen
// row 0 sits at absolute `history_size`. Trailing pad spaces are trimmed and a
// newline is emitted per grid row, except rows flagged WRAPLINE (a soft-wrapped
// long line) which join to the next. Lines evicted from scrollback (only when a
// command's output exceeds the scrollback limit) are skipped.
fn capture_grid_text(
	term: &Term<crate::term::EventProxy>,
	start_abs: usize,
	end_abs: usize,
) -> String {
	let grid = term.grid();
	let hist = grid.history_size() as i64;
	let cols = grid.columns();
	let mut out = String::new();
	let mut a = start_abs;
	while a < end_abs {
		let gl = a as i64 - hist; // screen top is absolute `hist`; history is negative
		if gl < -hist {
			a += 1; // scrolled out of the buffer (output longer than scrollback)
			continue;
		}
		let row = &grid[Line(gl as i32)];
		let mut s = String::new();
		for c in 0..cols {
			let cell = &row[Column(c)];
			if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
				continue; // the trailing half of a wide glyph has no char of its own
			}
			s.push(cell.c);
		}
		if cols > 0 && row[Column(cols - 1)].flags.contains(Flags::WRAPLINE) {
			out.push_str(&s); // soft-wrapped: continue the logical line, no newline
		} else {
			out.push_str(s.trim_end());
			out.push('\n');
		}
		a += 1;
	}
	out
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
				manual: false,
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

// Path (false = a-child, true = b-child) from `node` down to leaf `id`, if present.
fn path_to(node: &Node, id: PaneId) -> Option<Vec<bool>> {
	match node {
		Node::Leaf(i) => (*i == id).then(Vec::new),
		Node::Split { a, b, .. } => {
			if let Some(mut p) = path_to(a, id) {
				p.insert(0, false);
				return Some(p);
			}
			if let Some(mut p) = path_to(b, id) {
				p.insert(0, true);
				return Some(p);
			}
			None
		}
	}
}

// Follow `path` from `node` (defensively stops at a leaf).
fn node_at_mut<'a>(mut node: &'a mut Node, path: &[bool]) -> &'a mut Node {
	for &b in path {
		let Node::Split { a, b: bb, .. } = node else {
			break;
		};
		node = if b { bb } else { a };
	}
	node
}

// Is the node at `path` a Split oriented along `dir`?
fn is_dir_split(root: &Node, path: &[bool], dir: Dir) -> bool {
	let mut node = root;
	for &b in path {
		let Node::Split { a, b: bb, .. } = node else {
			return false;
		};
		node = if b { bb } else { a };
	}
	matches!(node, Node::Split { dir: d, .. } if *d == dir)
}

// Leaves in the same-direction run rooted at `node`: a nested `dir` split counts
// its members; a leaf or a differently-oriented split counts as one unit (its own
// internal layout is separate).
fn group_leaf_count(node: &Node, dir: Dir) -> usize {
	match node {
		Node::Split { dir: d, a, b, .. } if *d == dir => {
			group_leaf_count(a, dir) + group_leaf_count(b, dir)
		}
		_ => 1,
	}
}

// Has any divider in the same-direction run been hand-dragged?
fn group_has_manual(node: &Node, dir: Dir) -> bool {
	match node {
		Node::Split {
			dir: d,
			manual,
			a,
			b,
			..
		} if *d == dir => *manual || group_has_manual(a, dir) || group_has_manual(b, dir),
		_ => false,
	}
}

// Set every ratio in the same-direction run so all its member leaves are equal:
// a split gives its a-child a share proportional to the leaves under it.
fn equalize(node: &mut Node, dir: Dir) {
	if let Node::Split {
		dir: d,
		ratio,
		a,
		b,
		..
	} = node
	{
		if *d == dir {
			let na = group_leaf_count(a, dir);
			let nb = group_leaf_count(b, dir);
			*ratio = na as f32 / (na + nb) as f32;
			equalize(a, dir);
			equalize(b, dir);
		}
	}
}

// After splitting to create leaf `id` along `dir`, even-distribute the whole
// same-direction run it joined - unless a divider in that run was hand-dragged
// (then the run keeps its sizes and the new 50/50 split stands).
fn equalize_dir_run(root: &mut Node, id: PaneId, dir: Dir) {
	let Some(path) = path_to(root, id) else {
		return;
	};
	if path.is_empty() {
		return; // the tree is a lone leaf
	}
	// walk up from the new pane's parent while ancestors stay same-direction; that
	// topmost same-direction split is the run's root
	let mut k = path.len() - 1;
	while k > 0 && is_dir_split(root, &path[..k - 1], dir) {
		k -= 1;
	}
	let top = node_at_mut(root, &path[..k]);
	if !group_has_manual(top, dir) {
		equalize(top, dir);
	}
}

fn prune(node: Node, id: PaneId) -> Option<Node> {
	match node {
		Node::Leaf(i) if i == id => None,
		Node::Leaf(i) => Some(Node::Leaf(i)),
		Node::Split {
			dir,
			ratio,
			manual,
			a,
			b,
		} => {
			let a2 = prune(*a, id);
			let b2 = prune(*b, id);
			match (a2, b2) {
				(Some(a), Some(b)) => Some(Node::Split {
					dir,
					ratio,
					manual,
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
		Node::Split {
			dir, ratio, a, b, ..
		} => {
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
	let Node::Split {
		dir, ratio, a, b, ..
	} = node
	else {
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
	let Node::Split {
		dir,
		ratio,
		manual,
		a,
		b,
	} = node
	else {
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
	*manual = true; // dragged: stop auto even-distribution for this run
}

// Lines the on-screen content scrolled up between frames, inferred from row
// fingerprints when scrollback growth can't tell us (the buffer is full). It's
// the smallest shift k where this frame's top (rows-k) lines equal last frame's
// bottom (rows-k) lines.
// Signed sibling of scroll_shift for alt-screen app-scroll easing: detect a clean
// vertical translate between two frames, in either direction, up to `max` lines.
// +k = scrolled forward (content moved up k rows), -k = scrolled back (down k).
// Matches only a TOP PREFIX of rows, not the full height: real full-screen apps
// (less, vim, muffer) keep a static status/input band at the bottom that never
// scrolls, so a whole-height match would never fire. A shift counts only if a
// solid majority of rows translate cleanly; otherwise 0 (in-place redraw, content
// change, or a jump bigger than `max`) and the caller hard-cuts. It never guesses
// a full turnover the way scroll_shift does - easing a non-scroll looks wrong.
fn scroll_shift_signed(cur: &[u64], last: &[u64], max: usize) -> i32 {
	let n = cur.len();
	if n == 0 || last.len() != n {
		return 0;
	}
	let need = (n / 2).max(3); // require most of the screen to translate as one block
	let lim = max.min(n - 1);
	let (mut best, mut best_run) = (0i32, 0usize);
	for k in 1..=lim {
		// forward: cur[i] == last[i+k] across a run down from the top
		let mut p = 0;
		while p < n - k && cur[p] == last[p + k] {
			p += 1;
		}
		if p >= need && p > best_run {
			best_run = p;
			best = k as i32;
		}
		// backward: cur[i+k] == last[i] (content slid down)
		let mut q = 0;
		while q < n - k && cur[q + k] == last[q] {
			q += 1;
		}
		if q >= need && q > best_run {
			best_run = q;
			best = -(k as i32);
		}
	}
	best
}

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
	use super::{
		Dir, Node, Rect, bell_brighten, distinct_pair, equalize_dir_run, layout, pair_inside,
		same_char_pair, scroll_shift, scroll_shift_signed,
	};

	fn leaf(id: u64) -> Node {
		Node::Leaf(id)
	}
	fn split(dir: Dir, ratio: f32, manual: bool, a: Node, b: Node) -> Node {
		Node::Split {
			dir,
			ratio,
			manual,
			a: Box::new(a),
			b: Box::new(b),
		}
	}
	fn widths(root: &Node, w: f32) -> Vec<(u64, f32)> {
		let mut out = Vec::new();
		layout(
			root,
			Rect {
				x: 0.0,
				y: 0.0,
				w,
				h: 100.0,
			},
			&mut out,
		);
		out.sort_by_key(|(id, _)| *id);
		out.into_iter().map(|(id, r)| (id, r.w)).collect()
	}

	#[test]
	fn equalize_three_in_a_row() {
		// split A vertically then split the new pane again: 50/25/25 -> equalize
		let mut root = split(
			Dir::Vertical,
			0.5,
			false,
			leaf(1),
			split(Dir::Vertical, 0.5, false, leaf(2), leaf(3)),
		);
		equalize_dir_run(&mut root, 3, Dir::Vertical);
		let ws = widths(&root, 900.0);
		for (_, w) in &ws {
			assert!((w - 300.0).abs() <= 2.0, "not equal thirds: {ws:?}");
		}
	}

	#[test]
	fn equalize_four_in_a_row() {
		let mut root = split(
			Dir::Vertical,
			0.5,
			false,
			leaf(1),
			split(
				Dir::Vertical,
				0.5,
				false,
				leaf(2),
				split(Dir::Vertical, 0.5, false, leaf(3), leaf(4)),
			),
		);
		equalize_dir_run(&mut root, 4, Dir::Vertical);
		let ws = widths(&root, 1200.0);
		for (_, w) in &ws {
			assert!((w - 300.0).abs() <= 3.0, "not equal quarters: {ws:?}");
		}
	}

	#[test]
	fn manual_divider_stops_equalization() {
		// the outer divider was hand-dragged (manual): a later split must not
		// re-equalize - the 0.7 ratio is preserved
		let mut root = split(
			Dir::Vertical,
			0.7,
			true,
			leaf(1),
			split(Dir::Vertical, 0.5, false, leaf(2), leaf(3)),
		);
		equalize_dir_run(&mut root, 3, Dir::Vertical);
		let Node::Split { ratio, .. } = &root else {
			panic!()
		};
		assert_eq!(*ratio, 0.7, "manual run must keep its sizes");
	}

	#[test]
	fn different_direction_counts_as_one_unit() {
		// a vertical run whose second member is a horizontal split: 2 units -> 50/50,
		// and the inner horizontal ratio is left untouched
		let mut root = split(
			Dir::Vertical,
			0.3,
			false,
			leaf(1),
			split(Dir::Horizontal, 0.4, false, leaf(2), leaf(3)),
		);
		equalize_dir_run(&mut root, 1, Dir::Vertical);
		let Node::Split { ratio, b, .. } = &root else {
			panic!()
		};
		assert!((ratio - 0.5).abs() < 0.01, "two units -> half each");
		let Node::Split { ratio: hr, .. } = b.as_ref() else {
			panic!()
		};
		assert_eq!(*hr, 0.4, "nested other-direction split is untouched");
	}

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
	fn signed_shift_detects_both_directions_and_hard_cuts_the_rest() {
		let last = [10u64, 20, 30, 40, 50];
		// scrolled forward: content moved up 2 (cur top == last[2..])
		let fwd = [30, 40, 50, 60, 70];
		assert_eq!(scroll_shift_signed(&fwd, &last, 8), 2);
		// scrolled back: content moved down 1 (cur[1..] == last[..n-1])
		let back = [5, 10, 20, 30, 40];
		assert_eq!(scroll_shift_signed(&back, &last, 8), -1);
		// no motion, in-place change, and full turnover all hard-cut (0), never a guess
		assert_eq!(scroll_shift_signed(&last, &last, 8), 0);
		assert_eq!(scroll_shift_signed(&[11, 20, 30, 40, 50], &last, 8), 0);
		assert_eq!(scroll_shift_signed(&[60, 70, 80, 90, 99], &last, 8), 0);
		// a jump bigger than max is not eased
		assert_eq!(scroll_shift_signed(&fwd, &last, 1), 0);
		// real-app shape: the top scrolls but a static status/input band at the
		// bottom stays put - the prefix still matches, so it's detected
		let last_s = [10u64, 20, 30, 40, 900, 901];
		let cur_s = [20u64, 30, 40, 50, 900, 901];
		assert_eq!(scroll_shift_signed(&cur_s, &last_s, 8), 1);
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
