//! Reading side shared by every container: header/footer handling, body
//! loading and the chunk decoder.

use std::{io::Read, sync::Arc};

use ruzstd::decoding::StreamingDecoder;
use zenith_error::{ZResult, ZenithError};

use crate::{
	build_id::BuildId,
	crypto::{Crypto, EncryptionKey},
	error::{corrupt, limit, to_u64, to_usize},
	format::{FOOTER_LEN, FOOTER_LEN_U64, Footer, HEADER_LEN, HEADER_LEN_U64, Header, Kind, MAX_BODY_LEN, Rd},
	layout::{ChunkMeta, FileLayout, tag_prefix},
	read_at::{ReadAt, read_exact_at},
};

/// An opened container file: its source, verified header and key material.
pub struct FileCtx {
	pub src: Arc<dyn ReadAt>,
	pub header: Header,
	pub crypto: Option<Arc<Crypto>>,
}

pub fn read_header(src: &dyn ReadAt, kind: Kind) -> ZResult<Header> {
	if src.len() < to_u64(HEADER_LEN + FOOTER_LEN) {
		return Err(corrupt("file is too small to be a container"));
	}
	let mut raw = [0u8; HEADER_LEN];
	read_exact_at(src, 0, &mut raw)?;
	Header::parse(raw, kind)
}

impl FileCtx {
	/// Opens the root file of a container (the archive or `build.index`) and
	/// checks the key against the header's key check.
	pub fn open_root(src: Arc<dyn ReadAt>, kind: Kind, key: Option<&EncryptionKey>) -> ZResult<Self> {
		let header = read_header(&*src, kind)?;
		let crypto = match (header.encrypted, key) {
			(true, None) => return Err(ZenithError::ArchiveEncryptedNoKey),
			(false, Some(_)) => return Err(ZenithError::ArchiveNotEncrypted),
			(false, None) => None,
			(true, Some(key)) => {
				let crypto = Crypto::new(key)?;
				if crypto.key_check() != header.key_check {
					return Err(ZenithError::ArchiveWrongKey);
				}
				Some(Arc::new(crypto))
			}
		};
		Ok(Self { src, header, crypto })
	}

	/// Opens a data file that belongs to the same build as `self` (the index).
	/// Its header must agree with the index on everything but the magic, so a
	/// file from another build (other key or chunk size) is rejected early.
	pub fn open_sibling(&self, src: Arc<dyn ReadAt>, kind: Kind) -> ZResult<Self> {
		let header = read_header(&*src, kind)?;
		if header.encrypted != self.header.encrypted
			|| header.chunk_size_log2 != self.header.chunk_size_log2
			|| header.key_check != self.header.key_check
		{
			return Err(corrupt("data file does not belong to this build"));
		}
		Ok(Self {
			src,
			header,
			crypto: self.crypto.clone(),
		})
	}

	/// Loads and verifies the trailing body (archive index, folder index or
	/// chunk table): footer, bounds, then CRC32 or AEAD tag. Returns the plain
	/// body, the offset where it starts, and the [`BuildId`] of the container
	/// (made from its header and the body as stored).
	pub fn load_body(&self, kind: Kind, ident: &[u8]) -> ZResult<(Vec<u8>, u64, BuildId)> {
		let footer_offset = self
			.src
			.len()
			.checked_sub(FOOTER_LEN_U64)
			.ok_or_else(|| corrupt("file is too small to be a container"))?;
		let mut raw = [0u8; FOOTER_LEN];
		read_exact_at(&*self.src, footer_offset, &mut raw)?;
		let footer = Footer::parse(&raw, kind)?;

		if footer.len > MAX_BODY_LEN {
			return Err(limit("index or table is larger than the format limit"));
		}
		if footer.offset < HEADER_LEN_U64 || (kind == Kind::FolderIndex && footer.offset != HEADER_LEN_U64) {
			return Err(corrupt("body offset out of range"));
		}
		let end = footer
			.offset
			.checked_add(footer.len)
			.ok_or_else(|| corrupt("body offset overflow"))?;
		if end != footer_offset {
			return Err(corrupt("body does not end at the footer"));
		}

		let mut stored = vec![0u8; to_usize(footer.len)?];
		read_exact_at(&*self.src, footer.offset, &mut stored)?;
		let id = BuildId::of(&self.header.raw, &stored);
		let plain = if let Some(crypto) = &self.crypto {
			if footer.crc != 0 {
				return Err(corrupt("body checksum must be zero in an encrypted container"));
			}
			crypto.open(&stored, &self.header.raw, ident, 0)?
		} else {
			if crc32fast::hash(&stored) != footer.crc {
				return Err(corrupt("body checksum mismatch"));
			}
			stored
		};
		Ok((plain, footer.offset, id))
	}

