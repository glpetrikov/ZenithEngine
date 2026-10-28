use std::{
	io::{Read, Seek, SeekFrom},
	sync::{
		Arc,
		atomic::{AtomicU64, Ordering},
	},
};

use sha2::{Digest, Sha256};
use zenith_archive::{ArchiveReader, ArchiveWriter, EncryptionKey, EntryReader, FileSource, ReadAt};
use zenith_error::ZenithError;

use crate::common::{
	CHUNK, FOOTER_LEN, HEADER_LEN, TestResult, body_offset, build_archive, build_archive_with, fix_footer,
	fix_header_crc, hex, key, opts, put, random_bytes, record_fields, record_offsets, sample_files, u64_at,
};

fn mem(bytes: &[u8]) -> Arc<[u8]> {
	Arc::from(bytes.to_vec())
}

fn open(bytes: &[u8], k: Option<&EncryptionKey>) -> Result<ArchiveReader, ZenithError> {
	ArchiveReader::open(mem(bytes), k)
}

/// True when opening or reading the archive reports an error somewhere.
fn detects_damage(bytes: &[u8], k: Option<&EncryptionKey>) -> bool {
	open(bytes, k).map_or(true, |reader| {
		reader.entries().iter().any(|e| reader.read_all(&e.path).is_err())
	})
}

#[test]
fn stored_chunks_roundtrip_and_stay_readable() -> TestResult {
	let files = sample_files();
	for k in [None, Some(key(1))] {
		let options = zenith_archive::WriterOptions {
			compress: false,
			..opts(k.clone())
		};
		let bytes = build_archive_with(&files, &options)?;
		let reader = open(&bytes, k.as_ref())?;
		for (path, data) in &files {
			assert_eq!(&reader.read_all(path)?, data, "content of {path}");
		}
		let marker = b"name = \"Test\"";
		let visible = bytes.windows(marker.len()).any(|w| w == marker);
		assert_eq!(visible, k.is_none(), "the text is in the file unless it is encrypted");
		assert!(
			bytes.len() > files.values().map(Vec::len).sum::<usize>(),
			"nothing is compressed"
		);
		assert!(
			detects_damage(&flip(&bytes, bytes.len() / 2), k.as_ref()),
			"damage is found"
		);
	}
	Ok(())
}

fn flip(bytes: &[u8], at: usize) -> Vec<u8> {
	let mut damaged = bytes.to_vec();
	if let Some(byte) = damaged.get_mut(at) {
		*byte ^= 0xff;
	}
	damaged
}

#[test]
fn roundtrip_plain_and_encrypted() -> TestResult {
	let files = sample_files();
	for k in [None, Some(key(1))] {
		let bytes = build_archive(&files, k.clone())?;
		let reader = open(&bytes, k.as_ref())?;
		assert_eq!(reader.is_encrypted(), k.is_some(), "encryption flag is reported");
		assert_eq!(reader.entries().len(), files.len(), "all entries are listed");
		for (path, data) in &files {
			let entry = reader.find(path).ok_or("entry missing")?;
			assert_eq!(entry.size, data.len() as u64, "size of {path}");
			assert_eq!(&reader.read_all(path)?, data, "content of {path}");
		}
		assert!(reader.find("nope").is_none(), "unknown path is not found");
		assert!(
			matches!(reader.open_entry("nope"), Err(ZenithError::ArchiveEntryNotFound(_))),
			"opening an unknown path reports it"
		);
	}
	Ok(())
}

