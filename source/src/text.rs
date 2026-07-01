// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

use std::collections::HashMap;
use std::sync::RwLock;

use glyphon::cosmic_text::fontdb;
use glyphon::{
	Attrs, Buffer, Cache, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea,
	TextAtlas, TextRenderer, Viewport, Wrap,
};

use crate::config;

// Concrete family name behind `Family::Monospace`, re-resolved on each TextCtx
// build (so the Settings font field / "Use system font" apply live). cosmic-text
// picks the best face *per query*, so a BOLD run can land in a different family
// than the regular run; pinning one name keeps every weight in it. `Attrs` needs
// a 'static name, so the resolved string is leaked (rare - only on a font change).
static MONO_FAMILY: RwLock<Option<&'static str>> = RwLock::new(None);

fn mono_family() -> Option<&'static str> {
	*MONO_FAMILY.read().unwrap()
}

// Re-resolve and pin the monospace family for the current config + font system.
fn pin_mono_family(fs: &FontSystem) {
	let name = resolve_mono_family(fs).map(|s| &*Box::leak(s.into_boxed_str()));
	*MONO_FAMILY.write().unwrap() = name;
}

pub fn mono_attrs() -> Attrs<'static> {
	let mut a = Attrs::new();
	a.family = match mono_family() {
		Some(name) => Family::Name(name),
		None => Family::Monospace,
	};
	a
}

// Concrete family behind chrome's `sans_attrs`, pinned alongside the mono family.
static SANS_FAMILY: RwLock<Option<&'static str>> = RwLock::new(None);

fn sans_family() -> Option<&'static str> {
	*SANS_FAMILY.read().unwrap()
}

fn pin_sans_family(fs: &FontSystem) {
	let name = resolve_sans_family(fs).map(|s| &*Box::leak(s.into_boxed_str()));
	*SANS_FAMILY.write().unwrap() = name;
}

// Proportional (sans-serif) attrs for chrome - menus, the menu bar, dialogs -
// so they read like native UI rather than terminal text. Uses a pinned concrete
// family (resolve_sans_family); generic `Family::SansSerif` is unreliable here.
pub fn sans_attrs() -> Attrs<'static> {
	let mut a = Attrs::new();
	a.family = match sans_family() {
		Some(name) => Family::Name(name),
		None => Family::SansSerif,
	};
	a
}

// Resolve a concrete sans-serif family for chrome. `Family::SansSerif` can't be
// trusted: fontdb's generic sans defaults to "Arial", and when that isn't
// installed the query falls through to whatever matches - often a serif (e.g. a
// GNOME serif document font). So pin the OS sans-serif if installed, else a
// known-good sans, validated against the db so a bad name can't slip through.
fn resolve_sans_family(fs: &FontSystem) -> Option<String> {
	let db = fs.db();
	let installed = |fam: &str| {
		let q = fontdb::Query {
			families: &[fontdb::Family::Name(fam)],
			..Default::default()
		};
		db.query(&q)
			.and_then(|id| db.face(id))
			.is_some_and(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(fam)))
	};
	let curated = [
		"DejaVu Sans",
		"Noto Sans",
		"Liberation Sans",
		"Cantarell",
		"Ubuntu",
		"Segoe UI",
		"Helvetica Neue",
		"Arial",
	];
	crate::sysfont::sans_serif()
		.map(str::to_string)
		.into_iter()
		.chain(curated.iter().map(|s| s.to_string()))
		.find(|fam| installed(fam))
}

// Resolve the monospace family to pin for every weight: the user's configured
// `font_family` if installed, else the OS monospace family, else whatever
// `Family::Monospace` maps to. Validated against the db so a bad name doesn't
// silently fall back to an unrelated font.
fn resolve_mono_family(fs: &FontSystem) -> Option<String> {
	use glyphon::cosmic_text::fontdb;
	let db = fs.db();

	let installed = |fam: &str| {
		let q = fontdb::Query {
			families: &[fontdb::Family::Name(fam)],
			..Default::default()
		};
		db.query(&q)
			.and_then(|id| db.face(id))
			.is_some_and(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(fam)))
	};

	let s = config::settings();
	let sys = crate::sysfont::monospace().family.clone();
	// Priority: the OS monospace first when following the system font; otherwise
	// each family in the user's comma-separated fallback stack, then the OS mono.
	let mut candidates: Vec<String> = Vec::new();
	if s.use_system_font {
		candidates.extend(sys.clone());
	} else {
		candidates.extend(
			s.font_family
				.iter()
				.flat_map(|f| f.split(','))
				.map(|f| f.trim().to_string())
				.filter(|f| !f.is_empty()),
		);
		candidates.extend(sys.clone());
	}
	for fam in candidates {
		if installed(&fam) {
			return Some(fam);
		}
	}

	let q = fontdb::Query {
		families: &[fontdb::Family::Monospace],
		..Default::default()
	};
	db.query(&q)
		.and_then(|id| db.face(id))?
		.families
		.first()
		.map(|(name, _)| name.clone())
}

