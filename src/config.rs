// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use serde::Deserialize;

// Display name (window title, default tab title). The Cargo package / binary
// name lives in Cargo.toml; see README "Renaming the project".
pub const APP_NAME: &str = "SilkTerm";

// internal, not user-tunable (yet)
pub const PANE_GAP_PX: f32 = 1.0;
pub const DIVIDER_GRAB_PX: f32 = 5.0; // mouse tolerance for grabbing a pane divider
pub const FOCUS_RING_PX: f32 = 2.0;
pub const SETTLE_EPS: f32 = 0.002;

pub const DIVIDER: [u8; 3] = [0x2c, 0x2c, 0x36];

// text-selection highlight
pub const SELECTION_BG: [u8; 3] = [0x33, 0x44, 0x66];

// drag-and-drop pane reorder: drop-target tint
pub const DROP_TARGET: [u8; 3] = [0x55, 0x80, 0xc8];

// tab bar
pub const TAB_BAR_BG: [u8; 3] = [0x2c, 0x2c, 0x31];
pub const TAB_ACTIVE: [u8; 3] = [0x47, 0x47, 0x4f];
pub const TAB_INACTIVE: [u8; 3] = [0x36, 0x36, 0x3b];

// Used only when the system monospace size can't be read (see default_font_size).
const FALLBACK_FONT_SIZE: f32 = 17.0;

// right-click context menu
pub const MENU_BG: [u8; 3] = [0x36, 0x36, 0x3b];
pub const MENU_HOVER: [u8; 3] = [0x4c, 0x4c, 0x55];
pub const MENU_BORDER: [u8; 3] = [0x58, 0x58, 0x60];
pub const MENU_FG: [u8; 3] = [0xf0, 0xf0, 0xf2];
pub const MENU_SEP: [u8; 3] = [0x4a, 0x4a, 0x51]; // faint group-separator line
pub const MENU_LINK: [u8; 3] = [0x6c, 0x9c, 0xff]; // clickable URL
pub const MENU_PAD_X: f32 = 12.0;
pub const MENU_ITEM_PAD_Y: f32 = 6.0;
pub const MENU_SEP_H: f32 = 9.0; // height of a separator row (line + spacing)
pub const MENU_GUTTER: f32 = 20.0; // left checkmark gutter; item text starts after it

// How a background image fills the window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit {
	Zoom,    // cover: fill, preserve aspect, crop overflow
	Stretch, // fill exactly, ignore aspect
}

// Resolved, validated settings used throughout the app.
#[derive(Clone)]
pub struct Settings {
	pub font_family: Option<String>, // None = system default monospace
	pub font_size: f32,
	pub line_height_scale: f32,
	pub scrollback: usize,
	pub scroll_tau_ms: f32,
	pub wheel_lines: f32,
	pub alt_scroll_lines: f32,
	pub output_ease_lines: f32,
	pub margin: f32,                       // logical px between content and pane edge
	pub opacity: f32,                      // background opacity 0..1 (1 = fully opaque)
	pub transparent_background: bool, // X11: per-pixel bg transparency (text stays opaque) via a GL surface
	pub transparent_background_blur: bool, // X11: ask a KWin/picom compositor to blur the desktop behind the window
	pub background_image: Option<PathBuf>, // resolved path, or None
	pub background_opacity: f32,           // image visibility 0..1
	pub background_fit: Fit,
	pub background_blur: f32, // Gaussian blur sigma applied to the image (0 = none)
	pub text_glow: bool, // bg-colored blurry halo behind glyphs (readability over busy/transparent bg)
	pub text_glow_radius: f32, // glow blur sigma in px
	pub text_glow_softness: f32, // 0 = hard/solid glow, 1 = soft/faint (maps to the intensity boost)
	pub cursor_blink: bool, // fade-blink the block cursor while it sits idle
	pub columns: usize,  // initial window grid size (used when !remember_size)
	pub rows: usize,
	pub remember_size: bool, // launch at the last window size instead of columns/rows
	pub remembered_columns: usize, // last actual window size (not shown in the dialog)
	pub remembered_rows: usize,
	pub word_separators: String, // delimiters for double-click word selection
	pub selection_pairs: String, // matched pairs a double-click selects inside of
	pub default_shell: String,   // command for new tabs/panes (empty = system shell)
	pub command_line: String,    // default CLI layout/options when launched with no args
	pub bg: [u8; 3],
	pub fg: [u8; 3],
	pub cursor: [u8; 3],
	pub focus: [u8; 3],
	pub ansi: [[u8; 3]; 16], // 16-colour ANSI palette, resolved from the active theme
	pub theme: String,       // active theme name (see theme.rs)
	pub theme_mode: String,  // "dark" | "light" | "system"
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			font_family: None,
			font_size: FALLBACK_FONT_SIZE,
			line_height_scale: 1.22,
			scrollback: 10_000,
			scroll_tau_ms: 230.0, // ~ "Initial scroll speed" 25 (slow/smooth; ramps up under bursts)
			wheel_lines: 3.0,
			alt_scroll_lines: 3.0,
			output_ease_lines: 1.0,
			margin: 8.0,
			opacity: 0.95,
			transparent_background: false,
			transparent_background_blur: false,
			background_image: None,
			background_opacity: 0.33, // image visibility relative to bg color
			background_fit: Fit::Stretch,
			background_blur: 8.0,
			text_glow: true,
			text_glow_radius: 5.0,
			text_glow_softness: 0.5,
			cursor_blink: false, // off by default: blinking redraws continuously (see config note)
			columns: 160,
			rows: 48,
			remember_size: false,
			remembered_columns: 160,
			remembered_rows: 48,
			// alacritty's default delimiters: keep /.-_~ as word chars so paths
			// and similar stay together on a double-click.
			word_separators: alacritty_terminal::term::SEMANTIC_ESCAPE_CHARS.to_owned(),
			selection_pairs: DEFAULT_SELECTION_PAIRS.to_owned(),
			default_shell: String::new(),
			command_line: String::new(),
			bg: [0x00, 0x00, 0x00],
			fg: [0xd2, 0xd2, 0xda],
			cursor: [0x7a, 0x9a, 0xd0],
			focus: [0x55, 0x80, 0xc8],
			ansi: crate::theme::resolve("SilkTerm", "dark", true).ansi,
			theme: "SilkTerm".to_string(),
			theme_mode: "dark".to_string(),
		}
	}
}

