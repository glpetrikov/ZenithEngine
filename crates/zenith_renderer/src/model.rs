//! Models: meshes with materials, read from a Wavefront `.obj` in the project.

use std::{io::Cursor, ops::Range};

use glam::{Vec2, Vec3};
use wgpu::util::DeviceExt;
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{Project, ZPath};

use crate::texture::Texture;

/// What the vertex shader reads for every vertex of a mesh.
///
/// The normal, the tangent and the bitangent make the *tangent space* of the
/// vertex: the directions of the texture lying on the surface. A normal map
/// keeps its normals in that space, so lighting needs all three.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ModelVertex {
	pub position: Vec3,
	pub tex_coords: Vec2,
	pub normal: Vec3,
	pub tangent: Vec3,
	pub bitangent: Vec3,
}

impl ModelVertex {
	#[must_use]
	pub const fn desc() -> wgpu::VertexBufferLayout<'static> {
		use std::mem::{offset_of, size_of};

		wgpu::VertexBufferLayout {
			array_stride: size_of::<Self>() as wgpu::BufferAddress,
			step_mode: wgpu::VertexStepMode::Vertex,
			attributes: &[
				wgpu::VertexAttribute {
					offset: offset_of!(Self, position) as wgpu::BufferAddress,
					shader_location: 0,
					format: wgpu::VertexFormat::Float32x3,
				},
				wgpu::VertexAttribute {
					offset: offset_of!(Self, tex_coords) as wgpu::BufferAddress,
					shader_location: 1,
					format: wgpu::VertexFormat::Float32x2,
				},
				wgpu::VertexAttribute {
					offset: offset_of!(Self, normal) as wgpu::BufferAddress,
					shader_location: 2,
					format: wgpu::VertexFormat::Float32x3,
				},
				wgpu::VertexAttribute {
					offset: offset_of!(Self, tangent) as wgpu::BufferAddress,
					shader_location: 3,
					format: wgpu::VertexFormat::Float32x3,
				},
				wgpu::VertexAttribute {
					offset: offset_of!(Self, bitangent) as wgpu::BufferAddress,
					shader_location: 4,
					format: wgpu::VertexFormat::Float32x3,
				},
			],
		}
	}
}

pub struct Material {
	pub name: String,
	pub diffuse_texture: Texture,
	pub normal_texture: Texture,
	pub bind_group: wgpu::BindGroup,
}

impl Material {
	/// The layout of [`Material::bind_group`]: the color, then the normal map,
	/// each with its sampler.
	#[must_use]
	pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
		let texture = |binding| wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture {
				multisampled: false,
				view_dimension: wgpu::TextureViewDimension::D2,
				sample_type: wgpu::TextureSampleType::Float { filterable: true },
			},
			count: None,
		};
		let sampler = |binding| wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
			count: None,
		};

		device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("Material Bind Group Layout"),
			entries: &[texture(0), sampler(1), texture(2), sampler(3)],
		})
	}

	#[must_use]
	pub fn new(
		device: &wgpu::Device,
		name: &str,
		diffuse_texture: Texture,
		normal_texture: Texture,
		layout: &wgpu::BindGroupLayout,
	) -> Self {
		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some(name),
			layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(&diffuse_texture.view),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::Sampler(&diffuse_texture.sampler),
				},
				wgpu::BindGroupEntry {
					binding: 2,
					resource: wgpu::BindingResource::TextureView(&normal_texture.view),
				},
				wgpu::BindGroupEntry {
					binding: 3,
					resource: wgpu::BindingResource::Sampler(&normal_texture.sampler),
				},
			],
		});

		Self {
			name: name.to_string(),
			diffuse_texture,
			normal_texture,
			bind_group,
		}
	}
}

pub struct Mesh {
	pub name: String,
	pub vertex_buffer: wgpu::Buffer,
	pub index_buffer: wgpu::Buffer,
	pub num_elements: u32,
	/// Index into [`Model::materials`].
	pub material: usize,
}

impl Mesh {
	/// Draws the mesh with the material set by the caller (or none at all, for
	/// a pipeline that does not read one).
	pub fn draw_geometry(&self, pass: &mut wgpu::RenderPass<'_>, instances: Range<u32>) {
		pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
		pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
		pass.draw_indexed(0..self.num_elements, 0, instances);
	}
}

