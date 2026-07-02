// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

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
fn dlg() -> &'static Dlg {
	if config::is_dark() {
		&DARK_DLG
	} else {
		&LIGHT_DLG
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
const RADIO_PITCH: f32 = 96.0; // px per option (box + label + gap)

// Tabs ("super-sections"); each config section maps to one via tab_for_section.
pub const TAB_TITLES: [&str; 5] = ["Appearance", "Font", "Colors", "Window", "Scrolling"];
fn tab_for_section(h: &str) -> usize {
	match h {
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
		Key::GlowBorder => &["text_glow_border"],
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
			label: "Glow border",
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

#[derive(Clone, Copy, PartialEq)]
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
	edit: Option<EditState>, // row being typed (hex for Color, path for Text)
	alt: bool,               // Alt held: underline button accelerators (Cancel/Apply/OK)
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
		for (i, s) in specs.iter().enumerate() {
			if spec_tab[i] != tab {
				continue;
			}
			if matches!(s.kind, Kind::Header(_)) && !first {
				h += HEADER_EXTRA;
			}
			h += Self::row_h_for(&s.kind, line_h);
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
		let mut cur = 0usize;
		let spec_tab: Vec<usize> = specs
			.iter()
			.map(|s| {
				if let Kind::Header(h) = s.kind {
					cur = tab_for_section(h);
				}
				cur
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
		let w = (W + (label_w - LABEL_W) + (btn_w - BTN_W) * 3.0).max(tabs_w);
		let rect = Rect {
			x: ((screen_w - w) / 2.0).max(0.0),
			y: ((screen_h - h) / 2.0).max(0.0),
			w,
			h,
		};
		let s = (*config::settings()).clone();
		Self {
			orig: s.clone(),
			edited: s,
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
			edit: None,
			alt: false,
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
		let ms = self.max_scroll();
		if ms <= 0.0 {
			return None;
		}
		let vp = self.viewport();
		let th = (vp.h * vp.h / self.content_h()).max(24.0);
		Some(Rect {
			x: self.rect.x + self.rect.w - PAD / 2.0 - SCROLLBAR_W,
			y: vp.y + (self.scroll / ms) * (vp.h - th),
			w: SCROLLBAR_W,
			h: th,
		})
	}

	// Alt-key accelerators: while Alt is held the buttons underline their first
	// letter (Cancel/Apply/OK), and Alt+that-letter triggers the button.
	pub fn set_alt(&mut self, on: bool) {
		self.alt = on;
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
		for (j, s) in self.specs.iter().enumerate() {
			if self.spec_tab[j] != self.tab {
				continue;
			}
			if matches!(s.kind, Kind::Header(_)) && !first {
				y += HEADER_EXTRA;
			}
			if j == i {
				return y;
			}
			y += self.row_h(&s.kind);
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
	// indicator box for radio option `k` in row `i`
	fn radio_box(&self, i: usize, k: usize) -> Rect {
		Rect {
			x: self.control_x() + k as f32 * RADIO_PITCH,
			y: self.row_y(i) + (ROW_H - RADIO_BOX) / 2.0,
			w: RADIO_BOX,
			h: RADIO_BOX,
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
		let s = &self.edited;
		match key {
			Key::Opacity => s.opacity,
			Key::BgOpacity => s.background_opacity,
			Key::BgBlur => s.background_blur,
			Key::GlowRadius => s.text_glow_radius,
			Key::GlowSoftness => s.text_glow_softness,
			Key::GlowBorder => s.text_glow_border,
			Key::FontSize => s.font_size,
			Key::LineHeight => s.line_height_scale,
			Key::Margin => s.margin,
			// shown as an intuitive 1..100 speed (higher = faster); stored as tau
			Key::ScrollTau => tau_to_speed(s.scroll_tau_ms),
			Key::WheelLines => s.wheel_lines,
			Key::Columns => s.columns as f32,
			Key::Rows => s.rows as f32,
			_ => 0.0,
		}
	}
	fn set_f32(&mut self, key: Key, v: f32) {
		// adjusting the size explicitly means we're no longer following the OS
		if key == Key::FontSize {
			self.edited.use_system_font = false;
		}
		let s = &mut self.edited;
		match key {
			Key::Opacity => s.opacity = v,
			Key::BgOpacity => s.background_opacity = v,
			Key::BgBlur => s.background_blur = v,
			Key::GlowRadius => s.text_glow_radius = v,
			Key::GlowSoftness => s.text_glow_softness = v,
			Key::GlowBorder => s.text_glow_border = v,
			Key::FontSize => s.font_size = v,
			Key::LineHeight => s.line_height_scale = v,
			Key::Margin => s.margin = v,
			Key::ScrollTau => s.scroll_tau_ms = speed_to_tau(v),
			Key::WheelLines => s.wheel_lines = v,
			Key::Columns => s.columns = v.round().max(1.0) as usize,
			Key::Rows => s.rows = v.round().max(1.0) as usize,
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
				.map(|p| p.to_string_lossy().into_owned())
				.unwrap_or_default(),
			Key::FontFamily => self.edited.font_family.clone().unwrap_or_default(),
			Key::DefaultShell => self.edited.default_shell.clone(),
			_ => String::new(),
		}
	}
	fn set_text(&mut self, key: Key, s: &str) {
		let t = s.trim();
		match key {
			Key::BgImage => {
				self.edited.background_image = if t.is_empty() {
					None
				} else {
					Some(std::path::PathBuf::from(t))
				};
			}
			Key::FontFamily => {
				// an explicit family means we're not following the OS font
				self.edited.use_system_font = false;
				self.edited.font_family = if t.is_empty() {
					None
				} else {
					Some(t.to_string())
				};
			}
			Key::DefaultShell => self.edited.default_shell = t.to_string(),
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
		let s = &self.edited;
		match key {
			Key::ColBg => s.bg,
			Key::ColFg => s.fg,
			Key::ColCursor => s.cursor,
			Key::ColFocus => s.focus,
			_ => [0, 0, 0],
		}
	}
	fn set_col(&mut self, key: Key, c: [u8; 3]) {
		let s = &mut self.edited;
		match key {
			Key::ColBg => s.bg = c,
			Key::ColFg => s.fg = c,
			Key::ColCursor => s.cursor = c,
			Key::ColFocus => s.focus = c,
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
		let p = self.theme_palette();
		match key {
			Key::ColBg => p.bg,
			Key::ColFg => p.fg,
			Key::ColCursor => p.cursor,
			Key::ColFocus => p.focus,
			_ => [0, 0, 0],
		}
	}

	// Is this setting at its config default? Drives the revert icon's state.
	fn is_default(&self, key: Key) -> bool {
		let e = &self.edited;
		let d = &self.defaults;
		match key {
			Key::Transparency => e.transparent_background == d.transparent_background,
			Key::BackdropBlur => e.transparent_background_blur == d.transparent_background_blur,
			Key::TextGlow => e.text_glow == d.text_glow,
			Key::SystemFont => e.use_system_font == d.use_system_font,
			Key::RememberSize => e.remember_size == d.remember_size,
			Key::BgFit => e.background_fit == d.background_fit,
			Key::GlowRamp => e.text_glow_ramp == d.text_glow_ramp,
			Key::BgImage => e.background_image == d.background_image,
			Key::FontFamily => e.font_family == d.font_family,
			Key::DefaultShell => e.default_shell == d.default_shell,
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
		let d = &self.defaults;
		match key {
			Key::Opacity => d.opacity,
			Key::BgOpacity => d.background_opacity,
			Key::BgBlur => d.background_blur,
			Key::GlowRadius => d.text_glow_radius,
			Key::GlowSoftness => d.text_glow_softness,
			Key::GlowBorder => d.text_glow_border,
			Key::FontSize => d.font_size,
			Key::LineHeight => d.line_height_scale,
			Key::Margin => d.margin,
			Key::ScrollTau => tau_to_speed(d.scroll_tau_ms),
			Key::WheelLines => d.wheel_lines,
			Key::Columns => d.columns as f32,
			Key::Rows => d.rows as f32,
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
				let d = match key {
					Key::Transparency => self.defaults.transparent_background,
					Key::BackdropBlur => self.defaults.transparent_background_blur,
					Key::TextGlow => self.defaults.text_glow,
					Key::SystemFont => self.defaults.use_system_font,
					_ => self.defaults.remember_size,
				};
				self.set_toggle(key, d);
			}
			Key::BgFit => self.edited.background_fit = self.defaults.background_fit,
			Key::GlowRamp => self.edited.text_glow_ramp = self.defaults.text_glow_ramp.clone(),
			Key::BgImage => self.edited.background_image = self.defaults.background_image.clone(),
			Key::FontFamily => self.edited.font_family = self.defaults.font_family.clone(),
			Key::DefaultShell => self.edited.default_shell = self.defaults.default_shell.clone(),
			Key::ColBg | Key::ColFg | Key::ColCursor | Key::ColFocus => {
				let c = self.default_col(key);
				self.set_col(key, c);
			}
			// direct: set_f32 would also clear use_system_font (its "explicit
			// size" side effect), which a revert must not do
			Key::FontSize => self.edited.font_size = self.defaults.font_size,
			Key::None => {}
			_ => {
				let v = self.default_f32(key);
				self.set_f32(key, v);
			}
		}
		for k in cfg_keys(key) {
			if !self.reverted.contains(k) {
				self.reverted.push(k);
			}
		}
	}
	// Config keys reverted since the last Apply (cleared by taking them).
	pub fn take_reverted(&mut self) -> Vec<&'static str> {
		std::mem::take(&mut self.reverted)
	}

	fn fmt_val(&self, key: Key, int: bool) -> String {
		let v = self.get_f32(key);
		if int {
			format!("{}", v.round() as i64)
		} else {
			format!("{v:.2}")
		}
	}

	pub fn mouse_down(&mut self, x: f32, y: f32) -> Action {
		// buttons first
		for (action, r, _) in self.buttons() {
			if r.contains(x, y) {
				return action;
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
				}
				return Action::None;
			}
		}
		// scrollbar: drag the thumb, or jump-and-drag from the track
		if let Some(t) = self.thumb() {
			if t.contains(x, y) {
				self.drag_thumb = Some(y - t.y);
				return Action::None;
			}
			let vp = self.viewport();
			if x >= t.x && x <= t.x + t.w && y >= vp.y && y <= vp.y + vp.h {
				let frac = ((y - vp.y - t.h / 2.0) / (vp.h - t.h).max(1.0)).clamp(0.0, 1.0);
				self.scroll = frac * self.max_scroll();
				self.drag_thumb = Some(t.h / 2.0);
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
				Kind::Slider { .. } => {
					if self.disabled(self.specs[i].key) {
						continue; // greyed-out slider ignores clicks
					}
					let t = self.track(i);
					let hit = x >= t.x - 8.0
						&& x <= t.x + t.w + 8.0
						&& (y - (t.y + t.h / 2.0)).abs() <= 12.0;
					if hit {
						self.drag = Some(i);
						self.drag_to(x);
						return Action::None;
					}
				}
				Kind::Color => {
					if self.swatch(i).contains(x, y) || self.hexbox(i).contains(x, y) {
						// start a fresh hex entry (type 6 digits); swatch updates live
						self.edit = Some(EditState {
							row: i,
							buf: "#".to_string(),
							cur: 1,
						});
						return Action::None;
					}
				}
				Kind::Text => {
					if self.textbox(i).contains(x, y) {
						// edit the current value (empty when none), caret at the end
						let buf = self.get_text(self.specs[i].key);
						let cur = buf.len();
						self.edit = Some(EditState { row: i, buf, cur });
						return Action::None;
					}
				}
				Kind::Toggle => {
					if self.checkbox(i).contains(x, y) {
						let key = self.specs[i].key;
						self.set_toggle(key, !self.get_toggle(key));
						return Action::None;
					}
				}
				Kind::Radio(options) => {
					for k in 0..options.len() {
						let b = self.radio_box(i, k);
						// click the box or its label
						if x >= b.x
							&& x <= b.x + RADIO_PITCH - 8.0
							&& (y - (b.y + b.h / 2.0)).abs() <= RADIO_BOX / 2.0 + 4.0
						{
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
			let th = self.thumb().map_or(24.0, |t| t.h);
			let frac = ((y - grab - vp.y) / (vp.h - th).max(1.0)).clamp(0.0, 1.0);
			self.scroll = frac * self.max_scroll();
			return;
		}
		if self.drag.is_some() {
			self.drag_to(x);
		}
	}
	pub fn mouse_up(&mut self) {
		self.drag = None;
		self.drag_thumb = None;
	}

	fn drag_to(&mut self, x: f32) {
		let Some(i) = self.drag else { return };
		let Kind::Slider { min, max, int } = self.specs[i].kind else {
			return;
		};
		let t = self.track(i);
		let frac = ((x - t.x) / t.w).clamp(0.0, 1.0);
		let mut v = min + frac * (max - min);
		if int {
			v = v.round();
		}
		let key = self.specs[i].key;
		self.set_f32(key, v);
	}

	pub fn char_input(&mut self, c: char) {
		let Some(e) = &mut self.edit else {
			return;
		};
		match self.specs[e.row].kind {
			Kind::Color => {
				if (c == '#' || c.is_ascii_hexdigit()) && e.buf.len() < 7 {
					e.buf.insert(e.cur, c);
					e.cur += c.len_utf8();
					self.reparse_edit();
				}
			}
			Kind::Text if !c.is_control() && e.buf.len() < 256 => {
				e.buf.insert(e.cur, c);
				e.cur += c.len_utf8();
				self.reparse_edit();
			}
			_ => {}
		}
	}
	pub fn backspace(&mut self) {
		if let Some(e) = &mut self.edit {
			if e.cur > 0 {
				let p = prev_boundary(&e.buf, e.cur);
				e.buf.replace_range(p..e.cur, "");
				e.cur = p;
				self.reparse_edit();
			}
		}
	}
	pub fn delete_forward(&mut self) {
		if let Some(e) = &mut self.edit {
			if e.cur < e.buf.len() {
				let n = next_boundary(&e.buf, e.cur);
				e.buf.replace_range(e.cur..n, "");
				self.reparse_edit();
			}
		}
	}
	// caret movement within the focused field (Left/Right/Home/End)
	pub fn cursor_left(&mut self) {
		if let Some(e) = &mut self.edit {
			e.cur = prev_boundary(&e.buf, e.cur);
		}
	}
	pub fn cursor_right(&mut self) {
		if let Some(e) = &mut self.edit {
			e.cur = next_boundary(&e.buf, e.cur);
		}
	}
	pub fn cursor_home(&mut self) {
		if let Some(e) = &mut self.edit {
			e.cur = 0;
		}
	}
	pub fn cursor_end(&mut self) {
		if let Some(e) = &mut self.edit {
			e.cur = e.buf.len();
		}
	}
	// live-apply the in-progress edit (hex color, or background-image path)
	fn reparse_edit(&mut self) {
		let Some((i, buf)) = self.edit.as_ref().map(|e| (e.row, e.buf.clone())) else {
			return;
		};
		match self.specs[i].kind {
			Kind::Color => {
				if let Some(c) = config::parse_hex(&buf) {
					self.set_col(self.specs[i].key, c);
				}
			}
			Kind::Text => self.set_text(self.specs[i].key, &buf),
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
		let Some(e) = &self.edit else { return };
		let x = (field.x + 6.0 + measure(&e.buf[..e.cur])).min(field.x + field.w - 2.0);
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
		let q = |x: f32, y: f32, w: f32, h: f32, c: [u8; 3]| RectInstance {
			pos: [x, y],
			size: [w, h],
			color: config::srgb_f32(c),
		};
		let border = |out: &mut Vec<RectInstance>, r: Rect, t: f32, c: [u8; 3]| {
			out.push(q(r.x - t, r.y - t, r.w + 2.0 * t, t, c));
			out.push(q(r.x - t, r.y + r.h, r.w + 2.0 * t, t, c));
			out.push(q(r.x - t, r.y, t, r.h, c));
			out.push(q(r.x + r.w, r.y, t, r.h, c));
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
		if let Some(t) = self.thumb() {
			let vp = self.viewport();
			fixed.push(q(t.x, vp.y, t.w, vp.h, dlg().track));
			fixed.push(q(t.x, t.y, t.w, t.h, dlg().handle));
		}

		for i in 0..self.specs.len() {
			if self.spec_tab[i] != self.tab {
				continue;
			}
			match self.specs[i].kind {
				Kind::Slider { min, max, int } => {
					let off = self.disabled(self.specs[i].key);
					let t = self.track(i);
					out.push(q(t.x, t.y, t.w, t.h, dlg().track));
					let v = self.get_f32(self.specs[i].key);
					let frac = ((v - min) / (max - min)).clamp(0.0, 1.0);
					let hx = t.x + frac * t.w - 5.0;
					let _ = int;
					out.push(q(
						hx,
						t.y - 6.0,
						10.0,
						t.h + 12.0,
						if off {
							dlg().panel_border
						} else {
							dlg().handle
						},
					));
				}
				Kind::Color => {
					let sw = self.swatch(i);
					out.push(q(sw.x, sw.y, sw.w, sw.h, self.get_col(self.specs[i].key)));
					border(&mut out, sw, 1.0, dlg().panel_border);
					let hb = self.hexbox(i);
					out.push(q(hb.x, hb.y, hb.w, hb.h, dlg().field_bg));
					let focused = matches!(&self.edit, Some(e) if e.row == i);
					border(
						&mut out,
						hb,
						1.0,
						if focused {
							dlg().focus_out
						} else {
							dlg().panel_border
						},
					);
					if focused {
						self.caret_quad(&mut out, hb, &mut measure);
					}
				}
				Kind::Text => {
					let tb = self.textbox(i);
					out.push(q(tb.x, tb.y, tb.w, tb.h, dlg().field_bg));
					let focused = matches!(&self.edit, Some(e) if e.row == i);
					border(
						&mut out,
						tb,
						1.0,
						if focused {
							dlg().focus_out
						} else {
							dlg().panel_border
						},
					);
					if focused {
						self.caret_quad(&mut out, tb, &mut measure);
					}
				}
				Kind::Toggle => {
					let cb = self.checkbox(i);
					out.push(q(cb.x, cb.y, cb.w, cb.h, dlg().field_bg));
					border(&mut out, cb, 1.0, dlg().panel_border);
					// filled inner square when on (the checkmark glyph is drawn in texts)
					if self.get_toggle(self.specs[i].key) {
						out.push(q(
							cb.x + 4.0,
							cb.y + 4.0,
							cb.w - 8.0,
							cb.h - 8.0,
							dlg().handle,
						));
					}
				}
				Kind::Radio(options) => {
					let sel = self.get_radio(self.specs[i].key);
					for k in 0..options.len() {
						let b = self.radio_box(i, k);
						out.push(q(b.x, b.y, b.w, b.h, dlg().field_bg));
						border(&mut out, b, 1.0, dlg().panel_border);
						if k == sel {
							out.push(q(b.x + 4.0, b.y + 4.0, b.w - 8.0, b.h - 8.0, dlg().handle));
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
		for (_, r, label) in self.buttons() {
			fixed.push(q(r.x, r.y, r.w, r.h, dlg().btn_bg));
			border(&mut fixed, r, 1.0, dlg().btn_hl);
			// Alt held: underline the accelerator (the label's first letter). The
			// label is drawn left-aligned at r.x+14; the cap glyph is ~0.55*line_h
			// wide, and its baseline sits near the text bottom.
			if self.alt && !label.is_empty() {
				let tx = r.x + 14.0;
				let ty = r.y + (r.h - line_h) / 2.0 + line_h * 0.82;
				fixed.push(q(tx, ty, line_h * 0.5, 1.5, dlg().text));
			}
		}
		(fixed, out)
	}

	// `line_h` is the rendered text line height (the app's cell_h); rows, hex
	// fields, and buttons center their text vertically against it so alignment
	// holds for any font/size rather than a baked-in guess.
	pub fn texts(&self, line_h: f32) -> Vec<TextItem> {
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
		for (k, t) in TAB_TITLES.iter().enumerate() {
			let r = self.tab_rect(k);
			out.push(mk((*t).into(), r.x + 11.0, row_text_y(r.y, r.h)));
		}
		// row text clips to the scroll viewport so it can't ride over the chrome
		let vp = self.viewport();
		let isect = |r: Rect| -> Rect {
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
			if let Kind::Header(h) = self.specs[i].kind {
				// heading near the top of the row; the rule sits lower (gap between)
				let hy = self.row_y(i) + 5.0;
				out.push(TextItem {
					bold: true,
					clip: Some(vp),
					..mk(h.into(), self.rect.x + PAD, hy)
				});
				continue;
			}
			let off = self.disabled(self.specs[i].key);
			let lbl_color = if off { dlg().dim } else { dlg().text };
			out.push(TextItem {
				color: lbl_color,
				clip: Some(vp),
				..mk(self.specs[i].label.into(), self.rect.x + PAD, ty)
			});
			// revert-to-default icon: bright + clickable when off-default, dim when at it
			let rb = self.revert_box(i);
			out.push(TextItem {
				color: if self.is_default(self.specs[i].key) {
					dlg().dim
				} else {
					dlg().handle
				},
				clip: Some(vp),
				..mk(REVERT_ICON.into(), rb.x + 4.0, ty)
			});
			match self.specs[i].kind {
				Kind::Slider { int, .. } => {
					let vx = self.control_x() + SLIDER_W + 14.0;
					out.push(TextItem {
						color: lbl_color,
						clip: Some(vp),
						..mk(self.fmt_val(self.specs[i].key, int), vx, ty)
					});
				}
				Kind::Color => {
					let hb = self.hexbox(i);
					let txt = match &self.edit {
						Some(e) if e.row == i => e.buf.clone(),
						_ => config::format_hex(self.get_col(self.specs[i].key)),
					};
					out.push(TextItem {
						clip: Some(vp),
						..mk(txt, hb.x + 6.0, row_text_y(hb.y, hb.h))
					});
				}
				Kind::Text => {
					let tb = self.textbox(i);
					let val = match &self.edit {
						Some(e) if e.row == i => e.buf.clone(),
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
						clip: Some(isect(tb)),
						..mk(txt, tb.x + 6.0, row_text_y(tb.y, tb.h))
					});
				}
				Kind::Radio(options) => {
					let off = self.disabled(self.specs[i].key);
					let c = if off { dlg().dim } else { dlg().text };
					for (k, opt) in options.iter().enumerate() {
						let b = self.radio_box(i, k);
						out.push(TextItem {
							color: c,
							clip: Some(vp),
							..mk((*opt).into(), b.x + RADIO_BOX + 6.0, ty)
						});
					}
				}
				Kind::Toggle | Kind::Header(_) => {}
			}
		}
		for (_, r, label) in self.buttons() {
			out.push(mk(label.into(), r.x + 14.0, row_text_y(r.y, r.h)));
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
		.map(|f| text.measure_ui_text(f.label, &attrs))
		.fold(0.0f32, f32::max)
		+ 14.0;
	let btn_w = ["Cancel", "Apply", "OK"]
		.iter()
		.map(|t| text.measure_ui_text(t, &attrs))
		.fold(0.0f32, f32::max)
		+ 24.0;
	let tab_ws = TAB_TITLES
		.iter()
		.map(|t| text.measure_ui_text(t, &attrs) + 22.0)
		.collect();
	(label_w, btn_w, tab_ws)
}

// Returns true if `a` and `b` differ in any field that needs a text-context
// rebuild (cell metrics change) rather than just a re-render.
pub fn needs_text_rebuild(a: &Settings, b: &Settings) -> bool {
	a.font_size != b.font_size
		|| a.line_height_scale != b.line_height_scale
		|| a.font_family != b.font_family
		// the toggle alone changes the effective family/size (fields keep
		// their values), so it must force a rebuild too
		|| a.use_system_font != b.use_system_font
		|| a.margin != b.margin
}

// Returns true if a background-image-affecting setting changed.
pub fn bg_image_changed(a: &Settings, b: &Settings) -> bool {
	a.background_opacity != b.background_opacity
		|| a.background_fit != b.background_fit
		|| a.background_image != b.background_image
		|| a.background_blur != b.background_blur
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
