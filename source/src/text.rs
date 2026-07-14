// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

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
	let name = resolve_mono_family(fs).map(|family| &*Box::leak(family.into_boxed_str()));
	*MONO_FAMILY.write().unwrap() = name;
}

pub fn mono_attrs() -> Attrs<'static> {
	let mut attrs = Attrs::new();
	attrs.family = match mono_family() {
		Some(name) => Family::Name(name),
		None => Family::Monospace,
	};
	attrs
}

// Concrete family + style behind chrome's `ui_attrs`, pinned alongside the mono
// family. Chrome follows the DESKTOP interface font - family, weight and slant -
// serif or not (for example "GentiumAlt Bold"); a sans is only the fallback
// when no desktop setting is readable.
//
// The weights are pinned as exact face weights, not the desktop's nominal ones:
// cosmic-text only uses the requested family when a face matches the requested
// weight EXACTLY (its fallback filters font_weight_diff == 0), so asking for
// Bold in a family that ships no bold face silently swaps in a bold fallback
// sans - the family must win over the weight. It also compares family names
// case-SENSITIVELY, so the db's own spelling is what gets pinned.
static UI_FAMILY: RwLock<Option<&'static str>> = RwLock::new(None);
static UI_WEIGHT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(400);
static UI_WEIGHT_BOLD: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(700);
static UI_ITALIC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn ui_family() -> Option<&'static str> {
	*UI_FAMILY.read().unwrap()
}

// Nearest face the family actually has to (weight, slant); None when the family
// has no faces at all (shouldn't happen for a db-validated name).
fn nearest_face(
	db: &fontdb::Database,
	fam: &str,
	want_weight: u16,
	want_italic: bool,
) -> Option<(u16, bool)> {
	let mut best: Option<(u16, bool)> = None;
	for face in db.faces() {
		if !face.families.iter().any(|(name, _)| name == fam) {
			continue;
		}
		let is_italic = face.style == fontdb::Style::Italic;
		let candidate = (
			is_italic != want_italic,
			face.weight.0.abs_diff(want_weight),
			face.weight.0,
		);
		let beats = best.is_none_or(|(best_weight, best_italic)| {
			candidate
				< (
					best_italic != want_italic,
					best_weight.abs_diff(want_weight),
					best_weight,
				)
		});
		if beats {
			best = Some((face.weight.0, is_italic));
		}
	}
	best
}

fn pin_ui_family(fs: &FontSystem) {
	use std::sync::atomic::Ordering;
	let sys_font = crate::sysfont::interface();
	let name = resolve_ui_family(fs).map(|family| &*Box::leak(family.into_boxed_str()));
	*UI_FAMILY.write().unwrap() = name;
	// honour the desktop's weight/slant only when its family actually resolved
	// (a fallback sans shouldn't inherit "Bold" meant for another face)
	let using_sys = match (name, &sys_font.family) {
		(Some(resolved), Some(family)) => resolved.eq_ignore_ascii_case(family),
		_ => false,
	};
	let want_weight: u16 = if using_sys && sys_font.bold { 700 } else { 400 };
	let want_italic = using_sys && sys_font.italic;
	let (body_weight, body_italic, title_weight) = match name {
		Some(family_name) => {
			let db = fs.db();
			let (weight, italic) =
				nearest_face(db, family_name, want_weight, want_italic).unwrap_or((400, false));
			// emphasis (dialog titles/headers): the family's boldest-available
			// take on 700, again snapped so it can't eject the family
			let title_weight = nearest_face(db, family_name, 700, italic)
				.map_or(weight, |(nearest_weight, _)| nearest_weight);
			(weight, italic && want_italic, title_weight)
		}
		None => (400, false, 700),
	};
	UI_WEIGHT.store(body_weight, Ordering::Relaxed);
	UI_WEIGHT_BOLD.store(title_weight, Ordering::Relaxed);
	UI_ITALIC.store(body_italic, Ordering::Relaxed);
}

// Chrome ascent/descent scaled to `ui_px`, read the SAME way cosmic-text does
// (`ui_px * ascent/units_per_em`), so `ui_text_top` predicts the real baseline.
// A proportional fallback if the pinned face can't be read.
fn ui_vmetrics(fs: &mut FontSystem, ui_px: f32) -> (f32, f32) {
	use std::sync::atomic::Ordering;
	let want_weight = fontdb::Weight(UI_WEIGHT.load(Ordering::Relaxed));
	let id = {
		let query = fontdb::Query {
			families: &[ui_family().map_or(fontdb::Family::SansSerif, fontdb::Family::Name)],
			weight: want_weight,
			..Default::default()
		};
		fs.db().query(&query)
	};
	let fallback = (ui_px * 0.8, ui_px * 0.2);
	let Some(font) = id.and_then(|id| fs.get_font(id, want_weight)) else {
		return fallback;
	};
	let metrics = font.as_swash().metrics(&[]);
	let scale = ui_px / f32::from(metrics.units_per_em).max(1.0);
	(metrics.ascent * scale, metrics.descent * scale)
}

