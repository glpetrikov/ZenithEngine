use bytemuck::{Pod, Zeroable};
use glam::{Vec3, Vec4, f32, u8, u16, u64};
use wgpu::util::DeviceExt;

pub struct Mesh {
	pub buffer: wgpu::Buffer,
	pub offset: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
	pub position: [f32; 3],
	pub color: [f32; 4],
}

impl Vertex {
	pub const fn get_layout() -> wgpu::VertexBufferLayout<'static> {
		const ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];

		wgpu::VertexBufferLayout {
			array_stride: std::mem::size_of::<Self>() as u64,
			step_mode: wgpu::VertexStepMode::Vertex,
			attributes: &ATTRIBUTES,
		}
	}

	pub fn make_quad(device: &wgpu::Device) -> Mesh {
		let vertices: [Self; 4] = [
			Self {
				position: Vec3::new(-0.5, -0.5, 0.0).to_array(),
				color: Vec4::ONE.to_array(),
			},
			Self {
				position: Vec3::new(0.5, -0.5, 0.0).to_array(),
				color: Vec4::ONE.to_array(),
			},
			Self {
				position: Vec3::new(0.5, 0.5, 0.0).to_array(),
				color: Vec4::ONE.to_array(),
			},
			Self {
				position: Vec3::new(-0.5, 0.5, 0.0).to_array(),
				color: Vec4::ONE.to_array(),
			},
		];
		let indices: [u16; 6] = [0, 1, 2, 0, 2, 3];

		let bytes_1: &[u8] = bytemuck::cast_slice(&vertices);
		let bytes_2: &[u8] = bytemuck::cast_slice(&indices);
		let bytes_merged: &[u8] = &[bytes_1, bytes_2].concat();

		let buffer_descriptor = wgpu::util::BufferInitDescriptor {
			label: Some("Quad vertex & index buffer"),
			contents: bytes_merged,
			usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::INDEX,
		};

		let buffer = device.create_buffer_init(&buffer_descriptor);
		let offset: u64 = bytes_1.len().try_into().expect("Cannot get len of bytes_1 as u64");

		Mesh { buffer, offset }
	}
}
