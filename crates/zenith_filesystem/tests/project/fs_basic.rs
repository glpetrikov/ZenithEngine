use std::io::{Read, Seek, SeekFrom};

use zenith_error::ZenithError;
use zenith_filesystem::{BackendKind, CacheConfig, CacheSettings, EntryKind, OpenOptions, Project};

use crate::common::{TestResult, copy_tree, names, new_project, z};

#[test]
fn create_makes_the_whole_layout() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	assert_eq!(project.kind(), BackendKind::Fs, "kind");
	assert_eq!(
		names(&root),
		vec![
			".gitignore",
			".v1",
			"Assets",
			"Game.zenithproject",
			"Packages",
			"Settings",
			"Temp",
			"Trash"
		],
		"top level (no Build/ at creation)"
	);
	assert_eq!(
		names(&root.join("Assets")),
		vec!["Worlds"],
		"Assets holds only Worlds; folders like Scripts are the user's to make"
	);
	assert_eq!(
		std::fs::read_to_string(root.join(".gitignore"))?,
		"Temp/\nTrash/\n.zenitheditor-backup/\n*.zenith-tmp\n",
		"gitignore content"
	);
	assert_eq!(
		std::fs::metadata(root.join(".v1"))?.len(),
		0,
		".v1 is an empty flag file"
	);

	let manifest = project.manifest();
	assert_eq!(manifest.name, "Game", "manifest name");
	assert_eq!(manifest.version, 1, "manifest format version is separate from .v1");
	assert_eq!(manifest.project_version.to_string(), "0.1.0", "project version");
	assert!(
		manifest.engine_version.to_string().starts_with('^'),
		"engine requirement is a caret range"
	);
	assert!(project.warnings().is_empty(), "a new project has no warnings");
	Ok(())
}

#[test]
fn create_refuses_bad_names_and_used_folders() -> TestResult {
	let tmp = tempfile::tempdir()?;
	assert!(
		matches!(
			Project::create(tmp.path().join(".hidden")),
			Err(ZenithError::BadProjectName(_))
		),
		"bad name"
	);
	let busy = tmp.path().join("Busy");
	std::fs::create_dir(&busy)?;
	std::fs::write(busy.join("x"), b"1")?;
	assert!(
		matches!(Project::create(&busy), Err(ZenithError::ProjectAlreadyExists(_))),
		"non-empty folder"
	);
	let empty = tmp.path().join("Empty");
	std::fs::create_dir(&empty)?;
	assert!(Project::create(&empty).is_ok(), "an empty folder is fine");
	Ok(())
}

#[test]
fn opens_the_sandbox_project_made_by_the_old_version() -> TestResult {
	let sandbox = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Sandbox/Test");
	let tmp = tempfile::tempdir()?;
	let copy = tmp.path().join("Test");
	copy_tree(&sandbox, &copy)?;
	let project = Project::open(&copy)?;
	assert_eq!(project.manifest().name, "Test", "name from the old manifest");
	assert_eq!(project.manifest().version, 1, "old manifest format");
	assert!(
		project.exists(&z("Worlds://Test.zenith")?)?,
		"the old world file is visible through Worlds://"
	);
	assert!(
		project.exists(&z("Assets://Worlds/Test.zenith")?)?,
		"and through Assets://"
	);
	Ok(())
}

#[test]
fn open_rejects_things_that_are_not_projects() -> TestResult {
	let tmp = tempfile::tempdir()?;

	let empty = tmp.path().join("Empty");
	std::fs::create_dir(&empty)?;
	assert!(
		matches!(Project::open(&empty), Err(ZenithError::UnknownProjectFormat(_))),
		"an empty folder"
	);

	let future = tmp.path().join("Future");
	std::fs::create_dir(&future)?;
	std::fs::write(future.join(".v2"), b"")?;
	assert!(
		matches!(Project::open(&future), Err(ZenithError::UnsupportedProjectVersion)),
		"unknown structure flag"
	);

	let both = tmp.path().join("Both");
	std::fs::create_dir(&both)?;
	std::fs::write(both.join(".v1"), b"")?;
	std::fs::write(both.join("build.index"), b"x")?;
	assert!(
		matches!(Project::open(&both), Err(ZenithError::UnknownProjectFormat(_))),
		"both a flag and an index"
	);

	let text = tmp.path().join("notes.txt");
	std::fs::write(&text, b"hello")?;
	assert!(
		matches!(Project::open(&text), Err(ZenithError::UnknownProjectFormat(_))),
		"an ordinary file"
	);

	assert!(
		matches!(
			Project::open(tmp.path().join("missing")),
			Err(ZenithError::InvalidProjectPath(_))
		),
		"missing path"
	);

	// A flag but no manifest, then a manifest that names another project.
	let no_manifest = tmp.path().join("NoManifest");
	std::fs::create_dir(&no_manifest)?;
	std::fs::write(no_manifest.join(".v1"), b"")?;
	assert!(
		matches!(Project::open(&no_manifest), Err(ZenithError::InvalidProject(_))),
		"no manifest"
	);

	let wrong = tmp.path().join("Wrong");
	std::fs::create_dir(&wrong)?;
	std::fs::write(wrong.join(".v1"), b"")?;
	std::fs::write(
		wrong.join("Wrong.zenithproject"),
		"[project]\nname = \"Other\"\nproject_version = \"0.1.0\"\nengine_version = \"^0.1.0\"\nversion = 1\n",
	)?;
	assert!(
		matches!(Project::open(&wrong), Err(ZenithError::InvalidProject(_))),
		"manifest name differs from the folder"
	);
	Ok(())
}

