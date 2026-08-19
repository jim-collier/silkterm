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

// The one address worth handing straight to someone who has already decided.
// DONATE.md carries the rest; --donate prints both.
pub const SPONSOR_URL: &str = "https://github.com/sponsors/jim-collier";

// Which of the cross builds this binary is - otherwise indistinguishable at a
// glance. Shared by the About dialog and `--about` so the two can't drift.
pub fn build_target() -> String {
	let profile = if cfg!(debug_assertions) {
		"debug"
	} else {
		"release"
	};
	format!(
		"{} / {} ({profile})",
		std::env::consts::ARCH,
		std::env::consts::OS
	)
}

// The display scale factor to lay out at, given what the window reports.
// SILK_SCALE overrides it, which is the only way to see a high-DPI layout on a
// 1x display: chrome written in raw pixels is INVISIBLE at 1x and only thins out
// as the factor rises, so the defect it guards against cannot be looked at
// without one. Read once (var_os takes the env lock and scans environ), same
// pattern as SILK_MAX_FPS. Off X11 there is no winit knob for this at all.
pub fn display_scale(reported: f64) -> f32 {
	use std::sync::OnceLock;
	static OVERRIDE: OnceLock<Option<f32>> = OnceLock::new();
	let over = OVERRIDE.get_or_init(|| {
		std::env::var("SILK_SCALE")
			.ok()
			.and_then(|raw| raw.trim().parse::<f32>().ok())
			.filter(|s| *s > 0.0 && s.is_finite())
	});
	over.unwrap_or(reported as f32)
}

// Chrome measurements are written in DIP (a CSS pixel, 1/96 inch) and converted
// to physical pixels where they are used. The main window's chrome shares a
// coordinate space with the terminal grid, so there is no single boundary to
// divide at the way settings_ui.rs has - each measurement scales at its own use
// site, through here or `TextCtx::dip`.
//
// Rounded to whole pixels so a rule, a ring or a hairline gap stays crisp, and a
// measurement the author asked to be visible never rounds away to nothing (a 1
// DIP gap under a scale factor below 1 would otherwise vanish).
pub fn dip(v: f32, scale: f32) -> f32 {
	let px = v * scale;
	if v > 0.0 {
		px.round().max(1.0)
	} else {
		px.round()
	}
}

// internal, not user-tunable (yet); DIP, see `dip`
pub const PANE_GAP_PX: f32 = 1.0;
pub const DIVIDER_GRAB_PX: f32 = 5.0; // mouse tolerance for grabbing a pane divider
pub const FOCUS_RING_PX: f32 = 2.0;
pub const SETTLE_EPS: f32 = 0.002; // a settle threshold, not a measurement - never scaled

pub const DIVIDER: [u8; 3] = [0x2c, 0x2c, 0x36];

// text-selection highlight
pub const SELECTION_BG: [u8; 3] = [0x33, 0x44, 0x66];

// drag-and-drop pane reorder: drop-target tint
pub const DROP_TARGET: [u8; 3] = [0x55, 0x80, 0xc8];

// Scrollbar. Neutral mid-gray in every theme rather than a palette color: desktop
// scrollbars read as chrome, not as part of the terminal's own color scheme. There
// is no portable way to ask the OS for its actual value (GTK only names a theme),
// so this is the shade those themes converge on. colors.scrollbar_* overrides.
pub const SCROLLBAR_THUMB_DEF: [u8; 3] = [0x8a, 0x8a, 0x92];
pub const SCROLLBAR_TROUGH_DEF: [u8; 3] = [0x2e, 0x2e, 0x36];
// Opacity the bar settles at, and what it rises to while hovered or dragged.
pub const SCROLLBAR_IDLE_A: f32 = 0.55;
pub const SCROLLBAR_ACTIVE_A: f32 = 0.95;
// The trough is a faint backing strip, well under the thumb.
pub const SCROLLBAR_TROUGH_A: f32 = 0.34;

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

// Menu bar / dropdown colors: bg + text come from the active theme (overridable
// via colors.menu_background/menu_foreground); hover, border, and the group
// separator are derived shades of the bg, so a custom menu color stays coherent
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
// Nudge a color toward more contrast: lighten a dark base, darken a light one.
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
// Dropdown/context-menu geometry, DIP (see `dip`). The pop-out dialogs lay out
// in DIP throughout and use these raw; the main window's menus convert at each
// use site.
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
	pub scroll_smooth: bool, // master switch: false = every scroll (wheel, output, app slide) lands instantly
	// The five knobs below are the named segments of the output-scroll speed
	// curve, in the order one burst traverses them: leave rest, accelerate,
	// top out, wind down, land. Each hands its end point to the next and has
	// no other influence on it (scroll.rs holds the model).
	pub scroll_ease_in_ms: f32, // how long the lift from rest to the ramp handoff takes
	pub scroll_ramp_up_ms: f32, // catch-up speed doubles this often while output stays ahead
	pub scroll_single_screen_tau_ms: f32, // burst speed ceiling while the burst is still wholly on screen
	pub scroll_ramp_down_ms: f32, // catch-up speed halves this often winding down to the landing
	pub scroll_ease_out_ms: f32,  // how long the last STOP_BAND of a line takes to land
	pub wheel_lines: f32,
	pub alt_scroll_lines: f32,
	pub output_ease_lines: f32,
	pub smooth_scroll_apps: bool, // ease the line-jumps of full-screen / repaint apps (less/vim/nano; ConPTY TUIs that scroll above a fixed input line)
	pub scrollbar: bool,          // draw a scrollbar over each pane's right edge
	pub scrollbar_thickness: f32, // scrollbar width in logical px
	pub scrollbar_auto_hide: bool, // fade the scrollbar out while idle at the bottom
	pub margin: f32,              // logical px between content and pane edge
	pub opacity: f32,             // background opacity 0..1 (1 = fully opaque)
	pub transparent_background: bool, // X11: per-pixel bg transparency (text stays opaque) via a GL surface
	pub transparent_background_blur: bool, // X11: ask a KWin/picom compositor to blur the desktop behind the window
	pub wallpaper_enabled: bool,           // master switch: false = no wallpaper at all
	pub wallpaper: Option<PathBuf>,        // resolved path, or None
	pub wallpaper_raw: String, // the value as configured ("" = auto-detect); what the dialog shows
	pub wallpaper_fallback_builtin: bool, // no image/folder configured: show the built-in one
	pub wallpaper_rotate_enabled: bool, // master switch for folder rotation
	pub wallpaper_folder: Option<PathBuf>, // rotate the wallpaper through this folder's images (overrides wallpaper)
	pub wallpaper_folder_auto: bool,       // the folder above was found by convention, not configured
	pub wallpaper_rotate_random: bool,     // rotate randomly instead of in filename order
	pub wallpaper_rotate_interval_s: f32,  // seconds between rotations (0 = pick one at startup only)
	pub wallpaper_opacity: f32,            // image visibility 0..1
	pub wallpaper_default_fit: Fit,        // used unless the image's own tags say otherwise
	pub wallpaper_honor_xmp: bool,         // let a wallpaper's own Fit/Anchor tags win
	pub wallpaper_blur: f32,               // Gaussian blur sigma applied to the image (0 = none)
	pub wallpaper_contrast_mask: bool, // flatten the image's contrast so it stops competing with text
	pub wallpaper_contrast_mask_size: f32, // flatten scale 0..1 (1 = half the longest pixel dim)
	pub wallpaper_contrast_mask_strength: f32, // how far toward the local mean 0..1
	pub wallpaper_contrast_mask_auto: f32, // blend manual knobs with image-derived auto 0..1 (1 = full auto)
	pub text_scrim: bool, // bg-colored blurry halo behind glyphs (readability over busy/transparent bg)
	pub text_scrim_radius: f32, // scrim blur sigma in px
	pub text_scrim_softness: f32, // 0 = hard/solid scrim, 1 = soft/faint (maps to the intensity boost)
	pub text_scrim_strength: f32, // 0..100% -> 0..5 doublings of the halo alpha (0 = as built)
	pub text_outline: f32, // antialiased outline around glyphs, px (0 = none; scrim color rules)
	pub text_scrim_ramp: String, // halo falloff curve: "sigmoid" | "half_normal" | "linear" | "log" | "exp"
	pub text_scrim_function: String, // halo build: "dilate" | "sdf" | "dt" | "gaussian" (legacy blur)
	pub text_scrim_regular_weight: bool, // blur bold text at regular weight (uniform halo; crisp text keeps its weight)
	pub color_emoji: bool, // paint COLRv1 color glyphs (emoji) instead of falling back to a monochrome face
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
	pub hyperlinks: bool,        // underline URLs in output on hover; Ctrl+click opens them
	pub hyperlink_open_command: String, // opener for a clicked link (empty = the desktop's own)
	pub bg: [u8; 3],
	pub fg: [u8; 3],
	pub cursor: [u8; 3],
	// Two attention colors (see theme.rs): `highlight` marks several things at
	// once, `focus` marks only what the keyboard is on.
	pub highlight: [u8; 3],
	pub focus: [u8; 3],
	// chrome colors (menu bar / dropdowns, and pop-out dialogs), from the theme
	// palette; colors.menu_*/colors.dialog_* keys override
	pub menu_bg: [u8; 3],
	pub menu_fg: [u8; 3],
	pub dialog_bg: [u8; 3],
	pub dialog_fg: [u8; 3],
	pub gutter: [u8; 3], // chrome areas holding no control (the dialog's tab strip)
	// scrollbar, neutral in every theme (see SCROLLBAR_THUMB_DEF); the
	// colors.scrollbar_* keys override
	pub scrollbar_thumb: [u8; 3],
	pub scrollbar_trough: [u8; 3],
	pub ansi: [[u8; 3]; 16], // 16-color ANSI palette, resolved from the active theme
	pub theme: String,       // active theme name (see theme.rs)
	pub theme_mode: String,  // "dark" | "light" | "system"
	// Themes saved from the Settings dialog, whole, in file order. They resolve
	// ahead of the built-ins, so one may carry a built-in's name.
	pub user_themes: Vec<crate::theme::UserTheme>,
}

impl Settings {
	// The rotation folder, or None when either master switch is off. Both callers
	// (arming rotation, and the built-in fallback's "nothing is configured" test)
	// go through this so they cannot disagree - and it is derived rather than
	// folded into `wallpaper_folder` at load, because the Settings dialog edits a
	// Settings struct directly and never re-runs `resolve`.
	pub fn rotation_folder(&self) -> Option<&PathBuf> {
		(self.wallpaper_enabled && self.wallpaper_rotate_enabled)
			.then_some(self.wallpaper_folder.as_ref())
			.flatten()
	}

