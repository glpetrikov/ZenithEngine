use std::{collections::BTreeMap, io::Read, sync::Arc};

use zenith_archive::{EncryptionKey, FolderIndexEntry, FolderReader, FolderWriter, MemDir};
use zenith_error::ZenithError;

use crate::common::{
	CHUNK, Folder, LOG2, TestResult, body_offset, build_folder, build_folder_with, content_hash, fix_footer, hex, key,
	opts, random_bytes, sample_files,
};

fn open(folder: &Folder, k: Option<&EncryptionKey>) -> Result<FolderReader, ZenithError> {
	FolderReader::open(folder.mem_dir(), k)
}

fn detects_damage(folder: &Folder, k: Option<&EncryptionKey>) -> bool {
	open(folder, k).map_or(true, |reader| {
		reader.entries().iter().any(|e| reader.read_all(&e.path).is_err())
	})
}

fn replaced(folder: &Folder, name: &str, bytes: Vec<u8>) -> Folder {
	let mut files = folder.files.clone();
	files.insert(name.to_string(), Arc::from(bytes));
	Folder { files }
}

#[test]
fn stored_files_roundtrip_and_stay_readable() -> TestResult {
	let files = sample_files();
	for k in [None, Some(key(1))] {
		let options = zenith_archive::WriterOptions {
			compress: false,
			..opts(k.clone())
		};
		let folder = build_folder_with(&files, &options)?;
		let reader = open(&folder, k.as_ref())?;
		for (path, data) in &files {
			assert_eq!(&reader.read_all(path)?, data, "content of {path}");
		}
		let marker = b"name = \"Test\"";
		let visible = folder
			.files
			.values()
			.any(|file| file.windows(marker.len()).any(|w| w == marker));
		assert_eq!(visible, k.is_none(), "the text is in a file unless it is encrypted");
	}
	Ok(())
}

#[test]
fn roundtrip_plain_and_encrypted() -> TestResult {
	let files = sample_files();
	for k in [None, Some(key(1))] {
		let folder = build_folder(&files, k.clone())?;
		let reader = open(&folder, k.as_ref())?;
		assert_eq!(reader.is_encrypted(), k.is_some(), "encryption flag is reported");
		assert_eq!(reader.entries().len(), files.len(), "all entries are listed");
		for (path, data) in &files {
			assert_eq!(&reader.read_all(path)?, data, "content of {path}");
			let mut streamed = Vec::new();
			reader.open_entry(path)?.read_to_end(&mut streamed)?;
			assert_eq!(&streamed, data, "streamed content of {path}");
		}
		assert!(
			matches!(reader.open_entry("nope"), Err(ZenithError::ArchiveEntryNotFound(_))),
			"unknown path"
		);
	}
	Ok(())
}

#[test]
fn data_file_names() -> TestResult {
	let files = sample_files();
	let plain = build_folder(&files, None)?;
	for path in files.keys() {
		assert!(
			plain.files.contains_key(path),
			"unencrypted data file is named after {path}"
		);
	}

	let enc = build_folder(&files, Some(key(1)))?;
	let again = build_folder(&files, Some(key(1)))?;
	let other_key = build_folder(&files, Some(key(2)))?;
	let names: Vec<&String> = enc.files.keys().filter(|n| n.as_str() != "build.index").collect();
	assert_eq!(names.len(), files.len(), "one data file per entry");
	for name in names {
		let (dir, file) = name.split_once('/').ok_or("fan-out directory expected")?;
		assert!(
			dir.len() == 2 && file.len() == 30 && name.bytes().all(|b| b == b'/' || b.is_ascii_hexdigit()),
			"encrypted name {name} is 2 + 30 lowercase hex"
		);
		assert!(
			files.keys().all(|p| !name.contains(p.as_str())),
			"name {name} must not reveal a path"
		);
		assert!(again.files.contains_key(name), "names are stable between builds");
		assert!(!other_key.files.contains_key(name), "names depend on the key");
	}
	Ok(())
}

#[test]
fn unchanged_files_are_byte_identical_between_builds() -> TestResult {
	let files = sample_files();
	for k in [None, Some(key(1))] {
		let before = build_folder(&files, k.clone())?;
		let mut changed = files.clone();
		changed.insert("Assets/text.txt".into(), b"different".to_vec());
		let after = build_folder(&changed, k.clone())?;

		let diff: Vec<&String> = before
			.files
			.iter()
			.filter(|(name, bytes)| after.files.get(*name) != Some(*bytes))
			.map(|(name, _)| name)
			.collect();
		assert_eq!(
			diff.len(),
			2,
			"only the changed data file and the index differ: {diff:?}"
		);
		assert!(diff.iter().any(|n| n.as_str() == "build.index"), "the index differs");
	}
	Ok(())
}

