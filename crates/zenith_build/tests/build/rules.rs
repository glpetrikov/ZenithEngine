use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
};

use zenith_build::{BuildFormat, Builder, ZPath};
use zenith_error::ZenithError;
use zenith_filesystem::{OpenOptions, Project};

use crate::common::{TestResult, archive_contents, archive_paths, key, standard_project, tree_hashes, write};

fn z(text: &str) -> Result<ZPath, Box<dyn std::error::Error>> {
	Ok(ZPath::parse(text)?)
}

fn builder(root: &Path, out: &Path) -> Builder {
	Builder::new().path(root).output(out).zstd_compression(3)
}

#[test]
fn the_base_list_is_the_manifest_assets_settings_and_packages() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("b.zenithbuild");
	builder(&root, &out).build()?;
	assert_eq!(
		archive_paths(&out)?,
		vec![
			"Assets/Scripts/lib.rs",
			"Assets/Worlds/A.zenith",
			"Assets/tex/a.png",
			"Game.zenithproject",
			"Packages/p.txt",
			"Settings/Game.toml",
		],
		"no Temp, Trash, Build, .gitignore, .v1, nor Docs"
	);
	Ok(())
}

#[test]
fn the_git_folder_is_never_walked() -> TestResult {
	let (tmp, root) = standard_project()?;
	write(&root, ".git/config", b"[core]")?;
	write(&root, ".git/objects/ab/cdef", b"object")?;
	let out = tmp.path().join("b.zenithbuild");
	builder(&root, &out).include("**").build()?;
	let paths = archive_paths(&out)?;
	assert!(
		paths.iter().any(|p| p == "Docs/readme.txt"),
		"include still reaches outside the base list: {paths:?}"
	);
	assert!(
		!paths.iter().any(|p| p.starts_with(".git/")),
		"but never into the repository: {paths:?}"
	);
	Ok(())
}

#[test]
fn exclude_removes_and_include_adds_from_anywhere() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("b.zenithbuild");
	builder(&root, &out)
		.exclude("Assets/Scripts/**")
		.exclude("Packages/*")
		.include("Docs/**")
		.include("Temp/*.txt")
		.build()?;
	assert_eq!(
		archive_paths(&out)?,
		vec![
			"Assets/Worlds/A.zenith",
			"Assets/tex/a.png",
			"Docs/readme.txt",
			"Game.zenithproject",
			"Settings/Game.toml",
			"Temp/junk.txt",
		],
		"excluded files are gone; files outside the base list come in with include"
	);
	Ok(())
}

#[test]
fn the_last_matching_step_wins_per_path() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("b.zenithbuild");

	builder(&root, &out)
		.exclude("Assets/**")
		.include("Assets/Worlds/*")
		.build()?;
	let paths = archive_paths(&out)?;
	assert!(
		paths.contains(&"Assets/Worlds/A.zenith".to_string()),
		"include after exclude brings the file back: {paths:?}"
	);
	assert!(
		!paths.contains(&"Assets/tex/a.png".to_string()),
		"the rest stays excluded: {paths:?}"
	);

	builder(&root, &out)
		.include("Assets/Worlds/*")
		.exclude("Assets/**")
		.build()?;
	assert!(
		!archive_paths(&out)?.contains(&"Assets/Worlds/A.zenith".to_string()),
		"exclude after include wins"
	);

	builder(&root, &out)
		.include("Docs/**")
		.exclude("Docs/readme.txt")
		.build()?;
	assert!(
		!archive_paths(&out)?.iter().any(|p| p.starts_with("Docs")),
		"excluded again"
	);

	builder(&root, &out)
		.exclude("Settings/**")
		.add_bytes(z("Settings://Game.toml")?, b"added back".to_vec())
		.build()?;
	assert_eq!(
		archive_contents(&out, None)?
			.get("Settings/Game.toml")
			.map(Vec::as_slice),
		Some(&b"added back"[..]),
		"add after exclude"
	);

	builder(&root, &out)
		.add_bytes(z("Settings://Game.toml")?, b"gone".to_vec())
		.exclude("Settings/Game.toml")
		.build()?;
	assert!(
		!archive_paths(&out)?.contains(&"Settings/Game.toml".to_string()),
		"exclude after add removes the added entry"
	);
	Ok(())
}

