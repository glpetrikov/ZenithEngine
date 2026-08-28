use std::{
	ffi::OsStr,
	path::{Component, Path, PathBuf},
};

use zenith_error::{ZPubResult, ZenithError};

/// A path to a world file, relative to `project_root/Assets`.
///
/// Always starts with the literal `Worlds/` segment and never escapes the
/// project directory. Validation happens once, here, so trait methods and
/// every version-specific implementation don't need to re-check or
/// re-document the rule themselves — the type is the contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldPath(PathBuf);

impl WorldPath {
	/// Validates and wraps a raw path.
	///
	/// Fails if `path` doesn't start with `Worlds/`, or contains a `..`
	/// component that could let it escape the project.
	pub fn new(path: impl AsRef<Path>) -> ZPubResult<Self> {
		let path = path.as_ref();

		// Checked as an explicit component, not a string prefix — a string
		// check would also accept something like `WorldsButNotReally/x`.
		if path.components().next() != Some(Component::Normal(OsStr::new("Worlds"))) {
			return Err(ZenithError::InvalidWorldPath(path.to_path_buf()));
		}

		// Checked separately from the prefix above, since `..` can appear
		// anywhere in the path, not just at the start — e.g.
		// `Worlds/../Scripts/evil.rs` passes the prefix check but must
		// still be rejected here.
		if path.components().any(|c| matches!(c, Component::ParentDir)) {
			return Err(ZenithError::InvalidWorldPath(path.to_path_buf()));
		}

		Ok(Self(path.to_path_buf()))
	}

	pub fn as_path(&self) -> &Path { &self.0 }
}
