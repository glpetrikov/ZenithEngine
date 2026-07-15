mod backend;
pub mod components;
pub mod editor_camera_system;
pub mod render_system;

use std::sync::Arc;

pub use components::*;
pub use editor_camera_system::*;
pub use render_system::*;
use wgpu::util::DeviceExt;
use ze_assets::{AssetRef, ResourceManager};
use ze_core::{Mat4, Vec3};
use ze_ui::ui_manager::UiManagerHandle;

use crate::backend::{
	mesh::{Mesh, Vertex},
	pipeline::{self, Pipeline},
	texture::{SpriteMaterial, SpriteMaterialUniform, TextureCache},
	ubo::{Ubo, UboGroup},
};

const VIEWPORT_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositeMode {
	Standalone,
	Editor,
}

pub struct Renderer {
	// GPU resources — dropped first while the device is still alive.
	viewport_texture: wgpu::Texture,
	viewport_view: wgpu::TextureView,
	viewport_depth_texture: wgpu::Texture,
	viewport_depth_view: wgpu::TextureView,
	pipeline: Pipeline,
	debug_pipeline: Pipeline,
	blit_pipeline: Pipeline,
	quad_mesh: Mesh,
	texture_cache: TextureCache,
	materials: Vec<SpriteMaterial>,
	material_bind_group_layout: wgpu::BindGroupLayout,
	ubo_bind_group_layout: wgpu::BindGroupLayout,
	blit_bind_group_layout: wgpu::BindGroupLayout,
	blit_sampler: wgpu::Sampler,
	blit_bind_group: wgpu::BindGroup,
	ubo: Option<UboGroup>,
	projection_ubo: Option<Ubo>,
	ui_manager: Option<UiManagerHandle>,
	// Core wgpu objects — dropped last in the correct order:
	// queue → device → surface.
	queue: wgpu::Queue,
	device: wgpu::Device,
	surface: wgpu::Surface<'static>,
	// Pure data — order does not matter.
	config: wgpu::SurfaceConfiguration,
	size: winit::dpi::PhysicalSize<u32>,
	composite_mode: CompositeMode,
	viewport_size: winit::dpi::PhysicalSize<u32>,
	viewport_generation: u64,
	ubo_object_count: usize,
}
impl Drop for Renderer {
	fn drop(&mut self) { self.device.poll(wgpu::PollType::wait_indefinitely()).ok(); }
}
impl Renderer {
	pub async fn new(window: Arc<winit::window::Window>) -> ze_core::Result<Self> {
		ze_log::info!("Creating renderer");
		let instance = wgpu::Instance::default();

		let surface = instance.create_surface(window.clone())?;

		let adapter_descriptor = wgpu::RequestAdapterOptionsBase {
			power_preference: wgpu::PowerPreference::HighPerformance,
			compatible_surface: Some(&surface),
			force_fallback_adapter: false,
		};

		let adapter = instance.request_adapter(&adapter_descriptor).await?;

		let info = adapter.get_info();

		println!("Renderer:");
		println!("-> Vendor: {}", info.driver);
		println!("-> Name: {}", info.name);
		println!("-> Backend: {}", info.backend);
		println!("-> Driver: {} {}", info.driver, info.driver_info);

		let device_descriptor = wgpu::DeviceDescriptor {
			required_features: wgpu::Features::POLYGON_MODE_LINE,
			required_limits: wgpu::Limits::default(),
			label: Some("Device"),
			memory_hints: wgpu::MemoryHints::Performance,
			trace: wgpu::Trace::Off,
			experimental_features: wgpu::ExperimentalFeatures::disabled(),
		};

		let (device, queue) = adapter.request_device(&device_descriptor).await?;

		let size = window.inner_size();

		let surface_capabilities = surface.get_capabilities(&adapter);
		let surface_format = surface_capabilities
			.formats
			.iter()
			.copied()
			.find(wgpu::TextureFormat::is_srgb)
			.unwrap_or(surface_capabilities.formats[0]);

		let config = wgpu::SurfaceConfiguration {
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			format: surface_format,
			width: size.width.max(1),
			height: size.height.max(1),
			present_mode: wgpu::PresentMode::AutoVsync,
			alpha_mode: surface_capabilities.alpha_modes[0],
			view_formats: vec![],
			desired_maximum_frame_latency: 2,
		};

		surface.configure(&device, &config);

		let viewport_size = winit::dpi::PhysicalSize::new(config.width, config.height);
		let (viewport_texture, viewport_view) = Self::create_viewport_texture(&device, viewport_size);
		let (viewport_depth_texture, viewport_depth_view) = Self::create_depth_texture(&device, viewport_size);

		let quad_mesh = Vertex::make_quad(&device);

		let material_bind_group_layout = Self::create_material_bind_group_layout(&device);
		let ubo_bind_group_layout = Self::create_ubo_bind_group_layout(&device);
		let blit_bind_group_layout = Self::create_blit_bind_group_layout(&device);
		let blit_sampler = Self::create_blit_sampler(&device);
		let blit_bind_group =
			Self::create_blit_bind_group(&device, &blit_bind_group_layout, &viewport_view, &blit_sampler);

		let render_pipeline =
			Self::create_scene_pipeline(&device, &material_bind_group_layout, &ubo_bind_group_layout)?;
		let debug_pipeline = Self::create_debug_pipeline(&device, &ubo_bind_group_layout)?;
		let blit_pipeline = Self::create_blit_pipeline(&device, surface_format, &blit_bind_group_layout)?;

		let projection_ubo = Some(Ubo::new(&device, &ubo_bind_group_layout));
		let texture_cache = TextureCache::new(&device, &queue);
		let materials = Vec::new();

		Ok(Self {
			viewport_texture,
			viewport_view,
			viewport_depth_texture,
			viewport_depth_view,
			pipeline: render_pipeline,
			debug_pipeline,
			blit_pipeline,
			quad_mesh,
			texture_cache,
			materials,
			material_bind_group_layout,
			ubo_bind_group_layout,
			blit_bind_group_layout,
			blit_sampler,
			blit_bind_group,
			ubo: None,
			projection_ubo,
			ui_manager: None,
			queue,
			device,
			surface,
			config,
			size,
			composite_mode: CompositeMode::Standalone,
			viewport_size,
			viewport_generation: 0,
			ubo_object_count: 0,
		})
	}

