// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

//! Text readability glow: a blurred, background-coloured halo behind glyphs so
//! text stays legible over a light/busy background image or a near-transparent
//! terminal. The scene's text is rendered to a texture, blurred (2-pass separable
//! Gaussian), and composited UNDER the crisp text, coloured per-pixel by a
//! `bgcolor` map so a glyph's halo takes ITS cell's bg colour (a glyph on a
//! one-off colored cell isn't smeared with the global bg colour).
//!
//! Ping-pong: tex_a <- text, then H-blur tex_a->tex_b, V-blur tex_b->tex_a; tex_a is
//! the blurred coverage; the composite multiplies it by `bgcolor`.

use crate::gfx::{RectInstance, RectRenderer};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurU {
	resolution: [f32; 2],
	dir: [f32; 2],
	sigma: f32,
	_pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CompU {
	intensity: f32, // coverage boost; the colour comes from the bgcolor texture
	_pad: [f32; 3],
}

pub struct Glow {
	tex_a: wgpu::Texture,
	tex_b: wgpu::Texture,
	view_a: wgpu::TextureView,
	view_b: wgpu::TextureView,
	sampler: wgpu::Sampler,
	blur_pipe: wgpu::RenderPipeline,
	blur_bgl: wgpu::BindGroupLayout,
	// one uniform PER direction: all queue.write_buffer calls are applied before
	// the command buffer runs, so a single shared buffer would give BOTH passes
	// the last-written dir (-> vertical blur twice, no horizontal). Two buffers fix it.
	blur_u_h: wgpu::Buffer,
	blur_u_v: wgpu::Buffer,
	blur_a2b: wgpu::BindGroup, // sample tex_a (uses blur_u_h), write tex_b
	blur_b2a: wgpu::BindGroup, // sample tex_b (uses blur_u_v), write tex_a
	comp_pipe: wgpu::RenderPipeline,
	comp_bgl: wgpu::BindGroupLayout,
	comp_u: wgpu::Buffer,
	comp_bind: wgpu::BindGroup, // sample tex_a (glow alpha) + bgcolor (rgb)
	// per-pixel glow colour: cleared to the global bg, with per-cell bg rects drawn
	// over it, so a glyph's halo takes ITS cell's bg colour (not always the global).
	bgcolor: wgpu::Texture,
	bgcolor_view: wgpu::TextureView,
	bg_rects: RectRenderer,
	w: u32,
	h: u32,
}

impl Glow {
	pub fn new(device: &wgpu::Device, target: wgpu::TextureFormat, w: u32, h: u32) -> Self {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("glow shader"),
			source: wgpu::ShaderSource::Wgsl(WGSL.into()),
		});
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some("glow sampler"),
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			address_mode_u: wgpu::AddressMode::ClampToEdge,
			address_mode_v: wgpu::AddressMode::ClampToEdge,
			..Default::default()
		});
		// blur: uniform + sampled texture + sampler
		let blur_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("glow blur bgl"),
			entries: &[ubuf_entry(0), tex_entry(1), samp_entry(2)],
		});
		let mku = |label| {
			device.create_buffer(&wgpu::BufferDescriptor {
				label: Some(label),
				size: std::mem::size_of::<BlurU>() as u64,
				usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
				mapped_at_creation: false,
			})
		};
		let blur_u_h = mku("glow blur u h");
		let blur_u_v = mku("glow blur u v");
		let blur_pipe = pipeline(device, &shader, "fs_blur", FMT, &blur_bgl, "glow blur");

		let comp_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("glow comp bgl"),
			entries: &[ubuf_entry(0), tex_entry(1), samp_entry(2), tex_entry(3)],
		});
		let comp_u = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("glow comp u"),
			size: std::mem::size_of::<CompU>() as u64,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		let comp_pipe = pipeline_blend(device, &shader, "fs_comp", target, &comp_bgl, "glow comp");

		let (tex_a, tex_b, view_a, view_b) = make_textures(device, w, h);
		let bgcolor = bgcolor_tex(device, w, h);
		let bgcolor_view = bgcolor.create_view(&Default::default());
		let bg_rects = RectRenderer::new(device, FMT);
		let (blur_a2b, blur_b2a, comp_bind) = binds(
			device,
			&blur_bgl,
			&comp_bgl,
			&blur_u_h,
			&blur_u_v,
			&comp_u,
			&sampler,
			&view_a,
			&view_b,
			&bgcolor_view,
		);

		Self {
			tex_a,
			tex_b,
			view_a,
			view_b,
			sampler,
			blur_pipe,
			blur_bgl,
			blur_u_h,
			blur_u_v,
			blur_a2b,
			blur_b2a,
			comp_pipe,
			comp_bgl,
			comp_u,
			comp_bind,
			bgcolor,
			bgcolor_view,
			bg_rects,
			w,
			h,
		}
	}

	pub fn resize(&mut self, device: &wgpu::Device, w: u32, h: u32) {
		if w == 0 || h == 0 || (w == self.w && h == self.h) {
			return;
		}
		let (ta, tb, va, vb) = make_textures(device, w, h);
		self.tex_a = ta;
		self.tex_b = tb;
		self.view_a = va;
		self.view_b = vb;
		self.bgcolor = bgcolor_tex(device, w, h);
		self.bgcolor_view = self.bgcolor.create_view(&Default::default());
		let (a2b, b2a, cb) = binds(
			device,
			&self.blur_bgl,
			&self.comp_bgl,
			&self.blur_u_h,
			&self.blur_u_v,
			&self.comp_u,
			&self.sampler,
			&self.view_a,
			&self.view_b,
			&self.bgcolor_view,
		);
		self.blur_a2b = a2b;
		self.blur_b2a = b2a;
		self.comp_bind = cb;
		self.w = w;
		self.h = h;
	}

	// Build the per-pixel glow-colour map: clear to the global bg colour, then draw
	// the per-cell bg rects (opaque) over it. A glyph's halo then takes its own
	// cell's bg colour instead of always the global one.
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
			label: Some("glow bgcolor"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: &self.bgcolor_view,
				resolve_target: None,
				depth_slice: None,
				ops: wgpu::Operations {
					load: wgpu::LoadOp::Clear(wgpu::Color {
						r: global_bg[0] as f64,
						g: global_bg[1] as f64,
						b: global_bg[2] as f64,
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
		self.bg_rects.draw(&mut pass, 0..cells.len() as u32);
	}

	// The render target for the scene's text (tex_a). Clear it transparent and
	// render the prepared text into it before calling `blur`.
	pub fn text_view(&self) -> &wgpu::TextureView {
		&self.view_a
	}

	// Two separable passes: H (tex_a->tex_b) then V (tex_b->tex_a). After this tex_a
	// holds the blurred glow.
	pub fn blur(&self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, sigma: f32) {
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
				_pad: [0.0; 3],
			}),
		);
		queue.write_buffer(
			&self.blur_u_v,
			0,
			bytemuck::bytes_of(&BlurU {
				resolution: res,
				dir: [0.0, 1.0],
				sigma,
				_pad: [0.0; 3],
			}),
		);
		for (src_bind, dst) in [
			(&self.blur_a2b, &self.view_b),
			(&self.blur_b2a, &self.view_a),
		] {
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("glow blur pass"),
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

	// Draw the glow into the current pass, under the text: blurred coverage from
	// tex_a, coloured per-pixel by the bgcolor map.
	pub fn composite(&self, queue: &wgpu::Queue, pass: &mut wgpu::RenderPass<'_>, intensity: f32) {
		queue.write_buffer(
			&self.comp_u,
			0,
			bytemuck::bytes_of(&CompU {
				intensity,
				_pad: [0.0; 3],
			}),
		);
		pass.set_pipeline(&self.comp_pipe);
		pass.set_bind_group(0, &self.comp_bind, &[]);
		pass.draw(0..3, 0..1);
	}
}

fn make_textures(
	device: &wgpu::Device,
	w: u32,
	h: u32,
) -> (
	wgpu::Texture,
	wgpu::Texture,
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
	let a = device.create_texture(&desc("glow tex a"));
	let b = device.create_texture(&desc("glow tex b"));
	let va = a.create_view(&Default::default());
	let vb = b.create_view(&Default::default());
	(a, b, va, vb)
}

fn bgcolor_tex(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
	device.create_texture(&wgpu::TextureDescriptor {
		label: Some("glow bgcolor"),
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
	view_a: &wgpu::TextureView,
	view_b: &wgpu::TextureView,
	bgcolor_view: &wgpu::TextureView,
) -> (wgpu::BindGroup, wgpu::BindGroup, wgpu::BindGroup) {
	let mk_blur = |ubuf: &wgpu::Buffer, view| {
		device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("glow blur bind"),
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
			],
		})
	};
	let comp = device.create_bind_group(&wgpu::BindGroupDescriptor {
		label: Some("glow comp bind"),
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
		],
	});
	// a2b: H pass samples tex_a (horizontal uniform); b2a: V pass samples tex_b.
	(mk_blur(blur_u_h, view_a), mk_blur(blur_u_v, view_b), comp)
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

