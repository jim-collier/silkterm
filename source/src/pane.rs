// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use alacritty_terminal::grid::{Dimensions, Grid, Scroll as GridScroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
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

// SILK_SCROLLDBG: per-frame app-scroll trace (dev-only, gated by the env var).
static DBG_FRAME: AtomicU64 = AtomicU64::new(0);
fn scroll_dbg() -> bool {
	use std::sync::OnceLock;
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| std::env::var_os("SILK_SCROLLDBG").is_some())
}

// Cursor animation tunables (internal).
const CURSOR_MOVE_TAU_MS: f32 = 55.0; // horizontal slide responsiveness (lower = snappier)
const CURSOR_ALPHA: f32 = 0.55; // solid block-cursor alpha
const CURSOR_INPUT_PAUSE_S: f32 = 0.35; // idle time before the animation resumes (input-pause mode)
const BELL_BRIGHTEN: f32 = 0.6; // max lerp of text toward white at the bell flash peak

// Alt-screen app-scroll tunables.
const APP_SCROLL_MAX: usize = 24; // max per-step shift the slide detector accepts (in step with scroll::APP_OFF_CAP)
// Whether the smooth slide engages for full-screen apps that keep a static TOP
// band (title bar: nano, muffer). Was off while the reveal strip was filled from
// a single retained frame: the strip could under-fill by the ease lag and its
// re-capture repositioned it every step - the band/scrim bounce. The scrolled-off
// strip (OffStrip below) fills the gap exactly and never repositions, so the
// slide is on for top-band apps again. Apps that fill from the top with only a
// bottom status line (less, vim) have no top band and slide regardless.
const SLIDE_TOP_BAND_APPS: bool = true;

// One styled cell captured for the scrolled-off strip. Colours are resolved at
// capture time (the palette/theme can change later; the strip shows what was on
// screen). `wide` is the cell count: 0 = wide-char spacer (skip), 1, or 2.
#[derive(Clone, Copy, PartialEq, Debug)]
struct StripCell {
	c: char,
	fg: [u8; 3],
	bg: Option<[u8; 3]>,
	bold: bool,
	italic: bool,
	wide: u8,
}

// Scrolled-off strip: the rows an alt-screen app's scroll pushed out of its
// region, retained styled and in visual order (top to bottom) so the slide can
// draw them in the gap it reveals. The strip is welded to the content edge and
// grows by exactly each step's shift while app_off grows by the same amount, so
// the gap is always exactly filled - no under-fill, no re-capture jump, and no
// furniture bleed (only region rows are ever captured). `dir`: +1 = strip above
// the content (content moved up), -1 = below.
struct OffStrip {
	rows: std::collections::VecDeque<Vec<StripCell>>,
	dir: i8,
}

impl OffStrip {
	// app_off can't lag past scroll::APP_OFF_CAP, so older rows are invisible
	const CAP: usize = APP_SCROLL_MAX + 2;

	fn new() -> Self {
		Self {
			rows: std::collections::VecDeque::new(),
			dir: 0,
		}
	}

	fn len(&self) -> usize {
		self.rows.len()
	}

	fn clear(&mut self) {
		self.rows.clear();
		self.dir = 0;
	}

	// Append the rows a step pushed off the region (`chunk` in visual order). A
	// direction flip discards the old strip - it belongs on the other side.
	fn push_step(&mut self, dir: i8, chunk: Vec<Vec<StripCell>>) {
		if self.dir != dir {
			self.clear();
			self.dir = dir;
		}
		if dir > 0 {
			// content moved up: rows left off the top of the region, the newest
			// chunk nearest the content = at the strip's bottom
			self.rows.extend(chunk);
			while self.rows.len() > Self::CAP {
				self.rows.pop_front();
			}
		} else {
			// content moved down: rows left off the bottom, the newest chunk at
			// the strip's top (nearest the content), keeping its internal order
			for row in chunk.into_iter().rev() {
				self.rows.push_front(row);
			}
			while self.rows.len() > Self::CAP {
				self.rows.pop_back();
			}
		}
	}
}

// The rows a detected step pushed out of the scroll region, as a range into the
// PREVIOUS frame's rows. shift > 0 = content moved up, rows left off the top of
// the region (just under any title band); shift < 0 = off the bottom.
fn vanished_range(shift: i32, st: usize, sb: usize, lines: usize) -> std::ops::Range<usize> {
	let region_top = st.min(lines);
	let region_bot = lines.saturating_sub(sb).max(region_top);
	let k = (shift.unsigned_abs() as usize).min(region_bot - region_top);
	if shift > 0 {
		region_top..region_top + k
	} else {
		region_bot - k..region_bot
	}
}

// The slide's region clip: band boundaries tightened to the shifted content's
// extent. The gap between a band and the content edge belongs to the strip;
// without the weld, band rows translated by voff render inside the band clip
// as ghost copies (see the Slide doc).
fn weld_region_clip(
	top_split_y: f32,
	split_y: f32,
	content_top_y: f32,
	content_bot_y: f32,
) -> (f32, f32) {
	(top_split_y.max(content_top_y), split_y.min(content_bot_y))
}

// Fingerprint every visible row (FNV-1a over the chars) and, when `styled` is
// given, snapshot the styled cells too - the scrolled-off strip's source data.
// Colours resolve the same way build()'s cell loop does (minus the transient
// bell flash and selection, which don't belong in a retained row). Recycles the
// caller's row allocations. One entry per column; a wide-char spacer stays as a
// wide=0 placeholder so indexes keep matching columns.
fn snapshot_rows(
	grid: &Grid<Cell>,
	lines: usize,
	cols: usize,
	styled: Option<(&Colors, &config::Settings, &mut Vec<Vec<StripCell>>)>,
) -> Vec<u64> {
	let mut rows: Vec<u64> = Vec::with_capacity(lines);
	let mut styled = styled;
	if let Some((_, _, out)) = &mut styled {
		out.resize_with(lines, Vec::new);
	}
	for i in 0..lines as i32 {
		let row = &grid[Line(i)];
		let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
		for c in 0..cols {
			hash = (hash ^ row[Column(c)].c as u64).wrapping_mul(0x100_0000_01b3);
		}
		rows.push(hash);
		if let Some((colors, settings, out)) = &mut styled {
			let out_row = &mut out[i as usize];
			out_row.clear();
			out_row.reserve(cols);
			for c in 0..cols {
				let cell = &row[Column(c)];
				let flags = cell.flags;
				if flags.contains(Flags::WIDE_CHAR_SPACER) {
					out_row.push(StripCell {
						c: ' ',
						fg: [0; 3],
						bg: None,
						bold: false,
						italic: false,
						wide: 0,
					});
					continue;
				}
				let mut fg = palette::resolve(cell.fg, colors, settings);
				let mut cell_bg = palette::resolve(cell.bg, colors, settings);
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
				out_row.push(StripCell {
					c: cell.c,
					fg,
					bg: (cell_bg != settings.bg).then_some(cell_bg),
					bold: flags.contains(Flags::BOLD)
						|| (settings.embolden_inverse && flags.contains(Flags::INVERSE)),
					italic: flags.contains(Flags::ITALIC),
					wide: if flags.contains(Flags::WIDE_CHAR) {
						2
					} else {
						1
					},
				});
			}
		}
	}
	rows
}

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
			let settings = config::settings();
			(
				(settings.cursor_size_width / 100.0).clamp(0.02, 1.0), // width, from left
				(settings.cursor_size_height / 100.0).clamp(0.02, 1.0), // height, from bottom
			)
		}
	}
}

// Pulse envelope over one cycle: grow, hold full, shrink, then a brief disappear.
fn pulse_env(phase: f32) -> f32 {
	let smooth = |t: f32| {
		let t = t.clamp(0.0, 1.0);
		t * t * (3.0 - 2.0 * t)
	};
	if phase < 0.40 {
		smooth(phase / 0.40) // grow 0 -> 1
	} else if phase < 0.60 {
		1.0 // hold at full
	} else if phase < 0.90 {
		1.0 - smooth((phase - 0.60) / 0.30) // shrink 1 -> 0
	} else {
		0.0 // disappear momentarily
	}
}

// Lerp a text colour toward white by `t` (0..1) of the BELL_BRIGHTEN ceiling, for
// the visual-bell flash. Identity at t<=0.
fn bell_brighten(color: [u8; 3], t: f32) -> [u8; 3] {
	if t <= 0.0 {
		return color;
	}
	let t = (t * BELL_BRIGHTEN).clamp(0.0, 1.0);
	let up = |v: u8| (v as f32 + (255.0 - v as f32) * t).round() as u8;
	[up(color[0]), up(color[1]), up(color[2])]
}

// FNV-1a over a row's chars: the fingerprint copy-output uses to re-find the
// arm-time prompt row at capture time (same constants as build()'s inline rows).
fn fnv_row(chars: impl Iterator<Item = char>) -> u64 {
	let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
	for c in chars {
		hash = (hash ^ c as u64).wrapping_mul(0x100_0000_01b3);
	}
	hash
}

// The char to feed the shaper for a grid cell. A cell may hold a literal control
// char - alacritty leaves the '\t' in the first tab cell and fills to the tab
// stop with spaces - and cosmic-text shapes a raw tab as a full 8-col stop,
// shifting the rest of the row off the col*cell_w grid so the visible text no
// longer lines up with the selectable cells (misaligned double-click on tabbed
// output like `zpool status`). The cell already carries the padding, so render
// any control char as a plain 1-cell space.
fn render_char(c: char) -> char {
	if c.is_control() { ' ' } else { c }
}

// Advance the blink phase one frame toward `full_phase` (the point in the cycle
// where the cursor is full-size) at its normal speed. Returns the new blink_t
// and whether it reached full this step.
fn glide_to_full(blink_t: f32, dt: f32, period: f32, full_phase: f32) -> (f32, bool) {
	let prev = (blink_t / period).fract();
	let next = blink_t + dt;
	let now = (next / period).fract();
	let crossed = if prev <= now {
		prev <= full_phase && full_phase <= now
	} else {
		full_phase >= prev || full_phase <= now // wrapped past 1.0 this step
	};
	if crossed {
		(full_phase * period, true)
	} else {
		(next, false)
	}
}

