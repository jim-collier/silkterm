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

// A clickable region in the About box. `button` links draw a filled box behind
// their label (the Support button); plain links are just coloured text. A link
// with a `tooltip` shows that text as a flyover while the cursor is over it -
// used so the Support button can reveal the URL it opens without baking it into
// the label.
struct AboutLink {
	rect: Rect,
	url: String,
	tooltip: Option<String>,
	button: bool,
}

enum Content {
	About {
		lines: Vec<Line>,
		links: Vec<AboutLink>,
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
	// the terminal window this dialog belongs to, so we can restack it beneath
	// us when we're activated (see raise_parent).
	parent: Option<RawWindowHandle>,
}

impl DialogWin {
	pub fn id(&self) -> WindowId {
		self.window.id()
	}

	// Restack the terminal to sit directly beneath this dialog. Called when the
	// dialog gains focus, so a window that got in front of the terminal can't
	// stay wedged between them - the transient hints alone don't force this on
	// WMs that don't raise a transient's parent with it (Compiz).
	pub fn raise_parent(&self) {
		// SILK_MODALDBG=1 traces the restack + the resulting stack order, so a WM
		// where this misbehaves (e.g. a Compiz profile that ignores the restack)
		// can be diagnosed from the terminal without a headless rig.
		let dbg = std::env::var_os("SILK_MODALDBG").is_some();
		restack_parent_below(&self.window, self.parent.as_ref(), dbg, self.kind());
	}

	fn kind(&self) -> &'static str {
		match self.content {
			Content::About { .. } => "About",
			Content::Settings(_) => "Settings",
		}
	}