// Baseline y within a single-line UI buffer of height `ui_line_h`: cosmic-text
// centers the ascent+descent box in the line, so the baseline sits at the line
// center shifted by (ascent-descent)/2. `vmetrics` is (ascent, descent, cap).
fn ui_baseline_in_buf(ui_line_h: f32, vmetrics: (f32, f32)) -> f32 {
	let (ascent, descent) = vmetrics;
	ui_line_h / 2.0 + (ascent - descent) / 2.0
}

// Buffer `top` that centers chrome text's visible box in a bar
// `[bar_top, bar_top+bar_h]`. Chrome titles (File/Edit/tab names) have no
// descenders but do have ascenders (l/d/h/i), so their visible extent is
// ascender-top..baseline; centering THAT (not cap..baseline) is what actually
// looks balanced - cap-centering leaves the empty descent reading as space
// below. The rare descender then just dips into the natural descent room.
fn ui_visible_center_top(ui_line_h: f32, vmetrics: (f32, f32), bar_top: f32, bar_h: f32) -> f32 {
	let (ascent, _) = vmetrics;
	bar_top + bar_h / 2.0 - ui_baseline_in_buf(ui_line_h, vmetrics) + ascent / 2.0
}

// Emphasis weight for chrome (dialog titles, section headers): the closest
// weight to Bold the pinned family really ships. Use this instead of a literal
// `Weight::BOLD`, which kicks the whole family out when no 700 face exists.
pub fn ui_bold_weight() -> glyphon::Weight {
	use std::sync::atomic::Ordering;
	glyphon::Weight(UI_WEIGHT_BOLD.load(Ordering::Relaxed))
}

// Proportional attrs for chrome - menus, the menu bar, dialogs - in the pinned
// desktop interface font (family/weight/slant), so chrome reads like the rest
// of the user's desktop rather than terminal text.
pub fn ui_attrs() -> Attrs<'static> {
	use std::sync::atomic::Ordering;
	let mut attrs = Attrs::new();
	attrs.family = match ui_family() {
		Some(name) => Family::Name(name),
		None => Family::SansSerif,
	};
	attrs.weight = glyphon::Weight(UI_WEIGHT.load(Ordering::Relaxed));
	if UI_ITALIC.load(Ordering::Relaxed) {
		attrs.style = glyphon::Style::Italic;
	}
	attrs
}

