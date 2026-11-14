//! `Renderer`: the device, the queue and the surface; the start and the end of
//! a frame.

use bevy_ecs::resource::Resource;
use zenith_error::{ZResult, ZenithError};

use crate::{
	settings::RenderSettings,
	surface::{Acquired, SurfaceFrame, SurfaceState},
};

/// The GPU and what the frames go to. A resource of the world.
#[derive(Resource)]
pub struct Renderer {
	/// `None` when the device came from elsewhere ([`Renderer::from_device`]):
	/// then no surface can be made.
	instance: Option<wgpu::Instance>,
	adapter: Option<wgpu::Adapter>,
	device: wgpu::Device,
	queue: wgpu::Queue,
	surface: Option<SurfaceState>,
	/// The settings the GPU was opened with and the surface shows its frames
	/// by.
	settings: RenderSettings,
	output_format: wgpu::TextureFormat,
	frame: u64,
	/// So a lost surface is reported once, not every frame.
	reported_lost: bool,
}

impl Renderer {
	/// ## Opens the GPU and the surface of `target` (a window, a canvas)
	///
	/// `width` and `height` are the size of the target in pixels. On the web
	/// this has to be awaited by the browser: there is no blocking there.
	///
	/// ## Errors
	/// `SurfaceCreationFailed` if no surface can be made for `target`,
	/// `RequestAdapterError` if no GPU can draw on it, `RequestDeviceError` if
	/// the GPU refuses to open, `NoSuitableSurfaceFormat` if the surface has no
	/// format the renderer can draw in.
	pub async fn new(
		target: impl Into<wgpu::SurfaceTarget<'static>>,
		width: u32,
		height: u32,
		settings: &RenderSettings,
	) -> ZResult<Self> {
		let instance = wgpu::Instance::new(instance_descriptor(settings));
		let surface = instance
			.create_surface(target)
			.map_err(|e| ZenithError::SurfaceCreationFailed(e.to_string()))?;
		let adapter = instance
			.request_adapter(&wgpu::RequestAdapterOptions {
				power_preference: settings.power_preference(),
				compatible_surface: Some(&surface),
				force_fallback_adapter: settings.fallback_adapter,
				apply_limit_buckets: true,
			})
			.await
			.map_err(|e| ZenithError::RequestAdapterError(e.to_string()))?;
		log_adapter(&adapter.get_info());
		let (device, queue) = request_device(&adapter, settings).await?;
		let surface = SurfaceState::new(surface, &adapter, &device, width, height, settings)
			.ok_or_else(|| ZenithError::NoSuitableSurfaceFormat("the surface has no 8-bit color format".into()))?;

		Ok(Self {
			output_format: surface.view_format(),
			instance: Some(instance),
			adapter: Some(adapter),
			device,
			queue,
			surface: Some(surface),
			settings: settings.clone(),
			frame: 0,
			reported_lost: false,
		})
	}

	/// A renderer on a device made elsewhere (the editor shares its own with
	/// the UI; a test has one without a window). It has no surface: it draws
	/// into textures, and `output_format` is the format of those. The settings
	/// that open the GPU mean nothing to it.
	pub fn from_device(device: wgpu::Device, queue: wgpu::Queue, output_format: wgpu::TextureFormat) -> Self {
		Self {
			instance: None,
			adapter: None,
			device,
			queue,
			surface: None,
			settings: RenderSettings::default(),
			output_format,
			frame: 0,
			reported_lost: false,
		}
	}

	/// The settings the renderer works by now.
	pub const fn settings(&self) -> &RenderSettings {
		&self.settings
	}

	/// Works by `settings` from now on: the surface shows its frames the new
	/// way at once. What opens the GPU (its power, the API, ...) cannot change
	/// while it is open: that waits for the next start, and the log says so.
	pub fn apply_settings(&mut self, settings: &RenderSettings) {
		if self.settings == *settings {
			return;
		}
		if self.instance.is_some() && !self.settings.same_device(settings) {
			zenith_log::info!("The new settings of the GPU take effect at the next start");
		}
		if let Some(surface) = &mut self.surface {
			surface.apply(&self.device, settings);
		}
		self.settings = settings.clone();
	}

	pub const fn device(&self) -> &wgpu::Device {
		&self.device
	}

	pub const fn queue(&self) -> &wgpu::Queue {
		&self.queue
	}

	/// What the GPU is, if the renderer opened it.
	pub fn adapter_info(&self) -> Option<wgpu::AdapterInfo> {
		self.adapter.as_ref().map(wgpu::Adapter::get_info)
	}

	/// The format of the frames: the sRGB view of the surface, or what
	/// [`Renderer::from_device`] was given.
	pub const fn output_format(&self) -> wgpu::TextureFormat {
		self.output_format
	}

