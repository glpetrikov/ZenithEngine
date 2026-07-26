use crate::render_target::{RenderTarget, RenderTargetDescriptor};

pub const ALBEDO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm; // == VIEWPORT_TEXTURE_FORMAT
pub const EMISSIVE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float; // HDR headroom for bloom
pub const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The geometry pass's multi-attachment output. Depth is deliberately not
/// duplicated here -- `Renderer` already owns `viewport_depth_texture`/
/// `viewport_depth_view` with identical lifecycle (single-mip, viewport-sized,
/// resized at the same trigger point), so the geometry pass just borrows that.
pub struct GBuffer {
	pub(crate) albedo: RenderTarget,
	pub(crate) emissive: RenderTarget,
	pub(crate) normal: RenderTarget,
}

impl GBuffer {
	pub fn new(device: &wgpu::Device, size: winit::dpi::PhysicalSize<u32>) -> Self {
		let albedo = RenderTarget::new(
			device,
			&RenderTargetDescriptor {
				label: "GBuffer Albedo",
				width: size.width,
				height: size.height,
				format: ALBEDO_FORMAT,
				mip_level_count: 1,
				usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			},
		);
		let emissive = RenderTarget::new(
			device,
			&RenderTargetDescriptor {
				label: "GBuffer Emissive",
				width: size.width,
				height: size.height,
				format: EMISSIVE_FORMAT,
				mip_level_count: 1,
				usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			},
		);
		// Not sampled by anything yet -- purely establishes the GBuffer seam for
		// future 2D lighting, so no TEXTURE_BINDING usage.
		let normal = RenderTarget::new(
			device,
			&RenderTargetDescriptor {
				label: "GBuffer Normal",
				width: size.width,
				height: size.height,
				format: NORMAL_FORMAT,
				mip_level_count: 1,
				usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			},
		);

		Self {
			albedo,
			emissive,
			normal,
		}
	}

	pub fn resize(&mut self, device: &wgpu::Device, size: winit::dpi::PhysicalSize<u32>) {
		*self = Self::new(device, size);
	}
}
