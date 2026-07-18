pub struct Builder<'a> {
	entries: Vec<wgpu::BindGroupLayoutEntry>,
	device: &'a wgpu::Device,
}

impl<'a> Builder<'a> {
	pub const fn new(device: &'a wgpu::Device) -> Self {
		// TODO: replace &'a to pointer
		Self {
			entries: Vec::new(),
			device,
		}
	}

	fn reset(&mut self) { self.entries.clear(); }

	/// A texture + sampler pair, shared by every instance drawn against it.
	/// Per-sprite parameters (tint, uv rect, glow, ...) travel as per-instance
	/// vertex attributes instead of a per-draw-call material uniform, so this
	/// bind group no longer carries a uniform buffer.
	pub fn add_texture(&mut self) {
		self.entries.push(wgpu::BindGroupLayoutEntry {
			binding: self.entries.len() as u32,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture {
				sample_type: wgpu::TextureSampleType::Float { filterable: true },
				view_dimension: wgpu::TextureViewDimension::D2,
				multisampled: false,
			},
			count: None,
		});

		self.entries.push(wgpu::BindGroupLayoutEntry {
			binding: self.entries.len() as u32,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
			count: None,
		});
	}

	pub fn add_ubo(&mut self) {
		self.entries.push(wgpu::BindGroupLayoutEntry {
			binding: self.entries.len() as u32,
			visibility: wgpu::ShaderStages::VERTEX,
			ty: wgpu::BindingType::Buffer {
				ty: wgpu::BufferBindingType::Uniform,
				has_dynamic_offset: false,
				min_binding_size: None,
			},
			count: None,
		});
	}

	pub fn build(&mut self, label: &str) -> wgpu::BindGroupLayout {
		let layout_descriptor = wgpu::BindGroupLayoutDescriptor {
			label: Some(label),
			entries: &self.entries,
		};

		let layout = self.device.create_bind_group_layout(&layout_descriptor);
		self.reset();

		layout
	}
}
