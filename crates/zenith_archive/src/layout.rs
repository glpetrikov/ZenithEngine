//! Per-file chunk tables and the checks that tie them to the container size.

use zenith_error::ZResult;

use crate::{
	error::{corrupt, to_u64},
	format::{Header, MAX_ENTRY_SIZE, Rd, TAG_LEN},
};

/// Bytes of one chunk-table record: `stored_len u32` + `crc32 u32`.
pub const CHUNK_RECORD_LEN: usize = 8;

#[derive(Clone, Copy, Debug)]
pub struct ChunkMeta {
	pub stored_len: u32,
	/// CRC32 of the stored bytes. When encrypted the AEAD tag covers the chunk
	/// and this is [`tag_prefix`] of it instead, which ties the chunk to the
	/// build that lists it.
	pub crc: u32,
}

/// First four bytes of the AEAD tag at the end of an encrypted chunk.
///
/// A chunk is authenticated by its tag, but nothing in the chunk says which
/// build it belongs to, and the nonces are deterministic on purpose: a chunk of
/// the same file and position from an older build made with the same key would
/// authenticate. The index and the chunk tables are authenticated too and list
/// this value for every chunk, so only the chunk the build was made with fits.
pub fn tag_prefix(stored: &[u8]) -> Option<u32> {
	let tag = stored.get(stored.len().checked_sub(TAG_LEN)?..)?;
	Some(u32::from_le_bytes(tag.get(..4)?.try_into().ok()?))
}

/// Where the chunks of one file live and how big the file is.
#[derive(Debug)]
pub struct FileLayout {
	pub size: u64,
	pub chunks: Vec<ChunkMeta>,
	/// Absolute offset of each chunk in the container file.
	pub offsets: Vec<u64>,
}

/// Upper bound for one stored chunk: zstd worst case plus the AEAD tag.
pub const fn max_stored_len(chunk_size: usize) -> usize {
	chunk_size
		.saturating_add(chunk_size >> 7)
		.saturating_add(512)
		.saturating_add(TAG_LEN)
}

pub fn expected_chunk_count(size: u64, chunk_size: usize) -> u64 {
	size.div_ceil(to_u64(chunk_size))
}

/// Reads `count` chunk records. The count is checked against the bytes that
/// are actually left before anything is allocated.
pub fn read_chunk_table(r: &mut Rd<'_>, count: u64) -> ZResult<Vec<ChunkMeta>> {
	let needed = count
		.checked_mul(to_u64(CHUNK_RECORD_LEN))
		.ok_or_else(|| corrupt("chunk table size overflow"))?;
	if needed > to_u64(r.remaining()) {
		return Err(corrupt("chunk table is larger than the data that is left"));
	}
	let count = usize::try_from(count).map_err(|_| corrupt("chunk count overflow"))?;
	let mut chunks = Vec::with_capacity(count);
	for _ in 0..count {
		let stored_len = r.u32()?;
		let crc = r.u32()?;
		chunks.push(ChunkMeta { stored_len, crc });
	}
	Ok(chunks)
}

/// Validates a chunk list against the file size and turns it into absolute
/// offsets. Returns the layout and the offset right after the last chunk.
pub fn make_layout(
	size: u64,
	data_offset: u64,
	chunks: Vec<ChunkMeta>,
	header: &Header,
	limit_end: u64,
) -> ZResult<(FileLayout, u64)> {
	if size > MAX_ENTRY_SIZE {
		return Err(corrupt("entry size exceeds the format limit"));
	}
	if expected_chunk_count(size, header.chunk_size) != to_u64(chunks.len()) {
		return Err(corrupt("chunk count does not match the entry size"));
	}
	let max = to_u64(max_stored_len(header.chunk_size));
	let min = if header.encrypted {
		to_u64(TAG_LEN).saturating_add(1)
	} else {
		1
	};
	let mut offsets = Vec::with_capacity(chunks.len());
	let mut next = data_offset;
	for chunk in &chunks {
		let len = u64::from(chunk.stored_len);
		if len < min || len > max {
			return Err(corrupt("stored chunk length out of range"));
		}
		offsets.push(next);
		next = next.checked_add(len).ok_or_else(|| corrupt("chunk offset overflow"))?;
	}
	if next > limit_end {
		return Err(corrupt("chunk data runs past the end of the data area"));
	}
	Ok((FileLayout { size, chunks, offsets }, next))
}

#[cfg(feature = "write")]
pub fn encode_chunk_table(chunks: &[ChunkMeta], out: &mut Vec<u8>) {
	for chunk in chunks {
		out.extend_from_slice(&chunk.stored_len.to_le_bytes());
		out.extend_from_slice(&chunk.crc.to_le_bytes());
	}
}
