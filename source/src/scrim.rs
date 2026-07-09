// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

//! Text readability scrim: a blurred, background-coloured halo behind glyphs so
//! text stays legible over a light/busy background image or a near-transparent
//! terminal. The scene's text is rendered to a texture, blurred (2-pass separable
//! Gaussian), and composited UNDER the crisp text, coloured per-pixel by a
//! `bgcolor` map so a glyph's halo takes ITS cell's bg colour (a glyph on a
//! one-off colored cell isn't smeared with the global bg colour).
//!
//! tex_t <- crisp TEXT coverage; tex_cur <- crisp CURSOR coverage (kept apart so
//! the cursor can join the halo and the outline independently). H-blur folds in
//! tex_cur (when cursor_scrim) tex_t->tex_b, V-blur tex_b->tex_a; tex_a is the
//! blurred coverage. The composite multiplies it by `bgcolor`, and samples tex_t
//! (+ tex_cur when cursor_outline) to add a thin dilated outline around the crisp
//! coverage (the "border" - solid, same per-pixel colour as the scrim).

use crate::gfx::{RectInstance, RectRenderer};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurU {
	resolution: [f32; 2],
	dir: [f32; 2],
	sigma: f32,
	ramp: f32,   // falloff kernel: 0 = gaussian, 1 = linear, 2 = s-curve
	cursor: f32, // 1 = fold the cursor coverage into the halo, 0 = leave it out
	_pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CompU {
	resolution: [f32; 2],
	intensity: f32, // coverage boost; the colour comes from the bgcolor texture
	border_px: f32, // dilated outline radius around the crisp coverage (0 = none)
	cursor: f32,    // 1 = give the cursor an outline too, 0 = text only
	_pad: [f32; 3],
}

pub struct Scrim {
	tex_t: wgpu::Texture, // crisp text coverage (kept for the border pass)
	tex_a: wgpu::Texture,
	tex_b: wgpu::Texture,
	view_t: wgpu::TextureView,
	view_a: wgpu::TextureView,
	view_b: wgpu::TextureView,
	// crisp cursor coverage, separate from the text so cursor_scrim (halo) and
	// cursor_outline (border) are independent toggles - folded into the halo by
	// the blur and into the border by the composite, each gated by its own flag.
	tex_cur: wgpu::Texture,
	view_cur: wgpu::TextureView,
	sampler: wgpu::Sampler,
	blur_pipe: wgpu::RenderPipeline,
	blur_bgl: wgpu::BindGroupLayout,
	// one uniform PER direction: all queue.write_buffer calls are applied before
	// the command buffer runs, so a single shared buffer would give BOTH passes
	// the last-written dir (-> vertical blur twice, no horizontal). Two buffers fix it.
	blur_u_h: wgpu::Buffer,
	blur_u_v: wgpu::Buffer,
	blur_t2b: wgpu::BindGroup, // sample tex_t (uses blur_u_h), write tex_b
	blur_b2a: wgpu::BindGroup, // sample tex_b (uses blur_u_v), write tex_a
	comp_pipe: wgpu::RenderPipeline,
	comp_bgl: wgpu::BindGroupLayout,
	comp_u: wgpu::Buffer,
	comp_bind: wgpu::BindGroup, // sample tex_a (scrim alpha) + bgcolor (rgb) + tex_t (border)
	// per-pixel scrim colour: cleared to the global bg, with per-cell bg rects drawn
	// over it, so a glyph's halo takes ITS cell's bg colour (not always the global).
	bgcolor: wgpu::Texture,
	bgcolor_view: wgpu::TextureView,
	bg_rects: RectRenderer,
	// cursor quads drawn into tex_cur (its own coverage texture). Separate renderer:
	// bg_rects' instance buffer is uploaded for the bgcolor map in the SAME encoder,
	// and a second upload would clobber the first (queue writes all land before the
	// command buffer runs - same rule as the blur uniforms above).
	cursor_rects: RectRenderer,
	cursor_count: u32,
	w: u32,
	h: u32,
}

impl Scrim {
	pub fn new(device: &wgpu::Device, target: wgpu::TextureFormat, w: u32, h: u32) -> Self {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("scrim shader"),
			source: wgpu::ShaderSource::Wgsl(WGSL.into()),
		});
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some("scrim sampler"),
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			address_mode_u: wgpu::AddressMode::ClampToEdge,
			address_mode_v: wgpu::AddressMode::ClampToEdge,
			..Default::default()
		});
		// blur: uniform + sampled texture + sampler
		let blur_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("scrim blur bgl"),
			// binding 3 = the bgcolor map, whose alpha is an "own-bg" mask (see fs_blur);
			// binding 4 = the crisp cursor coverage (folded in when cursor_scrim)
			entries: &[
				ubuf_entry(0),
				tex_entry(1),
				samp_entry(2),
				tex_entry(3),
				tex_entry(4),
			],
		});
		let make_uniform_buf = |label| {
			device.create_buffer(&wgpu::BufferDescriptor {
				label: Some(label),
				size: std::mem::size_of::<BlurU>() as u64,
				usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
				mapped_at_creation: false,
			})
		};
		let blur_u_h = make_uniform_buf("scrim blur u h");
		let blur_u_v = make_uniform_buf("scrim blur u v");
		let blur_pipe = pipeline(device, &shader, "fs_blur", FMT, &blur_bgl, "scrim blur");

		let comp_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("scrim comp bgl"),
			entries: &[
				ubuf_entry(0),
				tex_entry(1),
				samp_entry(2),
				tex_entry(3),
				tex_entry(4),
				tex_entry(5),
			],
		});
		let comp_u = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("scrim comp u"),
			size: std::mem::size_of::<CompU>() as u64,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		let comp_pipe = pipeline_blend(device, &shader, "fs_comp", target, &comp_bgl, "scrim comp");

		let (tex_t, tex_a, tex_b, view_t, view_a, view_b) = make_textures(device, w, h);
		let (tex_cur, view_cur) = cover_tex(device, w, h);
		let bgcolor = bgcolor_tex(device, w, h);
		let bgcolor_view = bgcolor.create_view(&Default::default());
		let bg_rects = RectRenderer::new(device, FMT);
		let cursor_rects = RectRenderer::new(device, FMT);
		let (blur_t2b, blur_b2a, comp_bind) = binds(
			device,
			&blur_bgl,
			&comp_bgl,
			&blur_u_h,
			&blur_u_v,
			&comp_u,
			&sampler,
			&view_t,
			&view_a,
			&view_b,
			&bgcolor_view,
			&view_cur,
		);

		Self {
			tex_t,
			tex_a,
			tex_b,
			view_t,
			view_a,
			view_b,
			tex_cur,
			view_cur,
			sampler,
			blur_pipe,
			blur_bgl,
			blur_u_h,
			blur_u_v,
			blur_t2b,
			blur_b2a,
			comp_pipe,
			comp_bgl,
			comp_u,
			comp_bind,
			bgcolor,
			bgcolor_view,
			bg_rects,
			cursor_rects,
			cursor_count: 0,
			w,
			h,
		}
	}

	pub fn resize(&mut self, device: &wgpu::Device, w: u32, h: u32) {
		if w == 0 || h == 0 || (w == self.w && h == self.h) {
			return;
		}
		let (tex_t, tex_a, tex_b, view_t, view_a, view_b) = make_textures(device, w, h);
		self.tex_t = tex_t;
		self.tex_a = tex_a;
		self.tex_b = tex_b;
		self.view_t = view_t;
		self.view_a = view_a;
		self.view_b = view_b;
		let (tex_cur, view_cur) = cover_tex(device, w, h);
		self.tex_cur = tex_cur;
		self.view_cur = view_cur;
		self.bgcolor = bgcolor_tex(device, w, h);
		self.bgcolor_view = self.bgcolor.create_view(&Default::default());
		let (blur_t2b, blur_b2a, comp_bind) = binds(
			device,
			&self.blur_bgl,
			&self.comp_bgl,
			&self.blur_u_h,
			&self.blur_u_v,
			&self.comp_u,
			&self.sampler,
			&self.view_t,
			&self.view_a,
			&self.view_b,
			&self.bgcolor_view,
			&self.view_cur,
		);
		self.blur_t2b = blur_t2b;
		self.blur_b2a = blur_b2a;
		self.comp_bind = comp_bind;
		self.w = w;
		self.h = h;
	}

	// Build the per-pixel scrim-colour map: clear to the global bg colour, then draw
	// the per-cell bg rects (opaque) over it. A glyph's halo then takes its own
	// cell's bg colour instead of always the global one. The alpha channel doubles
	// as an "own-bg" mask - cleared to 0, the opaque cell rects write 1, so the blur
	// can drop coverage from cells that already carry a solid bg (reverse video,
	// coloured bg, selection): they have full contrast, so a halo there is only
	// artifact (nano's reverse header cast a jumping drop-shadow). See fs_blur.
	pub fn render_bgcolor(
		&mut self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		encoder: &mut wgpu::CommandEncoder,
		cells: &[RectInstance],
		global_bg: [f32; 4],
	) {
		self.bg_rects
			.set_resolution(queue, self.w as f32, self.h as f32);
		self.bg_rects.upload(device, queue, cells);
		let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("scrim bgcolor"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: &self.bgcolor_view,
				resolve_target: None,
				depth_slice: None,
				ops: wgpu::Operations {
					load: wgpu::LoadOp::Clear(wgpu::Color {
						r: global_bg[0] as f64,
						g: global_bg[1] as f64,
						b: global_bg[2] as f64,
						a: 0.0, // own-bg mask: 0 here, the cell rects write 1 (see fs_blur)
					}),
					store: wgpu::StoreOp::Store,
				},
			})],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		self.bg_rects.draw(&mut pass, 0..cells.len() as u32);
	}

	// The render target for the scene's text (tex_t). Clear it transparent and
	// render the prepared text into it before calling `blur`.
	pub fn text_view(&self) -> &wgpu::TextureView {
		&self.view_t
	}

	// The render target for the cursor coverage (tex_cur), separate from the text.
	// Clear it transparent and draw the cursor quads (`draw_cursors`) into it before
	// calling `blur`; the flags in `blur`/`composite` decide where it contributes.
	pub fn cursor_view(&self) -> &wgpu::TextureView {
		&self.view_cur
	}

	// Upload the cursor quads destined for tex_cur. Call before the cursor pass;
	// draw with `draw_cursors`.
	pub fn upload_cursors(
		&mut self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		quads: &[RectInstance],
	) {
		self.cursor_rects
			.set_resolution(queue, self.w as f32, self.h as f32);
		self.cursor_rects.upload(device, queue, quads);
		self.cursor_count = quads.len() as u32;
	}

	// Draw the uploaded cursor quads into the current (tex_cur) pass.
	pub fn draw_cursors(&self, pass: &mut wgpu::RenderPass<'_>) {
		if self.cursor_count > 0 {
			self.cursor_rects.draw(pass, 0..self.cursor_count);
		}
	}

	// Two separable passes: H (tex_t->tex_b) then V (tex_b->tex_a). After this tex_a
	// holds the blurred scrim; tex_t keeps the crisp coverage for the border pass.
	// `cursor` (0/1) folds the cursor coverage into the halo - only in the H pass
	// (the V pass reads tex_b, which already carries it, so its flag stays 0).
	pub fn blur(
		&self,
		queue: &wgpu::Queue,
		encoder: &mut wgpu::CommandEncoder,
		sigma: f32,
		ramp: f32,
		cursor: f32,
	) {
		let res = [self.w as f32, self.h as f32];
		// write both uniforms up front (they target different buffers, so neither
		// overwrites the other when the queue applies them before the passes run)
		queue.write_buffer(
			&self.blur_u_h,
			0,
			bytemuck::bytes_of(&BlurU {
				resolution: res,
				dir: [1.0, 0.0],
				sigma,
				ramp,
				cursor,
				_pad: 0.0,
			}),
		);
		queue.write_buffer(
			&self.blur_u_v,
			0,
			bytemuck::bytes_of(&BlurU {
				resolution: res,
				dir: [0.0, 1.0],
				sigma,
				ramp,
				cursor: 0.0,
				_pad: 0.0,
			}),
		);
		for (src_bind, dst) in [
			(&self.blur_t2b, &self.view_b),
			(&self.blur_b2a, &self.view_a),
		] {
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("scrim blur pass"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: dst,
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
			pass.set_pipeline(&self.blur_pipe);
			pass.set_bind_group(0, src_bind, &[]);
			pass.draw(0..3, 0..1);
		}
	}

	// Draw the scrim into the current pass, under the text: blurred coverage from
	// tex_a, coloured per-pixel by the bgcolor map, plus a `border_px` dilated
	// outline of the crisp coverage (tex_t, + tex_cur when `cursor` is 1).
	pub fn composite(
		&self,
		queue: &wgpu::Queue,
		pass: &mut wgpu::RenderPass<'_>,
		intensity: f32,
		border_px: f32,
		cursor: f32,
	) {
		queue.write_buffer(
			&self.comp_u,
			0,
			bytemuck::bytes_of(&CompU {
				resolution: [self.w as f32, self.h as f32],
				intensity,
				border_px,
				cursor,
				_pad: [0.0; 3],
			}),
		);
		pass.set_pipeline(&self.comp_pipe);
		pass.set_bind_group(0, &self.comp_bind, &[]);
		pass.draw(0..3, 0..1);
	}
}

