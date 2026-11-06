//! The window, the GPU and Dear `ImGui`: what the editor draws on.

use std::{path::PathBuf, sync::Arc};

use dear_imgui_rs::{ConfigFlags, Context, FontSource, Ui};
use dear_imgui_wgpu::{FramebufferExtent, GammaMode, WgpuInitInfo, WgpuRenderer, wgpu};
use dear_imgui_winit::{HiDpiMode, WinitPlatform};
use winit::{
	dpi::{LogicalSize, PhysicalSize},
	event::WindowEvent,
	event_loop::ActiveEventLoop,
	window::Window,
};
use zenith_error::{ZInternalResult, eyre};

use crate::theme::zenith_theme;

const TITLE: &str = "Zenith Editor";
const START_SIZE: (f64, f64) = (1280.0, 720.0);
const CLEAR_COLOR: wgpu::Color = wgpu::Color {
	r: 0.1,
	g: 0.1,
	b: 0.1,
	a: 1.0,
};
const FONT_SIZE: f32 = 18.0;

pub struct Gfx {
	pub window: Arc<Window>,
	surface: wgpu::Surface<'static>,
	surface_config: wgpu::SurfaceConfiguration,
	device: wgpu::Device,
	queue: wgpu::Queue,
	context: Context,
	platform: WinitPlatform,
	renderer: WgpuRenderer,
}