#[test]
fn star_does_not_cross_slashes() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("b.zenithbuild");
	builder(&root, &out).exclude("Assets/*").build()?;
	assert_eq!(
		archive_paths(&out)?.iter().filter(|p| p.starts_with("Assets")).count(),
		3,
		"`Assets/*` matches only direct children, and there are none (all files are deeper)"
	);
	builder(&root, &out).exclude("Assets/*/*").build()?;
	assert!(
		!archive_paths(&out)?.iter().any(|p| p.starts_with("Assets")),
		"`Assets/*/*` matches exactly two levels, which is all of them"
	);

	write(&root, "Docs/sub/deep.txt", b"deep")?;
	builder(&root, &out).include("Docs/*").build()?;
	let docs: Vec<String> = archive_paths(&out)?
		.into_iter()
		.filter(|p| p.starts_with("Docs"))
		.collect();
	assert_eq!(docs, vec!["Docs/readme.txt"], "a single `*` stays within one folder");
	Ok(())
}

#[test]
fn add_file_moves_a_project_file_and_add_bytes_replaces_entries() -> TestResult {
	let (tmp, root) = standard_project()?;
	write(&root, "Temp/Build/scripts.wasm", b"\0asm compiled")?;
	let out = tmp.path().join("b.zenithbuild");
	let patched = b"a = 2\nscripts = 'wasm'\n".to_vec();

	// The documented use: scripts are compiled before the build, the source is
	// swapped for the compiled file and the settings are replaced.
	builder(&root, &out)
		.exclude("Assets/Scripts/**")
		.add_file(z("Assets://Scripts/scripts.wasm")?, z("Temp://Build/scripts.wasm")?)
		.add_bytes(z("Settings://Game.toml")?, patched.clone())
		.build()?;
	let contents = archive_contents(&out, None)?;
	assert_eq!(
		contents.get("Assets/Scripts/scripts.wasm").map(Vec::as_slice),
		Some(&b"\0asm compiled"[..]),
		"compiled scripts go where the sources were"
	);
	assert!(
		!contents.contains_key("Assets/Scripts/lib.rs"),
		"source scripts are excluded"
	);
	assert_eq!(
		contents.get("Settings/Game.toml"),
		Some(&patched),
		"an existing entry is replaced by add_bytes"
	);
	assert_eq!(
		std::fs::read(root.join("Settings/Game.toml"))?,
		b"a = 1\n",
		"the project on disk is not changed"
	);
	assert_eq!(
		std::fs::read(root.join("Temp/Build/scripts.wasm"))?,
		b"\0asm compiled",
		"nor is the source of add_file"
	);

	builder(&root, &out)
		.add_file(z("Assets://Worlds/A.zenith")?, z("Packages://p.txt")?)
		.build()?;
	assert_eq!(
		archive_contents(&out, None)?
			.get("Assets/Worlds/A.zenith")
			.map(Vec::as_slice),
		Some(&b"package"[..]),
		"add_file replaces an existing entry"
	);
	builder(&root, &out)
		.add_bytes(z("Worlds://New.zenith")?, b"new".to_vec())
		.build()?;
	assert!(
		archive_contents(&out, None)?.contains_key("Assets/Worlds/New.zenith"),
		"add_bytes creates an entry"
	);
	Ok(())
}

#[test]
fn bad_add_file_and_add_bytes_are_errors() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("b.zenithbuild");
	let missing = builder(&root, &out)
		.add_file(z("Assets://x")?, z("Assets://nothing")?)
		.build();
	assert!(
		matches!(missing, Err(ZenithError::ProjectNotFound(_))),
		"source does not exist"
	);
	let folder = builder(&root, &out)
		.add_file(z("Assets://x")?, z("Assets://tex")?)
		.build();
	assert!(
		matches!(folder, Err(ZenithError::InvalidBuildOption(_))),
		"source is a folder"
	);
	let trash = builder(&root, &out).add_bytes(z("Trash://x")?, b"x".to_vec()).build();
	assert!(
		matches!(trash, Err(ZenithError::InvalidBuildOption(_))),
		"destination in the trash"
	);
	let scheme = builder(&root, &out).add_bytes(z("nope://x")?, b"x".to_vec()).build();
	assert!(
		matches!(scheme, Err(ZenithError::SchemeNotRegistered(_))),
		"unknown scheme"
	);
	assert!(!out.exists(), "nothing was written");
	Ok(())
}