#[allow(clippy::type_complexity)]
fn make_textures(
	device: &wgpu::Device,
	w: u32,
	h: u32,
) -> (
	wgpu::Texture,
	wgpu::Texture,
	wgpu::Texture,
	wgpu::TextureView,
	wgpu::TextureView,
	wgpu::TextureView,
) {
	let desc = |label| wgpu::TextureDescriptor {
		label: Some(label),
		size: wgpu::Extent3d {
			width: w.max(1),
			height: h.max(1),
			depth_or_array_layers: 1,
		},
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: FMT,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
		view_formats: &[],
	};
	let tex_t = device.create_texture(&desc("scrim tex t"));
	let tex_a = device.create_texture(&desc("scrim tex a"));
	let tex_b = device.create_texture(&desc("scrim tex b"));
	let view_t = tex_t.create_view(&Default::default());
	let view_a = tex_a.create_view(&Default::default());
	let view_b = tex_b.create_view(&Default::default());
	(tex_t, tex_a, tex_b, view_t, view_a, view_b)
}

// A single FMT coverage texture + its view (the cursor's crisp coverage).
fn cover_tex(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
	let tex = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("scrim tex cur"),
		size: wgpu::Extent3d {
			width: w.max(1),
			height: h.max(1),
			depth_or_array_layers: 1,
		},
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: FMT,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
		view_formats: &[],
	});
	let view = tex.create_view(&Default::default());
	(tex, view)
}