impl Gfx {
	/// Opens the window and sets up the GPU and the UI on it. `layout_file` is
	/// where the arrangement of the panels is kept between runs.
	pub fn new(event_loop: &ActiveEventLoop, layout_file: Option<PathBuf>) -> ZInternalResult<Self> {
		let window = Arc::new(
			event_loop
				.create_window(
					Window::default_attributes()
						.with_title(TITLE)
						.with_inner_size(LogicalSize::new(START_SIZE.0, START_SIZE.1)),
				)
				.map_err(|e| eyre!("cannot open the window: {e}"))?,
		);

		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
		let surface = instance
			.create_surface(Arc::clone(&window))
			.map_err(|e| eyre!("cannot create the surface: {e}"))?;
		let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
			compatible_surface: Some(&surface),
			..Default::default()
		}))
		.map_err(|e| eyre!("no graphics adapter: {e}"))?;
		let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
			label: Some("Zenith Editor device"),
			..Default::default()
		}))
		.map_err(|e| eyre!("cannot open the graphics device: {e}"))?;

		let capabilities = surface.get_capabilities(&adapter);
		let format = [wgpu::TextureFormat::Bgra8UnormSrgb, wgpu::TextureFormat::Rgba8UnormSrgb]
			.into_iter()
			.find(|f| capabilities.formats.contains(f))
			.or_else(|| capabilities.formats.first().copied())
			.ok_or_else(|| eyre!("the surface supports no format"))?;
		let size = window.inner_size();
		let surface_config = wgpu::SurfaceConfiguration {
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			format,
			color_space: wgpu::SurfaceColorSpace::Auto,
			width: size.width.max(1),
			height: size.height.max(1),
			present_mode: wgpu::PresentMode::Fifo,
			alpha_mode: wgpu::CompositeAlphaMode::Auto,
			view_formats: vec![],
			desired_maximum_frame_latency: 2,
		};
		surface.configure(&device, &surface_config);

		let mut context = Context::create();
		if let Some(file) = layout_file {
			if let Some(folder) = file.parent() {
				let _ = std::fs::create_dir_all(folder);
			}
			let _ = context.set_ini_filename(Some(file));
		} else {
			let _ = context.set_ini_filename(None::<PathBuf>);
		}
		let flags = context.io().config_flags() | ConfigFlags::DOCKING_ENABLE;
		context.io_mut().set_config_flags(flags);
		zenith_theme().apply_to_context(&mut context);
		let font = include_bytes!("../assets/fonts/Inter-Regular.ttf");
		// SAFETY: the font data is `'static`, and the atlas is not shared with
		// another thread.
		unsafe {
			context
				.font_atlas()
				.add_font(&[FontSource::ttf_data_with_size(font, FONT_SIZE)]);
		}

		let mut platform =
			WinitPlatform::new(&mut context).map_err(|e| eyre!("cannot set up the window events: {e}"))?;
		platform
			.attach_window(Arc::clone(&window), HiDpiMode::Default, &mut context)
			.map_err(|e| eyre!("cannot attach the window to the UI: {e}"))?;
		let mut renderer = WgpuRenderer::new(WgpuInitInfo::new(device.clone(), queue.clone(), format), &mut context)
			.map_err(|e| eyre!("cannot set up the UI renderer: {e}"))?;
		renderer.set_gamma_mode(GammaMode::Auto);

		Ok(Self {
			window,
			surface,
			surface_config,
			device,
			queue,
			context,
			platform,
			renderer,
		})
	}

	/// Lets the UI see a window event. `true` if it dealt with it.
	pub fn handle_event(&mut self, event: &WindowEvent) -> bool {
		match self
			.platform
			.handle_window_event(&mut self.context, &self.window, event)
		{
			Ok(handled) => handled,
			Err(e) => {
				zenith_log::warn!("The UI did not take a window event: {e}");
				false
			}
		}
	}

	/// Whether the UI wants the keyboard (a text field has the focus).
	pub fn wants_text_input(&self) -> bool {
		self.context.io().want_text_input()
	}

	pub fn resize(&mut self, size: PhysicalSize<u32>) {
		if size.width == 0 || size.height == 0 {
			return;
		}
		self.surface_config.width = size.width;
		self.surface_config.height = size.height;
		self.surface.configure(&self.device, &self.surface_config);
	}

	/// Builds the UI with `build` and draws it. A frame that cannot be drawn
	/// now (the window is hidden, or its surface has to be set up again) is
	/// skipped.
	pub fn frame(&mut self, build: impl FnOnce(&Ui)) -> ZInternalResult<()> {
		// The surface first: if there is none, no frame is begun.
		let surface_frame = match self.surface.get_current_texture() {
			wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
			wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
				self.surface.configure(&self.device, &self.surface_config);
				return Ok(());
			}
			wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
			wgpu::CurrentSurfaceTexture::Validation => return Err(eyre!("the surface was refused by the GPU")),
		};

		self.platform
			.prepare_frame(&mut self.context, &self.window)
			.map_err(|e| eyre!("cannot prepare the frame: {e}"))?;
		let frame = self
			.context
			.try_begin_frame()
			.map_err(|e| eyre!("cannot begin the frame: {e}"))?;
		build(frame.ui());
		self.platform
			.prepare_render(frame.ui(), &self.window)
			.map_err(|e| eyre!("cannot prepare the render: {e}"))?;
		let pending = frame.render(
			self.renderer
				.renderer_consumer()
				.map_err(|e| eyre!("the UI renderer is not ready: {e}"))?,
		);
		let reconciled = self
			.renderer
			.reconcile_frame(pending)
			.map_err(|e| eyre!("cannot prepare the textures: {e}"))?;

		let extent = FramebufferExtent::from_texture(&surface_frame.texture);
		let view = surface_frame
			.texture
			.create_view(&wgpu::TextureViewDescriptor::default());
		let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
			label: Some("Zenith Editor encoder"),
		});
		{
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("Zenith Editor pass"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &view,
					resolve_target: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Clear(CLEAR_COLOR),
						store: wgpu::StoreOp::Store,
					},
					depth_slice: None,
				})],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			self.renderer
				.render_reconciled(reconciled, &mut pass, extent)
				.map_err(|e| eyre!("cannot draw the UI: {e}"))?;
		}
		self.queue.submit(Some(encoder.finish()));
		self.queue.present(surface_frame);
		Ok(())
	}
}

impl Drop for Gfx {
	fn drop(&mut self) {
		// The renderer lets go of its GPU resources while the device is still
		// alive.
		if let Err(e) = self.renderer.shutdown(&mut self.context) {
			zenith_log::warn!("The UI renderer did not shut down cleanly: {e}");
		}
		if let Err(e) = self.platform.detach_window(&mut self.context) {
			zenith_log::warn!("The window did not detach cleanly: {e}");
		}
	}
}
