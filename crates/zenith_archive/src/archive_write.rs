//! Single-pass archive writer (feature `write`).

use std::io::{Read, Write};

use zenith_error::ZResult;

use crate::{
	error::{limit, to_u64},
	format::{HEADER_LEN, HEADER_LEN_U64, Header, INDEX_IDENT, Kind, MAX_ENTRIES},
	layout::{ChunkMeta, encode_chunk_table},
	path::{validate_entry_path, validate_entry_set},
	write::{Sealer, WriterOptions, write_err},
};

struct Pending {
	path: String,
	size: u64,
	data_offset: u64,
	chunks: Vec<ChunkMeta>,
}

/// A file that has been compressed (and encrypted) but not written yet.
pub struct SealedEntry {
	size: u64,
	chunks: Vec<ChunkMeta>,
	data: Vec<u8>,
	/// Header of the writer that sealed it, so that an entry sealed with
	/// another key or chunk size cannot be mixed in.
	header: [u8; HEADER_LEN],
}

impl SealedEntry {
	/// Uncompressed size.
	#[must_use]
	pub const fn size(&self) -> u64 {
		self.size
	}

	/// Bytes the entry will take in the archive.
	#[must_use]
	pub const fn stored_len(&self) -> usize {
		self.data.len()
	}
}

/// Compresses files for an [`ArchiveWriter`] without writing them.
///
/// Several threads can work at the same time and the writer only appends the
/// results, in path order. Make one per thread with the writer's options; a
/// chunk depends only on its own bytes, so the archive is the same as when the
/// writer does all the work itself.
pub struct EntrySealer {
	sealer: Sealer,
	header: Header,
}

impl EntrySealer {
	/// # Errors
	/// Fails for invalid options.
	pub fn new(opts: &WriterOptions) -> ZResult<Self> {
		let sealer = Sealer::new(opts)?;
		let header = sealer.header(Kind::Archive)?;
		Ok(Self { sealer, header })
	}

	/// Compresses `src` to the end as the entry `path`. The result is held in
	/// memory: size big files by chunks with [`ArchiveWriter::add`] instead.
	///
	/// # Errors
	/// Fails for an invalid path or an unreadable source.
	pub fn seal(&mut self, path: &str, mut src: impl Read) -> ZResult<SealedEntry> {
		validate_entry_path(path)?;
		let mut data = Vec::new();
		let mut pos = 0u64;
		let (size, chunks) = self
			.sealer
			.write_chunks(&mut data, &self.header, path, &mut src, &mut pos)?;
		Ok(SealedEntry {
			size,
			chunks,
			data,
			header: self.header.raw,
		})
	}
}

/// Writes an archive in one pass without knowing sizes in advance and without
/// seeking, so it also works on pipes and sockets.
///
/// Entries must be added in strictly ascending path order (byte-wise); the
/// caller sorts. That keeps data and index in the same order and makes the
/// output reproducible.
///
/// An `add` that fails while reading or writing leaves part of the entry in the
/// output, and a writer that cannot seek cannot take it back: the writer then
/// refuses every further `add` and `finish`. Throw the output away.
pub struct ArchiveWriter<W: Write> {
	out: W,
	sealer: Sealer,
	header: Header,
	pos: u64,
	entries: Vec<Pending>,
	broken: bool,
}

impl<W: Write> ArchiveWriter<W> {
	/// Starts an archive by writing the header.
	///
	/// # Errors
	/// Fails for invalid options or if the header cannot be written.
	pub fn new(mut out: W, opts: &WriterOptions) -> ZResult<Self> {
		let sealer = Sealer::new(opts)?;
		let header = sealer.header(Kind::Archive)?;
		out.write_all(&header.raw)?;
		Ok(Self {
			out,
			sealer,
			header,
			pos: HEADER_LEN_U64,
			entries: Vec::new(),
			broken: false,
		})
	}

	fn check_usable(&self) -> ZResult<()> {
		if self.broken {
			return Err(write_err(
				"an earlier add failed half-way, so this archive cannot be finished",
			));
		}
		Ok(())
	}

