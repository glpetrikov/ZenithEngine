//! What a model becomes, whatever its source: `MeshData`.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use zenith_error::{ZResult, ZenithError};

use super::{gltf, obj};
use crate::assets::AssetId;

/// The import settings of a model, kept in its container.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelImport {
	/// What the source is: `obj`, or `glb` (a `.gltf` becomes a `.glb` when
	/// it is imported, so the model is one file).
	pub source_format: String,
	/// Every position is multiplied by it.
	pub scale: f32,
	/// An `.obj` counts `v` from the bottom of the picture, the GPU (and glTF)
	/// from the top.
	pub flip_v: bool,
	/// The material of each mesh of the model, in the order of [`parse`]. The
	/// materials of the source are not read: they became assets of their own
	/// when it was imported.
	pub mesh_materials: Vec<Option<AssetId>>,
}

impl Default for ModelImport {
	fn default() -> Self {
		Self::for_format("obj")
	}
}

impl ModelImport {
	/// What suits a source of `format` (`obj` or `glb`).
	pub fn for_format(format: &str) -> Self {
		Self {
			source_format: format.to_ascii_lowercase(),
			scale: 1.0,
			flip_v: format.eq_ignore_ascii_case("obj"),
			mesh_materials: Vec::new(),
		}
	}
}

/// What the vertex shader reads for every vertex: the position, where the
/// texture is, and the tangent space (normal, tangent, bitangent) that a
/// normal map needs.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
	pub position: [f32; 3],
	pub uv: [f32; 2],
	pub normal: [f32; 3],
	pub tangent: [f32; 3],
	pub bitangent: [f32; 3],
}

/// One mesh of a model: one material.
#[derive(Clone, Debug, PartialEq)]
pub struct SubMesh {
	pub name: String,
	pub vertices: Vec<Vertex>,
	/// Three per triangle.
	pub indices: Vec<u32>,
	pub material: Option<AssetId>,
}

/// A model, ready for the GPU (or for physics).
#[derive(Clone, Debug, PartialEq)]
pub struct MeshData {
	pub submeshes: Vec<SubMesh>,
}

/// Reads the source of a model.
///
/// # Errors
/// `InvalidAsset` for a source of a format that cannot be read, that cannot be
/// read, or that has no triangles.
pub fn parse(source: &[u8], settings: &ModelImport) -> ZResult<MeshData> {
	let mut submeshes = match settings.source_format.as_str() {
		"obj" => obj::submeshes(source, settings)?,
		"glb" => gltf::submeshes(source, settings)?
			.into_iter()
			.map(|(submesh, _)| submesh)
			.collect(),
		other => {
			return Err(ZenithError::InvalidAsset(format!(
				"a model from {other:?} cannot be read"
			)));
		}
	};
	for (at, submesh) in submeshes.iter_mut().enumerate() {
		submesh.material = settings.mesh_materials.get(at).copied().flatten();
	}
	Ok(MeshData { submeshes })
}

/// The vertices of a mesh, with the tangent space worked out. `normals` may
/// be missing: then each vertex gets the normal of the triangles around it.
pub(crate) fn build_vertices(
	positions: &[Vec3],
	uvs: &[Vec2],
	normals: Option<&[Vec3]>,
	indices: &[u32],
) -> Vec<Vertex> {
	let count = positions.len();
	let uv = |i: usize| uvs.get(i).copied().unwrap_or(Vec2::ZERO);
	let mut tangents = vec![Vec3::ZERO; count];
	let mut bitangents = vec![Vec3::ZERO; count];
	let mut face_normals = vec![Vec3::ZERO; count];

	// A vertex shared by several triangles gets the sum of their tangent
	// spaces (normalized below), so the light does not break at the edges.
	for &[a, b, c] in indices.as_chunks::<3>().0 {
		let (i0, i1, i2) = (a as usize, b as usize, c as usize);
		let (Some(p0), Some(p1), Some(p2)) = (positions.get(i0), positions.get(i1), positions.get(i2)) else {
			continue;
		};
		let delta_pos1 = *p1 - *p0;
		let delta_pos2 = *p2 - *p0;
		let face_normal = delta_pos1.cross(delta_pos2);
		for i in [i0, i1, i2] {
			face_normals[i] += face_normal;
		}

		// Solving   delta_pos = delta_uv.x * tangent + delta_uv.y * bitangent
		// for the tangent and the bitangent.
		let delta_uv1 = uv(i1) - uv(i0);
		let delta_uv2 = uv(i2) - uv(i0);
		let det = delta_uv1.x.mul_add(delta_uv2.y, -(delta_uv1.y * delta_uv2.x));
		if det.abs() < f32::EPSILON {
			continue;
		}
		let r = 1.0 / det;
		let tangent = (delta_pos1 * delta_uv2.y - delta_pos2 * delta_uv1.y) * r;
		// Negated: the picture counts `y` downwards, a normal map (OpenGL
		// style) has its green channel pointing up.
		let bitangent = (delta_pos2 * delta_uv1.x - delta_pos1 * delta_uv2.x) * -r;
		for i in [i0, i1, i2] {
			tangents[i] += tangent;
			bitangents[i] += bitangent;
		}
	}

	(0..count)
		.map(|i| {
			let normal = normals
				.and_then(|normals| normals.get(i).copied())
				.unwrap_or_else(|| face_normals[i].normalize_or_zero());
			let tangent = tangents[i].normalize_or_zero();
			let (tangent, bitangent) = if tangent == Vec3::ZERO {
				// No texture to follow (or a degenerate one): any pair of
				// directions across the normal will do.
				let helper = if normal.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
				let tangent = normal.cross(helper).normalize_or_zero();
				(tangent, normal.cross(tangent))
			} else {
				(tangent, bitangents[i].normalize_or_zero())
			};
			Vertex {
				position: positions[i].to_array(),
				uv: uv(i).to_array(),
				normal: normal.to_array(),
				tangent: tangent.to_array(),
				bitangent: bitangent.to_array(),
			}
		})
		.collect()
}

#[cfg(test)]
pub(crate) mod tests {
	use super::*;

	pub fn approx(a: [f32; 3], b: [f32; 3]) -> bool {
		a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
	}

	#[test]
	fn a_source_of_another_format_is_refused() {
		let fbx = ModelImport::for_format("fbx");
		assert!(parse(b"anything", &fbx).is_err());
		assert!(parse(b"not a glb", &ModelImport::for_format("glb")).is_err());
	}

	#[test]
	fn the_v_is_flipped_for_an_obj_only() {
		assert!(ModelImport::for_format("OBJ").flip_v);
		assert!(!ModelImport::for_format("glb").flip_v);
	}
}
