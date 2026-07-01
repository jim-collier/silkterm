// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

use std::num::NonZeroU32;
use std::sync::Arc;

use glutin::config::GlConfig;
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext, Version};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{Surface as GlWindowSurface, SurfaceAttributesBuilder, WindowSurface};
use glutin_winit::DisplayBuilder;
use raw_window_handle::HasWindowHandle;
use wgpu::hal::api::Gles;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

// How a frame reaches the screen. `Native` is the normal wgpu surface (Vulkan/
// Metal/DX/Wayland - supports premultiplied alpha where the platform does).
// `Gl` runs wgpu on a glutin-created GL context so X11 can do per-pixel alpha:
// the wgpu surface there can't bind the window's ARGB visual, glutin can. We
// render to the GL default framebuffer (fbo 0) and present via swap_buffers.
enum Backend {
	Native(wgpu::Surface<'static>),
	// The GL default framebuffer (fbo 0) is Y-flipped vs wgpu's top-left origin,
	// which flips our quads and clips glyphon's bounds-limited text out entirely.
	// So the scene renders to `offscreen` (normal orientation, exactly like the
	// native path), then `blit` flips it into the default framebuffer `fb`.
	Gl {
		ctx: PossiblyCurrentContext,
		surface: GlWindowSurface<WindowSurface>,
		fb: wgpu::Texture,
		offscreen: wgpu::Texture,
		blit: Blit,
	},
}

// Fullscreen flip-blit of the offscreen texture into the GL default framebuffer.
struct Blit {
	pipeline: wgpu::RenderPipeline,
	sampler: wgpu::Sampler,
	layout: wgpu::BindGroupLayout,
	bind: wgpu::BindGroup,
}

impl Blit {
	fn new(device: &wgpu::Device, format: wgpu::TextureFormat, src: &wgpu::TextureView) -> Self {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("blit shader"),
			source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
		});
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("blit bgl"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Float { filterable: true },
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
					count: None,
				},
			],
		});
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("blit layout"),
			bind_group_layouts: &[Some(&layout)],
			immediate_size: 0,
		});
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("blit pipeline"),
			layout: Some(&pl),
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
					blend: None, // straight copy; offscreen already holds premultiplied rgba
					write_mask: wgpu::ColorWrites::ALL,
				})],
			}),
			primitive: wgpu::PrimitiveState::default(),
			depth_stencil: None,
			multisample: wgpu::MultisampleState::default(),
			multiview_mask: None,
			cache: None,
		});
		let bind = Self::bind(device, &layout, &sampler, src);
		Self {
			pipeline,
			sampler,
			layout,
			bind,
		}
	}

	fn bind(
		device: &wgpu::Device,
		layout: &wgpu::BindGroupLayout,
		sampler: &wgpu::Sampler,
		src: &wgpu::TextureView,
	) -> wgpu::BindGroup {
		device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("blit bind"),
			layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(src),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::Sampler(sampler),
				},
			],
		})
	}

	fn rebind(&mut self, device: &wgpu::Device, src: &wgpu::TextureView) {
		self.bind = Self::bind(device, &self.layout, &self.sampler, src);
	}
}

// A frame in flight, returned by `begin_frame` and consumed by `end_frame`.
pub enum Frame {
	Native(wgpu::SurfaceTexture),
	Gl,
}

pub struct Gfx {
	pub device: wgpu::Device,
	pub queue: wgpu::Queue,
	pub config: wgpu::SurfaceConfiguration,
	pub format: wgpu::TextureFormat,
	pub transparent: bool, // surface can show the desktop through (compositor present)
	pub adapter_info: wgpu::AdapterInfo,
	backend: Backend,
	_window: Arc<Window>,
}

impl Gfx {
	pub fn new(window: Arc<Window>) -> anyhow::Result<Self> {
		Self::with_backends(window, wgpu::Backends::all())
	}