fn store() -> &'static RwLock<Arc<Settings>> {
	static S: OnceLock<RwLock<Arc<Settings>>> = OnceLock::new();
	S.get_or_init(|| RwLock::new(Arc::new(load())))
}

// Current settings snapshot. Cheap to call (an Arc clone); the settings dialog
// can swap the whole thing at runtime via `update`. Callers in hot paths should
// snapshot once per frame rather than per cell.
// Live OS dark/light bit (winit `Window::theme()`), used only when theme_mode = "system".
static OS_DARK: AtomicBool = AtomicBool::new(true);

// The effective dark/light for the active mode (chrome + dialogs follow this).
pub fn is_dark() -> bool {
	match settings().theme_mode.as_str() {
		"light" => false,
		"system" => OS_DARK.load(Ordering::Relaxed),
		_ => true,
	}
}

// On an OS dark/light change (System mode only): recompute the theme palette and
// swap it in (no file write). Returns true if anything changed (caller redraws).
// NOTE: re-derives from the theme, so a one-off [colors] override is dropped on an
// OS flip; overrides re-apply on the next full config load.
pub fn reapply_for_os(dark: bool) -> bool {
	let prev = OS_DARK.swap(dark, Ordering::Relaxed);
	let s = settings();
	if prev == dark || s.theme_mode != "system" {
		return false;
	}
	let pal = crate::theme::resolve(&s.theme, &s.theme_mode, dark);
	let mut new = (*s).clone();
	new.bg = pal.bg;
	new.fg = pal.fg;
	new.cursor = pal.cursor;
	new.focus = pal.focus;
	new.ansi = pal.ansi;
	update(new);
	true
}

pub fn settings() -> Arc<Settings> {
	store().read().unwrap().clone()
}

// Default double-click inclusion pairs, in precedence order (highest first):
// backticks, double quotes, single quotes, then {} () [] <>.
pub const DEFAULT_SELECTION_PAIRS: &str = "`` \"\" '' {} () [] <>";

// argv for the configured default shell, or None to use the system default.
pub fn default_shell_argv() -> Option<Vec<String>> {
	let s = settings().default_shell.clone();
	if s.trim().is_empty() {
		return None;
	}
	crate::cli::shell_split(&s).ok()
}

// Parse `selection_pairs` into (open, close) char pairs, in precedence order.
pub fn selection_pairs() -> Vec<(char, char)> {
	settings()
		.selection_pairs
		.split_whitespace()
		.filter_map(|t| {
			let mut c = t.chars();
			Some((c.next()?, c.next()?))
		})
		.collect()
}

