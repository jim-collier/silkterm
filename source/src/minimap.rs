// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

//! Minimap: the whole scroll buffer in miniature, in its own column beside the
//! text. The buffer always maps linearly onto the column and never slides, so
//! the highlight over the preview and the thumb in the far-edge bar are one
//! object at the same pixels. See design.md for why that matters.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor};

use crate::config;
use crate::palette;
use crate::pane::Rect;

// The slim always-visible scrollbar at the far edge, in DIP.
pub const BAR_W: f32 = 8.0;
// Shortest the viewport handle may draw, so a deep buffer still leaves
// something to grab.
const MIN_HANDLE: f32 = 14.0;
// Tallest one buffer line draws. A short buffer stops short of the column's
// bottom rather than stretching to fill it.
const MAX_LINE_PX: f32 = 2.0;
// What a non-blank cell contributes to its pixel. Below 1 so a run of text
// reads as a bar rather than a slab.
const INK: f32 = 0.85;
// A text line does not fill its own height, and once a line draws more than a
// pixel tall the gap above and below is what keeps a page of text from reading
// as one block. This is the ink's share of the line at the tallest a line ever
// draws; below a pixel there is no room for a gap and the line is taken whole.
const BAND: f32 = 0.5;
// How far down the line the ink starts, at that same tallest.
const BAND_TOP: f32 = 0.1;
// A pixel row's ink is what actually landed in it, so mostly blank lines read
// dimmer than a solid page. One line among many still has to be findable, so
// it never falls below this share of its own strength.
const LONE: f32 = 0.45;
// Preview opacity. The column sits over the pane background (and the wallpaper
// through it), so the miniature stays a hint rather than a second screen.
const PREVIEW_A: f32 = 0.72;
// Slowest the preview is allowed to recompose. Under a flood the buffer shifts
// every frame and every pixel of the map moves with it, so this is what keeps
// a feature that is only a hint from costing what the text costs.
const COMPOSE_MS: u64 = 90;
// A cache that has fallen behind the grid is rebuilt whole, at most this often.
const RESYNC_MS: u64 = 400;

// Rasterized buffer line: one RGBA byte group per preview pixel, straight
// (not premultiplied) - the shader premultiplies in linear light.
type Row = Vec<u8>;

// The column's pieces for one pane, in absolute window px. `handle` is the
// viewport marker; None on the alt screen, where there is nothing to scroll.
#[derive(Clone, Copy, Debug)]
pub struct Geom {
	pub preview: Rect,
	pub bar: Rect,
	pub handle: Option<Rect>,
}

// Total width of the column (preview plus bar), 0 when the minimap is off or
// the pane is too narrow to give up the room.
pub fn column_w(cfg: &config::Settings, pane_w: f32, scale: f32) -> f32 {
	if !cfg.minimap {
		return 0.0;
	}
	let bar = config::dip(BAR_W, scale);
	let want = config::dip(cfg.minimap_width, scale) + bar;
	let w = want.min((pane_w * 0.5).floor());
	if w < bar * 2.0 { 0.0 } else { w }
}

// The part of a pane's area the terminal text gets.
pub fn text_rect(full: Rect, cfg: &config::Settings, scale: f32) -> Rect {
	Rect {
		w: (full.w - column_w(cfg, full.w, scale)).max(0.0),
		..full
	}
}

// How tall one buffer line draws. Capped, so a buffer shorter than the column
// simply does not reach the bottom of it.
fn line_px(track_h: f32, total: usize, scale: f32) -> f32 {
	if total == 0 {
		return 0.0;
	}
	(track_h / total as f32).min(config::dip(MAX_LINE_PX, scale))
}

// Where the viewport marker sits, as (y offset down the track, height). `pos`
// is the scroll model's lines-back-from-the-bottom, the same number the
// scrollbar rides.
fn handle_span(track_h: f32, total: usize, rows: usize, pos: f32, scale: f32) -> (f32, f32) {
	let lh = line_px(track_h, total, scale);
	let used = lh * total as f32;
	let h = (rows as f32 * lh)
		.max(config::dip(MIN_HANDLE, scale))
		.min(used);
	let top = (total.saturating_sub(rows) as f32 - pos).max(0.0);
	let y = (top * lh).clamp(0.0, (used - h).max(0.0));
	(y, h)
}

