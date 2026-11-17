//! `RenderTargets`: offscreen textures a camera can draw into, by
//! `RenderTargetId` (the viewport of the editor, a camera that draws into a
//! texture).

use std::collections::HashMap;

use bevy_ecs::{
	resource::Resource,
	system::{Res, ResMut},
};

use crate::{
	assets::AssetId,
	extract::{ExtractedViews, ViewTarget},
	renderer::Renderer,
};

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
	textures: HashMap<AssetId, RenderTargetId>,
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

	/// The target of the render texture `id`, made or resized to `size`.
	pub fn ensure_texture(
		&mut self,
		device: &wgpu::Device,
		id: AssetId,
		size: (u32, u32),
		format: wgpu::TextureFormat,
	) -> RenderTargetId {
		if let Some(&target) = self.textures.get(&id)
			&& self.targets.contains_key(&target)
		{
			self.resize(device, target, size);
			return target;
		}
		let target = self.create(device, size, format);
		self.textures.insert(id, target);
		target
	}

	/// The target of the render texture `id`, if a camera has drawn into it.
	pub fn texture_target(&self, id: AssetId) -> Option<RenderTargetId> {
		self.textures.get(&id).copied()
	}

	/// The target a view draws into, if it exists.
	pub fn resolve(&self, target: ViewTarget) -> Option<&RenderTarget> {
		match target {
			ViewTarget::Window => None,
			ViewTarget::Target(id) => self.get(id),
			ViewTarget::Texture { id, .. } => self.get(self.texture_target(id)?),
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

/// Makes (or resizes) the textures the cameras of the frame draw into. Runs
/// before `prepare_views`.
#[allow(clippy::needless_pass_by_value)] // a system takes its parameters by value
pub fn prepare_targets(views: Res<ExtractedViews>, renderer: Res<Renderer>, mut targets: ResMut<RenderTargets>) {
	for view in &views.0 {
		if let ViewTarget::Texture { id, size } = view.target {
			targets.ensure_texture(renderer.device(), id, size, renderer.output_format());
		}
	}
}
