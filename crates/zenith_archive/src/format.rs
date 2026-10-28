//! Fixed-size pieces shared by all containers: constants, header, footer and a
//! bounds-checked byte reader.

use zenith_error::{ZResult, ZenithError};

use crate::error::corrupt;

/// Format version written into every header. Readers reject anything else.
pub const FORMAT_VERSION: u16 = 1;

pub const HEADER_LEN: usize = 48;
pub const HEADER_LEN_U64: u64 = 48;
pub const FOOTER_LEN: usize = 32;
pub const FOOTER_LEN_U64: u64 = 32;
const HEADER_CRC_OFFSET: usize = 44;
const FOOTER_CRC_OFFSET: usize = 28;

pub const FLAG_ENCRYPTED: u16 = 1;
pub const AEAD_NONE: u8 = 0;
pub const AEAD_AES_256_GCM_SIV: u8 = 1;
/// Chunks are stored as they are.
pub const COMPRESSION_NONE: u8 = 0;
pub const COMPRESSION_ZSTD: u8 = 1;
pub const TAG_LEN: usize = 16;

/// Smallest and largest allowed `chunk_size_log2` (4 KiB .. 16 MiB).
pub const MIN_CHUNK_SIZE_LOG2: u8 = 12;
pub const MAX_CHUNK_SIZE_LOG2: u8 = 24;
/// 256 KiB: below 64 KiB zstd loses noticeable ratio, far above it random
/// access and `Seek` get expensive and wasm memory pressure grows.
pub const DEFAULT_CHUNK_SIZE_LOG2: u8 = 18;

/// Longest allowed entry path in bytes.
pub const MAX_PATH_LEN: usize = 1024;
/// Longest allowed single path segment in bytes (common filesystem limit).
pub const MAX_SEGMENT_LEN: usize = 255;
/// Most entries one container may hold.
pub const MAX_ENTRIES: usize = 1 << 22;
/// Largest index/table body that is accepted, checked before allocating.
pub const MAX_BODY_LEN: u64 = 256 * 1024 * 1024;
/// Largest uncompressed size of a single entry.
pub const MAX_ENTRY_SIZE: u64 = 1 << 48;

/// Name of the index file inside a folder build.
pub const INDEX_FILE_NAME: &str = "build.index";
/// A build holds exactly one root file with this extension.
pub const MANIFEST_EXTENSION: &str = "zenithproject";

/// Nonce/AAD identifiers of non-chunk bodies. They start with 0xFF, which can
/// never begin a valid UTF-8 path, so they cannot collide with an entry path.
pub const INDEX_IDENT: &[u8] = b"\xffindex";
pub const TABLE_IDENT_PREFIX: &[u8] = b"\xfftable";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	Archive,
	FolderIndex,
	FolderData,
}

impl Kind {
	pub const fn magic(self) -> [u8; 8] {
		match self {
			Self::Archive => *b"\x89ZARC\r\n\x1a",
			Self::FolderIndex => *b"\x89ZFIX\r\n\x1a",
			Self::FolderData => *b"\x89ZDAT\r\n\x1a",
		}
	}

	const fn end_magic(self) -> [u8; 8] {
		match self {
			Self::Archive => *b"\x89ZARCEND",
			Self::FolderIndex => *b"\x89ZFIXEND",
			Self::FolderData => *b"\x89ZDATEND",
		}
	}
}

/// Bounds-checked little-endian reader over untrusted bytes.
pub struct Rd<'a> {
	buf: &'a [u8],
	pos: usize,
}

impl<'a> Rd<'a> {
	pub const fn new(buf: &'a [u8]) -> Self {
		Self { buf, pos: 0 }
	}

	pub fn take(&mut self, n: usize) -> ZResult<&'a [u8]> {
		let end = self.pos.checked_add(n).ok_or_else(|| corrupt("length overflow"))?;
		let slice = self
			.buf
			.get(self.pos..end)
			.ok_or_else(|| corrupt("unexpected end of data"))?;
		self.pos = end;
		Ok(slice)
	}

	pub fn array<const N: usize>(&mut self) -> ZResult<[u8; N]> {
		self.take(N)?.try_into().map_err(|_| corrupt("unexpected end of data"))
	}

	pub fn u8(&mut self) -> ZResult<u8> {
		Ok(u8::from_le_bytes(self.array()?))
	}

	pub fn u16(&mut self) -> ZResult<u16> {
		Ok(u16::from_le_bytes(self.array()?))
	}

	pub fn u32(&mut self) -> ZResult<u32> {
		Ok(u32::from_le_bytes(self.array()?))
	}

	pub fn u64(&mut self) -> ZResult<u64> {
		Ok(u64::from_le_bytes(self.array()?))
	}

	pub const fn remaining(&self) -> usize {
		self.buf.len().saturating_sub(self.pos)
	}

	pub const fn is_empty(&self) -> bool {
		self.remaining() == 0
	}
}

/// Parsed and verified container header. `raw` is kept because the exact
/// header bytes are part of the AAD of every encrypted chunk.
#[derive(Clone, Debug)]
pub struct Header {
	pub raw: [u8; HEADER_LEN],
	pub encrypted: bool,
	/// Chunks are zstd frames; otherwise they hold the bytes of the file.
	pub compressed: bool,
	pub chunk_size_log2: u8,
	pub chunk_size: usize,
	pub key_check: [u8; 16],
}

