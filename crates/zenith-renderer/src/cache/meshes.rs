//! Models on the GPU: one vertex buffer and one index buffer for all the
//! meshes of a model.

use std::ops::Range;

use wgpu::util::DeviceExt;
use zenith_error::{ZResult, ZenithError};

use crate::assets::{
	AssetId,
	import::mesh::{MeshData, Vertex},
};

/// One mesh of a model: a range of its index buffer, drawn with one material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuSubMesh {
	pub name: String,
	/// What to give `draw_indexed`.
	pub indices: Range<u32>,
	/// Where its vertices start in the vertex buffer.
	pub base_vertex: i32,
	pub material: Option<AssetId>,
}

/// A model on the GPU.
#[derive(Debug)]
pub struct GpuMesh {
	pub vertices: wgpu::Buffer,
	pub indices: wgpu::Buffer,
	pub submeshes: Vec<GpuSubMesh>,
	/// The box around the vertices: its smallest and its largest corner.
	pub bounds: ([f32; 3], [f32; 3]),
	bytes: u64,
}

impl GpuMesh {
	/// How the vertex shader reads a [`Vertex`]: `@location(0)` the position,
	/// `1` the uv, `2` the normal, `3` the tangent, `4` the bitangent.
	pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
		array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
		step_mode: wgpu::VertexStepMode::Vertex,
		attributes: &wgpu::vertex_attr_array![
			0 => Float32x3,
			1 => Float32x2,
			2 => Float32x3,
			3 => Float32x3,
			4 => Float32x3,
		],
	};

	pub const INDEX_FORMAT: wgpu::IndexFormat = wgpu::IndexFormat::Uint32;

	/// Puts `mesh` on the GPU.
	///
	/// # Errors
	/// `InvalidAsset` for a model with no triangles (the GPU refuses an empty
	/// buffer), or one too big to be counted in `u32`.
	pub fn upload(device: &wgpu::Device, label: &str, mesh: &MeshData) -> ZResult<Self> {
		let too_big = || ZenithError::InvalidAsset(format!("{label}: the model is too big"));
		let mut vertices: Vec<Vertex> = Vec::new();
		let mut indices: Vec<u32> = Vec::new();
		let mut submeshes = Vec::new();
		for submesh in &mesh.submeshes {
			if submesh.indices.is_empty() {
				continue;
			}
			let first = u32::try_from(indices.len()).map_err(|_| too_big())?;
			let base_vertex = i32::try_from(vertices.len()).map_err(|_| too_big())?;
			vertices.extend_from_slice(&submesh.vertices);
			indices.extend_from_slice(&submesh.indices);
			let end = u32::try_from(indices.len()).map_err(|_| too_big())?;
			submeshes.push(GpuSubMesh {
				name: submesh.name.clone(),
				indices: first..end,
				base_vertex,
				material: submesh.material,
			});
		}
		if submeshes.is_empty() || vertices.is_empty() {
			return Err(ZenithError::InvalidAsset(format!(
				"{label}: the model has no triangles"
			)));
		}
		Ok(Self::from_parts(device, label, &vertices, &indices, submeshes))
	}

	/// The buffers of `vertices` and `indices`, which must not be empty.
	pub(crate) fn from_parts(
		device: &wgpu::Device,
		label: &str,
		vertices: &[Vertex],
		indices: &[u32],
		submeshes: Vec<GpuSubMesh>,
	) -> Self {
		let bounds = bounds_of(vertices);
		let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some(&format!("{label} vertices")),
			contents: bytemuck::cast_slice(vertices),
			usage: wgpu::BufferUsages::VERTEX,
		});
		let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some(&format!("{label} indices")),
			contents: bytemuck::cast_slice(indices),
			usage: wgpu::BufferUsages::INDEX,
		});
		Self {
			bytes: vertices.size() + indices.size(),
			vertices,
			indices,
			submeshes,
			bounds,
		}
	}

	/// Sets the vertex buffer (slot `slot`) and the index buffer of the model.
	pub fn bind(&self, pass: &mut wgpu::RenderPass<'_>, slot: u32) {
		pass.set_vertex_buffer(slot, self.vertices.slice(..));
		pass.set_index_buffer(self.indices.slice(..), Self::INDEX_FORMAT);
	}

	/// What it takes of the memory of the GPU.
	pub const fn bytes(&self) -> u64 {
		self.bytes
	}
}

/// The smallest box that holds every vertex.
fn bounds_of(vertices: &[Vertex]) -> ([f32; 3], [f32; 3]) {
	let mut min = [f32::MAX; 3];
	let mut max = [f32::MIN; 3];
	for vertex in vertices {
		for axis in 0..3 {
			min[axis] = min[axis].min(vertex.position[axis]);
			max[axis] = max[axis].max(vertex.position[axis]);
		}
	}
	(min, max)
}
