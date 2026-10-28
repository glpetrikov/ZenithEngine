//! Chunk encoder shared by both writers (feature `write`).

use std::{
	io::{Read, Write},
	sync::Arc,
};

use zenith_error::{ZResult, ZenithError};
use zstd::bulk::Compressor;

use crate::{
	crypto::{Crypto, EncryptionKey},
	error::{limit, to_u64},
	format::{DEFAULT_CHUNK_SIZE_LOG2, Footer, Header, Kind, MAX_CHUNK_SIZE_LOG2, MAX_ENTRY_SIZE, MIN_CHUNK_SIZE_LOG2},
	layout::{ChunkMeta, tag_prefix},
};

/// Most chunks one writer compresses at the same time.
const MAX_THREADS: usize = 256;

/// Parameters of a build. The defaults are a 256 KiB chunk, zstd level 9, no
/// encryption and one thread.
#[derive(Clone, Debug)]
pub struct WriterOptions {
	/// Whether chunks are compressed (default). Without it they hold the bytes
	/// of the file as they are, which makes a build bigger and its files
	/// readable in a hex dump or a text editor (unless encrypted).
	pub compress: bool,
	/// zstd level, `1..=22`. Ignored without `compress`.
	pub compression_level: i32,
	/// Chunk size as a power of two, `12..=24`.
	pub chunk_size_log2: u8,
	/// Encrypts the build when set.
	pub key: Option<EncryptionKey>,
	/// Chunks of one file that are compressed at the same time (`1` compresses
	/// in the calling thread, `0` counts as `1`). The output is the same for
	/// every value: a chunk depends only on its own bytes. More threads make a
	/// big build faster and need [`Self::thread_memory`] more memory each.
	pub threads: usize,
}

impl Default for WriterOptions {
	fn default() -> Self {
		Self {
			compress: true,
			compression_level: 9,
			chunk_size_log2: DEFAULT_CHUNK_SIZE_LOG2,
			key: None,
			threads: 1,
		}
	}
}

impl WriterOptions {
	/// A deliberately generous estimate of the memory one compression thread
	/// needs, in bytes: the zstd state (which grows with the level and with the
	/// chunk size, since zstd sizes its tables to the input), the input chunk
	/// and the compressed and encrypted copies. Meant for choosing how many
	/// threads to run, not for accounting.
	#[must_use]
	pub fn thread_memory(&self) -> u64 {
		let chunk = 1u64 << self.chunk_size_log2.min(MAX_CHUNK_SIZE_LOG2);
		let zstd: u64 = match self.compression_level {
			_ if !self.compress => 0,
			..=3 => 4,
			4..=9 => 6,
			10..=15 => 10,
			16..=19 => 14,
			_ => 20,
		};
		chunk.saturating_mul(zstd.saturating_add(4))
	}
}

pub fn write_err(msg: impl Into<String>) -> ZenithError {
	ZenithError::ArchiveWrite(msg.into())
}

/// One compression thread's state: its zstd context and the chunk it works on.
struct Worker {
	compressor: Option<Compressor<'static>>,
	buf: Vec<u8>,
	len: usize,
}

/// Compresses (and encrypts) chunks. One instance is reused for a whole build
/// so the zstd contexts and the derived keys are set up once.
pub struct Sealer {
	workers: Vec<Worker>,
	pub crypto: Option<Arc<Crypto>>,
	chunk_size_log2: u8,
	compressed: bool,
}

impl Sealer {
	pub fn new(opts: &WriterOptions) -> ZResult<Self> {
		if opts.compress && !(1..=22).contains(&opts.compression_level) {
			return Err(write_err("zstd compression level must be in 1..=22"));
		}
		if !(MIN_CHUNK_SIZE_LOG2..=MAX_CHUNK_SIZE_LOG2).contains(&opts.chunk_size_log2) {
			return Err(write_err("chunk size must be between 4 KiB and 16 MiB"));
		}
		let crypto = opts.key.as_ref().map(Crypto::new).transpose()?.map(Arc::new);
		let workers = (0..opts.threads.clamp(1, MAX_THREADS))
			.map(|_| {
				Ok(Worker {
					compressor: opts
						.compress
						.then(|| Compressor::new(opts.compression_level))
						.transpose()?,
					buf: Vec::new(),
					len: 0,
				})
			})
			.collect::<ZResult<Vec<_>>>()?;
		Ok(Self {
			workers,
			crypto,
			chunk_size_log2: opts.chunk_size_log2,
			compressed: opts.compress,
		})
	}

	pub const fn chunk_size_log2(&self) -> u8 {
		self.chunk_size_log2
	}

	/// Whether chunks are compressed.
	pub const fn compressed(&self) -> bool {
		self.compressed
	}

	/// Header for one container file. It carries nothing that changes between
	/// builds, which keeps the output (and the AAD) reproducible.
	pub fn header(&self, kind: Kind) -> ZResult<Header> {
		Header::new(
			kind,
			self.chunk_size_log2,
			self.crypto.as_ref().map(|c| c.key_check()),
			self.compressed,
		)
	}

