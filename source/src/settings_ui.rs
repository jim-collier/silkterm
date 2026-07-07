// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

//! Modal settings dialog: sliders for numeric tunables, swatch + hex field for
//! colors, and Cancel / Apply / OK. Edits a working copy of `Settings`; the app
//! reads it back on Apply/OK to live-apply + persist. Renders as flat quads
//! (rects) + positioned text the app draws in an overlay pass.
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
const TITLE_H: f32 = 38.0; // taller for the prominent (scaled) title
const ROW_H: f32 = 32.0;
const HEADER_H: f32 = 42.0; // a section heading row (extra top spacing + gap to its rule)
const TITLE_SCALE: f32 = 1.4; // dialog title rendered this much larger
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
	TextGlow,
	GlowRadius,
	GlowSoftness,
	GlowBorder,
	GlowRamp,
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
	Slider { min: f32, max: f32, int: bool },
	Color,
	Text,                           // free-text field (path / font family; empty = default)
	Toggle,                         // checkbox (e.g. use system font)
	Radio(&'static [&'static str]), // pick one of N mutually-exclusive options
	Header(&'static str),           // a section heading, no control
}

const RADIO_BOX: f32 = 16.0; // radio indicator square
const RADIO_PITCH: f32 = 96.0; // px per option (box + label + gap) at BASE_LH
const BASE_LH: f32 = 19.0; // UI line height the fixed radio consts were tuned for

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
		Key::BgOpacity => &["background_opacity"],
		Key::BgBlur => &["background_blur"],
		Key::BgFit => &["background_fit"],
		Key::TextGlow => &["text_glow"],
		Key::GlowRadius => &["text_glow_radius"],
		Key::GlowSoftness => &["text_glow_softness"],
		Key::GlowBorder => &["text_outline"],
		Key::GlowRamp => &["text_glow_ramp"],
		Key::BgImage => &["background_image"],
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

// What holds keyboard focus: a control row, or one of the footer buttons (index
// into `buttons()`: 0 = Cancel, 1 = Apply, 2 = OK). Tab walks rows then buttons.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Focus {
	Row(usize),
	Button(usize),
}

