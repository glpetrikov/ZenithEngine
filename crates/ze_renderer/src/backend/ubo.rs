use std::num::NonZeroU64;

use crate::backend::bind_group;

pub struct Ubo {
	pub buffer: wgpu::Buffer,
	pub bind_group: wgpu::BindGroup,
}

impl Ubo {
	pub fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
		let buffer_descriptor = wgpu::BufferDescriptor {
			label: Some("UBO"),
			size: std::mem::size_of::<glam::Mat4>() as u64,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		};

		let buffer = device.create_buffer(&buffer_descriptor);

		let bind_group: wgpu::BindGroup;
		{
			let mut builder = bind_group::Builder::new(device);
			builder.set_layout(layout);
			builder.add_buffer(
				&buffer,
				0,
				NonZeroU64::new(std::mem::size_of::<glam::Mat4>() as u64).expect("size of glam::Mat4 is zero"),
			);
			bind_group = builder.build("Matrix");
		}

		Self { buffer, bind_group }
	}

	pub fn upload(&self, matrix: &glam::Mat4, queue: &wgpu::Queue) {
		let data = bytemuck::bytes_of(matrix);
		queue.write_buffer(&self.buffer, 0, data);
	}
}
