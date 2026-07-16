/// Parameters for `RenderTarget::new`. Deliberately generic -- this module
/// knows nothing about `GBuffers` or bloom, it's just "create N render targets,
/// get views for a render pass, resize them all when the viewport resizes."
pub struct RenderTargetDescriptor<'a> {
	pub label: &'a str,
	pub width: u32,
	pub height: u32,
	pub format: wgpu::TextureFormat,
	pub mip_level_count: u32,
	pub usage: wgpu::TextureUsages,
}

/// A `wgpu::Texture` plus one single-mip-level `TextureView` per mip, cached
/// eagerly at construction (and rebuilt wholesale on resize) rather than
/// created on demand. Every mip view here is reused every frame both as a
/// render-pass color attachment and as a sample source in a later pass, and
/// bind groups reference a specific view's identity -- recreating views
/// per-frame would force recreating bind groups per-frame too.
#[allow(dead_code)] // generic reusable API surface; not every accessor has a consumer yet
pub struct RenderTarget {
	texture: wgpu::Texture,
	mip_views: Vec<wgpu::TextureView>,
	format: wgpu::TextureFormat,
	width: u32,
	height: u32,
	mip_level_count: u32,
}

#[allow(dead_code)]
impl RenderTarget {
	pub fn new(device: &wgpu::Device, desc: &RenderTargetDescriptor) -> Self {
		let mip_level_count = desc.mip_level_count.max(1);
		let width = desc.width.max(1);
		let height = desc.height.max(1);

		let texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some(desc.label),
			size: wgpu::Extent3d {
				width,
				height,
				depth_or_array_layers: 1,
			},
			mip_level_count,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: desc.format,
			usage: desc.usage,
			view_formats: &[],
		});

		let mip_views = (0..mip_level_count)
			.map(|level| {
				texture.create_view(&wgpu::TextureViewDescriptor {
					label: Some(desc.label),
					format: None,
					dimension: None,
					usage: None,
					aspect: wgpu::TextureAspect::All,
					base_mip_level: level,
					mip_level_count: Some(1),
					base_array_layer: 0,
					array_layer_count: None,
				})
			})
			.collect();

		Self {
			texture,
			mip_views,
			format: desc.format,
			width,
			height,
			mip_level_count,
		}
	}

	pub const fn texture(&self) -> &wgpu::Texture { &self.texture }

	/// Sugar for `mip_view(0)` -- the common case for single-mip render
	/// targets.
	pub fn view(&self) -> &wgpu::TextureView { &self.mip_views[0] }

	pub fn mip_view(&self, level: u32) -> &wgpu::TextureView { &self.mip_views[level as usize] }

	pub const fn format(&self) -> wgpu::TextureFormat { self.format }

	pub const fn size(&self) -> (u32, u32) { (self.width, self.height) }

	pub const fn mip_level_count(&self) -> u32 { self.mip_level_count }
}