	/// Whether an entry called `path` may be added now: a valid path that comes
	/// after the previous one, and room in the index.
	fn check_next(&self, path: &str) -> ZResult<()> {
		validate_entry_path(path)?;
		if self.entries.last().is_some_and(|last| last.path.as_str() >= path) {
			return Err(write_err(format!(
				"entry {path:?} is not after the previous one in path order"
			)));
		}
		if self.entries.len() >= MAX_ENTRIES {
			return Err(limit("too many entries"));
		}
		Ok(())
	}

	/// Adds one file, reading `src` to the end.
	///
	/// # Errors
	/// Fails for an invalid or out-of-order path, an unreadable source, or a
	/// write error. The first two leave the writer as it was; after a read or
	/// write error it is unusable (see the type's documentation).
	pub fn add(&mut self, path: &str, mut src: impl Read) -> ZResult<()> {
		self.check_usable()?;
		self.check_next(path)?;
		let data_offset = self.pos;
		let written = self
			.sealer
			.write_chunks(&mut self.out, &self.header, path, &mut src, &mut self.pos);
		let (size, chunks) = match written {
			Ok(done) => done,
			Err(e) => {
				self.broken = true;
				return Err(e);
			}
		};
		self.entries.push(Pending {
			path: path.to_string(),
			size,
			data_offset,
			chunks,
		});
		Ok(())
	}

	/// Adds a file from memory.
	///
	/// # Errors
	/// Same as [`Self::add`].
	pub fn add_bytes(&mut self, path: &str, bytes: &[u8]) -> ZResult<()> {
		self.add(path, bytes)
	}

	/// Adds a file that was compressed elsewhere (see [`EntrySealer`]), for
	/// example on another thread. The result is the same as with [`Self::add`].
	///
	/// # Errors
	/// Fails for an invalid or out-of-order path, for an entry sealed with
	/// other options than this writer's, and on a write error (after which the
	/// writer is unusable, as with `add`).
	pub fn add_sealed(&mut self, path: &str, entry: SealedEntry) -> ZResult<()> {
		self.check_usable()?;
		self.check_next(path)?;
		if entry.header != self.header.raw {
			return Err(write_err(
				"the entry was sealed with other options (key or chunk size) than this writer's",
			));
		}
		let end = self
			.pos
			.checked_add(to_u64(entry.data.len()))
			.ok_or_else(|| limit("archive is too large"))?;
		if let Err(e) = self.out.write_all(&entry.data) {
			self.broken = true;
			return Err(e.into());
		}
		self.entries.push(Pending {
			path: path.to_string(),
			size: entry.size,
			data_offset: self.pos,
			chunks: entry.chunks,
		});
		self.pos = end;
		Ok(())
	}

	/// Writes the index and footer and returns the underlying writer.
	///
	/// # Errors
	/// Fails if the entry set is invalid (for example no `*.zenithproject` in
	/// the root) or on a write error.
	pub fn finish(mut self) -> ZResult<W> {
		self.check_usable()?;
		let paths: Vec<&str> = self.entries.iter().map(|e| e.path.as_str()).collect();
		validate_entry_set(&paths)?;

		let mut body = Vec::new();
		body.extend_from_slice(
			&u32::try_from(self.entries.len())
				.map_err(|_| limit("too many entries"))?
				.to_le_bytes(),
		);
		for e in &self.entries {
			let path_len = u16::try_from(e.path.len()).map_err(|_| limit("path is too long"))?;
			body.extend_from_slice(&path_len.to_le_bytes());
			body.extend_from_slice(e.path.as_bytes());
			body.extend_from_slice(&e.size.to_le_bytes());
			body.extend_from_slice(&e.data_offset.to_le_bytes());
			body.extend_from_slice(
				&u32::try_from(e.chunks.len())
					.map_err(|_| limit("too many chunks"))?
					.to_le_bytes(),
			);
			encode_chunk_table(&e.chunks, &mut body);
		}
		self.sealer
			.write_body_and_footer(&mut self.out, &self.header, Kind::Archive, INDEX_IDENT, body, self.pos)?;
		self.out.flush()?;
		debug_assert!(
			to_u64(self.entries.len()) <= to_u64(MAX_ENTRIES),
			"entry limit enforced in add"
		);
		Ok(self.out)
	}
}
