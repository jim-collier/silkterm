// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use serde::Deserialize;

// Display name (window title, default tab title). The Cargo package / binary
// name lives in Cargo.toml; see README "Renaming the project".
pub const APP_NAME: &str = "SilkTerm";

// Where Help -> Support SilkTerm sends the browser. Points at DONATE.md (the
// canonical list of sponsor options and addresses) rather than
// a single link baked into the binary. HEAD resolves to the repo default branch.
pub const DONATE_URL: &str = "https://github.com/jim-collier/silkterm/blob/HEAD/DONATE.md";

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

// Cross-platform monospace fallback stack (first installed wins), used when
// `use_system_font` is off. Ends in the generic "monospace" so something always
// resolves. Covers common Linux / macOS / Windows faces.
pub const DEFAULT_FONT_STACK: &str = "JetBrains Mono, Fira Code, Cascadia Code, DejaVu Sans Mono, Menlo, Consolas, Liberation Mono, monospace";

// right-click context menu
pub const MENU_LINK: [u8; 3] = [0x6c, 0x9c, 0xff]; // clickable URL

// Menu bar / dropdown colours: bg + text come from the active theme (overridable
// via [colors] menu_background/menu_foreground); hover, border, and the group
// separator are derived shades of the bg, so a custom menu colour stays coherent
// in either a dark or a light direction.
pub fn menu_bg() -> [u8; 3] {
	settings().menu_bg
}
pub fn menu_fg() -> [u8; 3] {
	settings().menu_fg
}
pub fn menu_hover() -> [u8; 3] {
	shade(menu_bg(), 22)
}
pub fn menu_border() -> [u8; 3] {
	shade(menu_bg(), 34)
}
pub fn menu_sep() -> [u8; 3] {
	shade(menu_bg(), 20)
}
// Nudge a colour toward more contrast: lighten a dark base, darken a light one.
fn shade(color: [u8; 3], magnitude: i16) -> [u8; 3] {
	let luminance = (color[0] as i16 * 30 + color[1] as i16 * 59 + color[2] as i16 * 11) / 100;
	let delta = if luminance < 128 {
		magnitude
	} else {
		-magnitude
	};
	let adjust = |channel: u8| (channel as i16 + delta).clamp(0, 255) as u8;
	[adjust(color[0]), adjust(color[1]), adjust(color[2])]
}
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
	pub use_system_font: bool, // true = OS monospace family+size, overriding font_family/font_size
	pub font_family: Option<String>, // comma-separated fallback stack (first installed wins)
	pub font_size: f32,
	pub line_height_scale: f32,
	pub scrollback: usize,
	pub scroll_tau_ms: f32,
	pub wheel_lines: f32,
	pub alt_scroll_lines: f32,
	pub output_ease_lines: f32,
	pub smooth_scroll_apps: bool, // ease the line-jumps of full-screen apps (less/vim/muffer)
	pub margin: f32,              // logical px between content and pane edge
	pub opacity: f32,             // background opacity 0..1 (1 = fully opaque)
	pub transparent_background: bool, // X11: per-pixel bg transparency (text stays opaque) via a GL surface
	pub transparent_background_blur: bool, // X11: ask a KWin/picom compositor to blur the desktop behind the window
	pub background_image: Option<PathBuf>, // resolved path, or None
	pub background_opacity: f32,           // image visibility 0..1
	pub background_fit: Fit,
	pub background_blur: f32, // Gaussian blur sigma applied to the image (0 = none)
	pub text_scrim: bool, // bg-colored blurry halo behind glyphs (readability over busy/transparent bg)
	pub text_scrim_radius: f32, // scrim blur sigma in px
	pub text_scrim_softness: f32, // 0 = hard/solid scrim, 1 = soft/faint (maps to the intensity boost)
	pub text_outline: f32, // antialiased outline around glyphs, px (0 = none; scrim colour rules)
	pub text_scrim_ramp: String, // halo falloff curve: "s" | "gaussian" | "linear" | "log" | "exp"
	pub text_scrim_function: String, // halo build: "dilate" | "sdf" | "dt" | "gaussian" (legacy blur)
	pub text_scrim_regular_weight: bool, // blur bold text at regular weight (uniform halo; crisp text keeps its weight)
	pub embolden_inverse: bool, // render reverse-video (dark-on-light) text bold so it reads as strongly as normal text (the scrim only boosts light-on-dark)
	pub cursor_scrim: bool,     // cursor joins the text scrim halo (default off)
	pub cursor_outline: bool,   // cursor joins the text outline (default on)
	pub cursor_size_height: f32, // cursor height, 1..100% of the cell (from the bottom)
	pub cursor_size_width: f32, // cursor width, 1..100% of the cell (from the left)
	pub cursor_animation: String, // "none" | "phase" | "pulse_vertical" | "pulse_horizontal" | "pulse_both"
	pub cursor_animation_input: String, // "continuous" (default) | "pause" (glide to full + hold while typing)
	pub cursor_blink_rate_ms: f32,      // one animation cycle (ms)
	pub columns: usize,                 // initial window grid size (used when !remember_size)
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
	// chrome colours (menu bar / dropdowns, and pop-out dialogs), from the theme
	// palette; [colors] menu_*/dialog_* keys override
	pub menu_bg: [u8; 3],
	pub menu_fg: [u8; 3],
	pub dialog_bg: [u8; 3],
	pub dialog_fg: [u8; 3],
	pub ansi: [[u8; 3]; 16], // 16-colour ANSI palette, resolved from the active theme
	pub theme: String,       // active theme name (see theme.rs)
	pub theme_mode: String,  // "dark" | "light" | "system"
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			use_system_font: true,
			font_family: Some(DEFAULT_FONT_STACK.to_string()),
			font_size: FALLBACK_FONT_SIZE,
			line_height_scale: 1.22,
			scrollback: 10_000,
			scroll_tau_ms: 230.0, // ~ "Initial scroll speed" 25 (slow/smooth; ramps up under bursts)
			wheel_lines: 3.0,
			alt_scroll_lines: 3.0,
			output_ease_lines: 1.0,
			smooth_scroll_apps: false,
			margin: 8.0,
			opacity: 0.95,
			transparent_background: false,
			transparent_background_blur: false,
			background_image: None,
			background_opacity: 0.10, // image visibility relative to bg color
			background_fit: Fit::Stretch,
			background_blur: 10.0,
			text_scrim: true,
			text_scrim_radius: 5.0,
			text_scrim_softness: 0.5,
			text_outline: 2.0,
			text_scrim_ramp: "s".to_string(),
			text_scrim_function: "sdf".to_string(),
			text_scrim_regular_weight: true,
			embolden_inverse: true,
			cursor_scrim: false,
			cursor_outline: true,
			cursor_size_height: 100.0, // full height
			cursor_size_width: 25.0,   // ~quarter-width bar
			cursor_animation: "pulse_vertical".to_string(),
			cursor_animation_input: "continuous".to_string(),
			cursor_blink_rate_ms: 500.0,
			columns: 160,
			rows: 48,
			remember_size: true,
			remembered_columns: 160,
			remembered_rows: 48,
			// alacritty's default delimiters: keep /.-_~ as word chars so paths
			// and similar stay together on a double-click.
			word_separators: alacritty_terminal::term::SEMANTIC_ESCAPE_CHARS.to_owned(),
			selection_pairs: DEFAULT_SELECTION_PAIRS.to_owned(),
			default_shell: String::new(),
			command_line: String::new(),
			bg: [0x00, 0x00, 0x00],
			fg: [0x88, 0xff, 0xee],
			cursor: [0xff, 0x88, 0xaa],
			focus: [0x55, 0x80, 0xc8],
			menu_bg: crate::theme::MENU_BG_DEF,
			menu_fg: crate::theme::MENU_FG_DEF,
			dialog_bg: [0x20, 0x20, 0x2a],
			dialog_fg: [0xe2, 0xe2, 0xea],
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
	let current = settings();
	if prev == dark || current.theme_mode != "system" {
		return false;
	}
	let pal = crate::theme::resolve(&current.theme, &current.theme_mode, dark);
	let mut new = (*current).clone();
	new.bg = pal.bg;
	new.fg = pal.fg;
	new.cursor = pal.cursor;
	new.focus = pal.focus;
	new.menu_bg = pal.menu_bg;
	new.menu_fg = pal.menu_fg;
	new.dialog_bg = pal.dialog_bg;
	new.dialog_fg = pal.dialog_fg;
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
	let shell = settings().default_shell.clone();
	if shell.trim().is_empty() {
		return None;
	}
	crate::cli::shell_split(&shell).ok()
}

