// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

use std::collections::HashMap;
use std::path::PathBuf;
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
use crate::config::DONATE_URL;
use crate::gfx::{Gfx, RectInstance, RectRenderer};
use crate::input;
use crate::pane::{CopyKind, Dir, Pane, PaneManager, Rect};
use crate::term::{PaneId, UserEvent};
use crate::text::TextCtx;

// Delayed re-assertions of "terminal stays under the dialog" after the dialog is
// focused, to win the race against the WM's own activation restacking (Compiz).
// The window must outlast the WM's raise/focus animation (Compiz fade/zoom can
// keep re-stacking for a few hundred ms), so it spans ~1.2s - a too-short window
// let the animation re-bury the terminal after the last retry (About showed this;
// Settings happened to settle in time). Each retry is one cheap X message.
const RAISE_REASSERTS: u8 = 24;
const RAISE_REASSERT_IVL: Duration = Duration::from_millis(50);

pub struct App {
	proxy: EventLoopProxy<UserEvent>,
	state: Option<State>,
	cli: crate::cli::Cli,
	// pop-out dialog window (About/Settings), if open. Its own surface + text
	// context, so it can be larger than the main window.
	dialog: Option<crate::dialog::DialogWin>,
	dialog_dirty: bool,
	// after the dialog is focused, re-assert "keep the terminal under me" a few
	// times: the WM's own activation (raising the dialog) can land just after our
	// first restack and re-bury the terminal, so a couple of delayed retries
	// settle it (see handle_dialog_event / about_to_wait).
	raise_reassert: u8,
	raise_next: Instant,
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
			raise_reassert: 0,
			raise_next: Instant::now(),
			#[cfg(feature = "profiling")]
			profile_secs: std::env::var("SILK_PROFILE_SECS")
				.ok()
				.and_then(|raw| raw.parse().ok())
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
			WindowEvent::Focused(true) => {
				// keep the terminal directly beneath us when we're activated, so
				// nothing stays wedged between the two (Compiz doesn't do this). Do
				// it now and arm delayed retries - the WM's own raise/animation of
				// the dialog can keep re-stacking for a while and re-bury the
				// terminal. We don't disarm on focus-out: the restack only positions
				// the terminal relative to us (never raises us), so retrying after
				// the user switched away can't pop the pair over another window -
				// and Compiz's animation briefly drops+restores focus, which would
				// otherwise kill the retries mid-flight.
				if let Some(d) = &self.dialog {
					d.raise_parent();
				}
				self.raise_reassert = RAISE_REASSERTS;
				self.raise_next = Instant::now() + RAISE_REASSERT_IVL;
			}
			WindowEvent::Resized(size) => {
				if let Some(d) = &mut self.dialog {
					d.resize(size.width, size.height);
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
						ElementState::Released => act = d.mouse_up(),
					}
					self.dialog_dirty = true;
				}
			}
			WindowEvent::KeyboardInput {
				event: key_event, ..
			} if key_event.state == ElementState::Pressed => {
				if let Some(d) = &mut self.dialog {
					match &key_event.logical_key {
						Key::Named(NamedKey::Escape) => act = d.key_escape(),
						Key::Named(NamedKey::Enter) => act = d.key_enter(),
						Key::Named(NamedKey::Tab) => d.key_tab(),
						Key::Named(NamedKey::PageUp) => d.key_page(false),
						Key::Named(NamedKey::PageDown) => d.key_page(true),
						Key::Named(NamedKey::Backspace) => d.backspace(),
						Key::Named(NamedKey::Space) => act = d.key_space(),
						Key::Named(NamedKey::ArrowUp) => d.focus_vertical(false),
						Key::Named(NamedKey::ArrowDown) => d.focus_vertical(true),
						Key::Named(NamedKey::ArrowLeft) => d.key_horizontal(-1),
						Key::Named(NamedKey::ArrowRight) => d.key_horizontal(1),
						Key::Named(
							nav_key @ (NamedKey::Home | NamedKey::End | NamedKey::Delete),
						) => d.edit_nav(*nav_key),
						Key::Character(typed) => {
							for c in typed.chars() {
								if let Some(action) = d.key_char(c) {
									act = Some(action);
								}
							}
						}
						_ => {}
					}
					self.dialog_dirty = true;
				}
			}
			WindowEvent::ModifiersChanged(mods) => {
				if let Some(d) = &mut self.dialog {
					let mod_state = mods.state();
					d.set_mods(
						mod_state.alt_key(),
						mod_state.shift_key(),
						mod_state.control_key(),
					);
					self.dialog_dirty = true;
				}
			}
			WindowEvent::MouseWheel { delta, .. } => {
				if let Some(d) = &mut self.dialog {
					let dy = match delta {
						MouseScrollDelta::LineDelta(_, y) => y * 40.0,
						MouseScrollDelta::PixelDelta(pos) => pos.y as f32,
					};
					d.wheel(dy);
					self.dialog_dirty = true;
				}
			}
			_ => {}
		}
		if let Some(action) = act {
			self.apply_dialog_action(action);
		}
	}

	fn apply_dialog_action(&mut self, action: crate::dialog::DialogAction) {
		use crate::dialog::DialogAction as DA;
		match action {
			DA::OpenUrl(u) => open_url(&u),
			DA::Close => self.dialog = None,
			DA::Apply => {
				self.apply_dialog_settings();
			}
			DA::ApplyAndClose => {
				// Only close on OK if the save actually landed; if the file looked
				// open elsewhere the change applied live but wasn't written, so we
				// keep the dialog up (the FYI went to stderr).
				if self.apply_dialog_settings() {
					self.dialog = None;
				}
			}
		}
	}

	// Pull the edited Settings from the dialog window and live-apply them to the
	// main window (config + persist + rebuild). The dialog has its own surface,
	// so it's unaffected.
	// Returns true when the change was written to disk (false = file open elsewhere,
	// applied live but not saved - OK then leaves the dialog open).
	fn apply_dialog_settings(&mut self) -> bool {
		let mut wrote = true;
		if let Some((orig, edited, sys)) = self.dialog.as_ref().and_then(|d| d.settings_values()) {
			if let Some(state) = self.state.as_mut() {
				wrote = state.apply_settings_values(&orig, edited, sys);
			}
			// Reverted-to-default keys: after persist wrote the diffs, comment
			// them back out so the file returns to the template's default line.
			// Skip when the write was deferred (revert_keys would just no-op busy).
			if wrote {
				if let Some(reverted) = self.dialog.as_mut().map(|d| d.take_reverted()) {
					config::revert_keys(&reverted);
				}
			}
			// The applied values are the new baseline, so a later Apply diffs against
			// the live state (without this, re-selecting the open-time value - e.g.
			// Bg fit back to Stretch - reads as "no change" and isn't re-applied).
			if let Some(d) = self.dialog.as_mut() {
				d.commit_baseline();
			}
			self.dialog_dirty = true;
		}
		wrote
	}
}

