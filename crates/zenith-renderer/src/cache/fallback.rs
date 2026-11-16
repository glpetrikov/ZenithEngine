//! What is drawn in place of an asset that is lost or cannot be used, and
//! the textures a material without its own gets.

use glam::{Vec2, Vec3};

use super::{
	materials::GpuMaterial,
	meshes::{GpuMesh, GpuSubMesh},
	pipelines::{Pipelines, SamplerKey},
	textures::{GpuTexture, TexturePixels},
};
use crate::assets::{
	import::mesh::{self, MeshData, SubMesh},
	json::MaterialData,
};

/// The stand-ins. They are made with the cache and never evicted.
#[derive(Debug)]
pub struct Fallbacks {
	/// A model that is lost: a cube of one unit.
	pub cube: GpuMesh,
	/// A texture that is lost: a magenta and black checker.
	pub checker: GpuTexture,
	/// The color texture of a material without one.
	pub white: GpuTexture,
	/// The normal map of a material without one (or with one that is lost):
	/// the normal does not tilt.
	pub flat_normal: GpuTexture,
	/// A material that is lost: plain white.
	pub material: GpuMaterial,
}

impl Fallbacks {
	pub(crate) fn new(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		pipelines: &mut Pipelines,
		material_layout: &wgpu::BindGroupLayout,
	) -> Self {
		let mut one_color = |label: &str, rgba: [u8; 4], format| {
			GpuTexture::create_unchecked(
				device,
				queue,
				pipelines,
				label,
				&TexturePixels {
					width: 1,
					height: 1,
					format,
					bytes: &rgba,
					mipmaps: false,
					sampler: SamplerKey {
						address_mode: wgpu::AddressMode::Repeat,
						..SamplerKey::LINEAR_CLAMP
					},
				},
			)
		};
		let white = one_color("Zenith White", [255; 4], wgpu::TextureFormat::Rgba8UnormSrgb);
		let flat_normal = one_color(
			"Zenith Flat Normal",
			[128, 128, 255, 255],
			wgpu::TextureFormat::Rgba8Unorm,
		);
		let checker = GpuTexture::create_unchecked(
			device,
			queue,
			pipelines,
			"Zenith Missing Texture",
			&TexturePixels {
				width: CHECKER,
				height: CHECKER,
				format: wgpu::TextureFormat::Rgba8UnormSrgb,
				bytes: &checker_pixels(),
				mipmaps: false,
				sampler: SamplerKey {
					address_mode: wgpu::AddressMode::Repeat,
					mag_filter: wgpu::FilterMode::Nearest,
					min_filter: wgpu::FilterMode::Nearest,
					mipmap_filter: wgpu::MipmapFilterMode::Nearest,
					compare: None,
				},
			},
		);
		let material = GpuMaterial::create(
			device,
			material_layout,
			"Zenith Standard Material",
			&MaterialData::default(),
			&white,
			&flat_normal,
		);
		let cube = cube();
		let (vertices, indices) = cube.submeshes.first().map_or((&[][..], &[][..]), |mesh| {
			(mesh.vertices.as_slice(), mesh.indices.as_slice())
		});
		Self {
			cube: GpuMesh::from_parts(
				device,
				"Zenith Missing Model",
				vertices,
				indices,
				vec![GpuSubMesh {
					name: "cube".into(),
					indices: 0..indices.len() as u32,
					base_vertex: 0,
					material: None,
				}],
			),
			checker,
			white,
			flat_normal,
			material,
		}
	}
}

/// The side of the checker, in squares of one pixel.
const CHECKER: u32 = 8;

fn checker_pixels() -> Vec<u8> {
	(0..CHECKER * CHECKER)
		.flat_map(|i| {
			if (i % CHECKER + i / CHECKER).is_multiple_of(2) {
				[255, 0, 255, 255]
			} else {
				[0, 0, 0, 255]
			}
		})
		.collect()
}

/// A cube of one unit around the origin: four vertices a face, so each face
/// has its own normal and the whole texture.
pub fn cube() -> MeshData {
	// The normal of a face, then two directions across it with `u × v = n`,
	// so the corners below go counter-clockwise seen from outside.
	let faces = [
		(Vec3::X, Vec3::NEG_Z, Vec3::Y),
		(Vec3::NEG_X, Vec3::Z, Vec3::Y),
		(Vec3::Y, Vec3::X, Vec3::NEG_Z),
		(Vec3::NEG_Y, Vec3::X, Vec3::Z),
		(Vec3::Z, Vec3::X, Vec3::Y),
		(Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
	];
	let mut positions = Vec::new();
	let mut uvs = Vec::new();
	let mut normals = Vec::new();
	let mut indices = Vec::new();
	for (face, (n, u, v)) in (0_u32..).zip(faces) {
		for (du, dv, uv) in [
			(-1.0, -1.0, Vec2::new(0.0, 1.0)),
			(1.0, -1.0, Vec2::new(1.0, 1.0)),
			(1.0, 1.0, Vec2::new(1.0, 0.0)),
			(-1.0, 1.0, Vec2::new(0.0, 0.0)),
		] {
			positions.push((n + u * du + v * dv) * 0.5);
			uvs.push(uv);
			normals.push(n);
		}
		let first = face * 4;
		indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
	}
	MeshData {
		submeshes: vec![SubMesh {
			name: "cube".into(),
			vertices: mesh::build_vertices(&positions, &uvs, Some(&normals), &indices),
			indices,
			material: None,
		}],
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_triangles_of_the_cube_face_outwards() {
		let cube = cube();
		assert_eq!(cube.submeshes.len(), 1, "one mesh");
		let Some(mesh) = cube.submeshes.first() else {
			return;
		};
		assert_eq!((mesh.vertices.len(), mesh.indices.len()), (24, 36));
		for &[a, b, c] in mesh.indices.as_chunks::<3>().0 {
			let [p0, p1, p2] = [a, b, c].map(|i| Vec3::from(mesh.vertices[i as usize].position));
			let facing = (p1 - p0).cross(p2 - p0).normalize();
			let normal = Vec3::from(mesh.vertices[a as usize].normal);
			assert!(facing.abs_diff_eq(normal, 1e-5), "{facing} against {normal}");
			let center = (p0 + p1 + p2) / 3.0;
			assert!(center.dot(normal) > 0.0, "on the outside");
		}
	}

	#[test]
	fn the_checker_alternates() {
		let pixels = checker_pixels();
		assert_eq!(pixels.len(), (CHECKER * CHECKER * 4) as usize);
		assert_eq!(pixels.get(..4), Some([255, 0, 255, 255].as_slice()));
		assert_eq!(pixels.get(4..8), Some([0, 0, 0, 255].as_slice()));
		let next_row = (CHECKER * 4) as usize;
		assert_eq!(pixels.get(next_row..next_row + 4), Some([0, 0, 0, 255].as_slice()));
	}
}
