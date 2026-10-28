use std::path::{Path, PathBuf};

use zenith_filesystem::{Project, ZPath};

pub type TestResult = Result<(), Box<dyn std::error::Error>>;

pub fn z(text: &str) -> Result<ZPath, Box<dyn std::error::Error>> {
	Ok(ZPath::parse(text)?)
}

/// Creates project `name` inside a fresh temporary folder.
pub fn new_project(name: &str) -> Result<(tempfile::TempDir, PathBuf, Project), Box<dyn std::error::Error>> {
	let tmp = tempfile::tempdir()?;
	let root = tmp.path().join(name);
	let project = Project::create(&root)?;
	Ok((tmp, root, project))
}

/// Recursive copy, used to run the tests against a copy of `Sandbox/Test`.
pub fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
	std::fs::create_dir_all(to)?;
	for entry in std::fs::read_dir(from)? {
		let entry = entry?;
		let target = to.join(entry.file_name());
		if entry.file_type()?.is_dir() {
			copy_tree(&entry.path(), &target)?;
		} else {
			std::fs::copy(entry.path(), target)?;
		}
	}
	Ok(())
}

/// Sorted names of the entries of a folder on disk.
pub fn names(dir: &Path) -> Vec<String> {
	let mut names: Vec<String> = std::fs::read_dir(dir)
		.map(|rd| rd.flatten().filter_map(|e| e.file_name().into_string().ok()).collect())
		.unwrap_or_default();
	names.sort();
	names
}
