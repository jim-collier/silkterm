// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

//! Modal settings dialog: sliders for numeric tunables, swatch + hex field for
//! colors, toggles, few-option radios, dropdown list boxes for longer enums, and
//! Cancel / Apply / OK. Edits a working copy of `Settings`; the app reads it back
//! on Apply/OK to live-apply + persist. Renders as flat quads (rects) + positioned
//! text; an open dropdown's popup draws in a second (`LoadOp::Load`) pass on top so
//! covered rows' text can't bleed through it (see `dropdown_overlay`).
//!
//! Sections are grouped into tabs (see `tab_titles()`/`tab_for_section`) so the
//! dialog stays well under screen height; if a tab still doesn't fit (huge UI
//! font / short screen) the rows region scrolls (wheel + draggable thumb) and
//! the window height is capped instead of clipping the buttons.
//!
//! Units: every measurement below is a DIP - a CSS pixel, i.e. 1/96 inch - and
//! the whole layout is solved in that space. The window's scale factor is
//! applied only at the boundary: pointer positions, measured text widths, the
//! UI line height and the height cap divide down on the way in; the window
//! size, the scissor viewport, quads and text positions multiply back out on
//! the way to the renderer. So the dialog keeps its proportions at any DPI
//! rather than shrinking as the scale factor grows, and there is exactly one
//! set of numbers to reason about. At scale 1 nothing changes.

use crate::config::{self, Settings};
use crate::gfx::RectInstance;
use crate::pane::Rect;
use crate::ui_spec::{self, Key, Kind, Layout, Spec, ui};

// The declared geometry, all of it in DIP (see the units note above).
fn lay() -> &'static Layout {
	&ui().layout
}
pub fn tab_titles() -> &'static [&'static str] {
	&ui().tabs
}

// Dialog colors adapt to the active mode (dark-gray for dark, light-gray for
// light); see config::is_dark(). The menu/main-window chrome stays a fixed gray.
struct Dlg {
	panel_bg: [u8; 3],
	panel_border: [u8; 3],
	gutter: [u8; 3], // the strip the tabs stand on
	tab_bg: [u8; 3], // a tab that is not the current one
	tab_hl: [u8; 3], // the current tab: a lighter gray, deliberately not an accent
	track: [u8; 3],
	handle: [u8; 3],
	field_bg: [u8; 3],
	focus_out: [u8; 3],
	btn_bg: [u8; 3],
	btn_hl: [u8; 3],
	text: [u8; 3],
	dim: [u8; 3],
	// The dialog's one destructive control (the shells grid's remove). Chrome,
	// not theme-derived: "this deletes something" is a fixed meaning, and a
	// theme whose accent happened to be red would say it about everything.
	danger: [u8; 3],
}
#[rustfmt::skip]
const DARK_DLG: Dlg = Dlg {
	panel_bg: [0x20, 0x20, 0x2a], panel_border: [0x50, 0x50, 0x60],
	gutter: [0x16, 0x16, 0x1e],
	tab_bg: [0x28, 0x28, 0x32], tab_hl: [0x40, 0x40, 0x4c],
	track: [0x14, 0x14, 0x1c], handle: [0x7a, 0x9a, 0xd0],
	field_bg: [0x14, 0x14, 0x1c], focus_out: [0x7a, 0x9a, 0xd0],
	btn_bg: [0x34, 0x34, 0x40], btn_hl: [0x4a, 0x6a, 0x9a],
	text: [0xe2, 0xe2, 0xea], dim: [0x9a, 0x9a, 0xa6],
	danger: [0xe2, 0x6a, 0x6a],
};
#[rustfmt::skip]
const LIGHT_DLG: Dlg = Dlg {
	panel_bg: [0xe6, 0xe6, 0xe3], panel_border: [0xb2, 0xb2, 0xb6],
	gutter: [0xd3, 0xd3, 0xcf],
	tab_bg: [0xdd, 0xdd, 0xd9], tab_hl: [0xf4, 0xf4, 0xf1],
	track: [0xcf, 0xcf, 0xcc], handle: [0x4a, 0x6a, 0xa8],
	field_bg: [0xf8, 0xf8, 0xf6], focus_out: [0x3a, 0x6a, 0xc0],
	btn_bg: [0xd6, 0xd6, 0xd2], btn_hl: [0x9a, 0xb6, 0xe0],
	text: [0x22, 0x24, 0x2c], dim: [0x70, 0x70, 0x76],
	danger: [0xb8, 0x2c, 0x2c],
};
// The dialog color set for the active mode, with the panel background + text
// overridden by the configured dialog colors (theme default or a colors
// dialog_*/menu_* override). The remaining shades (border/track/handle/fields/
// buttons) stay from the mode preset so contrast holds.
// sRGB-space blend of two colors (selection highlight = field bg toward accent)
fn mix3(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
	let mut out = [0u8; 3];
	for k in 0..3 {
		out[k] = (a[k] as f32 + (b[k] as f32 - a[k] as f32) * t).round() as u8;
	}
	out
}
fn dlg() -> Dlg {
	let base = if config::is_dark() {
		DARK_DLG
	} else {
		LIGHT_DLG
	};
	let settings = config::settings();
	// The two attention colors are themable and mean different things (see
	// theme.rs). `highlight` paints everything that calls attention at once -
	// slider handles, the scrollbar, revert arrows, the default button - and the
	// button fill is a dimmed version of it, mixed toward the panel so a pressed
	// button reads as pressed rather than as the focused one. `focus` paints only
	// the ring around whatever the keyboard is on.
	Dlg {
		panel_bg: settings.dialog_bg,
		text: settings.dialog_fg,
		gutter: settings.gutter,
		handle: settings.highlight,
		btn_hl: mix3(settings.dialog_bg, settings.highlight, 0.62),
		focus_out: settings.focus,
		..base
	}
}

// Mode-adaptive dialog colors for the pop-out window (clear + About text).
pub fn dialog_bg() -> [u8; 3] {
	dlg().panel_bg
}
pub fn dialog_text() -> [u8; 3] {
	dlg().text
}
pub fn dialog_dim() -> [u8; 3] {
	dlg().dim
}
pub fn dialog_btn() -> [u8; 3] {
	dlg().btn_bg
}
pub fn dialog_btn_hl() -> [u8; 3] {
	dlg().btn_hl
}
pub fn dialog_border() -> [u8; 3] {
	dlg().panel_border
}

// The Scrolling tab's time constants are all shown as a friendly 1..100 but
// stored as milliseconds. Logarithmic: a time constant is felt by ratio, and a
// linear map wastes most of the travel on values that look identical (the old
// 300ms floor was why the slow end read as a no-op - barely below the 230ms
// default). Each range spans two decades around the value its segment was
// first tuned to, so a default landing anywhere in it still leaves room to
// move either way.
fn log_pos(v: f32, min: f32, max: f32) -> f32 {
	(v.clamp(min, max) / min).ln() / (max / min).ln()
}
fn log_val(pos: f32, min: f32, max: f32) -> f32 {
	min * (max / min).powf(pos.clamp(0.0, 1.0))
}
// Every feel slider falls: a HIGHER number means a SMALLER stored value,
// because each is stored as a time (per-line tau, doubling/halving period,
// ease duration) and a shorter time is a faster/harder/crisper feel. One
// direction across the whole tab - higher = faster.
fn falling_slider(v: f32, min: f32, max: f32) -> f32 {
	(100.0 - log_pos(v, min, max) * 99.0).round()
}
fn falling_value(slider: f32, min: f32, max: f32) -> f32 {
	log_val((100.0 - slider.clamp(1.0, 100.0)) / 99.0, min, max)
}
// 1 = one line/s, 100 = a hundred lines/s.
const TAU_MIN: f32 = 10.0;
const TAU_MAX: f32 = 1000.0;
// A fraction the config stores as 0..1 reads as a whole percent in the dialog:
// nobody thinks in 0.35. Only the display moves - the file keeps the decimal,
// so the two are the same transform in opposite directions and every default
// comparison happens on the same side of it.
fn to_percent(fraction: f32) -> f32 {
	fraction * 100.0
}
fn from_percent(percent: f32) -> f32 {
	percent / 100.0
}

fn tau_to_speed(tau: f32) -> f32 {
	falling_slider(tau, TAU_MIN, TAU_MAX)
}
fn speed_to_tau(speed: f32) -> f32 {
	falling_value(speed, TAU_MIN, TAU_MAX)
}
// Leave-from-rest duration: 8ms is instant, 800ms a long slow lift.
const EASE_IN_MIN: f32 = 8.0;
const EASE_IN_MAX: f32 = 800.0;
// Chase doubling period: 30ms is a near-instant ramp, 3s barely ramps at all.
const RAMP_UP_MIN: f32 = 30.0;
const RAMP_UP_MAX: f32 = 3000.0;
// Wind-down halving period, the same two-decade span as the ramp up.
const RAMP_DOWN_MIN: f32 = 45.0;
const RAMP_DOWN_MAX: f32 = 4500.0;
// Tail duration: 13ms is an abrupt landing, 1.3s a long float-in.
const EASE_OUT_MIN: f32 = 13.0;
const EASE_OUT_MAX: f32 = 1300.0;

// What holds keyboard focus: one control within a row, or a footer button (index
// into `buttons()`: 0 = Cancel, 1 = Apply, 2 = OK). `Row(i, part)` names a row and
// which of its focusable sub-controls (part 0 for a plain control; sliders and the
// combined cursor row expose two parts). Tab walks parts then buttons.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Focus {
	Row(usize, u16),
	Button(usize),
}

// Where the user was looking when the dialog closed, so reopening it shortly
// after lands on the same tab and scroll position instead of the top of
// Appearance. Only the view - edits are discarded on close as before.
#[derive(Clone, Copy)]
pub struct View {
	tab: usize,
	scroll: f32,
}

// In-progress field edit: the row, its text, the caret (a byte index into
// `buf`, always on a char boundary), and an optional selection anchor. The
// selection spans anchor..caret in either direction; None = no selection.
struct EditState {
	row: usize,
	buf: String,
	cur: usize,
	sel: Option<usize>,
	// Horizontal view: px of text hidden left of the box. `view` is the smoothed
	// offset actually drawn, easing toward `view_to` (kept caret-in-view with a
	// lookahead margin by `animate`). Everything that maps px<->byte (clicks,
	// drags, the caret/selection quads, the drawn text x) offsets by `view`.
	view: f32,
	view_to: f32,
	// smoothed caret x in text-space px; None until first measured (then snaps)
	caret_vis: Option<f32>,
	blink_t: f32, // seconds since the last caret/text activity (drives the blink)
	// (cur, sel, buf.len()) at the last animate pass - a change resets the blink
	last_sig: (usize, Option<usize>, usize),
}
impl EditState {
	// A field opened on `buf`, caret at the end. `row` is `usize::MAX` for the
	// prompt box's field, which belongs to no row.
	fn new(row: usize, buf: String) -> EditState {
		let cur = buf.len();
		EditState {
			row,
			buf,
			cur,
			sel: None,
			view: 0.0,
			view_to: 0.0,
			caret_vis: None,
			blink_t: 0.0,
			last_sig: (usize::MAX, None, usize::MAX),
		}
	}
	// Smooth blink: solid just after activity, then a soft cosine pulse (never a
	// hard on/off pop).
	fn caret_alpha(&self) -> f32 {
		const HOLD: f32 = 0.55;
		const PERIOD: f32 = 1.1;
		if self.blink_t <= HOLD {
			return 1.0;
		}
		0.5 + 0.5 * ((self.blink_t - HOLD) / PERIOD * std::f32::consts::TAU).cos()
	}
	// normalized selection byte range, None when empty/absent
	fn sel_range(&self) -> Option<(usize, usize)> {
		let anchor = self.sel?;
		if anchor == self.cur {
			return None;
		}
		Some((anchor.min(self.cur), anchor.max(self.cur)))
	}
	// remove the selected span (caret lands at its start); true if anything went
	fn remove_selection(&mut self) -> bool {
		let Some((a, b)) = self.sel_range() else {
			self.sel = None;
			return false;
		};
		self.buf.replace_range(a..b, "");
		self.cur = a;
		self.sel = None;
		true
	}
}
fn prev_boundary(s: &str, i: usize) -> usize {
	let mut j = i.min(s.len());
	while j > 0 {
		j -= 1;
		if s.is_char_boundary(j) {
			return j;
		}
	}
	0
}
fn next_boundary(s: &str, i: usize) -> usize {
	let mut j = i;
	while j < s.len() {
		j += 1;
		if s.is_char_boundary(j) {
			return j;
		}
	}
	s.len()
}
// Word motion (Ctrl+Left/Right, Ctrl+Backspace/Delete, double-click): a word is
// a run of alphanumerics/underscore; everything else is a separator.
fn is_word_char(c: char) -> bool {
	c.is_alphanumeric() || c == '_'
}
fn word_left(s: &str, i: usize) -> usize {
	let mut j = i.min(s.len());
	// skip separators, then the word itself
	while j > 0 {
		let p = prev_boundary(s, j);
		if s[p..].chars().next().is_some_and(is_word_char) {
			break;
		}
		j = p;
	}
	while j > 0 {
		let p = prev_boundary(s, j);
		if !s[p..].chars().next().is_some_and(is_word_char) {
			break;
		}
		j = p;
	}
	j
}
fn word_right(s: &str, i: usize) -> usize {
	let mut j = i.min(s.len());
	while j < s.len() && !s[j..].chars().next().is_some_and(is_word_char) {
		j = next_boundary(s, j);
	}
	while j < s.len() && s[j..].chars().next().is_some_and(is_word_char) {
		j = next_boundary(s, j);
	}
	j
}
// Byte range of the word (or separator run) under byte index `i` (double-click).
fn word_at(s: &str, i: usize) -> (usize, usize) {
	if s.is_empty() {
		return (0, 0);
	}
	let i = if i >= s.len() {
		prev_boundary(s, s.len())
	} else {
		i
	};
	let wordy = s[i..].chars().next().is_some_and(is_word_char);
	let mut a = i;
	while a > 0 {
		let p = prev_boundary(s, a);
		if s[p..].chars().next().is_some_and(is_word_char) != wordy {
			break;
		}
		a = p;
	}
	let mut b = next_boundary(s, i);
	while b < s.len() && s[b..].chars().next().is_some_and(is_word_char) == wordy {
		b = next_boundary(s, b);
	}
	(a, b)
}
// Byte index of the caret nearest a click at `rel_x` px into the text (0 = the
// field's left text edge). Walks char boundaries, picking the one whose measured
// prefix width is closest to the click.
fn caret_from_click(text: &str, rel_x: f32, measure: &mut impl FnMut(&str) -> f32) -> usize {
	if rel_x <= 0.0 {
		return 0;
	}
	let (mut best_caret, mut best_dist) = (0usize, f32::MAX);
	let mut i = 0;
	loop {
		let dist = (measure(&text[..i]) - rel_x).abs();
		if dist < best_dist {
			best_dist = dist;
			best_caret = i;
		}
		if i >= text.len() {
			return best_caret;
		}
		i = next_boundary(text, i);
	}
}

// One arrow-key increment for a slider: ~1/100 of the range normally, ~1/10 with
// Shift (so ~100 / ~10 steps span it), rounded to a whole unit (>=1) for int fields.
fn slider_step(min: f32, max: f32, int: bool, shift: bool) -> f32 {
	let span = (max - min).abs();
	let raw = if shift { span / 10.0 } else { span / 100.0 };
	if int { raw.round().max(1.0) } else { raw }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
	None,
	Apply,
	Ok,
	Cancel,
	// a field context-menu command; the clipboard glue lives in dialog.rs
	Edit(EditCmd),
}

// Field context-menu commands (right-click / Menu key in an editable field).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum EditCmd {
	Cut,
	Copy,
	Paste,
	Delete,
	SelectAll,
}
const EDIT_MENU: [(&str, EditCmd); 5] = [
	("Cut", EditCmd::Cut),
	("Copy", EditCmd::Copy),
	("Paste", EditCmd::Paste),
	("Delete", EditCmd::Delete),
	("Select all", EditCmd::SelectAll),
];

// The theme row's four buttons, in the order they are declared.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ThemeBtn {
	Save,
	SaveAs,
	Rename,
	Delete,
}
impl ThemeBtn {
	fn of(part: u16) -> ThemeBtn {
		match part {
			0 => ThemeBtn::Save,
			1 => ThemeBtn::SaveAs,
			2 => ThemeBtn::Rename,
			_ => ThemeBtn::Delete,
		}
	}
}

// A small box over the panel: name a new theme, rename one, or confirm a delete
// - of a theme, or of a shell. It is drawn in the overlay pass and takes every
// click and key while it is up, so the panel behind it can be left exactly as it
// was.
// Where the keyboard is inside the box. A confirmation has no field, so `Field`
// is unreachable there and the focus walk starts at Cancel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PromptFocus {
	Field,
	Cancel,
	Ok,
}

// What OK will do. The box itself is the same either way; only this says who
// asked for it and what to carry out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PromptJob {
	Theme(ThemeBtn),
	DropShell(usize), // index into `edited.shells`
}

#[derive(Debug)]
struct Prompt {
	job: PromptJob,
	title: String,
	focus: PromptFocus,
	warn: Option<String>, // why OK is refusing (a blank or taken name)
}

impl Prompt {
	// Derived rather than stored: only a confirmation asks nothing, so a separate
	// flag could only ever disagree with the button that opened the box.
	fn has_field(&self) -> bool {
		matches!(
			self.job,
			PromptJob::Theme(ThemeBtn::SaveAs | ThemeBtn::Rename)
		)
	}
}

// The prompt's text field and the shells grid's own fields are the dialog's ONE
// open edit, so every bit of field behavior - selection, word ops, the
// clipboard, the caret ease, the right-click menu - works in all of them without
// a second copy. None belongs to a spec row, so each borrows an index no row can
// have: every `specs[edit.row]` comparison in this file simply never matches
// one, and the four places that INDEX specs by it each carry their own guard.
const PSEUDO_ROW: usize = usize::MAX / 2;
const PROMPT_ROW: usize = usize::MAX;
// Two per shell entry, counting down: name, then command.
const SHELL_ROW_BASE: usize = usize::MAX - 1;

// Which grid field a pseudo row stands for: (entry index, is-the-command-field).
fn shell_field_of(row: usize) -> Option<(usize, bool)> {
	if row < PSEUDO_ROW || row == PROMPT_ROW {
		return None;
	}
	let k = SHELL_ROW_BASE - row;
	Some((k / 2, k % 2 == 1))
}
fn shell_field_row(entry: usize, command: bool) -> usize {
	SHELL_ROW_BASE - (entry * 2 + usize::from(command))
}

// One shell's own controls, in Tab order - which is also left to right across
// its line. `Add` is the single stop past the last entry, so a grid of `n`
// entries has `n * ShellPart::COUNT + 1` stops.
//
// The grip is deliberately NOT one of them. Reordering is a mouse gesture now,
// so a Tab through the grid walks the values and nothing else; there is no stop
// that draws a control the keyboard cannot work.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ShellPart {
	Name,
	Command,
	Remove,
	Active,
}

impl ShellPart {
	const COUNT: u16 = 4;
	const ALL: [ShellPart; 4] = [
		ShellPart::Name,
		ShellPart::Command,
		ShellPart::Remove,
		ShellPart::Active,
	];
	fn of(k: u16) -> ShellPart {
		ShellPart::ALL[(k % ShellPart::COUNT) as usize]
	}
}

// Left edge of every column on a shells line, plus the width of the one column
// that is not fixed. The fixed columns are placed from BOTH ends and the command
// takes whatever is left between them, so a wider panel widens the one value
// that is routinely too long to read.
struct ShellCols {
	grip: f32,
	name: f32,
	command: f32,
	command_w: f32,
	remove: f32,
	seen: f32,
	active: f32,
}

// A line being dragged by its grip. `at` is where it currently sits, because the
// list is reordered as the pointer moves rather than on release - the line the
// user is dragging is the line they can see moving, which is the whole reason to
// use a grip instead of buttons.
struct ShellDrag {
	at: usize,
	// where inside the line the pointer took hold, so it does not jump on grab
	grab_dy: f32,
}

// Where a part index lands in the grid: an entry's control, or the Add button
// past the end.
enum ShellStop {
	Entry(usize, ShellPart),
	Add,
}

fn shell_stop(part: u16, entries: usize) -> ShellStop {
	let entry = (part / ShellPart::COUNT) as usize;
	if entry >= entries {
		ShellStop::Add
	} else {
		ShellStop::Entry(entry, ShellPart::of(part))
	}
}
fn shell_part_index(entry: usize, part: ShellPart) -> u16 {
	entry as u16 * ShellPart::COUNT
		+ ShellPart::ALL.iter().position(|p| *p == part).unwrap_or(0) as u16
}

// Open field context menu: anchor point, keyboard-highlighted item, and whether
// the clipboard held text when it opened (grays Paste).
struct EMenu {
	x: f32,
	y: f32,
	hover: Option<usize>,
	paste_ok: bool,
}

pub struct TextItem {
	pub text: String,
	pub x: f32,
	pub y: f32,
	pub color: [u8; 3],
	pub clip: Option<Rect>, // when set, clip drawing to this rect (e.g. a field)
	pub bold: bool,
	pub scale: f32, // 1.0 normal; >1 for the prominent dialog title
}

pub struct SettingsDialog {
	orig: Settings,
	edited: Settings,
	defaults: Settings,          // config defaults, for the revert-to-default buttons
	reverted: Vec<&'static str>, // config keys reverted this session -> comment out on Apply
	rect: Rect,
	specs: &'static [Spec],
	tab: usize,                        // active tab
	tab_ws: Vec<f32>,                  // measured tab-button widths (UI font)
	scroll: f32,                       // rows-region scroll offset (0 when everything fits)
	drag_thumb: Option<f32>,           // scrollbar-thumb drag: grab offset within the thumb
	drag: Option<usize>,               // slider row being dragged
	shell_drag: Option<ShellDrag>,     // shells line being dragged by its grip
	pressed: Option<usize>,            // footer button held down (fires on release; drawn pressed)
	pressed_row: Option<(usize, u16)>, // a row's push-button held down (same press/release)
	prompt: Option<Prompt>,            // the name / confirm box a theme action puts over the panel
	edit: Option<EditState>,           // row being typed (hex for Color, path for Text)
	edit_drag: Option<usize>,          // field row being drag-selected with the mouse
	select_all_on_up: bool, // a fresh single-click field entry: select all on release unless it became a drag
	// multi-click detection (double = select word, triple = select all)
	last_click: Option<(std::time::Instant, f32, f32)>,
	click_streak: u8,
	open: Option<usize>,  // row whose dropdown popup is open (None = all closed)
	pending: usize,       // highlighted option in the open popup (commits on Enter/click)
	emenu: Option<EMenu>, // open field context menu (right-click / Menu key)
	mouse: (f32, f32),    // last cursor pos (drag edge-autoscroll replays it)
	focus: Option<Focus>, // keyboard-focused control/button (None = mouse-only)
	alt: bool,            // Alt held: underline button accelerators (Cancel/Apply/OK)
	shift: bool,          // Shift held (Shift+Tab walks focus backwards)
	ctrl: bool,           // Ctrl held (Ctrl+Tab switches tabs)
	// UI-font-driven geometry: rows/title/buttons grow with the desktop font so
	// a large or wide (e.g. bold serif) interface font never truncates. The
	// consts above are the floor (the classic look at small sizes).
	line_h: f32,
	label_w: f32,
	btn_w: f32,
	row_btn_w: f32, // push-buttons that sit on a row (see chrome_widths)
	// DIP -> physical pixel factor for the window this dialog lives in. Every
	// measurement in here is a DIP; this is applied only at the boundary.
	scale: f32,
}

impl SettingsDialog {
	// `shells` is the length of the stored list: the grid is one spec row and n
	// lines on screen, so it is the one kind whose height is not a constant.
	fn row_h_for(kind: &Kind, line_h: f32, shells: usize) -> f32 {
		let line = lay().row_height.max(line_h + lay().row_pad);
		match kind {
			Kind::Header(_) => lay().header_height.max(line_h + lay().header_pad),
			// column titles, a line per shell, then the Add button
			Kind::ShellList => {
				line_h
					+ lay().shell_head_gap
					+ shells as f32 * line
					+ lay().shell_add_gap
					+ lay().button_height.max(line_h + lay().row_pad)
			}
			_ => line,
		}
	}
	fn row_h(&self, kind: &Kind) -> f32 {
		Self::row_h_for(kind, self.line_h, self.edited.shells.len())
	}
	// One line of the grid - the same height as an ordinary settings row, so the
	// fields in it match every other field in the dialog.
	fn shell_line_h(&self) -> f32 {
		lay().row_height.max(self.line_h + lay().row_pad)
	}
	fn btn_h(&self) -> f32 {
		lay().button_height.max(self.line_h + lay().row_pad)
	}

	// A heading that only repeats its own tab's title says nothing the tab strip
	// has not already said, so it takes no space and draws nothing. It stays in
	// the declarations because a heading is also what assigns the rows under it
	// to a tab - deleting it there would orphan them.
	fn header_is_tab_title(spec: &Spec) -> bool {
		matches!(spec.kind, Kind::Header(label) if tab_titles().get(spec.tab) == Some(&label))
	}

	// The rows one tab actually draws, in order.
	fn visible(specs: &[Spec], tab: usize) -> impl Iterator<Item = (usize, &Spec)> {
		specs
			.iter()
			.enumerate()
			.filter(move |(_, spec)| spec.tab == tab && !Self::header_is_tab_title(spec))
	}

	// A row leads a sub-group when the row drawn under it is indented further.
	// Read off the indentation rather than declared a second time, so the two
	// cannot disagree.
	fn leads_subgroup(specs: &[Spec], i: usize, tab: usize) -> bool {
		Self::visible(specs, tab)
			.find(|(j, _)| *j > i)
			.is_some_and(|(_, next)| next.indent > specs[i].indent)
	}

	// Clear space above a drawn row: a group heading is set off from the section
	// before it, a sub-group's leader from the sub-group before it. Neither
	// applies at the top of a section, where the separation is already there.
	fn gap_above(specs: &[Spec], i: usize, tab: usize, prev: Option<&Spec>) -> f32 {
		let Some(prev) = prev else { return 0.0 };
		if matches!(specs[i].kind, Kind::Header(_)) {
			return lay().header_gap;
		}
		if matches!(prev.kind, Kind::Header(_)) || !Self::leads_subgroup(specs, i, tab) {
			return 0.0;
		}
		lay().subgroup_gap
	}

	// Natural height of one tab's rows (gaps included). Static so `new` can size
	// the window before Self exists; row_y must walk rows the same way.
	fn tab_content_h(specs: &[Spec], tab: usize, line_h: f32, shells: usize) -> f32 {
		let mut h = 0.0;
		let mut prev: Option<&Spec> = None;
		for (i, spec) in Self::visible(specs, tab) {
			h += Self::gap_above(specs, i, tab, prev);
			h += Self::row_h_for(&spec.kind, line_h, shells);
			prev = Some(spec);
		}
		h
	}

	// `line_h` is the chrome (UI font) line height; `label_w`/`btn_w`/`tab_ws`
	// are the measured widths in that font (see chrome_widths) so nothing
	// truncates. `max_h` caps the window height (short screens / huge fonts);
	// a tab that doesn't fit scrolls instead of clipping the buttons.
	// `scale` is the window's DIP -> physical factor; every other argument arrives
	// in physical pixels and is converted on the way in (see the module note on
	// the DIP boundary).
	pub fn new(
		screen_w: f32,
		screen_h: f32,
		line_h: f32,
		label_w: f32,
		btn_w: f32,
		row_btn_w: f32,
		tab_ws: Vec<f32>,
		max_h: f32,
		scale: f32,
	) -> Self {
		let scale = if scale.is_finite() && scale > 0.0 {
			scale
		} else {
			1.0
		};
		let (screen_w, screen_h) = (screen_w / scale, screen_h / scale);
		let (line_h, max_h) = (line_h / scale, max_h / scale);
		let (label_w, btn_w, row_btn_w) = (label_w / scale, btn_w / scale, row_btn_w / scale);
		let tab_ws: Vec<f32> = tab_ws.into_iter().map(|w| w / scale).collect();
		let specs: &'static [Spec] = &ui().specs;
		let label_w = label_w.max(lay().label_width);
		let btn_w = btn_w.max(lay().button_width);
		let row_btn_w = row_btn_w.max(lay().button_width);
		let btn_h = lay().button_height.max(line_h + lay().row_pad);
		let shells = config::settings().shells.len();
		let tallest = (0..tab_titles().len())
			.map(|t| Self::tab_content_h(specs, t, line_h, shells))
			.fold(0.0f32, f32::max);
		let h = (Self::gutter_h_for(line_h)
			+ 1.0 + lay().tabs_gap
			+ tallest + lay().buttons_gap
			+ btn_h + lay().pad)
			.min(max_h.max(300.0));
		let tabs_w = lay().pad * 2.0
			+ tab_ws.iter().sum::<f32>()
			+ lay().tab_gap * tab_ws.len().saturating_sub(1) as f32;
		// widest radio row (scaled pitch at HiDPI / large fonts) must fit the panel,
		// or the last option overflows the right edge
		let font_scale = (line_h / lay().base_line_height).max(1.0);
		let max_radio_opts = specs
			.iter()
			.filter_map(|spec| match spec.kind {
				Kind::Radio(opts) => Some(opts.len()),
				_ => None,
			})
			.max()
			.unwrap_or(0) as f32;
		let radio_w =
			lay().pad + label_w + max_radio_opts * lay().radio_pitch * font_scale + lay().pad;
		// a dropdown's collapsed box (+ revert column) must fit too
		let has_dropdown = specs.iter().any(|s| matches!(s.kind, Kind::Dropdown(_)));
		let dd_w = if has_dropdown {
			lay().pad
				+ label_w + lay().dropdown_width * font_scale
				+ 6.0 + lay().revert_width
				+ lay().pad
		} else {
			0.0
		};
		// a row of push-buttons starts at the control column and must fit too
		let max_row_btns = specs
			.iter()
			.filter_map(|spec| match spec.kind {
				Kind::Buttons(captions) => Some(captions.len()),
				_ => None,
			})
			.max()
			.unwrap_or(0) as f32;
		let btns_w = if max_row_btns > 0.0 {
			lay().pad
				+ label_w + max_row_btns * row_btn_w
				+ (max_row_btns - 1.0) * lay().button_gap
				+ lay().pad
		} else {
			0.0
		};
		// the shells grid spans the whole content width, so its columns are a floor
		// on the panel rather than on a column of it
		let grid_w = if specs.iter().any(|s| matches!(s.kind, Kind::ShellList)) {
			lay().pad + Self::shell_columns_w(font_scale) + lay().pad
		} else {
			0.0
		};
		let w = (lay().width + (label_w - lay().label_width) + (btn_w - lay().button_width) * 3.0)
			.max(tabs_w)
			.max(radio_w)
			.max(dd_w)
			.max(btns_w)
			.max(grid_w);
		let rect = Rect {
			x: ((screen_w - w) / 2.0).max(0.0),
			y: ((screen_h - h) / 2.0).max(0.0),
			w,
			h,
		};
		let settings = (*config::settings()).clone();
		Self {
			orig: settings.clone(),
			edited: settings,
			defaults: Settings::default(),
			reverted: Vec::new(),
			rect,
			specs,
			tab: 0,
			tab_ws,
			scroll: 0.0,
			drag_thumb: None,
			drag: None,
			shell_drag: None,
			pressed: None,
			pressed_row: None,
			prompt: None,
			edit: None,
			edit_drag: None,
			select_all_on_up: false,
			last_click: None,
			click_streak: 0,
			open: None,
			pending: 0,
			emenu: None,
			mouse: (0.0, 0.0),
			focus: None,
			alt: false,
			shift: false,
			ctrl: false,
			line_h,
			label_w,
			btn_w,
			row_btn_w,
			scale,
		}
	}

	// DIP <-> physical pixels. Coordinates and sizes cross the boundary in the
	// public methods only: everything below them is DIP.
	fn to_dip(&self, px: f32) -> f32 {
		px / self.scale
	}
	fn to_px(&self, dip: f32) -> f32 {
		dip * self.scale
	}
	fn rect_px(&self, r: Rect) -> Rect {
		Rect {
			x: self.to_px(r.x),
			y: self.to_px(r.y),
			w: self.to_px(r.w),
			h: self.to_px(r.h),
		}
	}
	// Scale a batch of quads out to physical pixels. `params.y` is a stroke width
	// or corner radius, so it is a measurement too and scales with the rest.
	fn quads_px(&self, quads: &mut [RectInstance]) {
		for quad in quads {
			quad.pos = [self.to_px(quad.pos[0]), self.to_px(quad.pos[1])];
			quad.size = [self.to_px(quad.size[0]), self.to_px(quad.size[1])];
			quad.params[1] = self.to_px(quad.params[1]);
		}
	}
	fn texts_px(&self, items: &mut [TextItem]) {
		for item in items {
			item.x = self.to_px(item.x);
			item.y = self.to_px(item.y);
			item.clip = item.clip.map(|r| self.rect_px(r));
		}
	}