// Inverse of `handle_span`: a marker top back to a scroll position in lines.
fn span_to_pos(track_h: f32, total: usize, rows: usize, y: f32, scale: f32) -> f32 {
	let lh = line_px(track_h, total, scale);
	if lh <= 0.0 {
		return 0.0;
	}
	total.saturating_sub(rows) as f32 - y / lh
}

// The column's geometry for a pane. `pos` rides the eased scroll position;
// `alt` drops the marker.
pub fn geom(
	full: Rect,
	margin: f32,
	scale: f32,
	cfg: &config::Settings,
	total: usize,
	rows: usize,
	pos: f32,
	alt: bool,
) -> Option<Geom> {
	let w = column_w(cfg, full.w, scale);
	let h = (full.h - 2.0 * margin).max(0.0);
	if w <= 0.0 || h <= 0.0 {
		return None;
	}
	let bar_w = config::dip(BAR_W, scale);
	let preview = Rect {
		x: full.x + full.w - w,
		y: full.y + margin,
		w: w - bar_w,
		h,
	};
	let bar = Rect {
		x: preview.x + preview.w,
		y: preview.y,
		w: bar_w,
		h,
	};
	let handle = (!alt && total > rows).then(|| {
		let (y, hh) = handle_span(h, total, rows, pos, scale);
		Rect {
			x: preview.x,
			y: preview.y + y,
			w,
			h: hh,
		}
	});
	Some(Geom {
		preview,
		bar,
		handle,
	})
}

// Where a press in the column landed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
	Handle,
	Track,
}

// Where a press at (x, y) landed, if it landed on the column at all.
pub fn hit(g: &Geom, x: f32, y: f32) -> Option<Hit> {
	let col = Rect {
		x: g.preview.x,
		y: g.preview.y,
		w: g.preview.w + g.bar.w,
		h: g.preview.h,
	};
	if !col.contains(x, y) {
		return None;
	}
	let handle = g.handle?;
	Some(if y >= handle.y && y < handle.y + handle.h {
		Hit::Handle
	} else {
		Hit::Track
	})
}

// The scroll position a click at `y` should center the viewport on.
pub fn center_on(g: &Geom, total: usize, rows: usize, y: f32, scale: f32) -> f32 {
	let want = (y - g.preview.y - rows as f32 * line_px(g.preview.h, total, scale) * 0.5).max(0.0);
	span_to_pos(g.preview.h, total, rows, want, scale).clamp(0.0, total.saturating_sub(rows) as f32)
}

// Drag: put the marker's top where the pointer says, and map back to lines.
pub fn drag_to(g: &Geom, total: usize, rows: usize, top: f32, scale: f32) -> f32 {
	span_to_pos(
		g.preview.h,
		total,
		rows,
		(top - g.preview.y).max(0.0),
		scale,
	)
	.clamp(0.0, total.saturating_sub(rows) as f32)
}

// ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
// Per-pane cache
// ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

// A pane's rasterized buffer plus the image composed from it. History lines
// never change, so each one rasterizes once, when it scrolls off; only the
// live screen rows are redone per frame.
#[derive(Default)]
pub struct Minimap {
	rows: VecDeque<Row>,
	spare: Vec<Row>,
	hist: usize, // how many of `rows` are history rather than screen
	width: usize,
	cols: usize,
	lines: usize,
	// composed image, `width` px wide by `img_h` tall, straight RGBA
	img: Vec<u8>,
	img_h: usize,
	pub rev: u64, // bumped on every compose, so the renderer can skip re-uploads
	// rows changed since the last compose, and the compose was throttled out -
	// the pane reports this as animation so the frame after picks it up
	pending: bool,
	last_compose: Option<Instant>,
	stale_since: Option<Instant>,
	tail: u64, // fingerprint of the newest history line
	acc: Acc,
}

