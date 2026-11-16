//! Assets: ids, files, the registry and the parsing into CPU structures.
//!
//! Nothing here uses wgpu or the rest of the crate, so the module can become a
//! crate of its own (physics and sound will want the same assets).
//!
//! One file per asset, no `.meta` files beside it:
//!
//! * models and textures are a binary container ([`container`]): a header with
//!   the id, the import settings as JSON, then the source file as it was (an
//!   `.obj`, a `.png`);
//! * everything else (materials, render graphs, ...) is JSON with the id inside
//!   ([`json`]), so it reads and merges well in version control.
//!
//! The [`registry`] finds every asset of a project by reading only the header
//! or the id of each file.

pub mod container;
pub mod id;
pub mod import;
pub mod json;
pub mod load;
pub mod ops;
pub mod registry;

pub use id::AssetId;
pub use registry::{AssetEntry, AssetProblem, AssetRegistry};
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::ZPath;

/// What an asset is. Its file has the extension of its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssetKind {
	/// A mesh from an `.obj` or a glTF: [`import::mesh`].
	Model,
	/// A picture: [`import::image`].
	Texture,
	/// [`json::MaterialData`].
	Material,
	/// A render graph (stage 4 of the plan).
	RenderGraph,
}

impl AssetKind {
	pub const ALL: [Self; 4] = [Self::Model, Self::Texture, Self::Material, Self::RenderGraph];

	/// The extension of its files, without the dot.
	pub const fn extension(self) -> &'static str {
		match self {
			Self::Model => "zenithmodel",
			Self::Texture => "zenithtexture",
			Self::Material => "zenithmaterial",
			Self::RenderGraph => "zenithgraph",
		}
	}

	/// The kind of a file, by its extension (in any case).
	pub fn of_path(path: &ZPath) -> Option<Self> {
		let name = path.file_name()?;
		let (_, extension) = name.rsplit_once('.')?;
		Self::ALL
			.into_iter()
			.find(|kind| kind.extension().eq_ignore_ascii_case(extension))
	}

	/// A binary container ([`container`]) or JSON ([`json`]).
	pub const fn is_binary(self) -> bool {
		matches!(self, Self::Model | Self::Texture)
	}

	/// The `type` of a JSON asset of this kind.
	pub const fn type_name(self) -> &'static str {
		match self {
			Self::Model => "model",
			Self::Texture => "texture",
			Self::Material => "material",
			Self::RenderGraph => "render_graph",
		}
	}
}

/// What a file that Git LFS has not fetched starts with.
const LFS_POINTER: &[u8] = b"version https://git-lfs";

/// An error for a file that is a Git LFS pointer instead of the file.
pub(crate) fn refuse_lfs_pointer(bytes: &[u8], what: &dyn std::fmt::Display) -> ZResult<()> {
	if bytes.starts_with(LFS_POINTER) {
		return Err(ZenithError::InvalidAsset(format!(
			"{what} is a Git LFS pointer, not the file: run `git lfs pull`"
		)));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_kind_of_a_file_is_its_extension() -> ZResult<()> {
		assert_eq!(
			AssetKind::of_path(&ZPath::new("Assets://Models/Cube.ZenithModel")?),
			Some(AssetKind::Model)
		);
		assert_eq!(
			AssetKind::of_path(&ZPath::new("Assets://Graphs/Main.zenithgraph")?),
			Some(AssetKind::RenderGraph)
		);
		assert_eq!(AssetKind::of_path(&ZPath::new("Assets://Models/cube.obj")?), None);
		assert_eq!(AssetKind::of_path(&ZPath::new("Assets://README")?), None);
		Ok(())
	}

	#[test]
	fn a_git_lfs_pointer_is_named() {
		let pointer = b"version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 12\n";
		let error = refuse_lfs_pointer(pointer, &"cube.obj").err().map(|e| e.to_string());
		assert!(error.is_some_and(|e| e.contains("git lfs pull")));
		assert!(refuse_lfs_pointer(b"v 1 2 3", &"cube.obj").is_ok());
	}
}