#[test]
fn files_roundtrip_through_schemes() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://textures/a.txt")?, b"hello")?;
	assert_eq!(
		std::fs::read(root.join("Assets/textures/a.txt"))?,
		b"hello",
		"parents are created"
	);
	assert_eq!(
		&*project.read_bytes(&z("Assets://textures/a.txt")?)?,
		b"hello",
		"read_bytes"
	);
	assert_eq!(
		project.read_string(&z("Assets://textures/a.txt")?)?,
		"hello",
		"read_string"
	);

	project.write(&z("Worlds://A.zenith")?, b"world")?;
	assert_eq!(
		&*project.read_bytes(&z("Assets://Worlds/A.zenith")?)?,
		b"world",
		"Worlds:// and Assets://Worlds are the same place"
	);

	let meta = project.metadata(&z("Assets://textures/a.txt")?)?;
	assert_eq!((meta.kind, meta.size), (EntryKind::File, 5), "file metadata");
	assert_eq!(
		project.metadata(&z("Assets://textures")?)?.kind,
		EntryKind::Dir,
		"folder metadata"
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
			("textures".to_string(), EntryKind::Dir)
		],
		"sorted listing"
	);

	assert!(project.exists(&z("Assets://textures/a.txt")?)?, "exists");
	assert!(!project.exists(&z("Assets://nothing")?)?, "does not exist");
	assert!(
		matches!(
			project.metadata(&z("Assets://nothing")?),
			Err(ZenithError::ProjectNotFound(_))
		),
		"missing metadata"
	);
	assert!(
		matches!(
			project.read_bytes(&z("Assets://nothing")?),
			Err(ZenithError::ProjectNotFound(_))
		),
		"missing file"
	);
	assert!(
		matches!(
			project.read_bytes(&z("Assets://textures")?),
			Err(ZenithError::ProjectWrongKind(_))
		),
		"reading a folder"
	);
	assert!(
		matches!(
			project.write(&z("Assets://textures")?, b"x"),
			Err(ZenithError::ProjectWrongKind(_))
		),
		"writing over a folder"
	);
	assert!(
		matches!(
			project.exists(&z("nope://x")?),
			Err(ZenithError::SchemeNotRegistered(_))
		),
		"unknown scheme"
	);

	project.write(&z("Assets://bin")?, &[0xff, 0xfe])?;
	assert!(
		matches!(
			project.read_string(&z("Assets://bin")?),
			Err(ZenithError::InvalidUtf8(_))
		),
		"invalid UTF-8"
	);
	Ok(())
}

#[test]
fn writes_are_atomic_and_leave_no_temp_files() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	let path = z("Assets://data.bin")?;
	project.write(&path, b"first")?;
	project.write(&path, b"second, longer")?;
	assert_eq!(&*project.read_bytes(&path)?, b"second, longer", "replaced");
	assert_eq!(
		names(&root.join("Assets")),
		vec!["Worlds", "data.bin"],
		"no temporary files remain"
	);
	Ok(())
}

#[test]
fn streaming_reads_seek() -> TestResult {
	let (_tmp, _root, project) = new_project("Game")?;
	project.write(&z("Assets://n.txt")?, b"0123456789")?;
	let mut reader = project.open_read(&z("Assets://n.txt")?)?;
	reader.seek(SeekFrom::Start(4))?;
	let mut buf = [0u8; 3];
	reader.read_exact(&mut buf)?;
	assert_eq!(&buf, b"456", "read after seek");
	assert!(
		project.open_read(&z("Assets://Worlds")?).is_err(),
		"a folder cannot be streamed"
	);
	Ok(())
}