// Per-dest-row accumulators, kept so a compose allocates nothing.
#[derive(Default)]
struct Acc {
	rgb: Vec<f32>,
	weight: Vec<f32>,
	alpha: Vec<f32>,
}

impl Minimap {
	pub fn image(&self) -> (&[u8], usize, usize) {
		(&self.img, self.width, self.img_h)
	}

	// Free everything. Called when the column goes away.
	pub fn clear(&mut self) {
		*self = Self::default();
	}

	// Fold this build's grid into the cache and recompose if it is time.
	// `advanced` is the count of lines that entered history since the last
	// build - the same number the output ease rides.
	#[allow(clippy::too_many_arguments)]
	pub fn update(
		&mut self,
		grid: &Grid<Cell>,
		colors: &Colors,
		cfg: &config::Settings,
		width: usize,
		img_h: usize,
		scale: f32,
		lines: usize,
		cols: usize,
		advanced: usize,
		cut: bool,
	) {
		if width == 0 || img_h == 0 || lines == 0 || cols == 0 {
			return;
		}
		let hist = grid.history_size();
		let now = Instant::now();
		let mut rebuild = cut
			|| self.rows.is_empty()
			|| self.width != width
			|| self.cols != cols
			|| self.lines != lines
			|| advanced > hist
			|| self.hist + advanced < hist;
		// Scrolled back with a full scrollback: nothing reports the push count, so
		// a changed newest-history line is the only sign the cache has fallen
		// behind. Rebuilding is the whole cache, so it waits out RESYNC_MS.
		let tail = if hist > 0 {
			row_hash(grid, Line(-1), cols)
		} else {
			0
		};
		if !rebuild && advanced == 0 && tail != self.tail {
			let since = *self.stale_since.get_or_insert(now);
			rebuild = now.duration_since(since) >= Duration::from_millis(RESYNC_MS);
		}

		self.width = width;
		self.cols = cols;
		self.lines = lines;
		let mut readable = palette::Readable::default();
		if rebuild {
			self.recycle_from(0);
			for line in -(hist as i32)..lines as i32 {
				let row = self.take_row();
				self.raster(grid, Line(line), colors, cfg, &mut readable, row);
			}
			self.hist = hist;
		} else {
			self.recycle_from(self.hist);
			for k in (1..=advanced).rev() {
				let row = self.take_row();
				self.raster(grid, Line(-(k as i32)), colors, cfg, &mut readable, row);
			}
			self.hist += advanced;
			while self.hist > hist {
				if let Some(row) = self.rows.pop_front() {
					self.spare.push(row);
				}
				self.hist -= 1;
			}
			for line in 0..lines as i32 {
				let row = self.take_row();
				self.raster(grid, Line(line), colors, cfg, &mut readable, row);
			}
		}
		self.tail = tail;
		self.stale_since = None;

		let due = self
			.last_compose
			.is_none_or(|t| now.duration_since(t) >= Duration::from_millis(COMPOSE_MS));
		if due || self.img_h != img_h {
			self.last_compose = Some(now);
			self.compose(img_h, scale);
		} else {
			self.pending = true;
		}
	}

	// A compose is owed. The build gate reads this so the next pass pays it,
	// rather than leaving the map a step behind once output stops.
	pub fn pending(&self) -> bool {
		self.pending
	}

	// When that compose comes due. A timed wake rather than an animation flag:
	// marking the window animating would bring it straight back, find the
	// throttle still closed, and spin at the frame rate.
	pub fn wake(&self) -> Option<Instant> {
		let at = self.last_compose?;
		self.pending.then(|| at + Duration::from_millis(COMPOSE_MS))
	}

	// Drop cached rows from `keep` onward, holding on to the allocations.
	fn recycle_from(&mut self, keep: usize) {
		while self.rows.len() > keep {
			if let Some(row) = self.rows.pop_back() {
				self.spare.push(row);
			}
		}
		if keep == 0 {
			self.hist = 0;
		}
	}

	fn take_row(&mut self) -> Row {
		self.spare.pop().unwrap_or_default()
	}

