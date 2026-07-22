// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

//! Modal settings dialog: sliders for numeric tunables, swatch + hex field for
//! colors, toggles, few-option radios, dropdown list boxes for longer enums, and
//! Cancel / Apply / OK. Edits a working copy of `Settings`; the app reads it back
//! on Apply/OK to live-apply + persist. Renders as flat quads (rects) + positioned
//! text; an open dropdown's popup draws in a second (`LoadOp::Load`) pass on top so
//! covered rows' text can't bleed through it (see `dropdown_overlay`).
//!
//! Sections are grouped into tabs (see `TAB_TITLES`/`tab_for_section`) so the
//! dialog stays well under screen height; if a tab still doesn't fit (huge UI
//! font / short screen) the rows region scrolls (wheel + draggable thumb) and
//! the window height is capped instead of clipping the buttons.

use crate::config::{self, Settings};
use crate::gfx::RectInstance;
use crate::pane::Rect;

const W: f32 = 540.0;
const PAD: f32 = 18.0;
const ROW_H: f32 = 32.0;
const HEADER_H: f32 = 42.0; // a section heading row (extra top spacing + gap to its rule)
const LABEL_W: f32 = 168.0;
const SLIDER_W: f32 = 220.0;
const SWATCH: f32 = 20.0;
const HEX_W: f32 = 92.0;
const VAL_W: f32 = 56.0; // editable numeric field to the right of a slider
const BTN_H: f32 = 30.0;
const BTN_W: f32 = 76.0;
const BTN_GAP: f32 = 10.0;

// Dialog colours adapt to the active mode (dark-gray for dark, light-gray for
// light); see config::is_dark(). The menu/main-window chrome stays a fixed gray.
struct Dlg {
	panel_bg: [u8; 3],
	panel_border: [u8; 3],
	track: [u8; 3],
	handle: [u8; 3],
	field_bg: [u8; 3],
	focus_out: [u8; 3],
	btn_bg: [u8; 3],
	btn_hl: [u8; 3],
	text: [u8; 3],
	dim: [u8; 3],
}
#[rustfmt::skip]
const DARK_DLG: Dlg = Dlg {
	panel_bg: [0x20, 0x20, 0x2a], panel_border: [0x50, 0x50, 0x60],
	track: [0x14, 0x14, 0x1c], handle: [0x7a, 0x9a, 0xd0],
	field_bg: [0x14, 0x14, 0x1c], focus_out: [0x7a, 0x9a, 0xd0],
	btn_bg: [0x34, 0x34, 0x40], btn_hl: [0x4a, 0x6a, 0x9a],
	text: [0xe2, 0xe2, 0xea], dim: [0x9a, 0x9a, 0xa6],
};
#[rustfmt::skip]
const LIGHT_DLG: Dlg = Dlg {
	panel_bg: [0xe6, 0xe6, 0xe3], panel_border: [0xb2, 0xb2, 0xb6],
	track: [0xcf, 0xcf, 0xcc], handle: [0x4a, 0x6a, 0xa8],
	field_bg: [0xf8, 0xf8, 0xf6], focus_out: [0x3a, 0x6a, 0xc0],
	btn_bg: [0xd6, 0xd6, 0xd2], btn_hl: [0x9a, 0xb6, 0xe0],
	text: [0x22, 0x24, 0x2c], dim: [0x70, 0x70, 0x76],
};
// The dialog colour set for the active mode, with the panel background + text
// overridden by the configured dialog colours (theme default or a [colors]
// dialog_*/menu_* override). The remaining shades (border/track/handle/fields/
// buttons) stay from the mode preset so contrast holds.
// sRGB-space blend of two colours (selection highlight = field bg toward accent)
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
	Dlg {
		panel_bg: settings.dialog_bg,
		text: settings.dialog_fg,
		..base
	}
}

// Mode-adaptive dialog colours for the pop-out window (clear + About text).
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

#[derive(Clone, Copy, PartialEq)]
enum Key {
	None, // section headers
	Transparency,
	Opacity,
	BackdropBlur,
	BgOpacity,
	BgBlur,
	BgFit,
	BgContrastMask,
	BgContrastSize,
	BgContrastStrength,
	BgContrastAuto,
	TextScrim,
	ScrimRadius,
	ScrimSoftness,
	Outline,
	ScrimFunction,
	ScrimRamp,
	CursorScrim,
	CursorOutline,
	BgImage,
	SystemFont,
	FontFamily,
	DefaultShell,
	FontSize,
	LineHeight,
	Columns,
	Rows,
	RememberSize,
	Margin,
	ScrollTau,
	WheelLines,
	ColBg,
	ColFg,
	ColCursor,
	ColFocus,
}

// "Initial scroll speed" is shown as a friendly 1..100 (higher = faster) but
// stored as the easing time constant `scroll_tau_ms` (higher = slower), so the
// slider is the inverse of tau over [TAU_MIN, TAU_MAX].
const TAU_MIN: f32 = 10.0;
const TAU_MAX: f32 = 300.0;
fn tau_to_speed(tau: f32) -> f32 {
	(1.0 + (TAU_MAX - tau.clamp(TAU_MIN, TAU_MAX)) / (TAU_MAX - TAU_MIN) * 99.0).round()
}
fn speed_to_tau(speed: f32) -> f32 {
	TAU_MAX - (speed.clamp(1.0, 100.0) - 1.0) / 99.0 * (TAU_MAX - TAU_MIN)
}

enum Kind {
	Slider {
		min: f32,
		max: f32,
		int: bool,
	},
	Color,
	Text,   // free-text field (path / font family; empty = default)
	Toggle, // checkbox (e.g. use system font)
	// two labelled checkboxes on one row sharing the row label + revert (e.g.
	// Cursor: Scrim / Outline); each checkbox is a separate focus stop
	Dual {
		keys: [Key; 2],
		labels: [&'static str; 2],
	},
	Radio(&'static [&'static str]), // pick one of N mutually-exclusive options
	Dropdown(&'static [&'static str]), // one-of-N via a collapsed box + popup list
	Header(&'static str),           // a section heading, no control
}

const RADIO_BOX: f32 = 16.0; // radio indicator square
const RADIO_PITCH: f32 = 96.0; // px per option (box + label + gap) at BASE_LH
const DUAL_PITCH: f32 = 118.0; // px per [checkbox + label] pair on a Dual row at BASE_LH
const BASE_LH: f32 = 19.0; // UI line height the fixed radio consts were tuned for
const DD_W: f32 = 208.0; // collapsed dropdown box width at BASE_LH (fits the longest option + arrow)
const FIELD_PAD: f32 = 6.0; // text inset inside an editable field
const CARET_PAD: f32 = 6.0; // spare px kept visible right of the caret at end-of-text
const VIEW_AHEAD: f32 = 28.0; // lookahead margin: keep ~a few chars visible past the caret
const EM_W: f32 = 132.0; // field context-menu width at BASE_LH
const DD_ARROW: &str = "\u{25be}"; // small down-triangle in the collapsed box
const DD_CHECK: &str = "\u{2713}"; // marks the current value in the open popup

// Tabs ("super-sections"); each config section maps to one via tab_for_section.
pub const TAB_TITLES: [&str; 5] = ["Appearance", "Font", "Colors", "Window", "Scrolling"];
fn tab_for_section(section: &str) -> usize {
	match section {
		"Font" => 1,
		"Colors" => 2,
		"Window" | "Shell" => 3,
		"Scrolling" => 4,
		_ => 0, // "Appearance"
	}
}
const TAB_GAP: f32 = 6.0; // px between tab buttons
const HEADER_EXTRA: f32 = 10.0; // extra gap above a section header that follows another section
const SCROLLBAR_W: f32 = 8.0;
const REVERT_W: f32 = 22.0; // right-edge revert-to-default icon column
const REVERT_ICON: &str = "\u{21ba}"; // anticlockwise open-circle arrow

