//! Single-file container ("archive"): header, chunks of all files back to
//! back, index, footer.

use std::sync::Arc;

use zenith_error::{ZResult, ZenithError};

use crate::{
	build_id::BuildId,
	crypto::EncryptionKey,
	ctx::FileCtx,
	entry_reader::EntryReader,
	error::{corrupt, limit, to_usize},
	format::{HEADER_LEN_U64, INDEX_IDENT, Kind, MAX_ENTRIES, MAX_PATH_LEN, Rd},
	layout::{FileLayout, expected_chunk_count, make_layout, read_chunk_table},
	path::validate_entry_set,
	read_at::ReadAt,
};

/// Smallest index record: `path_len`(2) + path(1) + size(8) + offset(8) +
/// chunks(4).
const MIN_RECORD_LEN: usize = 23;

/// One file stored in an archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveEntry {
	pub path: String,
	/// Uncompressed size in bytes.
	pub size: u64,
}

/// Reader of a single-file build. Cheap to share: all methods take `&self`.
pub struct ArchiveReader {
	ctx: Arc<FileCtx>,
	entries: Vec<ArchiveEntry>,
	layouts: Vec<Arc<FileLayout>>,
	id: BuildId,
}

impl ArchiveReader {
	/// Opens an archive and verifies its header, footer and index. Chunks are
	/// verified lazily, when they are read.
	///
	/// `src` is a file ([`crate::FileSource`]) or bytes in memory
	/// (`Arc<[u8]>`).
	///
	/// # Errors
	/// Fails for damaged or truncated archives, unsupported versions, and a
	/// missing, unneeded or wrong key.
	pub fn open(src: impl ReadAt + 'static, key: Option<&EncryptionKey>) -> ZResult<Self> {
		let ctx = FileCtx::open_root(Arc::new(src), Kind::Archive, key)?;
		let (body, index_offset, id) = ctx.load_body(Kind::Archive, INDEX_IDENT)?;
		let (entries, layouts) = parse_index(&body, &ctx, index_offset)?;
		Ok(Self {
			ctx: Arc::new(ctx),
			entries,
			layouts,
			id,
		})
	}

	/// The identity of this build (see [`BuildId`]).
	#[must_use]
	pub const fn build_id(&self) -> BuildId {
		self.id
	}

	#[must_use]
	pub fn is_encrypted(&self) -> bool {
		self.ctx.header.encrypted
	}

	/// All entries, sorted by path.
	#[must_use]
	pub fn entries(&self) -> &[ArchiveEntry] {
		&self.entries
	}

	#[must_use]
	pub fn find(&self, path: &str) -> Option<&ArchiveEntry> {
		self.position(path).and_then(|i| self.entries.get(i))
	}

	fn position(&self, path: &str) -> Option<usize> {
		self.entries.binary_search_by(|e| e.path.as_str().cmp(path)).ok()
	}

	/// Opens a streaming `Read + Seek` view of one entry.
	///
	/// # Errors
	/// `ArchiveEntryNotFound` if the path is not in the archive.
	pub fn open_entry(&self, path: &str) -> ZResult<EntryReader> {
		let idx = self
			.position(path)
			.ok_or_else(|| ZenithError::ArchiveEntryNotFound(path.to_string()))?;
		let layout = self
			.layouts
			.get(idx)
			.ok_or_else(|| ZenithError::ArchiveEntryNotFound(path.to_string()))?;
		Ok(EntryReader::new(self.ctx.clone(), Arc::from(path), layout.clone()))
	}

	/// Reads one entry completely, decompressing only that entry's chunks.
	///
	/// # Errors
	/// `ArchiveEntryNotFound`, or a corruption error for damaged chunks.
	pub fn read_all(&self, path: &str) -> ZResult<Vec<u8>> {
		self.open_entry(path)?.read_all()
	}
}

type Parsed = (Vec<ArchiveEntry>, Vec<Arc<FileLayout>>);

fn parse_index(body: &[u8], ctx: &FileCtx, index_offset: u64) -> ZResult<Parsed> {
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
	let mut layouts = Vec::with_capacity(count);
	let mut next = HEADER_LEN_U64;
	for _ in 0..count {
		let path_len = usize::from(r.u16()?);
		if path_len > MAX_PATH_LEN {
			return Err(corrupt("entry path is too long"));
		}
		let path = std::str::from_utf8(r.take(path_len)?)
			.map_err(|_| corrupt("entry path is not valid UTF-8"))?
			.to_string();
		let size = r.u64()?;
		let data_offset = r.u64()?;
		let chunk_count = u64::from(r.u32()?);
		if data_offset != next {
			return Err(corrupt("entry data is not contiguous"));
		}
		if chunk_count != expected_chunk_count(size, ctx.header.chunk_size) {
			return Err(corrupt("chunk count does not match the entry size"));
		}
		let chunks = read_chunk_table(&mut r, chunk_count)?;
		let (layout, end) = make_layout(size, data_offset, chunks, &ctx.header, index_offset)?;
		next = end;
		entries.push(ArchiveEntry { path, size });
		layouts.push(Arc::new(layout));
	}
	if next != index_offset {
		return Err(corrupt("unused bytes between the last entry and the index"));
	}
	if !r.is_empty() {
		return Err(corrupt("trailing bytes after the index"));
	}
	let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
	validate_entry_set(&paths)?;
	Ok((entries, layouts))
}
