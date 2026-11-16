//! What the editor (and the command line) does with assets: importing them,
//! copying, moving, replacing a source, giving a file an id of its own. Each
//! keeps the registry up to date.
//!
//! The sources come from the computer (`std::path::Path`); the assets go into
//! the project (`ZPath`).

use std::{collections::HashMap, path::Path};

use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{EntryKind, Project, ZPath};

use super::{
	AssetId, AssetKind, AssetRegistry, container,
	import::{
		gltf,
		image::{self, TextureImport, TextureUsage},
		mesh::{self, ModelImport},
		obj,
	},
	json::{self, BlendMode, MaterialData},
	refuse_lfs_pointer,
	registry::read_id,
};

/// An asset an operation made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Imported {
	pub id: AssetId,
	pub kind: AssetKind,
	pub path: ZPath,
}

fn file_name(source: &Path) -> ZResult<&str> {
	source
		.file_name()
		.and_then(|name| name.to_str())
		.ok_or_else(|| ZenithError::InvalidAsset(format!("{} has no usable file name", source.display())))
}

/// The name of a file without its extension, and its extension.
fn split_name(name: &str) -> (&str, &str) {
	name.rsplit_once('.').unwrap_or((name, ""))
}

fn read_source(source: &Path) -> ZResult<Vec<u8>> {
	let bytes = std::fs::read(source).map_err(|e| ZenithError::Other(format!("{}: {e}", source.display())))?;
	refuse_lfs_pointer(&bytes, &source.display())?;
	Ok(bytes)
}

/// `folder/<stem>.<extension of kind>`.
fn destination(folder: &ZPath, stem: &str, kind: AssetKind) -> ZResult<ZPath> {
	folder.join(&format!("{stem}.{}", kind.extension()))
}

fn already_exists(path: &ZPath) -> ZenithError {
	ZenithError::ProjectAlreadyExists(format!("{path}: replace its source, or remove it first"))
}

/// Imports a picture into `folder` as `<name>.zenithtexture`. `usage` is
/// guessed from the name when it is `None` (`normal` in it, an `.hdr`).
///
/// # Errors
/// `ProjectAlreadyExists` if the texture is already there, `ImageDecodeFailed`
/// for a file that is not a picture, or an error of reading or writing.
pub fn import_texture(
	project: &Project,
	registry: &mut AssetRegistry,
	source: &Path,
	folder: &ZPath,
	usage: Option<TextureUsage>,
) -> ZResult<Imported> {
	let name = file_name(source)?;
	let (stem, extension) = split_name(name);
	let path = destination(folder, stem, AssetKind::Texture)?;
	if project.exists(&path)? {
		return Err(already_exists(&path));
	}
	let bytes = read_source(source)?;
	let mut writes = Writes::new(project);
	writes.texture(
		registry,
		path,
		&bytes,
		extension,
		usage.unwrap_or_else(|| TextureUsage::guess(name)),
	)
}

/// The asset of `kind` at `path`, if one is already there. A file that is
/// there but cannot be used is an error: it is not written over.
fn existing(project: &Project, path: &ZPath, kind: AssetKind) -> ZResult<Option<Imported>> {
	if !project.exists(path)? {
		return Ok(None);
	}
	let id = read_id(project, path, kind).map_err(|e| {
		ZenithError::InvalidAsset(format!(
			"{path} is there but cannot be used ({e}): fix it or remove it first"
		))
	})?;
	Ok(Some(Imported {
		id,
		kind,
		path: path.clone(),
	}))
}

/// The files one import writes, so that a failed import takes them back: a
/// model that comes in half (some textures, no model) is worse than none.
struct Writes<'a> {
	project: &'a Project,
	written: Vec<ZPath>,
}

impl<'a> Writes<'a> {
	const fn new(project: &'a Project) -> Self {
		Self {
			project,
			written: Vec::new(),
		}
	}

	fn write(
		&mut self,
		registry: &mut AssetRegistry,
		path: ZPath,
		kind: AssetKind,
		id: AssetId,
		bytes: &[u8],
	) -> ZResult<Imported> {
		self.project.write(&path, bytes)?;
		self.written.push(path.clone());
		registry.insert(path.clone(), kind, id);
		Ok(Imported { id, kind, path })
	}

	/// A texture asset of the picture `bytes` (whose format `extension`
	/// names) at `path`.
	fn texture(
		&mut self,
		registry: &mut AssetRegistry,
		path: ZPath,
		bytes: &[u8],
		extension: &str,
		usage: TextureUsage,
	) -> ZResult<Imported> {
		image::check(bytes)?;
		let settings = TextureImport::for_usage(usage, extension);
		let id = AssetId::generate()?;
		let file = container::write(AssetKind::Texture, id, &settings, bytes)?;
		self.write(registry, path, AssetKind::Texture, id, &file)
	}

	/// The material named `name` in `folder` if it is already there, otherwise
	/// a new one: `make` fills it in.
	fn material(
		&mut self,
		registry: &mut AssetRegistry,
		folder: &ZPath,
		name: &str,
		fallback: &str,
		make: impl FnOnce(MaterialData) -> MaterialData,
	) -> ZResult<Imported> {
		let path = destination(folder, name, AssetKind::Material)
			.or_else(|_| destination(folder, fallback, AssetKind::Material))?;
		if let Some(imported) = existing(self.project, &path, AssetKind::Material)? {
			return Ok(imported);
		}
		let data = make(MaterialData::new(AssetId::generate()?));
		self.write(
			registry,
			path,
			AssetKind::Material,
			data.uuid,
			data.to_text()?.as_bytes(),
		)
	}

	/// The model asset at `path`.
	fn model(
		&mut self,
		registry: &mut AssetRegistry,
		path: ZPath,
		settings: &ModelImport,
		source: &[u8],
	) -> ZResult<Imported> {
		let id = AssetId::generate()?;
		let file = container::write(AssetKind::Model, id, settings, source)?;
		self.write(registry, path, AssetKind::Model, id, &file)
	}