// "pause" input mode. On input the cycle keeps running at its normal speed until
// it next reaches the full-size phase, parks there while input stays recent,
// then resumes the cycle from that same point - so the size is continuous at
// every step: no snap to full on a keystroke, and no snap to small on resume
// even when the glide outlasts the idle timeout (slow blink rates).
#[derive(Default)]
struct PauseState {
	active: bool, // an input episode is in progress (gliding or holding)
	parked: bool, // reached the full-size phase, holding there
	hold_t: f32,  // seconds parked at full
}

impl PauseState {
	fn advance(
		&mut self,
		blink_t: f32,
		dt: f32,
		period: f32,
		full_phase: f32,
		timeout: f32,
		moved: bool,
		idle_t: f32,
	) -> f32 {
		if moved && !self.active {
			self.active = true;
			self.parked = false;
			self.hold_t = 0.0;
		}
		if !self.active {
			return blink_t + dt;
		}
		if !self.parked {
			let (next, parked) = glide_to_full(blink_t, dt, period, full_phase);
			self.parked = parked;
			return next;
		}
		// parked: hold at full until input has been idle AND the hold has lasted
		// the timeout (a long glide can eat the whole idle window)
		self.hold_t += dt;
		if self.hold_t >= timeout && idle_t >= timeout {
			self.active = false;
			return full_phase * period + dt; // resume the cycle from full
		}
		full_phase * period
	}
}

// Expand `line` up and down across soft-wrapped rows, clamped to [top, bot].
// `wrapped(l)` is true when grid row l's last cell carries WRAPLINE (the logical
// line continues into row l+1). Returns the (first, last) grid row of the whole
// logical line - used for triple-click line selection.
fn logical_line_bounds(line: i32, top: i32, bot: i32, wrapped: impl Fn(i32) -> bool) -> (i32, i32) {
	let line = line.clamp(top, bot);
	let mut start = line;
	while start > top && wrapped(start - 1) {
		start -= 1;
	}
	let mut end = line;
	while end < bot && wrapped(end) {
		end += 1;
	}
	(start, end)
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
	// App-scroll slide (None = common case: whole pane at `top`). While a
	// full-screen app's scroll eases, the current frame draws shifted at `top`
	// and the scrolled-off strip (pane.strip_buf) fills the revealed gap.
	pub slide: Option<Slide>,
}

// One frame of an easing app-scroll slide. The current frame renders at
// `PaneDraw.top`, clipped to `[region_clip_t, region_clip_b]`; the scrolled-off
// strip renders at `strip_top` clipped to the scroll region `[top_split_y,
// split_y]` (it holds only region rows, so nothing can bleed into the bands);
// and the fixed bands - a bottom status/input line (`has_band`, below `split_y`)
// and a top title bar (`has_top_band`, above `top_split_y`) - redraw unshifted
// at `band_top`. `top_split_y` is f32::MIN when there's no top band (open clip).
//
// The region clip is WELDED to the shifted content's extent, not just the band
// boundaries: the current-frame draw is the whole buffer translated by voff, so
// band rows ride into the region during a slide - the title's glyphs (and their
// scrim) land voff below the real title, the status rows land voff above theirs
// - rendering as ghost copies that bounce with the ease. Clipping at the
// content edge cuts them off; the strip owns the gap on the other side of the
// weld.
#[derive(Clone)]
pub struct Slide {
	pub strip_top: f32,
	pub top_split_y: f32,
	pub split_y: f32,
	pub region_clip_t: f32,
	pub region_clip_b: f32,
	pub band_top: f32,
	pub has_band: bool,
	pub has_top_band: bool,
}

pub struct Pane {
	pub id: PaneId,
	pub term: TermInstance,
	pub scroll: Scroll,
	pub buffer: Buffer,
	// Scrolled-off strip (see OffStrip): styled rows the app's scroll pushed out
	// of its region, shaped into `strip_buf` and drawn welded to the content edge
	// so the slide's reveal gap is always exactly filled. `strip_dirty` re-shapes
	// the buffer on the next build (rows changed, or a font rebuild).
	strip: OffStrip,
	strip_buf: Buffer,
	strip_dirty: bool,
	// Previous frame's styled cells (captured only in alt-screen smooth-scroll
	// mode): the rows a step pushes off the region are gone from the grid by the
	// time the step is detected, so they must be captured a frame ahead.
	// `cells_scratch` recycles the row allocations frame to frame.
	last_cells: Vec<Vec<StripCell>>,
	cells_scratch: Vec<Vec<StripCell>>,
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
	// Rows of static TOP band (title bar - nano, muffer) that must NOT slide, the
	// mirror of slide_static. The scrolling region is between the two bands.
	slide_static_top: usize,
	// Last detected step's shift (signed lines). Only feeds the SILK_SCROLLDBG
	// trace now - the strip is positioned by app_off alone - but the harness
	// regex reads the field, so it stays.
	slide_sh: f32,
	// Previous frame's alt-screen state. An enter/exit is an instant screen swap,
	// not a scroll - detected here to hard-cut it instead of animating the swap.
	last_alt: bool,
	// Fallback glyphs (not in the primary mono font) pulled out of `buffer` and
	// drawn one-per-cell so their font advance can't shift the row. `glyph_bufs`
	// is a reused pool; `glyphs` holds (x, y, color, scale) for the first N of
	// them - `scale` shrinks an over-wide fallback glyph to fit its cell box.
	glyph_bufs: Vec<Buffer>,
	glyphs: Vec<(f32, f32, GColor, f32)>,
	// Scrim source with bold stripped (text_scrim_regular_weight): shaped alongside
	// the main buffer only on rebuild frames that actually contain bold runs.
	// `scrim_debold` says the buffer is valid for the current content.
	scrim_buf: Option<Buffer>,
	scrim_debold: bool,
	// Cursor animation: `cursor_x` (visual column) eases toward the target column
	// so the cursor slides as you type; `blink_t` drives a smooth fade-blink while
	// it sits idle. Snaps on a row change so it doesn't slide diagonally on a newline.
	cursor_x: f32,
	cursor_col: f32,
	cursor_row: i32,
	cursor_init: bool,
	blink_t: f32,
	cursor_idle_t: f32, // seconds since the cursor last moved (input-pause gating)
	cursor_pause: PauseState,
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
	// Fingerprint of the arm-time prompt row. cmd_start is "history + row", an
	// index whose origin MOVES once scrollback is at cap (each pushed line evicts
	// the oldest), so capture re-finds the prompt row by content instead and only
	// falls back to cmd_start when it can't (evicted, or redrawn on Enter).
	cmd_anchor: Option<u64>,
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
		let settings = config::settings(); // snapshot once, not per cell

		// Never block the render thread: the PTY reader thread can hold the
		// terminal lock through long bursts (e.g. a chatty shell rc). If it's
		// busy this frame, reuse the last built frame.
		let mut guard = match self.term.term.try_lock_unfair() {
			Some(g) => g,
			None => return self.last_draw.clone(),
		};
		self.mode = *guard.mode();
		self.content_dirty = false;

		// Alt-screen enter/exit is an instant full-screen swap, not a scroll. Flag the
		// transition so the scroll probes below hard-cut it: on enter the app-scroll
		// probe would match blank rows between the old and new screens (nano "jiggles"/
		// scrolls in on launch); on exit the history_size jump (the alt grid carries no
		// scrollback) would fire an output-ease that scrolls the restored screen back
		// in. `gesture_active` (an alt-scroll slide already easing) freezes the band
		// sizes across a continuous scroll - see the app-scroll block.
		let alt = self.mode.contains(TermMode::ALT_SCREEN);
		let alt_transition = alt != self.last_alt;
		self.last_alt = alt;
		let gesture_active = self.scroll.app_offset() != 0.0;

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
		if alt_transition {
			// hard-cut the screen swap: drop any in-flight slide and rebaseline the
			// row fingerprints (and the styled snapshot the strip captures from) to
			// the NEW screen, so neither the output-scroll probe nor the app-scroll
			// probe diffs against the old screen next frame.
			self.scroll.cancel_app_scroll();
			self.strip.clear();
			self.last_rows = if settings.smooth_scroll_apps {
				let mut cur_cells = std::mem::take(&mut self.cells_scratch);
				let rows = snapshot_rows(
					guard.grid(),
					lines,
					cols,
					Some((guard.colors(), &settings, &mut cur_cells)),
				);
				self.cells_scratch = std::mem::replace(&mut self.last_cells, cur_cells);
				rows
			} else {
				snapshot_rows(guard.grid(), lines, cols, None)
			};
		}
		let follow = self.scroll.following();
		let full = settings.scrollback > 0 && history >= settings.scrollback;
		let advanced = if grew > 0 {
			grew
		} else if follow && full {
			let rows = snapshot_rows(guard.grid(), lines, cols, None);
			let inferred_advance = scroll_shift(&rows, &self.last_rows);
			self.last_rows = rows;
			inferred_advance
		} else {
			0
		};
		if advanced > 0 && follow && !alt_transition {
			self.scroll.nudge_output(advanced as f32);
		}

