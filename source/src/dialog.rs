// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

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
		let sz = window.inner_size();
		gfx.resize(sz.width, sz.height);
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
		let want = winit::dpi::PhysicalSize::new(size.0.ceil() as u32, size.1.ceil() as u32);
		if let Some(applied) = window.request_inner_size(want) {
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
			.map(|m| m.size().height as f32 - 38.0)
			.unwrap_or(1010.0)
			.min(1010.0);
		// laid out at the origin
		let dlg = SettingsDialog::new(0.0, 0.0, text.ui_line_h, label_w, btn_w, tab_ws, max_h);
		let (w, h) = dlg.size();
		let want = winit::dpi::PhysicalSize::new(w.ceil() as u32, h.ceil() as u32);
		if let Some(applied) = window.request_inner_size(want) {
			gfx.resize(applied.width, applied.height);
		}
		Ok(Self {
			window,
			gfx,
			text,
			rects,
			content: Content::Settings(dlg),
			mouse: (0.0, 0.0),
		})
	}

	// (orig, edited, use_system_font) for the app to apply, if this is Settings.
	pub fn settings_values(&self) -> Option<(config::Settings, config::Settings, bool)> {
		match &self.content {
			Content::Settings(d) => {
				Some((d.orig().clone(), d.edited().clone(), d.use_system_font()))
			}
			_ => None,
		}
	}

	// After an Apply, reset the settings baseline to the applied values so a later
	// Apply diffs against the live state (see SettingsDialog::commit_baseline).
	pub fn commit_baseline(&mut self) {
		if let Content::Settings(d) = &mut self.content {
			d.commit_baseline();
		}
	}

	pub fn set_cursor(&mut self, x: f32, y: f32) {
		self.mouse = (x, y);
		if let Content::Settings(d) = &mut self.content {
			d.mouse_move(x, y); // continues a slider drag
		}
	}

	pub fn mouse_down(&mut self) -> Option<DialogAction> {
		let (mx, my) = self.mouse;
		match &mut self.content {
			Content::About { link_rect, url, .. } => link_rect
				.contains(mx, my)
				.then(|| DialogAction::OpenUrl(url.clone())),
			Content::Settings(d) => {
				let (w, h) = d.size();
				// ignore clicks outside the panel (would otherwise Cancel)
				if mx < 0.0 || my < 0.0 || mx > w || my > h {
					return None;
				}
				map_action(d.mouse_down(mx, my))
			}
		}
	}

	pub fn mouse_up(&mut self) {
		if let Content::Settings(d) = &mut self.content {
			d.mouse_up();
		}
	}

	// wheel scroll for an overflowing settings tab (positive dy = scroll up)
	pub fn wheel(&mut self, dy_px: f32) {
		if let Content::Settings(d) = &mut self.content {
			d.wheel(dy_px);
		}
	}

	pub fn char_input(&mut self, c: char) {
		if let Content::Settings(d) = &mut self.content {
			d.char_input(c);
		}
	}

	// Alt held (from ModifiersChanged): underline the button accelerators.
	pub fn set_alt(&mut self, on: bool) {
		if let Content::Settings(d) = &mut self.content {
			d.set_alt(on);
		}
	}

	// A character key: while Alt is held it's an accelerator (Cancel/Apply/OK),
	// otherwise it types into the focused field.
	pub fn key_char(&mut self, c: char) -> Option<DialogAction> {
		match &mut self.content {
			Content::Settings(d) if d.alt() => map_action(d.alt_key(c)),
			Content::Settings(d) => {
				d.char_input(c);
				None
			}
			_ => None,
		}
	}

	pub fn backspace(&mut self) {
		if let Content::Settings(d) = &mut self.content {
			d.backspace();
		}
	}

	pub fn key_escape(&mut self) -> Option<DialogAction> {
		match &mut self.content {
			Content::About { .. } => Some(DialogAction::Close),
			Content::Settings(d) => map_action(d.key_escape()),
		}
	}

	pub fn key_enter(&mut self) -> Option<DialogAction> {
		match &mut self.content {
			Content::Settings(d) => map_action(d.key_enter()),
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
				for ln in lines {
					let mut a = ui_attrs();
					a.color_opt = Some(GColor::rgb(ln.color[0], ln.color[1], ln.color[2]));
					if ln.bold {
						a.weight = crate::text::ui_bold_weight();
					}
					let mut b = self.text.new_ui_buffer(w as f32, self.text.ui_line_h);
					b.set_text(
						&mut self.text.font_system,
						&ln.text,
						&a,
						Shaping::Advanced,
						None,
					);
					b.shape_until_scroll(&mut self.text.font_system, false);
					bufs.push((ln.x, ln.y, ln.scale, ln.color, None, b));
				}
			}
			Content::Settings(d) => {
				clear = crate::settings_ui::dialog_bg();
				let (fixed, rows) = d.rects(self.text.ui_line_h);
				rect_split = fixed.len();
				scissor_vp = Some(d.viewport());
				rect_inst = fixed;
				rect_inst.extend(rows);
				for it in d.texts(self.text.ui_line_h) {
					let mut a = ui_attrs();
					a.color_opt = Some(GColor::rgb(it.color[0], it.color[1], it.color[2]));
					if it.bold {
						a.weight = crate::text::ui_bold_weight();
					}
					let mut b = self
						.text
						.new_ui_buffer(w as f32, self.text.ui_line_h * it.scale.max(1.0));
					b.set_text(
						&mut self.text.font_system,
						&it.text,
						&a,
						Shaping::Advanced,
						None,
					);
					b.shape_until_scroll(&mut self.text.font_system, false);
					bufs.push((it.x, it.y, it.scale, it.color, it.clip, b));
				}
			}
		}

		let areas: Vec<TextArea> = bufs
			.iter()
			.map(|(x, y, scale, color, clip, b)| {
				let bounds = match clip {
					Some(c) => TextBounds {
						left: c.x as i32,
						top: c.y as i32,
						right: (c.x + c.w) as i32,
						bottom: (c.y + c.h) as i32,
					},
					None => TextBounds {
						left: 0,
						top: 0,
						right: w as i32,
						bottom: h as i32,
					},
				};
				TextArea {
					buffer: b,
					left: *x,
					top: *y,
					scale: *scale,
					bounds,
					default_color: GColor::rgb(color[0], color[1], color[2]),
					custom_glyphs: &[],
				}
			})
			.collect();
		if let Err(e) = self.text.prepare(&self.gfx.device, &self.gfx.queue, areas) {
			// same atlas-full recovery as the main window: trim so the next
			// frame re-prepares with room, instead of dropping the dialog text
			eprintln!(
				"{}: dialog text prepare failed; trimming atlas: {e:?}",
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
		let mut enc = self
			.gfx
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor {
				label: Some("dialog"),
			});
		{
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
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
		self.gfx.queue.submit(Some(enc.finish()));
		self.gfx.end_frame(frame);
		self.text.trim_atlas();
	}
}

