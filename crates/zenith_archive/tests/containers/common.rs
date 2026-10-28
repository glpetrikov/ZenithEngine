//! Shared helpers for the integration tests: sample data, builders, and byte
//! patching that follows the documented layout (not the crate's internals).

use std::{collections::BTreeMap, sync::Arc};

use zenith_archive::{ArchiveWriter, EncryptionKey, FolderIndexEntry, FolderWriter, MemDir, WriterOptions};

pub type TestResult = Result<(), Box<dyn std::error::Error>>;

/// 4 KiB chunks keep the fixtures small while still spanning many chunks.
pub const LOG2: u8 = 12;
pub const CHUNK: usize = 1 << LOG2;
pub const HEADER_LEN: usize = 48;
pub const FOOTER_LEN: usize = 32;

pub const fn key(seed: u8) -> EncryptionKey {
	EncryptionKey::new([seed; 32])
}

/// Deterministic pseudo-random bytes (xorshift), incompressible.
pub fn random_bytes(len: usize, seed: u64) -> Vec<u8> {
	let mut x = seed | 1;
	(0..len)
		.map(|_| {
			x ^= x << 13;
			x ^= x >> 7;
			x ^= x << 17;
			(x & 0xff) as u8
		})
		.collect()
}

/// Compressible text-like bytes.
pub fn text_bytes(len: usize) -> Vec<u8> {
	"Zenith engine asset line.\n".bytes().cycle().take(len).collect()
}

/// A small project: sorted by path, edge-case sizes around the chunk size.
pub fn sample_files() -> BTreeMap<String, Vec<u8>> {
	let mut files = BTreeMap::new();
	files.insert(
		"Test.zenithproject".to_string(),
		b"[project]\nname = \"Test\"\n".to_vec(),
	);
	files.insert("Assets/random.bin".to_string(), random_bytes(CHUNK * 3 + 17, 1));
	files.insert("Assets/empty".to_string(), Vec::new());
	files.insert("Assets/exact".to_string(), random_bytes(CHUNK, 2));
	files.insert("Assets/one_more".to_string(), random_bytes(CHUNK + 1, 3));
	files.insert("Assets/text.txt".to_string(), text_bytes(CHUNK * 5 + 100));
	files.insert("Settings/x.toml".to_string(), b"a = 1\n".to_vec());
	files
}

pub const fn opts(key: Option<EncryptionKey>) -> WriterOptions {
	WriterOptions {
		compress: true,
		compression_level: 3,
		chunk_size_log2: LOG2,
		key,
		threads: 1,
	}
}

pub fn build_archive(
	files: &BTreeMap<String, Vec<u8>>,
	key: Option<EncryptionKey>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
	build_archive_with(files, &opts(key))
}

pub fn build_archive_with(
	files: &BTreeMap<String, Vec<u8>>,
	options: &WriterOptions,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
	let mut writer = ArchiveWriter::new(Vec::new(), options)?;
	for (path, data) in files {
		writer.add_bytes(path, data)?;
	}
	Ok(writer.finish()?)
}

pub struct Folder {
	pub files: BTreeMap<String, Arc<[u8]>>,
}

impl Folder {
	pub fn mem_dir(&self) -> MemDir {
		let mut dir = MemDir::new();
		for (name, bytes) in &self.files {
			dir.insert(name.clone(), bytes.clone());
		}
		dir
	}
}

pub fn build_folder(
	files: &BTreeMap<String, Vec<u8>>,
	key: Option<EncryptionKey>,
) -> Result<Folder, Box<dyn std::error::Error>> {
	build_folder_with(files, &opts(key))
}