fn bgcolor_tex(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
	device.create_texture(&wgpu::TextureDescriptor {
		label: Some("scrim bgcolor"),
		size: wgpu::Extent3d {
			width: w.max(1),
			height: h.max(1),
			depth_or_array_layers: 1,
		},
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: FMT,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
		view_formats: &[],
	})
}

#[allow(clippy::too_many_arguments)]
fn binds(
	device: &wgpu::Device,
	blur_bgl: &wgpu::BindGroupLayout,
	comp_bgl: &wgpu::BindGroupLayout,
	blur_u_h: &wgpu::Buffer,
	blur_u_v: &wgpu::Buffer,
	comp_u: &wgpu::Buffer,
	sampler: &wgpu::Sampler,
	view_t: &wgpu::TextureView,
	view_a: &wgpu::TextureView,
	view_b: &wgpu::TextureView,
	bgcolor_view: &wgpu::TextureView,
	view_cur: &wgpu::TextureView,
) -> (wgpu::BindGroup, wgpu::BindGroup, wgpu::BindGroup) {
	let mk_blur = |ubuf: &wgpu::Buffer, view| {
		device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("scrim blur bind"),
			layout: blur_bgl,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: ubuf.as_entire_binding(),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::TextureView(view),
				},
				wgpu::BindGroupEntry {
					binding: 2,
					resource: wgpu::BindingResource::Sampler(sampler),
				},
				wgpu::BindGroupEntry {
					binding: 3,
					resource: wgpu::BindingResource::TextureView(bgcolor_view),
				},
				wgpu::BindGroupEntry {
					binding: 4,
					resource: wgpu::BindingResource::TextureView(view_cur),
				},
			],
		})
	};
	let comp = device.create_bind_group(&wgpu::BindGroupDescriptor {
		label: Some("scrim comp bind"),
		layout: comp_bgl,
		entries: &[
			wgpu::BindGroupEntry {
				binding: 0,
				resource: comp_u.as_entire_binding(),
			},
			wgpu::BindGroupEntry {
				binding: 1,
				resource: wgpu::BindingResource::TextureView(view_a),
			},
			wgpu::BindGroupEntry {
				binding: 2,
				resource: wgpu::BindingResource::Sampler(sampler),
			},
			wgpu::BindGroupEntry {
				binding: 3,
				resource: wgpu::BindingResource::TextureView(bgcolor_view),
			},
			wgpu::BindGroupEntry {
				binding: 4,
				resource: wgpu::BindingResource::TextureView(view_t),
			},
			wgpu::BindGroupEntry {
				binding: 5,
				resource: wgpu::BindingResource::TextureView(view_cur),
			},
		],
	});
	// t2b: H pass samples tex_t (horizontal uniform); b2a: V pass samples tex_b.
	(mk_blur(blur_u_h, view_t), mk_blur(blur_u_v, view_b), comp)
}

