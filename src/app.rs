// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::index::Side;
use alacritty_terminal::selection::SelectionType;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Fullscreen, Window, WindowId};

use alacritty_terminal::term::TermMode;
use glyphon::{Buffer, Color as GColor, Shaping, TextArea, TextBounds};

use crate::bgimage::ImageRenderer;
use crate::clipboard::Clipboard;
use crate::config;
use crate::gfx::{Gfx, RectInstance, RectRenderer};
use crate::input;
use crate::pane::{Dir, PaneManager, Rect};
use crate::term::{PaneId, UserEvent};
use crate::text::TextCtx;

pub struct App {
	proxy: EventLoopProxy<UserEvent>,
	state: Option<State>,
	cli: crate::cli::Cli,
	// pop-out dialog window (About/Settings), if open. Its own surface + text
	// context, so it can be larger than the main window.
	dialog: Option<crate::dialog::DialogWin>,
	dialog_dirty: bool,
	// cicd profiler stage: when SILK_PROFILE_OUT is set the app runs a workload
	// (via --shell) for SILK_PROFILE_SECS then exits, so main can dump a flamegraph.
	#[cfg(feature = "profiling")]
	profile_secs: u64,
	#[cfg(feature = "profiling")]
	profile_deadline: Option<std::time::Instant>,
}

impl App {
	pub fn new(proxy: EventLoopProxy<UserEvent>, cli: crate::cli::Cli) -> Self {
		Self {
			proxy,
			state: None,
			cli,
			dialog: None,
			dialog_dirty: false,
			#[cfg(feature = "profiling")]
			profile_secs: std::env::var("SILK_PROFILE_SECS")
				.ok()
				.and_then(|s| s.parse().ok())
				.unwrap_or(8),
			#[cfg(feature = "profiling")]
			profile_deadline: None,
		}
	}

	// Events for the pop-out dialog window (its own surface/input).
	fn handle_dialog_event(&mut self, event: WindowEvent) {
		use crate::dialog::DialogAction as DA;
		let mut act: Option<DA> = None;
		match event {
			WindowEvent::CloseRequested => {
				self.dialog = None;
				return;
			}
			WindowEvent::Resized(sz) => {
				if let Some(d) = &mut self.dialog {
					d.resize(sz.width, sz.height);
				}
				self.dialog_dirty = true;
			}
			WindowEvent::RedrawRequested => {
				if let Some(d) = &mut self.dialog {
					d.render();
				}
			}
			WindowEvent::CursorMoved { position, .. } => {
				if let Some(d) = &mut self.dialog {
					d.set_cursor(position.x as f32, position.y as f32);
					self.dialog_dirty = true; // slider drag feedback
				}
			}
			WindowEvent::MouseInput {
				state,
				button: MouseButton::Left,
				..
			} => {
				if let Some(d) = &mut self.dialog {
					match state {
						ElementState::Pressed => act = d.mouse_down(),
						ElementState::Released => d.mouse_up(),
					}
					self.dialog_dirty = true;
				}
			}
			WindowEvent::KeyboardInput { event: ke, .. } if ke.state == ElementState::Pressed => {
				if let Some(d) = &mut self.dialog {
					match &ke.logical_key {
						Key::Named(NamedKey::Escape) => act = d.key_escape(),
						Key::Named(NamedKey::Enter) => act = d.key_enter(),
						Key::Named(NamedKey::Backspace) => d.backspace(),
						Key::Named(NamedKey::Space) => d.char_input(' '),
						Key::Character(s) => {
							for c in s.chars() {
								d.char_input(c);
							}
						}
						_ => {}
					}
					self.dialog_dirty = true;
				}
			}
			_ => {}
		}
		if let Some(a) = act {
			self.apply_dialog_action(a);
		}
	}

	fn apply_dialog_action(&mut self, action: crate::dialog::DialogAction) {
		use crate::dialog::DialogAction as DA;
		match action {
			DA::OpenUrl(u) => open_url(&u),
			DA::Close => self.dialog = None,
			DA::Apply => self.apply_dialog_settings(),
			DA::ApplyAndClose => {
				self.apply_dialog_settings();
				self.dialog = None;
			}
		}
	}

	// Pull the edited Settings from the dialog window and live-apply them to the
	// main window (config + persist + rebuild). The dialog has its own surface,
	// so it's unaffected.
	fn apply_dialog_settings(&mut self) {
		if let Some((orig, edited, sys)) = self.dialog.as_ref().and_then(|d| d.settings_values()) {
			if let Some(state) = self.state.as_mut() {
				state.apply_settings_values(&orig, edited, sys);
			}
			// The applied values are the new baseline, so a later Apply diffs against
			// the live state (without this, re-selecting the open-time value - e.g.
			// Bg fit back to Stretch - reads as "no change" and isn't re-applied).
			if let Some(d) = self.dialog.as_mut() {
				d.commit_baseline();
			}
			self.dialog_dirty = true;
		}
	}
}

#[derive(Clone, Copy)]
enum MenuAction {
	Copy,
	Paste,
	PasteSelection,
	ToggleReadOnly,
	NewTab,
	CloseTab,
	NextTab,
	PrevTab,
	SplitVertical,
	SplitHorizontal,
	Close,
	ToggleFullscreen,
	ToggleFrame,
	ToggleMenuBar,
	ReloadConfig,
	Settings,
	About,
	Quit,
}

// One row of a menu: an action item (optionally a checkmark toggle) or a group
// separator. Separators render as a faint horizontal line, never hover/click.
enum Entry {
	Item {
		label: String,
		action: MenuAction,
		check: Option<bool>,
	},
	Sep,
}

fn mi(label: &str, action: MenuAction) -> Entry {
	Entry::Item {
		label: label.into(),
		action,
		check: None,
	}
}
fn mt(on: bool, label: &str, action: MenuAction) -> Entry {
	Entry::Item {
		label: label.into(),
		action,
		check: Some(on),
	}
}

// right-click context menu / menu-bar dropdown over a pane
struct ContextMenu {
	x: f32,
	y: f32,
	w: f32,
	item_h: f32,
	target: PaneId,
	entries: Vec<Entry>,
	hover: Option<usize>, // index into entries; only ever an Item
}

impl ContextMenu {
	fn height(&self) -> f32 {
		let rows: f32 = self.entries.iter().map(|e| self.entry_h(e)).sum();
		rows + config::MENU_ITEM_PAD_Y * 2.0
	}
	fn entry_h(&self, e: &Entry) -> f32 {
		match e {
			Entry::Sep => config::MENU_SEP_H,
			Entry::Item { .. } => self.item_h,
		}
	}
	fn row_top(&self, i: usize) -> f32 {
		self.y
			+ config::MENU_ITEM_PAD_Y
			+ self.entries[..i]
				.iter()
				.map(|e| self.entry_h(e))
				.sum::<f32>()
	}
	fn item_at(&self, mx: f32, my: f32) -> Option<usize> {
		if mx < self.x || mx >= self.x + self.w {
			return None;
		}
		let mut y = self.y + config::MENU_ITEM_PAD_Y;
		for (i, e) in self.entries.iter().enumerate() {
			let h = self.entry_h(e);
			if my >= y && my < y + h {
				return matches!(e, Entry::Item { .. }).then_some(i);
			}
			y += h;
		}
		None
	}
	// Next selectable item from `from` in direction `dir` (+1 down / -1 up),
	// wrapping and skipping separators. None only if there are no items.
	fn step(&self, from: Option<usize>, dir: i32) -> Option<usize> {
		let n = self.entries.len() as i32;
		if n == 0 {
			return None;
		}
		let mut i = from
			.map(|i| i as i32)
			.unwrap_or(if dir > 0 { -1 } else { 0 });
		for _ in 0..n {
			i = (i + dir).rem_euclid(n);
			if matches!(self.entries[i as usize], Entry::Item { .. }) {
				return Some(i as usize);
			}
		}
		None
	}
}

// Tab strip: each tab owns its own pane split-tree. Detach/dock to other
// windows is deferred (needs multi-window support).
struct Tabs {
	list: Vec<PaneManager>,
	active: usize,
}

impl Tabs {
	fn cur(&self) -> &PaneManager {
		&self.list[self.active]
	}
	fn cur_mut(&mut self) -> &mut PaneManager {
		&mut self.list[self.active]
	}
	fn len(&self) -> usize {
		self.list.len()
	}
	fn next(&mut self) {
		let n = self.list.len();
		self.active = (self.active + 1) % n;
	}
	fn prev(&mut self) {
		let n = self.list.len();
		self.active = (self.active + n - 1) % n;
	}
	// swap the active tab with its neighbour and follow it
	fn move_active(&mut self, fwd: bool) {
		let n = self.list.len();
		if n < 2 {
			return;
		}
		let j = if fwd {
			(self.active + 1) % n
		} else {
			(self.active + n - 1) % n
		};
		self.list.swap(self.active, j);
		self.active = j;
	}
}

// Menu/tab bars auto-size to the menu (proportional) font: height = the text line
// height (cell_h) + this vertical padding, so a larger font isn't clipped (#124).
const MENU_BAR_VPAD: f32 = 6.0;
const TAB_BAR_VPAD: f32 = 11.0; // enough that tab-title descenders (g/j/p/q/y) clear the button bottom
const BELL_TAU_S: f32 = 0.18; // visual-bell flash fade time-constant (~0.8s to settle)
const MENU_BAR_PAD: f32 = 10.0; // px around each top-level title
const MENU_BAR: [&str; 6] = ["File", "Edit", "View", "Tabs", "Panes", "Help"];

struct State {
	window: Arc<Window>,
	gfx: Gfx,
	text: TextCtx,
	rects: RectRenderer,
	bg_image: Option<ImageRenderer>,
	glow: crate::glow::Glow, // text readability glow (used only when config.text_glow)
	tabs: Tabs,
	mods: ModifiersState,
	mouse: (f32, f32),
	selecting: Option<PaneId>, // pane with an in-progress drag-select
	last_click: Option<(Instant, f32, f32)>, // for double-click detection
	resizing: Option<Vec<bool>>, // split-tree path of the divider being dragged
	dragging_pane: Option<PaneId>, // pane being drag-reordered (Shift+drag)
	cursor_icon: CursorIcon,
	clipboard: Clipboard,
	last_frame: Instant,
	dirty: bool,
	bell_flash: f32,    // visual-bell brightness, set to 1.0 on BEL, decays to 0
	overwrite: bool, // false = Insert (bar cursor, default), true = Overwrite (block); Insert key toggles
	size_tracked: bool, // false until the first frame, so startup/programmatic resizes don't overwrite remembered_size
	menu: Option<ContextMenu>,
	menu_buffer: Buffer,
	decorated: bool,           // window frame shown (winit has no getter, so track it)
	menu_bar: bool,            // window menu bar (File/Edit/...) shown
	bar_open: Option<usize>,   // which top-level menu's dropdown is open, if any
	quit: bool,                // set by File->Quit; the event handler exits after applying
	win_opacity: Option<f32>,  // CLI --background-opacity override (this window only)
	win_title: Option<String>, // CLI --title override (else the app name)
	pending_about: bool, // request to open the About window (App acts on it; needs the event loop)
	pending_settings: bool, // request to open the Settings window
}