		// Alt-screen app-scroll easing: a full-screen app owns its screen and scrolls
		// by repainting whole lines. Detect a clean vertical translate between this
		// repaint and the last (same row-fingerprints as the output-scroll probe) and
		// nudge a slide offset so the frame eases into place instead of snapping. The
		// revealed gap fills from the scrolled-off strip: the styled rows each step
		// pushes out of the region, captured from the previous frame's snapshot.
		// Only clean line-scrolls (up to APP_SCROLL_MAX rows) match - in-place redraws
		// and big page-jumps don't, so they hard-cut. Opt-in (experimental).
		// Skipped on pure cursor-animation frames (the fast path below): a shift can
		// only appear when the grid content changed, and that always forces a full
		// build - so the styled snapshot isn't paid per blink frame.
		let mut shift_dbg = 0i32;
		if settings.smooth_scroll_apps
			&& alt && !alt_transition
			&& (force_rebuild || !self.text_built)
		{
			let mut cur_cells = std::mem::take(&mut self.cells_scratch);
			let rows = snapshot_rows(
				guard.grid(),
				lines,
				cols,
				Some((guard.colors(), &settings, &mut cur_cells)),
			);
			let shift = scroll_shift_signed(&rows, &self.last_rows, APP_SCROLL_MAX);
			shift_dbg = shift;
			if shift != 0 {
				// Freeze the static-band sizes on the gesture's first step (a clean
				// settled-vs-scrolled diff); re-measuring per step fluctuates by a row
				// whenever a blank/matching line abuts a band. Held while the slide eases.
				if !gesture_active {
					let (st, sb) = static_bands(&rows, &self.last_rows);
					self.slide_static = sb;
					self.slide_static_top = st;
				}
				if SLIDE_TOP_BAND_APPS || self.slide_static_top == 0 {
					// ACCUMULATE the visual offset so the CURRENT content stays continuous
					// across overlapping steps: screen row = grid_row + app_off, the grid
					// already advanced by shift, so app_off must GROW by shift to hold a
					// line fixed for that instant. The strip grows by the same rows the
					// step pushed off the region (from the frame-old snapshot; a stale or
					// resized snapshot just skips the fill for this one step), so the gap
					// the accumulated offset opens is always exactly covered.
					self.slide_sh = shift as f32;
					if self.last_cells.len() == lines
						&& self.last_cells.first().is_none_or(|r| r.len() == cols)
					{
						let range =
							vanished_range(shift, self.slide_static_top, self.slide_static, lines);
						let chunk = self.last_cells[range].to_vec();
						if !chunk.is_empty() {
							self.strip.push_step(shift.signum() as i8, chunk);
							self.strip_dirty = true;
						}
					}
					self.scroll
						.app_scroll(self.scroll.app_offset() + shift as f32);
				}
			}
			self.last_rows = rows;
			self.cells_scratch = std::mem::replace(&mut self.last_cells, cur_cells);
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
		// Dev trace for the alt-screen slide (SILK_SCROLLDBG). Off = one cached bool
		// check per frame. The per-frame (sh, app_off, slide_sh, st, sb) sequence is
		// the deterministic proof that the slide eases smoothly (app_off monotonic, no
		// bounce) without needing to eyeball a render - see the headless bounce harness.
		if scroll_dbg() && settings.smooth_scroll_apps && alt {
			let frame = DBG_FRAME.fetch_add(1, Ordering::Relaxed);
			eprintln!(
				"SCROLLDBG f={frame} pane={} sh={shift_dbg} app_off={app_off:.4} slide_sh={:.4} st={} sb={} frac={frac:.4}",
				self.id, self.slide_sh, self.slide_static_top, self.slide_static,
			);
		}
		// Region-aware slide: only the middle scroll region shifts by voff; a static
		// bottom band (status/input line) and a static top band (title bar) hold their
		// fractional-only position. `split_row` is the first row of the bottom band;
		// `top_split_row` is the first row of the scroll region (just below the title).
		// No bands (or no active slide) => whole pane at voff.
		let static_rows = if app_off != 0.0 {
			self.slide_static.min(lines)
		} else {
			0
		};
		let static_top = if app_off != 0.0 {
			self.slide_static_top.min(lines.saturating_sub(static_rows))
		} else {
			0
		};
		let split_row = (lines - static_rows) as i32;
		let top_split_row = static_top as i32;
		let voff_of = |screen_row: i32| {
			if (static_top > 0 && screen_row < top_split_row)
				|| (static_rows > 0 && screen_row >= split_row)
			{
				frac
			} else {
				voff
			}
		};
		let display_offset = desired as i32;
		let hist = history as i32;
		// fractional scroll shifts content DOWN by frac of a cell; we render an
		// extra row above (screen row -1) so the revealed strip is filled.
		let y_of = |screen_row: i32| {
			self.rect.y + margin + (screen_row as f32 + voff_of(screen_row)) * cell_h
		};
		// The scroll-region draw origin is always the SHIFTED position, independent of
		// the bands (which are redrawn unshifted at band_top); grid elements use y_of.
		let top = self.rect.y + margin + (-1.0 + voff) * cell_h;
		// Slide geometry (only while app_off is easing). The current frame draws at
		// `top` (scroll region, clipped to [top_split_y, split_y]); the scrolled-off
		// strip fills the gap the shift opens - above the content when sliding down
		// (app_off > 0), below it when sliding up. The strip is welded to the content
		// edge and rides the same eased offset, so it never moves relative to the
		// content: its last row ends exactly at the region's first row (up-scroll),
		// or its first row starts one past the region's last (down-scroll).
		let slide = if app_off != 0.0 {
			// split_y bounds the scroll region below; top_split_y bounds it above (a
			// static top band sits above it; f32::MIN = no band, so the clip is open).
			let split_y = if static_rows > 0 {
				self.rect.y + margin + (split_row as f32 + frac) * cell_h
			} else {
				self.rect.y + self.rect.h
			};
			let top_split_y = if static_top > 0 {
				self.rect.y + margin + (top_split_row as f32 + frac) * cell_h
			} else {
				f32::MIN
			};
			let band_top = self.rect.y + margin + (-1.0 + frac) * cell_h;
			let strip_top = if app_off > 0.0 {
				self.rect.y
					+ margin + (top_split_row as f32 + voff - self.strip.len() as f32) * cell_h
			} else {
				self.rect.y + margin + (split_row as f32 + voff) * cell_h
			};
			// content extent = first/one-past-last region row at the shifted position
			let content_top_y = self.rect.y + margin + (top_split_row as f32 + voff) * cell_h;
			let content_bot_y = self.rect.y + margin + (split_row as f32 + voff) * cell_h;
			let (region_clip_t, region_clip_b) =
				weld_region_clip(top_split_y, split_y, content_top_y, content_bot_y);
			Some(Slide {
				strip_top,
				top_split_y,
				split_y,
				region_clip_t,
				region_clip_b,
				band_top,
				has_band: static_rows > 0,
				has_top_band: static_top > 0,
			})
		} else {
			None
		};
		// gesture over: the revealed gap is gone, drop the strip
		if slide.is_none() && self.strip.len() > 0 {
			self.strip.clear();
		}

		// Cursor position/shape as plain values (no lasting borrow of the lock), so
		// the fast path below can drop the term lock immediately.
		let cursor_pt = guard.grid().cursor.point;
		let cursor_shape = guard.cursor_style().shape;
		// Alt-screen apps own their cursor shape; on the primary screen it's the
		// configured geometry (or the app's DECSCUSR). See cursor_geometry.
		let cursor_geom =
			cursor_geometry(cursor_shape, guard.mode().contains(TermMode::ALT_SCREEN));
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
				cursor_geom,
				display_offset,
				lines,
				following,
				content_x,
				cell_w,
				cell_h,
				margin,
				voff,
				dt,
				settings.cursor,
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

		// While a slide eases, region rows shift by voff but rect quads only get the
		// per-pane scissor (no per-area clip like text) - clamp region-row rects to
		// the region so an own-bg row (inverse video, a coloured block) can't poke
		// into the title/status bands mid-slide.
		let region_rect_clip = slide.as_ref().map(|sl| {
			(
				if sl.has_top_band {
					sl.top_split_y
				} else {
					self.rect.y + margin
				},
				if sl.has_band {
					sl.split_y
				} else {
					self.rect.y + self.rect.h - margin
				},
			)
		});

		// Build attr-runs spanning the viewport (+1 overscan row). Newlines are
		// embedded into runs (never empty/standalone spans) - empty spans make
		// cosmic-text's set_rich_text loop forever.
		let mut spans: Vec<(String, Attrs)> = Vec::with_capacity(lines + 1);
		let mut run = String::new();
		let mut run_color = settings.fg;
		let mut run_bold = false;
		let mut run_italic = false;
		let mut saw_bold = false;

		macro_rules! flush_run {
			() => {
				if !run.is_empty() {
					let mut attrs = mono_attrs();
					attrs.color_opt = Some(GColor::rgb(run_color[0], run_color[1], run_color[2]));
					if run_bold {
						attrs.weight = Weight::BOLD;
					}
					if run_italic {
						attrs.style = Style::Italic;
					}
					spans.push((std::mem::take(&mut run), attrs));
				}
			};
		}

		for screen_row in -1..(lines as i32) {
			if screen_row != -1 {
				run.push('\n');
			}
			let grid_line = screen_row - display_offset; // grid line for this screen row
			if grid_line < -hist || grid_line > (lines as i32 - 1) {
				continue; // off the top/bottom of real content: blank row
			}
			let row = &grid[Line(grid_line)];
			let y = y_of(screen_row);
			for c in 0..cols {
				let cell = &row[Column(c)];
				let flags = cell.flags;
				if flags.contains(Flags::WIDE_CHAR_SPACER) {
					continue;
				}
				let mut fg = palette::resolve(cell.fg, colors, &settings);
				let mut cell_bg = palette::resolve(cell.bg, colors, &settings);
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
					sel_range.is_some_and(|r| r.contains(Point::new(Line(grid_line), Column(c))));
				let bg_color = if selected {
					Some(config::SELECTION_BG)
				} else if cell_bg != settings.bg {
					Some(cell_bg)
				} else {
					None
				};
				if let Some(col) = bg_color {
					let (mut rect_top, mut rect_bot) = (y, y + cell_h);
					if let Some((clip_t, clip_b)) = region_rect_clip {
						if screen_row >= top_split_row && screen_row < split_row {
							rect_top = rect_top.max(clip_t);
							rect_bot = rect_bot.min(clip_b);
						}
					}
					if rect_bot > rect_top {
						bg.push(RectInstance {
							pos: [content_x + c as f32 * cell_w, rect_top],
							size: [cell_w, rect_bot - rect_top],
							color: config::srgb_f32(col),
						});
					}
				}

				// reverse-video (dark-on-light) text renders visually thinner than the
				// same weight light-on-dark; embolden it so inverse chrome (nano/vim
				// title+status bars) reads as strongly as normal text.
				let bold = flags.contains(Flags::BOLD)
					|| (settings.embolden_inverse && flags.contains(Flags::INVERSE));
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
					glyph_specs.push((cell.c, fg, bold, italic, c, screen_row, w as u8));
				} else {
					if (fg, bold, italic) != (run_color, run_bold, run_italic) {
						flush_run!();
						run_color = fg;
						run_bold = bold;
						run_italic = italic;
					}
					run.push(render_char(cell.c));
				}
			}
		}
		flush_run!();