fn ubuf_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
	wgpu::BindGroupLayoutEntry {
		binding,
		visibility: wgpu::ShaderStages::FRAGMENT,
		ty: wgpu::BindingType::Buffer {
			ty: wgpu::BufferBindingType::Uniform,
			has_dynamic_offset: false,
			min_binding_size: None,
		},
		count: None,
	}
}
fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
	wgpu::BindGroupLayoutEntry {
		binding,
		visibility: wgpu::ShaderStages::FRAGMENT,
		ty: wgpu::BindingType::Texture {
			sample_type: wgpu::TextureSampleType::Float { filterable: true },
			view_dimension: wgpu::TextureViewDimension::D2,
			multisampled: false,
		},
		count: None,
	}
}
fn samp_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
	wgpu::BindGroupLayoutEntry {
		binding,
		visibility: wgpu::ShaderStages::FRAGMENT,
		ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
		count: None,
	}
}

fn pipeline(
	device: &wgpu::Device,
	shader: &wgpu::ShaderModule,
	fs: &str,
	format: wgpu::TextureFormat,
	bgl: &wgpu::BindGroupLayout,
	label: &str,
) -> wgpu::RenderPipeline {
	make_pipeline(device, shader, fs, format, bgl, label, None)
}
fn pipeline_blend(
	device: &wgpu::Device,
	shader: &wgpu::ShaderModule,
	fs: &str,
	format: wgpu::TextureFormat,
	bgl: &wgpu::BindGroupLayout,
	label: &str,
) -> wgpu::RenderPipeline {
	make_pipeline(
		device,
		shader,
		fs,
		format,
		bgl,
		label,
		Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
	)
}