pub struct Model {
	pub meshes: Vec<Mesh>,
	pub materials: Vec<Material>,
}

impl Model {
	/// ## Loads a model from the project
	///
	/// `path` is the `.obj`; the `.mtl` it names, and the textures that one
	/// names, are looked for in the same folder.
	///
	/// ## Errors
	/// The project's error if a file cannot be read, `Other` if the `.obj` or
	/// the `.mtl` is not valid, `ImageDecodeFailed` for a bad texture.
	pub fn load(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		project: &Project,
		path: &ZPath,
		layout: &wgpu::BindGroupLayout,
	) -> ZResult<Self> {
		let folder = path.parent();
		let beside = |name: &str| -> ZResult<ZPath> {
			folder
				.as_ref()
				.map_or_else(|| ZPath::from_parts(path.scheme(), name), |f| f.join(name))
		};

		let obj_text = project.read_string(path)?;
		let load_options = tobj::LoadOptions {
			// Quads and polygons become triangles, and every vertex gets one
			// index for its position, uv and normal together.
			triangulate: true,
			single_index: true,
			..Default::default()
		};
		let (obj_models, obj_materials) = tobj::load_obj_buf(&mut Cursor::new(obj_text), &load_options, |mtl_path| {
			let text = mtl_path
				.to_str()
				.ok_or(tobj::LoadError::OpenFileFailed)
				.and_then(|name| {
					beside(name).and_then(|p| project.read_string(&p)).map_err(|e| {
						zenith_log::warn!("Cannot read the material file {}: {e}", mtl_path.display());
						tobj::LoadError::OpenFileFailed
					})
				})?;
			tobj::load_mtl_buf(&mut Cursor::new(text))
		})
		.map_err(|e| ZenithError::Other(format!("{path}: {e}")))?;
		// A model without a material file is fine: it gets the plain one below.
		let obj_materials = obj_materials.unwrap_or_else(|e| {
			zenith_log::warn!("{path}: the materials are not used: {e}");
			Vec::new()
		});

		let load_texture = |file: &str, is_normal_map: bool| -> ZResult<Texture> {
			let bytes = project.read_bytes(&beside(file)?)?;
			Texture::from_bytes(device, queue, &bytes, file, is_normal_map)
		};

		let mut materials = Vec::new();
		for m in &obj_materials {
			let diffuse = match &m.diffuse_texture {
				Some(file) => load_texture(file, false)?,
				None => Texture::from_color(device, queue, [255; 4], "White", false),
			};
			let normal = match &m.normal_texture {
				Some(file) => load_texture(file, true)?,
				None => Texture::from_color(device, queue, [128, 128, 255, 255], "Flat Normal", true),
			};
			materials.push(Material::new(device, &m.name, diffuse, normal, layout));
		}
		// For a mesh that names no material, or one that does not exist.
		let plain = materials.len();
		materials.push(Material::new(
			device,
			"Plain",
			Texture::from_color(device, queue, [255; 4], "White", false),
			Texture::from_color(device, queue, [128, 128, 255, 255], "Flat Normal", true),
			layout,
		));

		let meshes: Vec<Mesh> = obj_models
			.into_iter()
			// A mesh without triangles cannot be drawn (the GPU refuses an
			// empty buffer), and has nothing to show.
			.filter(|m| !m.mesh.indices.is_empty())
			.map(|m| {
				let vertices = build_vertices(&m.mesh);
				let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
					label: Some(&format!("{path} {} Vertex Buffer", m.name)),
					contents: bytemuck::cast_slice(&vertices),
					usage: wgpu::BufferUsages::VERTEX,
				});
				let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
					label: Some(&format!("{path} {} Index Buffer", m.name)),
					contents: bytemuck::cast_slice(&m.mesh.indices),
					usage: wgpu::BufferUsages::INDEX,
				});
				Mesh {
					name: m.name,
					vertex_buffer,
					index_buffer,
					num_elements: m.mesh.indices.len() as u32,
					material: m.mesh.material_id.filter(|&id| id < plain).unwrap_or(plain),
				}
			})
			.collect();

		if meshes.is_empty() {
			return Err(ZenithError::Other(format!(
				"{path}: the model has no triangles (a Git LFS pointer instead of the file? run `git lfs pull`)"
			)));
		}

		Ok(Self { meshes, materials })
	}

	/// Draws every mesh with its own material, which goes to bind group 0.
	pub fn draw_instanced(&self, pass: &mut wgpu::RenderPass<'_>, instances: Range<u32>) {
		for mesh in &self.meshes {
			pass.set_bind_group(0, &self.materials[mesh.material].bind_group, &[]);
			mesh.draw_geometry(pass, instances.clone());
		}
	}

	/// Draws every mesh and no material: for a pipeline that does not read one.
	pub fn draw_geometry(&self, pass: &mut wgpu::RenderPass<'_>, instances: Range<u32>) {
		for mesh in &self.meshes {
			mesh.draw_geometry(pass, instances.clone());
		}
	}
}