	// The pointer/measurement boundary. Pointer positions arrive in physical
	// pixels and the caller's text measurement answers in them too, so both are
	// divided down before any of the layout below sees them.
	pub fn mouse_down(&mut self, x: f32, y: f32, measure: &mut impl FnMut(&str) -> f32) -> Action {
		let s = self.scale;
		self.mouse_down_dip(x / s, y / s, &mut |t| measure(t) / s)
	}
	pub fn mouse_up(&mut self, x: f32, y: f32) -> Action {
		let s = self.scale;
		self.mouse_up_dip(x / s, y / s)
	}
	pub fn mouse_move(&mut self, x: f32, y: f32, measure: &mut impl FnMut(&str) -> f32) {
		let s = self.scale;
		self.mouse_move_dip(x / s, y / s, &mut |t| measure(t) / s);
	}
	pub fn mouse_right(
		&mut self,
		x: f32,
		y: f32,
		paste_ok: bool,
		measure: &mut impl FnMut(&str) -> f32,
	) {
		let s = self.scale;
		self.mouse_right_dip(x / s, y / s, paste_ok, &mut |t| measure(t) / s);
	}
	pub fn menu_key(&mut self, paste_ok: bool, measure: &mut impl FnMut(&str) -> f32) {
		let s = self.scale;
		self.menu_key_dip(paste_ok, &mut |t| measure(t) / s);
	}
	pub fn animate(&mut self, dt: f32, measure: &mut impl FnMut(&str) -> f32) -> Option<u64> {
		let s = self.scale;
		self.animate_dip(dt, &mut |t| measure(t) / s)
	}
	pub fn hover_tip(&self, mx: f32, my: f32) -> Option<(&'static str, Rect)> {
		let s = self.scale;
		self.hover_tip_dip(mx / s, my / s)
			.map(|(tip, anchor)| (tip, self.rect_px(anchor)))
	}

	// The drawing boundary: the layout is solved in DIP, then everything handed
	// to the renderer is multiplied out to physical pixels.
	pub fn rects(
		&self,
		line_h: f32,
		mut measure: impl FnMut(&str) -> f32,
	) -> (Vec<RectInstance>, Vec<RectInstance>) {
		let s = self.scale;
		let (mut fixed, mut rows) = self.rects_dip(line_h / s, |t| measure(t) / s);
		self.quads_px(&mut fixed);
		self.quads_px(&mut rows);
		(fixed, rows)
	}
	pub fn texts(&self, line_h: f32, mut measure: impl FnMut(&str) -> f32) -> Vec<TextItem> {
		let s = self.scale;
		let mut items = self.texts_dip(line_h / s, |t| measure(t) / s);
		self.texts_px(&mut items);
		items
	}
	pub fn overlay(
		&self,
		measure: &mut impl FnMut(&str) -> f32,
	) -> (Vec<RectInstance>, Vec<TextItem>) {
		let s = self.scale;
		let (mut quads, mut items) = self.overlay_dip(&mut |t| measure(t) / s);
		self.quads_px(&mut quads);
		self.texts_px(&mut items);
		(quads, items)
	}

	// Tab-strip / rows-viewport / scrollbar geometry. The rows region sits between
	// the strip and the buttons; only it scrolls (chrome stays put).
	//
	// The tabs stand on a gutter strip that runs the panel's full width, and the
	// line closing that strip is what they stand ON - so a tab is shorter than a
	// footer button (it is chrome, not a control) and the strip's height is
	// simply the drop from the panel edge plus that tab.
	fn tab_h(&self) -> f32 {
		Self::tab_h_for(self.line_h)
	}
	fn tab_h_for(line_h: f32) -> f32 {
		lay().tab_height.max(line_h + lay().tab_pad_v)
	}
	fn gutter_h_for(line_h: f32) -> f32 {
		lay().tab_top + Self::tab_h_for(line_h)
	}
	fn tab_bar_y(&self) -> f32 {
		self.rect.y + lay().tab_top
	}
	// The strip, and the 1px rule closing it off from the rows below.
	fn gutter_rect(&self) -> Rect {
		Rect {
			x: self.rect.x,
			y: self.rect.y,
			w: self.rect.w,
			h: Self::gutter_h_for(self.line_h),
		}
	}
	fn tab_rect(&self, k: usize) -> Rect {
		let x = self.rect.x
			+ lay().pad
			+ self.tab_ws[..k].iter().sum::<f32>()
			+ lay().tab_gap * k as f32;
		Rect {
			x,
			y: self.tab_bar_y(),
			w: self.tab_ws[k],
			h: self.tab_h(),
		}
	}
	fn rows_y0(&self) -> f32 {
		let g = self.gutter_rect();
		g.y + g.h + 1.0 + lay().tabs_gap
	}
	// The scroll viewport in physical pixels (the render pass scissors to it).
	pub fn viewport_px(&self) -> Rect {
		self.rect_px(self.viewport())
	}
	fn viewport(&self) -> Rect {
		let y0 = self.rows_y0();
		Rect {
			x: self.rect.x,
			y: y0,
			w: self.rect.w,
			h: (self.rect.y + self.rect.h - lay().pad - self.btn_h() - lay().buttons_gap - y0)
				.max(0.0),
		}
	}
	fn content_h(&self) -> f32 {
		Self::tab_content_h(self.specs, self.tab, self.line_h, self.edited.shells.len())
	}
	fn max_scroll(&self) -> f32 {
		(self.content_h() - self.viewport().h).max(0.0)
	}
	pub fn wheel(&mut self, dy_px: f32) {
		if self.prompt.is_some() {
			return; // nothing behind the prompt box may move under it
		}
		self.dismiss_menu();
		let dy = self.to_dip(dy_px);
		self.scroll = (self.scroll - dy).clamp(0.0, self.max_scroll());
	}
	pub fn view(&self) -> View {
		View {
			tab: self.tab,
			scroll: self.scroll,
		}
	}
	// A restored view comes from a dialog that no longer exists, so nothing about
	// its geometry can be assumed: the UI font, screen height or field set may all
	// have changed since. Clamp rather than trust.
	pub fn restore(&mut self, view: View) {
		if view.tab >= tab_titles().len() {
			return;
		}
		self.tab = view.tab;
		self.scroll = view.scroll.clamp(0.0, self.max_scroll());
	}
	fn thumb(&self) -> Option<Rect> {
		let scroll_max = self.max_scroll();
		if scroll_max <= 0.0 {
			return None;
		}
		let vp = self.viewport();
		let thumb_h = (vp.h * vp.h / self.content_h()).max(lay().scrollbar_thumb_min);
		Some(Rect {
			x: self.rect.x + self.rect.w - lay().scrollbar_inset - lay().scrollbar_width,
			y: vp.y + (self.scroll / scroll_max) * (vp.h - thumb_h),
			w: lay().scrollbar_width,
			h: thumb_h,
		})
	}

	// Alt-key accelerators: while Alt is held the buttons underline their first
	// letter (Cancel/Apply/OK), and Alt+that-letter triggers the button. Shift
	// (Shift+Tab) and Ctrl (Ctrl+Tab) steer keyboard focus / tab switching.
	pub fn set_mods(&mut self, alt: bool, shift: bool, ctrl: bool) {
		self.alt = alt;
		self.shift = shift;
		self.ctrl = ctrl;
	}
	pub fn alt(&self) -> bool {
		self.alt
	}
	pub fn ctrl(&self) -> bool {
		self.ctrl
	}
	pub fn shift(&self) -> bool {
		self.shift
	}
	// Takes &self so the prompt can swallow it: the footer accelerators would
	// otherwise apply and close the dialog out from under an open theme box.
	pub fn alt_key(&self, c: char) -> Action {
		if self.prompt.is_some() {
			return Action::None;
		}
		match c.to_ascii_lowercase() {
			'c' => Action::Cancel,
			'a' => Action::Apply,
			'o' => Action::Ok,
			_ => Action::None,
		}
	}

	// ---- dropdown popup (open list; commits on Enter / click) -----------------

	// A dropdown's option list. Every one but the theme picker is fixed in the
	// declarations; that one is whatever themes exist right now, so the list has
	// to be built per call rather than borrowed from the document.
	fn dd_options(&self, i: usize) -> Vec<String> {
		match self.specs[i].kind {
			_ if self.specs[i].key == Key::Theme => {
				crate::theme::all_names(&self.edited.user_themes)
			}
			Kind::Dropdown(opts) => opts.iter().map(|o| (*o).to_string()).collect(),
			_ => Vec::new(),
		}
	}
	// Open row `i`'s popup with the current value highlighted.
	fn dd_open(&mut self, i: usize) {
		self.commit_edit();
		self.open = Some(i);
		self.pending = self.get_radio(self.specs[i].key);
		self.focus = Some(Focus::Row(i, 0));
		self.scroll_focus_into_view();
	}
	// Apply the highlighted option and close (Enter / Space / click on an option).
	fn dd_commit(&mut self) {
		if let Some(i) = self.open.take() {
			self.set_radio(self.specs[i].key, self.pending);
		}
	}

	// ---- keyboard focus + control activation ----------------------------------

	// Rows on the active tab with at least one focusable (enabled, non-header)
	// sub-control, in visual order. (Used by the focus tests.)
	#[cfg(test)]
	fn focusables(&self) -> Vec<usize> {
		(0..self.specs.len())
			.filter(|&i| {
				self.specs[i].tab == self.tab
					&& (0..self.parts_of(i)).any(|p| !self.part_disabled(i, p))
			})
			.collect()
	}
	fn first_focus(&self) -> Option<Focus> {
		self.focus_ring().first().copied()
	}
	// The full Tab order for the active tab: each enabled sub-control (a slider's
	// track then its field, a Dual row's two checkboxes, else the single control),
	// then the three footer buttons (Cancel / Apply / OK), always reachable.
	fn focus_ring(&self) -> Vec<Focus> {
		let mut ring = Vec::new();
		for i in 0..self.specs.len() {
			if self.specs[i].tab != self.tab || Self::header_is_tab_title(&self.specs[i]) {
				continue;
			}
			for p in 0..self.parts_of(i) {
				if !self.part_disabled(i, p) {
					ring.push(Focus::Row(i, p));
				}
			}
		}
		ring.extend((0..3).map(Focus::Button));
		ring
	}
	// Sort key matching the ring above: rows in spec order, footer buttons last.
	fn focus_order(f: Focus) -> (usize, u16) {
		match f {
			Focus::Row(i, p) => (i, p),
			Focus::Button(b) => (usize::MAX, b as u16),
		}
	}
	// Tab / Shift+Tab (and Down / Up off a non-slider row): move focus to the
	// next/prev item in the ring, wrapping, and scroll a focused row into view.
	fn focus_move(&mut self, forward: bool) {
		self.commit_edit();
		self.open = None; // Tab/arrow away closes any open popup
		let ring = self.focus_ring();
		if ring.is_empty() {
			self.focus = None;
			return;
		}
		let cur = self.focus.and_then(|f| ring.iter().position(|&r| r == f));
		let n = ring.len();
		let next = match cur {
			Some(p) if forward => (p + 1) % n,
			Some(p) => (p + n - 1) % n,
			// Nothing focused, or the focused control grayed out from under us
			// (pressing Save turns Save off). Resume from where it sat rather
			// than snapping back to the top of the tab.
			None => match self.focus.map(Self::focus_order) {
				Some(k) if forward => ring
					.iter()
					.position(|&r| Self::focus_order(r) > k)
					.unwrap_or(0),
				Some(k) => ring
					.iter()
					.rposition(|&r| Self::focus_order(r) < k)
					.unwrap_or(n - 1),
				None if forward => 0,
				None => n - 1,
			},
		};
		self.focus = Some(ring[next]);
		self.scroll_focus_into_view();
	}
	// Scroll the rows region so a focused control row is fully visible (buttons
	// are fixed chrome - always visible).
	fn scroll_focus_into_view(&mut self) {
		let Some(Focus::Row(i, part)) = self.focus else {
			return;
		};
		let vp = self.viewport();
		// The shells grid is one row and many lines, and a long one is taller
		// than the viewport - so it is the focused CONTROL that has to come into
		// view there, not the row, which may not fit at all.
		let (top, bottom) = if matches!(self.specs[i].kind, Kind::ShellList) {
			let r = self.shell_stop_rect(i, part);
			(r.y - 4.0, r.y + r.h + 4.0)
		} else {
			let top = self.row_y(i);
			(top, top + self.row_h(&self.specs[i].kind))
		};
		if top < vp.y {
			self.scroll -= vp.y - top; // row above viewport -> scroll it down into view
		} else if bottom > vp.y + vp.h {
			self.scroll += bottom - (vp.y + vp.h); // row below -> scroll up
		}
		self.scroll = self.scroll.clamp(0.0, self.max_scroll());
	}
	// Ctrl+Tab / Ctrl+Shift+Tab: cycle the active tab, focusing its first control.
	fn tab_switch(&mut self, forward: bool) {
		self.commit_edit();
		self.open = None;
		let n = self.tab_ws.len();
		if n == 0 {
			return;
		}
		self.tab = if forward {
			(self.tab + 1) % n
		} else {
			(self.tab + n - 1) % n
		};
		self.scroll = 0.0;
		self.drag = None;
		self.focus = self.first_focus();
	}
	// The Tab key: Ctrl switches tabs, otherwise walk control focus (Shift = back).
	pub fn key_tab(&mut self) {
		self.dismiss_menu();
		if self.prompt.is_some() {
			self.prompt_focus_move(!self.shift);
			return;
		}
		if self.ctrl {
			self.tab_switch(!self.shift);
		} else {
			self.focus_move(!self.shift);
		}
	}
	// Ctrl+PageUp / Ctrl+PageDown cycle the active tab (PageDown = next).
	pub fn key_page(&mut self, forward: bool) {
		if self.ctrl && self.prompt.is_none() {
			self.tab_switch(forward);
		}
	}
	// Up / Down arrows: navigate an open popup, else Alt+Down opens a focused
	// dropdown, else step a focused numeric slider (spinbox feel), else walk control
	// focus (a peer of Tab).
	pub fn key_vertical(&mut self, forward: bool) {
		if self.prompt.is_some() && self.emenu.is_none() {
			self.prompt_focus_move(forward);
			return;
		}
		if self.emenu.is_some() {
			// walk the field context-menu items (wraps)
			let n = EDIT_MENU.len() as i32;
			if let Some(menu) = &mut self.emenu {
				let step = if forward { 1 } else { -1 };
				let cur = menu
					.hover
					.map_or(if forward { -1 } else { 0 }, |h| h as i32);
				menu.hover = Some((cur + step).rem_euclid(n) as usize);
			}
			return;
		}
		if let Some(i) = self.open {
			let n = self.dd_options(i).len();
			if n > 0 {
				let step = if forward { 1 } else { -1 };
				self.pending = (self.pending as i32 + step).rem_euclid(n as i32) as usize;
			}
			return;
		}
		if forward && self.alt {
			if let Some(Focus::Row(i, _)) = self.focus {
				if matches!(self.specs[i].kind, Kind::Dropdown(_))
					&& !self.disabled(self.specs[i].key)
				{
					self.dd_open(i);
					return;
				}
			}
		}
		// Up/Down step a focused numeric field (spinbox feel; Shift = 10x). Tab still
		// walks between controls. Works whether the field is just focused or open.
		// forward = Down (decrease); !forward = Up (increase).
		if let Some(Focus::Row(i, _)) = self.focus {
			if matches!(self.specs[i].kind, Kind::Slider { .. })
				&& !self.disabled(self.specs[i].key)
			{
				self.step_slider(i, if forward { -1 } else { 1 }, self.shift);
				return;
			}
		}
		self.focus_move(forward);
	}
	// Adjust a focused/open slider by one arrow step (dir = +1/-1, Shift = 10x). When
	// the field is open for editing, its buffer is refreshed to the new value and
	// fully selected, so continued stepping and a following commit see the number.
	fn step_slider(&mut self, i: usize, dir: i32, shift: bool) {
		let Kind::Slider { min, max, int } = self.specs[i].kind else {
			return;
		};
		let key = self.specs[i].key;
		if self.disabled(key) {
			return;
		}
		let step = slider_step(min, max, int, shift);
		let mut value = (self.get_f32(key) + dir as f32 * step).clamp(min, max);
		if int {
			value = value.round();
		}
		self.set_f32(key, value);
		if self.edit.as_ref().is_some_and(|e| e.row == i) {
			let buf = self.fmt_val(key, int);
			if let Some(edit) = &mut self.edit {
				edit.cur = buf.len();
				edit.sel = (!buf.is_empty()).then_some(0);
				edit.buf = buf;
				edit.view_to = 0.0;
			}
		}
	}
	// Left / Right: caret motion while a field is being edited, otherwise adjust
	// the focused slider (by one step) or move a focused radio's selection.
	pub fn key_horizontal(&mut self, dir: i32) {
		self.dismiss_menu();
		// in the prompt box, the field owns Left/Right while it has focus and the
		// two buttons share them otherwise
		if let Some(prompt) = self.prompt.as_ref() {
			if prompt.focus != PromptFocus::Field {
				self.prompt_focus_move(dir > 0);
				return;
			}
		}
		if self.edit.is_some() {
			if dir < 0 {
				self.cursor_left();
			} else {
				self.cursor_right();
			}
			return;
		}
		if self.open.is_some() {
			return; // an open popup owns arrow keys (Up/Down navigate it)
		}
		let Some(Focus::Row(i, _)) = self.focus else {
			return;
		};
		let key = self.specs[i].key;
		if self.disabled(key) {
			return;
		}
		match self.specs[i].kind {
			Kind::Slider { .. } => self.step_slider(i, dir, self.shift),
			// closed dropdown: Left/Right nudge the value without opening (combobox feel)
			Kind::Radio(options) => {
				let sel = self.get_radio(key) as i32;
				let new_sel = (sel + dir).clamp(0, options.len() as i32 - 1);
				self.set_radio(key, new_sel as usize);
			}
			Kind::Dropdown(_) => {
				let n = self.dd_options(i).len() as i32;
				if n > 0 {
					let sel = self.get_radio(key) as i32;
					self.set_radio(key, (sel + dir).clamp(0, n - 1) as usize);
				}
			}
			_ => {}
		}
	}
	// Space: type into an active edit, activate a focused button, else activate the
	// focused control - flip a toggle or open a text/color field for editing.
	pub fn key_space(&mut self) -> Action {
		if let Some(prompt) = self.prompt.as_ref() {
			match prompt.focus {
				PromptFocus::Cancel => self.prompt_close(),
				PromptFocus::Ok => self.prompt_accept(),
				PromptFocus::Field => self.char_input(' '),
			}
			return Action::None;
		}
		if self.open.is_some() {
			self.dd_commit(); // Space picks the highlighted option
			return Action::None;
		}
		if self.edit.is_some() {
			self.char_input(' ');
			return Action::None;
		}
		let (i, part) = match self.focus {
			Some(Focus::Button(b)) => return self.buttons()[b].0,
			Some(Focus::Row(i, p)) => (i, p),
			None => return Action::None,
		};
		let key = self.part_key(i, part);
		if self.disabled(key) {
			return Action::None;
		}
		match self.specs[i].kind {
			// flip the focused checkbox (for Dual, key is that part's key)
			Kind::Toggle | Kind::Dual { .. } => self.set_toggle(key, !self.get_toggle(key)),
			// open the field pre-filled with the current value, fully selected
			// (standard field-entry: typing replaces, arrows keep it)
			Kind::Text | Kind::Color | Kind::Slider { .. } => self.open_edit(i, true),
			Kind::Dropdown(_) => self.dd_open(i),
			Kind::Buttons(_) => self.theme_action(ThemeBtn::of(part)),
			Kind::ShellList => self.shell_activate(i, part),
			_ => {}
		}
		Action::None
	}

	// Space or Enter on a grid stop: open a field, flip the switch, move the
	// entry, or ask before dropping it.
	fn shell_activate(&mut self, i: usize, part: u16) {
		match shell_stop(part, self.edited.shells.len()) {
			ShellStop::Add => self.shell_add(i),
			ShellStop::Entry(k, ShellPart::Name) => self.open_edit(shell_field_row(k, false), true),
			ShellStop::Entry(k, ShellPart::Command) => {
				self.open_edit(shell_field_row(k, true), true);
			}
			ShellStop::Entry(k, ShellPart::Active) => {
				if let Some(entry) = self.edited.shells.get_mut(k) {
					entry.active = !entry.active;
				}
			}
			ShellStop::Entry(k, ShellPart::Remove) => self.shell_confirm_remove(k),
		}
	}

	// Dropping an entry is the one grid action that cannot be undone by doing the
	// opposite, so it asks - the same box the theme delete uses.
	fn shell_confirm_remove(&mut self, k: usize) {
		let Some(entry) = self.edited.shells.get(k) else {
			return;
		};
		let name = if entry.title.trim().is_empty() {
			entry.command.clone()
		} else {
			entry.title.clone()
		};
		self.commit_edit();
		self.prompt = Some(Prompt {
			job: PromptJob::DropShell(k),
			title: format!("Really remove \"{name}\" from the list?"),
			focus: PromptFocus::Ok,
			warn: None,
		});
	}

	// Current value of row i's editable field, as text.
	fn edit_buf(&self, i: usize) -> String {
		if i == PROMPT_ROW {
			return self.edit.as_ref().map_or(String::new(), |e| e.buf.clone());
		}
		if let Some((k, command)) = shell_field_of(i) {
			return self.edited.shells.get(k).map_or_else(String::new, |entry| {
				if command {
					entry.command.clone()
				} else {
					entry.title.clone()
				}
			});
		}
		match self.specs[i].kind {
			Kind::Text => self.get_text(self.specs[i].key),
			Kind::Color => {
				let c = self.get_col(self.specs[i].key);
				format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
			}
			Kind::Slider { int, .. } => self.fmt_val(self.specs[i].key, int),
			_ => String::new(),
		}
	}
	// Open row i's field for editing; select_all puts the whole value under the
	// selection so the next keystroke replaces it.
	fn open_edit(&mut self, i: usize, select_all: bool) {
		let mut edit = EditState::new(i, self.edit_buf(i));
		edit.sel = (select_all && edit.cur > 0).then_some(0);
		self.edit = Some(edit);
	}

	// Panel size (used to size a dedicated dialog window when the panel is laid
	// out at the origin - `new(0.0, 0.0)`).
	// Window size in physical pixels.
	pub fn size(&self) -> (f32, f32) {
		(self.to_px(self.rect.w), self.to_px(self.rect.h))
	}

	pub fn edited(&self) -> &Settings {
		&self.edited
	}
	pub fn orig(&self) -> &Settings {
		&self.orig
	}
	// After an Apply, make the applied values the new baseline so a later Apply
	// compares against the live state, not the stale open-time snapshot (otherwise
	// re-selecting the original value reads as "no change" and isn't applied).
	// A scan landed while this dialog was open. Both copies move, so a user who
	// has changed nothing still has nothing changed - the same reasoning that
	// keeps the list out of an ordinary Apply diff. Anything they have already
	// done to the list (a rename, a reorder, a removal) is what the merge folds
	// INTO, so none of it is undone; a scan only ever appends and switches off.
	pub fn fold_shells(&mut self, found: &[crate::shells::Found]) {
		self.orig.shells = crate::shells::merge(&self.orig.shells, found);
		self.edited.shells = crate::shells::merge(&self.edited.shells, found);
	}

	pub fn commit_baseline(&mut self) {
		self.orig = self.edited.clone();
	}
	pub fn use_system_font(&self) -> bool {
		self.edited.use_system_font
	}

	// Top of row `i` on the active tab (scrolled). Walks visible rows the same
	// way tab_content_h does so heights and header gaps stay in sync.
	fn row_y(&self, i: usize) -> f32 {
		let mut y = self.rows_y0() - self.scroll;
		let mut prev: Option<&Spec> = None;
		for (j, spec) in Self::visible(self.specs, self.tab) {
			y += Self::gap_above(self.specs, j, self.tab, prev);
			if j == i {
				return y;
			}
			y += self.row_h(&spec.kind);
			prev = Some(spec);
		}
		y
	}
	// ---- the shells grid ------------------------------------------------------
	//
	// The spec index of the grid, for the pseudo-row fields, which know which
	// entry they belong to but not which row draws it.
	fn shell_row(&self) -> Option<usize> {
		(0..self.specs.len()).find(|&i| matches!(self.specs[i].kind, Kind::ShellList))
	}
	//
	// The grid spans the whole content width rather than starting at the control
	// column: there is no label beside it, and the command it holds is the one
	// value in the dialog that is routinely too long to read. Columns are laid
	// out from BOTH ends - the fixed ones from the right, the name from the left
	// - and the command takes whatever is left between them, so a wider panel
	// widens the column that needs it.

	// Total width of everything except the command's own slack: what the panel
	// must clear for the grid to be readable at all. Static, so `new` can size
	// the window before Self exists.
	fn shell_columns_w(font_scale: f32) -> f32 {
		let l = lay();
		l.shell_name_width
			+ l.shell_command_width
			+ l.shell_seen_width
			+ l.shell_active_width
			+ l.shell_col_gap * 5.0
			+ (l.shell_grip + l.shell_button) * font_scale
	}
	// The two icon columns follow the UI font the way the checkboxes do, so a
	// bigger desktop font gets a bigger grab handle rather than a fiddlier one.
	fn shell_grip_w(&self) -> f32 {
		lay().shell_grip * self.ui_scale()
	}
	fn shell_button_w(&self) -> f32 {
		lay().shell_button * self.ui_scale()
	}
	// x of each column's left edge, plus the command column's width. Remove sits
	// between the command and the read-only date rather than at the end of the
	// line: it is the one control here that doing the opposite cannot undo, so
	// it is deliberately kept off the right-hand edge the pointer travels down.
	fn shell_cols(&self) -> ShellCols {
		let l = lay();
		let left = self.rect.x + l.pad;
		let right = self.rect.x + self.rect.w - l.pad;
		let active = right - l.shell_active_width;
		let seen = active - l.shell_col_gap - l.shell_seen_width;
		let remove = seen - l.shell_col_gap - self.shell_button_w();
		let name = left + self.shell_grip_w() + l.shell_col_gap;
		let command = name + l.shell_name_width + l.shell_col_gap;
		let command_w = (remove - l.shell_col_gap - command).max(l.shell_command_width / 2.0);
		ShellCols {
			grip: left,
			name,
			command,
			command_w,
			remove,
			seen,
			active,
		}
	}
	// Top of the grid's column titles, and of entry `k`'s own line.
	fn shell_head_y(&self, i: usize) -> f32 {
		self.row_y(i)
	}
	fn shell_line_y(&self, i: usize, k: usize) -> f32 {
		self.shell_head_y(i) + self.line_h + lay().shell_head_gap + k as f32 * self.shell_line_h()
	}
	// A boxed control centered in entry `k`'s line, at `x` and `w` wide.
	fn shell_box(&self, i: usize, k: usize, x: f32, w: f32) -> Rect {
		let line = self.shell_line_h();
		let h = lay().swatch.max(self.line_h + 4.0);
		Rect {
			x,
			y: self.shell_line_y(i, k) + (line - h) / 2.0,
			w,
			h,
		}
	}
	fn shell_name_box(&self, i: usize, k: usize) -> Rect {
		self.shell_box(i, k, self.shell_cols().name, lay().shell_name_width)
	}
	fn shell_cmd_box(&self, i: usize, k: usize) -> Rect {
		let cols = self.shell_cols();
		self.shell_box(i, k, cols.command, cols.command_w)
	}
	// The Active checkbox, centered under its own column title.
	fn shell_active_box(&self, i: usize, k: usize) -> Rect {
		let size = lay().swatch;
		let x = self.shell_cols().active + (lay().shell_active_width - size) / 2.0;
		self.shell_box(i, k, x, size)
	}
	// The drag handle. As tall as the fields beside it rather than square,
	// because it is grabbed rather than aimed at - the taller box is the whole
	// difference between a reorder that feels direct and one that keeps missing.
	fn shell_grip_box(&self, i: usize, k: usize) -> Rect {
		self.shell_box(i, k, self.shell_cols().grip, self.shell_grip_w())
	}
	fn shell_remove_box(&self, i: usize, k: usize) -> Rect {
		let size = self.shell_button_w();
		let line = self.shell_line_h();
		Rect {
			x: self.shell_cols().remove,
			y: self.shell_line_y(i, k) + (line - size) / 2.0,
			w: size,
			h: size,
		}
	}
	fn shell_add_box(&self, i: usize) -> Rect {
		let n = self.edited.shells.len();
		Rect {
			x: self.shell_cols().grip,
			y: self.shell_line_y(i, n) + lay().shell_add_gap,
			w: self.row_btn_w,
			h: self.btn_h(),
		}
	}
	// The rect the keyboard ring goes around, for any stop in the grid.
	fn shell_stop_rect(&self, i: usize, part: u16) -> Rect {
		match shell_stop(part, self.edited.shells.len()) {
			ShellStop::Add => self.shell_add_box(i),
			ShellStop::Entry(k, ShellPart::Name) => self.shell_name_box(i, k),
			ShellStop::Entry(k, ShellPart::Command) => self.shell_cmd_box(i, k),
			ShellStop::Entry(k, ShellPart::Active) => self.shell_active_box(i, k),
			ShellStop::Entry(k, ShellPart::Remove) => self.shell_remove_box(i, k),
		}
	}
	// Move one entry to another place in the list - the whole point of the grip.
	// The list IS the order the Tabs menu offers, and its first switched-on line
	// is the default shell, so a reorder is a real edit and not a view setting.
	fn shell_move_to(&mut self, from: usize, to: usize) {
		let last = self.edited.shells.len().saturating_sub(1);
		let to = to.min(last);
		if from > last || from == to {
			return;
		}
		let entry = self.edited.shells.remove(from);
		self.edited.shells.insert(to, entry);
	}
	// Where a pointer at `y` wants the dragged line to sit. Measured from the
	// line's own TOP - the pointer keeps whatever offset inside the grip it took
	// hold at - and rounded, so an entry changes place once it has travelled half
	// a line rather than a whole one.
	//
	// Both ends are clamped and neither needs a branch: a float-to-integer `as`
	// saturates in Rust, so a line dragged off the top comes back 0 rather than
	// wrapping, and `min` catches the other end.
	fn shell_drop_at(&self, i: usize, y: f32, grab_dy: f32) -> usize {
		let last = self.edited.shells.len().saturating_sub(1);
		let line = self.shell_line_h().max(1.0);
		let offset = (y - grab_dy - self.shell_line_y(i, 0)) / line;
		(offset.round() as usize).min(last)
	}
	fn shell_remove(&mut self, k: usize) {
		if k < self.edited.shells.len() {
			self.edited.shells.remove(k);
		}
		self.commit_edit();
		self.focus = None;
	}
	// Add opens the new entry's Command field straight away: an entry with no
	// command names nothing to run, and is dropped rather than saved.
	fn shell_add(&mut self, i: usize) {
		self.commit_edit();
		let entry = crate::shells::adopted("", &self.edited.shells);
		self.edited.shells.push(entry);
		let k = self.edited.shells.len() - 1;
		self.focus = Some(Focus::Row(i, shell_part_index(k, ShellPart::Command)));
		self.open_edit(shell_field_row(k, true), true);
	}

