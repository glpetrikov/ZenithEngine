use bevy_ecs::component::Component;
use glam::{Vec3, Vec4};
use serde::{Deserialize, Serialize};

#[derive(Component, Serialize, Deserialize, Default, Clone)]
pub struct Camera {
	pub projection: glam::Mat4,
}

pub enum PowerMode {
	LowPerformance,
	HighPerformance,
}

pub struct SurfaceFormats {
	pub storage: wgpu::TextureFormat,
	pub view: wgpu::TextureFormat,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
	color: Vec4,
	position: Vec3,
	_pad: f32,
}

impl Vertex {
	pub const fn new(position: Vec3, color: Vec4) -> Self {
		Self {
			color,
			position,
			_pad: 0.0,
		}
	}

	pub const fn desc() -> wgpu::VertexBufferLayout<'static> {
		wgpu::VertexBufferLayout {
			array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
			step_mode: wgpu::VertexStepMode::Vertex,
			attributes: &[
				wgpu::VertexAttribute {
					offset: std::mem::offset_of!(Self, position) as wgpu::BufferAddress,
					shader_location: 0,
					format: wgpu::VertexFormat::Float32x3,
				},
				wgpu::VertexAttribute {
					offset: std::mem::offset_of!(Self, color) as wgpu::BufferAddress,
					shader_location: 1,
					format: wgpu::VertexFormat::Float32x4,
				},
			],
		}
	}
}

pub const VERTICES: &[Vertex] = &[
	Vertex::new(
		Vec3::new(-0.086_824_1, 0.492_403_86, 0.0),
		Vec4::new(1.0, 0.0, 0.0, 1.0),
	),
	Vertex::new(
		Vec3::new(-0.495_134_06, 0.069_586_47, 0.0),
		Vec4::new(1.0, 0.9, 0.0, 1.0),
	),
	Vertex::new(
		Vec3::new(-0.219_185_49, -0.449_397_06, 0.0),
		Vec4::new(0.0, 0.8, 0.2, 1.0),
	),
	Vertex::new(
		Vec3::new(0.359_669_98, -0.347_329_1, 0.0),
		Vec4::new(0.0, 0.3, 1.0, 1.0),
	),
	Vertex::new(Vec3::new(0.441_473_72, 0.234_735_9, 0.0), Vec4::new(0.7, 0.0, 1.0, 1.0)),
];

pub const INDICES: &[u16] = &[0, 1, 4, 1, 2, 4, 2, 3, 4];