// Config-file key(s) behind a dialog Key, for revert's comment-out (dotted =
// the [colors] table). Empty for headers.
fn cfg_keys(key: Key) -> &'static [&'static str] {
	match key {
		Key::Transparency => &["transparent_background"],
		Key::Opacity => &["opacity"],
		Key::BackdropBlur => &["transparent_background_blur"],
		Key::BgOpacity => &["wallpaper_opacity"],
		Key::BgBlur => &["wallpaper_blur"],
		Key::BgFit => &["wallpaper_fit"],
		Key::BgContrastMask => &["wallpaper_contrast_mask"],
		Key::BgContrastSize => &["wallpaper_contrast_mask_size"],
		Key::BgContrastStrength => &["wallpaper_contrast_mask_strength"],
		Key::BgContrastAuto => &["wallpaper_contrast_mask_auto"],
		Key::TextScrim => &["text_scrim"],
		Key::ScrimRadius => &["text_scrim_radius"],
		Key::ScrimSoftness => &["text_scrim_softness"],
		Key::Outline => &["text_outline"],
		Key::ScrimFunction => &["text_scrim_function"],
		Key::ScrimRamp => &["text_scrim_ramp"],
		Key::CursorScrim => &["cursor_scrim"],
		Key::CursorOutline => &["cursor_outline"],
		Key::BgImage => &["wallpaper"],
		Key::SystemFont => &["use_system_font"],
		Key::FontFamily => &["font_family"],
		Key::DefaultShell => &["default_shell"],
		Key::FontSize => &["font_size"],
		Key::LineHeight => &["line_height_scale"],
		Key::Columns => &["columns"],
		Key::Rows => &["rows"],
		Key::RememberSize => &["remember_size"],
		Key::Margin => &["margin"],
		Key::ScrollTau => &["scroll_tau_ms"],
		Key::WheelLines => &["wheel_lines"],
		Key::ColBg => &["colors.background"],
		Key::ColFg => &["colors.foreground"],
		Key::ColCursor => &["colors.cursor"],
		Key::ColFocus => &["colors.focus"],
		Key::None => &[],
	}
}

struct Spec {
	label: &'static str,
	key: Key,
	kind: Kind,
}

// What holds keyboard focus: one control within a row, or a footer button (index
// into `buttons()`: 0 = Cancel, 1 = Apply, 2 = OK). `Row(i, part)` names a row and
// which of its focusable sub-controls (part 0 for a plain control; sliders and the
// combined cursor row expose two parts). Tab walks parts then buttons.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Focus {
	Row(usize, u8),
	Button(usize),
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

fn fields() -> Vec<Spec> {
	use Key::*;
	use Kind::*;
	let hdr = |t| Spec {
		label: "",
		key: None,
		kind: Header(t),
	};
	vec![
		hdr("Appearance"),
		Spec {
			label: "Transparency",
			key: Transparency,
			kind: Toggle,
		},
		Spec {
			label: "Opacity",
			key: Opacity,
			kind: Slider {
				min: 0.0,
				max: 1.0,
				int: false,
			},
		},
		Spec {
			label: "Blur-behind",
			key: BackdropBlur,
			kind: Toggle,
		},
		Spec {
			label: "Background image",
			key: BgImage,
			kind: Text,
		},
		Spec {
			label: "Bg image opacity",
			key: BgOpacity,
			kind: Slider {
				min: 0.0,
				max: 1.0,
				int: false,
			},
		},
		Spec {
			label: "Bg image blur",
			key: BgBlur,
			kind: Slider {
				min: 0.0,
				max: 50.0,
				int: false,
			},
		},
		Spec {
			label: "Bg image fit",
			key: BgFit,
			kind: Radio(&["Stretch", "Zoom"]),
		},
		Spec {
			label: "Contrast mask",
			key: BgContrastMask,
			kind: Toggle,
		},
		Spec {
			label: "Mask size",
			key: BgContrastSize,
			kind: Slider {
				min: 0.0,
				max: 1.0,
				int: false,
			},
		},
		Spec {
			label: "Mask strength",
			key: BgContrastStrength,
			kind: Slider {
				min: 0.0,
				max: 1.0,
				int: false,
			},
		},
		Spec {
			label: "Mask auto",
			key: BgContrastAuto,
			kind: Slider {
				min: 0.0,
				max: 1.0,
				int: false,
			},
		},
		Spec {
			label: "Text scrim",
			key: TextScrim,
			kind: Toggle,
		},
		Spec {
			label: "Scrim radius",
			key: ScrimRadius,
			kind: Slider {
				min: 0.0,
				max: 20.0,
				int: false,
			},
		},
		Spec {
			label: "Softness",
			key: ScrimSoftness,
			kind: Slider {
				min: 0.0,
				max: 1.0,
				int: false,
			},
		},
		Spec {
			label: "Text outline",
			key: Outline,
			kind: Slider {
				min: 0.0,
				max: 4.0,
				int: false,
			},
		},
		Spec {
			label: "Scrim function",
			key: ScrimFunction,
			kind: Dropdown(&[
				"Distance field",
				"Distance transform",
				"Dilate + feather",
				"Gaussian [ugly]",
			]),
		},
		Spec {
			label: "Scrim falloff",
			key: ScrimRamp,
			kind: Dropdown(&[
				"Exponential",
				"Gaussian",
				"Logarithmic",
				"S-curve",
				"Linear",
			]),
		},
		Spec {
			label: "Cursor",
			key: None,
			kind: Dual {
				keys: [CursorScrim, CursorOutline],
				labels: ["Scrim", "Outline"],
			},
		},
		hdr("Font"),
		Spec {
			label: "Use system font",
			key: SystemFont,
			kind: Toggle,
		},
		Spec {
			label: "Font family",
			key: FontFamily,
			kind: Text,
		},
		Spec {
			label: "Font size",
			key: FontSize,
			kind: Slider {
				min: 6.0,
				max: 40.0,
				int: true,
			},
		},
		Spec {
			label: "Line height",
			key: LineHeight,
			kind: Slider {
				min: 0.8,
				max: 2.0,
				int: false,
			},
		},
		hdr("Window"),
		Spec {
			label: "Columns",
			key: Columns,
			kind: Slider {
				min: 20.0,
				max: 400.0,
				int: true,
			},
		},
		Spec {
			label: "Rows",
			key: Rows,
			kind: Slider {
				min: 6.0,
				max: 120.0,
				int: true,
			},
		},
		Spec {
			label: "Remember last size",
			key: RememberSize,
			kind: Toggle,
		},
		Spec {
			label: "Margin",
			key: Margin,
			kind: Slider {
				min: 0.0,
				max: 40.0,
				int: true,
			},
		},
		hdr("Scrolling"),
		Spec {
			label: "Initial scroll speed",
			key: ScrollTau,
			kind: Slider {
				min: 1.0,
				max: 100.0,
				int: true,
			},
		},
		Spec {
			label: "Wheel lines",
			key: WheelLines,
			kind: Slider {
				min: 1.0,
				max: 10.0,
				int: true,
			},
		},
		hdr("Shell"),
		Spec {
			label: "Default shell",
			key: DefaultShell,
			kind: Text,
		},
		hdr("Colors"),
		Spec {
			label: "Background",
			key: ColBg,
			kind: Color,
		},
		Spec {
			label: "Foreground",
			key: ColFg,
			kind: Color,
		},
		Spec {
			label: "Cursor",
			key: ColCursor,
			kind: Color,
		},
		Spec {
			label: "Focus ring",
			key: ColFocus,
			kind: Color,
		},
	]
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

// Open field context menu: anchor point, keyboard-highlighted item, and whether
// the clipboard held text when it opened (greys Paste).
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
	specs: Vec<Spec>,
	spec_tab: Vec<usize>,     // which tab each spec lives on
	tab: usize,               // active tab
	tab_ws: Vec<f32>,         // measured tab-button widths (UI font)
	scroll: f32,              // rows-region scroll offset (0 when everything fits)
	drag_thumb: Option<f32>,  // scrollbar-thumb drag: grab offset within the thumb
	drag: Option<usize>,      // slider row being dragged
	pressed: Option<usize>,   // footer button held down (fires on release; drawn pressed)
	edit: Option<EditState>,  // row being typed (hex for Color, path for Text)
	edit_drag: Option<usize>, // field row being drag-selected with the mouse
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
}

impl SettingsDialog {
	fn row_h_for(kind: &Kind, line_h: f32) -> f32 {
		match kind {
			Kind::Header(_) => HEADER_H.max(line_h + 20.0),
			_ => ROW_H.max(line_h + 8.0),
		}
	}
	fn row_h(&self, kind: &Kind) -> f32 {
		Self::row_h_for(kind, self.line_h)
	}
	fn btn_h(&self) -> f32 {
		BTN_H.max(self.line_h + 8.0)
	}

	// Natural height of one tab's rows (header gaps included). Static so `new`
	// can size the window before Self exists; row_y must walk rows the same way.
	fn tab_content_h(specs: &[Spec], spec_tab: &[usize], tab: usize, line_h: f32) -> f32 {
		let mut h = 0.0;
		let mut first = true;
		for (i, spec) in specs.iter().enumerate() {
			if spec_tab[i] != tab {
				continue;
			}
			if matches!(spec.kind, Kind::Header(_)) && !first {
				h += HEADER_EXTRA;
			}
			h += Self::row_h_for(&spec.kind, line_h);
			first = false;
		}
		h
	}

