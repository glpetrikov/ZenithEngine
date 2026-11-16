//! Reading an asset by its id, into the CPU structures of [`import`] and
//! [`json`]. The GPU cache uploads what these return.
//!
//! [`import`]: super::import
//! [`json`]: super::json

use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::Project;

use super::{
	AssetEntry, AssetId, AssetKind, AssetRegistry, container,
	import::{
		image::{self, ImageData, TextureImport},
		mesh::{self, MeshData, ModelImport},
	},
	json::MaterialData,
};

/// Where the asset `id` of `kind` is.
fn find(registry: &AssetRegistry, id: AssetId, kind: AssetKind) -> ZResult<&AssetEntry> {
	let entry = registry
		.get(id)
		.ok_or_else(|| ZenithError::AssetNotFound(format!("no asset has the id {id}")))?;
	if entry.kind != kind {
		return Err(ZenithError::InvalidAsset(format!(
			"{} is a {:?}, a {kind:?} was asked for",
			entry.path, entry.kind
		)));
	}
	Ok(entry)
}

/// The meshes of the model `id`.
///
/// # Errors
/// `AssetNotFound` for an id the registry does not have, `InvalidAsset` for
/// another kind of asset or a model that cannot be read, or an error of the
/// project.
pub fn mesh(project: &Project, registry: &AssetRegistry, id: AssetId) -> ZResult<MeshData> {
	let entry = find(registry, id, AssetKind::Model)?;
	let bytes = project.read_bytes(&entry.path)?;
	let model: container::Container<ModelImport> = container::read(&bytes, AssetKind::Model)?;
	mesh::parse(&model.source, &model.settings).map_err(|e| ZenithError::InvalidAsset(format!("{}: {e}", entry.path)))
}

/// The pixels of the texture `id`.
///
/// # Errors
/// As [`mesh`]; `ImageDecodeFailed` for a picture that cannot be decoded.
pub fn image(project: &Project, registry: &AssetRegistry, id: AssetId) -> ZResult<ImageData> {
	texture(project, registry, id).map(|(image, _)| image)
}

/// The pixels of the texture `id`, and how it was imported (its filter, wrap
/// and mipmaps, which the GPU needs too).
///
/// # Errors
/// As [`image`].
pub fn texture(project: &Project, registry: &AssetRegistry, id: AssetId) -> ZResult<(ImageData, TextureImport)> {
	let entry = find(registry, id, AssetKind::Texture)?;
	let bytes = project.read_bytes(&entry.path)?;
	let texture: container::Container<TextureImport> = container::read(&bytes, AssetKind::Texture)?;
	let image = image::decode(&texture.source, &texture.settings).map_err(|e| match e {
		ZenithError::ImageDecodeFailed(message) => ZenithError::ImageDecodeFailed(format!("{}: {message}", entry.path)),
		ZenithError::InvalidAsset(message) => ZenithError::InvalidAsset(format!("{}: {message}", entry.path)),
		other => other,
	})?;
	Ok((image, texture.settings))
}

/// The material `id`.
///
/// # Errors
/// As [`mesh`]; `Json` for a material that cannot be read.
pub fn material(project: &Project, registry: &AssetRegistry, id: AssetId) -> ZResult<MaterialData> {
	let entry = find(registry, id, AssetKind::Material)?;
	MaterialData::parse(&project.read_string(&entry.path)?)
}