#[test]
fn copy_and_move_work_between_schemes() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://dir/a.txt")?, b"a")?;
	project.write(&z("Assets://dir/sub/b.txt")?, b"b")?;

	project.copy(&z("Assets://dir")?, &z("Settings://copy")?)?;
	assert_eq!(
		std::fs::read(root.join("Settings/copy/sub/b.txt"))?,
		b"b",
		"folder copied recursively"
	);
	assert!(root.join("Assets/dir/a.txt").exists(), "the source stays");

	project.copy(&z("Assets://dir/a.txt")?, &z("Worlds://a-copy.txt")?)?;
	assert_eq!(
		std::fs::read(root.join("Assets/Worlds/a-copy.txt"))?,
		b"a",
		"file copied to another scheme"
	);

	assert!(
		matches!(
			project.copy(&z("Assets://dir")?, &z("Settings://copy")?),
			Err(ZenithError::ProjectAlreadyExists(_))
		),
		"no silent overwrite"
	);
	assert!(
		project.copy(&z("Assets://dir")?, &z("Assets://dir/inside")?).is_err(),
		"a folder cannot be copied into itself"
	);

	project.move_to(&z("Assets://dir")?, &z("Packages://moved")?)?;
	assert!(!root.join("Assets/dir").exists(), "the source is gone");
	assert_eq!(
		std::fs::read(root.join("Packages/moved/sub/b.txt"))?,
		b"b",
		"moved with contents"
	);
	assert!(
		matches!(
			project.move_to(&z("Assets://nothing")?, &z("Assets://x")?),
			Err(ZenithError::ProjectNotFound(_))
		),
		"missing source"
	);
	assert!(
		project
			.move_to(&z("Packages://moved")?, &z("Packages://moved/deeper")?)
			.is_err(),
		"no move into itself"
	);

	project.create_dir(&z("Assets://a/b/c")?)?;
	assert!(root.join("Assets/a/b/c").is_dir(), "create_dir makes parents");
	Ok(())
}

#[test]
fn custom_schemes() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.register_scheme("textures", "Assets/Textures")?;
	project.write(&z("textures://a.png")?, b"png")?;
	assert_eq!(
		std::fs::read(root.join("Assets/Textures/a.png"))?,
		b"png",
		"custom scheme resolves"
	);
	assert!(
		matches!(
			project.register_scheme("textures", "X"),
			Err(ZenithError::SchemeAlreadyRegistered(_))
		),
		"duplicate"
	);
	assert!(
		project.register_scheme("evil", "../outside").is_err(),
		"a root that escapes is refused"
	);
	Ok(())
}

#[test]
fn cache_settings_sources() -> TestResult {
	let (tmp, root, project) = new_project("Game")?;
	assert_eq!(
		project.cache_settings(),
		&CacheSettings::default(),
		"defaults without a file"
	);
	assert!(
		!root.join("Settings/CacheConfig.toml").exists(),
		"the file is not created by opening"
	);
	drop(project);

	std::fs::write(root.join("Settings/CacheConfig.toml"), "max_file_size = 123\n")?;
	let project = Project::open(&root)?;
	assert_eq!(
		project.cache_settings().max_file_size,
		123,
		"read from Settings/CacheConfig.toml"
	);
	assert_eq!(
		project.cache_settings().total_budget,
		CacheSettings::default().total_budget,
		"other keys keep defaults"
	);
	drop(project);

	let manual = CacheSettings {
		max_file_size: 7,
		..CacheSettings::default()
	};
	let project = OpenOptions::new()
		.cache_config(CacheConfig::Manual(manual.clone()))
		.open(&root)?;
	assert_eq!(project.cache_settings(), &manual, "Manual wins over the file");
	assert_eq!(
		std::fs::read_to_string(root.join("Settings/CacheConfig.toml"))?,
		"max_file_size = 123\n",
		"and is not written to disk"
	);

	project.save_cache_config()?;
	let saved = CacheSettings::parse(&std::fs::read_to_string(root.join("Settings/CacheConfig.toml"))?)?;
	assert_eq!(saved, manual, "save_cache_config writes the settings in effect");

	std::fs::write(root.join("Settings/CacheConfig.toml"), "max_file_sise = 1\n")?;
	assert!(Project::open(&root).is_err(), "a misspelled key is an error");
	drop(tmp);
	Ok(())
}

#[test]
fn project_is_send_and_sync() {
	fn check<T: Send + Sync>() {}
	check::<Project>();
}