	// Left edge of a row's label: its own sub-group depth in from the panel pad.
	fn label_x(&self, i: usize) -> f32 {
		self.rect.x + lay().pad + f32::from(self.specs[i].indent) * lay().indent
	}
	fn control_x(&self) -> f32 {
		self.rect.x + lay().pad + self.label_w
	}
	fn track(&self, i: usize) -> Rect {
		Rect {
			x: self.control_x(),
			y: self.row_y(i) + lay().row_height / 2.0 - 3.0,
			w: lay().slider_width,
			h: 6.0,
		}
	}
	fn swatch(&self, i: usize) -> Rect {
		Rect {
			x: self.control_x(),
			y: self.row_y(i) + (lay().row_height - lay().swatch) / 2.0,
			w: lay().swatch,
			h: lay().swatch,
		}
	}
	fn hexbox(&self, i: usize) -> Rect {
		Rect {
			x: self.control_x() + lay().swatch + 8.0,
			y: self.row_y(i) + (lay().row_height - lay().swatch) / 2.0,
			w: lay().hex_width,
			h: lay().swatch,
		}
	}
	// editable numeric field to the right of a slider (shows/edits the value)
	fn valbox(&self, i: usize) -> Rect {
		Rect {
			x: self.control_x() + lay().slider_width + 14.0,
			y: self.row_y(i) + (lay().row_height - lay().swatch) / 2.0,
			w: lay().value_width,
			h: lay().swatch,
		}
	}
	// wide editable field (background-image path), control_x -> the revert column
	fn textbox(&self, i: usize) -> Rect {
		let x = self.control_x();
		Rect {
			x,
			y: self.row_y(i) + (lay().row_height - lay().swatch) / 2.0,
			w: self.rect.x + self.rect.w - lay().pad - lay().revert_width - 6.0 - x,
			h: lay().swatch,
		}
	}
	// right-edge revert-to-default icon for row `i`
	fn revert_box(&self, i: usize) -> Rect {
		Rect {
			x: self.rect.x + self.rect.w - lay().pad - lay().revert_width,
			y: self.row_y(i) + (lay().row_height - lay().swatch) / 2.0,
			w: lay().revert_width,
			h: lay().swatch,
		}
	}
	fn checkbox(&self, i: usize) -> Rect {
		Rect {
			x: self.control_x(),
			y: self.row_y(i) + (lay().row_height - lay().swatch) / 2.0,
			w: lay().swatch,
			h: lay().swatch,
		}
	}
	fn dual_pitch(&self) -> f32 {
		lay().dual_pitch * self.ui_scale()
	}
	// checkbox `p` (0/1) on a Dual row; its label sits just to the right
	fn dual_box(&self, i: usize, p: u16) -> Rect {
		Rect {
			x: self.control_x() + p as f32 * self.dual_pitch(),
			y: self.row_y(i) + (lay().row_height - lay().swatch) / 2.0,
			w: lay().swatch,
			h: lay().swatch,
		}
	}
	// Radio geometry scales with the UI font (HiDPI or a large desktop font), so
	// multi-option labels don't collide the way fixed 96px pitch does at 2x.
	fn ui_scale(&self) -> f32 {
		(self.line_h / lay().base_line_height).max(1.0)
	}
	fn radio_pitch(&self) -> f32 {
		lay().radio_pitch * self.ui_scale()
	}
	fn radio_box_sz(&self) -> f32 {
		lay().radio_box * self.ui_scale()
	}
	// indicator box for radio option `k` in row `i`
	fn radio_box(&self, i: usize, k: usize) -> Rect {
		let size = self.radio_box_sz();
		Rect {
			x: self.control_x() + k as f32 * self.radio_pitch(),
			y: self.row_y(i) + (lay().row_height - size) / 2.0,
			w: size,
			h: size,
		}
	}
	// Collapsed dropdown box (the always-visible control): shows the current option
	// + a down-arrow; clicking it opens the popup list.
	fn dd_box(&self, i: usize) -> Rect {
		let h = (self.line_h + 6.0).max(lay().swatch);
		Rect {
			x: self.control_x(),
			y: self.row_y(i) + (lay().row_height - h) / 2.0,
			w: lay().dropdown_width * self.ui_scale(),
			h,
		}
	}
	// One option row inside the open popup.
	fn dd_item_h(&self) -> f32 {
		(self.line_h + lay().dropdown_item_pad).max(lay().dropdown_item_min)
	}
	// The open popup box. Opens downward from the collapsed box, or upward when that
	// would spill past the viewport bottom (so a dropdown low in a scrolled tab still
	// shows all its options).
	fn dd_popup(&self, i: usize, n: usize) -> Rect {
		let boxr = self.dd_box(i);
		let h = n as f32 * self.dd_item_h();
		let vp = self.viewport();
		let down_y = boxr.y + boxr.h;
		let y = if down_y + h <= vp.y + vp.h || boxr.y - h < vp.y {
			down_y
		} else {
			boxr.y - h
		};
		Rect {
			x: boxr.x,
			y,
			w: boxr.w,
			h,
		}
	}
	fn dd_item_rect(&self, i: usize, n: usize, k: usize) -> Rect {
		let popup = self.dd_popup(i, n);
		Rect {
			x: popup.x,
			y: popup.y + k as f32 * self.dd_item_h(),
			w: popup.w,
			h: self.dd_item_h(),
		}
	}
	// Number of focusable sub-controls in row `i` (0 for a header). Sliders and
	// the Dual (cursor) row expose two; every other control is a single part.
	fn parts_of(&self, i: usize) -> u16 {
		match self.specs[i].kind {
			Kind::Header(_) => 0,
			Kind::Slider { .. } | Kind::Dual { .. } => 2,
			Kind::Buttons(captions) => captions.len() as u16,
			// every entry's own controls, then the one Add stop past the end
			Kind::ShellList => self.edited.shells.len() as u16 * ShellPart::COUNT + 1,
			_ => 1,
		}
	}
	// The config Key that governs part `p` of row `i` (Dual parts differ; every
	// other kind uses the row's single key for both the value and its graying).
	fn part_key(&self, i: usize, p: u16) -> Key {
		match self.specs[i].kind {
			Kind::Dual { keys, .. } => keys[p as usize],
			_ => self.specs[i].key,
		}
	}
	// A push-button decides for itself (there is nothing to gate on); everything
	// else asks the setting behind it.
	fn part_disabled(&self, i: usize, p: u16) -> bool {
		match self.specs[i].kind {
			Kind::Buttons(_) => !self.theme_btn_enabled(ThemeBtn::of(p)),
			// Nothing in the grid is ever grayed: every stop is a value the user
			// can always edit, and reordering left the keyboard with the arrows.
			Kind::ShellList => false,
			_ => self.disabled(self.part_key(i, p)),
		}
	}
	// Flyover text for a control the environment disables rather than another
	// setting - explains why it is inert. Only the system-font toggles today,
	// and only when the OS reports no such setting to follow: Windows has a
	// system font size but no monospace family, a bare desktop may have neither.
	fn disabled_tip(key: Key) -> Option<&'static str> {
		let os = crate::sysfont::monospace();
		match key {
			Key::SystemFont if os.family.is_none() => Some("No system monospace font to follow"),
			Key::SystemFontSize if os.size_pt.is_none() => Some("No system font size to follow"),
			_ => None,
		}
	}
	// The flyover to show while the cursor rests on something that has one:
	// (text, anchor rect to hang the tip box under). Why a control is GRAYED
	// wins over what it does - that is the more urgent question when it is.
	fn hover_tip_dip(&self, mx: f32, my: f32) -> Option<(&'static str, Rect)> {
		for (action, r, _) in self.buttons() {
			if r.contains(mx, my) {
				let help = &ui().help;
				return Some((
					match action {
						Action::Cancel => help.cancel,
						Action::Apply => help.apply,
						_ => help.ok,
					},
					r,
				));
			}
		}
		let vp = self.viewport();
		if !vp.contains(mx, my) {
			return None;
		}
		for i in 0..self.specs.len() {
			if self.specs[i].tab != self.tab || matches!(self.specs[i].kind, Kind::Header(_)) {
				continue;
			}
			let grayed = self.disabled(self.specs[i].key);
			let tip = match Self::disabled_tip(self.specs[i].key).filter(|_| grayed) {
				Some(why) => why,
				None if !self.specs[i].help.is_empty() => self.specs[i].help,
				None => continue,
			};
			// hover target: the row's label + control span. The shells grid is
			// the exception - its "row" is the whole grid, and a tip that popped
			// up over every line of it would be in the way of the work; it hangs
			// off the column titles instead, which is where the question is.
			let ctl = self.checkbox(i);
			let hit = if matches!(self.specs[i].kind, Kind::ShellList) {
				Rect {
					x: self.rect.x + lay().pad,
					y: self.shell_head_y(i),
					w: self.rect.w - lay().pad * 2.0,
					h: self.line_h,
				}
			} else {
				Rect {
					x: self.rect.x + lay().pad,
					y: self.row_y(i),
					w: ctl.x + ctl.w - (self.rect.x + lay().pad),
					h: self.row_h(&self.specs[i].kind),
				}
			};
			if hit.contains(mx, my) {
				return Some((
					tip,
					if matches!(self.specs[i].kind, Kind::ShellList) {
						hit
					} else {
						ctl
					},
				));
			}
		}
		None
	}
	// Tight box around one focused sub-control (the keyboard-focus ring hugs this,
	// a couple px out, instead of spanning the whole row).
	fn focus_ctl_rect(&self, i: usize, p: u16) -> Rect {
		match self.specs[i].kind {
			Kind::Slider { .. } => {
				if p == 0 {
					let t = self.track(i);
					Rect {
						x: t.x,
						y: t.y - 7.0,
						w: t.w,
						h: t.h + 14.0,
					}
				} else {
					self.valbox(i)
				}
			}
			Kind::Dual { .. } => {
				let bx = self.dual_box(i, p);
				Rect {
					x: bx.x,
					y: bx.y,
					w: self.dual_pitch() - 12.0,
					h: bx.h,
				}
			}
			Kind::Toggle => self.checkbox(i),
			Kind::Text => self.textbox(i),
			Kind::Color => {
				let s = self.swatch(i);
				let h = self.hexbox(i);
				Rect {
					x: s.x,
					y: s.y,
					w: h.x + h.w - s.x,
					h: s.h,
				}
			}
			Kind::Radio(opts) => {
				let first = self.radio_box(i, 0);
				Rect {
					x: first.x,
					y: first.y - 2.0,
					w: opts.len() as f32 * self.radio_pitch() - 12.0,
					h: first.h + 4.0,
				}
			}
			Kind::Dropdown(_) => self.dd_box(i),
			Kind::Buttons(_) => self.row_btn_rect(i, p),
			Kind::ShellList => self.shell_stop_rect(i, p),
			Kind::Header(_) => self.track(i), // unreachable (headers aren't focusable)
		}
	}
	// Does the keyboard ring land exactly on a control's own outline? For a boxed
	// control it does, and then the box must not draw its border as well - a
	// field ringed twice reads as two outlines for one control. A color row is
	// the near miss: its ring spans the chip AND the hex field, so it stays a
	// couple of pixels out and only the hex field's own border stands down.
	fn ring_is_the_box(&self, i: usize, part: u16) -> bool {
		match self.specs[i].kind {
			Kind::Text | Kind::Dropdown(_) | Kind::Buttons(_) => true,
			Kind::Slider { .. } => part == 1,
			// the two fields and the Add button are boxes; the checkbox and the
			// icon buttons are not, so they keep the ring a couple of pixels out
			Kind::ShellList => matches!(
				shell_stop(part, self.edited.shells.len()),
				ShellStop::Add | ShellStop::Entry(_, ShellPart::Name | ShellPart::Command)
			),
			_ => false,
		}
	}
	fn ring_on(&self, i: usize, part: u16) -> bool {
		self.focus == Some(Focus::Row(i, part))
	}
	// Is this row at its config default? (drives the revert icon). A Dual row is
	// "default" only when both its keys are.
	fn row_is_default(&self, i: usize) -> bool {
		match self.specs[i].kind {
			Kind::Dual { keys, .. } => keys.iter().all(|&k| self.is_default(k)),
			_ => self.is_default(self.specs[i].key),
		}
	}
	// A row of push-buttons has no value, and the shells grid is a list rather
	// than a setting - neither has a default to go back to.
	fn has_revert(&self, i: usize) -> bool {
		!matches!(
			self.specs[i].kind,
			Kind::Header(_) | Kind::Buttons(_) | Kind::ShellList
		)
	}
	// Revert a whole row to defaults (both keys for a Dual row).
	fn row_revert(&mut self, i: usize) {
		match self.specs[i].kind {
			Kind::Dual { keys, .. } => {
				for k in keys {
					if !self.is_default(k) {
						self.revert(k);
					}
				}
			}
			_ => self.revert(self.specs[i].key),
		}
	}
	// Cancel, Apply, OK rects (right-aligned)
	fn buttons(&self) -> [(Action, Rect, &'static str); 3] {
		let y = self.rect.y + self.rect.h - lay().pad - self.btn_h();
		let x_ok = self.rect.x + self.rect.w - lay().pad - self.btn_w;
		let x_apply = x_ok - lay().button_gap - self.btn_w;
		let x_cancel = x_apply - lay().button_gap - self.btn_w;
		let mk = |x| Rect {
			x,
			y,
			w: self.btn_w,
			h: self.btn_h(),
		};
		[
			(Action::Cancel, mk(x_cancel), "Cancel"),
			(Action::Apply, mk(x_apply), "Apply"),
			(Action::Ok, mk(x_ok), "OK"),
		]
	}

	// ---- themes ---------------------------------------------------------------

	// One of a Buttons row's push-buttons. They start at the control column, so
	// they line up under whatever the row above them holds.
	fn row_btn_rect(&self, i: usize, part: u16) -> Rect {
		let h = self.btn_h();
		Rect {
			x: self.control_x() + f32::from(part) * (self.row_btn_w + lay().button_gap),
			y: self.row_y(i) + (self.row_h(&self.specs[i].kind) - h) / 2.0,
			w: self.row_btn_w,
			h,
		}
	}

	// The saved theme the `theme` setting currently names, if it names one.
	fn user_theme_index(&self) -> Option<usize> {
		let name = self.edited.theme.trim();
		self.edited
			.user_themes
			.iter()
			.position(|t| t.name.eq_ignore_ascii_case(name))
	}

	// Has the user moved a color away from what the current theme says? That IS
	// the unsaved-changes test, and it needs no flag of its own: an edited color
	// lives on as a `colors.*` line, so the answer survives a restart for free.
	fn theme_dirty(&self) -> bool {
		// resolve the palette once - this runs per button, per frame
		let palette = self.theme_palette();
		(0..crate::theme::PALETTE_KEYS.len())
			.any(|i| self.get_col(Self::palette_key(i)) != palette.get(i))
	}

	// The dialog row key holding palette color `i` (same order as PALETTE_KEYS).
	fn palette_key(i: usize) -> Key {
		match i {
			0 => Key::ColBg,
			1 => Key::ColFg,
			2 => Key::ColCursor,
			3 => Key::ColHighlight,
			4 => Key::ColFocus,
			5 => Key::ColMenuBg,
			6 => Key::ColMenuFg,
			7 => Key::ColDialogBg,
			8 => Key::ColDialogFg,
			_ => Key::ColGutter,
		}
	}

	fn theme_btn_enabled(&self, which: ThemeBtn) -> bool {
		match which {
			ThemeBtn::Save => self.theme_dirty(),
			ThemeBtn::SaveAs => true,
			// a built-in is not the user's to rename or throw away; saving over its
			// name first makes a copy that is
			ThemeBtn::Rename | ThemeBtn::Delete => self.user_theme_index().is_some(),
		}
	}

	// Take on the colors of whatever theme and mode are now selected. Switching
	// theme adopts the new scheme rather than keeping tweaks made to the old one
	// on top of it - a picker that visibly changed nothing would be worse, and the
	// tweaks were changes to the theme being left behind.
	//
	// Reverting each key rather than just setting it is what keeps the file honest:
	// the colors on screen are now the theme's own, so the per-color overrides have
	// nothing left to say and Apply comments them out. Setting them alone would
	// write ten active colors.* lines pinning this one palette, which then wins over
	// every later theme change and freezes one variant under theme_mode: system.
	fn adopt_theme(&mut self) {
		let pal = self.theme_palette();
		for i in 0..crate::theme::PALETTE_KEYS.len() {
			// default_col resolves through theme_palette, so this lands on pal.get(i)
			self.revert(Self::palette_key(i));
		}
		self.edited.ansi = pal.ansi;
	}

	// Store the colors on screen under `name`, replacing a saved theme of that
	// name or adding one. The variant the dialog is NOT showing is carried over
	// from whatever `name` resolves to today, so a theme is always complete.
	fn save_theme_as(&mut self, name: &str) {
		let name = name.trim().to_string();
		let dark_now = crate::theme::is_dark_mode(&self.edited.theme_mode, config::is_dark());
		let other = crate::theme::resolve_in(
			&self.edited.user_themes,
			&self.edited.theme,
			if dark_now { "light" } else { "dark" },
			config::is_dark(),
		);
		let mut shown = other; // start from a full palette, then overwrite with the edits
		for i in 0..crate::theme::PALETTE_KEYS.len() {
			shown.set(i, self.get_col(Self::palette_key(i)));
		}
		shown.ansi = self.edited.ansi;
		let (dark, light) = if dark_now {
			(shown, other)
		} else {
			(other, shown)
		};
		let existing = self
			.edited
			.user_themes
			.iter()
			.position(|t| t.name.eq_ignore_ascii_case(&name));
		let slug = match existing {
			Some(k) => self.edited.user_themes[k].slug.clone(),
			None => self.free_slug(&name),
		};
		let theme = crate::theme::UserTheme {
			slug,
			name: name.clone(),
			dark,
			light,
		};
		match existing {
			Some(k) => self.edited.user_themes[k] = theme,
			None => self.edited.user_themes.push(theme),
		}
		self.edited.theme = name;
		// the tweaks are the theme's own colors now, so the per-color overrides
		// have nothing left to say and are commented back out on Apply
		for i in 0..crate::theme::PALETTE_KEYS.len() {
			self.revert(Self::palette_key(i));
		}
	}

	// A config path segment for a new theme: the name reduced to something a path
	// can hold, then made unique. The slug never changes afterwards, so a rename
	// rewrites one line rather than moving a subtree.
	fn free_slug(&self, name: &str) -> String {
		let base: String = name
			.chars()
			.map(|c| {
				if c.is_ascii_alphanumeric() {
					c.to_ascii_lowercase()
				} else {
					'_'
				}
			})
			.collect();
		let base = base.trim_matches('_').to_string();
		let base = if base.is_empty() { "theme" } else { &base }.to_string();
		let taken = |s: &str| self.edited.user_themes.iter().any(|t| t.slug == s);
		if !taken(&base) {
			return base;
		}
		(2..=u32::from(u16::MAX))
			.map(|n| format!("{base}{n}"))
			.find(|s| !taken(s))
			.unwrap_or(base)
	}

	// Why OK cannot accept the typed name, if it cannot.
	fn name_problem(&self, which: ThemeBtn, name: &str) -> Option<String> {
		let name = name.trim();
		if name.is_empty() {
			return Some("Enter a name.".into());
		}
		let clashes = self
			.edited
			.user_themes
			.iter()
			.any(|t| t.name.eq_ignore_ascii_case(name));
		// Save as over a saved theme's name replaces it, which is a fair reading of
		// the button; a rename onto another theme's name would merge two into one.
		if which == ThemeBtn::Rename && clashes {
			return Some("That name is already taken.".into());
		}
		None
	}

	// Press a theme button: Save acts at once, the other three ask first.
	fn theme_action(&mut self, which: ThemeBtn) {
		if !self.theme_btn_enabled(which) {
			return;
		}
		self.commit_edit();
		match which {
			ThemeBtn::Save => {
				let name = self.edited.theme.clone();
				self.save_theme_as(&name);
			}
			ThemeBtn::SaveAs | ThemeBtn::Rename => {
				let (title, seed) = if which == ThemeBtn::SaveAs {
					("Enter a new theme name".to_string(), String::new())
				} else {
					("Rename theme".to_string(), self.edited.theme.clone())
				};
				// a rename opens on the existing name, selected, the way a rename
				// field does everywhere else
				let mut edit = EditState::new(PROMPT_ROW, seed);
				edit.sel = (!edit.buf.is_empty()).then_some(0);
				self.edit = Some(edit);
				self.prompt = Some(Prompt {
					job: PromptJob::Theme(which),
					title,
					focus: PromptFocus::Field,
					warn: None,
				});
			}
			ThemeBtn::Delete => {
				self.prompt = Some(Prompt {
					job: PromptJob::Theme(which),
					title: format!("Really delete theme \"{}\"?", self.edited.theme),
					focus: PromptFocus::Ok,
					warn: None,
				});
			}
		}
	}

	// OK in the prompt box. A name that will not do keeps the box open and says why.
	fn prompt_accept(&mut self) {
		let Some(prompt) = self.prompt.as_ref() else {
			return;
		};
		let job = prompt.job;
		let typed = prompt
			.has_field()
			.then(|| self.edit.as_ref().map_or(String::new(), |e| e.buf.clone()));
		match (job, typed) {
			(PromptJob::Theme(which), Some(name)) => {
				if let Some(warn) = self.name_problem(which, &name) {
					if let Some(prompt) = self.prompt.as_mut() {
						prompt.warn = Some(warn);
					}
					return;
				}
				if which == ThemeBtn::Rename {
					self.rename_theme(&name);
				} else {
					self.save_theme_as(&name);
				}
			}
			(PromptJob::Theme(_), None) => self.delete_theme(),
			(PromptJob::DropShell(at), _) => self.shell_remove(at),
		}
		self.prompt_close();
	}
	fn prompt_close(&mut self) {
		self.prompt = None;
		self.emenu = None;
		if self.edit.as_ref().is_some_and(|e| e.row == PROMPT_ROW) {
			self.edit = None;
		}
	}

	// ---- the prompt box -------------------------------------------------------

	// Centered over the panel, sized to what it holds. Two buttons, right-aligned,
	// the same way the dialog's own footer reads.
	fn prompt_rect(&self) -> Rect {
		let Some(prompt) = &self.prompt else {
			return Rect {
				x: 0.0,
				y: 0.0,
				w: 0.0,
				h: 0.0,
			};
		};
		let w = (self.rect.w - lay().pad * 6.0).max(240.0);
		let row = self.line_h + lay().row_pad;
		let mut h = lay().pad + row + lay().pad;
		if prompt.has_field() {
			h += row + lay().row_pad;
		}
		if prompt.warn.is_some() {
			h += row;
		}
		h += self.btn_h();
		Rect {
			x: self.rect.x + (self.rect.w - w) / 2.0,
			y: self.rect.y + (self.rect.h - h) / 2.0,
			w,
			h,
		}
	}
	fn prompt_field_rect(&self) -> Option<Rect> {
		let prompt = self.prompt.as_ref()?;
		if !prompt.has_field() {
			return None;
		}
		let r = self.prompt_rect();
		Some(Rect {
			x: r.x + lay().pad,
			y: r.y + lay().pad + self.line_h + lay().row_pad,
			w: r.w - lay().pad * 2.0,
			h: lay().swatch.max(self.line_h + lay().row_pad),
		})
	}
	fn prompt_btn_rect(&self, part: PromptFocus) -> Rect {
		let r = self.prompt_rect();
		let h = self.btn_h();
		let x_ok = r.x + r.w - lay().pad - self.btn_w;
		Rect {
			x: if part == PromptFocus::Ok {
				x_ok
			} else {
				x_ok - lay().button_gap - self.btn_w
			},
			y: r.y + r.h - lay().pad - h,
			w: self.btn_w,
			h,
		}
	}
	// Tab / arrows walk field -> Cancel -> OK, wrapping; a confirmation has no field.
	fn prompt_focus_move(&mut self, forward: bool) {
		let Some(prompt) = self.prompt.as_mut() else {
			return;
		};
		let stops: &[PromptFocus] = if prompt.has_field() {
			&[PromptFocus::Field, PromptFocus::Cancel, PromptFocus::Ok]
		} else {
			&[PromptFocus::Cancel, PromptFocus::Ok]
		};
		let cur = stops.iter().position(|&s| s == prompt.focus).unwrap_or(0);
		let step = if forward { 1 } else { stops.len() - 1 };
		prompt.focus = stops[(cur + step) % stops.len()];
	}
	// Every click while the box is up belongs to it: its own controls act, and
	// anything outside is swallowed rather than reaching the panel behind.
	fn prompt_mouse_down(&mut self, x: f32, y: f32, measure: &mut impl FnMut(&str) -> f32) {
		if self.emenu.is_some() {
			return;
		}
		for part in [PromptFocus::Cancel, PromptFocus::Ok] {
			if !self.prompt_btn_rect(part).contains(x, y) {
				continue;
			}
			if let Some(prompt) = self.prompt.as_mut() {
				prompt.focus = part;
			}
			match part {
				PromptFocus::Ok => self.prompt_accept(),
				_ => self.prompt_close(),
			}
			return;
		}
		if let Some(field) = self.prompt_field_rect() {
			if field.contains(x, y) {
				if let Some(prompt) = self.prompt.as_mut() {
					prompt.focus = PromptFocus::Field;
				}
				self.field_click(PROMPT_ROW, (PROMPT_ROW, 0), field, x, measure);
			}
		}
	}

	fn rename_theme(&mut self, name: &str) {
		let name = name.trim().to_string();
		if let Some(k) = self.user_theme_index() {
			self.edited.user_themes[k].name.clone_from(&name);
			self.edited.theme = name;
		}
	}

	// Drop the saved theme. A built-in of the same name comes back out from behind
	// it; otherwise the selection falls to the first theme left.
	fn delete_theme(&mut self) {
		let Some(k) = self.user_theme_index() else {
			return;
		};
		let name = self.edited.user_themes[k].name.clone();
		self.edited.user_themes.remove(k);
		if !crate::theme::is_builtin(&name) {
			self.edited.theme = crate::theme::all_names(&self.edited.user_themes)
				.first()
				.cloned()
				.unwrap_or_else(|| name.clone());
		}
		self.adopt_theme();
	}

	fn get_f32(&self, key: Key) -> f32 {
		let settings = &self.edited;
		match key {
			Key::Opacity => to_percent(settings.opacity),
			Key::BgOpacity => to_percent(settings.wallpaper_opacity),
			Key::BgBlur => settings.wallpaper_blur,
			Key::BgContrastSize => to_percent(settings.wallpaper_contrast_mask_size),
			Key::BgContrastStrength => to_percent(settings.wallpaper_contrast_mask_strength),
			Key::BgContrastAuto => to_percent(settings.wallpaper_contrast_mask_auto),
			Key::ScrimRadius => settings.text_scrim_radius,
			Key::ScrimSoftness => to_percent(settings.text_scrim_softness),
			Key::ScrimStrength => settings.text_scrim_strength,
			Key::Outline => settings.text_outline,
			Key::CursorBlink => settings.cursor_blink_rate_ms,
			Key::CursorHeight => settings.cursor_size_height,
			Key::CursorWidth => settings.cursor_size_width,
			Key::CursorResume => settings.cursor_animation_resume_s,
			Key::FontSize => settings.font_size,
			Key::LineHeight => settings.line_height_scale,
			Key::Margin => settings.margin,
			// shown as an intuitive 1..100 speed (higher = faster); stored as tau
			Key::ScrollEaseIn => {
				falling_slider(settings.scroll_ease_in_ms, EASE_IN_MIN, EASE_IN_MAX)
			}
			Key::ScrollRampUp => {
				falling_slider(settings.scroll_ramp_up_ms, RAMP_UP_MIN, RAMP_UP_MAX)
			}
			Key::SingleScreenTau => tau_to_speed(settings.scroll_single_screen_tau_ms),
			Key::ScrollRampDown => {
				falling_slider(settings.scroll_ramp_down_ms, RAMP_DOWN_MIN, RAMP_DOWN_MAX)
			}
			Key::ScrollEaseOut => {
				falling_slider(settings.scroll_ease_out_ms, EASE_OUT_MIN, EASE_OUT_MAX)
			}
			Key::WheelLines => settings.wheel_lines,
			Key::ScrollbarThickness => settings.scrollbar_thickness,
			Key::Columns => settings.columns as f32,
			Key::Rows => settings.rows as f32,
			_ => 0.0,
		}
	}
	fn set_f32(&mut self, key: Key, value: f32) {
		// adjusting the size explicitly means we're no longer following the OS size
		if key == Key::FontSize {
			self.edited.use_system_font_size = false;
		}
		let settings = &mut self.edited;
		match key {
			Key::Opacity => settings.opacity = from_percent(value),
			Key::BgOpacity => settings.wallpaper_opacity = from_percent(value),
			Key::BgBlur => settings.wallpaper_blur = value,
			Key::BgContrastSize => settings.wallpaper_contrast_mask_size = from_percent(value),
			Key::BgContrastStrength => {
				settings.wallpaper_contrast_mask_strength = from_percent(value);
			}
			Key::BgContrastAuto => settings.wallpaper_contrast_mask_auto = from_percent(value),
			Key::ScrimRadius => settings.text_scrim_radius = value,
			Key::ScrimSoftness => settings.text_scrim_softness = from_percent(value),
			Key::ScrimStrength => settings.text_scrim_strength = value,
			Key::Outline => settings.text_outline = value,
			Key::CursorBlink => settings.cursor_blink_rate_ms = value,
			Key::CursorHeight => settings.cursor_size_height = value,
			Key::CursorWidth => settings.cursor_size_width = value,
			Key::CursorResume => settings.cursor_animation_resume_s = value,
			Key::FontSize => settings.font_size = value,
			Key::LineHeight => settings.line_height_scale = value,
			Key::Margin => settings.margin = value,
			Key::ScrollEaseIn => {
				settings.scroll_ease_in_ms = falling_value(value, EASE_IN_MIN, EASE_IN_MAX);
			}
			Key::ScrollRampUp => {
				settings.scroll_ramp_up_ms = falling_value(value, RAMP_UP_MIN, RAMP_UP_MAX);
			}
			Key::SingleScreenTau => settings.scroll_single_screen_tau_ms = speed_to_tau(value),
			Key::ScrollRampDown => {
				settings.scroll_ramp_down_ms = falling_value(value, RAMP_DOWN_MIN, RAMP_DOWN_MAX);
			}
			Key::ScrollEaseOut => {
				settings.scroll_ease_out_ms = falling_value(value, EASE_OUT_MIN, EASE_OUT_MAX);
			}
			Key::WheelLines => settings.wheel_lines = value,
			Key::ScrollbarThickness => settings.scrollbar_thickness = value,
			Key::Columns => settings.columns = value.round().max(1.0) as usize,
			Key::Rows => settings.rows = value.round().max(1.0) as usize,
			_ => {}
		}
	}
	// Current value of a Text field (background image path / font family).
	fn get_text(&self, key: Key) -> String {
		match key {
			// the configured text, not the resolved path (auto-detect still shows
			// the path it found, since there is no configured text to show)
			Key::BgImage => {
				if self.edited.wallpaper_raw.is_empty() {
					self.edited
						.wallpaper
						.as_ref()
						.map(|path| path.to_string_lossy().into_owned())
						.unwrap_or_default()
				} else {
					self.edited.wallpaper_raw.clone()
				}
			}
			Key::FontFamily => self.edited.font_family.clone().unwrap_or_default(),
			Key::LinkOpenCommand => self.edited.hyperlink_open_command.clone(),
			Key::StartupDirectory => self.edited.startup_directory.clone(),
			_ => String::new(),
		}
	}
	fn set_text(&mut self, key: Key, text: &str) {
		let trimmed = text.trim();
		match key {
			Key::BgImage => {
				self.edited.wallpaper_raw = trimmed.to_string();
				// resolve like the loader does (relative to the config dir),
				// so a typed relative name live-applies instead of missing
				self.edited.wallpaper = crate::config::resolve_wallpaper(
					(!trimmed.is_empty()).then(|| trimmed.to_string()),
				);
			}
			Key::FontFamily => {
				// an explicit family means we're not following the OS font
				self.edited.use_system_font = false;
				self.edited.font_family = if trimmed.is_empty() {
					None
				} else {
					Some(trimmed.to_string())
				};
			}
			Key::LinkOpenCommand => self.edited.hyperlink_open_command = trimmed.to_string(),
			Key::StartupDirectory => self.edited.startup_directory = trimmed.to_string(),
			_ => {}
		}
	}
	fn get_toggle(&self, key: Key) -> bool {
		match key {
			// These two read what is STORED, like every other row. Where the desktop
			// names no font to follow they are grayed and the flyover says why -
			// that is how the dialog says "inert" everywhere else. Showing them
			// unchecked instead misreported the setting: the box sat unchecked
			// beside a dimmed revert arrow, claiming unchecked was the default when
			// the default is on. `gate_ok` still asks the EFFECTIVE state, so the
			// family field it overrides stays editable.
			Key::SystemFont => self.edited.use_system_font,
			Key::SystemFontSize => self.edited.use_system_font_size,
			Key::Transparency => self.edited.transparent_background,
			Key::BackdropBlur => self.edited.transparent_background_blur,
			Key::TextScrim => self.edited.text_scrim,
			Key::CursorScrim => self.edited.cursor_scrim,
			Key::CursorOutline => self.edited.cursor_outline,
			Key::RememberSize => self.edited.remember_size,
			Key::CopyOnSelect => self.edited.copy_on_select,
			Key::Hyperlinks => self.edited.hyperlinks,
			Key::BgContrastMask => self.edited.wallpaper_contrast_mask,
			Key::BgEnabled => self.edited.wallpaper_enabled,
			Key::BgRotate => self.edited.wallpaper_rotate_enabled,
			Key::BgHonorXmp => self.edited.wallpaper_honor_xmp,
			Key::SmoothScroll => self.edited.scroll_smooth,
			Key::Scrollbar => self.edited.scrollbar,
			Key::ScrollbarAutoHide => self.edited.scrollbar_auto_hide,
			_ => false,
		}
	}
	fn set_toggle(&mut self, key: Key, on: bool) {
		match key {
			Key::SystemFont => self.edited.use_system_font = on,
			Key::SystemFontSize => self.edited.use_system_font_size = on,
			Key::Transparency => self.edited.transparent_background = on,
			Key::BackdropBlur => self.edited.transparent_background_blur = on,
			Key::TextScrim => self.edited.text_scrim = on,
			Key::CursorScrim => self.edited.cursor_scrim = on,
			Key::CursorOutline => self.edited.cursor_outline = on,
			Key::RememberSize => self.edited.remember_size = on,
			Key::CopyOnSelect => self.edited.copy_on_select = on,
			Key::Hyperlinks => self.edited.hyperlinks = on,
			Key::BgContrastMask => self.edited.wallpaper_contrast_mask = on,
			Key::BgEnabled => self.edited.wallpaper_enabled = on,
			Key::BgRotate => self.edited.wallpaper_rotate_enabled = on,
			Key::BgHonorXmp => self.edited.wallpaper_honor_xmp = on,
			Key::SmoothScroll => self.edited.scroll_smooth = on,
			Key::Scrollbar => self.edited.scrollbar = on,
			Key::ScrollbarAutoHide => self.edited.scrollbar_auto_hide = on,
			_ => {}
		}
	}
	fn get_radio(&self, key: Key) -> usize {
		match key {
			Key::BgFit => match self.edited.wallpaper_default_fit {
				config::Fit::Zoom => 1,
				config::Fit::Stretch => 0,
			},
			// display order: SDF, DT, Dilate, Gaussian
			Key::ScrimFunction => match self.edited.text_scrim_function.as_str() {
				"dt" => 1,
				"dilate" => 2,
				"gaussian" => 3,
				_ => 0, // sdf
			},
			// display order: Exponential, Half-normal, Log, Sigmoid, Linear
			Key::ScrimRamp => match self.edited.text_scrim_ramp.as_str() {
				"half_normal" => 1,
				"log" => 2,
				"sigmoid" => 3,
				"linear" => 4,
				_ => 0, // exp
			},
			Key::CursorAnimation => match self.edited.cursor_animation.as_str() {
				"phase" => 1,
				"pulse_horizontal" => 3,
				"pulse_both" => 4,
				"none" => 0,
				_ => 2, // pulse_vertical
			},
			Key::Theme => crate::theme::all_names(&self.edited.user_themes)
				.iter()
				.position(|n| n.eq_ignore_ascii_case(self.edited.theme.trim()))
				.unwrap_or(0),
			Key::ThemeMode => match self.edited.theme_mode.as_str() {
				"light" => 1,
				"system" => 2,
				_ => 0, // dark
			},
			_ => 0,
		}
	}
	fn set_radio(&mut self, key: Key, idx: usize) {
		match key {
			Key::BgFit => {
				self.edited.wallpaper_default_fit = if idx == 1 {
					config::Fit::Zoom
				} else {
					config::Fit::Stretch
				};
			}
			Key::ScrimFunction => {
				self.edited.text_scrim_function = match idx {
					1 => "dt",
					2 => "dilate",
					3 => "gaussian",
					_ => "sdf",
				}
				.to_string();
			}
			Key::ScrimRamp => {
				self.edited.text_scrim_ramp = match idx {
					1 => "half_normal",
					2 => "log",
					3 => "sigmoid",
					4 => "linear",
					_ => "exp",
				}
				.to_string();
			}
			Key::CursorAnimation => {
				self.edited.cursor_animation = match idx {
					0 => "none",
					1 => "phase",
					3 => "pulse_horizontal",
					4 => "pulse_both",
					_ => "pulse_vertical",
				}
				.to_string();
			}
			// picking a theme or a mode re-reads the whole palette, so the color
			// rows below follow the selection instead of describing the last one
			Key::Theme => {
				let names = crate::theme::all_names(&self.edited.user_themes);
				if let Some(name) = names.get(idx) {
					self.edited.theme.clone_from(name);
					self.adopt_theme();
				}
			}
			Key::ThemeMode => {
				self.edited.theme_mode = match idx {
					1 => "light",
					2 => "system",
					_ => "dark",
				}
				.to_string();
				self.adopt_theme();
			}
			_ => {}
		}
	}
	// A control grayed out because a prerequisite toggle is off (the opacity
	// slider needs Transparency; the scrim radius needs Text scrim; the explicit
	// columns/rows are inactive when "Remember last size" is on).
	fn disabled(&self, key: Key) -> bool {
		!ui().needs_of(key).iter().all(|need| self.gate_ok(need))
			// nothing for a system-font toggle to follow (the tip says so)
			|| Self::disabled_tip(key).is_some()
	}
	// Is one declared prerequisite satisfied? A slider counts while it sits above
	// zero, everything else while it is switched on - except the two system-font
	// switches, which only bite when the desktop actually names a font to follow.
	fn gate_ok(&self, need: &ui_spec::Need) -> bool {
		let on = match need.key {
			Key::SystemFont => config::system_font_face_active(&self.edited),
			Key::SystemFontSize => config::system_font_size_active(&self.edited),
			_ if need.numeric => self.get_f32(need.key) > 0.0,
			_ => self.get_toggle(need.key),
		};
		on != need.invert
	}
	fn get_col(&self, key: Key) -> [u8; 3] {
		let settings = &self.edited;
		match key {
			Key::ColBg => settings.bg,
			Key::ColFg => settings.fg,
			Key::ColCursor => settings.cursor,
			Key::ColHighlight => settings.highlight,
			Key::ColFocus => settings.focus,
			Key::ColGutter => settings.gutter,
			Key::ColMenuBg => settings.menu_bg,
			Key::ColMenuFg => settings.menu_fg,
			Key::ColDialogBg => settings.dialog_bg,
			Key::ColDialogFg => settings.dialog_fg,
			Key::ColScrollbarThumb => settings.scrollbar_thumb,
			Key::ColScrollbarTrough => settings.scrollbar_trough,
			_ => [0, 0, 0],
		}
	}
	fn set_col(&mut self, key: Key, color: [u8; 3]) {
		let settings = &mut self.edited;
		match key {
			Key::ColBg => settings.bg = color,
			Key::ColFg => settings.fg = color,
			Key::ColCursor => settings.cursor = color,
			Key::ColHighlight => settings.highlight = color,
			Key::ColFocus => settings.focus = color,
			Key::ColGutter => settings.gutter = color,
			Key::ColMenuBg => settings.menu_bg = color,
			Key::ColMenuFg => settings.menu_fg = color,
			Key::ColDialogBg => settings.dialog_bg = color,
			Key::ColDialogFg => settings.dialog_fg = color,
			Key::ColScrollbarThumb => settings.scrollbar_thumb = color,
			Key::ColScrollbarTrough => settings.scrollbar_trough = color,
			_ => {}
		}
	}

	// The active theme's palette - the effective default for the colors.* keys
	// (commented-out colors fall back to the theme, not to SilkTerm-dark).
	fn theme_palette(&self) -> crate::theme::Palette {
		crate::theme::resolve_in(
			&self.edited.user_themes,
			&self.edited.theme,
			&self.edited.theme_mode,
			config::is_dark(),
		)
	}
	fn default_col(&self, key: Key) -> [u8; 3] {
		let palette = self.theme_palette();
		match key {
			Key::ColBg => palette.bg,
			Key::ColFg => palette.fg,
			Key::ColCursor => palette.cursor,
			Key::ColHighlight => palette.highlight,
			Key::ColFocus => palette.focus,
			Key::ColGutter => palette.gutter,
			Key::ColMenuBg => palette.menu_bg,
			Key::ColMenuFg => palette.menu_fg,
			Key::ColDialogBg => palette.dialog_bg,
			Key::ColDialogFg => palette.dialog_fg,
			// chrome, not a palette color - the same neutral under every theme
			Key::ColScrollbarThumb => config::SCROLLBAR_THUMB_DEF,
			Key::ColScrollbarTrough => config::SCROLLBAR_TROUGH_DEF,
			_ => [0, 0, 0],
		}
	}

	// Is this setting at its config default? Drives the revert icon's state.
	fn is_default(&self, key: Key) -> bool {
		let edited = &self.edited;
		let defaults = &self.defaults;
		match key {
			Key::Transparency => edited.transparent_background == defaults.transparent_background,
			Key::BackdropBlur => {
				edited.transparent_background_blur == defaults.transparent_background_blur
			}
			Key::TextScrim => edited.text_scrim == defaults.text_scrim,
			Key::CursorScrim => edited.cursor_scrim == defaults.cursor_scrim,
			Key::CursorOutline => edited.cursor_outline == defaults.cursor_outline,
			Key::BgContrastMask => {
				edited.wallpaper_contrast_mask == defaults.wallpaper_contrast_mask
			}
			Key::SystemFont => edited.use_system_font == defaults.use_system_font,
			Key::SystemFontSize => edited.use_system_font_size == defaults.use_system_font_size,
			Key::RememberSize => edited.remember_size == defaults.remember_size,
			Key::CopyOnSelect => edited.copy_on_select == defaults.copy_on_select,
			Key::Hyperlinks => edited.hyperlinks == defaults.hyperlinks,
			Key::SmoothScroll => edited.scroll_smooth == defaults.scroll_smooth,
			Key::Scrollbar => edited.scrollbar == defaults.scrollbar,
			Key::ScrollbarAutoHide => edited.scrollbar_auto_hide == defaults.scrollbar_auto_hide,
			Key::BgFit => edited.wallpaper_default_fit == defaults.wallpaper_default_fit,
			Key::BgEnabled => edited.wallpaper_enabled == defaults.wallpaper_enabled,
			Key::BgRotate => edited.wallpaper_rotate_enabled == defaults.wallpaper_rotate_enabled,
			Key::BgHonorXmp => edited.wallpaper_honor_xmp == defaults.wallpaper_honor_xmp,
			Key::ScrimRamp => edited.text_scrim_ramp == defaults.text_scrim_ramp,
			Key::BgImage => edited.wallpaper == defaults.wallpaper,
			Key::FontFamily => edited.font_family == defaults.font_family,
			Key::LinkOpenCommand => {
				edited.hyperlink_open_command == defaults.hyperlink_open_command
			}
			Key::StartupDirectory => edited.startup_directory == defaults.startup_directory,
			Key::Theme => edited.theme == defaults.theme,
			Key::ThemeMode => edited.theme_mode == defaults.theme_mode,
			Key::ColBg
			| Key::ColFg
			| Key::ColCursor
			| Key::ColHighlight
			| Key::ColFocus
			| Key::ColGutter
			| Key::ColMenuBg
			| Key::ColMenuFg
			| Key::ColDialogBg
			| Key::ColDialogFg
			| Key::ColScrollbarThumb
			| Key::ColScrollbarTrough => self.get_col(key) == self.default_col(key),
			// buttons and headings hold nothing to revert
			Key::None | Key::ThemeActions => true,
			// the sliders
			_ => self.get_f32(key) == self.default_f32(key),
		}
	}
	// Default for a slider key, in get_f32's own units (speed for SingleScreenTau).
	fn default_f32(&self, key: Key) -> f32 {
		let defaults = &self.defaults;
		match key {
			Key::Opacity => to_percent(defaults.opacity),
			Key::BgOpacity => to_percent(defaults.wallpaper_opacity),
			Key::BgBlur => defaults.wallpaper_blur,
			Key::BgContrastSize => to_percent(defaults.wallpaper_contrast_mask_size),
			Key::BgContrastStrength => to_percent(defaults.wallpaper_contrast_mask_strength),
			Key::BgContrastAuto => to_percent(defaults.wallpaper_contrast_mask_auto),
			Key::ScrimRadius => defaults.text_scrim_radius,
			Key::ScrimSoftness => to_percent(defaults.text_scrim_softness),
			Key::ScrimStrength => defaults.text_scrim_strength,
			Key::Outline => defaults.text_outline,
			Key::CursorBlink => defaults.cursor_blink_rate_ms,
			Key::CursorHeight => defaults.cursor_size_height,
			Key::CursorWidth => defaults.cursor_size_width,
			Key::CursorResume => defaults.cursor_animation_resume_s,
			Key::FontSize => defaults.font_size,
			Key::LineHeight => defaults.line_height_scale,
			Key::Margin => defaults.margin,
			Key::ScrollEaseIn => {
				falling_slider(defaults.scroll_ease_in_ms, EASE_IN_MIN, EASE_IN_MAX)
			}
			Key::ScrollRampUp => {
				falling_slider(defaults.scroll_ramp_up_ms, RAMP_UP_MIN, RAMP_UP_MAX)
			}
			Key::SingleScreenTau => tau_to_speed(defaults.scroll_single_screen_tau_ms),
			Key::ScrollRampDown => {
				falling_slider(defaults.scroll_ramp_down_ms, RAMP_DOWN_MIN, RAMP_DOWN_MAX)
			}
			Key::ScrollEaseOut => {
				falling_slider(defaults.scroll_ease_out_ms, EASE_OUT_MIN, EASE_OUT_MAX)
			}
			Key::WheelLines => defaults.wheel_lines,
			Key::ScrollbarThickness => defaults.scrollbar_thickness,
			Key::Columns => defaults.columns as f32,
			Key::Rows => defaults.rows as f32,
			_ => 0.0,
		}
	}
	// Revert a setting to its default and remember its config key(s), so Apply
	// can comment them out in config.shcl (config::revert_keys).
	fn revert(&mut self, key: Key) {
		match key {
			Key::Transparency
			| Key::BackdropBlur
			| Key::TextScrim
			| Key::CursorScrim
			| Key::CursorOutline
			| Key::SystemFont
			| Key::SystemFontSize
			| Key::RememberSize
			| Key::CopyOnSelect
			| Key::Hyperlinks
			| Key::BgEnabled
			| Key::BgRotate
			| Key::BgHonorXmp
			| Key::Scrollbar
			| Key::ScrollbarAutoHide
			| Key::BgContrastMask => {
				let default_val = match key {
					Key::Transparency => self.defaults.transparent_background,
					Key::BackdropBlur => self.defaults.transparent_background_blur,
					Key::TextScrim => self.defaults.text_scrim,
					Key::CursorScrim => self.defaults.cursor_scrim,
					Key::CursorOutline => self.defaults.cursor_outline,
					Key::SystemFont => self.defaults.use_system_font,
					Key::SystemFontSize => self.defaults.use_system_font_size,
					Key::CopyOnSelect => self.defaults.copy_on_select,
					Key::Hyperlinks => self.defaults.hyperlinks,
					Key::BgContrastMask => self.defaults.wallpaper_contrast_mask,
					Key::BgEnabled => self.defaults.wallpaper_enabled,
					Key::BgRotate => self.defaults.wallpaper_rotate_enabled,
					Key::BgHonorXmp => self.defaults.wallpaper_honor_xmp,
					Key::SmoothScroll => self.defaults.scroll_smooth,
					Key::Scrollbar => self.defaults.scrollbar,
					Key::ScrollbarAutoHide => self.defaults.scrollbar_auto_hide,
					_ => self.defaults.remember_size,
				};
				self.set_toggle(key, default_val);
			}
			Key::BgFit => self.edited.wallpaper_default_fit = self.defaults.wallpaper_default_fit,
			Key::ScrimRamp => self.edited.text_scrim_ramp = self.defaults.text_scrim_ramp.clone(),
			Key::BgImage => {
				self.edited.wallpaper = self.defaults.wallpaper.clone();
				self.edited.wallpaper_raw = self.defaults.wallpaper_raw.clone();
			}
			Key::Theme => {
				self.edited.theme = self.defaults.theme.clone();
				self.adopt_theme();
			}
			Key::ThemeMode => {
				self.edited.theme_mode = self.defaults.theme_mode.clone();
				self.adopt_theme();
			}
			Key::FontFamily => self.edited.font_family = self.defaults.font_family.clone(),
			Key::LinkOpenCommand => {
				self.edited.hyperlink_open_command = self.defaults.hyperlink_open_command.clone();
			}
			Key::StartupDirectory => {
				self.edited.startup_directory = self.defaults.startup_directory.clone();
			}
			Key::ColBg
			| Key::ColFg
			| Key::ColCursor
			| Key::ColHighlight
			| Key::ColFocus
			| Key::ColGutter
			| Key::ColMenuBg
			| Key::ColMenuFg
			| Key::ColDialogBg
			| Key::ColDialogFg
			| Key::ColScrollbarThumb
			| Key::ColScrollbarTrough => {
				let color = self.default_col(key);
				self.set_col(key, color);
			}
			// direct: set_f32 would also clear use_system_font_size (its "explicit
			// size" side effect), which a revert must not do
			Key::FontSize => self.edited.font_size = self.defaults.font_size,
			Key::None => {}
			_ => {
				let value = self.default_f32(key);
				self.set_f32(key, value);
			}
		}
		for cfg_key in ui().settings_of(key) {
			if !self.reverted.contains(cfg_key) {
				self.reverted.push(cfg_key);
			}
		}
	}
	// Config keys reverted since the last Apply (cleared by taking them).
	pub fn take_reverted(&mut self) -> Vec<&'static str> {
		std::mem::take(&mut self.reverted)
	}

	fn fmt_val(&self, key: Key, int: bool) -> String {
		let value = self.get_f32(key);
		if int {
			format!("{}", value.round() as i64)
		} else {
			format!("{value:.2}")
		}
	}

	// `measure` gives a string's rendered width in the UI font (for placing the
	// caret at the clicked position inside a text field).
	fn mouse_down_dip(&mut self, x: f32, y: f32, measure: &mut impl FnMut(&str) -> f32) -> Action {
		// double/triple-click detection (word / whole-value selection in fields)
		let now = std::time::Instant::now();
		self.click_streak = match self.last_click {
			Some((t, lx, ly))
				if now.duration_since(t).as_millis() < 400
					&& (x - lx).abs() < 6.0
					&& (y - ly).abs() < 6.0 =>
			{
				self.click_streak.saturating_add(1)
			}
			_ => 1,
		};
		self.last_click = Some((now, x, y));
		// the prompt box is modal over the panel: it takes the click either way
		if self.prompt.is_some() {
			if self.emenu.is_some() {
				let hit = (0..EDIT_MENU.len()).find(|&k| self.em_item_rect(k).contains(x, y));
				let cmd = hit.filter(|&k| self.em_enabled(k)).map(|k| EDIT_MENU[k].1);
				self.emenu = None;
				return cmd.map_or(Action::None, Action::Edit);
			}
			self.prompt_mouse_down(x, y, measure);
			return Action::None;
		}
		// an open field context menu captures the click: an enabled item fires its
		// command (clipboard glue in dialog.rs), anywhere else just dismisses
		if self.emenu.is_some() {
			let hit = (0..EDIT_MENU.len()).find(|&k| self.em_item_rect(k).contains(x, y));
			let cmd = hit.filter(|&k| self.em_enabled(k)).map(|k| EDIT_MENU[k].1);
			self.emenu = None;
			return cmd.map_or(Action::None, Action::Edit);
		}
		// an open dropdown captures the click: on an option -> pick it, anywhere
		// else -> just close (a click-away dismiss, consumed either way)
		if let Some(oi) = self.open.take() {
			let n = self.dd_options(oi).len();
			for k in 0..n {
				if self.dd_item_rect(oi, n, k).contains(x, y) {
					self.set_radio(self.specs[oi].key, k);
					break;
				}
			}
			return Action::None;
		}
		// footer buttons arm on press (drawn pressed) and fire on release, so a
		// press-drag-off cancels - and the user gets click feedback
		for (btn_idx, (_, r, _)) in self.buttons().into_iter().enumerate() {
			if r.contains(x, y) {
				self.pressed = Some(btn_idx);
				return Action::None;
			}
		}
		// click outside the panel cancels
		if !self.rect.contains(x, y) {
			return Action::Cancel;
		}
		// a click inside the field being edited keeps the edit (caret/selection
		// handling below); anywhere else commits it
		let keep_edit = self
			.edit
			.as_ref()
			.is_some_and(|e| self.field_rect(e.row).is_some_and(|r| r.contains(x, y)));
		if !keep_edit {
			self.commit_edit();
		}
		// tab bar
		for k in 0..self.tab_ws.len() {
			if self.tab_rect(k).contains(x, y) {
				if k != self.tab {
					self.tab = k;
					self.scroll = 0.0;
					self.drag = None;
					self.focus = None; // mouse mode; Tab re-establishes focus
				}
				return Action::None;
			}
		}
		// scrollbar: drag the thumb, or jump-and-drag from the track
		if let Some(thumb) = self.thumb() {
			if thumb.contains(x, y) {
				self.drag_thumb = Some(y - thumb.y);
				return Action::None;
			}
			let vp = self.viewport();
			if x >= thumb.x && x <= thumb.x + thumb.w && y >= vp.y && y <= vp.y + vp.h {
				let frac = ((y - vp.y - thumb.h / 2.0) / (vp.h - thumb.h).max(1.0)).clamp(0.0, 1.0);
				self.scroll = frac * self.max_scroll();
				self.drag_thumb = Some(thumb.h / 2.0);
				return Action::None;
			}
		}
		// rows: only within the (possibly scrolled) viewport, only the active tab
		let vp = self.viewport();
		if y < vp.y || y > vp.y + vp.h {
			return Action::None;
		}
		for i in 0..self.specs.len() {
			if self.specs[i].tab != self.tab || Self::header_is_tab_title(&self.specs[i]) {
				continue;
			}
			// revert-to-default icon (any control row; inert when already default)
			if self.has_revert(i) && self.revert_box(i).contains(x, y) {
				if !self.row_is_default(i) {
					self.row_revert(i);
				}
				return Action::None;
			}
			match self.specs[i].kind {
				Kind::Slider { .. } => {
					if self.disabled(self.specs[i].key) {
						continue; // grayed-out slider ignores clicks
					}
					// click the numeric field -> edit the value, caret at the click
					let val_box = self.valbox(i);
					if val_box.contains(x, y) {
						self.field_click(i, (i, 1), val_box, x, measure);
						return Action::None;
					}
					let track = self.track(i);
					let hit = x >= track.x - 8.0
						&& x <= track.x + track.w + 8.0
						&& (y - (track.y + track.h / 2.0)).abs() <= 12.0;
					if hit {
						self.focus = Some(Focus::Row(i, 0));
						self.drag = Some(i);
						self.drag_to(x);
						return Action::None;
					}
				}
				Kind::Color => {
					// swatch click opens the hex with the value selected (type to
					// replace); hex-box click places the caret
					if self.swatch(i).contains(x, y) {
						self.focus = Some(Focus::Row(i, 0));
						self.open_edit(i, true);
						return Action::None;
					}
					let hex_box = self.hexbox(i);
					if hex_box.contains(x, y) {
						self.field_click(i, (i, 0), hex_box, x, measure);
						return Action::None;
					}
				}
				Kind::Text => {
					let text_box = self.textbox(i);
					if text_box.contains(x, y) {
						self.field_click(i, (i, 0), text_box, x, measure);
						return Action::None;
					}
				}
				Kind::Toggle => {
					if self.checkbox(i).contains(x, y) {
						if self.disabled(self.specs[i].key) {
							continue; // grayed checkbox ignores clicks
						}
						let key = self.specs[i].key;
						self.focus = Some(Focus::Row(i, 0));
						self.set_toggle(key, !self.get_toggle(key));
						return Action::None;
					}
				}
				Kind::Dual { keys, .. } => {
					// hit either checkbox (or its label span, out to the next pitch)
					for p in 0u16..2 {
						let bx = self.dual_box(i, p);
						if x >= bx.x
							&& x <= bx.x + self.dual_pitch() - 8.0
							&& (y - (bx.y + bx.h / 2.0)).abs() <= bx.h / 2.0 + 4.0
						{
							if self.disabled(keys[p as usize]) {
								continue; // grayed checkbox ignores clicks
							}
							let key = keys[p as usize];
							self.focus = Some(Focus::Row(i, p));
							self.set_toggle(key, !self.get_toggle(key));
							return Action::None;
						}
					}
				}
				Kind::Radio(options) => {
					for k in 0..options.len() {
						let radio_rect = self.radio_box(i, k);
						// click the box or its label
						if x >= radio_rect.x
							&& x <= radio_rect.x + self.radio_pitch() - 8.0
							&& (y - (radio_rect.y + radio_rect.h / 2.0)).abs()
								<= radio_rect.h / 2.0 + 4.0
						{
							self.focus = Some(Focus::Row(i, 0));
							self.set_radio(self.specs[i].key, k);
							return Action::None;
						}
					}
				}
				Kind::Dropdown(_) => {
					if self.disabled(self.specs[i].key) {
						continue;
					}
					if self.dd_box(i).contains(x, y) {
						self.dd_open(i);
						return Action::None;
					}
				}
				// same press-arm / release-fire as the footer buttons, so a
				// press-drag-off cancels and the press is visible
				Kind::Buttons(captions) => {
					for p in 0..captions.len() as u16 {
						if self.row_btn_rect(i, p).contains(x, y) {
							if self.part_disabled(i, p) {
								continue;
							}
							self.focus = Some(Focus::Row(i, p));
							self.pressed_row = Some((i, p));
							return Action::None;
						}
					}
				}
				Kind::ShellList => {
					if self.shell_mouse_down(i, x, y, measure) {
						return Action::None;
					}
				}
				Kind::Header(_) => {}
			}
		}
		Action::None
	}

	// A click somewhere in the grid. Returns whether it landed on something.
	// The move and remove buttons arm on press and fire on release, the same way
	// the footer and the theme buttons do, so a press that drifts off cancels.
	fn shell_mouse_down(
		&mut self,
		i: usize,
		x: f32,
		y: f32,
		measure: &mut impl FnMut(&str) -> f32,
	) -> bool {
		// The grip first, and outside the part walk: it is not a keyboard stop,
		// so there is no part index that names it.
		for k in 0..self.edited.shells.len() {
			if self.shell_grip_box(i, k).contains(x, y) {
				self.commit_edit();
				self.shell_drag = Some(ShellDrag {
					at: k,
					grab_dy: y - self.shell_line_y(i, k),
				});
				return true;
			}
		}
		for part in 0..self.parts_of(i) {
			if !self.shell_stop_rect(i, part).contains(x, y) {
				continue;
			}
			match shell_stop(part, self.edited.shells.len()) {
				ShellStop::Entry(k, ShellPart::Name) => {
					let field = self.shell_name_box(i, k);
					let row = shell_field_row(k, false);
					self.field_click(row, (i, part), field, x, measure);
				}
				ShellStop::Entry(k, ShellPart::Command) => {
					let field = self.shell_cmd_box(i, k);
					let row = shell_field_row(k, true);
					self.field_click(row, (i, part), field, x, measure);
				}
				ShellStop::Entry(k, ShellPart::Active) => {
					self.focus = Some(Focus::Row(i, part));
					if let Some(entry) = self.edited.shells.get_mut(k) {
						entry.active = !entry.active;
					}
				}
				// Add and Remove arm on press and fire on release, the same way
				// the footer and theme buttons do, so a press that drifts off
				// cancels - which matters most for the one that deletes a line.
				ShellStop::Add | ShellStop::Entry(_, ShellPart::Remove) => {
					self.focus = Some(Focus::Row(i, part));
					self.pressed_row = Some((i, part));
				}
			}
			return true;
		}
		false
	}

	// The editable text box of row i, by kind (None for non-field rows).
	fn field_rect(&self, i: usize) -> Option<Rect> {
		if i == PROMPT_ROW {
			return self.prompt_field_rect();
		}
		if let Some((k, command)) = shell_field_of(i) {
			let grid = self.shell_row()?;
			return Some(if command {
				self.shell_cmd_box(grid, k)
			} else {
				self.shell_name_box(grid, k)
			});
		}
		match self.specs[i].kind {
			Kind::Slider { .. } => Some(self.valbox(i)),
			Kind::Color => Some(self.hexbox(i)),
			Kind::Text => Some(self.textbox(i)),
			_ => None,
		}
	}
	// Click into an editable field: caret at the click; Shift extends the
	// selection; double-click selects the word, triple selects all; a plain
	// click starts a drag-selection.
	// `row` is the edit's own index - a spec row, or one of the pseudo rows the
	// grid's fields use - and `focus` is the spec row and part the ring goes on.
	// For every field but the grid's they are the same row; for those two they
	// cannot be, since no spec row draws them.
	fn field_click(
		&mut self,
		row: usize,
		focus: (usize, u16),
		field: Rect,
		x: f32,
		measure: &mut impl FnMut(&str) -> f32,
	) {
		let i = row;
		let same_row = self.edit.as_ref().is_some_and(|e| e.row == i);
		if !same_row {
			self.open_edit(i, false);
		}
		self.select_all_on_up = false;
		let (shift, streak) = (self.shift, self.click_streak);
		self.focus = Some(Focus::Row(focus.0, focus.1));
		let Some(edit) = &mut self.edit else { return };
		let cur = caret_from_click(
			&edit.buf,
			x - (field.x + lay().field_pad) + edit.view,
			measure,
		);
		if shift && same_row {
			if edit.sel.is_none() {
				edit.sel = Some(edit.cur);
			}
			edit.cur = cur;
			return;
		}
		match streak {
			2 => {
				let (a, b) = word_at(&edit.buf, cur);
				edit.sel = (a != b).then_some(a);
				edit.cur = b;
			}
			n if n >= 3 => {
				edit.sel = (!edit.buf.is_empty()).then_some(0);
				edit.cur = edit.buf.len();
			}
			_ => {
				edit.cur = cur;
				edit.sel = None;
				self.edit_drag = Some(i);
				// fresh entry (not repositioning a caret in the field already open):
				// select all on release, unless the click turns into a drag-select
				self.select_all_on_up = !same_row;
			}
		}
	}

	// --- field context menu (right-click / Menu key inside an editable field) ---
	fn em_item_h(&self) -> f32 {
		self.dd_item_h()
	}
	fn em_rect(&self) -> Rect {
		let Some(menu) = &self.emenu else {
			return Rect {
				x: 0.0,
				y: 0.0,
				w: 0.0,
				h: 0.0,
			};
		};
		let w = lay().edit_menu_width * self.ui_scale();
		let h = EDIT_MENU.len() as f32 * self.em_item_h();
		// clamp into the panel; flip upward when it would spill past the bottom
		let x = menu
			.x
			.min(self.rect.x + self.rect.w - w - 2.0)
			.max(self.rect.x);
		let y = if menu.y + h > self.rect.y + self.rect.h - 2.0 {
			(menu.y - h).max(self.rect.y)
		} else {
			menu.y
		};
		Rect { x, y, w, h }
	}
	fn em_item_rect(&self, k: usize) -> Rect {
		let r = self.em_rect();
		Rect {
			x: r.x,
			y: r.y + k as f32 * self.em_item_h(),
			w: r.w,
			h: self.em_item_h(),
		}
	}
	fn em_enabled(&self, k: usize) -> bool {
		let edit = self.edit.as_ref();
		match EDIT_MENU[k].1 {
			EditCmd::Cut | EditCmd::Copy | EditCmd::Delete => {
				edit.is_some_and(|e| e.sel_range().is_some())
			}
			EditCmd::Paste => self.emenu.as_ref().is_some_and(|m| m.paste_ok),
			EditCmd::SelectAll => edit.is_some_and(|e| !e.buf.is_empty()),
		}
	}
	fn dismiss_menu(&mut self) {
		self.emenu = None;
	}
	// Right-click in an editable field: open (or keep) the edit, place the caret
	// at the click unless it lands inside the selection (standard), pop the menu.
	fn mouse_right_dip(
		&mut self,
		x: f32,
		y: f32,
		paste_ok: bool,
		measure: &mut impl FnMut(&str) -> f32,
	) {
		self.mouse = (x, y);
		self.emenu = None;
		self.open = None;
		// The prompt takes every click while it is up. Its own field still gets the
		// menu; anywhere else does nothing, or the click would open an edit on the
		// row behind the box and OK would then save under that row's value.
		if self.prompt.is_some() {
			if let Some(field) = self.prompt_field_rect().filter(|f| f.contains(x, y)) {
				self.pop_field_menu(field, x, y, paste_ok, measure);
			}
			return;
		}
		let vp = self.viewport();
		if y < vp.y || y > vp.y + vp.h {
			return;
		}
		for i in 0..self.specs.len() {
			if self.specs[i].tab != self.tab || Self::header_is_tab_title(&self.specs[i]) {
				continue;
			}
			let Some(field) = self.field_rect(i) else {
				continue;
			};
			if !field.contains(x, y) {
				continue;
			}
			if self.disabled(self.specs[i].key) {
				return;
			}
			let same_row = self.edit.as_ref().is_some_and(|e| e.row == i);
			if !same_row {
				self.commit_edit();
				self.open_edit(i, false);
			}
			let part = u16::from(matches!(self.specs[i].kind, Kind::Slider { .. }));
			self.focus = Some(Focus::Row(i, part));
			self.pop_field_menu(field, x, y, paste_ok, measure);
			return;
		}
	}
	// Caret placement plus the menu itself, shared by the panel rows and the theme
	// prompt's own field. A click inside an existing selection leaves it alone, so
	// the menu can act on it (standard).
	fn pop_field_menu(
		&mut self,
		field: Rect,
		x: f32,
		y: f32,
		paste_ok: bool,
		measure: &mut impl FnMut(&str) -> f32,
	) {
		if let Some(edit) = &mut self.edit {
			let rel_x = x - (field.x + lay().field_pad) + edit.view;
			let cur = caret_from_click(&edit.buf, rel_x, measure);
			let inside = edit.sel_range().is_some_and(|(a, b)| cur >= a && cur <= b);
			if !inside {
				edit.cur = cur;
				edit.sel = None;
			}
		}
		self.emenu = Some(EMenu {
			x,
			y,
			hover: None,
			paste_ok,
		});
	}
	// Keyboard Menu key: pop the context menu at the caret of the active edit.
	fn menu_key_dip(&mut self, paste_ok: bool, measure: &mut impl FnMut(&str) -> f32) {
		let Some(edit) = &self.edit else { return };
		let Some(field) = self.field_rect(edit.row) else {
			return;
		};
		let cx = (field.x + lay().field_pad + measure(&edit.buf[..edit.cur]) - edit.view)
			.clamp(field.x, field.x + field.w);
		self.emenu = Some(EMenu {
			x: cx,
			y: field.y + field.h,
			hover: Some(0),
			paste_ok,
		});
	}

	// Per-frame upkeep of the active field edit, with real frame time: eases the
	// horizontal view (caret kept visible with a lookahead margin so several
	// characters show ahead of travel; lay().caret_pad keeps the caret clear of the
	// right edge at end-of-text), eases the caret x, advances the blink, and
	// replays a drag past the box edges (edge autoscroll). Returns the wake the
	// caller should schedule: fast while something moves, blink-rate while an
	// idle edit pulses, None when there's nothing to animate.
	fn animate_dip(&mut self, dt: f32, measure: &mut impl FnMut(&str) -> f32) -> Option<u64> {
		if self.edit_drag.is_some() {
			let (mx, my) = self.mouse;
			self.mouse_move_dip(mx, my, measure);
		}
		let row = self.edit.as_ref().map(|e| e.row)?;
		let field = self.field_rect(row)?;
		let inner_w = (field.w - 2.0 * lay().field_pad).max(1.0);
		let ahead = (lay().view_ahead * self.ui_scale()).min(inner_w / 3.0);
		let (caret_x, text_w, sig) = {
			let edit = self.edit.as_ref().unwrap(); // Some: row extracted above
			(
				measure(&edit.buf[..edit.cur]),
				measure(&edit.buf),
				(edit.cur, edit.sel, edit.buf.len()),
			)
		};
		let dragging = self.edit_drag.is_some();
		let edit = self.edit.as_mut().unwrap(); // Some: row extracted above
		if sig == edit.last_sig {
			edit.blink_t += dt;
		} else {
			edit.last_sig = sig;
			edit.blink_t = 0.0; // activity holds the caret solid
		}
		// target view: keep the caret in sight with the margin; the clamp snaps
		// the margin away at the true ends so 0 / end-of-text sit flush
		let max_view = (text_w + lay().caret_pad - inner_w).max(0.0);
		let mut to = edit.view_to;
		if caret_x < to + ahead {
			to = caret_x - ahead;
		}
		if caret_x > to + inner_w - ahead {
			to = caret_x - (inner_w - ahead);
		}
		edit.view_to = to.clamp(0.0, max_view);
		// exponential ease toward the targets (same idiom as the pane scroll)
		edit.view += (edit.view_to - edit.view) * (1.0 - (-dt / 0.05).exp());
		let cv = edit.caret_vis.get_or_insert(caret_x);
		*cv += (caret_x - *cv) * (1.0 - (-dt / 0.04).exp());
		let moving = (edit.view_to - edit.view).abs() > 0.25 || (caret_x - *cv).abs() > 0.25;
		if !moving {
			edit.view = edit.view_to;
			*cv = caret_x;
		}
		Some(if moving || dragging { 8 } else { 33 })
	}

	fn mouse_move_dip(&mut self, x: f32, y: f32, measure: &mut impl FnMut(&str) -> f32) {
		self.mouse = (x, y);
		// a line being dragged by its grip, reordered as it travels
		if let Some(drag) = &self.shell_drag {
			let (at, grab_dy) = (drag.at, drag.grab_dy);
			if let Some(i) = self.shell_row() {
				let want = self.shell_drop_at(i, y, grab_dy);
				if want != at {
					self.shell_move_to(at, want);
					if let Some(drag) = &mut self.shell_drag {
						drag.at = want;
					}
				}
			}
			return;
		}
		// open field context menu: track the hovered item
		if self.emenu.is_some() {
			let hover = (0..EDIT_MENU.len()).find(|&k| self.em_item_rect(k).contains(x, y));
			if let Some(menu) = &mut self.emenu {
				menu.hover = hover.or(menu.hover);
			}
			return;
		}
		// drag-selection inside an editable field (a drag past the box edges keeps
		// selecting: `animate` replays this pos while the view crawls)
		if let Some(row) = self.edit_drag {
			if let Some(field) = self.field_rect(row) {
				let moved = if let Some(edit) = &mut self.edit {
					let rel_x = x - (field.x + lay().field_pad) + edit.view;
					let cur = caret_from_click(&edit.buf, rel_x, measure);
					if cur == edit.cur {
						false
					} else {
						if edit.sel.is_none() {
							edit.sel = Some(edit.cur);
						}
						edit.cur = cur;
						true
					}
				} else {
					false
				};
				// a click that turned into a drag keeps the dragged range, not select-all
				if moved {
					self.select_all_on_up = false;
				}
			}
			return;
		}
		if let Some(oi) = self.open {
			let n = self.dd_options(oi).len();
			for k in 0..n {
				if self.dd_item_rect(oi, n, k).contains(x, y) {
					self.pending = k;
					break;
				}
			}
			return;
		}
		if let Some(grab) = self.drag_thumb {
			let vp = self.viewport();
			let thumb_h = self.thumb().map_or(lay().scrollbar_thumb_min, |t| t.h);
			let frac = ((y - grab - vp.y) / (vp.h - thumb_h).max(1.0)).clamp(0.0, 1.0);
			self.scroll = frac * self.max_scroll();
			return;
		}
		if self.drag.is_some() {
			self.drag_to(x);
		}
	}
	// Release: end any slider/thumb drag, and fire an armed button's action only if
	// the cursor is still over it (a press that drifted off cancels).
	fn mouse_up_dip(&mut self, x: f32, y: f32) -> Action {
		// A release that ends a grip drag is not a click on anything: the list was
		// reordered as the pointer moved, and there is nothing left to fire.
		if self.shell_drag.take().is_some() {
			return Action::None;
		}
		self.drag = None;
		self.drag_thumb = None;
		self.edit_drag = None;
		// an empty drag-selection collapses back to a plain caret
		if let Some(edit) = &mut self.edit {
			if edit.sel == Some(edit.cur) {
				edit.sel = None;
			}
		}
		// a fresh single-click field entry that never became a drag selects all, so
		// the next keystroke replaces the value (standard field entry)
		if std::mem::take(&mut self.select_all_on_up) {
			if let Some(edit) = &mut self.edit {
				if edit.sel.is_none() && !edit.buf.is_empty() {
					edit.sel = Some(0);
					edit.cur = edit.buf.len();
				}
			}
		}
		if let Some(btn_idx) = self.pressed.take() {
			let (action, r, _) = self.buttons()[btn_idx];
			if r.contains(x, y) {
				return action;
			}
		}
		if let Some((i, p)) = self.pressed_row.take() {
			if matches!(self.specs[i].kind, Kind::ShellList) {
				if self.shell_stop_rect(i, p).contains(x, y) {
					self.shell_activate(i, p);
				}
			} else if self.row_btn_rect(i, p).contains(x, y) {
				self.theme_action(ThemeBtn::of(p));
			}
		}
		Action::None
	}

	fn drag_to(&mut self, x: f32) {
		let Some(i) = self.drag else { return };
		let Kind::Slider { min, max, int } = self.specs[i].kind else {
			return;
		};
		let track = self.track(i);
		let frac = ((x - track.x) / track.w).clamp(0.0, 1.0);
		let mut value = min + frac * (max - min);
		if int {
			value = value.round();
		}
		let key = self.specs[i].key;
		self.set_f32(key, value);
	}

	pub fn char_input(&mut self, c: char) {
		self.dismiss_menu();
		if self.ctrl {
			return; // Ctrl+letter is a shortcut (copy/paste/...), never types
		}
		// typing into a keyboard-focused (but not-yet-open) field opens it with
		// the value selected, so the keystroke replaces it (standard field entry).
		// The delete confirmation has no field of its own, so without the prompt
		// test this would open the row sitting behind the box and edit that.
		if self.edit.is_none() {
			let (Some(Focus::Row(i, _)), None) = (self.focus, self.prompt.as_ref()) else {
				return;
			};
			match self.specs[i].kind {
				Kind::Text | Kind::Color | Kind::Slider { .. } => self.open_edit(i, true),
				_ => return,
			}
		}
		if self.insert_char(c) {
			self.reparse_edit();
		}
	}
	// One char through the field's own validation (replacing any selection).
	// Returns whether the buffer changed; caller reparses.
	fn insert_char(&mut self, c: char) -> bool {
		let Some(edit) = &mut self.edit else {
			return false;
		};
		let sel_len = edit.sel_range().map_or(0, |(a, b)| b - a);
		// where the char would land once any selection is gone
		let landing = edit.sel_range().map_or(edit.cur, |(a, _)| a);
		// a theme name, a shell's title or its command line: any ordinary
		// character, within a sane length (a command may be a long path)
		if edit.row >= PSEUDO_ROW {
			let cap = if shell_field_of(edit.row).is_some_and(|(_, cmd)| cmd) {
				512
			} else {
				64
			};
			if c.is_control() || edit.buf.len() - sel_len >= cap {
				return false;
			}
			edit.remove_selection();
			edit.buf.insert(edit.cur, c);
			edit.cur += c.len_utf8();
			return true;
		}
		let ok = match self.specs[edit.row].kind {
			Kind::Color => {
				(c == '#' || c.is_ascii_hexdigit())
					&& edit.buf.len() - sel_len < 7
					// '#' only makes sense up front
					&& (c != '#' || landing == 0)
			}
			Kind::Text => !c.is_control() && edit.buf.len() - sel_len < 256,
			// numeric slider field: digits always; one '.' only for float sliders
			Kind::Slider { int, .. } => {
				let kept = match edit.sel_range() {
					Some((a, b)) => format!("{}{}", &edit.buf[..a], &edit.buf[b..]),
					None => edit.buf.clone(),
				};
				let dot_ok = !int && c == '.' && !kept.contains('.');
				(c.is_ascii_digit() || dot_ok) && kept.len() < 8
			}
			_ => false,
		};
		if !ok {
			return false;
		}
		edit.remove_selection();
		edit.buf.insert(edit.cur, c);
		edit.cur += c.len_utf8();
		true
	}
	// Paste: run the text through the same per-field validation, one char at a
	// time (invalid chars are dropped, length caps hold).
	pub fn insert_str(&mut self, text: &str) {
		let mut changed = false;
		for c in text.chars() {
			changed |= self.insert_char(c);
		}
		if changed {
			self.reparse_edit();
		}
	}
	pub fn select_all(&mut self) {
		// Ctrl+A on a focused-but-closed field opens it first - but not through an
		// open prompt, whose delete variant leaves no edit for this to find
		if self.edit.is_none() && self.prompt.is_none() {
			if let Some(Focus::Row(i, _)) = self.focus {
				if matches!(
					self.specs[i].kind,
					Kind::Text | Kind::Color | Kind::Slider { .. }
				) {
					self.open_edit(i, true);
				}
			}
			return;
		}
		if let Some(edit) = &mut self.edit {
			edit.sel = (!edit.buf.is_empty()).then_some(0);
			edit.cur = edit.buf.len();
		}
	}
	pub fn selected_text(&self) -> Option<String> {
		let edit = self.edit.as_ref()?;
		let (a, b) = edit.sel_range()?;
		Some(edit.buf[a..b].to_string())
	}
	pub fn delete_selection(&mut self) {
		if let Some(edit) = &mut self.edit {
			if edit.remove_selection() {
				self.reparse_edit();
			}
		}
	}
	pub fn backspace(&mut self) {
		self.dismiss_menu();
		let ctrl = self.ctrl;
		if let Some(edit) = &mut self.edit {
			if edit.remove_selection() {
				self.reparse_edit();
				return;
			}
			if edit.cur > 0 {
				let prev = if ctrl {
					word_left(&edit.buf, edit.cur)
				} else {
					prev_boundary(&edit.buf, edit.cur)
				};
				edit.buf.replace_range(prev..edit.cur, "");
				edit.cur = prev;
				self.reparse_edit();
			}
		}
	}
	pub fn delete_forward(&mut self) {
		self.dismiss_menu();
		let ctrl = self.ctrl;
		if let Some(edit) = &mut self.edit {
			if edit.remove_selection() {
				self.reparse_edit();
				return;
			}
			if edit.cur < edit.buf.len() {
				let next = if ctrl {
					word_right(&edit.buf, edit.cur)
				} else {
					next_boundary(&edit.buf, edit.cur)
				};
				edit.buf.replace_range(edit.cur..next, "");
				self.reparse_edit();
			}
		}
	}
	// Caret movement within the focused field (Left/Right/Home/End). Shift
	// extends the selection; Ctrl jumps by words; a plain move collapses any
	// selection to its edge (standard).
	fn move_caret(&mut self, to: usize) {
		let (shift, _) = (self.shift, self.ctrl);
		if let Some(edit) = &mut self.edit {
			if shift {
				if edit.sel.is_none() {
					edit.sel = Some(edit.cur);
				}
			} else {
				edit.sel = None;
			}
			edit.cur = to;
			// an emptied extension drops the anchor so a lone Shift press is inert
			if edit.sel == Some(edit.cur) {
				edit.sel = None;
			}
		}
	}
	pub fn cursor_left(&mut self) {
		let Some(edit) = &self.edit else { return };
		// plain Left with a selection collapses to its start
		if !self.shift {
			if let Some((a, _)) = edit.sel_range() {
				self.move_caret(a);
				return;
			}
		}
		let to = if self.ctrl {
			word_left(&edit.buf, edit.cur)
		} else {
			prev_boundary(&edit.buf, edit.cur)
		};
		self.move_caret(to);
	}
	pub fn cursor_right(&mut self) {
		let Some(edit) = &self.edit else { return };
		if !self.shift {
			if let Some((_, b)) = edit.sel_range() {
				self.move_caret(b);
				return;
			}
		}
		let to = if self.ctrl {
			word_right(&edit.buf, edit.cur)
		} else {
			next_boundary(&edit.buf, edit.cur)
		};
		self.move_caret(to);
	}
	pub fn cursor_home(&mut self) {
		if self.edit.is_some() {
			self.move_caret(0);
		}
	}
	pub fn cursor_end(&mut self) {
		let Some(edit) = &self.edit else { return };
		let end = edit.buf.len();
		self.move_caret(end);
	}
	// live-apply the in-progress edit (hex color, or background-image path)
	fn reparse_edit(&mut self) {
		let Some((i, buf)) = self.edit.as_ref().map(|edit| (edit.row, edit.buf.clone())) else {
			return;
		};
		if i == PROMPT_ROW {
			// nothing to apply yet - but the name just changed, so whatever OK
			// last objected to may no longer be true
			if let Some(prompt) = self.prompt.as_mut() {
				prompt.warn = None;
			}
			return;
		}
		if let Some((k, command)) = shell_field_of(i) {
			let Some(entry) = self.edited.shells.get_mut(k) else {
				return;
			};
			if command {
				// The command is REQUIRED, and this is where that is enforced:
				// a blank buffer simply isn't written, so committing an emptied
				// field leaves the stored command standing and the box shows it
				// again. (An entry that never got one is dropped on the way out
				// of the dialog - see `app::apply_settings_values`.)
				if !buf.trim().is_empty() {
					entry.command = buf.trim().to_string();
				}
			} else {
				entry.title = buf.trim().to_string();
			}
			return;
		}
		match self.specs[i].kind {
			Kind::Color => {
				if let Some(color) = config::parse_hex(&buf) {
					self.set_col(self.specs[i].key, color);
				}
			}
			Kind::Text => self.set_text(self.specs[i].key, &buf),
			// a valid partial number applies live, clamped to the slider range
			Kind::Slider { min, max, int } => {
				if let Ok(value) = buf.trim().parse::<f32>() {
					let mut value = value.clamp(min, max);
					if int {
						value = value.round();
					}
					self.set_f32(self.specs[i].key, value);
				}
			}
			_ => {}
		}
	}
	fn commit_edit(&mut self) {
		self.edit = None;
		self.emenu = None;
	}

	// Esc cancels the dialog; Enter commits an active hex edit (or OK otherwise).
	pub fn key_escape(&mut self) -> Action {
		// Esc closes the field context menu / dropdown popup first, not the dialog
		if self.emenu.take().is_some() || self.open.take().is_some() {
			Action::None
		} else if self.prompt.is_some() {
			self.prompt_close(); // then the prompt box, still not the dialog
			Action::None
		} else if self.edit.is_some() {
			self.edit = None;
			Action::None
		} else {
			Action::Cancel
		}
	}
	pub fn key_enter(&mut self) -> Action {
		if self.emenu.is_some() {
			// fire the highlighted (enabled) menu item
			let cmd = self
				.emenu
				.as_ref()
				.and_then(|m| m.hover)
				.filter(|&k| self.em_enabled(k))
				.map(|k| EDIT_MENU[k].1);
			self.emenu = None;
			return cmd.map_or(Action::None, Action::Edit);
		}
		// Enter in the prompt box is its OK, unless Cancel is the focused button
		if let Some(prompt) = self.prompt.as_ref() {
			if prompt.focus == PromptFocus::Cancel {
				self.prompt_close();
			} else {
				self.prompt_accept();
			}
			return Action::None;
		}
		if self.open.is_some() {
			self.dd_commit();
			Action::None
		} else if self.edit.is_some() {
			self.commit_edit();
			Action::None
		} else if let Some(Focus::Button(b)) = self.focus {
			self.buttons()[b].0 // a focused footer button
		} else if let Some(Focus::Row(i, p)) = self.focus {
			// a focused push-button is what Enter presses, not the dialog's OK -
			// and every stop in the shells grid is one of those or a field
			match self.specs[i].kind {
				Kind::Buttons(_) => {
					self.theme_action(ThemeBtn::of(p));
					Action::None
				}
				Kind::ShellList => {
					self.shell_activate(i, p);
					Action::None
				}
				_ => Action::Ok,
			}
		} else {
			Action::Ok
		}
	}

	// caret line (and selection highlight) inside a focused field, at the
	// measured prefix widths
	fn caret_quad(
		&self,
		out: &mut Vec<RectInstance>,
		field: Rect,
		measure: &mut impl FnMut(&str) -> f32,
	) {
		let Some(edit) = &self.edit else { return };
		let left = field.x + lay().field_pad - edit.view;
		let (lo, hi) = (field.x + 1.0, field.x + field.w - 1.0);
		// the caret's own x is the eased position (smooth caret travel); other
		// selection edges are exact
		let caret_x = edit
			.caret_vis
			.unwrap_or_else(|| measure(&edit.buf[..edit.cur]));
		if let Some((a, b)) = edit.sel_range() {
			let edge = |i: usize, measure: &mut dyn FnMut(&str) -> f32| {
				if i == edit.cur {
					caret_x
				} else {
					measure(&edit.buf[..i])
				}
			};
			let x1 = (left + edge(a, measure)).clamp(lo, hi);
			let x2 = (left + edge(b, measure)).clamp(lo, hi);
			if x2 > x1 {
				// the text draws after the rects, so it stays legible on top
				out.push(RectInstance {
					pos: [x1, field.y + 2.0],
					size: [x2 - x1, field.h - 4.0],
					color: config::srgb_f32(mix3(dlg().field_bg, dlg().focus_out, 0.45)),
					..Default::default()
				});
			}
		}
		let x = (left + caret_x).clamp(lo, hi - 1.5);
		// smooth blink: fade the bar toward the field bg instead of a hard on/off
		let color = mix3(dlg().field_bg, dlg().focus_out, edit.caret_alpha());
		out.push(RectInstance {
			pos: [x, field.y + 2.0],
			size: [1.5, field.h - 4.0],
			color: config::srgb_f32(color),
			..Default::default()
		});
	}

	// (fixed chrome, scrolled rows): the rows vec is drawn scissored to
	// `viewport()` so scrolled-out controls can't paint over the chrome.
	// `measure` gives the rendered width of a string in the UI font (for the caret).
	fn rects_dip(
		&self,
		line_h: f32,
		mut measure: impl FnMut(&str) -> f32,
	) -> (Vec<RectInstance>, Vec<RectInstance>) {
		let mut fixed = Vec::new();
		let mut out = Vec::new();
		let q = |x: f32, y: f32, w: f32, h: f32, color: [u8; 3]| RectInstance {
			pos: [x, y],
			size: [w, h],
			color: config::srgb_f32(color),
			..Default::default()
		};
		let border = |out: &mut Vec<RectInstance>, r: Rect, thickness: f32, color: [u8; 3]| {
			out.push(q(
				r.x - thickness,
				r.y - thickness,
				r.w + 2.0 * thickness,
				thickness,
				color,
			));
			out.push(q(
				r.x - thickness,
				r.y + r.h,
				r.w + 2.0 * thickness,
				thickness,
				color,
			));
			out.push(q(r.x - thickness, r.y, thickness, r.h, color));
			out.push(q(r.x + r.w, r.y, thickness, r.h, color));
		};
		// panel
		fixed.push(q(
			self.rect.x,
			self.rect.y,
			self.rect.w,
			self.rect.h,
			dlg().panel_bg,
		));
		border(&mut fixed, self.rect, 1.0, dlg().panel_border);
		// The tab strip: a recessed gutter closed off by a rule, with the tabs
		// standing on that rule. The current one is a lighter gray rather than an
		// accent - it says "you are here", which is not the same job as the
		// highlight color's "look at this".
		let gut = self.gutter_rect();
		fixed.push(q(gut.x, gut.y, gut.w, gut.h, dlg().gutter));
		fixed.push(q(gut.x, gut.y + gut.h, gut.w, 1.0, dlg().panel_border));
		for k in 0..self.tab_ws.len() {
			let r = self.tab_rect(k);
			let active = k == self.tab;
			fixed.push(q(
				r.x,
				r.y,
				r.w,
				r.h,
				if active { dlg().tab_hl } else { dlg().tab_bg },
			));
		}
		// scrollbar (only when the active tab overflows the viewport)
		if let Some(thumb) = self.thumb() {
			let vp = self.viewport();
			fixed.push(q(thumb.x, vp.y, thumb.w, vp.h, dlg().track));
			fixed.push(q(thumb.x, thumb.y, thumb.w, thumb.h, dlg().handle));
		}

		for i in 0..self.specs.len() {
			if self.specs[i].tab != self.tab || Self::header_is_tab_title(&self.specs[i]) {
				continue;
			}
			match self.specs[i].kind {
				Kind::Slider { min, max, int } => {
					let off = self.disabled(self.specs[i].key);
					let track = self.track(i);
					out.push(q(track.x, track.y, track.w, track.h, dlg().track));
					let value = self.get_f32(self.specs[i].key);
					let frac = ((value - min) / (max - min)).clamp(0.0, 1.0);
					let handle_x = track.x + frac * track.w - 5.0;
					let _ = int;
					out.push(q(
						handle_x,
						track.y - 6.0,
						10.0,
						track.h + 12.0,
						if off {
							dlg().panel_border
						} else {
							dlg().handle
						},
					));
					// editable numeric field
					let val_box = self.valbox(i);
					out.push(q(
						val_box.x,
						val_box.y,
						val_box.w,
						val_box.h,
						dlg().field_bg,
					));
					let focused = matches!(&self.edit, Some(edit) if edit.row == i);
					if !self.ring_on(i, 1) {
						border(
							&mut out,
							val_box,
							1.0,
							if focused && !off {
								dlg().focus_out
							} else {
								dlg().panel_border
							},
						);
					}
					if focused && !off {
						self.caret_quad(&mut out, val_box, &mut measure);
					}
				}
				Kind::Color => {
					let swatch = self.swatch(i);
					out.push(q(
						swatch.x,
						swatch.y,
						swatch.w,
						swatch.h,
						self.get_col(self.specs[i].key),
					));
					border(&mut out, swatch, 1.0, dlg().panel_border);
					let hex_box = self.hexbox(i);
					out.push(q(
						hex_box.x,
						hex_box.y,
						hex_box.w,
						hex_box.h,
						dlg().field_bg,
					));
					let focused = matches!(&self.edit, Some(edit) if edit.row == i);
					if !self.ring_on(i, 0) {
						border(
							&mut out,
							hex_box,
							1.0,
							if focused {
								dlg().focus_out
							} else {
								dlg().panel_border
							},
						);
					}
					if focused {
						self.caret_quad(&mut out, hex_box, &mut measure);
					}
				}
				Kind::Text => {
					let text_box = self.textbox(i);
					out.push(q(
						text_box.x,
						text_box.y,
						text_box.w,
						text_box.h,
						dlg().field_bg,
					));
					let focused = matches!(&self.edit, Some(edit) if edit.row == i);
					if !self.ring_on(i, 0) {
						border(
							&mut out,
							text_box,
							1.0,
							if focused {
								dlg().focus_out
							} else {
								dlg().panel_border
							},
						);
					}
					if focused {
						self.caret_quad(&mut out, text_box, &mut measure);
					}
				}
				Kind::Toggle => {
					let off = self.disabled(self.specs[i].key);
					let check_box = self.checkbox(i);
					out.push(q(
						check_box.x,
						check_box.y,
						check_box.w,
						check_box.h,
						dlg().field_bg,
					));
					border(&mut out, check_box, 1.0, dlg().panel_border);
					// filled inner square when on (the checkmark glyph is drawn in texts)
					if self.get_toggle(self.specs[i].key) {
						out.push(q(
							check_box.x + 4.0,
							check_box.y + 4.0,
							check_box.w - 8.0,
							check_box.h - 8.0,
							if off {
								dlg().panel_border
							} else {
								dlg().handle
							},
						));
					}
				}
				Kind::Dual { keys, .. } => {
					for p in 0u16..2 {
						let off = self.disabled(keys[p as usize]);
						let bx = self.dual_box(i, p);
						out.push(q(bx.x, bx.y, bx.w, bx.h, dlg().field_bg));
						border(&mut out, bx, 1.0, dlg().panel_border);
						if self.get_toggle(keys[p as usize]) {
							out.push(q(
								bx.x + 4.0,
								bx.y + 4.0,
								bx.w - 8.0,
								bx.h - 8.0,
								if off {
									dlg().panel_border
								} else {
									dlg().handle
								},
							));
						}
					}
				}
				Kind::Radio(options) => {
					let sel = self.get_radio(self.specs[i].key);
					for k in 0..options.len() {
						let radio_rect = self.radio_box(i, k);
						out.push(q(
							radio_rect.x,
							radio_rect.y,
							radio_rect.w,
							radio_rect.h,
							dlg().field_bg,
						));
						border(&mut out, radio_rect, 1.0, dlg().panel_border);
						if k == sel {
							out.push(q(
								radio_rect.x + 4.0,
								radio_rect.y + 4.0,
								radio_rect.w - 8.0,
								radio_rect.h - 8.0,
								dlg().handle,
							));
						}
					}
				}
				Kind::Dropdown(_) => {
					// collapsed box only; the open popup is drawn in the overlay pass
					let off = self.disabled(self.specs[i].key);
					let box_r = self.dd_box(i);
					out.push(q(box_r.x, box_r.y, box_r.w, box_r.h, dlg().field_bg));
					if !self.ring_on(i, 0) {
						border(
							&mut out,
							box_r,
							1.0,
							if self.open == Some(i) && !off {
								dlg().focus_out
							} else {
								dlg().panel_border
							},
						);
					}
				}
				// A pressed button fills with the highlight, the same click feedback
				// the footer gives. Its outline is left to the focus ring below,
				// which lands exactly on this box rather than outside it.
				Kind::Buttons(captions) => {
					for p in 0..captions.len() as u16 {
						let r = self.row_btn_rect(i, p);
						let fill = if self.pressed_row == Some((i, p)) {
							dlg().btn_hl
						} else {
							dlg().btn_bg
						};
						out.push(q(r.x, r.y, r.w, r.h, fill));
						if !self.ring_on(i, p) {
							border(&mut out, r, 1.0, dlg().panel_border);
						}
					}
				}
				Kind::ShellList => self.shell_rects(i, &mut out, &q, &border, &mut measure),
				Kind::Header(_) => {
					// faint rule near the bottom of the (tall) heading row, leaving a
					// clear gap below the heading text above it
					let y = self.row_y(i) + self.row_h(&Kind::Header("")) - 8.0;
					let x = self.rect.x + lay().pad;
					out.push(q(
						x,
						y,
						self.rect.w - lay().pad * 2.0,
						1.0,
						dlg().panel_border,
					));
				}
			}
		}
		// keyboard-focus ring around the active control row (scrolls + clips with
		// the rows; a focused button is ringed below, in the fixed chrome).
		if let Some(Focus::Row(fr, fp)) = self.focus {
			if self.specs[fr].tab == self.tab && !matches!(self.specs[fr].kind, Kind::Header(_)) {
				let r = self.focus_ctl_rect(fr, fp);
				let inset = if self.ring_is_the_box(fr, fp) {
					0.0
				} else {
					2.0
				};
				let ring = Rect {
					x: r.x - inset,
					y: r.y - inset,
					w: r.w + inset * 2.0,
					h: r.h + inset * 2.0,
				};
				border(&mut out, ring, 1.0, dlg().focus_out);
			}
		}
		for (btn_idx, (_, r, label)) in self.buttons().into_iter().enumerate() {
			// pressed button fills with the highlight for click feedback
			let fill = if self.pressed == Some(btn_idx) {
				dlg().btn_hl
			} else {
				dlg().btn_bg
			};
			fixed.push(q(r.x, r.y, r.w, r.h, fill));
			let ring = self.focus == Some(Focus::Button(btn_idx));
			// Only the default button (OK) is outlined in the highlight color;
			// the others take the same quiet gray the tabs use, so "this is the
			// one Enter fires" stays a single, readable signal.
			let outline = if ring {
				dlg().focus_out
			} else if btn_idx == 2 {
				dlg().btn_hl
			} else {
				dlg().panel_border
			};
			border(&mut fixed, r, if ring { 2.0 } else { 1.0 }, outline);
			// Alt held: underline the accelerator (the label's first letter). The
			// label is drawn left-aligned at r.x+14; the cap glyph is ~0.55*line_h
			// wide, and its baseline sits near the text bottom.
			if self.alt && !label.is_empty() {
				let tx = r.x + (r.w - measure(label)).max(0.0) / 2.0;
				let ty = r.y + (r.h - line_h) / 2.0 + line_h * 0.82;
				fixed.push(q(tx, ty, line_h * 0.5, 1.5, dlg().text));
			}
		}
		(fixed, out)
	}

	// `line_h` is the rendered text line height (the app's cell_h); rows, hex
	// fields, and buttons center their text vertically against it so alignment
	// holds for any font/size rather than a baked-in guess.
	// The grid's own quads: the two field boxes and the checkbox per entry, the
	// five icon buttons, and the Add button. The arrows are shader-drawn (mode 3
	// with a quarter-turn) for the same reason the tab close mark is - no
	// interface font can be relied on to carry one, and a glyph's own metrics
	// decide where it lands.
	fn shell_rects(
		&self,
		i: usize,
		out: &mut Vec<RectInstance>,
		q: &impl Fn(f32, f32, f32, f32, [u8; 3]) -> RectInstance,
		border: &impl Fn(&mut Vec<RectInstance>, Rect, f32, [u8; 3]),
		measure: &mut impl FnMut(&str) -> f32,
	) {
		let scale = self.ui_scale();
		let mut field = |out: &mut Vec<RectInstance>, r: Rect, row: usize, part: u16| {
			out.push(q(r.x, r.y, r.w, r.h, dlg().field_bg));
			let focused = matches!(&self.edit, Some(edit) if edit.row == row);
			if !self.ring_on(i, part) {
				border(
					out,
					r,
					1.0,
					if focused {
						dlg().focus_out
					} else {
						dlg().panel_border
					},
				);
			}
			if focused {
				self.caret_quad(out, r, measure);
			}
		};
		for k in 0..self.edited.shells.len() {
			let name = self.shell_name_box(i, k);
			field(
				out,
				name,
				shell_field_row(k, false),
				shell_part_index(k, ShellPart::Name),
			);
			let cmd = self.shell_cmd_box(i, k);
			field(
				out,
				cmd,
				shell_field_row(k, true),
				shell_part_index(k, ShellPart::Command),
			);
			// Active checkbox, drawn the way every other checkbox in the dialog is
			let box_r = self.shell_active_box(i, k);
			out.push(q(box_r.x, box_r.y, box_r.w, box_r.h, dlg().field_bg));
			border(out, box_r, 1.0, dlg().panel_border);
			if self.edited.shells.get(k).is_some_and(|e| e.active) {
				let inset = (box_r.w * 0.25).max(3.0);
				out.push(q(
					box_r.x + inset,
					box_r.y + inset,
					box_r.w - inset * 2.0,
					box_r.h - inset * 2.0,
					dlg().handle,
				));
			}
			// The grip: three stacked bars, the shape every reorderable list uses.
			// No box and no border around it - it is a texture to grab, not a
			// button to press, and drawing it as one would invite a click that
			// does nothing. Plain quads, so it costs no shader mode at all.
			let grip = self.shell_grip_box(i, k);
			let held = self.shell_drag.as_ref().is_some_and(|d| d.at == k);
			let bar_h = (1.0 * scale).max(1.0);
			let bar_w = (grip.w * 0.56).max(4.0);
			let bar_x = grip.x + (grip.w - bar_w) / 2.0;
			let pitch = bar_h * 3.0;
			let stack = bar_h + pitch * 2.0;
			let top = grip.y + (grip.h - stack) / 2.0;
			for n in 0..3 {
				out.push(q(
					bar_x,
					top + n as f32 * pitch,
					bar_w,
					bar_h,
					if held { dlg().handle } else { dlg().dim },
				));
			}
			// Remove, between the command and the date. Red, because it is the
			// one control in the whole dialog that destroys something.
			let r = self.shell_remove_box(i, k);
			out.push(q(r.x, r.y, r.w, r.h, dlg().btn_bg));
			if !self.ring_on(i, shell_part_index(k, ShellPart::Remove)) {
				border(out, r, 1.0, dlg().panel_border);
			}
			out.push(RectInstance {
				pos: [r.x, r.y],
				size: [r.w, r.h],
				color: config::srgb_f32(dlg().danger),
				params: [1.0, (r.w * 0.12).max(1.2)],
			});
		}
		let add = self.shell_add_box(i);
		out.push(q(add.x, add.y, add.w, add.h, dlg().btn_bg));
		if !self.ring_on(i, self.parts_of(i).saturating_sub(1)) {
			border(out, add, 1.0, dlg().panel_border);
		}
	}

	fn texts_dip(&self, line_h: f32, mut measure: impl FnMut(&str) -> f32) -> Vec<TextItem> {
		let mut out = Vec::new();
		let mk = |text: String, x: f32, y: f32| TextItem {
			text,
			x,
			y,
			color: dlg().text,
			clip: None,
			bold: false,
			scale: 1.0,
		};
		let row_text_y = |y: f32, h: f32| y + (h - line_h) / 2.0;
		// tab titles - the current one reads at full strength, the rest step back
		for (k, title) in tab_titles().iter().enumerate() {
			let r = self.tab_rect(k);
			out.push(TextItem {
				color: if k == self.tab { dlg().text } else { dlg().dim },
				..mk(
					(*title).into(),
					r.x + lay().tab_pad / 2.0,
					row_text_y(r.y, r.h),
				)
			});
		}
		// row text clips to the scroll viewport so it can't ride over the chrome
		let vp = self.viewport();
		let intersect = |r: Rect| -> Rect {
			let x0 = r.x.max(vp.x);
			let y0 = r.y.max(vp.y);
			let x1 = (r.x + r.w).min(vp.x + vp.w);
			let y1 = (r.y + r.h).min(vp.y + vp.h);
			Rect {
				x: x0,
				y: y0,
				w: (x1 - x0).max(0.0),
				h: (y1 - y0).max(0.0),
			}
		};
		for i in 0..self.specs.len() {
			if self.specs[i].tab != self.tab || Self::header_is_tab_title(&self.specs[i]) {
				continue;
			}
			let ty = row_text_y(self.row_y(i), lay().row_height);
			if let Kind::Header(section) = self.specs[i].kind {
				// heading near the top of the row; the rule sits lower (gap between)
				let hy = self.row_y(i) + 5.0;
				out.push(TextItem {
					bold: true,
					clip: Some(vp),
					..mk(section.into(), self.rect.x + lay().pad, hy)
				});
				continue;
			}
			let off = self.disabled(self.specs[i].key);
			let label_color = if off { dlg().dim } else { dlg().text };
			out.push(TextItem {
				color: label_color,
				clip: Some(vp),
				..mk(self.specs[i].label.into(), self.label_x(i), ty)
			});
			// revert-to-default icon: bright + clickable when off-default, dim when at it
			if self.has_revert(i) {
				let revert_rect = self.revert_box(i);
				out.push(TextItem {
					color: if self.row_is_default(i) {
						dlg().dim
					} else {
						dlg().handle
					},
					clip: Some(vp),
					..mk(ui().icons.revert.into(), revert_rect.x + 4.0, ty)
				});
			}
			// horizontal view offset of row i's field while it's being edited (the
			// text slides left as the view scrolls; the box clip crops the rest)
			let view = |i: usize| -> f32 {
				self.edit
					.as_ref()
					.filter(|e| e.row == i)
					.map_or(0.0, |e| e.view)
			};
			match self.specs[i].kind {
				Kind::Slider { int, .. } => {
					let val_box = self.valbox(i);
					let txt = match &self.edit {
						Some(edit) if edit.row == i => edit.buf.clone(),
						_ => self.fmt_val(self.specs[i].key, int),
					};
					out.push(TextItem {
						color: label_color,
						clip: Some(intersect(val_box)),
						..mk(
							txt,
							val_box.x + lay().field_pad - view(i),
							row_text_y(val_box.y, val_box.h),
						)
					});
				}
				Kind::Color => {
					let hex_box = self.hexbox(i);
					let txt = match &self.edit {
						Some(edit) if edit.row == i => edit.buf.clone(),
						_ => config::format_hex(self.get_col(self.specs[i].key)),
					};
					out.push(TextItem {
						clip: Some(intersect(hex_box)),
						..mk(
							txt,
							hex_box.x + lay().field_pad - view(i),
							row_text_y(hex_box.y, hex_box.h),
						)
					});
				}
				Kind::Text => {
					let text_box = self.textbox(i);
					let val = match &self.edit {
						Some(edit) if edit.row == i => edit.buf.clone(),
						_ => self.get_text(self.specs[i].key),
					};
					let placeholder = if matches!(self.specs[i].key, Key::FontFamily) {
						"(system default)"
					} else {
						"(none)"
					};
					let (txt, color) = if val.is_empty() {
						(placeholder.to_string(), dlg().dim)
					} else {
						(val, dlg().text)
					};
					out.push(TextItem {
						color,
						clip: Some(intersect(text_box)),
						..mk(
							txt,
							text_box.x + lay().field_pad - view(i),
							row_text_y(text_box.y, text_box.h),
						)
					});
				}
				Kind::Dual { keys, labels } => {
					for p in 0u16..2 {
						let off = self.disabled(keys[p as usize]);
						let color = if off { dlg().dim } else { dlg().text };
						let bx = self.dual_box(i, p);
						out.push(TextItem {
							color,
							clip: Some(vp),
							..mk(labels[p as usize].into(), bx.x + bx.w + 6.0, ty)
						});
					}
				}
				Kind::Radio(options) => {
					let off = self.disabled(self.specs[i].key);
					let color = if off { dlg().dim } else { dlg().text };
					for (k, opt) in options.iter().enumerate() {
						let radio_rect = self.radio_box(i, k);
						out.push(TextItem {
							color,
							clip: Some(vp),
							..mk((*opt).into(), radio_rect.x + radio_rect.w + 6.0, ty)
						});
					}
				}
				Kind::Dropdown(_) => {
					let off = self.disabled(self.specs[i].key);
					let color = if off { dlg().dim } else { dlg().text };
					let box_r = self.dd_box(i);
					let sel = self.get_radio(self.specs[i].key);
					let options = self.dd_options(i);
					let label = options.get(sel).cloned().unwrap_or_default();
					out.push(TextItem {
						color,
						clip: Some(intersect(box_r)),
						..mk(label, box_r.x + 8.0, row_text_y(box_r.y, box_r.h))
					});
					out.push(TextItem {
						color,
						clip: Some(vp),
						..mk(
							ui().icons.dropdown_arrow.into(),
							box_r.x + box_r.w - 18.0,
							row_text_y(box_r.y, box_r.h),
						)
					});
				}
				Kind::Buttons(captions) => {
					for (p, caption) in captions.iter().enumerate() {
						let r = self.row_btn_rect(i, p as u16);
						let color = if self.part_disabled(i, p as u16) {
							dlg().dim
						} else {
							dlg().text
						};
						let lx = r.x + (r.w - measure(caption)).max(0.0) / 2.0;
						out.push(TextItem {
							color,
							clip: Some(vp),
							..mk((*caption).into(), lx, row_text_y(r.y, r.h))
						});
					}
				}
				Kind::ShellList => {
					let cols = self.shell_cols();
					// Column titles, once, above the whole grid. The grip and the
					// remove button get none: neither is a value, and a title over
					// either would read as a column of data that is not there.
					let head_y = self.shell_head_y(i);
					for (title, tx) in [
						("Name", cols.name),
						("Command", cols.command),
						("Last seen", cols.seen),
						("Active", cols.active),
					] {
						out.push(TextItem {
							color: dlg().dim,
							clip: Some(vp),
							..mk((*title).to_string(), tx, head_y)
						});
					}
					for k in 0..self.edited.shells.len() {
						let name_box = self.shell_name_box(i, k);
						let cmd_box = self.shell_cmd_box(i, k);
						let name_row = shell_field_row(k, false);
						let cmd_row = shell_field_row(k, true);
						let entry = &self.edited.shells[k];
						// an inactive shell is still listed, but reads as parked
						let color = if entry.active { dlg().text } else { dlg().dim };
						let text = |row: usize, stored: &str| -> String {
							match &self.edit {
								Some(edit) if edit.row == row => edit.buf.clone(),
								_ => stored.to_string(),
							}
						};
						out.push(TextItem {
							color,
							clip: Some(intersect(name_box)),
							..mk(
								text(name_row, &entry.title),
								name_box.x + lay().field_pad - view(name_row),
								row_text_y(name_box.y, name_box.h),
							)
						});
						let cmd = text(cmd_row, &entry.command);
						let (cmd, cmd_color) = if cmd.is_empty() {
							("(required)".to_string(), dlg().dim)
						} else {
							(cmd, color)
						};
						out.push(TextItem {
							color: cmd_color,
							clip: Some(intersect(cmd_box)),
							..mk(
								cmd,
								cmd_box.x + lay().field_pad - view(cmd_row),
								row_text_y(cmd_box.y, cmd_box.h),
							)
						});
						// Last seen is the program's own note, never edited here
						let seen_text = if entry.last_seen.is_empty() {
							"never".to_string()
						} else {
							entry.last_seen.clone()
						};
						out.push(TextItem {
							color: dlg().dim,
							clip: Some(vp),
							..mk(
								seen_text,
								cols.seen,
								row_text_y(self.shell_line_y(i, k), self.shell_line_h()),
							)
						});
					}
					let add = self.shell_add_box(i);
					let caption = "Add";
					let lx = add.x + (add.w - measure(caption)).max(0.0) / 2.0;
					out.push(TextItem {
						clip: Some(vp),
						..mk(caption.to_string(), lx, row_text_y(add.y, add.h))
					});
				}
				Kind::Toggle | Kind::Header(_) => {}
			}
		}
		for (_, r, label) in self.buttons() {
			// center the caption within the button
			let lx = r.x + (r.w - measure(label)).max(0.0) / 2.0;
			out.push(mk(label.into(), lx, row_text_y(r.y, r.h)));
		}
		out
	}

	// The open dropdown's popup, as (rects, text), for a second (LoadOp::Load) pass
	// drawn on top of the dialog so the covered rows' text can't bleed through the
	// opaque box (same reason the context menu uses its own pass). Empty when closed.
	fn dropdown_overlay(&self) -> (Vec<RectInstance>, Vec<TextItem>) {
		let mut rects = Vec::new();
		let mut texts = Vec::new();
		let Some(i) = self.open else {
			return (rects, texts);
		};
		let options = self.dd_options(i);
		let n = options.len();
		if n == 0 {
			return (rects, texts);
		}
		let popup = self.dd_popup(i, n);
		let q = |x: f32, y: f32, w: f32, h: f32, color: [u8; 3]| RectInstance {
			pos: [x, y],
			size: [w, h],
			color: config::srgb_f32(color),
			..Default::default()
		};
		rects.push(q(popup.x, popup.y, popup.w, popup.h, dlg().field_bg));
		let t = 1.0;
		rects.push(q(
			popup.x - t,
			popup.y - t,
			popup.w + 2.0 * t,
			t,
			dlg().panel_border,
		));
		rects.push(q(
			popup.x - t,
			popup.y + popup.h,
			popup.w + 2.0 * t,
			t,
			dlg().panel_border,
		));
		rects.push(q(popup.x - t, popup.y, t, popup.h, dlg().panel_border));
		rects.push(q(
			popup.x + popup.w,
			popup.y,
			t,
			popup.h,
			dlg().panel_border,
		));
		let sel = self.get_radio(self.specs[i].key);
		let mk = |text: String, x: f32, y: f32| TextItem {
			text,
			x,
			y,
			color: dlg().text,
			clip: None,
			bold: false,
			scale: 1.0,
		};
		for (k, opt) in options.iter().enumerate() {
			let r = self.dd_item_rect(i, n, k);
			if k == self.pending {
				rects.push(q(r.x + 1.0, r.y, r.w - 2.0, r.h, dlg().btn_hl));
			}
			let ty = r.y + (r.h - self.line_h) / 2.0;
			if k == sel {
				texts.push(mk(ui().icons.dropdown_check.into(), r.x + r.w - 18.0, ty));
			}
			texts.push(mk(opt.clone(), r.x + 10.0, ty));
		}
		(rects, texts)
	}

	// The name / confirm box, drawn over everything the panel just drew. Its own
	// field caret rides the same edit state the rows use, so it blinks and eases
	// the same way.
	fn prompt_overlay(
		&self,
		measure: &mut impl FnMut(&str) -> f32,
	) -> (Vec<RectInstance>, Vec<TextItem>) {
		let mut rects = Vec::new();
		let mut texts = Vec::new();
		let Some(prompt) = &self.prompt else {
			return (rects, texts);
		};
		let q = |x: f32, y: f32, w: f32, h: f32, color: [u8; 3]| RectInstance {
			pos: [x, y],
			size: [w, h],
			color: config::srgb_f32(color),
			..Default::default()
		};
		let border = |out: &mut Vec<RectInstance>, r: Rect, t: f32, color: [u8; 3]| {
			out.push(q(r.x - t, r.y - t, r.w + 2.0 * t, t, color));
			out.push(q(r.x - t, r.y + r.h, r.w + 2.0 * t, t, color));
			out.push(q(r.x - t, r.y, t, r.h, color));
			out.push(q(r.x + r.w, r.y, t, r.h, color));
		};
		let mk = |text: String, x: f32, y: f32| TextItem {
			text,
			x,
			y,
			color: dlg().text,
			clip: None,
			bold: false,
			scale: 1.0,
		};
		let row_text_y = |y: f32, h: f32| y + (h - self.line_h) / 2.0;
		// dim the panel behind, so it is plain that it is not taking input
		rects.push(RectInstance {
			pos: [self.rect.x, self.rect.y],
			size: [self.rect.w, self.rect.h],
			color: [0.0, 0.0, 0.0, 0.45],
			..Default::default()
		});
		let box_r = self.prompt_rect();
		rects.push(q(box_r.x, box_r.y, box_r.w, box_r.h, dlg().panel_bg));
		border(&mut rects, box_r, 1.0, dlg().panel_border);
		texts.push(mk(
			prompt.title.clone(),
			box_r.x + lay().pad,
			box_r.y + lay().pad + (self.line_h + lay().row_pad - self.line_h) / 2.0,
		));
		if let Some(field) = self.prompt_field_rect() {
			rects.push(q(field.x, field.y, field.w, field.h, dlg().field_bg));
			if prompt.focus == PromptFocus::Field {
				border(&mut rects, field, 1.0, dlg().focus_out);
			} else {
				border(&mut rects, field, 1.0, dlg().panel_border);
			}
			self.caret_quad(&mut rects, field, measure);
			let view = self.edit.as_ref().map_or(0.0, |e| e.view);
			texts.push(TextItem {
				clip: Some(field),
				..mk(
					self.edit.as_ref().map_or(String::new(), |e| e.buf.clone()),
					field.x + lay().field_pad - view,
					row_text_y(field.y, field.h),
				)
			});
		}
		if let Some(warn) = &prompt.warn {
			let y = self.prompt_btn_rect(PromptFocus::Ok).y - (self.line_h + lay().row_pad);
			texts.push(TextItem {
				color: dlg().btn_hl,
				..mk(warn.clone(), box_r.x + lay().pad, y)
			});
		}
		for (part, caption) in [(PromptFocus::Cancel, "Cancel"), (PromptFocus::Ok, "OK")] {
			let r = self.prompt_btn_rect(part);
			rects.push(q(r.x, r.y, r.w, r.h, dlg().btn_bg));
			let ring = prompt.focus == part;
			// OK is the default here too, so it keeps the highlight outline when
			// the keyboard is elsewhere
			let outline = if ring {
				dlg().focus_out
			} else if part == PromptFocus::Ok {
				dlg().btn_hl
			} else {
				dlg().panel_border
			};
			border(&mut rects, r, if ring { 2.0 } else { 1.0 }, outline);
			let lx = r.x + (r.w - measure(caption)).max(0.0) / 2.0;
			texts.push(mk(caption.into(), lx, row_text_y(r.y, r.h)));
		}
		(rects, texts)
	}

	// True when anything needs the second (on-top) render pass.
	pub fn overlay_open(&self) -> bool {
		self.open.is_some() || self.emenu.is_some() || self.prompt.is_some()
	}
	// Everything for the second pass: the open dropdown popup and/or the field
	// context menu (only one is ever open at a time in practice).
	fn overlay_dip(
		&self,
		measure: &mut impl FnMut(&str) -> f32,
	) -> (Vec<RectInstance>, Vec<TextItem>) {
		let (mut rects, mut texts) = self.dropdown_overlay();
		// the prompt box sits over everything, including an open popup
		let (prompt_rects, prompt_texts) = self.prompt_overlay(measure);
		rects.extend(prompt_rects);
		texts.extend(prompt_texts);
		if self.emenu.is_none() {
			return (rects, texts);
		}
		let q = |x: f32, y: f32, w: f32, h: f32, color: [u8; 3]| RectInstance {
			pos: [x, y],
			size: [w, h],
			color: config::srgb_f32(color),
			..Default::default()
		};
		let menu = self.em_rect();
		let t = 1.0;
		rects.push(q(
			menu.x - t,
			menu.y - t,
			menu.w + 2.0 * t,
			menu.h + 2.0 * t,
			dlg().panel_border,
		));
		rects.push(q(menu.x, menu.y, menu.w, menu.h, dlg().field_bg));
		let hover = self.emenu.as_ref().and_then(|m| m.hover);
		for (k, (label, _)) in EDIT_MENU.iter().enumerate() {
			let r = self.em_item_rect(k);
			let enabled = self.em_enabled(k);
			if enabled && hover == Some(k) {
				rects.push(q(r.x + 1.0, r.y, r.w - 2.0, r.h, dlg().btn_hl));
			}
			texts.push(TextItem {
				text: (*label).into(),
				x: r.x + 10.0,
				y: r.y + (r.h - self.line_h) / 2.0,
				color: if enabled { dlg().text } else { dlg().dim },
				clip: None,
				bold: false,
				scale: 1.0,
			});
		}
		(rects, texts)
	}
}