#[test]
fn streaming_read_and_seek() -> TestResult {
	let files = sample_files();
	let data = files.get("Assets/text.txt").ok_or("missing")?;
	let bytes = build_archive(&files, Some(key(2)))?;
	let reader = open(&bytes, Some(&key(2)))?;
	let mut entry = reader.open_entry("Assets/text.txt")?;
	assert_eq!(entry.size(), data.len() as u64, "size is exposed");

	let mut buf = vec![0u8; 1000];
	for start in [0usize, 1, CHUNK - 3, CHUNK, CHUNK * 4 + 7, data.len() - 1000] {
		entry.seek(SeekFrom::Start(start as u64))?;
		entry.read_exact(&mut buf)?;
		assert_eq!(&buf[..], &data[start..start + 1000], "bytes at {start}");
	}
	assert_eq!(entry.seek(SeekFrom::End(-10))?, data.len() as u64 - 10, "seek from end");
	let mut tail = Vec::new();
	entry.read_to_end(&mut tail)?;
	assert_eq!(tail, data[data.len() - 10..], "tail after seeking from the end");
	assert_eq!(entry.read(&mut buf)?, 0, "reading at the end returns 0");
	assert!(
		entry.seek(SeekFrom::Current(-1_000_000)).is_err(),
		"negative position is an error"
	);

	let mut all = Vec::new();
	entry.seek(SeekFrom::Start(0))?;
	entry.read_to_end(&mut all)?;
	assert_eq!(&all, data, "full streaming read");
	Ok(())
}

#[test]
fn key_errors() -> TestResult {
	let files = sample_files();
	let plain = build_archive(&files, None)?;
	let enc = build_archive(&files, Some(key(1)))?;

	assert!(
		matches!(open(&enc, None), Err(ZenithError::ArchiveEncryptedNoKey)),
		"no key"
	);
	assert!(
		matches!(open(&enc, Some(&key(2))), Err(ZenithError::ArchiveWrongKey)),
		"wrong key"
	);
	assert!(
		matches!(open(&plain, Some(&key(1))), Err(ZenithError::ArchiveNotEncrypted)),
		"unneeded key"
	);
	assert!(open(&enc, Some(&key(1))).is_ok(), "right key");
	Ok(())
}

#[test]
fn every_flipped_byte_is_detected() -> TestResult {
	let files = sample_files();
	for k in [None, Some(key(3))] {
		let bytes = build_archive(&files, k.clone())?;
		assert!(!detects_damage(&bytes, k.as_ref()), "intact archive is fine");
		for i in 0..bytes.len() {
			let mut damaged = bytes.clone();
			damaged[i] ^= 0x01;
			assert!(
				detects_damage(&damaged, k.as_ref()),
				"flip at byte {i} (encrypted: {}) went unnoticed",
				k.is_some()
			);
		}
	}
	Ok(())
}

#[test]
fn every_truncation_is_detected() -> TestResult {
	let files = sample_files();
	for k in [None, Some(key(3))] {
		let bytes = build_archive(&files, k.clone())?;
		for len in 0..bytes.len() {
			assert!(
				open(&bytes[..len], k.as_ref()).is_err(),
				"prefix of {len} bytes must not open"
			);
		}
		let mut extended = bytes.clone();
		extended.push(0);
		assert!(open(&extended, k.as_ref()).is_err(), "appended byte must not open");
	}
	Ok(())
}

#[test]
fn unsupported_version_and_bad_header_fields() -> TestResult {
	let bytes = build_archive(&sample_files(), None)?;

	let mut v = bytes.clone();
	v[8..10].copy_from_slice(&2u16.to_le_bytes());
	fix_header_crc(&mut v);
	assert!(
		matches!(open(&v, None), Err(ZenithError::UnsupportedArchiveVersion(2))),
		"future version"
	);

	for (at, value) in [
		(10usize, 0x80u8),
		(12, 1),
		(13, 9),
		(14, 25),
		(14, 11),
		(15, 1),
		(40, 1),
	] {
		let mut b = bytes.clone();
		b[at] = value;
		fix_header_crc(&mut b);
		assert!(open(&b, None).is_err(), "header byte {at} = {value} must be rejected");
	}
	assert!(open(&[], None).is_err(), "empty input");
	assert!(open(&[0u8; 200], None).is_err(), "zeros");
	Ok(())
}