impl State {
	// Pixels reserved at the very top by the menu bar (0 when hidden).
	// Bar heights track the menu font's line height so they scale with font size.
	fn menu_bar_h(&self) -> f32 {
		self.text.cell_h + MENU_BAR_VPAD
	}
	fn tab_bar_h(&self) -> f32 {
		self.text.cell_h + TAB_BAR_VPAD
	}
	fn menubar_h(&self) -> f32 {
		if self.menu_bar {
			self.menu_bar_h()
		} else {
			0.0
		}
	}

	fn area(&self) -> Rect {
		// Panes sit below the menu bar (always when shown) and the tab bar
		// (only with >1 tab), stacked in that order.
		let bar = self.menubar_h()
			+ if self.tabs.len() > 1 {
				self.tab_bar_h()
			} else {
				0.0
			};
		Rect {
			x: 0.0,
			y: bar,
			w: self.gfx.config.width as f32,
			h: (self.gfx.config.height as f32 - bar).max(1.0),
		}
	}

	fn focus_at(&mut self, x: f32, y: f32) {
		if let Some(id) = self.tabs.cur().pane_at(x, y) {
			self.tabs.cur_mut().focused = id;
			self.update_title();
		}
	}

	// The window title (taskbar / alt-tab) is just the app name (or a CLI
	// --title override); per-program info lives in the tab titles.
	fn update_title(&self) {
		self.window
			.set_title(self.win_title.as_deref().unwrap_or(config::APP_NAME));
	}

	// Effective window opacity: a CLI --background-opacity override for this
	// window, else the configured value.
	fn opacity(&self) -> f32 {
		self.win_opacity
			.unwrap_or_else(|| config::settings().opacity)
	}

	fn open_menu(&mut self, target: PaneId, mx: f32, my: f32) {
		let ro = self
			.tabs
			.cur()
			.panes
			.get(&target)
			.is_some_and(|p| p.read_only);
		let entries = vec![
			mi("Copy", MenuAction::Copy),
			mi("Paste", MenuAction::Paste),
			mi("Paste Selection", MenuAction::PasteSelection),
			Entry::Sep,
			mt(ro, "Read-only", MenuAction::ToggleReadOnly),
			Entry::Sep,
			mi("New Tab", MenuAction::NewTab),
			mi("Split Vertical", MenuAction::SplitVertical),
			mi("Split Horizontal", MenuAction::SplitHorizontal),
			mi("Close Pane", MenuAction::Close),
			Entry::Sep,
			mt(
				self.window.fullscreen().is_some(),
				"Fullscreen",
				MenuAction::ToggleFullscreen,
			),
			mt(
				!self.decorated,
				"Hide window frame",
				MenuAction::ToggleFrame,
			),
			mt(self.menu_bar, "Menu bar", MenuAction::ToggleMenuBar),
			Entry::Sep,
			mi("Reload Config", MenuAction::ReloadConfig),
			mi("Settings\u{2026}", MenuAction::Settings),
		];
		self.bar_open = None;
		self.popup(target, entries, mx, my);
	}

	// Build and place a dropdown/context popup, clamped on-screen. Width is the
	// widest (proportional) label plus the checkmark gutter and padding.
	fn popup(&mut self, target: PaneId, entries: Vec<Entry>, mx: f32, my: f32) {
		let attrs = crate::text::sans_attrs();
		let mut textw: f32 = 0.0;
		for e in &entries {
			if let Entry::Item { label, .. } = e {
				textw = textw.max(self.text.measure_text(label, &attrs));
			}
		}
		let w = config::MENU_GUTTER + textw + config::MENU_PAD_X * 2.0;
		let item_h = self.text.cell_h;
		let menu = ContextMenu {
			x: mx,
			y: my,
			w,
			item_h,
			target,
			entries,
			hover: None,
		};
		let sw = self.gfx.config.width as f32;
		let sh = self.gfx.config.height as f32;
		let x = mx.min((sw - w).max(0.0));
		let y = my.min((sh - menu.height()).max(0.0));
		self.menu = Some(ContextMenu { x, y, ..menu });
	}

	// The dropdown entries for top-level menu-bar entry `idx` (File/Edit/...).
	fn bar_menu_items(&self, idx: usize) -> Vec<Entry> {
		let ro = self
			.tabs
			.cur()
			.panes
			.get(&self.tabs.cur().focused)
			.is_some_and(|p| p.read_only);
		match idx {
			0 => vec![
				mi("Reload Config", MenuAction::ReloadConfig),
				mi("Settings\u{2026}", MenuAction::Settings),
				Entry::Sep,
				mi("Quit", MenuAction::Quit),
			],
			1 => vec![
				mi("Copy", MenuAction::Copy),
				mi("Paste", MenuAction::Paste),
				mi("Paste Selection", MenuAction::PasteSelection),
				Entry::Sep,
				mt(ro, "Read-only", MenuAction::ToggleReadOnly),
			],
			2 => vec![
				mt(
					self.window.fullscreen().is_some(),
					"Fullscreen",
					MenuAction::ToggleFullscreen,
				),
				mt(
					!self.decorated,
					"Hide window frame",
					MenuAction::ToggleFrame,
				),
				mt(self.menu_bar, "Menu bar", MenuAction::ToggleMenuBar),
			],
			3 => vec![
				mi("New Tab", MenuAction::NewTab),
				mi("Next Tab", MenuAction::NextTab),
				mi("Previous Tab", MenuAction::PrevTab),
				Entry::Sep,
				mi("Close Tab", MenuAction::CloseTab),
			],
			4 => vec![
				mi("Split Vertical", MenuAction::SplitVertical),
				mi("Split Horizontal", MenuAction::SplitHorizontal),
				Entry::Sep,
				mi("Close Pane", MenuAction::Close),
			],
			_ => vec![mi("About\u{2026}", MenuAction::About)],
		}
	}

	// Open the dropdown for top-level menu `idx`, anchored under its title.
	fn open_bar_menu(&mut self, idx: usize) {
		let items = self.bar_menu_items(idx);
		let x = self.menubar_layout().get(idx).map_or(0.0, |&(x, _)| x);
		let target = self.tabs.cur().focused;
		let bar_h = self.menu_bar_h();
		self.popup(target, items, x, bar_h);
		self.bar_open = Some(idx);
	}

	// Per-title (x_left, width) layout of the menu bar, used for drawing and
	// hit-testing so they can't disagree. Titles use the proportional font.
	fn menubar_layout(&mut self) -> Vec<(f32, f32)> {
		let attrs = crate::text::sans_attrs();
		let mut x = 0.0;
		let mut out = Vec::with_capacity(MENU_BAR.len());
		for t in MENU_BAR {
			let w = self.text.measure_text(t, &attrs) + MENU_BAR_PAD * 2.0;
			out.push((x, w));
			x += w;
		}
		out
	}

	fn menubar_hit(&mut self, mx: f32) -> Option<usize> {
		self.menubar_layout()
			.iter()
			.position(|&(x, w)| mx >= x && mx < x + w)
	}

	// Request the About window. App opens it (window creation needs the event
	// loop); the old in-surface overlay path is no longer used.
	fn open_about(&mut self) {
		self.pending_about = true;
		self.menu = None;
		self.bar_open = None;
	}

	fn apply_menu(
		&mut self,
		action: MenuAction,
		target: PaneId,
		proxy: &EventLoopProxy<UserEvent>,
	) {
		let area = self.area();
		match action {
			MenuAction::Copy => {
				if let Some(text) = self
					.tabs
					.cur()
					.panes
					.get(&target)
					.and_then(|p| p.selection_text())
				{
					self.clipboard.set_clipboard(text);
				}
			}
			MenuAction::Paste => {
				if let Some(text) = self.clipboard.get_clipboard() {
					if let Some(p) = self.tabs.cur().panes.get(&target) {
						p.paste(&text);
					}
				}
			}
			MenuAction::PasteSelection => {
				if let Some(text) = self.clipboard.get_primary() {
					if let Some(p) = self.tabs.cur().panes.get(&target) {
						p.paste(&text);
					}
				}
			}
			MenuAction::ToggleReadOnly => {
				if let Some(p) = self.tabs.cur_mut().panes.get_mut(&target) {
					p.read_only = !p.read_only;
				}
			}
			MenuAction::SplitVertical => {
				let _ =
					self.tabs
						.cur_mut()
						.split(&mut self.text, proxy, target, Dir::Vertical, area);
			}
			MenuAction::SplitHorizontal => {
				let _ =
					self.tabs
						.cur_mut()
						.split(&mut self.text, proxy, target, Dir::Horizontal, area);
			}
			MenuAction::Close => {
				if self.tabs.cur().panes.len() > 1 {
					self.tabs.cur_mut().close(&mut self.text, target, area);
				} else if self.tabs.len() > 1 {
					// last pane in this tab -> close the tab
					self.close_tab();
				} else {
					// last pane of the last tab -> close the window
					self.quit = true;
				}
			}
			MenuAction::NewTab => self.new_tab(proxy),
			MenuAction::CloseTab => self.close_tab(),
			MenuAction::NextTab => {
				self.tabs.next();
				self.relayout_all();
			}
			MenuAction::PrevTab => {
				self.tabs.prev();
				self.relayout_all();
			}
			MenuAction::ToggleFullscreen => self.toggle_fullscreen(),
			MenuAction::ToggleFrame => {
				self.decorated = !self.decorated;
				self.window.set_decorations(self.decorated);
			}
			MenuAction::ToggleMenuBar => {
				self.menu_bar = !self.menu_bar;
				self.relayout_all();
			}
			MenuAction::ReloadConfig => self.reload_config(),
			MenuAction::Settings => self.open_settings(),
			MenuAction::About => self.open_about(),
			MenuAction::Quit => self.quit = true,
		}
		self.update_title();
	}

	// relayout every tab (not just the active one) - needed when the tab bar
	// appears/disappears (1<->2 tabs) and the pane area changes.
	fn relayout_all(&mut self) {
		let area = self.area();
		for pm in &mut self.tabs.list {
			pm.relayout(&mut self.text, area);
		}
	}