	/// Reads, verifies and decompresses chunk `idx` of a file. Every chunk is
	/// independent, so neighbours are never touched.
	pub fn read_chunk(&self, layout: &FileLayout, path: &str, idx: usize) -> ZResult<Vec<u8>> {
		let meta = layout
			.chunks
			.get(idx)
			.ok_or_else(|| corrupt("chunk index out of range"))?;
		let offset = *layout
			.offsets
			.get(idx)
			.ok_or_else(|| corrupt("chunk index out of range"))?;

		let chunk_size = to_u64(self.header.chunk_size);
		let before = to_u64(idx)
			.checked_mul(chunk_size)
			.ok_or_else(|| corrupt("chunk position overflow"))?;
		let expected = to_usize(layout.size.saturating_sub(before).min(chunk_size))?;

		let mut stored = vec![0u8; to_usize(u64::from(meta.stored_len))?];
		read_exact_at(&*self.src, offset, &mut stored)?;
		self.decode_chunk(path, to_u64(idx), *meta, &stored, expected)
	}

	pub fn decode_chunk(
		&self,
		path: &str,
		idx: u64,
		meta: ChunkMeta,
		stored: &[u8],
		expected: usize,
	) -> ZResult<Vec<u8>> {
		let decrypted;
		let frame: &[u8] = if let Some(crypto) = &self.crypto {
			// The index lists the tag of the chunk it was built with; a chunk
			// of another build has a valid tag of its own and would pass the
			// authentication below.
			if tag_prefix(stored) != Some(meta.crc) {
				return Err(corrupt(
					"chunk is not the one the index lists (a chunk of another version of the file, or damaged)",
				));
			}
			decrypted = crypto.open(stored, &self.header.raw, path.as_bytes(), idx)?;
			&decrypted
		} else {
			if crc32fast::hash(stored) != meta.crc {
				return Err(corrupt("chunk checksum mismatch"));
			}
			stored
		};
		if !self.header.compressed {
			if frame.len() != expected {
				return Err(corrupt("stored chunk does not have the size the index lists"));
			}
			return Ok(frame.to_vec());
		}
		decompress_frame(frame, self.header.chunk_size, expected)
	}
}

/// Checks the zstd frame header before the decoder sees it: the decoder
/// allocates its window from this header, so a hostile window size must be
/// rejected here. The frame must declare its content size and it must be
/// exactly the size the chunk table implies.
fn validate_frame_header(frame: &[u8], chunk_size: usize, expected: usize) -> ZResult<()> {
	let mut r = Rd::new(frame);
	if r.u32()? != 0xFD2F_B528 {
		return Err(corrupt("chunk is not a zstd frame"));
	}
	let descriptor = r.u8()?;
	let dictionary_flag = descriptor & 0b11;
	// Bit 2 is the content checksum (never written: the chunk table's CRC or
	// the AEAD tag covers the chunk) and bit 3 is reserved.
	let checksum_or_reserved = descriptor & 0b1100;
	let single_segment = descriptor & 0b10_0000 != 0;
	let content_size_flag = descriptor >> 6;
	if dictionary_flag != 0 || checksum_or_reserved != 0 {
		return Err(corrupt("unsupported zstd frame features"));
	}
	if !single_segment {
		let window_descriptor = r.u8()?;
		let exponent = u32::from(window_descriptor >> 3).checked_add(10);
		let mantissa = u64::from(window_descriptor & 7);
		let base = exponent.and_then(|e| 1u64.checked_shl(e));
		let window = base
			.and_then(|b| {
				b.div_euclid(8)
					.checked_mul(mantissa)
					.and_then(|extra| b.checked_add(extra))
			})
			.ok_or_else(|| corrupt("zstd window size overflow"))?;
		if window > to_u64(chunk_size) {
			return Err(corrupt("zstd window is larger than a chunk"));
		}
	}
	let declared = match (content_size_flag, single_segment) {
		(0, true) => u64::from(r.u8()?),
		(0, false) => return Err(corrupt("zstd frame does not declare its content size")),
		(1, _) => u64::from(r.u16()?)
			.checked_add(256)
			.ok_or_else(|| corrupt("content size overflow"))?,
		(2, _) => u64::from(r.u32()?),
		_ => r.u64()?,
	};
	if declared != to_u64(expected) {
		return Err(corrupt("zstd content size does not match the chunk size"));
	}
	Ok(())
}