// Replace the live settings (used by the settings dialog's Apply/OK).
pub fn update(new: Settings) {
	*store().write().unwrap() = Arc::new(new);
}

// Re-read config.toml from disk (e.g. after the user edited it by hand). Returns
// the freshly parsed settings; the caller applies them. Does not mutate the live
// store - pair with `update` plus whatever rebuild the change needs.
pub fn reload_from_disk() -> Settings {
	load()
}

// Write the values that differ from `orig` back into config.toml in place,
// preserving the user's comments and layout (toml_edit). Untouched settings keep
// whatever they were (commented / following the system).
pub fn persist(orig: &Settings, s: &Settings) {
	let Some(path) = config_path() else { return };
	let text = std::fs::read_to_string(&path).unwrap_or_default();
	let Ok(mut doc) = text.parse::<toml_edit::DocumentMut>() else {
		return;
	};
	use toml_edit::value;
	// round f32 -> a clean decimal so persisted floats aren't 0.2000000029...
	let r = |v: f32| (v as f64 * 1000.0).round() / 1000.0;

	if s.theme != orig.theme {
		doc["theme"] = value(s.theme.as_str());
	}
	if s.theme_mode != orig.theme_mode {
		doc["theme_mode"] = value(s.theme_mode.as_str());
	}

	if s.font_family != orig.font_family {
		if let Some(f) = &s.font_family {
			doc["font_family"] = value(f);
		}
	}
	if s.font_size != orig.font_size {
		doc["font_size"] = value(r(s.font_size));
	}
	if s.line_height_scale != orig.line_height_scale {
		doc["line_height_scale"] = value(r(s.line_height_scale));
	}
	if s.scrollback != orig.scrollback {
		doc["scrollback"] = value(s.scrollback as i64);
	}
	if s.scroll_tau_ms != orig.scroll_tau_ms {
		doc["scroll_tau_ms"] = value(r(s.scroll_tau_ms));
	}
	if s.wheel_lines != orig.wheel_lines {
		doc["wheel_lines"] = value(r(s.wheel_lines));
	}
	if s.alt_scroll_lines != orig.alt_scroll_lines {
		doc["alt_scroll_lines"] = value(r(s.alt_scroll_lines));
	}
	if s.output_ease_lines != orig.output_ease_lines {
		doc["output_ease_lines"] = value(r(s.output_ease_lines));
	}
	if s.margin != orig.margin {
		doc["margin"] = value(r(s.margin));
	}
	if s.opacity != orig.opacity {
		doc["opacity"] = value(r(s.opacity));
	}
	if s.transparent_background != orig.transparent_background {
		doc["transparent_background"] = value(s.transparent_background);
	}
	if s.transparent_background_blur != orig.transparent_background_blur {
		doc["transparent_background_blur"] = value(s.transparent_background_blur);
	}
	if s.background_opacity != orig.background_opacity {
		doc["background_opacity"] = value(r(s.background_opacity));
	}
	if s.background_fit != orig.background_fit {
		doc["background_fit"] = value(match s.background_fit {
			Fit::Zoom => "zoom",
			Fit::Stretch => "stretch",
		});
	}
	if s.background_blur != orig.background_blur {
		doc["background_blur"] = value(r(s.background_blur));
	}
	if s.text_glow != orig.text_glow {
		doc["text_glow"] = value(s.text_glow);
	}
	if s.text_glow_radius != orig.text_glow_radius {
		doc["text_glow_radius"] = value(r(s.text_glow_radius));
	}
	if s.text_glow_softness != orig.text_glow_softness {
		doc["text_glow_softness"] = value(r(s.text_glow_softness));
	}
	if s.columns != orig.columns {
		doc["columns"] = value(s.columns as i64);
	}
	if s.rows != orig.rows {
		doc["rows"] = value(s.rows as i64);
	}
	if s.remember_size != orig.remember_size {
		doc["remember_size"] = value(s.remember_size);
	}
	if s.remembered_columns != orig.remembered_columns {
		doc["remembered_columns"] = value(s.remembered_columns as i64);
	}
	if s.remembered_rows != orig.remembered_rows {
		doc["remembered_rows"] = value(s.remembered_rows as i64);
	}
	if s.word_separators != orig.word_separators {
		doc["word_separators"] = value(&s.word_separators);
	}
	if s.selection_pairs != orig.selection_pairs {
		doc["selection_pairs"] = value(&s.selection_pairs);
	}
	if s.default_shell != orig.default_shell {
		doc["default_shell"] = value(&s.default_shell);
	}
	if s.command_line != orig.command_line {
		doc["command_line"] = value(&s.command_line);
	}
	if s.background_image != orig.background_image {
		match &s.background_image {
			Some(p) => doc["background_image"] = value(p.to_string_lossy().as_ref()),
			None => {
				doc.remove("background_image");
			}
		}
	}

	let mut set_color = |key: &str, c: [u8; 3], oc: [u8; 3]| {
		if c != oc {
			doc["colors"][key] = value(format_hex(c));
		}
	};
	set_color("background", s.bg, orig.bg);
	set_color("foreground", s.fg, orig.fg);
	set_color("cursor", s.cursor, orig.cursor);
	set_color("focus", s.focus, orig.focus);

	let _ = std::fs::write(&path, doc.to_string());
}

