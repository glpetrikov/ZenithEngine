//! The surface: its configuration, the choice of a format, acquiring and
//! presenting a frame.

use crate::settings::RenderSettings;

/// A surface (a window, a canvas) and how it is configured.
pub struct SurfaceState {
	surface: wgpu::Surface<'static>,
	/// What the surface can do: the settings are matched against it.
	caps: wgpu::SurfaceCapabilities,
	config: wgpu::SurfaceConfiguration,
	/// The format the frames are drawn in: the sRGB view of what the surface
	/// stores.
	view_format: wgpu::TextureFormat,
	/// `false` while the window has no size (minimized, hidden): a surface of
	/// no size cannot be configured.
	configured: bool,
}

/// What [`SurfaceState::acquire`] got.
pub enum Acquired {
	/// A frame to draw into.
	Frame(SurfaceFrame),
	/// No frame now (the window is hidden, or the surface was configured
	/// again); the next one will do.
	Skip,
	/// The surface is gone and has to be made again with
	/// [`Renderer::set_surface`](crate::renderer::Renderer::set_surface).
	Lost,
}

/// The texture of the surface for this frame.
pub struct SurfaceFrame {
	texture: wgpu::SurfaceTexture,
	/// The sRGB view to draw into.
	pub view: wgpu::TextureView,
	suboptimal: bool,
}

impl SurfaceFrame {
	/// The texture behind [`SurfaceFrame::view`].
	pub const fn texture(&self) -> &wgpu::Texture {
		&self.texture.texture
	}
}

impl SurfaceState {
	/// Configures `surface` for `device`. `None` if it supports no format the
	/// renderer can draw in.
	pub(crate) fn new(
		surface: wgpu::Surface<'static>,
		adapter: &wgpu::Adapter,
		device: &wgpu::Device,
		width: u32,
		height: u32,
		settings: &RenderSettings,
	) -> Option<Self> {
		let caps = surface.get_capabilities(adapter);
		let (storage, view_format) = pick_format(&caps)?;
		let config = wgpu::SurfaceConfiguration {
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			format: storage,
			width: width.max(1),
			height: height.max(1),
			present_mode: settings.pick_present_mode(&caps),
			alpha_mode: settings.pick_alpha_mode(&caps),
			view_formats: if view_format == storage {
				vec![]
			} else {
				vec![view_format]
			},
			desired_maximum_frame_latency: settings.max_frame_latency,
			color_space: wgpu::SurfaceColorSpace::Auto,
		};
		let mut state = Self {
			surface,
			caps,
			config,
			view_format,
			configured: false,
		};
		state.resize(device, width, height);
		Some(state)
	}

	/// Shows the frames the way `settings` say from now on. Configures the
	/// surface again only if something changed.
	pub(crate) fn apply(&mut self, device: &wgpu::Device, settings: &RenderSettings) {
		let present_mode = settings.pick_present_mode(&self.caps);
		let alpha_mode = settings.pick_alpha_mode(&self.caps);
		let latency = settings.max_frame_latency;
		let config = &mut self.config;
		if config.present_mode == present_mode
			&& config.alpha_mode == alpha_mode
			&& config.desired_maximum_frame_latency == latency
		{
			return;
		}
		config.present_mode = present_mode;
		config.alpha_mode = alpha_mode;
		config.desired_maximum_frame_latency = latency;
		if self.configured {
			self.surface.configure(device, &self.config);
		}
	}

	pub const fn view_format(&self) -> wgpu::TextureFormat {
		self.view_format
	}

	/// The size of the surface, in pixels.
	pub const fn size(&self) -> (u32, u32) {
		(self.config.width, self.config.height)
	}

	/// The window changed its size. A size of zero leaves the surface
	/// unconfigured until a real size comes back.
	pub(crate) fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
		if width == 0 || height == 0 {
			self.configured = false;
			return;
		}
		let max = device.limits().max_texture_dimension_2d;
		self.config.width = width.min(max);
		self.config.height = height.min(max);
		self.surface.configure(device, &self.config);
		self.configured = true;
	}

	/// The texture to draw this frame into.
	pub(crate) fn acquire(&mut self, device: &wgpu::Device) -> Acquired {
		if !self.configured {
			return Acquired::Skip;
		}
		let (texture, suboptimal) = match self.surface.get_current_texture() {
			wgpu::CurrentSurfaceTexture::Success(texture) => (texture, false),
			wgpu::CurrentSurfaceTexture::Suboptimal(texture) => (texture, true),
			wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return Acquired::Skip,
			wgpu::CurrentSurfaceTexture::Outdated => {
				self.surface.configure(device, &self.config);
				return Acquired::Skip;
			}
			wgpu::CurrentSurfaceTexture::Validation => {
				zenith_log::error!("The surface refused to give a frame (a validation error)");
				return Acquired::Skip;
			}
			wgpu::CurrentSurfaceTexture::Lost => {
				self.configured = false;
				return Acquired::Lost;
			}
		};
		let view = texture.texture.create_view(&wgpu::TextureViewDescriptor {
			format: Some(self.view_format),
			..Default::default()
		});
		Acquired::Frame(SurfaceFrame {
			texture,
			view,
			suboptimal,
		})
	}

	/// Shows the frame. A frame that no longer fits the surface well gets the
	/// surface configured again for the next one.
	pub(crate) fn present(&self, device: &wgpu::Device, queue: &wgpu::Queue, frame: SurfaceFrame) {
		let suboptimal = frame.suboptimal;
		drop(frame.view);
		queue.present(frame.texture);
		if suboptimal {
			self.surface.configure(device, &self.config);
		}
	}
}

/// The format the surface stores, and the sRGB one to draw in.
///
/// A native surface usually takes an sRGB format as it is. A browser only
/// stores the plain ones, and gives an sRGB view of them.
pub fn pick_format(caps: &wgpu::SurfaceCapabilities) -> Option<(wgpu::TextureFormat, wgpu::TextureFormat)> {
	use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm, Rgba8UnormSrgb};

	[Bgra8UnormSrgb, Rgba8UnormSrgb]
		.into_iter()
		.find(|format| caps.formats.contains(format))
		.map(|format| (format, format))
		.or_else(|| {
			[Bgra8Unorm, Rgba8Unorm]
				.into_iter()
				.find(|format| caps.formats.contains(format))
				.map(|format| (format, format.add_srgb_suffix()))
		})
}

#[cfg(test)]
mod tests {
	use super::*;

	fn caps(formats: &[wgpu::TextureFormat], alpha_modes: &[wgpu::CompositeAlphaMode]) -> wgpu::SurfaceCapabilities {
		wgpu::SurfaceCapabilities {
			formats: formats.to_vec(),
			alpha_modes: alpha_modes.to_vec(),
			..Default::default()
		}
	}

	#[test]
	fn an_srgb_format_is_taken_as_it_is() {
		use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb};
		assert_eq!(
			pick_format(&caps(&[Bgra8Unorm, Bgra8UnormSrgb], &[])),
			Some((Bgra8UnormSrgb, Bgra8UnormSrgb))
		);
	}

	#[test]
	fn a_browser_gets_an_srgb_view_of_a_plain_format() {
		use wgpu::TextureFormat::{Rgba8Unorm, Rgba8UnormSrgb, Rgba16Float};
		assert_eq!(
			pick_format(&caps(&[Rgba16Float, Rgba8Unorm], &[])),
			Some((Rgba8Unorm, Rgba8UnormSrgb))
		);
	}

	#[test]
	fn no_format_to_draw_in() {
		assert_eq!(pick_format(&caps(&[wgpu::TextureFormat::Rgba16Float], &[])), None);
	}
}