pub struct TextCtx {
	pub font_system: FontSystem,
	pub swash_cache: SwashCache,
	pub atlas: TextAtlas,
	pub viewport: Viewport,
	pub renderer: TextRenderer,
	// separate renderer for the context-menu overlay (second pass, on top)
	pub overlay: TextRenderer,
	pub cell_w: f32,
	pub cell_h: f32,
	// physical-px inset between content and pane edge
	pub margin: f32,
	pub metrics: Metrics,
	// primary monospace face + a coverage cache, so the pane can tell which
	// glyphs fall back to another font (those drift from the cell grid and get
	// rendered per-cell instead - see Pane::build).
	mono_face: Option<fontdb::ID>,
	cover_cache: HashMap<char, bool>,
}

impl TextCtx {
	pub fn new(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		format: wgpu::TextureFormat,
		scale: f32,
	) -> Self {
		let mut font_system = FontSystem::new();
		pin_mono_family(&font_system);
		pin_sans_family(&font_system);

		let font_size = (config::effective_font_size() * scale).round();
		let line_height = (font_size * config::settings().line_height_scale).round();
		let metrics = Metrics::new(font_size, line_height);

		let cell_w = measure_cell(&mut font_system, metrics);
		let cell_h = line_height.max(1.0);

		let mono_face = {
			let fam = mono_family();
			let q = fontdb::Query {
				families: &[fam.map_or(fontdb::Family::Monospace, fontdb::Family::Name)],
				..Default::default()
			};
			font_system.db().query(&q)
		};

		let cache = Cache::new(device);
		let mut atlas = TextAtlas::new(device, queue, &cache, format);
		let viewport = Viewport::new(device, &cache);
		let renderer =
			TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
		let overlay =
			TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);

