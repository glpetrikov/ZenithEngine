//! A store in a folder of the user's data.

use std::{
	io::{self, Write},
	path::{Path, PathBuf},
};

use cap_std::{ambient_authority, fs::Dir};
use zenith_error::{ZResult, ZenithError};

use super::UserStore;
use crate::{
	layout::TMP_SUFFIX,
	types::{DirEntry, EntryKind, Metadata},
};

/// Files in a folder. The folder is made when the first file is written, not
/// before: opening a game must not leave empty folders behind.
///
/// Every access goes through a directory handle, so nothing can leave the
/// folder (a link in it included).
#[derive(Debug)]
pub struct DirStore {
	root: PathBuf,
}

fn map_io(error: io::Error, path: &str) -> ZenithError {
	match error.kind() {
		io::ErrorKind::NotFound => ZenithError::ProjectNotFound(format!("Data://{path}")),
		_ => ZenithError::Io(error),
	}
}

impl DirStore {
	#[must_use]
	pub fn new(root: impl Into<PathBuf>) -> Self {
		Self { root: root.into() }
	}

	/// The folder the files are in.
	#[must_use]
	pub fn root(&self) -> &Path {
		&self.root
	}

	/// The folder, if it exists.
	fn open(&self) -> io::Result<Option<Dir>> {
		match Dir::open_ambient_dir(&self.root, ambient_authority()) {
			Ok(dir) => Ok(Some(dir)),
			Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
			Err(e) => Err(e),
		}
	}

	/// The folder, made if it is not there.
	fn open_or_create(&self) -> io::Result<Dir> {
		std::fs::create_dir_all(&self.root)?;
		Dir::open_ambient_dir(&self.root, ambient_authority())
	}
}

impl UserStore for DirStore {
	fn read(&self, path: &str) -> ZResult<Option<Vec<u8>>> {
		let Some(dir) = self.open()? else {
			return Ok(None);
		};
		match dir.metadata(path) {
			Ok(meta) if meta.is_dir() => Err(ZenithError::ProjectWrongKind(format!("Data://{path} is a folder"))),
			Ok(_) => dir.read(path).map(Some).map_err(|e| map_io(e, path)),
			Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
			Err(e) => Err(ZenithError::Io(e)),
		}
	}

	fn write(&self, path: &str, data: &[u8]) -> ZResult<()> {
		let dir = self.open_or_create()?;
		if let Some((parent, _)) = path.rsplit_once('/') {
			dir.create_dir_all(parent).map_err(|e| map_io(e, path))?;
		}
		if dir.metadata(path).is_ok_and(|meta| meta.is_dir()) {
			return Err(ZenithError::ProjectWrongKind(format!("Data://{path} is a folder")));
		}
		// Written next to the target and flushed to disk before the rename:
		// after a crash or a power loss the old file or the new one is there,
		// never an empty or a half one.
		let tmp = format!("{path}{TMP_SUFFIX}");
		let written = dir.create(&tmp).and_then(|mut file| {
			file.write_all(data)?;
			file.sync_all()
		});
		if let Err(e) = written {
			let _ = dir.remove_file(&tmp);
			return Err(map_io(e, path));
		}
		dir.rename(&tmp, &dir, path).map_err(|e| {
			let _ = dir.remove_file(&tmp);
			map_io(e, path)
		})
	}

	fn remove(&self, path: &str) -> ZResult<bool> {
		let Some(dir) = self.open()? else {
			return Ok(false);
		};
		match dir.symlink_metadata(path) {
			Ok(meta) if meta.is_dir() => dir.remove_dir_all(path).map(|()| true).map_err(|e| map_io(e, path)),
			Ok(_) => dir.remove_file(path).map(|()| true).map_err(|e| map_io(e, path)),
			Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
			Err(e) => Err(ZenithError::Io(e)),
		}
	}

	fn metadata(&self, path: &str) -> ZResult<Option<Metadata>> {
		let Some(dir) = self.open()? else {
			// The root always exists, whether or not it has been made yet.
			return Ok(path.is_empty().then_some(Metadata {
				kind: EntryKind::Dir,
				size: 0,
			}));
		};
		if path.is_empty() {
			return Ok(Some(Metadata {
				kind: EntryKind::Dir,
				size: 0,
			}));
		}
		match dir.metadata(path) {
			Ok(meta) => Ok(Some(Metadata {
				kind: if meta.is_dir() { EntryKind::Dir } else { EntryKind::File },
				size: if meta.is_dir() { 0 } else { meta.len() },
			})),
			Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
			Err(e) => Err(ZenithError::Io(e)),
		}
	}

