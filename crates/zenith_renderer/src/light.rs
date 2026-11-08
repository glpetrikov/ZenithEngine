use glam::{Quat, Vec3};
use wgpu::util::DeviceExt;

/// The light as the shaders read it. A `vec3` takes 16 bytes in a uniform
/// buffer, which is why each one is followed by a padding float.
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightUniform {
	pub position: Vec3,
	_padding: f32,
	pub color: Vec3,
	_padding2: f32,
}

impl LightUniform {
	#[must_use]
	pub const fn new(position: Vec3, color: Vec3) -> Self {
		Self {
			position,
			_padding: 0.0,
			color,
			_padding2: 0.0,
		}
	}
}

/// One point light, with its buffer and bind group.
pub struct Light {
	pub uniform: LightUniform,
	buffer: wgpu::Buffer,
	pub bind_group: wgpu::BindGroup,
}

impl Light {
	/// How fast the light goes around the origin.
	const DEGREES_PER_SECOND: f32 = 60.0;

	#[must_use]
	pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
		device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("Light Bind Group Layout"),
			entries: &[wgpu::BindGroupLayoutEntry {
				binding: 0,
				// The vertex shader of the lamp needs the position, the
				// fragment shader of the scene the position and the color.
				visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
				ty: wgpu::BindingType::Buffer {
					ty: wgpu::BufferBindingType::Uniform,
					has_dynamic_offset: false,
					min_binding_size: None,
				},
				count: None,
			}],
		})
	}

	#[must_use]
	pub fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, position: Vec3, color: Vec3) -> Self {
		let uniform = LightUniform::new(position, color);
		let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some("Light Buffer"),
			contents: bytemuck::bytes_of(&uniform),
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
		});
		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("Light Bind Group"),
			layout,
			entries: &[wgpu::BindGroupEntry {
				binding: 0,
				resource: buffer.as_entire_binding(),
			}],
		});

		Self {
			uniform,
			buffer,
			bind_group,
		}
	}

	/// Takes the light around the origin (around the `y` axis) and sends the
	/// new position to the GPU.
	pub fn orbit(&mut self, queue: &wgpu::Queue, dt: f32) {
		let turn = Quat::from_axis_angle(Vec3::Y, (Self::DEGREES_PER_SECOND * dt).to_radians());
		self.uniform.position = turn * self.uniform.position;
		queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&self.uniform));
	}
}