	pub const fn set_composite_mode(&mut self, mode: CompositeMode) { self.composite_mode = mode; }

	pub const fn composite_mode(&self) -> CompositeMode { self.composite_mode }

	pub const fn surface_size(&self) -> winit::dpi::PhysicalSize<u32> { self.size }

	pub const fn surface_format(&self) -> wgpu::TextureFormat { self.config.format }

	pub const fn viewport_size(&self) -> winit::dpi::PhysicalSize<u32> { self.viewport_size }

	pub const fn viewport_generation(&self) -> u64 { self.viewport_generation }

	pub const fn viewport_texture_view(&self) -> &wgpu::TextureView { &self.viewport_view }

	pub const fn device(&self) -> &wgpu::Device { &self.device }

	pub const fn queue(&self) -> &wgpu::Queue { &self.queue }

	pub fn invalidate_textures<'a>(&mut self, assets: impl IntoIterator<Item = &'a AssetRef>) -> usize {
		self.texture_cache.invalidate_many(assets)
	}

	pub fn set_ui_manager(&mut self, handle: UiManagerHandle) {
		handle
			.borrow_mut()
			.set_viewport_size(self.viewport_size.width as f32, self.viewport_size.height as f32);
		self.ui_manager = Some(handle);
	}

	pub fn flush_game_pass(&self) {
		if let Err(error) = self.device.poll(wgpu::PollType::Poll) {
			ze_log::warn!("failed to poll renderer device after game pass: {error:?}");
		}
	}

	pub fn build_ubos_for_objects(&mut self, objects_count: usize) {
		let objects_count = objects_count.max(1);
		self.ubo = Some(UboGroup::new(&self.device, objects_count, &self.ubo_bind_group_layout));
		self.ubo_object_count = objects_count;
	}

	fn ensure_ubos_for_objects(&mut self, objects_count: usize) {
		let objects_count = objects_count.max(1);
		if self.ubo.is_none() || self.ubo_object_count != objects_count {
			self.build_ubos_for_objects(objects_count);
		}
	}

	pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
		if new_size.width == 0 || new_size.height == 0 {
			return;
		}

		self.size = new_size;
		self.config.width = new_size.width;
		self.config.height = new_size.height;
		self.reconfigure_surface("window resize");

		if self.composite_mode == CompositeMode::Standalone {
			self.resize_viewport(new_size);
		}
	}

	pub fn set_viewport_size(&mut self, width: u32, height: u32) {
		let new_size = winit::dpi::PhysicalSize::new(width.max(1), height.max(1));
		self.resize_viewport(new_size);
	}

	fn resize_viewport(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
		if self.viewport_size == new_size {
			return;
		}

		let (viewport_texture, viewport_view) = Self::create_viewport_texture(&self.device, new_size);
		let (viewport_depth_texture, viewport_depth_view) = Self::create_depth_texture(&self.device, new_size);
		let blit_bind_group = Self::create_blit_bind_group(
			&self.device,
			&self.blit_bind_group_layout,
			&viewport_view,
			&self.blit_sampler,
		);

		self.viewport_texture = viewport_texture;
		self.viewport_view = viewport_view;
		self.viewport_depth_texture = viewport_depth_texture;
		self.viewport_depth_view = viewport_depth_view;
		self.viewport_size = new_size;
		self.viewport_generation = self.viewport_generation.wrapping_add(1);
		self.blit_bind_group = blit_bind_group;

		if let Some(ref handle) = self.ui_manager {
			handle
				.borrow_mut()
				.set_viewport_size(new_size.width as f32, new_size.height as f32);
		}
	}

	fn reconfigure_surface(&self, reason: &str) {
		if self.config.width == 0 || self.config.height == 0 {
			ze_log::warn!("cannot reconfigure zero-sized renderer surface after {reason}");
			return;
		}

		self.surface.configure(&self.device, &self.config);
	}

	fn create_material_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
		let mut builder = backend::bind_group_layout::Builder::new(device);
		builder.add_material();
		builder.build("Material Bind Group Layout")
	}

	fn create_ubo_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
		let mut builder = backend::bind_group_layout::Builder::new(device);
		builder.add_ubo();
		builder.build("UBO Bind Group Layout")
	}

	fn create_blit_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
		device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("Viewport Blit Bind Group Layout"),
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
		})
	}

	fn create_blit_sampler(device: &wgpu::Device) -> wgpu::Sampler {
		device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some("Viewport Blit Sampler"),
			address_mode_u: wgpu::AddressMode::ClampToEdge,
			address_mode_v: wgpu::AddressMode::ClampToEdge,
			address_mode_w: wgpu::AddressMode::ClampToEdge,
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			mipmap_filter: wgpu::MipmapFilterMode::Nearest,
			..wgpu::SamplerDescriptor::default()
		})
	}

	fn create_blit_bind_group(
		device: &wgpu::Device,
		layout: &wgpu::BindGroupLayout,
		viewport_view: &wgpu::TextureView,
		sampler: &wgpu::Sampler,
	) -> wgpu::BindGroup {
		device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("Viewport Blit Bind Group"),
			layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(viewport_view),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::Sampler(sampler),
				},
			],
		})
	}

	fn create_scene_pipeline(
		device: &wgpu::Device,
		material_layout: &wgpu::BindGroupLayout,
		ubo_layout: &wgpu::BindGroupLayout,
	) -> ze_core::Result<Pipeline> {
		let resources = ResourceManager::new("Sandbox/assets");
		let shader_source = match resources.game_string("shaders/sprite.wgsl") {
			Ok(source) => source,
			Err(_) => resources.engine_string("shaders/engine/sprite.wgsl")?.to_owned(),
		};
		pipeline::Builder::new(device)
			.with_shader_source(shader_source)
			.with_pixel_format(VIEWPORT_TEXTURE_FORMAT)
			.with_vertex_buffer_layout(Vertex::get_layout())
			.with_depth_write_enabled(false)
			.with_depth_compare(wgpu::CompareFunction::Always)
			.with_bind_group_layout(material_layout)
			.with_bind_group_layout(ubo_layout)
			.with_bind_group_layout(ubo_layout)
			.build()
	}

	fn create_debug_pipeline(device: &wgpu::Device, ubo_layout: &wgpu::BindGroupLayout) -> ze_core::Result<Pipeline> {
		pipeline::Builder::new(device)
			.with_name("Physics Debug Draw")
			.with_shader_source(DEBUG_LINE_SHADER)
			.with_pixel_format(VIEWPORT_TEXTURE_FORMAT)
			.with_vertex_buffer_layout(Vertex::get_layout())
			.with_bind_group_layout(ubo_layout)
			.with_topology(wgpu::PrimitiveTopology::LineList)
			.with_polygon_mode(wgpu::PolygonMode::Line)
			.with_cull_mode(None)
			.with_depth_write_enabled(false)
			.with_depth_compare(wgpu::CompareFunction::Always)
			.build()
	}

	fn create_blit_pipeline(
		device: &wgpu::Device,
		surface_format: wgpu::TextureFormat,
		blit_layout: &wgpu::BindGroupLayout,
	) -> ze_core::Result<Pipeline> {
		pipeline::Builder::new(device)
			.with_name("Viewport Blit")
			.with_shader_source(VIEWPORT_BLIT_SHADER)
			.with_pixel_format(surface_format)
			.with_bind_group_layout(blit_layout)
			.with_depth_stencil_enabled(false)
			.build()
	}

	fn create_viewport_texture(
		device: &wgpu::Device,
		size: winit::dpi::PhysicalSize<u32>,
	) -> (wgpu::Texture, wgpu::TextureView) {
		let texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("Viewport Texture"),
			size: wgpu::Extent3d {
				width: size.width.max(1),
				height: size.height.max(1),
				depth_or_array_layers: 1,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: VIEWPORT_TEXTURE_FORMAT,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[VIEWPORT_TEXTURE_FORMAT.add_srgb_suffix()],
		});

		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		(texture, view)
	}

	fn create_depth_texture(
		device: &wgpu::Device,
		size: winit::dpi::PhysicalSize<u32>,
	) -> (wgpu::Texture, wgpu::TextureView) {
		let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("Viewport Depth Texture"),
			size: wgpu::Extent3d {
				width: size.width.max(1),
				height: size.height.max(1),
				depth_or_array_layers: 1,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Depth32Float,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			view_formats: &[],
		});

		let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());
		(depth_texture, depth_view)
	}

	pub fn aspect_ratio(&self) -> f32 { self.viewport_size.width as f32 / self.viewport_size.height.max(1) as f32 }

	pub fn present_editor<F>(&mut self, draw_editor: F)
	where
		F: FnOnce(
			&wgpu::Device,
			&wgpu::Queue,
			&mut wgpu::CommandEncoder,
			&wgpu::TextureView,
		) -> Vec<wgpu::CommandBuffer>,
	{
		if self.composite_mode != CompositeMode::Editor {
			return;
		}

		let Some((drawable, surface_view)) = self.acquire_surface_view() else {
			return;
		};

		let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
			label: Some("Editor Composite Encoder"),
		});
		let extra_command_buffers = draw_editor(&self.device, &self.queue, &mut encoder, &surface_view);
		self.queue.submit(
			extra_command_buffers
				.into_iter()
				.chain(std::iter::once(encoder.finish())),
		);
		drawable.present();
	}

	pub fn present_viewport(&mut self) {
		let Some((drawable, surface_view)) = self.acquire_surface_view() else {
			return;
		};

		let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
			label: Some("Viewport Present Encoder"),
		});

		{
			let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("Viewport Blit Pass"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &surface_view,
					resolve_target: None,
					depth_slice: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Clear(wgpu::Color {
							r: 0.0,
							g: 0.0,
							b: 0.0,
							a: 0.0,
						}),
						store: wgpu::StoreOp::Store,
					},
				})],
				depth_stencil_attachment: None,
				occlusion_query_set: None,
				timestamp_writes: None,
				multiview_mask: None,
			});

			render_pass.set_pipeline(&self.blit_pipeline.render_pipeline);
			render_pass.set_bind_group(0, &self.blit_bind_group, &[]);
			render_pass.draw(0..3, 0..1);
		}

		self.queue.submit(std::iter::once(encoder.finish()));
		drawable.present();
	}

	fn acquire_surface_view(&self) -> Option<(wgpu::SurfaceTexture, wgpu::TextureView)> {
		match self.surface.get_current_texture() {
			wgpu::CurrentSurfaceTexture::Success(drawable) => {
				let view = drawable.texture.create_view(&wgpu::TextureViewDescriptor::default());
				Some((drawable, view))
			}
			wgpu::CurrentSurfaceTexture::Suboptimal(drawable) => {
				ze_log::warn!("acquired suboptimal renderer surface texture; presentation will continue");
				let view = drawable.texture.create_view(&wgpu::TextureViewDescriptor::default());
				Some((drawable, view))
			}
			wgpu::CurrentSurfaceTexture::Outdated => {
				ze_log::warn!("renderer surface is outdated; reconfiguring");
				self.reconfigure_surface("outdated surface");
				self.acquire_surface_view_after_reconfigure()
			}
			wgpu::CurrentSurfaceTexture::Lost => {
				ze_log::warn!("renderer surface was lost; reconfiguring");
				self.reconfigure_surface("lost surface");
				self.acquire_surface_view_after_reconfigure()
			}
			wgpu::CurrentSurfaceTexture::Timeout => {
				ze_log::warn!("timed out acquiring renderer surface texture");
				None
			}
			wgpu::CurrentSurfaceTexture::Occluded => None,
			wgpu::CurrentSurfaceTexture::Validation => {
				ze_log::error!("validation error while acquiring renderer surface texture");
				None
			}
		}
	}

	fn acquire_surface_view_after_reconfigure(&self) -> Option<(wgpu::SurfaceTexture, wgpu::TextureView)> {
		match self.surface.get_current_texture() {
			wgpu::CurrentSurfaceTexture::Success(drawable) | wgpu::CurrentSurfaceTexture::Suboptimal(drawable) => {
				let view = drawable.texture.create_view(&wgpu::TextureViewDescriptor::default());
				Some((drawable, view))
			}
			wgpu::CurrentSurfaceTexture::Timeout => {
				ze_log::warn!("timed out acquiring renderer surface texture after reconfigure");
				None
			}
			wgpu::CurrentSurfaceTexture::Occluded => None,
			wgpu::CurrentSurfaceTexture::Outdated => {
				ze_log::warn!("renderer surface is still outdated after reconfigure");
				None
			}
			wgpu::CurrentSurfaceTexture::Lost => {
				ze_log::warn!("renderer surface is still lost after reconfigure");
				None
			}
			wgpu::CurrentSurfaceTexture::Validation => {
				ze_log::error!("validation error acquiring renderer surface texture after reconfigure");
				None
			}
		}
	}

	fn update_projection(&mut self, camera: &render_system::CameraRenderData) -> bool {
		let Some(projection_ubo) = self.projection_ubo.as_mut() else {
			ze_log::error!("cannot render viewport without projection UBO");
			return false;
		};

		projection_ubo.upload(&camera.view_projection, &self.queue);
		true
	}

	pub fn request_sprite_redraw(
		&mut self,
		items: &[render_system::SpriteRenderItem],
		debug_lines: &[render_system::DebugLine],
		camera: &render_system::CameraRenderData,
		resources: &ResourceManager,
	) {
		self.render_sprite_items_to_viewport(items, debug_lines, camera, resources);

		// Paint yakui UI overlay on top of sprites
		if let Some(ref handle) = self.ui_manager {
			let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
				label: Some("UI Overlay Encoder"),
			});
			handle
				.borrow_mut()
				.paint(&mut encoder, &self.viewport_view, VIEWPORT_TEXTURE_FORMAT, 1);
			self.queue.submit(std::iter::once(encoder.finish()));
		}

		if self.composite_mode == CompositeMode::Standalone {
			self.present_viewport();
		}
	}

	fn render_sprite_items_to_viewport(
		&mut self,
		items: &[render_system::SpriteRenderItem],
		debug_lines: &[render_system::DebugLine],
		camera: &render_system::CameraRenderData,
		resources: &ResourceManager,
	) {
		self.ensure_ubos_for_objects(items.len());
		if !self.update_projection(camera) {
			return;
		}

		self.materials.clear();
		self.materials
			.reserve(items.len().saturating_sub(self.materials.capacity()));

		for (i, item) in items.iter().enumerate() {
			let texture = self
				.texture_cache
				.get_or_load(&item.texture, resources, &self.device, &self.queue);
			let sprite_size = sprite_size_to_world_scale(&item.size, texture.dimensions());
			let transform = item.transform * Mat4::from_scale(Vec3::new(sprite_size[0], sprite_size[1], 1.0));

			let Some(ubo) = self.ubo.as_mut() else {
				return;
			};
			ubo.upload(i as u64, &transform, &self.queue);

			let tint = item.color.tint.unwrap_or([1.0, 1.0, 1.0, 1.0]);
			let mode = match item.color.mode {
				components::SpriteColorMode::None => 0.0,
				components::SpriteColorMode::Multiply => 1.0,
				components::SpriteColorMode::GrayscaleTint => 2.0,
			};
			let uniform = SpriteMaterialUniform {
				tint,
				params: [
					mode,
					item.color.strength,
					item.color.saturation_threshold,
					item.texture_rotation_degrees.to_radians(),
				],
			};

			self.materials.push(SpriteMaterial::new(
				"Sprite Material",
				texture,
				uniform,
				&self.device,
				&self.material_bind_group_layout,
			));
		}

		let debug_vertices = debug_lines
			.iter()
			.flat_map(|line| {
				[
					Vertex {
						position: line.start.to_array(),
						color: line.color,
					},
					Vertex {
						position: line.end.to_array(),
						color: line.color,
					},
				]
			})
			.collect::<Vec<_>>();
		let debug_vertex_buffer = (!debug_vertices.is_empty()).then(|| {
			self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
				label: Some("Physics Debug Lines"),
				contents: bytemuck::cast_slice(&debug_vertices),
				usage: wgpu::BufferUsages::VERTEX,
			})
		});

		let mut command_encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
			label: Some("Viewport Render Encoder"),
		});

		let mut render_pass = command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("Viewport Render Pass"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: &self.viewport_view,
				resolve_target: None,
				depth_slice: None,
				ops: wgpu::Operations {
					load: wgpu::LoadOp::Clear(wgpu::Color {
						r: f64::from(camera.clear_color[0]),
						g: f64::from(camera.clear_color[1]),
						b: f64::from(camera.clear_color[2]),
						a: f64::from(camera.clear_color[3]),
					}),
					store: wgpu::StoreOp::Store,
				},
			})],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
				view: &self.viewport_depth_view,
				depth_ops: Some(wgpu::Operations {
					load: wgpu::LoadOp::Clear(1.0),
					store: wgpu::StoreOp::Store,
				}),
				stencil_ops: None,
			}),
			occlusion_query_set: None,
			timestamp_writes: None,
			multiview_mask: None,
		});
		let viewport_width = self.viewport_size.width.max(1);
		let viewport_height = self.viewport_size.height.max(1);
		render_pass.set_viewport(0.0, 0.0, viewport_width as f32, viewport_height as f32, 0.0, 1.0);
		render_pass.set_scissor_rect(0, 0, viewport_width, viewport_height);
		render_pass.set_pipeline(&self.pipeline.render_pipeline);
		let Some(projection_ubo) = self.projection_ubo.as_ref() else {
			return;
		};
		render_pass.set_bind_group(2, &projection_ubo.bind_group, &[]);

		for (i, material) in self.materials.iter().enumerate() {
			let Some(ubo) = self.ubo.as_ref() else {
				ze_log::error!("cannot render sprite without object UBOs");
				return;
			};
			let Some(object_bind_group) = ubo.bind_groups.get(i) else {
				ze_log::error!("missing object UBO bind group for sprite {i}");
				return;
			};

			render_pass.set_bind_group(0, &material.bind_group, &[]);
			render_pass.set_bind_group(1, object_bind_group, &[]);

			render_pass.set_vertex_buffer(0, self.quad_mesh.buffer.slice(..self.quad_mesh.offset));
			render_pass.set_index_buffer(
				self.quad_mesh.buffer.slice(self.quad_mesh.offset..),
				wgpu::IndexFormat::Uint16,
			);
			render_pass.draw_indexed(0..6, 0, 0..1);
		}

		if let Some(debug_vertex_buffer) = &debug_vertex_buffer {
			render_pass.set_pipeline(&self.debug_pipeline.render_pipeline);
			render_pass.set_bind_group(0, &projection_ubo.bind_group, &[]);
			render_pass.set_vertex_buffer(0, debug_vertex_buffer.slice(..));
			render_pass.draw(0..debug_vertices.len() as u32, 0..1);
		}

		drop(render_pass);
		self.queue.submit(std::iter::once(command_encoder.finish()));
	}
}

const DEBUG_LINE_SHADER: &str = r"
@group(0) @binding(0) var<uniform> view_projection: mat4x4<f32>;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    out.position = view_projection * vec4<f32>(vertex.position, 1.0);
    out.color = vertex.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
";

const VIEWPORT_BLIT_SHADER: &str = r"
@group(0) @binding(0) var viewport_texture: texture_2d<f32>;
@group(0) @binding(1) var viewport_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let pos = positions[vertex_index];
    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.uv = vec2<f32>(pos.x * 0.5 + 0.5, 1.0 - (pos.y * 0.5 + 0.5));
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(viewport_texture, viewport_sampler, in.uv);
}
";

fn sprite_size_to_world_scale(size: &components::SpriteSize, dimensions: (u32, u32)) -> [f32; 2] {
	match size {
		components::SpriteSize::Auto => auto_sprite_size(dimensions.0, dimensions.1),
		components::SpriteSize::Custom { width, height } => [*width, *height],
	}
}

fn auto_sprite_size(width: u32, height: u32) -> [f32; 2] {
	if width == 0 || height == 0 {
		return [1.0, 1.0];
	}

	if width >= height {
		[width as f32 / height as f32, 1.0]
	} else {
		[1.0, height as f32 / width as f32]
	}
}
