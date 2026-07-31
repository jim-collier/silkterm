// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

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

// Cross-platform monospace fallback stack (first installed wins): the
// font_family default, and the resolver's last resort on every platform when
// neither the configured family nor the OS monospace resolves. Windows always
// goes through it (no OS monospace setting exists there), so every entry must
// carry a real bold face - the bare Family::Monospace db query this replaces
// could land on a family without one, silently ejecting bold runs to an
// arbitrary (often proportional) fallback.
pub const DEFAULT_FONT_STACK: &str = "Monaspace Argon, Fira Code, JetBrains Mono, Cascadia Mono, Consolas, Ubuntu Mono, SF Mono, Menlo, Courier New";

// Stacks that shipped as the font_family default in an earlier version. Backfill
// only ever adds a missing key, so a config written back then still carries its
// stack forever; migration rewrites one to the current default. Matched whole
// and exactly, so anything the user has actually edited is left alone. Append
// the outgoing value here whenever DEFAULT_FONT_STACK changes.
const SUPERSEDED_FONT_STACKS: &[&str] = &[
	"JetBrains Mono, Fira Code, Cascadia Code, DejaVu Sans Mono, Menlo, Consolas, Liberation Mono, monospace",
];

// right-click context menu
pub const MENU_LINK: [u8; 3] = [0x6c, 0x9c, 0xff]; // clickable URL

// Menu bar / dropdown colours: bg + text come from the active theme (overridable
// via colors.menu_background/menu_foreground); hover, border, and the group
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
	pub use_system_font: bool, // true = OS monospace FAMILY, overriding font_family
	pub use_system_font_size: bool, // true = OS monospace SIZE, overriding font_size
	pub font_family: Option<String>, // comma-separated fallback stack (first installed wins)
	pub font_size: f32,
	pub line_height_scale: f32,
	pub scrollback: usize,
	pub scroll_tau_ms: f32,
	pub wheel_lines: f32,
	pub alt_scroll_lines: f32,
	pub output_ease_lines: f32,
	pub smooth_scroll_apps: bool, // ease the line-jumps of full-screen / repaint apps (less/vim/nano; ConPTY TUIs that scroll above a fixed input line)
	pub margin: f32,              // logical px between content and pane edge
	pub opacity: f32,             // background opacity 0..1 (1 = fully opaque)
	pub transparent_background: bool, // X11: per-pixel bg transparency (text stays opaque) via a GL surface
	pub transparent_background_blur: bool, // X11: ask a KWin/picom compositor to blur the desktop behind the window
	pub wallpaper: Option<PathBuf>,        // resolved path, or None
	pub wallpaper_raw: String, // the value as configured ("" = auto-detect); what the dialog shows
	pub wallpaper_default: bool, // when no image/folder is configured, show the built-in wallpaper
	pub wallpaper_folder: Option<PathBuf>, // rotate the wallpaper through this folder's images (overrides wallpaper)
	pub wallpaper_rotate_random: bool,     // rotate randomly instead of in filename order
	pub wallpaper_rotate_interval_s: f32,  // seconds between rotations (0 = pick one at startup only)
	pub wallpaper_opacity: f32,            // image visibility 0..1
	pub wallpaper_fit: Fit,
	pub wallpaper_blur: f32, // Gaussian blur sigma applied to the image (0 = none)
	pub wallpaper_contrast_mask: bool, // flatten the image's contrast so it stops competing with text
	pub wallpaper_contrast_mask_size: f32, // flatten scale 0..1 (1 = half the longest pixel dim)
	pub wallpaper_contrast_mask_strength: f32, // how far toward the local mean 0..1
	pub wallpaper_contrast_mask_auto: f32, // blend manual knobs with image-derived auto 0..1 (1 = full auto)
	pub text_scrim: bool, // bg-colored blurry halo behind glyphs (readability over busy/transparent bg)
	pub text_scrim_radius: f32, // scrim blur sigma in px
	pub text_scrim_softness: f32, // 0 = hard/solid scrim, 1 = soft/faint (maps to the intensity boost)
	pub text_outline: f32, // antialiased outline around glyphs, px (0 = none; scrim colour rules)
	pub text_scrim_ramp: String, // halo falloff curve: "s" | "gaussian" | "linear" | "log" | "exp"
	pub text_scrim_function: String, // halo build: "dilate" | "sdf" | "dt" | "gaussian" (legacy blur)
	pub text_scrim_regular_weight: bool, // blur bold text at regular weight (uniform halo; crisp text keeps its weight)
	pub color_emoji: bool, // paint COLRv1 colour glyphs (emoji) instead of falling back to a monochrome face
	pub embolden_inverse: bool, // render reverse-video (dark-on-light) text bold so it reads as strongly as normal text (the scrim only boosts light-on-dark)
	pub cursor_scrim: bool,     // cursor joins the text scrim halo (default off)
	pub cursor_outline: bool,   // cursor joins the text outline (default on)
	pub cursor_size_height: f32, // cursor height, 1..100% of the cell (from the bottom)
	pub cursor_size_width: f32, // cursor width, 1..100% of the cell (from the left)
	pub cursor_animation: String, // "none" | "phase" | "pulse_vertical" | "pulse_horizontal" | "pulse_both"
	pub cursor_animation_resume_s: f32, // idle seconds after typing before the animation resumes (output does not wait this out)
	pub cursor_animation_idle_stop_s: f32, // idle seconds until the animation stops (parked at full); 0 = never
	pub cursor_blink_rate_ms: f32,         // one animation cycle (ms)
	pub columns: usize,                    // initial window grid size (used when !remember_size)
	pub rows: usize,
	pub remember_size: bool, // launch at the last window size instead of columns/rows
	pub hide_single_tab: bool, // hide the tab bar while only one tab is open
	pub remembered_columns: usize, // last actual window size (not shown in the dialog)
	pub remembered_rows: usize,
	pub word_separators: String, // delimiters for double-click word selection
	pub selection_pairs: String, // matched pairs a double-click selects inside of
	pub default_shell: String,   // command for new tabs/panes (empty = system shell)
	pub command_line: String,    // default CLI layout/options when launched with no args
	pub copy_on_select: bool,    // panes start with copy-on-select enabled
	pub bg: [u8; 3],
	pub fg: [u8; 3],
	pub cursor: [u8; 3],
	pub focus: [u8; 3],
	// chrome colours (menu bar / dropdowns, and pop-out dialogs), from the theme
	// palette; colors.menu_*/colors.dialog_* keys override
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
			use_system_font_size: true,
			font_family: Some(DEFAULT_FONT_STACK.to_string()),
			font_size: FALLBACK_FONT_SIZE,
			line_height_scale: 1.22,
			scrollback: 10_000,
			scroll_tau_ms: 230.0, // ~ "Initial scroll speed" 25 (slow/smooth; ramps up under bursts)
			wheel_lines: 3.0,
			alt_scroll_lines: 3.0,
			output_ease_lines: 1.0,
			smooth_scroll_apps: true,
			margin: 8.0,
			opacity: 0.95,
			transparent_background: false,
			transparent_background_blur: false,
			wallpaper: None,
			wallpaper_raw: String::new(),
			wallpaper_default: true,
			wallpaper_folder: None,
			wallpaper_rotate_random: true,
			wallpaper_rotate_interval_s: 0.0,
			wallpaper_opacity: 0.10, // image visibility relative to bg color
			wallpaper_fit: Fit::Stretch,
			wallpaper_blur: 10.0,
			wallpaper_contrast_mask: true,
			wallpaper_contrast_mask_size: 0.5,
			wallpaper_contrast_mask_strength: 0.5,
			wallpaper_contrast_mask_auto: 0.5,
			text_scrim: true,
			text_scrim_radius: 5.0,
			text_scrim_softness: 0.5,
			text_outline: 2.0,
			text_scrim_ramp: "gaussian".to_string(),
			text_scrim_function: "sdf".to_string(),
			text_scrim_regular_weight: true,
			color_emoji: true,
			embolden_inverse: true,
			cursor_scrim: false,
			cursor_outline: true,
			cursor_size_height: 100.0, // full height
			cursor_size_width: 100.0,  // full width - a block
			cursor_animation: "pulse_vertical".to_string(),
			cursor_animation_resume_s: 1.0,
			cursor_animation_idle_stop_s: 60.0,
			cursor_blink_rate_ms: 500.0,
			columns: 160,
			rows: 48,
			remember_size: true,
			hide_single_tab: false,
			remembered_columns: 160,
			remembered_rows: 48,
			// alacritty's default delimiters minus ':', so a Windows drive path
			// (C:\...) stays whole on a double-click - and namespaced idents
			// (std::vec) and URLs (http://) with it. /.-_~ are already word chars.
			word_separators: alacritty_terminal::term::SEMANTIC_ESCAPE_CHARS
				.chars()
				.filter(|&c| c != ':')
				.collect(),
			selection_pairs: DEFAULT_SELECTION_PAIRS.to_owned(),
			default_shell: String::new(),
			command_line: String::new(),
			copy_on_select: false,
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
// NOTE: re-derives from the theme, so a one-off colours override is dropped on an
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

// Re-read config.shcl from disk (e.g. after the user edited it by hand). Returns
// the freshly parsed settings; the caller applies them. Does not mutate the live
// store - pair with `update` plus whatever rebuild the change needs.
pub fn reload_from_disk() -> Settings {
	load()
}

// Read the config as an editable document. The parser is forgiving (a bad line
// becomes a diagnostic, not a failed load), so unlike the old strict TOML path
// this cannot bail on a file the loader reads fine and silently save nothing.
fn read_doc(path: &std::path::Path) -> Option<shcl::Document> {
	let text = std::fs::read_to_string(path).ok()?;
	Some(shcl::Document::parse(&text))
}