	fn make(
		el: &ActiveEventLoop,
		title: String,
		w: f32,
		h: f32,
		parent: Option<RawWindowHandle>,
	) -> anyhow::Result<(Arc<Window>, Gfx, TextCtx, RectRenderer)> {
		#[allow(unused_mut)] // reassigned on linux/windows/macos below
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
		// X11: create unmapped so WM_TRANSIENT_FOR + the modal/dialog hints are all
		// set BEFORE the WM maps the window and fixes its stacking group. A post-map
		// property write is read too late by Compiz et al - that's why re-selecting
		// the dialog raised it alone and left the parent buried. The caller shows the
		// window after the final resize (see new_about / new_settings).
		#[cfg(target_os = "linux")]
		{
			attrs = attrs.with_visible(false);
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
		let (lines, links, size) = layout_about(&mut text, adapter);
		let requested_size =
			winit::dpi::PhysicalSize::new(size.0.ceil() as u32, size.1.ceil() as u32);
		if let Some(applied) = window.request_inner_size(requested_size) {
			gfx.resize(applied.width, applied.height);
		}
		// mapped last, at the final size, with the transient hints already in place
		#[cfg(target_os = "linux")]
		window.set_visible(true);
		Ok(Self {
			window,
			gfx,
			text,
			rects,
			content: Content::About { lines, links },
			mouse: (0.0, 0.0),
			parent,
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
		// mapped last, at the final size, with the transient hints already in place
		#[cfg(target_os = "linux")]
		window.set_visible(true);
		Ok(Self {
			window,
			gfx,
			text,
			rects,
			content: Content::Settings(dialog),
			mouse: (0.0, 0.0),
			parent,
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
			dialog.mouse_move(x, y); // slider drag + open-dropdown hover
		}
	}

	pub fn mouse_down(&mut self) -> Option<DialogAction> {
		let (mx, my) = self.mouse;
		match &mut self.content {
			Content::About { links, .. } => links
				.iter()
				.find(|link| link.rect.contains(mx, my))
				.map(|link| DialogAction::OpenUrl(link.url.clone())),
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
	// Ctrl+PageUp / Ctrl+PageDown: cycle tabs (PageDown = next).
	pub fn key_page(&mut self, forward: bool) {
		if let Content::Settings(dialog) = &mut self.content {
			dialog.key_page(forward);
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

		// gather rects (About button + flyover, or the Settings controls) + text
		let mut rect_inst: Vec<RectInstance> = Vec::new();
		// rects before `rect_split` draw unclipped; Settings rows after it draw
		// scissored to the scroll viewport. Both match arms set it.
		let rect_split;
		let mut scissor_vp: Option<Rect> = None;
		// (left, top, scale, color, clip, buffer)
		let mut bufs: Vec<(f32, f32, f32, [u8; 3], Option<Rect>, glyphon::Buffer)> = Vec::new();
		// end of the scissored settings rows (an open dropdown appends its popup
		// rects after this; they draw unscissored, on top, in a second pass)
		let mut rows_end = 0usize;
		let mut overlay_range: Option<(u32, u32)> = None;
		let mut ov_bufs: Vec<(f32, f32, f32, [u8; 3], Option<Rect>, glyphon::Buffer)> = Vec::new();
		let clear: [u8; 3];

		match &self.content {
			Content::About { lines, links } => {
				clear = crate::settings_ui::dialog_bg();
				let (mx, my) = self.mouse;
				let border_col = crate::settings_ui::dialog_border();
				let q = |x: f32, y: f32, bw: f32, bh: f32, color: [u8; 3]| RectInstance {
					pos: [x, y],
					size: [bw, bh],
					color: config::srgb_f32(color),
				};
				// filled boxes behind button-style links (the Support button),
				// brightened while hovered
				for link in links.iter().filter(|link| link.button) {
					let fill = if link.rect.contains(mx, my) {
						crate::settings_ui::dialog_btn_hl()
					} else {
						crate::settings_ui::dialog_btn()
					};
					let r = link.rect;
					rect_inst.push(q(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0, border_col));
					rect_inst.push(q(r.x, r.y, r.w, r.h, fill));
				}
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
				// flyover: show the destination URL of the hovered link in a small
				// box under it (the Support label hides its URL; this reveals it).
				if let Some((tip, anchor)) = links
					.iter()
					.find(|link| link.rect.contains(mx, my))
					.and_then(|link| link.tooltip.as_ref().map(|tip| (tip, link.rect)))
				{
					let attrs = ui_attrs();
					let line_h = self.text.ui_line_h;
					let tip_w = self.text.measure_ui_text(tip, &attrs);
					let (pad_x, pad_y) = (8.0, 4.0);
					let box_w = tip_w + pad_x * 2.0;
					let box_h = line_h + pad_y * 2.0;
					let bx = (anchor.x + anchor.w * 0.5 - box_w * 0.5)
						.clamp(4.0, (w as f32 - box_w - 4.0).max(4.0));
					let by = (anchor.y + anchor.h + 8.0).min((h as f32 - box_h - 4.0).max(4.0));
					rect_inst.push(q(bx - 1.0, by - 1.0, box_w + 2.0, box_h + 2.0, border_col));
					rect_inst.push(q(bx, by, box_w, box_h, crate::settings_ui::dialog_btn()));
					let dim = crate::settings_ui::dialog_dim();
					let mut a = ui_attrs();
					a.color_opt = Some(GColor::rgb(dim[0], dim[1], dim[2]));
					let mut buf = self.text.new_ui_buffer(w as f32, line_h);
					buf.set_text(&mut self.text.font_system, tip, &a, Shaping::Advanced, None);
					buf.shape_until_scroll(&mut self.text.font_system, false);
					bufs.push((bx + pad_x, by + pad_y, 1.0, dim, None, buf));
				}
				rect_split = rect_inst.len();
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
				rows_end = rect_inst.len();
				// open dropdown popup: rects appended after the rows (drawn on top,
				// unscissored, in a second pass); its text goes to the overlay renderer
				if dialog.dropdown_open() {
					let (ov_rects, ov_texts) = dialog.dropdown_overlay();
					let start = rect_inst.len() as u32;
					rect_inst.extend(ov_rects);
					overlay_range = Some((start, rect_inst.len() as u32));
					for item in ov_texts {
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
						ov_bufs.push((item.x, item.y, item.scale, item.color, item.clip, buf));
					}
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
		// open-dropdown popup text prepared into the overlay renderer (second pass)
		if overlay_range.is_some() {
			let ov_areas: Vec<TextArea> = ov_bufs
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
			if let Err(err) = self
				.text
				.prepare_overlay(&self.gfx.device, &self.gfx.queue, ov_areas)
			{
				eprintln!(
					"{}: dialog overlay prepare failed; trimming atlas: {err:?}",
					config::APP_NAME
				);
				self.text.trim_atlas();
			}
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
				if rows_end > rect_split {
					if let Some(vp) = scissor_vp {
						let x = vp.x.max(0.0).min(w as f32) as u32;
						let y = vp.y.max(0.0).min(h as f32) as u32;
						let sw = vp.w.max(0.0).min(w as f32 - x as f32) as u32;
						let sh = vp.h.max(0.0).min(h as f32 - y as f32) as u32;
						if sw > 0 && sh > 0 {
							pass.set_scissor_rect(x, y, sw, sh);
							self.rects
								.draw(&mut pass, rect_split as u32..rows_end as u32);
							pass.set_scissor_rect(0, 0, w, h);
						}
					}
				}
			}
			let _ = self.text.render(&mut pass);
		}
		// second pass: the open dropdown popup on top (preserves the first pass)
		if let Some((start, end)) = overlay_range {
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("dialog overlay pass"),
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
			self.rects.draw(&mut pass, start..end);
			let _ = self.text.render_overlay(&mut pass);
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

// Keep the parent (terminal) window directly below `dialog` in the stack, via an
// EWMH _NET_RESTACK_WINDOW client message to the root window. xfwm4/GNOME raise a
// transient's parent with it automatically; Compiz does not, so when the dialog
// is re-activated after another window came forward, the terminal stays buried
// behind that window - this slots it back just beneath the dialog. The message
// goes to root (the only stacking path Compiz honours for a managed window: it
// reparents clients into decoration frames, so a direct XConfigureWindow on the
// client isn't redirected to the WM and does nothing). Focus is untouched.
#[cfg(target_os = "linux")]
fn restack_parent_below(dialog: &Window, parent: Option<&RawWindowHandle>, dbg: bool, kind: &str) {
	use winit::raw_window_handle::HasWindowHandle;
	use x11rb::connection::Connection;
	use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask};

	let xid = |h: &RawWindowHandle| -> Option<u32> {
		match h {
			RawWindowHandle::Xlib(x) => Some(x.window as u32),
			RawWindowHandle::Xcb(x) => Some(x.window.get()),
			_ => None,
		}
	};
	let Some(parent_xid) = parent.and_then(xid) else {
		return;
	};
	let Ok(handle) = dialog.window_handle() else {
		return;
	};
	let Some(dlg_xid) = xid(&handle.as_raw()) else {
		return;
	};
	let Ok((conn, screen)) = x11rb::connect(None) else {
		return;
	};
	let root = conn.setup().roots[screen].root;
	let atom = |name: &[u8]| -> Option<u32> {
		Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom)
	};
	let Some(restack) = atom(b"_NET_RESTACK_WINDOW") else {
		return;
	};
	let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
	// terminal below the dialog (source = application(1), sibling = dialog,
	// detail = Below(1)).
	let ev = ClientMessageEvent::new(32, parent_xid, restack, [1, dlg_xid, 1, 0, 0]);
	let _ = conn.send_event(false, root, mask, ev);
	let _ = conn.flush();

	if dbg {
		// read back where the WM actually put things (parent should end up right
		// below dialog). Prints "term below dialog" or the offending gap.
		let order = atom(b"_NET_CLIENT_LIST_STACKING")
			.and_then(|prop| {
				conn.get_property(false, root, prop, AtomEnum::WINDOW, 0, 1024)
					.ok()?
					.reply()
					.ok()
			})
			.map(|reply| reply.value32().map(Iterator::collect).unwrap_or_default())
			.unwrap_or_else(Vec::new);
		let pos = |w: u32| order.iter().position(|&x| x == w);
		let (tp, dp) = (pos(parent_xid), pos(dlg_xid));
		let ok = matches!((tp, dp), (Some(t), Some(d)) if t + 1 == d);
		eprintln!(
			"[modal] {kind}: restack term={parent_xid:#x} below dialog={dlg_xid:#x} -> \
			 term_pos={tp:?} dialog_pos={dp:?} adjacent={ok}"
		);
	}
}
#[cfg(not(target_os = "linux"))]
fn restack_parent_below(_d: &Window, _p: Option<&RawWindowHandle>, _dbg: bool, _kind: &str) {}

fn map_action(action: Action) -> Option<DialogAction> {
	match action {
		Action::None => None,
		Action::Apply => Some(DialogAction::Apply),
		Action::Ok => Some(DialogAction::ApplyAndClose),
		Action::Cancel => Some(DialogAction::Close),
	}
}

// Build the About content laid out at the window origin; returns
// (lines, clickable links, (width, height)) in physical px.
fn layout_about(
	text: &mut TextCtx,
	info: &wgpu::AdapterInfo,
) -> (Vec<Line>, Vec<AboutLink>, (f32, f32)) {
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
	let repo_url = env!("CARGO_PKG_REPOSITORY").to_string();
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
		(repo_url.clone(), menu_link, 0.0, gap, false, 1.0),
		("Click a link to open it in your browser  ·  Esc to close".into(), menu_dim, 0.0, gap, false, 1.0),
	];

	let attrs = ui_attrs();
	let pad = 20.0;
	let line_h = text.ui_line_h;
	let mut content_w: f32 = 0.0;
	let mut widths = Vec::with_capacity(content.len());
	for (line_text, _, indent, _, _, scale) in &content {
		let width = indent + text.measure_ui_text(line_text, &attrs) * scale;
		widths.push(width);
		content_w = content_w.max(width);
	}

	// Support button: a filled box with a centred label; opens DONATE.md and
	// reveals that URL as a flyover on hover (config::DONATE_URL).
	let btn_label = "Support SilkTerm!";
	let (btn_pad_x, btn_pad_y) = (16.0, 8.0);
	let label_w = text.measure_ui_text(btn_label, &attrs);
	let btn_w = label_w + btn_pad_x * 2.0;
	let btn_h = line_h + btn_pad_y * 2.0;
	content_w = content_w.max(btn_w);
	// the button's hover flyover shows the full donate URL; size the window so it
	// isn't clipped
	content_w = content_w.max(text.measure_ui_text(config::DONATE_URL, &attrs));
	let box_w = content_w + pad * 2.0;

	let mut lines = Vec::with_capacity(content.len() + 1);
	let mut links = Vec::with_capacity(2);
	let mut y = pad;
	for (i, (line_text, color, indent, gap_before, bold, scale)) in content.into_iter().enumerate()
	{
		y += gap_before;
		let x = pad + indent;
		if color == menu_link {
			links.push(AboutLink {
				rect: Rect {
					x,
					y,
					w: widths[i],
					h: line_h,
				},
				url: repo_url.clone(),
				tooltip: None,
				button: false,
			});
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

	// Support button below the text, centred in the content column
	y += gap * 1.5;
	let btn_x = pad + (content_w - btn_w) * 0.5;
	links.push(AboutLink {
		rect: Rect {
			x: btn_x,
			y,
			w: btn_w,
			h: btn_h,
		},
		url: config::DONATE_URL.to_string(),
		tooltip: Some(config::DONATE_URL.to_string()),
		button: true,
	});
	lines.push(Line {
		text: btn_label.into(),
		x: btn_x + (btn_w - label_w) * 0.5,
		y: y + btn_pad_y,
		color: menu_fg,
		bold: true,
		scale: 1.0,
	});
	y += btn_h;

	// leave room below the button for the URL flyover to appear on hover
	let box_h = y + pad + line_h + 14.0;
	(lines, links, (box_w, box_h))
}