	// One grid line to one strip of preview pixels. Blank cells are skipped
	// before any color is resolved, which is most of a terminal buffer.
	fn raster(
		&mut self,
		grid: &Grid<Cell>,
		line: Line,
		colors: &Colors,
		cfg: &config::Settings,
		readable: &mut palette::Readable,
		mut out: Row,
	) {
		let width = self.width;
		out.clear();
		out.resize(width * 4, 0);
		let acc = &mut self.acc;
		acc.reset(width);
		let row = &grid[line];
		let per_cell = width as f32 / self.cols as f32;
		for c in 0..self.cols {
			let cell = &row[Column(c)];
			if blank(cell) {
				continue;
			}
			let mut fg = palette::resolve(cell.fg, colors, cfg);
			let mut bg = palette::resolve(cell.bg, colors, cfg);
			if cell.flags.contains(Flags::INVERSE) {
				std::mem::swap(&mut fg, &mut bg);
			}
			if cell.flags.contains(Flags::HIDDEN) {
				fg = bg;
			}
			fg = readable.get(fg, bg, cfg.text_min_contrast);
			let ink = if cell.c == ' ' { 0.0 } else { INK };
			// A cell with its own background paints solid; otherwise only its ink
			// shows, so an indented or short line reads as one.
			let (rgb, alpha) = if bg == cfg.bg {
				(fg, ink)
			} else {
				(mix(bg, fg, ink), 1.0)
			};
			let x0 = c as f32 * per_cell;
			let x1 = x0 + per_cell;
			let first = x0.floor() as usize;
			let last = ((x1.ceil() as usize).max(first + 1)).min(width);
			for px in first..last {
				let lo = x0.max(px as f32);
				let hi = x1.min(px as f32 + 1.0);
				let w = (hi - lo).max(0.0) * alpha;
				if w <= 0.0 {
					continue;
				}
				acc.rgb[px * 3] += rgb[0] as f32 * w;
				acc.rgb[px * 3 + 1] += rgb[1] as f32 * w;
				acc.rgb[px * 3 + 2] += rgb[2] as f32 * w;
				acc.weight[px] += w;
			}
		}
		for px in 0..width {
			let w = acc.weight[px];
			if w <= 0.0 {
				continue;
			}
			out[px * 4] = (acc.rgb[px * 3] / w).round() as u8;
			out[px * 4 + 1] = (acc.rgb[px * 3 + 1] / w).round() as u8;
			out[px * 4 + 2] = (acc.rgb[px * 3 + 2] / w).round() as u8;
			out[px * 4 + 3] = (w.min(1.0) * 255.0).round() as u8;
		}
		self.rows.push_back(out);
	}

	// Where a line's ink sits inside the `lh` pixels the line occupies, as
	// (offset, height). Ramped between whole-line and BAND so the map does not
	// change brightness as a growing buffer crosses a pixel per line.
	fn band(lh: f32) -> (f32, f32) {
		let t = (lh - 1.0).clamp(0.0, 1.0);
		(lh * BAND_TOP * t, lh * (1.0 - t * (1.0 - BAND)))
	}