// Resolve a concrete family for chrome: the desktop interface font first (the
// whole point - serif or not), else the OS sans-serif, else a known-good sans.
// Everything is validated against the db so a bad name can't slip through -
// generic `Family::SansSerif` can't be trusted (fontdb defaults it to "Arial"
// and falls through to whatever matches when that's absent).
fn resolve_ui_family(fs: &FontSystem) -> Option<String> {
	let db = fs.db();
	// returns the db's canonical spelling of the family (cosmic-text's fallback
	// compares face family names case-sensitively - the candidate string won't do)
	let installed = |fam: &str| {
		let query = fontdb::Query {
			families: &[fontdb::Family::Name(fam)],
			..Default::default()
		};
		db.query(&query)
			.and_then(|id| db.face(id))
			.and_then(|face| {
				face.families
					.iter()
					.find(|(name, _)| name.eq_ignore_ascii_case(fam))
					.map(|(name, _)| name.clone())
			})
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
	crate::sysfont::interface()
		.family
		.clone()
		.into_iter()
		.chain(crate::sysfont::sans_serif().map(str::to_string))
		.chain(curated.iter().map(|s| s.to_string()))
		.find_map(|fam| installed(&fam))
}

// Resolve the monospace family to pin for every weight: the user's configured
// `font_family` if installed, else the OS monospace family, else whatever
// `Family::Monospace` maps to. Validated against the db so a bad name doesn't
// silently fall back to an unrelated font.
fn resolve_mono_family(fs: &FontSystem) -> Option<String> {
	use glyphon::cosmic_text::fontdb;
	let db = fs.db();

	let installed = |fam: &str| {
		let query = fontdb::Query {
			families: &[fontdb::Family::Name(fam)],
			..Default::default()
		};
		db.query(&query)
			.and_then(|id| db.face(id))
			.is_some_and(|face| {
				face.families
					.iter()
					.any(|(name, _)| name.eq_ignore_ascii_case(fam))
			})
	};

	let settings = config::settings();
	let sys_family = crate::sysfont::monospace().family.clone();
	// Priority: the OS monospace first when following the system font; otherwise
	// each family in the user's comma-separated fallback stack, then the OS mono.
	let mut candidates: Vec<String> = Vec::new();
	if settings.use_system_font {
		candidates.extend(sys_family.clone());
	} else {
		candidates.extend(
			settings
				.font_family
				.iter()
				.flat_map(|list| list.split(','))
				.map(|name| name.trim().to_string())
				.filter(|name| !name.is_empty()),
		);
		candidates.extend(sys_family.clone());
	}
	for fam in candidates {
		if installed(&fam) {
			return Some(fam);
		}
	}

	let query = fontdb::Query {
		families: &[fontdb::Family::Monospace],
		..Default::default()
	};
	db.query(&query)
		.and_then(|id| db.face(id))?
		.families
		.first()
		.map(|(name, _)| name.clone())
}

pub struct TextCtx {
	pub font_system: FontSystem,
	pub swash_cache: SwashCache,
	pub atlas: TextAtlas,
	// The scrim renders into an Rgba16Float coverage texture, a different format
	// than the surface, so its glyphon renderer needs its own same-format atlas.
	scrim_atlas: TextAtlas,
	pub viewport: Viewport,
	pub renderer: TextRenderer,
	// separate renderer for the context-menu overlay (second pass, on top)
	pub overlay: TextRenderer,
	// separate renderer for the scrim source pass: pane text only (no chrome), and
	// panes may substitute a de-bolded buffer (text_scrim_regular_weight)
	pub scrim: TextRenderer,
	pub cell_w: f32,
	pub cell_h: f32,
	// physical-px inset between content and pane edge
	pub margin: f32,
	pub metrics: Metrics,
	// Chrome (menus/tabs/dialogs) renders at the DESKTOP interface font size,
	// independent of the terminal font size; bars and rows size from this.
	pub ui_line_h: f32,
	ui_metrics: Metrics,
	// chrome vertical metrics at ui_px (ascent, descent) in the units cosmic-text
	// lays the line out in, so a bar can center chrome text on its real visible
	// box (see `ui_text_top`).
	ui_vmetrics: (f32, f32),
	// primary monospace face + a coverage cache, so the pane can tell which
	// glyphs fall back to another font (those drift from the cell grid and get
	// rendered per-cell instead - see Pane::build).
	mono_face: Option<fontdb::ID>,
	cover_cache: HashMap<char, bool>,
	// Measured chrome-text widths. Keyed by text only: every chrome measurement
	// uses the base UI attrs (colour varies, which doesn't affect width), and the
	// font is fixed for this TextCtx's life. Measuring shapes a throwaway buffer,
	// and the menu bar re-measures its titles every rendered frame - the memo
	// turns that into a lookup. Bounded (cleared) so dynamic tab titles can't
	// grow it without limit.
	ui_measure_cache: HashMap<String, f32>,
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
		pin_ui_family(&font_system);

		let font_size = (config::effective_font_size() * scale).round();
		let line_height = (font_size * config::settings().line_height_scale).round();
		let metrics = Metrics::new(font_size, line_height);

		let cell_w = measure_cell(&mut font_system, metrics);
		let cell_h = line_height.max(1.0);

		// Chrome follows the desktop UI font size (pt -> px at the 96-DPI
		// reference, like the mono path); terminal size is the fallback so the
		// old chrome look is kept where no desktop setting is readable.
		let ui_px = crate::sysfont::interface()
			.size_pt
			.map(|pt| pt * 96.0 / 72.0)
			.filter(|px| *px >= 4.0)
			.unwrap_or_else(config::effective_font_size);
		let ui_px = (ui_px * scale).round().max(8.0);
		let ui_line_h = (ui_px * 1.35).round(); // roomy UI leading; descenders must clear buttons
		let ui_metrics = Metrics::new(ui_px, ui_line_h);
		let ui_vmetrics = ui_vmetrics(&mut font_system, ui_px);

		let mono_face = {
			let fam = mono_family();
			let query = fontdb::Query {
				families: &[fam.map_or(fontdb::Family::Monospace, fontdb::Family::Name)],
				..Default::default()
			};
			font_system.db().query(&query)
		};

		let cache = Cache::new(device);
		let mut atlas = TextAtlas::new(device, queue, &cache, format);
		// The scrim text pass renders into a separate Rgba16Float coverage texture
		// (crate::scrim::FMT), so its glyphon renderer must target THAT format. On the
		// X11 GL path gfx.format is already Rgba16Float and a shared atlas happened to
		// match; on the native path (Windows, Wayland) gfx.format is an sRGB surface
		// format, so a shared atlas targets the wrong format - wgpu rejects it as
		// "incompatible color attachments" on the first scrim frame. One Cache backs
		// both atlases (it's built to serve multiple target formats).
		let mut scrim_atlas = TextAtlas::new(device, queue, &cache, crate::scrim::FMT);
		let viewport = Viewport::new(device, &cache);
		let renderer =
			TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
		let overlay =
			TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
		let scrim =
			TextRenderer::new(&mut scrim_atlas, device, wgpu::MultisampleState::default(), None);

		Self {
			font_system,
			swash_cache: SwashCache::new(),
			atlas,
			scrim_atlas,
			viewport,
			renderer,
			overlay,
			scrim,
			cell_w,
			cell_h,
			margin: (config::settings().margin * scale).round(),
			metrics,
			ui_line_h,
			ui_metrics,
			ui_vmetrics,
			mono_face,
			cover_cache: HashMap::new(),
			ui_measure_cache: HashMap::new(),
		}
	}

	// Does the primary monospace face have a glyph for `ch`? ASCII is always
	// assumed covered. Cached because it's hit per visible cell per frame.
	pub fn covered(&mut self, ch: char) -> bool {
		if ch.is_ascii() {
			return true;
		}
		if let Some(&cached) = self.cover_cache.get(&ch) {
			return cached;
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
	pub fn fill_glyph(&mut self, buf: &mut Buffer, ch: char, attrs: &Attrs) -> (f32, f32) {
		let mut utf8_buf = [0u8; 4];
		buf.set_text(
			&mut self.font_system,
			ch.encode_utf8(&mut utf8_buf),
			attrs,
			Shaping::Advanced,
			None,
		);
		buf.shape_until_scroll(&mut self.font_system, false);
		let phys = buf
			.layout_runs()
			.next()
			.and_then(|run| run.glyphs.first())
			.map(|glyph| glyph.physical((0.0, 0.0), 1.0));
		let Some(phys) = phys else {
			return (self.cell_w, 0.0);
		};
		match self
			.swash_cache
			.get_image(&mut self.font_system, phys.cache_key)
		{
			Some(image) => (
				image.placement.width.max(1) as f32,
				phys.x as f32 + image.placement.left as f32,
			),
			None => (self.cell_w, 0.0),
		}
	}

	// `top` for a chrome text buffer so its VISIBLE box (cap-top to baseline)
	// centers in a bar of height `bar_h` at `bar_top`. Uses the real font
	// metrics, so it stays centered across font/size changes - unlike the old
	// hand-tuned per-bar padding, which left menu titles (no descenders) riding
	// high with empty descent space below.
	pub fn ui_text_top(&self, bar_top: f32, bar_h: f32) -> f32 {
		ui_visible_center_top(self.ui_line_h, self.ui_vmetrics, bar_top, bar_h)
	}

	// `top` that centers the full ascent..descent ink box instead. Right for
	// lowercase labels with descenders ("select"/"output"), which read
	// bottom-heavy under the ascent..baseline centering above. cosmic-text
	// centers that box in the line, so this is just centering the buffer.
	pub fn ui_text_top_ink(&self, bar_top: f32, bar_h: f32) -> f32 {
		bar_top + (bar_h - self.ui_line_h) / 2.0
	}

	// Screen-space baseline of chrome text placed with `ui_text_top` - for the
	// Alt-accelerator underline.
	pub fn ui_baseline(&self, bar_top: f32, bar_h: f32) -> f32 {
		self.ui_text_top(bar_top, bar_h) + ui_baseline_in_buf(self.ui_line_h, self.ui_vmetrics)
	}

	// Width in px of chrome `text` shaped with `attrs` at the UI font size.
	// Sizes menus, bar titles, dialog labels to the real rendered text.
	// Memoized by text (see ui_measure_cache).
	pub fn measure_ui_text(&mut self, text: &str, attrs: &Attrs) -> f32 {
		if let Some(&w) = self.ui_measure_cache.get(text) {
			return w;
		}
		let w = self.measure_at(text, attrs, self.ui_metrics);
		if self.ui_measure_cache.len() >= 512 {
			self.ui_measure_cache.clear();
		}
		self.ui_measure_cache.insert(text.to_string(), w);
		w
	}

	fn measure_at(&mut self, text: &str, attrs: &Attrs, metrics: Metrics) -> f32 {
		let mut buf = Buffer::new(&mut self.font_system, metrics);
		buf.set_wrap(&mut self.font_system, Wrap::None);
		buf.set_size(&mut self.font_system, None, None);
		buf.set_text(&mut self.font_system, text, attrs, Shaping::Advanced, None);
		buf.shape_until_scroll(&mut self.font_system, false);
		buf.layout_runs().next().map_or(0.0, |run| run.line_w)
	}

	// Chrome buffer: UI-font metrics, natural (proportional) advances - no
	// cell-grid snap, chrome has no grid.
	pub fn new_ui_buffer(&mut self, w_px: f32, h_px: f32) -> Buffer {
		let mut buf = Buffer::new(&mut self.font_system, self.ui_metrics);
		buf.set_wrap(&mut self.font_system, Wrap::None);
		buf.set_size(
			&mut self.font_system,
			Some(w_px.max(1.0)),
			Some(h_px.max(1.0)),
		);
		buf
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

	pub fn prepare_scrim(
		&mut self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		areas: Vec<TextArea<'_>>,
	) -> Result<(), glyphon::PrepareError> {
		self.scrim.prepare(
			device,
			queue,
			&mut self.font_system,
			&mut self.scrim_atlas,
			&self.viewport,
			areas,
			&mut self.swash_cache,
		)
	}

	pub fn render_scrim(
		&self,
		pass: &mut wgpu::RenderPass<'_>,
	) -> Result<(), glyphon::RenderError> {
		self.scrim.render(&self.scrim_atlas, &self.viewport, pass)
	}

	pub fn trim_atlas(&mut self) {
		self.atlas.trim();
		self.scrim_atlas.trim();
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
		.map(|run| run.line_w / N as f32)
		.unwrap_or(metrics.font_size * 0.6)
		.max(1.0)
}

#[cfg(test)]
mod tests {
	use super::*;

	// Chrome must pin a concrete face, never fall back to generic
	// `Family::SansSerif` (which lands on a serif when fontdb's "Arial" default
	// is absent). Only needs a FontSystem (no GPU), so it runs with no display.
	// A chrome line placed by ui_visible_center_top must sit with its visible
	// (ascender-top..baseline) box centered in the bar, for any bar height and
	// metrics - so a font/size change stays balanced without hand-tuned padding.
	#[test]
	fn chrome_text_visible_box_centers_in_bar() {
		let vmetrics = (13.6f32, 3.4f32); // ascent, descent px
		let ui_line_h = 23.0;
		for &(bar_top, bar_h) in &[(0.0f32, 29.0f32), (29.0, 29.0), (10.0, 40.0)] {
			let top = ui_visible_center_top(ui_line_h, vmetrics, bar_top, bar_h);
			let baseline = top + ui_baseline_in_buf(ui_line_h, vmetrics);
			let visible_center = baseline - vmetrics.0 / 2.0; // midpoint of ascender..baseline
			assert!(
				(visible_center - (bar_top + bar_h / 2.0)).abs() < 0.01,
				"visible center {visible_center} != bar center {}",
				bar_top + bar_h / 2.0
			);
		}
	}

	#[test]
	fn ui_font_resolves_to_concrete_family() {
		let fs = FontSystem::new();
		let fam = resolve_ui_family(&fs);
		eprintln!("resolved chrome UI family: {fam:?}");
		assert!(fam.is_some(), "no concrete UI family resolved for chrome");
	}

	#[test]
	fn ui_attrs_shape_in_pinned_family() {
		let mut fs = FontSystem::new();
		pin_ui_family(&fs);
		let attrs = ui_attrs();
		let Family::Name(want) = attrs.family else {
			eprintln!("no pinned family on this box; skipping");
			return;
		};
		let mut b = Buffer::new(&mut fs, Metrics::new(17.0, 22.0));
		b.set_size(&mut fs, Some(400.0), Some(30.0));
		b.set_text(&mut fs, "File Edit", &attrs, Shaping::Advanced, None);
		b.shape_until_scroll(&mut fs, false);
		for run in b.layout_runs() {
			for g in run.glyphs {
				let fams: Vec<String> = fs
					.db()
					.face(g.font_id)
					.map(|f| f.families.iter().map(|(n, _)| n.clone()).collect())
					.unwrap_or_default();
				eprintln!("glyph font: {fams:?} weight_req={:?}", attrs.weight);
				assert!(
					fams.iter().any(|n| n == want),
					"chrome glyph shaped in {fams:?}, not the pinned family {want:?}"
				);
			}
		}
	}
}
