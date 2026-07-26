use std::num::NonZeroU64;

pub struct Builder<'a> {
	entries: Vec<wgpu::BindGroupEntry<'a>>,
	layout: Option<&'a wgpu::BindGroupLayout>,
	device: &'a wgpu::Device,
}

impl<'a> Builder<'a> {
	pub const fn new(device: &'a wgpu::Device) -> Self {
		// TODO: replace &'a to pointer
		Self {
			entries: Vec::new(),
			layout: None,
			device,
		}
	}

	pub const fn set_layout(&mut self, layout: &'a wgpu::BindGroupLayout) { self.layout = Some(layout); }

	fn reset(&mut self) { self.entries.clear(); }

	pub fn add_texture(&mut self, view: &'a wgpu::TextureView, sampler: &'a wgpu::Sampler) {
		self.entries.push(wgpu::BindGroupEntry {
			binding: self.entries.len() as u32,
			resource: wgpu::BindingResource::TextureView(view),
		});

		self.entries.push(wgpu::BindGroupEntry {
			binding: self.entries.len() as u32,
			resource: wgpu::BindingResource::Sampler(sampler),
		});
	}

	pub fn add_buffer(&mut self, buffer: &'a wgpu::Buffer, offset: u64, size: NonZeroU64) {
		self.entries.push(wgpu::BindGroupEntry {
			binding: self.entries.len() as u32,
			resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
				buffer,
				offset,
				size: Some(size),
			}),
		});
	}

	pub fn build(&mut self, label: &str) -> wgpu::BindGroup {
		let descriptor = wgpu::BindGroupDescriptor {
			label: Some(label),
			layout: self.layout.expect("Layout is None..."),
			entries: &self.entries,
		};

		let bind_group = self.device.create_bind_group(&descriptor);
		self.reset();

		bind_group
	}
}