	// Squash the cached rows into the column image. Colour is the average of the
	// lines that actually have ink, so a lone red line is not washed out by its
	// blank neighbours; how bright the pixel gets is how much ink landed in it.
	fn compose(&mut self, img_h: usize, scale: f32) {
		let width = self.width;
		let total = self.rows.len();
		self.img_h = img_h;
		self.img.clear();
		self.img.resize(width * img_h * 4, 0);
		self.pending = false;
		self.rev = self.rev.wrapping_add(1);
		if total == 0 || width == 0 {
			return;
		}
		let lh = line_px(img_h as f32, total, scale);
		if lh <= 0.0 {
			return;
		}
		let used = (lh * total as f32).ceil().min(img_h as f32) as usize;
		// The band takes ink out of the line; putting it back concentrated keeps
		// a solid page as bright as it was, with the gap between lines showing.
		let (band_top, band_h) = Self::band(lh);
		let gain = if band_h > 0.0 { lh / band_h } else { 0.0 };
		for py in 0..used {
			let acc = &mut self.acc;
			acc.reset(width);
			let y0 = py as f32;
			let y1 = y0 + 1.0;
			let first = ((y0 / lh).floor() as usize).min(total.saturating_sub(1));
			let last = ((y1 / lh).ceil() as usize).clamp(first + 1, total);
			for i in first..last {
				let top = i as f32 * lh + band_top;
				let lo = top.max(y0);
				let hi = (top + band_h).min(y1);
				let cover = (hi - lo).max(0.0) * gain;
				if cover <= 0.0 {
					continue;
				}
				let row = &self.rows[i];
				for px in 0..width {
					let a = row[px * 4 + 3] as f32 / 255.0;
					if a <= 0.0 {
						continue;
					}
					let w = a * cover;
					acc.rgb[px * 3] += row[px * 4] as f32 * w;
					acc.rgb[px * 3 + 1] += row[px * 4 + 1] as f32 * w;
					acc.rgb[px * 3 + 2] += row[px * 4 + 2] as f32 * w;
					acc.weight[px] += w;
					if a > acc.alpha[px] {
						acc.alpha[px] = a;
					}
				}
			}
			let base = py * width * 4;
			for px in 0..width {
				let w = acc.weight[px];
				if w <= 0.0 {
					continue;
				}
				self.img[base + px * 4] = (acc.rgb[px * 3] / w).round() as u8;
				self.img[base + px * 4 + 1] = (acc.rgb[px * 3 + 1] / w).round() as u8;
				self.img[base + px * 4 + 2] = (acc.rgb[px * 3 + 2] / w).round() as u8;
				let a = w.min(1.0).max(acc.alpha[px] * LONE);
				self.img[base + px * 4 + 3] = (a * 255.0).round() as u8;
			}
		}
	}
}

impl Acc {
	fn reset(&mut self, width: usize) {
		self.rgb.clear();
		self.rgb.resize(width * 3, 0.0);
		self.weight.clear();
		self.weight.resize(width, 0.0);
		self.alpha.clear();
		self.alpha.resize(width, 0.0);
	}
}

// Nothing to draw: an unstyled space. Checked before any color is resolved,
// which is what keeps rasterizing a mostly-empty buffer cheap.
fn blank(cell: &Cell) -> bool {
	cell.c == ' '
		&& cell.bg == Color::Named(NamedColor::Background)
		&& !cell.flags.intersects(Flags::INVERSE)
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
	let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
	[m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}

fn row_hash(grid: &Grid<Cell>, line: Line, cols: usize) -> u64 {
	let row = &grid[line];
	let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
	for c in 0..cols {
		hash = (hash ^ row[Column(c)].c as u64).wrapping_mul(0x100_0000_01b3);
	}
	hash
}

// ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
// Renderer
// ••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
	resolution: [f32; 2],
	pos: [f32; 2],
	size: [f32; 2],
	alpha: f32,
	_pad: f32,
}

struct PaneTex {
	texture: wgpu::Texture,
	bind: wgpu::BindGroup,
	uniform: wgpu::Buffer,
	w: u32,
	h: u32,
	rev: u64,
	used: bool,
}

// One textured quad per pane. Each pane owns its texture and uniform, so the
// column can be drawn with one draw call each inside the main pass.
pub struct MapRenderer {
	pipeline: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	sampler: wgpu::Sampler,
	panes: HashMap<u64, PaneTex>,
}