/// Patches the unencrypted archive and fixes CRCs so only the structural
/// checks can reject it.
fn patched(bytes: &[u8], f: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
	let mut b = bytes.to_vec();
	f(&mut b);
	fix_footer(&mut b, false);
	b
}

#[test]
fn hostile_sizes_and_offsets_are_rejected() -> TestResult {
	let bytes = build_archive(&sample_files(), None)?;
	let records = record_offsets(&bytes);
	// sorted: 0 Assets/empty, 1 Assets/exact, 2 Assets/one_more, ...
	let (_, empty_size, ..) = record_fields(&bytes, *records.first().ok_or("record")?);
	let (_, _, exact_offset, exact_chunks, exact_first) = record_fields(&bytes, *records.get(1).ok_or("record")?);
	let body = body_offset(&bytes);

	let cases: Vec<(&str, Vec<u8>)> = vec![
		(
			"giant size",
			patched(&bytes, |b| {
				put(b, empty_size, &u64::MAX.to_le_bytes());
			}),
		),
		(
			"2^60 size",
			patched(&bytes, |b| {
				put(b, empty_size, &(1u64 << 60).to_le_bytes());
			}),
		),
		(
			"offset u64::MAX",
			patched(&bytes, |b| {
				put(b, exact_offset, &u64::MAX.to_le_bytes());
			}),
		),
		(
			"offset + 1",
			patched(&bytes, |b| {
				let v = u64_at(b, exact_offset) + 1;
				put(b, exact_offset, &v.to_le_bytes());
			}),
		),
		(
			"chunk count u32::MAX",
			patched(&bytes, |b| {
				put(b, exact_chunks, &u32::MAX.to_le_bytes());
			}),
		),
		(
			"stored_len u32::MAX",
			patched(&bytes, |b| {
				put(b, exact_first, &u32::MAX.to_le_bytes());
			}),
		),
		(
			"stored_len 0",
			patched(&bytes, |b| {
				put(b, exact_first, &0u32.to_le_bytes());
			}),
		),
		(
			"entry count u32::MAX",
			patched(&bytes, |b| put(b, body, &u32::MAX.to_le_bytes())),
		),
		(
			"entry count over limit",
			patched(&bytes, |b| {
				put(b, body, &((1u32 << 22) + 1).to_le_bytes());
			}),
		),
		(
			"entry count + 1",
			patched(&bytes, |b| {
				put(b, body, &(records.len() as u32 + 1).to_le_bytes());
			}),
		),
		(
			"entry count - 1",
			patched(&bytes, |b| {
				put(b, body, &(records.len() as u32 - 1).to_le_bytes());
			}),
		),
		(
			"body offset overflow",
			patched(&bytes, |b| {
				let f = b.len() - FOOTER_LEN;
				b[f + 8..f + 16].copy_from_slice(&(u64::MAX - 5).to_le_bytes());
			}),
		),
		(
			"body length 1 TiB",
			patched(&bytes, |b| {
				let f = b.len() - FOOTER_LEN;
				b[f + 16..f + 24].copy_from_slice(&(1u64 << 40).to_le_bytes());
			}),
		),
		(
			"body offset inside header",
			patched(&bytes, |b| {
				let f = b.len() - FOOTER_LEN;
				b[f + 8..f + 16].copy_from_slice(&4u64.to_le_bytes());
			}),
		),
	];
	for (name, damaged) in cases {
		assert!(open(&damaged, None).is_err(), "{name} must be rejected at open");
	}
	Ok(())
}