#[test]
fn key_errors() -> TestResult {
	let files = sample_files();
	let plain = build_folder(&files, None)?;
	let enc = build_folder(&files, Some(key(1)))?;
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
	Ok(())
}

#[test]
fn every_flipped_byte_is_detected() -> TestResult {
	let mut files = BTreeMap::new();
	files.insert("Assets/a.bin".to_string(), random_bytes(CHUNK + 40, 5));
	files.insert("T.zenithproject".to_string(), b"manifest".to_vec());
	for k in [None, Some(key(3))] {
		let folder = build_folder(&files, k.clone())?;
		assert!(!detects_damage(&folder, k.as_ref()), "intact build is fine");
		for (name, bytes) in &folder.files {
			for i in 0..bytes.len() {
				let mut damaged = bytes.to_vec();
				damaged[i] ^= 0x01;
				let broken = replaced(&folder, name, damaged);
				assert!(
					detects_damage(&broken, k.as_ref()),
					"flip at byte {i} of {name} (encrypted: {}) went unnoticed",
					k.is_some()
				);
			}
		}
	}
	Ok(())
}

#[test]
fn truncated_files_are_detected() -> TestResult {
	let files = sample_files();
	for k in [None, Some(key(3))] {
		let folder = build_folder(&files, k.clone())?;
		for (name, bytes) in &folder.files {
			for len in [0, 1, 47, 48, bytes.len() / 2, bytes.len() - 1] {
				let broken = replaced(&folder, name, bytes[..len].to_vec());
				assert!(detects_damage(&broken, k.as_ref()), "{name} cut to {len} bytes");
			}
		}
	}
	Ok(())
}

#[test]
fn swapped_data_files_of_one_build_fail_authentication() -> TestResult {
	let mut files = BTreeMap::new();
	files.insert("Assets/a".to_string(), random_bytes(CHUNK * 2, 10));
	files.insert("Assets/b".to_string(), random_bytes(CHUNK * 2, 11));
	files.insert("T.zenithproject".to_string(), b"m".to_vec());
	let k = key(5);
	let folder = build_folder(&files, Some(k.clone()))?;

	let writer = FolderWriter::new(&opts(Some(k.clone())))?;
	let (name_a, name_b) = (writer.data_file_name("Assets/a")?, writer.data_file_name("Assets/b")?);
	let a = folder.files.get(&name_a).ok_or("data file a")?.to_vec();
	let b = folder.files.get(&name_b).ok_or("data file b")?.to_vec();
	assert_eq!(a.len(), b.len(), "test needs equally sized data files");

	let swapped = replaced(&replaced(&folder, &name_a, b), &name_b, a);
	let reader = open(&swapped, Some(&k))?;
	assert!(
		reader.read_all("Assets/a").is_err(),
		"a's slot holding b's file must fail"
	);
	assert!(
		reader.read_all("Assets/b").is_err(),
		"b's slot holding a's file must fail"
	);
	assert!(
		reader.read_all("T.zenithproject").is_ok(),
		"other entries are unaffected"
	);
	Ok(())
}

#[test]
fn data_file_from_another_build_is_rejected() -> TestResult {
	let files = sample_files();
	let ours = build_folder(&files, Some(key(1)))?;
	let theirs = build_folder(&files, Some(key(2)))?;
	let writer = FolderWriter::new(&opts(Some(key(1))))?;
	let name = writer.data_file_name("Assets/text.txt")?;
	let foreign = replaced(
		&ours,
		&name,
		theirs.files.values().next().map(|b| b.to_vec()).unwrap_or_default(),
	);
	let reader = open(&foreign, Some(&key(1)))?;
	assert!(
		reader.read_all("Assets/text.txt").is_err(),
		"a file with a foreign header is rejected"
	);
	Ok(())
}