impl MapRenderer {
	pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("minimap bgl"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
					ty: wgpu::BindingType::Buffer {
						ty: wgpu::BufferBindingType::Uniform,
						has_dynamic_offset: false,
						min_binding_size: None,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Float { filterable: true },
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 2,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
					count: None,
				},
			],
		});
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some("minimap sampler"),
			mag_filter: wgpu::FilterMode::Nearest,
			min_filter: wgpu::FilterMode::Nearest,
			..Default::default()
		});
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("minimap shader"),
			source: wgpu::ShaderSource::Wgsl(MAP_WGSL.into()),
		});
		let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("minimap layout"),
			bind_group_layouts: &[Some(&layout)],
			immediate_size: 0,
		});
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("minimap pipeline"),
			layout: Some(&pipeline_layout),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs"),
				compilation_options: Default::default(),
				buffers: &[],
			},
			fragment: Some(wgpu::FragmentState {
				module: &shader,
				entry_point: Some("fs"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState {
					format,
					blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
					write_mask: wgpu::ColorWrites::ALL,
				})],
			}),
			primitive: wgpu::PrimitiveState {
				topology: wgpu::PrimitiveTopology::TriangleStrip,
				..Default::default()
			},
			depth_stencil: None,
			multisample: wgpu::MultisampleState::default(),
			multiview_mask: None,
			cache: None,
		});
		Self {
			pipeline,
			layout,
			sampler,
			panes: HashMap::new(),
		}
	}

	pub fn begin_frame(&mut self) {
		for p in self.panes.values_mut() {
			p.used = false;
		}
	}

	// Upload a pane's column image and place its quad. Must run before the pass
	// that draws it.
	#[allow(clippy::too_many_arguments)]
	pub fn prepare(
		&mut self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		id: u64,
		at: Rect,
		res: (f32, f32),
		map: &Minimap,
	) {
		let (pixels, w, h) = map.image();
		if pixels.is_empty() || w == 0 || h == 0 {
			return;
		}
		let (w, h) = (w as u32, h as u32);
		if self.panes.get(&id).is_none_or(|p| p.w != w || p.h != h) {
			self.panes
				.insert(id, make_tex(device, &self.layout, &self.sampler, w, h));
		}
		let Some(entry) = self.panes.get_mut(&id) else {
			return;
		};
		entry.used = true;
		if entry.rev != map.rev {
			entry.rev = map.rev;
			queue.write_texture(
				wgpu::TexelCopyTextureInfo {
					texture: &entry.texture,
					mip_level: 0,
					origin: wgpu::Origin3d::ZERO,
					aspect: wgpu::TextureAspect::All,
				},
				pixels,
				wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(4 * w),
					rows_per_image: Some(h),
				},
				wgpu::Extent3d {
					width: w,
					height: h,
					depth_or_array_layers: 1,
				},
			);
		}
		queue.write_buffer(
			&entry.uniform,
			0,
			bytemuck::bytes_of(&Uniform {
				resolution: [res.0, res.1],
				pos: [at.x, at.y],
				size: [at.w, at.h],
				alpha: PREVIEW_A,
				_pad: 0.0,
			}),
		);
	}

	pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, id: u64) {
		let Some(p) = self.panes.get(&id) else { return };
		if !p.used {
			return;
		}
		pass.set_pipeline(&self.pipeline);
		pass.set_bind_group(0, &p.bind, &[]);
		pass.draw(0..4, 0..1);
	}

	// Release the textures of panes that drew nothing this frame (closed, or
	// the column switched off).
	pub fn end_frame(&mut self) {
		self.panes.retain(|_, p| p.used);
	}
}

fn make_tex(
	device: &wgpu::Device,
	layout: &wgpu::BindGroupLayout,
	sampler: &wgpu::Sampler,
	w: u32,
	h: u32,
) -> PaneTex {
	let texture = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("minimap tex"),
		size: wgpu::Extent3d {
			width: w,
			height: h,
			depth_or_array_layers: 1,
		},
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: wgpu::TextureFormat::Rgba8UnormSrgb,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	});
	let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
	let uniform = device.create_buffer(&wgpu::BufferDescriptor {
		label: Some("minimap uniform"),
		size: std::mem::size_of::<Uniform>() as u64,
		usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
		mapped_at_creation: false,
	});
	let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
		label: Some("minimap bind"),
		layout,
		entries: &[
			wgpu::BindGroupEntry {
				binding: 0,
				resource: uniform.as_entire_binding(),
			},
			wgpu::BindGroupEntry {
				binding: 1,
				resource: wgpu::BindingResource::TextureView(&view),
			},
			wgpu::BindGroupEntry {
				binding: 2,
				resource: wgpu::BindingResource::Sampler(sampler),
			},
		],
	});
	PaneTex {
		texture,
		bind,
		uniform,
		w,
		h,
		rev: u64::MAX,
		used: false,
	}
}