#[test]
fn hostile_paths_in_the_index_are_rejected() -> TestResult {
	let bytes = build_archive(&sample_files(), None)?;
	let records = record_offsets(&bytes);
	let path_of = |i: usize| record_fields(&bytes, records[i]).0;
	// records: 0 Assets/empty, 1 Assets/exact, ..., 5 Settings/x.toml (15
	// bytes), 6 Test.zenithproject
	let settings = path_of(5);
	assert_eq!(
		&bytes[settings..settings + 15],
		b"Settings/x.toml",
		"test layout assumption"
	);

	let set = |at: usize, text: &[u8]| patched(&bytes, |b| b[at..at + text.len()].copy_from_slice(text));
	let cases = vec![
		("dot-dot", set(settings, b"../../../x/y/zz")),
		("absolute", set(settings, b"/Settings/x.tom")),
		("backslash", set(settings, b"Settings\\x.toml")),
		("drive", set(settings, b"C:/Settings/x.t")),
		("duplicate", set(path_of(1), b"Assets/empty")),
		("unsorted", set(path_of(0), b"Zssets/empty")),
	];
	for (name, damaged) in cases {
		assert!(open(&damaged, None).is_err(), "{name} path must be rejected");
	}

	// file/dir conflict: "a" and "aXb" with X turned into '/'
	let mut files = sample_files();
	files.insert("a".into(), b"1".to_vec());
	files.insert("aXb".into(), b"2".to_vec());
	let bytes = build_archive(&files, None)?;
	let records = record_offsets(&bytes);
	let target = records
		.iter()
		.map(|r| record_fields(&bytes, *r).0)
		.find(|p| &bytes[*p..*p + 3] == b"aXb")
		.ok_or("record not found")?;
	let damaged = patched(&bytes, |b| b[target + 1] = b'/');
	assert!(
		open(&damaged, None).is_err(),
		"file and directory with the same prefix must be rejected"
	);

	// two manifests: "B.zenithprojecx" with x turned into t
	let mut files = sample_files();
	files.insert("B.zenithprojecx".into(), b"x".to_vec());
	let bytes = build_archive(&files, None)?;
	let target = record_offsets(&bytes)
		.iter()
		.map(|r| record_fields(&bytes, *r).0)
		.find(|p| &bytes[*p..*p + 15] == b"B.zenithprojecx")
		.ok_or("record not found")?;
	let damaged = patched(&bytes, |b| b[target + 14] = b't');
	assert!(open(&damaged, None).is_err(), "a second root manifest must be rejected");
	Ok(())
}

#[test]
fn writer_enforces_the_same_rules() -> TestResult {
	let mut w = ArchiveWriter::new(Vec::new(), &opts(None))?;
	w.add_bytes("b", b"1")?;
	assert!(w.add_bytes("a", b"1").is_err(), "out-of-order path");
	assert!(w.add_bytes("b", b"1").is_err(), "duplicate path");
	assert!(w.add_bytes("../x", b"1").is_err(), "traversal path");
	assert!(w.add_bytes("/x", b"1").is_err(), "absolute path");
	assert!(w.finish().is_err(), "no manifest");

	let mut bad = opts(None);
	bad.compression_level = 0;
	assert!(ArchiveWriter::new(Vec::new(), &bad).is_err(), "level 0");
	bad.compression_level = 23;
	assert!(ArchiveWriter::new(Vec::new(), &bad).is_err(), "level 23");
	bad.compression_level = 3;
	bad.chunk_size_log2 = 30;
	assert!(ArchiveWriter::new(Vec::new(), &bad).is_err(), "huge chunk size");
	Ok(())
}

/// A reader that hands out `good` bytes and then fails.
struct FailsAfter(usize);

impl Read for FailsAfter {
	fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
		if self.0 == 0 {
			return Err(std::io::Error::other("the source went away"));
		}
		let n = self.0.min(buf.len());
		buf.iter_mut().take(n).for_each(|b| *b = 7);
		self.0 -= n;
		Ok(n)
	}
}