// X11: tie the dialog to the terminal window with WM_TRANSIENT_FOR (winit has
// no API for it there - its parent_window means literal X reparenting). Same
// throwaway-connection pattern as app::set_blur_behind. No-op off X11.
#[cfg(target_os = "linux")]
fn set_transient_for(window: &Window, parent: Option<&RawWindowHandle>) {
	use winit::raw_window_handle::HasWindowHandle;
	use x11rb::connection::Connection;
	use x11rb::protocol::xproto::{AtomEnum, PropMode};
	use x11rb::wrapper::ConnectionExt as _;

	let parent_xid = match parent {
		Some(RawWindowHandle::Xlib(h)) => h.window as u32,
		Some(RawWindowHandle::Xcb(h)) => h.window.get(),
		_ => return,
	};
	let Ok(handle) = window.window_handle() else {
		return;
	};
	let xid = match handle.as_raw() {
		RawWindowHandle::Xlib(h) => h.window as u32,
		RawWindowHandle::Xcb(h) => h.window.get(),
		_ => return,
	};
	let Ok((conn, _)) = x11rb::connect(None) else {
		return;
	};
	let _ = conn.change_property32(
		PropMode::REPLACE,
		xid,
		AtomEnum::WM_TRANSIENT_FOR,
		AtomEnum::WINDOW,
		&[parent_xid],
	);
	let _ = conn.flush();
}
#[cfg(not(target_os = "linux"))]
fn set_transient_for(_window: &Window, _parent: Option<&RawWindowHandle>) {}

fn map_action(a: Action) -> Option<DialogAction> {
	match a {
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
		("Copyright (C) 2026 Jim Collier".into(), menu_dim, 0.0, 0.0, false, 1.0),
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
	for (t, _, indent, gap_before, _, scale) in &content {
		let wdt = indent + text.measure_ui_text(t, &attrs) * scale;
		widths.push(wdt);
		content_w = content_w.max(wdt);
		total_h += gap_before + line_h * scale;
	}
	let bw = content_w + pad * 2.0;
	let bh = total_h + pad * 2.0;

	let mut lines = Vec::with_capacity(content.len());
	let mut link_rect = Rect {
		x: 0.0,
		y: 0.0,
		w: 0.0,
		h: 0.0,
	};
	let mut y = pad;
	for (i, (t, color, indent, gap_before, bold, scale)) in content.into_iter().enumerate() {
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
			text: t,
			x,
			y,
			color,
			bold,
			scale,
		});
		y += line_h * scale;
	}
	(lines, link_rect, url, (bw, bh))
}
