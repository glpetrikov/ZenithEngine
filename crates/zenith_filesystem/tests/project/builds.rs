//! The read-only backends, driven by builds made with the archive crate's
//! writers.

use std::{
	collections::BTreeMap,
	io::{Read, Seek, SeekFrom},
	path::Path,
	sync::Arc,
};

use zenith_archive::{ArchiveWriter, FolderIndexEntry, FolderWriter, WriterOptions};
use zenith_error::ZenithError;
use zenith_filesystem::{
	BackendKind, CacheConfig, CacheSettings, EncryptionKey, EntryKind, Manifest, MemDir, OpenOptions, Project,
	WatchConfig,
};

use crate::common::{TestResult, z};

fn files() -> Result<BTreeMap<String, Vec<u8>>, Box<dyn std::error::Error>> {
	let mut files = BTreeMap::new();
	files.insert(
		"Game.zenithproject".to_string(),
		Manifest::new("Game")?.to_toml()?.into_bytes(),
	);
	files.insert("Assets/Worlds/A.zenith".to_string(), b"world a".to_vec());
	files.insert(
		"Assets/tex/a.png".to_string(),
		(0..20_000u32).map(|i| u8::try_from(i % 251).unwrap_or(0)).collect(),
	);
	files.insert("Packages/p.txt".to_string(), b"package".to_vec());
	files.insert(
		"Settings/CacheConfig.toml".to_string(),
		b"max_file_size = 321\n".to_vec(),
	);
	Ok(files)
}

fn opts(key: Option<&EncryptionKey>) -> WriterOptions {
	WriterOptions {
		compress: true,
		compression_level: 3,
		chunk_size_log2: 12,
		key: key.cloned(),
		threads: 1,
	}
}

fn archive_bytes(
	files: &BTreeMap<String, Vec<u8>>,
	key: Option<&EncryptionKey>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
	let mut writer = ArchiveWriter::new(Vec::new(), &opts(key))?;
	for (path, data) in files {
		writer.add_bytes(path, data)?;
	}
	Ok(writer.finish()?)
}

/// Writes a folder build into `dir` the way a build tool would.
fn write_folder(dir: &Path, files: &BTreeMap<String, Vec<u8>>, key: Option<&EncryptionKey>) -> TestResult {
	let mut writer = FolderWriter::new(&opts(key))?;
	let mut index = Vec::new();
	for (path, data) in files {
		let name = writer.data_file_name(path)?;
		let target = dir.join(&name);
		if let Some(parent) = target.parent() {
			std::fs::create_dir_all(parent)?;
		}
		let info = writer.write_data_file(std::fs::File::create(&target)?, path, &mut data.as_slice(), |_| [0; 32])?;
		index.push(FolderIndexEntry {
			path: path.clone(),
			size: info.size,
			data_len: info.data_len,
			content_hash: [0; 32],
			compression_level: 3,
			chunk_size_log2: 12,
		});
	}
	std::fs::write(dir.join("build.index"), writer.encode_index(&index)?)?;
	Ok(())
}

fn mem_dir(
	files: &BTreeMap<String, Vec<u8>>,
	key: Option<&EncryptionKey>,
) -> Result<MemDir, Box<dyn std::error::Error>> {
	let mut writer = FolderWriter::new(&opts(key))?;
	let mut dir = MemDir::new();
	let mut index = Vec::new();
	for (path, data) in files {
		let mut bytes = Vec::new();
		let info = writer.write_data_file(&mut bytes, path, &mut data.as_slice(), |_| [0; 32])?;
		dir.insert(writer.data_file_name(path)?, bytes);
		index.push(FolderIndexEntry {
			path: path.clone(),
			size: info.size,
			data_len: info.data_len,
			content_hash: [0; 32],
			compression_level: 3,
			chunk_size_log2: 12,
		});
	}
	dir.insert("build.index", writer.encode_index(&index)?);
	Ok(dir)
}