// Serialize a document back to disk text.
//
// `to_canonical` keeps comments and never rewrites a scalar, but it drops blank
// lines between comment-only regions - and a config whose settings are mostly
// commented-out defaults is exactly that, so a raw canonical write would collapse
// the whole file into one dense block. Restore the user's grouping: any line that
// had a blank line above it in `before` gets one again.
fn to_text(doc: &shcl::Document, before: &str) -> String {
	// Match lines positionally, not by text: a repeated line (the '##====' rules
	// between sections) must only regain the blank line the *matching* one had.
	let mut prior: Vec<(String, bool)> = Vec::new();
	let mut blank_above = false;
	for line in before.lines() {
		if line.trim().is_empty() {
			blank_above = true;
		} else {
			prior.push((line_identity(line), blank_above));
			blank_above = false;
		}
	}

	let canonical = doc.to_canonical();
	let mut out = String::new();
	let mut at = 0; // how far through `prior` we've matched
	let mut prev_blank = true; // suppress a leading blank at the top of the file
	for line in canonical.lines() {
		if line.trim().is_empty() {
			if !prev_blank {
				out.push('\n');
			}
			prev_blank = true;
			continue;
		}
		let id = line_identity(line);
		if let Some(offset) = prior[at..].iter().position(|(pid, _)| *pid == id) {
			if prior[at + offset].1 && !prev_blank {
				out.push('\n');
			}
			at += offset + 1;
		}
		out.push_str(line);
		out.push('\n');
		prev_blank = false;
	}
	out
}

// What makes two lines "the same line" across a rewrite. A setting is its key
// (so re-quoting or respacing a value still matches, commented or not);
// anything else is its trimmed text.
fn line_identity(line: &str) -> String {
	line_setting_key(line).map_or_else(|| line.trim().to_string(), |key| format!("\0{key}"))
}

// Write `doc` back to `path`, restoring the blank-line grouping of what was there.
fn write_doc(path: &std::path::Path, doc: &shcl::Document, before: &str) {
	if let Err(e) = std::fs::write(path, to_text(doc, before)) {
		eprintln!("{APP_NAME}: could not save config {}: {e}", path.display());
	}
}

// Write the values that differ from `orig` back into the config in place. The
// user's comments and blank-line grouping survive (see `to_text`); untouched
// settings keep whatever they were (commented / following the system). Returns
// false (writing nothing) if the file looks open in another program, so the
// caller can hold off - e.g. the Settings dialog stays open instead of
// clobbering an in-flight edit.
#[must_use]
pub fn persist(orig: &Settings, s: &Settings) -> bool {
	let Some(path) = config_path() else {
		return true;
	};
	if config_open_elsewhere(&path) {
		note_config_busy(&path);
		return false;
	}
	let before = std::fs::read_to_string(&path).unwrap_or_default();
	let Some(mut doc) = read_doc(&path) else {
		return true;
	};
	// round f32 -> a clean decimal so persisted floats aren't 0.2000000029...
	let r = |v: f32| (v as f64 * 1000.0).round() / 1000.0;

	if s.theme != orig.theme {
		doc.set_string("theme", s.theme.as_str());
	}
	if s.theme_mode != orig.theme_mode {
		doc.set_string("theme_mode", s.theme_mode.as_str());
	}

	if s.use_system_font != orig.use_system_font {
		doc.set_bool("use_system_font", s.use_system_font);
	}
	if s.use_system_font_size != orig.use_system_font_size {
		doc.set_bool("use_system_font_size", s.use_system_font_size);
	}
	if s.font_family != orig.font_family {
		if let Some(f) = &s.font_family {
			doc.set_string("font_family", f);
		}
	}
	if s.font_size != orig.font_size {
		doc.set_float("font_size", r(s.font_size));
	}
	if s.line_height_scale != orig.line_height_scale {
		doc.set_float("line_height_scale", r(s.line_height_scale));
	}
	if s.scrollback != orig.scrollback {
		doc.set_int("scrollback", s.scrollback as i64);
	}
	if s.scroll_tau_ms != orig.scroll_tau_ms {
		doc.set_float("scroll_tau_ms", r(s.scroll_tau_ms));
	}
	if s.wheel_lines != orig.wheel_lines {
		doc.set_float("wheel_lines", r(s.wheel_lines));
	}
	if s.alt_scroll_lines != orig.alt_scroll_lines {
		doc.set_float("alt_scroll_lines", r(s.alt_scroll_lines));
	}
	if s.output_ease_lines != orig.output_ease_lines {
		doc.set_float("output_ease_lines", r(s.output_ease_lines));
	}
	if s.margin != orig.margin {
		doc.set_float("margin", r(s.margin));
	}
	if s.opacity != orig.opacity {
		doc.set_float("opacity", r(s.opacity));
	}
	if s.transparent_background != orig.transparent_background {
		doc.set_bool("transparent_background", s.transparent_background);
	}
	if s.transparent_background_blur != orig.transparent_background_blur {
		doc.set_bool("transparent_background_blur", s.transparent_background_blur);
	}
	if s.wallpaper_opacity != orig.wallpaper_opacity {
		doc.set_float("wallpaper_opacity", r(s.wallpaper_opacity));
	}
	if s.wallpaper_fit != orig.wallpaper_fit {
		doc.set_string(
			"wallpaper_fit",
			match s.wallpaper_fit {
				Fit::Zoom => "zoom",
				Fit::Stretch => "stretch",
			},
		);
	}
	if s.wallpaper_blur != orig.wallpaper_blur {
		doc.set_float("wallpaper_blur", r(s.wallpaper_blur));
	}
	if s.wallpaper_contrast_mask != orig.wallpaper_contrast_mask {
		doc.set_bool("wallpaper_contrast_mask", s.wallpaper_contrast_mask);
	}
	if s.wallpaper_contrast_mask_size != orig.wallpaper_contrast_mask_size {
		doc.set_float(
			"wallpaper_contrast_mask_size",
			r(s.wallpaper_contrast_mask_size),
		);
	}
	if s.wallpaper_contrast_mask_strength != orig.wallpaper_contrast_mask_strength {
		doc.set_float(
			"wallpaper_contrast_mask_strength",
			r(s.wallpaper_contrast_mask_strength),
		);
	}
	if s.wallpaper_contrast_mask_auto != orig.wallpaper_contrast_mask_auto {
		doc.set_float(
			"wallpaper_contrast_mask_auto",
			r(s.wallpaper_contrast_mask_auto),
		);
	}
	if s.text_scrim != orig.text_scrim {
		doc.set_bool("text_scrim", s.text_scrim);
	}
	if s.text_scrim_radius != orig.text_scrim_radius {
		doc.set_float("text_scrim_radius", r(s.text_scrim_radius));
	}
	if s.text_scrim_softness != orig.text_scrim_softness {
		doc.set_float("text_scrim_softness", r(s.text_scrim_softness));
	}
	if s.text_outline != orig.text_outline {
		doc.set_float("text_outline", r(s.text_outline));
	}
	if s.text_scrim_ramp != orig.text_scrim_ramp {
		doc.set_string("text_scrim_ramp", &s.text_scrim_ramp);
	}
	if s.text_scrim_function != orig.text_scrim_function {
		doc.set_string("text_scrim_function", &s.text_scrim_function);
	}
	if s.text_scrim_regular_weight != orig.text_scrim_regular_weight {
		doc.set_bool("text_scrim_regular_weight", s.text_scrim_regular_weight);
	}
	if s.color_emoji != orig.color_emoji {
		doc.set_bool("color_emoji", s.color_emoji);
	}
	if s.embolden_inverse != orig.embolden_inverse {
		doc.set_bool("embolden_inverse", s.embolden_inverse);
	}
	if s.cursor_scrim != orig.cursor_scrim {
		doc.set_bool("cursor_scrim", s.cursor_scrim);
	}
	if s.cursor_outline != orig.cursor_outline {
		doc.set_bool("cursor_outline", s.cursor_outline);
	}
	if s.columns != orig.columns {
		doc.set_int("columns", s.columns as i64);
	}
	if s.rows != orig.rows {
		doc.set_int("rows", s.rows as i64);
	}
	if s.remember_size != orig.remember_size {
		doc.set_bool("remember_size", s.remember_size);
	}
	if s.hide_single_tab != orig.hide_single_tab {
		doc.set_bool("hide_single_tab", s.hide_single_tab);
	}
	if s.remembered_columns != orig.remembered_columns {
		doc.set_int("remembered_columns", s.remembered_columns as i64);
	}
	if s.remembered_rows != orig.remembered_rows {
		doc.set_int("remembered_rows", s.remembered_rows as i64);
	}
	if s.word_separators != orig.word_separators {
		doc.set_string("word_separators", &s.word_separators);
	}
	if s.selection_pairs != orig.selection_pairs {
		doc.set_string("selection_pairs", &s.selection_pairs);
	}
	if s.default_shell != orig.default_shell {
		doc.set_string("default_shell", &s.default_shell);
	}
	if s.command_line != orig.command_line {
		doc.set_string("command_line", &s.command_line);
	}
	if s.copy_on_select != orig.copy_on_select {
		doc.set_bool("copy_on_select", s.copy_on_select);
	}
	if s.wallpaper != orig.wallpaper || s.wallpaper_raw != orig.wallpaper_raw {
		// the file keeps whatever form the user wrote (bare/relative/absolute)
		if s.wallpaper_raw.trim().is_empty() {
			doc.remove("wallpaper");
		} else {
			doc.set_string("wallpaper", s.wallpaper_raw.trim());
		}
	}
	if s.wallpaper_default != orig.wallpaper_default {
		doc.set_bool("wallpaper_default", s.wallpaper_default);
	}

	let mut set_color = |key: &str, color: [u8; 3], orig_color: [u8; 3]| {
		if color != orig_color {
			doc.set_string(&format!("colors.{key}"), &format_hex(color));
		}
	};
	set_color("background", s.bg, orig.bg);
	set_color("foreground", s.fg, orig.fg);
	set_color("cursor", s.cursor, orig.cursor);
	set_color("focus", s.focus, orig.focus);

	write_doc(&path, &doc, &before);
	true
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
	let s = if c <= 0.003_130_8 {
		c * 12.92
	} else {
		1.055 * c.powf(1.0 / 2.4) - 0.055
	};
	(s * 255.0 + 0.5) as u8
}

