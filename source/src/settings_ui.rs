// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

//! Modal settings dialog: sliders for numeric tunables, swatch + hex field for
//! colors, and Cancel / Apply / OK. Edits a working copy of `Settings`; the app
//! reads it back on Apply/OK to live-apply + persist. Renders as flat quads
//! (rects) + positioned text the app draws in an overlay pass.

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

struct Spec {
	label: &'static str,
	key: Key,
	kind: Kind,
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
	rect: Rect,
	specs: Vec<Spec>,
	drag: Option<usize>,           // slider row being dragged
	edit: Option<(usize, String)>, // row being typed (hex for Color, path for Text)
	alt: bool,                     // Alt held: underline button accelerators (Cancel/Apply/OK)
}

impl SettingsDialog {
	fn row_h(kind: &Kind) -> f32 {
		match kind {
			Kind::Header(_) => HEADER_H,
			_ => ROW_H,
		}
	}

	pub fn new(screen_w: f32, screen_h: f32) -> Self {
		let specs = fields();
		let rows: f32 = specs.iter().map(|s| Self::row_h(&s.kind)).sum();
		let h = PAD + TITLE_H + rows + 14.0 + BTN_H + PAD;
		let rect = Rect {
			x: ((screen_w - W) / 2.0).max(0.0),
			y: ((screen_h - h) / 2.0).max(0.0),
			w: W,
			h,
		};
		let s = (*config::settings()).clone();
		Self {
			orig: s.clone(),
			edited: s,
			rect,
			specs,
			drag: None,
			edit: None,
			alt: false,
		}
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

	fn row_y(&self, i: usize) -> f32 {
		self.rect.y
			+ PAD + TITLE_H
			+ self.specs[..i]
				.iter()
				.map(|s| Self::row_h(&s.kind))
				.sum::<f32>()
	}
	fn control_x(&self) -> f32 {
		self.rect.x + PAD + LABEL_W
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
	// wide editable field (background-image path), control_x -> right padding
	fn textbox(&self, i: usize) -> Rect {
		let x = self.control_x();
		Rect {
			x,
			y: self.row_y(i) + (ROW_H - SWATCH) / 2.0,
			w: self.rect.x + self.rect.w - PAD - x,
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
		let y = self.rect.y + self.rect.h - PAD - BTN_H;
		let x_ok = self.rect.x + self.rect.w - PAD - BTN_W;
		let x_apply = x_ok - BTN_GAP - BTN_W;
		let x_cancel = x_apply - BTN_GAP - BTN_W;
		let mk = |x| Rect {
			x,
			y,
			w: BTN_W,
			h: BTN_H,
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
			_ => 0,
		}
	}
	fn set_radio(&mut self, key: Key, idx: usize) {
		if key == Key::BgFit {
			self.edited.background_fit = if idx == 1 {
				config::Fit::Zoom
			} else {
				config::Fit::Stretch
			};
		}
	}
	// A control greyed out because a prerequisite toggle is off (the opacity
	// slider needs Transparency; the glow radius needs Text glow; the explicit
	// columns/rows are inactive when "Remember last size" is on).
	fn disabled(&self, key: Key) -> bool {
		(matches!(key, Key::Opacity | Key::BackdropBlur) && !self.edited.transparent_background)
			|| (matches!(key, Key::GlowRadius | Key::GlowSoftness) && !self.edited.text_glow)
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
		for i in 0..self.specs.len() {
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
						self.edit = Some((i, "#".to_string()));
						return Action::None;
					}
				}
				Kind::Text => {
					if self.textbox(i).contains(x, y) {
						// edit the current value (empty when none)
						self.edit = Some((i, self.get_text(self.specs[i].key)));
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

	pub fn mouse_move(&mut self, x: f32, _y: f32) {
		if self.drag.is_some() {
			self.drag_to(x);
		}
	}
	pub fn mouse_up(&mut self) {
		self.drag = None;
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
		let Some((i, buf)) = &mut self.edit else {
			return;
		};
		match self.specs[*i].kind {
			Kind::Color => {
				if (c == '#' || c.is_ascii_hexdigit()) && buf.len() < 7 {
					buf.push(c);
					self.reparse_edit();
				}
			}
			Kind::Text if !c.is_control() && buf.len() < 256 => {
				buf.push(c);
				self.reparse_edit();
			}
			_ => {}
		}
	}
	pub fn backspace(&mut self) {
		if let Some((_, buf)) = &mut self.edit {
			buf.pop();
			self.reparse_edit();
		}
	}

	// live-apply the in-progress edit (hex color, or background-image path)
	fn reparse_edit(&mut self) {
		let Some((i, buf)) = self.edit.clone() else {
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

	pub fn rects(&self, line_h: f32) -> Vec<RectInstance> {
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
		out.push(q(
			self.rect.x,
			self.rect.y,
			self.rect.w,
			self.rect.h,
			dlg().panel_bg,
		));
		border(&mut out, self.rect, 1.0, dlg().panel_border);

		for i in 0..self.specs.len() {
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
					let focused = matches!(self.edit, Some((j, _)) if j == i);
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
				}
				Kind::Text => {
					let tb = self.textbox(i);
					out.push(q(tb.x, tb.y, tb.w, tb.h, dlg().field_bg));
					let focused = matches!(self.edit, Some((j, _)) if j == i);
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
					let y = self.row_y(i) + HEADER_H - 8.0;
					let x = self.rect.x + PAD;
					out.push(q(x, y, self.rect.w - PAD * 2.0, 1.0, dlg().panel_border));
				}
			}
		}
		for (_, r, label) in self.buttons() {
			out.push(q(r.x, r.y, r.w, r.h, dlg().btn_bg));
			border(&mut out, r, 1.0, dlg().btn_hl);
			// Alt held: underline the accelerator (the label's first letter). The
			// label is drawn left-aligned at r.x+14; the cap glyph is ~0.55*line_h
			// wide, and its baseline sits near the text bottom.
			if self.alt && !label.is_empty() {
				let tx = r.x + 14.0;
				let ty = r.y + (r.h - line_h) / 2.0 + line_h * 0.82;
				out.push(q(tx, ty, line_h * 0.5, 1.5, dlg().text));
			}
		}
		out
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
		for i in 0..self.specs.len() {
			let ty = row_text_y(self.row_y(i), ROW_H);
			if let Kind::Header(h) = self.specs[i].kind {
				// heading near the top of the row; the rule sits lower (gap between)
				let hy = self.row_y(i) + 5.0;
				out.push(TextItem {
					bold: true,
					..mk(h.into(), self.rect.x + PAD, hy)
				});
				continue;
			}
			let off = self.disabled(self.specs[i].key);
			let lbl_color = if off { dlg().dim } else { dlg().text };
			out.push(TextItem {
				color: lbl_color,
				..mk(self.specs[i].label.into(), self.rect.x + PAD, ty)
			});
			match self.specs[i].kind {
				Kind::Slider { int, .. } => {
					let vx = self.control_x() + SLIDER_W + 14.0;
					out.push(TextItem {
						color: lbl_color,
						..mk(self.fmt_val(self.specs[i].key, int), vx, ty)
					});
				}
				Kind::Color => {
					let hb = self.hexbox(i);
					let txt = match &self.edit {
						Some((j, buf)) if *j == i => buf.clone(),
						_ => config::format_hex(self.get_col(self.specs[i].key)),
					};
					out.push(mk(txt, hb.x + 6.0, row_text_y(hb.y, hb.h)));
				}
				Kind::Text => {
					let tb = self.textbox(i);
					let val = match &self.edit {
						Some((j, buf)) if *j == i => buf.clone(),
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
						clip: Some(tb),
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
	use super::{TAU_MAX, TAU_MIN, speed_to_tau, tau_to_speed};

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