#[test]
fn a_failed_add_makes_the_writer_unusable() -> TestResult {
	let mut w = ArchiveWriter::new(Vec::new(), &opts(None))?;
	assert!(
		w.add("a.bin", FailsAfter(CHUNK + 100)).is_err(),
		"the source fails after one whole chunk was written"
	);
	assert!(
		w.add_bytes("T.zenithproject", b"m").is_err(),
		"later adds are refused instead of building an unreadable archive"
	);
	assert!(w.finish().is_err(), "and so is finish");

	let mut w = ArchiveWriter::new(Vec::new(), &opts(None))?;
	assert!(
		w.add_bytes("../x", b"1").is_err(),
		"a bad path fails before anything is written"
	);
	w.add_bytes("T.zenithproject", b"m")?;
	let bytes = w.finish()?;
	assert!(open(&bytes, None).is_ok(), "a rejected path does not spoil the writer");
	Ok(())
}

#[test]
fn windows_hostile_paths_cannot_be_written() -> TestResult {
	for path in ["CON", "dir/nul.txt", "ends.", "ends /x"] {
		let mut w = ArchiveWriter::new(Vec::new(), &opts(None))?;
		assert!(w.add_bytes(path, b"1").is_err(), "{path:?} is refused by the writer");
	}
	let mut w = ArchiveWriter::new(Vec::new(), &opts(None))?;
	w.add_bytes("A.png", b"1")?;
	w.add_bytes("T.zenithproject", b"m")?;
	w.add_bytes("a.png", b"2")?;
	assert!(w.finish().is_err(), "two spellings of one file are refused at finish");
	Ok(())
}

#[test]
fn swapped_chunks_between_files_fail_authentication() -> TestResult {
	// Equal-sized random files have equal stored sizes, so their chunk regions
	// can be swapped without touching the index. Only the path in the AAD can
	// catch it.
	let mut files = std::collections::BTreeMap::new();
	files.insert("Assets/a".to_string(), random_bytes(CHUNK * 2, 10));
	files.insert("Assets/b".to_string(), random_bytes(CHUNK * 2, 11));
	files.insert("T.zenithproject".to_string(), b"m".to_vec());
	let k = key(5);

	let manifest_only = std::collections::BTreeMap::from([("T.zenithproject".to_string(), b"m".to_vec())]);
	let m = build_archive(&manifest_only, Some(k.clone()))?;
	let manifest_region = body_offset(&m) - HEADER_LEN;

	let bytes = build_archive(&files, Some(k.clone()))?;
	let data_region = body_offset(&bytes) - HEADER_LEN;
	let each = (data_region - manifest_region) / 2;
	let (a, b) = (HEADER_LEN, HEADER_LEN + each);

	let mut swapped = bytes.clone();
	swapped[a..a + each].copy_from_slice(&bytes[b..b + each]);
	swapped[b..b + each].copy_from_slice(&bytes[a..a + each]);

	let reader = open(&swapped, Some(&k))?;
	assert!(
		reader.read_all("Assets/a").is_err(),
		"swapped chunks of a must not authenticate"
	);
	assert!(
		reader.read_all("Assets/b").is_err(),
		"swapped chunks of b must not authenticate"
	);
	assert!(
		reader.read_all("T.zenithproject").is_ok(),
		"untouched entries still read"
	);
	Ok(())
}

struct Counting {
	bytes: Arc<[u8]>,
	read: Arc<AtomicU64>,
}

impl ReadAt for Counting {
	fn len(&self) -> u64 {
		ReadAt::len(&self.bytes)
	}

	fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
		let n = self.bytes.read_at(offset, buf)?;
		self.read.fetch_add(n as u64, Ordering::Relaxed);
		Ok(n)
	}
}

#[test]
fn reading_one_file_does_not_touch_the_others() -> TestResult {
	let mut files = sample_files();
	files.insert("Assets/big.bin".into(), random_bytes(CHUNK * 200, 99));
	let bytes = build_archive(&files, Some(key(4)))?;
	let counter = Arc::new(AtomicU64::new(0));
	let reader = ArchiveReader::open(
		Counting {
			bytes: mem(&bytes),
			read: counter.clone(),
		},
		Some(&key(4)),
	)?;
	let after_open = counter.load(Ordering::Relaxed);
	assert_eq!(reader.read_all("Settings/x.toml")?, b"a = 1\n", "small file reads");
	let used = counter.load(Ordering::Relaxed);
	assert!(
		used < bytes.len() as u64 / 20,
		"read {used} of {} bytes (open took {after_open})",
		bytes.len()
	);
	Ok(())
}

