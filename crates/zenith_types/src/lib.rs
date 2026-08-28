pub mod ecs;
// pub mod physics;
pub mod paths;
pub mod project;
pub mod settings;

pub use glam::{
	BVec4, Quat, Vec2, Vec3,
	bool::{BVec2, BVec3},
};
pub use semver::{BuildMetadata, Comparator, Error, Prerelease, Version, VersionReq};
pub use serde::{Deserialize, Deserializer, Serialize, Serializer};

// for serde
pub const fn default_true() -> bool { true }
pub const fn default_false() -> bool { false }

pub const ZENITH_PROJECT_EXTENSION: &str = "zenithproject";
pub const ZENITH_VERSION: &str = env!("CARGO_PKG_VERSION"); // "0.1.0";
pub const ZENITH_WORLD_EXTENSION: &str = "zenith";
pub const PROJECT_GITIGNORE: &[u8] = b"Temp/\nTrash/\n";

pub const DEFAULT_PROJECT_DIRS: &[&str] = &["Assets", "Assets/Worlds", "Settings", "Packages", "Temp", "Trash"];

use std::{
	fs,
	path::{Path, PathBuf},
};

use zenith_error::{ZPubResult, ZenithError};

/// Recursively walks `root` and returns the paths of every file found
/// underneath it, relative to `root` — e.g. `Levels/Level2.zenith`,
/// not `root/Levels/Level2.zenith`.
pub fn walk_files(root: &Path) -> Vec<ZPubResult<PathBuf>> {
	let mut result: Vec<ZPubResult<PathBuf>> = Vec::new();
	walk_files_into(root, root, &mut result);
	result
}

fn walk_files_into(root: &Path, dir: &Path, out: &mut Vec<ZPubResult<PathBuf>>) {
	// read_dir itself can fail (e.g. dir doesn't exist or isn't readable) —
	// pushed as an error entry instead of propagated, so a failure at any
	// depth (including the root) is reported through `out`, not silently
	// dropped by an unhandled Result at the call site.
	let entries = match fs::read_dir(dir) {
		Ok(entries) => entries,
		Err(e) => {
			out.push(Err(e.into()));
			return;
		}
	};

	for entry in entries {
		let entry = match entry {
			Ok(entry) => entry,
			Err(e) => {
				out.push(Err(e.into()));
				continue;
			}
		};
		let path = entry.path();
		if path.is_dir() {
			walk_files_into(root, &path, out);
		} else {
			// `path` always comes from an entry under `dir`, and `dir` is
			// always `root` or a descendant of it, so `path` is always
			// under `root` — strip_prefix cannot actually fail here.
			match path.strip_prefix(root) {
				Ok(relative) => out.push(Ok(relative.to_path_buf())),
				Err(_) => out.push(Err(ZenithError::PathEscapesRoot(path))),
			}
		}
	}
}