#[test]
fn options_are_validated() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("b.zenithbuild");
	for level in [0, 23, -1, i32::MAX] {
		assert!(
			matches!(
				builder(&root, &out).zstd_compression(level).build(),
				Err(ZenithError::InvalidBuildOption(_))
			),
			"level {level}"
		);
	}
	for level in [1, 22] {
		assert!(
			builder(&root, &out).zstd_compression(level).build().is_ok(),
			"level {level} is allowed"
		);
	}
	assert!(
		matches!(
			Builder::new().output(&out).build(),
			Err(ZenithError::InvalidBuildOption(_))
		),
		"no path"
	);
	assert!(
		matches!(
			builder(&root, &out).exclude("[").build(),
			Err(ZenithError::InvalidBuildOption(_))
		),
		"bad exclude glob"
	);
	assert!(
		matches!(
			builder(&root, &out).include("{a").build(),
			Err(ZenithError::InvalidBuildOption(_))
		),
		"bad include glob"
	);
	assert!(
		Builder::new().path(tmp.path().join("missing")).build().is_err(),
		"missing project"
	);
	assert!(
		builder(&root, &out).exclude("Game.zenithproject").build().is_err(),
		"a build without its manifest is refused"
	);

	// A build is not a project folder.
	let built = tmp.path().join("built.zenithbuild");
	builder(&root, &built).build()?;
	assert!(
		matches!(
			Builder::new().path(&built).build(),
			Err(ZenithError::InvalidBuildOption(_))
		),
		"a build as the project"
	);
	Ok(())
}

#[test]
fn encrypted_builds_open_with_the_key_through_project() -> TestResult {
	let (tmp, root) = standard_project()?;
	let archive = tmp.path().join("e.zenithbuild");
	let folder = tmp.path().join("e-folder");
	builder(&root, &archive).encryption_key(key(3)).build()?;
	builder(&root, &folder)
		.format(BuildFormat::Fs)
		.encryption_key(key(3))
		.build()?;

	for path in [&archive, &folder] {
		assert!(Project::open(path).is_err(), "no key: {}", path.display());
		let project = OpenOptions::new().encryption_key(key(3)).open(path)?;
		assert_eq!(
			project.read_string(&z("Worlds://A.zenith")?)?,
			"world a",
			"{}",
			path.display()
		);
	}
	let names: BTreeMap<String, String> = tree_hashes(&folder);
	assert!(
		names
			.keys()
			.all(|n| n == "build.index" || (n.len() == 33 && n.as_bytes().get(2) == Some(&b'/'))),
		"data files are 2 + 30 hex, no real names: {:?}",
		names.keys().collect::<Vec<_>>()
	);
	Ok(())
}

#[test]
fn an_uncompressed_build_is_bigger_readable_and_opens_the_same() -> TestResult {
	let (tmp, root) = standard_project()?;
	write(
		&root,
		"Assets/readme.txt",
		"Hello, uncompressed world! ".repeat(400).as_bytes(),
	)?;
	for format in [BuildFormat::Archive, BuildFormat::Fs] {
		let packed = tmp.path().join(format!("packed-{format:?}"));
		let stored = tmp.path().join(format!("stored-{format:?}"));
		builder(&root, &packed).format(format).build()?;
		builder(&root, &stored).format(format).compress(false).build()?;

		let size = |path: &Path| -> std::io::Result<u64> {
			if path.is_dir() {
				tree_hashes(path)
					.keys()
					.try_fold(0, |sum, name| std::fs::metadata(path.join(name)).map(|m| sum + m.len()))
			} else {
				std::fs::metadata(path).map(|m| m.len())
			}
		};
		assert!(size(&stored)? > size(&packed)?, "{format:?}: nothing is compressed");

		let project = Project::open(&stored)?;
		assert_eq!(
			project.read_bytes(&z("Assets://readme.txt")?)?,
			"Hello, uncompressed world! ".repeat(400).into_bytes().into(),
			"{format:?}: reads back"
		);
		let needle = b"Hello, uncompressed world!";
		let files: Vec<PathBuf> = if stored.is_dir() {
			tree_hashes(&stored).keys().map(|name| stored.join(name)).collect()
		} else {
			vec![stored.clone()]
		};
		assert!(
			files
				.iter()
				.any(|f| std::fs::read(f).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle))),
			"{format:?}: the text can be found in the build"
		);
	}

	// Switching the mode rebuilds a folder build instead of mixing the two.
	let out = tmp.path().join("switch");
	builder(&root, &out).format(BuildFormat::Fs).build()?;
	let again = builder(&root, &out).format(BuildFormat::Fs).compress(false).build()?;
	assert_eq!(again.reused, 0, "files written another way are written again");
	assert!(Project::open(&out).is_ok(), "and the result opens");

	assert!(
		builder(&root, &tmp.path().join("level.zenithbuild"))
			.zstd_compression(99)
			.compress(false)
			.build()
			.is_ok(),
		"the level does not matter without compression"
	);
	Ok(())
}