	// Native wgpu path with a chosen backend set. Pop-out dialog windows pass
	// `Backends::PRIMARY` (Vulkan/Metal/DX12, NO GL): initializing wgpu's GL
	// backend while the main window holds a glutin GL/EGL context panics in
	// wgpu-hal's EGL teardown (`unmake_current().unwrap()`), so dialogs must avoid
	// touching EGL entirely.
	pub fn with_backends(window: Arc<Window>, backends: wgpu::Backends) -> anyhow::Result<Self> {
		let size = window.inner_size();
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
			backends,
			flags: wgpu::InstanceFlags::default(),
			memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
			backend_options: wgpu::BackendOptions::default(),
			display: None,
		});
		let surface = instance.create_surface(window.clone())?;

		// Prefer a real GPU; if none can be acquired, retry forcing a software
		// (CPU) adapter so the app still runs without hardware acceleration.
		let pick = |fallback| {
			pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
				power_preference: wgpu::PowerPreference::HighPerformance,
				compatible_surface: Some(&surface),
				force_fallback_adapter: fallback,
			}))
		};
		let adapter = pick(false).or_else(|_| pick(true))?;
		let adapter_info = adapter.get_info();
		log_renderer(&adapter_info);

		let (device, queue) =
			pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
				label: Some("silkterm device"),
				required_features: wgpu::Features::empty(),
				required_limits: adapter.limits(),
				..Default::default()
			}))?;

		let caps = surface.get_capabilities(&adapter);
		let format = caps
			.formats
			.iter()
			.copied()
			.find(|f| f.is_srgb())
			.unwrap_or(caps.formats[0]);

		// Prefer a premultiplied-alpha mode so a translucent background shows the
		// desktop through. If only Opaque is available (no compositor), stay
		// opaque - transparency is silently ignored.
		let alpha_mode = caps
			.alpha_modes
			.iter()
			.copied()
			.find(|m| *m == wgpu::CompositeAlphaMode::PreMultiplied)
			.unwrap_or(caps.alpha_modes[0]);
		let transparent = alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied;

		let config = wgpu::SurfaceConfiguration {
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			format,
			width: size.width.max(1),
			height: size.height.max(1),
			present_mode: wgpu::PresentMode::AutoVsync,
			alpha_mode,
			view_formats: vec![],
			desired_maximum_frame_latency: 2,
		};
		surface.configure(&device, &config);

		Ok(Self {
			device,
			queue,
			config,
			format,
			transparent,
			adapter_info,
			backend: Backend::Native(surface),
			_window: window,
		})
	}

	// X11-only per-pixel transparency: glutin creates the window with a 32-bit
	// ARGB visual + transparent GL context, and wgpu runs on it via hal external
	// interop (see examples/wgpu_on_glutin.rs). Returns the window it created.
	pub fn new_gl_transparent(
		el: &ActiveEventLoop,
		attrs: WindowAttributes,
	) -> anyhow::Result<(Self, Arc<Window>)> {
		// No transparency requirement in the template: the picker closure must
		// return a Config (can't say "none fit"), and a panic there would abort
		// past resumed()'s native-backend fallback (panic=abort in release).
		// So match broadly, prefer transparent+deepest-alpha, validate after.
		let template = glutin::config::ConfigTemplateBuilder::new();
		let (window, config) = DisplayBuilder::new()
			.with_window_attributes(Some(attrs))
			.build(el, template, |cfgs| {
				cfgs.reduce(|a, c| {
					let (ta, tc) = (
						a.supports_transparency().unwrap_or(false),
						c.supports_transparency().unwrap_or(false),
					);
					if (tc, c.alpha_size()) > (ta, a.alpha_size()) {
						c
					} else {
						a
					}
				})
				// unreachable unless GL reports zero framebuffer configs at all
				.expect("GL reported no framebuffer configs")
			})
			.map_err(|e| anyhow::anyhow!("glutin display build: {e}"))?;
		if !config.supports_transparency().unwrap_or(false) || config.alpha_size() < 8 {
			return Err(anyhow::anyhow!(
				"no transparency-capable GL config (no ARGB visual?)"
			));
		}
		let window = Arc::new(window.ok_or_else(|| anyhow::anyhow!("glutin made no window"))?);
		let raw = window.window_handle()?.as_raw();
		let gl_display = config.display();

		// Request a high GL version. NVIDIA/Linux honors the *exact* version asked
		// (gfx-rs/wgpu#8676), and many wgpu GL bugs - including rendering into a 2D
		// texture view, which is how glyphon draws its atlas - only disappear on
		// GL >=4.2 (gfx-rs/wgpu#8675). A 3.3/4.1 context renders no glyphon text.
		// Try 4.6 down so non-NVIDIA drivers still get a context.
		let ctx = {
			let mut made = None;
			for (maj, min) in [(4u8, 6u8), (4, 3), (4, 2), (4, 1), (3, 3)] {
				let attrs = ContextAttributesBuilder::new()
					.with_context_api(ContextApi::OpenGl(Some(Version::new(maj, min))))
					.build(Some(raw));
				if let Ok(c) = unsafe { gl_display.create_context(&config, &attrs) } {
					made = Some(c);
					break;
				}
			}
			made.ok_or_else(|| anyhow::anyhow!("no GL context could be created"))?
		};
		let size = window.inner_size();
		let surface = unsafe {
			gl_display.create_window_surface(
				&config,
				&SurfaceAttributesBuilder::<WindowSurface>::new().build(
					raw,
					NonZeroU32::new(size.width.max(1)).unwrap(),
					NonZeroU32::new(size.height.max(1)).unwrap(),
				),
			)?
		};
		let ctx = ctx.make_current(&surface)?;

		// wrap glutin's GL context as a wgpu device (hal external interop)
		let exposed = unsafe {
			wgpu::hal::gles::Adapter::new_external(
				|name| {
					std::ffi::CString::new(name)
						.map(|c| gl_display.get_proc_address(&c) as *const _)
						.unwrap_or(std::ptr::null())
				},
				wgpu::GlBackendOptions::default(),
			)
		}
		.ok_or_else(|| anyhow::anyhow!("wgpu GL external adapter init failed"))?;

		// empty flags: no indirect-validation (needs compute the GL 3.3 context
		// lacks; we never use indirect draws).
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
			backends: wgpu::Backends::GL,
			flags: wgpu::InstanceFlags::empty(),
			memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
			backend_options: wgpu::BackendOptions::default(),
			display: None,
		});
		let adapter = unsafe { instance.create_adapter_from_hal::<Gles>(exposed) };
		let adapter_info = adapter.get_info();
		log_renderer(&adapter_info);
		let (device, queue) =
			pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
				label: Some("silkterm gl device"),
				required_features: wgpu::Features::empty(),
				required_limits: adapter.limits(),
				..Default::default()
			}))?;

		// The GL offscreen is linear-light, so it must NOT be sRGB (an sRGB-declared
		// offscreen makes the blit's textureSample DECODE, cancelling its lin2srgb).
		// It must also be HIGH-PRECISION: an 8-bit *linear* offscreen starves dark
		// gradients of codes -> pronounced banding (esp. a blurred background image).
		// Rgba16Float gives a linear intermediate with no banding; the blit then
		// does the single linear->sRGB encode (+ dither) into the 8-bit fbo 0.
		let format = wgpu::TextureFormat::Rgba16Float;
		let config = wgpu::SurfaceConfiguration {
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			format,
			width: size.width.max(1),
			height: size.height.max(1),
			present_mode: wgpu::PresentMode::AutoVsync,
			alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
			view_formats: vec![],
			desired_maximum_frame_latency: 2,
		};
		let fb = default_fb(&device, FB_FORMAT, config.width, config.height);
		let offscreen = offscreen_tex(&device, format, config.width, config.height);
		let blit = Blit::new(
			&device,
			FB_FORMAT,
			&offscreen.create_view(&Default::default()),
		);

		Ok((
			Self {
				device,
				queue,
				config,
				format,
				transparent: true,
				adapter_info,
				backend: Backend::Gl {
					ctx,
					surface,
					fb,
					offscreen,
					blit,
				},
				_window: window.clone(),
			},
			window,
		))
	}

	// Acquire the frame's render target. None -> skip this frame (surface lost).
	pub fn begin_frame(&mut self) -> Option<Frame> {
		match &self.backend {
			Backend::Native(surface) => {
				use wgpu::CurrentSurfaceTexture::*;
				match surface.get_current_texture() {
					Success(t) | Suboptimal(t) => Some(Frame::Native(t)),
					Outdated | Lost => {
						surface.configure(&self.device, &self.config);
						None
					}
					_ => None,
				}
			}
			Backend::Gl { .. } => Some(Frame::Gl),
		}
	}

	pub fn frame_view(&self, frame: &Frame) -> wgpu::TextureView {
		match (frame, &self.backend) {
			(Frame::Native(t), _) => t
				.texture
				.create_view(&wgpu::TextureViewDescriptor::default()),
			// the scene renders to the offscreen texture (normal orientation)
			(Frame::Gl, Backend::Gl { offscreen, .. }) => {
				offscreen.create_view(&wgpu::TextureViewDescriptor::default())
			}
			_ => unreachable!("frame/backend mismatch"),
		}
	}

	pub fn end_frame(&self, frame: Frame) {
		match (frame, &self.backend) {
			(Frame::Native(t), _) => t.present(),
			(
				Frame::Gl,
				Backend::Gl {
					ctx,
					surface,
					fb,
					blit,
					..
				},
			) => {
				// flip-blit the offscreen scene into the GL default framebuffer
				let target = fb.create_view(&wgpu::TextureViewDescriptor::default());
				let mut enc = self
					.device
					.create_command_encoder(&wgpu::CommandEncoderDescriptor {
						label: Some("blit"),
					});
				{
					let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
						label: Some("blit pass"),
						color_attachments: &[Some(wgpu::RenderPassColorAttachment {
							view: &target,
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
					pass.set_pipeline(&blit.pipeline);
					pass.set_bind_group(0, &blit.bind, &[]);
					pass.draw(0..3, 0..1);
				}
				self.queue.submit(Some(enc.finish()));
				let _ = surface.swap_buffers(ctx);
			}
			_ => {}
		}
	}

	pub fn resize(&mut self, w: u32, h: u32) {
		if w == 0 || h == 0 {
			return;
		}
		self.config.width = w;
		self.config.height = h;
		match &mut self.backend {
			Backend::Native(surface) => surface.configure(&self.device, &self.config),
			Backend::Gl {
				surface,
				ctx,
				fb,
				offscreen,
				blit,
			} => {
				surface.resize(
					ctx,
					NonZeroU32::new(w).unwrap(),
					NonZeroU32::new(h).unwrap(),
				);
				*fb = default_fb(&self.device, FB_FORMAT, w, h);
				*offscreen = offscreen_tex(&self.device, self.format, w, h);
				blit.rebind(
					&self.device,
					&offscreen.create_view(&wgpu::TextureViewDescriptor::default()),
				);
			}
		}
	}
}

impl Gfx {
	// Diagnostic: read the GL offscreen texture back and save it as a PNG. Bypasses
	// the compositor/X-pixmap quirks that make screenshotting GL windows unreliable.
	pub fn dump_offscreen(&self, path: &str) {
		let Backend::Gl { offscreen, .. } = &self.backend else {
			return;
		};
		let (w, h) = (self.config.width, self.config.height);
		let unpadded = w * 8; // Rgba16Float = 8 bytes/texel
		let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
		let bpr = unpadded.div_ceil(align) * align;
		let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("dump"),
			size: (bpr * h) as u64,
			usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
			mapped_at_creation: false,
		});
		let mut enc = self.device.create_command_encoder(&Default::default());
		enc.copy_texture_to_buffer(
			wgpu::TexelCopyTextureInfo {
				texture: offscreen,
				mip_level: 0,
				origin: wgpu::Origin3d::ZERO,
				aspect: wgpu::TextureAspect::All,
			},
			wgpu::TexelCopyBufferInfo {
				buffer: &buf,
				layout: wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(bpr),
					rows_per_image: Some(h),
				},
			},
			wgpu::Extent3d {
				width: w,
				height: h,
				depth_or_array_layers: 1,
			},
		);
		self.queue.submit(Some(enc.finish()));
		buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
		let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
		let data = buf.slice(..).get_mapped_range();
		// offscreen is linear Rgba16Float; decode f16 -> linear -> sRGB -> 8-bit so the
		// PNG matches what the blit produces on screen.
		let mut pix = Vec::with_capacity((w * h * 4) as usize);
		for row in 0..h {
			let s = (row * bpr) as usize;
			for px in data[s..s + unpadded as usize].chunks_exact(8) {
				let ch = |i: usize| f16_to_f32(u16::from_le_bytes([px[i * 2], px[i * 2 + 1]]));
				let enc = |c: f32| {
					let c = c.clamp(0.0, 1.0);
					let s = if c <= 0.0031308 {
						c * 12.92
					} else {
						1.055 * c.powf(1.0 / 2.4) - 0.055
					};
					(s * 255.0 + 0.5) as u8
				};
				pix.extend_from_slice(&[
					enc(ch(0)),
					enc(ch(1)),
					enc(ch(2)),
					(ch(3).clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
				]);
			}
		}
		let _ = image::save_buffer(path, &pix, w, h, image::ExtendedColorType::Rgba8);
	}
}

