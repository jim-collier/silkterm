// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

// Pop-out dialog windows (About / Settings) as real child OS windows, so a
// dialog larger than the main window is still fully visible (the in-surface
// overlay was clipped by the main window). Each dialog owns its surface + text
// context and is sized to its content (non-resizable).
use std::sync::Arc;

use glyphon::{Color as GColor, Shaping, TextArea, TextBounds};
use winit::event_loop::ActiveEventLoop;
use winit::raw_window_handle::RawWindowHandle;
use winit::window::{Window, WindowId};

use crate::config;
use crate::gfx::{Gfx, RectInstance, RectRenderer};
use crate::pane::Rect;
use crate::settings_ui::{Action, SettingsDialog};
use crate::text::{TextCtx, ui_attrs};

// A laid-out line of static dialog text (window-relative coords).
struct Line {
	text: String,
	x: f32,
	y: f32,
	color: [u8; 3],
	bold: bool,
	scale: f32,
}

enum Content {
	About {
		lines: Vec<Line>,
		link_rect: Rect,
		url: String,
	},
	Settings(SettingsDialog),
}

pub enum DialogAction {
	OpenUrl(String),
	Apply,         // apply settings, keep the dialog open (live preview)
	ApplyAndClose, // OK
	Close,         // Cancel / Esc / window close
}

pub struct DialogWin {
	pub window: Arc<Window>,
	gfx: Gfx,
	text: TextCtx,
	rects: RectRenderer,
	content: Content,
	mouse: (f32, f32),
}

impl DialogWin {
	pub fn id(&self) -> WindowId {
		self.window.id()
	}

	fn make(
		el: &ActiveEventLoop,
		title: String,
		w: f32,
		h: f32,
		parent: Option<RawWindowHandle>,
	) -> anyhow::Result<(Arc<Window>, Gfx, TextCtx, RectRenderer)> {
		#[allow(unused_mut)] // reassigned only on windows/macos below
		let mut attrs = Window::default_attributes()
			.with_title(title)
			.with_window_icon(crate::app::load_icon())
			.with_resizable(false)
			.with_inner_size(winit::dpi::PhysicalSize::new(
				w.ceil().max(1.0) as u32,
				h.ceil().max(1.0) as u32,
			));
		// Tie the dialog to the terminal window so the WM keeps it above its
		// parent and groups them. Windows/macOS: winit's parent_window is owner/
		// parent semantics - what we want. X11: parent_window means literal X
		// reparenting (an embedded child, unmanaged by the WM), so DON'T pass it
		// there; WM_TRANSIENT_FOR is set after creation instead (below).
		// SAFETY: the handle comes from the live main window on this same thread.
		#[cfg(any(target_os = "windows", target_os = "macos"))]
		if parent.is_some() {
			attrs = unsafe { attrs.with_parent_window(parent) };
		}
		let window = Arc::new(el.create_window(attrs)?);
		set_transient_for(&window, parent.as_ref());
		// PRIMARY (no GL): the main window may hold a glutin GL/EGL context, and a
		// second wgpu GL instance would panic in EGL teardown. Dialogs are opaque,
		// so Vulkan/Metal/DX12 is all they need.
		let mut gfx = Gfx::with_backends(window.clone(), wgpu::Backends::PRIMARY)?;
		// adopt the size winit actually gave us
		let size = window.inner_size();
		gfx.resize(size.width, size.height);
		let scale = window.scale_factor() as f32;
		let text = TextCtx::new(&gfx.device, &gfx.queue, gfx.format, scale);
		let rects = RectRenderer::new(&gfx.device, gfx.format);
		Ok((window, gfx, text, rects))
	}