		Self {
			font_system,
			swash_cache: SwashCache::new(),
			atlas,
			viewport,
			renderer,
			overlay,
			cell_w,
			cell_h,
			margin: (config::settings().margin * scale).round(),
			metrics,
			mono_face,
			cover_cache: HashMap::new(),
		}
	}

	// Does the primary monospace face have a glyph for `ch`? ASCII is always
	// assumed covered. Cached because it's hit per visible cell per frame.
	pub fn covered(&mut self, ch: char) -> bool {
		if ch.is_ascii() {
			return true;
		}
		if let Some(&b) = self.cover_cache.get(&ch) {
			return b;
		}
		let covered = self
			.mono_face
			.and_then(|id| self.font_system.get_font(id, fontdb::Weight::NORMAL))
			.is_some_and(|font| font.as_swash().charmap().map(ch) != 0);
		self.cover_cache.insert(ch, covered);
		covered
	}

	// Buffer for a single fallback glyph: no monospace snapping (render at its
	// natural width), positioned per-cell by the caller.
	pub fn new_plain_buffer(&mut self) -> Buffer {
		let mut buf = Buffer::new(&mut self.font_system, self.metrics);
		buf.set_wrap(&mut self.font_system, Wrap::None);
		buf.set_size(
			&mut self.font_system,
			Some(self.cell_w * 2.5),
			Some(self.cell_h),
		);
		buf
	}

	// Shape one fallback glyph into `buf` and return its *ink* box (rasterized,
	// at scale 1): `(width_px, left_px)` where `left_px` is the ink's x offset
	// from the text-area origin. The caller fits this to the cell box - using
	// the ink box, not the advance, because these fallback symbols routinely
	// paint wider than they advance and would otherwise overlap the next cell.
	pub fn fill_glyph(&mut self, buf: &mut Buffer, ch: char, a: &Attrs) -> (f32, f32) {
		let mut s = [0u8; 4];
		buf.set_text(
			&mut self.font_system,
			ch.encode_utf8(&mut s),
			a,
			Shaping::Advanced,
			None,
		);
		buf.shape_until_scroll(&mut self.font_system, false);
		let phys = buf
			.layout_runs()
			.next()
			.and_then(|r| r.glyphs.first())
			.map(|g| g.physical((0.0, 0.0), 1.0));
		let Some(phys) = phys else {
			return (self.cell_w, 0.0);
		};
		match self
			.swash_cache
			.get_image(&mut self.font_system, phys.cache_key)
		{
			Some(im) => (
				im.placement.width.max(1) as f32,
				phys.x as f32 + im.placement.left as f32,
			),
			None => (self.cell_w, 0.0),
		}
	}

	// Width in px of `text` shaped with `attrs` (proportional or mono). Used to
	// size chrome (menu widths, menu-bar title hit-boxes) to the real text.
	pub fn measure_text(&mut self, text: &str, attrs: &Attrs) -> f32 {
		let mut buf = Buffer::new(&mut self.font_system, self.metrics);
		buf.set_wrap(&mut self.font_system, Wrap::None);
		buf.set_size(&mut self.font_system, None, None);
		buf.set_text(&mut self.font_system, text, attrs, Shaping::Advanced, None);
		buf.shape_until_scroll(&mut self.font_system, false);
		buf.layout_runs().next().map_or(0.0, |r| r.line_w)
	}

	pub fn new_buffer(&mut self, w_px: f32, h_px: f32) -> Buffer {
		let mut buf = Buffer::new(&mut self.font_system, self.metrics);
		buf.set_wrap(&mut self.font_system, Wrap::None);
		// Snap every glyph to exactly one cell wide so the text lines up with
		// the cell grid (cursor / background quads at col*cell_w). Without this
		// glyphon lays out at the font's natural advance and the text drifts
		// from the grid across a line.
		buf.set_monospace_width(&mut self.font_system, Some(self.cell_w));
		buf.set_size(
			&mut self.font_system,
			Some(w_px.max(1.0)),
			Some(h_px.max(1.0)),
		);
		buf
	}

	pub fn resize_buffer(&mut self, buf: &mut Buffer, w_px: f32, h_px: f32) {
		buf.set_size(
			&mut self.font_system,
			Some(w_px.max(1.0)),
			Some(h_px.max(1.0)),
		);
	}

	pub fn update_viewport(&mut self, queue: &wgpu::Queue, w: u32, h: u32) {
		self.viewport.update(
			queue,
			Resolution {
				width: w,
				height: h,
			},
		);
	}

	pub fn prepare(
		&mut self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		areas: Vec<TextArea<'_>>,
	) -> Result<(), glyphon::PrepareError> {
		self.renderer.prepare(
			device,
			queue,
			&mut self.font_system,
			&mut self.atlas,
			&self.viewport,
			areas,
			&mut self.swash_cache,
		)
	}

	pub fn render(&self, pass: &mut wgpu::RenderPass<'_>) -> Result<(), glyphon::RenderError> {
		self.renderer.render(&self.atlas, &self.viewport, pass)
	}

	pub fn prepare_overlay(
		&mut self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		areas: Vec<TextArea<'_>>,
	) -> Result<(), glyphon::PrepareError> {
		self.overlay.prepare(
			device,
			queue,
			&mut self.font_system,
			&mut self.atlas,
			&self.viewport,
			areas,
			&mut self.swash_cache,
		)
	}

	pub fn render_overlay(
		&self,
		pass: &mut wgpu::RenderPass<'_>,
	) -> Result<(), glyphon::RenderError> {
		self.overlay.render(&self.atlas, &self.viewport, pass)
	}

	pub fn trim_atlas(&mut self) {
		self.atlas.trim();
	}
}

// Measure the per-cell advance the *render* buffer actually produces. We shape
// with `Shaping::Advanced` (what panes use) over a long run so per-glyph hinting
// rounding averages out. The result is intentionally NOT rounded: cosmic-text's
// `set_monospace_width` only snaps advances for fonts that report a monospace
// em-width, so for many system fonts the real pitch is the font's natural
// advance (~fractional). Rounding cell_w away from that pitch made the cursor,
// cell backgrounds, and per-cell fallback glyphs (all placed at col*cell_w)
// drift right of the text, worsening with column count. Matching the real pitch
// keeps the drift sub-pixel (bounded by hinting, not accumulating).
fn measure_cell(fs: &mut FontSystem, metrics: Metrics) -> f32 {
	const N: usize = 40;
	let mut buf = Buffer::new(fs, metrics);
	buf.set_size(fs, None, None);
	let attrs = mono_attrs();
	buf.set_text(fs, &"M".repeat(N), &attrs, Shaping::Advanced, None);
	buf.shape_until_scroll(fs, false);
	buf.layout_runs()
		.next()
		.map(|r| r.line_w / N as f32)
		.unwrap_or(metrics.font_size * 0.6)
		.max(1.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	// Chrome must pin a concrete sans face, never fall back to generic
	// `Family::SansSerif` (which lands on a serif when fontdb's "Arial" default
	// is absent). Only needs a FontSystem (no GPU), so it runs headless.
	#[test]
	fn sans_resolves_to_concrete_family() {
		let fs = FontSystem::new();
		let fam = resolve_sans_family(&fs);
		eprintln!("resolved chrome sans family: {fam:?}");
		assert!(fam.is_some(), "no concrete sans family resolved for chrome");
	}
}