#[test]
fn file_source_and_threads() -> TestResult {
	let files = sample_files();
	let bytes = build_archive(&files, Some(key(6)))?;
	let dir = tempfile::tempdir()?;
	let path = dir.path().join("build.zenithbuild");
	std::fs::write(&path, &bytes)?;

	let reader = Arc::new(ArchiveReader::open(FileSource::open(&path)?, Some(&key(6)))?);
	std::thread::scope(|scope| {
		for _ in 0..4 {
			let reader = reader.clone();
			let files = &files;
			scope.spawn(move || {
				for (path, data) in files {
					assert_eq!(
						&reader.read_all(path).unwrap_or_default(),
						data,
						"threaded read of {path}"
					);
				}
			});
		}
	});
	Ok(())
}

#[test]
fn reader_types_are_thread_safe() {
	fn send_sync<T: Send + Sync>() {}
	fn send<T: Send>() {}
	send_sync::<ArchiveReader>();
	send::<EntryReader>();
}

#[test]
fn output_is_reproducible_and_golden() -> TestResult {
	let files = sample_files();
	for k in [None, Some(key(7))] {
		let a = build_archive(&files, k.clone())?;
		let b = build_archive(&files, k.clone())?;
		assert_eq!(a, b, "two builds of the same input are byte-identical");
	}

	// Incompressible data only: zstd stores it as raw blocks, so these hashes
	// do not depend on the zstd library version. If they change, the on-disk
	// format changed and FORMAT_VERSION must be considered.
	let mut golden = std::collections::BTreeMap::new();
	golden.insert("G.zenithproject".to_string(), random_bytes(100, 21));
	golden.insert("data.bin".to_string(), random_bytes(CHUNK * 2 + 5, 22));
	let hash = |k: Option<EncryptionKey>| -> Result<String, Box<dyn std::error::Error>> {
		let digest = Sha256::digest(build_archive(&golden, k)?);
		Ok(hex(&digest))
	};
	assert_eq!(hash(None)?, GOLDEN_PLAIN, "golden hash of the plain archive");
	assert_eq!(
		hash(Some(key(7)))?,
		GOLDEN_ENCRYPTED,
		"golden hash of the encrypted archive"
	);
	Ok(())
}

const GOLDEN_PLAIN: &str = "f72fd3df95aa0aa1ef8e0fb487dab4b93074526c8a84fb4218e11c60b6093afe";
const GOLDEN_ENCRYPTED: &str = "ac5b5c854bf01e75cf4c703bdd8daaa5ebdd59d2d29d3b8fd6bd4b0d4a11c147";

#[test]
fn the_output_does_not_depend_on_the_number_of_threads() -> TestResult {
	// Files of many sizes, so that the last batch of chunks is full, partial or
	// empty for different thread counts.
	let mut files = sample_files();
	for (i, chunks) in [1usize, 2, 3, 5, 7, 8, 9, 16, 17].into_iter().enumerate() {
		files.insert(format!("Assets/f{i}"), random_bytes(CHUNK * chunks + i, 100 + i as u64));
	}
	for k in [None, Some(key(3))] {
		let build = |threads: usize| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
			let mut options = opts(k.clone());
			options.threads = threads;
			let mut writer = ArchiveWriter::new(Vec::new(), &options)?;
			for (path, data) in &files {
				writer.add_bytes(path, data)?;
			}
			Ok(writer.finish()?)
		};
		let single = build(1)?;
		for threads in [0, 2, 3, 4, 8, 64] {
			assert_eq!(
				build(threads)?,
				single,
				"{threads} threads gave other bytes (encrypted: {})",
				k.is_some()
			);
		}
		let reader = open(&single, k.as_ref())?;
		for (path, data) in &files {
			assert_eq!(&reader.read_all(path)?, data, "{path} reads back");
		}
	}
	Ok(())
}