/// Every way of opening a build must behave the same.
fn check_reads(project: &Project, expected: BackendKind) -> TestResult {
	let files = files()?;
	assert_eq!(project.kind(), expected, "kind");
	assert_eq!(project.manifest().name, "Game", "manifest read from the build");

	assert_eq!(
		project.read_string(&z("Worlds://A.zenith")?)?,
		"world a",
		"ZPath resolves like on a project folder"
	);
	assert_eq!(
		project.read_string(&z("Assets://Worlds/A.zenith")?)?,
		"world a",
		"through Assets:// as well"
	);
	assert_eq!(
		&*project.read_bytes(&z("Assets://tex/a.png")?)?,
		files["Assets/tex/a.png"].as_slice(),
		"multi-chunk file"
	);
	assert_eq!(
		&*project.read_bytes(&z("Assets://tex/a.png")?)?,
		files["Assets/tex/a.png"].as_slice(),
		"and again, through the cache"
	);

	let listing: Vec<_> = project
		.list(&z("Assets://")?)?
		.into_iter()
		.map(|e| (e.name, e.kind))
		.collect();
	assert_eq!(
		listing,
		vec![
			("Worlds".to_string(), EntryKind::Dir),
			("tex".to_string(), EntryKind::Dir)
		],
		"folders are derived from the file list"
	);
	let meta = project.metadata(&z("Assets://tex/a.png")?)?;
	assert_eq!((meta.kind, meta.size), (EntryKind::File, 20_000), "file metadata");
	assert_eq!(
		project.metadata(&z("Assets://tex")?)?.kind,
		EntryKind::Dir,
		"folder metadata"
	);
	assert!(project.exists(&z("Packages://p.txt")?)?, "exists");
	assert!(!project.exists(&z("Assets://nothing")?)?, "does not exist");
	assert!(
		matches!(
			project.read_bytes(&z("Assets://nothing")?),
			Err(ZenithError::ProjectNotFound(_))
		),
		"missing file"
	);
	assert!(
		matches!(
			project.read_bytes(&z("Assets://tex")?),
			Err(ZenithError::ProjectWrongKind(_))
		),
		"reading a folder"
	);
	assert!(
		!project.is_symlink(&z("Assets://tex/a.png")?)?,
		"builds have no symlinks"
	);

	let mut reader = project.open_read(&z("Assets://tex/a.png")?)?;
	reader.seek(SeekFrom::Start(5000))?;
	let mut buf = [0u8; 100];
	reader.read_exact(&mut buf)?;
	assert_eq!(
		&buf[..],
		&files["Assets/tex/a.png"][5000..5100],
		"streaming read with seek"
	);

	assert_eq!(
		project.cache_settings().max_file_size,
		321,
		"cache settings come from Settings/CacheConfig.toml inside the build"
	);
	project.register_scheme("tex", "Assets/tex")?;
	assert_eq!(
		project.metadata(&z("tex://a.png")?)?.size,
		20_000,
		"custom schemes work in builds"
	);
	Ok(())
}

fn check_read_only(project: &Project) -> TestResult {
	let a = z("Assets://x")?;
	assert!(matches!(project.write(&a, b"x"), Err(ZenithError::ReadOnly)), "write");
	assert!(
		matches!(project.create_dir(&a), Err(ZenithError::ReadOnly)),
		"create_dir"
	);
	assert!(matches!(project.remove(&a), Err(ZenithError::ReadOnly)), "remove");
	assert!(
		matches!(project.copy(&a, &z("Assets://y")?), Err(ZenithError::ReadOnly)),
		"copy"
	);
	assert!(
		matches!(project.move_to(&a, &z("Assets://y")?), Err(ZenithError::ReadOnly)),
		"move"
	);
	assert!(
		matches!(project.save_cache_config(), Err(ZenithError::ReadOnly)),
		"save_cache_config"
	);

	assert!(
		matches!(project.trash_list(), Err(ZenithError::Unsupported(_))),
		"trash_list"
	);
	assert!(
		matches!(project.purge_all(), Err(ZenithError::Unsupported(_))),
		"purge_all"
	);
	assert!(
		matches!(project.restore_last(), Err(ZenithError::Unsupported(_))),
		"restore_last"
	);
	assert!(
		matches!(project.sync_gitignore(), Err(ZenithError::Unsupported(_))),
		"sync_gitignore"
	);
	assert!(
		matches!(project.ensure_ignored_dir("x"), Err(ZenithError::Unsupported(_))),
		"ensure_ignored_dir"
	);
	assert!(
		matches!(
			project.watch(&WatchConfig::File).map(|_| ()),
			Err(ZenithError::Unsupported(_))
		),
		"watch"
	);
	Ok(())
}

#[test]
fn archive_on_disk() -> TestResult {
	let tmp = tempfile::tempdir()?;
	let path = tmp.path().join("game.zenithbuild");
	std::fs::write(&path, archive_bytes(&files()?, None)?)?;
	let project = Project::open(&path)?;
	check_reads(&project, BackendKind::BuildArchive)?;
	check_read_only(&project)
}

#[test]
fn archive_from_bytes_as_on_the_web() -> TestResult {
	let bytes: Arc<[u8]> = Arc::from(archive_bytes(&files()?, None)?);
	let project = Project::open_archive_bytes(bytes, &OpenOptions::new())?;
	check_reads(&project, BackendKind::BuildArchive)?;
	check_read_only(&project)
}

#[test]
fn folder_on_disk() -> TestResult {
	let tmp = tempfile::tempdir()?;
	write_folder(tmp.path(), &files()?, None)?;
	let project = Project::open(tmp.path())?;
	check_reads(&project, BackendKind::BuildFs)?;
	check_read_only(&project)
}