// Remove keys from config.toml entirely (e.g. font_family/font_size when the
// user picks "Use system font", so future launches follow the OS again).
pub fn remove_keys(keys: &[&str]) {
	let Some(path) = config_path() else { return };
	let text = std::fs::read_to_string(&path).unwrap_or_default();
	let Ok(mut doc) = text.parse::<toml_edit::DocumentMut>() else {
		return;
	};
	for k in keys {
		doc.remove(k);
	}
	let _ = std::fs::write(&path, doc.to_string());
}

pub fn format_hex(c: [u8; 3]) -> String {
	format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

// The surface is an sRGB format, so the GPU re-encodes linear->sRGB on write.
// Feed it linear values derived from our sRGB byte colors.
pub fn srgb_f32(c: [u8; 3]) -> [f32; 4] {
	[to_linear(c[0]), to_linear(c[1]), to_linear(c[2]), 1.0]
}

fn to_linear(b: u8) -> f32 {
	let c = b as f32 / 255.0;
	if c <= 0.04045 {
		c / 12.92
	} else {
		((c + 0.055) / 1.055).powf(2.4)
	}
}

// config file loading

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawConfig {
	font_family: Option<String>,
	font_size: Option<f32>,
	line_height_scale: Option<f32>,
	scrollback: Option<usize>,
	scroll_tau_ms: Option<f32>,
	wheel_lines: Option<f32>,
	alt_scroll_lines: Option<f32>,
	output_ease_lines: Option<f32>,
	margin: Option<f32>,
	opacity: Option<f32>,
	transparent_background: Option<bool>,
	transparent_background_blur: Option<bool>,
	background_image: Option<String>,
	background_opacity: Option<f32>,
	background_fit: Option<String>,
	background_blur: Option<f32>,
	theme: Option<String>,
	theme_mode: Option<String>,
	text_glow: Option<bool>,
	text_glow_radius: Option<f32>,
	text_glow_softness: Option<f32>,
	cursor_blink: Option<bool>,
	columns: Option<usize>,
	rows: Option<usize>,
	remember_size: Option<bool>,
	remembered_columns: Option<usize>,
	remembered_rows: Option<usize>,
	word_separators: Option<String>,
	selection_pairs: Option<String>,
	default_shell: Option<String>,
	command_line: Option<String>,
	colors: RawColors,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawColors {
	background: Option<String>,
	foreground: Option<String>,
	cursor: Option<String>,
	focus: Option<String>,
}

fn load() -> Settings {
	let Some(path) = config_path() else {
		return Settings::default();
	};
	if !path.exists() {
		if let Some(dir) = path.parent() {
			let _ = std::fs::create_dir_all(dir);
		}
		let _ = std::fs::write(&path, DEFAULT_CONFIG);
	}
	// Backfill any keys a (possibly older) config is missing, so new settings
	// appear documented without clobbering the user's existing file.
	backfill_config(&path);
	let raw: RawConfig = match std::fs::read_to_string(&path) {
		Ok(text) => match toml::from_str(&lenient_floats(&text)) {
			Ok(raw) => raw,
			Err(e) => {
				eprintln!("{APP_NAME}: ignoring config {} ({e})", path.display());
				RawConfig::default()
			}
		},
		Err(_) => RawConfig::default(),
	};
	resolve(raw)
}

// TOML requires a leading zero on floats (`.25` is a parse error that would sink
// the whole file). Rewrite a bare-decimal value right after `=` to `0.25`.
fn lenient_floats(text: &str) -> String {
	text.lines()
		.map(|line| {
			let Some(eq) = line.find('=') else {
				return line.to_string();
			};
			let (head, after) = line.split_at(eq + 1);
			let val = after.trim_start();
			let ws = &after[..after.len() - val.len()];
			if let Some(rest) = val.strip_prefix('.').filter(|r| starts_digit(r)) {
				format!("{head}{ws}0.{rest}")
			} else if let Some(rest) = val.strip_prefix("-.").filter(|r| starts_digit(r)) {
				format!("{head}{ws}-0.{rest}")
			} else {
				line.to_string()
			}
		})
		.collect::<Vec<_>>()
		.join("\n")
}

fn starts_digit(s: &str) -> bool {
	s.chars().next().is_some_and(|c| c.is_ascii_digit())
}

fn resolve(raw: RawConfig) -> Settings {
	let d = Settings::default();
	let theme_name = raw.theme.unwrap_or_else(|| d.theme.clone());
	let theme_mode = raw.theme_mode.unwrap_or_else(|| d.theme_mode.clone());
	// system-mode OS dark/light detection is wired later; default to dark for now
	let pal = crate::theme::resolve(&theme_name, &theme_mode, OS_DARK.load(Ordering::Relaxed));
	let color =
		|s: Option<String>, fallback: [u8; 3]| s.as_deref().and_then(parse_hex).unwrap_or(fallback);
	Settings {
		font_family: raw.font_family.filter(|s| !s.trim().is_empty()),
		font_size: raw.font_size.unwrap_or_else(default_font_size).max(4.0),
		line_height_scale: raw
			.line_height_scale
			.unwrap_or(d.line_height_scale)
			.max(0.5),
		scrollback: raw.scrollback.unwrap_or(d.scrollback),
		scroll_tau_ms: raw.scroll_tau_ms.unwrap_or(d.scroll_tau_ms).max(1.0),
		wheel_lines: raw.wheel_lines.unwrap_or(d.wheel_lines),
		alt_scroll_lines: raw.alt_scroll_lines.unwrap_or(d.alt_scroll_lines),
		output_ease_lines: raw.output_ease_lines.unwrap_or(d.output_ease_lines),
		margin: raw.margin.unwrap_or(d.margin).max(0.0),
		opacity: raw.opacity.unwrap_or(d.opacity).clamp(0.0, 1.0),
		transparent_background: raw
			.transparent_background
			.unwrap_or(d.transparent_background),
		transparent_background_blur: raw
			.transparent_background_blur
			.unwrap_or(d.transparent_background_blur),
		background_image: resolve_bg_image(raw.background_image),
		background_opacity: raw
			.background_opacity
			.unwrap_or(d.background_opacity)
			.clamp(0.0, 1.0),
		background_blur: raw
			.background_blur
			.unwrap_or(d.background_blur)
			.clamp(0.0, 100.0),
		text_glow: raw.text_glow.unwrap_or(d.text_glow),
		text_glow_radius: raw
			.text_glow_radius
			.unwrap_or(d.text_glow_radius)
			.clamp(0.0, 50.0),
		text_glow_softness: raw
			.text_glow_softness
			.unwrap_or(d.text_glow_softness)
			.clamp(0.0, 1.0),
		cursor_blink: raw.cursor_blink.unwrap_or(d.cursor_blink),
		background_fit: match raw.background_fit.as_deref() {
			Some("zoom") => Fit::Zoom,
			_ => Fit::Stretch,
		},
		columns: raw.columns.unwrap_or(d.columns).max(1),
		rows: raw.rows.unwrap_or(d.rows).max(1),
		remember_size: raw.remember_size.unwrap_or(d.remember_size),
		remembered_columns: raw
			.remembered_columns
			.unwrap_or(d.remembered_columns)
			.max(1),
		remembered_rows: raw.remembered_rows.unwrap_or(d.remembered_rows).max(1),
		word_separators: raw.word_separators.unwrap_or(d.word_separators),
		selection_pairs: raw.selection_pairs.unwrap_or(d.selection_pairs),
		default_shell: raw.default_shell.unwrap_or(d.default_shell),
		command_line: raw.command_line.unwrap_or(d.command_line),
		bg: color(raw.colors.background, pal.bg),
		fg: color(raw.colors.foreground, pal.fg),
		cursor: color(raw.colors.cursor, pal.cursor),
		focus: color(raw.colors.focus, pal.focus),
		ansi: pal.ansi,
		theme: theme_name,
		theme_mode,
	}
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
	let s = s.trim().trim_start_matches('#');
	if s.len() != 6 {
		return None;
	}
	Some([
		u8::from_str_radix(&s[0..2], 16).ok()?,
		u8::from_str_radix(&s[2..4], 16).ok()?,
		u8::from_str_radix(&s[4..6], 16).ok()?,
	])
}

// Default font size (logical px) when the user hasn't set one: follow the OS's
// monospace size if we can detect it, else FALLBACK_FONT_SIZE.
pub fn default_font_size() -> f32 {
	crate::sysfont::monospace()
		.size_pt
		.map(|pt| pt * 96.0 / 72.0) // points -> logical px at the 96-DPI reference
		.filter(|px| *px >= 4.0)
		.unwrap_or(FALLBACK_FONT_SIZE)
}

// Resolve the background image: an explicit path (absolute, or a filename
// relative to the config dir), else auto-detect backgrounds/background.{png,jpg,jpeg}
// under the config dir.
fn resolve_bg_image(explicit: Option<String>) -> Option<PathBuf> {
	let dir = config_path()?.parent()?.to_path_buf();
	if let Some(s) = explicit.filter(|s| !s.trim().is_empty()) {
		let p = PathBuf::from(&s);
		let p = if p.is_absolute() { p } else { dir.join(s) };
		return p.exists().then_some(p);
	}
	let bg = dir.join("backgrounds");
	["background.png", "background.jpg", "background.jpeg"]
		.into_iter()
		.map(|n| bg.join(n))
		.find(|p| p.exists())
}

// A config file's settings as (table, key, original-line) - `table` is None for
// top-level keys, Some("colors") for a `[colors]` entry. Recognizes both active
// (`k = ...`) and commented (`# k = ...`) lines.
fn setting_lines(text: &str) -> Vec<(Option<String>, String, String)> {
	let mut table: Option<String> = None;
	let mut out = Vec::new();
	for line in text.lines() {
		if let Some(t) = line_table(line) {
			table = Some(t.to_string());
		} else if let Some(k) = line_setting_key(line) {
			out.push((table.clone(), k.to_string(), line.to_string()));
		}
	}
	out
}

// Like `setting_lines`, but each setting carries the contiguous comment lines
// directly above it (its block), plus `new_group` = whether a blank line (or a
// table header) precedes it in the template. Backfill uses this to keep a
// template group's settings together (no internal blank) while separating
// different groups by a blank line.
fn setting_groups(text: &str) -> Vec<(Option<String>, String, Vec<String>, bool)> {
	let mut table: Option<String> = None;
	let mut pending: Vec<String> = Vec::new();
	let mut group_break = true; // the first setting begins a group
	let mut out = Vec::new();
	for line in text.lines() {
		if let Some(t) = line_table(line) {
			table = Some(t.to_string());
			pending.clear();
			group_break = true;
		} else if let Some(k) = line_setting_key(line) {
			let mut block = std::mem::take(&mut pending);
			block.push(line.to_string());
			out.push((table.clone(), k.to_string(), block, group_break));
			group_break = false;
		} else if line.trim().is_empty() {
			pending.clear();
			group_break = true;
		} else if line.trim_start().starts_with('#') {
			pending.push(line.to_string());
		} else {
			pending.clear();
		}
	}
	out
}

fn line_table(line: &str) -> Option<&str> {
	let t = line.trim();
	t.strip_prefix('[').and_then(|r| r.strip_suffix(']'))
}

fn line_setting_key(line: &str) -> Option<&str> {
	let t = line.trim_start();
	let t = t.strip_prefix('#').map(str::trim_start).unwrap_or(t);
	let end = t.find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))?;
	let key = &t[..end];
	if key.is_empty() {
		return None;
	}
	t[end..].trim_start().starts_with('=').then_some(key)
}