/// The vertices of a loaded mesh, with the tangent space worked out.
fn build_vertices(mesh: &tobj::Mesh) -> Vec<ModelVertex> {
	let count = mesh.positions.len() / 3;
	let has_uv = mesh.texcoords.len() >= count * 2;
	let has_normals = mesh.normals.len() >= count * 3;

	let mut vertices: Vec<ModelVertex> = (0..count)
		.map(|i| ModelVertex {
			position: Vec3::new(
				mesh.positions[i * 3],
				mesh.positions[i * 3 + 1],
				mesh.positions[i * 3 + 2],
			),
			// An .obj counts `v` from the bottom of the image, wgpu from the
			// top.
			tex_coords: if has_uv {
				Vec2::new(mesh.texcoords[i * 2], 1.0 - mesh.texcoords[i * 2 + 1])
			} else {
				Vec2::ZERO
			},
			normal: if has_normals {
				Vec3::new(mesh.normals[i * 3], mesh.normals[i * 3 + 1], mesh.normals[i * 3 + 2])
			} else {
				Vec3::ZERO
			},
			tangent: Vec3::ZERO,
			bitangent: Vec3::ZERO,
		})
		.collect();

	// A vertex shared by several triangles gets the average of their tangent
	// spaces, so the light does not break at the edges between them.
	let mut triangles_included = vec![0_u32; count];
	let mut face_normals = vec![Vec3::ZERO; count];

	for &[a, b, c] in mesh.indices.as_chunks::<3>().0 {
		let (i0, i1, i2) = (a as usize, b as usize, c as usize);
		let (v0, v1, v2) = (vertices[i0], vertices[i1], vertices[i2]);

		let delta_pos1 = v1.position - v0.position;
		let delta_pos2 = v2.position - v0.position;
		let face_normal = delta_pos1.cross(delta_pos2);
		for i in [i0, i1, i2] {
			face_normals[i] += face_normal;
		}

		// Solving   delta_pos = delta_uv.x * tangent + delta_uv.y * bitangent
		// for the tangent and the bitangent.
		let delta_uv1 = v1.tex_coords - v0.tex_coords;
		let delta_uv2 = v2.tex_coords - v0.tex_coords;
		let det = delta_uv1.x.mul_add(delta_uv2.y, -(delta_uv1.y * delta_uv2.x));
		if det.abs() < f32::EPSILON {
			continue;
		}
		let r = 1.0 / det;
		let tangent = (delta_pos1 * delta_uv2.y - delta_pos2 * delta_uv1.y) * r;
		// Negated: the image counts `y` downwards, a normal map (OpenGL style)
		// has its green channel pointing up.
		let bitangent = (delta_pos2 * delta_uv1.x - delta_pos1 * delta_uv2.x) * -r;

		for i in [i0, i1, i2] {
			vertices[i].tangent += tangent;
			vertices[i].bitangent += bitangent;
			triangles_included[i] += 1;
		}
	}

	for (i, v) in vertices.iter_mut().enumerate() {
		if !has_normals {
			v.normal = face_normals[i].normalize_or_zero();
		}
		let n = triangles_included[i];
		if n > 0 {
			let k = 1.0 / n as f32;
			v.tangent = (v.tangent * k).normalize_or_zero();
			v.bitangent = (v.bitangent * k).normalize_or_zero();
		}
		if v.tangent == Vec3::ZERO {
			// No texture to follow (or a degenerate one): any pair of
			// directions across the normal will do.
			let helper = if v.normal.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
			v.tangent = v.normal.cross(helper).normalize_or_zero();
			v.bitangent = v.normal.cross(v.tangent);
		}
	}

	vertices
}