// Minimal half-float decode for the offscreen dump (no `half` dep).
fn f16_to_f32(bits: u16) -> f32 {
	let sign = (bits >> 15) & 1;
	let exp = (bits >> 10) & 0x1f;
	let mant = bits & 0x3ff;
	let v = if exp == 0 {
		(mant as f32) * 2f32.powi(-24)
	} else if exp == 0x1f {
		f32::MAX
	} else {
		(1.0 + mant as f32 / 1024.0) * 2f32.powi(exp as i32 - 15)
	};
	if sign == 1 { -v } else { v }
}

fn log_renderer(info: &wgpu::AdapterInfo) {
	eprintln!(
		"{}: renderer = {} [{:?} / {:?}]",
		crate::config::APP_NAME,
		info.name,
		info.backend,
		info.device_type,
	);
}

// Offscreen scene target for the GL path: rendered top-left like the native
// surface, then flip-blitted into the default framebuffer.
fn offscreen_tex(
	device: &wgpu::Device,
	format: wgpu::TextureFormat,
	w: u32,
	h: u32,
) -> wgpu::Texture {
	device.create_texture(&wgpu::TextureDescriptor {
		label: Some("offscreen"),
		size: wgpu::Extent3d {
			width: w.max(1),
			height: h.max(1),
			depth_or_array_layers: 1,
		},
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT
			| wgpu::TextureUsages::TEXTURE_BINDING
			| wgpu::TextureUsages::COPY_SRC, // for the dump_offscreen diagnostic
		view_formats: &[],
	})
}

