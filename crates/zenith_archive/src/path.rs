//! Validation of entry paths and of the whole entry set of a build.

use std::collections::HashSet;

use zenith_error::{ZResult, ZenithError};

use crate::format::{MANIFEST_EXTENSION, MAX_PATH_LEN, MAX_SEGMENT_LEN};

fn bad(path: &str, why: &str) -> ZenithError {
	ZenithError::ArchiveInvalidPath(format!("{path:?}: {why}"))
}

/// Whether Windows treats `name` as a device (`CON`, `NUL`, `COM1`, ...),
/// whatever the case and with any extension (`aux.txt`). Such a name cannot be
/// used for a file there.
#[must_use]
pub fn is_windows_reserved_name(name: &str) -> bool {
	let stem = name.split('.').next().unwrap_or(name).trim_end();
	let upper = stem.to_ascii_uppercase();
	matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
		|| upper
			.strip_prefix("COM")
			.or_else(|| upper.strip_prefix("LPT"))
			.is_some_and(|digit| matches!(digit.as_bytes(), [b'1'..=b'9']))
}

/// Checks one entry path: relative, `/`-separated, no `.`/`..`/empty segments.
///
/// `:` and `\` are rejected as well: on Windows they would turn a joined path
/// into a drive-relative or absolute one. For the same reason a segment may not
/// end in a dot or a space or be a Windows device name: the unencrypted folder
/// build stores a file under its own path, and a hostile index must not be able
/// to name a device.
///
/// # Errors
/// Returns `ArchiveInvalidPath` describing the first violated rule.
pub fn validate_entry_path(path: &str) -> ZResult<()> {
	if path.is_empty() {
		return Err(bad(path, "empty path"));
	}
	if path.len() > MAX_PATH_LEN {
		return Err(bad(path, "path is too long"));
	}
	for segment in path.split('/') {
		if segment.is_empty() {
			return Err(bad(path, "empty segment (absolute path or double slash)"));
		}
		if segment == "." || segment == ".." {
			return Err(bad(path, "`.` and `..` segments are not allowed"));
		}
		if segment.len() > MAX_SEGMENT_LEN {
			return Err(bad(path, "segment is too long"));
		}
		if segment.chars().any(|c| c == '\\' || c == ':' || c.is_control()) {
			return Err(bad(path, "forbidden character"));
		}
		if segment.ends_with(['.', ' ']) {
			return Err(bad(
				path,
				"a segment may not end in a dot or a space (Windows drops it)",
			));
		}
		if is_windows_reserved_name(segment) {
			return Err(bad(
				path,
				"a Windows device name (CON, NUL, COM1, ...) cannot be a file name",
			));
		}
	}
	Ok(())
}

fn is_manifest(path: &str) -> bool {
	!path.contains('/')
		&& path
			.strip_suffix(MANIFEST_EXTENSION)
			.is_some_and(|stem| stem.len() > 1 && stem.ends_with('.'))
}

/// Checks the entry set of a build.
///
/// Paths are valid and strictly ascending (which also rules out duplicates),
/// no two paths differ only by letter case (they would be one file on Windows
/// and macOS), no entry is both a file and a directory, and the root holds
/// exactly one `*.zenithproject`.
///
/// # Errors
/// Returns `ArchiveInvalidPath` or `ArchiveCorrupt` for the first violation.
pub fn validate_entry_set(paths: &[&str]) -> ZResult<()> {
	let mut previous: Option<&str> = None;
	let mut files: HashSet<String> = HashSet::with_capacity(paths.len());
	for path in paths {
		validate_entry_path(path)?;
		if previous.is_some_and(|prev| prev >= *path) {
			return Err(bad(path, "entries must be unique and sorted by path"));
		}
		previous = Some(path);
		if !files.insert(path.to_lowercase()) {
			return Err(bad(path, "differs from another entry only by letter case"));
		}
	}
	for path in paths {
		let mut ancestor = path.to_lowercase();
		while let Some((parent, _)) = ancestor.rsplit_once('/') {
			if files.contains(parent) {
				return Err(bad(path, "a parent of this path is a file"));
			}
			ancestor = parent.to_string();
		}
	}
	let manifests = paths.iter().filter(|p| is_manifest(p)).count();
	if manifests != 1 {
		return Err(ZenithError::ArchiveCorrupt(format!(
			"a build must contain exactly one root *.{MANIFEST_EXTENSION}, found {manifests}"
		)));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rejects_bad_paths() {
		for p in [
			"", "/a", "a//b", "a/", "../a", "a/../b", "a/./b", "a\\b", "C:/a", "a\0b", "a:b",
		] {
			assert!(validate_entry_path(p).is_err(), "{p:?} must be rejected");
		}
		assert!(
			validate_entry_path(&"a/".repeat(600)).is_err(),
			"too long path must be rejected"
		);
		assert!(
			validate_entry_path(&"a".repeat(256)).is_err(),
			"too long segment must be rejected"
		);
		for p in ["a", "Assets/Worlds/A.zenith", "dir/файл.txt", "a b/c.d"] {
			assert!(validate_entry_path(p).is_ok(), "{p:?} must be accepted");
		}
	}

	#[test]
	fn windows_hostile_names_are_rejected() {
		for p in [
			"a.",
			"a /b",
			"dir./x",
			"Assets/CON",
			"nul.txt",
			"Aux",
			"x/com1",
			"x/LPT9.log",
			"con .txt",
			"...",
		] {
			assert!(validate_entry_path(p).is_err(), "{p:?} must be rejected");
		}
		for p in ["com0", "com10", "console", "lpt", "a.con", "Assets/null", ".hidden"] {
			assert!(validate_entry_path(p).is_ok(), "{p:?} is a fine name");
		}
		assert!(is_windows_reserved_name("CoM3.tar.gz"), "any extension, any case");
		assert!(!is_windows_reserved_name("COM"), "COM alone is not a device");
	}

	#[test]
	fn names_that_differ_only_by_case_conflict() {
		assert!(
			validate_entry_set(&["A.png", "T.zenithproject", "a.png"]).is_err(),
			"two spellings of one file"
		);
		assert!(
			validate_entry_set(&["T.zenithproject", "X", "x/y"]).is_err(),
			"a file and a folder that differ by case"
		);
		assert!(
			validate_entry_set(&["T.zenithproject", "a.png", "b.png"]).is_ok(),
			"different names are fine"
		);
	}

	#[test]
	fn entry_set_rules() {
		assert!(
			validate_entry_set(&["Assets/a", "Test.zenithproject"]).is_ok(),
			"valid set"
		);
		assert!(validate_entry_set(&["Assets/a"]).is_err(), "no manifest");
		assert!(
			validate_entry_set(&["A.zenithproject", "B.zenithproject"]).is_err(),
			"two manifests"
		);
		assert!(
			validate_entry_set(&["T.zenithproject", "sub/X.zenithproject"]).is_ok(),
			"nested manifest does not count"
		);
		assert!(validate_entry_set(&["b", "a", "T.zenithproject"]).is_err(), "unsorted");
		assert!(validate_entry_set(&["a", "a", "T.zenithproject"]).is_err(), "duplicate");
		assert!(
			validate_entry_set(&["T.zenithproject", "x", "x/y"]).is_err(),
			"file and directory conflict"
		);
		assert!(
			validate_entry_set(&[".zenithproject"]).is_err(),
			"empty stem is not a manifest"
		);
	}
}
