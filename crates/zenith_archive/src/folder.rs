//! Folder container: `build.index` plus one self-contained data file per
//! project file, so an update only touches the files that changed.

use std::{collections::HashMap, io, sync::Arc};

use zenith_error::{ZResult, ZenithError};

use crate::{
	build_id::BuildId,
	crypto::EncryptionKey,
	ctx::FileCtx,
	entry_reader::EntryReader,
	error::{corrupt, limit, to_usize},
	format::{HEADER_LEN_U64, INDEX_FILE_NAME, INDEX_IDENT, Kind, MAX_ENTRIES, MAX_PATH_LEN, Rd, TABLE_IDENT_PREFIX},
	layout::{expected_chunk_count, make_layout, read_chunk_table},
	path::validate_entry_set,
	read_at::ReadAt,
};

/// Smallest index record: `path_len`(2) + path(1) + `name_len`(2) + name(1) +
/// size(8) + `data_len`(8) + `content_hash`(32) + level(1) +
/// `chunk_size_log2`(1).
const MIN_RECORD_LEN: usize = 56;

/// Where the files of a folder build come from.
///
/// `name` is always a validated relative path with `/` separators; it never
/// contains `..`, a drive prefix or a leading slash, so implementations may
/// join it onto their root. The std-file implementation lives where the file
/// API is available (not in this crate), keeping the reader free of it.
pub trait DirSource: Send + Sync {
	/// Opens `build.index`.
	///
	/// # Errors
	/// Propagates the underlying I/O error.
	fn open_index(&self) -> io::Result<Arc<dyn ReadAt>>;

	/// Opens a data file by its name from the index.
	///
	/// # Errors
	/// Propagates the underlying I/O error.
	fn open_data(&self, name: &str) -> io::Result<Arc<dyn ReadAt>>;
}

/// A folder build held in memory: file name to bytes. Used by tests and by
/// hosts that fetched the files themselves.
#[derive(Default)]
pub struct MemDir {
	files: HashMap<String, Arc<[u8]>>,
}

impl MemDir {
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Adds a file; the index goes under [`INDEX_FILE_NAME`].
	pub fn insert(&mut self, name: impl Into<String>, bytes: impl Into<Arc<[u8]>>) {
		self.files.insert(name.into(), bytes.into());
	}

	fn get(&self, name: &str) -> io::Result<Arc<dyn ReadAt>> {
		let bytes = self
			.files
			.get(name)
			.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no such file in build: {name}")))?;
		Ok(Arc::new(bytes.clone()))
	}
}

impl DirSource for MemDir {
	fn open_index(&self) -> io::Result<Arc<dyn ReadAt>> {
		self.get(INDEX_FILE_NAME)
	}

	fn open_data(&self, name: &str) -> io::Result<Arc<dyn ReadAt>> {
		self.get(name)
	}
}

/// One file of a folder build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderEntry {
	pub path: String,
	/// Uncompressed size in bytes.
	pub size: u64,
	/// Name of the data file, relative to the build folder.
	pub data_name: String,
	/// Size of the data file on disk.
	pub data_len: u64,
	/// Hash of the entry's source content, chosen by the build tool. It is also
	/// stored in the data file, and the two must agree, which ties a data file
	/// to the index it was built with. Lets a rebuild skip files that did not
	/// change.
	pub content_hash: [u8; 32],
	/// zstd level the data file was compressed with (`1..=22`).
	pub compression_level: u8,
	/// Chunk size of the data file as a power of two; always the build's.
	pub chunk_size_log2: u8,
}

/// Reader of a folder build.
pub struct FolderReader {
	src: Arc<dyn DirSource>,
	root: FileCtx,
	entries: Vec<FolderEntry>,
	id: BuildId,
}

impl FolderReader {
	/// Opens `build.index` and verifies it. Data files are opened and verified
	/// when an entry is read.
	///
	/// # Errors
	/// Fails for a damaged index, unsupported versions, and a missing,
	/// unneeded or wrong key.
	pub fn open(src: impl DirSource + 'static, key: Option<&EncryptionKey>) -> ZResult<Self> {
		let root = FileCtx::open_root(src.open_index()?, Kind::FolderIndex, key)?;
		let (body, _, id) = root.load_body(Kind::FolderIndex, INDEX_IDENT)?;
		let entries = parse_index(&body, &root)?;
		Ok(Self {
			src: Arc::new(src),
			root,
			entries,
			id,
		})
	}

	/// The identity of this build (see [`BuildId`]): made from `build.index`,
	/// which lists every data file with what pins its content.
	#[must_use]
	pub const fn build_id(&self) -> BuildId {
		self.id
	}

	#[must_use]
	pub const fn is_encrypted(&self) -> bool {
		self.root.header.encrypted
	}

	/// All entries, sorted by path.
	#[must_use]
	pub fn entries(&self) -> &[FolderEntry] {
		&self.entries
	}

