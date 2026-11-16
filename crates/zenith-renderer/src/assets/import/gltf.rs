//! glTF 2.0 models: `.gltf` (with the files it names) and `.glb`.
//!
//! A model is one file in its container, so a `.gltf` is turned into a `.glb`
//! when it is imported ([`to_glb`]): every buffer goes into the binary chunk.
//! The pictures it names become texture assets, and its materials material
//! assets, at the same time; [`submeshes`] reads only the meshes.

use std::{borrow::Cow, path::Path};

use base64::Engine;
use glam::{Mat4, Vec2, Vec3};
use gltf::{
	Gltf,
	binary::{Glb, Header},
	json::{self, validation::USize64},
	mesh::Mode,
};
use zenith_error::{ZResult, ZenithError};

use super::mesh::{ModelImport, SubMesh, build_vertices};
use crate::assets::refuse_lfs_pointer;

fn bad(what: impl std::fmt::Display) -> ZenithError {
	ZenithError::InvalidAsset(format!("the glTF cannot be read: {what}"))
}

/// A glTF read for import: its document and the data of its buffers.
pub struct Source {
	pub document: gltf::Document,
	pub buffers: Vec<gltf::buffer::Data>,
}

/// Reads a `.gltf` (the buffers it names are looked for in `beside`) or a
/// `.glb`.
///
/// # Errors
/// `InvalidAsset` for what is not glTF, or a buffer that cannot be read.
pub fn read(bytes: &[u8], beside: &Path) -> ZResult<Source> {
	refuse_lfs_pointer(bytes, &"the glTF")?;
	let gltf = Gltf::from_slice(bytes).map_err(bad)?;
	let buffers = gltf::import_buffers(&gltf.document, Some(beside), gltf.blob).map_err(bad)?;
	Ok(Source {
		document: gltf.document,
		buffers,
	})
}

/// The model as one `.glb`: every buffer in its binary chunk.
///
/// # Errors
/// `InvalidAsset` if it cannot be written (a buffer of more than 4 GiB).
pub fn to_glb(source: &Source) -> ZResult<Vec<u8>> {
	let mut root = source.document.as_json().clone();
	let mut bin = Vec::new();
	let mut starts = Vec::with_capacity(source.buffers.len());
	for data in &source.buffers {
		starts.push(bin.len() as u64);
		bin.extend_from_slice(&data.0);
		// Every view starts at a multiple of four, as glTF wants.
		bin.resize(bin.len().next_multiple_of(4), 0);
	}
	for view in &mut root.buffer_views {
		let start = starts.get(view.buffer.value()).copied().unwrap_or(0);
		view.byte_offset = Some(USize64(view.byte_offset.map_or(0, |offset| offset.0) + start));
		view.buffer = json::Index::new(0);
	}
	root.buffers.truncate(1);
	if let Some(buffer) = root.buffers.first_mut() {
		buffer.byte_length = USize64(bin.len() as u64);
		buffer.uri = None;
	}
	let json = serde_json::to_vec(&root).map_err(|e| ZenithError::Json(e.to_string()))?;
	let length = u32::try_from(json.len() + bin.len()).map_err(|_| bad("the model is larger than 4 GiB"))?;
	Glb {
		header: Header {
			magic: *b"glTF",
			version: 2,
			length,
		},
		json: Cow::Owned(json),
		bin: (!bin.is_empty()).then_some(Cow::Owned(bin)),
	}
	.to_vec()
	.map_err(bad)
}