	// The app-slide gate, derived for the same reason: the smooth-scroll master
	// covers every scroll animation, so every smooth_scroll_apps consumer reads
	// this instead of the raw flag and cannot miss the master.
	pub fn smooth_apps(&self) -> bool {
		self.scroll_smooth && self.smooth_scroll_apps
	}
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
			scroll_smooth: true,
			scroll_ease_in_ms: 82.0, // ~ "Ease-in" 50 (motion builds over the first ~tenth of a second)
			scroll_ramp_up_ms: 96.0, // ~ "Ramp-up" 75 (catch-up speed doubles ~10x a second)
			scroll_single_screen_tau_ms: 32.0, // ~ "Single-screen speed" 75 (on-screen burst ceiling: ~31 lines/s)
			scroll_ramp_down_ms: 144.0, // ~ "Ramp-down" 75 (catch-up winds down by halving ~7x a second)
			scroll_ease_out_ms: 212.0,  // ~ "Ease-out" 40 (the tail lands in ~a fifth of a second)
			wheel_lines: 3.0,
			alt_scroll_lines: 3.0,
			output_ease_lines: 1.0,
			smooth_scroll_apps: true,
			scrollbar: true,
			scrollbar_thickness: 16.0,
			scrollbar_auto_hide: true,
			margin: 8.0,
			opacity: 0.95,
			transparent_background: false,
			transparent_background_blur: false,
			wallpaper: None,
			wallpaper_enabled: true,
			wallpaper_raw: String::new(),
			wallpaper_fallback_builtin: true,
			wallpaper_rotate_enabled: true,
			wallpaper_folder: None,
			wallpaper_folder_auto: false,
			wallpaper_rotate_random: true,
			wallpaper_rotate_interval_s: 0.0,
			wallpaper_opacity: 0.10, // image visibility relative to bg color
			wallpaper_default_fit: Fit::Stretch,
			wallpaper_honor_xmp: true,
			wallpaper_blur: 10.0,
			wallpaper_contrast_mask: true,
			wallpaper_contrast_mask_size: 0.5,
			wallpaper_contrast_mask_strength: 0.5,
			wallpaper_contrast_mask_auto: 0.5,
			text_scrim: true,
			text_scrim_radius: 5.0,
			text_scrim_softness: 0.5,
			text_scrim_strength: 15.0,
			text_outline: 1.0,
			text_scrim_ramp: "exp".to_string(),
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
			hyperlinks: true,
			hyperlink_open_command: String::new(),
			bg: [0x00, 0x00, 0x00],
			fg: [0x88, 0xee, 0xcc],
			cursor: [0x96, 0x49, 0xaf],
			highlight: [0xc8, 0xa0, 0x5a],
			focus: [0x40, 0x86, 0xff],
			menu_bg: crate::theme::MENU_BG_DEF,
			menu_fg: crate::theme::MENU_FG_DEF,
			dialog_bg: [0x20, 0x20, 0x2a],
			dialog_fg: [0xe2, 0xe2, 0xea],
			gutter: [0x16, 0x16, 0x1e],
			scrollbar_thumb: SCROLLBAR_THUMB_DEF,
			scrollbar_trough: SCROLLBAR_TROUGH_DEF,
			ansi: crate::theme::resolve("SilkTerm", "dark", true).ansi,
			theme: "SilkTerm".to_string(),
			theme_mode: "dark".to_string(),
			user_themes: Vec::new(),
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
// NOTE: re-derives from the theme, so a one-off colors override is dropped on an
// OS flip; overrides re-apply on the next full config load.
pub fn reapply_for_os(dark: bool) -> bool {
	let prev = OS_DARK.swap(dark, Ordering::Relaxed);
	let current = settings();
	if prev == dark || current.theme_mode != "system" {
		return false;
	}
	let pal = crate::theme::resolve_in(
		&current.user_themes,
		&current.theme,
		&current.theme_mode,
		dark,
	);
	let mut new = (*current).clone();
	new.bg = pal.bg;
	new.fg = pal.fg;
	new.cursor = pal.cursor;
	new.highlight = pal.highlight;
	new.focus = pal.focus;
	new.menu_bg = pal.menu_bg;
	new.menu_fg = pal.menu_fg;
	new.dialog_bg = pal.dialog_bg;
	new.dialog_fg = pal.dialog_fg;
	new.gutter = pal.gutter;
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

// One classified line of a config text, with its full nested path resolved from
// the indentation context (an `enabled:` two levels deep under `wallpaper:` /
// `rotate:` is "wallpaper.rotate.enabled"). Comment lines that spell a setting
// (`# enabled: true`) get a path too - the machinery treats them as that
// setting's disabled default. Lines inside a raw fence are passed through as
// `Fence`; blank lines as `Blank`; anything else as `Other`.
enum WalkLine {
	Setting {
		index: usize,
		path: String,
		active: bool,
		header: bool,
	},
	Fence,
	Blank,
	Other(usize),
}

// Walk a config text line by line, resolving each setting line's full path from
// the enclosing active (or commented) block headers. Indentation is compared by
// leading-whitespace length, matching how the template is written (tabs); a
// line's own key may itself be dotted, which simply extends the path.
fn walk_settings(text: &str) -> Vec<WalkLine> {
	let mut out = Vec::new();
	// (indent, name) of each enclosing block header
	let mut stack: Vec<(usize, String)> = Vec::new();
	let mut fence: Option<(char, usize)> = None;
	for (index, line) in text.lines().enumerate() {
		if let Some((ch, len)) = fence {
			out.push(WalkLine::Fence);
			let t = line.trim();
			if t.chars().all(|c| c == ch) && t.len() >= len {
				fence = None; // closing fence
			}
			continue;
		}
		if let Some(open) = fence_run(line) {
			out.push(WalkLine::Fence);
			fence = Some(open);
			continue;
		}
		if line.trim().is_empty() {
			out.push(WalkLine::Blank);
			continue;
		}
		let Some(key) = line_setting_key(line) else {
			out.push(WalkLine::Other(index));
			continue;
		};
		let trimmed = line.trim_start();
		let active = !trimmed.starts_with('#');
		let indent = line.len() - trimmed.len();
		while stack.last().is_some_and(|(col, _)| indent <= *col) {
			stack.pop();
		}
		let path = stack
			.iter()
			.map(|(_, name)| name.as_str())
			.chain(std::iter::once(key))
			.collect::<Vec<_>>()
			.join(".");
		let header = line_setting_value(line).is_some_and(|v| {
			let v = v.trim();
			v.is_empty() || v.starts_with('#')
		});
		if header {
			stack.push((indent, key.to_string()));
		}
		out.push(WalkLine::Setting {
			index,
			path,
			active,
			header,
		});
	}
	out
}

// A raw-block fence opener: a non-comment line whose content (after an optional
// `key:`) starts a run of 3+ backticks or tildes. Returns the char and length.
fn fence_run(line: &str) -> Option<(char, usize)> {
	if line.trim_start().starts_with('#') {
		return None;
	}
	let after = line
		.split_once(':')
		.map_or(line.trim(), |(_, rest)| rest.trim());
	let ch = after.chars().next()?;
	if ch != '`' && ch != '~' {
		return None;
	}
	let len = after.chars().take_while(|c| *c == ch).count();
	(len >= 3).then_some((ch, len))
}

// Serialize a document back to disk text.
//
// `to_canonical` keeps comments, blank-line grouping, indentation and line
// order, and never rewrites a scalar - so it IS the disk text. shcl 1.2 was
// what made that true: before it, a comment run under a block whose children
// are all commented-out defaults (most of this template) came back at the
// header's depth, and a positional repair pass put the user's own indentation
// back. Verified against the shipped template, a live config and a torture
// fixture: canonical is byte-identical to each.
fn write_doc(path: &std::path::Path, doc: &shcl::Document) {
	if let Err(e) = std::fs::write(path, doc.to_canonical()) {
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
	write_user_themes(&mut doc, &orig.user_themes, &s.user_themes);

	if s.use_system_font != orig.use_system_font {
		doc.set_bool("font.use_system_family", s.use_system_font);
	}
	if s.use_system_font_size != orig.use_system_font_size {
		doc.set_bool("font.use_system_size", s.use_system_font_size);
	}
	if s.font_family != orig.font_family {
		if let Some(f) = &s.font_family {
			doc.set_string("font.family", f);
		}
	}
	if s.font_size != orig.font_size {
		doc.set_float("font.size", r(s.font_size));
	}
	if s.line_height_scale != orig.line_height_scale {
		doc.set_float("font.line_height_scale", r(s.line_height_scale));
	}
	if s.scrollback != orig.scrollback {
		doc.set_int("scroll.scrollback", s.scrollback as i64);
	}
	if s.scroll_smooth != orig.scroll_smooth {
		doc.set_bool("scroll.smooth", s.scroll_smooth);
	}
	if s.scroll_ease_in_ms != orig.scroll_ease_in_ms {
		doc.set_float("scroll.ease_in_ms", r(s.scroll_ease_in_ms));
	}
	if s.scroll_ramp_up_ms != orig.scroll_ramp_up_ms {
		doc.set_float("scroll.ramp_up_ms", r(s.scroll_ramp_up_ms));
	}
	if s.scroll_single_screen_tau_ms != orig.scroll_single_screen_tau_ms {
		doc.set_float(
			"scroll.single_screen_tau_ms",
			r(s.scroll_single_screen_tau_ms),
		);
	}
	if s.scroll_ramp_down_ms != orig.scroll_ramp_down_ms {
		doc.set_float("scroll.ramp_down_ms", r(s.scroll_ramp_down_ms));
	}
	if s.scroll_ease_out_ms != orig.scroll_ease_out_ms {
		doc.set_float("scroll.ease_out_ms", r(s.scroll_ease_out_ms));
	}
	if s.wheel_lines != orig.wheel_lines {
		doc.set_float("scroll.wheel_lines", r(s.wheel_lines));
	}
	if s.alt_scroll_lines != orig.alt_scroll_lines {
		doc.set_float("scroll.alt_scroll_lines", r(s.alt_scroll_lines));
	}
	if s.output_ease_lines != orig.output_ease_lines {
		doc.set_float("scroll.output_ease_lines", r(s.output_ease_lines));
	}
	if s.scrollbar != orig.scrollbar {
		doc.set_bool("scroll.scrollbar.enabled", s.scrollbar);
	}
	if s.scrollbar_thickness != orig.scrollbar_thickness {
		doc.set_float("scroll.scrollbar.thickness", r(s.scrollbar_thickness));
	}
	if s.scrollbar_auto_hide != orig.scrollbar_auto_hide {
		doc.set_bool("scroll.scrollbar.auto_hide", s.scrollbar_auto_hide);
	}
	if s.margin != orig.margin {
		doc.set_float("window.margin", r(s.margin));
	}
	if s.opacity != orig.opacity {
		doc.set_float("transparency.opacity", r(s.opacity));
	}
	if s.transparent_background != orig.transparent_background {
		doc.set_bool("transparency.enabled", s.transparent_background);
	}
	if s.transparent_background_blur != orig.transparent_background_blur {
		doc.set_bool("transparency.blur_behind", s.transparent_background_blur);
	}
	if s.wallpaper_opacity != orig.wallpaper_opacity {
		doc.set_float("wallpaper.opacity", r(s.wallpaper_opacity));
	}
	if s.wallpaper_enabled != orig.wallpaper_enabled {
		doc.set_bool("wallpaper.enabled", s.wallpaper_enabled);
	}
	if s.wallpaper_rotate_enabled != orig.wallpaper_rotate_enabled {
		doc.set_bool("wallpaper.rotate.enabled", s.wallpaper_rotate_enabled);
	}
	if s.wallpaper_default_fit != orig.wallpaper_default_fit {
		doc.set_string(
			"wallpaper.default_fit",
			match s.wallpaper_default_fit {
				Fit::Zoom => "zoom",
				Fit::Stretch => "stretch",
			},
		);
	}
	if s.wallpaper_honor_xmp != orig.wallpaper_honor_xmp {
		doc.set_bool("wallpaper.honor_xmp", s.wallpaper_honor_xmp);
	}
	if s.wallpaper_blur != orig.wallpaper_blur {
		doc.set_float("wallpaper.blur", r(s.wallpaper_blur));
	}
	if s.wallpaper_contrast_mask != orig.wallpaper_contrast_mask {
		doc.set_bool("wallpaper.contrast_mask.enabled", s.wallpaper_contrast_mask);
	}
	if s.wallpaper_contrast_mask_size != orig.wallpaper_contrast_mask_size {
		doc.set_float(
			"wallpaper.contrast_mask.size",
			r(s.wallpaper_contrast_mask_size),
		);
	}
	if s.wallpaper_contrast_mask_strength != orig.wallpaper_contrast_mask_strength {
		doc.set_float(
			"wallpaper.contrast_mask.strength",
			r(s.wallpaper_contrast_mask_strength),
		);
	}
	if s.wallpaper_contrast_mask_auto != orig.wallpaper_contrast_mask_auto {
		doc.set_float(
			"wallpaper.contrast_mask.auto",
			r(s.wallpaper_contrast_mask_auto),
		);
	}
	if s.text_scrim != orig.text_scrim {
		doc.set_bool("text.scrim.enabled", s.text_scrim);
	}
	if s.text_scrim_radius != orig.text_scrim_radius {
		doc.set_float("text.scrim.radius", r(s.text_scrim_radius));
	}
	if s.text_scrim_softness != orig.text_scrim_softness {
		doc.set_float("text.scrim.softness", r(s.text_scrim_softness));
	}
	if s.text_scrim_strength != orig.text_scrim_strength {
		doc.set_float("text.scrim.strength", r(s.text_scrim_strength));
	}
	if s.text_outline != orig.text_outline {
		doc.set_float("text.outline", r(s.text_outline));
	}
	if s.text_scrim_ramp != orig.text_scrim_ramp {
		doc.set_string("text.scrim.ramp", &s.text_scrim_ramp);
	}
	if s.text_scrim_function != orig.text_scrim_function {
		doc.set_string("text.scrim.function", &s.text_scrim_function);
	}
	if s.text_scrim_regular_weight != orig.text_scrim_regular_weight {
		doc.set_bool("text.scrim.regular_weight", s.text_scrim_regular_weight);
	}
	if s.color_emoji != orig.color_emoji {
		doc.set_bool("text.color_emoji", s.color_emoji);
	}
	if s.embolden_inverse != orig.embolden_inverse {
		doc.set_bool("text.embolden_inverse", s.embolden_inverse);
	}
	if s.cursor_scrim != orig.cursor_scrim {
		doc.set_bool("cursor.scrim", s.cursor_scrim);
	}
	if s.cursor_outline != orig.cursor_outline {
		doc.set_bool("cursor.outline", s.cursor_outline);
	}
	if s.cursor_size_height != orig.cursor_size_height {
		doc.set_float("cursor.size.height", r(s.cursor_size_height));
	}
	if s.cursor_size_width != orig.cursor_size_width {
		doc.set_float("cursor.size.width", r(s.cursor_size_width));
	}
	if s.cursor_animation != orig.cursor_animation {
		doc.set_string("cursor.animation", &s.cursor_animation);
	}
	if s.cursor_animation_resume_s != orig.cursor_animation_resume_s {
		doc.set_float("cursor.animation_resume_s", r(s.cursor_animation_resume_s));
	}
	if s.cursor_blink_rate_ms != orig.cursor_blink_rate_ms {
		doc.set_float("cursor.blink_rate_ms", r(s.cursor_blink_rate_ms));
	}
	if s.columns != orig.columns {
		doc.set_int("window.columns", s.columns as i64);
	}
	if s.rows != orig.rows {
		doc.set_int("window.rows", s.rows as i64);
	}
	if s.remember_size != orig.remember_size {
		doc.set_bool("window.remember_size", s.remember_size);
	}
	if s.hide_single_tab != orig.hide_single_tab {
		doc.set_bool("window.hide_single_tab", s.hide_single_tab);
	}
	if s.remembered_columns != orig.remembered_columns {
		doc.set_int("window.remembered_columns", s.remembered_columns as i64);
	}
	if s.remembered_rows != orig.remembered_rows {
		doc.set_int("window.remembered_rows", s.remembered_rows as i64);
	}
	if s.word_separators != orig.word_separators {
		doc.set_string("selection.word_separators", &s.word_separators);
	}
	if s.selection_pairs != orig.selection_pairs {
		doc.set_string("selection.pairs", &s.selection_pairs);
	}
	if s.default_shell != orig.default_shell {
		doc.set_string("shell.default", &s.default_shell);
	}
	if s.command_line != orig.command_line {
		doc.set_string("shell.command_line", &s.command_line);
	}
	if s.copy_on_select != orig.copy_on_select {
		doc.set_bool("shell.copy_on_select", s.copy_on_select);
	}
	if s.hyperlinks != orig.hyperlinks {
		doc.set_bool("hyperlinks.enabled", s.hyperlinks);
	}
	if s.hyperlink_open_command != orig.hyperlink_open_command {
		doc.set_string("hyperlinks.open_command", &s.hyperlink_open_command);
	}
	if s.wallpaper != orig.wallpaper || s.wallpaper_raw != orig.wallpaper_raw {
		// the file keeps whatever form the user wrote (bare/relative/absolute)
		if s.wallpaper_raw.trim().is_empty() {
			doc.remove("wallpaper.image");
		} else {
			doc.set_string("wallpaper.image", s.wallpaper_raw.trim());
		}
	}
	if s.wallpaper_fallback_builtin != orig.wallpaper_fallback_builtin {
		doc.set_bool("wallpaper.fallback_builtin", s.wallpaper_fallback_builtin);
	}

	let mut set_color = |key: &str, color: [u8; 3], orig_color: [u8; 3]| {
		if color != orig_color {
			doc.set_string(&format!("colors.{key}"), &format_hex(color));
		}
	};
	set_color("background", s.bg, orig.bg);
	set_color("foreground", s.fg, orig.fg);
	set_color("cursor", s.cursor, orig.cursor);
	set_color("highlight", s.highlight, orig.highlight);
	set_color("focus", s.focus, orig.focus);
	set_color("gutter", s.gutter, orig.gutter);
	set_color("menu_background", s.menu_bg, orig.menu_bg);
	set_color("menu_foreground", s.menu_fg, orig.menu_fg);
	set_color("dialog_background", s.dialog_bg, orig.dialog_bg);
	set_color("dialog_foreground", s.dialog_fg, orig.dialog_fg);
	// the two scrollbar colors have had dialog rows since the bar shipped but
	// were never written back, so an edit lasted only as long as the session
	set_color("scrollbar_thumb", s.scrollbar_thumb, orig.scrollbar_thumb);
	set_color(
		"scrollbar_trough",
		s.scrollbar_trough,
		orig.scrollbar_trough,
	);

	write_doc(&path, &doc);
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

// LUT: this runs per background cell per rebuilt frame (thousands of powf
// calls otherwise - see pane.rs build).
pub fn to_linear(b: u8) -> f32 {
	static LUT: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
	LUT.get_or_init(|| std::array::from_fn(|i| linear_of(i as u8)))[b as usize]
}

fn linear_of(b: u8) -> f32 {
	let c = f32::from(b) / 255.0;
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
	scroll_smooth: Option<bool>,
	scroll_ease_in_ms: Option<f32>,
	scroll_ramp_up_ms: Option<f32>,
	scroll_single_screen_tau_ms: Option<f32>,
	scroll_ramp_down_ms: Option<f32>,
	scroll_ease_out_ms: Option<f32>,
	wheel_lines: Option<f32>,
	alt_scroll_lines: Option<f32>,
	output_ease_lines: Option<f32>,
	smooth_scroll_apps: Option<bool>,
	scrollbar: Option<bool>,
	scrollbar_thickness: Option<f32>,
	scrollbar_auto_hide: Option<bool>,
	margin: Option<f32>,
	opacity: Option<f32>,
	transparent_background: Option<bool>,
	transparent_background_blur: Option<bool>,
	wallpaper_enabled: Option<bool>,
	wallpaper: Option<String>,
	wallpaper_fallback_builtin: Option<bool>,
	wallpaper_rotate_enabled: Option<bool>,
	wallpaper_folder: Option<String>,
	wallpaper_rotate_random: Option<bool>,
	wallpaper_rotate_interval_s: Option<f32>,
	wallpaper_opacity: Option<f32>,
	wallpaper_default_fit: Option<String>,
	wallpaper_honor_xmp: Option<bool>,
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
	text_scrim_strength: Option<f32>,
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
	hyperlinks: Option<bool>,
	hyperlink_open_command: Option<String>,
	colors: RawColors,
	user_themes: Vec<crate::theme::UserTheme>,
}

#[derive(Default)]
struct RawColors {
	background: Option<String>,
	foreground: Option<String>,
	cursor: Option<String>,
	highlight: Option<String>,
	focus: Option<String>,
	menu_background: Option<String>,
	menu_foreground: Option<String>,
	dialog_background: Option<String>,
	dialog_foreground: Option<String>,
	gutter: Option<String>,
	scrollbar_thumb: Option<String>,
	scrollbar_trough: Option<String>,
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
	// A pre-nesting config converts wholesale first (backed up to .bak, active
	// values carried over). Then migrate an existing config in place
	// (rename/remove changed keys) and backfill any keys it's missing, so an
	// updated config stays current without clobbering the user's existing
	// values. These are the only launch-time writes, and each runs only when
	// the program's own option set changed. The in-place writes defer (with an
	// FYI) if the file looks open in another program.
	convert_legacy_config(&path);
	migrate_config(&path);
	backfill_config(&path);
	let raw = match std::fs::read_to_string(&path) {
		// The writes above defer when the file looks open elsewhere, so parse the
		// migrated text rather than what is on disk: a renamed key must never be
		// read under its old spelling, which matters most where a rename hands an
		// old name to a new setting (colors.focus).
		Ok(text) => read_raw(&migrate_config_text(&text).unwrap_or(text), &path),
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

// " line 4" / " lines 2, 4" for a diagnostic, empty when there is nothing to
// cite (a writer-built node reports 0, and a wildcard slot that missed does too).
fn line_list(lines: &[usize]) -> String {
	let cited: Vec<String> = lines
		.iter()
		.filter(|n| **n > 0)
		.map(std::string::ToString::to_string)
		.collect();
	match cited.len() {
		0 => String::new(),
		1 => format!(" line {}", cited[0]),
		_ => format!(" lines {}", cited.join(", ")),
	}
}

impl Reader<'_> {
	// Complain once about a value that is present but the wrong type. Anything
	// else (absent, empty) is silent - a commented-out setting is the norm here.
	fn note<T>(&self, key: &str, got: Result<T, shcl::Status>) -> Option<T> {
		match got {
			Ok(v) => Some(v),
			Err(shcl::Status::BadType) => {
				eprintln!(
					"{APP_NAME}: {}{}: ignoring invalid value for `{key}`",
					self.path.display(),
					line_list(&self.doc.lines(key))
				);
				None
			}
			Err(shcl::Status::Multiple) => {
				// Set more than once. shcl refuses to pick a winner, so the
				// default is what actually takes effect - which used to happen
				// in silence, and reads as the setting being ignored outright.
				// Cite every line: the point is that there IS more than one.
				eprintln!(
					"{APP_NAME}: {}{}: `{key}` is set more than once, so its default is used",
					self.path.display(),
					line_list(&self.doc.lines(key))
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
		use_system_font: r.b("font.use_system_family"),
		use_system_font_size: r.b("font.use_system_size"),
		font_family: r.s("font.family"),
		font_size: r.f("font.size"),
		line_height_scale: r.f("font.line_height_scale"),
		scrollback: r.u("scroll.scrollback"),
		scroll_smooth: r.b("scroll.smooth"),
		scroll_ease_in_ms: r.f("scroll.ease_in_ms"),
		scroll_ramp_up_ms: r.f("scroll.ramp_up_ms"),
		scroll_single_screen_tau_ms: r.f("scroll.single_screen_tau_ms"),
		scroll_ramp_down_ms: r.f("scroll.ramp_down_ms"),
		scroll_ease_out_ms: r.f("scroll.ease_out_ms"),
		wheel_lines: r.f("scroll.wheel_lines"),
		alt_scroll_lines: r.f("scroll.alt_scroll_lines"),
		output_ease_lines: r.f("scroll.output_ease_lines"),
		smooth_scroll_apps: r.b("scroll.smooth_apps"),
		scrollbar: r.b("scroll.scrollbar.enabled"),
		scrollbar_thickness: r.f("scroll.scrollbar.thickness"),
		scrollbar_auto_hide: r.b("scroll.scrollbar.auto_hide"),
		margin: r.f("window.margin"),
		opacity: r.f("transparency.opacity"),
		transparent_background: r.b("transparency.enabled"),
		transparent_background_blur: r.b("transparency.blur_behind"),
		wallpaper_enabled: r.b("wallpaper.enabled"),
		wallpaper: r.s("wallpaper.image"),
		wallpaper_fallback_builtin: r.b("wallpaper.fallback_builtin"),
		wallpaper_rotate_enabled: r.b("wallpaper.rotate.enabled"),
		wallpaper_folder: r.s("wallpaper.rotate.folder"),
		wallpaper_rotate_random: r.b("wallpaper.rotate.random"),
		wallpaper_rotate_interval_s: r.f("wallpaper.rotate.interval_s"),
		wallpaper_opacity: r.f("wallpaper.opacity"),
		wallpaper_default_fit: r.s("wallpaper.default_fit"),
		wallpaper_honor_xmp: r.b("wallpaper.honor_xmp"),
		wallpaper_blur: r.f("wallpaper.blur"),
		wallpaper_contrast_mask: r.b("wallpaper.contrast_mask.enabled"),
		wallpaper_contrast_mask_size: r.f("wallpaper.contrast_mask.size"),
		wallpaper_contrast_mask_strength: r.f("wallpaper.contrast_mask.strength"),
		wallpaper_contrast_mask_auto: r.f("wallpaper.contrast_mask.auto"),
		theme: r.s("theme"),
		theme_mode: r.s("theme_mode"),
		text_scrim: r.b("text.scrim.enabled"),
		text_scrim_radius: r.f("text.scrim.radius"),
		text_scrim_softness: r.f("text.scrim.softness"),
		text_scrim_strength: r.f("text.scrim.strength"),
		text_outline: r.f("text.outline"),
		text_scrim_ramp: r.s("text.scrim.ramp"),
		text_scrim_function: r.s("text.scrim.function"),
		text_scrim_regular_weight: r.b("text.scrim.regular_weight"),
		color_emoji: r.b("text.color_emoji"),
		embolden_inverse: r.b("text.embolden_inverse"),
		cursor_scrim: r.b("cursor.scrim"),
		cursor_outline: r.b("cursor.outline"),
		cursor_size_height: r.f("cursor.size.height"),
		cursor_size_width: r.f("cursor.size.width"),
		cursor_animation: r.s("cursor.animation"),
		cursor_animation_resume_s: r.f("cursor.animation_resume_s"),
		cursor_animation_idle_stop_s: r.f("cursor.animation_idle_stop_s"),
		cursor_blink_rate_ms: r.f("cursor.blink_rate_ms"),
		columns: r.u("window.columns"),
		rows: r.u("window.rows"),
		remember_size: r.b("window.remember_size"),
		hide_single_tab: r.b("window.hide_single_tab"),
		remembered_columns: r.u("window.remembered_columns"),
		remembered_rows: r.u("window.remembered_rows"),
		word_separators: r.s("selection.word_separators"),
		selection_pairs: r.s("selection.pairs"),
		default_shell: r.s("shell.default"),
		command_line: r.s("shell.command_line"),
		copy_on_select: r.b("shell.copy_on_select"),
		hyperlinks: r.b("hyperlinks.enabled"),
		hyperlink_open_command: r.s("hyperlinks.open_command"),
		colors: RawColors {
			background: r.s("colors.background"),
			foreground: r.s("colors.foreground"),
			cursor: r.s("colors.cursor"),
			highlight: r.s("colors.highlight"),
			focus: r.s("colors.focus"),
			menu_background: r.s("colors.menu_background"),
			menu_foreground: r.s("colors.menu_foreground"),
			dialog_background: r.s("colors.dialog_background"),
			dialog_foreground: r.s("colors.dialog_foreground"),
			gutter: r.s("colors.gutter"),
			scrollbar_thumb: r.s("colors.scrollbar_thumb"),
			scrollbar_trough: r.s("colors.scrollbar_trough"),
		},
		user_themes: read_user_themes(&r.doc),
	}
}

// Saved themes, in file order. A slug with no readable colors at all is skipped;
// anything else missing falls back to the first built-in, so a hand-edited or
// half-written block still yields a usable theme rather than none.
fn read_user_themes(doc: &shcl::Document) -> Vec<crate::theme::UserTheme> {
	let base = crate::theme::THEMES[0].1;
	let mut out = Vec::new();
	for slug in doc.children("themes") {
		let read = |mode: &str, fallback: crate::theme::Palette| {
			let mut pal = fallback;
			let mut any = false;
			for (i, key) in crate::theme::PALETTE_KEYS.iter().enumerate() {
				if let Some(c) = doc
					.get_string(&format!("themes.{slug}.{mode}.{key}"))
					.ok()
					.as_deref()
					.and_then(parse_hex)
				{
					pal.set(i, c);
					any = true;
				}
			}
			if let Ok(list) = doc.get_string_array(&format!("themes.{slug}.{mode}.ansi")) {
				for (slot, text) in pal.ansi.iter_mut().zip(list.iter()) {
					if let Some(c) = parse_hex(text) {
						*slot = c;
						any = true;
					}
				}
			}
			(pal, any)
		};
		let (dark, got_dark) = read("dark", base.dark);
		let (light, got_light) = read("light", base.light);
		if !got_dark && !got_light {
			continue;
		}
		let name = doc
			.get_string(&format!("themes.{slug}.name"))
			.ok()
			.filter(|n| !n.trim().is_empty())
			.unwrap_or_else(|| slug.clone());
		out.push(crate::theme::UserTheme {
			slug,
			name,
			dark,
			light,
		});
	}
	out
}

// Bring the file's `themes.*` subtrees in line with the dialog's list. A theme
// that changed at all is dropped and rewritten whole rather than edited field by
// field: saving, renaming and deleting are then one operation with one shape, and
// a stale color cannot survive under a name that no longer sets it.
fn write_user_themes(
	doc: &mut shcl::Document,
	orig: &[crate::theme::UserTheme],
	now: &[crate::theme::UserTheme],
) {
	for old in orig {
		if !now.iter().any(|t| t.slug == old.slug) {
			doc.remove(&format!("themes.{}", old.slug));
		}
	}
	for theme in now {
		if orig.iter().any(|t| t == theme) {
			continue;
		}
		let at = format!("themes.{}", theme.slug);
		doc.remove(&at);
		doc.set_string(&format!("{at}.name"), &theme.name);
		for (mode, pal) in [("dark", &theme.dark), ("light", &theme.light)] {
			for (i, key) in crate::theme::PALETTE_KEYS.iter().enumerate() {
				doc.set_string(&format!("{at}.{mode}.{key}"), &format_hex(pal.get(i)));
			}
			let ansi: Vec<String> = pal.ansi.iter().map(|c| format_hex(*c)).collect();
			let ansi: Vec<&str> = ansi.iter().map(String::as_str).collect();
			doc.set_string_array(&format!("{at}.{mode}.ansi"), &ansi);
		}
	}
}

fn resolve(raw: RawConfig) -> Settings {
	let d = Settings::default();
	let theme_name = raw.theme.unwrap_or_else(|| d.theme.clone());
	let theme_mode = raw.theme_mode.unwrap_or_else(|| d.theme_mode.clone());
	// system-mode OS dark/light detection is wired later; default to dark for now
	let pal = crate::theme::resolve_in(
		&raw.user_themes,
		&theme_name,
		&theme_mode,
		OS_DARK.load(Ordering::Relaxed),
	);
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
	let configured_folder = resolve_wallpaper_folder(raw.wallpaper_folder);
	let folder = configured_folder
		.clone()
		.or_else(|| (!pinned_wallpaper).then(default_wallpaper_folder).flatten());
	let wallpaper_enabled = raw.wallpaper_enabled.unwrap_or(d.wallpaper_enabled);
	let wallpaper_rotate_enabled = raw
		.wallpaper_rotate_enabled
		.unwrap_or(d.wallpaper_rotate_enabled);
	// With rotation live, don't also hunt for a conventional wallpaper file: that
	// is a run of stats on paths which may be a slow mount, and the first rotation
	// pick replaces whatever it found anyway.
	let rotating = wallpaper_enabled && wallpaper_rotate_enabled && folder.is_some();
	let wallpaper = (pinned_wallpaper || !rotating)
		.then(|| resolve_wallpaper(raw.wallpaper.clone()))
		.flatten();
	Settings {
		// only the convention folder is "auto"; whether it holds anything is the
		// scan's business, and the scan runs off this thread
		wallpaper_folder_auto: configured_folder.is_none(),
		use_system_font,
		// absent = follow the face toggle, so configs predating the split (and an
		// explicit font_size, which used to imply off) keep their exact behavior
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
		scroll_smooth: raw.scroll_smooth.unwrap_or(d.scroll_smooth),
		scroll_ease_in_ms: raw
			.scroll_ease_in_ms
			.unwrap_or(d.scroll_ease_in_ms)
			.max(1.0),
		scroll_ramp_up_ms: raw
			.scroll_ramp_up_ms
			.unwrap_or(d.scroll_ramp_up_ms)
			.max(1.0),
		scroll_single_screen_tau_ms: raw
			.scroll_single_screen_tau_ms
			.unwrap_or(d.scroll_single_screen_tau_ms)
			.max(1.0),
		scroll_ramp_down_ms: raw
			.scroll_ramp_down_ms
			.unwrap_or(d.scroll_ramp_down_ms)
			.max(1.0),
		scroll_ease_out_ms: raw
			.scroll_ease_out_ms
			.unwrap_or(d.scroll_ease_out_ms)
			.max(1.0),
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
		scrollbar: raw.scrollbar.unwrap_or(d.scrollbar),
		// floor keeps it grabbable; ceiling keeps it from swallowing a narrow pane
		scrollbar_thickness: raw
			.scrollbar_thickness
			.unwrap_or(d.scrollbar_thickness)
			.clamp(4.0, 64.0),
		scrollbar_auto_hide: raw.scrollbar_auto_hide.unwrap_or(d.scrollbar_auto_hide),
		margin: raw.margin.unwrap_or(d.margin).max(0.0),
		opacity: raw.opacity.unwrap_or(d.opacity).clamp(0.0, 1.0),
		transparent_background: raw
			.transparent_background
			.unwrap_or(d.transparent_background),
		transparent_background_blur: raw
			.transparent_background_blur
			.unwrap_or(d.transparent_background_blur),
		wallpaper_enabled,
		wallpaper_raw: raw.wallpaper.clone().unwrap_or_default(),
		wallpaper,
		wallpaper_fallback_builtin: raw
			.wallpaper_fallback_builtin
			.unwrap_or(d.wallpaper_fallback_builtin),
		wallpaper_rotate_enabled,
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
		text_scrim_strength: raw
			.text_scrim_strength
			.unwrap_or(d.text_scrim_strength)
			.clamp(0.0, 100.0),
		text_outline: raw.text_outline.unwrap_or(d.text_outline).clamp(0.0, 8.0),
		// the older spellings still parse: "s" was renamed to "sigmoid" (which is
		// what a smoothstep is), and the falloff's "gaussian" to "half_normal" so
		// it stops reading like the gaussian BLUR the function list also offers.
		text_scrim_ramp: match raw.text_scrim_ramp.as_deref() {
			Some("linear") => "linear".to_string(),
			Some("half_normal" | "gaussian") => "half_normal".to_string(),
			Some("sigmoid" | "s") => "sigmoid".to_string(),
			Some("log") => "log".to_string(),
			Some("exp") => "exp".to_string(),
			_ => d.text_scrim_ramp.clone(), // missing/unknown -> default (exponential)
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
		wallpaper_default_fit: match raw.wallpaper_default_fit.as_deref() {
			Some("zoom") => Fit::Zoom,
			_ => Fit::Stretch,
		},
		wallpaper_honor_xmp: raw.wallpaper_honor_xmp.unwrap_or(d.wallpaper_honor_xmp),
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
		hyperlinks: raw.hyperlinks.unwrap_or(d.hyperlinks),
		hyperlink_open_command: raw
			.hyperlink_open_command
			.unwrap_or(d.hyperlink_open_command),
		bg: color(raw.colors.background, pal.bg),
		fg: color(raw.colors.foreground, pal.fg),
		cursor: color(raw.colors.cursor, pal.cursor),
		highlight: color(raw.colors.highlight, pal.highlight),
		focus: color(raw.colors.focus, pal.focus),
		menu_bg: color(raw.colors.menu_background, pal.menu_bg),
		menu_fg: color(raw.colors.menu_foreground, pal.menu_fg),
		dialog_bg: color(raw.colors.dialog_background, pal.dialog_bg),
		dialog_fg: color(raw.colors.dialog_foreground, pal.dialog_fg),
		gutter: color(raw.colors.gutter, pal.gutter),
		scrollbar_thumb: color(raw.colors.scrollbar_thumb, SCROLLBAR_THUMB_DEF),
		scrollbar_trough: color(raw.colors.scrollbar_trough, SCROLLBAR_TROUGH_DEF),
		ansi: pal.ansi,
		theme: theme_name,
		theme_mode,
		user_themes: raw.user_themes,
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
// to follow resolves from font_family / font_size as if off, and grays out.
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
// `~` and `~/...` expand to $HOME (USERPROFILE on Windows). A config is text a
// person edits by hand, and that is how they write a home-relative path. `~user`
// is left literal - there is nothing to resolve it against.
fn expand_tilde(value: &str) -> PathBuf {
	let rest = match value.strip_prefix('~') {
		Some("") => "",
		Some(rest) if rest.starts_with('/') || rest.starts_with('\\') => &rest[1..],
		_ => return PathBuf::from(value),
	};
	std::env::var_os("HOME")
		.or_else(|| std::env::var_os("USERPROFILE"))
		.filter(|home| !home.is_empty())
		.map_or_else(
			|| PathBuf::from(value),
			|home| PathBuf::from(home).join(rest),
		)
}

pub fn resolve_wallpaper(explicit: Option<String>) -> Option<PathBuf> {
	let dir = config_path()?.parent()?.to_path_buf();
	if let Some(given) = explicit.filter(|value| !value.trim().is_empty()) {
		let path = expand_tilde(given.trim());
		// Handed back unchecked: the loader opens it on its own thread and says so
		// if it can't, which keeps a wallpaper the user explicitly named from
		// costing a stat here - it may be the very mount that answers slowly.
		return Some(if path.is_absolute() {
			path
		} else {
			dir.join(path)
		});
	}
	// Current convention first (wallpaper/wallpaper.*), then the older spellings
	// so existing setups keep working.
	[
		("wallpaper", "wallpaper"),
		("wallpapers", "wallpaper"),
		("backgrounds", "background"),
	]
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
// dir (like the single wallpaper). Not checked for existence here - the scan
// runs off the startup thread and reports an unreadable folder itself, so a typo
// still just leaves rotation off.
pub fn resolve_wallpaper_folder(explicit: Option<String>) -> Option<PathBuf> {
	let given = explicit.filter(|value| !value.trim().is_empty())?;
	let path = expand_tilde(given.trim());
	if path.is_absolute() {
		return Some(path);
	}
	Some(config_path()?.parent()?.join(&path))
}

// Enough kept resets that nobody hits the ceiling in practice, low enough that a
// script looping on --reset-config stops piling up files instead of forever.
const BACKUPS_MAX: u32 = 99;

// Move a config aside to the first free `.bak` name - so doing it twice never
// overwrites the copy from the first time. Returns where it went.
fn backup_aside(path: &std::path::Path) -> Option<PathBuf> {
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
	match std::fs::rename(path, &backup) {
		Ok(()) => Some(backup),
		Err(e) => {
			eprintln!("{APP_NAME}: could not move {} aside: {e}", path.display());
			None
		}
	}
}

// Move the config aside so the next load writes a fresh one from the template.
// The old file is kept, not deleted. Returns where it went, or None if there
// was nothing to move.
pub fn reset_config() -> Option<PathBuf> {
	let path = config_path()?;
	if !path.exists() {
		return None;
	}
	backup_aside(&path)
}

// Where the wallpaper shuffle keeps its recently-shown list. Beside the config,
// so a --config override gets its own history instead of sharing one.
pub fn wallpaper_history_path() -> Option<PathBuf> {
	Some(config_path()?.parent()?.join(".wallpaper-history"))
}

// Image files we're willing to load as a wallpaper. One list, so the folder
// auto-detect below and the rotation scan can't disagree about what counts.
// Must track the `image` crate's enabled features (png + jpeg) - it is built
// with default-features off to keep the binary small, so listing anything else
// here just picks a file that then fails to decode.
pub fn is_image_file(path: &std::path::Path) -> bool {
	path.extension()
		.and_then(|ext| ext.to_str())
		.is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg"))
}

// The rotation folder to use when none is configured: the conventional
// wallpaper/ dir (or the legacy spellings) under the config dir. Only its
// existence is tested - reading it to see whether it holds an image is the
// scan's job, off the startup thread, and an empty one still means no rotation
// and no diagnostic, since the user never asked for one.
fn default_wallpaper_folder() -> Option<PathBuf> {
	let dir = config_path()?.parent()?.to_path_buf();
	["wallpaper", "wallpapers", "backgrounds"]
		.into_iter()
		.map(|sub| dir.join(sub))
		.find(|sub| sub.is_dir())
}

// A config file's settings as (key, original-line). Nested settings are written
// in dotted form, so a key like "colors.focus" needs no table context.
// Recognizes both active (`k: ...`) and commented (`# k: ...`) lines.
// Every setting line of `text` as (full path, verbatim line), commented ones
// included - the walker resolves nesting, so a `# height: 100` two levels down
// comes back as "cursor.size.height".
fn setting_lines(text: &str) -> Vec<(String, String)> {
	let lines: Vec<&str> = text.lines().collect();
	walk_settings(text)
		.into_iter()
		.filter_map(|w| match w {
			WalkLine::Setting { index, path, .. } => Some((path, lines[index].to_string())),
			_ => None,
		})
		.collect()
}

// Like `setting_lines`, but each setting carries the contiguous comment lines
// directly above it (its block), plus `new_group` = whether a blank line
// precedes it in the template. Backfill uses this to keep a template group's
// settings together (no internal blank) while separating groups by a blank line.
fn setting_groups(text: &str) -> Vec<(String, Vec<String>, bool)> {
	let lines: Vec<&str> = text.lines().collect();
	let mut pending: Vec<String> = Vec::new();
	let mut group_break = true; // the first setting begins a group
	let mut out = Vec::new();
	for w in walk_settings(text) {
		match w {
			WalkLine::Setting { index, path, .. } => {
				let mut block = std::mem::take(&mut pending);
				block.push(lines[index].to_string());
				out.push((path, block, group_break));
				group_break = false;
			}
			WalkLine::Blank => {
				pending.clear();
				group_break = true;
			}
			WalkLine::Other(index) if lines[index].trim_start().starts_with('#') => {
				pending.push(lines[index].to_string());
			}
			_ => pending.clear(),
		}
	}
	out
}

// The key of a settings line, active or commented-out. Dots are part of the key
// ("colors.foreground"), so a dotted setting stays one self-contained line; the
// walker supplies the enclosing-block context for truly nested ones.
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

// Paths that were renamed across versions (old -> new). A rename rewrites the
// key on its line, preserving the comment/active state; if the new path is
// already present the old one is just dropped. Renames must stay within the
// same parent block - the machinery rewrites the line in place, it does not
// move lines between blocks. (The whole pre-nesting flat namespace is handled
// separately by `convert_legacy_config`, not here.)
const CONFIG_RENAMES: &[(&str, &str)] = &[
	("scroll.inview_tau_ms", "scroll.single_screen_tau_ms"),
	// The one attention color became two. The old key's value IS the calmer of
	// the pair, so it carries to `highlight` and the freed-up `colors.focus`
	// starts from its own default. That reuse is the reason `load` migrates the
	// text it parses as well as the file: a config open in an editor defers the
	// write, and the old spelling must not read as its successor even once.
	("colors.focus", "colors.highlight"),
];
// Paths that no longer exist and should be removed from an existing config.
// scroll.tau_ms ("Initial scroll speed") has no successor: the speed curve now
// leaves rest through Ease-in and the one knob that fed four mechanisms is
// gone. scroll.ease_in was a unitless fraction; its replacement is a duration
// (scroll.ease_in_ms), so the old value cannot be carried by a rename.
const CONFIG_REMOVED: &[&str] = &["scroll.tau_ms", "scroll.ease_in"];

// Defaults that changed, as (path, the value that used to be the default). An
// existing config carries the template's commented lines verbatim, so after a
// default changes those lines quietly describe the old behavior. A commented
// line matching the outgoing default is refreshed to the current template line.
// An ACTIVE line is never touched: that value is the user's own choice, and it
// keeps working exactly as they set it. NOTE: the stored value is the raw
// post-colon text, trailing `## Default` marker included.
const SUPERSEDED_DEFAULTS: &[(&str, &str)] = &[
	// the falloff's "gaussian" is spelled "half_normal" now (both still parse),
	// and the shipped curve moved on again to the exponential
	("text.scrim.ramp", "\"gaussian\"  ## Default"),
	("text.scrim.ramp", "\"half_normal\"  ## Default"),
	// the halo used to ship exactly as built, before the scale halved, and the
	// first tuned value on the new scale was a shade heavier
	("text.scrim.strength", "0  ## Default"),
	("text.scrim.strength", "30  ## Default"),
	("text.scrim.strength", "20  ## Default"),
	// the outline shipped at two pixels before the halo carried more of the work
	("text.outline", "2.0  ## Default"),
	// these two never tracked the theme they document - they carried a gray and a
	// steel blue from before themes existed, so every config in the wild has them
	("colors.foreground", "\"#d2d2da\"  ## Default"),
	("colors.cursor", "\"#7a9ad0\"  ## Default"),
	// the cursor briefly shipped as the cool third of the same triad, then as the
	// warm one - both at the text's own brightness, which is why neither could be
	// read through
	("colors.cursor", "\"#cc88ee\"  ## Default"),
	("colors.cursor", "\"#eecc88\"  ## Default"),
	// the pane ring was a cold blue, picked for the palette before this one (the
	// key was `colors.focus` then, so a config carrying it arrives here renamed)
	("colors.highlight", "\"#5580c8\"  ## Default"),
];

// The whole pre-nesting flat namespace, old key -> new nested path. Primary
// (most recent flat) names first; still-older aliases after, so when a config
// somehow carries both spellings the newer one wins. `colors.*` map to
// themselves - the path didn't change, but active overrides still carry over.
#[rustfmt::skip]
const LEGACY_KEYS: &[(&str, &str)] = &[
	("use_system_font", "font.use_system_family"),
	("use_system_font_size", "font.use_system_size"),
	("font_family", "font.family"),
	("font_size", "font.size"),
	("line_height_scale", "font.line_height_scale"),
	("margin", "window.margin"),
	("columns", "window.columns"),
	("rows", "window.rows"),
	("remember_size", "window.remember_size"),
	("remembered_columns", "window.remembered_columns"),
	("remembered_rows", "window.remembered_rows"),
	("hide_single_tab", "window.hide_single_tab"),
	("transparent_background", "transparency.enabled"),
	("opacity", "transparency.opacity"),
	("transparent_background_blur", "transparency.blur_behind"),
	("wallpaper_enabled", "wallpaper.enabled"),
	("wallpaper", "wallpaper.image"),
	("wallpaper_fallback_builtin", "wallpaper.fallback_builtin"),
	("wallpaper_rotate_enabled", "wallpaper.rotate.enabled"),
	("wallpaper_folder", "wallpaper.rotate.folder"),
	("wallpaper_rotate_interval_s", "wallpaper.rotate.interval_s"),
	("wallpaper_rotate_random", "wallpaper.rotate.random"),
	("wallpaper_opacity", "wallpaper.opacity"),
	("wallpaper_default_fit", "wallpaper.default_fit"),
	("wallpaper_honor_xmp", "wallpaper.honor_xmp"),
	("wallpaper_blur", "wallpaper.blur"),
	("wallpaper_contrast_mask", "wallpaper.contrast_mask.enabled"),
	("wallpaper_contrast_mask_size", "wallpaper.contrast_mask.size"),
	("wallpaper_contrast_mask_strength", "wallpaper.contrast_mask.strength"),
	("wallpaper_contrast_mask_auto", "wallpaper.contrast_mask.auto"),
	("text_scrim", "text.scrim.enabled"),
	("text_scrim_radius", "text.scrim.radius"),
	("text_scrim_softness", "text.scrim.softness"),
	("text_scrim_function", "text.scrim.function"),
	("text_scrim_ramp", "text.scrim.ramp"),
	("text_scrim_regular_weight", "text.scrim.regular_weight"),
	("text_outline", "text.outline"),
	("color_emoji", "text.color_emoji"),
	("embolden_inverse", "text.embolden_inverse"),
	("cursor_scrim", "cursor.scrim"),
	("cursor_outline", "cursor.outline"),
	("cursor_size_height", "cursor.size.height"),
	("cursor_size_width", "cursor.size.width"),
	("cursor_animation", "cursor.animation"),
	("cursor_animation_resume_s", "cursor.animation_resume_s"),
	("cursor_animation_idle_stop_s", "cursor.animation_idle_stop_s"),
	("cursor_blink_rate_ms", "cursor.blink_rate_ms"),
	("word_separators", "selection.word_separators"),
	("selection_pairs", "selection.pairs"),
	("default_shell", "shell.default"),
	("command_line", "shell.command_line"),
	("copy_on_select", "shell.copy_on_select"),
	("scrollback", "scroll.scrollback"),
	("scroll_tau_ms", "scroll.tau_ms"),
	("wheel_lines", "scroll.wheel_lines"),
	("alt_scroll_lines", "scroll.alt_scroll_lines"),
	("output_ease_lines", "scroll.output_ease_lines"),
	("smooth_scroll_apps", "scroll.smooth_apps"),
	// still-older spellings (pre-rename vintages), lowest precedence
	("cursor_size_vertical", "cursor.size.height"),
	("cursor_size_horizontal", "cursor.size.width"),
	("text_glow", "text.scrim.enabled"),
	("text_glow_radius", "text.scrim.radius"),
	("text_glow_softness", "text.scrim.softness"),
	("text_glow_ramp", "text.scrim.ramp"),
	("text_glow_regular_weight", "text.scrim.regular_weight"),
	("text_glow_border", "text.outline"),
	("cursor_glow", "cursor.scrim"),
	("background_image", "wallpaper.image"),
	("background_folder", "wallpaper.rotate.folder"),
	("background_default", "wallpaper.fallback_builtin"),
	("wallpaper_default", "wallpaper.fallback_builtin"),
	("background_fit", "wallpaper.default_fit"),
	("wallpaper_fit", "wallpaper.default_fit"),
	("background_blur", "wallpaper.blur"),
	("background_opacity", "wallpaper.opacity"),
	("background_rotate_random", "wallpaper.rotate.random"),
	("background_rotate_interval_s", "wallpaper.rotate.interval_s"),
	("background_contrast_mask", "wallpaper.contrast_mask.enabled"),
	("background_contrast_mask_size", "wallpaper.contrast_mask.size"),
	("background_contrast_mask_strength", "wallpaper.contrast_mask.strength"),
	("background_contrast_mask_auto", "wallpaper.contrast_mask.auto"),
];

// A carried value keeps its exact spelling but not an old trailing comment -
// the new template's comments describe the setting already. `#` inside a
// quoted value (every color) survives.
fn strip_trailing_comment(value: &str) -> &str {
	let mut quote: Option<char> = None;
	let mut escaped = false;
	for (at, c) in value.char_indices() {
		if escaped {
			escaped = false;
			continue;
		}
		match c {
			'\\' => escaped = true,
			'"' | '\'' => match quote {
				Some(q) if q == c => quote = None,
				None => quote = Some(c),
				_ => {}
			},
			'#' if quote.is_none() => return value[..at].trim_end(),
			_ => {}
		}
	}
	value
}

// Rewrite the template line for `path` as an active assignment of `value`,
// keeping the template's own indentation and key spelling.
fn activate_line(lines: &mut [String], path: &str, value: &str) -> bool {
	let text = lines.join("\n");
	for w in walk_settings(&text) {
		if let WalkLine::Setting { index, path: p, .. } = w {
			if p == path {
				let line = &lines[index];
				let trimmed = line.trim_start();
				let indent = &line[..line.len() - trimmed.len()];
				let Some(key) = line_setting_key(line) else {
					return false;
				};
				lines[index] = format!("{indent}{key}: {value}");
				return true;
			}
		}
	}
	false
}

// One-time conversion of a pre-nesting config: the flat `wallpaper_*`-style
// namespace became nested blocks, and rewriting that in place would shred the
// old file's comments and grouping. Instead the old file is moved aside to a
// `.bak` and a fresh template is written with every ACTIVE old value carried
// over to its new path - settings survive, and the file's documentation is
// current instead of half-old. Unknown `themes.*` subtrees (user data for a
// future feature) are carried verbatim in dotted form.
fn convert_legacy_config(path: &std::path::Path) {
	let Ok(text) = std::fs::read_to_string(path) else {
		return;
	};
	let lines: Vec<&str> = text.lines().collect();
	let walked = walk_settings(&text);
	// Only an ACTIVE flat key marks a file as legacy: any real old config has
	// several (the template shipped with them), while a comment that merely
	// spells an old name - e.g. one a relayouted save left at column 0 - must
	// never nuke a current-format file.
	let legacy = walked.iter().any(|w| {
		matches!(w, WalkLine::Setting { path: p, header: false, active: true, .. }
			if !p.contains('.') && LEGACY_KEYS.iter().any(|(old, _)| old == p))
	});
	if !legacy {
		return;
	}

	// active values: current-format paths carry as themselves (a mixed file
	// loses nothing), old spellings map through the table, best (lowest table
	// index) spelling winning per new path
	let known_new: std::collections::HashSet<String> = setting_lines(DEFAULT_CONFIG)
		.into_iter()
		.map(|(p, _)| p)
		.collect();
	let mut carry: std::collections::HashMap<String, (usize, String)> =
		std::collections::HashMap::new();
	let mut extras: Vec<String> = Vec::new();
	let mut had_font_family = false;
	let mut had_use_system = false;
	for w in &walked {
		let WalkLine::Setting {
			index,
			path: p,
			active,
			header,
		} = w
		else {
			continue;
		};
		if !active || *header {
			continue;
		}
		let Some(value) = line_setting_value(lines[*index]) else {
			continue;
		};
		let value = strip_trailing_comment(value).trim();
		if value.is_empty() {
			continue;
		}
		if p == "use_system_font" {
			had_use_system = true;
		}
		if p == "font_family" {
			had_font_family = true;
		}
		let target = if known_new.contains(p) {
			Some((0, p.clone()))
		} else {
			LEGACY_KEYS
				.iter()
				.position(|(old, _)| old == p)
				.map(|rank| (rank + 1, LEGACY_KEYS[rank].1.to_string()))
		};
		if let Some((rank, new)) = target {
			// a mapped path that has since been retired stays retired - the
			// old value is still in the .bak, but never resurrects here
			if !CONFIG_REMOVED.contains(&new.as_str()) {
				let slot = carry.entry(new).or_insert((rank, value.to_string()));
				if rank < slot.0 {
					*slot = (rank, value.to_string());
				}
			}
		} else if p.starts_with("themes.") {
			extras.push(format!("{p}: {value}"));
		}
		// anything else: dropped from the new file, still in the .bak
	}
	// Old semantics: no use_system_font line + an explicit font_family meant
	// "use that font" - keep meaning that, not the new template's default.
	if had_font_family && !had_use_system {
		carry
			.entry("font.use_system_family".to_string())
			.or_insert((usize::MAX, "false".to_string()));
	}

	if config_open_elsewhere(path) {
		note_config_busy(path);
		return;
	}
	let Some(backup) = backup_aside(path) else {
		return;
	};
	let mut out: Vec<String> = DEFAULT_CONFIG.lines().map(str::to_string).collect();
	for (new_path, (_, value)) in &carry {
		if !activate_line(&mut out, new_path, value) {
			// no template line (shouldn't happen) - keep it as a dotted line
			extras.push(format!("{new_path}: {value}"));
		}
	}
	if !extras.is_empty() {
		out.push(String::new());
		extras.sort();
		out.extend(extras);
	}
	let mut joined = out.join("\n");
	joined.push('\n');
	if let Err(e) = std::fs::write(path, joined) {
		eprintln!(
			"{APP_NAME}: could not convert config {}: {e}",
			path.display()
		);
		return;
	}
	eprintln!(
		"{APP_NAME}: config converted to the new nested layout; the old file is kept at {}",
		backup.display()
	);
}

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
// current line for that path. Only a bare, exactly-matching value migrates, so an
// edited value - or one trailing a note - stays as the user wrote it.
fn refresh_superseded_default(line: &str, path: &str) -> Option<String> {
	if !line.trim_start().starts_with('#') {
		return None; // active: the user's own value, leave it alone
	}
	// a path can carry several superseded values (a default retuned more than
	// once), so every entry for it is a candidate - not just the first
	let value = line_setting_value(line)?;
	if !SUPERSEDED_DEFAULTS
		.iter()
		.any(|(name, old)| *name == path && *old == value)
	{
		return None;
	}
	setting_lines(DEFAULT_CONFIG)
		.into_iter()
		.find(|(name, _)| name == path)
		.map(|(_, template)| template)
		.filter(|template| template != line)
}

// The rename/remove/refresh transform, as a pure fn (testable). Returns
// Some(new text) only if something changed.
fn migrate_config_text(text: &str) -> Option<String> {
	let lines: Vec<&str> = text.lines().collect();
	// full path per line index, for the lines that are settings
	let mut path_of: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
	for w in walk_settings(text) {
		if let WalkLine::Setting { index, path, .. } = w {
			path_of.insert(index, path);
		}
	}
	// rename targets already present (active or commented): don't create a dup
	let have: std::collections::HashSet<&str> = path_of.values().map(String::as_str).collect();

	let mut changed = false;
	let mut out: Vec<String> = Vec::new();
	for (index, line) in lines.iter().enumerate() {
		let Some(path) = path_of.get(&index) else {
			out.push((*line).to_string());
			continue;
		};
		if CONFIG_REMOVED.contains(&path.as_str()) {
			changed = true;
			continue; // drop
		}
		// A rename fires only while the new spelling is absent. Where both are
		// present the old line is left ALONE, never dropped: a rename can free
		// its old name for a NEW setting (colors.focus did exactly that), and
		// dropping there would delete that setting's own line on every launch.
		let renamed = CONFIG_RENAMES
			.iter()
			.find(|(old, _)| old == path)
			.filter(|(_, new)| !have.contains(*new));
		let mut kept = match renamed {
			Some((_, new)) => {
				changed = true;
				// the line spells the leaf (nested) or the full path
				// (dotted); rewrite whichever token is actually there
				let old_leaf = path.rsplit('.').next().unwrap_or(path);
				let new_leaf = new.rsplit('.').next().unwrap_or(new);
				let key = line_setting_key(line).unwrap_or(old_leaf);
				let target = if key == *path { new } else { new_leaf };
				line.replacen(key, target, 1)
			}
			None => (*line).to_string(),
		};
		// the refreshes below key on where the line ENDS UP, not where it came
		// from - a just-renamed line belongs to its new path now
		let path: &str = renamed.map_or(path.as_str(), |(_, new)| new);
		if let Some(refreshed) = refresh_superseded_default(&kept, path) {
			kept = refreshed;
			changed = true;
		}
		if path == "font.family" && !kept.trim_start().starts_with('#') {
			if let Some(refreshed) = refresh_font_stack(&kept) {
				kept = refreshed;
				changed = true;
			}
		}
		out.push(kept);
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
	let Some(mut doc) = read_doc(&path) else {
		return;
	};
	// A dotted key ("colors.foreground") is already a path, nested or not.
	for full_key in keys {
		doc.remove(full_key);
	}
	write_doc(&path, &doc);
	backfill_config(&path);
}

// Insert any settings the `DEFAULT_CONFIG` template defines that `path` lacks,
// using the template's own (commented or active) line so follow-system keys stay
// absent and behavior is unchanged. Existing values, comments, and formatting are
// preserved (nothing already in the file is rewritten). Every setting is one
// self-contained line - nested keys are written in dotted form - so there is no
// table header to insert under.
//
// A template group is one comment block plus the settings it introduces, and the
// block belongs to the group's FIRST setting. A group the file has never seen is
// appended whole, comments and all - but a group the file already has PART of
// already carries those comments, and re-appending them would duplicate the whole
// paragraph at the end of the file. Stragglers from a part-present group are put
// back beside the siblings that explain them instead.
fn backfill_config(path: &std::path::Path) {
	let Ok(text) = std::fs::read_to_string(path) else {
		return;
	};
	let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

	let mut groups: Vec<Vec<(String, Vec<String>)>> = Vec::new();
	for (p, block, new_group) in setting_groups(DEFAULT_CONFIG) {
		if new_group || groups.is_empty() {
			groups.push(Vec::new());
		}
		if let Some(group) = groups.last_mut() {
			group.push((p, block));
		}
	}
	// template order of every path, for sibling anchoring across group bounds
	let order: Vec<String> = groups.iter().flatten().map(|(p, _)| p.clone()).collect();

	let mut changed = false;
	for group in &groups {
		// fresh view after any earlier insertion
		let at = paths_at(&lines);
		let present = group.iter().filter(|(p, _)| at.contains_key(p)).count();
		if present == group.len() {
			continue;
		}
		if present == 0 {
			// wholly-new group: comments and all, in template position
			let block: Vec<String> = group.iter().flat_map(|(_, b)| b.iter().cloned()).collect();
			match anchor_for(&group[0].0, &order, &at, &lines, true) {
				Anchor::Before(index) => {
					// separate from the next group's comment block below
					lines.insert(index, String::new());
					for (offset, line) in block.into_iter().enumerate() {
						lines.insert(index + offset, line);
					}
				}
				Anchor::After(index) => {
					let mut insert = vec![String::new()];
					insert.extend(block);
					for (offset, line) in insert.into_iter().enumerate() {
						lines.insert(index + 1 + offset, line);
					}
				}
				Anchor::Append => {
					lines.push(String::new());
					lines.extend(block);
				}
			}
			changed = true;
			continue;
		}
		// part-present group: the comments are already in the file next to the
		// siblings, so re-appending them would duplicate the paragraph - put
		// each straggler back beside its siblings, line only, template order
		for (p, block) in group {
			let at = paths_at(&lines);
			if at.contains_key(p) {
				continue;
			}
			let Some(line) = block.last() else { continue };
			match anchor_for(p, &order, &at, &lines, false) {
				Anchor::Before(index) => lines.insert(index, line.clone()),
				Anchor::After(index) => lines.insert(index + 1, line.clone()),
				Anchor::Append => lines.push(line.clone()),
			}
			changed = true;
		}
	}
	if !changed {
		return;
	}

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

// Path -> line index for the current file lines.
fn paths_at(lines: &[String]) -> std::collections::HashMap<String, usize> {
	let text = lines.join("\n");
	walk_settings(&text)
		.into_iter()
		.filter_map(|w| match w {
			WalkLine::Setting { index, path, .. } => Some((path, index)),
			_ => None,
		})
		.collect()
}

enum Anchor {
	Before(usize), // insert at this line index
	After(usize),  // insert just after this line index
	Append,        // end of file
}

// Where a missing template path belongs in the file: before the next present
// template sibling (same parent), after the last present earlier sibling's
// subtree, after the parent block header, or appended at the end. Whole-group
// inserts back up over the next sibling's comment block so the new group lands
// above it rather than splitting the comments from their setting.
fn anchor_for(
	p: &str,
	order: &[String],
	at: &std::collections::HashMap<String, usize>,
	lines: &[String],
	whole_group: bool,
) -> Anchor {
	let parent = p.rsplit_once('.').map_or("", |(head, _)| head);
	let my_pos = order.iter().position(|o| o == p);
	let siblings: Vec<&String> = order
		.iter()
		.filter(|o| o.as_str() != p && o.rsplit_once('.').map_or("", |(head, _)| head) == parent)
		.collect();
	if let Some(my_pos) = my_pos {
		// next sibling in template order that the file has
		let next = siblings.iter().find(|s| {
			order
				.iter()
				.position(|o| o == **s)
				.is_some_and(|pos| pos > my_pos)
				&& at.contains_key(**s)
		});
		if let Some(next) = next {
			let mut index = at[*next];
			if whole_group {
				// land above the sibling's own comment block, not inside it
				while index > 0 && lines[index - 1].trim_start().starts_with('#') {
					index -= 1;
				}
			}
			return Anchor::Before(index);
		}
		// last earlier sibling present: insert after its whole subtree
		let prev = siblings.iter().rev().find(|s| {
			order
				.iter()
				.position(|o| o == **s)
				.is_some_and(|pos| pos < my_pos)
				&& at.contains_key(**s)
		});
		if let Some(prev) = prev {
			let prefix = format!("{prev}.");
			let end = at
				.iter()
				.filter(|(path, _)| *path == *prev || path.starts_with(&prefix))
				.map(|(_, index)| *index)
				.max()
				.unwrap_or(at[*prev]);
			return Anchor::After(end);
		}
	}
	if !parent.is_empty() {
		if let Some(index) = at.get(parent) {
			return Anchor::After(*index);
		}
	}
	Anchor::Append
}

// Set by `--config PATH` before any settings are read; overrides the default
// location for this process.
static CONFIG_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();
// Serializes the tests that install a config-path override. The override is
// process-global, so two of them running at once would each read the other's
// file - and they live in different modules, so the guard has to live here.
#[cfg(test)]
pub fn test_config_lock() -> std::sync::MutexGuard<'static, ()> {
	static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
	LOCK.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner)
}

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
## `key: value` is a commented-out (disabled) setting you can uncomment - it
## shows the built-in default. This file is yours to edit: your values and
## comments are kept. Saving may tidy layout (indentation, grouping), but never
## rewrites what you wrote. A malformed line is skipped on its own rather than
## sinking the whole file. On launch SilkTerm only adds options new to this
## version (and renames/removes ones that changed) - and even that is skipped
## if the file looks open elsewhere.

## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
## Font
## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

font:

	## System font face
	## Use the OS default monospace font FAMILY: put it at the head of the family
	## stack below. Turn off to start at family instead. Ignored where the OS has
	## no monospace setting to read (Windows has none).
	use_system_family: true

	## System font size
	## Use the OS default monospace font SIZE, overriding size below. Turn off to
	## size the font yourself. Ignored where the OS reports no size.
	# use_system_size: true  ## Default

	## Font family
	## A comma-separated fallback stack, first installed one wins. The same list
	## is consulted on every platform; use_system_family above only decides
	## whether the OS font is tried ahead of it. Anything not installed is
	## skipped, and a built-in stack backs the whole list up.
	family: "Monaspace Argon, Fira Code, JetBrains Mono, Cascadia Mono, Consolas, Ubuntu Mono, SF Mono, Menlo, Courier New"

	## Font size
	## In logical pixels. Used when use_system_size is off.
	## Range: 4.0 and up
	# size: 17.0  ## Default

	## Line height
	## As a multiple of the font's natural height.
	## Range: 0.5 and up - 1.0 is tight, higher is airier
	line_height_scale: 1.22

## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
## Window
## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

window:

	## Margin
	## Pixels between the text and the pane edge.
	margin: 8.0

	## Initial window size
	## In character cells. Used when remember_size is off.
	columns: 160
	rows: 48

	## Remember last size
	## Launch at the last window size instead of columns/rows. The remembered
	## size updates automatically whenever you resize the window (kept separate
	## from columns/rows so unchecking reverts to your defined size).
	# remember_size: true  ## Default
	# remembered_columns: 160  ## Default
	# remembered_rows: 48  ## Default

	## Hide single tab
	## Hide the tab bar while only one tab is open (also in the View menu).
	# hide_single_tab: false  ## Default

## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
## Background and transparency
## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

transparency:

	## Transparency
	## When on, the terminal background (only - never the text, window frame, or
	## menus) becomes see-through, using opacity below as its alpha. The code
	## picks the method (per-pixel via a GL surface on X11; native elsewhere).
	# enabled: true  ## Default

	## Background opacity
	## Only takes effect when enabled above is on.
	## Range: 0.0 to 1.0 - 0.0 is fully transparent, 1.0 is opaque
	opacity: 0.95

	## Blur-behind
	## Ask the compositor to blur the desktop showing through the translucent
	## background ("frosted glass"); text stays crisp. Only honored by KWin and
	## picom-with-blur; on Compiz/GNOME it does nothing (enable blur in the
	## compositor instead). The compositor controls the blur radius.
	# blur_behind: true  ## Default

wallpaper:

	## Wallpaper
	## Master switch for the wallpaper image.
	# enabled: true  ## Default

	## Wallpaper image
	## A single pinned image. Leave commented to auto-detect
	## wallpaper/wallpaper.{png,jpg,jpeg}. The value may be an absolute path or
	## a filename relative to this directory.
	# image: "wallpaper.png"  ## Default

	## Built-in fallback
	## Show a built-in wallpaper when none is configured (no image found above
	## and no rotation folder below).
	# fallback_builtin: true  ## Default

	## Rotation
	## Rotate the wallpaper through a folder of images (overrides image while
	## set). The folder is absolute or relative to this directory. Left
	## commented, a wallpaper/ dir here with images in it rotates on its own -
	## unless image above pins one, or one is given on the command line for that
	## run. Random picks avoid whatever came up recently, so runs feel varied
	## rather than repeating; set random false for plain filename order.
	## Interval 0 means only at launch.
	rotate:
		# enabled: true  ## Default
		# folder: "wallpaper/"  ## Default
		# interval_s: 0.0  ## Default
		# random: true  ## Default

	## Image visibility
	## How visible the image is against the background color (independent of the
	## transparency opacity above).
	## Range: 0.0 to 1.0 - 0.0 is all background color, 1.0 is all image
	# opacity: 0.10  ## Default

	## Default fit
	## How the image fits when it has nothing to say for itself: "stretch"
	## (fill, ignore aspect) or "zoom" (cover, keep aspect, crop the overhang).
	# default_fit: "stretch"  ## Default

	## Honor tags
	## Let a wallpaper carry its own layout in its XMP metadata, overriding the
	## default above per image. `wallpaper:Fit` is "stretch" or "zoom";
	## `wallpaper:Anchor` is "<horizontal>%, <vertical>%" (0% is left/top, 100%
	## is right/bottom) and picks which part of the image a zoom crop keeps. A
	## photo can then refuse to be squashed while a gradient still fills the
	## window.
	# honor_xmp: true  ## Default

	## Wallpaper blur
	## Gaussian blur applied to the wallpaper (sigma in pixels; 0 = none).
	## Range: 0.0 to 100.0
	# blur: 10.0  ## Default

	## Contrast mask
	## Flatten the wallpaper's contrast so it stops competing with text. size is
	## the flatten scale (1.0 = half the longest pixel dimension, so the whole
	## image collapses toward one tone; small = only fine detail flattens).
	## strength is how far each pixel is pulled toward that local mean. auto
	## blends the two manual knobs with values derived from the image's own
	## busyness (1.0 = full auto override, 0.0 = manual only, 0.5 = average).
	contrast_mask:
		# enabled: true  ## Default
		# size: 0.5  ## Default
		# strength: 0.5  ## Default
		# auto: 0.5  ## Default

## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
## Text
## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

text:

	## Text scrim
	## A blurry background-colored halo behind each glyph, so text stays legible
	## over a light/busy background or a near-transparent terminal. On by
	## default; uncomment enabled and set it false to disable.
	scrim:
		# enabled: true  ## Default
		## How much bolder to make the finished halo, as a percent: each 20% doubles
		## its opacity (100% = five doublings), so a faint halo turns into a solid
		## plate. 0 leaves it exactly as built.
		## Range: 0 to 100
		# strength: 15  ## Default
		## Halo radius in pixels.
		# radius: 5.0  ## Default
		## Range: 0.0 to 1.0 - 0.0 is hard/solid, 1.0 is soft/faint
		# softness: 0.5  ## Default
		## Halo shape: "sdf" (round, full corners), "dt", "dilate" (square), or
		## "gaussian" (legacy, corners recede).
		# function: "sdf"  ## Default
		## Halo falloff curve: "exp", "half_normal", "log", "sigmoid", or "linear".
		# ramp: "exp"  ## Default
		## Blur bold text at regular weight so its halo matches non-bold text.
		# regular_weight: true  ## Default

	## Text outline
	## Antialiased outline around glyphs, in pixels (0 = none).
	## Range: 0.0 to 8.0
	# outline: 1.0  ## Default

	## Color emoji
	## Paint color emoji (COLRv1); false renders them as monochrome outlines.
	# color_emoji: true  ## Default

	## Embolden inverse
	## Render reverse-video (dark-on-light) text bold so it reads as strongly as
	## normal text.
	# embolden_inverse: true  ## Default

## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
## Cursor
## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

cursor:

	## Cursor size
	## As a percent of the cell: height grows from the bottom, width from the
	## left. Together they make any shape: a block (100 / 100), a thin bar
	## (100 / 25), or an underline (15 / 100). Used when the app doesn't set its
	## own; alt-screen apps (vim, less) still control theirs.
	## Range: 1 to 100
	size:
		# height: 100  ## Default
		# width: 100  ## Default

	## Cursor animation
	## "none" (steady), "phase" (smooth fade), or a pulse that grows/shrinks
	## each cycle - "pulse_vertical", "pulse_horizontal", "pulse_both". The
	## cursor always slides smoothly as you type.
	# animation: "pulse_vertical"  ## Default

	## Animation resume delay
	## While you type, the animation glides to the cursor's full size and holds
	## there; it resumes this many seconds after input goes idle. Pausing and
	## resuming always happen at full size, so the cursor never jumps. This
	## delay is for typing only - a command's output holds the cursor still
	## while it writes, then hands it straight back when the prompt returns.
	## Refocusing the window, tab, or pane also resumes at once.
	## Range: 0.05 to 3600.0 (seconds)
	# animation_resume_s: 1  ## Default

	## Animation idle stop
	## After this many seconds with no input the animation stops entirely,
	## parked at full size, so an idle window costs nothing. Typing - or
	## refocusing the window, tab, or pane - brings it back. 0 = never stop.
	## Range: 0.0 to 86400.0 (seconds)
	# animation_idle_stop_s: 60  ## Default

	## Blink rate
	## Cursor animation cycle length, in milliseconds.
	## Range: 50.0 and up
	# blink_rate_ms: 500  ## Default

	## Cursor scrim
	## The cursor joins the scrim halo.
	# scrim: false  ## Default

	## Cursor outline
	## The cursor joins the text outline.
	# outline: true  ## Default

## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
## Selection
## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

selection:

	## Word separators
	## Delimiters that bound a double-click word selection. The default keeps
	## : / . - _ ~ as part of a word, so paths (incl. C:\ drive paths), URLs and
	## namespaced identifiers stay selected whole. Leave commented for the
	## default; set to your own string of separator characters to override (add
	## ':' back to split on it).
	# word_separators: ",|\"' ()[]{}<>"  ## Default

	## Selection pairs
	## Pairs whose contents a double-click selects when the click is inside a
	## matched pair (highest precedence first). Leave commented for the default.
	# pairs: "`` \"\" '' {} () [] <>"  ## Default

## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
## Shell
## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

shell:

	## Default shell
	## Shell/command for new windows, tabs, and panes when nothing else is given
	## (CLI --shell and per-pane inheritance take precedence). argv-split, so
	## "bash --norc" works. Leave blank/commented for the system default shell.
	# default: "bash --norc"  ## Default

	## Default command line
	## Applied when SilkTerm is launched with no arguments - the same
	## window/tab/pane options the CLI accepts (see --help). Any actual
	## command-line arguments override this entirely. Leave blank/commented for
	## none.
	# command_line: "--new-pane --right --size 35%"  ## Default

	## Copy on select
	## Start every pane with "Copy on select" enabled (selected text goes to the
	## clipboard). The menu-bar checkbox still toggles it live per pane.
	# copy_on_select: false  ## Default

## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
## Hyperlinks
## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

hyperlinks:

	## Hyperlinks
	## Underline a URL in the output while the pointer is over it; Ctrl+click
	## opens it, and a right-click there offers "Open link" and "Copy link".
	## Only these schemes are recognized, and nothing else can be opened:
	## http, https, ftp, ftps, sftp, ssh, file, mailto.
	# enabled: true  ## Default

	## Open command
	## Program that opens a clicked link, argv-split, with the URL appended as
	## its last argument. Leave blank/commented to use the desktop's own handler
	## (xdg-open, or start on Windows).
	# open_command: "firefox --new-tab"  ## Default

## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
## Scrolling
## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

scroll:

	## Scrollback
	## Lines of scrollback history kept per pane.
	scrollback: 10000

	## Smooth scrolling
	## Master switch for all scroll animation: eased wheel scrolling, eased
	## output scrolling, and the sliding of full-screen apps. Off = every
	## scroll lands instantly; the speed settings below then have no effect.
	# smooth: true  ## Default

	## The five settings below are the named segments of the output-scroll
	## speed curve, in the order one burst traverses them: Ease-in lifts the
	## speed from rest, Ramp-up accelerates it toward the top, Single-screen
	## speed is the top while the burst still fits on screen (unbounded once it
	## has scrolled off), Ramp-down winds the speed back down when output
	## ceases, and Ease-out lands the last fraction of a line. Each segment
	## hands its end point to the next and controls nothing else.

	## Ease-in
	## How long the view takes to build speed when it starts moving from rest:
	## the first few lines per second arrive over this many milliseconds
	## (wheel scrolling eases in over the same time). 82 ms is about 50 on the
	## 1..100 dialog scale, where higher means a crisper start.
	## Range: 1.0 and up (milliseconds) - higher is gentler
	ease_in_ms: 82.0

	## Ramp-up
	## How quickly a burst accelerates once past the ease-in: the catch-up
	## speed doubles every this many milliseconds until it reaches the top
	## speed that applies. Lower ramps harder, so a buffer dump is caught
	## sooner. 96 ms (about ten doublings a second) is around 75 on the
	## 1..100 dialog scale, where higher means a harder ramp.
	## Range: 1.0 and up (milliseconds) - lower ramps harder
	ramp_up_ms: 96.0

	## Single-screen speed
	## Top scrolling speed for an output burst whose own first line is still on
	## screen (a short directory listing, say): one line per this many
	## milliseconds. Once a burst has scrolled its first line off the top, the
	## ramp-up is unlimited and reaches whatever speed keeps up. 32 ms (~31
	## lines/s) is about 75 on the 1..100 dialog scale.
	## Range: 1.0 and up (milliseconds) - lower is faster
	single_screen_tau_ms: 32.0

	## Ramp-down
	## How gradually the speed winds down once output ceases with lines still
	## to render: the catch-up speed halves every this many milliseconds on the
	## way to the landing. The view keeps that wind-down's distance in reserve
	## behind a fast burst, so the stop is a descent, never a cliff. 144 ms is
	## around 75 on the 1..100 dialog scale, where higher means a harder stop.
	## Range: 1.0 and up (milliseconds) - lower stops harder
	ramp_down_ms: 144.0

	## Ease-out
	## How gradually the view settles onto its final line. The last fraction of
	## a line is given at least this long, so the tail sweeps in instead of
	## crawling to a halt. Higher is a softer, longer landing; lower is crisper.
	## 212 ms is about 40 on the 1..100 dialog scale, where higher means a
	## crisper landing.
	## Range: 1.0 and up (milliseconds) - higher is gentler
	ease_out_ms: 212.0

	## Wheel lines
	## Lines per wheel notch (smooth scrollback).
	wheel_lines: 3.0

	## Alt-screen wheel lines
	## Lines per wheel notch in full-screen apps (less, nano).
	alt_scroll_lines: 3.0

	## Output ease distance
	## How far new output slides in before easing to rest, in lines.
	output_ease_lines: 1.0

	## Smooth-scroll apps
	## Ease the whole-line jumps of apps that repaint a scrolling region instead
	## of growing scrollback: full-screen apps that own the screen (less, vim,
	## nano, htop, tmux, ...) and, on Windows, ConPTY-driven TUIs whose output
	## scrolls above a fixed input line. Their scrolling slides instead of
	## snapping; the revealed strip fills with the background during the
	## ~quarter-second slide. Only clean line-scrolls are eased (big page-jumps
	## still snap).
	# smooth_apps: true  ## Default

	## Scrollbar
	## A scrollbar over each pane's right edge, showing where the view sits in
	## the scrollback. It floats over the text rather than taking a column, so
	## turning it on or off never changes the grid. Full-screen apps (less, vim)
	## keep their own screen and get no scrollbar. Drag the thumb to scroll;
	## click the track to page.
	scrollbar:

		## Enabled
		# enabled: true  ## Default

		## Thickness
		## Width in pixels.
		## Range: 4 to 64
		# thickness: 16.0  ## Default

		## Auto-hide
		## Fade the scrollbar out while the view sits idle at the bottom, and back
		## in on scroll or when the pointer nears it. Off keeps it always visible.
		# auto_hide: true  ## Default

## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
## Theme and colors
## ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

## Color theme
## Pick a built-in (SilkTerm, Matrix, Retro Amber) or one you add in a themes.*
## entry. theme_mode is "dark", "light", or "system" (follow the OS).
theme: SilkTerm
theme_mode: dark

## Color overrides
## Per-color overrides on top of the theme (uncomment any to tweak one color).
## The menu_*/dialog_*/scrollbar_*/gutter keys recolor the chrome (menu bar +
## dropdowns, the pop-out Settings/About dialogs, the scrollbar, and the strip
## the dialog's tabs sit on); by default every theme shares the same neutral
## chrome. Menu hover/border shades derive from menu_background automatically.
## highlight marks several things at once (the live pane's ring, slider handles,
## revert arrows, the default button); focus marks only what the keyboard is on.
colors:
	# background: "#000000"  ## Default
	# foreground: "#88eecc"  ## Default
	# cursor: "#9649af"  ## Default
	# highlight: "#c8a05a"  ## Default
	# focus: "#4086ff"  ## Default
	# menu_background: "#36363b"  ## Default
	# menu_foreground: "#f0f0f2"  ## Default
	# dialog_background: "#20202a"  ## Default
	# dialog_foreground: "#e2e2ea"  ## Default
	# gutter: "#16161e"  ## Default
	# scrollbar_thumb: "#8a8a92"  ## Default
	# scrollbar_trough: "#2e2e36"  ## Default
"##;

#[cfg(test)]
mod tests {
	use super::*;

	// The whole point of the DIP pass: a chrome measurement is the same physical
	// size on any display, so it doubles when the scale factor does. The floor is
	// the other half - a hairline the author asked to be visible must never round
	// down to nothing, which is what a 1 DIP rule does on a display below 1x.
	#[test]
	fn a_chrome_measurement_scales_and_a_hairline_survives() {
		assert_eq!(dip(10.0, 1.0), 10.0);
		assert_eq!(dip(10.0, 2.0), 20.0);
		assert_eq!(dip(FOCUS_RING_PX, 2.0), FOCUS_RING_PX * 2.0);
		// fractional factors round to whole pixels so a rule stays crisp
		assert_eq!(dip(10.0, 1.5), 15.0);
		assert_eq!(dip(9.0, 1.25), 11.0);
		// a hairline holds at every factor, including down-scaled displays
		for scale in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
			assert!(
				dip(PANE_GAP_PX, scale) >= 1.0,
				"the pane gap vanished at {scale}x"
			);
		}
		// zero stays zero - the floor is for measurements meant to be seen
		assert_eq!(dip(0.0, 2.0), 0.0);
	}

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
		let _guard = super::test_config_lock();
		let _ = settings();
		let dir = std::env::temp_dir().join(format!("silkterm_cfgsave_{}", std::process::id()));
		let _ = std::fs::create_dir_all(&dir);
		let path = dir.join("config.shcl");
		std::fs::write(&path, "wallpaper.opacity: .1\ntext.scrim.ramp: \"s\"\n").unwrap();
		set_config_override(path.clone());

		let orig = load();
		assert_eq!(orig.text_scrim_ramp, "sigmoid"); // the file's older spelling
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
			saved.contains("opacity: .1"),
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
			"scroll.scrollback: 4242\nwindow.margin: not-a-number\ncolors.focus: \"#abcdef\"\n",
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

	// A key written twice cannot resolve to one value, so the default takes
	// effect. That is the right outcome, but it has to be SAID - the setting is
	// there in the file, plainly set, and doing nothing.
	#[test]
	fn a_setting_written_twice_falls_back_and_is_reported() {
		let p = std::path::Path::new("test.shcl");
		let s = resolve(read_raw(
			"font:\n\tfamily: \"One\"\n\tsize: 13.0\n\tfamily: \"Two\"\n",
			p,
		));
		// as good as absent: neither spelling wins
		let absent = resolve(read_raw("font:\n\tsize: 13.0\n", p));
		assert_eq!(
			s.font_family, absent.font_family,
			"a repeated key falls back as if it were not there"
		);
		assert_eq!(s.font_size, 13.0, "its siblings are unaffected");
		// the message is what makes the fallback discoverable; both lines cited
		assert_eq!(line_list(&[2, 4]), " lines 2, 4");
		assert_eq!(line_list(&[7]), " line 7");
		assert_eq!(line_list(&[0]), "", "an uncitable node adds nothing");
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
	// first save would reflow the file we just wrote. Nearly every setting here is
	// a commented default with no active sibling - the shape that shcl used to
	// re-pad to the block header's depth - so this is now the guard on the writer
	// itself, and a bump that reintroduced the reflow would fail here.
	#[test]
	fn default_config_survives_a_save_unchanged() {
		let doc = shcl::Document::parse(DEFAULT_CONFIG);
		assert_eq!(
			doc.to_canonical(),
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
		// 15% on the 20%-per-doubling scale, so a shade under one doubling
		assert_eq!(d.text_scrim_strength, 15.0);
		assert_eq!(d.text_outline, 1.0);
		assert_eq!(d.text_scrim_ramp, "exp");
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
	// defaults (sdf / exponential). The falloff's two renamed curves keep parsing
	// under their old spellings, so a config written before the rename still reads
	// as the same curve rather than silently falling back to the default.
	#[test]
	fn scrim_function_and_ramp_resolve() {
		let p = std::path::Path::new("test.shcl");
		for f in ["dilate", "sdf", "dt", "gaussian"] {
			let s = resolve(read_raw(&format!("text.scrim.function: \"{f}\"\n"), p));
			assert_eq!(s.text_scrim_function, f);
		}
		for r in ["sigmoid", "half_normal", "linear", "log", "exp"] {
			let s = resolve(read_raw(&format!("text.scrim.ramp: \"{r}\"\n"), p));
			assert_eq!(s.text_scrim_ramp, r);
		}
		for (old, new) in [("s", "sigmoid"), ("gaussian", "half_normal")] {
			let s = resolve(read_raw(&format!("text.scrim.ramp: \"{old}\"\n"), p));
			assert_eq!(s.text_scrim_ramp, new, "{old} should still parse");
		}
		let s = resolve(read_raw("text.scrim.function: \"bogus\"\n", p));
		assert_eq!(s.text_scrim_function, "sdf", "unknown -> default");
		let s = resolve(read_raw("text.scrim.ramp: \"bogus\"\n", p));
		assert_eq!(s.text_scrim_ramp, "exp", "unknown -> default");
	}

	// The face/size split's inference for configs predating use_system_font_size:
	// absent = follow the face toggle, except an explicit font_size (which the old
	// single toggle silently ignored) reads as intent and turns the size follow off.
	#[test]
	fn system_font_size_split_inference() {
		let p = std::path::Path::new("test.shcl");
		let s = resolve(read_raw("", p));
		assert!(s.use_system_font && s.use_system_font_size, "defaults on");
		let s = resolve(read_raw("font.use_system_family: false\n", p));
		assert!(!s.use_system_font_size, "size follows the face toggle");
		let s = resolve(read_raw("font.size: 20.0\n", p));
		assert!(s.use_system_font, "explicit size keeps the system face");
		assert!(
			!s.use_system_font_size,
			"explicit size wins over the OS size"
		);
		let s = resolve(read_raw("font.size: 20.0\nfont.use_system_size: true\n", p));
		assert!(s.use_system_font_size, "explicit key beats the inference");
	}

	#[test]
	fn copy_on_select_key_parses_and_defaults_off() {
		let p = std::path::Path::new("test.shcl");
		assert!(!resolve(read_raw("", p)).copy_on_select, "default off");
		assert!(resolve(read_raw("shell.copy_on_select: true\n", p)).copy_on_select);
	}

	#[test]
	fn hyperlink_keys_parse_in_their_block() {
		let p = std::path::Path::new("test.shcl");
		let d = resolve(read_raw("", p));
		assert!(d.hyperlinks, "on by default");
		assert!(
			d.hyperlink_open_command.is_empty(),
			"opener is the desktop's"
		);
		let s = resolve(read_raw(
			"hyperlinks:\n\tenabled: false\n\topen_command: \"firefox --new-tab\"\n",
			p,
		));
		assert!(!s.hyperlinks);
		assert_eq!(s.hyperlink_open_command, "firefox --new-tab");
	}

	// An over-range output_ease_lines must clamp: scroll's backlog clamp uses it
	// as a lower bound and panics (aborts, in release) when it exceeds the cap.
	#[test]
	fn output_ease_lines_clamps_to_backlog_cap() {
		let raw = read_raw(
			"scroll.output_ease_lines: 20.0\n",
			std::path::Path::new("test.shcl"),
		);
		let s = resolve(raw);
		assert!(s.output_ease_lines < 20.0, "over-range value must clamp");
		assert!(s.output_ease_lines <= crate::scroll::MAX_BACKLOG);
		let raw = read_raw(
			"scroll.output_ease_lines: -3.0\n",
			std::path::Path::new("test.shcl"),
		);
		assert!(resolve(raw).output_ease_lines >= 0.0);
	}

	// One syntax-broken line must not sink the valid settings around it.
	#[test]
	fn parse_lenient_drops_only_the_bad_line() {
		let text = "transparency.opacity: 0.7\ncursor_blink: enable\nwindow.margin: 12.0\n";
		let raw = read_raw(text, std::path::Path::new("test.shcl"));
		assert_eq!(raw.opacity, Some(0.7)); // before the bad line
		assert_eq!(raw.margin, Some(12.0)); // after the bad line
	}

	#[test]
	fn chrome_colors_default_and_override() {
		// theme provides the chrome; the default matches the shared menu colors
		let d = Settings::default();
		assert_eq!(d.menu_bg, crate::theme::MENU_BG_DEF);
		assert_eq!(d.menu_fg, crate::theme::MENU_FG_DEF);
		// a colors override wins; unspecified chrome stays at the theme default
		let raw = read_raw(
			"colors.menu_background: \"#123456\"\ncolors.dialog_foreground: \"#abcdef\"\n",
			std::path::Path::new("test.shcl"),
		);
		let s = resolve(raw);
		assert_eq!(s.menu_bg, [0x12, 0x34, 0x56]);
		assert_eq!(s.dialog_fg, [0xab, 0xcd, 0xef]);
		assert_eq!(s.menu_fg, crate::theme::MENU_FG_DEF);
	}

	// A pre-nesting config converts wholesale: every ACTIVE value lands at its
	// new nested path (oldest alias spellings included), obsolete keys drop,
	// themes.* user data survives, and the original file is kept as a .bak.
	// When both an old alias and its newer flat spelling are present, the newer
	// one wins.
	#[test]
	fn legacy_config_converts_with_values_carried() {
		let dir = std::env::temp_dir().join(format!("silkterm_convert_{}", std::process::id()));
		let _ = std::fs::create_dir_all(&dir);
		let path = dir.join("config.shcl");
		let legacy = "## my own note\n\
			scrollback: 5000\n\
			cursor_size_vertical: 40\n\
			cursor_shape: \"block\"\n\
			background_fit: \"zoom\"\n\
			wallpaper_opacity: 0.4\n\
			background_opacity: 0.9\n\
			opacity: 0.8\n\
			scroll_tau_ms: 120.0\n\
			wheel_lines: 4  ## an old trailing note\n\
			# margin: 8.0\n\
			font_family: \"Iosevka\"\n\
			themes.mine.dark.background: \"#010203\"\n\
			colors.focus: \"#abcdef\"\n\
			theme: Matrix\n";
		std::fs::write(&path, legacy).unwrap();
		convert_legacy_config(&path);
		let out = std::fs::read_to_string(&path).unwrap();

		assert!(out.contains("\tscrollback: 5000"), "value carried:\n{out}");
		assert!(
			out.contains("\t\theight: 40"),
			"oldest alias lands at the nested path:\n{out}"
		);
		assert!(!out.contains("cursor_shape"), "obsolete key dropped");
		assert!(
			out.contains("\tdefault_fit: \"zoom\""),
			"background_* alias carried:\n{out}"
		);
		assert!(
			out.contains("\topacity: 0.4") && !out.contains("\topacity: 0.9"),
			"the newer flat spelling wins over its alias:\n{out}"
		);
		assert!(
			out.contains("\topacity: 0.8"),
			"old bare opacity is the transparency one:\n{out}"
		);
		assert!(
			!out.contains("tau_ms: 120.0"),
			"a since-retired setting must not resurrect through conversion:\n{out}"
		);
		assert!(
			out.contains("\twheel_lines: 4\n"),
			"value carried without its stale trailing note:\n{out}"
		);
		assert!(
			out.contains("\tmargin: 8.0"),
			"a commented old line just leaves the fresh default in place:\n{out}"
		);
		assert!(
			out.contains("\tfamily: \"Iosevka\"") && out.contains("\tuse_system_family: false"),
			"an explicit font pins the system toggle off, as it always meant:\n{out}"
		);
		assert!(
			out.contains("themes.mine.dark.background: \"#010203\""),
			"future-feature user data carried:\n{out}"
		);
		assert!(
			out.contains("\tfocus: \"#abcdef\""),
			"color override carried"
		);
		assert!(out.contains("theme: Matrix"), "theme choice carried");
		assert!(
			!out.contains("## my own note"),
			"old comments live in the .bak, not the fresh template"
		);
		let bak = std::fs::read_to_string(dir.join("config.shcl.bak")).unwrap();
		assert_eq!(bak, legacy, "the original file survives untouched as .bak");

		// the converted file is current-format: converting again is a no-op
		convert_legacy_config(&path);
		assert_eq!(out, std::fs::read_to_string(&path).unwrap());
		let _ = std::fs::remove_file(&path);
		let _ = std::fs::remove_file(dir.join("config.shcl.bak"));
	}

	// The shipped template itself must never read as legacy.
	#[test]
	fn a_new_format_config_never_converts() {
		let dir = std::env::temp_dir().join(format!("silkterm_noconvert_{}", std::process::id()));
		let _ = std::fs::create_dir_all(&dir);
		let path = dir.join("config.shcl");
		std::fs::write(&path, DEFAULT_CONFIG).unwrap();
		convert_legacy_config(&path);
		assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
		assert!(
			!dir.join("config.shcl.bak").exists(),
			"no backup for a current-format file"
		);
		let _ = std::fs::remove_file(&path);
	}

	// A hand-edited config is where a home-relative path gets typed, so `~` has
	// to expand. `~user` has nothing to resolve against and stays literal.
	#[test]
	fn tilde_expands_to_home_but_only_for_this_user() {
		// Resolve home the same way expand_tilde does - Windows has no HOME, only USERPROFILE.
		let home = std::env::var_os("HOME")
			.or_else(|| std::env::var_os("USERPROFILE"))
			.expect("HOME or USERPROFILE");
		assert_eq!(expand_tilde("~/pics"), PathBuf::from(&home).join("pics"));
		assert_eq!(expand_tilde("~"), PathBuf::from(&home));
		for literal in ["~someone/pics", "/abs/pics", "rel/pics", "wallpaper/"] {
			assert_eq!(
				expand_tilde(literal),
				PathBuf::from(literal),
				"{literal} should stay literal"
			);
		}
	}

	// The first path-level rename since the config went nested. A rename rewrites
	// the key on its own line and must keep everything else about it: the value,
	// the active/commented state, and the indentation that says which block it
	// belongs to. Renames may not cross blocks - the machinery rewrites lines, it
	// does not move them - so this one stays inside `scroll:`.
	#[test]
	fn a_renamed_setting_keeps_its_value_and_its_block() {
		let out = migrate_config_text("scroll:\n\tinview_tau_ms: 45.0\n").expect("should migrate");
		assert_eq!(out, "scroll:\n\tsingle_screen_tau_ms: 45.0\n");
		// the value has to survive the trip through the loader, not just the text
		let s = resolve(read_raw(&out, std::path::Path::new("test.shcl")));
		assert!((s.scroll_single_screen_tau_ms - 45.0).abs() < f32::EPSILON);
		// the dotted spelling reads and rewrites the same way
		assert_eq!(
			migrate_config_text("scroll.inview_tau_ms: 45.0\n").as_deref(),
			Some("scroll.single_screen_tau_ms: 45.0\n")
		);
		// a commented line is renamed too, so the file keeps documenting itself
		assert_eq!(
			migrate_config_text("scroll:\n\t# inview_tau_ms: 60.0  ## Default\n").as_deref(),
			Some("scroll:\n\t# single_screen_tau_ms: 60.0  ## Default\n")
		);
		// and a config that already carries the new spelling is left alone
		assert!(migrate_config_text("scroll:\n\tsingle_screen_tau_ms: 45.0\n").is_none());
	}

	// A rename can hand its old name to a NEW setting - `colors.focus` became
	// `colors.highlight` and the freed name now holds the vivid focus color.
	// Once both spellings are in the file the old line must be left exactly
	// where it is: it is no longer stale, it is the new setting's own line, and
	// dropping or re-renaming it would delete a user's color on every launch.
	#[test]
	fn a_renamed_key_frees_its_old_name_for_a_new_setting() {
		// first launch: the one color there is becomes the calm one
		let once = migrate_config_text("colors:\n\tfocus: \"#abcdef\"\n").expect("should migrate");
		assert_eq!(once, "colors:\n\thighlight: \"#abcdef\"\n");
		let s = resolve(read_raw(&once, std::path::Path::new("test.shcl")));
		assert_eq!(s.highlight, [0xab, 0xcd, 0xef]);
		assert_eq!(
			s.focus,
			Settings::default().focus,
			"the new one starts fresh"
		);

		// after backfill both spellings are present, and every launch after that
		// is a no-op - the file has reached its resting state
		let both = "colors:\n\thighlight: \"#abcdef\"\n\tfocus: \"#123456\"\n";
		assert!(migrate_config_text(both).is_none());
		let s = resolve(read_raw(both, std::path::Path::new("test.shcl")));
		assert_eq!(s.highlight, [0xab, 0xcd, 0xef]);
		assert_eq!(s.focus, [0x12, 0x34, 0x56]);

		// a stale line carrying the pre-theme default still refreshes, under the
		// path it lands on rather than the one it was written under
		let stale = migrate_config_text("colors:\n\t# focus: \"#5580c8\"  ## Default\n")
			.expect("should refresh");
		assert!(
			stale.contains("# highlight: \"#c8a05a\"  ## Default"),
			"got {stale}"
		);
	}

	// The retired speed knobs leave existing configs entirely: tau_ms has no
	// successor, and ease_in changed units (fraction -> milliseconds), so its
	// old value must not be carried into the new key. Active and stale
	// commented lines both go; the settings around them stay put.
	#[test]
	fn retired_scroll_knobs_are_removed_not_carried() {
		let out = migrate_config_text(
			"scroll:\n\ttau_ms: 120.0\n\tease_in: 0.5\n\tramp_up_ms: 200.0\n\t# tau_ms: 230.0  ## Default\n",
		)
		.expect("should migrate");
		assert_eq!(out, "scroll:\n\tramp_up_ms: 200.0\n");
		let s = resolve(read_raw(&out, std::path::Path::new("test.shcl")));
		assert!((s.scroll_ramp_up_ms - 200.0).abs() < f32::EPSILON);
		// the old fraction never leaks into the new duration
		assert!((s.scroll_ease_in_ms - Settings::default().scroll_ease_in_ms).abs() < f32::EPSILON);
	}

	// A config with nothing to migrate is left untouched (no needless rewrite).
	#[test]
	fn migrate_config_noop_when_current() {
		assert!(
			migrate_config_text("transparency.opacity: 0.7\ncursor.animation: \"phase\"\n")
				.is_none()
		);
		assert!(migrate_config_text(DEFAULT_CONFIG).is_none());
	}

	// Backfill only ever adds a missing key, so a config written when an older
	// stack was the default kept that stack forever. Migration refreshes exactly
	// the shipped defaults and nothing the user chose themselves - now keyed on
	// the nested font.family path.
	#[test]
	fn migrate_refreshes_a_superseded_default_font_stack() {
		let stale = SUPERSEDED_FONT_STACKS[0];
		let out = migrate_config_text(&format!(
			"font:\n\tuse_system_family: true\n\tfamily: \"{stale}\"\n"
		))
		.expect("stale default should be refreshed");
		assert!(
			out.contains(&format!("\tfamily: \"{DEFAULT_FONT_STACK}\"")),
			"{out:?}"
		);
		assert!(!out.contains(stale));

		// the current value is already right, so nothing to do
		let current = format!("font:\n\tfamily: \"{DEFAULT_FONT_STACK}\"\n");
		assert!(migrate_config_text(&current).is_none());
		// a stack the user edited, or one they commented out, is theirs - leave it
		let edited = format!("font:\n\tfamily: \"Iosevka, {stale}\"\n");
		assert!(migrate_config_text(&edited).is_none());
		assert!(migrate_config_text(&format!("font:\n\t# family: \"{stale}\"\n")).is_none());
		// a top-level dotted spelling refreshes too
		assert!(migrate_config_text(&format!("font.family: \"{stale}\"\n")).is_some());
	}

	// A commented line still echoing an outgoing default is brought up to the
	// template's current one; an active line, or one the user annotated, is theirs.
	// Every entry refreshes, including a second one for a path whose default has
	// been retuned twice - the lookup must not stop at the first match.
	#[test]
	fn migrate_refreshes_a_superseded_commented_default() {
		// the path's blocks, one indent level each, with the leaf last
		let nest = |path: &str, line: &str| {
			let parts: Vec<&str> = path.split('.').collect();
			let leaf_depth = parts.len() - 1;
			let mut out: Vec<String> = parts[..leaf_depth]
				.iter()
				.enumerate()
				.map(|(depth, block)| "\t".repeat(depth) + block + ":")
				.collect();
			out.push("\t".repeat(leaf_depth) + line);
			out.join("\n") + "\n"
		};
		for (path, stale) in SUPERSEDED_DEFAULTS {
			let leaf = path.rsplit('.').next().unwrap();
			let current = setting_lines(DEFAULT_CONFIG)
				.into_iter()
				.find_map(|(name, line)| (name == *path).then_some(line))
				.unwrap_or_else(|| panic!("{path} has no template line"));
			let out = migrate_config_text(&nest(path, &format!("# {leaf}: {stale}")))
				.unwrap_or_else(|| panic!("{path}: stale default should be refreshed"));
			assert!(out.lines().any(|l| l == current), "{path}: {out:?}");
			// their own choice, either way they made it
			assert!(migrate_config_text(&nest(path, &format!("{leaf}: {stale}"))).is_none());
			let noted = nest(path, &format!("# {leaf}: {stale}  ## mine"));
			assert!(migrate_config_text(&noted).is_none(), "{path}");
		}
	}

	// The walker is what gives every line its full nested path - the whole
	// line-oriented machinery keys on it.
	#[test]
	fn walker_resolves_nested_paths() {
		let text = "top: 1\nwallpaper:\n\t# enabled: true\n\trotate:\n\t\t# folder: \"x\"\n\t\tinterval_s: 2.0\n\t# opacity: 0.5\ncolors.focus: \"#123456\"\n";
		let got: Vec<(String, bool)> = walk_settings(text)
			.into_iter()
			.filter_map(|w| match w {
				WalkLine::Setting { path, active, .. } => Some((path, active)),
				_ => None,
			})
			.collect();
		let want = [
			("top", true),
			("wallpaper", true),
			("wallpaper.enabled", false),
			("wallpaper.rotate", true),
			("wallpaper.rotate.folder", false),
			("wallpaper.rotate.interval_s", true),
			("wallpaper.opacity", false),
			("colors.focus", true),
		];
		let want: Vec<(String, bool)> = want.iter().map(|(p, a)| ((*p).to_string(), *a)).collect();
		assert_eq!(got, want);
	}

	// A dialog save on a fresh nested config: the new active value lands inside
	// its block, and everything else in the file is left exactly as it stands.
	// The whole-file diff is the strong form of that - a save may only ever add
	// the lines it was asked to add - and the spot checks below say WHICH shapes
	// are being relied on, so a failure names the one that moved.
	#[test]
	fn a_save_keeps_nested_comment_layout() {
		let mut doc = shcl::Document::parse(DEFAULT_CONFIG);
		doc.set_float("wallpaper.opacity", 0.5);
		doc.set_bool("text.scrim.enabled", false);
		let out = doc.to_canonical();

		let added: Vec<&str> = out
			.lines()
			.filter(|l| !DEFAULT_CONFIG.lines().any(|d| d == *l))
			.collect();
		assert_eq!(
			added,
			vec!["\topacity: 0.5", "\t\tenabled: false"],
			"a save changed lines it was not asked to:\n{out}"
		);

		assert!(
			out.contains("\topacity: 0.5"),
			"new value lands in the wallpaper block:\n{out}"
		);
		assert!(
			out.contains("\t\tenabled: false"),
			"new value lands in the scrim block:\n{out}"
		);
		assert!(
			out.contains("\t\t# random: true  ## Default"),
			"commented defaults keep their nesting depth:\n{out}"
		);
		assert!(
			out.contains("\t# blur: 10.0  ## Default"),
			"comment runs after the change point keep depth too:\n{out}"
		);
		assert!(
			out.contains("\n\n## •"),
			"blank lines before section rules survive:\n{out}"
		);
		assert!(
			out.contains("\t# dialog_foreground: \"#e2e2ea\"  ## Default"),
			"the trailing colors block keeps its indentation:\n{out}"
		);
	}

	// A new key added to a group the file already has PART of must land beside
	// its siblings INSIDE their block, at the right depth, NOT be appended with
	// a second copy of the group's comment block - that paragraph is already in
	// the file, attached to the siblings.
	#[test]
	fn a_straggler_lands_beside_its_siblings_not_with_a_second_paragraph() {
		let path = std::env::temp_dir().join("silkterm_backfill_straggler_test.shcl");
		// interval_s is missing from an otherwise-present rotation block
		let drifted = "wallpaper:\n\
			\n\
			\t## Rotation\n\
			\t## Rotate the wallpaper through a folder of images.\n\
			\trotate:\n\
			\t\t# enabled: true  ## Default\n\
			\t\t# folder: \"wallpaper/\"  ## Default\n\
			\t\t# random: true  ## Default\n";
		std::fs::write(&path, drifted).unwrap();
		backfill_config(&path);
		let out = std::fs::read_to_string(&path).unwrap();

		let paragraphs = out.matches("Rotate the wallpaper through a folder").count();
		assert_eq!(paragraphs, 1, "comment block duplicated:\n{out}");
		let straggler = out
			.find("\t\t# interval_s: 0.0")
			.expect("straggler backfilled at depth");
		let folder = out.find("\t\t# folder:").expect("sibling still there");
		let random = out.find("\t\t# random:").expect("sibling still there");
		assert!(
			folder < straggler && straggler < random,
			"template order among siblings not kept:\n{out}"
		);
		// a group the file has never seen still arrives whole, comments and all
		assert!(
			out.contains("## Master switch for the wallpaper image.")
				&& out.contains("# enabled: true  ## Default"),
			"new group needs its comments:\n{out}"
		);
		// and a wholly-missing top-level section arrives as a block
		assert!(
			out.contains("font:") && out.contains("\tuse_system_family: true"),
			"missing sections backfilled whole:\n{out}"
		);

		backfill_config(&path);
		assert_eq!(
			out,
			std::fs::read_to_string(&path).unwrap(),
			"backfill not idempotent"
		);
		let _ = std::fs::remove_file(&path);
	}

	// The real on-disk load pipeline (convert -> migrate -> backfill) on a
	// pre-nesting config: values land at their nested paths, the original file
	// is kept as .bak, missing keys arrive, and the chain is stable.
	#[test]
	fn pipeline_convert_migrate_backfill_on_disk() {
		let dir = std::env::temp_dir().join(format!("silkterm_pipeline_{}", std::process::id()));
		let _ = std::fs::create_dir_all(&dir);
		let path = dir.join("config.shcl");
		let drifted = "scrollback: 5000\n\
			cursor_size_vertical: 40\n\
			cursor_shape: \"block\"\n\
			margin: 12.0\n\
			opacity: 0.8\n\
			\n\
			themes.mine.dark.background: \"#010203\"\n\
			\n\
			colors.focus: \"#abcdef\"\n";
		std::fs::write(&path, drifted).unwrap();
		convert_legacy_config(&path);
		migrate_config(&path);
		backfill_config(&path);
		let out = std::fs::read_to_string(&path).unwrap();

		assert!(
			!out.contains("cursor_shape"),
			"obsolete key dropped:\n{out}"
		);
		assert!(
			out.contains("\t\theight: 40"),
			"renamed key kept its value:\n{out}"
		);
		assert!(
			out.contains("\tmargin: 12.0") && out.contains("\topacity: 0.8"),
			"values landed nested:\n{out}"
		);
		assert!(out.contains("\tscrollback: 5000"), "scrollback value kept");
		assert!(out.contains("\tfocus: \"#abcdef\""), "color override kept");
		assert!(
			out.contains("themes.mine.dark.background"),
			"unknown key kept"
		);
		assert!(
			out.contains("\tuse_system_family: true"),
			"missing key present with its default"
		);

		// stable: a second pass changes nothing.
		convert_legacy_config(&path);
		migrate_config(&path);
		backfill_config(&path);
		assert_eq!(
			out,
			std::fs::read_to_string(&path).unwrap(),
			"pipeline not idempotent"
		);
		let _ = std::fs::remove_file(&path);
		let _ = std::fs::remove_file(dir.join("config.shcl.bak"));
	}

	// The scan must not offer the loader a file it can't decode: `image` is built
	// with only png + jpeg, so a wider list picks a wallpaper that then fails.
	#[test]
	fn image_scan_only_accepts_what_the_decoder_has() {
		use std::path::Path;
		for ok in ["a.png", "a.jpg", "a.jpeg", "a.JPG", "a.PNG"] {
			assert!(is_image_file(Path::new(ok)), "{ok} should be scanned");
		}
		for no in ["a.webp", "a.bmp", "a.gif", "a.tiff", "a.tif", "a.txt", "a"] {
			assert!(
				!is_image_file(Path::new(no)),
				"{no} has no decoder - scanning it picks an unloadable wallpaper"
			);
		}
	}
}
