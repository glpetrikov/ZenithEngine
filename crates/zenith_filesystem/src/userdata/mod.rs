//! Where the data of a player goes: settings they changed, save files.
//!
//! It is not part of the project. A project folder is the author's, a build is
//! read-only and may sit where nothing can be written (an install folder that
//! an update or a file check wipes). The player's data lives in a place of its
//! own, reached through the `Data://` scheme of any
//! [`Project`](crate::Project):
//!
//! * on a computer, in the user's data folder, `<data folder>/<project name>`
//!   ([`DirStore`]);
//! * in a browser, in `IndexedDB` (`IdbStore`);
//! * in tests, in memory ([`MemStore`]).
//!
//! All of them are a [`UserStore`]: whole files under `/`-separated paths,
//! replaced atomically, so a file is the old one or the new one and never half
//! of each.

mod mem;

#[cfg(feature = "fs")]
mod dir;
#[cfg(target_arch = "wasm32")]
mod idb;

#[cfg(feature = "fs")]
pub use dir::DirStore;
#[cfg(target_arch = "wasm32")]
pub use idb::IdbStore;
pub use mem::MemStore;
use zenith_error::{ZResult, ZenithError};

use crate::types::{DirEntry, Metadata};

/// The scheme of the player's data: `Data://Saves/slot1.zenithsave`.
pub const DATA_SCHEME: &str = "Data";

/// Storage for the data of a player. Paths are relative, `/`-separated and
/// already checked with [`check_path`]; the empty path is the root.
pub trait UserStore: Send + Sync + std::fmt::Debug {
	/// The file, or `None` if there is none.
	///
	/// # Errors
	/// `ProjectWrongKind` if the path is a folder, or an I/O error.
	fn read(&self, path: &str) -> ZResult<Option<Vec<u8>>>;

	/// Replaces the file (or creates it, with its folders) atomically.
	///
	/// # Errors
	/// `ProjectWrongKind` if the path is a folder, or an I/O error.
	fn write(&self, path: &str, data: &[u8]) -> ZResult<()>;

	/// Deletes a file or a folder with all that is in it for good; `false` if
	/// there was nothing.
	///
	/// # Errors
	/// An I/O error.
	fn remove(&self, path: &str) -> ZResult<bool>;

	/// What the path is; `None` if there is nothing.
	///
	/// # Errors
	/// An I/O error.
	fn metadata(&self, path: &str) -> ZResult<Option<Metadata>>;

	/// The entries of a folder, sorted by name. The root always exists.
	///
	/// # Errors
	/// `ProjectNotFound` if there is no such folder.
	fn list(&self, path: &str) -> ZResult<Vec<DirEntry>>;

	/// Makes sure a folder exists (a store of files only has nothing to do).
	///
	/// # Errors
	/// An I/O error.
	fn create_dir(&self, _path: &str) -> ZResult<()> {
		Ok(())
	}

	/// Waits until everything written is safe, for stores that keep it in
	/// memory for a while. A store that writes at once has nothing to wait
	/// for.
	///
	/// # Errors
	/// An I/O error.
	fn flush(&self) -> ZResult<()> {
		Ok(())
	}
}

/// Checks a path of the store: relative, made of ordinary names that are safe
/// on every platform (the rules of paths inside a build), not naming a
/// temporary file. `allow_root` lets the empty path through.
///
/// # Errors
/// `InvalidZPath` describing the first problem.
pub fn check_path(path: &str, allow_root: bool) -> ZResult<()> {
	if path.is_empty() {
		return if allow_root {
			Ok(())
		} else {
			Err(ZenithError::InvalidZPath("Data:// needs a file name".into()))
		};
	}
	zenith_archive::validate_entry_path(path).map_err(|e| ZenithError::InvalidZPath(format!("Data://{path}: {e}")))?;
	if path.split('/').any(|name| name.ends_with(crate::layout::TMP_SUFFIX)) {
		return Err(ZenithError::InvalidZPath(format!(
			"Data://{path}: names ending in {} are the engine's temporary files",
			crate::layout::TMP_SUFFIX
		)));
	}
	Ok(())
}

/// The folder a game's data goes to by default on a computer.
///
/// It is the data folder of the system for the user (`dirs::data_dir`,
/// wherever that is on this machine), then `folder`, which is `user_data` of
/// the manifest or the name of the project (see `Manifest::user_data_folder`).
/// `None` where the system has no such folder.
#[cfg(feature = "fs")]
#[must_use]
pub fn default_data_dir(folder: &str) -> Option<std::path::PathBuf> {
	dirs::data_dir().map(|dir| dir.join(folder))
}
