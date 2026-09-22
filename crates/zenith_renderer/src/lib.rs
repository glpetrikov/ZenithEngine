use crate::types::SurfaceFormats;

pub mod render_pass;
pub mod render_target;
pub mod types;
pub mod viewport;

pub fn pick_alpha_mode(caps: &wgpu::SurfaceCapabilities, transparent: bool) -> wgpu::CompositeAlphaMode {
	use wgpu::CompositeAlphaMode::{Auto, Inherit, Opaque, PostMultiplied, PreMultiplied};

	let preference: &[_] = if transparent {
		&[PreMultiplied, PostMultiplied, Inherit]
	} else {
		&[Opaque]
	};

	preference
		.iter()
		.copied()
		.find(|mode| caps.alpha_modes.contains(mode))
		.unwrap_or(Auto)
}

pub fn pick_surface_format(caps: &wgpu::SurfaceCapabilities) -> Option<SurfaceFormats> {
	use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm, Rgba8UnormSrgb};

	// Native
	for format in [Bgra8UnormSrgb, Rgba8UnormSrgb] {
		if caps.formats.contains(&format) {
			return Some(SurfaceFormats {
				storage: format,
				view: format,
			});
		}
	}

	// Browser
	for format in [Bgra8Unorm, Rgba8Unorm] {
		if caps.formats.contains(&format) {
			return Some(SurfaceFormats {
				storage: format,
				view: format.add_srgb_suffix(),
			});
		}
	}

	None
}