// Widest field label, button caption, and per-tab title widths at the current
// UI font, so the dialog sizes to the real text (a wide serif or a big desktop
// size never truncates).
pub fn chrome_widths(text: &mut crate::text::TextCtx) -> (f32, f32, f32, Vec<f32>) {
	let attrs = crate::text::ui_attrs();
	// an indented label starts further right, so the column has to clear the
	// deepest one plus its own indent - not merely the longest string
	let label_w = ui()
		.specs
		.iter()
		.map(|spec| {
			text.measure_ui_text(spec.label, &attrs) + f32::from(spec.indent) * lay().indent
		})
		.fold(0.0f32, f32::max)
		+ lay().label_gap;
	let btn_w = ["Cancel", "Apply", "OK"]
		.iter()
		.map(|caption| text.measure_ui_text(caption, &attrs))
		.fold(0.0f32, f32::max)
		+ lay().button_pad;
	// the buttons that sit on a row are measured apart from the footer's, so a
	// long caption there widens its own row instead of every button in the dialog
	let row_btn_w = ui()
		.specs
		.iter()
		.filter_map(|spec| match spec.kind {
			Kind::Buttons(captions) => Some(captions),
			_ => None,
		})
		.flatten()
		.map(|caption| text.measure_ui_text(caption, &attrs))
		.fold(0.0f32, f32::max)
		+ lay().button_pad;
	let tab_ws = tab_titles()
		.iter()
		.map(|title| text.measure_ui_text(title, &attrs) + lay().tab_pad)
		.collect();
	(label_w, btn_w, row_btn_w, tab_ws)
}