const MAP_WGSL: &str = r"
struct Uniform {
    resolution: vec2<f32>,
    pos: vec2<f32>,
    size: vec2<f32>,
    alpha: f32,
    pad: f32,
};
@group(0) @binding(0) var<uniform> u: Uniform;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let px = u.pos + corner * u.size;
    var out: VOut;
    out.pos = vec4<f32>(px.x / u.resolution.x * 2.0 - 1.0, 1.0 - px.y / u.resolution.y * 2.0, 0.0, 1.0);
    out.uv = corner;
    return out;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, in.uv);
    let a = c.a * u.alpha;
    return vec4<f32>(c.rgb * a, a); // premultiplied
}
";

#[cfg(test)]
impl Minimap {
	// Seed the cache directly, so the compose can be driven without a live grid.
	fn seed(&mut self, width: usize, rows: Vec<Row>) {
		self.width = width;
		self.rows = rows.into();
	}
	fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
		let i = (y * self.width + x) * 4;
		[
			self.img[i],
			self.img[i + 1],
			self.img[i + 2],
			self.img[i + 3],
		]
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// One rasterized line: `ink` pixels of `rgb` at full coverage, rest empty.
	fn row(width: usize, ink: usize, rgb: [u8; 3]) -> Row {
		let mut out = vec![0u8; width * 4];
		for px in 0..ink {
			out[px * 4] = rgb[0];
			out[px * 4 + 1] = rgb[1];
			out[px * 4 + 2] = rgb[2];
			out[px * 4 + 3] = 255;
		}
		out
	}

	fn cfg(on: bool, width: f32) -> config::Settings {
		config::Settings {
			minimap: on,
			minimap_width: width,
			..Default::default()
		}
	}

	#[test]
	fn off_costs_the_pane_nothing() {
		let full = Rect {
			x: 0.0,
			y: 0.0,
			w: 800.0,
			h: 600.0,
		};
		let text = text_rect(full, &cfg(false, 100.0), 1.0);
		assert_eq!(text.w, full.w);
		assert_eq!(column_w(&cfg(false, 100.0), full.w, 1.0), 0.0);
	}

	#[test]
	fn the_column_takes_its_width_plus_the_bar() {
		let s = cfg(true, 100.0);
		assert_eq!(column_w(&s, 800.0, 1.0), 108.0);
		let full = Rect {
			x: 10.0,
			y: 20.0,
			w: 800.0,
			h: 600.0,
		};
		assert_eq!(text_rect(full, &s, 1.0).w, 692.0);
	}

	#[test]
	fn a_narrow_pane_gives_up_the_column() {
		// half a pane is the most the column may take, and below two bar widths
		// there is nothing worth showing
		assert_eq!(column_w(&cfg(true, 100.0), 20.0, 1.0), 0.0);
		assert_eq!(column_w(&cfg(true, 100.0), 120.0, 1.0), 60.0);
	}

	#[test]
	fn a_short_buffer_does_not_stretch_to_fill() {
		// 50 lines in a 600px column: capped at 2px each, so 100px is used
		let lh = line_px(600.0, 50, 1.0);
		assert_eq!(lh, 2.0);
		// 10,000 lines compress instead
		assert!(line_px(600.0, 10_000, 1.0) < 0.1);
	}

	#[test]
	fn the_handle_rides_the_scroll_position() {
		let (top_y, _) = handle_span(600.0, 1000, 40, 960.0, 1.0);
		assert_eq!(top_y, 0.0); // scrolled to the oldest line
		let (bot_y, bot_h) = handle_span(600.0, 1000, 40, 0.0, 1.0);
		let used = line_px(600.0, 1000, 1.0) * 1000.0;
		assert!((bot_y + bot_h - used).abs() < 0.01); // at the newest
	}

	#[test]
	fn a_drag_round_trips_through_the_mapping() {
		let track = 600.0;
		let (total, rows) = (1000, 40);
		for pos in [0.0, 120.0, 500.0, 960.0] {
			let (y, _) = handle_span(track, total, rows, pos, 1.0);
			let back = span_to_pos(track, total, rows, y, 1.0);
			assert!((back - pos).abs() < 1.0, "{pos} -> {back}");
		}
	}