// The GL default framebuffer (fbo 0) is treated as plain (non-sRGB) RGBA: it isn't
// sRGB-capable, so the blit shader sRGB-encodes explicitly and writes raw here.
const FB_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

// A wgpu texture aliasing the GL default framebuffer (fbo 0 = glutin's window).
fn default_fb(device: &wgpu::Device, format: wgpu::TextureFormat, w: u32, h: u32) -> wgpu::Texture {
	let hal = wgpu::hal::gles::Texture::default_framebuffer(format);
	unsafe {
		device.create_texture_from_hal::<Gles>(
			hal,
			&wgpu::TextureDescriptor {
				label: Some("default fb"),
				size: wgpu::Extent3d {
					width: w.max(1),
					height: h.max(1),
					depth_or_array_layers: 1,
				},
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format,
				usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
				view_formats: &[],
			},
		)
	}
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RectInstance {
	pub pos: [f32; 2],
	pub size: [f32; 2],
	pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
	resolution: [f32; 2],
	_pad: [f32; 2],
}

// flat colored quads: backgrounds, cursor, dividers, focus ring
pub struct RectRenderer {
	pipeline: wgpu::RenderPipeline,
	instances: wgpu::Buffer,
	capacity: u64,
	uniform: wgpu::Buffer,
	bind_group: wgpu::BindGroup,
}

impl RectRenderer {
	pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("rect shader"),
			source: wgpu::ShaderSource::Wgsl(RECT_WGSL.into()),
		});

		let uniform = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("rect uniform"),
			size: std::mem::size_of::<Uniform>() as u64,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});

		let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("rect bgl"),
			entries: &[wgpu::BindGroupLayoutEntry {
				binding: 0,
				visibility: wgpu::ShaderStages::VERTEX,
				ty: wgpu::BindingType::Buffer {
					ty: wgpu::BufferBindingType::Uniform,
					has_dynamic_offset: false,
					min_binding_size: None,
				},
				count: None,
			}],
		});

		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("rect bg"),
			layout: &bgl,
			entries: &[wgpu::BindGroupEntry {
				binding: 0,
				resource: uniform.as_entire_binding(),
			}],
		});

		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("rect layout"),
			bind_group_layouts: &[Some(&bgl)],
			immediate_size: 0,
		});

		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("rect pipeline"),
			layout: Some(&layout),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs"),
				compilation_options: Default::default(),
				buffers: &[wgpu::VertexBufferLayout {
					array_stride: std::mem::size_of::<RectInstance>() as u64,
					step_mode: wgpu::VertexStepMode::Instance,
					attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
				}],
			},
			fragment: Some(wgpu::FragmentState {
				module: &shader,
				entry_point: Some("fs"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState {
					format,
					// premultiplied so it composites onto a transparent surface;
					// the shader premultiplies, so RGB results match straight alpha
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

		let capacity = 256;
		let instances = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("rect instances"),
			size: capacity * std::mem::size_of::<RectInstance>() as u64,
			usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});

		Self {
			pipeline,
			instances,
			capacity,
			uniform,
			bind_group,
		}
	}

	pub fn set_resolution(&self, queue: &wgpu::Queue, w: f32, h: f32) {
		let u = Uniform {
			resolution: [w, h],
			_pad: [0.0, 0.0],
		};
		queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
	}

	pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[RectInstance]) {
		let needed = data.len() as u64;
		if needed > self.capacity {
			self.capacity = needed.next_power_of_two();
			self.instances = device.create_buffer(&wgpu::BufferDescriptor {
				label: Some("rect instances"),
				size: self.capacity * std::mem::size_of::<RectInstance>() as u64,
				usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
				mapped_at_creation: false,
			});
		}
		if !data.is_empty() {
			queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(data));
		}
	}

	pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, range: std::ops::Range<u32>) {
		if range.is_empty() {
			return;
		}
		pass.set_pipeline(&self.pipeline);
		pass.set_bind_group(0, &self.bind_group, &[]);
		pass.set_vertex_buffer(0, self.instances.slice(..));
		pass.draw(0..4, range);
	}
}