// In-progress field edit: the row, its text, and the caret (a byte index into
// `buf`, always on a char boundary).
struct EditState {
	row: usize,
	buf: String,
	cur: usize,
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
			label: "Backdrop blur",
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
			label: "Text glow",
			key: TextGlow,
			kind: Toggle,
		},
		Spec {
			label: "Glow radius",
			key: GlowRadius,
			kind: Slider {
				min: 0.0,
				max: 20.0,
				int: false,
			},
		},
		Spec {
			label: "Softness",
			key: GlowSoftness,
			kind: Slider {
				min: 0.0,
				max: 1.0,
				int: false,
			},
		},
		Spec {
			label: "Text outline",
			key: GlowBorder,
			kind: Slider {
				min: 0.0,
				max: 4.0,
				int: false,
			},
		},
		Spec {
			label: "Glow falloff",
			key: GlowRamp,
			kind: Radio(&["Gaussian", "Linear", "S-curve"]),
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
	spec_tab: Vec<usize>,    // which tab each spec lives on
	tab: usize,              // active tab
	tab_ws: Vec<f32>,        // measured tab-button widths (UI font)
	scroll: f32,             // rows-region scroll offset (0 when everything fits)
	drag_thumb: Option<f32>, // scrollbar-thumb drag: grab offset within the thumb
	drag: Option<usize>,     // slider row being dragged
	pressed: Option<usize>,  // footer button held down (fires on release; drawn pressed)
	edit: Option<EditState>, // row being typed (hex for Color, path for Text)
	focus: Option<Focus>,    // keyboard-focused control/button (None = mouse-only)
	alt: bool,               // Alt held: underline button accelerators (Cancel/Apply/OK)
	shift: bool,             // Shift held (Shift+Tab walks focus backwards)
	ctrl: bool,              // Ctrl held (Ctrl+Tab switches tabs)
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
	fn title_h(&self) -> f32 {
		TITLE_H.max(self.line_h * TITLE_SCALE + 10.0)
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
		let title_h = TITLE_H.max(line_h * TITLE_SCALE + 10.0);
		let btn_h = BTN_H.max(line_h + 8.0);
		let tallest = (0..TAB_TITLES.len())
			.map(|t| Self::tab_content_h(&specs, &spec_tab, t, line_h))
			.fold(0.0f32, f32::max);
		let h = (PAD + title_h + btn_h + 10.0 + tallest + 14.0 + btn_h + PAD).min(max_h.max(300.0));
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
		let w = (W + (label_w - LABEL_W) + (btn_w - BTN_W) * 3.0)
			.max(tabs_w)
			.max(radio_w);
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
		self.rect.y + PAD + self.title_h()
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
	pub fn alt_key(&mut self, c: char) -> Action {
		match c.to_ascii_lowercase() {
			'c' => Action::Cancel,
			'a' => Action::Apply,
			'o' => Action::Ok,
			_ => Action::None,
		}
	}

	// ---- keyboard focus + control activation ----------------------------------

	// A row that can hold keyboard focus: a real control (not a header) that isn't
	// greyed out by a prerequisite toggle. Tab order skips both.
	fn is_focusable(&self, i: usize) -> bool {
		!matches!(self.specs[i].kind, Kind::Header(_)) && !self.disabled(self.specs[i].key)
	}
	// Focusable rows on the active tab, in visual order.
	fn focusables(&self) -> Vec<usize> {
		(0..self.specs.len())
			.filter(|&i| self.spec_tab[i] == self.tab && self.is_focusable(i))
			.collect()
	}
	fn first_focus(&self) -> Option<Focus> {
		self.focus_ring().first().copied()
	}
	// The full Tab order for the active tab: focusable control rows, then the
	// three footer buttons (Cancel / Apply / OK), which are always reachable.
	fn focus_ring(&self) -> Vec<Focus> {
		let mut ring: Vec<Focus> = self.focusables().into_iter().map(Focus::Row).collect();
		ring.extend((0..3).map(Focus::Button));
		ring
	}
	// Tab / Shift+Tab (and Down / Up): move focus to the next/prev item in the
	// ring, wrapping, and scroll a focused row into view.
	fn focus_move(&mut self, forward: bool) {
		self.commit_edit();
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
		let Some(Focus::Row(i)) = self.focus else {
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
		if self.ctrl {
			self.tab_switch(!self.shift);
		} else {
			self.focus_move(!self.shift);
		}
	}
	// Up / Down arrows walk control focus (a peer of Tab).
	pub fn key_vertical(&mut self, forward: bool) {
		self.focus_move(forward);
	}
	// Left / Right: caret motion while a field is being edited, otherwise adjust
	// the focused slider (by one step) or move a focused radio's selection.
	pub fn key_horizontal(&mut self, dir: i32) {
		if self.edit.is_some() {
			if dir < 0 {
				self.cursor_left();
			} else {
				self.cursor_right();
			}
			return;
		}
		let Some(Focus::Row(i)) = self.focus else {
			return;
		};
		let key = self.specs[i].key;
		if self.disabled(key) {
			return;
		}
		match self.specs[i].kind {
			Kind::Slider { min, max, int } => {
				let step = if int { 1.0 } else { (max - min) / 100.0 };
				let mut value = (self.get_f32(key) + dir as f32 * step).clamp(min, max);
				if int {
					value = value.round();
				}
				self.set_f32(key, value);
			}
			Kind::Radio(options) => {
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
		if self.edit.is_some() {
			self.char_input(' ');
			return Action::None;
		}
		let i = match self.focus {
			Some(Focus::Button(b)) => return self.buttons()[b].0,
			Some(Focus::Row(i)) => i,
			None => return Action::None,
		};
		let key = self.specs[i].key;
		if self.disabled(key) {
			return Action::None;
		}
		match self.specs[i].kind {
			Kind::Toggle => self.set_toggle(key, !self.get_toggle(key)),
			Kind::Text => {
				let buf = self.get_text(key);
				let cur = buf.len();
				self.edit = Some(EditState { row: i, buf, cur });
			}
			Kind::Color => {
				self.edit = Some(EditState {
					row: i,
					buf: "#".to_string(),
					cur: 1,
				});
			}
			// open the numeric field pre-filled with the current value (edit in place)
			Kind::Slider { int, .. } => {
				let buf = self.fmt_val(key, int);
				let cur = buf.len();
				self.edit = Some(EditState { row: i, buf, cur });
			}
			_ => {}
		}
		Action::None
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
	// Row-spanning box drawn around the keyboard-focused control.
	fn focus_rect(&self, i: usize) -> Rect {
		let y = self.row_y(i);
		Rect {
			x: self.rect.x + PAD - 2.0,
			y: y + 3.0,
			w: self.rect.w - PAD * 2.0 + 4.0,
			h: self.row_h(&self.specs[i].kind) - 6.0,
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
			Key::BgOpacity => settings.background_opacity,
			Key::BgBlur => settings.background_blur,
			Key::GlowRadius => settings.text_glow_radius,
			Key::GlowSoftness => settings.text_glow_softness,
			Key::GlowBorder => settings.text_outline,
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
			Key::BgOpacity => settings.background_opacity = value,
			Key::BgBlur => settings.background_blur = value,
			Key::GlowRadius => settings.text_glow_radius = value,
			Key::GlowSoftness => settings.text_glow_softness = value,
			Key::GlowBorder => settings.text_outline = value,
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
			Key::BgImage => self
				.edited
				.background_image
				.as_ref()
				.map(|path| path.to_string_lossy().into_owned())
				.unwrap_or_default(),
			Key::FontFamily => self.edited.font_family.clone().unwrap_or_default(),
			Key::DefaultShell => self.edited.default_shell.clone(),
			_ => String::new(),
		}
	}
	fn set_text(&mut self, key: Key, text: &str) {
		let trimmed = text.trim();
		match key {
			Key::BgImage => {
				self.edited.background_image = if trimmed.is_empty() {
					None
				} else {
					Some(std::path::PathBuf::from(trimmed))
				};
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
			Key::SystemFont => self.edited.use_system_font,
			Key::Transparency => self.edited.transparent_background,
			Key::BackdropBlur => self.edited.transparent_background_blur,
			Key::TextGlow => self.edited.text_glow,
			Key::RememberSize => self.edited.remember_size,
			_ => false,
		}
	}
	fn set_toggle(&mut self, key: Key, on: bool) {
		match key {
			Key::SystemFont => self.edited.use_system_font = on,
			Key::Transparency => self.edited.transparent_background = on,
			Key::BackdropBlur => self.edited.transparent_background_blur = on,
			Key::TextGlow => self.edited.text_glow = on,
			Key::RememberSize => self.edited.remember_size = on,
			_ => {}
		}
	}
	fn get_radio(&self, key: Key) -> usize {
		match key {
			Key::BgFit => match self.edited.background_fit {
				config::Fit::Zoom => 1,
				_ => 0,
			},
			Key::GlowRamp => match self.edited.text_glow_ramp.as_str() {
				"linear" => 1,
				"s" => 2,
				_ => 0,
			},
			_ => 0,
		}
	}
	fn set_radio(&mut self, key: Key, idx: usize) {
		match key {
			Key::BgFit => {
				self.edited.background_fit = if idx == 1 {
					config::Fit::Zoom
				} else {
					config::Fit::Stretch
				};
			}
			Key::GlowRamp => {
				self.edited.text_glow_ramp = match idx {
					1 => "linear",
					2 => "s",
					_ => "gaussian",
				}
				.to_string();
			}
			_ => {}
		}
	}
	// A control greyed out because a prerequisite toggle is off (the opacity
	// slider needs Transparency; the glow radius needs Text glow; the explicit
	// columns/rows are inactive when "Remember last size" is on).
	fn disabled(&self, key: Key) -> bool {
		(matches!(key, Key::Opacity | Key::BackdropBlur) && !self.edited.transparent_background)
			|| (matches!(
				key,
				Key::GlowRadius | Key::GlowSoftness | Key::GlowBorder | Key::GlowRamp
			) && !self.edited.text_glow)
			|| (matches!(key, Key::Columns | Key::Rows) && self.edited.remember_size)
			|| (matches!(key, Key::FontFamily | Key::FontSize) && self.edited.use_system_font)
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
			Key::TextGlow => edited.text_glow == defaults.text_glow,
			Key::SystemFont => edited.use_system_font == defaults.use_system_font,
			Key::RememberSize => edited.remember_size == defaults.remember_size,
			Key::BgFit => edited.background_fit == defaults.background_fit,
			Key::GlowRamp => edited.text_glow_ramp == defaults.text_glow_ramp,
			Key::BgImage => edited.background_image == defaults.background_image,
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
			Key::BgOpacity => defaults.background_opacity,
			Key::BgBlur => defaults.background_blur,
			Key::GlowRadius => defaults.text_glow_radius,
			Key::GlowSoftness => defaults.text_glow_softness,
			Key::GlowBorder => defaults.text_outline,
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
			| Key::TextGlow
			| Key::SystemFont
			| Key::RememberSize => {
				let default_val = match key {
					Key::Transparency => self.defaults.transparent_background,
					Key::BackdropBlur => self.defaults.transparent_background_blur,
					Key::TextGlow => self.defaults.text_glow,
					Key::SystemFont => self.defaults.use_system_font,
					_ => self.defaults.remember_size,
				};
				self.set_toggle(key, default_val);
			}
			Key::BgFit => self.edited.background_fit = self.defaults.background_fit,
			Key::GlowRamp => self.edited.text_glow_ramp = self.defaults.text_glow_ramp.clone(),
			Key::BgImage => self.edited.background_image = self.defaults.background_image.clone(),
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
		self.commit_edit();
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
				let key = self.specs[i].key;
				if !self.is_default(key) {
					self.revert(key);
				}
				return Action::None;
			}
			match self.specs[i].kind {
				Kind::Slider { int, .. } => {
					if self.disabled(self.specs[i].key) {
						continue; // greyed-out slider ignores clicks
					}
					// click the numeric field -> edit the value, caret at the click
					let val_box = self.valbox(i);
					if val_box.contains(x, y) {
						let buf = self.fmt_val(self.specs[i].key, int);
						let cur = caret_from_click(&buf, x - (val_box.x + 6.0), measure);
						self.focus = Some(Focus::Row(i));
						self.edit = Some(EditState { row: i, buf, cur });
						return Action::None;
					}
					let track = self.track(i);
					let hit = x >= track.x - 8.0
						&& x <= track.x + track.w + 8.0
						&& (y - (track.y + track.h / 2.0)).abs() <= 12.0;
					if hit {
						self.focus = Some(Focus::Row(i));
						self.drag = Some(i);
						self.drag_to(x);
						return Action::None;
					}
				}
				Kind::Color => {
					if self.swatch(i).contains(x, y) || self.hexbox(i).contains(x, y) {
						// start a fresh hex entry (type 6 digits); swatch updates live
						self.focus = Some(Focus::Row(i));
						self.edit = Some(EditState {
							row: i,
							buf: "#".to_string(),
							cur: 1,
						});
						return Action::None;
					}
				}
				Kind::Text => {
					let text_box = self.textbox(i);
					if text_box.contains(x, y) {
						// edit the current value (empty when none); caret at the click
						let buf = self.get_text(self.specs[i].key);
						let cur = caret_from_click(&buf, x - (text_box.x + 6.0), measure);
						self.focus = Some(Focus::Row(i));
						self.edit = Some(EditState { row: i, buf, cur });
						return Action::None;
					}
				}
				Kind::Toggle => {
					if self.checkbox(i).contains(x, y) {
						let key = self.specs[i].key;
						self.focus = Some(Focus::Row(i));
						self.set_toggle(key, !self.get_toggle(key));
						return Action::None;
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
							self.focus = Some(Focus::Row(i));
							self.set_radio(self.specs[i].key, k);
							return Action::None;
						}
					}
				}
				Kind::Header(_) => {}
			}
		}
		Action::None
	}

	pub fn mouse_move(&mut self, x: f32, y: f32) {
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
		// typing into a keyboard-focused (but not-yet-open) text/color field opens it
		if self.edit.is_none() {
			let Some(Focus::Row(i)) = self.focus else {
				return;
			};
			match self.specs[i].kind {
				Kind::Text => {
					let buf = self.get_text(self.specs[i].key);
					let cur = buf.len();
					self.edit = Some(EditState { row: i, buf, cur });
				}
				Kind::Color => {
					self.edit = Some(EditState {
						row: i,
						buf: "#".to_string(),
						cur: 1,
					});
				}
				// typing a digit into a focused slider starts a fresh number
				Kind::Slider { .. } => {
					self.edit = Some(EditState {
						row: i,
						buf: String::new(),
						cur: 0,
					});
				}
				_ => return,
			}
		}
		let Some(edit) = &mut self.edit else {
			return;
		};
		match self.specs[edit.row].kind {
			Kind::Color => {
				if (c == '#' || c.is_ascii_hexdigit()) && edit.buf.len() < 7 {
					edit.buf.insert(edit.cur, c);
					edit.cur += c.len_utf8();
					self.reparse_edit();
				}
			}
			Kind::Text if !c.is_control() && edit.buf.len() < 256 => {
				edit.buf.insert(edit.cur, c);
				edit.cur += c.len_utf8();
				self.reparse_edit();
			}
			// numeric slider field: digits always; one '.' only for float sliders
			Kind::Slider { int, .. } => {
				let dot_ok = !int && c == '.' && !edit.buf.contains('.');
				if (c.is_ascii_digit() || dot_ok) && edit.buf.len() < 8 {
					edit.buf.insert(edit.cur, c);
					edit.cur += c.len_utf8();
					self.reparse_edit();
				}
			}
			_ => {}
		}
	}
	pub fn backspace(&mut self) {
		if let Some(edit) = &mut self.edit {
			if edit.cur > 0 {
				let prev = prev_boundary(&edit.buf, edit.cur);
				edit.buf.replace_range(prev..edit.cur, "");
				edit.cur = prev;
				self.reparse_edit();
			}
		}
	}
	pub fn delete_forward(&mut self) {
		if let Some(edit) = &mut self.edit {
			if edit.cur < edit.buf.len() {
				let next = next_boundary(&edit.buf, edit.cur);
				edit.buf.replace_range(edit.cur..next, "");
				self.reparse_edit();
			}
		}
	}
	// caret movement within the focused field (Left/Right/Home/End)
	pub fn cursor_left(&mut self) {
		if let Some(edit) = &mut self.edit {
			edit.cur = prev_boundary(&edit.buf, edit.cur);
		}
	}
	pub fn cursor_right(&mut self) {
		if let Some(edit) = &mut self.edit {
			edit.cur = next_boundary(&edit.buf, edit.cur);
		}
	}
	pub fn cursor_home(&mut self) {
		if let Some(edit) = &mut self.edit {
			edit.cur = 0;
		}
	}
	pub fn cursor_end(&mut self) {
		if let Some(edit) = &mut self.edit {
			edit.cur = edit.buf.len();
		}
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
	}

	// Esc cancels the dialog; Enter commits an active hex edit (or OK otherwise).
	pub fn key_escape(&mut self) -> Action {
		if self.edit.is_some() {
			self.edit = None;
			Action::None
		} else {
			Action::Cancel
		}
	}
	pub fn key_enter(&mut self) -> Action {
		if self.edit.is_some() {
			self.commit_edit();
			Action::None
		} else if let Some(Focus::Button(b)) = self.focus {
			self.buttons()[b].0 // a focused footer button
		} else {
			Action::Ok
		}
	}

	// caret line inside a focused field, at the measured prefix width
	fn caret_quad(
		&self,
		out: &mut Vec<RectInstance>,
		field: Rect,
		measure: &mut impl FnMut(&str) -> f32,
	) {
		let Some(edit) = &self.edit else { return };
		let x = (field.x + 6.0 + measure(&edit.buf[..edit.cur])).min(field.x + field.w - 2.0);
		out.push(RectInstance {
			pos: [x, field.y + 2.0],
			size: [1.5, field.h - 4.0],
			color: config::srgb_f32(dlg().focus_out),
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
							dlg().handle,
						));
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
		if let Some(Focus::Row(focus_row)) = self.focus {
			if self.spec_tab[focus_row] == self.tab
				&& !matches!(self.specs[focus_row].kind, Kind::Header(_))
			{
				border(&mut out, self.focus_rect(focus_row), 1.0, dlg().focus_out);
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
		// prominent title: bold + scaled up
		out.push(TextItem {
			bold: true,
			scale: TITLE_SCALE,
			..mk("Settings".into(), self.rect.x + PAD, self.rect.y + PAD)
		});
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
				color: if self.is_default(self.specs[i].key) {
					dlg().dim
				} else {
					dlg().handle
				},
				clip: Some(vp),
				..mk(REVERT_ICON.into(), revert_rect.x + 4.0, ty)
			});
			match self.specs[i].kind {
				Kind::Slider { int, .. } => {
					let val_box = self.valbox(i);
					let txt = match &self.edit {
						Some(edit) if edit.row == i => edit.buf.clone(),
						_ => self.fmt_val(self.specs[i].key, int),
					};
					out.push(TextItem {
						color: label_color,
						clip: Some(vp),
						..mk(txt, val_box.x + 6.0, row_text_y(val_box.y, val_box.h))
					});
				}
				Kind::Color => {
					let hex_box = self.hexbox(i);
					let txt = match &self.edit {
						Some(edit) if edit.row == i => edit.buf.clone(),
						_ => config::format_hex(self.get_col(self.specs[i].key)),
					};
					out.push(TextItem {
						clip: Some(vp),
						..mk(txt, hex_box.x + 6.0, row_text_y(hex_box.y, hex_box.h))
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
						..mk(txt, text_box.x + 6.0, row_text_y(text_box.y, text_box.h))
					});
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
pub fn bg_image_changed(old: &Settings, new: &Settings) -> bool {
	old.background_opacity != new.background_opacity
		|| old.background_fit != new.background_fit
		|| old.background_image != new.background_image
		|| old.background_blur != new.background_blur
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
		d.key_tab(); // from nothing -> first control
		assert_eq!(d.focus, Some(Focus::Row(f[0])));
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Row(f[1])));
		// after the last control the ring visits the three footer buttons
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Button(0)));
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Button(1)));
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Button(2)));
		d.key_tab(); // wraps back to the first control
		assert_eq!(d.focus, Some(Focus::Row(f[0])));
		d.set_mods(false, true, false); // Shift+Tab walks back (wraps to last button)
		d.key_tab();
		assert_eq!(d.focus, Some(Focus::Button(2)));
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
		// the widest (3-option) radio's last option stays inside the panel
		let ri = big
			.specs
			.iter()
			.position(|s| matches!(s.kind, Kind::Radio(o) if o.len() == 3))
			.unwrap();
		let last = big.radio_box(ri, 2);
		assert!(
			last.x + last.w <= big.rect.x + big.rect.w,
			"last radio option overflows the panel at 2x"
		);
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
		// with transparency + glow off, the opacity/blur/glow rows are disabled
		d.edited.transparent_background = false;
		d.edited.text_glow = false;
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
		d.focus = Some(super::Focus::Row(i));
		let before = d.get_radio(Key::BgFit);
		d.key_horizontal(1);
		assert!(d.get_radio(Key::BgFit) > before || before == 1);
		d.key_horizontal(-1);
		assert_eq!(d.get_radio(Key::BgFit), 0);
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
		d.focus = Some(Focus::Row(i));
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
		d.focus = Some(Focus::Row(i));
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
}