	pub fn new_about(
		el: &ActiveEventLoop,
		adapter: &wgpu::AdapterInfo,
		parent: Option<RawWindowHandle>,
	) -> anyhow::Result<Self> {
		// provisional window so we have a TextCtx to measure with
		let (window, mut gfx, mut text, rects) = Self::make(
			el,
			format!("About {}", config::APP_NAME),
			560.0,
			360.0,
			parent,
		)?;
		let (lines, link_rect, url, size) = layout_about(&mut text, adapter);
		let requested_size =
			winit::dpi::PhysicalSize::new(size.0.ceil() as u32, size.1.ceil() as u32);
		if let Some(applied) = window.request_inner_size(requested_size) {
			gfx.resize(applied.width, applied.height);
		}
		Ok(Self {
			window,
			gfx,
			text,
			rects,
			content: Content::About {
				lines,
				link_rect,
				url,
			},
			mouse: (0.0, 0.0),
		})
	}

	pub fn new_settings(
		el: &ActiveEventLoop,
		parent: Option<RawWindowHandle>,
	) -> anyhow::Result<Self> {
		// provisional window first: sizing needs a TextCtx to measure labels in
		// the real UI font (same pattern as About)
		let (window, mut gfx, mut text, rects) =
			Self::make(el, "Settings".into(), 560.0, 800.0, parent)?;
		let (label_w, btn_w, tab_ws) = crate::settings_ui::chrome_widths(&mut text);
		// cap the window height to the monitor (minus decorations headroom) and to
		// ~1010px total; a tab that doesn't fit scrolls instead of clipping buttons
		let max_h = window
			.current_monitor()
			.map(|monitor| monitor.size().height as f32 - 38.0)
			.unwrap_or(1010.0)
			.min(1010.0);
		// laid out at the origin
		let dialog = SettingsDialog::new(0.0, 0.0, text.ui_line_h, label_w, btn_w, tab_ws, max_h);
		let (w, h) = dialog.size();
		let requested_size = winit::dpi::PhysicalSize::new(w.ceil() as u32, h.ceil() as u32);
		if let Some(applied) = window.request_inner_size(requested_size) {
			gfx.resize(applied.width, applied.height);
		}
		Ok(Self {
			window,
			gfx,
			text,
			rects,
			content: Content::Settings(dialog),
			mouse: (0.0, 0.0),
		})
	}

	// (orig, edited, use_system_font) for the app to apply, if this is Settings.
	pub fn settings_values(&self) -> Option<(config::Settings, config::Settings, bool)> {
		match &self.content {
			Content::Settings(dialog) => Some((
				dialog.orig().clone(),
				dialog.edited().clone(),
				dialog.use_system_font(),
			)),
			_ => None,
		}
	}

	// After an Apply, reset the settings baseline to the applied values so a later
	// Apply diffs against the live state (see SettingsDialog::commit_baseline).
	pub fn commit_baseline(&mut self) {
		if let Content::Settings(dialog) = &mut self.content {
			dialog.commit_baseline();
		}
	}

