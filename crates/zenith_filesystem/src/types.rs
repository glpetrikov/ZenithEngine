use std::io::{Read, Seek};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
	File,
	Dir,
	/// Only reported by `list`/`is_symlink`; reads follow the link (inside the
	/// project).
	Symlink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metadata {
	/// `File` or `Dir`; links are followed.
	pub kind: EntryKind,
	/// Size in bytes; 0 for a folder.
	pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
	pub name: String,
	/// What the entry itself is; a link is reported as `Symlink`.
	pub kind: EntryKind,
}

/// Streaming read access to one file.
pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}