		drop(guard);
		let mut cursor = self.cursor_quad(
			cursor_pt,
			cursor_shape,
			cursor_geom,
			display_offset,
			lines,
			following,
			content_x,
			cell_w,
			cell_h,
			margin,
			voff_of(cursor_pt.line.0 + display_offset),
			dt,
			settings.cursor,
		);
		// the cursor rides the sliding region too - clamp it like the bg rects
		// (only when it's a region row; nano parks it in the status band on ^W)
		if let Some((clip_t, clip_b)) = region_rect_clip {
			let cursor_row = cursor_pt.line.0 + display_offset;
			if cursor_row >= top_split_row && cursor_row < split_row {
				if let Some(q) = &mut cursor {
					let bot = (q.pos[1] + q.size[1]).min(clip_b);
					q.pos[1] = q.pos[1].max(clip_t);
					q.size[1] = (bot - q.pos[1]).max(0.0);
				}
			}
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

		// Re-shape the scrolled-off strip when its rows changed this frame. Cheap:
		// the strip is at most OffStrip::CAP short rows, and only steps dirty it.
		if self.strip_dirty {
			self.strip_dirty = false;
			self.shape_strip(ctx, &settings);
		}
		// Strip cells with their own background (inverse video, coloured bg) keep it
		// while revealed: emit their rects at the strip's slide position, clamped to
		// the region clip like the sliding content's rects above.
		if let (Some(sl), Some((clip_t, clip_b))) = (&slide, region_rect_clip) {
			for (j, row) in self.strip.rows.iter().enumerate() {
				let y = sl.strip_top + j as f32 * cell_h;
				let (rect_top, rect_bot) = (y.max(clip_t), (y + cell_h).min(clip_b));
				if rect_bot <= rect_top {
					continue;
				}
				for (c, cell) in row.iter().enumerate() {
					if cell.wide == 0 {
						continue;
					}
					if let Some(col) = cell.bg {
						bg.push(RectInstance {
							pos: [content_x + c as f32 * cell_w, rect_top],
							size: [cell_w, rect_bot - rect_top],
							color: config::srgb_f32(col),
						});
					}
				}
			}
		}

		// Scrim source with uniform weight: bold ink is wider, so its halo reads
		// heavier than the neighbours'. When text_scrim_regular_weight is on and
		// bold is on screen, shape a parallel buffer with bold stripped for the
		// scrim pass (crisp text on top keeps its real weight). Costs a second
		// shape only on rebuild frames that contain bold. Per-cell fallback
		// glyphs keep their weight - rare, and not worth a second glyph pool.
		self.scrim_debold = settings.text_scrim
			&& settings.text_scrim_radius > 0.0
			&& settings.text_scrim_regular_weight
			&& saw_bold;
		if self.scrim_debold {
			let (buf_w, buf_h) = self.buffer.size();
			let scrim_buffer = self.scrim_buf.get_or_insert_with(|| {
				let mut buf = Buffer::new(&mut ctx.font_system, ctx.metrics);
				buf.set_wrap(&mut ctx.font_system, glyphon::Wrap::None);
				buf.set_monospace_width(&mut ctx.font_system, Some(cell_w));
				buf
			});
			scrim_buffer.set_metrics(&mut ctx.font_system, ctx.metrics);
			scrim_buffer.set_size(&mut ctx.font_system, buf_w, buf_h);
			let despan = spans.iter().map(|(text, attrs)| {
				let mut debold_attrs = attrs.clone();
				debold_attrs.weight = default_attrs.weight;
				(text.as_str(), debold_attrs)
			});
			scrim_buffer.set_rich_text(
				&mut ctx.font_system,
				despan,
				&default_attrs,
				Shaping::Advanced,
				None,
			);
			scrim_buffer.shape_until_scroll(&mut ctx.font_system, false);
		}

		// build the per-cell fallback glyphs (reusing the buffer pool)
		self.glyphs.clear();
		let rect_y = self.rect.y;
		for (i, (ch, color, bold, italic, c, screen_row, cells)) in
			glyph_specs.into_iter().enumerate()
		{
			let mut attrs = mono_attrs();
			attrs.color_opt = Some(GColor::rgb(color[0], color[1], color[2]));
			if bold {
				attrs.weight = Weight::BOLD;
			}
			if italic {
				attrs.style = Style::Italic;
			}
			if i >= self.glyph_bufs.len() {
				let buf = ctx.new_plain_buffer();
				self.glyph_bufs.push(buf);
			}
			let (ink_w, ink_off) = ctx.fill_glyph(&mut self.glyph_bufs[i], ch, &attrs);
			// Fit the ink inside its cell box (cells * cell_w wide), only ever
			// shrinking, and center it there - a fallback face's wider-than-a-cell
			// ink would otherwise spill over the next cell and collide with its
			// text. Back out the ink offset so centering is on the ink, not the pen.
			let target = cells as f32 * cell_w;
			let scale = if ink_w > target { target / ink_w } else { 1.0 };
			let cell_x = content_x + c as f32 * cell_w;
			let x = cell_x + (target - ink_w * scale) / 2.0 - ink_off * scale;
			let y = rect_y
				+ margin + (screen_row as f32 + voff_of(screen_row)) * cell_h
				+ cell_h * (1.0 - scale) / 2.0;
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
		cursor_geom: (f32, f32),
		display_offset: i32,
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
		let cursor_screen_row = cursor_pt.line.0 + display_offset;
		let shown = following
			&& cursor_shape != CursorShape::Hidden
			&& cursor_screen_row >= 0
			&& (cursor_screen_row as usize) < lines;
		self.cursor_animating = false;
		if !shown {
			return None;
		}
		let target_col = cursor_pt.column.0 as f32;
		let row_jump = !self.cursor_init || cursor_screen_row != self.cursor_row;
		let moved = row_jump || (target_col - self.cursor_col).abs() > 0.001;
		if row_jump {
			self.cursor_x = target_col; // snap on first sight / newline (no diagonal slide)
		}
		if moved {
			self.cursor_idle_t = 0.0; // reset idle timer on any cursor move
		} else {
			self.cursor_idle_t += dt;
		}
		self.cursor_init = true;
		self.cursor_row = cursor_screen_row;
		self.cursor_col = target_col;
		let k = 1.0 - (-dt * 1000.0 / CURSOR_MOVE_TAU_MS).exp();
		self.cursor_x += (target_col - self.cursor_x) * k;
		let easing = (target_col - self.cursor_x).abs() > 0.01;
		if !easing {
			self.cursor_x = target_col;
		}
		// Animation: "none" = steady; "phase" = smooth cosine fade; "pulse_*" =
		// grow/shrink a dimension over one cycle. The envelope applies whenever the
		// animation is on - including during a horizontal slide - so the size never
		// jumps on a keystroke. "continuous" runs the cycle unconditionally;
		// "pause" routes through PauseState (glide to full, hold, resume from full).
		let settings = config::settings();
		let anim = settings.cursor_animation.as_str();
		let period = (settings.cursor_blink_rate_ms / 1000.0 * 2.0).max(0.05); // full on->off->on
		let anim_on = anim != "none";
		let full_phase = if anim == "phase" { 0.0 } else { 0.5 };
		if anim_on && settings.cursor_animation_input != "continuous" {
			self.blink_t = self.cursor_pause.advance(
				self.blink_t,
				dt,
				period,
				full_phase,
				CURSOR_INPUT_PAUSE_S,
				moved,
				self.cursor_idle_t,
			);
		} else {
			self.blink_t += dt;
		}
		let animating = anim_on;
		let phase = (self.blink_t / period).fract();

		let (mut w_frac, mut h_frac) = cursor_geom;
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
					let envelope = pulse_env(phase);
					w_frac *= envelope;
					h_frac *= envelope;
					(true, true)
				}
				_ => (false, false),
			}
		} else {
			(false, false)
		};
		// keep frames flowing while a pulse/phase cursor is shown (even during an
		// input pause) so the animation can resume once input goes idle
		self.cursor_animating = easing || anim_on;
		let mut cursor_color = config::srgb_f32(cursor_rgb);
		cursor_color[3] = alpha;
		let cell_y = self.rect.y + margin + (cursor_screen_row as f32 + voff) * cell_h;
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
			color: cursor_color,
		})
	}

	// Same as `text_area` but for the scrim source pass: uses the de-bolded buffer
	// when it was built this frame (text_scrim_regular_weight + bold on screen), so
	// the halo weight matches non-bold text while the crisp text keeps its weight.
	pub fn scrim_text_area<'a>(&'a self, top: f32, margin: f32) -> TextArea<'a> {
		let mut area = self.text_area(top, margin);
		if self.scrim_debold {
			if let Some(scrim_buffer) = &self.scrim_buf {
				area.buffer = scrim_buffer;
			}
		}
		area
	}

	// scrim_text_area with the band clip of text_area_band (see there).
	pub fn scrim_text_area_band<'a>(
		&'a self,
		top: f32,
		margin: f32,
		clip_top: f32,
		clip_bottom: f32,
	) -> TextArea<'a> {
		let mut area = self.scrim_text_area(top, margin);
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
		let mut area = self.text_area(top, margin);
		area.bounds.top = area.bounds.top.max(clip_top as i32);
		area.bounds.bottom = area.bounds.bottom.min(clip_bottom as i32);
		area
	}

	// The scrolled-off strip at its slide position, clipped to the scroll region
	// exactly like the current content (it holds only region rows, so the bands
	// need no protection from it; descender spill across the weld matches what
	// adjacent rows in one buffer do). None while the strip is empty. Serves the
	// scrim pass too - the strip is always scrim-safe, unlike the old retained
	// frame whose own-bg furniture had to be guarded out.
	pub fn strip_text_area<'a>(&'a self, slide: &Slide, margin: f32) -> Option<TextArea<'a>> {
		if self.strip.len() == 0 {
			return None;
		}
		let mut area = self.buf_area(&self.strip_buf, slide.strip_top, margin);
		area.bounds.top = area.bounds.top.max(slide.top_split_y as i32);
		area.bounds.bottom = area.bounds.bottom.min(slide.split_y as i32);
		Some(area)
	}

	// Re-shape the scrolled-off strip buffer from its captured rows. Same span
	// rules as build()'s main loop: runs merged by (colour, bold, italic),
	// newlines embedded into non-empty runs, never empty/standalone spans (they
	// make set_rich_text loop forever). Glyphs the primary mono face lacks stay
	// space placeholders - the strip is transient reveal content, not worth a
	// per-cell fallback pool.
	fn shape_strip(&mut self, ctx: &mut TextCtx, settings: &config::Settings) {
		if self.strip.len() == 0 {
			return;
		}
		fn flush(spans: &mut Vec<(String, Attrs)>, run: &mut String, style: ([u8; 3], bool, bool)) {
			if run.is_empty() {
				return;
			}
			let mut attrs = mono_attrs();
			attrs.color_opt = Some(GColor::rgb(style.0[0], style.0[1], style.0[2]));
			if style.1 {
				attrs.weight = Weight::BOLD;
			}
			if style.2 {
				attrs.style = Style::Italic;
			}
			spans.push((std::mem::take(run), attrs));
		}
		let mut spans: Vec<(String, Attrs)> = Vec::with_capacity(self.strip.len() + 1);
		let mut run = String::new();
		let mut run_style = (settings.fg, false, false);
		for (j, row) in self.strip.rows.iter().enumerate() {
			if j != 0 {
				run.push('\n');
			}
			for cell in row {
				if cell.wide == 0 {
					continue; // wide-char spacer
				}
				if !cell.c.is_ascii() && !ctx.covered(cell.c) {
					for _ in 0..cell.wide {
						run.push(' ');
					}
					continue;
				}
				let style = (cell.fg, cell.bold, cell.italic);
				if style != run_style {
					flush(&mut spans, &mut run, run_style);
					run_style = style;
				}
				run.push(render_char(cell.c));
			}
		}
		flush(&mut spans, &mut run, run_style);
		ctx.resize_buffer(
			&mut self.strip_buf,
			self.rect.w.max(1.0),
			(self.strip.len() as f32 + 1.0) * ctx.cell_h,
		);
		let span_refs = spans.iter().map(|(s, a)| (s.as_str(), a.clone()));
		self.strip_buf.set_rich_text(
			&mut ctx.font_system,
			span_refs,
			&mono_attrs(),
			Shaping::Advanced,
			None,
		);
		self.strip_buf
			.shape_until_scroll(&mut ctx.font_system, false);
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
	// Blocking (unfair) lock: a try_lock here silently skipped that command's
	// copy whenever Enter raced a PTY burst.
	pub fn arm_capture(&mut self) {
		if !self.term.at_shell_prompt() {
			return;
		}
		let guard = self.term.term.lock_unfair();
		let grid = guard.grid();
		let cursor_line = grid.cursor.point.line;
		self.cmd_start = grid.history_size() + cursor_line.0.max(0) as usize + 1;
		// fingerprint the prompt row so capture can re-find it (see cmd_anchor);
		// an all-blank row is too ambiguous to anchor on (blank output lines match)
		let cols = grid.columns();
		let row = &grid[cursor_line];
		let blank = (0..cols).all(|c| row[Column(c)].c == ' ');
		self.cmd_anchor = (!blank).then(|| fnv_row((0..cols).map(|c| row[Column(c)].c)));
		self.capture_armed = true;
		self.last_output = std::time::Instant::now();
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
		let guard = self.term.term.try_lock_unfair()?;
		self.capture_armed = false;
		let end = {
			let grid = guard.grid();
			grid.history_size() + grid.cursor.point.line.0.max(0) as usize
		};
		let start = capture_start(&guard, self.cmd_start, self.cmd_anchor, end);
		let text = capture_grid_text(&guard, start, end);
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
		let screen_row = ((y - self.rect.y - ctx.margin) / ctx.cell_h)
			.floor()
			.clamp(0.0, (lines - 1) as f32) as i32;
		let display_offset = self.term.term.lock_unfair().grid().display_offset() as i32;
		Some((
			Point::new(Line(screen_row - display_offset), Column(col as usize)),
			side,
		))
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
			let guard = self.term.term.lock_unfair();
			let grid = guard.grid();
			(0..cols).map(|c| grid[point.line][Column(c)].c).collect()
		};
		let (start, end) = pair_inside(&row, col, pairs)?;
		Some((
			Point::new(point.line, Column(start)),
			Point::new(point.line, Column(end)),
		))
	}

	// The whole logical line containing `point`, spanning soft-wrapped rows, as
	// (top-row col 0 .. bottom-row last col) - the span a triple-click selects.
	pub fn line_span(&self, point: Point) -> (Point, Point) {
		let cols = self.term.cols;
		let last_col = Column(cols.saturating_sub(1));
		let guard = self.term.term.lock_unfair();
		let grid = guard.grid();
		let top = -(grid.history_size() as i32);
		let bot = self.term.lines as i32 - 1;
		let wrapped = |l: i32| cols > 0 && grid[Line(l)][last_col].flags.contains(Flags::WRAPLINE);
		let (start, end) = logical_line_bounds(point.line.0, top, bot, wrapped);
		(
			Point::new(Line(start), Column(0)),
			Point::new(Line(end), last_col),
		)
	}

	pub fn begin_selection(&self, point: Point, side: Side, ty: SelectionType) {
		self.term.term.lock_unfair().selection = Some(Selection::new(ty, point, side));
	}

	pub fn update_selection(&self, point: Point, side: Side) {
		let mut guard = self.term.term.lock_unfair();
		if let Some(sel) = guard.selection.as_mut() {
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
			pane.strip_buf = ctx.new_buffer(pane.rect.w.max(1.0), ctx.cell_h);
			pane.strip.clear(); // metrics changed; a mid-slide strip would misalign
			pane.strip_dirty = false;
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
				// a resize invalidates the strip's captured columns and the
				// frame-old styled snapshot it fills from
				pane.strip.clear();
				pane.strip_dirty = false;
				pane.last_cells.clear();
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
		Node::Split {
			a: child_a,
			b: child_b,
			..
		} => {
			swap_leaves(child_a, a, b);
			swap_leaves(child_b, a, b);
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
	let strip_buf = ctx.new_buffer(cw, ctx.cell_h);
	Ok(Pane {
		id,
		term,
		scroll: Scroll::new(),
		buffer,
		strip: OffStrip::new(),
		strip_buf,
		strip_dirty: false,
		last_cells: Vec::new(),
		cells_scratch: Vec::new(),
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
		slide_static_top: 0,
		slide_sh: 0.0,
		last_alt: false,
		glyph_bufs: Vec::new(),
		glyphs: Vec::new(),
		scrim_buf: None,
		scrim_debold: false,
		cursor_x: 0.0,
		cursor_col: 0.0,
		cursor_row: i32::MIN,
		cursor_init: false,
		blink_t: 0.0,
		cursor_idle_t: 0.0,
		cursor_pause: PauseState::default(),
		cursor_animating: false,
		text_built: false,
		mode: TermMode::empty(),
		content_dirty: true,
		auto_copy: false,
		capture_armed: false,
		cmd_start: 0,
		cmd_anchor: None,
		last_output: std::time::Instant::now(),
	})
}

// Where the captured output starts, as a capture-time absolute line index.
// `cmd_start` was recorded at arm time in "history + row" coordinates, but that
// origin moves once the scrollback is at cap: each pushed line evicts the
// oldest, shifting every absolute index down, so the stale index lands past the
// start and the copy silently drops the first lines of the output. Re-find the
// arm-time prompt row by its content hash instead, scanning back from the end
// (the nearest match is the arm-time prompt unless the output itself repeats
// that exact row); the output starts on the next line. Fall back to `cmd_start`
// when there's no anchor or no match (blank prompt row, the row was evicted, or
// the shell redrew it on Enter).
fn capture_start<T: alacritty_terminal::event::EventListener>(
	term: &Term<T>,
	cmd_start: usize,
	anchor: Option<u64>,
	end_abs: usize,
) -> usize {
	let Some(anchor) = anchor else {
		return cmd_start;
	};
	let grid = term.grid();
	let hist = grid.history_size() as i64;
	let cols = grid.columns();
	for abs in (0..end_abs).rev() {
		let row = &grid[Line((abs as i64 - hist) as i32)];
		if fnv_row((0..cols).map(|c| row[Column(c)].c)) == anchor {
			return abs + 1;
		}
	}
	cmd_start
}

// Extract the grid text for absolute line range [start_abs, end_abs) as plain
// Unicode. Absolute index 0 is the oldest line currently in the buffer; screen
// row 0 sits at absolute `history_size`. Trailing pad spaces are trimmed and a
// newline is emitted per grid row, except rows flagged WRAPLINE (a soft-wrapped
// long line) which join to the next. Lines evicted from scrollback (only when a
// command's output exceeds the scrollback limit) are skipped.
fn capture_grid_text<T: alacritty_terminal::event::EventListener>(
	term: &Term<T>,
	start_abs: usize,
	end_abs: usize,
) -> String {
	let grid = term.grid();
	let hist = grid.history_size() as i64;
	let cols = grid.columns();
	let mut out = String::new();
	let mut abs_line = start_abs;
	while abs_line < end_abs {
		let grid_line = abs_line as i64 - hist; // screen top is absolute `hist`; history is negative
		if grid_line < -hist {
			abs_line += 1; // scrolled out of the buffer (output longer than scrollback)
			continue;
		}
		let row = &grid[Line(grid_line as i32)];
		let mut row_text = String::new();
		for c in 0..cols {
			let cell = &row[Column(c)];
			if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
				continue; // the trailing half of a wide glyph has no char of its own
			}
			row_text.push(cell.c);
		}
		if cols > 0 && row[Column(cols - 1)].flags.contains(Flags::WRAPLINE) {
			out.push_str(&row_text); // soft-wrapped: continue the logical line, no newline
		} else {
			out.push_str(row_text.trim_end());
			out.push('\n');
		}
		abs_line += 1;
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
		if let Some((open_idx, close_idx)) = found {
			if close_idx > open_idx + 1 {
				// Exclude runs of spaces directly against the delimiters (keep any
				// interior spaces): `" Now is the time. "` selects `Now is the time.`.
				let (mut start, mut end) = (open_idx + 1, close_idx - 1);
				while start < end && row[start] == ' ' {
					start += 1;
				}
				while end > start && row[end] == ' ' {
					end -= 1;
				}
				// all-spaces inside: fall back to the full inside span
				return Some(if row[start] == ' ' {
					(open_idx + 1, close_idx - 1)
				} else {
					(start, end)
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
	let mut open_idx = None;
	for i in (0..col).rev() {
		if row[i] == close {
			depth += 1;
		} else if row[i] == open {
			if depth == 0 {
				open_idx = Some(i);
				break;
			}
			depth -= 1;
		}
	}
	let open_idx = open_idx?;
	let mut depth = 0i32;
	for (i, &ch) in row.iter().enumerate().skip(col + 1) {
		if ch == open {
			depth += 1;
		} else if ch == close {
			if depth == 0 {
				return Some((open_idx, i));
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
			if let Some(mut path) = path_to(a, id) {
				path.insert(0, false);
				return Some(path);
			}
			if let Some(mut path) = path_to(b, id) {
				path.insert(0, true);
				return Some(path);
			}
			None
		}
	}
}

// Follow `path` from `node` (defensively stops at a leaf).
fn node_at_mut<'a>(mut node: &'a mut Node, path: &[bool]) -> &'a mut Node {
	for &take_b in path {
		let Node::Split { a, b, .. } = node else {
			break;
		};
		node = if take_b { b } else { a };
	}
	node
}

// Is the node at `path` a Split oriented along `dir`?
fn is_dir_split(root: &Node, path: &[bool], dir: Dir) -> bool {
	let mut node = root;
	for &take_b in path {
		let Node::Split { a, b, .. } = node else {
			return false;
		};
		node = if take_b { b } else { a };
	}
	matches!(node, Node::Split { dir: node_dir, .. } if *node_dir == dir)
}

// Leaves in the same-direction run rooted at `node`: a nested `dir` split counts
// its members; a leaf or a differently-oriented split counts as one unit (its own
// internal layout is separate).
fn group_leaf_count(node: &Node, dir: Dir) -> usize {
	match node {
		Node::Split {
			dir: node_dir,
			a,
			b,
			..
		} if *node_dir == dir => group_leaf_count(a, dir) + group_leaf_count(b, dir),
		_ => 1,
	}
}

// Has any divider in the same-direction run been hand-dragged?
fn group_has_manual(node: &Node, dir: Dir) -> bool {
	match node {
		Node::Split {
			dir: node_dir,
			manual,
			a,
			b,
			..
		} if *node_dir == dir => *manual || group_has_manual(a, dir) || group_has_manual(b, dir),
		_ => false,
	}
}

// Set every ratio in the same-direction run so all its member leaves are equal:
// a split gives its a-child a share proportional to the leaves under it.
fn equalize(node: &mut Node, dir: Dir) {
	if let Node::Split {
		dir: node_dir,
		ratio,
		a,
		b,
		..
	} = node
	{
		if *node_dir == dir {
			let leaves_a = group_leaf_count(a, dir);
			let leaves_b = group_leaf_count(b, dir);
			*ratio = leaves_a as f32 / (leaves_a + leaves_b) as f32;
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
			let pruned_a = prune(*a, id);
			let pruned_b = prune(*b, id);
			match (pruned_a, pruned_b) {
				(Some(a), Some(b)) => Some(Node::Split {
					dir,
					ratio,
					manual,
					a: Box::new(a),
					b: Box::new(b),
				}),
				(Some(survivor), None) | (None, Some(survivor)) => Some(survivor),
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
			let a_width = ((area.w - gap) * ratio).floor();
			(
				Rect {
					x: area.x,
					y: area.y,
					w: a_width,
					h: area.h,
				},
				Rect {
					x: area.x + a_width + gap,
					y: area.y,
					w: area.w - gap - a_width,
					h: area.h,
				},
			)
		}
		Dir::Horizontal => {
			let a_height = ((area.h - gap) * ratio).floor();
			(
				Rect {
					x: area.x,
					y: area.y,
					w: area.w,
					h: a_height,
				},
				Rect {
					x: area.x,
					y: area.y + a_height + gap,
					w: area.w,
					h: area.h - gap - a_height,
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
		if let Some(found_dir) = divider_at(a, a_area, x, y, path) {
			return Some(found_dir);
		}
		path.pop();
	}
	if b_area.contains(x, y) {
		path.push(true);
		if let Some(found_dir) = divider_at(b, b_area, x, y, path) {
			return Some(found_dir);
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
	let new_ratio = match dir {
		Dir::Vertical => (x - area.x) / (area.w - gap),
		Dir::Horizontal => (y - area.y) / (area.h - gap),
	};
	*ratio = new_ratio.clamp(0.05, 0.95);
	*manual = true; // dragged: stop auto even-distribution for this run
}

// Lines the on-screen content scrolled up between frames, inferred from row
// fingerprints when scrollback growth can't tell us (the buffer is full). It's
// the smallest shift k where this frame's top (rows-k) lines equal last frame's
// bottom (rows-k) lines.
// Signed sibling of scroll_shift for alt-screen app-scroll easing: detect a clean
// vertical translate between two frames, in either direction, up to `max` lines.
// +k = scrolled forward (content moved up k rows), -k = scrolled back (down k).
// Real full-screen apps keep static chrome bands - a status/input line at the
// BOTTOM (less, vim) and often a title bar at the TOP (nano, muffer) - so the
// scrolling region is a middle block, not a top prefix. We therefore count, for
// each candidate k, how many rows translate cleanly ANYWHERE (cur[i]==last[i+k]),
// and pick the k with the most. A shift counts only if a solid block translates
// (>= `need`) AND enough of those rows actually MOVED (cur[i]!=last[i], >= MOVED_MIN)
// - a static or blank field matches positionally but hasn't scrolled, and easing
// that produces the apt/blank-jitter bounce. Otherwise 0 (in-place redraw, content
// change, or a jump bigger than `max`) and the caller hard-cuts. 64-bit row
// fingerprints make a coincidental non-translation match vanishingly unlikely, so
// no contiguity check is needed. It never guesses a full turnover the way
// scroll_shift does - easing a non-scroll looks wrong.
const MOVED_MIN: usize = 3; // a real scroll must move at least this many rows
fn scroll_shift_signed(cur: &[u64], last: &[u64], max: usize) -> i32 {
	let n = cur.len();
	if n == 0 || last.len() != n {
		return 0;
	}
	// a quarter of the screen, since static top+bottom bands shrink the middle
	let need = (n / 4).max(3);
	let limit = max.min(n - 1);
	let (mut best, mut best_score) = (0i32, 0usize);
	for k in 1..=limit {
		// forward: content moved up k rows -> cur[i] == last[i+k]
		let (mut matched, mut moved) = (0usize, 0usize);
		for i in 0..n - k {
			if cur[i] == last[i + k] {
				matched += 1;
				if cur[i] != last[i] {
					moved += 1;
				}
			}
		}
		if matched >= need && moved >= MOVED_MIN && matched > best_score {
			best_score = matched;
			best = k as i32;
		}
		// backward: content moved down k rows -> cur[i+k] == last[i]
		let (mut matched, mut moved) = (0usize, 0usize);
		for i in 0..n - k {
			if cur[i + k] == last[i] {
				matched += 1;
				if cur[i + k] != last[i + k] {
					moved += 1;
				}
			}
		}
		if matched >= need && moved >= MOVED_MIN && matched > best_score {
			best_score = matched;
			best = -(k as i32);
		}
	}
	best
}

// Count the static (unchanged) rows at the top and bottom edges between two
// frames: a fixed title bar (nano/muffer) above and a status/help band below the
// scrolling region. Returns (top, bottom); zeroed if the two would meet or cover
// the whole screen (no distinct scroll region). Measured only on a gesture's first
// step - see build() - so mid-scroll fluctuation can't jitter the band boundary.
fn static_bands(cur: &[u64], last: &[u64]) -> (usize, usize) {
	let n = cur.len();
	if last.len() != n {
		return (0, 0);
	}
	let mut st = 0;
	while st < n && cur[st] == last[st] {
		st += 1;
	}
	let mut sb = 0;
	while sb < n && cur[n - 1 - sb] == last[n - 1 - sb] {
		sb += 1;
	}
	if st + sb >= n { (0, 0) } else { (st, sb) }
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
		APP_SCROLL_MAX, Dir, Node, OffStrip, PauseState, Rect, SLIDE_TOP_BAND_APPS, StripCell,
		bell_brighten, capture_grid_text, capture_start, distinct_pair, equalize_dir_run, fnv_row,
		glide_to_full, layout, logical_line_bounds, pair_inside, render_char, same_char_pair,
		scroll_shift, scroll_shift_signed, static_bands, vanished_range, weld_region_clip,
	};
	use alacritty_terminal::event::{Event, EventListener};
	use alacritty_terminal::grid::Dimensions;
	use alacritty_terminal::index::{Column, Line};
	use alacritty_terminal::term::{Config as TermConfig, Term};
	use alacritty_terminal::vte::ansi::Processor;

	struct VoidListener;
	impl EventListener for VoidListener {
		fn send_event(&self, _e: Event) {}
	}

	// A small live Term fed via the real parser, for the copy-output tests.
	fn term_fed(cols: usize, lines: usize, scrollback: usize, input: &str) -> Term<VoidListener> {
		let cfg = TermConfig {
			scrolling_history: scrollback,
			..Default::default()
		};
		let dims = crate::term::TermDimensions {
			columns: cols,
			screen_lines: lines,
		};
		let mut term = Term::new(cfg, &dims, VoidListener);
		let mut parser: Processor = Processor::new();
		parser.advance(&mut term, input.as_bytes());
		term
	}
	fn feed(term: &mut Term<VoidListener>, input: &str) {
		let mut parser: Processor = Processor::new();
		parser.advance(term, input.as_bytes());
	}
	fn row_hash(term: &Term<VoidListener>, line: i32) -> u64 {
		let grid = term.grid();
		let cols = grid.columns();
		fnv_row((0..cols).map(|c| grid[Line(line)][Column(c)].c))
	}

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
	fn logical_line_bounds_spans_wrapped_rows() {
		// rows 2 and 3 each wrap into the next, so 2..=4 is one logical line
		let w = |l: i32| l == 2 || l == 3;
		assert_eq!(logical_line_bounds(3, -10, 9, w), (2, 4));
		assert_eq!(logical_line_bounds(2, -10, 9, w), (2, 4));
		assert_eq!(logical_line_bounds(4, -10, 9, w), (2, 4));
		// an unwrapped row is its own line
		assert_eq!(logical_line_bounds(6, -10, 9, w), (6, 6));
		// clamps to [top, bot]
		assert_eq!(logical_line_bounds(100, 0, 9, |_| false), (9, 9));
		assert_eq!(logical_line_bounds(-100, 0, 9, |_| false), (0, 0));
		// never walks past top, and walks the full run downward
		assert_eq!(logical_line_bounds(0, 0, 9, |_| true), (0, 9));
	}

	#[test]
	fn glide_to_full_runs_at_normal_speed_and_flags_arrival() {
		let period = 1.0;
		// pulse: full_phase 0.5. Starting mid-shrink (0.7) it advances plain +dt
		// (no speed change, no snap), wraps, and flags arrival crossing 0.5.
		let (t, arrived) = glide_to_full(0.7, 0.01, period, 0.5);
		assert!(!arrived);
		assert!((t - 0.71).abs() < 1e-6);
		let mut t = 0.7;
		let mut steps = 0;
		loop {
			let (next, arrived) = glide_to_full(t, 0.01, period, 0.5);
			t = next;
			steps += 1;
			if arrived {
				break;
			}
			assert!(steps < 1000, "never reached full");
		}
		// 0.7 -> wrap -> 0.5 is 0.8 of a period at 0.01/step (float slack of one)
		assert!((79..=81).contains(&steps), "steps = {steps}");
		assert!(((t / period).fract() - 0.5).abs() < 1e-6);
		// phase mode: full_phase 0.0 - arrival lands on a whole-period multiple
		let (p, arrived) = glide_to_full(0.95, 0.1, period, 0.0);
		assert!(arrived);
		assert!((p / period).fract().abs() < 1e-6 || ((p / period).fract() - 1.0).abs() < 1e-6);
	}

	#[test]
	fn pause_state_glides_holds_then_resumes_from_full() {
		let period = 1.0;
		let timeout = 0.35;
		let mut st = PauseState::default();
		// input mid-shrink: the cycle keeps running forward at normal speed - the
		// very next frame is a plain +dt, not a jump to full
		let mut t = st.advance(0.7, 0.01, period, 0.5, timeout, true, 0.0);
		assert!((t - 0.71).abs() < 1e-6);
		assert!(st.active && !st.parked);
		// runs on around the cycle and parks exactly at the full-size phase, even
		// though the idle timeout expires long before it gets there
		let mut idle = 0.01;
		for _ in 0..200 {
			t = st.advance(t, 0.01, period, 0.5, timeout, false, idle);
			idle += 0.01;
			if st.parked {
				break;
			}
		}
		assert!(st.parked);
		assert!(((t / period).fract() - 0.5).abs() < 1e-6);
		// typing while parked keeps it parked at full
		t = st.advance(t, 0.01, period, 0.5, timeout, true, 0.0);
		assert!(st.parked && ((t / period).fract() - 0.5).abs() < 1e-6);
		// holds through the timeout after the last input, then resumes from full:
		// the first resumed frame is full_phase + dt, so the size is continuous
		idle = 0.01;
		let mut resumed = None;
		for _ in 0..200 {
			t = st.advance(t, 0.01, period, 0.5, timeout, false, idle);
			idle += 0.01;
			if !st.active {
				resumed = Some(t);
				break;
			}
		}
		let t = resumed.expect("should resume");
		assert!((t - (0.5 * period + 0.01)).abs() < 1e-6);
		// and once resumed it just accumulates
		let t2 = st.advance(t, 0.01, period, 0.5, timeout, false, 1.0);
		assert!((t2 - (t + 0.01)).abs() < 1e-6);
	}

	#[test]
	fn pause_state_hold_needs_both_idle_and_hold_timeouts() {
		let period = 1.0;
		let timeout = 0.35;
		let mut st = PauseState::default();
		// start already near full so it parks on the first step
		let mut t = st.advance(0.49, 0.02, period, 0.5, timeout, true, 0.0);
		assert!(st.parked);
		// idle long past the timeout, but the hold itself must also last it: a
		// glide that ate the idle window still gets a real pause at full
		t = st.advance(t, 0.01, period, 0.5, timeout, false, 10.0);
		assert!(st.active && ((t / period).fract() - 0.5).abs() < 1e-6);
		// conversely, held long enough but input still recent keeps it parked
		for _ in 0..100 {
			t = st.advance(t, 0.01, period, 0.5, timeout, false, 0.1);
		}
		assert!(st.active && ((t / period).fract() - 0.5).abs() < 1e-6);
	}

	#[test]
	fn capture_finds_output_start_at_full_scrollback() {
		// 3 rows, scrollback cap 4 - the command's output fills the buffer to cap
		// and evicts old lines, the long-lived-shell case. The arm-time absolute
		// index goes stale with each eviction; the content anchor must not.
		let mut term = term_fed(20, 3, 4, "h1\r\nh2\r\nh3\r\nh4\r\nuser$ cmd");
		// arm at the prompt (before Enter reaches the terminal)
		let grid = term.grid();
		let cmd_start = grid.history_size() + grid.cursor.point.line.0.max(0) as usize + 1;
		let anchor = Some(row_hash(&term, grid.cursor.point.line.0));
		// the command echoes Enter, prints 4 lines, and a fresh prompt appears
		feed(&mut term, "\r\nO1\r\nO2\r\nO3\r\nO4\r\nuser$ ");
		let grid = term.grid();
		assert_eq!(grid.history_size(), 4, "buffer must have hit the cap");
		let end = grid.history_size() + grid.cursor.point.line.0.max(0) as usize;
		// the anchor recovers the true start; the stale index alone drops lines
		let start = capture_start(&term, cmd_start, anchor, end);
		assert_eq!(capture_grid_text(&term, start, end), "O1\nO2\nO3\nO4\n");
		assert_ne!(
			capture_grid_text(&term, cmd_start, end),
			"O1\nO2\nO3\nO4\n",
			"the stale index should demonstrate the bug this guards against"
		);
		// no anchor (blank prompt row) or no match (row evicted/redrawn): the
		// recorded index is the fallback, never a panic
		assert_eq!(capture_start(&term, cmd_start, None, end), cmd_start);
		assert_eq!(capture_start(&term, cmd_start, Some(1), end), cmd_start);
	}

	#[test]
	fn capture_below_scrollback_cap_matches_either_way() {
		// plenty of scrollback: no eviction, so the stale-index and anchor paths
		// agree - the anchor must not regress the common case
		let mut term = term_fed(20, 3, 100, "user$ cmd");
		let grid = term.grid();
		let cmd_start = grid.history_size() + grid.cursor.point.line.0.max(0) as usize + 1;
		let anchor = Some(row_hash(&term, grid.cursor.point.line.0));
		feed(&mut term, "\r\nA\r\nB\r\nuser$ ");
		let grid = term.grid();
		let end = grid.history_size() + grid.cursor.point.line.0.max(0) as usize;
		let start = capture_start(&term, cmd_start, anchor, end);
		assert_eq!(start, cmd_start);
		assert_eq!(capture_grid_text(&term, start, end), "A\nB\n");
	}

	#[test]
	fn render_char_maps_controls_to_space() {
		// a tab (or any control) left in a cell must shape as a 1-cell space, else
		// the row shifts off the grid and double-click selection misaligns
		assert_eq!(render_char('\t'), ' ');
		assert_eq!(render_char('\0'), ' ');
		assert_eq!(render_char('\r'), ' ');
		assert_eq!(render_char('a'), 'a');
		assert_eq!(render_char(' '), ' ');
		assert_eq!(render_char('世'), '世');
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
		// real-app shape: the middle scrolls but a static status/input band at the
		// bottom stays put - the middle block still translates, so it's detected
		let last_s = [10u64, 20, 30, 40, 900, 901];
		let cur_s = [20u64, 30, 40, 50, 900, 901];
		assert_eq!(scroll_shift_signed(&cur_s, &last_s, 8), 1);
	}

	#[test]
	fn signed_shift_tolerates_static_top_band_and_rejects_static_fields() {
		// nano/muffer shape: a static title bar at the TOP and a status band at the
		// BOTTOM, with the middle region scrolling up by 1. The old top-anchored
		// matcher returned 0 here (row 0 never moved); the block matcher detects it.
		let last = [700u64, 701, 10, 20, 30, 40, 900, 901];
		let cur = [700u64, 701, 20, 30, 40, 50, 900, 901];
		assert_eq!(scroll_shift_signed(&cur, &last, 8), 1);
		// backward (middle slid down 1) with the same static bands
		let back = [700u64, 701, 5, 10, 20, 30, 900, 901];
		assert_eq!(scroll_shift_signed(&back, &last, 8), -1);
		// a large static/blank field matches positionally but hasn't MOVED - must not
		// be read as a scroll (this is the apt/blank-jitter guard). Here rows 1..6 are
		// all identical (a blank band); only row 0 changed, in place. No real scroll.
		let bl_last = [1u64, 5, 5, 5, 5, 5, 5, 9];
		let bl_cur = [2u64, 5, 5, 5, 5, 5, 5, 9];
		assert_eq!(scroll_shift_signed(&bl_cur, &bl_last, 8), 0);
	}

	#[test]
	fn static_bands_measures_title_and_status() {
		// nano shape: static title (rows 0..2), scroll region (2..6), status band (6..8)
		let last = [700u64, 701, 10, 20, 30, 40, 900, 901];
		let cur = [700u64, 701, 20, 30, 40, 50, 900, 901];
		assert_eq!(static_bands(&cur, &last), (2, 2));
		// no bands: every row changed
		let a = [1u64, 2, 3, 4];
		let b = [5u64, 6, 7, 8];
		assert_eq!(static_bands(&a, &b), (0, 0));
		// a fully static frame would have the bands meet -> zeroed (no scroll region)
		assert_eq!(static_bands(&last, &last), (0, 0));
		// length mismatch is not measurable
		assert_eq!(static_bands(&a, &last), (0, 0));
	}

	// ---- App-scroll scenario matrix -------------------------------------------
	// Per-app regression coverage for the alt-screen slide: each real full-screen
	// app repaints in a characteristic shape, and the (shift, static-band) pair the
	// detector extracts decides whether the pane slides smoothly or hard-cuts. These
	// model the shapes so a change to the detector/bands (or the SLIDE_TOP_BAND_APPS
	// toggle) is caught without a live GL run. The committed headless harness
	// (cicd/tests/scroll) exercises the same shapes end-to-end via SILK_SCROLLDBG.

	// Build a (last, cur) frame pair modelling a full-screen app whose middle scroll
	// region moved up by `shift` rows (forward = content scrolls up, newer rows in at
	// the bottom), with `top` static title rows above and `bot` static status rows
	// below. Row fingerprints are arbitrary distinct u64s viewing a rolling window, so
	// a shift reuses neighbouring content rows exactly as a real repaint does.
	fn app_frames(rows: usize, top: usize, bot: usize, shift: i32) -> (Vec<u64>, Vec<u64>) {
		let pool: Vec<u64> = (1000u64..1000 + rows as u64 * 4).collect(); // content pool
		let title: Vec<u64> = (1u64..=top as u64).collect(); // static top band
		let status: Vec<u64> = (900u64..900 + bot as u64).collect(); // static bottom band
		let mid = rows - top - bot;
		let frame = |off: usize| -> Vec<u64> {
			let mut v = title.clone();
			v.extend_from_slice(&pool[off..off + mid]);
			v.extend_from_slice(&status);
			v
		};
		let base = rows; // window origin with room to move either way
		let last = frame(base);
		let cur = frame((base as i32 + shift) as usize);
		(last, cur)
	}

	// The build() decision: engage the smooth slide only when there's no static top
	// band, unless the top-band toggle is on. Mirrors the gate in build().
	fn slide_engages(top_band: usize) -> bool {
		SLIDE_TOP_BAND_APPS || top_band == 0
	}

	#[test]
	fn less_slides_no_top_band() {
		// less fills from the top and keeps only a bottom status line, so there's no
		// static top band: the middle scrolls, the detector sees it, and build slides.
		let (last, cur) = app_frames(24, 0, 1, 1);
		assert_eq!(scroll_shift_signed(&cur, &last, APP_SCROLL_MAX), 1);
		let (st, sb) = static_bands(&cur, &last);
		assert_eq!(st, 0, "less has no static top band");
		assert_eq!(sb, 1, "less keeps a single-row status line");
		assert!(slide_engages(st), "less must slide smoothly");
	}

	#[test]
	fn vim_slides_no_top_band() {
		// vim/vim.tiny paints text from row 0 with a status + command line at the
		// bottom (two static rows), no title bar: same "no top band -> slide" as less.
		let (last, cur) = app_frames(24, 0, 2, 2);
		assert_eq!(scroll_shift_signed(&cur, &last, APP_SCROLL_MAX), 2);
		let (st, sb) = static_bands(&cur, &last);
		assert_eq!(st, 0, "vim has no static top band");
		assert_eq!(sb, 2, "vim keeps a status + command line");
		assert!(slide_engages(st), "vim must slide smoothly");
	}

	#[test]
	fn nano_slides_with_top_band() {
		// nano keeps a title bar at the top and a two-row help band at the bottom, so
		// the middle scroll region has a static top band. With SLIDE_TOP_BAND_APPS on
		// (the scrolled-off strip fills the reveal gap exactly) the slide engages; the
		// expectation tracks the toggle, and the band detection asserted below is the
		// real surface either way.
		let (last, cur) = app_frames(24, 1, 2, 1);
		assert_eq!(scroll_shift_signed(&cur, &last, APP_SCROLL_MAX), 1);
		let (st, sb) = static_bands(&cur, &last);
		assert_eq!(st, 1, "nano keeps a title bar (static top band)");
		assert_eq!(sb, 2, "nano keeps a two-row help band");
		assert_eq!(
			slide_engages(st),
			SLIDE_TOP_BAND_APPS,
			"top-band app slides per the toggle"
		);
	}

	#[test]
	fn muffer_slides_with_top_band() {
		// muffer (the TUI) keeps a static header, so like nano it has a top band and
		// follows the toggle. Model a two-row header + one-row footer.
		let (last, cur) = app_frames(30, 2, 1, 1);
		assert_eq!(scroll_shift_signed(&cur, &last, APP_SCROLL_MAX), 1);
		let (st, _sb) = static_bands(&cur, &last);
		assert_eq!(st, 2, "muffer keeps a static header (top band)");
		assert_eq!(
			slide_engages(st),
			SLIDE_TOP_BAND_APPS,
			"top-band app slides per the toggle"
		);
	}

	#[test]
	fn app_wheel_multi_line_jump_still_detected() {
		// a wheel notch in a mouse-tracking app repaints a several-line jump, not one
		// line: it must still be detected as a clean scroll (up to APP_SCROLL_MAX), not
		// hard-cut as a page turnover. less-shaped so it slides.
		let (last, cur) = app_frames(40, 0, 1, 6);
		assert_eq!(scroll_shift_signed(&cur, &last, APP_SCROLL_MAX), 6);
		// but a jump past the window is not eased (hard-cut) - it isn't a clean scroll
		let (last2, cur2) = app_frames(40, 0, 1, (APP_SCROLL_MAX + 5) as i32);
		assert_eq!(scroll_shift_signed(&cur2, &last2, APP_SCROLL_MAX), 0);
	}

	// ---- Scrolled-off strip -----------------------------------------------------

	// a marker row for strip tests: one cell whose char encodes the row identity
	fn strip_row(tag: char) -> Vec<StripCell> {
		vec![StripCell {
			c: tag,
			fg: [255; 3],
			bg: None,
			bold: false,
			italic: false,
			wide: 1,
		}]
	}

	fn strip_tags(s: &OffStrip) -> String {
		s.rows.iter().map(|r| r[0].c).collect()
	}

	#[test]
	fn vanished_range_picks_the_rows_a_step_pushed_off() {
		// 10 lines, title 1 row, status 2 rows -> region rows 1..8
		// content moved up 2: the region's top two rows left off the top
		assert_eq!(vanished_range(2, 1, 2, 10), 1..3);
		// content moved down 2: the region's bottom two rows left off the bottom
		assert_eq!(vanished_range(-2, 1, 2, 10), 6..8);
		// no bands (less): rows come off the screen edges
		assert_eq!(vanished_range(1, 0, 0, 10), 0..1);
		assert_eq!(vanished_range(-1, 0, 0, 10), 9..10);
		// a shift bigger than the region clamps to it (nothing panics)
		assert_eq!(vanished_range(50, 1, 2, 10), 1..8);
		assert_eq!(vanished_range(-50, 1, 2, 10), 1..8);
	}

	#[test]
	fn region_clip_welds_to_the_content_edge() {
		// down-slide (voff +2 cells, cell_h 20): bands at y=20 (title) / y=160
		// (status); content starts at 20+40=60. The gap 20..60 belongs to the
		// strip - the clip must start at the content edge so the title's
		// translated copy (drawn at 40..60) is cut off. Bottom stays band-bound.
		assert_eq!(weld_region_clip(20.0, 160.0, 60.0, 200.0), (60.0, 160.0));
		// up-slide (voff -2): content ends at 160-40=120; the status rows'
		// translated copies (drawn just above 120) must be cut, gap 120..160 is
		// the strip's. Top stays band-bound.
		assert_eq!(weld_region_clip(20.0, 160.0, -20.0, 120.0), (20.0, 120.0));
		// no top band: f32::MIN stays open until the content edge
		assert_eq!(
			weld_region_clip(f32::MIN, 160.0, 40.0, 200.0),
			(40.0, 160.0)
		);
	}

	#[test]
	fn off_strip_accumulates_in_visual_order() {
		// up-scroll: each step's rows leave off the region's top, newest nearest the
		// content = at the strip's bottom
		let mut s = OffStrip::new();
		s.push_step(1, vec![strip_row('a'), strip_row('b')]);
		s.push_step(1, vec![strip_row('c')]);
		assert_eq!(strip_tags(&s), "abc");
		// down-scroll: rows leave off the bottom, newest at the strip's top,
		// each chunk keeping its internal order
		let mut d = OffStrip::new();
		d.push_step(-1, vec![strip_row('y'), strip_row('z')]);
		d.push_step(-1, vec![strip_row('w'), strip_row('x')]);
		assert_eq!(strip_tags(&d), "wxyz");
	}

	#[test]
	fn off_strip_direction_flip_discards_and_cap_trims_oldest() {
		let mut s = OffStrip::new();
		s.push_step(1, vec![strip_row('a'), strip_row('b')]);
		// flipping direction discards the old strip (it belongs on the other side)
		s.push_step(-1, vec![strip_row('c')]);
		assert_eq!(strip_tags(&s), "c");
		assert_eq!(s.dir, -1);
		// the cap trims the rows farthest from the content (oldest)
		let mut long = OffStrip::new();
		for i in 0..(OffStrip::CAP + 3) {
			long.push_step(1, vec![strip_row(char::from(b'a' + (i % 26) as u8))]);
		}
		assert_eq!(long.len(), OffStrip::CAP);
		// the newest row (nearest the content, strip bottom) survives
		assert_eq!(
			long.rows.back().unwrap()[0].c,
			char::from(b'a' + ((OffStrip::CAP + 2) % 26) as u8)
		);
	}

	// ---- Normal-output (non-alt-screen) scroll scenarios ----------------------
	// Plain shell output eases via scroll_shift (unsigned) + nudge_output. The bugs to
	// guard against: the page "re-listing" itself or "jumping around" (over-reporting
	// a small advance as a full turnover) and not scrolling at all on an in-place
	// bottom redraw (which would bounce). The desired behaviour for a finishing
	// command is just adding new lines at the bottom.

	#[test]
	fn ls_output_adds_lines_at_bottom() {
		// `ls -lA` finishes and the prompt returns: the viewport advanced by exactly
		// the lines printed, not a re-list. One new line at the bottom -> advance 1.
		let last = [10u64, 20, 30, 40, 50, 60];
		let cur = [20u64, 30, 40, 50, 60, 70];
		assert_eq!(scroll_shift(&cur, &last), 1);
		// a short multi-line result advances by exactly that many lines (no re-list)
		let cur3 = [40u64, 50, 60, 70, 80, 90];
		assert_eq!(scroll_shift(&cur3, &last), 3);
	}

	#[test]
	fn command_on_last_line_in_place_does_not_scroll() {
		// running a command whose prompt sits on the last row and only the bottom row
		// changes in place (no newline yet) must not be read as a scroll - nudging here
		// was the old apt/status-line bounce.
		let last = [10u64, 20, 30, 40, 50, 60];
		let cur = [10u64, 20, 30, 40, 50, 99]; // only the last row changed
		assert_eq!(scroll_shift(&cur, &last), 0);
	}

	#[test]
	fn fast_burst_reports_full_backlog_not_a_reversal() {
		// a fast burst (e.g. `seq 100000`) turns the whole screen over in one frame:
		// report the backlog cap so the ease ramps to catch up, still moving the
		// content one way (down as new lines arrive) - never a jump back up.
		let last = [10u64, 20, 30, 40, 50, 60];
		let cur = [70u64, 80, 90, 100, 110, 120]; // no overlap
		assert_eq!(
			scroll_shift(&cur, &last),
			crate::scroll::MAX_BACKLOG as usize
		);
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