#[test]
fn a_failing_source_fails_the_add_with_any_thread_count() -> TestResult {
	for threads in [1usize, 4] {
		let mut options = opts(None);
		options.threads = threads;
		let mut w = ArchiveWriter::new(Vec::new(), &options)?;
		assert!(
			w.add("a.bin", FailsAfter(CHUNK * 5 + 3)).is_err(),
			"{threads} threads: the error reaches the caller"
		);
	}
	Ok(())
}

#[test]
fn the_memory_estimate_grows_with_the_level_and_the_chunk() {
	let at = |level: i32, log2: u8| {
		let mut o = opts(None);
		o.compression_level = level;
		o.chunk_size_log2 = log2;
		o.thread_memory()
	};
	assert!(at(3, 18) < at(19, 18), "a higher level needs more");
	assert!(at(9, 18) < at(9, 24), "a bigger chunk needs more");
	assert!(
		at(22, 24) < (1 << 30),
		"the worst case is still under a gigabyte per thread: {}",
		at(22, 24)
	);
}

#[test]
fn sealed_entries_give_the_same_archive_as_adding_directly() -> TestResult {
	use zenith_archive::EntrySealer;

	let files = sample_files();
	for k in [None, Some(key(6))] {
		let direct = build_archive(&files, k.clone())?;

		// Seal every file on its own thread, then append in path order.
		let options = opts(k.clone());
		let sealed: Vec<_> = std::thread::scope(|scope| {
			// Every thread is started before the first is joined.
			#[allow(clippy::needless_collect)]
			let handles: Vec<_> = files
				.iter()
				.map(|(path, data)| {
					let options = options.clone();
					scope.spawn(move || {
						let mut sealer = EntrySealer::new(&options)?;
						sealer.seal(path, data.as_slice()).map(|entry| (path.clone(), entry))
					})
				})
				.collect();
			handles.into_iter().map(std::thread::ScopedJoinHandle::join).collect()
		});
		let mut writer = ArchiveWriter::new(Vec::new(), &options)?;
		for result in sealed {
			let (path, entry) = result.map_err(|_| "a sealing thread panicked")??;
			writer.add_sealed(&path, entry)?;
		}
		assert_eq!(
			writer.finish()?,
			direct,
			"sealed on other threads, the archive is byte-identical (encrypted: {})",
			k.is_some()
		);
	}
	Ok(())
}

#[test]
fn a_sealed_entry_must_fit_the_writer() -> TestResult {
	use zenith_archive::EntrySealer;

	let mut sealer = EntrySealer::new(&opts(Some(key(1))))?;
	let entry = sealer.seal("T.zenithproject", b"m".as_slice())?;
	let mut other_key = ArchiveWriter::new(Vec::new(), &opts(Some(key(2))))?;
	assert!(
		other_key.add_sealed("T.zenithproject", entry).is_err(),
		"sealed with another key"
	);

	let mut sealer = EntrySealer::new(&opts(None))?;
	let mut writer = ArchiveWriter::new(Vec::new(), &opts(None))?;
	writer.add_sealed("T.zenithproject", sealer.seal("T.zenithproject", b"m".as_slice())?)?;
	writer.add_sealed("b", sealer.seal("b", b"1".as_slice())?)?;
	assert!(
		writer.add_sealed("a", sealer.seal("a", b"1".as_slice())?).is_err(),
		"order is checked as with add"
	);
	assert!(
		writer.add_sealed("../x", sealer.seal("c", b"1".as_slice())?).is_err(),
		"and so is the path"
	);
	assert!(writer.finish().is_ok(), "rejected entries did not spoil the writer");
	Ok(())
}