#[allow(clippy::too_many_arguments)]
fn make_pipeline(
	device: &wgpu::Device,
	shader: &wgpu::ShaderModule,
	fs: &str,
	format: wgpu::TextureFormat,
	bgl: &wgpu::BindGroupLayout,
	label: &str,
	blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
	let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
		label: Some(label),
		bind_group_layouts: &[Some(bgl)],
		immediate_size: 0,
	});
	device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
		label: Some(label),
		layout: Some(&layout),
		vertex: wgpu::VertexState {
			module: shader,
			entry_point: Some("vs"),
			compilation_options: Default::default(),
			buffers: &[],
		},
		fragment: Some(wgpu::FragmentState {
			module: shader,
			entry_point: Some(fs),
			compilation_options: Default::default(),
			targets: &[Some(wgpu::ColorTargetState {
				format,
				blend,
				write_mask: wgpu::ColorWrites::ALL,
			})],
		}),
		primitive: wgpu::PrimitiveState::default(),
		depth_stencil: None,
		multisample: wgpu::MultisampleState::default(),
		multiview_mask: None,
		cache: None,
	})
}

const WGSL: &str = r#"
struct VsOut { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    var xy = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    let p = xy[i];
    var o: VsOut;
    o.clip = vec4<f32>(p, 0.0, 1.0);
    o.uv = vec2<f32>((p.x + 1.0) * 0.5, 1.0 - (p.y + 1.0) * 0.5);
    return o;
}

// scalar pads (NOT vec3, which would force 16-byte alignment -> 48 bytes
// and mismatch the 32-byte Rust struct)
struct BlurU { resolution: vec2<f32>, dir: vec2<f32>, sigma: f32, ramp: f32, cursor: f32, _p2: f32 };
@group(0) @binding(0) var<uniform> bu: BlurU;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var bmask: texture_2d<f32>; // bgcolor map; .a = own-bg mask
@group(0) @binding(4) var bcur: texture_2d<f32>;  // crisp cursor coverage