const RECT_WGSL: &str = r#"
struct Uniform { resolution: vec2<f32>, _pad: vec2<f32> };
@group(0) @binding(0) var<uniform> u: Uniform;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) color: vec4<f32>,
    @builtin(vertex_index) vi: u32,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs(in: VsIn) -> VsOut {
    var corner = vec2<f32>(f32(in.vi & 1u), f32((in.vi >> 1u) & 1u));
    var px = in.pos + corner * in.size;
    var ndc = vec2<f32>(px.x / u.resolution.x * 2.0 - 1.0, 1.0 - px.y / u.resolution.y * 2.0);
    var out: VsOut;
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    // premultiply: lets translucent backgrounds composite over the desktop
    return vec4<f32>(in.color.rgb * in.color.a, in.color.a);
}
"#;

// Fullscreen-triangle flip-blit: samples the offscreen scene and writes it to
// the GL default framebuffer with V flipped (fbo 0 has a bottom-left origin).
// The offscreen already holds premultiplied rgba, so this is a straight copy.
const BLIT_WGSL: &str = r#"
struct VsOut { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    var xy = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    let p = xy[i];
    var o: VsOut;
    o.clip = vec4<f32>(p, 0.0, 1.0);
    // default framebuffer (fbo 0) is bottom-origin, so DON'T apply the usual
    // top-left flip: clip.y=+1 maps to the window bottom and should sample the
    // offscreen bottom (uv.y=1) - i.e. uv.y rises with clip.y.
    o.uv = vec2<f32>((p.x + 1.0) * 0.5, (p.y + 1.0) * 0.5);
    return o;
}
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
// linear -> sRGB. The GL default framebuffer (fbo 0) is NOT sRGB-capable here, so
// wgpu won't encode on write; without this every pixel lands ~half-bright (opaque
// text then reads as "faded/transparent"). Encode manually and write to a non-sRGB
// target so there's no double conversion. rgb is premultiplied; encode per-channel.
fn lin2srgb(c: vec3<f32>) -> vec3<f32> {
    let cl = max(c, vec3<f32>(0.0));
    let lo = cl * 12.92;
    let hi = 1.055 * pow(cl, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, cl <= vec3<f32>(0.0031308));
}
// cheap per-pixel hash for ordered dithering
fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}
@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(t, s, in.uv);
    // TPDF dither (~1 LSB) before the 8-bit fbo write breaks gradient banding
    // (the offscreen is high-precision linear; the final framebuffer is 8-bit).
    let p = in.clip.xy;
    let d = (hash12(p) - hash12(p + vec2<f32>(13.7, 91.3))) / 255.0;
    return vec4<f32>(lin2srgb(c.rgb) + vec3<f32>(d), c.a);
}
"#;