#[test]
fn folder_from_a_custom_source() -> TestResult {
	let project = Project::open_build_dir(mem_dir(&files()?, None)?, &OpenOptions::new())?;
	check_reads(&project, BackendKind::BuildFs)?;
	check_read_only(&project)
}

#[test]
fn encrypted_builds_need_the_right_key() -> TestResult {
	let key = EncryptionKey::generate()?;
	let other = EncryptionKey::generate()?;
	let tmp = tempfile::tempdir()?;
	let archive = tmp.path().join("game.zenithbuild");
	std::fs::write(&archive, archive_bytes(&files()?, Some(&key))?)?;
	let folder = tmp.path().join("folder");
	std::fs::create_dir(&folder)?;
	write_folder(&folder, &files()?, Some(&key))?;

	for path in [&archive, &folder] {
		assert!(
			matches!(Project::open(path), Err(ZenithError::ArchiveEncryptedNoKey)),
			"no key: {}",
			path.display()
		);
		let wrong = OpenOptions::new().encryption_key(other.clone()).open(path);
		assert!(
			matches!(wrong, Err(ZenithError::ArchiveWrongKey)),
			"wrong key: {}",
			path.display()
		);
		let project = OpenOptions::new().encryption_key(key.clone()).open(path)?;
		assert_eq!(
			project.read_string(&z("Worlds://A.zenith")?)?,
			"world a",
			"right key: {}",
			path.display()
		);
	}
	let bytes: Arc<[u8]> = Arc::from(archive_bytes(&files()?, Some(&key))?);
	assert!(
		Project::open_archive_bytes(bytes.clone(), &OpenOptions::new()).is_err(),
		"bytes without a key"
	);
	assert!(
		Project::open_archive_bytes(bytes, &OpenOptions::new().encryption_key(key)).is_ok(),
		"bytes with the key"
	);
	Ok(())
}

#[test]
fn a_key_nobody_needs_is_ignored_by_project() -> TestResult {
	let key = EncryptionKey::generate()?;
	let tmp = tempfile::tempdir()?;
	let archive = tmp.path().join("game.zenithbuild");
	std::fs::write(&archive, archive_bytes(&files()?, None)?)?;
	let folder = tmp.path().join("folder");
	std::fs::create_dir(&folder)?;
	write_folder(&folder, &files()?, None)?;

	for path in [&archive, &folder] {
		let project = OpenOptions::new().encryption_key(key.clone()).open(path)?;
		assert_eq!(
			project.read_string(&z("Packages://p.txt")?)?,
			"package",
			"{} opens although a key was given",
			path.display()
		);
	}
	// The archive reader itself stays strict about it.
	let strict = zenith_archive::ArchiveReader::open(Arc::<[u8]>::from(archive_bytes(&files()?, None)?), Some(&key));
	assert!(
		matches!(strict.map(|_| ()), Err(ZenithError::ArchiveNotEncrypted)),
		"ArchiveReader still refuses an unneeded key"
	);
	Ok(())
}

#[test]
fn damaged_and_foreign_files_are_errors() -> TestResult {
	let tmp = tempfile::tempdir()?;
	let good = archive_bytes(&files()?, None)?;

	let truncated = tmp.path().join("truncated.zenithbuild");
	std::fs::write(&truncated, &good[..good.len() / 2])?;
	let err = Project::open(&truncated).err().ok_or("expected an error")?;
	assert!(
		!matches!(err, ZenithError::UnknownProjectFormat(_)),
		"a damaged archive is damaged, not unknown: {err}"
	);

	let mut flipped = good;
	let middle = flipped.len() / 2;
	flipped[middle] ^= 1;
	let flipped_path = tmp.path().join("flipped.zenithbuild");
	std::fs::write(&flipped_path, flipped)?;
	let noticed = Project::open(&flipped_path).map_or(true, |project| {
		["Assets://Worlds/A.zenith", "Assets://tex/a.png", "Packages://p.txt"]
			.iter()
			.any(|p| z(p).is_ok_and(|p| project.read_bytes(&p).is_err()))
	});
	assert!(
		noticed,
		"a flipped byte is noticed when the build is opened or its data is read"
	);

	let foreign = tmp.path().join("foreign.bin");
	std::fs::write(&foreign, vec![7u8; 4096])?;
	assert!(
		matches!(Project::open(&foreign), Err(ZenithError::UnknownProjectFormat(_))),
		"a file without the archive magic"
	);
	Ok(())
}

#[test]
fn the_manifest_must_match_its_file_name() -> TestResult {
	let mut files = files()?;
	let manifest = files.remove("Game.zenithproject").ok_or("manifest")?;
	files.insert("Other.zenithproject".to_string(), manifest);
	let bytes: Arc<[u8]> = Arc::from(archive_bytes(&files, None)?);
	assert!(
		matches!(
			Project::open_archive_bytes(bytes, &OpenOptions::new()),
			Err(ZenithError::InvalidProject(_))
		),
		"name mismatch"
	);
	Ok(())
}