	// Track the live window size as columns/rows so "remember last size" can
	// restore it next launch. Kept separate from the user's defined columns/rows
	// (unchecking the option reverts to those). The inverse of the launch sizing.
	fn save_window_size(&mut self, w: u32, h: u32) {
		// skip the creation/programmatic resizes that fire before the first frame,
		// so they don't clobber the remembered size with the launch size
		if !self.size_tracked {
			return;
		}
		let inv = |px: f32, cell: f32, chrome: f32| {
			(((px - 2.0 * self.text.margin - chrome) / cell).floor() as i64).max(1) as usize
		};
		let cols = inv(w as f32, self.text.cell_w, 0.0);
		let rows = inv(h as f32, self.text.cell_h, self.menubar_h());
		let orig = (*config::settings()).clone();
		if cols == orig.remembered_columns && rows == orig.remembered_rows {
			return;
		}
		let mut new = orig.clone();
		new.remembered_columns = cols;
		new.remembered_rows = rows;
		config::persist(&orig, &new);
		config::update(new);
	}

	fn new_tab(&mut self, proxy: &EventLoopProxy<UserEvent>) {
		// area with the bar shown (we're about to have >1 tab); relayout_all fixes
		// the exact rects right after, this is just the new pane's provisional box
		let bar = self.menubar_h() + self.tab_bar_h();
		let area = Rect {
			x: 0.0,
			y: bar,
			w: self.gfx.config.width as f32,
			h: (self.gfx.config.height as f32 - bar).max(1.0),
		};
		if let Ok(pm) = PaneManager::new(&mut self.text, proxy, area, config::default_shell_argv())
		{
			self.tabs.list.push(pm);
			self.tabs.active = self.tabs.list.len() - 1;
			self.relayout_all(); // existing tab(s) shrink for the now-shown bar
			self.update_title();
			self.dirty = true;
		}
	}

	fn close_tab(&mut self) {
		self.close_tab_at(self.tabs.active);
	}

	// Close the tab at `idx` (not necessarily the active one - a background tab's
	// shell can exit). Keeps `active` pointing at the same tab where it can.
	fn close_tab_at(&mut self, idx: usize) {
		if self.tabs.list.len() <= 1 {
			return; // keep at least one tab; close the window to exit
		}
		self.tabs.list.remove(idx);
		if self.tabs.active > idx {
			self.tabs.active -= 1; // a tab before the active one went away
		}
		if self.tabs.active >= self.tabs.list.len() {
			self.tabs.active = self.tabs.list.len() - 1;
		}
		self.relayout_all(); // if back to 1 tab, the bar hides and panes grow
		self.update_title();
		self.dirty = true;
	}

	fn toggle_fullscreen(&self) {
		let fs = match self.window.fullscreen() {
			Some(_) => None,
			None => Some(Fullscreen::Borderless(None)),
		};
		self.window.set_fullscreen(fs);
	}

	// Request the Settings window (App opens it; window creation needs the loop).
	fn open_settings(&mut self) {
		self.pending_settings = true;
		self.menu = None;
		self.bar_open = None;
	}

	// Live-apply edited settings (from the dialog), persist, and rebuild whatever
	// the change touched (text metrics, background image, opacity, window size).
	fn apply_settings_values(
		&mut self,
		orig: &config::Settings,
		edited: config::Settings,
		system_font: bool,
	) {
		config::persist(orig, &edited);
		// "Use system font" means follow the OS: drop the keys so future launches
		// re-detect (the live font values were already applied via `edited`).
		if system_font {
			config::remove_keys(&["font_family", "font_size"]);
		}
		self.apply_new_settings(orig, edited, false);
	}

	// Re-read config.toml from disk and live-apply it (the "internal command" for
	// picking up hand-edits without a file watcher). The file is the source here,
	// so nothing is persisted back.
	fn reload_config(&mut self) {
		let orig = config::settings().as_ref().clone();
		let edited = config::reload_from_disk();
		// Force the background image to re-read even when its path is unchanged:
		// the user may have swapped the file contents under the same name (#167).
		self.apply_new_settings(&orig, edited, true);
	}

	// Swap in `edited` and rebuild whatever changed vs `orig` (text metrics,
	// background image, window opacity). Shared by the dialog and config reload.
	// `force_bg` re-reads the image even if the path string didn't change.
	fn apply_new_settings(
		&mut self,
		orig: &config::Settings,
		edited: config::Settings,
		force_bg: bool,
	) {
		let rebuild = crate::settings_ui::needs_text_rebuild(orig, &edited);
		let bg = force_bg || crate::settings_ui::bg_image_changed(orig, &edited);
		let resize = edited.columns != orig.columns || edited.rows != orig.rows;
		let blur_changed = edited.transparent_background_blur != orig.transparent_background_blur;
		config::update(edited);

		// Backdrop-blur hint toggled -> set/clear the compositor property live.
		if blur_changed {
			set_blur_behind(&self.window, config::settings().transparent_background_blur);
		}

		// Transparency is per-pixel (terminal background only) - never whole-window.
		// Nothing to do here; the bg fill picks up the new opacity on the next frame.
		// window dimensions changed in Settings -> resize to the new cell grid
		if resize {
			let s = config::settings();
			let want = winit::dpi::PhysicalSize::new(
				(s.columns as f32 * self.text.cell_w + 2.0 * self.text.margin).ceil() as u32,
				(s.rows as f32 * self.text.cell_h + 2.0 * self.text.margin + self.menubar_h())
					.ceil() as u32,
			);
			if let Some(applied) = self.window.request_inner_size(want) {
				self.gfx.resize(applied.width, applied.height);
			}
		}
		if rebuild {
			let scale = self.window.scale_factor() as f32;
			self.text = TextCtx::new(&self.gfx.device, &self.gfx.queue, self.gfx.format, scale);
			self.menu_buffer = self.text.new_buffer(400.0, 400.0);
			for pm in &mut self.tabs.list {
				pm.rebuild_buffers(&mut self.text);
			}
			self.relayout_all();
		}
		if bg {
			self.bg_image = load_bg_image(&self.gfx);
		}
		self.dirty = true;
	}