	/// The size of the surface in pixels, if there is one.
	pub fn surface_size(&self) -> Option<(u32, u32)> {
		self.surface.as_ref().map(SurfaceState::size)
	}

	/// How many frames were begun.
	pub const fn frame(&self) -> u64 {
		self.frame
	}

	/// The window changed its size. A size of zero (a minimized window) stops
	/// the drawing until a real size comes.
	pub fn resize_surface(&mut self, width: u32, height: u32) {
		if let Some(surface) = &mut self.surface {
			surface.resize(&self.device, width, height);
		}
	}

	/// ## Draws into a new surface from now on
	///
	/// For a surface that was lost, or a window made again (Android makes a
	/// new one each time the app comes back).
	///
	/// ## Errors
	/// `SurfaceCreationFailed` if the renderer has no instance of its own
	/// ([`Renderer::from_device`]) or the surface cannot be made,
	/// `RequestAdapterError` if the GPU cannot draw on it,
	/// `NoSuitableSurfaceFormat` if it has no format the renderer can draw in.
	pub fn set_surface(
		&mut self,
		target: impl Into<wgpu::SurfaceTarget<'static>>,
		width: u32,
		height: u32,
	) -> ZResult<()> {
		let (Some(instance), Some(adapter)) = (&self.instance, &self.adapter) else {
			return Err(ZenithError::SurfaceCreationFailed(
				"this renderer was given its device and cannot make a surface".into(),
			));
		};
		let surface = instance
			.create_surface(target)
			.map_err(|e| ZenithError::SurfaceCreationFailed(e.to_string()))?;
		if !adapter.is_surface_supported(&surface) {
			return Err(ZenithError::RequestAdapterError(
				"the GPU of the renderer cannot draw on this surface".into(),
			));
		}
		let surface = SurfaceState::new(surface, adapter, &self.device, width, height, &self.settings)
			.ok_or_else(|| ZenithError::NoSuitableSurfaceFormat("the surface has no 8-bit color format".into()))?;
		if surface.view_format() != self.output_format {
			zenith_log::warn!(
				"The new surface draws in {:?}, the old one in {:?}",
				surface.view_format(),
				self.output_format
			);
			self.output_format = surface.view_format();
		}
		self.surface = Some(surface);
		self.reported_lost = false;
		Ok(())
	}

	/// Stops drawing into the surface (the window is going away).
	pub fn remove_surface(&mut self) {
		self.surface = None;
	}

	/// Counts a new frame and takes the texture of the surface to draw it
	/// into. `None`: there is nothing to draw into this frame.
	pub(crate) fn begin_frame(&mut self) -> Option<SurfaceFrame> {
		self.frame += 1;
		match self.surface.as_mut()?.acquire(&self.device) {
			Acquired::Frame(frame) => Some(frame),
			Acquired::Skip => None,
			Acquired::Lost => {
				if !self.reported_lost {
					zenith_log::error!("The surface was lost: nothing is drawn until a new one is set");
					self.reported_lost = true;
				}
				None
			}
		}
	}

	/// Shows a frame of [`Renderer::begin_frame`].
	pub(crate) fn present(&self, frame: SurfaceFrame) {
		if let Some(surface) = &self.surface {
			surface.present(&self.device, &self.queue, frame);
		}
	}
}

fn instance_descriptor(settings: &RenderSettings) -> wgpu::InstanceDescriptor {
	wgpu::InstanceDescriptor {
		backends: settings.backends(),
		flags: settings.instance_flags(),
		memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
		backend_options: wgpu::BackendOptions::default(),
		display: None,
	}
}

/// Opens the device with what WebGPU has and the adapter can give: the same
/// renderer has to run in a browser.
async fn request_device(adapter: &wgpu::Adapter, settings: &RenderSettings) -> ZResult<(wgpu::Device, wgpu::Queue)> {
	let wanted = wgpu::Features::all_webgpu_mask();
	let missing = wanted - adapter.features();
	if !missing.is_empty() {
		zenith_log::info!("The GPU lacks the features {missing:?}");
	}
	adapter
		.request_device(&wgpu::DeviceDescriptor {
			label: Some("Zenith Device"),
			required_features: wanted & adapter.features(),
			required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
			experimental_features: wgpu::ExperimentalFeatures::default(),
			memory_hints: settings.memory_hints(),
			trace: wgpu::Trace::Off,
		})
		.await
		.map_err(|e| ZenithError::RequestDeviceError(e.to_string()))
}

fn log_adapter(info: &wgpu::AdapterInfo) {
	zenith_log::info!("GPU: {} ({:?})", info.name, info.device_type);
	zenith_log::info!("Backend: {:?}", info.backend);
	zenith_log::info!("Driver: {} {}", info.driver, info.driver_info);
}