#[test]
fn a_chunk_of_an_older_build_with_the_same_key_does_not_fit() -> TestResult {
	// Two releases of one file: same path, same size, other content. The key is
	// the same (it has to be, or a folder build could not be updated).
	let k = key(8);
	let release = |seed: u64| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
		let mut files = std::collections::BTreeMap::new();
		files.insert("G.zenithproject".to_string(), b"m".to_vec());
		files.insert("data.bin".to_string(), random_bytes(CHUNK * 3, seed));
		build_archive(&files, Some(k.clone()))
	};
	let (old, new) = (release(1)?, release(2)?);
	assert_eq!(old.len(), new.len(), "the releases differ only in content");

	let manifest_only = std::collections::BTreeMap::from([("G.zenithproject".to_string(), b"m".to_vec())]);
	let manifest_region = body_offset(&build_archive(&manifest_only, Some(k.clone()))?) - HEADER_LEN;
	let chunk = (body_offset(&new) - HEADER_LEN - manifest_region) / 3;
	let at = HEADER_LEN + manifest_region + chunk;

	assert!(
		open(&new, Some(&k))?.read_all("data.bin").is_ok(),
		"the release itself reads"
	);

	let mut spliced = new.clone();
	spliced[at..at + chunk].copy_from_slice(&old[at..at + chunk]);
	assert_ne!(spliced, new, "a chunk of the old release is in the new one");
	let reader = open(&spliced, Some(&k))?;
	let err = reader.read_all("data.bin").err().ok_or("the old chunk was accepted")?;
	assert!(
		matches!(err, ZenithError::ArchiveCorrupt(ref m) if m.contains("another version")),
		"{err:?}"
	);
	// The chunks before and after it are still fine on their own.
	let mut stream = reader.open_entry("data.bin")?;
	let mut first = vec![0u8; CHUNK];
	stream.read_exact(&mut first)?;
	assert_eq!(
		first,
		random_bytes(CHUNK * 3, 2)[..CHUNK],
		"the untouched first chunk reads"
	);
	Ok(())
}

#[test]
fn the_build_id_names_the_build() -> TestResult {
	let files = sample_files();
	let mut ids = Vec::new();
	for k in [None, Some(key(3))] {
		let id = open(&build_archive(&files, k.clone())?, k.as_ref())?.build_id();
		assert_eq!(
			id,
			open(&build_archive(&files, k.clone())?, k.as_ref())?.build_id(),
			"the same build has the same id"
		);

		let mut longer = files.clone();
		longer.get_mut("Assets/text.txt").ok_or("missing")?.push(b'!');
		assert_ne!(
			id,
			open(&build_archive(&longer, k.clone())?, k.as_ref())?.build_id(),
			"a file of another size"
		);

		// Same size, other bytes: only what pins the chunks (their CRC, or with
		// a key their tag) tells.
		let mut other = files.clone();
		let len = other.get("Assets/random.bin").ok_or("missing")?.len();
		other.insert("Assets/random.bin".to_string(), random_bytes(len, 99));
		assert_ne!(
			id,
			open(&build_archive(&other, k.clone())?, k.as_ref())?.build_id(),
			"a file with the same size and other content (encrypted: {})",
			k.is_some()
		);
		ids.push(id);
	}
	assert_ne!(ids.first(), ids.get(1), "an encrypted build is another build");
	Ok(())
}

#[test]
fn an_older_release_has_another_id() -> TestResult {
	let k = key(8);
	let release = |seed: u64| -> Result<_, Box<dyn std::error::Error>> {
		let mut files = std::collections::BTreeMap::new();
		files.insert("G.zenithproject".to_string(), b"m".to_vec());
		files.insert("data.bin".to_string(), random_bytes(CHUNK * 3, seed));
		Ok(open(&build_archive(&files, Some(k.clone()))?, Some(&k))?.build_id())
	};
	assert_ne!(release(1)?, release(2)?, "same key, size and path, other content");
	assert_eq!(release(1)?, release(1)?, "and the same release is the same id");
	Ok(())
}