#[derive(Clone, Copy)]
enum MenuAction {
	Copy,
	Paste,
	PasteSelection,
	ToggleReadOnly,
	ToggleCopySelect,
	ToggleCopyOutput,
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
	Support,
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
		let rows: f32 = self.entries.iter().map(|entry| self.entry_h(entry)).sum();
		rows + config::MENU_ITEM_PAD_Y * 2.0
	}
	fn entry_h(&self, entry: &Entry) -> f32 {
		match entry {
			Entry::Sep => config::MENU_SEP_H,
			Entry::Item { .. } => self.item_h,
		}
	}
	fn row_top(&self, i: usize) -> f32 {
		self.y
			+ config::MENU_ITEM_PAD_Y
			+ self.entries[..i]
				.iter()
				.map(|entry| self.entry_h(entry))
				.sum::<f32>()
	}
	fn item_at(&self, mx: f32, my: f32) -> Option<usize> {
		if mx < self.x || mx >= self.x + self.w {
			return None;
		}
		let mut y = self.y + config::MENU_ITEM_PAD_Y;
		for (i, entry) in self.entries.iter().enumerate() {
			let h = self.entry_h(entry);
			if my >= y && my < y + h {
				return matches!(entry, Entry::Item { .. }).then_some(i);
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

// Shaped chrome text, kept frame to frame: menu-bar titles + the copybox label,
// the tab close-"x", and per-tab title buffers. Re-shaping these every rendered
// frame was constant background work during any animation (even the idle cursor
// pulse). Rebuilt when the menu colour changes; a tab entry re-shapes only when
// its title or the tab width changes; the whole cache is dropped on a
// text-context rebuild (buffers are tied to the FontSystem they were made with).
struct ChromeCache {
	menu_fg: [u8; 3],
	menubar: Vec<Buffer>, // MENU_BAR titles + trailing "Copy output" label
	close: Buffer,
	close_w: f32, // advance of the "x" glyph, for centering it in the button box
	tab_w: f32,
	tabs: Vec<(String, Buffer)>,
}

// The menu bar's right-side copy-mode cluster: "Copy on [ ] select [ ] output".
// Drawing, label placement, and click hit-testing all read this one layout.
struct CopyBoxes {
	boxes: [Rect; 2],  // select, output checkbox squares
	label_x: [f32; 3], // left edge per COPYBOX_LABELS entry
	label_w: [f32; 3],
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
	// PaneIds are globally unique; the pane may live in any tab, not just the
	// active one (background-tab shells reply to ESC[6n etc. too)
	fn find_pane(&self, id: PaneId) -> Option<&Pane> {
		self.list.iter().find_map(|pm| pm.panes.get(&id))
	}
	fn find_pane_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
		self.list.iter_mut().find_map(|pm| pm.panes.get_mut(&id))
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
const TAB_BAR_VPAD: f32 = 6.0; // text is metric-centered in the bar; descenders clear via that
const BELL_TAU_S: f32 = 0.18; // visual-bell flash fade time-constant (~0.8s to settle)
const SIZE_SAVE_DEBOUNCE: Duration = Duration::from_millis(500); // remember-size settle time before hitting disk
const CAPTURE_SETTLE: Duration = Duration::from_millis(120); // copy-output: idle-at-prompt debounce marking a command done
const MENU_BAR_PAD: f32 = 10.0; // px around each top-level title
const TAB_MAX_W: f32 = 220.0; // tab button width cap - drawing AND click hit-testing use this
const TAB_CLOSE_W: f32 = 26.0; // right-edge close-button region per tab (title clips before it)
const TAB_CLOSE_M: f32 = 5.0; // balanced top/right/bottom margin around the close button box

// The close-"x" button box within a tab: a square with equal top/right/bottom
// margins (the extra room falls to the left, separating it from the title).
// Shared by the rect draw, the glyph placement, and the click hit-test so they
// can't drift apart.
fn tab_close_box(tab_x: f32, tab_w: f32, bar_y: f32, tab_h: f32) -> Rect {
	let side = (tab_h - 2.0 * TAB_CLOSE_M).max(8.0);
	Rect {
		x: tab_x + tab_w - TAB_CLOSE_M - side,
		y: bar_y + TAB_CLOSE_M,
		w: side,
		h: side,
	}
}
const MENU_BAR: [&str; 6] = ["File", "Edit", "View", "Tabs", "Panes", "Help"];
const COPYBOX_LABELS: [&str; 3] = ["Copy on:", "select", "output"]; // menu-bar auto-copy checkboxes

struct State {
	window: Arc<Window>,
	gfx: Gfx,
	text: TextCtx,
	rects: RectRenderer,
	bg_image: Option<ImageRenderer>,
	scrim: crate::scrim::Scrim, // text readability scrim (used only when config.text_scrim)
	tabs: Tabs,
	mods: ModifiersState,
	mouse: (f32, f32),
	mouse_btn: Option<input::MouseBtn>, // button held after a reported press (mouse-tracking apps)
	mouse_cell: Option<(usize, usize)>, // last cell reported, to de-dupe motion
	selecting: Option<PaneId>,          // pane with an in-progress drag-select
	last_click: Option<(Instant, f32, f32)>, // for multi-click detection
	click_count: u32,                   // consecutive clicks in the same spot (2=double, 3=triple)
	resizing: Option<Vec<bool>>,        // split-tree path of the divider being dragged
	dragging_pane: Option<PaneId>,      // pane being drag-reordered (Shift+drag)
	cursor_icon: CursorIcon,
	clipboard: Clipboard,
	last_frame: Instant,
	dirty: bool,
	bell_flash: f32,    // visual-bell brightness, set to 1.0 on BEL, decays to 0
	size_tracked: bool, // false until the first frame, so startup/programmatic resizes don't overwrite remembered_size
	pending_size: Option<(usize, usize)>, // debounced remember-size: persisted after the size holds, not per resize tick
	pending_size_at: Instant,
	menu: Option<ContextMenu>,
	decorated: bool,             // window frame shown (winit has no getter, so track it)
	menu_bar: bool,              // window menu bar (File/Edit/...) shown
	bar_open: Option<usize>,     // which top-level menu's dropdown is open, if any
	quit: bool,                  // set by File->Quit; the event handler exits after applying
	win_opacity: Option<f32>,    // CLI --background-opacity override (this window only)
	win_title: Option<String>,   // CLI --title override (else "AppName - <tab title>")
	last_win_title: String,      // last string set on the window (skip redundant set_title)
	focused: bool, // window has keyboard focus (gates copy-output: never copy from a background window)
	pending_about: bool, // request to open the About window (App acts on it; needs the event loop)
	pending_settings: bool, // request to open the Settings window
	chrome: Option<ChromeCache>, // shaped menu/tab text, reused across frames
	wp_images: Vec<PathBuf>, // wallpaper-rotation folder contents (empty = rotation off)
	wp_index: usize, // which of wp_images is currently shown
	wp_next: Option<Instant>, // when to rotate next (None = no timer / startup-only)
}

impl State {
	// Pixels reserved at the very top by the menu bar (0 when hidden).
	// Bar heights track the menu font's line height so they scale with font size.
	fn menu_bar_h(&self) -> f32 {
		self.text.ui_line_h + MENU_BAR_VPAD
	}
	fn tab_bar_h(&self) -> f32 {
		self.text.ui_line_h + TAB_BAR_VPAD
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

	// Mouse reporting: forward a button press/release to the pane under the cursor
	// when the app has mouse tracking on. Shift is the local-action override (so the
	// user can still select/paste/menu). Returns true when the event was reported
	// (and should not be handled locally). Records the held button for drag + release.
	fn report_mouse_button(&mut self, button: MouseButton, pressed: bool) -> bool {
		let Some(btn) = mouse_btn_of(button) else {
			return false;
		};
		// Right-click is reserved for SilkTerm's own context menu and never
		// forwarded to a mouse-tracking app (else e.g. muffer pastes on it).
		if btn == input::MouseBtn::Right {
			return false;
		}
		let (x, y) = self.mouse;
		if pressed {
			if self.mods.shift_key() {
				return false;
			}
			let cur = self.tabs.cur();
			let Some(id) = cur.pane_at(x, y) else {
				return false;
			};
			let Some(p) = cur.panes.get(&id) else {
				return false;
			};
			if !input::wants_mouse(p.mode) {
				return false;
			}
			let Some((col, row)) = p.screen_cell_at(x, y, &self.text) else {
				return false;
			};
			if let Some(seq) = input::mouse_report(p.mode, btn, true, false, col, row, self.mods) {
				p.term.write(seq);
			}
			self.mouse_btn = Some(btn);
			self.mouse_cell = Some((col, row));
			true
		} else {
			// only our business if we owned the matching press
			if self.mouse_btn.take().is_none() {
				return false;
			}
			let cur = self.tabs.cur();
			if let Some(p) = cur.pane_at(x, y).and_then(|id| cur.panes.get(&id)) {
				if input::wants_mouse(p.mode) {
					if let Some((col, row)) = p.screen_cell_at(x, y, &self.text) {
						if let Some(seq) =
							input::mouse_report(p.mode, btn, false, false, col, row, self.mods)
						{
							p.term.write(seq);
						}
					}
				}
			}
			self.mouse_cell = None;
			true
		}
	}

	// Mouse reporting: forward cursor motion when the app requests it - MOUSE_MOTION
	// (any move) or MOUSE_DRAG (only while a button is held). De-duped per cell so a
	// pixel jiggle inside one cell doesn't flood the PTY. Returns true when reported.
	fn report_mouse_motion(&mut self) -> bool {
		if self.mods.shift_key() {
			return false;
		}
		let (x, y) = self.mouse;
		let held = self.mouse_btn;
		let last = self.mouse_cell;
		let new_cell = {
			let cur = self.tabs.cur();
			let Some(id) = cur.pane_at(x, y) else {
				return false;
			};
			let Some(p) = cur.panes.get(&id) else {
				return false;
			};
			let motion = p.mode.contains(TermMode::MOUSE_MOTION);
			let drag = p.mode.contains(TermMode::MOUSE_DRAG) && held.is_some();
			if !(motion || drag) {
				return false;
			}
			let Some((col, row)) = p.screen_cell_at(x, y, &self.text) else {
				return false;
			};
			if last == Some((col, row)) {
				return false;
			}
			let btn = held.unwrap_or(input::MouseBtn::None);
			if let Some(seq) = input::mouse_report(p.mode, btn, true, true, col, row, self.mods) {
				p.term.write(seq);
			}
			(col, row)
		};
		self.mouse_cell = Some(new_cell);
		true
	}

	// Copy-output: when the focused pane's foreground command finishes, copy its
	// output text to the desktop clipboard - gated on this window being focused and
	// the pane's per-pane opt-in, so a background window/pane never exports output.
	fn poll_output_copy(&mut self) {
		if !self.focused {
			return;
		}
		let focused_id = self.tabs.cur().focused;
		let text = {
			let Some(pane) = self.tabs.cur_mut().panes.get_mut(&focused_id) else {
				return;
			};
			if !pane.copy_output {
				return;
			}
			pane.poll_capture(CAPTURE_SETTLE)
		};
		if let Some(text) = text {
			self.clipboard.set_clipboard(text);
		}
	}

	// When the focused pane is armed for copy-output, the instant its settle timer
	// should fire, so an idle loop wakes to run the capture check.
	fn capture_wake(&self) -> Option<Instant> {
		if !self.focused {
			return None;
		}
		let focused_id = self.tabs.cur().focused;
		let p = self.tabs.cur().panes.get(&focused_id)?;
		p.copy_output
			.then(|| p.capture_deadline(CAPTURE_SETTLE))
			.flatten()
	}

	// The active tab's title ("<shell> [<program>]" or a per-tab --title override).
	fn active_tab_title(&mut self) -> String {
		let pm = self.tabs.cur_mut();
		if let Some(title) = &pm.title_override {
			return title.clone();
		}
		let focused_id = pm.focused;
		pm.panes
			.get_mut(&focused_id)
			.map(|p| p.term.tab_title())
			.unwrap_or_else(|| config::APP_NAME.into())
	}

	// The window title (taskbar / alt-tab): a CLI --title override verbatim, else
	// "AppName - <active tab title>" so it tracks the focused tab's program.
	// Called on tab/focus change and each rendered frame; set_title only fires when
	// the string actually changed (avoids WM flicker / churn).
	fn update_title(&mut self) {
		let title = match &self.win_title {
			Some(custom_title) => custom_title.clone(),
			None => format!("{} - {}", config::APP_NAME, self.active_tab_title()),
		};
		if title != self.last_win_title {
			self.window.set_title(&title);
			self.last_win_title = title;
		}
	}

	// Effective window opacity: a CLI --background-opacity override for this
	// window, else the configured value.
	fn opacity(&self) -> f32 {
		self.win_opacity
			.unwrap_or_else(|| config::settings().opacity)
	}

	fn open_menu(&mut self, target: PaneId, mx: f32, my: f32) {
		let p = self.tabs.cur().panes.get(&target);
		let read_only = p.is_some_and(|p| p.read_only);
		let copy_select = p.is_some_and(|p| p.copy_select);
		let copy_output = p.is_some_and(|p| p.copy_output);
		let entries = vec![
			mi("Copy", MenuAction::Copy),
			mi("Paste", MenuAction::Paste),
			mi("Paste Selection", MenuAction::PasteSelection),
			Entry::Sep,
			mt(copy_select, "Copy on select", MenuAction::ToggleCopySelect),
			mt(copy_output, "Copy on output", MenuAction::ToggleCopyOutput),
			mt(read_only, "Read-only", MenuAction::ToggleReadOnly),
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
		let attrs = crate::text::ui_attrs();
		let mut max_label_w: f32 = 0.0;
		for entry in &entries {
			if let Entry::Item { label, .. } = entry {
				max_label_w = max_label_w.max(self.text.measure_ui_text(label, &attrs));
			}
		}
		let w = config::MENU_GUTTER + max_label_w + config::MENU_PAD_X * 2.0;
		let item_h = self.text.ui_line_h;
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
		let p = self.tabs.cur().panes.get(&self.tabs.cur().focused);
		let read_only = p.is_some_and(|p| p.read_only);
		let copy_select = p.is_some_and(|p| p.copy_select);
		let copy_output = p.is_some_and(|p| p.copy_output);
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
				mt(copy_select, "Copy on select", MenuAction::ToggleCopySelect),
				mt(copy_output, "Copy on output", MenuAction::ToggleCopyOutput),
				mt(read_only, "Read-only", MenuAction::ToggleReadOnly),
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
			_ => vec![
				mi("Support SilkTerm\u{2026}", MenuAction::Support),
				Entry::Sep,
				mi("About\u{2026}", MenuAction::About),
			],
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
		let attrs = crate::text::ui_attrs();
		let mut x = 0.0;
		let mut out = Vec::with_capacity(MENU_BAR.len());
		for title in MENU_BAR {
			let w = self.text.measure_ui_text(title, &attrs) + MENU_BAR_PAD * 2.0;
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

	// Always-visible "Copy on [ ] select [ ] output" pair on the right of the
	// menu bar (security: the user can always see when the focused pane is
	// auto-copying). label_x/label_w index-match COPYBOX_LABELS.
	fn copybox_layout(&mut self) -> CopyBoxes {
		let attrs = crate::text::ui_attrs();
		let mut label_w = [0.0f32; 3];
		for (w, label) in label_w.iter_mut().zip(COPYBOX_LABELS) {
			*w = self.text.measure_ui_text(label, &attrs);
		}
		let box_sz = (self.text.ui_line_h * 0.6).round();
		let box_y = (self.menu_bar_h() - box_sz) / 2.0;
		let right = self.gfx.config.width as f32 - MENU_BAR_PAD;
		let out_x = right - label_w[2];
		let out_box = Rect {
			x: out_x - 6.0 - box_sz,
			y: box_y,
			w: box_sz,
			h: box_sz,
		};
		let sel_x = out_box.x - 14.0 - label_w[1];
		let sel_box = Rect {
			x: sel_x - 6.0 - box_sz,
			y: box_y,
			w: box_sz,
			h: box_sz,
		};
		let lead_x = sel_box.x - 10.0 - label_w[0];
		CopyBoxes {
			boxes: [sel_box, out_box],
			label_x: [lead_x, sel_x, out_x],
			label_w,
		}
	}

	// Which copy-mode checkbox (the square or its word) a menu-bar click hit.
	fn copybox_hit(&mut self, mx: f32) -> Option<CopyKind> {
		let cb = self.copybox_layout();
		if mx >= cb.boxes[0].x && mx <= cb.label_x[1] + cb.label_w[1] {
			Some(CopyKind::Select)
		} else if mx >= cb.boxes[1].x && mx <= cb.label_x[2] + cb.label_w[2] {
			Some(CopyKind::Output)
		} else {
			None
		}
	}

	// Flip one of a pane's two auto-copy triggers. The two are independent and can
	// both be on; nothing else is touched (other panes/tabs/windows keep theirs -
	// only the focused pane of the active tab actually copies, gated at copy time).
	// A toggle from a context menu on an unfocused pane focuses it so the menu-bar
	// checkboxes reflect the pane just changed.
	fn toggle_copy(&mut self, target: PaneId, kind: CopyKind) {
		let Some(p) = self.tabs.find_pane_mut(target) else {
			return;
		};
		let now = !p.copy_enabled(kind);
		p.set_copy(kind, now);
		if self.tabs.cur().panes.contains_key(&target) {
			self.tabs.cur_mut().focused = target;
		}
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
			MenuAction::ToggleCopySelect => self.toggle_copy(target, CopyKind::Select),
			MenuAction::ToggleCopyOutput => self.toggle_copy(target, CopyKind::Output),
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
			MenuAction::Support => open_url(DONATE_URL),
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
		let px_to_cells = |px: f32, cell: f32, chrome: f32| {
			(((px - 2.0 * self.text.margin - chrome) / cell).floor() as i64).max(1) as usize
		};
		let cols = px_to_cells(w as f32, self.text.cell_w, 0.0);
		let rows = px_to_cells(h as f32, self.text.cell_h, self.menubar_h());
		// debounce: an interactive drag fires many Resized events; writing
		// config.toml on each would be dozens of file writes/sec. Persist in
		// flush_window_size once the size has held (or on exit).
		self.pending_size = Some((cols, rows));
		self.pending_size_at = Instant::now();
	}

	fn flush_window_size(&mut self, force: bool) {
		let Some((cols, rows)) = self.pending_size else {
			return;
		};
		if !force && self.pending_size_at.elapsed() < SIZE_SAVE_DEBOUNCE {
			return;
		}
		self.pending_size = None;
		let orig = (*config::settings()).clone();
		if cols == orig.remembered_columns && rows == orig.remembered_rows {
			return;
		}
		let mut new = orig.clone();
		new.remembered_columns = cols;
		new.remembered_rows = rows;
		// If the file's open elsewhere persist skips it (retried on the next resize
		// or at exit); the live size still updates in memory either way.
		let _ = config::persist(&orig, &new);
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
		let fullscreen = match self.window.fullscreen() {
			Some(_) => None,
			None => Some(Fullscreen::Borderless(None)),
		};
		self.window.set_fullscreen(fullscreen);
	}

	// Request the Settings window (App opens it; window creation needs the loop).
	fn open_settings(&mut self) {
		self.pending_settings = true;
		self.menu = None;
		self.bar_open = None;
	}

	// Live-apply edited settings (from the dialog), persist, and rebuild whatever
	// the change touched (text metrics, background image, opacity, window size).
	// Returns false if the config file looked open elsewhere so the write was
	// skipped - the caller (dialog OK) then keeps the dialog open instead of
	// closing over an unsaved change. The values still apply live regardless.
	fn apply_settings_values(
		&mut self,
		orig: &config::Settings,
		edited: config::Settings,
		_system_font: bool,
	) -> bool {
		// use_system_font is now a persisted setting that overrides font_family at
		// resolve time, so nothing special to strip - persist the diff as usual.
		let wrote = config::persist(orig, &edited);
		self.apply_new_settings(orig, edited, false);
		wrote
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

	// Control-socket wallpaper change: live-only and window-scoped, like the
	// launch-time --background-image - nothing is persisted to config.toml.
	fn set_wallpaper(&mut self, image: Option<std::path::PathBuf>) {
		let orig = config::settings().as_ref().clone();
		let mut edited = orig.clone();
		edited.background_image_raw = image
			.as_ref()
			.map(|path| path.to_string_lossy().into_owned())
			.unwrap_or_default();
		edited.background_image = image;
		self.apply_new_settings(&orig, edited, true);
	}

	// Wallpaper rotation: if background_folder is configured, scan it, show the
	// first (or a random) image now, and arm the timer if an interval is set.
	fn init_wallpaper_rotation(&mut self) {
		let settings = config::settings();
		let Some(dir) = settings.background_folder.clone() else {
			return;
		};
		self.wp_images = list_folder_images(&dir);
		if self.wp_images.is_empty() {
			eprintln!(
				"{}: background_folder {} has no images",
				config::APP_NAME,
				dir.display()
			);
			return;
		}
		self.wp_index = if settings.background_rotate_random {
			next_wallpaper_index(self.wp_images.len(), 0, true, time_entropy())
		} else {
			0
		};
		let first = self.wp_images[self.wp_index].clone();
		self.set_wallpaper(Some(first));
		let ivl = settings.background_rotate_interval_s;
		if ivl > 0.0 {
			self.wp_next = Some(Instant::now() + Duration::from_secs_f32(ivl));
		}
	}

	// Rotate to the next image (order or random) and re-arm the timer.
	fn advance_wallpaper(&mut self) {
		if self.wp_images.len() < 2 {
			// one image (or none): nothing to rotate to; keep the timer off
			self.wp_next = None;
			return;
		}
		let settings = config::settings();
		self.wp_index = next_wallpaper_index(
			self.wp_images.len(),
			self.wp_index,
			settings.background_rotate_random,
			time_entropy(),
		);
		let next = self.wp_images[self.wp_index].clone();
		self.set_wallpaper(Some(next));
		self.dirty = true;
		let ivl = settings.background_rotate_interval_s;
		self.wp_next = (ivl > 0.0).then(|| Instant::now() + Duration::from_secs_f32(ivl));
	}

	// Rebuild the text context (cell metrics, chrome, pane buffers) for a new
	// scale factor or font, then relayout. Shared by settings-driven font
	// rebuilds and DPI scale-factor changes. The surface itself is reconfigured
	// separately (a Resized event follows a scale change).
	fn rebuild_text(&mut self, scale: f32) {
		self.text = TextCtx::new(&self.gfx.device, &self.gfx.queue, self.gfx.format, scale);
		self.chrome = None; // cached chrome buffers are tied to the old FontSystem
		for pm in &mut self.tabs.list {
			pm.rebuild_buffers(&mut self.text);
		}
		self.relayout_all();
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
			let settings = config::settings();
			let want = winit::dpi::PhysicalSize::new(
				(settings.columns as f32 * self.text.cell_w + 2.0 * self.text.margin).ceil() as u32,
				(settings.rows as f32 * self.text.cell_h
					+ 2.0 * self.text.margin
					+ self.menubar_h())
				.ceil() as u32,
			);
			if let Some(applied) = self.window.request_inner_size(want) {
				self.gfx.resize(applied.width, applied.height);
			}
		}
		if rebuild {
			self.rebuild_text(self.window.scale_factor() as f32);
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
		// keep the window title tracking the active tab's foreground program
		// (deduped inside update_title, so this is cheap when nothing changed)
		self.update_title();

		let now = Instant::now();
		let dt = (now - self.last_frame).as_secs_f32().min(0.1);
		self.last_frame = now;
		let cfg = config::settings(); // one snapshot per frame, not per use/pane

		// Visual-bell flash decays toward 0; while >0 the text is brightened (in
		// build) and we keep rendering so the fade is smooth.
		if self.bell_flash > 0.0 {
			self.bell_flash = (self.bell_flash * (-dt / BELL_TAU_S).exp()).max(0.0);
			if self.bell_flash < 0.01 {
				self.bell_flash = 0.0;
			}
		}
		let bell = self.bell_flash;

		// translucent background only when the surface supports it AND the user has
		// Transparency on - and it only ever affects the bg, never text/chrome.
		let bg_alpha = if self.gfx.transparent && cfg.transparent_background {
			self.opacity()
		} else {
			1.0
		};

		let mut under: Vec<RectInstance> = Vec::new();
		// per-pane (bg cells + cursor), scissored to the pane so overscan rows
		// don't bleed into neighbours
		let mut groups: Vec<(Rect, Vec<RectInstance>)> = Vec::new();
		// cursors are drawn separately (above the scrim, so its halo can't obscure them)
		let mut cursors: Vec<(Rect, RectInstance)> = Vec::new();
		let mut tops: HashMap<u64, f32> = HashMap::new();
		// retained-frame app-scroll slide geometry per pane (None = no active slide)
		let mut slides: HashMap<u64, Option<crate::pane::Slide>> = HashMap::new();
		let mut animating = bell > 0.0;
		// text-scrim colour map needs each cell's bg (so a glyph's halo takes its
		// own cell colour, not always the global) - collect them while building
		let scrim_on = cfg.text_scrim && cfg.text_scrim_radius > 0.0;
		let mut scrim_cells: Vec<RectInstance> = Vec::new();

		for (id, pane) in self.tabs.cur_mut().panes.iter_mut() {
			pane.scroll.advance(dt);
			let rect = pane.rect;
			// scope the expensive re-shape to panes that actually changed: fresh
			// PTY output (content_dirty), an active scroll ease, or a global
			// cause (bell flash, chrome/UI change) - idle siblings reuse their
			// cached frame instead of re-shaping at the busy pane's rate
			let force = force_rebuild || pane.content_dirty || pane.scroll.animating();
			let draw = pane.build(&mut self.text, dt, bell, force);
			if pane.scroll.animating() || pane.cursor_animating {
				animating = true;
			}
			tops.insert(*id, draw.top);
			slides.insert(*id, draw.slide);
			let mut bg = config::srgb_f32(cfg.bg);
			bg[3] = bg_alpha;
			under.push(RectInstance {
				pos: [rect.x, rect.y],
				size: [rect.w, rect.h],
				color: bg,
			});
			if scrim_on {
				scrim_cells.extend_from_slice(&draw.bg);
			}
			groups.push((rect, draw.bg));
			if let Some(cursor_quad) = draw.cursor {
				cursors.push((rect, cursor_quad));
			}
		}

		let under_len = under.len() as u32;
		let mut instances = under;
		let mut group_ranges: Vec<(Rect, u32, u32)> = Vec::new();
		for (rect, bg_quads) in groups {
			let start = instances.len() as u32;
			instances.extend(bg_quads);
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
			if let Some(target_id) = self.tabs.cur().pane_at(self.mouse.0, self.mouse.1) {
				if target_id != src {
					if let Some(p) = self.tabs.cur().panes.get(&target_id) {
						let mut color = config::srgb_f32(config::DROP_TARGET);
						color[3] = 0.30;
						instances.push(RectInstance {
							pos: [p.rect.x, p.rect.y],
							size: [p.rect.w, p.rect.h],
							color,
						});
					}
				}
			}
		}
		let ring_end = instances.len() as u32;

		// cursor quads also feed the scrim's cursor-coverage texture (its own tex,
		// so cursor_scrim/cursor_outline gate it independently); the cursor still
		// draws crisp ABOVE the composite below. Collect them whenever the scrim is
		// on - the shader flags decide whether they reach the halo and/or outline.
		let scrim_cursor_quads: Vec<RectInstance> = if scrim_on {
			cursors.iter().map(|(_, q)| *q).collect()
		} else {
			Vec::new()
		};

		// cursor quads get their own per-pane ranges, drawn after the scrim composite
		let mut cursor_ranges: Vec<(Rect, u32, u32)> = Vec::new();
		for (rect, cursor_quad) in cursors {
			let start = instances.len() as u32;
			instances.push(cursor_quad);
			cursor_ranges.push((rect, start, instances.len() as u32));
		}

		let win_w = self.gfx.config.width as f32;
		let menu_h = self.menu_bar_h();
		let tab_h = self.tab_bar_h();

		// menu bar (File/Edit/...), drawn in the main pass at the very top; the
		// open menu's title is highlighted.
		let menubar_range = if self.menu_bar {
			let start = instances.len() as u32;
			instances.push(rect_inst(0.0, 0.0, win_w, menu_h, config::TAB_BAR_BG));
			let layout = self.menubar_layout();
			if let Some(idx) = self.bar_open {
				if let Some(&(x, w)) = layout.get(idx) {
					instances.push(rect_inst(x, 0.0, w, menu_h, config::menu_hover()));
				}
			} else if self.mods.alt_key() {
				// Alt held (no dropdown open): underline each title's accelerator
				// letter, like the open-dropdown items do (press the letter to open).
				let attrs = crate::text::ui_attrs();
				let underline_y = self.text.ui_baseline(0.0, menu_h) + 1.0;
				for (i, &(x, _)) in layout.iter().enumerate() {
					if let Some(c) = MENU_BAR[i].chars().next() {
						let mut buf = [0u8; 4];
						let letter_w = self.text.measure_ui_text(c.encode_utf8(&mut buf), &attrs);
						instances.push(rect_inst(
							x + MENU_BAR_PAD,
							underline_y,
							letter_w,
							1.0,
							config::menu_fg(),
						));
					}
				}
			}
			// always-visible copy-mode checkboxes (right side): outlines always,
			// filled per the focused pane's two independent triggers, so the state
			// is never hidden. Dimmed when this window isn't focused - the flags
			// stay set, but nothing copies until it regains focus.
			let fp = self.tabs.cur().panes.get(&self.tabs.cur().focused);
			let checked = [
				fp.is_some_and(|p| p.copy_select),
				fp.is_some_and(|p| p.copy_output),
			];
			let cb = self.copybox_layout();
			let border = copy_dim(config::menu_border(), self.focused);
			let fill = copy_dim(config::menu_fg(), self.focused);
			for (checkbox, on) in cb.boxes.iter().zip(checked) {
				instances.push(rect_inst(
					checkbox.x - 1.0,
					checkbox.y - 1.0,
					checkbox.w + 2.0,
					checkbox.h + 2.0,
					border,
				));
				instances.push(rect_inst(
					checkbox.x,
					checkbox.y,
					checkbox.w,
					checkbox.h,
					config::TAB_BAR_BG,
				));
				if on {
					instances.push(rect_inst(
						checkbox.x + 3.0,
						checkbox.y + 3.0,
						checkbox.w - 6.0,
						checkbox.h - 6.0,
						fill,
					));
				}
			}
			Some((start, instances.len() as u32))
		} else {
			None
		};

		// tab bar (only with >1 tab), drawn just below the menu bar
		let tab_bar_y = self.menubar_h();
		let tabbar_range = if self.tabs.len() > 1 {
			let start = instances.len() as u32;
			instances.push(rect_inst(0.0, tab_bar_y, win_w, tab_h, config::TAB_BAR_BG));
			let n = self.tabs.len();
			let tab_w = (win_w / n as f32).min(TAB_MAX_W);
			for i in 0..n {
				let x = i as f32 * tab_w;
				let color = if i == self.tabs.active {
					config::TAB_ACTIVE
				} else {
					config::TAB_INACTIVE
				};
				instances.push(rect_inst(
					x + 1.0,
					tab_bar_y + 2.0,
					tab_w - 2.0,
					tab_h - 3.0,
					color,
				));
				// close-button box: a 1px outline (border rect + inner tab-bg fill)
				let cb = tab_close_box(x, tab_w, tab_bar_y, tab_h);
				instances.push(rect_inst(
					cb.x - 1.0,
					cb.y - 1.0,
					cb.w + 2.0,
					cb.h + 2.0,
					config::menu_border(),
				));
				instances.push(rect_inst(cb.x, cb.y, cb.w, cb.h, color));
			}
			Some((start, instances.len() as u32))
		} else {
			None
		};

		// context menu quads (drawn in a second pass, on top of everything)
		let menu_range = if let Some(menu) = &self.menu {
			let start = instances.len() as u32;
			let popup_h = menu.height();
			let border = 1.0;
			instances.push(rect_inst(
				menu.x - border,
				menu.y - border,
				menu.w + 2.0 * border,
				popup_h + 2.0 * border,
				config::menu_border(),
			));
			instances.push(rect_inst(
				menu.x,
				menu.y,
				menu.w,
				popup_h,
				config::menu_bg(),
			));
			if let Some(i) = menu.hover {
				instances.push(rect_inst(
					menu.x,
					menu.row_top(i),
					menu.w,
					menu.item_h,
					config::menu_hover(),
				));
			}
			// faint separator lines between logical groups
			for (i, entry) in menu.entries.iter().enumerate() {
				if matches!(entry, Entry::Sep) {
					let sep_y = menu.row_top(i) + config::MENU_SEP_H / 2.0;
					instances.push(rect_inst(
						menu.x + config::MENU_PAD_X,
						sep_y,
						menu.w - config::MENU_PAD_X * 2.0,
						1.0,
						config::menu_sep(),
					));
				}
			}
			// accelerator underline under each item's first letter (press it to pick)
			let acc_attrs = crate::text::ui_attrs();
			let line_h = self.text.ui_line_h;
			let acc_x = menu.x + config::MENU_PAD_X + config::MENU_GUTTER;
			for (i, entry) in menu.entries.iter().enumerate() {
				if let Entry::Item { label, .. } = entry {
					if let Some(c) = label.chars().next() {
						let mut buf = [0u8; 4];
						let letter_w = self
							.text
							.measure_ui_text(c.encode_utf8(&mut buf), &acc_attrs);
						let top = menu.row_top(i) + (menu.item_h - line_h) / 2.0;
						instances.push(rect_inst(
							acc_x,
							top + line_h - 3.0,
							letter_w,
							1.0,
							config::menu_fg(),
						));
					}
				}
			}
			Some((start, instances.len() as u32))
		} else {
			None
		};

		let overlay_range = menu_range;

		let margin = self.text.margin;
		let menu_fg_rgb = config::menu_fg();
		let menu_fg = GColor::rgb(menu_fg_rgb[0], menu_fg_rgb[1], menu_fg_rgb[2]);
		// copy-mode labels dim with their checkboxes when the window is unfocused
		let copy_label_fg = {
			let c = copy_dim(menu_fg_rgb, self.focused);
			GColor::rgb(c[0], c[1], c[2])
		};
		// subtle close-"x" glyph colour, dimmed toward the tab bg (~0.6)
		let close_fg = {
			let dim = |v: u8| ((v as u16 * 3) / 5) as u8;
			GColor::rgb(
				dim(menu_fg_rgb[0]),
				dim(menu_fg_rgb[1]),
				dim(menu_fg_rgb[2]),
			)
		};
		// tab titles ("<shell> [<program>]") - computed first (tab_title is &mut)
		// before self.text is borrowed for the buffers below
		let tab_titles: Vec<String> = if self.tabs.len() > 1 {
			self.tabs
				.list
				.iter_mut()
				.map(|pm| {
					if let Some(title) = &pm.title_override {
						return title.clone();
					}
					let focused_id = pm.focused;
					pm.panes
						.get_mut(&focused_id)
						.map(|p| p.term.tab_title())
						.unwrap_or_else(|| config::APP_NAME.into())
				})
				.collect()
		} else {
			Vec::new()
		};
		let tab_w = (self.gfx.config.width as f32 / self.tabs.len().max(1) as f32).min(TAB_MAX_W);
		// keep the shaped chrome text current (see ChromeCache) - a colour change
		// rebuilds it all, otherwise only changed tab titles re-shape
		if self
			.chrome
			.as_ref()
			.is_some_and(|cache| cache.menu_fg != menu_fg_rgb)
		{
			self.chrome = None;
		}
		if self.chrome.is_none() {
			let shape_ui = |text: &mut TextCtx, s: &str, w: f32, h: f32, color: GColor| {
				let mut buf = text.new_ui_buffer(w, h);
				let mut attrs = crate::text::ui_attrs();
				attrs.color_opt = Some(color);
				buf.set_text(&mut text.font_system, s, &attrs, Shaping::Advanced, None);
				buf.shape_until_scroll(&mut text.font_system, false);
				buf
			};
			// menu-bar titles (one per top-level menu) plus the trailing
			// "Copy on / select / output" labels for the always-visible checkboxes
			let menubar = MENU_BAR
				.iter()
				.chain(COPYBOX_LABELS.iter())
				.map(|title| shape_ui(&mut self.text, title, 240.0, menu_h, menu_fg))
				.collect();
			// the close "x" is bold so it reads as a button glyph
			let close = {
				let mut buf = self.text.new_ui_buffer(TAB_CLOSE_W, tab_h);
				let mut attrs = crate::text::ui_attrs();
				attrs.weight = crate::text::ui_bold_weight();
				attrs.color_opt = Some(close_fg);
				buf.set_text(
					&mut self.text.font_system,
					"\u{00d7}",
					&attrs,
					Shaping::Advanced,
					None,
				);
				buf.shape_until_scroll(&mut self.text.font_system, false);
				buf
			};
			let close_w = {
				let mut attrs = crate::text::ui_attrs();
				attrs.weight = crate::text::ui_bold_weight();
				self.text.measure_ui_text("\u{00d7}", &attrs)
			};
			self.chrome = Some(ChromeCache {
				menu_fg: menu_fg_rgb,
				menubar,
				close,
				close_w,
				tab_w: -1.0, // force the tab pass below to fill in
				tabs: Vec::new(),
			});
		}
		{
			let cache = self.chrome.as_mut().unwrap();
			if cache.tab_w != tab_w {
				cache.tab_w = tab_w;
				cache.tabs.clear(); // width changed: every title buffer re-wraps
			}
			cache.tabs.truncate(tab_titles.len());
			for (i, title) in tab_titles.iter().enumerate() {
				if cache.tabs.get(i).is_some_and(|(cached, _)| cached == title) {
					continue; // unchanged title keeps its shaped buffer
				}
				let mut buf = self
					.text
					.new_ui_buffer((tab_w - 16.0 - TAB_CLOSE_W).max(8.0), tab_h);
				let mut attrs = crate::text::ui_attrs();
				attrs.color_opt = Some(menu_fg);
				buf.set_text(
					&mut self.text.font_system,
					title,
					&attrs,
					Shaping::Advanced,
					None,
				);
				buf.shape_until_scroll(&mut self.text.font_system, false);
				if i < cache.tabs.len() {
					cache.tabs[i] = (title.clone(), buf);
				} else {
					cache.tabs.push((title.clone(), buf));
				}
			}
		}
		// compute before borrowing panes for `areas` (menubar_layout takes &mut self)
		let bar_layout = self.menubar_layout();
		let copyboxes = self.copybox_layout();
		let chrome = self.chrome.as_ref().unwrap(); // ensured above
		let mut areas: Vec<TextArea> = Vec::new();
		for p in self.tabs.cur().panes.values() {
			// app-scroll slide: fill the revealed gap from the scrolled-off strip,
			// draw the current scroll region over it, then the static bands unshifted
			match &slides[&p.id] {
				Some(slide) => {
					if let Some(strip) = p.strip_text_area(slide, margin) {
						areas.push(strip);
					}
					areas.push(p.text_area_band(
						tops[&p.id],
						margin,
						slide.region_clip_t,
						slide.region_clip_b,
					));
					if slide.has_top_band {
						areas.push(p.text_area_band(
							slide.band_top,
							margin,
							f32::MIN,
							slide.top_split_y,
						));
					}
					if slide.has_band {
						areas.push(p.text_area_band(
							slide.band_top,
							margin,
							slide.split_y,
							f32::MAX,
						));
					}
				}
				None => areas.push(p.text_area(tops[&p.id], margin)),
			}
			areas.extend(p.glyph_areas());
		}
		if self.menu_bar {
			for (i, buf) in chrome.menubar.iter().enumerate() {
				// the trailing buffers are the right-aligned copy-mode labels;
				// their lowercase words center on full ink, not ascent..baseline
				let (left, left_bound, right_bound, top) = if i < bar_layout.len() {
					let (x, w) = bar_layout[i];
					(
						x + MENU_BAR_PAD,
						x,
						x + w,
						self.text.ui_text_top(0.0, menu_h),
					)
				} else {
					let j = i - bar_layout.len();
					let x = copyboxes.label_x[j];
					let w = copyboxes.label_w[j];
					(x, x, x + w, self.text.ui_text_top_ink(0.0, menu_h))
				};
				// trailing buffers are the copy-mode labels - dim them off-focus
				let color = if i < bar_layout.len() {
					menu_fg
				} else {
					copy_label_fg
				};
				areas.push(TextArea {
					buffer: buf,
					left,
					top,
					scale: 1.0,
					bounds: TextBounds {
						left: left_bound as i32,
						top: 0,
						right: right_bound as i32,
						bottom: menu_h as i32,
					},
					default_color: color,
					custom_glyphs: &[],
				});
			}
		}
		for (i, (_, buf)) in chrome.tabs.iter().enumerate() {
			let x = i as f32 * tab_w;
			let close_x = x + tab_w - TAB_CLOSE_W;
			let cb = tab_close_box(x, tab_w, tab_bar_y, tab_h);
			areas.push(TextArea {
				buffer: buf,
				left: x + 8.0,
				// center the visible text box in the tab bar (metric-based)
				top: self.text.ui_text_top(tab_bar_y, tab_h),
				scale: 1.0,
				bounds: TextBounds {
					left: x as i32,
					top: tab_bar_y as i32,
					right: close_x as i32, // leave room for the close "x"
					bottom: (tab_bar_y + tab_h) as i32,
				},
				default_color: menu_fg,
				custom_glyphs: &[],
			});
			areas.push(TextArea {
				buffer: &chrome.close,
				left: cb.x + (cb.w - chrome.close_w).max(0.0) / 2.0,
				top: self.text.ui_text_top(cb.y, cb.h),
				scale: 1.0,
				bounds: TextBounds {
					left: cb.x as i32,
					top: cb.y as i32,
					right: (cb.x + cb.w) as i32,
					bottom: (cb.y + cb.h) as i32,
				},
				default_color: close_fg,
				custom_glyphs: &[],
			});
		}

		// All rect instances and the bg-image shader work in absolute
		// framebuffer pixels (matching the glyphon viewport), so the resolution
		// is the whole window - NOT the content `area`, which is shorter by the
		// menu/tab bars and would shift cell bg + cursor down relative to text.
		let (frame_w, frame_h) = (self.gfx.config.width as f32, self.gfx.config.height as f32);
		self.text.update_viewport(
			&self.gfx.queue,
			self.gfx.config.width,
			self.gfx.config.height,
		);
		self.rects.set_resolution(&self.gfx.queue, frame_w, frame_h);
		if let Some(img) = &self.bg_image {
			img.set_resolution(&self.gfx.queue, frame_w, frame_h);
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
		// scrim source pass has its own prepared set: pane text only (no chrome),
		// with de-bolded buffers where a pane built one (text_scrim_regular_weight)
		if scrim_on {
			let mut scrim_areas: Vec<TextArea> = Vec::new();
			for p in self.tabs.cur().panes.values() {
				// scrim follows the current frame's slide, INCLUDING the scrolled-off
				// strip filling the reveal gap - without it the strip's text (e.g. the
				// row just below a static header) loses its readability halo mid-slide
				// and the halo "pops" when the slide settles, reading as a shadow that
				// jumps at the band boundary. The strip holds only region rows, so it
				// is always scrim-safe (no furniture to guard out of the scrim).
				match &slides[&p.id] {
					Some(slide) => {
						if let Some(strip) = p.strip_text_area(slide, margin) {
							scrim_areas.push(strip);
						}
						scrim_areas.push(p.scrim_text_area_band(
							tops[&p.id],
							margin,
							slide.region_clip_t,
							slide.region_clip_b,
						));
						if slide.has_top_band {
							scrim_areas.push(p.scrim_text_area_band(
								slide.band_top,
								margin,
								f32::MIN,
								slide.top_split_y,
							));
						}
						if slide.has_band {
							scrim_areas.push(p.scrim_text_area_band(
								slide.band_top,
								margin,
								slide.split_y,
								f32::MAX,
							));
						}
					}
					None => scrim_areas.push(p.scrim_text_area(tops[&p.id], margin)),
				}
				scrim_areas.extend(p.glyph_areas());
			}
			if let Err(e) = self
				.text
				.prepare_scrim(&self.gfx.device, &self.gfx.queue, scrim_areas)
			{
				eprintln!(
					"{}: scrim prepare failed; trimming atlas to recover: {e:?}",
					config::APP_NAME
				);
				self.text.trim_atlas();
				return animating;
			}
		}

		// lay out the menu into the overlay renderer: one proportional buffer
		// per item label (at the gutter), plus a checkmark buffer for checked toggles.
		if let Some(menu) = &self.menu {
			// (left, top, buffer) collected first so the borrow of self.text ends
			let mut specs: Vec<(f32, f32, Buffer)> = Vec::new();
			let mut attrs = crate::text::ui_attrs();
			attrs.color_opt = Some(GColor::rgb(
				config::menu_fg()[0],
				config::menu_fg()[1],
				config::menu_fg()[2],
			));
			for (i, entry) in menu.entries.iter().enumerate() {
				let Entry::Item { label, check, .. } = entry else {
					continue;
				};
				let top = menu.row_top(i) + (menu.item_h - self.text.ui_line_h) / 2.0;
				let mut buf = self.text.new_ui_buffer(menu.w, menu.item_h);
				buf.set_text(
					&mut self.text.font_system,
					label,
					&attrs,
					Shaping::Advanced,
					None,
				);
				buf.shape_until_scroll(&mut self.text.font_system, false);
				specs.push((menu.x + config::MENU_PAD_X + config::MENU_GUTTER, top, buf));
				if *check == Some(true) {
					let mut check_buf = self.text.new_ui_buffer(config::MENU_GUTTER, menu.item_h);
					check_buf.set_text(
						&mut self.text.font_system,
						"\u{2713}",
						&attrs,
						Shaping::Advanced,
						None,
					);
					check_buf.shape_until_scroll(&mut self.text.font_system, false);
					specs.push((menu.x + config::MENU_PAD_X, top, check_buf));
				}
			}
			let (sw, sh) = (self.gfx.config.width as i32, self.gfx.config.height as i32);
			let menu_color = GColor::rgb(
				config::menu_fg()[0],
				config::menu_fg()[1],
				config::menu_fg()[2],
			);
			let areas: Vec<TextArea> = specs
				.iter()
				.map(|(left, top, buf)| TextArea {
					buffer: buf,
					left: *left,
					top: *top,
					scale: 1.0,
					bounds: TextBounds {
						left: 0,
						top: 0,
						right: sw,
						bottom: sh,
					},
					default_color: menu_color,
					custom_glyphs: &[],
				})
				.collect();
			let _ = self
				.text
				.prepare_overlay(&self.gfx.device, &self.gfx.queue, areas);
		}

		let frame = match self.gfx.begin_frame() {
			Some(frame) => frame,
			None => return animating,
		};
		let view = self.gfx.frame_view(&frame);
		let mut encoder = self
			.gfx
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor {
				label: Some("frame"),
			});

		// Text readability scrim: build the per-pixel colour map, render the prepared
		// text to the scrim texture, blur it, then composite under the crisp text.
		// "Softness" 0..1 -> coverage boost: 0 = hard/solid (x10), 1 = soft/faint (x1)
		let scrim_intensity = 10.0 - cfg.text_scrim_softness.clamp(0.0, 1.0) * 9.0;
		// falloff curve index: 0 s, 1 gaussian, 2 linear, 3 log, 4 exp
		let scrim_ramp = match cfg.text_scrim_ramp.as_str() {
			"gaussian" => 1.0,
			"linear" => 2.0,
			"log" => 3.0,
			"exp" => 4.0,
			_ => 0.0, // "s"
		};
		// build function index: 0 dilate, 1 sdf, 2 dt, 3 gaussian (legacy blur)
		let scrim_function = match cfg.text_scrim_function.as_str() {
			"dilate" => 0.0,
			"dt" => 2.0,
			"gaussian" => 3.0,
			_ => 1.0, // "sdf"
		};
		// distance paths measure the halo extent in px; keep it a touch wider than
		// the (sigma-based) gaussian look so switching functions doesn't shrink it.
		let scrim_ext = cfg.text_scrim_radius * 2.0;
		if scrim_on {
			self.scrim.render_bgcolor(
				&self.gfx.device,
				&self.gfx.queue,
				&mut encoder,
				&scrim_cells,
				config::srgb_f32(cfg.bg),
			);
			self.scrim
				.upload_cursors(&self.gfx.device, &self.gfx.queue, &scrim_cursor_quads);
			{
				let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
					label: Some("scrim text"),
					color_attachments: &[Some(wgpu::RenderPassColorAttachment {
						view: self.scrim.text_view(),
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
				let _ = self.text.render_scrim(&mut pass);
			}
			// cursor coverage in its own texture (kept apart from the text so the
			// halo and the outline can each include it independently)
			{
				let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
					label: Some("scrim cursor"),
					color_attachments: &[Some(wgpu::RenderPassColorAttachment {
						view: self.scrim.cursor_view(),
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
				self.scrim.draw_cursors(&mut pass);
			}
			self.scrim.blur(
				&self.gfx.queue,
				&mut encoder,
				cfg.text_scrim_radius,
				scrim_ext,
				scrim_ramp,
				if cfg.cursor_scrim { 1.0 } else { 0.0 },
				scrim_function,
			);
		}

		let content_area = self.area(); // for clipping the scrim to the terminal region
		{
			let divider = config::srgb_f32(config::DIVIDER);
			// transparent base when compositing: pane-gap dividers show the
			// desktop through; opaque divider color otherwise (premultiplied)
			let clear = if self.gfx.transparent {
				wgpu::Color::TRANSPARENT
			} else {
				wgpu::Color {
					r: divider[0] as f64,
					g: divider[1] as f64,
					b: divider[2] as f64,
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
			if let Some((start, end)) = menubar_range {
				self.rects.draw(&mut pass, start..end);
			}
			if let Some((start, end)) = tabbar_range {
				self.rects.draw(&mut pass, start..end);
			}
			// scrim goes under the crisp text, over the cell backgrounds. Clip it to
			// the content area so the halo only affects terminal text, never the
			// menu bar / tab titles above it.
			if scrim_on {
				let (cx, cy, cw, ch) = scissor(content_area, sw, sh);
				pass.set_scissor_rect(cx, cy, cw, ch);
				self.scrim.composite(
					&self.gfx.queue,
					&mut pass,
					scrim_intensity,
					cfg.text_outline,
					if cfg.cursor_outline { 1.0 } else { 0.0 },
					scrim_function,
					scrim_ramp,
					scrim_ext,
				);
				pass.set_scissor_rect(0, 0, sw, sh);
			}
			// cursor above the scrim (halo can't obscure it), still under the crisp text
			for (rect, start, end) in &cursor_ranges {
				let (x, y, w, h) = scissor(*rect, sw, sh);
				if w == 0 || h == 0 {
					continue;
				}
				pass.set_scissor_rect(x, y, w, h);
				self.rects.draw(&mut pass, *start..*end);
			}
			pass.set_scissor_rect(0, 0, sw, sh);
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

// winit button -> the reportable subset (None for Back/Forward/etc.)
fn mouse_btn_of(button: MouseButton) -> Option<input::MouseBtn> {
	match button {
		MouseButton::Left => Some(input::MouseBtn::Left),
		MouseButton::Middle => Some(input::MouseBtn::Middle),
		MouseButton::Right => Some(input::MouseBtn::Right),
		_ => None,
	}
}

fn rect_inst(x: f32, y: f32, w: f32, h: f32, color: [u8; 3]) -> RectInstance {
	RectInstance {
		pos: [x, y],
		size: [w, h],
		color: config::srgb_f32(color),
	}
}

// Dim a chrome colour toward the bar background when the window isn't focused;
// used on the copy-mode checkboxes + labels to signal auto-copy is inert until
// the window regains focus (the pane's flags stay set meanwhile). Focused = no
// change.
fn copy_dim(color: [u8; 3], focused: bool) -> [u8; 3] {
	if focused {
		return color;
	}
	let bg = config::TAB_BAR_BG;
	let mix = |a: u8, b: u8| (a as f32 * 0.4 + b as f32 * 0.6) as u8;
	[
		mix(color[0], bg[0]),
		mix(color[1], bg[1]),
		mix(color[2], bg[2]),
	]
}

// The window/taskbar icon, decoded from the bundled logo (downscaled so the
// _NET_WM_ICON payload stays small). None if it can't be decoded.
// X11 session? (Per-pixel transparency needs the glutin GL path only on X11;
// Wayland's wgpu surface already does premultiplied alpha.)
fn is_x11(el: &ActiveEventLoop) -> bool {
	use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
	el.owned_display_handle()
		.display_handle()
		.map(|handle| {
			matches!(
				handle.as_raw(),
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
	// A bad --shell / default_shell (typo'd binary, PTY failure) should read
	// like the CLI parse errors, not a Rust panic + backtrace.
	let spawn = |text: &mut TextCtx, shell: Option<Vec<String>>| {
		PaneManager::new(text, proxy, area, shell).unwrap_or_else(|e| {
			eprintln!("{}: failed to start shell: {e}", config::APP_NAME);
			std::process::exit(2);
		})
	};
	if !cli.hierarchical {
		let shell = cli
			.win
			.style
			.shell
			.clone()
			.or_else(config::default_shell_argv);
		return vec![spawn(text, shell)];
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
		let mut pm = spawn(text, main_shell.clone());
		let main_id = pm.focused;
		let mut handles: HashMap<String, PaneId> = HashMap::new();
		handles.insert("main".into(), main_id);
		handles.insert("0".into(), main_id);
		if let Some(handle) = &tab.panes[0].id {
			handles.insert(handle.clone(), main_id);
		}
		let mut shells: HashMap<PaneId, Option<Vec<String>>> = HashMap::new();
		shells.insert(main_id, main_shell);
		let mut prev = main_id;

		for pane_spec in &tab.panes[1..] {
			let target = pane_spec
				.splits
				.as_deref()
				.and_then(|handle| handles.get(handle).copied())
				.unwrap_or(prev);
			let dir4 = pane_spec.dir.unwrap_or_else(|| default_dir(&pm, target));
			let (dir, before) = match dir4 {
				crate::cli::Dir4::Down => (Dir::Horizontal, false),
				crate::cli::Dir4::Up => (Dir::Horizontal, true),
				crate::cli::Dir4::Right => (Dir::Vertical, false),
				crate::cli::Dir4::Left => (Dir::Vertical, true),
			};
			// new pane's shell: explicit -> the pane it splits -> tab -> window
			let shell = pane_spec
				.style
				.shell
				.clone()
				.or_else(|| shells.get(&target).cloned().flatten())
				.or_else(|| tab.style.shell.clone())
				.or_else(|| cli.win.style.shell.clone())
				.or_else(config::default_shell_argv);
			let ratio = match pane_spec.size {
				None => 0.5,
				Some(Size::Percent(pct)) => pct / 100.0,
				Some(Size::Cells(n)) => {
					let rect = pm.panes.get(&target).map(|p| p.rect).unwrap_or(area);
					let denom = match dir {
						Dir::Vertical => (rect.w / text.cell_w).max(1.0),
						Dir::Horizontal => (rect.h / text.cell_h).max(1.0),
					};
					n as f32 / denom
				}
			};
			if let Some(new_id) = pm.split_at(
				text,
				proxy,
				target,
				dir,
				before,
				ratio,
				shell.clone(),
				area,
				false,
			) {
				if let Some(handle) = &pane_spec.id {
					handles.insert(handle.clone(), new_id);
				}
				shells.insert(new_id, shell);
				prev = new_id;
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
	let rect = pm.panes.get(&target).map(|p| p.rect);
	match rect {
		Some(rect) if rect.h > rect.w => crate::cli::Dir4::Down,
		_ => crate::cli::Dir4::Right,
	}
}

// Open a URL in the user's default browser (fire-and-forget, per platform).
fn open_url(url: &str) {
	let mut cmd = if cfg!(target_os = "macos") {
		let mut command = std::process::Command::new("open");
		command.arg(url);
		command
	} else if cfg!(target_os = "windows") {
		let mut command = std::process::Command::new("cmd");
		command.args(["/C", "start", "", url]);
		command
	} else {
		let mut command = std::process::Command::new("xdg-open");
		command.arg(url);
		command
	};
	let _ = cmd.spawn();
}

// Decode the configured background image and upload it to a texture.

// Files in `dir` that look like images the bg loader can decode, sorted by name
// (so "in order" rotation is stable and predictable).
fn list_folder_images(dir: &std::path::Path) -> Vec<PathBuf> {
	const EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tiff", "tif"];
	let Ok(entries) = std::fs::read_dir(dir) else {
		return Vec::new();
	};
	let mut images: Vec<PathBuf> = entries
		.flatten()
		.map(|e| e.path())
		.filter(|p| {
			p.is_file()
				&& p.extension()
					.and_then(|e| e.to_str())
					.is_some_and(|e| EXTS.contains(&e.to_ascii_lowercase().as_str()))
		})
		.collect();
	images.sort();
	images
}

// Next image index: wraps in order, or jumps to a different one at random. A
// non-zero `entropy` picks the random step so consecutive rotations differ.
fn next_wallpaper_index(len: usize, current: usize, random: bool, entropy: u64) -> usize {
	if len < 2 {
		return 0;
	}
	if random {
		// step in [1, len-1] guarantees a different index than `current`
		let step = 1 + (entropy % (len as u64 - 1)) as usize;
		(current + step) % len
	} else {
		(current + 1) % len
	}
}

// Cheap non-crypto entropy for random rotation, from the wall clock. Not used
// for anything security-sensitive - just to vary which image comes up next.
fn time_entropy() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_nanos() as u64)
		.unwrap_or(0)
}

fn load_bg_image(gfx: &Gfx) -> Option<ImageRenderer> {
	let settings = config::settings();
	let path = settings.background_image.as_ref()?;
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
	// Blur and contrast-flatten, done in LINEAR light (decode sRGB -> process in
	// f32 -> re-encode) so transitions are gamma-correct; an sRGB-space blur
	// darkens edges. The f32 intermediate also avoids 8-bit banding inside the
	// blur (final banding is handled by the high-precision offscreen + the blit's
	// dither).
	if settings.background_blur > 0.0 || settings.background_contrast_mask {
		let (w, h) = img.dimensions();
		let mut linear: image::ImageBuffer<image::Rgba<f32>, Vec<f32>> =
			image::ImageBuffer::new(w, h);
		for (dst, src) in linear.pixels_mut().zip(img.pixels()) {
			*dst = image::Rgba([
				config::to_linear(src[0]),
				config::to_linear(src[1]),
				config::to_linear(src[2]),
				src[3] as f32 / 255.0,
			]);
		}
		if settings.background_blur > 0.0 {
			linear = image::imageops::blur(&linear, settings.background_blur);
		}
		if settings.background_contrast_mask {
			crate::contrast::apply(
				&mut linear,
				settings.background_contrast_mask_size,
				settings.background_contrast_mask_strength,
				settings.background_contrast_mask_auto,
			);
		}
		for (dst, src) in img.pixels_mut().zip(linear.pixels()) {
			*dst = image::Rgba([
				config::from_linear_u8(src[0]),
				config::from_linear_u8(src[1]),
				config::from_linear_u8(src[2]),
				(src[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
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
		settings.background_opacity,
		settings.background_fit,
	))
}

// clamp a pane rect to an integer scissor box inside the surface
fn scissor(rect: Rect, sw: u32, sh: u32) -> (u32, u32, u32, u32) {
	let x = rect.x.max(0.0).min(sw as f32) as u32;
	let y = rect.y.max(0.0).min(sh as f32) as u32;
	let right = (rect.x + rect.w).max(0.0).min(sw as f32) as u32;
	let bottom = (rect.y + rect.h).max(0.0).min(sh as f32) as u32;
	(x, y, right.saturating_sub(x), bottom.saturating_sub(y))
}

fn focus_ring(rect: Rect) -> [RectInstance; 4] {
	let color = config::srgb_f32(config::settings().focus);
	let thickness = config::FOCUS_RING_PX;
	[
		RectInstance {
			pos: [rect.x, rect.y],
			size: [rect.w, thickness],
			color,
		},
		RectInstance {
			pos: [rect.x, rect.y + rect.h - thickness],
			size: [rect.w, thickness],
			color,
		},
		RectInstance {
			pos: [rect.x, rect.y],
			size: [thickness, rect.h],
			color,
		},
		RectInstance {
			pos: [rect.x + rect.w - thickness, rect.y],
			size: [thickness, rect.h],
			color,
		},
	]
}

impl ApplicationHandler<UserEvent> for App {
	fn resumed(&mut self, event_loop: &ActiveEventLoop) {
		if self.state.is_some() {
			return;
		}
		let cli_win = &self.cli.win;
		let decorated = !cli_win.hide_frame.unwrap_or(false);
		let menu_bar = !cli_win.hide_menu.unwrap_or(false);
		let win_title = cli_win.title.clone();
		let win_opacity = cli_win.opacity;
		// When both pixel dims are given, the window must be BORN at that size, not
		// resized into it: some EGL presents (VirtualGL's, for one) latch the surface
		// size at creation and never see later resizes, leaving a stale-offset blit.
		let initial_size: winit::dpi::Size = match (cli_win.pixel_width, cli_win.pixel_height) {
			(Some(w), Some(h)) => winit::dpi::PhysicalSize::new(w, h).into(),
			_ => winit::dpi::LogicalSize::new(1000.0, 640.0).into(),
		};
		let attrs = Window::default_attributes()
			.with_title(win_title.as_deref().unwrap_or(config::APP_NAME))
			.with_window_icon(load_icon())
			.with_decorations(decorated)
			.with_transparent(true)
			.with_inner_size(initial_size);
		let attrs = with_app_id(attrs); // stable WM_CLASS/app_id

		// On X11 the wgpu surface can't do per-pixel alpha, so we ALWAYS take the
		// glutin GL path there (transparent-capable backend), regardless of the
		// current Transparency setting - that way the toggle works live without a
		// relaunch (the bg alpha is gated per-frame, not the backend). Off-X11 the
		// normal wgpu path is used (Wayland already supports premultiplied alpha).
		// If the GL context can't be created, fall back to the native wgpu surface.
		let want_gl = is_x11(event_loop);
		let (mut gfx, window) =
			match want_gl.then(|| Gfx::new_gl_transparent(event_loop, attrs.clone())) {
				Some(Ok(pair)) => pair,
				other => {
					if let Some(Err(e)) = other {
						eprintln!(
							"{}: GL backend unavailable ({e}); using native surface (no transparency)",
							config::APP_NAME
						);
					}
					let window = Arc::new(event_loop.create_window(attrs).unwrap_or_else(|e| {
						eprintln!("{}: could not create a window: {e}", config::APP_NAME);
						std::process::exit(2);
					}));
					let gfx = Gfx::new(window.clone()).unwrap_or_else(|e| {
						eprintln!("{}: no usable GPU/renderer: {e}", config::APP_NAME);
						std::process::exit(2);
					});
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
		cli_win.apply_style();
		if cli_win.fullscreen.unwrap_or(false) {
			window.set_fullscreen(Some(Fullscreen::Borderless(None)));
		}
		// Request compositor backdrop blur (KWin/picom) if the setting is on; no-op
		// off-X11 and on compositors that don't honor the hint.
		set_blur_behind(&window, config::settings().transparent_background_blur);

		// Transparency only ever affects the terminal background (per-pixel), never
		// the whole window - so there's no compositor whole-window-opacity fallback.
		let scale = window.scale_factor() as f32;
		let mut text = TextCtx::new(&gfx.device, &gfx.queue, gfx.format, scale);
		let rects = RectRenderer::new(&gfx.device, gfx.format);
		let bg_image = load_bg_image(&gfx);
		let scrim =
			crate::scrim::Scrim::new(&gfx.device, gfx.format, gfx.config.width, gfx.config.height);

		// Resize to the configured initial grid now that cell metrics are known.
		// cell_w/cell_h/margin are physical px; floor() in content_dims gives the
		// exact column/row count at this size. If the request applies
		// synchronously winit returns the new size (no Resized event), so adopt
		// it here; otherwise a Resized event reconfigures the surface.
		let settings = config::settings();
		// CLI columns/rows override config; --pixel-width/height override either
		// dimension directly. Add the menu-bar height (when shown) so the content
		// still gets the requested row count (the tab bar only appears with >1 tab).
		// remember_size launches at the last actual size; CLI columns/rows still override
		let cols = cli_win.columns.unwrap_or(if settings.remember_size {
			settings.remembered_columns
		} else {
			settings.columns
		});
		let rows = cli_win.rows.unwrap_or(if settings.remember_size {
			settings.remembered_rows
		} else {
			settings.rows
		});
		let menu_bar_h = if menu_bar {
			text.ui_line_h + MENU_BAR_VPAD
		} else {
			0.0
		};
		let want = winit::dpi::PhysicalSize::new(
			cli_win
				.pixel_width
				.unwrap_or_else(|| (cols as f32 * text.cell_w + 2.0 * text.margin).ceil() as u32),
			cli_win.pixel_height.unwrap_or_else(|| {
				(rows as f32 * text.cell_h + 2.0 * text.margin + menu_bar_h).ceil() as u32
			}),
		);
		let mut scrim = scrim;
		if let Some(applied) = window.request_inner_size(want) {
			gfx.resize(applied.width, applied.height);
			scrim.resize(&gfx.device, applied.width, applied.height);
		}

		// initial content area, inset by the menu bar (when shown) and the tab
		// bar (when the CLI makes >1 tab), so panes start correctly sized.
		let n_tabs = if self.cli.hierarchical {
			self.cli.tabs.len().max(1)
		} else {
			1
		};
		let top = menu_bar_h
			+ if n_tabs > 1 {
				text.ui_line_h + TAB_BAR_VPAD
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
			scrim,
			tabs: Tabs { list, active: 0 },
			mods: ModifiersState::empty(),
			mouse: (0.0, 0.0),
			mouse_btn: None,
			mouse_cell: None,
			selecting: None,
			last_click: None,
			click_count: 0,
			resizing: None,
			dragging_pane: None,
			cursor_icon: CursorIcon::Default,
			clipboard: Clipboard::new(),
			last_frame: Instant::now(),
			dirty: true,
			bell_flash: 0.0,
			size_tracked: false,
			pending_size: None,
			pending_size_at: Instant::now(),
			menu: None,
			decorated,
			menu_bar,
			bar_open: None,
			quit: false,
			win_opacity,
			win_title,
			last_win_title: String::new(),
			focused: true,
			pending_about: false,
			pending_settings: false,
			chrome: None,
			wp_images: Vec::new(),
			wp_index: 0,
			wp_next: None,
		});
		if let Some(state) = self.state.as_mut() {
			state.init_wallpaper_rotation();
		}
	}

	fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
		let Some(state) = self.state.as_mut() else {
			return;
		};
		match event {
			UserEvent::Wakeup(id) => {
				// output easing is triggered in Pane::build when the screen
				// actually scrolls, not on every content change. Only the pane
				// that produced output is marked; a background tab's flag just
				// waits until its tab is shown (the switch forces a rebuild).
				if let Some(p) = state.tabs.find_pane_mut(id) {
					p.content_dirty = true;
					p.note_output(); // copy-output: push the settle deadline out
				}
			}
			UserEvent::PtyWrite(id, bytes) => {
				if let Some(p) = state.tabs.find_pane(id) {
					p.term.write(bytes);
				}
			}
			UserEvent::Title(id, title) => {
				if let Some(p) = state.tabs.find_pane_mut(id) {
					p.title = title;
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
				if let Some(tab_idx) = state
					.tabs
					.list
					.iter()
					.position(|pm| pm.panes.contains_key(&id))
				{
					if state.tabs.list[tab_idx].panes.len() > 1 {
						state.tabs.list[tab_idx].close(&mut state.text, id, area);
					} else if state.tabs.len() > 1 {
						state.close_tab_at(tab_idx);
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
			UserEvent::SetWallpaper(image) => state.set_wallpaper(image),
			UserEvent::ReloadSettings => state.reload_config(),
		}
	}

	fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
		// route events for a pop-out dialog window to its own handler
		if self.dialog.as_ref().is_some_and(|d| d.id() == id) {
			self.handle_dialog_event(event);
			return;
		}
		// Simulated modality: while a dialog is open the main window takes no
		// input; a click on it re-raises/focuses the dialog instead.
		if let Some(d) = &self.dialog {
			match &event {
				WindowEvent::KeyboardInput { .. }
				| WindowEvent::MouseWheel { .. }
				| WindowEvent::Ime(_) => return,
				WindowEvent::MouseInput {
					state: ElementState::Pressed,
					..
				} => {
					d.window.focus_window();
					return;
				}
				_ => {}
			}
		}
		let Some(state) = self.state.as_mut() else {
			return;
		};
		match event {
			WindowEvent::CloseRequested => event_loop.exit(),

			WindowEvent::Resized(size) => {
				state.gfx.resize(size.width, size.height);
				state
					.scrim
					.resize(&state.gfx.device, size.width, size.height);
				state.relayout_all();
				state.save_window_size(size.width, size.height);
				state.dirty = true;
			}

			// DPI/scale changed (monitor move or a live scaling change). Re-scale
			// cell metrics + chrome for the new factor; winit preserves the logical
			// size, so a Resized event follows to reconfigure the surface + scrim.
			WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
				state.rebuild_text(scale_factor as f32);
				state.dirty = true;
			}

			WindowEvent::ModifiersChanged(mods) => {
				state.mods = mods.state();
				// Alt toggles the menu-bar accelerator underlines, so redraw.
				state.dirty = true;
			}

			// Window focus gates copy-output: a background window never copies.
			WindowEvent::Focused(focused) => {
				state.focused = focused;
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
				// mouse-tracking app wants motion/drag reports; when it does, skip our
				// local hover/selection handling for this move. The report is
				// PTY-bound: nothing local changed, so no redraw - marking dirty here
				// forced a full re-shape of every pane per cell crossed.
				if state.report_mouse_motion() {
					return;
				}
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
					let hovered = menu.item_at(x, y);
					if hovered != menu.hover {
						menu.hover = hovered;
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
					// the always-visible copy-mode checkboxes toggle the focused pane
					if let Some(kind) = state.copybox_hit(x) {
						let focused_id = state.tabs.cur().focused;
						state.toggle_copy(focused_id, kind);
						state.menu = None;
						state.bar_open = None;
						state.dirty = true;
						return;
					}
					match (state.menubar_hit(x), state.bar_open) {
						(Some(i), Some(open)) if i == open => {
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
				// click on the tab bar selects a tab. Skip when a dropdown is open: it
				// opens flush under the menu bar, so its top item overlaps the tab-bar
				// band - without this guard the tab bar steals the click and (e.g.)
				// "Tabs|New Tab" selects a tab instead of firing, once >1 tab exists.
				let tab_bar_y = state.menubar_h();
				if button == MouseButton::Left
					&& state.menu.is_none()
					&& state.tabs.len() > 1
					&& y >= tab_bar_y
					&& y < tab_bar_y + state.tab_bar_h()
				{
					let tab_w =
						(state.gfx.config.width as f32 / state.tabs.len() as f32).min(TAB_MAX_W);
					let i = (x / tab_w).floor() as usize;
					if i < state.tabs.len() {
						// click in the close-button column closes that tab; else select it
						let cb =
							tab_close_box(i as f32 * tab_w, tab_w, tab_bar_y, state.tab_bar_h());
						if x >= cb.x {
							state.close_tab_at(i);
						} else {
							state.tabs.active = i;
							state.update_title();
						}
						state.dirty = true;
					}
					return;
				}
				// mouse-tracking app owns the pointer: report the press, skip local
				// selection/paste/menu (Shift bypasses to the local action). An open
				// menu must get the click (operate/dismiss it), not the app underneath.
				if state.menu.is_none() && state.report_mouse_button(button, true) {
					state.dirty = true;
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
							// 1 click = plain run (Ctrl = rectangle), 2 = word/pair,
							// 3 = whole line (wrapped lines included)
							let now = Instant::now();
							let (cell_w, cell_h) = (state.text.cell_w, state.text.cell_h);
							let near =
								state.last_click.is_some_and(|(last_time, last_x, last_y)| {
									now.duration_since(last_time) < Duration::from_millis(400)
										&& (x - last_x).abs() <= cell_w
										&& (y - last_y).abs() <= cell_h
								});
							// count consecutive same-spot clicks; a 4th wraps back to 1
							state.click_count = if near { (state.click_count % 3) + 1 } else { 1 };
							state.last_click = Some((now, x, y));
							let double = state.click_count == 2;
							let triple = state.click_count == 3;
							let pairs = if double {
								config::selection_pairs()
							} else {
								Vec::new()
							};
							let ctrl = state.mods.control_key();
							let started = state.tabs.cur().pane_at(x, y).and_then(|id| {
								let p = state.tabs.cur().panes.get(&id)?;
								let (point, side) = p.point_at(x, y, &state.text)?;
								if triple {
									// whole logical line, spanning wrapped continuation rows
									let (start, end) = p.line_span(point);
									p.begin_selection(start, Side::Left, SelectionType::Simple);
									p.update_selection(end, Side::Right);
								} else if double {
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
									let sel_type = if ctrl {
										SelectionType::Block
									} else {
										SelectionType::Simple
									};
									p.begin_selection(point, side, sel_type);
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

			// a mouse-tracking app owns the pointer: report the release we opened.
			// Only for the SAME button as the reported press - releasing a different
			// one must not clear the held state (the app would see an unbalanced
			// press) nor steal that button's local release handling below.
			WindowEvent::MouseInput {
				state: ElementState::Released,
				button,
				..
			} if state.mouse_btn.is_some() && state.mouse_btn == mouse_btn_of(button) => {
				if state.report_mouse_button(button, false) {
					state.dirty = true;
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
					if let Some(target_id) = state.tabs.cur().pane_at(x, y) {
						state
							.tabs
							.cur_mut()
							.swap_panes(&mut state.text, src, target_id, area);
					}
					state.window.set_cursor(CursorIcon::Default);
					state.cursor_icon = CursorIcon::Default;
					state.dirty = true;
				}
				// finish a drag-select: copy to primary, or clear if it was a click
				if let Some(id) = state.selecting.take() {
					let text = state.tabs.cur().panes.get(&id).and_then(|p| {
						let sel_text = p.selection_text();
						if sel_text.is_none() {
							p.clear_selection();
						}
						sel_text
					});
					match text {
						Some(sel_text) => {
							// copy-on-select: a finished selection also lands on
							// the desktop clipboard when the pane opted in
							// copy-on-select fires only for the focused pane of the
							// active tab in a focused window (only that pane copies)
							if state.focused
								&& id == state.tabs.cur().focused
								&& state
									.tabs
									.cur()
									.panes
									.get(&id)
									.is_some_and(|p| p.copy_select)
							{
								state.clipboard.set_clipboard(sel_text.clone());
							}
							state.clipboard.set_primary(sel_text);
						}
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
				// A mouse-tracking app (muffer, tmux, vim with mouse on, ...) wants
				// the wheel as button 64/65 reports, not our scrollback. Shift is the
				// local-scroll override. Report one notch per line, then stop here.
				if !state.mods.shift_key() {
					let (up, notches) = match delta {
						MouseScrollDelta::LineDelta(_, y) => {
							(y > 0.0, (y.abs().round() as u32).max(1))
						}
						MouseScrollDelta::PixelDelta(pos) => (
							(pos.y as f32) > 0.0,
							((pos.y.abs() as f32 / cell_h).round() as u32).max(1),
						),
					};
					if let Some(p) = state.tabs.cur().panes.get(&id) {
						if input::wants_mouse(p.mode) {
							if let Some((col, row)) = p.screen_cell_at(x, y, &state.text) {
								let btn = if up {
									input::MouseBtn::WheelUp
								} else {
									input::MouseBtn::WheelDown
								};
								for _ in 0..notches.min(8) {
									if let Some(seq) = input::mouse_report(
										p.mode, btn, true, false, col, row, state.mods,
									) {
										p.term.write(seq);
									}
								}
							}
							state.dirty = true;
							return;
						}
					}
				}
				// smooth scrollback uses WHEEL_LINES; full-screen apps get their
				// own (tunable) lines-per-notch via ALT_SCROLL_LINES
				let (lines, alt_lines) = match delta {
					MouseScrollDelta::LineDelta(_, y) => (
						y * config::settings().wheel_lines,
						y * config::settings().alt_scroll_lines,
					),
					MouseScrollDelta::PixelDelta(pos) => {
						let lines = pos.y as f32 / cell_h;
						(lines, lines)
					}
				};
				if let Some(p) = state.tabs.cur_mut().panes.get_mut(&id) {
					let mode = p.mode;
					// Alternate-scroll (DECSET 1007) is default-on, so gate the cursor-key
					// path on actually being in the alt screen. On the primary screen the
					// wheel must scroll our scrollback; sending cursor keys there recalls
					// shell history instead (the reported bug).
					let alt_scroll = mode.contains(TermMode::ALT_SCREEN)
						&& mode.contains(TermMode::ALTERNATE_SCROLL)
						&& !mode.intersects(TermMode::MOUSE_MODE);
					if alt_scroll {
						// full-screen apps (less, nano, ...) have no scrollback of
						// their own; the wheel drives their cursor-key scrolling
						let n = alt_lines.abs().round() as i32;
						if n > 0 {
							let letter = if alt_lines > 0.0 { b'A' } else { b'B' };
							let seq =
								input::cursor_seq(letter, mode.contains(TermMode::APP_CURSOR));
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
							if let Some(menu) = state.menu.as_mut() {
								menu.hover = menu.step(menu.hover, 1);
							}
						}
						Key::Named(NamedKey::ArrowUp) => {
							if let Some(menu) = state.menu.as_mut() {
								menu.hover = menu.step(menu.hover, -1);
							}
						}
						// Left/Right cycle between menu-bar dropdowns (no-op for a
						// right-click context menu, which isn't bar-anchored)
						Key::Named(NamedKey::ArrowLeft) | Key::Named(NamedKey::ArrowRight) => {
							if let Some(open_idx) = state.bar_open {
								let n = MENU_BAR.len();
								let next =
									if matches!(key.logical_key, Key::Named(NamedKey::ArrowLeft)) {
										(open_idx + n - 1) % n
									} else {
										(open_idx + 1) % n
									};
								state.open_bar_menu(next);
							}
						}
						Key::Named(NamedKey::Enter) => {
							if let Some(menu) = state.menu.take() {
								state.bar_open = None;
								if let Some(Entry::Item { action, .. }) =
									menu.hover.map(|i| &menu.entries[i])
								{
									state.apply_menu(*action, menu.target, &self.proxy);
								}
								if state.quit {
									event_loop.exit();
									return;
								}
							}
						}
						// accelerator: a letter activates the first item starting with it
						Key::Character(typed) => {
							let ch = typed.chars().next().map(|c| c.to_ascii_lowercase());
							let hit = ch.and_then(|ch| {
								state.menu.as_ref().and_then(|menu| {
									menu.entries.iter().position(|entry| {
										matches!(entry, Entry::Item { label, .. }
											if label.chars().next().map(|c| c.to_ascii_lowercase()) == Some(ch))
									})
								})
							});
							if let Some(i) = hit {
								if let Some(menu) = state.menu.take() {
									state.bar_open = None;
									if let Entry::Item { action, .. } = &menu.entries[i] {
										state.apply_menu(*action, menu.target, &self.proxy);
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
					&& matches!(&key.logical_key, Key::Character(typed) if typed == ",")
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
						let (rect_x, rect_y) = (p.rect.x, p.rect.y);
						state.open_menu(id, rect_x + 12.0, rect_y + 12.0);
						state.dirty = true;
					}
					return;
				}
				// Menu accelerators: Alt+F/E/V/T/P/H open the matching top-level
				// menu. NOTE: this shadows the shell's Meta+<those letters>
				// (e.g. Meta-f word-forward) - the standard menu-bar tradeoff.
				if state.menu_bar && state.mods.alt_key() && !state.mods.control_key() {
					if let Key::Character(typed) = &key.logical_key {
						if let Some(ch) = typed.chars().next().map(|c| c.to_ascii_uppercase()) {
							if let Some(i) = MENU_BAR.iter().position(|title| title.starts_with(ch))
							{
								state.open_bar_menu(i);
								state.dirty = true;
								return;
							}
						}
					}
				}
				// tab hotkeys (Ctrl based).
				if state.mods.control_key() {
					let shift = state.mods.shift_key();
					match &key.logical_key {
						// Ctrl+Shift+T: new tab (Shift so plain Ctrl+T reaches the shell)
						Key::Character(typed) if shift && typed.eq_ignore_ascii_case("t") => {
							state.new_tab(&self.proxy);
							return;
						}
						// Ctrl+Shift+W / Ctrl+F4: close the current tab (keeps >=1 tab;
						// close the window to exit). Shift on W so plain Ctrl+W reaches
						// the shell (word-erase).
						Key::Character(typed) if shift && typed.eq_ignore_ascii_case("w") => {
							state.close_tab();
							return;
						}
						Key::Named(NamedKey::F4) => {
							state.close_tab();
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
				if state.handle_hotkey(&key) {
					state.dirty = true;
					return;
				}
				let focused = state.tabs.cur().focused;
				let app_cursor = state
					.tabs
					.cur()
					.panes
					.get(&focused)
					.map(|p| p.mode.contains(TermMode::APP_CURSOR))
					.unwrap_or(false);
				if let Some(bytes) = input::encode(&key, state.mods, app_cursor) {
					// copy-output: Enter at the shell prompt may launch a command;
					// arm the capture so its output is copied once the pane settles.
					let is_enter = matches!(key.logical_key, Key::Named(NamedKey::Enter));
					if let Some(p) = state.tabs.cur_mut().panes.get_mut(&focused) {
						if !p.read_only {
							p.scroll.jump_bottom();
							p.term.write(bytes);
							if is_enter && p.copy_output {
								p.arm_capture();
							}
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
	fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
		// don't lose a resize done just before quitting
		if let Some(state) = self.state.as_mut() {
			state.flush_window_size(true);
		}
	}

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
			.is_some_and(|state| std::mem::take(&mut state.pending_about));
		// parent handle so the WM ties the dialog to the terminal window
		// (transient-for / owner)
		let parent = self.state.as_ref().and_then(|state| {
			use winit::raw_window_handle::HasWindowHandle;
			state
				.window
				.window_handle()
				.ok()
				.map(|handle| handle.as_raw())
		});
		if open_about {
			if let Some(info) = self
				.state
				.as_ref()
				.map(|state| state.gfx.adapter_info.clone())
			{
				match crate::dialog::DialogWin::new_about(event_loop, &info, parent) {
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
			.is_some_and(|state| std::mem::take(&mut state.pending_settings));
		if open_settings {
			match crate::dialog::DialogWin::new_settings(event_loop, parent) {
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

		// re-assert the dialog->terminal stacking a few times after focus (see the
		// field comment). Cleared when the dialog closes.
		if self.dialog.is_none() {
			self.raise_reassert = 0;
		} else if self.raise_reassert > 0 && Instant::now() >= self.raise_next {
			if let Some(d) = &self.dialog {
				d.raise_parent();
			}
			self.raise_reassert -= 1;
			self.raise_next = Instant::now() + RAISE_REASSERT_IVL;
		}
		let raise_wake = (self.raise_reassert > 0).then_some(self.raise_next);

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
		let content = state.tabs.cur().panes.values().any(|p| p.content_dirty);
		// copy-output: catch the focused pane's command finishing (see method)
		state.poll_output_copy();
		// wallpaper rotation: swap to the next image when its interval elapses
		// (sets state.dirty so the change renders this cycle)
		if state.wp_next.is_some_and(|next| Instant::now() >= next) {
			state.advance_wallpaper();
		}
		let bell_anim = state.bell_flash > 0.0;
		let flow = if state.dirty || content || scroll_anim || cursor_anim || bell_anim {
			// UI/chrome changes and the bell force ALL panes to re-shape; fresh
			// output and scroll eases are scoped per pane inside render (a pure
			// cursor-animation frame lets every pane reuse its cached frame).
			let force = state.dirty || bell_anim;
			state.dirty = false;
			let animating = state.render(force);
			// a pane whose term was locked kept its content_dirty (rebuild was
			// skipped) - retry shortly instead of waiting for the next event,
			// or the last wakeup of a burst could leave a stale frame up
			let retry = state.tabs.cur().panes.values().any(|p| p.content_dirty);
			if animating && (scroll_anim || bell_anim) {
				// scroll (the flagship smooth feature) and the bell flash render
				// at full rate; fresh content needs no Poll - each PTY read
				// batch arrives as its own Wakeup
				ControlFlow::Poll
			} else if retry {
				ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(5))
			} else if animating {
				// a lone idle cursor blink is capped to ~30fps so it isn't
				// re-rendering every frame just to pulse
				ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(33))
			} else {
				ControlFlow::Wait
			}
		} else {
			ControlFlow::Wait
		};
		// Debounced remember-size: persist once the size has held; while one is
		// pending, make sure the loop wakes up to flush it even when idle.
		state.flush_window_size(false);
		let flow = if let (ControlFlow::Wait, Some(_)) = (flow, state.pending_size) {
			ControlFlow::WaitUntil(state.pending_size_at + SIZE_SAVE_DEBOUNCE)
		} else {
			flow
		};
		// copy-output: while a capture is armed, make sure the loop wakes at its
		// settle deadline to run the capture check even when otherwise idle.
		let flow = match (flow, state.capture_wake()) {
			(ControlFlow::Wait, Some(wake)) => ControlFlow::WaitUntil(wake),
			(ControlFlow::WaitUntil(until), Some(wake)) => ControlFlow::WaitUntil(until.min(wake)),
			(other_flow, _) => other_flow,
		};
		// keep the loop waking while dialog-raise retries are pending
		let flow = match (flow, raise_wake) {
			(ControlFlow::Wait, Some(wake)) => ControlFlow::WaitUntil(wake),
			(ControlFlow::WaitUntil(until), Some(wake)) => ControlFlow::WaitUntil(until.min(wake)),
			(other_flow, _) => other_flow,
		};
		// wake to rotate the wallpaper when its interval is up, even when idle
		let flow = match (flow, state.wp_next) {
			(ControlFlow::Wait, Some(wake)) => ControlFlow::WaitUntil(wake),
			(ControlFlow::WaitUntil(until), Some(wake)) => ControlFlow::WaitUntil(until.min(wake)),
			(other_flow, _) => other_flow,
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
	fn handle_hotkey(&mut self, key: &winit::event::KeyEvent) -> bool {
		if !(self.mods.control_key() && self.mods.shift_key()) {
			return false;
		}
		let focused = self.tabs.cur().focused;
		match &key.logical_key {
			// Ctrl+Shift+C / Ctrl+Shift+V: clipboard copy / paste
			Key::Character(typed) if typed.eq_ignore_ascii_case("c") => {
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
			Key::Character(typed) if typed.eq_ignore_ascii_case("v") => {
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

#[cfg(test)]
mod tests {
	use super::{list_folder_images, next_wallpaper_index};

	#[test]
	fn wallpaper_order_wraps() {
		assert_eq!(next_wallpaper_index(3, 0, false, 0), 1);
		assert_eq!(next_wallpaper_index(3, 2, false, 0), 0); // wraps
		assert_eq!(next_wallpaper_index(1, 0, false, 0), 0); // single image: stays put
		assert_eq!(next_wallpaper_index(0, 0, false, 0), 0); // empty: safe
	}

	#[test]
	fn wallpaper_random_always_differs() {
		// whatever the entropy, a random pick lands on a different index
		for entropy in 0..100u64 {
			for current in 0..5usize {
				let next = next_wallpaper_index(5, current, true, entropy);
				assert_ne!(
					next, current,
					"random rotation must not repeat the same image"
				);
				assert!(next < 5);
			}
		}
	}

	#[test]
	fn folder_scan_filters_and_sorts() {
		let dir = std::env::temp_dir().join(format!("silkterm_wp_scan_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		for name in ["b.png", "a.JPG", "notes.txt", "c.gif", ".hidden"] {
			std::fs::write(dir.join(name), b"x").unwrap();
		}
		std::fs::create_dir_all(dir.join("d.png")).unwrap(); // a dir named like an image
		let imgs = list_folder_images(&dir);
		let names: Vec<String> = imgs
			.iter()
			.map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
			.collect();
		// only image files, case-insensitive ext, sorted; the .txt, the dir, kept out
		assert_eq!(names, vec!["a.JPG", "b.png", "c.gif"]);
		let _ = std::fs::remove_dir_all(&dir);
	}
}