// Returns true if `old` and `new` differ in any field that needs a text-context
// rebuild (cell metrics change) rather than just a re-render.
pub fn needs_text_rebuild(old: &Settings, new: &Settings) -> bool {
	old.font_size != new.font_size
		|| old.line_height_scale != new.line_height_scale
		|| old.font_family != new.font_family
		// the toggle alone changes the effective family/size (fields keep
		// their values), so it must force a rebuild too
		|| old.use_system_font != new.use_system_font
		|| old.use_system_font_size != new.use_system_font_size
		|| old.margin != new.margin
}

// Returns true if a background-image-affecting setting changed.
pub fn wallpaper_changed(old: &Settings, new: &Settings) -> bool {
	old.wallpaper_enabled != new.wallpaper_enabled
		|| old.wallpaper_rotate_enabled != new.wallpaper_rotate_enabled
		|| old.wallpaper_opacity != new.wallpaper_opacity
		|| old.wallpaper_default_fit != new.wallpaper_default_fit
		|| old.wallpaper_honor_xmp != new.wallpaper_honor_xmp
		|| old.wallpaper != new.wallpaper
		|| old.wallpaper_blur != new.wallpaper_blur
		|| old.wallpaper_contrast_mask != new.wallpaper_contrast_mask
		|| old.wallpaper_contrast_mask_size != new.wallpaper_contrast_mask_size
		|| old.wallpaper_contrast_mask_strength != new.wallpaper_contrast_mask_strength
		|| old.wallpaper_contrast_mask_auto != new.wallpaper_contrast_mask_auto
}

