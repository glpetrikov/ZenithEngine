#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RenderTargetHandle(u32);

pub struct RenderTarget {
	pub view: wgpu::TextureView,
	pub texture: wgpu::Texture,
	pub format: wgpu::TextureFormat,
	pub size: (u32, u32),
}

pub struct FrameResources {
	targets: Vec<RenderTarget>,
}

impl FrameResources {
	pub fn insert(&mut self, target: RenderTarget) -> RenderTargetHandle {
		let handle = RenderTargetHandle(self.targets.len() as u32);
		self.targets.push(target);
		handle
	}

	pub fn get(&self, handle: RenderTargetHandle) -> &RenderTarget { &self.targets[handle.0 as usize] }
}
