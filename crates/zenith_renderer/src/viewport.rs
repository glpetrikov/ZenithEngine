use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(target_arch = "wasm32")]
use web_time::Instant;
use zenith_error::{ZResult, ZenithError};

use crate::{camera::CameraInput, pick_surface_format, renderer::Renderer, types::PowerMode};

/// A window to draw the [`Renderer`] into.
pub struct Viewport {
	window: Arc<winit::window::Window>,

	device: wgpu::Device,
	surface: wgpu::Surface<'static>,
	queue: wgpu::Queue,
	config: wgpu::SurfaceConfiguration,
	view_format: wgpu::TextureFormat,
	is_surface_configured: bool,

	renderer: Renderer,

	base_title: String,
	frame_count: u32,
	fps_timer: Instant,
}

impl Viewport {
	/// ## Creates a new Viewport
	///
	/// ## Errors
	/// return error if surface creation failed.
	///
	/// return error if adapter creation failed.
	///
	/// return error if device & queue getting failed.
	pub async fn new(
		label: &str,
		width: u32,
		height: u32,
		window: Arc<winit::window::Window>,
		power_mode: PowerMode,
		fallback_to_cpu: bool,
		project: &zenith_filesystem::Project,
	) -> ZResult<Self> {
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
			#[cfg(not(target_arch = "wasm32"))]
			backends: wgpu::Backends::PRIMARY,
			#[cfg(target_arch = "wasm32")]
			backends: wgpu::Backends::BROWSER_WEBGPU,
			flags: wgpu::InstanceFlags::from_build_config().with_env(),
			memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
			backend_options: wgpu::BackendOptions::default(),
			display: None,
		});

		let surface = match instance.create_surface(window.clone()) {
			Ok(surface) => surface,
			Err(e) => return Err(ZenithError::SurfaceCreationFailed(e.to_string())),
		};

		let adapter = match instance
			.request_adapter(&wgpu::RequestAdapterOptions {
				power_preference: match power_mode {
					PowerMode::Economy => wgpu::PowerPreference::LowPower,
					PowerMode::Performance => wgpu::PowerPreference::HighPerformance,
				},
				compatible_surface: Some(&surface),
				force_fallback_adapter: fallback_to_cpu,
				apply_limit_buckets: true,
			})
			.await
		{
			Ok(adapter) => adapter,
			Err(e) => return Err(ZenithError::RequestAdapterError(e.to_string())),
		};

		let info = adapter.get_info();
		zenith_log::info!("GPU: {} ({:?})", info.name, info.device_type);
		zenith_log::info!("Backend: {:?}", info.backend);
		zenith_log::info!("Driver: {} {}", info.driver, info.driver_info);
		zenith_log::info!("Vendor: {:#x}, device: {:#x}", info.vendor, info.device);

		let wanted = wgpu::Features::all_webgpu_mask();
		let missing = wanted - adapter.features();
		if !missing.is_empty() {
			zenith_log::info!("adapter lacks features: {missing:?}");
		}
		let required_features = wanted & adapter.features();

		let base_limits = wgpu::Limits::default();

		let (device, queue) = match adapter
			.request_device(&wgpu::DeviceDescriptor {
				label: Some(label),
				required_features,
				required_limits: base_limits.using_resolution(adapter.limits()),
				experimental_features: wgpu::ExperimentalFeatures::default(),
				memory_hints: wgpu::MemoryHints::Performance,
				trace: wgpu::Trace::Off,
			})
			.await
		{
			Ok((device, queue)) => (device, queue),
			Err(e) => return Err(ZenithError::RequestDeviceError(e.to_string())),
		};

		let surface_caps = surface.get_capabilities(&adapter);

		let formats = pick_surface_format(&surface_caps)
			.ok_or_else(|| ZenithError::NoSuitableSurfaceFormat("no supported surface format".into()))?;

		let config = wgpu::SurfaceConfiguration {
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			format: formats.storage,
			width: width.max(1),
			height: height.max(1),
			present_mode: wgpu::PresentMode::AutoNoVsync,
			alpha_mode: crate::pick_alpha_mode(&surface_caps, true),
			view_formats: if formats.view == formats.storage {
				vec![]
			} else {
				vec![formats.view]
			},
			desired_maximum_frame_latency: 3,
			color_space: wgpu::SurfaceColorSpace::Auto,
		};

		surface.configure(&device, &config);

		let renderer = Renderer::new(
			&device,
			&queue,
			formats.view,
			config.width,
			config.height,
			project,
			label,
		)?;

		Ok(Self {
			window,

			device,
			surface,
			queue,
			config,

			view_format: formats.view,
			is_surface_configured: true,

			renderer,

			base_title: "Zenith Engine".to_string(),
			fps_timer: Instant::now(),
			frame_count: 0,
		})
	}

	/// Moves the camera by what the player asked in `dt` seconds, and goes on
	/// with the scene.
	pub fn update(&mut self, dt: f32, input: &CameraInput) {
		self.renderer.update(dt, input);
	}

	pub const fn renderer_mut(&mut self) -> &mut Renderer {
		&mut self.renderer
	}

	pub fn resize(&mut self, width: u32, height: u32) {
		if width == 0 || height == 0 {
			// A hidden window has no size to draw to; `Resized` with the real
			// size comes back when it is shown again.
			self.is_surface_configured = false;
			return;
		}

		let max = self.device.limits().max_texture_dimension_2d;
		self.config.width = width.min(max);
		self.config.height = height.min(max);

		self.surface.configure(&self.device, &self.config);
		self.renderer.resize(self.config.width, self.config.height);
		self.is_surface_configured = true;
	}

	/// ## Render clear color
	///
	/// ## Errors
	/// Return Error if Device lost.
	pub fn render(&mut self, encoder_label: &str, render_pass_label: &str) -> ZResult<()> {
		if !self.is_surface_configured {
			return Ok(()); // TODO: return error, and handle it
		}

		let output = match self.surface.get_current_texture() {
			wgpu::CurrentSurfaceTexture::Success(surface_texture)
			| wgpu::CurrentSurfaceTexture::Suboptimal(surface_texture) => surface_texture,

			wgpu::CurrentSurfaceTexture::Timeout
			| wgpu::CurrentSurfaceTexture::Occluded
			| wgpu::CurrentSurfaceTexture::Validation => {
				return Ok(());
			}

			wgpu::CurrentSurfaceTexture::Outdated => {
				// Configuring a surface of a window with no size is an error.
				let size = self.window.inner_size();
				if size.width == 0 || size.height == 0 {
					self.is_surface_configured = false;
				} else {
					self.surface.configure(&self.device, &self.config);
				}
				return Ok(());
			}

			wgpu::CurrentSurfaceTexture::Lost => {
				return Err(ZenithError::LostDevice("Lost device".to_string()));
			}
		};

		let view = output.texture.create_view(&wgpu::TextureViewDescriptor {
			format: Some(self.view_format),
			..Default::default()
		});
		let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
			label: Some(encoder_label),
		});

		self.renderer.render(&mut encoder, &view, render_pass_label);

		self.queue.submit(std::iter::once(encoder.finish()));
		self.queue.present(output);

		self.frame_count += 1;
		let elapsed = self.fps_timer.elapsed();

		if elapsed.as_secs_f32() >= 1.0 {
			let fps = self.frame_count as f32 / elapsed.as_secs_f32();
			self.window.set_title(&format!("{} — {:.0} FPS", self.base_title, fps));

			self.frame_count = 0;
			self.fps_timer = Instant::now();
		}

		Ok(())
	}
}