// Insert any settings the `DEFAULT_CONFIG` template defines that `path` lacks,
// using the template's own (commented or active) line so follow-system keys stay
// absent and behavior is unchanged. Existing values, comments, and formatting are
// preserved (we only insert). Top-level keys go before the first table; `[colors]`
// keys go under that header.
fn backfill_config(path: &std::path::Path) {
	let Ok(text) = std::fs::read_to_string(path) else {
		return;
	};
	let have: std::collections::HashSet<(Option<String>, String)> = setting_lines(&text)
		.into_iter()
		.map(|(t, k, _)| (t, k))
		.collect();

	// Each missing top-level key is inserted as its own group: a blank-line
	// separator, the template's comment lines, then the setting (comment + setting
	// stay together; different groups are blank-line separated). Colors are a tight
	// group, so they go in bare (no per-key comments/blanks).
	let mut top: Vec<String> = Vec::new();
	let mut colors: Vec<String> = Vec::new();
	let mut group_open = false; // have we emitted a setting in the current template group?
	for (table, key, block, new_group) in setting_groups(DEFAULT_CONFIG) {
		if new_group {
			group_open = false;
		}
		if have.contains(&(table.clone(), key)) {
			continue;
		}
		match table.as_deref() {
			Some("colors") => colors.extend(block),
			_ => {
				// a blank line only when this starts a new (visible) group
				if !group_open {
					top.push(String::new());
				}
				top.extend(block);
				group_open = true;
			}
		}
	}
	if top.is_empty() && colors.is_empty() {
		return;
	}

	let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
	if !colors.is_empty() {
		match lines.iter().position(|l| line_table(l) == Some("colors")) {
			Some(i) => {
				for (off, l) in colors.into_iter().enumerate() {
					lines.insert(i + 1 + off, l);
				}
			}
			None => {
				lines.push(String::new());
				lines.push("[colors]".to_string());
				lines.extend(colors);
			}
		}
	}
	if !top.is_empty() {
		top.push(String::new()); // blank before the following table
		match lines.iter().position(|l| line_table(l).is_some()) {
			Some(i) => {
				// avoid a double blank if the line above the table is already blank
				if i > 0
					&& lines[i - 1].trim().is_empty()
					&& top.first().is_some_and(|l| l.is_empty())
				{
					top.remove(0);
				}
				for (off, l) in top.into_iter().enumerate() {
					lines.insert(i + off, l);
				}
			}
			None => lines.extend(top),
		}
	}
	let mut out = lines.join("\n");
	out.push('\n');
	if out != text {
		let _ = std::fs::write(path, out);
	}
}