	// `line_h` is the chrome (UI font) line height; `label_w`/`btn_w`/`tab_ws`
	// are the measured widths in that font (see chrome_widths) so nothing
	// truncates. `max_h` caps the window height (short screens / huge fonts);
	// a tab that doesn't fit scrolls instead of clipping the buttons.
	pub fn new(
		screen_w: f32,
		screen_h: f32,
		line_h: f32,
		label_w: f32,
		btn_w: f32,
		tab_ws: Vec<f32>,
		max_h: f32,
	) -> Self {
		let specs = fields();
		let mut cur_tab = 0usize;
		let spec_tab: Vec<usize> = specs
			.iter()
			.map(|spec| {
				if let Kind::Header(section) = spec.kind {
					cur_tab = tab_for_section(section);
				}
				cur_tab
			})
			.collect();
		let label_w = label_w.max(LABEL_W);
		let btn_w = btn_w.max(BTN_W);
		let btn_h = BTN_H.max(line_h + 8.0);
		let tallest = (0..TAB_TITLES.len())
			.map(|t| Self::tab_content_h(&specs, &spec_tab, t, line_h))
			.fold(0.0f32, f32::max);
		let h = (PAD + btn_h + 10.0 + tallest + 14.0 + btn_h + PAD).min(max_h.max(300.0));
		let tabs_w = PAD * 2.0
			+ tab_ws.iter().sum::<f32>()
			+ TAB_GAP * tab_ws.len().saturating_sub(1) as f32;
		// widest radio row (scaled pitch at HiDPI / large fonts) must fit the panel,
		// or the last option overflows the right edge
		let scale = (line_h / BASE_LH).max(1.0);
		let max_radio_opts = specs
			.iter()
			.filter_map(|spec| match spec.kind {
				Kind::Radio(opts) => Some(opts.len()),
				_ => None,
			})
			.max()
			.unwrap_or(0) as f32;
		let radio_w = PAD + label_w + max_radio_opts * RADIO_PITCH * scale + PAD;
		// a dropdown's collapsed box (+ revert column) must fit too
		let has_dropdown = specs.iter().any(|s| matches!(s.kind, Kind::Dropdown(_)));
		let dd_w = if has_dropdown {
			PAD + label_w + DD_W * scale + 6.0 + REVERT_W + PAD
		} else {
			0.0
		};
		let w = (W + (label_w - LABEL_W) + (btn_w - BTN_W) * 3.0)
			.max(tabs_w)
			.max(radio_w)
			.max(dd_w);
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
			spec_tab,
			tab: 0,
			tab_ws,
			scroll: 0.0,
			drag_thumb: None,
			drag: None,
			pressed: None,
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
		}
	}

	// Tab-bar / rows-viewport / scrollbar geometry. The rows region sits between
	// the tab bar and the buttons; only it scrolls (chrome stays put).
	fn tab_bar_y(&self) -> f32 {
		self.rect.y + PAD
	}
	fn tab_rect(&self, k: usize) -> Rect {
		let x = self.rect.x + PAD + self.tab_ws[..k].iter().sum::<f32>() + TAB_GAP * k as f32;
		Rect {
			x,
			y: self.tab_bar_y(),
			w: self.tab_ws[k],
			h: self.btn_h(),
		}
	}
	fn rows_y0(&self) -> f32 {
		self.tab_bar_y() + self.btn_h() + 10.0
	}
	pub fn viewport(&self) -> Rect {
		let y0 = self.rows_y0();
		Rect {
			x: self.rect.x,
			y: y0,
			w: self.rect.w,
			h: (self.rect.y + self.rect.h - PAD - self.btn_h() - 14.0 - y0).max(0.0),
		}
	}
	fn content_h(&self) -> f32 {
		Self::tab_content_h(&self.specs, &self.spec_tab, self.tab, self.line_h)
	}
	fn max_scroll(&self) -> f32 {
		(self.content_h() - self.viewport().h).max(0.0)
	}
	pub fn wheel(&mut self, dy_px: f32) {
		self.dismiss_menu();
		self.scroll = (self.scroll - dy_px).clamp(0.0, self.max_scroll());
	}
	fn thumb(&self) -> Option<Rect> {
		let scroll_max = self.max_scroll();
		if scroll_max <= 0.0 {
			return None;
		}
		let vp = self.viewport();
		let thumb_h = (vp.h * vp.h / self.content_h()).max(24.0);
		Some(Rect {
			x: self.rect.x + self.rect.w - PAD / 2.0 - SCROLLBAR_W,
			y: vp.y + (self.scroll / scroll_max) * (vp.h - thumb_h),
			w: SCROLLBAR_W,
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
	pub fn alt_key(c: char) -> Action {
		match c.to_ascii_lowercase() {
			'c' => Action::Cancel,
			'a' => Action::Apply,
			'o' => Action::Ok,
			_ => Action::None,
		}
	}

	// ---- dropdown popup (open list; commits on Enter / click) -----------------

	fn dd_options(&self, i: usize) -> &'static [&'static str] {
		match self.specs[i].kind {
			Kind::Dropdown(opts) => opts,
			_ => &[],
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
				self.spec_tab[i] == self.tab
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
			if self.spec_tab[i] != self.tab {
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
			None if forward => 0,
			None => n - 1,
		};
		self.focus = Some(ring[next]);
		self.scroll_focus_into_view();
	}
	// Scroll the rows region so a focused control row is fully visible (buttons
	// are fixed chrome - always visible).
	fn scroll_focus_into_view(&mut self) {
		let Some(Focus::Row(i, _)) = self.focus else {
			return;
		};
		let vp = self.viewport();
		let top = self.row_y(i);
		let bottom = top + self.row_h(&self.specs[i].kind);
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
		if self.ctrl {
			self.tab_switch(!self.shift);
		} else {
			self.focus_move(!self.shift);
		}
	}
	// Ctrl+PageUp / Ctrl+PageDown cycle the active tab (PageDown = next).
	pub fn key_page(&mut self, forward: bool) {
		if self.ctrl {
			self.tab_switch(forward);
		}
	}
	// Up / Down arrows: navigate an open popup, else Alt+Down opens a focused
	// dropdown, else step a focused numeric slider (spinbox feel), else walk control
	// focus (a peer of Tab).
	pub fn key_vertical(&mut self, forward: bool) {
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
			Kind::Radio(options) | Kind::Dropdown(options) => {
				let sel = self.get_radio(key) as i32;
				let new_sel = (sel + dir).clamp(0, options.len() as i32 - 1);
				self.set_radio(key, new_sel as usize);
			}
			_ => {}
		}
	}
	// Space: type into an active edit, activate a focused button, else activate the
	// focused control - flip a toggle or open a text/color field for editing.
	pub fn key_space(&mut self) -> Action {
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
			_ => {}
		}
		Action::None
	}

	// Current value of row i's editable field, as text.
	fn edit_buf(&self, i: usize) -> String {
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
		let buf = self.edit_buf(i);
		let cur = buf.len();
		let sel = (select_all && cur > 0).then_some(0);
		self.edit = Some(EditState {
			row: i,
			buf,
			cur,
			sel,
			view: 0.0,
			view_to: 0.0,
			caret_vis: None,
			blink_t: 0.0,
			last_sig: (usize::MAX, None, usize::MAX),
		});
	}

	// Panel size (used to size a dedicated dialog window when the panel is laid
	// out at the origin - `new(0.0, 0.0)`).
	pub fn size(&self) -> (f32, f32) {
		(self.rect.w, self.rect.h)
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
		let mut first = true;
		for (j, spec) in self.specs.iter().enumerate() {
			if self.spec_tab[j] != self.tab {
				continue;
			}
			if matches!(spec.kind, Kind::Header(_)) && !first {
				y += HEADER_EXTRA;
			}
			if j == i {
				return y;
			}
			y += self.row_h(&spec.kind);
			first = false;
		}
		y
	}
	fn control_x(&self) -> f32 {
		self.rect.x + PAD + self.label_w
	}
	fn track(&self, i: usize) -> Rect {
		Rect {
			x: self.control_x(),
			y: self.row_y(i) + ROW_H / 2.0 - 3.0,
			w: SLIDER_W,
			h: 6.0,
		}
	}
	fn swatch(&self, i: usize) -> Rect {
		Rect {
			x: self.control_x(),
			y: self.row_y(i) + (ROW_H - SWATCH) / 2.0,
			w: SWATCH,
			h: SWATCH,
		}
	}
	fn hexbox(&self, i: usize) -> Rect {
		Rect {
			x: self.control_x() + SWATCH + 8.0,
			y: self.row_y(i) + (ROW_H - SWATCH) / 2.0,
			w: HEX_W,
			h: SWATCH,
		}
	}
	// editable numeric field to the right of a slider (shows/edits the value)
	fn valbox(&self, i: usize) -> Rect {
		Rect {
			x: self.control_x() + SLIDER_W + 14.0,
			y: self.row_y(i) + (ROW_H - SWATCH) / 2.0,
			w: VAL_W,
			h: SWATCH,
		}
	}
	// wide editable field (background-image path), control_x -> the revert column
	fn textbox(&self, i: usize) -> Rect {
		let x = self.control_x();
		Rect {
			x,
			y: self.row_y(i) + (ROW_H - SWATCH) / 2.0,
			w: self.rect.x + self.rect.w - PAD - REVERT_W - 6.0 - x,
			h: SWATCH,
		}
	}
	// right-edge revert-to-default icon for row `i`
	fn revert_box(&self, i: usize) -> Rect {
		Rect {
			x: self.rect.x + self.rect.w - PAD - REVERT_W,
			y: self.row_y(i) + (ROW_H - SWATCH) / 2.0,
			w: REVERT_W,
			h: SWATCH,
		}
	}
	fn checkbox(&self, i: usize) -> Rect {
		Rect {
			x: self.control_x(),
			y: self.row_y(i) + (ROW_H - SWATCH) / 2.0,
			w: SWATCH,
			h: SWATCH,
		}
	}
	fn dual_pitch(&self) -> f32 {
		DUAL_PITCH * self.ui_scale()
	}
	// checkbox `p` (0/1) on a Dual row; its label sits just to the right
	fn dual_box(&self, i: usize, p: u8) -> Rect {
		Rect {
			x: self.control_x() + p as f32 * self.dual_pitch(),
			y: self.row_y(i) + (ROW_H - SWATCH) / 2.0,
			w: SWATCH,
			h: SWATCH,
		}
	}
	// Radio geometry scales with the UI font (HiDPI or a large desktop font), so
	// multi-option labels don't collide the way fixed 96px pitch does at 2x.
	fn ui_scale(&self) -> f32 {
		(self.line_h / BASE_LH).max(1.0)
	}
	fn radio_pitch(&self) -> f32 {
		RADIO_PITCH * self.ui_scale()
	}
	fn radio_box_sz(&self) -> f32 {
		RADIO_BOX * self.ui_scale()
	}
	// indicator box for radio option `k` in row `i`
	fn radio_box(&self, i: usize, k: usize) -> Rect {
		let size = self.radio_box_sz();
		Rect {
			x: self.control_x() + k as f32 * self.radio_pitch(),
			y: self.row_y(i) + (ROW_H - size) / 2.0,
			w: size,
			h: size,
		}
	}
	// Collapsed dropdown box (the always-visible control): shows the current option
	// + a down-arrow; clicking it opens the popup list.
	fn dd_box(&self, i: usize) -> Rect {
		let h = (self.line_h + 6.0).max(SWATCH);
		Rect {
			x: self.control_x(),
			y: self.row_y(i) + (ROW_H - h) / 2.0,
			w: DD_W * self.ui_scale(),
			h,
		}
	}
	// One option row inside the open popup.
	fn dd_item_h(&self) -> f32 {
		(self.line_h + 8.0).max(24.0)
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
	fn parts_of(&self, i: usize) -> u8 {
		match self.specs[i].kind {
			Kind::Header(_) => 0,
			Kind::Slider { .. } | Kind::Dual { .. } => 2,
			_ => 1,
		}
	}
	// The config Key that governs part `p` of row `i` (Dual parts differ; every
	// other kind uses the row's single key for both the value and its greying).
	fn part_key(&self, i: usize, p: u8) -> Key {
		match self.specs[i].kind {
			Kind::Dual { keys, .. } => keys[p as usize],
			_ => self.specs[i].key,
		}
	}
	fn part_disabled(&self, i: usize, p: u8) -> bool {
		self.disabled(self.part_key(i, p))
	}
	// Flyover text for a control disabled by the platform rather than by another
	// setting - explains why it is inert. Only the system-font toggle today.
	fn disabled_tip(key: Key) -> Option<&'static str> {
		(cfg!(windows) && matches!(key, Key::SystemFont))
			.then_some("Windows has no system monospace font")
	}
	// The flyover to show while the cursor rests on a control with a
	// disabled_tip: (text, anchor rect to hang the tip box under).
	pub fn hover_tip(&self, mx: f32, my: f32) -> Option<(&'static str, Rect)> {
		let vp = self.viewport();
		if !vp.contains(mx, my) {
			return None;
		}
		for i in 0..self.specs.len() {
			if self.spec_tab[i] != self.tab || matches!(self.specs[i].kind, Kind::Header(_)) {
				continue;
			}
			let Some(tip) = Self::disabled_tip(self.specs[i].key) else {
				continue;
			};
			if !self.disabled(self.specs[i].key) {
				continue;
			}
			// hover target: the row's label + control span
			let ctl = self.checkbox(i);
			let hit = Rect {
				x: self.rect.x + PAD,
				y: self.row_y(i),
				w: ctl.x + ctl.w - (self.rect.x + PAD),
				h: self.row_h(&self.specs[i].kind),
			};
			if hit.contains(mx, my) {
				return Some((tip, ctl));
			}
		}
		None
	}
	// Tight box around one focused sub-control (the keyboard-focus ring hugs this,
	// a couple px out, instead of spanning the whole row).
	fn focus_ctl_rect(&self, i: usize, p: u8) -> Rect {
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
			Kind::Header(_) => self.track(i), // unreachable (headers aren't focusable)
		}
	}
	// Is this row at its config default? (drives the revert icon). A Dual row is
	// "default" only when both its keys are.
	fn row_is_default(&self, i: usize) -> bool {
		match self.specs[i].kind {
			Kind::Dual { keys, .. } => keys.iter().all(|&k| self.is_default(k)),
			_ => self.is_default(self.specs[i].key),
		}
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
		let y = self.rect.y + self.rect.h - PAD - self.btn_h();
		let x_ok = self.rect.x + self.rect.w - PAD - self.btn_w;
		let x_apply = x_ok - BTN_GAP - self.btn_w;
		let x_cancel = x_apply - BTN_GAP - self.btn_w;
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

	fn get_f32(&self, key: Key) -> f32 {
		let settings = &self.edited;
		match key {
			Key::Opacity => settings.opacity,
			Key::BgOpacity => settings.wallpaper_opacity,
			Key::BgBlur => settings.wallpaper_blur,
			Key::BgContrastSize => settings.wallpaper_contrast_mask_size,
			Key::BgContrastStrength => settings.wallpaper_contrast_mask_strength,
			Key::BgContrastAuto => settings.wallpaper_contrast_mask_auto,
			Key::ScrimRadius => settings.text_scrim_radius,
			Key::ScrimSoftness => settings.text_scrim_softness,
			Key::Outline => settings.text_outline,
			Key::FontSize => settings.font_size,
			Key::LineHeight => settings.line_height_scale,
			Key::Margin => settings.margin,
			// shown as an intuitive 1..100 speed (higher = faster); stored as tau
			Key::ScrollTau => tau_to_speed(settings.scroll_tau_ms),
			Key::WheelLines => settings.wheel_lines,
			Key::Columns => settings.columns as f32,
			Key::Rows => settings.rows as f32,
			_ => 0.0,
		}
	}
	fn set_f32(&mut self, key: Key, value: f32) {
		// adjusting the size explicitly means we're no longer following the OS
		if key == Key::FontSize {
			self.edited.use_system_font = false;
		}
		let settings = &mut self.edited;
		match key {
			Key::Opacity => settings.opacity = value,
			Key::BgOpacity => settings.wallpaper_opacity = value,
			Key::BgBlur => settings.wallpaper_blur = value,
			Key::BgContrastSize => settings.wallpaper_contrast_mask_size = value,
			Key::BgContrastStrength => settings.wallpaper_contrast_mask_strength = value,
			Key::BgContrastAuto => settings.wallpaper_contrast_mask_auto = value,
			Key::ScrimRadius => settings.text_scrim_radius = value,
			Key::ScrimSoftness => settings.text_scrim_softness = value,
			Key::Outline => settings.text_outline = value,
			Key::FontSize => settings.font_size = value,
			Key::LineHeight => settings.line_height_scale = value,
			Key::Margin => settings.margin = value,
			Key::ScrollTau => settings.scroll_tau_ms = speed_to_tau(value),
			Key::WheelLines => settings.wheel_lines = value,
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
			Key::DefaultShell => self.edited.default_shell.clone(),
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
			Key::DefaultShell => self.edited.default_shell = trimmed.to_string(),
			_ => {}
		}
	}
	fn get_toggle(&self, key: Key) -> bool {
		match key {
			// shows the EFFECTIVE state: unchecked on Windows even when the config
			// value is true, since the toggle is inert there (no OS monospace font)
			Key::SystemFont => config::system_font_active(&self.edited),
			Key::Transparency => self.edited.transparent_background,
			Key::BackdropBlur => self.edited.transparent_background_blur,
			Key::TextScrim => self.edited.text_scrim,
			Key::CursorScrim => self.edited.cursor_scrim,
			Key::CursorOutline => self.edited.cursor_outline,
			Key::RememberSize => self.edited.remember_size,
			Key::BgContrastMask => self.edited.wallpaper_contrast_mask,
			_ => false,
		}
	}
	fn set_toggle(&mut self, key: Key, on: bool) {
		match key {
			Key::SystemFont => self.edited.use_system_font = on,
			Key::Transparency => self.edited.transparent_background = on,
			Key::BackdropBlur => self.edited.transparent_background_blur = on,
			Key::TextScrim => self.edited.text_scrim = on,
			Key::CursorScrim => self.edited.cursor_scrim = on,
			Key::CursorOutline => self.edited.cursor_outline = on,
			Key::RememberSize => self.edited.remember_size = on,
			Key::BgContrastMask => self.edited.wallpaper_contrast_mask = on,
			_ => {}
		}
	}
	fn get_radio(&self, key: Key) -> usize {
		match key {
			Key::BgFit => match self.edited.wallpaper_fit {
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
			// display order: Exponential, Gaussian, Log, S-curve, Linear
			Key::ScrimRamp => match self.edited.text_scrim_ramp.as_str() {
				"gaussian" => 1,
				"log" => 2,
				"s" => 3,
				"linear" => 4,
				_ => 0, // exp
			},
			_ => 0,
		}
	}
	fn set_radio(&mut self, key: Key, idx: usize) {
		match key {
			Key::BgFit => {
				self.edited.wallpaper_fit = if idx == 1 {
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
					1 => "gaussian",
					2 => "log",
					3 => "s",
					4 => "linear",
					_ => "exp",
				}
				.to_string();
			}
			_ => {}
		}
	}
	// A control greyed out because a prerequisite toggle is off (the opacity
	// slider needs Transparency; the scrim radius needs Text scrim; the explicit
	// columns/rows are inactive when "Remember last size" is on).
	fn disabled(&self, key: Key) -> bool {
		(matches!(key, Key::Opacity | Key::BackdropBlur) && !self.edited.transparent_background)
			|| (matches!(
				key,
				Key::ScrimRadius
					| Key::ScrimSoftness | Key::Outline
					| Key::ScrimFunction | Key::ScrimRamp
					| Key::CursorScrim | Key::CursorOutline
			) && !self.edited.text_scrim)
			// the cursor outline needs an outline to join
			|| (matches!(key, Key::CursorOutline) && self.edited.text_outline <= 0.0)
			|| (matches!(
				key,
				Key::BgContrastSize | Key::BgContrastStrength | Key::BgContrastAuto
			) && !self.edited.wallpaper_contrast_mask)
			|| (matches!(key, Key::Columns | Key::Rows) && self.edited.remember_size)
			|| (matches!(key, Key::FontFamily | Key::FontSize)
				&& config::system_font_active(&self.edited))
			// Windows has no system monospace font to follow (see disabled_tip)
			|| (matches!(key, Key::SystemFont) && cfg!(windows))
	}
	fn get_col(&self, key: Key) -> [u8; 3] {
		let settings = &self.edited;
		match key {
			Key::ColBg => settings.bg,
			Key::ColFg => settings.fg,
			Key::ColCursor => settings.cursor,
			Key::ColFocus => settings.focus,
			_ => [0, 0, 0],
		}
	}
	fn set_col(&mut self, key: Key, color: [u8; 3]) {
		let settings = &mut self.edited;
		match key {
			Key::ColBg => settings.bg = color,
			Key::ColFg => settings.fg = color,
			Key::ColCursor => settings.cursor = color,
			Key::ColFocus => settings.focus = color,
			_ => {}
		}
	}

	// The active theme's palette - the effective default for the [colors] keys
	// (commented-out colors fall back to the theme, not to SilkTerm-dark).
	fn theme_palette(&self) -> crate::theme::Palette {
		crate::theme::resolve(
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
			Key::ColFocus => palette.focus,
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
			Key::RememberSize => edited.remember_size == defaults.remember_size,
			Key::BgFit => edited.wallpaper_fit == defaults.wallpaper_fit,
			Key::ScrimRamp => edited.text_scrim_ramp == defaults.text_scrim_ramp,
			Key::BgImage => edited.wallpaper == defaults.wallpaper,
			Key::FontFamily => edited.font_family == defaults.font_family,
			Key::DefaultShell => edited.default_shell == defaults.default_shell,
			Key::ColBg | Key::ColFg | Key::ColCursor | Key::ColFocus => {
				self.get_col(key) == self.default_col(key)
			}
			Key::None => true,
			// the sliders
			_ => self.get_f32(key) == self.default_f32(key),
		}
	}
	// Default for a slider key, in get_f32's own units (speed for ScrollTau).
	fn default_f32(&self, key: Key) -> f32 {
		let defaults = &self.defaults;
		match key {
			Key::Opacity => defaults.opacity,
			Key::BgOpacity => defaults.wallpaper_opacity,
			Key::BgBlur => defaults.wallpaper_blur,
			Key::BgContrastSize => defaults.wallpaper_contrast_mask_size,
			Key::BgContrastStrength => defaults.wallpaper_contrast_mask_strength,
			Key::BgContrastAuto => defaults.wallpaper_contrast_mask_auto,
			Key::ScrimRadius => defaults.text_scrim_radius,
			Key::ScrimSoftness => defaults.text_scrim_softness,
			Key::Outline => defaults.text_outline,
			Key::FontSize => defaults.font_size,
			Key::LineHeight => defaults.line_height_scale,
			Key::Margin => defaults.margin,
			Key::ScrollTau => tau_to_speed(defaults.scroll_tau_ms),
			Key::WheelLines => defaults.wheel_lines,
			Key::Columns => defaults.columns as f32,
			Key::Rows => defaults.rows as f32,
			_ => 0.0,
		}
	}
	// Revert a setting to its default and remember its config key(s), so Apply
	// can comment them out in config.toml (config::revert_keys).
	fn revert(&mut self, key: Key) {
		match key {
			Key::Transparency
			| Key::BackdropBlur
			| Key::TextScrim
			| Key::CursorScrim
			| Key::CursorOutline
			| Key::SystemFont
			| Key::RememberSize
			| Key::BgContrastMask => {
				let default_val = match key {
					Key::Transparency => self.defaults.transparent_background,
					Key::BackdropBlur => self.defaults.transparent_background_blur,
					Key::TextScrim => self.defaults.text_scrim,
					Key::CursorScrim => self.defaults.cursor_scrim,
					Key::CursorOutline => self.defaults.cursor_outline,
					Key::SystemFont => self.defaults.use_system_font,
					Key::BgContrastMask => self.defaults.wallpaper_contrast_mask,
					_ => self.defaults.remember_size,
				};
				self.set_toggle(key, default_val);
			}
			Key::BgFit => self.edited.wallpaper_fit = self.defaults.wallpaper_fit,
			Key::ScrimRamp => self.edited.text_scrim_ramp = self.defaults.text_scrim_ramp.clone(),
			Key::BgImage => {
				self.edited.wallpaper = self.defaults.wallpaper.clone();
				self.edited.wallpaper_raw = self.defaults.wallpaper_raw.clone();
			}
			Key::FontFamily => self.edited.font_family = self.defaults.font_family.clone(),
			Key::DefaultShell => self.edited.default_shell = self.defaults.default_shell.clone(),
			Key::ColBg | Key::ColFg | Key::ColCursor | Key::ColFocus => {
				let color = self.default_col(key);
				self.set_col(key, color);
			}
			// direct: set_f32 would also clear use_system_font (its "explicit
			// size" side effect), which a revert must not do
			Key::FontSize => self.edited.font_size = self.defaults.font_size,
			Key::None => {}
			_ => {
				let value = self.default_f32(key);
				self.set_f32(key, value);
			}
		}
		for cfg_key in cfg_keys(key) {
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
	pub fn mouse_down(&mut self, x: f32, y: f32, measure: &mut impl FnMut(&str) -> f32) -> Action {
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
			if self.spec_tab[i] != self.tab {
				continue;
			}
			// revert-to-default icon (any control row; inert when already default)
			if !matches!(self.specs[i].kind, Kind::Header(_)) && self.revert_box(i).contains(x, y) {
				if !self.row_is_default(i) {
					self.row_revert(i);
				}
				return Action::None;
			}
			match self.specs[i].kind {
				Kind::Slider { .. } => {
					if self.disabled(self.specs[i].key) {
						continue; // greyed-out slider ignores clicks
					}
					// click the numeric field -> edit the value, caret at the click
					let val_box = self.valbox(i);
					if val_box.contains(x, y) {
						self.field_click(i, 1, val_box, x, measure);
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
						self.field_click(i, 0, hex_box, x, measure);
						return Action::None;
					}
				}
				Kind::Text => {
					let text_box = self.textbox(i);
					if text_box.contains(x, y) {
						self.field_click(i, 0, text_box, x, measure);
						return Action::None;
					}
				}
				Kind::Toggle => {
					if self.checkbox(i).contains(x, y) {
						if self.disabled(self.specs[i].key) {
							continue; // greyed checkbox ignores clicks
						}
						let key = self.specs[i].key;
						self.focus = Some(Focus::Row(i, 0));
						self.set_toggle(key, !self.get_toggle(key));
						return Action::None;
					}
				}
				Kind::Dual { keys, .. } => {
					// hit either checkbox (or its label span, out to the next pitch)
					for p in 0u8..2 {
						let bx = self.dual_box(i, p);
						if x >= bx.x
							&& x <= bx.x + self.dual_pitch() - 8.0
							&& (y - (bx.y + bx.h / 2.0)).abs() <= bx.h / 2.0 + 4.0
						{
							if self.disabled(keys[p as usize]) {
								continue; // greyed checkbox ignores clicks
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
				Kind::Header(_) => {}
			}
		}
		Action::None
	}

	// The editable text box of row i, by kind (None for non-field rows).
	fn field_rect(&self, i: usize) -> Option<Rect> {
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
	fn field_click(
		&mut self,
		i: usize,
		part: u8,
		field: Rect,
		x: f32,
		measure: &mut impl FnMut(&str) -> f32,
	) {
		let same_row = self.edit.as_ref().is_some_and(|e| e.row == i);
		if !same_row {
			self.open_edit(i, false);
		}
		self.select_all_on_up = false;
		let (shift, streak) = (self.shift, self.click_streak);
		self.focus = Some(Focus::Row(i, part));
		let Some(edit) = &mut self.edit else { return };
		let cur = caret_from_click(&edit.buf, x - (field.x + FIELD_PAD) + edit.view, measure);
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
		let w = EM_W * self.ui_scale();
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
	pub fn mouse_right(
		&mut self,
		x: f32,
		y: f32,
		paste_ok: bool,
		measure: &mut impl FnMut(&str) -> f32,
	) {
		self.mouse = (x, y);
		self.emenu = None;
		self.open = None;
		let vp = self.viewport();
		if y < vp.y || y > vp.y + vp.h {
			return;
		}
		for i in 0..self.specs.len() {
			if self.spec_tab[i] != self.tab {
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
			let part = u8::from(matches!(self.specs[i].kind, Kind::Slider { .. }));
			self.focus = Some(Focus::Row(i, part));
			if let Some(edit) = &mut self.edit {
				let rel_x = x - (field.x + FIELD_PAD) + edit.view;
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
			return;
		}
	}
	// Keyboard Menu key: pop the context menu at the caret of the active edit.
	pub fn menu_key(&mut self, paste_ok: bool, measure: &mut impl FnMut(&str) -> f32) {
		let Some(edit) = &self.edit else { return };
		let Some(field) = self.field_rect(edit.row) else {
			return;
		};
		let cx = (field.x + FIELD_PAD + measure(&edit.buf[..edit.cur]) - edit.view)
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
	// characters show ahead of travel; CARET_PAD keeps the caret clear of the
	// right edge at end-of-text), eases the caret x, advances the blink, and
	// replays a drag past the box edges (edge autoscroll). Returns the wake the
	// caller should schedule: fast while something moves, blink-rate while an
	// idle edit pulses, None when there's nothing to animate.
	pub fn animate(&mut self, dt: f32, measure: &mut impl FnMut(&str) -> f32) -> Option<u64> {
		if self.edit_drag.is_some() {
			let (mx, my) = self.mouse;
			self.mouse_move(mx, my, measure);
		}
		let row = self.edit.as_ref().map(|e| e.row)?;
		let field = self.field_rect(row)?;
		let inner_w = (field.w - 2.0 * FIELD_PAD).max(1.0);
		let ahead = (VIEW_AHEAD * self.ui_scale()).min(inner_w / 3.0);
		let (caret_x, text_w, sig) = {
			let edit = self.edit.as_ref().unwrap(); // Some: row extracted above
			(
				measure(&edit.buf[..edit.cur]),
				measure(&edit.buf),
				(edit.cur, edit.sel, edit.buf.len()),
			)
		};
		let dragging = self.edit_drag.is_some();
		let edit = self.edit.as_mut().unwrap();
		if sig == edit.last_sig {
			edit.blink_t += dt;
		} else {
			edit.last_sig = sig;
			edit.blink_t = 0.0; // activity holds the caret solid
		}
		// target view: keep the caret in sight with the margin; the clamp snaps
		// the margin away at the true ends so 0 / end-of-text sit flush
		let max_view = (text_w + CARET_PAD - inner_w).max(0.0);
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

	pub fn mouse_move(&mut self, x: f32, y: f32, measure: &mut impl FnMut(&str) -> f32) {
		self.mouse = (x, y);
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
					let rel_x = x - (field.x + FIELD_PAD) + edit.view;
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
			let thumb_h = self.thumb().map_or(24.0, |t| t.h);
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
	pub fn mouse_up(&mut self, x: f32, y: f32) -> Action {
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
		// the value selected, so the keystroke replaces it (standard field entry)
		if self.edit.is_none() {
			let Some(Focus::Row(i, _)) = self.focus else {
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
		// Ctrl+A on a focused-but-closed field opens it first
		if self.edit.is_none() {
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
		if self.open.is_some() {
			self.dd_commit();
			Action::None
		} else if self.edit.is_some() {
			self.commit_edit();
			Action::None
		} else if let Some(Focus::Button(b)) = self.focus {
			self.buttons()[b].0 // a focused footer button
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
		let left = field.x + FIELD_PAD - edit.view;
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
	pub fn rects(
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
		// tab bar: active tab filled brighter with an accent strip underneath
		for k in 0..self.tab_ws.len() {
			let r = self.tab_rect(k);
			let active = k == self.tab;
			fixed.push(q(
				r.x,
				r.y,
				r.w,
				r.h,
				if active { dlg().btn_hl } else { dlg().btn_bg },
			));
			border(&mut fixed, r, 1.0, dlg().panel_border);
			if active {
				fixed.push(q(r.x, r.y + r.h, r.w, 2.0, dlg().handle));
			}
		}
		// scrollbar (only when the active tab overflows the viewport)
		if let Some(thumb) = self.thumb() {
			let vp = self.viewport();
			fixed.push(q(thumb.x, vp.y, thumb.w, vp.h, dlg().track));
			fixed.push(q(thumb.x, thumb.y, thumb.w, thumb.h, dlg().handle));
		}

		for i in 0..self.specs.len() {
			if self.spec_tab[i] != self.tab {
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
					for p in 0u8..2 {
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
				Kind::Header(_) => {
					// faint rule near the bottom of the (tall) heading row, leaving a
					// clear gap below the heading text above it
					let y = self.row_y(i) + self.row_h(&Kind::Header("")) - 8.0;
					let x = self.rect.x + PAD;
					out.push(q(x, y, self.rect.w - PAD * 2.0, 1.0, dlg().panel_border));
				}
			}
		}
		// keyboard-focus ring around the active control row (scrolls + clips with
		// the rows; a focused button is ringed below, in the fixed chrome).
		if let Some(Focus::Row(fr, fp)) = self.focus {
			if self.spec_tab[fr] == self.tab && !matches!(self.specs[fr].kind, Kind::Header(_)) {
				let r = self.focus_ctl_rect(fr, fp);
				let ring = Rect {
					x: r.x - 2.0,
					y: r.y - 2.0,
					w: r.w + 4.0,
					h: r.h + 4.0,
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
			border(
				&mut fixed,
				r,
				if ring { 2.0 } else { 1.0 },
				if ring { dlg().focus_out } else { dlg().btn_hl },
			);
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
	pub fn texts(&self, line_h: f32, mut measure: impl FnMut(&str) -> f32) -> Vec<TextItem> {
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
		// tab titles
		for (k, title) in TAB_TITLES.iter().enumerate() {
			let r = self.tab_rect(k);
			out.push(mk((*title).into(), r.x + 11.0, row_text_y(r.y, r.h)));
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
			if self.spec_tab[i] != self.tab {
				continue;
			}
			let ty = row_text_y(self.row_y(i), ROW_H);
			if let Kind::Header(section) = self.specs[i].kind {
				// heading near the top of the row; the rule sits lower (gap between)
				let hy = self.row_y(i) + 5.0;
				out.push(TextItem {
					bold: true,
					clip: Some(vp),
					..mk(section.into(), self.rect.x + PAD, hy)
				});
				continue;
			}
			let off = self.disabled(self.specs[i].key);
			let label_color = if off { dlg().dim } else { dlg().text };
			out.push(TextItem {
				color: label_color,
				clip: Some(vp),
				..mk(self.specs[i].label.into(), self.rect.x + PAD, ty)
			});
			// revert-to-default icon: bright + clickable when off-default, dim when at it
			let revert_rect = self.revert_box(i);
			out.push(TextItem {
				color: if self.row_is_default(i) {
					dlg().dim
				} else {
					dlg().handle
				},
				clip: Some(vp),
				..mk(REVERT_ICON.into(), revert_rect.x + 4.0, ty)
			});
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
							val_box.x + FIELD_PAD - view(i),
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
							hex_box.x + FIELD_PAD - view(i),
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
					let placeholder =
						if matches!(self.specs[i].key, Key::FontFamily | Key::DefaultShell) {
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
							text_box.x + FIELD_PAD - view(i),
							row_text_y(text_box.y, text_box.h),
						)
					});
				}
				Kind::Dual { keys, labels } => {
					for p in 0u8..2 {
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
				Kind::Dropdown(options) => {
					let off = self.disabled(self.specs[i].key);
					let color = if off { dlg().dim } else { dlg().text };
					let box_r = self.dd_box(i);
					let sel = self.get_radio(self.specs[i].key);
					let label = options.get(sel).copied().unwrap_or("");
					out.push(TextItem {
						color,
						clip: Some(intersect(box_r)),
						..mk(label.into(), box_r.x + 8.0, row_text_y(box_r.y, box_r.h))
					});
					out.push(TextItem {
						color,
						clip: Some(vp),
						..mk(
							DD_ARROW.into(),
							box_r.x + box_r.w - 18.0,
							row_text_y(box_r.y, box_r.h),
						)
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
	pub fn dropdown_overlay(&self) -> (Vec<RectInstance>, Vec<TextItem>) {
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
				texts.push(mk(DD_CHECK.into(), r.x + r.w - 18.0, ty));
			}
			texts.push(mk((*opt).into(), r.x + 10.0, ty));
		}
		(rects, texts)
	}

	// True when anything needs the second (on-top) render pass.
	pub fn overlay_open(&self) -> bool {
		self.open.is_some() || self.emenu.is_some()
	}
	// Everything for the second pass: the open dropdown popup and/or the field
	// context menu (only one is ever open at a time in practice).
	pub fn overlay(&self) -> (Vec<RectInstance>, Vec<TextItem>) {
		let (mut rects, mut texts) = self.dropdown_overlay();
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
pub fn chrome_widths(text: &mut crate::text::TextCtx) -> (f32, f32, Vec<f32>) {
	let attrs = crate::text::ui_attrs();
	let label_w = fields()
		.iter()
		.map(|spec| text.measure_ui_text(spec.label, &attrs))
		.fold(0.0f32, f32::max)
		+ 14.0;
	let btn_w = ["Cancel", "Apply", "OK"]
		.iter()
		.map(|caption| text.measure_ui_text(caption, &attrs))
		.fold(0.0f32, f32::max)
		+ 24.0;
	let tab_ws = TAB_TITLES
		.iter()
		.map(|title| text.measure_ui_text(title, &attrs) + 22.0)
		.collect();
	(label_w, btn_w, tab_ws)
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
		|| old.margin != new.margin
}

// Returns true if a background-image-affecting setting changed.
pub fn wallpaper_changed(old: &Settings, new: &Settings) -> bool {
	old.wallpaper_opacity != new.wallpaper_opacity
		|| old.wallpaper_fit != new.wallpaper_fit
		|| old.wallpaper != new.wallpaper
		|| old.wallpaper_blur != new.wallpaper_blur
		|| old.wallpaper_contrast_mask != new.wallpaper_contrast_mask
		|| old.wallpaper_contrast_mask_size != new.wallpaper_contrast_mask_size
		|| old.wallpaper_contrast_mask_strength != new.wallpaper_contrast_mask_strength
		|| old.wallpaper_contrast_mask_auto != new.wallpaper_contrast_mask_auto
}

#[cfg(test)]
mod tests {
	use super::{SettingsDialog, TAB_TITLES, TAU_MAX, TAU_MIN, speed_to_tau, tau_to_speed};

	fn mk_dialog(max_h: f32) -> SettingsDialog {
		SettingsDialog::new(
			0.0,
			0.0,
			18.0,
			170.0,
			80.0,
			vec![90.0; TAB_TITLES.len()],
			max_h,
		)
	}

	#[test]
	fn tabs_partition_all_specs() {
		let d = mk_dialog(2000.0);
		// every spec lands on a valid tab and no tab is empty
		assert!(d.spec_tab.iter().all(|&t| t < TAB_TITLES.len()));
		for t in 0..TAB_TITLES.len() {
			assert!(d.spec_tab.contains(&t), "tab {t} has no rows");
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
		assert!(rev.contains(&"opacity"));
		assert!(d.take_reverted().is_empty(), "taking clears the list");
		// reverting font size must not clear use_system_font (set_f32 side effect)
		d.edited.use_system_font = true;
		d.edited.font_size = 99.0;
		d.revert(super::Key::FontSize);
		assert!(d.edited.use_system_font);
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

	#[test]
	fn keyboard_focus_walks_controls_then_buttons() {
		use super::Focus;
		let mut d = mk_dialog(2000.0);
		d.tab = 4; // Scrolling: two always-enabled sliders
		let f = d.focusables();
		assert_eq!(f.len(), 2, "scrolling tab has two focusable rows");
		d.set_mods(false, false, false);
		// each slider is two focus stops (track, then numeric field)
		d.key_tab(); // from nothing -> first slider's track
		assert_eq!(d.focus, Some(Focus::Row(f[0], 0)));
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Row(f[0], 1)));
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Row(f[1], 0)));
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Row(f[1], 1)));
		// after the last control the ring visits the three footer buttons
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
			.position(|s| matches!(s.kind, Kind::Dual { .. }))
			.unwrap();
		d.tab = d.spec_tab[i];
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
		assert!(d.take_reverted().contains(&"cursor_scrim"));
	}

	#[test]
	fn system_font_toggle_inert_on_windows() {
		use super::Key;
		let mut d = mk_dialog(2000.0);
		let i = d
			.specs
			.iter()
			.position(|s| matches!(s.key, Key::SystemFont))
			.unwrap();
		d.tab = d.spec_tab[i];
		let bx = d.checkbox(i);
		if cfg!(windows) {
			assert!(d.disabled(Key::SystemFont));
			// checkbox shows the effective (off) state despite the config value
			d.edited.use_system_font = true;
			assert!(!d.get_toggle(Key::SystemFont));
			// clicking the greyed checkbox must not flip the setting
			let mut measure = |s: &str| s.len() as f32;
			d.mouse_down(bx.x + 2.0, bx.y + 2.0, &mut measure);
			assert!(d.edited.use_system_font);
			// the flyover explains why; only over the row
			assert!(d.hover_tip(bx.x + 2.0, bx.y + 2.0).is_some());
			assert!(d.hover_tip(bx.x + 2.0, bx.y - 200.0).is_none());
			// font family / size stay editable even with use_system_font = true
			assert!(!d.disabled(Key::FontFamily) && !d.disabled(Key::FontSize));
		} else {
			assert!(!d.disabled(Key::SystemFont));
			assert!(d.hover_tip(bx.x + 2.0, bx.y + 2.0).is_none());
		}
	}

	#[test]
	fn hidpi_scales_radio_layout_and_widens_panel() {
		use super::Kind;
		// base (1x) vs a 2x UI font (HiDPI or a large desktop font)
		let base = mk_dialog(4000.0);
		let big = SettingsDialog::new(
			0.0,
			0.0,
			38.0,
			340.0,
			160.0,
			vec![180.0; TAB_TITLES.len()],
			4000.0,
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

	#[test]
	fn dropdown_open_navigate_commit() {
		use super::{Action, Focus, Key, Kind};
		let mut d = mk_dialog(2000.0);
		d.tab = 0;
		d.edited.text_scrim = true; // not greyed out
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
		d.tab = 0;
		d.edited.text_scrim = true;
		let i = d
			.specs
			.iter()
			.position(|s| s.key == Key::ScrimRamp)
			.unwrap();
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
		let base = d.get_f32(Key::ScrollTau);
		d.key_horizontal(-1);
		let lower = d.get_f32(Key::ScrollTau);
		assert!(lower <= base);
		d.key_horizontal(1);
		d.key_horizontal(1);
		assert!(d.get_f32(Key::ScrollTau) >= lower);
		// radio: focus the (always-enabled) bg-fit radio and move its selection
		let i = d.specs.iter().position(|s| s.key == Key::BgFit).unwrap();
		d.tab = d.spec_tab[i];
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
			.position(|s| s.key == Key::ScrollTau)
			.unwrap();
		d.tab = d.spec_tab[i];
		d.focus = Some(super::Focus::Row(i, 0));
		d.set_f32(Key::ScrollTau, 50.0);
		d.key_vertical(false); // Up -> increase by 1 (int step)
		assert_eq!(d.get_f32(Key::ScrollTau), 51.0);
		d.key_vertical(true); // Down -> decrease
		d.key_vertical(true);
		assert_eq!(d.get_f32(Key::ScrollTau), 49.0);
		d.set_mods(false, true, false); // Shift held
		d.key_vertical(false); // Shift+Up -> ~1/10 of the range (10)
		assert_eq!(d.get_f32(Key::ScrollTau), 59.0);
	}

	#[test]
	fn up_down_step_slider_during_edit() {
		use super::Key;
		let mut d = mk_dialog(2000.0);
		let i = d
			.specs
			.iter()
			.position(|s| s.key == Key::ScrollTau)
			.unwrap();
		d.tab = d.spec_tab[i];
		d.focus = Some(super::Focus::Row(i, 0));
		d.set_f32(Key::ScrollTau, 30.0);
		d.key_space(); // open the field, fully selected
		assert!(d.edit.is_some());
		d.key_vertical(false); // Up steps the value and refreshes the buffer
		assert_eq!(d.get_f32(Key::ScrollTau), 31.0);
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
		d.tab = d.spec_tab[i0];
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
		d.tab = d.spec_tab[i0];
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

	#[test]
	fn ctrl_tab_switches_tabs() {
		let mut d = mk_dialog(2000.0);
		d.set_mods(false, false, true); // Ctrl held
		let t0 = d.tab;
		d.key_tab();
		assert_ne!(d.tab, t0);
		assert!(d.focus.is_some(), "tab switch lands focus on a control");
	}

	#[test]
	fn slider_numeric_field_edits_and_clamps() {
		use super::{Focus, Key};
		let mut d = mk_dialog(2000.0);
		// Font size: an int slider on the Font tab, range 6..40
		d.edited.use_system_font = false; // else Font size is greyed/disabled
		let i = d.specs.iter().position(|s| s.key == Key::FontSize).unwrap();
		d.tab = d.spec_tab[i];
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
		// Opacity: a float slider on Appearance, range 0..1
		let i = d.specs.iter().position(|s| s.key == Key::Opacity).unwrap();
		d.tab = d.spec_tab[i];
		d.edited.transparent_background = true; // opacity enabled
		d.focus = Some(Focus::Row(i, 0));
		// typing a digit into the focused (unopened) slider starts a fresh number
		d.char_input('0');
		d.char_input('.');
		d.char_input('5');
		assert_eq!(d.edited.opacity, 0.5);
		// a second '.' and any letter are ignored (buffer stays "0.5")
		d.char_input('.');
		d.char_input('x');
		assert_eq!(d.edit.as_ref().unwrap().buf, "0.5");
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
		d.tab = d.spec_tab[i];
		d.edited.wallpaper_raw = value.to_string();
		d.focus = Some(Focus::Row(i, 0));
		d.set_mods(false, false, false);
		d.key_space(); // opens with the value fully selected
		(d, i)
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
		d.tab = d.spec_tab[i];
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
		let i = d.specs.iter().position(|s| s.key == Key::Opacity).unwrap();
		d.tab = d.spec_tab[i];
		d.edited.transparent_background = true;
		d.focus = Some(Focus::Row(i, 0));
		d.key_space();
		d.select_all();
		d.insert_str("0.7.5x");
		assert_eq!(d.edit.as_ref().unwrap().buf, "0.75");
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
		// round-trips within slider rounding
		for tau in [10.0, 75.0, 150.0, 300.0] {
			let rt = speed_to_tau(tau_to_speed(tau));
			assert!((rt - tau).abs() <= 3.0, "tau {tau} -> {rt}");
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
		use super::{CARET_PAD, FIELD_PAD};
		let (mut d, i) = mk_text_edit(&"x".repeat(400));
		let mut m = |s: &str| s.chars().count() as f32; // 1px per char
		d.cursor_end(); // collapse the open-time selection, caret at char 400
		settle(&mut d, &mut m);
		let field = d.textbox(i);
		let inner = field.w - 2.0 * FIELD_PAD;
		let e = d.edit.as_ref().unwrap();
		// scrolled right, caret in view, with the end padding visible after it
		assert!(e.view_to > 0.0);
		assert!((400.0 - e.view) <= inner - CARET_PAD + 0.5);
		assert_eq!(e.view, e.view_to, "ease settles exactly on the target");
		// moving left keeps the lookahead margin of context before the caret
		for _ in 0..200 {
			d.cursor_left();
		}
		settle(&mut d, &mut m);
		let e = d.edit.as_ref().unwrap();
		assert!(200.0 - e.view_to >= 27.0, "margin ahead of leftward travel");
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
		use super::FIELD_PAD;
		let (mut d, i) = mk_text_edit(&"y".repeat(400));
		let mut m = |s: &str| s.chars().count() as f32;
		d.cursor_end();
		settle(&mut d, &mut m);
		let view = d.edit.as_ref().unwrap().view;
		assert!(view > 0.0);
		let field = d.textbox(i);
		let y = field.y + field.h / 2.0;
		// a click 10px into the box lands on the char 10px past the scrolled-off part
		d.last_click = None;
		d.mouse_down(field.x + FIELD_PAD + 10.0, y, &mut m);
		let cur = d.edit.as_ref().unwrap().cur;
		assert!(
			(cur as f32 - (view + 10.0)).abs() <= 0.5,
			"cur {cur} vs view {view}"
		);
		d.mouse_up(field.x + FIELD_PAD + 10.0, y);
		// from the far left, dragging past the right edge keeps selecting while
		// the view crawls (edge autoscroll)
		d.cursor_home();
		settle(&mut d, &mut m);
		d.last_click = None;
		d.mouse_down(field.x + FIELD_PAD, y, &mut m);
		d.mouse_move(field.x + field.w + 40.0, y, &mut m);
		let cur0 = d.edit.as_ref().unwrap().cur;
		assert!(cur0 < 400, "the first drag event lands short of the end");
		settle(&mut d, &mut m);
		d.mouse_up(field.x + field.w + 40.0, y);
		let e = d.edit.as_ref().unwrap();
		assert!(e.cur > cur0, "edge autoscroll extends the selection");
		assert!(e.view > 0.0, "view followed the drag");
		assert!(d.selected_text().is_some());
	}

	#[test]
	fn context_menu_open_fire_and_gating() {
		use super::{Action, EditCmd, FIELD_PAD};
		let (mut d, i) = mk_text_edit("hello world");
		let mut m = |s: &str| s.chars().count() as f32;
		let field = d.textbox(i);
		let y = field.y + field.h / 2.0;
		// right-click inside the (select-all) selection keeps it; menu opens
		d.mouse_right(field.x + FIELD_PAD + 3.0, y, true, &mut m);
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
		d.mouse_right(field.x + FIELD_PAD + 3.0, y, false, &mut m);
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
		d.mouse_right(field.x + FIELD_PAD + 3.0, y, true, &mut m);
		assert!(d.emenu.is_some());
		assert_eq!(d.key_escape(), Action::None);
		assert!(d.emenu.is_none() && d.edit.is_some());
		// typing dismisses a stale menu
		d.mouse_right(field.x + FIELD_PAD + 3.0, y, true, &mut m);
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
}
