use glam::{Mat3, Mat4, Quat, Vec3};

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
			model: (Mat4::from_translation(self.position) * Mat4::from_quat(self.rotation)).to_cols_array_2d(),
			// An instance is only moved and turned, never scaled, so the matrix
			// that turns its normals is the rotation itself. (With a scale it
			// would have to be the inverse transpose of the model matrix.)
			normal: Mat3::from_quat(self.rotation).to_cols_array_2d(),
		}
	}
}

/// What the vertex shader reads for every instance: the model matrix, and the
/// matrix that turns the normals, tangents and bitangents of the mesh.
///
/// A vertex attribute can hold four floats at most, so the matrix goes in as
/// four attributes (`shader_location` 5 to 8), one per column, and the normal
/// matrix as three (9 to 11). The locations 0 to 4 are the vertex of a
/// [`ModelVertex`](crate::model::ModelVertex).
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct InstanceRaw {
	// Plain arrays: a `Mat4` is aligned to 16 bytes, which would put padding
	// between the two and make the stride of the buffer 112 instead of 100.
	model: [[f32; 4]; 4],
	normal: [[f32; 3]; 3],
}

impl InstanceRaw {
	#[must_use]
	pub const fn desc() -> wgpu::VertexBufferLayout<'static> {
		use std::mem::size_of;

		const COLUMN: wgpu::BufferAddress = size_of::<[f32; 4]>() as wgpu::BufferAddress;
		const COLUMN3: wgpu::BufferAddress = size_of::<[f32; 3]>() as wgpu::BufferAddress;
		const NORMAL: wgpu::BufferAddress = size_of::<[[f32; 4]; 4]>() as wgpu::BufferAddress;

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
				wgpu::VertexAttribute {
					offset: NORMAL,
					shader_location: 9,
					format: wgpu::VertexFormat::Float32x3,
				},
				wgpu::VertexAttribute {
					offset: NORMAL + COLUMN3,
					shader_location: 10,
					format: wgpu::VertexFormat::Float32x3,
				},
				wgpu::VertexAttribute {
					offset: NORMAL + COLUMN3 * 2,
					shader_location: 11,
					format: wgpu::VertexFormat::Float32x3,
				},
			],
		}
	}
}