	#[test]
	fn the_handle_stays_grabbable_on_a_deep_buffer() {
		let (_, h) = handle_span(600.0, 100_000, 40, 0.0, 1.0);
		assert!(h >= MIN_HANDLE);
	}

	#[test]
	fn a_line_composes_where_the_mapping_puts_it() {
		// 100 lines in a 200px column: 2px each, and line 40 is the only red one
		let width = 8;
		let mut rows: Vec<Row> = (0..100).map(|_| row(width, 4, [0, 200, 0])).collect();
		rows[40] = row(width, 4, [200, 0, 0]);
		let mut map = Minimap::default();
		map.seed(width, rows);
		map.compose(200, 1.0);
		assert_eq!(map.pixel(0, 80), [200, 0, 0, 255]);
		assert_eq!(map.pixel(0, 78), [0, 200, 0, 255]);
		// a line two pixels tall keeps its ink in the first of them, so a page of
		// them reads as lines rather than one block
		assert!(map.pixel(0, 81)[3] < 180, "{:?}", map.pixel(0, 81));
		assert_eq!(map.pixel(0, 81)[0..3], [200, 0, 0]);
		// past the ink the row is clear, so the wallpaper shows through
		assert_eq!(map.pixel(6, 80)[3], 0);
	}

	#[test]
	fn a_short_buffer_leaves_the_bottom_of_the_column_empty() {
		let width = 4;
		let rows: Vec<Row> = (0..50).map(|_| row(width, 4, [0, 200, 0])).collect();
		let mut map = Minimap::default();
		map.seed(width, rows);
		map.compose(400, 1.0);
		// 50 lines capped at 2px each fill 100px of 400
		assert_eq!(map.pixel(0, 98)[3], 255);
		assert!(map.pixel(0, 99)[3] > 0);
		assert_eq!(map.pixel(0, 120)[3], 0);
	}

	#[test]
	fn one_inked_line_among_many_still_shows() {
		// 5,000 lines into 500px: 10 lines to a pixel row, and the single red one
		// must keep its color and stay findable, dimmer than a solid page
		let width = 4;
		let mut rows: Vec<Row> = (0..5000).map(|_| vec![0u8; width * 4]).collect();
		rows[2500] = row(width, 4, [200, 0, 0]);
		let mut map = Minimap::default();
		map.seed(width, rows);
		map.compose(500, 1.0);
		let lone = map.pixel(0, 250);
		assert_eq!(lone[0..3], [200, 0, 0]);
		assert!(lone[3] > 80 && lone[3] < 200, "{lone:?}");
	}

	// What the map is for is reading density from a distance, so a stretch of
	// mostly blank lines has to look different from a solid page.
	#[test]
	fn a_sparse_stretch_reads_dimmer_than_a_full_one() {
		let width = 4;
		let solid: Vec<Row> = (0..5000).map(|_| row(width, 4, [0, 200, 0])).collect();
		let mut sparse: Vec<Row> = (0..5000).map(|_| vec![0u8; width * 4]).collect();
		for i in (0..5000).step_by(4) {
			sparse[i] = row(width, 4, [0, 200, 0]);
		}
		let alpha = |rows: Vec<Row>| {
			let mut map = Minimap::default();
			map.seed(width, rows);
			map.compose(500, 1.0);
			map.pixel(0, 250)[3]
		};
		let (full, thin) = (alpha(solid), alpha(sparse));
		assert_eq!(full, 255);
		assert!(thin < full - 40, "solid {full}, sparse {thin}");
		assert!(thin > 0);
	}

	// Below a pixel a line has no room for a gap, so it is taken whole and a
	// full page is as bright as it ever was.
	#[test]
	fn a_line_under_a_pixel_keeps_its_whole_height() {
		assert_eq!(Minimap::band(0.4), (0.0, 0.4));
		assert_eq!(Minimap::band(1.0), (0.0, 1.0));
		let (top, h) = Minimap::band(2.0);
		assert!(top > 0.0 && h < 2.0 * 0.75);
	}
}
