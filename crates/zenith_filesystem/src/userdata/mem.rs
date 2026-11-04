//! A store in memory.

use std::{
	collections::BTreeMap,
	sync::{Mutex, MutexGuard, PoisonError},
};

use zenith_error::{ZResult, ZenithError};

use super::UserStore;
use crate::types::{DirEntry, EntryKind, Metadata};

/// Files kept in memory and gone with the process: for tests, and for a
/// browser that cannot keep anything.
#[derive(Debug, Default)]
pub struct MemStore {
	files: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl MemStore {
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	fn files(&self) -> MutexGuard<'_, BTreeMap<String, Vec<u8>>> {
		self.files.lock().unwrap_or_else(PoisonError::into_inner)
	}

	/// Puts a file in without any checks: for a store that loads what it kept
	/// somewhere else.
	pub fn insert_raw(&self, path: &str, data: Vec<u8>) {
		self.files().insert(path.to_string(), data);
	}

	/// Takes a file out; for the same kind of store.
	pub fn remove_raw(&self, path: &str) {
		self.files().remove(path);
	}

	/// The files at `path` (the file itself, or everything in the folder).
	#[must_use]
	pub fn paths_under(&self, path: &str) -> Vec<String> {
		let folder = prefix(path);
		self.files()
			.keys()
			.filter(|key| *key == path || key.starts_with(&folder))
			.cloned()
			.collect()
	}

	/// Every file, for a store that saves what it holds somewhere else.
	#[must_use]
	pub fn snapshot(&self) -> Vec<(String, Vec<u8>)> {
		self.files().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
	}
}

/// `path/` for a folder; the root is everything.
fn prefix(path: &str) -> String {
	if path.is_empty() {
		String::new()
	} else {
		format!("{path}/")
	}
}

// The lock is held for the whole of each short operation on purpose: a check
// and the change that follows it must not have another thread between them.
#[allow(clippy::significant_drop_tightening)]
impl UserStore for MemStore {
	fn read(&self, path: &str) -> ZResult<Option<Vec<u8>>> {
		let (data, is_folder) = {
			let files = self.files();
			let folder = prefix(path);
			(
				files.get(path).cloned(),
				!path.is_empty() && files.keys().any(|key| key.starts_with(&folder)),
			)
		};
		if data.is_none() && is_folder {
			return Err(ZenithError::ProjectWrongKind(format!("Data://{path} is a folder")));
		}
		Ok(data)
	}

	fn write(&self, path: &str, data: &[u8]) -> ZResult<()> {
		if path.is_empty() {
			return Err(ZenithError::ProjectWrongKind("path is empty".into()));
		}

		let mut files = self.files();
		let folder = prefix(path);
		if files.keys().any(|key| key.starts_with(&folder)) {
			return Err(ZenithError::ProjectWrongKind(format!("Data://{path} is a folder")));
		}
		// A file where a folder of the new file would have to be.
		let mut parent = String::new();
		for name in path.split('/').take(path.split('/').count().saturating_sub(1)) {
			if !parent.is_empty() {
				parent.push('/');
			}
			parent.push_str(name);
			if files.contains_key(&parent) {
				return Err(ZenithError::ProjectWrongKind(format!("Data://{parent} is a file")));
			}
		}
		files.insert(path.to_string(), data.to_vec());
		Ok(())
	}

	fn remove(&self, path: &str) -> ZResult<bool> {
		let mut files = self.files();
		let folder = prefix(path);
		let before = files.len();
		files.retain(|key, _| key != path && !key.starts_with(&folder));
		Ok(files.len() != before)
	}

	fn metadata(&self, path: &str) -> ZResult<Option<Metadata>> {
		let files = self.files();
		if let Some(data) = files.get(path) {
			return Ok(Some(Metadata {
				kind: EntryKind::File,
				size: data.len() as u64,
			}));
		}
		let folder = prefix(path);
		let is_dir = path.is_empty() || files.keys().any(|key| key.starts_with(&folder));
		Ok(is_dir.then_some(Metadata {
			kind: EntryKind::Dir,
			size: 0,
		}))
	}

	fn list(&self, path: &str) -> ZResult<Vec<DirEntry>> {
		let files = self.files();
		let folder = prefix(path);
		let mut entries: BTreeMap<String, EntryKind> = BTreeMap::new();
		for key in files.keys() {
			let Some(rest) = key.strip_prefix(&folder) else {
				continue;
			};
			match rest.split_once('/') {
				Some((name, _)) => {
					entries.insert(name.to_string(), EntryKind::Dir);
				}
				None => {
					entries.insert(rest.to_string(), EntryKind::File);
				}
			}
		}
		if entries.is_empty() && !path.is_empty() {
			return Err(ZenithError::ProjectNotFound(format!("Data://{path}")));
		}
		Ok(entries
			.into_iter()
			.map(|(name, kind)| DirEntry { name, kind })
			.collect())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn files_and_folders_behave() -> ZResult<()> {
		let store = MemStore::new();
		assert_eq!(store.read("a.txt")?, None, "nothing yet");
		assert!(store.list("")?.is_empty(), "the root exists and is empty");
		store.write("Saves/one.bin", b"1")?;
		store.write("Saves/two.bin", b"22")?;
		store.write("Input.toml", b"x")?;
		assert_eq!(store.read("Saves/two.bin")?, Some(b"22".to_vec()), "read back");
		assert_eq!(
			store.list("")?,
			vec![
				DirEntry {
					name: "Input.toml".into(),
					kind: EntryKind::File
				},
				DirEntry {
					name: "Saves".into(),
					kind: EntryKind::Dir
				},
			],
			"the root lists a file and a folder, sorted"
		);
		assert_eq!(store.list("Saves")?.len(), 2, "two files");
		assert!(store.list("Nope").is_err(), "a folder that is not there");
		assert_eq!(
			store.metadata("Saves")?.map(|m| m.kind),
			Some(EntryKind::Dir),
			"a folder"
		);
		assert_eq!(
			store.metadata("Saves/one.bin")?.map(|m| m.size),
			Some(1),
			"a file's size"
		);
		assert_eq!(store.metadata("Nope")?, None, "nothing");
		assert!(store.read("Saves").is_err(), "a folder is not a file");
		assert!(store.write("Saves", b"x").is_err(), "a file cannot replace a folder");
		assert!(
			store.write("Input.toml/inside", b"x").is_err(),
			"a file cannot be a folder"
		);
		assert!(store.remove("Saves")?, "a folder goes with what is in it");
		assert!(!store.remove("Saves")?, "only once");
		assert_eq!(store.read("Saves/one.bin")?, None, "gone");
		Ok(())
	}
}