	/// Takes back what was written: the files go to the trash of the project
	/// (nothing of a project is deleted for good), and out of the registry.
	fn undo(self, registry: &mut AssetRegistry) {
		for path in self.written.iter().rev() {
			if let Err(e) = self.project.remove(path) {
				zenith_log::warn!("{path} was made by an import that failed, and cannot be taken back: {e}");
			}
			registry.forget(path);
		}
	}
}

/// Imports a model into `folder`, with its materials and textures: an `.obj`
/// (and its `.mtl`), a `.gltf` or a `.glb`.
///
/// The pictures it names become `.zenithtexture`s, its materials
/// `.zenithmaterial`s, and the model a `.zenithmodel` whose meshes refer to
/// them. A texture or a material that is already in `folder` (another model of
/// the same set) is used as it is. A `.gltf` is kept as a `.glb`, so the model
/// is one file.
///
/// The model is read before anything is written, and if a texture or a
/// material fails, what was written for it goes to the trash: an import makes
/// all of it or nothing.
///
/// Returns what was made or used: the textures, the materials, the model last.
///
/// # Errors
/// `ProjectAlreadyExists` if the model is already there, `InvalidAsset` for a
/// model that cannot be read or a file in the way that is not an asset,
/// `ImageDecodeFailed` for a texture that is not a picture, or an error of
/// reading or writing.
pub fn import_model(
	project: &Project,
	registry: &mut AssetRegistry,
	source: &Path,
	folder: &ZPath,
) -> ZResult<Vec<Imported>> {
	let (stem, extension) = split_name(file_name(source)?);
	let model_path = destination(folder, stem, AssetKind::Model)?;
	if project.exists(&model_path)? {
		return Err(already_exists(&model_path));
	}
	let beside = source.parent().map(Path::to_path_buf).unwrap_or_default();
	let mut writes = Writes::new(project);
	let result = match extension.to_ascii_lowercase().as_str() {
		"obj" => import_obj(&mut writes, registry, source, &beside, folder, model_path),
		"gltf" | "glb" => import_gltf(&mut writes, registry, source, &beside, folder, model_path),
		_ => Err(ZenithError::InvalidAsset(format!(
			"{}: a model is an .obj, a .gltf or a .glb",
			source.display()
		))),
	};
	if result.is_err() {
		writes.undo(registry);
	}
	result
}

fn import_obj(
	writes: &mut Writes<'_>,
	registry: &mut AssetRegistry,
	source: &Path,
	beside: &Path,
	folder: &ZPath,
	model_path: ZPath,
) -> ZResult<Vec<Imported>> {
	let (stem, _) = split_name(file_name(source)?);
	let bytes = read_source(source)?;
	let (models, materials) = obj::read_obj(&bytes, |mtl| tobj::load_mtl(beside.join(mtl)))?;
	let materials = materials.unwrap_or_else(|e| {
		zenith_log::warn!("{}: its materials are not imported: {e}", source.display());
		Vec::new()
	});

	let mut made = Vec::new();
	let mut textures = Textures {
		folder,
		beside,
		ids: HashMap::new(),
		claimed: HashMap::new(),
	};

	let mut material_ids = Vec::new();
	for (at, material) in materials.iter().enumerate() {
		let base_color_texture = textures.get(
			writes,
			registry,
			material.diffuse_texture.as_deref(),
			TextureUsage::Color,
			&mut made,
		)?;
		let normal_texture = textures.get(
			writes,
			registry,
			material.normal_texture.as_deref(),
			TextureUsage::Normal,
			&mut made,
		)?;
		let fallback = format!("{stem}-{at}");
		let name = if material.name.is_empty() {
			&fallback
		} else {
			&material.name
		};
		let imported = writes.material(registry, folder, name, &fallback, |data| {
			let [r, g, b] = material.diffuse.unwrap_or([1.0; 3]);
			MaterialData {
				base_color: [r, g, b, material.dissolve.unwrap_or(1.0)],
				base_color_texture,
				normal_texture,
				..data
			}
		})?;
		material_ids.push(imported.id);
		made.push(imported);
	}

	let settings = ModelImport {
		mesh_materials: models
			.iter()
			.map(|model| model.mesh.material_id.and_then(|at| material_ids.get(at).copied()))
			.collect(),
		..ModelImport::for_format("obj")
	};
	made.push(writes.model(registry, model_path, &settings, &bytes)?);
	Ok(made)
}

fn import_gltf(
	writes: &mut Writes<'_>,
	registry: &mut AssetRegistry,
	source: &Path,
	beside: &Path,
	folder: &ZPath,
	model_path: ZPath,
) -> ZResult<Vec<Imported>> {
	let (stem, _) = split_name(file_name(source)?);
	let model = gltf::read(&read_source(source)?, beside)?;
	// The meshes are read first: a model that cannot be drawn makes nothing.
	let glb = gltf::to_glb(&model)?;
	let settings = ModelImport::for_format("glb");
	let meshes = gltf::submeshes(&glb, &settings)?;

	let mut made = Vec::new();
	let mut textures = GltfTextures {
		folder,
		stem,
		beside,
		model: &model,
		ids: HashMap::new(),
		claimed: HashMap::new(),
	};
	let mut material_ids = Vec::new();
	for material in model.document.materials() {
		let pbr = material.pbr_metallic_roughness();
		let base_color_texture = textures.get(
			writes,
			registry,
			pbr.base_color_texture().map(|info| info.texture()),
			TextureUsage::Color,
			&mut made,
		)?;
		let normal_texture = textures.get(
			writes,
			registry,
			material.normal_texture().map(|info| info.texture()),
			TextureUsage::Normal,
			&mut made,
		)?;
		let fallback = format!("{stem}-{}", material.index().unwrap_or_default());
		let imported = writes.material(
			registry,
			folder,
			material.name().unwrap_or(&fallback),
			&fallback,
			|data| MaterialData {
				base_color: pbr.base_color_factor(),
				base_color_texture,
				normal_texture,
				metallic: pbr.metallic_factor(),
				roughness: pbr.roughness_factor(),
				blend: match material.alpha_mode() {
					::gltf::material::AlphaMode::Opaque => BlendMode::Opaque,
					::gltf::material::AlphaMode::Mask => BlendMode::Mask,
					::gltf::material::AlphaMode::Blend => BlendMode::Blend,
				},
				alpha_cutoff: material.alpha_cutoff().unwrap_or(data.alpha_cutoff),
				double_sided: material.double_sided(),
				..data
			},
		)?;
		material_ids.push(imported.id);
		made.push(imported);
	}

	let settings = ModelImport {
		mesh_materials: meshes
			.iter()
			.map(|(_, material)| material.and_then(|at| material_ids.get(at).copied()))
			.collect(),
		..settings
	};
	made.push(writes.model(registry, model_path, &settings, &glb)?);
	Ok(made)
}