// Parse `selection_pairs` into (open, close) char pairs, in precedence order.
pub fn selection_pairs() -> Vec<(char, char)> {
	settings()
		.selection_pairs
		.split_whitespace()
		.filter_map(|pair| {
			let mut chars = pair.chars();
			Some((chars.next()?, chars.next()?))
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

	if s.use_system_font != orig.use_system_font {
		doc["use_system_font"] = value(s.use_system_font);
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
	if s.text_scrim != orig.text_scrim {
		doc["text_scrim"] = value(s.text_scrim);
	}
	if s.text_scrim_radius != orig.text_scrim_radius {
		doc["text_scrim_radius"] = value(r(s.text_scrim_radius));
	}
	if s.text_scrim_softness != orig.text_scrim_softness {
		doc["text_scrim_softness"] = value(r(s.text_scrim_softness));
	}
	if s.text_outline != orig.text_outline {
		doc["text_outline"] = value(r(s.text_outline));
	}
	if s.text_scrim_ramp != orig.text_scrim_ramp {
		doc["text_scrim_ramp"] = value(&s.text_scrim_ramp);
	}
	if s.text_scrim_function != orig.text_scrim_function {
		doc["text_scrim_function"] = value(&s.text_scrim_function);
	}
	if s.text_scrim_regular_weight != orig.text_scrim_regular_weight {
		doc["text_scrim_regular_weight"] = value(s.text_scrim_regular_weight);
	}
	if s.embolden_inverse != orig.embolden_inverse {
		doc["embolden_inverse"] = value(s.embolden_inverse);
	}
	if s.cursor_scrim != orig.cursor_scrim {
		doc["cursor_scrim"] = value(s.cursor_scrim);
	}
	if s.cursor_outline != orig.cursor_outline {
		doc["cursor_outline"] = value(s.cursor_outline);
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

	let mut set_color = |key: &str, color: [u8; 3], orig_color: [u8; 3]| {
		if color != orig_color {
			doc["colors"][key] = value(format_hex(color));
		}
	};
	set_color("background", s.bg, orig.bg);
	set_color("foreground", s.fg, orig.fg);
	set_color("cursor", s.cursor, orig.cursor);
	set_color("focus", s.focus, orig.focus);

	if let Err(e) = std::fs::write(&path, doc.to_string()) {
		eprintln!("{APP_NAME}: could not save config {}: {e}", path.display());
	}
}

pub fn format_hex(c: [u8; 3]) -> String {
	format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

// The surface is an sRGB format, so the GPU re-encodes linear->sRGB on write.
// Feed it linear values derived from our sRGB byte colors.
pub fn srgb_f32(c: [u8; 3]) -> [f32; 4] {
	[to_linear(c[0]), to_linear(c[1]), to_linear(c[2]), 1.0]
}

pub fn to_linear(b: u8) -> f32 {
	let c = b as f32 / 255.0;
	if c <= 0.04045 {
		c / 12.92
	} else {
		((c + 0.055) / 1.055).powf(2.4)
	}
}

// Inverse of to_linear: encode a linear value back to an sRGB byte. The one
// Rust-side copy - the WGSL lin2srgb in gfx.rs/scrim.rs is necessarily separate.
pub fn from_linear_u8(c: f32) -> u8 {
	let c = c.clamp(0.0, 1.0);
	let s = if c <= 0.0031308 {
		c * 12.92
	} else {
		1.055 * c.powf(1.0 / 2.4) - 0.055
	};
	(s * 255.0 + 0.5) as u8
}

// config file loading

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawConfig {
	use_system_font: Option<bool>,
	font_family: Option<String>,
	font_size: Option<f32>,
	line_height_scale: Option<f32>,
	scrollback: Option<usize>,
	scroll_tau_ms: Option<f32>,
	wheel_lines: Option<f32>,
	alt_scroll_lines: Option<f32>,
	output_ease_lines: Option<f32>,
	smooth_scroll_apps: Option<bool>,
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
	text_scrim: Option<bool>,
	text_scrim_radius: Option<f32>,
	text_scrim_softness: Option<f32>,
	text_outline: Option<f32>,
	text_scrim_ramp: Option<String>,
	text_scrim_function: Option<String>,
	text_scrim_regular_weight: Option<bool>,
	embolden_inverse: Option<bool>,
	cursor_scrim: Option<bool>,
	cursor_outline: Option<bool>,
	cursor_size_height: Option<f32>,
	cursor_size_width: Option<f32>,
	cursor_animation: Option<String>,
	cursor_animation_input: Option<String>,
	cursor_blink_rate_ms: Option<f32>,
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
	menu_background: Option<String>,
	menu_foreground: Option<String>,
	dialog_background: Option<String>,
	dialog_foreground: Option<String>,
}

fn load() -> Settings {
	let Some(path) = config_path() else {
		return Settings::default();
	};
	if !path.exists() {
		if let Some(dir) = path.parent() {
			let _ = std::fs::create_dir_all(dir);
		}
		if let Err(e) = std::fs::write(&path, DEFAULT_CONFIG) {
			eprintln!(
				"{APP_NAME}: could not create config {}: {e}",
				path.display()
			);
		}
	}
	// Migrate an older config in place (rename/remove changed keys) then backfill
	// any keys it's missing, so an updated config stays current without clobbering
	// the user's existing values.
	migrate_config(&path);
	backfill_config(&path);
	reorder_config(&path);
	let raw: RawConfig = match std::fs::read_to_string(&path) {
		Ok(text) => parse_lenient(&text, &path),
		Err(_) => RawConfig::default(),
	};
	resolve(raw)
}

// Parse the config into RawConfig, tolerating individually-broken lines: on a
// parse error, blank the offending line (located via the error span) and retry -
// so one bad value (e.g. `cursor_blink = enable`) drops just that setting instead
// of sinking EVERY setting to its default. Bounded so a pathological file can't
// loop. Unknown-but-valid keys are already ignored by serde; this handles the
// syntax/type errors that otherwise fail the whole document.
fn parse_lenient(text: &str, path: &std::path::Path) -> RawConfig {
	let mut lines: Vec<String> = lenient_floats(text).lines().map(str::to_string).collect();
	for _ in 0..=lines.len() {
		let joined = lines.join("\n");
		match toml::from_str::<RawConfig>(&joined) {
			Ok(raw) => return raw,
			Err(e) => {
				// byte span -> 0-based line index of the error start
				let line_index = e.span().map(|span| {
					joined[..span.start.min(joined.len())]
						.bytes()
						.filter(|&b| b == b'\n')
						.count()
				});
				match line_index {
					Some(i) if i < lines.len() => {
						eprintln!(
							"{APP_NAME}: {} line {}: ignoring invalid setting `{}`",
							path.display(),
							i + 1,
							lines[i].trim()
						);
						lines[i].clear(); // drop just this line; keep indices stable for the next error
					}
					_ => {
						eprintln!(
							"{APP_NAME}: {}: config parse error, using defaults ({e})",
							path.display()
						);
						return RawConfig::default();
					}
				}
			}
		}
	}
	RawConfig::default()
}

// TOML requires a leading zero on floats (`.25` is a parse error that would sink
// the whole file). Rewrite a bare-decimal value right after `=` to `0.25`.
fn lenient_floats(text: &str) -> String {
	text.lines()
		.map(|line| {
			let Some(eq_pos) = line.find('=') else {
				return line.to_string();
			};
			let (head, after) = line.split_at(eq_pos + 1);
			let val = after.trim_start();
			let whitespace = &after[..after.len() - val.len()];
			if let Some(rest) = val.strip_prefix('.').filter(|r| starts_digit(r)) {
				format!("{head}{whitespace}0.{rest}")
			} else if let Some(rest) = val.strip_prefix("-.").filter(|r| starts_digit(r)) {
				format!("{head}{whitespace}-0.{rest}")
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
	let color = |raw: Option<String>, fallback: [u8; 3]| {
		raw.as_deref().and_then(parse_hex).unwrap_or(fallback)
	};
	Settings {
		// Default enabled, but a config that predates the key and set an explicit
		// font_family keeps that font (infer off) instead of being overridden.
		use_system_font: raw.use_system_font.unwrap_or(raw.font_family.is_none()),
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
		// MUST clamp: scroll's backlog clamp uses this as its lower bound, and
		// f32::clamp panics (aborts, in release) when min > max - an over-range
		// value here killed the terminal on the first scrolling output.
		output_ease_lines: raw
			.output_ease_lines
			.unwrap_or(d.output_ease_lines)
			.clamp(0.0, crate::scroll::MAX_BACKLOG),
		smooth_scroll_apps: raw.smooth_scroll_apps.unwrap_or(d.smooth_scroll_apps),
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
		text_scrim: raw.text_scrim.unwrap_or(d.text_scrim),
		text_scrim_radius: raw
			.text_scrim_radius
			.unwrap_or(d.text_scrim_radius)
			.clamp(0.0, 50.0),
		text_scrim_softness: raw
			.text_scrim_softness
			.unwrap_or(d.text_scrim_softness)
			.clamp(0.0, 1.0),
		text_outline: raw.text_outline.unwrap_or(d.text_outline).clamp(0.0, 8.0),
		text_scrim_ramp: match raw.text_scrim_ramp.as_deref() {
			Some("linear") => "linear".to_string(),
			Some("gaussian") => "gaussian".to_string(),
			Some("s") => "s".to_string(),
			Some("log") => "log".to_string(),
			Some("exp") => "exp".to_string(),
			_ => d.text_scrim_ramp.clone(), // missing/unknown -> default (S-curve)
		},
		text_scrim_function: match raw.text_scrim_function.as_deref() {
			Some("dilate") => "dilate".to_string(),
			Some("sdf") => "sdf".to_string(),
			Some("dt") => "dt".to_string(),
			Some("gaussian") => "gaussian".to_string(),
			_ => d.text_scrim_function.clone(), // missing/unknown -> default (SDF)
		},
		text_scrim_regular_weight: raw
			.text_scrim_regular_weight
			.unwrap_or(d.text_scrim_regular_weight),
		embolden_inverse: raw.embolden_inverse.unwrap_or(d.embolden_inverse),
		cursor_scrim: raw.cursor_scrim.unwrap_or(d.cursor_scrim),
		cursor_outline: raw.cursor_outline.unwrap_or(d.cursor_outline),
		cursor_size_height: raw
			.cursor_size_height
			.unwrap_or(d.cursor_size_height)
			.clamp(1.0, 100.0),
		cursor_size_width: raw
			.cursor_size_width
			.unwrap_or(d.cursor_size_width)
			.clamp(1.0, 100.0),
		cursor_animation: raw.cursor_animation.unwrap_or(d.cursor_animation),
		cursor_animation_input: raw
			.cursor_animation_input
			.unwrap_or(d.cursor_animation_input),
		cursor_blink_rate_ms: raw
			.cursor_blink_rate_ms
			.unwrap_or(d.cursor_blink_rate_ms)
			.max(50.0),
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
		menu_bg: color(raw.colors.menu_background, pal.menu_bg),
		menu_fg: color(raw.colors.menu_foreground, pal.menu_fg),
		dialog_bg: color(raw.colors.dialog_background, pal.dialog_bg),
		dialog_fg: color(raw.colors.dialog_foreground, pal.dialog_fg),
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

// The size the text is actually rendered at: the OS monospace size while
// `use_system_font` is on, else the configured `font_size`.
pub fn effective_font_size() -> f32 {
	let current = settings();
	if current.use_system_font {
		default_font_size()
	} else {
		current.font_size
	}
}

// Resolve the background image: an explicit path (absolute, or a filename
// relative to the config dir), else auto-detect backgrounds/background.{png,jpg,jpeg}
// under the config dir.
fn resolve_bg_image(explicit: Option<String>) -> Option<PathBuf> {
	let dir = config_path()?.parent()?.to_path_buf();
	if let Some(given) = explicit.filter(|value| !value.trim().is_empty()) {
		let path = PathBuf::from(&given);
		let path = if path.is_absolute() {
			path
		} else {
			dir.join(given)
		};
		return path.exists().then_some(path);
	}
	let bg_dir = dir.join("backgrounds");
	["background.png", "background.jpg", "background.jpeg"]
		.into_iter()
		.map(|name| bg_dir.join(name))
		.find(|path| path.exists())
}

// A config file's settings as (table, key, original-line) - `table` is None for
// top-level keys, Some("colors") for a `[colors]` entry. Recognizes both active
// (`k = ...`) and commented (`# k = ...`) lines.
fn setting_lines(text: &str) -> Vec<(Option<String>, String, String)> {
	let mut table: Option<String> = None;
	let mut out = Vec::new();
	for line in text.lines() {
		if let Some(name) = line_table(line) {
			table = Some(name.to_string());
		} else if let Some(key) = line_setting_key(line) {
			out.push((table.clone(), key.to_string(), line.to_string()));
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
		if let Some(name) = line_table(line) {
			table = Some(name.to_string());
			pending.clear();
			group_break = true;
		} else if let Some(key) = line_setting_key(line) {
			let mut block = std::mem::take(&mut pending);
			block.push(line.to_string());
			out.push((table.clone(), key.to_string(), block, group_break));
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
	let trimmed = line.trim();
	trimmed.strip_prefix('[').and_then(|r| r.strip_suffix(']'))
}

fn line_setting_key(line: &str) -> Option<&str> {
	let trimmed = line.trim_start();
	let trimmed = trimmed
		.strip_prefix('#')
		.map(str::trim_start)
		.unwrap_or(trimmed);
	let end =
		trimmed.find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))?;
	let key = &trimmed[..end];
	if key.is_empty() {
		return None;
	}
	trimmed[end..].trim_start().starts_with('=').then_some(key)
}

// Keys that were renamed across versions (old -> new). A rename copies the value
// and preserves the comment/active state; if the new key is already present the
// old one is just dropped.
const CONFIG_RENAMES: &[(&str, &str)] = &[
	("cursor_size_vertical", "cursor_size_height"),
	("cursor_size_horizontal", "cursor_size_width"),
	("text_glow_border", "text_outline"),
	("text_glow", "text_scrim"),
	("text_glow_radius", "text_scrim_radius"),
	("text_glow_softness", "text_scrim_softness"),
	("text_glow_ramp", "text_scrim_ramp"),
	("text_glow_regular_weight", "text_scrim_regular_weight"),
	("cursor_glow", "cursor_scrim"),
];
// Keys that no longer exist and should be removed from an existing config. The
// cursor_shape/cursor_blink_style/cursor_insert_shape line was superseded by the
// cursor_size_*/cursor_animation/cursor_blink_rate_ms geometry+animation model.
const CONFIG_REMOVED: &[&str] = &[
	"cursor_overwrite_shape",
	"cursor_insert_shape",
	"cursor_blink",
	"cursor_shape",
	"cursor_blink_style",
];

// Migrate an existing config in place across program updates: rename keys whose
// name changed, drop keys that no longer exist. Preserves the user's values,
// comments, and layout (line-based, like backfill). New keys are added by
// backfill_config; this only renames/removes, so run it first.
fn migrate_config(path: &std::path::Path) {
	let Ok(text) = std::fs::read_to_string(path) else {
		return;
	};
	if let Some(out) = migrate_config_text(&text) {
		if let Err(e) = std::fs::write(path, out) {
			eprintln!(
				"{APP_NAME}: could not migrate config {}: {e}",
				path.display()
			);
		}
	}
}

// The rename/remove transform, as a pure fn (testable). Returns Some(new text)
// only if something changed.
fn migrate_config_text(text: &str) -> Option<String> {
	// new-key targets already present (active or commented): don't create a dup
	let have_new: std::collections::HashSet<&str> = text
		.lines()
		.filter_map(line_setting_key)
		.filter(|key| CONFIG_RENAMES.iter().any(|(_, new_name)| new_name == key))
		.collect();

	let has_key = |key: &str| {
		text.lines()
			.filter_map(line_setting_key)
			.any(|existing| existing == key)
	};
	let active = |line: &str| !line.trim_start().starts_with('#');

	let mut changed = false;
	let mut out: Vec<String> = Vec::new();
	let mut active_font_family: Option<usize> = None; // index in `out`, for the boolean migration
	for line in text.lines() {
		let kept = match line_setting_key(line) {
			Some(key) if CONFIG_REMOVED.contains(&key) => {
				changed = true;
				continue; // drop
			}
			Some(key) => match CONFIG_RENAMES.iter().find(|(old, _)| *old == key) {
				Some((_, new)) if !have_new.contains(new) => {
					changed = true;
					line.replacen(key, new, 1) // key is the first token
				}
				Some(_) => {
					changed = true;
					continue; // new key already there; drop the old
				}
				None => line.to_string(),
			},
			None => line.to_string(),
		};
		if line_setting_key(&kept) == Some("font_family") && active(&kept) {
			active_font_family = Some(out.len());
		}
		out.push(kept);
	}
	// A config predating `use_system_font` that pinned an explicit font_family keeps
	// that font: insert use_system_font = false so backfill won't add =true (default)
	// and silently override it.
	if let Some(idx) = active_font_family {
		if !has_key("use_system_font") {
			out.insert(idx + 1, "use_system_font = false".to_string());
			changed = true;
		}
	}
	changed.then(|| {
		let mut joined = out.join("\n");
		joined.push('\n');
		joined
	})
}

// Revert config keys to their defaults: drop the active assignment from
// config.toml (dotted keys address the [colors] table), then backfill so the
// key comes back as the template's commented default line. Used by the Settings
// dialog's revert-to-default buttons.
pub fn revert_keys(keys: &[&str]) {
	if keys.is_empty() {
		return;
	}
	let Some(path) = config_path() else { return };
	let Ok(text) = std::fs::read_to_string(&path) else {
		return;
	};
	let Ok(mut doc) = text.parse::<toml_edit::DocumentMut>() else {
		return;
	};
	for full_key in keys {
		match full_key.split_once('.') {
			Some((table, key)) => {
				if let Some(tbl) = doc.get_mut(table).and_then(|item| item.as_table_mut()) {
					tbl.remove(key);
				}
			}
			None => {
				doc.remove(full_key);
			}
		}
	}
	let _ = std::fs::write(&path, doc.to_string());
	backfill_config(&path);
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
		.map(|(table, key, _)| (table, key))
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
		match lines
			.iter()
			.position(|line| line_table(line) == Some("colors"))
		{
			Some(i) => {
				for (offset, line) in colors.into_iter().enumerate() {
					lines.insert(i + 1 + offset, line);
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
		match lines.iter().position(|line| line_table(line).is_some()) {
			Some(i) => {
				// avoid a double blank if the line above the table is already blank
				if i > 0
					&& lines[i - 1].trim().is_empty()
					&& top.first().is_some_and(|line| line.is_empty())
				{
					top.remove(0);
				}
				for (offset, line) in top.into_iter().enumerate() {
					lines.insert(i + offset, line);
				}
			}
			None => lines.extend(top),
		}
	}
	let mut out = lines.join("\n");
	out.push('\n');
	if out != text {
		if let Err(e) = std::fs::write(path, out) {
			eprintln!(
				"{APP_NAME}: could not update config {}: {e}",
				path.display()
			);
		}
	}
}

// Rewrite an existing config into the template's canonical section order, in
// place, when it has drifted (e.g. after an update reorders/regroups keys or the
// user's file predates the current layout). Pure transform in
// `reorder_config_text`; this just does the IO. Runs after migrate + backfill so
// every current key is present before it normalizes their order.
fn reorder_config(path: &std::path::Path) {
	let Ok(text) = std::fs::read_to_string(path) else {
		return;
	};
	if let Some(out) = reorder_config_text(&text) {
		if let Err(e) = std::fs::write(path, out) {
			eprintln!(
				"{APP_NAME}: could not reorder config {}: {e}",
				path.display()
			);
		}
	}
}

// Header shown above unknown (non-template) top-level keys we carry through, so
// nothing the user added is lost when we regenerate the template's layout.
const REORDER_EXTRA_HEADER: &str = "## Settings not in the current template (kept as-is).";

// Reorder + regroup an existing config to match `DEFAULT_CONFIG`'s section order,
// preserving each setting's VALUE and enabled/commented state while refreshing the
// surrounding explanatory comments and section headers from the template. Keys the
// template doesn't define, and any user-added tables (e.g. `[themes.*]`), are kept
// verbatim so nothing is dropped. Pure + idempotent: returns Some only when the
// result differs from the input (so a canonical file is never rewritten).
fn reorder_config_text(text: &str) -> Option<String> {
	use std::collections::HashMap;
	use std::collections::HashSet;

	let known: HashSet<(Option<String>, String)> = setting_lines(DEFAULT_CONFIG)
		.into_iter()
		.map(|(table, key, _)| (table, key))
		.collect();
	let is_active = |line: &str| !line.trim_start().starts_with('#');
	// store the user's line for a known key, preferring an active line over a
	// commented duplicate (an odd but possible hand-edit).
	let keep = |map: &mut HashMap<String, String>, key: &str, line: &str| match map.get(key) {
		Some(existing) if is_active(existing) => {}
		_ => {
			map.insert(key.to_string(), line.to_string());
		}
	};

	let lines: Vec<&str> = text.lines().collect();
	let first_table = lines.iter().position(|line| line_table(line).is_some());
	let (top_lines, table_lines) = match first_table {
		Some(i) => (&lines[..i], &lines[i..]),
		None => (&lines[..], &lines[lines.len()..]),
	};

	// --- top-level region: known keys -> map, unknown keys (+their comment block) kept ---
	let mut user_top: HashMap<String, String> = HashMap::new();
	let mut extra_top: Vec<String> = Vec::new();
	let mut pending: Vec<String> = Vec::new();
	for &line in top_lines {
		if let Some(key) = line_setting_key(line) {
			if known.contains(&(None, key.to_string())) {
				keep(&mut user_top, key, line);
			} else {
				extra_top.append(&mut pending);
				extra_top.push(line.to_string());
			}
			pending.clear();
		} else if line.trim().is_empty() {
			pending.clear();
		} else if line.trim_start().starts_with('#') {
			pending.push(line.to_string());
		} else {
			pending.clear();
		}
	}

	// --- table region: split into blocks, each header carrying its preceding
	// comment block. The first [colors] block feeds the color map; every other
	// table (and any extra [colors]) is preserved verbatim.
	let mut blocks: Vec<Vec<String>> = Vec::new();
	let mut pending_block: Vec<String> = Vec::new();
	for &line in table_lines {
		if line_table(line).is_some() {
			let mut block: Vec<String> = std::mem::take(&mut pending_block);
			block.push(line.to_string());
			blocks.push(block);
		} else if line.trim().is_empty() {
			if let Some(last_block) = blocks.last_mut() {
				last_block.append(&mut pending_block);
				last_block.push(line.to_string());
			} else {
				pending_block.clear();
			}
		} else if line.trim_start().starts_with('#') {
			pending_block.push(line.to_string());
		} else if let Some(last_block) = blocks.last_mut() {
			last_block.append(&mut pending_block);
			last_block.push(line.to_string());
		} else {
			pending_block.clear();
		}
	}
	if let Some(last_block) = blocks.last_mut() {
		last_block.append(&mut pending_block);
	}

	let mut user_colors: HashMap<String, String> = HashMap::new();
	let mut extra_colors: Vec<String> = Vec::new();
	let mut extra_tables: Vec<String> = Vec::new();
	let mut took_colors = false;
	for block in blocks {
		let header_idx = block
			.iter()
			.position(|line| line_table(line).is_some())
			.unwrap();
		let name = line_table(&block[header_idx]).unwrap().to_string();
		if name == "colors" && !took_colors {
			took_colors = true;
			for line in &block[header_idx + 1..] {
				if let Some(key) = line_setting_key(line) {
					if known.contains(&(Some("colors".to_string()), key.to_string())) {
						keep(&mut user_colors, key, line);
					} else {
						extra_colors.push(line.clone());
					}
				}
			}
		} else {
			extra_tables.extend(block);
		}
	}

	// --- render: walk the template, substitute the user's line for each key it
	// set, and splice the preserved extras in TOML-valid positions (unknown
	// top-level keys before [colors]; extra color keys under it; extra tables last).
	let template_lines: Vec<&str> = DEFAULT_CONFIG.lines().collect();
	let colors_idx = template_lines
		.iter()
		.position(|line| line_table(line) == Some("colors"))
		.unwrap();
	let mut intro = colors_idx; // first line of the [colors] intro comment block
	while intro > 0 && template_lines[intro - 1].trim_start().starts_with('#') {
		intro -= 1;
	}

	let mut out: Vec<String> = Vec::new();
	let mut table: Option<String> = None;
	for (i, line) in template_lines.iter().enumerate() {
		if i == intro && !extra_top.is_empty() {
			if !out.last().is_none_or(String::is_empty) {
				out.push(String::new());
			}
			out.push(REORDER_EXTRA_HEADER.to_string());
			out.push(String::new());
			out.extend(extra_top.iter().cloned());
			out.push(String::new());
		}
		if let Some(name) = line_table(line) {
			table = Some(name.to_string());
			out.push(line.to_string());
		} else if let Some(key) = line_setting_key(line) {
			let replacement = match table.as_deref() {
				None => user_top.get(key),
				Some("colors") => user_colors.get(key),
				_ => None,
			};
			out.push(replacement.cloned().unwrap_or_else(|| line.to_string()));
		} else {
			out.push(line.to_string());
		}
	}
	out.extend(extra_colors); // still inside [colors], the template's last table
	if !extra_tables.is_empty() {
		out.push(String::new());
		out.extend(extra_tables);
	}

	let mut joined = out.join("\n");
	joined.push('\n');
	(joined != text).then_some(joined)
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
## Sections and key order below are refreshed automatically on launch; your
## values and which settings are enabled are kept.

##=============================================================================
## Font
##=============================================================================

## Use the OS default monospace font (family + size). When true this overrides
## font_family / font_size below. Turn off to use them instead.
use_system_font = true

## Font family: a comma-separated fallback stack (first installed wins). Used
## only when use_system_font = false. Ends in the generic "monospace".
font_family = "JetBrains Mono, Fira Code, Cascadia Code, DejaVu Sans Mono, Menlo, Consolas, Liberation Mono, monospace"

## Font size in logical pixels. Used only when use_system_font = false.
# font_size = 17.0

## Line height as a multiple of the font's natural height (1.0 = tight).
line_height_scale = 1.22

##=============================================================================
## Window
##=============================================================================

## Pixels between the text and the pane edge.
margin = 8.0

## Initial window size, in character cells (used when remember_size = false).
columns = 160
rows = 48

## Launch at the last window size instead of columns/rows (default on). The
## remembered size is updated automatically whenever you resize the window (kept
## separate from columns/rows so unchecking reverts to your defined size).
# remember_size = true
# remembered_columns = 160
# remembered_rows = 48

##=============================================================================
## Background and transparency
##=============================================================================

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
# background_opacity = 0.10

## How the image fits: "stretch" (fill, ignore aspect) or "zoom" (cover, keep aspect).
# background_fit = "stretch"

## Gaussian blur applied to the background image (sigma in pixels; 0 = none).
# background_blur = 10.0

##=============================================================================
## Text scrim
##=============================================================================

## Text readability scrim: a blurry background-colored halo behind each glyph, so
## text stays legible over a light/busy background or near-transparent terminal.
## On by default; uncomment and set text_scrim = false to disable.
# text_scrim = true
# text_scrim_radius = 5.0     ## scrim halo radius in pixels
# text_scrim_softness = 0.5   ## 0 = hard/solid scrim, 1 = soft/faint
# text_outline = 2.0          ## antialiased outline around glyphs, in pixels (0 = none)
# text_scrim_function = "sdf" ## halo shape: "dilate" (square), "sdf" (round, full corners), "dt", or "gaussian" (legacy, corners recede)
# text_scrim_ramp = "s"       ## halo falloff curve: "s", "gaussian", "linear", "log", or "exp"
# text_scrim_regular_weight = true  ## blur bold text at regular weight so its halo matches non-bold text
# embolden_inverse = true     ## render reverse-video (dark-on-light) text bold so it reads as strongly as normal
# cursor_scrim = false        ## the cursor joins the scrim halo (default off)
# cursor_outline = true       ## the cursor joins the text outline (default on)

##=============================================================================
## Cursor
##=============================================================================

## Cursor size, as a percent of the cell: height grows from the bottom, width from
## the left. Together they make any shape: a thin bar (height 100 / width 25), an
## underline (15 / 100), or a block (100 / 100). Used when the app doesn't set its
## own; alt-screen apps (vim, less) still control theirs.
# cursor_size_height = 100
# cursor_size_width = 25

## Cursor animation: "none" (steady), "phase" (smooth fade), or a pulse that
## grows/shrinks each cycle - "pulse_vertical", "pulse_horizontal", "pulse_both".
## The cursor always slides smoothly as you type.
# cursor_animation = "pulse_vertical"

## What the animation does while you're typing. "continuous" (default) keeps
## animating right through typing. "pause" glides the cursor to full size and
## holds it while there's input, then resumes the animation once input has been
## idle briefly - so it doesn't restart on every keystroke.
# cursor_animation_input = "continuous"

## Cursor animation cycle length, in milliseconds (blink rate).
# cursor_blink_rate_ms = 500

##=============================================================================
## Selection
##=============================================================================

## Delimiters that bound a double-click word selection. The default keeps
## / . - _ ~ as part of a word, so paths stay selected whole. Leave commented
## for the default; set to your own string of separator characters to override.
# word_separators = ",|:\"' ()[]{}<>"

## Pairs whose contents a double-click selects when the click is inside a matched
## pair (highest precedence first). Leave commented for the default.
# selection_pairs = "`` \"\" '' {} () [] <>"

##=============================================================================
## Shell
##=============================================================================

## Default shell/command for new windows, tabs, and panes when nothing else is
## given (CLI --shell and per-pane inheritance take precedence). argv-split, so
## "bash --norc" works. Leave blank/commented to use the system default shell.
# default_shell = "bash --norc"

## Default command line applied when SilkTerm is launched with no arguments - the
## same window/tab/pane options the CLI accepts (see --help). Any actual
## command-line arguments override this entirely. Leave blank/commented for none.
# command_line = "--new-pane --right --size 35%"

##=============================================================================
## Scrolling
##=============================================================================

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

## Ease the whole-line jumps of full-screen apps that own the screen (less, vim,
## nano, htop, tmux, ...) so their scrolling slides instead of snapping. The
## revealed strip fills with the background during the ~quarter-second slide.
## Experimental; only clean line-scrolls are eased (big page-jumps still snap).
# smooth_scroll_apps = false

##=============================================================================
## Theme and colours
##=============================================================================

## Colour theme. Pick a built-in (SilkTerm, Matrix, Retro Amber) or one you add in
## a [themes.*] table. theme_mode is "dark", "light", or "system" (follow the OS).
theme = "SilkTerm"
theme_mode = "dark"

## Per-colour overrides on top of the theme (uncomment any to tweak one colour).
## The menu_*/dialog_* keys recolour the chrome (menu bar + dropdowns, and the
## pop-out Settings/About dialogs); by default every theme shares the same neutral
## chrome. Menu hover/border shades derive from menu_background automatically.
[colors]
# background        = "#000000"
# foreground        = "#d2d2da"
# cursor            = "#7a9ad0"
# focus             = "#5580c8"
# menu_background   = "#36363b"
# menu_foreground   = "#f0f0f2"
# dialog_background = "#20202a"
# dialog_foreground = "#e2e2ea"
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

	// #142: the default values.
	#[test]
	fn changed_defaults() {
		let d = Settings::default();
		assert!(d.text_scrim, "text_scrim should default on");
		assert_eq!(d.text_scrim_radius, 5.0);
		assert_eq!(d.text_scrim_softness, 0.5);
		assert_eq!(d.text_outline, 2.0);
		assert_eq!(d.text_scrim_ramp, "s");
		assert_eq!(d.text_scrim_function, "sdf");
		assert!(d.text_scrim_regular_weight);
		assert!(!d.cursor_scrim, "cursor scrim halo defaults off");
		assert!(d.cursor_outline, "cursor outline defaults on");
		assert_eq!(d.background_blur, 10.0);
		assert_eq!(d.background_opacity, 0.10);
	}

	// Scrim function + the five falloff curves resolve; unknown values fall to the
	// defaults (sdf / s-curve).
	#[test]
	fn scrim_function_and_ramp_resolve() {
		let p = std::path::Path::new("test.toml");
		for f in ["dilate", "sdf", "dt", "gaussian"] {
			let s = resolve(parse_lenient(
				&format!("text_scrim_function = \"{f}\"\n"),
				p,
			));
			assert_eq!(s.text_scrim_function, f);
		}
		for r in ["s", "gaussian", "linear", "log", "exp"] {
			let s = resolve(parse_lenient(&format!("text_scrim_ramp = \"{r}\"\n"), p));
			assert_eq!(s.text_scrim_ramp, r);
		}
		let s = resolve(parse_lenient("text_scrim_function = \"bogus\"\n", p));
		assert_eq!(s.text_scrim_function, "sdf", "unknown -> default");
		let s = resolve(parse_lenient("text_scrim_ramp = \"bogus\"\n", p));
		assert_eq!(s.text_scrim_ramp, "s", "unknown -> default");
	}

	// An over-range output_ease_lines must clamp: scroll's backlog clamp uses it
	// as a lower bound and panics (aborts, in release) when it exceeds the cap.
	#[test]
	fn output_ease_lines_clamps_to_backlog_cap() {
		let raw = parse_lenient(
			"output_ease_lines = 20.0\n",
			std::path::Path::new("test.toml"),
		);
		let s = resolve(raw);
		assert!(s.output_ease_lines <= crate::scroll::MAX_BACKLOG);
		let raw = parse_lenient(
			"output_ease_lines = -3.0\n",
			std::path::Path::new("test.toml"),
		);
		assert!(resolve(raw).output_ease_lines >= 0.0);
	}

	// One syntax-broken line must not sink the valid settings around it.
	#[test]
	fn parse_lenient_drops_only_the_bad_line() {
		let text = "opacity = 0.7\ncursor_blink = enable\nmargin = 12.0\n";
		let raw = parse_lenient(text, std::path::Path::new("test.toml"));
		assert_eq!(raw.opacity, Some(0.7)); // before the bad line
		assert_eq!(raw.margin, Some(12.0)); // after the bad line
	}

	#[test]
	fn chrome_colors_default_and_override() {
		// theme provides the chrome; the default matches the shared menu colours
		let d = Settings::default();
		assert_eq!(d.menu_bg, crate::theme::MENU_BG_DEF);
		assert_eq!(d.menu_fg, crate::theme::MENU_FG_DEF);
		// a [colors] override wins; unspecified chrome stays at the theme default
		let raw = parse_lenient(
			"[colors]\nmenu_background = \"#123456\"\ndialog_foreground = \"#abcdef\"\n",
			std::path::Path::new("test.toml"),
		);
		let s = resolve(raw);
		assert_eq!(s.menu_bg, [0x12, 0x34, 0x56]);
		assert_eq!(s.dialog_fg, [0xab, 0xcd, 0xef]);
		assert_eq!(s.menu_fg, crate::theme::MENU_FG_DEF);
	}

	#[test]
	fn migrate_renames_glow_border_to_outline() {
		// an existing (active) text_glow_border keeps its value under the new name
		let out =
			migrate_config_text("text_glow_border = 2.03\nmargin = 8.0\n").expect("should rename");
		assert!(!out.contains("text_glow_border"), "old name gone: {out:?}");
		assert!(
			out.contains("text_outline = 2.03"),
			"value preserved: {out:?}"
		);
	}

	// The text-glow -> text-scrim rename preserves values and active/commented state.
	#[test]
	fn migrate_renames_glow_to_scrim() {
		let out = migrate_config_text(
			"text_glow = false\ntext_glow_radius = 7.0\n# cursor_glow = false\ntext_glow_ramp = \"linear\"\n",
		)
		.expect("should rename");
		assert!(!out.contains("text_glow"), "old names gone: {out:?}");
		assert!(
			out.contains("text_scrim = false"),
			"value + active kept: {out:?}"
		);
		assert!(
			out.contains("text_scrim_radius = 7.0"),
			"value kept: {out:?}"
		);
		assert!(
			out.contains("# cursor_scrim = false"),
			"commented state kept: {out:?}"
		);
		assert!(
			out.contains("text_scrim_ramp = \"linear\""),
			"string value kept: {out:?}"
		);
	}

	// In-place migration: drop obsolete cursor keys, keep the rest.
	#[test]
	fn migrate_config_renames_and_removes() {
		let text = "opacity = 0.7\ncursor_shape = \"block\"\ncursor_insert_shape = \"bar\"\ncursor_blink_style = \"phase\"\nmargin = 12.0\n";
		let out = migrate_config_text(text).expect("should change");
		assert!(!out.contains("cursor_shape"), "obsolete removed: {out:?}");
		assert!(!out.contains("cursor_insert_shape"), "obsolete removed");
		assert!(!out.contains("cursor_blink_style"), "obsolete removed");
		assert!(
			out.contains("opacity = 0.7") && out.contains("margin = 12.0"),
			"kept the rest"
		);
	}

	// The vertical/horizontal -> height/width rename preserves the value.
	#[test]
	fn migrate_config_renames_cursor_size() {
		let out = migrate_config_text("cursor_size_vertical = 50\ncursor_size_horizontal = 25\n")
			.expect("should change");
		assert!(out.contains("cursor_size_height = 50"), "{out:?}");
		assert!(out.contains("cursor_size_width = 25"));
		assert!(!out.contains("cursor_size_vertical") && !out.contains("cursor_size_horizontal"));
	}

	// A config with nothing to migrate is left untouched (no needless rewrite).
	#[test]
	fn migrate_config_noop_when_current() {
		assert!(migrate_config_text("opacity = 0.7\ncursor_animation = \"phase\"\n").is_none());
	}

	// A pre-boolean config with an explicit font_family keeps it (use_system_font=false
	// inserted), so the backfilled default (true) can't silently override the font.
	#[test]
	fn migrate_config_pins_use_system_font_for_explicit_family() {
		let out = migrate_config_text("font_family = \"Iosevka\"\n").expect("should change");
		assert!(out.contains("font_family = \"Iosevka\""));
		assert!(out.contains("use_system_font = false"), "{out:?}");
		// but a commented family (following the system) doesn't trigger the insert
		assert!(migrate_config_text("# font_family = \"Iosevka\"\n").is_none());
		// and one that already has the key is left alone
		assert!(
			migrate_config_text("use_system_font = true\nfont_family = \"Iosevka\"\n").is_none()
		);
	}

	// A freshly-written template is already canonical: no rewrite on launch.
	#[test]
	fn reorder_noop_on_template() {
		assert!(
			reorder_config_text(DEFAULT_CONFIG).is_none(),
			"template must be its own fixed point"
		);
	}

	// Out-of-order keys are moved to template order; the [colors] table stays last.
	#[test]
	fn reorder_sorts_to_template_order() {
		let text = "scrollback = 5000\nmargin = 4.0\nuse_system_font = false\n";
		let out = reorder_config_text(text).expect("should reorder");
		let font = out.find("use_system_font").unwrap();
		let margin = out.find("margin =").unwrap();
		let scroll = out.find("scrollback =").unwrap();
		let colors = out.find("[colors]").unwrap();
		assert!(font < margin && margin < scroll, "template order: {out}");
		assert!(scroll < colors, "colors table stays last");
	}

	// Values and enabled/commented state survive the rewrite.
	#[test]
	fn reorder_preserves_value_and_state() {
		let text = "margin = 12.5\n# opacity = 0.5\nfont_size = 20.0\n";
		let out = reorder_config_text(text).expect("should reorder");
		assert!(out.contains("margin = 12.5"), "active value kept: {out}");
		assert!(out.contains("# opacity = 0.5"), "commented state kept");
		assert!(
			out.contains("font_size = 20.0"),
			"activated key kept active"
		);
		// a color override is placed under [colors]
		let with_color = reorder_config_text("[colors]\nbackground = \"#123456\"\n").unwrap();
		let ci = with_color.find("[colors]").unwrap();
		let bg = with_color.find("background = \"#123456\"").unwrap();
		assert!(bg > ci, "color override under the table: {with_color}");
	}

	// User-added tables (e.g. a [themes.*] table) are carried through verbatim.
	#[test]
	fn reorder_preserves_unknown_table() {
		let text = "margin = 8.0\n\n[themes.mine.dark]\nbackground = \"#010203\"\nforeground = \"#eeeeee\"\n";
		let out = reorder_config_text(text).expect("should reorder");
		assert!(
			out.contains("[themes.mine.dark]"),
			"kept table header: {out}"
		);
		assert!(out.contains("background = \"#010203\""), "kept table body");
		// and it lands after the managed [colors] table
		assert!(
			out.find("[themes.mine.dark]").unwrap() > out.find("[colors]").unwrap(),
			"extra table placed last"
		);
	}

	// An unknown top-level key is preserved under an "extras" header before [colors].
	#[test]
	fn reorder_preserves_unknown_top_key() {
		let out = reorder_config_text("mystery_key = 7\nmargin = 8.0\n").expect("should reorder");
		assert!(out.contains("mystery_key = 7"), "unknown key kept: {out}");
		assert!(out.contains(REORDER_EXTRA_HEADER), "extras header present");
		assert!(
			out.find("mystery_key").unwrap() < out.find("[colors]").unwrap(),
			"unknown top key stays a top-level key (before [colors])"
		);
	}

	// The rewrite is a fixed point: reordering its own output changes nothing.
	#[test]
	fn reorder_is_idempotent() {
		let text = "scrollback = 5000\n# opacity = 0.5\nmystery_key = 7\n\n[themes.x]\nk = 1\n[colors]\nfocus = \"#abcdef\"\n";
		let once = reorder_config_text(text).expect("first pass changes");
		assert!(
			reorder_config_text(&once).is_none(),
			"second pass must be a no-op:\n{once}"
		);
	}

	// A key the file lacks comes back as the template's default line (backfill via
	// the template), so reorder alone leaves a current, complete file.
	#[test]
	fn reorder_fills_missing_from_template() {
		let out = reorder_config_text("margin = 8.0\n").expect("should change");
		assert!(
			out.contains("use_system_font = true"),
			"template default added"
		);
		assert!(out.contains("[colors]"), "colors section present");
	}

	// The real on-disk load pipeline (migrate -> backfill -> reorder) on a drifted
	// pre-update config: obsolete keys dropped, renamed keys carried, user values
	// and a custom table preserved, layout normalized, and the whole chain stable.
	#[test]
	fn pipeline_migrate_backfill_reorder_on_disk() {
		let path = std::env::temp_dir().join("silkterm_pipeline_reorder_test.toml");
		let drifted = "scrollback = 5000\n\
			cursor_size_vertical = 40\n\
			cursor_shape = \"block\"\n\
			margin = 12.0\n\
			opacity = 0.8\n\
			\n\
			[themes.mine.dark]\n\
			background = \"#010203\"\n\
			\n\
			[colors]\n\
			focus = \"#abcdef\"\n";
		std::fs::write(&path, drifted).unwrap();
		migrate_config(&path);
		backfill_config(&path);
		reorder_config(&path);
		let out = std::fs::read_to_string(&path).unwrap();

		assert!(
			!out.contains("cursor_shape"),
			"obsolete key dropped:\n{out}"
		);
		assert!(
			out.contains("cursor_size_height = 40"),
			"renamed key kept its value"
		);
		assert!(
			out.contains("margin = 12.0") && out.contains("opacity = 0.8"),
			"values kept"
		);
		assert!(out.contains("scrollback = 5000"), "scrollback value kept");
		assert!(out.contains("focus = \"#abcdef\""), "color override kept");
		assert!(out.contains("[themes.mine.dark]"), "custom table kept");
		assert!(
			out.contains("use_system_font = true"),
			"missing key backfilled"
		);
		// canonical order: font section before window before scrolling before colors
		let (fam, marg, scr, col) = (
			out.find("use_system_font").unwrap(),
			out.find("margin =").unwrap(),
			out.find("scrollback =").unwrap(),
			out.find("[colors]").unwrap(),
		);
		assert!(
			fam < marg && marg < scr && scr < col,
			"normalized order:\n{out}"
		);

		// stable: a second full pipeline pass changes nothing.
		migrate_config(&path);
		backfill_config(&path);
		reorder_config(&path);
		assert_eq!(
			out,
			std::fs::read_to_string(&path).unwrap(),
			"pipeline not idempotent"
		);
		let _ = std::fs::remove_file(&path);
	}
}