	/// Reads `src` to the end, writing it as chunks. Returns the uncompressed
	/// size and the chunk table; `pos` is advanced by the bytes written.
	///
	/// Up to one chunk per thread is read, sealed at the same time and written
	/// in order, so the bytes written do not depend on the thread count.
	pub fn write_chunks<W: Write, R: Read>(
		&mut self,
		out: &mut W,
		header: &Header,
		path: &str,
		src: &mut R,
		pos: &mut u64,
	) -> ZResult<(u64, Vec<ChunkMeta>)> {
		let mut chunks = Vec::new();
		let mut total = 0u64;
		let mut idx = 0u64;
		let mut ended = false;
		while !ended {
			let mut filled = 0usize;
			for worker in &mut self.workers {
				worker.buf.resize(header.chunk_size, 0);
				worker.len = read_full(src, &mut worker.buf)?;
				if worker.len == 0 {
					ended = true;
					break;
				}
				filled = filled.saturating_add(1);
				if worker.len < header.chunk_size {
					ended = true;
					break;
				}
			}
			let batch = self.workers.get_mut(..filled).unwrap_or_default();
			let sealed = seal_batch(batch, self.crypto.as_deref(), header, path, idx);
			for (worker, result) in batch.iter().zip(sealed) {
				let (stored, crc) = result?;
				out.write_all(&stored)?;
				let stored_len = u32::try_from(stored.len()).map_err(|_| limit("stored chunk is too large"))?;
				*pos = pos
					.checked_add(to_u64(stored.len()))
					.ok_or_else(|| limit("archive is too large"))?;
				total = total
					.checked_add(to_u64(worker.len))
					.ok_or_else(|| limit("entry is too large"))?;
				if total > MAX_ENTRY_SIZE {
					return Err(limit("entry is larger than the format limit"));
				}
				chunks.push(ChunkMeta { stored_len, crc });
				idx = idx.checked_add(1).ok_or_else(|| limit("too many chunks"))?;
			}
		}
		Ok((total, chunks))
	}

	/// Encrypts a trailing body (or computes its CRC) and writes it followed by
	/// the footer. `pos` is the offset where the body starts.
	pub fn write_body_and_footer<W: Write>(
		&self,
		out: &mut W,
		header: &Header,
		kind: Kind,
		ident: &[u8],
		body: Vec<u8>,
		pos: u64,
	) -> ZResult<u64> {
		let (stored, body_crc) = protect(body, self.crypto.as_deref(), header, ident, 0, false)?;
		let body_len = to_u64(stored.len());
		out.write_all(&stored)?;
		let footer = Footer {
			offset: pos,
			len: body_len,
			crc: body_crc,
		};
		out.write_all(&footer.encode(kind))?;
		pos.checked_add(body_len)
			.and_then(|p| p.checked_add(to_u64(crate::format::FOOTER_LEN)))
			.ok_or_else(|| limit("archive is too large"))
	}
}

/// Encrypts a compressed frame or body, or checksums it when there is no key.
/// Returns the stored bytes and the checksum field: the CRC32 without a key;
/// with one, the tag prefix that ties a chunk to its build (`chunk`) or 0 for a
/// body, which is tied to its container by the footer.
fn protect(
	bytes: Vec<u8>,
	crypto: Option<&Crypto>,
	header: &Header,
	ident: &[u8],
	idx: u64,
	chunk: bool,
) -> ZResult<(Vec<u8>, u32)> {
	if let Some(crypto) = crypto {
		let stored = crypto.seal(&bytes, &header.raw, ident, idx)?;
		let field = if chunk {
			tag_prefix(&stored).ok_or_else(|| write_err("sealed chunk without a tag"))?
		} else {
			0
		};
		Ok((stored, field))
	} else {
		let crc = crc32fast::hash(&bytes);
		Ok((bytes, crc))
	}
}

/// Compresses and protects the chunk a worker holds.
fn seal_one(
	worker: &mut Worker,
	crypto: Option<&Crypto>,
	header: &Header,
	path: &str,
	idx: u64,
) -> ZResult<(Vec<u8>, u32)> {
	let raw = worker.buf.get(..worker.len).ok_or_else(|| write_err("chunk buffer"))?;
	let frame = match &mut worker.compressor {
		Some(compressor) => compressor.compress(raw)?,
		None => raw.to_vec(),
	};
	protect(frame, crypto, header, path.as_bytes(), idx, true)
}

/// Seals the chunks the workers hold, chunk `i` being number `first_idx + i` of
/// its file. One chunk is sealed in place; several on one thread each. The
/// results come back in chunk order.
fn seal_batch(
	workers: &mut [Worker],
	crypto: Option<&Crypto>,
	header: &Header,
	path: &str,
	first_idx: u64,
) -> Vec<ZResult<(Vec<u8>, u32)>> {
	if let [only] = workers {
		return vec![seal_one(only, crypto, header, path, first_idx)];
	}
	std::thread::scope(|scope| {
		let handles: Vec<_> = workers
			.iter_mut()
			.enumerate()
			.map(|(i, worker)| {
				let idx = first_idx.saturating_add(to_u64(i));
				scope.spawn(move || seal_one(worker, crypto, header, path, idx))
			})
			.collect();
		handles
			.into_iter()
			.map(|handle| {
				handle
					.join()
					.unwrap_or_else(|_| Err(write_err("a compression thread panicked")))
			})
			.collect()
	})
}

/// Fills `buf` unless the source ends first; returns the bytes read.
fn read_full<R: Read>(src: &mut R, buf: &mut [u8]) -> ZResult<usize> {
	let mut filled = 0;
	while filled < buf.len() {
		let rest = buf.get_mut(filled..).ok_or_else(|| write_err("read buffer"))?;
		match src.read(rest) {
			Ok(0) => break,
			Ok(n) => filled = filled.saturating_add(n),
			Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
			Err(e) => return Err(e.into()),
		}
	}
	Ok(filled)
}