// Set by `--config PATH` before any settings are read; overrides the default
// location for this process.
static CONFIG_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();
pub fn set_config_override(path: PathBuf) {
	let _ = CONFIG_OVERRIDE.set(path);
}

fn config_path() -> Option<PathBuf> {
	if let Some(p) = CONFIG_OVERRIDE.get() {
		return Some(p.clone());
	}
	let base = std::env::var_os("XDG_CONFIG_HOME")
		.map(PathBuf::from)
		.filter(|p| !p.as_os_str().is_empty())
		.or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
		.or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))?;
	Some(base.join("silkterm").join("config.toml"))
}

const DEFAULT_CONFIG: &str = r##"## SilkTerm configuration. Delete this file to regenerate defaults.
## Convention: '## ' starts an explanatory comment; a single '# ' before a
## `key = value` is a commented-out (disabled) setting you can uncomment.

## Font family by name; leave commented to use the system default monospace.
# font_family = "JetBrains Mono"

## Font size in logical pixels. Leave commented to follow the desktop's
## monospace size; uncomment to pin an explicit size.
# font_size = 17.0

## Line height as a multiple of the font's natural height (1.0 = tight).
line_height_scale = 1.22

## Pixels between the text and the pane edge.
margin = 8.0

## Transparency: when on, the terminal background (only - never the text, window
## frame, or menus) becomes see-through, using `opacity` below as its alpha. The
## code picks the method (per-pixel via a GL surface on X11; native elsewhere).
# transparent_background = true

