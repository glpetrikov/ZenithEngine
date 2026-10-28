//! Where a build goes, and what it must not overlap.

use std::path::{Path, PathBuf};

use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::layout;

use crate::{
	builder::{BuildFormat, base_dirs},
	plan::Plan,
};

/// `Build/game.zenithbuild` or `Build/game/` in the project; the name does not
/// depend on the project, so a runtime can open the build under a fixed name.
pub fn default_output(project: &Path, format: BuildFormat) -> PathBuf {
	let build = project.join(layout::DIR_BUILD);
	match format {
		BuildFormat::Archive => build.join(format!(
			"{}.{}",
			layout::DEFAULT_BUILD_NAME,
			layout::BUILD_FILE_EXTENSION
		)),
		BuildFormat::Fs => build.join(layout::DEFAULT_BUILD_NAME),
	}
}

const fn bad(message: String) -> ZenithError {
	ZenithError::InvalidBuildOutput(message)
}

/// Canonical form of a path that may not exist yet: the deepest existing
/// ancestor is canonicalized (so links and `\\?\` prefixes compare equal), the
/// rest is appended as written.
pub fn resolve_for_compare(path: &Path) -> ZResult<PathBuf> {
	let absolute = std::path::absolute(path)?;
	let mut existing = absolute.as_path();
	let mut tail = Vec::new();
	while !existing.exists() {
		let Some(name) = existing.file_name() else { break };
		tail.push(name.to_os_string());
		match existing.parent() {
			Some(parent) => existing = parent,
			None => break,
		}
	}
	let mut resolved = std::fs::canonicalize(existing)?;
	resolved.extend(tail.iter().rev());
	Ok(resolved)
}

/// Refuses outputs that overlap the project's inputs.
///
/// `root` is the canonical project folder, `output` an absolute path.
pub fn check_output(root: &Path, output: &Path, manifest_name: &str, plan: &Plan, format: BuildFormat) -> ZResult<()> {
	let resolved = resolve_for_compare(output)?;
	if root.starts_with(&resolved) {
		return Err(bad(format!("{} contains the project folder", output.display())));
	}
	match (format, std::fs::symlink_metadata(&resolved)) {
		(BuildFormat::Archive, Ok(meta)) if meta.is_dir() => {
			return Err(bad(format!(
				"{} is a folder, but an archive build needs a file",
				output.display()
			)));
		}
		(BuildFormat::Fs, Ok(meta)) if !meta.is_dir() => {
			return Err(ZenithError::NotABuildDir(output.to_path_buf()));
		}
		_ => {}
	}

	let Ok(relative) = resolved.strip_prefix(root) else {
		return Ok(());
	};
	let parts: Option<Vec<&str>> = relative.components().map(|c| c.as_os_str().to_str()).collect();
	let rel = parts
		.ok_or_else(|| bad("the output path is not valid UTF-8".into()))?
		.join("/");
	let top = rel.split('/').next().unwrap_or("");
	// Nothing of the project itself may be replaced by a build: not the files
	// that make the folder a project, not the repository, not the trash (which
	// `purge_all` empties).
	if rel == layout::V1_FLAG || rel == layout::GITIGNORE {
		return Err(bad(format!("{} is {rel}, a file the project needs", output.display())));
	}
	if top == layout::DIR_GIT || top == layout::DIR_TRASH {
		return Err(bad(format!(
			"{} is inside {top}/, which git or the trash manages",
			output.display()
		)));
	}
	if base_dirs().contains(&top) {
		return Err(bad(format!(
			"{} is inside {top}/, which goes into the build",
			output.display()
		)));
	}
	if rel == layout::manifest_file_name(manifest_name) {
		return Err(bad(format!("{} is the project manifest", output.display())));
	}
	let below = format!("{rel}/");
	if let Some(read) = plan
		.read_rels
		.iter()
		.find(|read| **read == rel || read.starts_with(&below))
	{
		return Err(bad(format!(
			"{} overlaps {read}, which the build reads",
			output.display()
		)));
	}
	Ok(())
}