/// The encoded bytes of a picture of the source (a `.png`, a `.jpg`) and the
/// extension that suits them. `beside` is where a file it names is.
///
/// # Errors
/// `InvalidAsset` for a picture that cannot be found or read.
pub fn image_bytes(source: &Source, image: &gltf::Image<'_>, beside: &Path) -> ZResult<(Vec<u8>, String)> {
	let extension = |mime: &str| mime.rsplit('/').next().unwrap_or("png").replace("jpeg", "jpg");
	match image.source() {
		gltf::image::Source::View { view, mime_type } => {
			let buffer = source
				.buffers
				.get(view.buffer().index())
				.ok_or_else(|| bad("a picture in a buffer that is not there"))?;
			let bytes = buffer
				.0
				.get(view.offset()..view.offset() + view.length())
				.ok_or_else(|| bad("a picture beyond the end of its buffer"))?;
			Ok((bytes.to_vec(), extension(mime_type)))
		}
		gltf::image::Source::Uri { uri, mime_type } => {
			if let Some(data) = uri.strip_prefix("data:") {
				let (header, encoded) = data
					.split_once(";base64,")
					.ok_or_else(|| bad("a picture in a data: URI that is not base64"))?;
				let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(bad)?;
				return Ok((bytes, extension(mime_type.unwrap_or(header))));
			}
			let name = urlencoding::decode(uri).map_err(bad)?;
			let file = beside.join(name.as_ref());
			let bytes = std::fs::read(&file).map_err(|e| bad(format!("{}: {e}", file.display())))?;
			refuse_lfs_pointer(&bytes, &file.display())?;
			let extension = file
				.extension()
				.and_then(|extension| extension.to_str())
				.map_or_else(|| extension(mime_type.unwrap_or("image/png")), str::to_ascii_lowercase);
			Ok((bytes, extension))
		}
	}
}

/// The meshes of a `.glb`, each with the index of its glTF material. The
/// nodes of the scene place them: their transforms are applied, so a model
/// made of parts comes in whole.
///
/// # Errors
/// `InvalidAsset` for what is not a `.glb` this engine wrote, or one without
/// triangles.
pub fn submeshes(glb: &[u8], settings: &ModelImport) -> ZResult<Vec<(SubMesh, Option<usize>)>> {
	let gltf = Gltf::from_slice(glb).map_err(bad)?;
	let blob = gltf.blob.as_deref();
	let mut out = Vec::new();
	let scene = gltf.default_scene().or_else(|| gltf.scenes().next());
	if let Some(scene) = scene {
		let mut stack: Vec<_> = scene.nodes().map(|node| (node, Mat4::IDENTITY)).collect();
		while let Some((node, parent)) = stack.pop() {
			let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
			if let Some(mesh) = node.mesh() {
				add_mesh(&mesh, world, settings, blob, &mut out)?;
			}
			stack.extend(node.children().map(|child| (child, world)));
		}
	} else {
		// No scene: the meshes as they are.
		for mesh in gltf.meshes() {
			add_mesh(&mesh, Mat4::IDENTITY, settings, blob, &mut out)?;
		}
	}
	if out.is_empty() {
		return Err(ZenithError::InvalidAsset("the model has no triangles".into()));
	}
	Ok(out)
}