/// The texture at `wanted`, or at `fallback` when another picture of this
/// import took `wanted` already: two pictures that share a name (`color.png`
/// in two folders) stay two textures.
fn free_path(
	claimed: &HashMap<ZPath, String>,
	key: &str,
	wanted: ZPath,
	fallback: impl FnOnce() -> ZResult<ZPath>,
) -> ZResult<ZPath> {
	match claimed.get(&wanted) {
		Some(owner) if owner != key => fallback(),
		_ => Ok(wanted),
	}
}

/// The pictures of a glTF being imported, each imported once.
struct GltfTextures<'a> {
	folder: &'a ZPath,
	/// The name of the model, for pictures that have none.
	stem: &'a str,
	/// The folder of the `.gltf`, where the files it names are.
	beside: &'a Path,
	model: &'a gltf::Source,
	/// By the index of the picture.
	ids: HashMap<usize, AssetId>,
	/// The texture paths this import gave out, and the picture each is of.
	claimed: HashMap<ZPath, String>,
}

impl GltfTextures<'_> {
	fn get(
		&mut self,
		writes: &mut Writes<'_>,
		registry: &mut AssetRegistry,
		texture: Option<::gltf::Texture<'_>>,
		usage: TextureUsage,
		made: &mut Vec<Imported>,
	) -> ZResult<Option<AssetId>> {
		let Some(image) = texture.map(|texture| texture.source()) else {
			return Ok(None);
		};
		if let Some(id) = self.ids.get(&image.index()) {
			return Ok(Some(*id));
		}
		let fallback = || {
			destination(
				self.folder,
				&format!("{}-image{}", self.stem, image.index()),
				AssetKind::Texture,
			)
		};
		let named = image.name().map(str::to_string).or_else(|| match image.source() {
			::gltf::image::Source::Uri { uri, .. } if !uri.starts_with("data:") => Path::new(uri)
				.file_stem()
				.and_then(|stem| stem.to_str())
				.map(str::to_string),
			_ => None,
		});
		let key = image.index().to_string();
		let path = match named.and_then(|name| destination(self.folder, &name, AssetKind::Texture).ok()) {
			Some(wanted) => free_path(&self.claimed, &key, wanted, fallback)?,
			None => fallback()?,
		};
		let imported = if let Some(imported) = existing(writes.project, &path, AssetKind::Texture)? {
			imported
		} else {
			let (bytes, extension) = gltf::image_bytes(self.model, &image, self.beside)?;
			writes.texture(registry, path.clone(), &bytes, &extension, usage)?
		};
		self.claimed.insert(path, key);
		self.ids.insert(image.index(), imported.id);
		let id = imported.id;
		made.push(imported);
		Ok(Some(id))
	}
}

/// The textures of an `.obj` being imported, each imported once.
struct Textures<'a> {
	folder: &'a ZPath,
	/// The folder of the `.obj`, where the `.mtl` names its textures.
	beside: &'a Path,
	/// By the name the `.mtl` gives.
	ids: HashMap<String, AssetId>,
	/// The texture paths this import gave out, and the name each is of.
	claimed: HashMap<ZPath, String>,
}

impl Textures<'_> {
	/// The texture a material names, imported (or found in the folder) the
	/// first time it is named.
	fn get(
		&mut self,
		writes: &mut Writes<'_>,
		registry: &mut AssetRegistry,
		name: Option<&str>,
		usage: TextureUsage,
		made: &mut Vec<Imported>,
	) -> ZResult<Option<AssetId>> {
		let Some(name) = name else {
			return Ok(None);
		};
		if let Some(id) = self.ids.get(name) {
			return Ok(Some(*id));
		}
		let file = self.beside.join(name);
		let (stem, extension) = split_name(file_name(&file)?);
		let count = self.claimed.len();
		let path = free_path(
			&self.claimed,
			name,
			destination(self.folder, stem, AssetKind::Texture)?,
			|| destination(self.folder, &format!("{stem}-{count}"), AssetKind::Texture),
		)?;
		let imported = if let Some(imported) = existing(writes.project, &path, AssetKind::Texture)? {
			imported
		} else {
			writes.texture(registry, path.clone(), &read_source(&file)?, extension, usage)?
		};
		self.claimed.insert(path, name.to_string());
		self.ids.insert(name.to_string(), imported.id);
		let id = imported.id;
		made.push(imported);
		Ok(Some(id))
	}
}

fn same_kind(from: &ZPath, to: &ZPath) -> ZResult<AssetKind> {
	let kind = AssetKind::of_path(from).ok_or_else(|| ZenithError::InvalidAsset(format!("{from} is not an asset")))?;
	if AssetKind::of_path(to) != Some(kind) {
		return Err(ZenithError::InvalidAsset(format!(
			"{to} must keep the extension .{}",
			kind.extension()
		)));
	}
	Ok(kind)
}