/// Decompresses one chunk. Output is capped at `chunk_size + 1` bytes, so a
/// frame that lies about its size can never expand beyond one chunk; anything
/// that is not exactly `expected` bytes is rejected.
fn decompress_frame(frame: &[u8], chunk_size: usize, expected: usize) -> ZResult<Vec<u8>> {
	validate_frame_header(frame, chunk_size, expected)?;
	let mut source: &[u8] = frame;
	let decoder = StreamingDecoder::new(&mut source).map_err(|e| corrupt(format!("zstd frame header: {e}")))?;
	let cap = to_u64(chunk_size).saturating_add(1);
	let mut out = Vec::with_capacity(expected.saturating_add(1));
	decoder
		.take(cap)
		.read_to_end(&mut out)
		.map_err(|e| corrupt(format!("zstd decode: {e}")))?;
	if out.len() > chunk_size {
		return Err(corrupt("chunk decompressed to more than the chunk size"));
	}
	if out.len() != expected {
		return Err(corrupt("chunk decompressed to an unexpected size"));
	}
	if !source.is_empty() {
		return Err(corrupt("trailing bytes after the zstd frame"));
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	const CS: usize = 4096;

	#[test]
	fn frame_header_rules() {
		// magic + descriptor (single segment, 2-byte content size flag=1) +
		// size-256
		let mut ok = vec![0x28, 0xB5, 0x2F, 0xFD, 0b0110_0000];
		ok.extend_from_slice(&(3840u16).to_le_bytes());
		assert!(
			validate_frame_header(&ok, CS, 4096).is_ok(),
			"matching content size passes"
		);
		assert!(
			validate_frame_header(&ok, CS, 4095).is_err(),
			"size mismatch is rejected"
		);

		let no_size = [0x28, 0xB5, 0x2F, 0xFD, 0b0000_0000, 0x00];
		assert!(
			validate_frame_header(&no_size, CS, 1).is_err(),
			"no content size is rejected"
		);

		// non single-segment frame whose window descriptor asks for a 1 GiB
		// window
		let huge_window = [0x28, 0xB5, 0x2F, 0xFD, 0b0100_0000, 20 << 3, 0, 0];
		assert!(
			validate_frame_header(&huge_window, CS, 4096).is_err(),
			"huge window is rejected"
		);

		let dict = [0x28, 0xB5, 0x2F, 0xFD, 0b0110_0001, 1, 0, 0];
		assert!(
			validate_frame_header(&dict, CS, 4096).is_err(),
			"dictionary frames are rejected"
		);
		for descriptor in [0b0110_0100u8, 0b0110_1000] {
			let mut frame = vec![0x28, 0xB5, 0x2F, 0xFD, descriptor];
			frame.extend_from_slice(&(3840u16).to_le_bytes());
			assert!(
				validate_frame_header(&frame, CS, 4096).is_err(),
				"checksum and reserved bits are rejected: {descriptor:#010b}"
			);
		}
		assert!(validate_frame_header(&[1, 2, 3], CS, 1).is_err(), "garbage is rejected");
	}

	#[cfg(feature = "write")]
	#[test]
	fn oversized_output_is_rejected() -> ZResult<()> {
		// Frame holds 8 KiB; patch its declared size down to one chunk so the
		// header check passes and only the decoder-side limit can catch it.
		let data = vec![7u8; CS * 2];
		let mut frame = zstd::bulk::compress(&data, 3)?;
		assert_eq!(
			frame.get(4),
			Some(&0b0110_0000),
			"test assumes a single-segment, 2-byte-size frame"
		);
		let patched = u16::try_from(CS - 256).map_err(|_| corrupt("test"))?.to_le_bytes();
		frame
			.get_mut(5..7)
			.ok_or_else(|| corrupt("test"))?
			.copy_from_slice(&patched);
		assert!(
			decompress_frame(&frame, CS, CS).is_err(),
			"a frame that lies about its size is rejected"
		);
		Ok(())
	}

	#[cfg(feature = "write")]
	#[test]
	fn exact_frame_roundtrips() -> ZResult<()> {
		let data: Vec<u8> = (0..CS).map(|i| u8::try_from(i % 251).unwrap_or(0)).collect();
		let frame = zstd::bulk::compress(&data, 3)?;
		assert_eq!(
			decompress_frame(&frame, CS, CS)?,
			data,
			"C zstd output decodes with ruzstd"
		);
		Ok(())
	}
}