// config file loading

#[derive(Default)]
struct RawConfig {
	use_system_font: Option<bool>,
	use_system_font_size: Option<bool>,
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
	wallpaper: Option<String>,
	wallpaper_default: Option<bool>,
	wallpaper_folder: Option<String>,
	wallpaper_rotate_random: Option<bool>,
	wallpaper_rotate_interval_s: Option<f32>,
	wallpaper_opacity: Option<f32>,
	wallpaper_fit: Option<String>,
	wallpaper_blur: Option<f32>,
	wallpaper_contrast_mask: Option<bool>,
	wallpaper_contrast_mask_size: Option<f32>,
	wallpaper_contrast_mask_strength: Option<f32>,
	wallpaper_contrast_mask_auto: Option<f32>,
	theme: Option<String>,
	theme_mode: Option<String>,
	text_scrim: Option<bool>,
	text_scrim_radius: Option<f32>,
	text_scrim_softness: Option<f32>,
	text_outline: Option<f32>,
	text_scrim_ramp: Option<String>,
	text_scrim_function: Option<String>,
	text_scrim_regular_weight: Option<bool>,
	color_emoji: Option<bool>,
	embolden_inverse: Option<bool>,
	cursor_scrim: Option<bool>,
	cursor_outline: Option<bool>,
	cursor_size_height: Option<f32>,
	cursor_size_width: Option<f32>,
	cursor_animation: Option<String>,
	cursor_animation_resume_s: Option<f32>,
	cursor_animation_idle_stop_s: Option<f32>,
	cursor_blink_rate_ms: Option<f32>,
	columns: Option<usize>,
	rows: Option<usize>,
	remember_size: Option<bool>,
	hide_single_tab: Option<bool>,
	remembered_columns: Option<usize>,
	remembered_rows: Option<usize>,
	word_separators: Option<String>,
	selection_pairs: Option<String>,
	default_shell: Option<String>,
	command_line: Option<String>,
	copy_on_select: Option<bool>,
	colors: RawColors,
}

#[derive(Default)]
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
	// the user's existing values. These are the only launch-time writes, and each
	// runs only when the program's own option set changed (a rename/removal, or a
	// new option). Both writes defer (with an FYI) if the file looks open in
	// another program.
	migrate_config(&path);
	backfill_config(&path);
	let raw = match std::fs::read_to_string(&path) {
		Ok(text) => read_raw(&text, &path),
		Err(_) => RawConfig::default(),
	};
	resolve(raw)
}

// Typed reads off a parsed document, warning about (and then ignoring) any single
// setting whose value won't coerce. A key that is absent, empty, or unreadable
// comes back None, so `resolve` falls through to that setting's default.
struct Reader<'a> {
	doc: shcl::Document,
	path: &'a std::path::Path,
}

impl Reader<'_> {
	// Complain once about a value that is present but the wrong type. Anything
	// else (absent, empty) is silent - a commented-out setting is the norm here.
	fn note<T>(&self, key: &str, got: Result<T, shcl::Status>) -> Option<T> {
		match got {
			Ok(v) => Some(v),
			Err(shcl::Status::BadType) => {
				eprintln!(
					"{APP_NAME}: {}: ignoring invalid value for `{key}`",
					self.path.display()
				);
				None
			}
			Err(_) => None,
		}
	}
	fn b(&self, key: &str) -> Option<bool> {
		self.note(key, self.doc.get_bool(key))
	}
	fn f(&self, key: &str) -> Option<f32> {
		self.note(key, self.doc.get_float(key)).map(|v| v as f32)
	}
	fn u(&self, key: &str) -> Option<usize> {
		self.note(key, self.doc.get_int(key))
			.map(|v| v.max(0) as usize)
	}
	fn s(&self, key: &str) -> Option<String> {
		self.note(key, self.doc.get_string(key))
	}
}