#[test]
fn manual_cache_config_wins_in_builds_too() -> TestResult {
	let bytes: Arc<[u8]> = Arc::from(archive_bytes(&files()?, None)?);
	let settings = CacheSettings {
		max_file_size: 5,
		..CacheSettings::default()
	};
	let project = Project::open_archive_bytes(
		bytes,
		&OpenOptions::new().cache_config(CacheConfig::Manual(settings.clone())),
	)?;
	assert_eq!(
		project.cache_settings(),
		&settings,
		"Manual overrides the file inside the build"
	);
	Ok(())
}

#[test]
fn a_build_cannot_make_the_reader_load_more_than_the_limit() -> TestResult {
	let bytes = Arc::<[u8]>::from(archive_bytes(&files()?, None)?);
	let project = Project::open_archive_bytes(bytes, &OpenOptions::new().max_read_size(100))?;
	assert_eq!(
		&*project.read_bytes(&z("Packages://p.txt")?)?,
		b"package",
		"a file under the limit loads"
	);
	assert!(
		matches!(
			project.read_bytes(&z("Assets://tex/a.png")?),
			Err(ZenithError::FileTooLarge(_))
		),
		"the declared size of the 20 000 byte file is over the limit, so nothing is decompressed"
	);
	let mut stream = project.open_read(&z("Assets://tex/a.png")?)?;
	let mut data = Vec::new();
	stream.read_to_end(&mut data)?;
	assert_eq!(data.len(), 20_000, "streaming the same file is not limited");
	Ok(())
}

#[test]
fn a_build_can_be_pinned_to_its_id_and_an_older_release_is_refused() -> TestResult {
	let old_files = files()?;
	let mut new_files = files()?;
	new_files.insert("Packages/p.txt".to_string(), b"another package".to_vec());

	// Both ways to open a build, with and without a key.
	for key in [None, Some(EncryptionKey::generate()?)] {
		let old_archive = Arc::<[u8]>::from(archive_bytes(&old_files, key.as_ref())?);
		let new_archive = Arc::<[u8]>::from(archive_bytes(&new_files, key.as_ref())?);
		let options = |id| {
			let o = OpenOptions::new();
			let o = match &key {
				Some(k) => o.encryption_key(k.clone()),
				None => o,
			};
			match id {
				Some(id) => o.expected_build_id(id),
				None => o,
			}
		};
		let id_of = |bytes: &Arc<[u8]>| -> Result<_, Box<dyn std::error::Error>> {
			Ok(Project::open_archive_bytes(bytes.clone(), &options(None))?
				.build_id()
				.ok_or("a build has an id")?)
		};
		let (old_id, new_id) = (id_of(&old_archive)?, id_of(&new_archive)?);
		assert_ne!(old_id, new_id, "two releases, two ids");

		assert!(
			Project::open_archive_bytes(new_archive.clone(), &options(Some(new_id))).is_ok(),
			"the build that was expected"
		);
		let err = Project::open_archive_bytes(old_archive, &options(Some(new_id)))
			.err()
			.ok_or("the old release was accepted")?;
		assert!(
			matches!(&err, ZenithError::BuildIdMismatch(expected, actual) if *expected == new_id.to_hex() && *actual == old_id.to_hex()),
			"a complete, valid, but older build: {err:?}"
		);

		// A folder build, from files in memory.
		let old_dir = mem_dir(&old_files, key.as_ref())?;
		let new_dir = mem_dir(&new_files, key.as_ref())?;
		let folder_new_id = Project::open_build_dir(mem_dir(&new_files, key.as_ref())?, &options(None))?
			.build_id()
			.ok_or("a folder build has an id")?;
		assert!(
			Project::open_build_dir(new_dir, &options(Some(folder_new_id))).is_ok(),
			"the expected folder build"
		);
		assert!(
			matches!(
				Project::open_build_dir(old_dir, &options(Some(folder_new_id))).err(),
				Some(ZenithError::BuildIdMismatch(..))
			),
			"an older folder build is refused"
		);
		assert_ne!(
			folder_new_id, new_id,
			"a folder build and an archive are different builds"
		);
	}
	Ok(())
}

#[test]
fn a_project_folder_has_no_build_id_and_ignores_the_pin() -> TestResult {
	let (_tmp, root, project) = crate::common::new_project("Game")?;
	assert!(project.build_id().is_none(), "only builds have ids");
	drop(project);
	let pinned = OpenOptions::new().expected_build_id(zenith_filesystem::BuildId::from_bytes([7; 32]));
	assert!(pinned.open(&root).is_ok(), "the pin is for builds");
	Ok(())
}
