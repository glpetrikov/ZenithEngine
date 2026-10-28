//! Folder-build writer pieces (feature `write`).
//!
//! This crate only produces bytes; deciding which files to rewrite, atomic
//! replacement and deleting stale files is the build tool's job.

use std::io::{Read, Write};

use zenith_error::ZResult;

use crate::{
	error::{limit, to_u64},
	format::{FOOTER_LEN, HEADER_LEN_U64, INDEX_FILE_NAME, INDEX_IDENT, Kind, MAX_ENTRIES, TABLE_IDENT_PREFIX},
	layout::encode_chunk_table,
	path::{validate_entry_path, validate_entry_set},
	write::{Sealer, WriterOptions, write_err},
};

/// Result of writing one data file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataFileInfo {
	/// Uncompressed size of the entry.
	pub size: u64,
	/// Size of the data file that was written.
	pub data_len: u64,
	/// The content hash that was stored in the data file; the index entry of
	/// the file must carry the same one.
	pub content_hash: [u8; 32],
}

/// One record of `build.index`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderIndexEntry {
	pub path: String,
	pub size: u64,
	pub data_len: u64,
	/// Hash of the source content, computed by the caller (BLAKE3 in
	/// `zenith_build`). Stored as is; inside an encrypted index it is hidden.
	pub content_hash: [u8; 32],
	/// zstd level used for the data file, `1..=22`.
	pub compression_level: u8,
	/// Chunk size used for the data file; must be the writer's.
	pub chunk_size_log2: u8,
}

/// Produces the data files and the index of a folder build.
pub struct FolderWriter {
	sealer: Sealer,
}

impl FolderWriter {
	/// # Errors
	/// Fails for invalid options.
	pub fn new(opts: &WriterOptions) -> ZResult<Self> {
		Ok(Self {
			sealer: Sealer::new(opts)?,
		})
	}

	/// Name of the data file for an entry, stable between builds: the project
	/// path itself, or with a key `ab/<30 hex>` derived from the path.
	///
	/// # Errors
	/// `ArchiveInvalidPath` for an invalid path.
	pub fn data_file_name(&self, path: &str) -> ZResult<String> {
		validate_entry_path(path)?;
		Ok(self
			.sealer
			.crypto
			.as_ref()
			.map_or_else(|| path.to_string(), |crypto| crypto.data_file_name(path)))
	}

	/// Writes one self-contained data file: header, chunks, chunk table,
	/// footer.
	///
	/// `content_hash` is called once `src` has been read to the end and returns
	/// the hash of what was read (the build tool hashes while it reads, for
	/// example in a wrapper around the source, which is why it gets `src`
	/// back). The hash goes into the data file's chunk table, and a reader
	/// checks it against the index entry's `content_hash`, so a data file of
	/// another version of the file does not fit an index it was not built
	/// with.
	///
	/// # Errors
	/// Fails for an invalid path, an unreadable source, or a write error.
	pub fn write_data_file<R: Read>(
		&mut self,
		mut out: impl Write,
		path: &str,
		src: &mut R,
		content_hash: impl FnOnce(&mut R) -> [u8; 32],
	) -> ZResult<DataFileInfo> {
		validate_entry_path(path)?;
		let header = self.sealer.header(Kind::FolderData)?;
		out.write_all(&header.raw)?;
		let mut pos = HEADER_LEN_U64;
		let (size, chunks) = self.sealer.write_chunks(&mut out, &header, path, src, &mut pos)?;
		let content_hash = content_hash(src);

		let mut table = Vec::new();
		table.extend_from_slice(&size.to_le_bytes());
		table.extend_from_slice(
			&u32::try_from(chunks.len())
				.map_err(|_| limit("too many chunks"))?
				.to_le_bytes(),
		);
		table.extend_from_slice(&content_hash);
		encode_chunk_table(&chunks, &mut table);
		let ident = [TABLE_IDENT_PREFIX, path.as_bytes()].concat();
		let data_len = self
			.sealer
			.write_body_and_footer(&mut out, &header, Kind::FolderData, &ident, table, pos)?;
		out.flush()?;
		debug_assert!(data_len > to_u64(FOOTER_LEN), "a data file holds at least a footer");
		Ok(DataFileInfo {
			size,
			data_len,
			content_hash,
		})
	}

	/// Encodes `build.index`. `entries` must be sorted by path.
	///
	/// # Errors
	/// Fails if the entry set is invalid (unsorted, duplicates, no single root
	/// `*.zenithproject`, a file named like the index) or too large.
	pub fn encode_index(&self, entries: &[FolderIndexEntry]) -> ZResult<Vec<u8>> {
		if entries.len() > MAX_ENTRIES {
			return Err(limit("too many entries"));
		}
		let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
		validate_entry_set(&paths)?;
		if self.sealer.crypto.is_none() && paths.contains(&INDEX_FILE_NAME) {
			return Err(write_err(format!("{INDEX_FILE_NAME} is reserved for the build index")));
		}

		let mut body = Vec::new();
		body.extend_from_slice(
			&u32::try_from(entries.len())
				.map_err(|_| limit("too many entries"))?
				.to_le_bytes(),
		);
		for e in entries {
			let name = self.data_file_name(&e.path)?;
			for text in [e.path.as_str(), name.as_str()] {
				let len = u16::try_from(text.len()).map_err(|_| limit("path is too long"))?;
				body.extend_from_slice(&len.to_le_bytes());
				body.extend_from_slice(text.as_bytes());
			}
			body.extend_from_slice(&e.size.to_le_bytes());
			body.extend_from_slice(&e.data_len.to_le_bytes());
			let level_ok = if self.sealer.compressed() {
				(1..=22).contains(&e.compression_level)
			} else {
				e.compression_level == 0
			};
			if !level_ok || e.chunk_size_log2 != self.sealer.chunk_size_log2() {
				return Err(write_err(format!(
					"index entry {:?} has compression parameters the writer does not use",
					e.path
				)));
			}
			body.extend_from_slice(&e.content_hash);
			body.push(e.compression_level);
			body.push(e.chunk_size_log2);
		}

		let header = self.sealer.header(Kind::FolderIndex)?;
		let mut out = header.raw.to_vec();
		self.sealer
			.write_body_and_footer(&mut out, &header, Kind::FolderIndex, INDEX_IDENT, body, HEADER_LEN_U64)?;
		Ok(out)
	}
}