	// returns true while any pane is still animating (caller keeps frames coming).
	// `force_rebuild` = the frame changed content/scroll/bell (not a pure cursor
	// animation), so panes re-shape text; false lets them reuse the cached frame.
	fn render(&mut self, force_rebuild: bool) -> bool {
		// once a frame has been drawn, later resizes are user-driven and may update
		// the remembered window size (startup/programmatic ones happen before this)
		self.size_tracked = true;
		let area = self.area();
		if area.w < 1.0 || area.h < 1.0 {
			return false;
		}

		let now = Instant::now();
		let dt = (now - self.last_frame).as_secs_f32().min(0.1);
		self.last_frame = now;

		// Visual-bell flash decays toward 0; while >0 the text is brightened (in
		// build) and we keep rendering so the fade is smooth.
		if self.bell_flash > 0.0 {
			self.bell_flash = (self.bell_flash * (-dt / BELL_TAU_S).exp()).max(0.0);
			if self.bell_flash < 0.01 {
				self.bell_flash = 0.0;
			}
		}
		let bell = self.bell_flash;
		let overwrite = self.overwrite;

		// translucent background only when the surface supports it AND the user has
		// Transparency on - and it only ever affects the bg, never text/chrome.
		let bg_alpha = if self.gfx.transparent && config::settings().transparent_background {
			self.opacity()
		} else {
			1.0
		};

		let mut under: Vec<RectInstance> = Vec::new();
		// per-pane (bg cells + cursor), scissored to the pane so overscan rows
		// don't bleed into neighbours
		let mut groups: Vec<(Rect, Vec<RectInstance>)> = Vec::new();
		let mut tops: HashMap<u64, f32> = HashMap::new();
		let mut animating = bell > 0.0;
		// text-glow colour map needs each cell's bg (so a glyph's halo takes its
		// own cell colour, not always the global) - collect them while building
		let glow_on = {
			let s = config::settings();
			s.text_glow && s.text_glow_radius > 0.0
		};
		let mut glow_cells: Vec<RectInstance> = Vec::new();

		for (id, pane) in self.tabs.cur_mut().panes.iter_mut() {
			pane.scroll.advance(dt);
			let rect = pane.rect;
			let draw = pane.build(&mut self.text, dt, bell, force_rebuild, overwrite);
			if pane.scroll.animating() || pane.cursor_animating {
				animating = true;
			}
			tops.insert(*id, draw.top);
			let mut bg = config::srgb_f32(config::settings().bg);
			bg[3] = bg_alpha;
			under.push(RectInstance {
				pos: [rect.x, rect.y],
				size: [rect.w, rect.h],
				color: bg,
			});
			if glow_on {
				glow_cells.extend_from_slice(&draw.bg);
			}
			let mut g = draw.bg;
			g.extend(draw.cursor);
			groups.push((rect, g));
		}

		let under_len = under.len() as u32;
		let mut instances = under;
		let mut group_ranges: Vec<(Rect, u32, u32)> = Vec::new();
		for (rect, g) in groups {
			let start = instances.len() as u32;
			instances.extend(g);
			group_ranges.push((rect, start, instances.len() as u32));
		}

		let ring_start = instances.len() as u32;
		// Focus ring only distinguishes panes when there's more than one; with a
		// single pane it's just an unwanted border line around the whole content
		// (the user wants background all the way to the edge), so skip it.
		if self.tabs.cur().panes.len() > 1 {
			if let Some(p) = self.tabs.cur().panes.get(&self.tabs.cur().focused) {
				instances.extend(focus_ring(p.rect));
			}
		}
		// drop-target tint while drag-reordering a pane
		if let Some(src) = self.dragging_pane {
			if let Some(tid) = self.tabs.cur().pane_at(self.mouse.0, self.mouse.1) {
				if tid != src {
					if let Some(p) = self.tabs.cur().panes.get(&tid) {
						let mut c = config::srgb_f32(config::DROP_TARGET);
						c[3] = 0.30;
						instances.push(RectInstance {
							pos: [p.rect.x, p.rect.y],
							size: [p.rect.w, p.rect.h],
							color: c,
						});
					}
				}
			}
		}
		let ring_end = instances.len() as u32;

		let bw = self.gfx.config.width as f32;
		let menu_h = self.menu_bar_h();
		let tab_h = self.tab_bar_h();

		// menu bar (File/Edit/...), drawn in the main pass at the very top; the
		// open menu's title is highlighted.
		let menubar_range = if self.menu_bar {
			let start = instances.len() as u32;
			instances.push(rect_inst(0.0, 0.0, bw, menu_h, config::TAB_BAR_BG));
			let layout = self.menubar_layout();
			if let Some(idx) = self.bar_open {
				if let Some(&(x, w)) = layout.get(idx) {
					instances.push(rect_inst(x, 0.0, w, menu_h, config::MENU_HOVER));
				}
			} else if self.mods.alt_key() {
				// Alt held (no dropdown open): underline each title's accelerator
				// letter, like the open-dropdown items do (press the letter to open).
				let acc = crate::text::sans_attrs();
				let uy = MENU_BAR_VPAD / 2.0 + self.text.cell_h - 2.0;
				for (i, &(x, _)) in layout.iter().enumerate() {
					if let Some(c) = MENU_BAR[i].chars().next() {
						let mut buf = [0u8; 4];
						let cw = self.text.measure_text(c.encode_utf8(&mut buf), &acc);
						instances.push(rect_inst(x + MENU_BAR_PAD, uy, cw, 1.0, config::MENU_FG));
					}
				}
			}
			Some((start, instances.len() as u32))
		} else {
			None
		};

		// tab bar (only with >1 tab), drawn just below the menu bar
		let tby = self.menubar_h();
		let tabbar_range = if self.tabs.len() > 1 {
			let start = instances.len() as u32;
			instances.push(rect_inst(0.0, tby, bw, tab_h, config::TAB_BAR_BG));
			let n = self.tabs.len();
			let tw = (bw / n as f32).min(220.0);
			for i in 0..n {
				let x = i as f32 * tw;
				let col = if i == self.tabs.active {
					config::TAB_ACTIVE
				} else {
					config::TAB_INACTIVE
				};
				instances.push(rect_inst(x + 1.0, tby + 2.0, tw - 2.0, tab_h - 3.0, col));
			}
			Some((start, instances.len() as u32))
		} else {
			None
		};

		// context menu quads (drawn in a second pass, on top of everything)
		let menu_range = if let Some(menu) = &self.menu {
			let start = instances.len() as u32;
			let mh = menu.height();
			let b = 1.0;
			instances.push(rect_inst(
				menu.x - b,
				menu.y - b,
				menu.w + 2.0 * b,
				mh + 2.0 * b,
				config::MENU_BORDER,
			));
			instances.push(rect_inst(menu.x, menu.y, menu.w, mh, config::MENU_BG));
			if let Some(i) = menu.hover {
				instances.push(rect_inst(
					menu.x,
					menu.row_top(i),
					menu.w,
					menu.item_h,
					config::MENU_HOVER,
				));
			}
			// faint separator lines between logical groups
			for (i, e) in menu.entries.iter().enumerate() {
				if matches!(e, Entry::Sep) {
					let ly = menu.row_top(i) + config::MENU_SEP_H / 2.0;
					instances.push(rect_inst(
						menu.x + config::MENU_PAD_X,
						ly,
						menu.w - config::MENU_PAD_X * 2.0,
						1.0,
						config::MENU_SEP,
					));
				}
			}
			// accelerator underline under each item's first letter (press it to pick)
			let acc_attrs = crate::text::sans_attrs();
			let ch = self.text.cell_h;
			let lx = menu.x + config::MENU_PAD_X + config::MENU_GUTTER;
			for (i, e) in menu.entries.iter().enumerate() {
				if let Entry::Item { label, .. } = e {
					if let Some(c) = label.chars().next() {
						let mut buf = [0u8; 4];
						let cw = self.text.measure_text(c.encode_utf8(&mut buf), &acc_attrs);
						let top = menu.row_top(i) + (menu.item_h - ch) / 2.0;
						instances.push(rect_inst(lx, top + ch - 3.0, cw, 1.0, config::MENU_FG));
					}
				}
			}
			Some((start, instances.len() as u32))
		} else {
			None
		};

		let overlay_range = menu_range;

		let margin = self.text.margin;
		let menu_fg = GColor::rgb(config::MENU_FG[0], config::MENU_FG[1], config::MENU_FG[2]);
		// tab titles ("<shell> [<program>]") - computed first (tab_title is &mut)
		// before self.text is borrowed for the buffers below
		let tab_titles: Vec<String> = if self.tabs.len() > 1 {
			self.tabs
				.list
				.iter_mut()
				.map(|pm| {
					if let Some(t) = &pm.title_override {
						return t.clone();
					}
					let fid = pm.focused;
					pm.panes
						.get_mut(&fid)
						.map(|p| p.term.tab_title())
						.unwrap_or_else(|| config::APP_NAME.into())
				})
				.collect()
		} else {
			Vec::new()
		};
		// tab titles need transient buffers; build them before `areas` borrows panes
		let mut tab_bufs: Vec<Buffer> = Vec::new();
		let tab_w = (self.gfx.config.width as f32 / self.tabs.len().max(1) as f32).min(220.0);
		for title in &tab_titles {
			let mut b = self.text.new_buffer((tab_w - 16.0).max(8.0), tab_h);
			let mut attrs = crate::text::sans_attrs();
			attrs.color_opt = Some(menu_fg);
			b.set_text(
				&mut self.text.font_system,
				title,
				&attrs,
				Shaping::Advanced,
				None,
			);
			b.shape_until_scroll(&mut self.text.font_system, false);
			tab_bufs.push(b);
		}
		// menu-bar title buffers (one per top-level menu), proportional font
		let mut menubar_bufs: Vec<Buffer> = Vec::new();
		if self.menu_bar {
			for t in MENU_BAR {
				let mut b = self.text.new_buffer(120.0, menu_h);
				let mut attrs = crate::text::sans_attrs();
				attrs.color_opt = Some(menu_fg);
				b.set_text(
					&mut self.text.font_system,
					t,
					&attrs,
					Shaping::Advanced,
					None,
				);
				b.shape_until_scroll(&mut self.text.font_system, false);
				menubar_bufs.push(b);
			}
		}
		// compute before borrowing panes for `areas` (menubar_layout takes &mut self)
		let bar_layout = self.menubar_layout();
		let mut areas: Vec<TextArea> = Vec::new();
		for p in self.tabs.cur().panes.values() {
			areas.push(p.text_area(tops[&p.id], margin));
			areas.extend(p.glyph_areas());
		}
		for (i, b) in menubar_bufs.iter().enumerate() {
			let (x, w) = bar_layout[i];
			areas.push(TextArea {
				buffer: b,
				left: x + MENU_BAR_PAD,
				top: MENU_BAR_VPAD / 2.0,
				scale: 1.0,
				bounds: TextBounds {
					left: x as i32,
					top: 0,
					right: (x + w) as i32,
					bottom: menu_h as i32,
				},
				default_color: menu_fg,
				custom_glyphs: &[],
			});
		}
		for (i, b) in tab_bufs.iter().enumerate() {
			let x = i as f32 * tab_w;
			areas.push(TextArea {
				buffer: b,
				left: x + 8.0,
				// sit a touch high in the bar so descenders get bottom clearance
				// (bug: "tab font doesn't have enough space on the bottom")
				top: tby + (TAB_BAR_VPAD / 2.0) - 1.0,
				scale: 1.0,
				bounds: TextBounds {
					left: x as i32,
					top: tby as i32,
					right: (x + tab_w) as i32,
					bottom: (tby + tab_h) as i32,
				},
				default_color: menu_fg,
				custom_glyphs: &[],
			});
		}

		// All rect instances and the bg-image shader work in absolute
		// framebuffer pixels (matching the glyphon viewport), so the resolution
		// is the whole window - NOT the content `area`, which is shorter by the
		// menu/tab bars and would shift cell bg + cursor down relative to text.
		let (fw, fh) = (self.gfx.config.width as f32, self.gfx.config.height as f32);
		self.text.update_viewport(
			&self.gfx.queue,
			self.gfx.config.width,
			self.gfx.config.height,
		);
		self.rects.set_resolution(&self.gfx.queue, fw, fh);
		if let Some(img) = &self.bg_image {
			img.set_resolution(&self.gfx.queue, fw, fh);
		}
		self.rects
			.upload(&self.gfx.device, &self.gfx.queue, &instances);
		if let Err(e) = self.text.prepare(&self.gfx.device, &self.gfx.queue, areas) {
			// Atlas full (after a long session of varied glyphs). The normal per-frame
			// trim is at the END of render, below this early return - so without
			// trimming here the atlas never recovers and ALL text goes black for good
			// (cursor/cell-bg quads use a separate renderer, so they still show). Trim
			// now to free space; the next frame re-prepares with room and recovers.
			eprintln!(
				"{}: text prepare failed; trimming atlas to recover: {e:?}",
				config::APP_NAME
			);
			self.text.trim_atlas();
			return animating;
		}

		// lay out the menu into the overlay renderer: one proportional buffer
		// per item label (at the gutter), plus a checkmark buffer for checked toggles.
		if let Some(menu) = &self.menu {
			// (left, top, buffer) collected first so the borrow of self.text ends
			let mut specs: Vec<(f32, f32, Buffer)> = Vec::new();
			let mut attrs = crate::text::sans_attrs();
			attrs.color_opt = Some(GColor::rgb(
				config::MENU_FG[0],
				config::MENU_FG[1],
				config::MENU_FG[2],
			));
			for (i, e) in menu.entries.iter().enumerate() {
				let Entry::Item { label, check, .. } = e else {
					continue;
				};
				let top = menu.row_top(i) + (menu.item_h - self.text.cell_h) / 2.0;
				let mut b = self.text.new_buffer(menu.w, menu.item_h);
				b.set_text(
					&mut self.text.font_system,
					label,
					&attrs,
					Shaping::Advanced,
					None,
				);
				b.shape_until_scroll(&mut self.text.font_system, false);
				specs.push((menu.x + config::MENU_PAD_X + config::MENU_GUTTER, top, b));
				if *check == Some(true) {
					let mut c = self.text.new_buffer(config::MENU_GUTTER, menu.item_h);
					c.set_text(
						&mut self.text.font_system,
						"\u{2713}",
						&attrs,
						Shaping::Advanced,
						None,
					);
					c.shape_until_scroll(&mut self.text.font_system, false);
					specs.push((menu.x + config::MENU_PAD_X, top, c));
				}
			}
			let (sw, sh) = (self.gfx.config.width as i32, self.gfx.config.height as i32);
			let fg = GColor::rgb(config::MENU_FG[0], config::MENU_FG[1], config::MENU_FG[2]);
			let areas: Vec<TextArea> = specs
				.iter()
				.map(|(left, top, b)| TextArea {
					buffer: b,
					left: *left,
					top: *top,
					scale: 1.0,
					bounds: TextBounds {
						left: 0,
						top: 0,
						right: sw,
						bottom: sh,
					},
					default_color: fg,
					custom_glyphs: &[],
				})
				.collect();
			let _ = self
				.text
				.prepare_overlay(&self.gfx.device, &self.gfx.queue, areas);
		}

		let frame = match self.gfx.begin_frame() {
			Some(f) => f,
			None => return animating,
		};
		let view = self.gfx.frame_view(&frame);
		let mut encoder = self
			.gfx
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor {
				label: Some("frame"),
			});

