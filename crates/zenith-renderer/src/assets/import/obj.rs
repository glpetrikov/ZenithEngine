//! Wavefront `.obj` models.

use std::io::Cursor;

use glam::{Vec2, Vec3};
use zenith_error::{ZResult, ZenithError};

use super::mesh::{ModelImport, SubMesh, build_vertices};
use crate::assets::refuse_lfs_pointer;

/// The meshes of an `.obj` as `tobj` reads them, without those that have no
/// triangles (the GPU refuses an empty buffer, and they show nothing). The
/// order is that of [`ModelImport::mesh_materials`].
///
/// `mtl` is called for the material files the `.obj` names.
pub(crate) fn read_obj(
	source: &[u8],
	mtl: impl Fn(&std::path::Path) -> tobj::MTLLoadResult,
) -> ZResult<(Vec<tobj::Model>, Result<Vec<tobj::Material>, tobj::LoadError>)> {
	refuse_lfs_pointer(source, &"the .obj")?;
	let options = tobj::LoadOptions {
		// Quads and polygons become triangles, and every vertex gets one
		// index for its position, uv and normal together.
		triangulate: true,
		single_index: true,
		..Default::default()
	};
	let (models, materials) = tobj::load_obj_buf(&mut Cursor::new(source), &options, mtl)
		.map_err(|e| ZenithError::InvalidAsset(format!("the .obj cannot be read: {e}")))?;
	let models: Vec<_> = models.into_iter().filter(|m| !m.mesh.indices.is_empty()).collect();
	if models.is_empty() {
		return Err(ZenithError::InvalidAsset("the model has no triangles".into()));
	}
	Ok((models, materials))
}

/// The meshes of an `.obj`, without their materials (see
/// [`super::mesh::parse`]).
pub(crate) fn submeshes(source: &[u8], settings: &ModelImport) -> ZResult<Vec<SubMesh>> {
	let (models, _) = read_obj(source, |_| Err(tobj::LoadError::OpenFileFailed))?;
	Ok(models
		.into_iter()
		.map(|model| {
			let mesh = &model.mesh;
			let positions: Vec<Vec3> = mesh
				.positions
				.as_chunks::<3>()
				.0
				.iter()
				.map(|p| Vec3::from_array(*p) * settings.scale)
				.collect();
			let uvs: Vec<Vec2> = mesh
				.texcoords
				.as_chunks::<2>()
				.0
				.iter()
				.map(|&[u, v]| Vec2::new(u, if settings.flip_v { 1.0 - v } else { v }))
				.collect();
			let normals: Vec<Vec3> = mesh
				.normals
				.as_chunks::<3>()
				.0
				.iter()
				.map(|n| Vec3::from_array(*n))
				.collect();
			let normals = (normals.len() == positions.len()).then_some(normals.as_slice());
			SubMesh {
				vertices: build_vertices(&positions, &uvs, normals, &mesh.indices),
				indices: model.mesh.indices.clone(),
				name: model.name,
				material: None,
			}
		})
		.collect())
}

#[cfg(test)]
pub(crate) mod tests {
	use super::*;
	use crate::assets::{
		AssetId,
		import::mesh::{parse, tests::approx},
	};

	/// Two triangles (a square) with uvs, and an empty object that is left out.
	pub const SQUARE: &str = "\
o empty
o square
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
vt 0 0
vt 1 0
vt 1 1
vt 0 1
vn 0 0 1
usemtl red
f 1/1/1 2/2/1 3/3/1 4/4/1
";

	#[test]
	fn a_model_is_read_with_its_tangent_space() -> ZResult<()> {
		let material = AssetId::from_bytes([4; 16]);
		let settings = ModelImport {
			scale: 2.0,
			mesh_materials: vec![Some(material)],
			..ModelImport::default()
		};
		let data = parse(SQUARE.as_bytes(), &settings)?;
		let [mesh] = data.submeshes.as_slice() else {
			return Err(ZenithError::Other(format!("one mesh, not {}", data.submeshes.len())));
		};
		assert_eq!(mesh.name, "square");
		assert_eq!(mesh.indices.len(), 6, "a quad is two triangles");
		assert_eq!(mesh.material, Some(material));
		let corner = mesh.vertices.iter().find(|v| approx(v.position, [2.0, 2.0, 0.0]));
		assert!(corner.is_some(), "scaled: {:?}", mesh.vertices);
		for vertex in &mesh.vertices {
			assert!(approx(vertex.normal, [0.0, 0.0, 1.0]));
			assert!(approx(vertex.tangent, [1.0, 0.0, 0.0]), "u grows along x: {vertex:?}");
			assert!(approx(vertex.bitangent, [0.0, 1.0, 0.0]), "{vertex:?}");
		}
		let top = mesh.vertices.iter().find(|v| approx(v.position, [0.0, 2.0, 0.0]));
		assert_eq!(top.map(|v| v.uv), Some([0.0, 0.0]), "v counted from the top");
		Ok(())
	}

	#[test]
	fn normals_are_made_when_the_obj_has_none() -> ZResult<()> {
		let data = parse(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n", &ModelImport::default())?;
		let normals: Vec<_> = data
			.submeshes
			.iter()
			.flat_map(|m| &m.vertices)
			.map(|v| v.normal)
			.collect();
		assert!(normals.iter().all(|n| approx(*n, [0.0, 0.0, 1.0])), "{normals:?}");
		Ok(())
	}

	#[test]
	fn a_model_without_triangles_is_refused() {
		assert!(parse(b"v 0 0 0\n", &ModelImport::default()).is_err());
	}
}