// separable blur; fixed 25 taps scaled by sigma so any radius is ~3sigma-covered.
// `ramp` picks the falloff kernel: 0 = gaussian, 1 = linear (tent), 2 = s-curve
// (smoothstep) - the shape of how the halo fades with distance.
//
// Each tap is gated by `keep` = 1 - own-bg mask, so coverage sitting over a cell
// with its own solid bg (reverse video, coloured bg, selection) contributes to no
// pixel's halo. Gating the SOURCE coverage (not the final pixel) is what stops a
// reverse-video header's glyphs from bleeding a halo below the bar - the artifact
// that read as a drop-shadow jumping with the app-scroll slide.
@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / bu.resolution;
    let s = max(bu.sigma, 0.0001);
    let spacing = max(1.0, s * 3.0 / 12.0);
    let ext = s * 3.0; // kernel extent; the linear/s kernels hit zero here
    var sum = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var i = -12; i <= 12; i = i + 1) {
        let off = f32(i) * spacing;
        var w = exp(-0.5 * off * off / (s * s));
        if (bu.ramp > 0.5 && bu.ramp < 1.5) {
            w = max(0.0, 1.0 - abs(off) / ext);
        } else if (bu.ramp >= 1.5) {
            let t = clamp(1.0 - abs(off) / ext, 0.0, 1.0);
            w = t * t * (3.0 - 2.0 * t);
        }
        let uv = in.uv + bu.dir * (off * texel);
        let keep = 1.0 - textureSample(bmask, samp, uv).a;
        // fold the cursor coverage into the H pass (bu.cursor is 0 in the V pass)
        let cov = textureSample(tex, samp, uv) + bu.cursor * textureSample(bcur, samp, uv);
        sum += cov * (w * keep);
        wsum += w;
    }
    return sum / wsum;
}

struct CompU { resolution: vec2<f32>, intensity: f32, border_px: f32, cursor: f32, _p1: f32, _p2: f32, _p3: f32 };
@group(0) @binding(0) var<uniform> cu: CompU;
@group(0) @binding(1) var gtex: texture_2d<f32>;   // blurred glyph coverage (alpha)
@group(0) @binding(2) var gsamp: sampler;
@group(0) @binding(3) var bgtex: texture_2d<f32>;  // per-pixel scrim colour
@group(0) @binding(4) var ttex: texture_2d<f32>;   // crisp glyph coverage
@group(0) @binding(5) var ccur: texture_2d<f32>;   // crisp cursor coverage

// colour the blurred coverage per-pixel by the local bg colour; premultiplied.
// border: dilate the crisp coverage by border_px (8 taps; linear sampling keeps
// it antialiased) and take the union with the scrim - a solid bg-coloured plate
// hugging each glyph. The crisp text draws over its interior, so what remains
// visible is the thin outline around the letterforms. Each border tap is gated by
// the own-bg mask (bgtex.a) too, matching fs_blur, so an own-bg glyph casts no
// outline (the blurred halo is already masked at its source). The cursor coverage
// joins the outline source only when cu.cursor is 1.
fn border_tap(uv: vec2<f32>) -> f32 {
    let cov = max(textureSample(ttex, gsamp, uv).a, cu.cursor * textureSample(ccur, gsamp, uv).a);
    return cov * (1.0 - textureSample(bgtex, gsamp, uv).a);
}
@fragment
fn fs_comp(in: VsOut) -> @location(0) vec4<f32> {
    let ga = clamp(textureSample(gtex, gsamp, in.uv).a * cu.intensity, 0.0, 1.0);
    let rgb = textureSample(bgtex, gsamp, in.uv).rgb;
    let texel = 1.0 / cu.resolution;
    let r = max(cu.border_px, 0.0001);
    let dg = r * 0.7071; // diagonal taps at the same radius -> round outline
    var m = 0.0;
    m = max(m, border_tap(in.uv + vec2<f32>( r, 0.0) * texel));
    m = max(m, border_tap(in.uv + vec2<f32>(-r, 0.0) * texel));
    m = max(m, border_tap(in.uv + vec2<f32>(0.0,  r) * texel));
    m = max(m, border_tap(in.uv + vec2<f32>(0.0, -r) * texel));
    m = max(m, border_tap(in.uv + vec2<f32>( dg,  dg) * texel));
    m = max(m, border_tap(in.uv + vec2<f32>( dg, -dg) * texel));
    m = max(m, border_tap(in.uv + vec2<f32>(-dg,  dg) * texel));
    m = max(m, border_tap(in.uv + vec2<f32>(-dg, -dg) * texel));
    let border = clamp(m, 0.0, 1.0) * step(0.001, cu.border_px);
    let a = max(ga, border);
    return vec4<f32>(rgb * a, a);
}
"#;