/// The bytes of the asset file `bytes` with the id `id`.
fn with_id(bytes: &[u8], kind: AssetKind, id: AssetId) -> ZResult<Vec<u8>> {
	if kind.is_binary() {
		let mut bytes = bytes.to_vec();
		container::set_id(&mut bytes, id)?;
		Ok(bytes)
	} else {
		let text = std::str::from_utf8(bytes).map_err(|e| ZenithError::InvalidAsset(format!("not UTF-8 JSON: {e}")))?;
		Ok(json::set_id(text, id)?.into_bytes())
	}
}

/// Copies an asset. The copy gets an id of its own: what referred to the
/// original still does.
///
/// # Errors
/// `InvalidAsset` if `from` is not an asset or `to` has another extension,
/// `ProjectAlreadyExists` if `to` is there, or an error of the project.
pub fn copy_asset(project: &Project, registry: &mut AssetRegistry, from: &ZPath, to: &ZPath) -> ZResult<Imported> {
	let kind = same_kind(from, to)?;
	if project.exists(to)? {
		return Err(ZenithError::ProjectAlreadyExists(to.to_string()));
	}
	let id = AssetId::generate()?;
	project.write(to, &with_id(&project.read_bytes(from)?, kind, id)?)?;
	registry.insert(to.clone(), kind, id);
	Ok(Imported {
		id,
		kind,
		path: to.clone(),
	})
}

/// Moves or renames an asset. It keeps its id: nothing that refers to it
/// breaks.
///
/// # Errors
/// As [`copy_asset`].
pub fn move_asset(project: &Project, registry: &mut AssetRegistry, from: &ZPath, to: &ZPath) -> ZResult<()> {
	same_kind(from, to)?;
	project.move_to(from, to)?;
	registry.rename(from, to);
	Ok(())
}

/// Gives the asset file at `path` an id of its own: for a copy made outside
/// the editor, which has the id of its original (see
/// [`AssetProblem::Duplicate`](super::AssetProblem::Duplicate)).
///
/// # Errors
/// `InvalidAsset` if it is not an asset, or an error of the project.
pub fn give_new_id(project: &Project, registry: &mut AssetRegistry, path: &ZPath) -> ZResult<AssetId> {
	let kind = AssetKind::of_path(path).ok_or_else(|| ZenithError::InvalidAsset(format!("{path} is not an asset")))?;
	let id = AssetId::generate()?;
	project.write(path, &with_id(&project.read_bytes(path)?, kind, id)?)?;
	registry.forget(path);
	registry.insert(path.clone(), kind, id);
	Ok(id)
}

/// Puts another source into a model or a texture. Its id and its import
/// settings stay, so everything that uses it gets the new one.
///
/// # Errors
/// `InvalidAsset` if it is not a model or a texture, or the new source cannot
/// be read as one; `ImageDecodeFailed` for a texture that is not a picture; or
/// an error of the project.
pub fn replace_source(project: &Project, path: &ZPath, source: &Path) -> ZResult<()> {
	let kind = AssetKind::of_path(path).ok_or_else(|| ZenithError::InvalidAsset(format!("{path} is not an asset")))?;
	let bytes = read_source(source)?;
	let (_, extension) = split_name(file_name(source)?);
	let extension = extension.to_ascii_lowercase();
	let old = project.read_bytes(path)?;
	let new = match kind {
		AssetKind::Model => {
			let model: container::Container<ModelImport> = container::read(&old, kind)?;
			// A glTF is kept as one `.glb`, as when it was imported.
			let (bytes, format) = match extension.as_str() {
				"gltf" | "glb" => {
					let beside = source.parent().map(Path::to_path_buf).unwrap_or_default();
					(gltf::to_glb(&gltf::read(&bytes, &beside)?)?, "glb")
				}
				_ => (bytes, extension.as_str()),
			};
			let settings = ModelImport {
				source_format: format.to_string(),
				flip_v: if format == model.settings.source_format {
					model.settings.flip_v
				} else {
					ModelImport::for_format(format).flip_v
				},
				..model.settings
			};
			let meshes = mesh::parse(&bytes, &settings)?.submeshes.len();
			if meshes != settings.mesh_materials.len() {
				zenith_log::warn!(
					"{path}: the new source has {meshes} meshes, the old one had materials for {}",
					settings.mesh_materials.len()
				);
			}
			container::write(kind, model.id, &settings, &bytes)?
		}
		AssetKind::Texture => {
			let texture: container::Container<TextureImport> = container::read(&old, kind)?;
			image::check(&bytes)?;
			let settings = TextureImport {
				source_format: extension,
				..texture.settings
			};
			container::write(kind, texture.id, &settings, &bytes)?
		}
		AssetKind::Material | AssetKind::RenderGraph => {
			return Err(ZenithError::InvalidAsset(format!(
				"{path} has no source: it is edited as it is"
			)));
		}
	};
	project.write(path, &new)
}

/// What an asset gives back out of the project: the source of a model or a
/// texture (a model from a glTF comes out as `.glb`), or the file of a
/// material or a render graph as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exported {
	pub bytes: Vec<u8>,
	/// Without the dot: `glb`, `png`, `zenithmaterial`.
	pub extension: String,
}

/// The file [`export_asset`] would write for the asset at `path`.
///
/// # Errors
/// `InvalidAsset` if it is not an asset or cannot be read as one, or an error
/// of the project.
pub fn exported(project: &Project, path: &ZPath) -> ZResult<Exported> {
	let kind = AssetKind::of_path(path).ok_or_else(|| ZenithError::InvalidAsset(format!("{path} is not an asset")))?;
	let bytes = project.read_bytes(path)?;
	Ok(match kind {
		AssetKind::Model => {
			let model: container::Container<ModelImport> = container::read(&bytes, kind)?;
			Exported {
				bytes: model.source,
				extension: model.settings.source_format,
			}
		}
		AssetKind::Texture => {
			let texture: container::Container<TextureImport> = container::read(&bytes, kind)?;
			Exported {
				bytes: texture.source,
				extension: texture.settings.source_format,
			}
		}
		AssetKind::Material | AssetKind::RenderGraph => Exported {
			bytes: bytes.to_vec(),
			extension: kind.extension().to_string(),
		},
	})
}