#[cfg(test)]
mod tests {
	use super::{
		EASE_IN_MAX, EASE_IN_MIN, EASE_OUT_MAX, EASE_OUT_MIN, Key, RAMP_DOWN_MAX, RAMP_DOWN_MIN,
		RAMP_UP_MAX, RAMP_UP_MIN, SettingsDialog, TAU_MAX, TAU_MIN, falling_slider, speed_to_tau,
		tab_titles, tau_to_speed,
	};
	use crate::config;

	fn mk_dialog(max_h: f32) -> SettingsDialog {
		mk_dialog_at(max_h, 1.0)
	}
	// Everything a real dialog is handed arrives in physical pixels, so a scale
	// of 2 means twice the line height, label width, tab widths and height cap.
	fn mk_dialog_at(max_h: f32, scale: f32) -> SettingsDialog {
		SettingsDialog::new(
			0.0,
			0.0,
			18.0 * scale,
			170.0 * scale,
			80.0 * scale,
			90.0 * scale,
			vec![90.0 * scale; tab_titles().len()],
			max_h * scale,
			scale,
		)
	}

	#[test]
	fn tabs_partition_all_specs() {
		let d = mk_dialog(2000.0);
		// every spec lands on a valid tab and no tab is empty
		assert!(d.specs.iter().all(|s| s.tab < tab_titles().len()));
		for t in 0..tab_titles().len() {
			assert!(d.specs.iter().any(|s| s.tab == t), "tab {t} has no rows");
		}
	}

	#[test]
	fn revert_restores_default_and_records_key() {
		let mut d = mk_dialog(2000.0);
		let def = d.defaults.opacity; // edited may start off-default (loaded config)
		d.edited.opacity = def + 0.5;
		assert!(!d.is_default(super::Key::Opacity));
		d.revert(super::Key::Opacity);
		assert!(d.is_default(super::Key::Opacity));
		assert_eq!(d.edited.opacity, def);
		let rev = d.take_reverted();
		assert!(rev.contains(&"transparency.opacity"));
		assert!(d.take_reverted().is_empty(), "taking clears the list");
		// reverting font size must not clear the system-size follow (set_f32
		// side effect)
		d.edited.use_system_font = true;
		d.edited.use_system_font_size = true;
		d.edited.font_size = 99.0;
		d.revert(super::Key::FontSize);
		assert!(d.edited.use_system_font && d.edited.use_system_font_size);
	}

	#[test]
	fn height_cap_enables_scroll() {
		// generous cap: natural size, nothing to scroll
		let d = mk_dialog(2000.0);
		assert!(d.size().1 < 2000.0);
		assert_eq!(d.max_scroll(), 0.0);
		assert!(d.thumb().is_none());
		// tight cap: window clamps, the (tallest) appearance tab overflows
		let mut d = mk_dialog(400.0);
		assert!(d.size().1 <= 400.0);
		assert!(d.max_scroll() > 0.0);
		assert!(d.thumb().is_some());
		// wheel scrolls rows up and clamps at both ends
		let y_first = d.row_y(1);
		d.wheel(-120.0);
		assert!(d.scroll > 0.0 && d.scroll <= d.max_scroll());
		assert!(d.row_y(1) < y_first);
		d.wheel(1e9);
		assert_eq!(d.scroll, 0.0);
		d.wheel(-1e9);
		assert_eq!(d.scroll, d.max_scroll());
	}

	// The strip is chrome: shorter than a footer button, dropped clear of the
	// panel edge, and closed off by a rule the rows start below.
	#[test]
	fn the_tabs_stand_on_the_line_that_closes_their_strip() {
		let d = mk_dialog(2000.0);
		let gut = d.gutter_rect();
		let tab = d.tab_rect(0);
		assert!(d.tab_h() < d.btn_h(), "a tab is shorter than a button");
		assert!(tab.y > gut.y, "the tabs are clear of the panel edge");
		assert!(
			(tab.y + tab.h - (gut.y + gut.h)).abs() < 0.01,
			"and stand on the strip's closing line"
		);
		assert!(d.rows_y0() > gut.y + gut.h, "rows begin below that line");
	}

	// The whole point of a sub-group: the label steps right, the control does
	// not. A control that moved with its label would break the one column every
	// row shares, which is what makes a settings list scannable.
	#[test]
	fn a_sub_group_indents_labels_and_nothing_else() {
		let mut d = mk_dialog(4000.0);
		let mut seen_indented = false;
		for tab in 0..tab_titles().len() {
			d.tab = tab;
			let rows: Vec<usize> = SettingsDialog::visible(d.specs, tab)
				.map(|(i, _)| i)
				.collect();
			for &i in &rows {
				let indent = f32::from(d.specs[i].indent);
				seen_indented |= indent > 0.0;
				assert!(
					(d.label_x(i) - (d.rect.x + super::lay().pad + indent * super::lay().indent))
						.abs() < 0.01
				);
				assert!(
					d.label_x(i) >= d.rect.x + super::lay().pad,
					"a label never steps left of the panel pad"
				);
				// every control on the tab starts in the same column
				assert!((d.control_x() - (d.rect.x + super::lay().pad + d.label_w)).abs() < 0.01);
				assert!(
					d.label_x(i) + super::lay().indent <= d.control_x(),
					"the label column still clears the deepest indent"
				);
			}
			// a member is never deeper than one step below its leader
			for pair in rows.windows(2) {
				let (prev, next) = (d.specs[pair[0]].indent, d.specs[pair[1]].indent);
				assert!(next <= prev + 1, "sub-group depth jumps more than one step");
			}
		}
		assert!(seen_indented, "no sub-groups declared at all");
	}

	// A sub-group's leader is set off from whatever sat above it, the way a
	// heading is - but its own members are not, or the run would not read as one.
	#[test]
	fn a_sub_group_leader_gets_the_gap_and_its_members_do_not() {
		let d = mk_dialog(4000.0);
		let mut leaders = 0;
		for tab in 0..tab_titles().len() {
			let rows: Vec<usize> = SettingsDialog::visible(d.specs, tab)
				.map(|(i, _)| i)
				.collect();
			for (n, &i) in rows.iter().enumerate() {
				let prev = n.checked_sub(1).map(|k| &d.specs[rows[k]]);
				let gap = SettingsDialog::gap_above(d.specs, i, tab, prev);
				let leads = SettingsDialog::leads_subgroup(d.specs, i, tab);
				let after_header = prev.is_some_and(|p| matches!(p.kind, super::Kind::Header(_)));
				if matches!(d.specs[i].kind, super::Kind::Header(_)) {
					continue; // headings carry their own gap, tested elsewhere
				}
				if leads && prev.is_some() && !after_header {
					leaders += 1;
					assert_eq!(gap, super::lay().subgroup_gap, "{}", d.specs[i].label);
				} else {
					assert_eq!(gap, 0.0, "{}", d.specs[i].label);
				}
			}
		}
		assert!(leaders >= 3, "expected several sub-groups, saw {leaders}");
	}

	// Change whatever a row edits, whichever kind it is, to something it is not.
	fn nudge(d: &mut SettingsDialog, i: usize, key: Key) {
		match d.specs[i].kind {
			super::Kind::Slider { min, max, int } => {
				let far = if (d.get_f32(key) - min).abs() < (max - d.get_f32(key)).abs() {
					max
				} else {
					min
				};
				d.set_f32(key, if int { far.round() } else { far });
			}
			super::Kind::Color => {
				let c = d.get_col(key);
				d.set_col(key, [c[0] ^ 0x7f, c[1] ^ 0x7f, c[2] ^ 0x7f]);
			}
			super::Kind::Text => d.set_text(key, "silkterm-roundtrip"),
			super::Kind::Toggle | super::Kind::Dual { .. } => {
				let was = d.get_toggle(key);
				d.set_toggle(key, !was);
			}
			super::Kind::Radio(_) | super::Kind::Dropdown(_) => {
				let n = d.dd_options(i).len().max(match d.specs[i].kind {
					super::Kind::Radio(opts) => opts.len(),
					_ => 0,
				});
				if n > 1 {
					let next = (d.get_radio(key) + 1) % n;
					d.set_radio(key, next);
				}
			}
			// no single value to nudge: a push-button row, a heading, or the
			// shells grid (a list, exercised by its own tests)
			super::Kind::Buttons(_) | super::Kind::Header(_) | super::Kind::ShellList => {}
		}
	}
	// What the row shows, whichever kind it is.
	fn row_value(d: &SettingsDialog, i: usize, key: Key) -> String {
		match d.specs[i].kind {
			super::Kind::Slider { .. } => format!("{}", d.get_f32(key)),
			super::Kind::Color => format!("{:?}", d.get_col(key)),
			super::Kind::Text => d.get_text(key),
			super::Kind::Toggle | super::Kind::Dual { .. } => format!("{}", d.get_toggle(key)),
			super::Kind::Radio(_) | super::Kind::Dropdown(_) => format!("{}", d.get_radio(key)),
			super::Kind::Buttons(_) | super::Kind::Header(_) | super::Kind::ShellList => {
				String::new()
			}
		}
	}

	// A row whose setting the writer never writes is a dead end: the change
	// applies for the session and is gone at relaunch, with nothing anywhere to
	// say so. Both scrollbar colors did exactly that from the day the bar
	// shipped, so the check is generic - every row, saved and read back.
	#[test]
	fn every_row_survives_a_save_and_a_relaunch() {
		let _guard = config::test_config_lock();
		let _ = config::settings(); // memoize before the override goes in
		let dir = std::env::temp_dir().join(format!("silkterm_rows_{}", std::process::id()));
		let _ = std::fs::create_dir_all(&dir);
		let path = dir.join("config.shcl");
		let _ = std::fs::write(&path, "");
		config::set_config_override(path.clone());
		config::reload_from_disk(); // lets backfill lay the template down once
		let pristine = std::fs::read_to_string(&path).unwrap();

		let mut d = mk_dialog(4000.0);
		let mut checked = 0;
		for i in 0..d.specs.len() {
			let keys: Vec<Key> = match d.specs[i].kind {
				// a heading and a row of push-buttons store nothing; the shells
				// grid stores a list rather than a setting
				super::Kind::Header(_) | super::Kind::Buttons(_) | super::Kind::ShellList => {
					vec![]
				}
				super::Kind::Dual { keys, .. } => keys.to_vec(),
				_ => vec![d.specs[i].key],
			};
			for key in keys {
				let _ = std::fs::write(&path, &pristine);
				let base = config::reload_from_disk();
				d.orig = base.clone();
				d.edited = base.clone();
				let before = row_value(&d, i, key);
				nudge(&mut d, i, key);
				let want = row_value(&d, i, key);
				assert_ne!(before, want, "{} did not budge", d.specs[i].label);
				assert!(
					config::persist(&base, &d.edited),
					"{} was not written at all",
					d.specs[i].label
				);
				let mut back = mk_dialog(4000.0);
				back.edited = config::reload_from_disk();
				assert_eq!(
					row_value(&back, i, key),
					want,
					"{} is lost on relaunch",
					d.specs[i].label
				);
				checked += 1;
			}
		}
		assert!(checked > 40, "only {checked} rows checked");
		let _ = std::fs::remove_dir_all(&dir);
	}

	// A 0..1 fraction reads as a whole percent and is stored as the decimal. The
	// two directions have to be exact inverses: a revert that landed a hair off
	// its own default would leave the arrow lit with nothing to undo.
	#[test]
	fn a_fraction_reads_as_a_whole_percent_and_stores_as_a_decimal() {
		let mut d = mk_dialog(4000.0);
		for key in [
			Key::Opacity,
			Key::BgOpacity,
			Key::ScrimSoftness,
			Key::BgContrastSize,
			Key::BgContrastStrength,
			Key::BgContrastAuto,
		] {
			let spec = d.specs.iter().find(|s| s.key == key).unwrap();
			let super::Kind::Slider { min, max, int } = spec.kind else {
				panic!("{} is not a slider", spec.label)
			};
			assert!(
				min == 0.0 && max == 100.0 && int,
				"{} must run 0..100 in whole steps",
				spec.label
			);
			d.set_f32(key, 35.0);
			assert_eq!(d.get_f32(key), 35.0, "{}", spec.label);
			d.revert(key);
			assert!(d.is_default(key), "{}", spec.label);
			assert_eq!(d.get_f32(key), d.default_f32(key), "{}", spec.label);
		}
		// and the decimal really is what reaches the settings the app runs on
		d.set_f32(Key::Opacity, 35.0);
		assert_eq!(d.edited.opacity, 0.35);
		d.set_f32(Key::BgContrastSize, 100.0);
		assert_eq!(d.edited.wallpaper_contrast_mask_size, 1.0);
	}

	// A heading that only repeats its tab's title is gone from the layout
	// entirely - not merely hidden, or it would leave a gap where it used to be.
	#[test]
	fn a_heading_that_repeats_its_tab_takes_no_room() {
		let mut d = mk_dialog(2000.0);
		for tab in 0..tab_titles().len() {
			d.tab = tab;
			let redundant = d
				.specs
				.iter()
				.enumerate()
				.find(|(_, spec)| spec.tab == tab && SettingsDialog::header_is_tab_title(spec));
			if redundant.is_none() {
				continue;
			}
			// the first row that IS drawn starts hard against the top of the
			// viewport, so the heading left no gap behind it
			let first = (0..d.specs.len())
				.find(|&j| {
					d.specs[j].tab == tab && !SettingsDialog::header_is_tab_title(&d.specs[j])
				})
				.expect("a tab with rows");
			assert!((d.row_y(first) - (d.rows_y0() - d.scroll)).abs() < 0.01);
			// and the tab's height accounts for the rows it draws and nothing
			// more: the surplus over them is header gaps, which are far smaller
			// than the heading row that was dropped
			let shells = d.edited.shells.len();
			let drawn: f32 = d
				.specs
				.iter()
				.filter(|s| s.tab == tab && !SettingsDialog::header_is_tab_title(s))
				.map(|s| SettingsDialog::row_h_for(&s.kind, d.line_h, shells))
				.sum();
			// the surplus over the rows is the declared gaps and NOTHING else -
			// stated exactly rather than bounded by a heading's height, so
			// retuning header_gap cannot quietly turn this into a near miss
			let mut gaps = 0.0;
			let mut prev = None;
			for (i, s) in SettingsDialog::visible(d.specs, tab) {
				gaps += SettingsDialog::gap_above(d.specs, i, tab, prev);
				prev = Some(s);
			}
			let counted = SettingsDialog::tab_content_h(d.specs, tab, d.line_h, shells);
			assert!(
				(counted - drawn - gaps).abs() < 0.01,
				"tab {tab}: {counted} != {drawn} rows + {gaps} gaps"
			);
		}
	}

	// ---- the shells grid ------------------------------------------------------

	fn shell_entry(title: &str, command: &str) -> crate::shells::ShellEntry {
		crate::shells::ShellEntry {
			slug: title.to_lowercase().replace(' ', "_"),
			title: title.into(),
			command: command.into(),
			active: true,
			comment: String::new(),
			last_seen: String::new(),
		}
	}