#[test]
fn hostile_index_is_rejected() -> TestResult {
	let files = sample_files();
	let folder = build_folder(&files, None)?;
	let index = folder.files.get("build.index").ok_or("index")?.to_vec();
	let body = body_offset(&index);
	assert_eq!(body, 48, "index body follows the header");

	let patch = |f: &dyn Fn(&mut Vec<u8>)| {
		let mut b = index.clone();
		f(&mut b);
		fix_footer(&mut b, false);
		replaced(&folder, "build.index", b)
	};
	// record 0 is "Assets/empty": path_len(2) path(12) name_len(2) name(12)
	// size data_len
	let name_at = body + 4 + 2 + 12 + 2;
	let cases = vec![
		("name differs from path", patch(&|b| b[name_at] = b'B')),
		(
			"entry count u32::MAX",
			patch(&|b| b[body..body + 4].copy_from_slice(&u32::MAX.to_le_bytes())),
		),
		(
			"entry count over limit",
			patch(&|b| b[body..body + 4].copy_from_slice(&((1u32 << 22) + 1).to_le_bytes())),
		),
		(
			"path traversal",
			patch(&|b| b[body + 6..body + 18].copy_from_slice(b"../../../x/y")),
		),
	];
	for (what, broken) in cases {
		assert!(open(&broken, None).is_err(), "{what} must be rejected");
	}
	Ok(())
}

#[test]
fn writer_rejects_bad_index_input() -> TestResult {
	let writer = FolderWriter::new(&opts(None))?;
	let entry = |path: &str| FolderIndexEntry {
		path: path.into(),
		size: 0,
		data_len: 80,
		content_hash: [0; 32],
		compression_level: 3,
		chunk_size_log2: LOG2,
	};
	assert!(writer.encode_index(&[entry("a")]).is_err(), "no manifest");
	assert!(
		writer
			.encode_index(&[entry("b"), entry("a"), entry("T.zenithproject")])
			.is_err(),
		"unsorted"
	);
	assert!(
		writer
			.encode_index(&[entry("build.index"), entry("T.zenithproject")])
			.is_err(),
		"reserved file name"
	);
	assert!(
		writer.encode_index(&[entry("T.zenithproject")]).is_ok(),
		"minimal valid index"
	);
	for (what, level, log2) in [
		("level 0", 0, LOG2),
		("level 23", 23, LOG2),
		("another chunk size", 3, LOG2 + 1),
	] {
		let mut bad = entry("T.zenithproject");
		bad.compression_level = level;
		bad.chunk_size_log2 = log2;
		assert!(writer.encode_index(&[bad]).is_err(), "{what} is refused by the writer");
	}
	assert!(writer.data_file_name("../x").is_err(), "traversal in a data file name");
	Ok(())
}

#[test]
fn missing_data_file_is_an_error() -> TestResult {
	let folder = build_folder(&sample_files(), None)?;
	let mut dir = MemDir::new();
	dir.insert("build.index", folder.files.get("build.index").ok_or("index")?.clone());
	let reader = FolderReader::open(dir, None)?;
	assert!(reader.read_all("Assets/text.txt").is_err(), "missing data file");
	Ok(())
}

#[test]
fn folder_output_is_reproducible_and_golden() -> TestResult {
	use sha2::{Digest, Sha256};
	let files = sample_files();
	for k in [None, Some(key(7))] {
		let a = build_folder(&files, k.clone())?;
		let b = build_folder(&files, k.clone())?;
		assert_eq!(a.files, b.files, "two folder builds are identical file by file");
	}

	let mut golden = BTreeMap::new();
	golden.insert("G.zenithproject".to_string(), random_bytes(100, 21));
	golden.insert("data.bin".to_string(), random_bytes(CHUNK * 2 + 5, 22));
	let hash = |k: Option<EncryptionKey>| -> Result<String, Box<dyn std::error::Error>> {
		let folder = build_folder(&golden, k)?;
		let mut h = Sha256::new();
		for (name, bytes) in &folder.files {
			h.update(name.as_bytes());
			h.update(bytes);
		}
		Ok(hex(&h.finalize()))
	};
	assert_eq!(hash(None)?, GOLDEN_PLAIN, "golden hash of the plain folder");
	assert_eq!(
		hash(Some(key(7)))?,
		GOLDEN_ENCRYPTED,
		"golden hash of the encrypted folder"
	);
	Ok(())
}

const GOLDEN_PLAIN: &str = "9c17f8fe26140a1dcf5b34e2b03f8f4eb545141d20c29c031a31dc01c4370d44";
const GOLDEN_ENCRYPTED: &str = "c893f5759d70afb73c1cb00d63d3ff3c87abb100c20609d1f3f32d725bcdbeb3";