/// Writes what the asset at `path` was made from into `to`, a file on the
/// computer (see [`exported`]). An existing file is replaced.
///
/// # Errors
/// As [`exported`], or an error of writing `to`.
pub fn export_asset(project: &Project, path: &ZPath, to: &Path) -> ZResult<()> {
	let out = exported(project, path)?;
	std::fs::write(to, out.bytes).map_err(|e| ZenithError::Other(format!("{}: {e}", to.display())))
}

/// `<name of the asset>.<extension of what it exports>`: the name to offer
/// when it is exported.
///
/// # Errors
/// As [`exported`].
pub fn export_file_name(project: &Project, path: &ZPath) -> ZResult<String> {
	let extension = exported(project, path)?.extension;
	let (stem, _) = split_name(path.file_name().unwrap_or_default());
	Ok(format!("{stem}.{extension}"))
}

/// The files a model is imported from.
pub const MODEL_SOURCES: [&str; 3] = ["obj", "gltf", "glb"];

/// `name` is the file of a model to import (`.obj`, `.gltf`, `.glb`, in any
/// case).
#[must_use]
pub fn is_model_source(name: &str) -> bool {
	let (_, extension) = split_name(name);
	MODEL_SOURCES
		.iter()
		.any(|source| extension.eq_ignore_ascii_case(source))
}

/// A model file put into `Assets://` by hand that is not an asset yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LooseModel {
	pub path: ZPath,
	/// The `.zenithmodel` it would become is there already, so it is left
	/// alone.
	pub taken_by: Option<ZPath>,
}

/// Every `.obj`, `.gltf` and `.glb` in `Assets://`, by path.
///
/// # Errors
/// An error of the project while listing a folder.
pub fn loose_models(project: &Project) -> ZResult<Vec<LooseModel>> {
	let root = ZPath::new("Assets://")?;
	let mut found = Vec::new();
	if project.exists(&root)? {
		walk_loose(project, &root, &mut found)?;
	}
	found.sort_by(|a, b| a.path.cmp(&b.path));
	Ok(found)
}

fn walk_loose(project: &Project, folder: &ZPath, found: &mut Vec<LooseModel>) -> ZResult<()> {
	for entry in project.list(folder)? {
		let path = folder.join(&entry.name)?;
		match entry.kind {
			EntryKind::Dir => walk_loose(project, &path, found)?,
			EntryKind::File if is_model_source(&entry.name) => {
				let model = destination(folder, split_name(&entry.name).0, AssetKind::Model)?;
				let taken_by = project.exists(&model)?.then_some(model);
				found.push(LooseModel { path, taken_by });
			}
			EntryKind::File | EntryKind::Symlink => {}
		}
	}
	Ok(())
}