	// Config keys the user hit "revert to default" on since the last Apply; the
	// app comments them out in config.toml (config::revert_keys).
	pub fn take_reverted(&mut self) -> Vec<&'static str> {
		match &mut self.content {
			Content::Settings(dialog) => dialog.take_reverted(),
			_ => Vec::new(),
		}
	}

	pub fn set_cursor(&mut self, x: f32, y: f32) {
		self.mouse = (x, y);
		if let Content::Settings(dialog) = &mut self.content {
			dialog.mouse_move(x, y); // continues a slider drag
		}
	}

	pub fn mouse_down(&mut self) -> Option<DialogAction> {
		let (mx, my) = self.mouse;
		match &mut self.content {
			Content::About { link_rect, url, .. } => link_rect
				.contains(mx, my)
				.then(|| DialogAction::OpenUrl(url.clone())),
			Content::Settings(dialog) => {
				let (w, h) = dialog.size();
				// ignore clicks outside the panel (would otherwise Cancel)
				if mx < 0.0 || my < 0.0 || mx > w || my > h {
					return None;
				}
				// disjoint field borrow: measure via the text context (click-to-caret)
				let attrs = ui_attrs();
				let text = &mut self.text;
				let mut measure = |s: &str| text.measure_ui_text(s, &attrs);
				map_action(dialog.mouse_down(mx, my, &mut measure))
			}
		}
	}

	pub fn mouse_up(&mut self) -> Option<DialogAction> {
		let (mx, my) = self.mouse;
		if let Content::Settings(dialog) = &mut self.content {
			map_action(dialog.mouse_up(mx, my))
		} else {
			None
		}
	}

	// wheel scroll for an overflowing settings tab (positive dy = scroll up)
	pub fn wheel(&mut self, dy_px: f32) {
		if let Content::Settings(dialog) = &mut self.content {
			dialog.wheel(dy_px);
		}
	}

	// Modifier state (from ModifiersChanged): Alt underlines button accelerators;
	// Shift/Ctrl steer Tab-key focus / tab switching.
	pub fn set_mods(&mut self, alt: bool, shift: bool, ctrl: bool) {
		if let Content::Settings(dialog) = &mut self.content {
			dialog.set_mods(alt, shift, ctrl);
		}
	}

	// Tab key: walk control focus (Ctrl = switch tabs, Shift = backwards).
	pub fn key_tab(&mut self) {
		if let Content::Settings(dialog) = &mut self.content {
			dialog.key_tab();
		}
	}
	// Up / Down: walk control focus.
	pub fn focus_vertical(&mut self, forward: bool) {
		if let Content::Settings(dialog) = &mut self.content {
			dialog.key_vertical(forward);
		}
	}
	// Left / Right: caret motion (editing) or adjust the focused slider/radio.
	pub fn key_horizontal(&mut self, dir: i32) {
		if let Content::Settings(dialog) = &mut self.content {
			dialog.key_horizontal(dir);
		}
	}
	// Space: type into an active edit, activate a focused footer button, or
	// activate the focused control.
	pub fn key_space(&mut self) -> Option<DialogAction> {
		match &mut self.content {
			Content::Settings(dialog) => map_action(dialog.key_space()),
			_ => None,
		}
	}

	// A character key: while Alt is held it's an accelerator (Cancel/Apply/OK),
	// otherwise it types into the focused field.
	pub fn key_char(&mut self, ch: char) -> Option<DialogAction> {
		match &mut self.content {
			Content::Settings(dialog) if dialog.alt() => map_action(dialog.alt_key(ch)),
			Content::Settings(dialog) => {
				dialog.char_input(ch);
				None
			}
			_ => None,
		}
	}

	pub fn backspace(&mut self) {
		if let Content::Settings(dialog) = &mut self.content {
			dialog.backspace();
		}
	}

	// Home / End / Delete inside a focused settings field (Left/Right go through
	// key_horizontal so they can double as slider/radio adjust when not editing).
	pub fn edit_nav(&mut self, key: winit::keyboard::NamedKey) {
		use winit::keyboard::NamedKey as N;
		if let Content::Settings(dialog) = &mut self.content {
			match key {
				N::Home => dialog.cursor_home(),
				N::End => dialog.cursor_end(),
				N::Delete => dialog.delete_forward(),
				_ => {}
			}
		}
	}

	pub fn key_escape(&mut self) -> Option<DialogAction> {
		match &mut self.content {
			Content::About { .. } => Some(DialogAction::Close),
			Content::Settings(dialog) => map_action(dialog.key_escape()),
		}
	}

	pub fn key_enter(&mut self) -> Option<DialogAction> {
		match &mut self.content {
			Content::Settings(dialog) => map_action(dialog.key_enter()),
			_ => None,
		}
	}

	pub fn resize(&mut self, w: u32, h: u32) {
		self.gfx.resize(w, h);
		self.window.request_redraw();
	}

	pub fn render(&mut self) {
		let frame = match self.gfx.begin_frame() {
			Some(f) => f,
			None => return,
		};
		let view = self.gfx.frame_view(&frame);
		let (w, h) = (self.gfx.config.width, self.gfx.config.height);
		self.text.update_viewport(&self.gfx.queue, w, h);

		// gather rects (Settings only) + per-line/-item text buffers
		let mut rect_inst: Vec<RectInstance> = Vec::new();
		// Settings rows are drawn scissored to the scroll viewport (rects after
		// `rect_split`); the chrome before it draws unclipped.
		let mut rect_split = 0usize;
		let mut scissor_vp: Option<Rect> = None;
		// (left, top, scale, color, clip, buffer)
		let mut bufs: Vec<(f32, f32, f32, [u8; 3], Option<Rect>, glyphon::Buffer)> = Vec::new();
		let clear: [u8; 3];

		match &self.content {
			Content::About { lines, .. } => {
				clear = crate::settings_ui::dialog_bg();
				for line in lines {
					let mut attrs = ui_attrs();
					attrs.color_opt =
						Some(GColor::rgb(line.color[0], line.color[1], line.color[2]));
					if line.bold {
						attrs.weight = crate::text::ui_bold_weight();
					}
					let mut buf = self.text.new_ui_buffer(w as f32, self.text.ui_line_h);
					buf.set_text(
						&mut self.text.font_system,
						&line.text,
						&attrs,
						Shaping::Advanced,
						None,
					);
					buf.shape_until_scroll(&mut self.text.font_system, false);
					bufs.push((line.x, line.y, line.scale, line.color, None, buf));
				}
			}
			Content::Settings(dialog) => {
				clear = crate::settings_ui::dialog_bg();
				let line_h = self.text.ui_line_h;
				let attrs = ui_attrs();
				let (fixed, rows) = {
					let text = &mut self.text;
					dialog.rects(line_h, |s| text.measure_ui_text(s, &attrs))
				};
				rect_split = fixed.len();
				scissor_vp = Some(dialog.viewport());
				rect_inst = fixed;
				rect_inst.extend(rows);
				let items = {
					let text = &mut self.text;
					let attrs = ui_attrs();
					dialog.texts(line_h, |s| text.measure_ui_text(s, &attrs))
				};
				for item in items {
					let mut attrs = ui_attrs();
					attrs.color_opt =
						Some(GColor::rgb(item.color[0], item.color[1], item.color[2]));
					if item.bold {
						attrs.weight = crate::text::ui_bold_weight();
					}
					let mut buf = self
						.text
						.new_ui_buffer(w as f32, self.text.ui_line_h * item.scale.max(1.0));
					buf.set_text(
						&mut self.text.font_system,
						&item.text,
						&attrs,
						Shaping::Advanced,
						None,
					);
					buf.shape_until_scroll(&mut self.text.font_system, false);
					bufs.push((item.x, item.y, item.scale, item.color, item.clip, buf));
				}
			}
		}

		let areas: Vec<TextArea> = bufs
			.iter()
			.map(|(x, y, scale, color, clip, buf)| {
				let bounds = match clip {
					Some(rect) => TextBounds {
						left: rect.x as i32,
						top: rect.y as i32,
						right: (rect.x + rect.w) as i32,
						bottom: (rect.y + rect.h) as i32,
					},
					None => TextBounds {
						left: 0,
						top: 0,
						right: w as i32,
						bottom: h as i32,
					},
				};
				TextArea {
					buffer: buf,
					left: *x,
					top: *y,
					scale: *scale,
					bounds,
					default_color: GColor::rgb(color[0], color[1], color[2]),
					custom_glyphs: &[],
				}
			})
			.collect();
		if let Err(err) = self.text.prepare(&self.gfx.device, &self.gfx.queue, areas) {
			// same atlas-full recovery as the main window: trim so the next
			// frame re-prepares with room, instead of dropping the dialog text
			eprintln!(
				"{}: dialog text prepare failed; trimming atlas: {err:?}",
				config::APP_NAME
			);
			self.text.trim_atlas();
		}
		if !rect_inst.is_empty() {
			self.rects
				.set_resolution(&self.gfx.queue, w as f32, h as f32);
			self.rects
				.upload(&self.gfx.device, &self.gfx.queue, &rect_inst);
		}

		let bg = config::srgb_f32(clear);
		let mut encoder = self
			.gfx
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor {
				label: Some("dialog"),
			});
		{
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("dialog pass"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &view,
					resolve_target: None,
					depth_slice: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Clear(wgpu::Color {
							r: bg[0] as f64,
							g: bg[1] as f64,
							b: bg[2] as f64,
							a: 1.0,
						}),
						store: wgpu::StoreOp::Store,
					},
				})],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			if !rect_inst.is_empty() {
				self.rects.draw(&mut pass, 0..rect_split as u32);
				// scrolled settings rows, clipped to the viewport
				if rect_inst.len() > rect_split {
					if let Some(vp) = scissor_vp {
						let x = vp.x.max(0.0).min(w as f32) as u32;
						let y = vp.y.max(0.0).min(h as f32) as u32;
						let sw = vp.w.max(0.0).min(w as f32 - x as f32) as u32;
						let sh = vp.h.max(0.0).min(h as f32 - y as f32) as u32;
						if sw > 0 && sh > 0 {
							pass.set_scissor_rect(x, y, sw, sh);
							self.rects
								.draw(&mut pass, rect_split as u32..rect_inst.len() as u32);
							pass.set_scissor_rect(0, 0, w, h);
						}
					}
				}
			}
			let _ = self.text.render(&mut pass);
		}
		self.gfx.queue.submit(Some(encoder.finish()));
		self.gfx.end_frame(frame);
		self.text.trim_atlas();
	}
}