pub fn build_folder_with(
	files: &BTreeMap<String, Vec<u8>>,
	options: &WriterOptions,
) -> Result<Folder, Box<dyn std::error::Error>> {
	let mut writer = FolderWriter::new(options)?;
	let mut out = BTreeMap::new();
	let mut index = Vec::new();
	for (path, data) in files {
		let mut bytes = Vec::new();
		let hash = content_hash(data);
		let info = writer.write_data_file(&mut bytes, path, &mut data.as_slice(), |_| hash)?;
		out.insert(writer.data_file_name(path)?, Arc::from(bytes));
		index.push(FolderIndexEntry {
			path: path.clone(),
			size: info.size,
			data_len: info.data_len,
			content_hash: content_hash(data),
			compression_level: if options.compress { 3 } else { 0 },
			chunk_size_log2: LOG2,
		});
	}
	out.insert("build.index".to_string(), Arc::from(writer.encode_index(&index)?));
	Ok(Folder { files: out })
}

/// Any 32 bytes will do for the archive crate; SHA-256 keeps the tests simple.
pub fn content_hash(data: &[u8]) -> [u8; 32] {
	use sha2::{Digest, Sha256};
	Sha256::digest(data).into()
}

// --- byte patching ---------------------------------------------------------

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
	bytes
		.iter()
		.flat_map(|b| [b >> 4, b & 0x0f])
		.map(|n| char::from_digit(u32::from(n), 16).unwrap_or('0'))
		.collect()
}

/// Overwrites `bytes[at..]` with `value`.
pub fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
	bytes[at..at + value.len()].copy_from_slice(value);
}

pub fn u64_at(bytes: &[u8], at: usize) -> u64 {
	u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or([0; 8]))
}

pub fn fix_header_crc(bytes: &mut [u8]) {
	let crc = crc32fast::hash(&bytes[..44]);
	bytes[44..48].copy_from_slice(&crc.to_le_bytes());
}

/// Recomputes the footer CRC and, for unencrypted containers, the body CRC,
/// so a patched body is rejected by the structural checks and not by a CRC.
pub fn fix_footer(bytes: &mut [u8], encrypted: bool) {
	let footer = bytes.len() - FOOTER_LEN;
	if !encrypted {
		let off = u64_at(bytes, footer + 8);
		let len = u64_at(bytes, footer + 16);
		// A deliberately broken footer may point outside the file; then there
		// is no body CRC to fix.
		let range = off
			.checked_add(len)
			.filter(|end| *end <= footer as u64)
			.map(|end| off as usize..end as usize);
		if let Some(range) = range {
			let crc = crc32fast::hash(&bytes[range]);
			bytes[footer + 24..footer + 28].copy_from_slice(&crc.to_le_bytes());
		}
	}
	let crc = crc32fast::hash(&bytes[footer..footer + 28]);
	bytes[footer + 28..footer + 32].copy_from_slice(&crc.to_le_bytes());
}

/// Offset of the (plain) index body of an archive or folder index.
pub fn body_offset(bytes: &[u8]) -> usize {
	u64_at(bytes, bytes.len() - FOOTER_LEN + 8) as usize
}

/// Start offset of every index record of an unencrypted archive, found by
/// walking the documented record layout: path length, path, size, data offset,
/// chunk count, then eight bytes per chunk.
pub fn record_offsets(bytes: &[u8]) -> Vec<usize> {
	let body = body_offset(bytes);
	let count = u32::from_le_bytes(bytes[body..body + 4].try_into().unwrap_or([0; 4])) as usize;
	let mut at = body + 4;
	let mut out = Vec::new();
	for _ in 0..count {
		out.push(at);
		let path_len = u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize;
		let chunks_at = at + 2 + path_len + 16;
		let chunks = u32::from_le_bytes(bytes[chunks_at..chunks_at + 4].try_into().unwrap_or([0; 4])) as usize;
		at = chunks_at + 4 + chunks * 8;
	}
	out
}

/// Field offsets inside a record: path, size, data offset, chunk count and the
/// first chunk record.
pub const fn record_fields(bytes: &[u8], record: usize) -> (usize, usize, usize, usize, usize) {
	let path_len = u16::from_le_bytes([bytes[record], bytes[record + 1]]) as usize;
	let path = record + 2;
	let size = path + path_len;
	(path, size, size + 8, size + 16, size + 20)
}
