//! `RenderTargets`: offscreen textures a camera can draw into, by
//! `RenderTargetId` (the viewport of the editor, a camera that draws into a
//! texture).

use std::collections::HashMap;

use bevy_ecs::resource::Resource;

/// An offscreen target of [`RenderTargets`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RenderTargetId(pub u32);

/// A texture a camera draws into.
pub struct RenderTarget {
	pub texture: wgpu::Texture,
	pub view: wgpu::TextureView,
	pub format: wgpu::TextureFormat,
	pub size: (u32, u32),
}

/// The offscreen targets of the renderer.
#[derive(Resource, Default)]
pub struct RenderTargets {
	targets: HashMap<RenderTargetId, RenderTarget>,
	next: u32,
}

impl RenderTargets {
	/// Makes a target of `size` (at least 1 a side) that can be drawn into and
	/// shown or read afterwards.
	pub fn create(&mut self, device: &wgpu::Device, size: (u32, u32), format: wgpu::TextureFormat) -> RenderTargetId {
		let id = RenderTargetId(self.next);
		self.next += 1;
		self.targets.insert(id, make(device, size, format));
		id
	}

	/// Makes the target `size` big. The same texture stays if it is that size
	/// already.
	pub fn resize(&mut self, device: &wgpu::Device, id: RenderTargetId, size: (u32, u32)) {
		let size = (size.0.max(1), size.1.max(1));
		if let Some(target) = self.targets.get_mut(&id)
			&& target.size != size
		{
			*target = make(device, size, target.format);
		}
	}

	pub fn get(&self, id: RenderTargetId) -> Option<&RenderTarget> {
		self.targets.get(&id)
	}

	pub fn remove(&mut self, id: RenderTargetId) -> Option<RenderTarget> {
		self.targets.remove(&id)
	}

	pub fn len(&self) -> usize {
		self.targets.len()
	}

	pub fn is_empty(&self) -> bool {
		self.targets.is_empty()
	}
}

fn make(device: &wgpu::Device, size: (u32, u32), format: wgpu::TextureFormat) -> RenderTarget {
	let size = (size.0.max(1), size.1.max(1));
	let texture = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("Zenith Render Target"),
		size: wgpu::Extent3d {
			width: size.0,
			height: size.1,
			depth_or_array_layers: 1,
		},
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT
			| wgpu::TextureUsages::TEXTURE_BINDING
			| wgpu::TextureUsages::COPY_SRC,
		view_formats: &[],
	});
	let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
	RenderTarget {
		texture,
		view,
		format,
		size,
	}
}