// X11: make the dialog a proper transient modal of the terminal via WM hints -
// WM_TRANSIENT_FOR (winit has no API for it there; its parent_window means
// literal X reparenting), plus the EWMH dialog type and the MODAL / SKIP_TASKBAR
// states. That gives the standard Linux modal behavior (kept off the taskbar,
// stacked above and raised with its parent, retains focus) without any input
// tricks. Same throwaway-connection pattern as app::set_blur_behind. No-op off X11.
#[cfg(target_os = "linux")]
fn set_transient_for(window: &Window, parent: Option<&RawWindowHandle>) {
	use winit::raw_window_handle::HasWindowHandle;
	use x11rb::connection::Connection;
	use x11rb::protocol::xproto::{
		AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, PropMode,
	};
	use x11rb::wrapper::ConnectionExt as _;

	let Ok(handle) = window.window_handle() else {
		return;
	};
	let xid = match handle.as_raw() {
		RawWindowHandle::Xlib(h) => h.window as u32,
		RawWindowHandle::Xcb(h) => h.window.get(),
		_ => return,
	};
	let Ok((conn, screen)) = x11rb::connect(None) else {
		return;
	};
	let root = conn.setup().roots[screen].root;

	let atom = |name: &[u8]| -> Option<u32> {
		Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom)
	};
	let (Some(wt), Some(wt_dialog), Some(state), Some(modal), Some(skip)) = (
		atom(b"_NET_WM_WINDOW_TYPE"),
		atom(b"_NET_WM_WINDOW_TYPE_DIALOG"),
		atom(b"_NET_WM_STATE"),
		atom(b"_NET_WM_STATE_MODAL"),
		atom(b"_NET_WM_STATE_SKIP_TASKBAR"),
	) else {
		return;
	};

	if let Some(parent_xid) = parent.and_then(|p| match p {
		RawWindowHandle::Xlib(h) => Some(h.window as u32),
		RawWindowHandle::Xcb(h) => Some(h.window.get()),
		_ => None,
	}) {
		let _ = conn.change_property32(
			PropMode::REPLACE,
			xid,
			AtomEnum::WM_TRANSIENT_FOR,
			AtomEnum::WINDOW,
			&[parent_xid],
		);
	}
	let _ = conn.change_property32(PropMode::REPLACE, xid, wt, AtomEnum::ATOM, &[wt_dialog]);
	let _ = conn.change_property32(
		PropMode::REPLACE,
		xid,
		state,
		AtomEnum::ATOM,
		&[modal, skip],
	);

	// the window is already mapped, so also request the states via the EWMH
	// client message (action ADD=1, source = application=1) for WMs that only
	// honour a state change that way rather than a bare property write.
	let add_state = |st: u32| {
		let ev = ClientMessageEvent::new(32, xid, state, [1, st, 0, 1, 0]);
		let _ = conn.send_event(
			false,
			root,
			EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
			ev,
		);
	};
	add_state(modal);
	add_state(skip);
	let _ = conn.flush();
}
#[cfg(not(target_os = "linux"))]
fn set_transient_for(_window: &Window, _parent: Option<&RawWindowHandle>) {}