		// Text readability glow: build the per-pixel colour map, render the prepared
		// text to the glow texture, blur it, then composite under the crisp text.
		let gs = config::settings();
		// "Softness" 0..1 -> coverage boost: 0 = hard/solid (x10), 1 = soft/faint (x1)
		let glow_intensity = 10.0 - gs.text_glow_softness.clamp(0.0, 1.0) * 9.0;
		if glow_on {
			self.glow.render_bgcolor(
				&self.gfx.device,
				&self.gfx.queue,
				&mut encoder,
				&glow_cells,
				config::srgb_f32(gs.bg),
			);
			{
				let mut p = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
					label: Some("glow text"),
					color_attachments: &[Some(wgpu::RenderPassColorAttachment {
						view: self.glow.text_view(),
						resolve_target: None,
						depth_slice: None,
						ops: wgpu::Operations {
							load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
							store: wgpu::StoreOp::Store,
						},
					})],
					depth_stencil_attachment: None,
					timestamp_writes: None,
					occlusion_query_set: None,
					multiview_mask: None,
				});
				let _ = self.text.render(&mut p);
			}
			self.glow
				.blur(&self.gfx.queue, &mut encoder, gs.text_glow_radius);
		}

		{
			let dv = config::srgb_f32(config::DIVIDER);
			// transparent base when compositing: pane-gap dividers show the
			// desktop through; opaque divider color otherwise (premultiplied)
			let clear = if self.gfx.transparent {
				wgpu::Color::TRANSPARENT
			} else {
				wgpu::Color {
					r: dv[0] as f64,
					g: dv[1] as f64,
					b: dv[2] as f64,
					a: 1.0,
				}
			};
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("main pass"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &view,
					resolve_target: None,
					depth_slice: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Clear(clear),
						store: wgpu::StoreOp::Store,
					},
				})],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});

			let (sw, sh) = (self.gfx.config.width, self.gfx.config.height);
			// pane backgrounds (exactly pane-sized, no clip needed)
			self.rects.draw(&mut pass, 0..under_len);
			// background image over the pane fill, under cells/text
			if let Some(img) = &self.bg_image {
				img.draw(&mut pass);
			}
			// per-pane cell bg + cursor, clipped to the pane
			for (rect, start, end) in &group_ranges {
				let (x, y, w, h) = scissor(*rect, sw, sh);
				if w == 0 || h == 0 {
					continue;
				}
				pass.set_scissor_rect(x, y, w, h);
				self.rects.draw(&mut pass, *start..*end);
			}
			pass.set_scissor_rect(0, 0, sw, sh);
			// menu/tab-bar quads before the text so their titles draw on top
			if let Some((ms, me)) = menubar_range {
				self.rects.draw(&mut pass, ms..me);
			}
			if let Some((ts, te)) = tabbar_range {
				self.rects.draw(&mut pass, ts..te);
			}
			// glow goes under the crisp text, over the cell backgrounds
			if glow_on {
				self.glow
					.composite(&self.gfx.queue, &mut pass, glow_intensity);
			}
			if let Err(e) = self.text.render(&mut pass) {
				eprintln!("{}: text render failed: {e:?}", config::APP_NAME);
			}
			self.rects.draw(&mut pass, ring_start..ring_end);
		}

		// second pass: context menu / menu-bar dropdown on top (preserves main pass)
		if let Some((mstart, mend)) = overlay_range {
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("overlay pass"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &view,
					resolve_target: None,
					depth_slice: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Load,
						store: wgpu::StoreOp::Store,
					},
				})],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			self.rects.draw(&mut pass, mstart..mend);
			let _ = self.text.render_overlay(&mut pass);
		}

		self.gfx.queue.submit(Some(encoder.finish()));
		self.gfx.end_frame(frame);
		if std::env::var_os("SILK_DUMP").is_some() {
			self.gfx.dump_offscreen("/tmp/silk_offscreen.png");
		}
		self.text.trim_atlas();
		animating
	}
}

fn rect_inst(x: f32, y: f32, w: f32, h: f32, color: [u8; 3]) -> RectInstance {
	RectInstance {
		pos: [x, y],
		size: [w, h],
		color: config::srgb_f32(color),
	}
}

// The window/taskbar icon, decoded from the bundled logo (downscaled so the
// _NET_WM_ICON payload stays small). None if it can't be decoded.
// X11 session? (Per-pixel transparency needs the glutin GL path only on X11;
// Wayland's wgpu surface already does premultiplied alpha.)
fn is_x11(el: &ActiveEventLoop) -> bool {
	use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
	el.owned_display_handle()
		.display_handle()
		.map(|h| {
			matches!(
				h.as_raw(),
				RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_)
			)
		})
		.unwrap_or(false)
}

// Stable X11 WM_CLASS (+ Wayland app_id) so the window is identifiable to the
// WM/taskbar and matchable in compositor rules - e.g. Compiz's blur "Blur
// Windows" = class=SilkTerm. winit's with_name(general, instance) yields
// WM_CLASS = "instance", "general", so res_class="SilkTerm", res_name="silkterm".
#[cfg(target_os = "linux")]
fn with_app_id(attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
	use winit::platform::wayland::WindowAttributesExtWayland;
	use winit::platform::x11::WindowAttributesExtX11;
	let attrs = WindowAttributesExtX11::with_name(attrs, "SilkTerm", "silkterm");
	WindowAttributesExtWayland::with_name(attrs, "SilkTerm", "silkterm")
}
#[cfg(not(target_os = "linux"))]
fn with_app_id(attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
	attrs
}

// Ask a KWin/picom-style compositor to blur the desktop behind the window's
// translucent regions (frosted glass) via _KDE_NET_WM_BLUR_BEHIND_REGION: a
// single 0 cardinal = blur the whole window, deleting the property turns it off.
// X11-only and compositor-dependent - Compiz/GNOME ignore the hint (there the
// user enables blur in the compositor), and the compositor, not us, owns the
// blur radius. Opens a throwaway connection; called only at startup / on toggle.
#[cfg(target_os = "linux")]
fn set_blur_behind(window: &Window, enable: bool) {
	use raw_window_handle::{HasWindowHandle, RawWindowHandle};
	use x11rb::connection::Connection;
	use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
	use x11rb::wrapper::ConnectionExt as _;

	let Ok(handle) = window.window_handle() else {
		return;
	};
	let xid = match handle.as_raw() {
		RawWindowHandle::Xlib(h) => h.window as u32,
		RawWindowHandle::Xcb(h) => h.window.get(),
		_ => return, // not X11 (Wayland/other): the hint is X11-only
	};
	let Ok((conn, _)) = x11rb::connect(None) else {
		return;
	};
	let Ok(cookie) = conn.intern_atom(false, b"_KDE_NET_WM_BLUR_BEHIND_REGION") else {
		return;
	};
	let Ok(reply) = cookie.reply() else {
		return;
	};
	let atom = reply.atom;
	if enable {
		let _ = conn.change_property32(PropMode::REPLACE, xid, atom, AtomEnum::CARDINAL, &[0u32]);
	} else {
		let _ = conn.delete_property(xid, atom);
	}
	let _ = conn.flush();
}
#[cfg(not(target_os = "linux"))]
fn set_blur_behind(_window: &Window, _enable: bool) {}