// Parse the config and pull out every key we know. The parser is forgiving by
// design - a malformed line becomes a diagnostic and is skipped rather than
// sinking the whole document - so this needs no retry loop of its own, and no
// leading-zero rewriting (`.25` is a valid float here).
fn read_raw(text: &str, path: &std::path::Path) -> RawConfig {
	let doc = shcl::Document::parse(text);
	for d in doc.diagnostics() {
		if matches!(d.severity, shcl::Severity::Error) {
			eprintln!(
				"{APP_NAME}: {} line {}: {} [{}]",
				path.display(),
				d.line,
				d.message,
				d.code
			);
		}
	}
	let r = Reader { doc, path };
	RawConfig {
		use_system_font: r.b("use_system_font"),
		use_system_font_size: r.b("use_system_font_size"),
		font_family: r.s("font_family"),
		font_size: r.f("font_size"),
		line_height_scale: r.f("line_height_scale"),
		scrollback: r.u("scrollback"),
		scroll_tau_ms: r.f("scroll_tau_ms"),
		wheel_lines: r.f("wheel_lines"),
		alt_scroll_lines: r.f("alt_scroll_lines"),
		output_ease_lines: r.f("output_ease_lines"),
		smooth_scroll_apps: r.b("smooth_scroll_apps"),
		margin: r.f("margin"),
		opacity: r.f("opacity"),
		transparent_background: r.b("transparent_background"),
		transparent_background_blur: r.b("transparent_background_blur"),
		wallpaper: r.s("wallpaper"),
		wallpaper_default: r.b("wallpaper_default"),
		wallpaper_folder: r.s("wallpaper_folder"),
		wallpaper_rotate_random: r.b("wallpaper_rotate_random"),
		wallpaper_rotate_interval_s: r.f("wallpaper_rotate_interval_s"),
		wallpaper_opacity: r.f("wallpaper_opacity"),
		wallpaper_fit: r.s("wallpaper_fit"),
		wallpaper_blur: r.f("wallpaper_blur"),
		wallpaper_contrast_mask: r.b("wallpaper_contrast_mask"),
		wallpaper_contrast_mask_size: r.f("wallpaper_contrast_mask_size"),
		wallpaper_contrast_mask_strength: r.f("wallpaper_contrast_mask_strength"),
		wallpaper_contrast_mask_auto: r.f("wallpaper_contrast_mask_auto"),
		theme: r.s("theme"),
		theme_mode: r.s("theme_mode"),
		text_scrim: r.b("text_scrim"),
		text_scrim_radius: r.f("text_scrim_radius"),
		text_scrim_softness: r.f("text_scrim_softness"),
		text_outline: r.f("text_outline"),
		text_scrim_ramp: r.s("text_scrim_ramp"),
		text_scrim_function: r.s("text_scrim_function"),
		text_scrim_regular_weight: r.b("text_scrim_regular_weight"),
		color_emoji: r.b("color_emoji"),
		embolden_inverse: r.b("embolden_inverse"),
		cursor_scrim: r.b("cursor_scrim"),
		cursor_outline: r.b("cursor_outline"),
		cursor_size_height: r.f("cursor_size_height"),
		cursor_size_width: r.f("cursor_size_width"),
		cursor_animation: r.s("cursor_animation"),
		cursor_animation_resume_s: r.f("cursor_animation_resume_s"),
		cursor_animation_idle_stop_s: r.f("cursor_animation_idle_stop_s"),
		cursor_blink_rate_ms: r.f("cursor_blink_rate_ms"),
		columns: r.u("columns"),
		rows: r.u("rows"),
		remember_size: r.b("remember_size"),
		hide_single_tab: r.b("hide_single_tab"),
		remembered_columns: r.u("remembered_columns"),
		remembered_rows: r.u("remembered_rows"),
		word_separators: r.s("word_separators"),
		selection_pairs: r.s("selection_pairs"),
		default_shell: r.s("default_shell"),
		command_line: r.s("command_line"),
		copy_on_select: r.b("copy_on_select"),
		colors: RawColors {
			background: r.s("colors.background"),
			foreground: r.s("colors.foreground"),
			cursor: r.s("colors.cursor"),
			focus: r.s("colors.focus"),
			menu_background: r.s("colors.menu_background"),
			menu_foreground: r.s("colors.menu_foreground"),
			dialog_background: r.s("colors.dialog_background"),
			dialog_foreground: r.s("colors.dialog_foreground"),
		},
	}
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
	// Default enabled, but a config that predates the key and set an explicit
	// font_family keeps that font (infer off) instead of being overridden.
	let use_system_font = raw.use_system_font.unwrap_or(raw.font_family.is_none());
	// A pinned wallpaper is a deliberate choice, so it suppresses the auto-detected
	// rotation folder; without one, a stocked wallpapers/ dir rotates by itself.
	let pinned_wallpaper = raw
		.wallpaper
		.as_deref()
		.is_some_and(|value| !value.trim().is_empty());
	let folder = resolve_wallpaper_folder(raw.wallpaper_folder)
		.or_else(|| (!pinned_wallpaper).then(default_wallpaper_folder).flatten());
	Settings {
		use_system_font,
		// absent = follow the face toggle, so configs predating the split (and an
		// explicit font_size, which used to imply off) keep their exact behaviour
		use_system_font_size: raw
			.use_system_font_size
			.unwrap_or(use_system_font && raw.font_size.is_none()),
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
		wallpaper_raw: raw.wallpaper.clone().unwrap_or_default(),
		wallpaper: resolve_wallpaper(raw.wallpaper),
		wallpaper_default: raw.wallpaper_default.unwrap_or(d.wallpaper_default),
		wallpaper_folder: folder,
		wallpaper_rotate_random: raw
			.wallpaper_rotate_random
			.unwrap_or(d.wallpaper_rotate_random),
		wallpaper_rotate_interval_s: raw
			.wallpaper_rotate_interval_s
			.unwrap_or(d.wallpaper_rotate_interval_s)
			.max(0.0),
		wallpaper_opacity: raw
			.wallpaper_opacity
			.unwrap_or(d.wallpaper_opacity)
			.clamp(0.0, 1.0),
		wallpaper_blur: raw
			.wallpaper_blur
			.unwrap_or(d.wallpaper_blur)
			.clamp(0.0, 100.0),
		wallpaper_contrast_mask: raw
			.wallpaper_contrast_mask
			.unwrap_or(d.wallpaper_contrast_mask),
		wallpaper_contrast_mask_size: raw
			.wallpaper_contrast_mask_size
			.unwrap_or(d.wallpaper_contrast_mask_size)
			.clamp(0.0, 1.0),
		wallpaper_contrast_mask_strength: raw
			.wallpaper_contrast_mask_strength
			.unwrap_or(d.wallpaper_contrast_mask_strength)
			.clamp(0.0, 1.0),
		wallpaper_contrast_mask_auto: raw
			.wallpaper_contrast_mask_auto
			.unwrap_or(d.wallpaper_contrast_mask_auto)
			.clamp(0.0, 1.0),
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
			_ => d.text_scrim_ramp.clone(), // missing/unknown -> default (Gaussian)
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
		color_emoji: raw.color_emoji.unwrap_or(d.color_emoji),
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
		cursor_animation_resume_s: raw
			.cursor_animation_resume_s
			.unwrap_or(d.cursor_animation_resume_s)
			.clamp(0.05, 3600.0),
		cursor_animation_idle_stop_s: raw
			.cursor_animation_idle_stop_s
			.unwrap_or(d.cursor_animation_idle_stop_s)
			.clamp(0.0, 86400.0),
		cursor_blink_rate_ms: raw
			.cursor_blink_rate_ms
			.unwrap_or(d.cursor_blink_rate_ms)
			.max(50.0),
		wallpaper_fit: match raw.wallpaper_fit.as_deref() {
			Some("zoom") => Fit::Zoom,
			_ => Fit::Stretch,
		},
		columns: raw.columns.unwrap_or(d.columns).max(1),
		rows: raw.rows.unwrap_or(d.rows).max(1),
		remember_size: raw.remember_size.unwrap_or(d.remember_size),
		hide_single_tab: raw.hide_single_tab.unwrap_or(d.hide_single_tab),
		remembered_columns: raw
			.remembered_columns
			.unwrap_or(d.remembered_columns)
			.max(1),
		remembered_rows: raw.remembered_rows.unwrap_or(d.remembered_rows).max(1),
		word_separators: raw.word_separators.unwrap_or(d.word_separators),
		selection_pairs: raw.selection_pairs.unwrap_or(d.selection_pairs),
		default_shell: raw.default_shell.unwrap_or(d.default_shell),
		command_line: raw.command_line.unwrap_or(d.command_line),
		copy_on_select: raw.copy_on_select.unwrap_or(d.copy_on_select),
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

// Whether "use system font" actually has an OS monospace setting to follow.
// Face and size follow the OS independently (the Settings dual checkboxes), and
// each is inert unless the OS really reports that half: Windows has a system
// font SIZE (the message-box font) but no monospace FAMILY, and a Linux desktop
// with no readable font setting reports neither. Keying on what was detected
// rather than on the platform keeps one rule everywhere - a toggle with nothing
// to follow resolves from font_family / font_size as if off, and greys out.
pub fn system_font_face_active(s: &Settings) -> bool {
	s.use_system_font && crate::sysfont::monospace().family.is_some()
}
pub fn system_font_size_active(s: &Settings) -> bool {
	s.use_system_font_size && crate::sysfont::monospace().size_pt.is_some()
}

// Session-only font zoom (Ctrl+-/+/= hotkeys), in logical px added to the
// effective size. Never persisted; process-wide is per-window since each
// window is its own process. Per-pane scoping is deferred - it needs per-pane
// text metrics the single-TextCtx architecture doesn't have.
static FONT_ZOOM_PX: AtomicI32 = AtomicI32::new(0);
pub fn font_zoom_px() -> i32 {
	FONT_ZOOM_PX.load(Ordering::Relaxed)
}
// Step the zoom, clamped so the effective size stays renderable - stepping
// past the floor must not bank offset the other direction has to pay back.
pub fn nudge_font_zoom(dir: i32) {
	let current = settings();
	let base = if system_font_size_active(&current) {
		default_font_size()
	} else {
		current.font_size
	};
	let z = font_zoom_px() + dir;
	let z = z.clamp((4.0 - base).ceil() as i32, (128.0 - base).floor() as i32);
	FONT_ZOOM_PX.store(z, Ordering::Relaxed);
}
// Drop the session zoom, back to the configured (or system) size.
pub fn reset_font_zoom() {
	FONT_ZOOM_PX.store(0, Ordering::Relaxed);
}

// The size the text is actually rendered at: the OS monospace size while
// `use_system_font_size` is on (and the OS has one), else the configured
// `font_size`; plus any session zoom, clamped to a renderable range.
pub fn effective_font_size() -> f32 {
	let current = settings();
	let base = if system_font_size_active(&current) {
		default_font_size()
	} else {
		current.font_size
	};
	(base + font_zoom_px() as f32).clamp(4.0, 128.0)
}

// Resolve the background image: an explicit path (absolute, or a filename
// relative to the config dir), else auto-detect backgrounds/background.{png,jpg,jpeg}
// under the config dir.
pub fn resolve_wallpaper(explicit: Option<String>) -> Option<PathBuf> {
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
	// New convention first (wallpapers/wallpaper.*), then the old one
	// (backgrounds/background.*) so existing setups keep working.
	[("wallpapers", "wallpaper"), ("backgrounds", "background")]
		.into_iter()
		.flat_map(|(sub, stem)| {
			let sub_dir = dir.join(sub);
			["png", "jpg", "jpeg"]
				.into_iter()
				.map(move |ext| sub_dir.join(format!("{stem}.{ext}")))
		})
		.find(|path| path.exists())
}

// The wallpaper-rotation folder: a relative value resolves against the config
// dir (like the single wallpaper). Returns it only when it's an existing
// directory, so a typo just leaves rotation off rather than erroring.
pub fn resolve_wallpaper_folder(explicit: Option<String>) -> Option<PathBuf> {
	let given = explicit.filter(|value| !value.trim().is_empty())?;
	let path = PathBuf::from(given.trim());
	let path = if path.is_absolute() {
		path
	} else {
		config_path()?.parent()?.join(&path)
	};
	path.is_dir().then_some(path)
}

// Enough kept resets that nobody hits the ceiling in practice, low enough that a
// script looping on --reset-config stops piling up files instead of forever.
const BACKUPS_MAX: u32 = 99;

// Move the config aside so the next load writes a fresh one from the template.
// The old file is kept, not deleted, under the first free `.bak` name - so
// resetting twice never overwrites the copy from the first time. Returns where
// it went, or None if there was nothing to move.
pub fn reset_config() -> Option<PathBuf> {
	let path = config_path()?;
	if !path.exists() {
		return None;
	}
	let name = path.file_name()?.to_string_lossy().into_owned();
	let backup = (1u32..=BACKUPS_MAX)
		.map(|n| match n {
			1 => path.with_file_name(format!("{name}.bak")),
			_ => path.with_file_name(format!("{name}.bak{n}")),
		})
		.find(|candidate| !candidate.exists());
	let Some(backup) = backup else {
		eprintln!(
			"{APP_NAME}: {BACKUPS_MAX} config backups already in {}; clear some out first",
			path.display()
		);
		return None;
	};
	match std::fs::rename(&path, &backup) {
		Ok(()) => Some(backup),
		Err(e) => {
			eprintln!("{APP_NAME}: could not reset {}: {e}", path.display());
			None
		}
	}
}

// Where the wallpaper shuffle keeps its recently-shown list. Beside the config,
// so a --config override gets its own history instead of sharing one.
pub fn wallpaper_history_path() -> Option<PathBuf> {
	Some(config_path()?.parent()?.join(".wallpaper-history"))
}

// Image files we're willing to load as a wallpaper. One list, so the folder
// auto-detect below and the rotation scan can't disagree about what counts.
pub fn is_image_file(path: &std::path::Path) -> bool {
	path.extension()
		.and_then(|ext| ext.to_str())
		.is_some_and(|ext| {
			matches!(
				ext.to_ascii_lowercase().as_str(),
				"png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" | "tiff" | "tif"
			)
		})
}

// The rotation folder to use when none is configured: the conventional
// wallpapers/ dir (or the legacy backgrounds/) under the config dir, but only
// once it actually holds an image - an absent or empty dir just means no
// rotation, silently, since the user never asked for one.
fn default_wallpaper_folder() -> Option<PathBuf> {
	let dir = config_path()?.parent()?.to_path_buf();
	["wallpapers", "backgrounds"]
		.into_iter()
		.map(|sub| dir.join(sub))
		.find(|sub| {
			std::fs::read_dir(sub).is_ok_and(|mut entries| {
				entries.any(|entry| entry.is_ok_and(|e| is_image_file(&e.path())))
			})
		})
}

// A config file's settings as (key, original-line). Nested settings are written
// in dotted form, so a key like "colors.focus" needs no table context.
// Recognizes both active (`k: ...`) and commented (`# k: ...`) lines.
fn setting_lines(text: &str) -> Vec<(String, String)> {
	let mut out = Vec::new();
	for line in text.lines() {
		if let Some(key) = line_setting_key(line) {
			out.push((key.to_string(), line.to_string()));
		}
	}
	out
}

// Like `setting_lines`, but each setting carries the contiguous comment lines
// directly above it (its block), plus `new_group` = whether a blank line
// precedes it in the template. Backfill uses this to keep a template group's
// settings together (no internal blank) while separating groups by a blank line.
fn setting_groups(text: &str) -> Vec<(String, Vec<String>, bool)> {
	let mut pending: Vec<String> = Vec::new();
	let mut group_break = true; // the first setting begins a group
	let mut out = Vec::new();
	for line in text.lines() {
		if let Some(key) = line_setting_key(line) {
			let mut block = std::mem::take(&mut pending);
			block.push(line.to_string());
			out.push((key.to_string(), block, group_break));
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

// The key of a settings line, active or commented-out. Dots are part of the key
// ("colors.foreground"), so a nested setting stays one self-contained line.
fn line_setting_key(line: &str) -> Option<&str> {
	let trimmed = line.trim_start();
	let trimmed = trimmed.strip_prefix('#').map_or(trimmed, str::trim_start);
	let end = trimmed
		.find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.'))?;
	let key = &trimmed[..end];
	if key.is_empty() {
		return None;
	}
	trimmed[end..].trim_start().starts_with(':').then_some(key)
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
	("background_image", "wallpaper"),
	("background_folder", "wallpaper_folder"),
	("background_default", "wallpaper_default"),
	("background_fit", "wallpaper_fit"),
	("background_blur", "wallpaper_blur"),
	("background_opacity", "wallpaper_opacity"),
	("background_rotate_random", "wallpaper_rotate_random"),
	(
		"background_rotate_interval_s",
		"wallpaper_rotate_interval_s",
	),
	("background_contrast_mask", "wallpaper_contrast_mask"),
	(
		"background_contrast_mask_size",
		"wallpaper_contrast_mask_size",
	),
	(
		"background_contrast_mask_strength",
		"wallpaper_contrast_mask_strength",
	),
	(
		"background_contrast_mask_auto",
		"wallpaper_contrast_mask_auto",
	),
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
	// superseded by cursor_animation_resume_s/_idle_stop_s: the animation now
	// always pauses on input ("continuous" is a source-level escape hatch only)
	"cursor_animation_input",
];

// Defaults that changed, as (key, the value that used to be the default). An
// existing config carries the template's commented lines verbatim, so after a
// default changes those lines quietly describe the old behaviour - a line
// reading `# cursor_size_width: 25` next to a cursor that is now a block. A
// commented line matching the outgoing default is refreshed to the current one.
// An ACTIVE line is never touched: that value is the user's own choice, and it
// keeps working exactly as they set it.
const SUPERSEDED_DEFAULTS: &[(&str, &str)] = &[
	("cursor_size_width", "25"),
	("wallpaper_rotate_random", "false"),
	("cursor_animation_resume_s", "2"),
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
		if config_open_elsewhere(path) {
			note_config_busy(path);
			return;
		}
		if let Err(e) = std::fs::write(path, out) {
			eprintln!(
				"{APP_NAME}: could not migrate config {}: {e}",
				path.display()
			);
		}
	}
}

// Best-effort check that some OTHER process has the config file open right now
// (e.g. the user is editing it). Linux only, via /proc/<pid>/fd; elsewhere we
// assume it's free. It only catches editors that hold the descriptor open, so a
// false "not busy" is possible - fine, because the writes we gate on it only add
// program-driven options and never touch the user's own values or comments.
#[cfg(target_os = "linux")]
fn config_open_elsewhere(path: &std::path::Path) -> bool {
	let Ok(target) = path.canonicalize() else {
		return false;
	};
	let me = std::process::id();
	let Ok(procs) = std::fs::read_dir("/proc") else {
		return false;
	};
	for proc in procs.flatten() {
		let Some(pid) = proc
			.file_name()
			.to_str()
			.and_then(|s| s.parse::<u32>().ok())
		else {
			continue;
		};
		if pid == me {
			continue;
		}
		let Ok(fds) = std::fs::read_dir(proc.path().join("fd")) else {
			continue; // not ours to read / gone - skip
		};
		for fd in fds.flatten() {
			if std::fs::read_link(fd.path()).is_ok_and(|link| link == target) {
				return true;
			}
		}
	}
	false
}

#[cfg(not(target_os = "linux"))]
fn config_open_elsewhere(_path: &std::path::Path) -> bool {
	false
}

fn note_config_busy(path: &std::path::Path) {
	eprintln!(
		"{APP_NAME}: {} looks open in another program; leaving it as-is for now.",
		path.display()
	);
}

// Bring a font_family line still carrying a superseded default stack up to the
// current one. Only a bare, exactly-matching quoted value migrates, so an edited
// stack - or one trailing a comment - is left exactly as the user wrote it.
fn refresh_font_stack(line: &str) -> Option<String> {
	let (head, value) = line.split_once(':')?;
	let inner = value.trim().strip_prefix('"')?.strip_suffix('"')?;
	SUPERSEDED_FONT_STACKS
		.contains(&inner)
		.then(|| format!("{head}: \"{DEFAULT_FONT_STACK}\""))
}

// Everything after a settings line's first colon, trimmed - a trailing `##`
// comment included, deliberately, so the exact-match test below refuses any line
// the user has written a note on.
fn line_setting_value(line: &str) -> Option<&str> {
	Some(line.split_once(':')?.1.trim())
}

// Bring a commented line still echoing a superseded default up to the template's
// current line for that key. Only a bare, exactly-matching value migrates, so an
// edited value - or one trailing a note - stays as the user wrote it.
fn refresh_superseded_default(line: &str) -> Option<String> {
	if !line.trim_start().starts_with('#') {
		return None; // active: the user's own value, leave it alone
	}
	let key = line_setting_key(line)?;
	let old = SUPERSEDED_DEFAULTS
		.iter()
		.find_map(|(name, old)| (*name == key).then_some(*old))?;
	if line_setting_value(line)? != old {
		return None;
	}
	setting_lines(DEFAULT_CONFIG)
		.into_iter()
		.find(|(name, _)| name == key)
		.map(|(_, template)| template)
		.filter(|template| template != line)
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
		let mut kept = match line_setting_key(line) {
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
		if let Some(refreshed) = refresh_superseded_default(&kept) {
			kept = refreshed;
			changed = true;
		}
		if line_setting_key(&kept) == Some("font_family") && active(&kept) {
			if let Some(refreshed) = refresh_font_stack(&kept) {
				kept = refreshed;
				changed = true;
			}
			active_font_family = Some(out.len());
		}
		out.push(kept);
	}
	// A config predating `use_system_font` that pinned an explicit font_family keeps
	// that font: insert use_system_font: false so backfill won't add true (the
	// default) and silently override it.
	if let Some(idx) = active_font_family {
		if !has_key("use_system_font") {
			out.insert(idx + 1, "use_system_font: false".to_string());
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
// config.shcl (dotted keys are paths), then backfill so the
// key comes back as the template's commented default line. Used by the Settings
// dialog's revert-to-default buttons.
pub fn revert_keys(keys: &[&str]) {
	if keys.is_empty() {
		return;
	}
	let Some(path) = config_path() else { return };
	if config_open_elsewhere(&path) {
		note_config_busy(&path);
		return;
	}
	let before = std::fs::read_to_string(&path).unwrap_or_default();
	let Some(mut doc) = read_doc(&path) else {
		return;
	};
	// A dotted key ("colors.foreground") is already a path, nested or not.
	for full_key in keys {
		doc.remove(full_key);
	}
	write_doc(&path, &doc, &before);
	backfill_config(&path);
}

// Insert any settings the `DEFAULT_CONFIG` template defines that `path` lacks,
// using the template's own (commented or active) line so follow-system keys stay
// absent and behavior is unchanged. Existing values, comments, and formatting are
// preserved (we only append). Every setting is one self-contained line - nested
// keys are written in dotted form - so there is no table header to insert under.
fn backfill_config(path: &std::path::Path) {
	let Ok(text) = std::fs::read_to_string(path) else {
		return;
	};
	let have: std::collections::HashSet<String> = setting_lines(&text)
		.into_iter()
		.map(|(key, _)| key)
		.collect();

	// Each missing key is inserted as its own group: a blank-line separator, the
	// template's comment lines, then the setting (comment + setting stay together;
	// different groups are blank-line separated).
	let mut add: Vec<String> = Vec::new();
	let mut group_open = false; // have we emitted a setting in the current template group?
	for (key, block, new_group) in setting_groups(DEFAULT_CONFIG) {
		if new_group {
			group_open = false;
		}
		if have.contains(&key) {
			continue;
		}
		// a blank line only when this starts a new (visible) group
		if !group_open {
			add.push(String::new());
		}
		add.extend(block);
		group_open = true;
	}
	if add.is_empty() {
		return;
	}

	let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
	lines.extend(add);
	let mut out = lines.join("\n");
	out.push('\n');
	if out != text {
		if config_open_elsewhere(path) {
			note_config_busy(path);
			return;
		}
		if let Err(e) = std::fs::write(path, out) {
			eprintln!(
				"{APP_NAME}: could not update config {}: {e}",
				path.display()
			);
		}
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
	Some(base.join("silkterm").join("config.shcl"))
}

const DEFAULT_CONFIG: &str = r##"# SilkTerm configuration file.
#
#
# This config file format is:
#
# SHCL: Simple Hierarchical Config Language.
#
#    "Predictable, precise, and forgiving. The parser does the hard work, not you."
#
#    Home     https://github.com/jim-collier/shcl
#    Syntax   https://github.com/jim-collier/shcl/blob/main/project/spec.md
#    License  MIT. Copyright © 2026 Jim Collier.
#
#
## Delete this file to regenerate defaults.
## Convention: '## ' starts an explanatory comment; a single '# ' before a
## `key: value` is a commented-out (disabled) setting you can uncomment.
## This file is yours to edit: your values and comments are kept. Saving may
## tidy layout (indentation, grouping), but never rewrites what you wrote.
## A malformed line is skipped on its own rather than sinking the whole file.
## On launch SilkTerm only adds options new to this version (and renames/removes
## ones that changed) - and even that is skipped if the file looks open elsewhere.

##=============================================================================
## Font
##=============================================================================

## Use the OS default monospace font FAMILY: put it at the head of the
## font_family stack below. Turn off to start at font_family instead. Ignored
## where the OS has no monospace setting to read (Windows has none).
use_system_font: true

## Use the OS default monospace font SIZE, overriding font_size below. Turn off
## to size the font yourself. Ignored where the OS reports no size.
# use_system_font_size: true

## Font family: a comma-separated fallback stack, first installed one wins. The
## same list is consulted on every platform; use_system_font above only decides
## whether the OS font is tried ahead of it. Anything not installed is skipped,
## and a built-in stack backs the whole list up.
font_family: "Monaspace Argon, Fira Code, JetBrains Mono, Cascadia Mono, Consolas, Ubuntu Mono, SF Mono, Menlo, Courier New"

## Font size in logical pixels. Used when use_system_font_size = false.
# font_size: 17.0

## Line height as a multiple of the font's natural height (1.0 = tight).
line_height_scale: 1.22

##=============================================================================
## Window
##=============================================================================

## Pixels between the text and the pane edge.
margin: 8.0

## Initial window size, in character cells (used when remember_size = false).
columns: 160
rows: 48

## Launch at the last window size instead of columns/rows (default on). The
## remembered size is updated automatically whenever you resize the window (kept
## separate from columns/rows so unchecking reverts to your defined size).
# remember_size: true
# remembered_columns: 160
# remembered_rows: 48

## Hide the tab bar while only one tab is open (also in the View menu).
# hide_single_tab: false

##=============================================================================
## Background and transparency
##=============================================================================

## Transparency: when on, the terminal background (only - never the text, window
## frame, or menus) becomes see-through, using `opacity` below as its alpha. The
## code picks the method (per-pixel via a GL surface on X11; native elsewhere).
# transparent_background: true

## Background opacity, 0.0 (fully transparent) to 1.0 (opaque). Only takes effect
## when `transparent_background` is on.
opacity: 0.95

## Ask the compositor to blur the desktop showing through the translucent
## background ("frosted glass"); text stays crisp. Only honored by KWin and
## picom-with-blur; on Compiz/GNOME it does nothing (enable blur in the
## compositor instead). The compositor, not SilkTerm, controls the blur radius.
# transparent_background_blur: true

## Wallpaper image. Leave commented to auto-detect wallpapers/wallpaper.{png,jpg,jpeg}
## (or the legacy backgrounds/background.{png,jpg,jpeg}) under this directory. Value
## may be an absolute path or a filename relative here.
# wallpaper: "wallpaper.png"

## Show a built-in wallpaper when none is configured (no wallpaper found
## above and no wallpaper_folder below). Set false for a plain terminal.
# wallpaper_default: true

## Rotate the wallpaper through a folder of images (overrides wallpaper while
## set). Path is absolute or relative to this directory. Left commented, a
## wallpapers/ dir here with images in it rotates on its own - unless `wallpaper`
## above pins one, or one is given on the command line for that run.
## Random picks avoid whatever came up recently, so runs feel varied rather than
## repeating; set false for plain filename order. Interval 0 = one per launch.
# wallpaper_folder: "wallpapers"
# wallpaper_rotate_random: true
# wallpaper_rotate_interval_s: 0.0

## Image visibility relative to the background color (independent of `opacity`
## above): 0.0 = all background color, 1.0 = all image.
# wallpaper_opacity: 0.10

## How the image fits: "stretch" (fill, ignore aspect) or "zoom" (cover, keep aspect).
# wallpaper_fit: "stretch"

## Gaussian blur applied to the wallpaper (sigma in pixels; 0 = none).
# wallpaper_blur: 10.0

## Contrast mask: flatten the wallpaper's contrast so it stops competing
## with text. `size` is the flatten scale (1.0 = half the longest pixel
## dimension, so the whole image collapses toward one tone; small = only fine
## detail flattens). `strength` is how far each pixel is pulled toward that local
## mean. `auto` blends the two manual knobs with values derived from the image's
## own busyness (1.0 = full auto override, 0.0 = manual only, 0.5 = average).
# wallpaper_contrast_mask: true
# wallpaper_contrast_mask_size: 0.5
# wallpaper_contrast_mask_strength: 0.5
# wallpaper_contrast_mask_auto: 0.5

##=============================================================================
## Text scrim
##=============================================================================

## Text readability scrim: a blurry background-colored halo behind each glyph, so
## text stays legible over a light/busy background or near-transparent terminal.
## On by default; uncomment and set text_scrim = false to disable.
# text_scrim: true
# text_scrim_radius: 5.0     ## scrim halo radius in pixels
# text_scrim_softness: 0.5   ## 0 = hard/solid scrim, 1 = soft/faint
# text_outline: 2.0          ## antialiased outline around glyphs, in pixels (0 = none)
# text_scrim_function: "sdf" ## halo shape: "sdf" (round, full corners), "dt", "dilate" (square), or "gaussian" (legacy, corners recede)
# text_scrim_ramp: "gaussian" ## halo falloff curve: "exp", "gaussian", "log", "s", or "linear"
# text_scrim_regular_weight: true  ## blur bold text at regular weight so its halo matches non-bold text
# color_emoji: true          ## paint colour emoji (COLRv1); false renders them as monochrome outlines
# embolden_inverse: true     ## render reverse-video (dark-on-light) text bold so it reads as strongly as normal
# cursor_scrim: false        ## the cursor joins the scrim halo (default off)
# cursor_outline: true       ## the cursor joins the text outline (default on)

##=============================================================================
## Cursor
##=============================================================================

## Cursor size, as a percent of the cell: height grows from the bottom, width from
## the left. Together they make any shape: a block (100 / 100), a thin bar
## (100 / 25), or an underline (15 / 100). Used when the app doesn't set its
## own; alt-screen apps (vim, less) still control theirs.
# cursor_size_height: 100
# cursor_size_width: 100

## Cursor animation: "none" (steady), "phase" (smooth fade), or a pulse that
## grows/shrinks each cycle - "pulse_vertical", "pulse_horizontal", "pulse_both".
## The cursor always slides smoothly as you type.
# cursor_animation: "pulse_vertical"

## While you type, the animation glides to the cursor's full size and holds
## there; it resumes this many seconds after input goes idle. Pausing and
## resuming always happen at full size, so the cursor never jumps. This delay is
## for typing only - a command's output holds the cursor still while it writes,
## then hands it straight back when the prompt returns. Refocusing the window,
## tab, or pane also resumes at once.
# cursor_animation_resume_s: 1

## After this many seconds with no input the animation stops entirely, parked at
## full size, so an idle window costs nothing. Typing - or refocusing the
## window, tab, or pane - brings it back. 0 = never stop.
# cursor_animation_idle_stop_s: 60

## Cursor animation cycle length, in milliseconds (blink rate).
# cursor_blink_rate_ms: 500

##=============================================================================
## Selection
##=============================================================================

## Delimiters that bound a double-click word selection. The default keeps
## : / . - _ ~ as part of a word, so paths (incl. C:\ drive paths), URLs and
## namespaced identifiers stay selected whole. Leave commented for the default;
## set to your own string of separator characters to override (add ':' back to
## split on it).
# word_separators: ",|\"' ()[]{}<>"

## Pairs whose contents a double-click selects when the click is inside a matched
## pair (highest precedence first). Leave commented for the default.
# selection_pairs: "`` \"\" '' {} () [] <>"

##=============================================================================
## Shell
##=============================================================================

## Default shell/command for new windows, tabs, and panes when nothing else is
## given (CLI --shell and per-pane inheritance take precedence). argv-split, so
## "bash --norc" works. Leave blank/commented to use the system default shell.
# default_shell: "bash --norc"

## Default command line applied when SilkTerm is launched with no arguments - the
## same window/tab/pane options the CLI accepts (see --help). Any actual
## command-line arguments override this entirely. Leave blank/commented for none.
# command_line: "--new-pane --right --size 35%"

## Start every pane with "Copy on select" enabled (selected text goes to the
## clipboard). The menu-bar checkbox still toggles it live per pane.
# copy_on_select: false

##=============================================================================
## Scrolling
##=============================================================================

## Lines of scrollback history kept per pane.
scrollback: 10000

## Smooth-scroll feel. This is the *initial* (slow, smooth) easing for sporadic
## output, shown in Settings as "Initial scroll speed"; lower tau = snappier. Under
## a fast output burst the scroll automatically ramps faster to keep up, then eases
## back to this speed once output stops.
scroll_tau_ms: 230.0  ## ms; ~ "Initial scroll speed" 25 on the 1..100 dialog scale
wheel_lines: 3.0  ## lines per wheel notch (smooth scrollback)
alt_scroll_lines: 3.0  ## lines per wheel notch in full-screen apps (less, nano)
output_ease_lines: 1.0  ## how far new output slides in before easing to rest

## Ease the whole-line jumps of apps that repaint a scrolling region instead of
## growing scrollback: full-screen apps that own the screen (less, vim, nano, htop,
## tmux, ...) and, on Windows, ConPTY-driven TUIs whose output scrolls above a fixed
## input line. Their scrolling slides instead of snapping; the revealed strip fills
## with the background during the ~quarter-second slide.
## Only clean line-scrolls are eased (big page-jumps still snap).
# smooth_scroll_apps: true

##=============================================================================
## Theme and colours
##=============================================================================

## Colour theme. Pick a built-in (SilkTerm, Matrix, Retro Amber) or one you add in
## a themes.* entry. theme_mode is "dark", "light", or "system" (follow the OS).
theme: SilkTerm
theme_mode: dark

## Per-colour overrides on top of the theme (uncomment any to tweak one colour).
## The menu_*/dialog_* keys recolour the chrome (menu bar + dropdowns, and the
## pop-out Settings/About dialogs); by default every theme shares the same neutral
## chrome. Menu hover/border shades derive from menu_background automatically.
## Dotted form ('colors.foreground') and an indented 'colors:' block mean the
## same thing; uncomment a line here and SilkTerm tidies it into a block on save.
# colors.background: "#000000"
# colors.foreground: "#d2d2da"
# colors.cursor: "#7a9ad0"
# colors.focus: "#5580c8"
# colors.menu_background: "#36363b"
# colors.menu_foreground: "#f0f0f2"
# colors.dialog_background: "#20202a"
# colors.dialog_foreground: "#e2e2ea"
"##;

#[cfg(test)]
mod tests {
	use super::*;

	// ':' must NOT be a word separator, else a double-click on C:\... drops the
	// drive prefix (the alacritty default splits on ':'). Regression guard.
	#[test]
	fn default_word_separators_keep_drive_colon() {
		let d = Settings::default();
		assert!(
			!d.word_separators.contains(':'),
			"':' should stay a word char so drive paths select whole"
		);
		// still a real separator set (space + comma remain delimiters)
		assert!(d.word_separators.contains(' '));
		assert!(d.word_separators.contains(','));
	}

	// A bare-decimal float (`.1`, missing leading zero) must not stop persist from
	// saving. Regressed once under TOML, where persist strict-parsed the raw file,
	// bailed on `.1`, and silently dropped every dialog change (relaunch reverted).
	// `.1` is simply a valid float now, and the writer never rewrites a scalar, so
	// the value is also left exactly as the user typed it.
	#[test]
	fn persist_survives_bare_decimal_float() {
		// Memoize settings() BEFORE installing the override: a test on another
		// thread initializing settings() after the override would load() - an
		// in-place migrate/backfill REWRITE of our temp file - racing our own
		// read below (parallel-suite flake: truncated read -> defaults).
		let _ = settings();
		let dir = std::env::temp_dir().join(format!("silkterm_cfgsave_{}", std::process::id()));
		let _ = std::fs::create_dir_all(&dir);
		let path = dir.join("config.shcl");
		std::fs::write(&path, "wallpaper_opacity: .1\ntext_scrim_ramp: \"s\"\n").unwrap();
		set_config_override(path.clone());

		let orig = load();
		assert_eq!(orig.text_scrim_ramp, "s");
		let mut edited = orig.clone();
		edited.text_scrim_ramp = "log".to_string();
		assert!(
			persist(&orig, &edited),
			"persist should write to our temp file"
		);

		assert_eq!(
			load().text_scrim_ramp,
			"log",
			"dialog change lost after relaunch"
		);
		// and the value is still spelled the way the user wrote it
		let saved = std::fs::read_to_string(&path).unwrap();
		assert!(
			saved.contains("wallpaper_opacity: .1"),
			"scalar should be left verbatim: {saved:?}"
		);
		assert_eq!(load().wallpaper_opacity, 0.1);
	}

	// The /proc-based busy check: a child process holding the file open is seen as
	// busy; once it exits the file reads as free again. Linux only (the check is a
	// no-op elsewhere).
	#[cfg(target_os = "linux")]
	#[test]
	fn config_open_elsewhere_sees_a_holder() {
		let path = std::env::temp_dir().join(format!("silkterm_busy_{}.shcl", std::process::id()));
		std::fs::write(&path, "margin: 8.0\n").unwrap();
		assert!(!config_open_elsewhere(&path), "nobody holds it yet");

		// A child with the file as its stdin holds the descriptor open until it exits.
		let hold = std::fs::File::open(&path).unwrap();
		let mut child = std::process::Command::new("sleep")
			.arg("30")
			.stdin(std::process::Stdio::from(hold))
			.spawn()
			.unwrap();

		// give the child a moment to exist in /proc, then confirm we see it
		let mut seen = false;
		for _ in 0..50 {
			if config_open_elsewhere(&path) {
				seen = true;
				break;
			}
			std::thread::sleep(std::time::Duration::from_millis(20));
		}
		let _ = child.kill();
		let _ = child.wait();
		let _ = std::fs::remove_file(&path);
		assert!(seen, "a process holding the file open should read as busy");
	}

	// One unusable line must cost only its own setting, never the whole file. This
	// used to need a hand-rolled retry loop (blank the offending line, reparse);
	// the parser is forgiving now, so the guarantee has to be re-proven here.
	#[test]
	fn a_bad_line_drops_only_its_own_setting() {
		let p = std::path::Path::new("test.shcl");
		let s = resolve(read_raw(
			"scrollback: 4242\nmargin: not-a-number\ncolors.focus: \"#abcdef\"\n",
			p,
		));
		assert_eq!(s.scrollback, 4242, "settings before the bad line survive");
		assert_eq!(s.focus, [0xab, 0xcd, 0xef], "and settings after it");
		assert_eq!(
			s.margin,
			Settings::default().margin,
			"the unusable one falls back to its default"
		);
	}

	#[test]
	fn default_config_is_valid_shcl() {
		let doc = shcl::Document::parse(DEFAULT_CONFIG);
		let errors: Vec<_> = doc
			.diagnostics()
			.iter()
			.filter(|d| matches!(d.severity, shcl::Severity::Error))
			.collect();
		assert!(errors.is_empty(), "DEFAULT_CONFIG has errors: {errors:?}");
	}

	// The shipped template must already be what a save would produce, or the very
	// first save would reflow the file we just wrote. Canonical output drops blank
	// lines between comment-only regions (nearly every line here), so this is the
	// guard on `to_text` putting that grouping back.
	#[test]
	fn default_config_survives_a_save_unchanged() {
		let doc = shcl::Document::parse(DEFAULT_CONFIG);
		assert_eq!(
			to_text(&doc, DEFAULT_CONFIG),
			DEFAULT_CONFIG,
			"a save would rewrite the shipped template"
		);
	}

	// #136 convention: explanatory comments use '## '; commented-out (disabled)
	// settings use a single '# '.
	#[test]
	fn default_config_comment_style() {
		// The file header (the format blurb, down to the first '##' line) is
		// verbatim prose in single-'#' form and is exempt from the convention.
		let body = DEFAULT_CONFIG
			.find("\n##")
			.map_or(DEFAULT_CONFIG, |i| &DEFAULT_CONFIG[i..]);
		for line in body.lines() {
			let t = line.trim_start();
			if !t.starts_with('#') {
				continue; // active setting / blank
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
		assert_eq!(d.text_scrim_ramp, "gaussian");
		assert_eq!(d.text_scrim_function, "sdf");
		assert!(d.text_scrim_regular_weight);
		assert!(!d.cursor_scrim, "cursor scrim halo defaults off");
		assert!(d.cursor_outline, "cursor outline defaults on");
		assert_eq!(d.wallpaper_blur, 10.0);
		assert_eq!(d.wallpaper_opacity, 0.10);
		// a block cursor: full height AND full width
		assert_eq!(d.cursor_size_height, 100.0);
		assert_eq!(d.cursor_size_width, 100.0);
		// rotation, when a folder turns up, varies instead of pinning image one
		assert!(d.wallpaper_rotate_random, "rotation defaults to shuffled");
		assert_eq!(d.cursor_animation_resume_s, 1.0);
	}

	// Scrim function + the five falloff curves resolve; unknown values fall to the
	// defaults (sdf / s-curve).
	#[test]
	fn scrim_function_and_ramp_resolve() {
		let p = std::path::Path::new("test.toml");
		for f in ["dilate", "sdf", "dt", "gaussian"] {
			let s = resolve(read_raw(&format!("text_scrim_function: \"{f}\"\n"), p));
			assert_eq!(s.text_scrim_function, f);
		}
		for r in ["s", "gaussian", "linear", "log", "exp"] {
			let s = resolve(read_raw(&format!("text_scrim_ramp: \"{r}\"\n"), p));
			assert_eq!(s.text_scrim_ramp, r);
		}
		let s = resolve(read_raw("text_scrim_function: \"bogus\"\n", p));
		assert_eq!(s.text_scrim_function, "sdf", "unknown -> default");
		let s = resolve(read_raw("text_scrim_ramp: \"bogus\"\n", p));
		assert_eq!(s.text_scrim_ramp, "gaussian", "unknown -> default");
	}

	// The face/size split's inference for configs predating use_system_font_size:
	// absent = follow the face toggle, except an explicit font_size (which the old
	// single toggle silently ignored) reads as intent and turns the size follow off.
	#[test]
	fn system_font_size_split_inference() {
		let p = std::path::Path::new("test.toml");
		let s = resolve(read_raw("", p));
		assert!(s.use_system_font && s.use_system_font_size, "defaults on");
		let s = resolve(read_raw("use_system_font: false\n", p));
		assert!(!s.use_system_font_size, "size follows the face toggle");
		let s = resolve(read_raw("font_size: 20.0\n", p));
		assert!(s.use_system_font, "explicit size keeps the system face");
		assert!(
			!s.use_system_font_size,
			"explicit size wins over the OS size"
		);
		let s = resolve(read_raw("font_size: 20.0\nuse_system_font_size: true\n", p));
		assert!(s.use_system_font_size, "explicit key beats the inference");
	}

	#[test]
	fn copy_on_select_key_parses_and_defaults_off() {
		let p = std::path::Path::new("test.toml");
		assert!(!resolve(read_raw("", p)).copy_on_select, "default off");
		assert!(resolve(read_raw("copy_on_select: true\n", p)).copy_on_select);
	}

	// An over-range output_ease_lines must clamp: scroll's backlog clamp uses it
	// as a lower bound and panics (aborts, in release) when it exceeds the cap.
	#[test]
	fn output_ease_lines_clamps_to_backlog_cap() {
		let raw = read_raw(
			"output_ease_lines: 20.0\n",
			std::path::Path::new("test.toml"),
		);
		let s = resolve(raw);
		assert!(s.output_ease_lines <= crate::scroll::MAX_BACKLOG);
		let raw = read_raw(
			"output_ease_lines: -3.0\n",
			std::path::Path::new("test.toml"),
		);
		assert!(resolve(raw).output_ease_lines >= 0.0);
	}

	// One syntax-broken line must not sink the valid settings around it.
	#[test]
	fn parse_lenient_drops_only_the_bad_line() {
		let text = "opacity: 0.7\ncursor_blink: enable\nmargin: 12.0\n";
		let raw = read_raw(text, std::path::Path::new("test.toml"));
		assert_eq!(raw.opacity, Some(0.7)); // before the bad line
		assert_eq!(raw.margin, Some(12.0)); // after the bad line
	}

	#[test]
	fn chrome_colors_default_and_override() {
		// theme provides the chrome; the default matches the shared menu colours
		let d = Settings::default();
		assert_eq!(d.menu_bg, crate::theme::MENU_BG_DEF);
		assert_eq!(d.menu_fg, crate::theme::MENU_FG_DEF);
		// a colours override wins; unspecified chrome stays at the theme default
		let raw = read_raw(
			"colors.menu_background: \"#123456\"\ncolors.dialog_foreground: \"#abcdef\"\n",
			std::path::Path::new("test.shcl"),
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
			migrate_config_text("text_glow_border: 2.03\nmargin: 8.0\n").expect("should rename");
		assert!(!out.contains("text_glow_border"), "old name gone: {out:?}");
		assert!(
			out.contains("text_outline: 2.03"),
			"value preserved: {out:?}"
		);
	}

	// The text-glow -> text-scrim rename preserves values and active/commented state.
	#[test]
	fn migrate_renames_glow_to_scrim() {
		let out = migrate_config_text(
			"text_glow: false\ntext_glow_radius: 7.0\n# cursor_glow: false\ntext_glow_ramp: \"linear\"\n",
		)
		.expect("should rename");
		assert!(!out.contains("text_glow"), "old names gone: {out:?}");
		assert!(
			out.contains("text_scrim: false"),
			"value + active kept: {out:?}"
		);
		assert!(
			out.contains("text_scrim_radius: 7.0"),
			"value kept: {out:?}"
		);
		assert!(
			out.contains("# cursor_scrim: false"),
			"commented state kept: {out:?}"
		);
		assert!(
			out.contains("text_scrim_ramp: \"linear\""),
			"string value kept: {out:?}"
		);
	}

	#[test]
	fn migrate_renames_background_to_wallpaper() {
		let out = migrate_config_text(
			"background_image: \"pic.jpg\"\nbackground_opacity: 0.4\n# background_blur: 6.0\nbackground_contrast_mask_size: 0.3\n",
		)
		.expect("should rename");
		assert!(!out.contains("background_"), "old names gone: {out:?}");
		assert!(
			out.contains("wallpaper: \"pic.jpg\""),
			"path value + active kept: {out:?}"
		);
		assert!(
			out.contains("wallpaper_opacity: 0.4"),
			"value kept: {out:?}"
		);
		assert!(
			out.contains("# wallpaper_blur: 6.0"),
			"commented state kept: {out:?}"
		);
		assert!(
			out.contains("wallpaper_contrast_mask_size: 0.3"),
			"longest-name key kept: {out:?}"
		);
	}

	// A commented line still carrying a superseded default is a stale echo of an
	// older template, so it gets refreshed. An ACTIVE line with the same value is
	// the user's own choice and must survive untouched - that distinction is the
	// whole point, since refreshing it would silently change their terminal.
	#[test]
	fn stale_commented_defaults_refresh_active_ones_do_not() {
		let out =
			migrate_config_text("# cursor_size_width: 25\n# wallpaper_rotate_random: false\n")
				.expect("stale commented defaults should refresh");
		assert!(
			out.contains("# cursor_size_width: 100"),
			"cursor default refreshed: {out:?}"
		);
		assert!(
			out.contains("# wallpaper_rotate_random: true"),
			"rotation default refreshed: {out:?}"
		);

		// active = deliberate; and an edited commented value is a note, not an echo
		let text = "cursor_size_width: 25\n# wallpaper_rotate_random: false ## mine\n";
		assert!(
			migrate_config_text(text).is_none_or(|out| out.contains("cursor_size_width: 25")
				&& out.contains("# wallpaper_rotate_random: false")),
			"an active or edited value must be left alone"
		);
	}

	// In-place migration: drop obsolete cursor keys, keep the rest.
	#[test]
	fn migrate_config_renames_and_removes() {
		let text = "opacity: 0.7\ncursor_shape: \"block\"\ncursor_insert_shape: \"bar\"\ncursor_blink_style: \"phase\"\nmargin: 12.0\n";
		let out = migrate_config_text(text).expect("should change");
		assert!(!out.contains("cursor_shape"), "obsolete removed: {out:?}");
		assert!(!out.contains("cursor_insert_shape"), "obsolete removed");
		assert!(!out.contains("cursor_blink_style"), "obsolete removed");
		assert!(
			out.contains("opacity: 0.7") && out.contains("margin: 12.0"),
			"kept the rest"
		);
	}

	// The vertical/horizontal -> height/width rename preserves the value.
	#[test]
	fn migrate_config_renames_cursor_size() {
		let out = migrate_config_text("cursor_size_vertical: 50\ncursor_size_horizontal: 25\n")
			.expect("should change");
		assert!(out.contains("cursor_size_height: 50"), "{out:?}");
		assert!(out.contains("cursor_size_width: 25"));
		assert!(!out.contains("cursor_size_vertical") && !out.contains("cursor_size_horizontal"));
	}

	// A config with nothing to migrate is left untouched (no needless rewrite).
	#[test]
	fn migrate_config_noop_when_current() {
		assert!(migrate_config_text("opacity: 0.7\ncursor_animation: \"phase\"\n").is_none());
	}

	#[test]
	fn migrate_drops_cursor_animation_input() {
		// removed key goes whether active or the stale commented template line
		let out = migrate_config_text(
			"# cursor_animation_input: \"continuous\"\ncursor_animation_input: \"pause\"\nmargin: 12.0\n",
		)
		.expect("should change");
		assert!(!out.contains("cursor_animation_input"));
		assert!(out.contains("margin: 12.0"));
	}

	// A pre-boolean config with an explicit font_family keeps it (use_system_font=false
	// inserted), so the backfilled default (true) can't silently override the font.
	#[test]
	fn migrate_config_pins_use_system_font_for_explicit_family() {
		let out = migrate_config_text("font_family: \"Iosevka\"\n").expect("should change");
		assert!(out.contains("font_family: \"Iosevka\""));
		assert!(out.contains("use_system_font: false"), "{out:?}");
		// but a commented family (following the system) doesn't trigger the insert
		assert!(migrate_config_text("# font_family: \"Iosevka\"\n").is_none());
		// and one that already has the key is left alone
		assert!(migrate_config_text("use_system_font: true\nfont_family: \"Iosevka\"\n").is_none());
	}

	// Backfill only ever adds a missing key, so a config written when an older
	// stack was the default kept that stack forever. Migration refreshes exactly
	// the shipped defaults and nothing the user chose themselves.
	#[test]
	fn migrate_refreshes_a_superseded_default_font_stack() {
		let stale = SUPERSEDED_FONT_STACKS[0];
		let out = migrate_config_text(&format!(
			"use_system_font: true\nfont_family: \"{stale}\"\n"
		))
		.expect("stale default should be refreshed");
		assert!(
			out.contains(&format!("font_family: \"{DEFAULT_FONT_STACK}\"")),
			"{out:?}"
		);
		assert!(!out.contains(stale));

		// the current value is already right, so nothing to do
		let current = format!("use_system_font: true\nfont_family: \"{DEFAULT_FONT_STACK}\"\n");
		assert!(migrate_config_text(&current).is_none());
		// a stack the user edited, or one they commented out, is theirs - leave it
		let edited = format!("use_system_font: true\nfont_family: \"Iosevka, {stale}\"\n");
		assert!(migrate_config_text(&edited).is_none());
		assert!(migrate_config_text(&format!("# font_family: \"{stale}\"\n")).is_none());
	}

	// The real on-disk load pipeline (migrate -> backfill) on a drifted pre-update
	// config: obsolete keys dropped, renamed keys carried, user values, comments,
	// and a custom table preserved, missing keys added, and the chain stable. The
	// user's own layout/comments are NOT normalized away (that was the old reorder
	// pass; removed so a hand-edited file isn't rewritten behind the user's back).
	#[test]
	fn pipeline_migrate_backfill_on_disk() {
		let path = std::env::temp_dir().join("silkterm_pipeline_migbf_test.shcl");
		let drifted = "## my own note\n\
			scrollback: 5000\n\
			cursor_size_vertical: 40\n\
			cursor_shape: \"block\"\n\
			margin: 12.0\n\
			opacity: 0.8\n\
			\n\
			themes.mine.dark.background: \"#010203\"\n\
			\n\
			colors.focus: \"#abcdef\"\n";
		std::fs::write(&path, drifted).unwrap();
		migrate_config(&path);
		backfill_config(&path);
		let out = std::fs::read_to_string(&path).unwrap();

		assert!(
			!out.contains("cursor_shape"),
			"obsolete key dropped:\n{out}"
		);
		assert!(
			out.contains("cursor_size_height: 40"),
			"renamed key kept its value"
		);
		assert!(
			out.contains("margin: 12.0") && out.contains("opacity: 0.8"),
			"values kept"
		);
		assert!(out.contains("scrollback: 5000"), "scrollback value kept");
		assert!(
			out.contains("colors.focus: \"#abcdef\""),
			"color override kept"
		);
		assert!(
			out.contains("themes.mine.dark.background"),
			"unknown key kept"
		);
		assert!(out.contains("## my own note"), "user comment kept");
		assert!(
			out.contains("use_system_font: true"),
			"missing key backfilled"
		);
		// the user's leading comment + first key stay put (no reorder)
		assert!(
			out.find("## my own note").unwrap() < out.find("scrollback: 5000").unwrap(),
			"user layout preserved:\n{out}"
		);

		// stable: a second pass changes nothing.
		migrate_config(&path);
		backfill_config(&path);
		assert_eq!(
			out,
			std::fs::read_to_string(&path).unwrap(),
			"pipeline not idempotent"
		);
		let _ = std::fs::remove_file(&path);
	}
}