#[test]
fn the_index_records_hash_and_compression_parameters() -> TestResult {
	let files = sample_files();
	for k in [None, Some(key(1))] {
		let folder = build_folder(&files, k.clone())?;
		let reader = open(&folder, k.as_ref())?;
		for (path, data) in &files {
			let entry = reader.find(path).ok_or("entry missing")?;
			assert_eq!(entry.content_hash, content_hash(data), "hash of {path}");
			assert_eq!(
				(entry.compression_level, entry.chunk_size_log2),
				(3, LOG2),
				"parameters of {path}"
			);
		}
		let index = folder.files.get("build.index").ok_or("index")?;
		let hash = content_hash(files.get("Assets/text.txt").ok_or("file")?);
		let visible = index.windows(32).any(|w| w == hash);
		assert_eq!(
			visible,
			k.is_none(),
			"a plain index shows the hash, an encrypted one hides it"
		);
	}
	Ok(())
}

#[test]
fn hostile_compression_parameters_are_rejected() -> TestResult {
	let folder = build_folder(&sample_files(), None)?;
	let index = folder.files.get("build.index").ok_or("index")?.to_vec();
	let body = body_offset(&index);
	// record 0 is "Assets/empty": path_len(2) path(12) name_len(2) name(12)
	// size(8) data_len(8), then the new fields
	let fields = body + 4 + 2 + 12 + 2 + 12 + 16;
	let (level_at, log2_at) = (fields + 32, fields + 33);
	let patch = |at: usize, value: u8| {
		let mut b = index.clone();
		b[at] = value;
		fix_footer(&mut b, false);
		replaced(&folder, "build.index", b)
	};
	assert!(open(&folder, None).is_ok(), "the unpatched index is fine");
	for (what, broken) in [
		("level 0", patch(level_at, 0)),
		("level 23", patch(level_at, 23)),
		("level 255", patch(level_at, 255)),
		("another chunk size", patch(log2_at, LOG2 + 1)),
		("chunk size 0", patch(log2_at, 0)),
	] {
		assert!(open(&broken, None).is_err(), "{what} must be rejected");
	}
	Ok(())
}

#[test]
fn a_data_file_of_an_older_build_with_the_same_key_does_not_fit() -> TestResult {
	for k in [None, Some(key(8))] {
		let release = |seed: u64| -> Result<_, Box<dyn std::error::Error>> {
			let mut files = BTreeMap::new();
			files.insert("G.zenithproject".to_string(), b"m".to_vec());
			files.insert("data.bin".to_string(), random_bytes(CHUNK * 3, seed));
			build_folder(&files, k.clone())
		};
		let (old, new) = (release(1)?, release(2)?);
		let name = new
			.files
			.keys()
			.find(|n| n.as_str() != "build.index" && n.as_str() != "G.zenithproject" && !n.contains("zenithproject"))
			.ok_or("no data file")?
			.clone();
		// With a key the names are hashed; the manifest's file is the other
		// one.
		let data_name = if k.is_some() {
			let writer = zenith_archive::FolderWriter::new(&opts(k.clone()))?;
			writer.data_file_name("data.bin")?
		} else {
			name
		};
		assert_eq!(
			old.files.get(&data_name).map(|b| b.len()),
			new.files.get(&data_name).map(|b| b.len()),
			"same length, so only the content can tell them apart"
		);

		assert!(
			FolderReader::open(new.mem_dir(), k.as_ref())?
				.read_all("data.bin")
				.is_ok(),
			"the release itself reads"
		);
		let mut mixed = new.mem_dir();
		mixed.insert(data_name.clone(), old.files.get(&data_name).ok_or("missing")?.clone());
		let err = FolderReader::open(mixed, k.as_ref())?
			.read_all("data.bin")
			.err()
			.ok_or("the data file of the old release was accepted")?;
		assert!(
			matches!(err, ZenithError::ArchiveCorrupt(ref m) if m.contains("other content")),
			"(encrypted: {}) {err:?}",
			k.is_some()
		);
	}
	Ok(())
}

#[test]
fn the_folder_build_id_names_the_build() -> TestResult {
	let files = sample_files();
	for k in [None, Some(key(3))] {
		let id = |files: &BTreeMap<String, Vec<u8>>| -> Result<_, Box<dyn std::error::Error>> {
			Ok(FolderReader::open(build_folder(files, k.clone())?.mem_dir(), k.as_ref())?.build_id())
		};
		let base = id(&files)?;
		assert_eq!(base, id(&files)?, "the same build has the same id");

		// Same size, other bytes: the content hash in the index tells.
		let mut other = files.clone();
		let len = other.get("Assets/random.bin").ok_or("missing")?.len();
		other.insert("Assets/random.bin".to_string(), random_bytes(len, 99));
		assert_ne!(base, id(&other)?, "other content (encrypted: {})", k.is_some());

		other.remove("Assets/random.bin");
		assert_ne!(base, id(&other)?, "a file less");
	}
	Ok(())
}