impl Header {
	pub fn new(kind: Kind, chunk_size_log2: u8, key_check: Option<[u8; 16]>, compressed: bool) -> ZResult<Self> {
		let encrypted = key_check.is_some();
		let mut v = Vec::with_capacity(HEADER_LEN);
		v.extend_from_slice(&kind.magic());
		v.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
		v.extend_from_slice(&(if encrypted { FLAG_ENCRYPTED } else { 0 }).to_le_bytes());
		v.push(if encrypted { AEAD_AES_256_GCM_SIV } else { AEAD_NONE });
		v.push(if compressed { COMPRESSION_ZSTD } else { COMPRESSION_NONE });
		v.push(chunk_size_log2);
		v.push(0);
		v.extend_from_slice(&key_check.unwrap_or([0; 16]));
		v.resize(HEADER_CRC_OFFSET, 0);
		let crc = crc32fast::hash(&v);
		v.extend_from_slice(&crc.to_le_bytes());
		let raw: [u8; HEADER_LEN] = v.try_into().map_err(|_| corrupt("header size"))?;
		Self::parse(raw, kind)
	}

	pub fn parse(raw: [u8; HEADER_LEN], kind: Kind) -> ZResult<Self> {
		let mut r = Rd::new(&raw);
		if r.take(8)? != kind.magic() {
			return Err(corrupt("bad header magic"));
		}
		let version = r.u16()?;
		if version != FORMAT_VERSION {
			return Err(ZenithError::UnsupportedArchiveVersion(version));
		}
		let flags = r.u16()?;
		let aead = r.u8()?;
		let compression = r.u8()?;
		let chunk_size_log2 = r.u8()?;
		let reserved = r.u8()?;
		let key_check: [u8; 16] = r.array()?;
		let padding = r.take(12)?;
		let crc = r.u32()?;

		let covered = raw.get(..HEADER_CRC_OFFSET).ok_or_else(|| corrupt("header size"))?;
		if crc32fast::hash(covered) != crc {
			return Err(corrupt("header checksum mismatch"));
		}
		if flags & !FLAG_ENCRYPTED != 0 {
			return Err(corrupt("unknown header flags"));
		}
		let encrypted = flags & FLAG_ENCRYPTED != 0;
		if aead != if encrypted { AEAD_AES_256_GCM_SIV } else { AEAD_NONE } {
			return Err(corrupt("AEAD id does not match the encryption flag"));
		}
		if compression != COMPRESSION_ZSTD && compression != COMPRESSION_NONE {
			return Err(corrupt("unknown compression id"));
		}
		if !(MIN_CHUNK_SIZE_LOG2..=MAX_CHUNK_SIZE_LOG2).contains(&chunk_size_log2) {
			return Err(corrupt("chunk size out of range"));
		}
		if reserved != 0 || padding.iter().any(|b| *b != 0) {
			return Err(corrupt("reserved header bytes are not zero"));
		}
		if !encrypted && key_check != [0; 16] {
			return Err(corrupt("key check set on an unencrypted container"));
		}
		let chunk_size = 1usize
			.checked_shl(u32::from(chunk_size_log2))
			.ok_or_else(|| corrupt("chunk size out of range"))?;
		Ok(Self {
			raw,
			encrypted,
			compressed: compression == COMPRESSION_ZSTD,
			chunk_size_log2,
			chunk_size,
			key_check,
		})
	}
}

/// Parsed footer: location of the trailing body (index or chunk table).
#[derive(Clone, Copy, Debug)]
pub struct Footer {
	pub offset: u64,
	pub len: u64,
	pub crc: u32,
}

impl Footer {
	#[cfg(feature = "write")]
	pub fn encode(self, kind: Kind) -> [u8; FOOTER_LEN] {
		let mut v = Vec::with_capacity(FOOTER_LEN);
		v.extend_from_slice(&kind.end_magic());
		v.extend_from_slice(&self.offset.to_le_bytes());
		v.extend_from_slice(&self.len.to_le_bytes());
		v.extend_from_slice(&self.crc.to_le_bytes());
		let crc = crc32fast::hash(&v);
		v.extend_from_slice(&crc.to_le_bytes());
		let mut out = [0u8; FOOTER_LEN];
		out.copy_from_slice(&v);
		out
	}

	pub fn parse(raw: &[u8; FOOTER_LEN], kind: Kind) -> ZResult<Self> {
		let mut r = Rd::new(raw);
		if r.take(8)? != kind.end_magic() {
			return Err(corrupt("bad footer magic (truncated or not a Zenith container)"));
		}
		let body_offset = r.u64()?;
		let body_len = r.u64()?;
		let body_crc = r.u32()?;
		let crc = r.u32()?;
		let covered = raw.get(..FOOTER_CRC_OFFSET).ok_or_else(|| corrupt("footer size"))?;
		if crc32fast::hash(covered) != crc {
			return Err(corrupt("footer checksum mismatch"));
		}
		Ok(Self {
			offset: body_offset,
			len: body_len,
			crc: body_crc,
		})
	}
}