/// Adds the triangles of `mesh`, placed by `world`. `blob` is the binary chunk
/// of the `.glb`, the only buffer it has.
fn add_mesh(
	mesh: &gltf::Mesh<'_>,
	world: Mat4,
	settings: &ModelImport,
	blob: Option<&[u8]>,
	out: &mut Vec<(SubMesh, Option<usize>)>,
) -> ZResult<()> {
	let normal_matrix = world.inverse().transpose();
	// A mirroring transform turns the triangles inside out: their corners go
	// the other way round.
	let mirrored = world.determinant() < 0.0;
	let primitives = mesh.primitives().len();
	for primitive in mesh.primitives() {
		let name = mesh
			.name()
			.map_or_else(|| format!("mesh{}", mesh.index()), str::to_string);
		let name = if primitives > 1 {
			format!("{name}-{}", primitive.index())
		} else {
			name
		};
		if primitive.mode() != Mode::Triangles {
			zenith_log::warn!("{name}: {:?} are not drawn, only triangles", primitive.mode());
			continue;
		}
		let reader = primitive.reader(|buffer| match buffer.source() {
			gltf::buffer::Source::Bin => blob,
			gltf::buffer::Source::Uri(_) => None,
		});
		let Some(positions) = reader.read_positions() else {
			continue;
		};
		let positions: Vec<Vec3> = positions
			.map(|p| world.transform_point3(Vec3::from_array(p)) * settings.scale)
			.collect();
		if positions.is_empty() {
			continue;
		}
		let normals: Option<Vec<Vec3>> = reader.read_normals().map(|normals| {
			normals
				.map(|n| normal_matrix.transform_vector3(Vec3::from_array(n)).normalize_or_zero())
				.collect()
		});
		let uvs: Vec<Vec2> = reader
			.read_tex_coords(0)
			.map(|uvs| {
				uvs.into_f32()
					.map(|[u, v]| Vec2::new(u, if settings.flip_v { 1.0 - v } else { v }))
					.collect()
			})
			.unwrap_or_default();
		let count = u32::try_from(positions.len()).map_err(|_| bad("a mesh of more than 4 billion vertices"))?;
		let mut indices: Vec<u32> = reader
			.read_indices()
			.map_or_else(|| (0..count).collect(), |indices| indices.into_u32().collect());
		indices.truncate(indices.len() / 3 * 3);
		if indices.is_empty() {
			// Nothing to draw, and the GPU refuses an empty buffer.
			continue;
		}
		if indices.iter().any(|&i| i >= count) {
			return Err(bad(format!("{name}: an index past the last vertex")));
		}
		if mirrored {
			for triangle in indices.as_chunks_mut::<3>().0 {
				triangle.swap(1, 2);
			}
		}
		let normals = normals.filter(|normals| normals.len() == positions.len());
		let vertices = build_vertices(&positions, &uvs, normals.as_deref(), &indices);
		out.push((
			SubMesh {
				name,
				vertices,
				indices,
				material: None,
			},
			primitive.material().index(),
		));
	}
	Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
	use super::*;
	use crate::assets::{
		AssetId,
		import::mesh::{parse, tests::approx},
	};

	/// The buffer of a square: four positions (`f32`), four uvs, six `u16`
	/// indices, padding to 96 bytes, then `extra` (a picture).
	pub fn square_buffer(extra: &[u8]) -> Vec<u8> {
		let mut bytes = Vec::new();
		for value in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0] {
			bytes.extend_from_slice(&value.to_le_bytes());
		}
		for value in [0.0f32, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0] {
			bytes.extend_from_slice(&value.to_le_bytes());
		}
		for index in [0u16, 1, 2, 0, 2, 3] {
			bytes.extend_from_slice(&index.to_le_bytes());
		}
		bytes.resize(96, 0);
		bytes.extend_from_slice(extra);
		bytes
	}

	/// A `.gltf` of a square on a node moved by 10 along x, whose buffer is
	/// `buffer_uri`; `extra` bytes of a picture follow the square in it (a
	/// fourth view), if there are any.
	pub fn square_gltf(buffer_uri: &str, extra: usize, image: &str, material: &str) -> String {
		let length = 96 + extra;
		let picture_view = if extra == 0 {
			String::new()
		} else {
			format!(r#",{{ "buffer": 0, "byteOffset": 96, "byteLength": {extra} }}"#)
		};
		format!(
			r#"{{
  "asset": {{ "version": "2.0" }},
  "scene": 0,
  "scenes": [{{ "nodes": [0] }}],
  "nodes": [{{ "mesh": 0, "translation": [10, 0, 0] }}],
  "meshes": [{{ "name": "square", "primitives": [{{ "attributes": {{ "POSITION": 0, "TEXCOORD_0": 1 }}, "indices": 2, "material": 0 }}] }}],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] }},
    {{ "bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC2" }},
    {{ "bufferView": 2, "componentType": 5123, "count": 6, "type": "SCALAR" }}
  ],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": 48 }},
    {{ "buffer": 0, "byteOffset": 48, "byteLength": 32 }},
    {{ "buffer": 0, "byteOffset": 80, "byteLength": 12 }}{picture_view}
  ],
  "buffers": [{{ "uri": "{buffer_uri}", "byteLength": {length} }}],
  "images": [{image}],
  "textures": [{{ "source": 0 }}],
  "materials": [{material}]
}}"#
		)
	}

	pub const RED_MATERIAL: &str = r#"{ "name": "red", "pbrMetallicRoughness": { "baseColorFactor": [1, 0, 0, 1], "baseColorTexture": { "index": 0 }, "metallicFactor": 0.25, "roughnessFactor": 0.75 }, "normalTexture": { "index": 0 }, "alphaMode": "MASK", "alphaCutoff": 0.3, "doubleSided": true }"#;

	fn glb_of_square(dir: &Path) -> ZResult<Vec<u8>> {
		std::fs::write(dir.join("square.bin"), square_buffer(&[]))?;
		let text = square_gltf("square.bin", 0, r#"{ "uri": "red.png" }"#, RED_MATERIAL);
		to_glb(&read(text.as_bytes(), dir)?)
	}

	#[test]
	fn a_gltf_becomes_one_glb_and_its_meshes_are_placed_by_their_nodes() -> ZResult<()> {
		let dir = tempfile::tempdir()?;
		let glb = glb_of_square(dir.path())?;
		assert_eq!(glb.get(..4), Some(b"glTF".as_slice()));

		let material = AssetId::from_bytes([6; 16]);
		let settings = ModelImport {
			mesh_materials: vec![Some(material)],
			..ModelImport::for_format("glb")
		};
		let data = parse(&glb, &settings)?;
		let [mesh] = data.submeshes.as_slice() else {
			return Err(ZenithError::Other(format!("one mesh, not {}", data.submeshes.len())));
		};
		assert_eq!(mesh.name, "square");
		assert_eq!(mesh.indices, [0, 1, 2, 0, 2, 3]);
		assert_eq!(mesh.material, Some(material));
		assert!(
			mesh.vertices.iter().any(|v| approx(v.position, [11.0, 1.0, 0.0])),
			"moved by its node: {:?}",
			mesh.vertices
		);
		for vertex in &mesh.vertices {
			assert!(approx(vertex.normal, [0.0, 0.0, 1.0]), "made from the triangles");
			assert!(approx(vertex.tangent, [1.0, 0.0, 0.0]), "{vertex:?}");
		}
		let top = mesh.vertices.iter().find(|v| approx(v.position, [10.0, 1.0, 0.0]));
		assert_eq!(
			top.map(|v| v.uv),
			Some([0.0, 0.0]),
			"glTF counts v from the top already"
		);
		Ok(())
	}

	#[test]
	fn a_mirroring_node_turns_the_triangles_back() -> ZResult<()> {
		let dir = tempfile::tempdir()?;
		std::fs::write(dir.path().join("square.bin"), square_buffer(&[]))?;
		let text = square_gltf("square.bin", 0, r#"{ "uri": "red.png" }"#, RED_MATERIAL)
			.replace(r#""translation": [10, 0, 0]"#, r#""scale": [-1, 1, 1]"#);
		let glb = to_glb(&read(text.as_bytes(), dir.path())?)?;
		let data = parse(&glb, &ModelImport::for_format("glb"))?;
		let indices = data.submeshes.first().map(|m| m.indices.clone()).unwrap_or_default();
		assert_eq!(indices, [0, 2, 1, 0, 3, 2]);
		let normals: Vec<_> = data
			.submeshes
			.iter()
			.flat_map(|m| &m.vertices)
			.map(|v| v.normal)
			.collect();
		assert!(
			normals.iter().all(|n| approx(*n, [0.0, 0.0, 1.0])),
			"still facing out: {normals:?}"
		);
		Ok(())
	}
}