/// Turns a model file in `Assets://` into an asset next to it, and moves the
/// file to the trash of the project.
///
/// The asset comes with its materials and textures, as [`import_model`]; the
/// file can be restored from the trash. The files it names (an `.mtl`, a
/// `.bin`, pictures) stay: something else may use them.
///
/// # Errors
/// As [`import_model`]; `Unsupported` for a build; or the error of moving the
/// file to the trash, after the asset is made.
pub fn import_loose_model(project: &Project, registry: &mut AssetRegistry, path: &ZPath) -> ZResult<Vec<Imported>> {
	let source = project
		.disk_path(path)
		.ok_or_else(|| ZenithError::Unsupported(format!("{path} is not in a project folder")))?;
	let folder = path
		.parent()
		.ok_or_else(|| ZenithError::InvalidAsset(format!("{path} is not in a folder")))?;
	let made = import_model(project, registry, &source, &folder)?;
	project
		.remove(path)
		.map_err(|e| ZenithError::Other(format!("{path} is imported, but it is not moved to the trash: {e}")))?;
	Ok(made)
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use super::*;
	use crate::assets::{AssetProblem, import::image::tests::png, load};

	const NO_PROBLEMS: &[AssetProblem] = &[];

	#[allow(clippy::expect_used)] // the paths of the tests are written by hand
	fn path(text: &str) -> ZPath {
		ZPath::new(text).expect("a valid path")
	}

	/// A project, and a folder of sources outside it: a square with a red
	/// material that has a texture and a normal map.
	fn setup() -> ZResult<(tempfile::TempDir, Project, PathBuf)> {
		let dir = tempfile::tempdir()?;
		let project = Project::create(dir.path().join("Game"))?;
		let sources = dir.path().join("Sources");
		std::fs::create_dir_all(&sources)?;
		std::fs::write(
			sources.join("square.obj"),
			format!("mtllib square.mtl\n{}", obj::tests::SQUARE),
		)?;
		std::fs::write(
			sources.join("square.mtl"),
			"newmtl red\nKd 1 0 0\nmap_Kd red.png\nmap_Bump flat-normal.png\n",
		)?;
		std::fs::write(sources.join("red.png"), png(2, 2))?;
		std::fs::write(sources.join("flat-normal.png"), png(1, 1))?;
		Ok((dir, project, sources))
	}

	#[test]
	fn a_model_comes_in_with_its_materials_and_textures() -> ZResult<()> {
		let (_dir, project, sources) = setup()?;
		let mut registry = AssetRegistry::scan(&project)?;
		let made = import_model(
			&project,
			&mut registry,
			&sources.join("square.obj"),
			&path("Assets://Square"),
		)?;

		let kinds: Vec<_> = made.iter().map(|m| (m.kind, m.path.to_string())).collect();
		assert_eq!(
			kinds,
			[
				(AssetKind::Texture, "Assets://Square/red.zenithtexture".to_string()),
				(
					AssetKind::Texture,
					"Assets://Square/flat-normal.zenithtexture".to_string()
				),
				(AssetKind::Material, "Assets://Square/red.zenithmaterial".to_string()),
				(AssetKind::Model, "Assets://Square/square.zenithmodel".to_string()),
			]
		);
		let [red, normal, material, model] = [0, 1, 2, 3].map(|at| made.get(at).map(|m| m.id));

		// What the import made is what a fresh scan finds.
		let scanned = AssetRegistry::scan(&project)?;
		assert_eq!(scanned.len(), 4);
		for imported in &made {
			assert_eq!(scanned.path(imported.id), Some(&imported.path));
		}

		let material_data = load::material(&project, &scanned, material.unwrap_or(AssetId::from_bytes([0; 16])))?;
		assert_eq!(material_data.base_color, [1.0, 0.0, 0.0, 1.0]);
		assert_eq!(material_data.base_color_texture, red);
		assert_eq!(material_data.normal_texture, normal);

		let mesh = load::mesh(&project, &scanned, model.unwrap_or(AssetId::from_bytes([0; 16])))?;
		assert_eq!(mesh.submeshes.first().and_then(|m| m.material), material);

		let picture = load::image(&project, &scanned, red.unwrap_or(AssetId::from_bytes([0; 16])))?;
		assert_eq!(
			(picture.width, picture.height, picture.usage),
			(2, 2, TextureUsage::Color)
		);
		let normal_map = load::image(&project, &scanned, normal.unwrap_or(AssetId::from_bytes([0; 16])))?;
		assert_eq!(normal_map.usage, TextureUsage::Normal);

		assert!(
			import_model(
				&project,
				&mut registry,
				&sources.join("square.obj"),
				&path("Assets://Square")
			)
			.is_err(),
			"the model is already there"
		);
		Ok(())
	}

	#[test]
	fn a_failed_import_leaves_nothing_behind() -> ZResult<()> {
		let (_dir, project, sources) = setup()?;
		std::fs::write(sources.join("flat-normal.png"), b"not a picture")?;
		let mut registry = AssetRegistry::scan(&project)?;
		let result = import_model(
			&project,
			&mut registry,
			&sources.join("square.obj"),
			&path("Assets://Square"),
		);
		assert!(result.is_err(), "the normal map is not a picture");
		assert!(
			!project.exists(&path("Assets://Square/red.zenithtexture"))?,
			"the texture made before it is taken back"
		);
		assert_eq!(
			project.trash_find(&path("Assets://Square/red.zenithtexture"))?.len(),
			1,
			"to the trash"
		);
		assert!(registry.is_empty(), "and out of the registry");
		Ok(())
	}

	#[test]
	fn a_file_in_the_way_is_not_written_over() -> ZResult<()> {
		let (_dir, project, sources) = setup()?;
		project.write(&path("Assets://Square/red.zenithtexture"), b"something of the user")?;
		let mut registry = AssetRegistry::scan(&project)?;
		let error = import_model(
			&project,
			&mut registry,
			&sources.join("square.obj"),
			&path("Assets://Square"),
		)
		.err()
		.map(|e| e.to_string());
		assert!(error.is_some_and(|e| e.contains("remove it first")), "it says why");
		assert_eq!(
			project.read_bytes(&path("Assets://Square/red.zenithtexture"))?.as_ref(),
			b"something of the user"
		);
		assert!(!project.exists(&path("Assets://Square/square.zenithmodel"))?);
		Ok(())
	}

	#[test]
	fn two_pictures_with_one_name_stay_two_textures() -> ZResult<()> {
		let (_dir, project, sources) = setup()?;
		for folder in ["a", "b"] {
			std::fs::create_dir_all(sources.join(folder))?;
			std::fs::write(sources.join(folder).join("color.png"), png(1, 1))?;
		}
		std::fs::write(
			sources.join("square.mtl"),
			"newmtl red\nmap_Kd a/color.png\nmap_Bump b/color.png\n",
		)?;
		let mut registry = AssetRegistry::scan(&project)?;
		let made = import_model(
			&project,
			&mut registry,
			&sources.join("square.obj"),
			&path("Assets://Square"),
		)?;
		let textures: Vec<_> = made
			.iter()
			.filter(|m| m.kind == AssetKind::Texture)
			.map(|m| m.path.to_string())
			.collect();
		assert_eq!(
			textures,
			[
				"Assets://Square/color.zenithtexture",
				"Assets://Square/color-1.zenithtexture"
			]
		);
		Ok(())
	}

	#[test]
	fn a_model_put_into_assets_by_hand_becomes_an_asset_and_its_file_goes_to_the_trash() -> ZResult<()> {
		let (_dir, project, sources) = setup()?;
		let models = project
			.disk_path(&path("Assets://Models"))
			.ok_or_else(|| ZenithError::Other("no disk".into()))?;
		std::fs::create_dir_all(models.join("Old"))?;
		for name in ["square.obj", "square.mtl", "red.png", "flat-normal.png"] {
			std::fs::copy(sources.join(name), models.join(name))?;
		}
		std::fs::copy(sources.join("square.obj"), models.join("Old/SQUARE.OBJ"))?;
		std::fs::write(models.join("Old/SQUARE.zenithmodel"), b"in the way")?;

		assert_eq!(
			loose_models(&project)?,
			[
				LooseModel {
					path: path("Assets://Models/Old/SQUARE.OBJ"),
					taken_by: Some(path("Assets://Models/Old/SQUARE.zenithmodel")),
				},
				LooseModel {
					path: path("Assets://Models/square.obj"),
					taken_by: None,
				},
			]
		);

		let mut registry = AssetRegistry::scan(&project)?;
		let made = import_loose_model(&project, &mut registry, &path("Assets://Models/square.obj"))?;
		assert_eq!(made.len(), 4, "the model, its material and its two textures");
		assert!(project.exists(&path("Assets://Models/square.zenithmodel"))?);
		assert!(
			!project.exists(&path("Assets://Models/square.obj"))?,
			"the file is gone"
		);
		assert!(
			project.exists(&path("Assets://Models/square.mtl"))?,
			"what it names stays"
		);
		let trashed = project.trash_find(&path("Assets://Models/square.obj"))?;
		assert_eq!(trashed.len(), 1, "it is in the trash");

		assert!(
			import_loose_model(&project, &mut registry, &path("Assets://Models/Old/SQUARE.OBJ")).is_err(),
			"a model that is there already is not replaced"
		);
		assert!(
			project.exists(&path("Assets://Models/Old/SQUARE.OBJ"))?,
			"and its file stays"
		);
		assert_eq!(
			project
				.read_bytes(&path("Assets://Models/Old/SQUARE.zenithmodel"))?
				.as_ref(),
			b"in the way"
		);

		assert_eq!(loose_models(&project)?.len(), 1, "only the one in the way is left");
		Ok(())
	}

	#[test]
	fn an_asset_exports_what_it_was_made_from() -> ZResult<()> {
		let (dir, project, sources) = setup()?;
		let mut registry = AssetRegistry::scan(&project)?;
		import_model(
			&project,
			&mut registry,
			&sources.join("square.obj"),
			&path("Assets://Square"),
		)?;
		let out = dir.path().join("Out");
		std::fs::create_dir_all(&out)?;
		for (asset, name, source) in [
			("Assets://Square/square.zenithmodel", "square.obj", Some("square.obj")),
			("Assets://Square/red.zenithtexture", "red.png", Some("red.png")),
			("Assets://Square/red.zenithmaterial", "red.zenithmaterial", None),
		] {
			let asset = path(asset);
			assert_eq!(export_file_name(&project, &asset)?, name);
			export_asset(&project, &asset, &out.join(name))?;
			let expected = match source {
				Some(source) => std::fs::read(sources.join(source))?,
				None => project.read_bytes(&asset)?.to_vec(),
			};
			assert_eq!(std::fs::read(out.join(name))?, expected, "{name}");
		}
		assert!(
			exported(&project, &path("Assets://Square/notes.txt")).is_err(),
			"not an asset"
		);
		Ok(())
	}

	#[test]
	fn a_second_model_of_the_set_shares_the_textures_and_materials() -> ZResult<()> {
		let (_dir, project, sources) = setup()?;
		let mut registry = AssetRegistry::scan(&project)?;
		let first = import_model(
			&project,
			&mut registry,
			&sources.join("square.obj"),
			&path("Assets://Set"),
		)?;
		std::fs::copy(sources.join("square.obj"), sources.join("other.obj"))?;
		let second = import_model(
			&project,
			&mut registry,
			&sources.join("other.obj"),
			&path("Assets://Set"),
		)?;
		let ids = |made: &[Imported]| {
			made.iter()
				.filter(|m| m.kind != AssetKind::Model)
				.map(|m| m.id)
				.collect::<Vec<_>>()
		};
		assert_eq!(ids(&first), ids(&second), "the same textures and material");
		assert_eq!(AssetRegistry::scan(&project)?.len(), 5, "and one more model");
		Ok(())
	}

	#[test]
	fn a_copy_gets_its_own_id_and_a_move_keeps_it() -> ZResult<()> {
		let (_dir, project, sources) = setup()?;
		let mut registry = AssetRegistry::scan(&project)?;
		let texture = import_texture(
			&project,
			&mut registry,
			&sources.join("red.png"),
			&path("Assets://T"),
			None,
		)?;
		let material = path("Assets://T/m.zenithmaterial");
		project.write(&material, MaterialData::new(AssetId::generate()?).to_text()?.as_bytes())?;
		registry.refresh(&project, &material)?;

		for from in [&texture.path, &material] {
			let to = path(&from.to_string().replace("/T/", "/Copies/"));
			let copy = copy_asset(&project, &mut registry, from, &to)?;
			assert_ne!(Some(copy.id), registry.id_of(from), "{from}: a new id");
			assert_eq!(registry.path(copy.id), Some(&to));
		}

		let moved = path("Assets://Moved/red.zenithtexture");
		move_asset(&project, &mut registry, &texture.path, &moved)?;
		assert_eq!(registry.path(texture.id), Some(&moved));
		assert!(project.exists(&moved)?);
		assert!(
			move_asset(&project, &mut registry, &moved, &path("Assets://Moved/red.png")).is_err(),
			"an asset keeps its extension"
		);

		let fresh = AssetRegistry::scan(&project)?;
		assert_eq!(fresh.len(), 4);
		assert_eq!(fresh.problems(), NO_PROBLEMS);
		Ok(())
	}

	#[test]
	fn a_copy_made_outside_the_editor_gets_an_id_of_its_own_on_request() -> ZResult<()> {
		let (_dir, project, sources) = setup()?;
		let mut registry = AssetRegistry::scan(&project)?;
		let texture = import_texture(
			&project,
			&mut registry,
			&sources.join("red.png"),
			&path("Assets://T"),
			None,
		)?;
		let copy = path("Assets://T/red-copy.zenithtexture");
		project.copy(&texture.path, &copy)?;
		registry.rescan(&project)?;
		assert_eq!(registry.problems().len(), 1, "a duplicate");

		let id = give_new_id(&project, &mut registry, &copy)?;
		assert_ne!(id, texture.id);
		assert_eq!(registry.problems(), NO_PROBLEMS);
		assert_eq!(registry.path(id), Some(&copy));
		assert_eq!(registry.path(texture.id), Some(&texture.path));
		Ok(())
	}

	#[test]
	fn a_new_source_keeps_the_id_and_the_settings() -> ZResult<()> {
		let (_dir, project, sources) = setup()?;
		let mut registry = AssetRegistry::scan(&project)?;
		let normal = import_texture(
			&project,
			&mut registry,
			&sources.join("flat-normal.png"),
			&path("Assets://T"),
			None,
		)?;
		std::fs::write(sources.join("bigger.png"), png(4, 4))?;
		replace_source(&project, &normal.path, &sources.join("bigger.png"))?;

		let picture = load::image(&project, &registry, normal.id)?;
		assert_eq!(
			(picture.width, picture.usage),
			(4, TextureUsage::Normal),
			"the same id and usage"
		);

		std::fs::write(sources.join("not.png"), b"not a picture")?;
		assert!(replace_source(&project, &normal.path, &sources.join("not.png")).is_err());
		assert_eq!(load::image(&project, &registry, normal.id)?.width, 4, "left as it was");
		Ok(())
	}

	use crate::assets::import::gltf::tests::{RED_MATERIAL, square_buffer, square_gltf};

	/// What `import_model` made, by kind, and the model read back.
	fn check_square(project: &Project, made: &[Imported]) -> ZResult<()> {
		let kinds: Vec<_> = made.iter().map(|m| m.kind).collect();
		assert_eq!(
			kinds,
			[AssetKind::Texture, AssetKind::Material, AssetKind::Model],
			"one picture, used twice: {made:?}"
		);
		let registry = AssetRegistry::scan(project)?;
		let [texture, material, model] =
			[0, 1, 2].map(|at| made.get(at).map_or(AssetId::from_bytes([0; 16]), |m| m.id));

		let data = load::material(project, &registry, material)?;
		assert_eq!(data.base_color, [1.0, 0.0, 0.0, 1.0]);
		assert_eq!(data.base_color_texture, Some(texture));
		assert_eq!(data.normal_texture, Some(texture));
		assert!((data.metallic - 0.25).abs() < f32::EPSILON && (data.roughness - 0.75).abs() < f32::EPSILON);
		assert_eq!(data.blend, BlendMode::Mask);
		assert!((data.alpha_cutoff - 0.3).abs() < f32::EPSILON);
		assert!(data.double_sided);

		let mesh = load::mesh(project, &registry, model)?;
		assert_eq!(mesh.submeshes.first().and_then(|m| m.material), Some(material));
		assert_eq!(load::image(project, &registry, texture)?.width, 2);
		Ok(())
	}

	#[test]
	fn a_gltf_comes_in_with_the_files_it_names() -> ZResult<()> {
		let (_dir, project, sources) = setup()?;
		std::fs::write(sources.join("square.bin"), square_buffer(&[]))?;
		std::fs::write(
			sources.join("square.gltf"),
			square_gltf("square.bin", 0, r#"{ "uri": "red.png" }"#, RED_MATERIAL),
		)?;
		let mut registry = AssetRegistry::scan(&project)?;
		let made = import_model(
			&project,
			&mut registry,
			&sources.join("square.gltf"),
			&path("Assets://G"),
		)?;
		check_square(&project, &made)?;
		assert_eq!(
			made.iter().map(|m| m.path.to_string()).collect::<Vec<_>>(),
			[
				"Assets://G/red.zenithtexture",
				"Assets://G/red.zenithmaterial",
				"Assets://G/square.zenithmodel"
			],
			"named after the file of the picture and the material"
		);
		Ok(())
	}

	#[test]
	fn a_gltf_with_everything_inside_and_a_glb_come_in() -> ZResult<()> {
		use base64::Engine;

		let (_dir, project, sources) = setup()?;
		let picture = png(2, 2);
		let buffer = base64::engine::general_purpose::STANDARD.encode(square_buffer(&picture));
		let text = square_gltf(
			&format!("data:application/octet-stream;base64,{buffer}"),
			picture.len(),
			r#"{ "bufferView": 3, "mimeType": "image/png" }"#,
			RED_MATERIAL,
		);
		std::fs::write(sources.join("inside.gltf"), &text)?;
		let mut registry = AssetRegistry::scan(&project)?;
		let made = import_model(
			&project,
			&mut registry,
			&sources.join("inside.gltf"),
			&path("Assets://In"),
		)?;
		check_square(&project, &made)?;
		assert_eq!(
			made.first().map(|m| m.path.to_string()),
			Some("Assets://In/inside-image0.zenithtexture".to_string()),
			"a picture without a name is named after the model"
		);

		let glb = gltf::to_glb(&gltf::read(text.as_bytes(), &sources)?)?;
		std::fs::write(sources.join("binary.glb"), glb)?;
		let made = import_model(
			&project,
			&mut registry,
			&sources.join("binary.glb"),
			&path("Assets://Glb"),
		)?;
		check_square(&project, &made)?;
		Ok(())
	}

	#[test]
	fn a_model_gets_a_gltf_source_in_place_of_an_obj() -> ZResult<()> {
		let (_dir, project, sources) = setup()?;
		let mut registry = AssetRegistry::scan(&project)?;
		let made = import_model(
			&project,
			&mut registry,
			&sources.join("square.obj"),
			&path("Assets://S"),
		)?;
		let model = made.last().map_or(AssetId::from_bytes([0; 16]), |m| m.id);
		let model_path = path("Assets://S/square.zenithmodel");

		std::fs::write(sources.join("square.bin"), square_buffer(&[]))?;
		std::fs::write(
			sources.join("new.gltf"),
			square_gltf("square.bin", 0, r#"{ "uri": "red.png" }"#, RED_MATERIAL),
		)?;
		replace_source(&project, &model_path, &sources.join("new.gltf"))?;
		let bytes = project.read_bytes(&model_path)?;
		let container: container::Container<ModelImport> = container::read(&bytes, AssetKind::Model)?;
		assert_eq!(container.id, model, "the same id");
		assert_eq!(container.settings.source_format, "glb");
		assert!(!container.settings.flip_v, "glTF counts v from the top");
		let mesh = load::mesh(&project, &registry, model)?;
		assert!(
			mesh.submeshes
				.iter()
				.flat_map(|m| &m.vertices)
				.any(|v| v.position[0] > 10.0),
			"the new source, moved by its node"
		);
		Ok(())
	}
}