## Background opacity, 0.0 (fully transparent) to 1.0 (opaque). Only takes effect
## when `transparent_background` is on.
opacity = 0.95

## Ask the compositor to blur the desktop showing through the translucent
## background ("frosted glass"); text stays crisp. Only honored by KWin and
## picom-with-blur; on Compiz/GNOME it does nothing (enable blur in the
## compositor instead). The compositor, not SilkTerm, controls the blur radius.
# transparent_background_blur = true

## Background image. Leave commented to auto-detect backgrounds/background.{png,jpg,jpeg}
## under this directory. Value may be an absolute path or a filename relative here.
# background_image = "background.png"

## Image visibility relative to the background color (independent of `opacity`
## above): 0.0 = all background color, 1.0 = all image.
# background_opacity = 0.33

## How the image fits: "stretch" (fill, ignore aspect) or "zoom" (cover, keep aspect).
# background_fit = "stretch"

## Gaussian blur applied to the background image (sigma in pixels; 0 = none).
# background_blur = 8.0

## Text readability glow: a blurry background-colored halo behind each glyph, so
## text stays legible over a light/busy background or near-transparent terminal.
## On by default; uncomment and set text_glow = false to disable.
# text_glow = true
# text_glow_radius = 5.0     ## glow blur sigma in pixels
# text_glow_softness = 0.5   ## 0 = hard/solid glow, 1 = soft/faint