	fn list(&self, path: &str) -> ZResult<Vec<DirEntry>> {
		let Some(dir) = self.open()? else {
			return if path.is_empty() {
				Ok(Vec::new())
			} else {
				Err(ZenithError::ProjectNotFound(format!("Data://{path}")))
			};
		};
		let entries = if path.is_empty() {
			dir.entries()
		} else {
			dir.read_dir(path)
		}
		.map_err(|e| map_io(e, path))?;
		let mut out = Vec::new();
		for entry in entries {
			let entry = entry?;
			let Ok(name) = entry.file_name().into_string() else {
				continue;
			};
			if name.ends_with(TMP_SUFFIX) {
				continue;
			}
			let kind = match entry.file_type()? {
				kind if kind.is_dir() => EntryKind::Dir,
				kind if kind.is_symlink() => EntryKind::Symlink,
				_ => EntryKind::File,
			};
			out.push(DirEntry { name, kind });
		}
		out.sort_by(|a, b| a.name.cmp(&b.name));
		Ok(out)
	}

	fn create_dir(&self, path: &str) -> ZResult<()> {
		let dir = self.open_or_create()?;
		dir.create_dir_all(path).map_err(|e| map_io(e, path))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	type TestResult = Result<(), Box<dyn std::error::Error>>;

	#[test]
	fn nothing_is_made_until_something_is_written() -> TestResult {
		let tmp = tempfile::tempdir()?;
		let root = tmp.path().join("Game");
		let store = DirStore::new(&root);
		assert_eq!(store.read("a.txt")?, None, "nothing to read");
		assert!(store.list("")?.is_empty(), "the root is empty");
		assert!(!store.remove("a.txt")?, "nothing to remove");
		assert!(store.metadata("")?.is_some(), "the root exists as an idea");
		assert!(!root.exists(), "and no folder was made for any of that");
		store.write("a.txt", b"x")?;
		assert!(root.is_dir(), "the first write makes it");
		Ok(())
	}

	#[test]
	fn a_write_replaces_atomically_and_leaves_nothing_behind() -> TestResult {
		let tmp = tempfile::tempdir()?;
		let store = DirStore::new(tmp.path().join("Game"));
		store.write("Saves/one.bin", b"first")?;
		store.write("Saves/one.bin", b"second, longer")?;
		assert_eq!(
			store.read("Saves/one.bin")?,
			Some(b"second, longer".to_vec()),
			"replaced"
		);
		let names: Vec<String> = store.list("Saves")?.into_iter().map(|e| e.name).collect();
		assert_eq!(names, ["one.bin"], "no temporary file is left");
		Ok(())
	}

	#[test]
	fn folders_files_and_errors() -> TestResult {
		let tmp = tempfile::tempdir()?;
		let store = DirStore::new(tmp.path().join("Game"));
		store.write("b.txt", b"2")?;
		store.write("a/inner.txt", b"1")?;
		store.create_dir("empty/deeper")?;
		let listed: Vec<(String, EntryKind)> = store.list("")?.into_iter().map(|e| (e.name, e.kind)).collect();
		assert_eq!(
			listed,
			vec![
				("a".to_string(), EntryKind::Dir),
				("b.txt".to_string(), EntryKind::File),
				("empty".to_string(), EntryKind::Dir)
			],
			"sorted, with kinds"
		);
		assert!(store.list("nope").is_err(), "a folder that is not there");
		assert!(store.read("a").is_err(), "a folder is not a file");
		assert!(store.write("a", b"x").is_err(), "a file cannot replace a folder");
		assert!(store.write("b.txt/inner", b"x").is_err(), "a file cannot be a folder");
		assert_eq!(store.metadata("b.txt")?.map(|m| m.size), Some(1), "size");
		assert!(store.remove("a")?, "a folder goes with its files");
		assert!(store.remove("b.txt")?, "a file goes");
		assert_eq!(store.read("b.txt")?, None, "gone");
		Ok(())
	}

	#[test]
	fn nothing_can_leave_the_folder() -> TestResult {
		let tmp = tempfile::tempdir()?;
		std::fs::write(tmp.path().join("outside.txt"), b"secret")?;
		let store = DirStore::new(tmp.path().join("Game"));
		store.write("in.txt", b"x")?;
		assert!(
			store.read("../outside.txt").is_err(),
			"the folder above is out of reach"
		);
		assert!(store.write("../evil.txt", b"x").is_err(), "so is writing to it");
		assert!(!tmp.path().join("evil.txt").exists(), "and nothing was written there");
		Ok(())
	}
}
