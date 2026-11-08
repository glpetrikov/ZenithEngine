use glam::{Mat4, Quat, Vec3};

/// One copy of a mesh: where it stands and how it is turned.
///
/// The GPU does not get this. It gets [`InstanceRaw`], the matrix made from it.
#[derive(Clone, Copy, Debug)]
pub struct Instance {
	pub position: Vec3,
	pub rotation: Quat,
}

impl Instance {
	#[must_use]
	pub fn to_raw(&self) -> InstanceRaw {
		InstanceRaw {
			model: Mat4::from_translation(self.position) * Mat4::from_quat(self.rotation),
		}
	}
}

/// What the vertex shader reads for every instance: the model matrix.
///
/// A vertex attribute can hold four floats at most, so the matrix goes in as
/// four attributes (`shader_location` 5 to 8), one per column. The locations
/// 2 to 4 stay free for the normal, the tangent and the bitangent of a vertex.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct InstanceRaw {
	model: Mat4,
}

impl InstanceRaw {
	#[must_use]
	pub const fn desc() -> wgpu::VertexBufferLayout<'static> {
		use std::mem::size_of;

		const COLUMN: wgpu::BufferAddress = size_of::<[f32; 4]>() as wgpu::BufferAddress;

		wgpu::VertexBufferLayout {
			array_stride: size_of::<Self>() as wgpu::BufferAddress,
			// The buffer moves on by one element per instance, not per vertex.
			step_mode: wgpu::VertexStepMode::Instance,
			attributes: &[
				wgpu::VertexAttribute {
					offset: 0,
					shader_location: 5,
					format: wgpu::VertexFormat::Float32x4,
				},
				wgpu::VertexAttribute {
					offset: COLUMN,
					shader_location: 6,
					format: wgpu::VertexFormat::Float32x4,
				},
				wgpu::VertexAttribute {
					offset: COLUMN * 2,
					shader_location: 7,
					format: wgpu::VertexFormat::Float32x4,
				},
				wgpu::VertexAttribute {
					offset: COLUMN * 3,
					shader_location: 8,
					format: wgpu::VertexFormat::Float32x4,
				},
			],
		}
	}
}