	// A dialog sitting on the Shell tab with `n` shells in it.
	fn mk_shell_dialog(n: usize) -> (SettingsDialog, usize) {
		let mut d = mk_dialog(4000.0);
		let i = d
			.specs
			.iter()
			.position(|s| matches!(s.kind, super::Kind::ShellList))
			.expect("a shells grid");
		d.tab = d.specs[i].tab;
		d.edited.shells = (0..n)
			.map(|k| shell_entry(&format!("Shell {k}"), &format!("/bin/sh{k}")))
			.collect();
		d.orig.shells.clone_from(&d.edited.shells);
		(d, i)
	}

	#[test]
	fn the_shells_grid_has_a_tab_to_itself() {
		let ui = super::ui_spec::ui();
		let grid = ui
			.specs
			.iter()
			.find(|s| matches!(s.kind, super::Kind::ShellList))
			.expect("a shells grid");
		assert_eq!(tab_titles()[grid.tab], "Shell");
		// The tab is the grid, its headings, and the startup directory that the
		// grid's own default shell starts in - nothing else belongs beside them.
		for spec in ui.specs.iter().filter(|s| s.tab == grid.tab) {
			assert!(
				matches!(spec.kind, super::Kind::ShellList | super::Kind::Header(_))
					|| spec.key == Key::StartupDirectory,
				"{} does not belong on the Shell tab",
				spec.label
			);
		}
		// and the directory reads BELOW the list it applies to
		let dir = ui
			.specs
			.iter()
			.position(|s| s.key == Key::StartupDirectory)
			.expect("a startup directory row");
		let at = ui
			.specs
			.iter()
			.position(|s| matches!(s.kind, super::Kind::ShellList))
			.expect("a shells grid");
		assert!(dir > at, "the startup directory sits below the shells");
	}

	// Asked for on the Cursor tab, and asked for LAST - so both halves are
	// pinned, or a row appended later quietly takes its place.
	#[test]
	fn copy_on_select_is_the_last_thing_on_the_cursor_tab() {
		let d = mk_dialog(4000.0);
		let i = d
			.specs
			.iter()
			.position(|s| s.key == Key::CopyOnSelect)
			.expect("a copy-on-select row");
		let tab = d.specs[i].tab;
		assert_eq!(tab_titles()[tab], "Cursor");
		let last = SettingsDialog::visible(d.specs, tab)
			.map(|(j, _)| j)
			.last()
			.expect("the tab draws rows");
		assert_eq!(last, i);
	}

	// Every control on every line is its own stop, and a part index names
	// exactly one of them - the encoding both the focus ring and the hit tests
	// read, so a mistake here would put the ring on one control and the click on
	// another.
	#[test]
	fn a_part_index_names_one_control_on_one_line() {
		let (d, i) = mk_shell_dialog(3);
		assert_eq!(d.parts_of(i), 3 * super::ShellPart::COUNT + 1);
		for k in 0..3 {
			for part in super::ShellPart::ALL {
				let p = super::shell_part_index(k, part);
				match super::shell_stop(p, 3) {
					super::ShellStop::Entry(entry, named) => {
						assert_eq!((entry, named), (k, part));
					}
					super::ShellStop::Add => panic!("{k}/{part:?} read as the Add button"),
				}
			}
		}
		assert!(matches!(
			super::shell_stop(d.parts_of(i) - 1, 3),
			super::ShellStop::Add
		));
	}

	// Reordering left the keyboard when the arrows did, so the grip must not be a
	// stop - a ring sitting on a control that Space cannot work is worse than no
	// ring at all. Nothing in the grid is grayed any more either.
	#[test]
	fn the_grip_is_a_gesture_and_not_a_keyboard_stop() {
		let (d, i) = mk_shell_dialog(3);
		assert_eq!(d.parts_of(i), 3 * super::ShellPart::COUNT + 1);
		for part in 0..d.parts_of(i) {
			assert!(!d.part_disabled(i, part), "part {part} came up grayed");
		}
		// no stop draws where the grip does
		for k in 0..3 {
			let grip = d.shell_grip_box(i, k);
			let (cx, cy) = (grip.x + grip.w / 2.0, grip.y + grip.h / 2.0);
			for part in 0..d.parts_of(i) {
				assert!(
					!d.shell_stop_rect(i, part).contains(cx, cy),
					"part {part} hit-tests over line {k}'s grip"
				);
			}
		}
	}

	// The grip's whole job. Dragging is live - the list reorders under the
	// pointer rather than on release - so each step of the gesture is asserted,
	// not just where it ended up.
	#[test]
	fn a_grip_drag_reorders_the_list() {
		let (mut d, i) = mk_shell_dialog(3);
		let mut measure = |s: &str| s.chars().count() as f32 * 7.0;
		let line = d.shell_line_h();
		let titles = |d: &super::SettingsDialog| -> Vec<String> {
			d.edited.shells.iter().map(|e| e.title.clone()).collect()
		};
		let grip = d.shell_grip_box(i, 0);
		let (x, y) = (grip.x + grip.w / 2.0, grip.y + grip.h / 2.0);
		assert!(
			d.shell_mouse_down(i, x, y, &mut measure),
			"the grip did not take the press"
		);
		// less than half a line is not yet a move
		d.mouse_move_dip(x, y + line * 0.4, &mut measure);
		assert_eq!(titles(&d), ["Shell 0", "Shell 1", "Shell 2"]);
		// past half, it swaps with the line below
		d.mouse_move_dip(x, y + line * 0.6, &mut measure);
		assert_eq!(titles(&d), ["Shell 1", "Shell 0", "Shell 2"]);
		// and keeps going, without letting go
		d.mouse_move_dip(x, y + line * 2.0, &mut measure);
		assert_eq!(titles(&d), ["Shell 1", "Shell 2", "Shell 0"]);
		// dragged past the end it stops at the end rather than vanishing
		d.mouse_move_dip(x, y + line * 40.0, &mut measure);
		assert_eq!(titles(&d), ["Shell 1", "Shell 2", "Shell 0"]);
		d.mouse_up_dip(x, y + line * 40.0);
		assert!(d.shell_drag.is_none(), "the drag outlived the release");
		// and a plain move afterwards moves nothing
		d.mouse_move_dip(x, y, &mut measure);
		assert_eq!(titles(&d), ["Shell 1", "Shell 2", "Shell 0"]);
	}

	// Dragged the other way, and off the top: the first line is as far as it
	// goes. The arithmetic is in f32 and lands on a usize, so this is the test
	// that says the saturating cast is being RELIED on rather than tolerated.
	#[test]
	fn a_line_dragged_off_the_top_lands_on_the_first() {
		let (mut d, i) = mk_shell_dialog(3);
		let mut measure = |s: &str| s.chars().count() as f32 * 7.0;
		let line = d.shell_line_h();
		let grip = d.shell_grip_box(i, 2);
		let (x, y) = (grip.x + grip.w / 2.0, grip.y + grip.h / 2.0);
		assert!(d.shell_mouse_down(i, x, y, &mut measure));
		d.mouse_move_dip(x, y - line * 99.0, &mut measure);
		let titles: Vec<&str> = d.edited.shells.iter().map(|e| e.title.as_str()).collect();
		assert_eq!(titles, ["Shell 2", "Shell 0", "Shell 1"]);
		d.mouse_up_dip(x, y);
	}

	// The command is REQUIRED: emptying the field cannot be what stores an entry
	// that names nothing to run. The stored value stands and the box shows it
	// again, which is the whole rule at the field.
	#[test]
	fn a_blank_command_cannot_replace_a_stored_one() {
		let (mut d, _) = mk_shell_dialog(2);
		let row = super::shell_field_row(1, true);
		d.open_edit(row, true);
		d.select_all();
		d.backspace();
		assert_eq!(
			d.edited.shells[1].command, "/bin/sh1",
			"an emptied field leaves the stored command standing"
		);
		// a real value does apply, so the guard is not simply inert
		d.insert_str("/usr/bin/fish");
		assert_eq!(d.edited.shells[1].command, "/usr/bin/fish");
		assert_eq!(d.edited.shells[0].command, "/bin/sh0", "and only that one");
	}

	// A name edit lands on the entry it was opened for, not on the row index -
	// the two are different numbers here, which is the whole point of the
	// pseudo-row scheme.
	#[test]
	fn a_field_edit_lands_on_its_own_entry() {
		let (mut d, _) = mk_shell_dialog(3);
		d.open_edit(super::shell_field_row(2, false), true);
		d.select_all();
		d.insert_str("Renamed");
		assert_eq!(d.edited.shells[2].title, "Renamed");
		assert_eq!(d.edited.shells[0].title, "Shell 0");
		assert_eq!(d.edited.shells[1].title, "Shell 1");
	}

	// Add lands an entry with no command and puts the caret straight in it - the
	// one field that has to be filled before the entry means anything.
	#[test]
	fn adding_a_shell_opens_the_field_it_needs() {
		let (mut d, i) = mk_shell_dialog(1);
		d.shell_add(i);
		assert_eq!(d.edited.shells.len(), 2);
		assert!(d.edited.shells[1].command.is_empty());
		assert_eq!(
			d.edit.as_ref().map(|e| e.row),
			Some(super::shell_field_row(1, true))
		);
		assert_eq!(
			d.focus,
			Some(super::Focus::Row(
				i,
				super::shell_part_index(1, super::ShellPart::Command)
			))
		);
	}

	// Removing is the one grid action doing the opposite cannot undo, so it asks
	// first - and asking must not remove anything by itself.
	#[test]
	fn removing_a_shell_asks_before_it_happens() {
		let (mut d, _) = mk_shell_dialog(3);
		d.shell_confirm_remove(1);
		assert_eq!(
			d.edited.shells.len(),
			3,
			"the question alone changes nothing"
		);
		assert!(matches!(
			d.prompt.as_ref().map(|p| p.job),
			Some(super::PromptJob::DropShell(1))
		));
		d.prompt_accept();
		let titles: Vec<&str> = d.edited.shells.iter().map(|e| e.title.as_str()).collect();
		assert_eq!(titles, vec!["Shell 0", "Shell 2"]);
		assert!(d.prompt.is_none());
	}

	// The columns are laid out from both ends with the command taking the slack,
	// so the one thing that can go wrong is them meeting in the middle. The
	// order asserted here is the order asked for: remove sits between the
	// command and the date, where it is hard to press by accident.
	#[test]
	fn the_grid_columns_stay_inside_the_panel_in_order() {
		let (d, i) = mk_shell_dialog(2);
		let left = d.rect.x + super::lay().pad;
		let right = d.rect.x + d.rect.w - super::lay().pad;
		let cols = d.shell_cols();
		for k in 0..2 {
			let grip = d.shell_grip_box(i, k);
			let name = d.shell_name_box(i, k);
			let cmd = d.shell_cmd_box(i, k);
			let remove = d.shell_remove_box(i, k);
			let active = d.shell_active_box(i, k);
			assert!(grip.x >= left - 0.01, "the grip starts inside the panel");
			assert!(grip.x + grip.w <= name.x + 0.01, "grip runs into the name");
			assert!(name.x + name.w <= cmd.x + 0.01, "name runs into command");
			assert!(cmd.w > 0.0, "the command column collapsed");
			assert!(cmd.x + cmd.w <= remove.x + 0.01, "command runs into remove");
			assert!(
				remove.x + remove.w <= cols.seen + 0.01,
				"remove runs into the date"
			);
			assert!(
				cols.seen + super::lay().shell_seen_width <= active.x + 0.01,
				"the date runs into active"
			);
			assert!(
				active.x + active.w <= right + 0.01,
				"the last column overruns the panel"
			);
			// and every line sits below the column titles
			assert!(d.shell_line_y(i, k) > d.shell_head_y(i));
		}
	}

	// The draw path has no other cover, and it is where a new control kind fails
	// silently: a grid that renders nothing at all still passes every geometry
	// test above. So this walks what would actually be handed to the renderer.
	#[test]
	fn the_grid_draws_a_line_for_every_shell() {
		let (d, i) = mk_shell_dialog(3);
		let mut measure = |s: &str| s.chars().count() as f32 * 7.0;
		let texts = d.texts_dip(d.line_h, &mut measure);
		let said: Vec<&str> = texts.iter().map(|t| t.text.as_str()).collect();
		for want in [
			"Name",
			"Command",
			"Last seen",
			"Active",
			"Add",
			"Shell 0",
			"Shell 1",
			"Shell 2",
			"/bin/sh0",
			"/bin/sh2",
		] {
			assert!(
				said.contains(&want),
				"the grid never drew {want:?}: {said:?}"
			);
		}
		// a shell no scan has vouched for says so rather than leaving a blank
		assert_eq!(
			said.iter().filter(|t| **t == "never").count(),
			3,
			"one 'last seen' per line"
		);
		// and the quads scale with the list rather than being drawn once
		let count = |n: usize| {
			let (mut d, _) = mk_shell_dialog(n);
			d.tab = d.specs[i].tab;
			let mut m = |s: &str| s.chars().count() as f32 * 7.0;
			let (fixed, rows) = d.rects_dip(d.line_h, &mut m);
			let _ = fixed;
			rows.len()
		};
		let (one, three) = (count(1), count(3));
		assert!(
			three > one && three - one == 2 * (one - count(0)),
			"each line costs the same quads: 0->{} 1->{one} 3->{three}",
			count(0)
		);
	}

	// An entry being edited shows the BUFFER, not the stored value - the field
	// would otherwise look inert while it is typed into.
	#[test]
	fn a_grid_field_being_edited_shows_what_is_typed() {
		let (mut d, _) = mk_shell_dialog(2);
		d.open_edit(super::shell_field_row(1, false), true);
		d.select_all();
		d.insert_str("Half typed");
		let mut measure = |s: &str| s.chars().count() as f32 * 7.0;
		let texts = d.texts_dip(d.line_h, &mut measure);
		let said: Vec<&str> = texts.iter().map(|t| t.text.as_str()).collect();
		assert!(said.contains(&"Half typed"));
		assert!(said.contains(&"Shell 0"), "and the other line is untouched");
	}

	// A scan landing while the dialog is open moves BOTH copies, so it does not
	// read as an edit the user made - and it folds into what they have already
	// done rather than replacing it.
	#[test]
	fn a_scan_that_lands_mid_edit_is_not_mistaken_for_an_edit() {
		let (mut d, _) = mk_shell_dialog(1);
		d.edited.shells[0].title = "Renamed by hand".into();
		let found = vec![crate::shells::Found {
			title: "Fish".into(),
			command: "/bin/sh0".into(), // the one already stored, so nothing is added
			comment: String::new(),
		}];
		d.fold_shells(&found);
		assert_eq!(
			d.edited.shells[0].title, "Renamed by hand",
			"a scan never rewrites what the user typed"
		);
		assert_eq!(d.orig.shells.len(), d.edited.shells.len());
	}

	#[test]
	fn a_restored_view_never_outruns_the_new_dialog() {
		// scrolled to the bottom of the last tab, as if the user had just closed it
		let mut d = mk_dialog(400.0);
		// the tallest tab, not merely the last: which tab overflows a short
		// window is a property of the content, and a new tab can change it
		d.tab = (0..tab_titles().len())
			.max_by(|a, b| {
				let h = |t: usize| {
					SettingsDialog::tab_content_h(d.specs, t, d.line_h, d.edited.shells.len())
				};
				h(*a).total_cmp(&h(*b))
			})
			.expect("at least one tab");
		d.wheel(-1e9);
		let view = d.view();
		assert!(view.scroll > 0.0);
		let mut same = mk_dialog(400.0);
		same.restore(view);
		assert_eq!(same.tab, view.tab);
		assert_eq!(same.scroll, view.scroll);
		// a roomier window has nothing to scroll, so the offset must not survive
		let mut roomy = mk_dialog(2000.0);
		roomy.restore(view);
		assert_eq!(roomy.tab, view.tab);
		assert_eq!(roomy.scroll, 0.0);
		// a tab that no longer exists leaves the fresh dialog alone
		let mut d = mk_dialog(400.0);
		d.restore(super::View {
			tab: tab_titles().len(),
			scroll: 50.0,
		});
		assert_eq!(d.tab, 0);
		assert_eq!(d.scroll, 0.0);
	}