fn map_action(action: Action) -> Option<DialogAction> {
	match action {
		Action::None => None,
		Action::Apply => Some(DialogAction::Apply),
		Action::Ok => Some(DialogAction::ApplyAndClose),
		Action::Cancel => Some(DialogAction::Close),
	}
}

// Build the About content laid out at the window origin; returns
// (lines, link rect, url, (width, height)) in physical px.
fn layout_about(
	text: &mut TextCtx,
	info: &wgpu::AdapterInfo,
) -> (Vec<Line>, Rect, String, (f32, f32)) {
	let menu_fg = crate::settings_ui::dialog_text();
	let menu_dim = crate::settings_ui::dialog_dim();
	let menu_link = config::MENU_LINK;
	let accel = match info.device_type {
		wgpu::DeviceType::Cpu => "Software (CPU)",
		wgpu::DeviceType::IntegratedGpu => "Hardware (integrated GPU)",
		wgpu::DeviceType::DiscreteGpu => "Hardware (discrete GPU)",
		wgpu::DeviceType::VirtualGpu => "Hardware (virtual GPU)",
		_ => "Unknown",
	};
	let url = env!("CARGO_PKG_REPOSITORY").to_string();
	let gap = config::MENU_SEP_H;
	// build target the binary was compiled for (distinguishes the cross builds)
	let profile = if cfg!(debug_assertions) {
		"debug"
	} else {
		"release"
	};
	let build = format!(
		"{} / {} ({profile})",
		std::env::consts::ARCH,
		std::env::consts::OS
	);
	#[rustfmt::skip]
	let content: Vec<(String, [u8; 3], f32, f32, bool, f32)> = vec![
		(format!("About {}", config::APP_NAME), menu_fg, 0.0, 0.0, true, 1.5),
		(format!("version {}", env!("CARGO_PKG_VERSION")), menu_dim, 0.0, 4.0, false, 1.0),
		("Copyright © 2026 Jim Collier".into(), menu_dim, 0.0, 0.0, false, 1.0),
		(format!("License: {}", env!("CARGO_PKG_LICENSE")), menu_dim, 0.0, 0.0, false, 1.0),
		("Info".into(), menu_fg, 0.0, gap, true, 1.0),
		(format!("Build:  {build}"), menu_dim, 16.0, 2.0, false, 1.0),
		(format!("Renderer:  {}", info.name), menu_dim, 16.0, 0.0, false, 1.0),
		(format!("Backend:  {:?}", info.backend), menu_dim, 16.0, 0.0, false, 1.0),
		(format!("Acceleration:  {accel}"), menu_dim, 16.0, 0.0, false, 1.0),
		(url.clone(), menu_link, 0.0, gap, false, 1.0),
		("Click the link to open it  ·  Esc to close".into(), menu_dim, 0.0, gap, false, 1.0),
	];

	let attrs = ui_attrs();
	let pad = 20.0;
	let line_h = text.ui_line_h;
	let mut content_w: f32 = 0.0;
	let mut total_h = 0.0;
	let mut widths = Vec::with_capacity(content.len());
	for (line_text, _, indent, gap_before, _, scale) in &content {
		let width = indent + text.measure_ui_text(line_text, &attrs) * scale;
		widths.push(width);
		content_w = content_w.max(width);
		total_h += gap_before + line_h * scale;
	}
	let box_w = content_w + pad * 2.0;
	let box_h = total_h + pad * 2.0;

	let mut lines = Vec::with_capacity(content.len());
	let mut link_rect = Rect {
		x: 0.0,
		y: 0.0,
		w: 0.0,
		h: 0.0,
	};
	let mut y = pad;
	for (i, (line_text, color, indent, gap_before, bold, scale)) in content.into_iter().enumerate()
	{
		y += gap_before;
		let x = pad + indent;
		if color == menu_link {
			link_rect = Rect {
				x,
				y,
				w: widths[i],
				h: line_h,
			};
		}
		lines.push(Line {
			text: line_text,
			x,
			y,
			color,
			bold,
			scale,
		});
		y += line_h * scale;
	}
	(lines, link_rect, url, (box_w, box_h))
}
