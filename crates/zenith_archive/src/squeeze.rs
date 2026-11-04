//! Compressing one chunk into one zstd frame that the readers accept (feature
//! `write`).
//!
//! With the `zstd` feature the C library does it. Without it a pure Rust
//! encoder does, which is what the web build uses.

use zenith_error::ZResult;

use crate::write::write_err;

const MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// One compression context. Not shared between threads.
pub enum Squeezer {
	#[cfg(feature = "zstd")]
	Native(zstd::bulk::Compressor<'static>),
	#[cfg(not(feature = "zstd"))]
	Pure,
}

// The pure Rust encoder has no context to set up or to change: the same
// signatures as for the C library, so nothing else needs to know which it is.
#[cfg_attr(
	not(feature = "zstd"),
	allow( // because when we don't have zstd feature, we can to do this function const and etc like that, but if we do that, with zstd feature that will be broken
		clippy::unnecessary_wraps,
		clippy::missing_const_for_fn,
		clippy::needless_pass_by_ref_mut
	)
)]
impl Squeezer {
	/// A context for `level` (`1..=22`; the pure Rust encoder has one level and
	/// ignores it).
	pub fn new(level: i32) -> ZResult<Self> {
		#[cfg(feature = "zstd")]
		{
			Ok(Self::Native(zstd::bulk::Compressor::new(level)?))
		}
		#[cfg(not(feature = "zstd"))]
		{
			let _ = level;
			Ok(Self::Pure)
		}
	}

	/// The chunk as one zstd frame that declares its content size.
	pub fn compress(&mut self, raw: &[u8]) -> ZResult<Vec<u8>> {
		match self {
			#[cfg(feature = "zstd")]
			Self::Native(compressor) => Ok(compressor.compress(raw)?),
			#[cfg(not(feature = "zstd"))]
			Self::Pure => pure(raw),
		}
	}
}

/// The chunk compressed by `ruzstd`.
///
/// Its frames do not declare their content size and name a window of 128 KiB,
/// which the readers refuse (they want the size, and a window that fits the
/// chunk). The header is rewritten as a single-segment frame with the size
/// written in: the window is then the content itself, which is enough for
/// anything the blocks refer to, since they only look back inside the chunk.
#[cfg_attr(feature = "zstd", allow(dead_code))]
pub fn pure(raw: &[u8]) -> ZResult<Vec<u8>> {
	let frame = ruzstd::encoding::compress_to_vec(raw, ruzstd::encoding::CompressionLevel::Fastest);
	reframe(&frame, raw.len())
}

fn reframe(frame: &[u8], content_len: usize) -> ZResult<Vec<u8>> {
	let unexpected = || write_err("the pure Rust zstd encoder wrote a frame in an unexpected shape");
	let (magic, rest) = frame.split_at_checked(4).ok_or_else(unexpected)?;
	let (descriptor, rest) = rest.split_first().ok_or_else(unexpected)?;
	// Bit 2 is the content checksum, which `ruzstd` writes when its `hash`
	// feature is on (another crate of the build may turn it on). Nothing else
	// may be set: no dictionary, no single segment, no content size.
	let checksum = descriptor & 0b100 != 0;
	if magic != MAGIC || descriptor & !0b100 != 0 {
		return Err(unexpected());
	}
	// The window descriptor goes; the checksum, if any, goes from the end.
	let (_window, blocks) = rest.split_first().ok_or_else(unexpected)?;
	let blocks = if checksum {
		blocks
			.len()
			.checked_sub(4)
			.and_then(|end| blocks.get(..end))
			.ok_or_else(unexpected)?
	} else {
		blocks
	};

	let mut out = Vec::with_capacity(blocks.len().saturating_add(9));
	out.extend_from_slice(&MAGIC);
	// Single segment (bit 5) and the size in the smallest field that holds it.
	if let Ok(size) = u8::try_from(content_len) {
		out.push(0b0010_0000);
		out.push(size);
	} else if let Ok(size) = u16::try_from(content_len.saturating_sub(256)) {
		out.push(0b0110_0000);
		out.extend_from_slice(&size.to_le_bytes());
	} else if let Ok(size) = u32::try_from(content_len) {
		out.push(0b1010_0000);
		out.extend_from_slice(&size.to_le_bytes());
	} else {
		return Err(write_err("a chunk is too large for the pure Rust zstd encoder"));
	}
	out.extend_from_slice(blocks);
	Ok(out)
}

#[cfg(test)]
mod tests {
	use std::io::Read;

	use ruzstd::decoding::StreamingDecoder;

	use super::*;

	fn text(len: usize) -> Vec<u8> {
		b"The quick brown fox jumps over the lazy dog. "
			.iter()
			.cycle()
			.take(len)
			.copied()
			.collect()
	}

	fn noise(len: usize) -> Vec<u8> {
		let mut x = 0x9E37_79B9_7F4A_7C15_u64;
		(0..len)
			.map(|_| {
				x ^= x << 13;
				x ^= x >> 7;
				x ^= x << 17;
				u8::try_from(x & 0xff).unwrap_or(0)
			})
			.collect()
	}

	#[test]
	fn the_frames_are_valid_zstd_for_the_reference_decoder() -> ZResult<()> {
		for len in [1, 2, 255, 256, 257, 4096, 70_000, 65_791, 65_792, 131_072, 262_144] {
			for data in [text(len), noise(len)] {
				let frame = pure(&data)?;
				let back = zstd::bulk::decompress(&frame, len + 1)?;
				assert_eq!(back, data, "the C decoder reads {len} bytes back");
			}
		}
		Ok(())
	}

	#[test]
	fn the_frames_are_valid_for_our_decoder_and_its_frame_rules() -> ZResult<()> {
		for len in [1, 300, 4096, 70_000, 262_144] {
			let data = text(len);
			let frame = pure(&data)?;
			let mut back = Vec::new();
			let mut source: &[u8] = &frame;
			StreamingDecoder::new(&mut source)
				.map_err(|e| write_err(e.to_string()))?
				.read_to_end(&mut back)?;
			assert_eq!(back, data, "our decoder reads {len} bytes back");
			assert!(
				crate::ctx::validate_frame_header(&frame, len, len).is_ok(),
				"the frame passes the reader's checks even when the chunk is as small as the data ({len})"
			);
		}
		Ok(())
	}

	#[test]
	fn text_gets_smaller() -> ZResult<()> {
		let data = text(100_000);
		assert!(pure(&data)?.len() < data.len() / 4, "repeating text compresses well");
		Ok(())
	}

	#[test]
	fn a_frame_in_another_shape_is_refused_not_mangled() {
		assert!(reframe(&[], 1).is_err(), "empty");
		assert!(reframe(&[1, 2, 3, 4, 0, 0], 1).is_err(), "not a zstd frame");
		assert!(
			reframe(&[0x28, 0xB5, 0x2F, 0xFD, 0b0010_0000, 0], 1).is_err(),
			"a frame that is already single-segment"
		);
	}
}
