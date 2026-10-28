//! Directory view of a build. Builds store a flat, sorted list of files; the
//! folders only exist implicitly, so `list`/`exists` need this derived tree.

use std::collections::{BTreeMap, HashMap};

use crate::types::{EntryKind, Metadata};

#[derive(Default)]
pub struct Tree {
	files: HashMap<String, u64>,
	/// folder path ("" is the root) -> children by name
	dirs: HashMap<String, BTreeMap<String, EntryKind>>,
}

impl Tree {
	pub fn from_files<'a>(files: impl IntoIterator<Item = (&'a str, u64)>) -> Self {
		let mut tree = Self::default();
		tree.dirs.insert(String::new(), BTreeMap::new());
		for (path, size) in files {
			tree.files.insert(path.to_string(), size);
			let mut child = path;
			let mut kind = EntryKind::File;
			loop {
				let (parent, name) = child.rsplit_once('/').unwrap_or(("", child));
				let known = tree.dirs.contains_key(parent);
				tree.dirs
					.entry(parent.to_string())
					.or_default()
					.insert(name.to_string(), kind);
				if known || parent.is_empty() {
					break;
				}
				child = parent;
				kind = EntryKind::Dir;
			}
		}
		tree
	}

	pub fn metadata(&self, rel: &str) -> Option<Metadata> {
		if let Some(size) = self.files.get(rel) {
			return Some(Metadata {
				kind: EntryKind::File,
				size: *size,
			});
		}
		self.dirs.contains_key(rel).then_some(Metadata {
			kind: EntryKind::Dir,
			size: 0,
		})
	}

	pub fn children(&self, rel: &str) -> Option<impl Iterator<Item = (&str, EntryKind)>> {
		self.dirs.get(rel).map(|c| c.iter().map(|(n, k)| (n.as_str(), *k)))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn derives_folders() {
		let t = Tree::from_files([("Assets/a/b.txt", 3), ("Assets/c.txt", 1), ("T.zenithproject", 9)]);
		assert_eq!(
			t.metadata("Assets/a/b.txt"),
			Some(Metadata {
				kind: EntryKind::File,
				size: 3
			}),
			"file"
		);
		assert_eq!(
			t.metadata("Assets/a").map(|m| m.kind),
			Some(EntryKind::Dir),
			"nested folder"
		);
		assert_eq!(t.metadata("").map(|m| m.kind), Some(EntryKind::Dir), "root");
		assert!(t.metadata("Assets/zzz").is_none(), "missing");
		let root: Vec<_> = t.children("").map(Iterator::collect).unwrap_or_default();
		assert_eq!(
			root,
			vec![("Assets", EntryKind::Dir), ("T.zenithproject", EntryKind::File)],
			"root children"
		);
		let assets: Vec<_> = t.children("Assets").map(Iterator::collect).unwrap_or_default();
		assert_eq!(
			assets,
			vec![("a", EntryKind::Dir), ("c.txt", EntryKind::File)],
			"sorted children"
		);
	}
}