	#[test]
	fn keyboard_focus_walks_controls_then_buttons() {
		use super::Focus;
		let mut d = mk_dialog(2000.0);
		d.tab = 3; // Movement: the smooth-scroll master toggle, then two sliders
		let f = d.focusables();
		assert!(f.len() >= 3, "scrolling tab has focusable rows");
		d.set_mods(false, false, false);
		d.key_tab(); // from nothing -> the master toggle (a single stop)
		assert_eq!(d.focus, Some(Focus::Row(f[0], 0)));
		// each slider is two focus stops (track, then numeric field)
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Row(f[1], 0)));
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Row(f[1], 1)));
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Row(f[2], 0)));
		// after the LAST control the ring visits the three footer buttons
		let last = *f.last().unwrap();
		d.focus = Some(super::Focus::Row(last, d.parts_of(last) - 1));
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Button(0)));
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Button(1)));
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Button(2)));
		d.key_tab(); // wraps back to the first control
		assert_eq!(d.focus, Some(Focus::Row(f[0], 0)));
		d.set_mods(false, true, false); // Shift+Tab walks back (wraps to last button)
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Button(2)));
	}

	#[test]
	fn dual_cursor_row_two_stops_toggle_and_revert() {
		use super::{Focus, Kind};
		let mut d = mk_dialog(2000.0);
		let i = d
			.specs
			.iter()
			.position(
				|s| matches!(s.kind, Kind::Dual { keys, .. } if keys[0] == super::Key::CursorScrim),
			)
			.unwrap();
		d.tab = d.specs[i].tab;
		// enabled prerequisites: scrim on, an outline present
		d.edited.text_scrim = true;
		d.edited.text_outline = 2.0;
		assert_eq!(d.parts_of(i), 2);
		assert!(!d.part_disabled(i, 0) && !d.part_disabled(i, 1));
		// Space on each part flips its own key
		let (s0, o0) = (d.edited.cursor_scrim, d.edited.cursor_outline);
		d.focus = Some(Focus::Row(i, 0));
		d.key_space();
		assert_eq!(d.edited.cursor_scrim, !s0);
		assert_eq!(d.edited.cursor_outline, o0, "part 0 leaves outline alone");
		d.focus = Some(Focus::Row(i, 1));
		d.key_space();
		assert_eq!(d.edited.cursor_outline, !o0);
		// no outline -> the Outline checkbox (part 1) drops out of the focus ring
		d.edited.text_outline = 0.0;
		assert!(d.part_disabled(i, 1) && !d.part_disabled(i, 0));
		// reverting the row restores both keys
		d.edited.text_outline = 2.0;
		d.edited.cursor_scrim = !d.defaults.cursor_scrim;
		d.edited.cursor_outline = !d.defaults.cursor_outline;
		assert!(!d.row_is_default(i));
		d.row_revert(i);
		assert_eq!(d.edited.cursor_scrim, d.defaults.cursor_scrim);
		assert_eq!(d.edited.cursor_outline, d.defaults.cursor_outline);
		assert!(d.row_is_default(i));
		assert!(d.take_reverted().contains(&"cursor.scrim"));
	}

	// The "use system font" face toggle is inert wherever the OS reports no
	// monospace family - always on Windows, and on a desktop with none set. That
	// is a property of the environment, not of the platform, so the test asks the
	// same question the code does.
	#[test]
	fn system_font_toggle_inert_without_an_os_family() {
		use super::Key;
		let mut d = mk_dialog(2000.0);
		let i = d
			.specs
			.iter()
			.position(|s| matches!(s.key, Key::SystemFont))
			.unwrap();
		d.tab = d.specs[i].tab;
		let bx = d.checkbox(i);
		if crate::sysfont::monospace().family.is_none() {
			assert!(d.disabled(Key::SystemFont));
			// Grayed is what says "inert"; the box still shows what is stored, both
			// ways. It used to show off whatever was stored, which put an unchecked
			// box beside an at-default revert arrow - the two disagreeing about a
			// default that is on.
			d.edited.use_system_font = true;
			assert!(d.get_toggle(Key::SystemFont));
			d.edited.use_system_font = false;
			assert!(!d.get_toggle(Key::SystemFont));
			d.edited.use_system_font = d.defaults.use_system_font;
			assert_eq!(
				d.get_toggle(Key::SystemFont),
				d.defaults.use_system_font,
				"the box must show the default it reports"
			);
			assert!(d.is_default(Key::SystemFont));
			// clicking the grayed checkbox must not flip the setting
			let mut measure = |s: &str| s.len() as f32;
			d.mouse_down(bx.x + 2.0, bx.y + 2.0, &mut measure);
			assert!(d.edited.use_system_font);
			// the flyover explains WHY it is grayed, in place of the row's own
			// help text, and only over the row
			assert_eq!(
				d.hover_tip(bx.x + 2.0, bx.y + 2.0).map(|(tip, _)| tip),
				Some("No system monospace font to follow")
			);
			assert!(d.hover_tip(bx.x + 2.0, bx.y - 200.0).is_none());
			// the family field stays editable, since it is what actually resolves
			assert!(!d.disabled(Key::FontFamily));
		} else {
			assert!(!d.disabled(Key::SystemFont));
			// live, so the row explains what it does rather than why it cannot
			assert_ne!(
				d.hover_tip(bx.x + 2.0, bx.y + 2.0).map(|(tip, _)| tip),
				Some("No system monospace font to follow")
			);
			// following the OS grays the field it overrides
			d.edited.use_system_font = true;
			assert!(d.disabled(Key::FontFamily));
			d.edited.use_system_font = false;
			assert!(!d.disabled(Key::FontFamily));
		}
	}

	#[test]
	fn a_large_ui_font_scales_radio_layout_and_widens_panel() {
		use super::Kind;
		// base vs a desktop UI font twice the size (a bigger font, same DPI)
		let base = mk_dialog(4000.0);
		let big = SettingsDialog::new(
			0.0,
			0.0,
			38.0,
			340.0,
			160.0,
			180.0,
			vec![180.0; tab_titles().len()],
			4000.0,
			1.0,
		);
		// radio pitch tracks the font so multi-option labels don't collide
		assert!(big.radio_pitch() > base.radio_pitch() * 1.5);
		// the widest radio's last option stays inside the panel
		let (ri, opts) = big
			.specs
			.iter()
			.enumerate()
			.filter_map(|(i, s)| match s.kind {
				Kind::Radio(o) => Some((i, o.len())),
				_ => None,
			})
			.max_by_key(|(_, n)| *n)
			.unwrap();
		let last = big.radio_box(ri, opts - 1);
		assert!(
			last.x + last.w <= big.rect.x + big.rect.w,
			"last radio option overflows the panel at 2x"
		);
	}

	// The layout is DIP, so a doubled scale factor may only multiply it: same
	// dialog, twice the pixels, and a pointer still lands on the same control.
	#[test]
	fn the_scale_factor_only_multiplies_the_layout() {
		let base = mk_dialog(4000.0);
		let hidpi = mk_dialog_at(4000.0, 2.0);
		let ((bw, bh), (hw, hh)) = (base.size(), hidpi.size());
		assert!(
			(hw - bw * 2.0).abs() < 0.01 && (hh - bh * 2.0).abs() < 0.01,
			"window {bw}x{bh} at 1x vs {hw}x{hh} at 2x"
		);
		let (bv, hv) = (base.viewport_px(), hidpi.viewport_px());
		assert!((hv.y - bv.y * 2.0).abs() < 0.01 && (hv.h - bv.h * 2.0).abs() < 0.01);
		// a click at the checkbox's physical center still toggles its setting
		let i = base
			.specs
			.iter()
			.position(|s| s.key == Key::Transparency)
			.unwrap();
		let target = base.checkbox(i);
		let mut d = hidpi;
		let before = d.edited.transparent_background;
		d.mouse_down(
			(target.x + target.w / 2.0) * 2.0,
			(target.y + target.h / 2.0) * 2.0,
			&mut |s: &str| s.len() as f32 * 12.0,
		);
		assert_ne!(d.edited.transparent_background, before);
	}

	#[test]
	fn dropdown_open_navigate_commit() {
		use super::{Action, Focus, Key, Kind};
		let mut d = mk_dialog(2000.0);
		d.tab = 0;
		d.edited.text_scrim = true; // not grayed out
		let i = d
			.specs
			.iter()
			.position(|s| s.key == Key::ScrimFunction)
			.unwrap();
		assert!(matches!(d.specs[i].kind, Kind::Dropdown(_)));
		d.edited.text_scrim_function = "sdf".into(); // option index 0
		d.focus = Some(Focus::Row(i, 0));
		// Space opens with the current value highlighted
		d.key_space();
		assert_eq!(d.open, Some(i));
		assert_eq!(d.pending, 0);
		// Down moves the highlight but does not commit yet
		d.key_vertical(true);
		assert_eq!(d.pending, 1);
		assert_eq!(
			d.edited.text_scrim_function, "sdf",
			"not committed until Enter"
		);
		// Enter commits + closes
		assert!(matches!(d.key_enter(), Action::None));
		assert_eq!(d.open, None);
		assert_eq!(d.edited.text_scrim_function, "dt"); // index 1
		// reopen, move, Esc -> closes and discards the highlight
		d.key_space();
		d.key_vertical(true);
		assert_eq!(d.key_escape(), Action::None);
		assert_eq!(d.open, None);
		assert_eq!(d.edited.text_scrim_function, "dt");
	}

	#[test]
	fn dropdown_mouse_open_and_pick() {
		use super::Key;
		let mut d = mk_dialog(2000.0);
		d.edited.text_scrim = true;
		let i = d
			.specs
			.iter()
			.position(|s| s.key == Key::ScrimRamp)
			.unwrap();
		d.tab = d.specs[i].tab;
		let n = d.dd_options(i).len();
		let mut m = |_: &str| 8.0;
		// click the collapsed box opens the popup
		let box_r = d.dd_box(i);
		d.mouse_down(box_r.x + 4.0, box_r.y + 4.0, &mut m);
		assert_eq!(d.open, Some(i));
		// click option 2 ("Logarithmic") selects it and closes
		let r = d.dd_item_rect(i, n, 2);
		d.mouse_down(r.x + 4.0, r.y + r.h / 2.0, &mut m);
		assert_eq!(d.open, None);
		assert_eq!(d.edited.text_scrim_ramp, "log");
	}

	#[test]
	fn the_scrolling_feel_sliders_read_where_their_defaults_claim() {
		// Every one of these is documented in the config template as landing on a
		// particular number, and each stored default was picked to land there. A
		// range or a default edited without its comment would drift silently.
		let d = config::Settings::default();
		assert_eq!(tau_to_speed(d.scroll_single_screen_tau_ms), 75.0);
		for (got, want, what) in [
			(
				falling_slider(d.scroll_ease_in_ms, EASE_IN_MIN, EASE_IN_MAX),
				50.0,
				"ease-in",
			),
			(
				falling_slider(d.scroll_ramp_up_ms, RAMP_UP_MIN, RAMP_UP_MAX),
				75.0,
				"ramp-up",
			),
			(
				falling_slider(d.scroll_ramp_down_ms, RAMP_DOWN_MIN, RAMP_DOWN_MAX),
				75.0,
				"ramp-down",
			),
			(
				falling_slider(d.scroll_ease_out_ms, EASE_OUT_MIN, EASE_OUT_MAX),
				40.0,
				"ease-out",
			),
		] {
			assert_eq!(got, want, "{what} default should read where it claims");
		}
	}

	#[test]
	fn the_scrolling_feel_sliders_round_trip_and_run_the_right_way() {
		// A slider that reads back as something else is the "setting does
		// nothing" bug in its quietest form. Also pins the DIRECTION of every
		// feel slider: higher = faster, whichever way each is stored
		// underneath.
		let mut d = mk_dialog(4000.0);
		for (key, label) in [
			(Key::ScrollEaseIn, "Ease-in"),
			(Key::ScrollRampUp, "Ramp-up"),
			(Key::SingleScreenTau, "Single-screen speed"),
			(Key::ScrollRampDown, "Ramp-down"),
			(Key::ScrollEaseOut, "Ease-out"),
		] {
			for want in [1.0, 25.0, 50.0, 75.0, 100.0] {
				d.set_f32(key, want);
				let got = d.get_f32(key);
				assert!(
					(got - want).abs() < 1.5,
					"{label} set to {want} read back as {got}"
				);
			}
		}
		// higher = crisper on both ends of the ease (stored as a shorter duration)
		d.set_f32(Key::ScrollEaseIn, 80.0);
		let crisp_in = d.edited.scroll_ease_in_ms;
		d.set_f32(Key::ScrollEaseIn, 20.0);
		assert!(crisp_in < d.edited.scroll_ease_in_ms);
		d.set_f32(Key::ScrollEaseOut, 80.0);
		let crisp_out = d.edited.scroll_ease_out_ms;
		d.set_f32(Key::ScrollEaseOut, 20.0);
		assert!(
			crisp_out < d.edited.scroll_ease_out_ms,
			"a higher Ease-out must be a SHORTER tail, matching its Ease-in partner"
		);
		// higher = harder on both ramps (stored as a shorter period)
		d.set_f32(Key::ScrollRampUp, 80.0);
		let hard_up = d.edited.scroll_ramp_up_ms;
		d.set_f32(Key::ScrollRampUp, 20.0);
		assert!(hard_up < d.edited.scroll_ramp_up_ms);
		d.set_f32(Key::ScrollRampDown, 80.0);
		let hard_down = d.edited.scroll_ramp_down_ms;
		d.set_f32(Key::ScrollRampDown, 20.0);
		assert!(hard_down < d.edited.scroll_ramp_down_ms);
	}

	#[test]
	fn buttons_fire_on_release_over_button() {
		use super::Action;
		let mut d = mk_dialog(2000.0);
		let (action, r, _) = d.buttons()[1]; // Apply
		assert_eq!(action, Action::Apply);
		let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
		let mut m = |_: &str| 10.0;
		// press arms the button (feedback) without firing
		assert_eq!(d.mouse_down(cx, cy, &mut m), Action::None);
		assert_eq!(d.pressed, Some(1));
		// release over the same button fires its action and disarms
		assert_eq!(d.mouse_up(cx, cy), Action::Apply);
		assert_eq!(d.pressed, None);
		// press then release away from the button cancels (no action)
		d.mouse_down(cx, cy, &mut m);
		assert_eq!(d.mouse_up(cx, r.y - 100.0), Action::None);
		assert_eq!(d.pressed, None);
	}

	#[test]
	fn space_or_enter_activates_focused_button() {
		use super::{Action, Focus};
		let mut d = mk_dialog(2000.0);
		d.focus = Some(Focus::Button(0)); // Cancel
		assert_eq!(d.key_space(), Action::Cancel);
		d.focus = Some(Focus::Button(2)); // OK
		assert_eq!(d.key_enter(), Action::Ok);
	}

	#[test]
	fn keyboard_skips_headers_and_disabled() {
		let mut d = mk_dialog(2000.0);
		d.tab = 0; // Appearance
		// with transparency + scrim off, the opacity/blur/scrim rows are disabled
		d.edited.transparent_background = false;
		d.edited.text_scrim = false;
		for &i in &d.focusables() {
			assert!(!matches!(d.specs[i].kind, super::Kind::Header(_)));
			assert!(!d.disabled(d.specs[i].key), "disabled row in tab order");
		}
	}

	#[test]
	fn space_toggles_focused_boolean() {
		let mut d = mk_dialog(2000.0);
		d.tab = 0;
		d.key_tab(); // first focusable = Transparency (a toggle)
		let before = d.edited.transparent_background;
		d.key_space();
		assert_eq!(d.edited.transparent_background, !before);
	}

	#[test]
	fn arrows_adjust_slider_and_radio() {
		use super::Key;
		let mut d = mk_dialog(2000.0);
		// slider: focus the scroll-speed slider, nudge it both ways
		d.tab = 4;
		d.key_tab();
		let base = d.get_f32(Key::SingleScreenTau);
		d.key_horizontal(-1);
		let lower = d.get_f32(Key::SingleScreenTau);
		assert!(lower <= base);
		d.key_horizontal(1);
		d.key_horizontal(1);
		assert!(d.get_f32(Key::SingleScreenTau) >= lower);
		// radio: focus the (always-enabled) bg-fit radio and move its selection
		let i = d.specs.iter().position(|s| s.key == Key::BgFit).unwrap();
		d.tab = d.specs[i].tab;
		d.focus = Some(super::Focus::Row(i, 0));
		let before = d.get_radio(Key::BgFit);
		d.key_horizontal(1);
		assert!(d.get_radio(Key::BgFit) > before || before == 1);
		d.key_horizontal(-1);
		assert_eq!(d.get_radio(Key::BgFit), 0);
	}

	#[test]
	fn slider_step_matches_spec() {
		use super::slider_step;
		// float: ~1/100 normally, ~1/10 with Shift
		assert!((slider_step(0.0, 1.0, false, false) - 0.01).abs() < 1e-6);
		assert!((slider_step(0.0, 1.0, false, true) - 0.1).abs() < 1e-6);
		// int: rounded to a whole unit, never below 1
		assert_eq!(slider_step(6.0, 40.0, true, false), 1.0); // 34/100 -> 0 -> 1
		assert_eq!(slider_step(20.0, 400.0, true, false), 4.0); // 380/100 -> 4
		assert_eq!(slider_step(20.0, 400.0, true, true), 38.0); // 380/10 -> 38
	}

	#[test]
	fn up_down_step_focused_slider() {
		use super::Key;
		let mut d = mk_dialog(2000.0);
		let i = d
			.specs
			.iter()
			.position(|s| s.key == Key::SingleScreenTau)
			.unwrap();
		d.tab = d.specs[i].tab;
		d.focus = Some(super::Focus::Row(i, 0));
		d.set_f32(Key::SingleScreenTau, 50.0);
		d.key_vertical(false); // Up -> increase by 1 (int step)
		assert_eq!(d.get_f32(Key::SingleScreenTau), 51.0);
		d.key_vertical(true); // Down -> decrease
		d.key_vertical(true);
		assert_eq!(d.get_f32(Key::SingleScreenTau), 49.0);
		d.set_mods(false, true, false); // Shift held
		d.key_vertical(false); // Shift+Up -> ~1/10 of the range (10)
		assert_eq!(d.get_f32(Key::SingleScreenTau), 59.0);
	}

	#[test]
	fn up_down_step_slider_during_edit() {
		use super::Key;
		let mut d = mk_dialog(2000.0);
		let i = d
			.specs
			.iter()
			.position(|s| s.key == Key::SingleScreenTau)
			.unwrap();
		d.tab = d.specs[i].tab;
		d.focus = Some(super::Focus::Row(i, 0));
		d.set_f32(Key::SingleScreenTau, 30.0);
		d.key_space(); // open the field, fully selected
		assert!(d.edit.is_some());
		d.key_vertical(false); // Up steps the value and refreshes the buffer
		assert_eq!(d.get_f32(Key::SingleScreenTau), 31.0);
		assert_eq!(d.edit.as_ref().unwrap().buf, "31");
		assert_eq!(d.selected_text().as_deref(), Some("31")); // stays fully selected
	}

	#[test]
	fn fresh_click_selects_all_but_drag_keeps_range() {
		use super::Key;
		let i0 = mk_dialog(4000.0)
			.specs
			.iter()
			.position(|s| s.key == Key::BgImage)
			.unwrap();
		let mut m = |s: &str| s.chars().count() as f32; // 1px per char
		// fresh single click into a text field: select all on release
		let mut d = mk_dialog(4000.0);
		d.tab = d.specs[i0].tab;
		d.edited.wallpaper_raw = "foo bar.png".to_string();
		let field = d.textbox(i0);
		let at = |k: usize| field.x + 6.0 + k as f32;
		let y = field.y + field.h / 2.0;
		d.mouse_down(at(2), y, &mut m);
		assert!(d.edit.is_some(), "click opens the field");
		assert!(d.selected_text().is_none(), "not selected until release");
		d.mouse_up(at(2), y);
		assert_eq!(
			d.selected_text().as_deref(),
			Some("foo bar.png"),
			"a no-drag click selects all"
		);
		// a click that drags selects the dragged range instead
		let mut d = mk_dialog(4000.0);
		d.tab = d.specs[i0].tab;
		d.edited.wallpaper_raw = "foo bar.png".to_string();
		d.mouse_down(at(2), y, &mut m);
		d.mouse_move(at(6), y, &mut m);
		d.mouse_up(at(6), y);
		assert_eq!(
			d.selected_text().as_deref(),
			Some("o ba"),
			"a drag keeps its range"
		);
	}

	// All four tab chords, in both directions, and the plain keys they must not
	// steal: Tab alone walks controls, and PageUp/PageDown alone do nothing here.
	#[test]
	fn ctrl_tab_and_ctrl_page_walk_the_tabs_both_ways() {
		let mut d = mk_dialog(2000.0);
		let last = tab_titles().len() - 1;
		d.set_mods(false, false, true); // Ctrl held
		d.key_tab();
		assert_eq!(d.tab, 1);
		assert!(d.focus.is_some(), "a tab switch lands focus on a control");
		d.set_mods(false, true, true); // Ctrl+Shift
		d.key_tab();
		assert_eq!(d.tab, 0);
		d.key_tab();
		assert_eq!(d.tab, last, "and wraps round the far end");

		d.set_mods(false, false, true);
		d.key_page(true);
		assert_eq!(d.tab, 0, "PageDown is forward, wrapping");
		d.key_page(false);
		assert_eq!(d.tab, last);

		// without Ctrl these are not tab keys at all
		d.tab = 1;
		d.set_mods(false, false, false);
		d.key_page(true);
		d.key_page(false);
		assert_eq!(d.tab, 1);
		d.key_tab();
		assert_eq!(d.tab, 1, "plain Tab walks controls, not tabs");
	}

	#[test]
	fn slider_numeric_field_edits_and_clamps() {
		use super::{Focus, Key};
		let mut d = mk_dialog(2000.0);
		// Font size: an int slider on the Font tab, range 6..40
		d.edited.use_system_font_size = false; // else Font size is grayed/disabled
		let i = d.specs.iter().position(|s| s.key == Key::FontSize).unwrap();
		d.tab = d.specs[i].tab;
		d.focus = Some(Focus::Row(i, 0));
		// Space opens the field pre-filled with the current value
		d.key_space();
		assert!(d.edit.is_some());
		// clear it and type an exact number
		while d.edit.as_ref().is_some_and(|e| !e.buf.is_empty()) {
			d.backspace();
		}
		d.char_input('2');
		d.char_input('4');
		assert_eq!(d.edited.font_size, 24.0);
		// over-range types clamp to the slider max (40)
		while d.edit.as_ref().is_some_and(|e| !e.buf.is_empty()) {
			d.backspace();
		}
		d.char_input('9');
		d.char_input('9');
		assert_eq!(d.edited.font_size, 40.0);
		// Enter commits; field closes and shows the clamped value
		assert_eq!(d.key_enter(), super::Action::None);
		assert!(d.edit.is_none());
	}

	#[test]
	fn slider_field_typing_starts_fresh_and_rejects_letters() {
		use super::{Focus, Key};
		let mut d = mk_dialog(2000.0);
		// Line height: a slider that really is a decimal, so the dot is legal
		let i = d
			.specs
			.iter()
			.position(|s| s.key == Key::LineHeight)
			.unwrap();
		d.tab = d.specs[i].tab;
		d.focus = Some(Focus::Row(i, 0));
		// typing a digit into the focused (unopened) slider starts a fresh number
		d.char_input('1');
		d.char_input('.');
		d.char_input('2');
		assert_eq!(d.edited.line_height_scale, 1.2);
		// a second '.' and any letter are ignored (buffer stays "1.2")
		d.char_input('.');
		d.char_input('x');
		assert_eq!(d.edit.as_ref().unwrap().buf, "1.2");
		// a fraction shown as a whole percent takes no dot at all - it is an
		// integer field, and 0.5 typed into one would read as half a percent
		d.commit_edit();
		let j = d.specs.iter().position(|s| s.key == Key::Opacity).unwrap();
		d.tab = d.specs[j].tab;
		d.edited.transparent_background = true; // opacity enabled
		d.focus = Some(Focus::Row(j, 0));
		d.char_input('5');
		d.char_input('.');
		d.char_input('0');
		assert_eq!(d.edit.as_ref().unwrap().buf, "50");
		assert_eq!(d.edited.opacity, 0.5);
	}

	#[test]
	fn caret_from_click_picks_nearest() {
		let mut m = |s: &str| s.chars().count() as f32; // 1 unit per ascii char
		assert_eq!(super::caret_from_click("hello", -5.0, &mut m), 0);
		assert_eq!(super::caret_from_click("hello", 0.0, &mut m), 0);
		assert_eq!(super::caret_from_click("hello", 2.4, &mut m), 2);
		assert_eq!(super::caret_from_click("hello", 100.0, &mut m), 5);
	}

	#[test]
	fn word_motion_and_word_at() {
		let s = "foo bar_baz/qux.png";
		assert_eq!(super::word_left(s, 7), 4); // inside bar_baz -> its start
		assert_eq!(super::word_left(s, 4), 0); // at bar_baz -> foo start
		assert_eq!(super::word_right(s, 0), 3); // foo end
		assert_eq!(super::word_right(s, 3), 11); // past the space, bar_baz end
		assert_eq!(super::word_at(s, 5), (4, 11)); // bar_baz
		assert_eq!(super::word_at(s, 3), (3, 4)); // the separator run
		assert_eq!(super::word_at("", 0), (0, 0));
		assert_eq!(super::word_at(s, s.len()), (16, 19)); // clamps to last word (png)
	}

	// open the Background image text field for editing, focused, with a value
	fn mk_text_edit(value: &str) -> (SettingsDialog, usize) {
		use super::{Focus, Key};
		let mut d = mk_dialog(4000.0);
		let i = d.specs.iter().position(|s| s.key == Key::BgImage).unwrap();
		d.tab = d.specs[i].tab;
		d.edited.wallpaper_raw = value.to_string();
		d.focus = Some(Focus::Row(i, 0));
		d.set_mods(false, false, false);
		d.key_space(); // opens with the value fully selected
		(d, i)
	}

	// The same, with a value comfortably wider than the field it sits in - at the
	// 1px-per-char measure these tests use. Derived from the field rather than
	// hardcoded, or a wider panel (a new tab, a longer label) quietly makes the
	// scrolling case untestable instead of failing.
	fn mk_long_text_edit(fill: char) -> (SettingsDialog, usize, usize) {
		let (probe, i) = mk_text_edit("");
		let n = probe.textbox(i).w as usize + 100;
		let (d, i) = mk_text_edit(&fill.to_string().repeat(n));
		(d, i, n)
	}

	#[test]
	fn open_selects_all_and_typing_replaces() {
		let (mut d, _) = mk_text_edit("old.png");
		assert_eq!(d.selected_text().as_deref(), Some("old.png"));
		d.char_input('n');
		assert_eq!(d.edit.as_ref().unwrap().buf, "n");
		assert_eq!(d.edited.wallpaper_raw, "n"); // live reparse
		// plain arrows collapse; shift+arrows extend a fresh selection
		d.char_input('e');
		d.char_input('w');
		d.set_mods(false, true, false);
		d.cursor_left();
		d.cursor_left();
		assert_eq!(d.selected_text().as_deref(), Some("ew"));
		// backspace removes the selection only
		d.set_mods(false, false, false);
		d.backspace();
		assert_eq!(d.edit.as_ref().unwrap().buf, "n");
	}

	#[test]
	fn ctrl_word_nav_and_word_delete() {
		let (mut d, _) = mk_text_edit("foo bar.png");
		d.cursor_end(); // also collapses the open-time selection
		d.set_mods(false, false, true); // Ctrl
		d.cursor_left(); // to "png" start
		assert_eq!(d.edit.as_ref().unwrap().cur, 8);
		d.backspace(); // Ctrl+Backspace eats "bar." ... no - the word left of caret
		assert_eq!(d.edit.as_ref().unwrap().buf, "foo png");
		// Ctrl never types (shortcut chars must not land in the buffer)
		d.char_input('c');
		assert_eq!(d.edit.as_ref().unwrap().buf, "foo png");
		// Ctrl+Shift+Right extends by a word
		d.set_mods(false, true, true);
		d.cursor_right();
		assert_eq!(d.selected_text().as_deref(), Some("png"));
	}

	#[test]
	fn select_all_cut_paste_roundtrip() {
		let (mut d, _) = mk_text_edit("keep me");
		d.cursor_end();
		d.select_all();
		assert_eq!(d.selected_text().as_deref(), Some("keep me"));
		d.delete_selection(); // the "cut" half (clipboard handled a level up)
		assert_eq!(d.edit.as_ref().unwrap().buf, "");
		assert_eq!(d.edited.wallpaper_raw, "");
		d.insert_str("pasted.png");
		assert_eq!(d.edited.wallpaper_raw, "pasted.png");
		// pasting over a selection replaces it
		d.select_all();
		d.insert_str("x");
		assert_eq!(d.edit.as_ref().unwrap().buf, "x");
	}

	#[test]
	fn paste_respects_field_validation() {
		use super::{Focus, Key, Kind};
		// color field: hex chars pass, junk drops, '#' only up front
		let mut d = mk_dialog(4000.0);
		let i = d
			.specs
			.iter()
			.position(|s| matches!(s.kind, Kind::Color))
			.unwrap();
		d.tab = d.specs[i].tab;
		d.focus = Some(Focus::Row(i, 0));
		d.key_space();
		d.select_all();
		d.insert_str("#a0b1c2");
		assert_eq!(d.edit.as_ref().unwrap().buf, "#a0b1c2");
		d.select_all();
		d.insert_str("zz#12 34-56");
		assert_eq!(d.edit.as_ref().unwrap().buf, "#123456");
		// slider field: digits/dot only, single dot
		let mut d = mk_dialog(4000.0);
		let i = d
			.specs
			.iter()
			.position(|s| s.key == Key::LineHeight)
			.unwrap();
		d.tab = d.specs[i].tab;
		d.focus = Some(Focus::Row(i, 0));
		d.key_space();
		d.select_all();
		d.insert_str("1.2.5x");
		assert_eq!(d.edit.as_ref().unwrap().buf, "1.25");
	}

	#[test]
	fn mouse_click_drag_and_multiclick_select() {
		let (mut d, i) = mk_text_edit("foo bar.png");
		let field = d.textbox(i);
		let mut m = |s: &str| s.chars().count() as f32; // 1px per char
		let at = |k: usize| field.x + 6.0 + k as f32;
		let y = field.y + field.h / 2.0;
		// single click: caret there, no selection
		d.mouse_down(at(2), y, &mut m);
		assert_eq!(d.edit.as_ref().unwrap().cur, 2);
		assert!(d.selected_text().is_none());
		// drag to char 6 selects "o ba"
		d.mouse_move(at(6), y, &mut m);
		d.mouse_up(at(6), y);
		assert_eq!(d.selected_text().as_deref(), Some("o ba"));
		// double-click on "bar" selects the word (streak reset: the 1-unit-per-
		// char test metric puts every click inside the multi-click radius)
		d.last_click = None;
		d.mouse_down(at(5), y, &mut m);
		d.mouse_up(at(5), y);
		d.mouse_down(at(5), y, &mut m);
		assert_eq!(d.selected_text().as_deref(), Some("bar"));
		d.mouse_up(at(5), y);
		// third click in place: the whole value
		d.mouse_down(at(5), y, &mut m);
		assert_eq!(d.selected_text().as_deref(), Some("foo bar.png"));
		d.mouse_up(at(5), y);
		// shift+click extends from a plain caret
		d.last_click = None;
		d.mouse_down(at(0), y, &mut m);
		d.mouse_up(at(0), y);
		d.last_click = None;
		d.set_mods(false, true, false);
		d.mouse_down(at(3), y, &mut m);
		assert_eq!(d.selected_text().as_deref(), Some("foo"));
	}

	#[test]
	fn scroll_speed_inverts_tau() {
		// endpoints: slowest tau = slowest speed, fastest tau = fastest speed
		assert_eq!(tau_to_speed(TAU_MAX), 1.0);
		assert_eq!(tau_to_speed(TAU_MIN), 100.0);
		// higher speed -> lower tau (faster)
		assert!(speed_to_tau(100.0) < speed_to_tau(1.0));
		// round-trips within slider rounding (log scale: error is proportional)
		for tau in [10.0f32, 75.0, 150.0, 300.0, 1000.0] {
			let rt = speed_to_tau(tau_to_speed(tau));
			assert!((rt - tau).abs() <= tau * 0.03, "tau {tau} -> {rt}");
		}
	}

	// settle the field-edit animation (view/caret eases converge)
	fn settle(d: &mut SettingsDialog, m: &mut impl FnMut(&str) -> f32) {
		for _ in 0..200 {
			d.animate(0.016, m);
		}
	}

	#[test]
	fn long_value_scrolls_to_keep_caret_visible() {
		use super::lay;
		let (mut d, i, n) = mk_long_text_edit('x');
		let mut m = |s: &str| s.chars().count() as f32; // 1px per char
		d.cursor_end(); // collapse the open-time selection, caret at the last char
		settle(&mut d, &mut m);
		let field = d.textbox(i);
		let inner = field.w - 2.0 * lay().field_pad;
		let e = d.edit.as_ref().unwrap();
		// scrolled right, caret in view, with the end padding visible after it
		assert!(e.view_to > 0.0);
		assert!((n as f32 - e.view) <= inner - lay().caret_pad + 0.5);
		assert_eq!(e.view, e.view_to, "ease settles exactly on the target");
		// moving left keeps the lookahead margin of context before the caret
		for _ in 0..200 {
			d.cursor_left();
		}
		settle(&mut d, &mut m);
		let e = d.edit.as_ref().unwrap();
		assert!(
			(n - 200) as f32 - e.view_to >= 27.0,
			"margin ahead of leftward travel"
		);
		// Home scrolls all the way back
		d.cursor_home();
		settle(&mut d, &mut m);
		assert_eq!(d.edit.as_ref().unwrap().view_to, 0.0);
	}

	#[test]
	fn short_value_never_scrolls() {
		let (mut d, _) = mk_text_edit("short.png");
		let mut m = |s: &str| s.chars().count() as f32;
		d.cursor_end();
		settle(&mut d, &mut m);
		assert_eq!(d.edit.as_ref().unwrap().view_to, 0.0);
	}

	#[test]
	fn click_and_drag_map_through_the_view() {
		use super::lay;
		let (mut d, i, n) = mk_long_text_edit('y');
		let mut m = |s: &str| s.chars().count() as f32;
		d.cursor_end();
		settle(&mut d, &mut m);
		let view = d.edit.as_ref().unwrap().view;
		assert!(view > 0.0);
		let field = d.textbox(i);
		let y = field.y + field.h / 2.0;
		// a click 10px into the box lands on the char 10px past the scrolled-off part
		d.last_click = None;
		d.mouse_down(field.x + lay().field_pad + 10.0, y, &mut m);
		let cur = d.edit.as_ref().unwrap().cur;
		assert!(
			(cur as f32 - (view + 10.0)).abs() <= 0.5,
			"cur {cur} vs view {view}"
		);
		d.mouse_up(field.x + lay().field_pad + 10.0, y);
		// from the far left, dragging past the right edge keeps selecting while
		// the view crawls (edge autoscroll)
		d.cursor_home();
		settle(&mut d, &mut m);
		d.last_click = None;
		d.mouse_down(field.x + lay().field_pad, y, &mut m);
		d.mouse_move(field.x + field.w + 40.0, y, &mut m);
		let cur0 = d.edit.as_ref().unwrap().cur;
		assert!(cur0 < n, "the first drag event lands short of the end");
		settle(&mut d, &mut m);
		d.mouse_up(field.x + field.w + 40.0, y);
		let e = d.edit.as_ref().unwrap();
		assert!(e.cur > cur0, "edge autoscroll extends the selection");
		assert!(e.view > 0.0, "view followed the drag");
		assert!(d.selected_text().is_some());
	}

	#[test]
	fn context_menu_open_fire_and_gating() {
		use super::{Action, EditCmd, lay};
		let (mut d, i) = mk_text_edit("hello world");
		let mut m = |s: &str| s.chars().count() as f32;
		let field = d.textbox(i);
		let y = field.y + field.h / 2.0;
		// right-click inside the (select-all) selection keeps it; menu opens
		d.mouse_right(field.x + lay().field_pad + 3.0, y, true, &mut m);
		assert!(d.emenu.is_some());
		assert_eq!(d.selected_text().as_deref(), Some("hello world"));
		// Copy is enabled; clicking it returns the command for the clipboard glue
		assert!(d.em_enabled(1));
		let r = d.em_item_rect(1);
		d.last_click = None;
		let act = d.mouse_down(r.x + 2.0, r.y + 2.0, &mut m);
		assert_eq!(act, Action::Edit(EditCmd::Copy));
		assert!(d.emenu.is_none());
		// no selection + empty clipboard: only Select all stays enabled
		d.cursor_end();
		d.mouse_right(field.x + lay().field_pad + 3.0, y, false, &mut m);
		assert!(
			d.selected_text().is_none(),
			"right-click outside sel places caret"
		);
		assert!(!d.em_enabled(0) && !d.em_enabled(1) && !d.em_enabled(2) && !d.em_enabled(3));
		assert!(d.em_enabled(4));
		// keyboard: walk to Select all, Enter fires it
		for _ in 0..5 {
			d.key_vertical(true);
		}
		assert_eq!(d.key_enter(), Action::Edit(EditCmd::SelectAll));
		assert!(d.emenu.is_none());
		// Esc closes the menu but keeps the edit alive
		d.mouse_right(field.x + lay().field_pad + 3.0, y, true, &mut m);
		assert!(d.emenu.is_some());
		assert_eq!(d.key_escape(), Action::None);
		assert!(d.emenu.is_none() && d.edit.is_some());
		// typing dismisses a stale menu
		d.mouse_right(field.x + lay().field_pad + 3.0, y, true, &mut m);
		d.char_input('a');
		assert!(d.emenu.is_none());
	}

	#[test]
	fn blink_holds_solid_on_activity() {
		let (mut d, _) = mk_text_edit("abc");
		let mut m = |s: &str| s.chars().count() as f32;
		settle(&mut d, &mut m); // ~3.2s idle: blink well past the hold
		assert!(d.edit.as_ref().unwrap().blink_t > 1.0);
		d.char_input('z');
		d.animate(0.016, &mut m);
		let e = d.edit.as_ref().unwrap();
		assert!(e.blink_t < 0.1);
		assert_eq!(e.caret_alpha(), 1.0);
	}

	// ---- themes ---------------------------------------------------------------

	fn theme_row(d: &SettingsDialog) -> usize {
		d.specs
			.iter()
			.position(|s| matches!(s.kind, super::Kind::Buttons(_)))
			.expect("the theme actions row")
	}
	// Put the dialog on a known theme with no color overrides on top.
	fn on_theme(name: &str) -> SettingsDialog {
		let mut d = mk_dialog(4000.0);
		d.edited = config::Settings::default();
		d.edited.theme = name.to_string();
		d.adopt_theme();
		d.orig = d.edited.clone();
		d.reverted.clear(); // adopting queues them; start each test from nothing pending
		d
	}

	// Nothing anywhere records "this theme has unsaved changes" - a color that
	// disagrees with the theme IS the record, and it lives in the config file, so
	// the answer is the same after a restart.
	#[test]
	fn an_edited_colour_is_what_makes_the_theme_dirty() {
		let mut d = on_theme("Matrix");
		let row = theme_row(&d);
		assert!(!d.theme_dirty());
		assert!(!d.theme_btn_enabled(super::ThemeBtn::Save));
		assert!(d.part_disabled(row, 0), "Save starts grayed");

		d.set_col(Key::ColFg, [1, 2, 3]);
		assert!(d.theme_dirty());
		assert!(!d.part_disabled(row, 0), "Save wakes up on an edit");
		// Save as is always available; the other two need a theme of the user's own
		assert!(!d.part_disabled(row, 1));
		assert!(d.part_disabled(row, 2), "Rename needs a saved theme");
		assert!(d.part_disabled(row, 3), "Delete needs a saved theme");
	}

	// Saving folds the edits into the theme itself, so the per-color overrides
	// have nothing left to say and are queued to be commented back out.
	#[test]
	fn saving_folds_the_edits_into_the_theme() {
		let mut d = on_theme("Matrix");
		d.set_col(Key::ColFg, [1, 2, 3]);
		d.save_theme_as("Mine");

		assert_eq!(d.edited.theme, "Mine");
		let saved = crate::theme::find_user(&d.edited.user_themes, "Mine").expect("saved");
		assert_eq!(saved.dark.fg, [1, 2, 3]);
		// the mode the dialog was NOT showing still comes out complete
		assert_eq!(
			saved.light.fg,
			crate::theme::resolve("Matrix", "light", true).fg
		);
		// and the ANSI set came along, so the theme stands on its own
		assert_eq!(
			saved.dark.ansi,
			crate::theme::resolve("Matrix", "dark", true).ansi
		);

		assert!(!d.theme_dirty(), "the edit is the theme's own color now");
		assert!(
			d.reverted.contains(&"colors.foreground"),
			"the override is queued for removal"
		);
	}

	// Saving grays Save out, so the control the keyboard was on drops out of the
	// Tab ring. Focus has to carry on to the next button rather than snapping
	// back to the first control on the tab.
	#[test]
	fn focus_carries_on_when_the_control_under_it_greys_out() {
		let mut d = on_theme("Matrix");
		let row = theme_row(&d);
		d.tab = d.specs[row].tab;
		d.set_col(Key::ColFg, [1, 2, 3]);
		d.focus = Some(super::Focus::Row(row, 0));
		d.theme_action(super::ThemeBtn::Save);
		assert!(d.part_disabled(row, 0), "Save grays out once it has saved");
		d.focus_move(true);
		assert_eq!(d.focus, Some(super::Focus::Row(row, 1)));
		// and backwards off the same gap lands on the row above, not the last button
		d.focus = Some(super::Focus::Row(row, 0));
		d.focus_move(false);
		assert!(matches!(d.focus, Some(super::Focus::Row(i, _)) if i < row));
	}

	// A theme may take a built-in's name and stand in for it; deleting it puts the
	// built-in back rather than leaving the name pointing at nothing.
	#[test]
	fn a_saved_theme_shadows_a_builtin_and_delete_uncovers_it() {
		let mut d = on_theme("Matrix");
		let builtin_fg = d.get_col(Key::ColFg);
		d.set_col(Key::ColFg, [9, 9, 9]);
		d.save_theme_as("Matrix");
		assert_eq!(d.get_col(Key::ColFg), [9, 9, 9]);
		// it is the user's theme now, so it can be renamed or thrown away
		let row = theme_row(&d);
		assert!(!d.part_disabled(row, 2) && !d.part_disabled(row, 3));
		// the name is listed once, not twice
		let names = crate::theme::all_names(&d.edited.user_themes);
		assert_eq!(names.iter().filter(|n| *n == "Matrix").count(), 1);

		d.delete_theme();
		assert_eq!(d.edited.theme, "Matrix");
		assert_eq!(d.get_col(Key::ColFg), builtin_fg);
	}

	// Picking a theme takes on its colors. Keeping the old theme's tweaks would
	// make the picker look broken on every color that had been edited.
	#[test]
	fn picking_a_theme_adopts_its_colours() {
		let mut d = on_theme("SilkTerm");
		d.set_col(Key::ColFg, [1, 2, 3]);
		let i = d
			.specs
			.iter()
			.position(|s| s.key == Key::Theme)
			.expect("the theme row");
		let names = d.dd_options(i);
		let k = names.iter().position(|n| n == "Matrix").expect("Matrix");
		d.set_radio(Key::Theme, k);

		assert_eq!(d.edited.theme, "Matrix");
		assert_eq!(
			d.get_col(Key::ColFg),
			crate::theme::resolve("Matrix", "dark", true).fg
		);
		assert!(!d.theme_dirty(), "a fresh theme starts unmodified");
	}

	// Picking a theme must not leave the old palette behind as colors.* overrides.
	// Those would be written as active lines and then outrank every later theme
	// change, which also freezes one variant when the mode follows the desktop.
	#[test]
	fn adopting_a_theme_clears_the_colour_overrides() {
		let mut d = on_theme("SilkTerm");
		d.set_col(Key::ColFg, [1, 2, 3]);
		d.set_col(Key::ColBg, [4, 5, 6]);
		d.reverted.clear();

		d.edited.theme = "Matrix".to_string();
		d.adopt_theme();

		let pending = d.take_reverted();
		for i in 0..crate::theme::PALETTE_KEYS.len() {
			for cfg_key in super::ui().settings_of(SettingsDialog::palette_key(i)) {
				assert!(
					pending.contains(cfg_key),
					"{cfg_key} must be commented out on Apply"
				);
			}
		}
		assert!(!d.theme_dirty(), "the adopted palette is not an edit");
	}

	// The box is modal by gate, and the gate is a list every input path has to be
	// on. These four were missed once: the accelerators applied and closed the
	// dialog through the box, and typing edited the row sitting behind it.
	#[test]
	fn the_prompt_swallows_every_input_path() {
		let mut m = |s: &str| s.chars().count() as f32;
		for which in [super::ThemeBtn::SaveAs, super::ThemeBtn::Delete] {
			let mut d = on_theme("Matrix");
			d.save_theme_as("Mine"); // Rename and Delete need a theme of the user's own
			d.reverted.clear();
			d.focus = Some(super::Focus::Row(
				d.specs.iter().position(|s| s.key == Key::ColFg).unwrap(),
				0,
			));
			let before = d.get_col(Key::ColFg);
			d.theme_action(which);
			assert!(d.prompt.is_some(), "{which:?} opens the box");

			for c in ['o', 'a', 'c'] {
				assert_eq!(
					d.alt_key(c),
					super::Action::None,
					"Alt+{c} must not reach OK"
				);
			}
			d.char_input('f');
			d.select_all();
			d.mouse_right(d.rect.x + 4.0, d.rect.y + d.rect.h - 4.0, true, &mut m);

			assert!(d.prompt.is_some(), "the box is still up");
			assert_eq!(d.get_col(Key::ColFg), before, "the row behind is untouched");
			assert!(
				d.edit.as_ref().is_none_or(|e| e.row == super::PROMPT_ROW),
				"no edit opened on a panel row"
			);
		}
	}

	// Renaming moves the name and the selection together; the slug behind it does
	// not move, so the config subtree stays where it is.
	#[test]
	fn a_rename_moves_the_name_and_the_selection() {
		let mut d = on_theme("Matrix");
		d.set_col(Key::ColFg, [4, 5, 6]);
		d.save_theme_as("Mine");
		let slug = d.edited.user_themes[0].slug.clone();

		d.rename_theme("Ours");
		assert_eq!(d.edited.theme, "Ours");
		assert_eq!(d.edited.user_themes[0].slug, slug);
		assert_eq!(d.get_col(Key::ColFg), [4, 5, 6]);

		// two themes cannot share a name, or one would swallow the other
		d.save_theme_as("Theirs");
		assert_eq!(d.edited.user_themes.len(), 2);
		assert!(d.name_problem(super::ThemeBtn::Rename, "ours").is_some());
		assert!(d.name_problem(super::ThemeBtn::Rename, "  ").is_some());
		assert!(d.name_problem(super::ThemeBtn::Rename, "Third").is_none());
		// Save as over an existing name replaces it, which is a fair reading
		assert!(d.name_problem(super::ThemeBtn::SaveAs, "ours").is_none());
	}

	// The prompt box takes the keyboard while it is up, and Esc leaves the theme
	// exactly as it was.
	#[test]
	fn the_prompt_box_owns_the_keyboard_until_it_closes() {
		let mut d = on_theme("Matrix");
		d.set_col(Key::ColFg, [7, 7, 7]);
		d.theme_action(super::ThemeBtn::SaveAs);
		assert!(d.prompt.is_some() && d.edit.is_some());

		for c in "My Theme".chars() {
			d.char_input(c);
		}
		// Esc closes the box, not the dialog, and saves nothing
		assert_eq!(d.key_escape(), super::Action::None);
		assert!(d.prompt.is_none() && d.edit.is_none());
		assert!(d.edited.user_themes.is_empty());

		// again, this time through OK
		d.theme_action(super::ThemeBtn::SaveAs);
		for c in "My Theme".chars() {
			d.char_input(c);
		}
		assert_eq!(d.key_enter(), super::Action::None);
		assert!(d.prompt.is_none());
		assert_eq!(d.edited.theme, "My Theme");
		assert_eq!(
			crate::theme::find_user(&d.edited.user_themes, "My Theme")
				.unwrap()
				.dark
				.fg,
			[7, 7, 7]
		);
	}

	// A name OK cannot take keeps the box open and says why, instead of closing
	// and quietly doing nothing.
	#[test]
	fn a_name_it_cannot_take_keeps_the_box_open() {
		let mut d = on_theme("Matrix");
		d.save_theme_as("Mine");
		d.theme_action(super::ThemeBtn::Rename);
		d.select_all();
		for c in "   ".chars() {
			d.char_input(c);
		}
		d.prompt_accept();
		assert!(d.prompt.is_some(), "still asking");
		assert!(d.prompt.as_ref().unwrap().warn.is_some(), "and saying why");
		// typing again clears the complaint
		d.char_input('x');
		assert!(d.prompt.as_ref().unwrap().warn.is_none());
	}

	// A saved theme has to come back after a restart, or saving it meant nothing.
	#[test]
	fn a_saved_theme_survives_a_relaunch() {
		let _guard = config::test_config_lock();
		let _ = config::settings();
		let dir = std::env::temp_dir().join(format!("silkterm_theme_{}", std::process::id()));
		let _ = std::fs::create_dir_all(&dir);
		let path = dir.join("config.shcl");
		let _ = std::fs::write(&path, "");
		config::set_config_override(path.clone());
		let base = config::reload_from_disk();

		let mut d = mk_dialog(4000.0);
		d.orig = base.clone();
		d.edited = base.clone();
		d.edited.theme = "Matrix".into();
		d.adopt_theme();
		d.set_col(Key::ColFg, [0x12, 0x34, 0x56]);
		d.save_theme_as("Saved One");
		assert!(config::persist(&base, &d.edited));

		let back = config::reload_from_disk();
		let saved = crate::theme::find_user(&back.user_themes, "Saved One").expect("on disk");
		assert_eq!(
			// which variant an edit lands in follows the mode, so name it: a mode
			// arriving out of somebody else's config file is what this last went
			// wrong as, and the colors alone do not say that
			saved.dark.fg,
			[0x12, 0x34, 0x56],
			"mode {:?}",
			d.edited.theme_mode
		);
		assert_eq!(
			saved.dark.ansi,
			crate::theme::resolve("Matrix", "dark", true).ansi
		);
		assert_eq!(back.theme, "Saved One");
		assert_eq!(
			back.fg,
			[0x12, 0x34, 0x56],
			"and it is what the terminal uses"
		);

		// deleting it takes the whole subtree with it
		let mut d2 = mk_dialog(4000.0);
		d2.orig = back.clone();
		d2.edited = back.clone();
		d2.delete_theme();
		assert!(config::persist(&back, &d2.edited));
		let after = config::reload_from_disk();
		assert!(after.user_themes.is_empty(), "gone from the file");
		assert!(
			!std::fs::read_to_string(&path)
				.unwrap()
				.contains("Saved One"),
			"and nothing of it is left behind"
		);

		let _ = std::fs::remove_dir_all(&dir);
	}
}