// three scalar pads (NOT vec3, which would force 16-byte alignment -> 48 bytes
// and mismatch the 32-byte Rust struct)
struct BlurU { resolution: vec2<f32>, dir: vec2<f32>, sigma: f32, _p0: f32, _p1: f32, _p2: f32 };
@group(0) @binding(0) var<uniform> bu: BlurU;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

// separable Gaussian; fixed 25 taps scaled by sigma so any radius is ~3sigma-covered
@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / bu.resolution;
    let s = max(bu.sigma, 0.0001);
    let spacing = max(1.0, s * 3.0 / 12.0);
    var sum = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var i = -12; i <= 12; i = i + 1) {
        let off = f32(i) * spacing;
        let w = exp(-0.5 * off * off / (s * s));
        let uv = in.uv + bu.dir * (off * texel);
        sum += textureSample(tex, samp, uv) * w;
        wsum += w;
    }
    return sum / wsum;
}

struct CompU { intensity: f32, _p0: f32, _p1: f32, _p2: f32 };
@group(0) @binding(0) var<uniform> cu: CompU;
@group(0) @binding(1) var gtex: texture_2d<f32>;   // blurred glyph coverage (alpha)
@group(0) @binding(2) var gsamp: sampler;
@group(0) @binding(3) var bgtex: texture_2d<f32>;  // per-pixel glow colour

// colour the blurred coverage per-pixel by the local bg colour; premultiplied
@fragment
fn fs_comp(in: VsOut) -> @location(0) vec4<f32> {
    let a = clamp(textureSample(gtex, gsamp, in.uv).a * cu.intensity, 0.0, 1.0);
    let rgb = textureSample(bgtex, gsamp, in.uv).rgb;
    return vec4<f32>(rgb * a, a);
}
"#;