	#[must_use]
	pub fn find(&self, path: &str) -> Option<&FolderEntry> {
		self.entries
			.binary_search_by(|e| e.path.as_str().cmp(path))
			.ok()
			.and_then(|i| self.entries.get(i))
	}

	/// Opens a streaming `Read + Seek` view of one entry. The data file's
	/// header, footer and chunk table are verified here.
	///
	/// # Errors
	/// `ArchiveEntryNotFound`, an I/O error, or a corruption error if the data
	/// file does not match the index (including a file swapped in from the
	/// same build, which fails authentication when encrypted).
	pub fn open_entry(&self, path: &str) -> ZResult<EntryReader> {
		let entry = self
			.find(path)
			.ok_or_else(|| ZenithError::ArchiveEntryNotFound(path.to_string()))?;
		let src = self.src.open_data(&entry.data_name)?;
		if src.len() != entry.data_len {
			return Err(corrupt("data file length does not match the index"));
		}
		let ctx = self.root.open_sibling(src, Kind::FolderData)?;

		let ident = [TABLE_IDENT_PREFIX, path.as_bytes()].concat();
		let (table, table_offset, _) = ctx.load_body(Kind::FolderData, &ident)?;
		let mut r = Rd::new(&table);
		let size = r.u64()?;
		let chunk_count = u64::from(r.u32()?);
		let content_hash: [u8; 32] = r.array()?;
		if size != entry.size {
			return Err(corrupt("data file size does not match the index"));
		}
		if content_hash != entry.content_hash {
			return Err(corrupt(
				"data file was built from other content than the index lists (a file of another version?)",
			));
		}
		if chunk_count != expected_chunk_count(size, ctx.header.chunk_size) {
			return Err(corrupt("chunk count does not match the entry size"));
		}
		let chunks = read_chunk_table(&mut r, chunk_count)?;
		if !r.is_empty() {
			return Err(corrupt("trailing bytes after the chunk table"));
		}
		let (layout, end) = make_layout(size, HEADER_LEN_U64, chunks, &ctx.header, table_offset)?;
		if end != table_offset {
			return Err(corrupt("unused bytes between the chunks and the table"));
		}
		Ok(EntryReader::new(Arc::new(ctx), Arc::from(path), Arc::new(layout)))
	}

	/// Reads one entry completely.
	///
	/// # Errors
	/// Same as [`Self::open_entry`], plus damaged chunks.
	pub fn read_all(&self, path: &str) -> ZResult<Vec<u8>> {
		self.open_entry(path)?.read_all()
	}
}

fn parse_index(body: &[u8], root: &FileCtx) -> ZResult<Vec<FolderEntry>> {
	let mut r = Rd::new(body);
	let count = to_usize(u64::from(r.u32()?))?;
	if count > MAX_ENTRIES {
		return Err(limit("too many entries"));
	}
	if count
		.checked_mul(MIN_RECORD_LEN)
		.is_none_or(|needed| needed > r.remaining())
	{
		return Err(corrupt("entry count does not fit into the index"));
	}
	let mut entries = Vec::with_capacity(count);
	for _ in 0..count {
		let path = read_str(&mut r, MAX_PATH_LEN)?;
		let name = read_str(&mut r, MAX_PATH_LEN)?;
		let size = r.u64()?;
		let data_len = r.u64()?;
		let content_hash: [u8; 32] = r.array()?;
		let compression_level = r.u8()?;
		let chunk_size_log2 = r.u8()?;
		// 0 is a stored (uncompressed) file.
		let level_ok = if root.header.compressed {
			(1..=22).contains(&compression_level)
		} else {
			compression_level == 0
		};
		if !level_ok {
			return Err(corrupt("compression level out of range"));
		}
		if chunk_size_log2 != root.header.chunk_size_log2 {
			return Err(corrupt("entry chunk size differs from the build's"));
		}
		// The name is derived, never trusted: recomputing it makes a hostile
		// index unable to point at an arbitrary file.
		let expected = root
			.crypto
			.as_ref()
			.map_or_else(|| path.clone(), |crypto| crypto.data_file_name(&path));
		if name != expected || (root.crypto.is_none() && path == INDEX_FILE_NAME) {
			return Err(corrupt("data file name does not match the entry path"));
		}
		entries.push(FolderEntry {
			path,
			size,
			data_name: name,
			data_len,
			content_hash,
			compression_level,
			chunk_size_log2,
		});
	}
	if !r.is_empty() {
		return Err(corrupt("trailing bytes after the index"));
	}
	let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
	validate_entry_set(&paths)?;
	Ok(entries)
}

fn read_str(r: &mut Rd<'_>, max: usize) -> ZResult<String> {
	let len = usize::from(r.u16()?);
	if len > max {
		return Err(corrupt("string in the index is too long"));
	}
	Ok(std::str::from_utf8(r.take(len)?)
		.map_err(|_| corrupt("string in the index is not valid UTF-8"))?
		.to_string())
}