## Fade-blink the block cursor when it sits idle. The cursor always slides
## smoothly to its new column as you type (that's free); blinking is separate and
## OFF by default because, while blinking, the view redraws continuously (capped
## ~30 fps) instead of idling. Set true if you want the fade-blink.
# cursor_blink = false

## Initial window size, in character cells (used when remember_size = false).
columns = 160
rows = 48

## Launch at the last window size instead of columns/rows. The remembered size is
## updated automatically whenever you resize the window (kept separate from
## columns/rows so unchecking reverts to your defined size); not shown in Settings.
# remember_size = false
# remembered_columns = 160
# remembered_rows = 48

## Delimiters that bound a double-click word selection. The default keeps
## / . - _ ~ as part of a word, so paths stay selected whole. Leave commented
## for the default; set to your own string of separator characters to override.
# word_separators = ",|:\"' ()[]{}<>"

## Pairs whose contents a double-click selects when the click is inside a matched
## pair (highest precedence first). Leave commented for the default.
# selection_pairs = "`` \"\" '' {} () [] <>"

## Default shell/command for new windows, tabs, and panes when nothing else is
## given (CLI --shell and per-pane inheritance take precedence). argv-split, so
## "bash --norc" works. Leave blank/commented to use the system default shell.
# default_shell = "bash --norc"

## Default command line applied when SilkTerm is launched with no arguments - the
## same window/tab/pane options the CLI accepts (see --help). Any actual
## command-line arguments override this entirely. Leave blank/commented for none.
# command_line = "--new-pane --right --size 35%"

## Lines of scrollback history kept per pane.
scrollback = 10000

## Smooth-scroll feel. This is the *initial* (slow, smooth) easing for sporadic
## output, shown in Settings as "Initial scroll speed"; lower tau = snappier. Under
## a fast output burst the scroll automatically ramps faster to keep up, then eases
## back to this speed once output stops.
scroll_tau_ms = 230.0      ## ms; ~ "Initial scroll speed" 25 on the 1..100 dialog scale
wheel_lines = 3.0          ## lines per wheel notch (smooth scrollback)
alt_scroll_lines = 3.0     ## lines per wheel notch in full-screen apps (less, nano)
output_ease_lines = 1.0    ## how far new output slides in before easing to rest

## Colour theme. Pick a built-in (SilkTerm, Matrix, Retro Amber) or one you add in
## a [themes.*] table. theme_mode is "dark", "light", or "system" (follow the OS).
theme = "SilkTerm"
theme_mode = "dark"

## Per-colour overrides on top of the theme (uncomment any to tweak one colour).
[colors]
# background = "#000000"
# foreground = "#d2d2da"
# cursor     = "#7a9ad0"
# focus      = "#5580c8"
"##;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_config_is_valid_toml() {
		assert!(
			DEFAULT_CONFIG.parse::<toml_edit::DocumentMut>().is_ok(),
			"DEFAULT_CONFIG is not valid TOML"
		);
		// and it deserializes through the real loader path
		assert!(
			toml::from_str::<RawConfig>(&lenient_floats(DEFAULT_CONFIG)).is_ok(),
			"DEFAULT_CONFIG active keys don't deserialize"
		);
	}

	// #136 convention: explanatory comments use '## '; commented-out (disabled)
	// settings use a single '# '.
	#[test]
	fn default_config_comment_style() {
		for line in DEFAULT_CONFIG.lines() {
			let t = line.trim_start();
			if !t.starts_with('#') {
				continue; // active setting / blank / table header
			}
			if line_setting_key(line).is_some() {
				assert!(
					!t.starts_with("##"),
					"disabled setting must use a single '# ': {line:?}"
				);
			} else {
				assert!(
					t.starts_with("##"),
					"explanatory comment must use '## ': {line:?}"
				);
			}
		}
	}

	// #142: the owner-requested default values.
	#[test]
	fn changed_defaults() {
		let d = Settings::default();
		assert!(d.text_glow, "text_glow should default on");
		assert_eq!(d.text_glow_radius, 5.0);
		assert_eq!(d.text_glow_softness, 0.5);
		assert_eq!(d.background_blur, 8.0);
	}
}