pub fn load_icon() -> Option<winit::window::Icon> {
	let img = image::load_from_memory(include_bytes!("../assets/logo.png")).ok()?;
	let img = img.thumbnail(64, 64).into_rgba8();
	let (w, h) = img.dimensions();
	winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

// Build the initial tabs/panes from the parsed command line. Without
// hierarchical flags, one tab with one pane (running any window-level --shell).
fn build_layout(
	cli: &crate::cli::Cli,
	text: &mut TextCtx,
	proxy: &EventLoopProxy<UserEvent>,
	area: Rect,
) -> Vec<PaneManager> {
	use crate::cli::Size;
	if !cli.hierarchical {
		let shell = cli
			.win
			.style
			.shell
			.clone()
			.or_else(config::default_shell_argv);
		let pm = PaneManager::new(text, proxy, area, shell).expect("spawn shell");
		return vec![pm];
	}
	let mut out = Vec::new();
	for tab in &cli.tabs {
		// main pane's shell cascades pane -> tab -> window
		let main_shell = tab.panes[0]
			.style
			.shell
			.clone()
			.or_else(|| tab.style.shell.clone())
			.or_else(|| cli.win.style.shell.clone())
			.or_else(config::default_shell_argv);
		let mut pm = PaneManager::new(text, proxy, area, main_shell.clone()).expect("spawn shell");
		let main_id = pm.focused;
		let mut handles: HashMap<String, PaneId> = HashMap::new();
		handles.insert("main".into(), main_id);
		handles.insert("0".into(), main_id);
		if let Some(h) = &tab.panes[0].id {
			handles.insert(h.clone(), main_id);
		}
		let mut shells: HashMap<PaneId, Option<Vec<String>>> = HashMap::new();
		shells.insert(main_id, main_shell);
		let mut prev = main_id;

		for ps in &tab.panes[1..] {
			let target = ps
				.splits
				.as_deref()
				.and_then(|h| handles.get(h).copied())
				.unwrap_or(prev);
			let dir4 = ps.dir.unwrap_or_else(|| default_dir(&pm, target));
			let (dir, before) = match dir4 {
				crate::cli::Dir4::Down => (Dir::Horizontal, false),
				crate::cli::Dir4::Up => (Dir::Horizontal, true),
				crate::cli::Dir4::Right => (Dir::Vertical, false),
				crate::cli::Dir4::Left => (Dir::Vertical, true),
			};
			// new pane's shell: explicit -> the pane it splits -> tab -> window
			let shell = ps
				.style
				.shell
				.clone()
				.or_else(|| shells.get(&target).cloned().flatten())
				.or_else(|| tab.style.shell.clone())
				.or_else(|| cli.win.style.shell.clone())
				.or_else(config::default_shell_argv);
			let ratio = match ps.size {
				None => 0.5,
				Some(Size::Percent(p)) => p / 100.0,
				Some(Size::Cells(n)) => {
					let r = pm.panes.get(&target).map(|p| p.rect).unwrap_or(area);
					let denom = match dir {
						Dir::Vertical => (r.w / text.cell_w).max(1.0),
						Dir::Horizontal => (r.h / text.cell_h).max(1.0),
					};
					n as f32 / denom
				}
			};
			if let Some(nid) =
				pm.split_at(text, proxy, target, dir, before, ratio, shell.clone(), area)
			{
				if let Some(h) = &ps.id {
					handles.insert(h.clone(), nid);
				}
				shells.insert(nid, shell);
				prev = nid;
			}
		}
		// focus the tab's first pane, not the last split
		pm.focused = main_id;
		pm.title_override = tab.title.clone();
		out.push(pm);
	}
	out
}

// Default split direction when none is given: split along the longer axis so the
// new pane lands where there's more room.
fn default_dir(pm: &PaneManager, target: PaneId) -> crate::cli::Dir4 {
	let r = pm.panes.get(&target).map(|p| p.rect);
	match r {
		Some(r) if r.h > r.w => crate::cli::Dir4::Down,
		_ => crate::cli::Dir4::Right,
	}
}

// Open a URL in the user's default browser (fire-and-forget, per platform).
fn open_url(url: &str) {
	let mut cmd = if cfg!(target_os = "macos") {
		let mut c = std::process::Command::new("open");
		c.arg(url);
		c
	} else if cfg!(target_os = "windows") {
		let mut c = std::process::Command::new("cmd");
		c.args(["/C", "start", "", url]);
		c
	} else {
		let mut c = std::process::Command::new("xdg-open");
		c.arg(url);
		c
	};
	let _ = cmd.spawn();
}

// Decode the configured background image and upload it to a texture.
fn srgb_to_lin(b: u8) -> f32 {
	let c = b as f32 / 255.0;
	if c <= 0.04045 {
		c / 12.92
	} else {
		((c + 0.055) / 1.055).powf(2.4)
	}
}
fn lin_to_srgb_u8(c: f32) -> u8 {
	let c = c.clamp(0.0, 1.0);
	let s = if c <= 0.0031308 {
		c * 12.92
	} else {
		1.055 * c.powf(1.0 / 2.4) - 0.055
	};
	(s * 255.0 + 0.5) as u8
}

fn load_bg_image(gfx: &Gfx) -> Option<ImageRenderer> {
	let s = config::settings();
	let path = s.background_image.as_ref()?;
	let mut img = match image::open(path) {
		Ok(i) => i.to_rgba8(),
		Err(e) => {
			eprintln!(
				"{}: background image {}: {e}",
				config::APP_NAME,
				path.display()
			);
			return None;
		}
	};
	// Gaussian blur, done in LINEAR light (decode sRGB -> blur in f32 -> re-encode)
	// so transitions are gamma-correct; an sRGB-space blur darkens edges. The f32
	// intermediate also avoids 8-bit banding inside the blur (final banding is
	// handled by the high-precision offscreen + the blit's dither).
	if s.background_blur > 0.0 {
		let (w, h) = img.dimensions();
		let mut lin: image::ImageBuffer<image::Rgba<f32>, Vec<f32>> = image::ImageBuffer::new(w, h);
		for (d, srcp) in lin.pixels_mut().zip(img.pixels()) {
			*d = image::Rgba([
				srgb_to_lin(srcp[0]),
				srgb_to_lin(srcp[1]),
				srgb_to_lin(srcp[2]),
				srcp[3] as f32 / 255.0,
			]);
		}
		let blurred = image::imageops::blur(&lin, s.background_blur);
		for (d, srcp) in img.pixels_mut().zip(blurred.pixels()) {
			*d = image::Rgba([
				lin_to_srgb_u8(srcp[0]),
				lin_to_srgb_u8(srcp[1]),
				lin_to_srgb_u8(srcp[2]),
				(srcp[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
			]);
		}
	}
	let (w, h) = img.dimensions();
	Some(ImageRenderer::new(
		&gfx.device,
		&gfx.queue,
		gfx.format,
		&img,
		w,
		h,
		s.background_opacity,
		s.background_fit,
	))
}

// clamp a pane rect to an integer scissor box inside the surface
fn scissor(r: Rect, sw: u32, sh: u32) -> (u32, u32, u32, u32) {
	let x = r.x.max(0.0).min(sw as f32) as u32;
	let y = r.y.max(0.0).min(sh as f32) as u32;
	let right = (r.x + r.w).max(0.0).min(sw as f32) as u32;
	let bottom = (r.y + r.h).max(0.0).min(sh as f32) as u32;
	(x, y, right.saturating_sub(x), bottom.saturating_sub(y))
}

fn focus_ring(r: Rect) -> [RectInstance; 4] {
	let c = config::srgb_f32(config::settings().focus);
	let t = config::FOCUS_RING_PX;
	[
		RectInstance {
			pos: [r.x, r.y],
			size: [r.w, t],
			color: c,
		},
		RectInstance {
			pos: [r.x, r.y + r.h - t],
			size: [r.w, t],
			color: c,
		},
		RectInstance {
			pos: [r.x, r.y],
			size: [t, r.h],
			color: c,
		},
		RectInstance {
			pos: [r.x + r.w - t, r.y],
			size: [t, r.h],
			color: c,
		},
	]
}

impl ApplicationHandler<UserEvent> for App {
	fn resumed(&mut self, event_loop: &ActiveEventLoop) {
		if self.state.is_some() {
			return;
		}
		let w = &self.cli.win;
		let decorated = !w.hide_frame.unwrap_or(false);
		let menu_bar = !w.hide_menu.unwrap_or(false);
		let win_title = w.title.clone();
		let win_opacity = w.opacity;
		let attrs = Window::default_attributes()
			.with_title(win_title.as_deref().unwrap_or(config::APP_NAME))
			.with_window_icon(load_icon())
			.with_decorations(decorated)
			.with_transparent(true)
			.with_inner_size(winit::dpi::LogicalSize::new(1000.0, 640.0));
		let attrs = with_app_id(attrs); // stable WM_CLASS/app_id

		// On X11 the wgpu surface can't do per-pixel alpha, so we ALWAYS take the
		// glutin GL path there (transparent-capable backend), regardless of the
		// current Transparency setting - that way the toggle works live without a
		// relaunch (the bg alpha is gated per-frame, not the backend). Off-X11 the
		// normal wgpu path is used (Wayland already supports premultiplied alpha).
		// If the GL context can't be created, fall back to the native wgpu surface.
		let want_gl = is_x11(event_loop);
		let (mut gfx, window) = match want_gl
			.then(|| Gfx::new_gl_transparent(event_loop, attrs.clone()))
			.and_then(Result::ok)
		{
			Some(pair) => pair,
			None => {
				let window = Arc::new(event_loop.create_window(attrs).expect("window"));
				let gfx = Gfx::new(window.clone()).expect("gpu init");
				(gfx, window)
			}
		};
		// System theme mode: seed the OS dark/light bit before the first frame so a
		// system-mode theme resolves to the right palette immediately (no flash).
		config::reapply_for_os(!matches!(window.theme(), Some(winit::window::Theme::Light)));
		// Window-level CLI style (--font-name/-size, colours, bg image/fit/opacity)
		// overrides the loaded settings before text + bg image are built. Applied
		// after the theme/OS palette settles so it isn't clobbered. Per-pane style
		// stays deferred (needs a per-pane renderer).
		w.apply_style();
		if w.fullscreen.unwrap_or(false) {
			window.set_fullscreen(Some(Fullscreen::Borderless(None)));
		}
		// Request compositor backdrop blur (KWin/picom) if the setting is on; no-op
		// off-X11 and on compositors that don't honor the hint.
		set_blur_behind(&window, config::settings().transparent_background_blur);

		// Transparency only ever affects the terminal background (per-pixel), never
		// the whole window - so there's no compositor whole-window-opacity fallback.
		let scale = window.scale_factor() as f32;
		let mut text = TextCtx::new(&gfx.device, &gfx.queue, gfx.format, scale);
		let menu_buffer = text.new_buffer(400.0, 400.0);
		let rects = RectRenderer::new(&gfx.device, gfx.format);
		let bg_image = load_bg_image(&gfx);
		let glow =
			crate::glow::Glow::new(&gfx.device, gfx.format, gfx.config.width, gfx.config.height);

		// Resize to the configured initial grid now that cell metrics are known.
		// cell_w/cell_h/margin are physical px; floor() in content_dims gives the
		// exact column/row count at this size. If the request applies
		// synchronously winit returns the new size (no Resized event), so adopt
		// it here; otherwise a Resized event reconfigures the surface.
		let s = config::settings();
		// CLI columns/rows override config; --pixel-width/height override either
		// dimension directly. Add the menu-bar height (when shown) so the content
		// still gets the requested row count (the tab bar only appears with >1 tab).
		// remember_size launches at the last actual size; CLI columns/rows still override
		let cols = w.columns.unwrap_or(if s.remember_size {
			s.remembered_columns
		} else {
			s.columns
		});
		let rows = w.rows.unwrap_or(if s.remember_size {
			s.remembered_rows
		} else {
			s.rows
		});
		let mb_h = if menu_bar {
			text.cell_h + MENU_BAR_VPAD
		} else {
			0.0
		};
		let want = winit::dpi::PhysicalSize::new(
			w.pixel_width
				.unwrap_or_else(|| (cols as f32 * text.cell_w + 2.0 * text.margin).ceil() as u32),
			w.pixel_height.unwrap_or_else(|| {
				(rows as f32 * text.cell_h + 2.0 * text.margin + mb_h).ceil() as u32
			}),
		);
		let mut glow = glow;
		if let Some(applied) = window.request_inner_size(want) {
			gfx.resize(applied.width, applied.height);
			glow.resize(&gfx.device, applied.width, applied.height);
		}

		// initial content area, inset by the menu bar (when shown) and the tab
		// bar (when the CLI makes >1 tab), so panes start correctly sized.
		let n_tabs = if self.cli.hierarchical {
			self.cli.tabs.len().max(1)
		} else {
			1
		};
		let top = mb_h
			+ if n_tabs > 1 {
				text.cell_h + TAB_BAR_VPAD
			} else {
				0.0
			};
		let area = Rect {
			x: 0.0,
			y: top,
			w: gfx.config.width as f32,
			h: (gfx.config.height as f32 - top).max(1.0),
		};
		let list = build_layout(&self.cli, &mut text, &self.proxy, area);

		self.state = Some(State {
			window,
			gfx,
			text,
			rects,
			bg_image,
			glow,
			tabs: Tabs { list, active: 0 },
			mods: ModifiersState::empty(),
			mouse: (0.0, 0.0),
			selecting: None,
			last_click: None,
			resizing: None,
			dragging_pane: None,
			cursor_icon: CursorIcon::Default,
			clipboard: Clipboard::new(),
			last_frame: Instant::now(),
			dirty: true,
			bell_flash: 0.0,
			overwrite: false,
			size_tracked: false,
			menu: None,
			menu_buffer,
			decorated,
			menu_bar,
			bar_open: None,
			quit: false,
			win_opacity,
			win_title,
			pending_about: false,
			pending_settings: false,
		});
	}

	fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
		let Some(state) = self.state.as_mut() else {
			return;
		};
		match event {
			UserEvent::Wakeup => {
				// output easing is triggered in Pane::build when the screen
				// actually scrolls, not on every content change
				state.dirty = true;
			}
			UserEvent::PtyWrite(id, bytes) => {
				if let Some(p) = state.tabs.cur().panes.get(&id) {
					p.term.write(bytes);
				}
			}
			UserEvent::Title(id, t) => {
				if let Some(p) = state.tabs.cur_mut().panes.get_mut(&id) {
					p.title = t;
				}
				if id == state.tabs.cur().focused {
					state.update_title();
				}
			}
			UserEvent::Exit(id) => {
				// A shell exited: close just its pane, not the whole app. The pane
				// may live in any tab (a background tab's shell can exit too), so
				// find its owner. Last pane in that tab -> close the tab; last pane
				// of the last tab -> quit. Mirrors the Close-Pane menu cascade.
				let area = state.area();
				if let Some(ti) = state
					.tabs
					.list
					.iter()
					.position(|pm| pm.panes.contains_key(&id))
				{
					if state.tabs.list[ti].panes.len() > 1 {
						state.tabs.list[ti].close(&mut state.text, id, area);
					} else if state.tabs.len() > 1 {
						state.close_tab_at(ti);
					} else {
						event_loop.exit();
					}
				}
				state.dirty = true;
			}
			UserEvent::Bell => {
				// Visual bell: brighten all text, then smoothly fade back (render).
				state.bell_flash = 1.0;
				state.dirty = true;
			}
		}
	}

	fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
		// route events for a pop-out dialog window to its own handler
		if self.dialog.as_ref().is_some_and(|d| d.id() == id) {
			self.handle_dialog_event(event);
			return;
		}
		let Some(state) = self.state.as_mut() else {
			return;
		};
		match event {
			WindowEvent::CloseRequested => event_loop.exit(),

			WindowEvent::Resized(size) => {
				state.gfx.resize(size.width, size.height);
				state
					.glow
					.resize(&state.gfx.device, size.width, size.height);
				state.relayout_all();
				state.save_window_size(size.width, size.height);
				state.dirty = true;
			}

			WindowEvent::ModifiersChanged(m) => {
				state.mods = m.state();
				// Alt toggles the menu-bar accelerator underlines, so redraw.
				state.dirty = true;
			}

			// OS switched dark/light: a "System" theme follows it live.
			WindowEvent::ThemeChanged(theme) => {
				let dark = !matches!(theme, winit::window::Theme::Light);
				if config::reapply_for_os(dark) {
					state.dirty = true;
				}
			}

			WindowEvent::CursorMoved { position, .. } => {
				state.mouse = (position.x as f32, position.y as f32);
				let (x, y) = state.mouse;
				// hovering a different top-level title with a bar menu open
				// switches to it (standard menu-bar behaviour)
				if state.bar_open.is_some() && y < state.menu_bar_h() {
					if let Some(i) = state.menubar_hit(x) {
						if state.bar_open != Some(i) {
							state.open_bar_menu(i);
							state.dirty = true;
						}
					}
				}
				if let Some(menu) = &mut state.menu {
					let h = menu.item_at(x, y);
					if h != menu.hover {
						menu.hover = h;
						state.dirty = true;
					}
				}
				if let Some(path) = state.resizing.clone() {
					// drag a pane divider
					let area = state.area();
					state
						.tabs
						.cur_mut()
						.drag_divider(&mut state.text, &path, area, x, y);
					state.dirty = true;
				} else if let Some(id) = state.selecting {
					// extend an in-progress drag-selection
					if let Some(p) = state.tabs.cur().panes.get(&id) {
						if let Some((point, side)) = p.point_at(x, y, &state.text) {
							p.update_selection(point, side);
						}
					}
					state.dirty = true;
				} else if state.dragging_pane.is_some() {
					// redraw the drop-target highlight as the cursor moves
					state.dirty = true;
				} else {
					// show a resize cursor when hovering a divider
					let area = state.area();
					let icon = match state.tabs.cur().divider_at(x, y, area) {
						Some((_, Dir::Vertical)) => CursorIcon::ColResize,
						Some((_, Dir::Horizontal)) => CursorIcon::RowResize,
						None => CursorIcon::Default,
					};
					if icon != state.cursor_icon {
						state.window.set_cursor(icon);
						state.cursor_icon = icon;
					}
				}
			}

			WindowEvent::MouseInput {
				state: ElementState::Pressed,
				button,
				..
			} => {
				let (x, y) = state.mouse;
				// click on the menu bar: toggle/open the top-level menu's dropdown
				if button == MouseButton::Left && state.menu_bar && y < state.menu_bar_h() {
					match (state.menubar_hit(x), state.bar_open) {
						(Some(i), Some(o)) if i == o => {
							state.menu = None;
							state.bar_open = None;
						}
						(Some(i), _) => state.open_bar_menu(i),
						(None, _) => {
							state.menu = None;
							state.bar_open = None;
						}
					}
					state.dirty = true;
					return;
				}
				// click on the tab bar selects a tab
				let tby = state.menubar_h();
				if button == MouseButton::Left
					&& state.tabs.len() > 1
					&& y >= tby && y < tby + state.tab_bar_h()
				{
					let tw = (state.gfx.config.width as f32 / state.tabs.len() as f32).min(220.0);
					let i = (x / tw).floor() as usize;
					if i < state.tabs.len() {
						state.tabs.active = i;
						state.update_title();
						state.dirty = true;
					}
					return;
				}
				match button {
					MouseButton::Left => {
						if let Some(menu) = state.menu.take() {
							// click an item to act, click anywhere else to dismiss
							state.bar_open = None;
							if let Some(i) = menu.item_at(x, y) {
								if let Entry::Item { action, .. } = &menu.entries[i] {
									state.apply_menu(*action, menu.target, &self.proxy);
								}
							}
							if state.quit {
								event_loop.exit();
								return;
							}
							state.dirty = true;
						} else if let Some((path, _)) =
							state.tabs.cur().divider_at(x, y, state.area())
						{
							// grab a divider to resize instead of selecting
							state.resizing = Some(path);
						} else if state.mods.shift_key() {
							// Shift+drag a pane to reorder it
							if let Some(id) = state.tabs.cur().pane_at(x, y) {
								state.focus_at(x, y);
								state.dragging_pane = Some(id);
								state.window.set_cursor(CursorIcon::Grabbing);
								state.cursor_icon = CursorIcon::Grabbing;
							}
						} else {
							state.focus_at(x, y);
							// double-click selects a word, Ctrl a rectangle,
							// else a plain run
							let now = Instant::now();
							let (cw, ch) = (state.text.cell_w, state.text.cell_h);
							let double = state.last_click.is_some_and(|(t, lx, ly)| {
								now.duration_since(t) < Duration::from_millis(400)
									&& (x - lx).abs() <= cw && (y - ly).abs() <= ch
							});
							state.last_click = Some((now, x, y));
							let pairs = if double {
								config::selection_pairs()
							} else {
								Vec::new()
							};
							let ctrl = state.mods.control_key();
							let started = state.tabs.cur().pane_at(x, y).and_then(|id| {
								let p = state.tabs.cur().panes.get(&id)?;
								let (point, side) = p.point_at(x, y, &state.text)?;
								if double {
									// inside a matched pair -> select its contents; else word
									match p.pair_span(point, &pairs) {
										Some((start, end)) => {
											p.begin_selection(
												start,
												Side::Left,
												SelectionType::Simple,
											);
											p.update_selection(end, Side::Right);
										}
										None => {
											p.begin_selection(point, side, SelectionType::Semantic)
										}
									}
								} else {
									let ty = if ctrl {
										SelectionType::Block
									} else {
										SelectionType::Simple
									};
									p.begin_selection(point, side, ty);
								}
								Some(id)
							});
							if started.is_some() {
								state.selecting = started;
								state.dirty = true;
							}
						}
					}
					MouseButton::Middle => {
						// paste the primary selection into the pane under the cursor
						if let Some(text) = state.clipboard.get_primary() {
							let id = state
								.tabs
								.cur()
								.pane_at(x, y)
								.unwrap_or(state.tabs.cur().focused);
							if let Some(p) = state.tabs.cur().panes.get(&id) {
								p.paste(&text);
							}
						}
					}
					MouseButton::Right => {
						if let Some(id) = state.tabs.cur().pane_at(x, y) {
							state.open_menu(id, x, y);
							state.dirty = true;
						}
					}
					_ => {}
				}
			}

			WindowEvent::MouseInput {
				state: ElementState::Released,
				button: MouseButton::Left,
				..
			} => {
				state.resizing = None;
				// drop a dragged pane onto the pane under the cursor (swap)
				if let Some(src) = state.dragging_pane.take() {
					let (x, y) = state.mouse;
					let area = state.area();
					if let Some(tid) = state.tabs.cur().pane_at(x, y) {
						state
							.tabs
							.cur_mut()
							.swap_panes(&mut state.text, src, tid, area);
					}
					state.window.set_cursor(CursorIcon::Default);
					state.cursor_icon = CursorIcon::Default;
					state.dirty = true;
				}
				// finish a drag-select: copy to primary, or clear if it was a click
				if let Some(id) = state.selecting.take() {
					let text = state.tabs.cur().panes.get(&id).and_then(|p| {
						let t = p.selection_text();
						if t.is_none() {
							p.clear_selection();
						}
						t
					});
					match text {
						Some(t) => state.clipboard.set_primary(t),
						None => state.dirty = true,
					}
				}
			}

			WindowEvent::MouseWheel { delta, .. } => {
				let (x, y) = state.mouse;
				let id = state
					.tabs
					.cur()
					.pane_at(x, y)
					.unwrap_or(state.tabs.cur().focused);
				let cell_h = state.text.cell_h;
				// smooth scrollback uses WHEEL_LINES; full-screen apps get their
				// own (tunable) lines-per-notch via ALT_SCROLL_LINES
				let (lines, alt_lines) = match delta {
					MouseScrollDelta::LineDelta(_, y) => (
						y * config::settings().wheel_lines,
						y * config::settings().alt_scroll_lines,
					),
					MouseScrollDelta::PixelDelta(p) => {
						let l = p.y as f32 / cell_h;
						(l, l)
					}
				};
				if let Some(p) = state.tabs.cur_mut().panes.get_mut(&id) {
					let m = p.term.mode();
					// Alternate-scroll (DECSET 1007) is default-on, so gate the cursor-key
					// path on actually being in the alt screen. On the primary screen the
					// wheel must scroll our scrollback; sending cursor keys there recalls
					// shell history instead (the reported bug).
					let alt_scroll = m.contains(TermMode::ALT_SCREEN)
						&& m.contains(TermMode::ALTERNATE_SCROLL)
						&& !m.intersects(TermMode::MOUSE_MODE);
					if alt_scroll {
						// full-screen apps (less, nano, ...) have no scrollback of
						// their own; the wheel drives their cursor-key scrolling
						let n = alt_lines.abs().round() as i32;
						if n > 0 {
							let letter = if alt_lines > 0.0 { b'A' } else { b'B' };
							let seq = input::cursor_seq(letter, m.contains(TermMode::APP_CURSOR));
							let mut bytes = Vec::with_capacity(seq.len() * n as usize);
							for _ in 0..n {
								bytes.extend_from_slice(&seq);
							}
							p.term.write(bytes);
						}
					} else {
						p.scroll.wheel(lines);
					}
				}
				state.dirty = true;
			}

			WindowEvent::KeyboardInput { event: key, .. } if key.state == ElementState::Pressed => {
				// An open menu (context menu / menu-bar dropdown) captures the
				// navigation keys - they drive the menu, not the terminal pane.
				if state.menu.is_some() {
					match &key.logical_key {
						Key::Named(NamedKey::Escape) => {
							state.menu = None;
							state.bar_open = None;
						}
						Key::Named(NamedKey::ArrowDown) => {
							if let Some(m) = state.menu.as_mut() {
								m.hover = m.step(m.hover, 1);
							}
						}
						Key::Named(NamedKey::ArrowUp) => {
							if let Some(m) = state.menu.as_mut() {
								m.hover = m.step(m.hover, -1);
							}
						}
						// Left/Right cycle between menu-bar dropdowns (no-op for a
						// right-click context menu, which isn't bar-anchored)
						Key::Named(NamedKey::ArrowLeft) | Key::Named(NamedKey::ArrowRight) => {
							if let Some(cur) = state.bar_open {
								let n = MENU_BAR.len();
								let next =
									if matches!(key.logical_key, Key::Named(NamedKey::ArrowLeft)) {
										(cur + n - 1) % n
									} else {
										(cur + 1) % n
									};
								state.open_bar_menu(next);
							}
						}
						Key::Named(NamedKey::Enter) => {
							if let Some(m) = state.menu.take() {
								state.bar_open = None;
								if let Some(Entry::Item { action, .. }) =
									m.hover.map(|i| &m.entries[i])
								{
									state.apply_menu(*action, m.target, &self.proxy);
								}
								if state.quit {
									event_loop.exit();
									return;
								}
							}
						}
						// accelerator: a letter activates the first item starting with it
						Key::Character(s) => {
							let ch = s.chars().next().map(|c| c.to_ascii_lowercase());
							let hit = ch.and_then(|ch| {
								state.menu.as_ref().and_then(|m| {
									m.entries.iter().position(|e| {
										matches!(e, Entry::Item { label, .. }
											if label.chars().next().map(|c| c.to_ascii_lowercase()) == Some(ch))
									})
								})
							});
							if let Some(i) = hit {
								if let Some(m) = state.menu.take() {
									state.bar_open = None;
									if let Entry::Item { action, .. } = &m.entries[i] {
										state.apply_menu(*action, m.target, &self.proxy);
									}
									if state.quit {
										event_loop.exit();
										return;
									}
								}
							}
						}
						_ => {}
					}
					state.dirty = true;
					return;
				}
				// Ctrl+, opens settings
				if state.mods.control_key()
					&& !state.mods.shift_key()
					&& matches!(&key.logical_key, Key::Character(s) if s == ",")
				{
					state.open_settings();
					return;
				}
				if matches!(&key.logical_key, Key::Named(NamedKey::F11)) {
					state.toggle_fullscreen();
					return;
				}
				// Menu/Apps key opens the context menu on the focused pane
				if matches!(&key.logical_key, Key::Named(NamedKey::ContextMenu)) {
					let id = state.tabs.cur().focused;
					if let Some(p) = state.tabs.cur().panes.get(&id) {
						let (rx, ry) = (p.rect.x, p.rect.y);
						state.open_menu(id, rx + 12.0, ry + 12.0);
						state.dirty = true;
					}
					return;
				}
				// Menu accelerators: Alt+F/E/V/T/P/H open the matching top-level
				// menu. NOTE: this shadows the shell's Meta+<those letters>
				// (e.g. Meta-f word-forward) - the standard menu-bar tradeoff.
				if state.menu_bar && state.mods.alt_key() && !state.mods.control_key() {
					if let Key::Character(s) = &key.logical_key {
						if let Some(ch) = s.chars().next().map(|c| c.to_ascii_uppercase()) {
							if let Some(i) = MENU_BAR.iter().position(|t| t.starts_with(ch)) {
								state.open_bar_menu(i);
								state.dirty = true;
								return;
							}
						}
					}
				}
				// tab hotkeys (Ctrl based). Close-tab has no hotkey by design
				// (use the menu / right-click / exit the shell).
				if state.mods.control_key() {
					let shift = state.mods.shift_key();
					match &key.logical_key {
						// Ctrl+Shift+T: new tab (Shift so plain Ctrl+T reaches the shell)
						Key::Character(s) if shift && s.eq_ignore_ascii_case("t") => {
							state.new_tab(&self.proxy);
							return;
						}
						Key::Named(NamedKey::PageUp) => {
							if shift {
								state.tabs.move_active(false)
							} else {
								state.tabs.prev()
							}
							state.update_title();
							state.dirty = true;
							return;
						}
						Key::Named(NamedKey::PageDown) => {
							if shift {
								state.tabs.move_active(true)
							} else {
								state.tabs.next()
							}
							state.update_title();
							state.dirty = true;
							return;
						}
						_ => {}
					}
				}
				if state.handle_hotkey(&key, &self.proxy) {
					state.dirty = true;
					return;
				}
				// Insert key toggles SilkTerm's Insert(bar)/Overwrite(block) cursor
				// mode; it still falls through to the shell (readline can follow).
				if matches!(&key.logical_key, Key::Named(NamedKey::Insert))
					&& !state.mods.shift_key()
					&& !state.mods.control_key()
					&& !state.mods.alt_key()
				{
					state.overwrite = !state.overwrite;
					state.dirty = true;
				}
				let focused = state.tabs.cur().focused;
				let app_cursor = state
					.tabs
					.cur()
					.panes
					.get(&focused)
					.map(|p| p.term.mode().contains(TermMode::APP_CURSOR))
					.unwrap_or(false);
				if let Some(bytes) = input::encode(&key, state.mods, app_cursor) {
					if let Some(p) = state.tabs.cur_mut().panes.get_mut(&focused) {
						if !p.read_only {
							p.scroll.jump_bottom();
							p.term.write(bytes);
						}
					}
					state.dirty = true;
				}
			}

			WindowEvent::RedrawRequested => {
				let _ = state.render(true);
			}

			_ => {}
		}
	}

	// request_redraw isn't reliable under some compositors, so we drive frames
	// here: render when something changed or an animation is in flight, and
	// poll only while animating (otherwise sleep until the next event).
	fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
		// cicd profiler: in profile mode run for SILK_PROFILE_SECS then exit, so
		// main can dump the flamegraph (the workload runs in the startup pane).
		#[cfg(feature = "profiling")]
		if std::env::var_os("SILK_PROFILE_OUT").is_some() {
			let now = std::time::Instant::now();
			let deadline = *self
				.profile_deadline
				.get_or_insert_with(|| now + std::time::Duration::from_secs(self.profile_secs));
			if now >= deadline {
				event_loop.exit();
				return;
			}
		}

		// Open the About window if requested (window creation needs the event loop,
		// so State only signals and we act here).
		let open_about = self
			.state
			.as_mut()
			.map_or(false, |s| std::mem::take(&mut s.pending_about));
		if open_about {
			if let Some(info) = self.state.as_ref().map(|s| s.gfx.adapter_info.clone()) {
				match crate::dialog::DialogWin::new_about(event_loop, &info) {
					Ok(d) => {
						self.dialog = Some(d);
						self.dialog_dirty = true;
					}
					Err(e) => eprintln!("{}: About window failed: {e}", config::APP_NAME),
				}
			}
		}
		let open_settings = self
			.state
			.as_mut()
			.map_or(false, |s| std::mem::take(&mut s.pending_settings));
		if open_settings {
			match crate::dialog::DialogWin::new_settings(event_loop) {
				Ok(d) => {
					self.dialog = Some(d);
					self.dialog_dirty = true;
				}
				Err(e) => eprintln!("{}: Settings window failed: {e}", config::APP_NAME),
			}
		}
		if self.dialog_dirty {
			if let Some(d) = &mut self.dialog {
				d.render();
			}
			self.dialog_dirty = false;
		}

		let Some(state) = self.state.as_mut() else {
			return;
		};
		let scroll_anim = state
			.tabs
			.cur()
			.panes
			.values()
			.any(|p| p.scroll.animating());
		let cursor_anim = state.tabs.cur().panes.values().any(|p| p.cursor_animating);
		let bell_anim = state.bell_flash > 0.0;
		let flow = if state.dirty || scroll_anim || cursor_anim || bell_anim {
			// content/scroll/bell changed this frame -> panes re-shape; a pure cursor
			// animation frame (only cursor_anim) lets them reuse the cached frame.
			let force = state.dirty || scroll_anim || bell_anim;
			state.dirty = false;
			if state.render(force) {
				// Scroll (the flagship smooth feature), a bell flash, and fresh
				// content render at full rate; a lone idle cursor blink is capped to
				// ~30fps so it isn't re-shaping text every frame just to pulse.
				if scroll_anim || bell_anim || state.dirty {
					ControlFlow::Poll
				} else {
					ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(33))
				}
			} else {
				ControlFlow::Wait
			}
		} else {
			ControlFlow::Wait
		};
		// Profiling keeps the loop hot so the workload is continuously exercised.
		#[cfg(feature = "profiling")]
		let flow = if std::env::var_os("SILK_PROFILE_OUT").is_some() {
			ControlFlow::Poll
		} else {
			flow
		};
		event_loop.set_control_flow(flow);
	}
}

impl State {
	// Ctrl+Shift chords for pane management. Returns true if consumed.
	// Only clipboard hotkeys live here now: pane management (split/close/cycle)
	// is menu-only by design - see the keyboard handler and design.md.
	fn handle_hotkey(
		&mut self,
		key: &winit::event::KeyEvent,
		_proxy: &EventLoopProxy<UserEvent>,
	) -> bool {
		if !(self.mods.control_key() && self.mods.shift_key()) {
			return false;
		}
		let focused = self.tabs.cur().focused;
		match &key.logical_key {
			// Ctrl+Shift+C / Ctrl+Shift+V: clipboard copy / paste
			Key::Character(s) if s.eq_ignore_ascii_case("c") => {
				if let Some(text) = self
					.tabs
					.cur()
					.panes
					.get(&focused)
					.and_then(|p| p.selection_text())
				{
					self.clipboard.set_clipboard(text);
				}
				true
			}
			Key::Character(s) if s.eq_ignore_ascii_case("v") => {
				if let Some(text) = self.clipboard.get_clipboard() {
					if let Some(p) = self.tabs.cur().panes.get(&focused) {
						p.paste(&text);
					}
				}
				true
			}
			_ => false,
		}
	}
}
