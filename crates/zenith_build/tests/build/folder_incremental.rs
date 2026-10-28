//! Rebuilding into the folder of a previous build: what is reused, what is
//! recompressed, and that the result never differs from a build from scratch.

use std::path::Path;

use zenith_build::{BuildFormat, BuildReport, Builder, EncryptionKey};
use zenith_error::ZenithError;
use zenith_filesystem::Project;

use crate::common::{TestResult, key, random_bytes, standard_project, tree_hashes, write};

fn build_at_level(
	root: &Path,
	out: &Path,
	key: Option<&EncryptionKey>,
	level: i32,
) -> Result<BuildReport, ZenithError> {
	let mut b = Builder::new()
		.path(root)
		.output(out)
		.format(BuildFormat::Fs)
		.zstd_compression(level);
	if let Some(key) = key {
		b = b.encryption_key(key.clone());
	}
	b.build()
}

fn build(root: &Path, out: &Path, key: Option<&EncryptionKey>) -> Result<BuildReport, ZenithError> {
	build_at_level(root, out, key, 3)
}

fn mtimes(dir: &Path) -> std::collections::BTreeMap<String, std::time::SystemTime> {
	let mut out = std::collections::BTreeMap::new();
	let mut stack = vec![dir.to_path_buf()];
	while let Some(current) = stack.pop() {
		for entry in std::fs::read_dir(current).into_iter().flatten().flatten() {
			let path = entry.path();
			if path.is_dir() {
				stack.push(path);
			} else if let (Ok(rel), Ok(time)) = (path.strip_prefix(dir), entry.metadata().and_then(|m| m.modified())) {
				out.insert(rel.to_string_lossy().replace('\\', "/"), time);
			}
		}
	}
	out
}

#[test]
fn rebuilding_an_untouched_project_compresses_nothing() -> TestResult {
	for key in [None, Some(key(4))] {
		let (tmp, root) = standard_project()?;
		let out = tmp.path().join("folder");
		let first = build(&root, &out, key.as_ref())?;
		assert_eq!(
			(first.compressed, first.reused),
			(first.entries, 0),
			"a first build compresses every file"
		);
		let (files, times) = (tree_hashes(&out), mtimes(&out));

		std::thread::sleep(std::time::Duration::from_millis(30));
		let second = build(&root, &out, key.as_ref())?;
		assert_eq!(
			second.compressed,
			0,
			"no file was compressed (encrypted: {})",
			key.is_some()
		);
		assert_eq!(second.reused, second.entries, "every file was taken over");
		assert_eq!(tree_hashes(&out), files, "same content everywhere");
		let after = mtimes(&out);
		for (name, time) in &times {
			if name != "build.index" {
				assert_eq!(after.get(name), Some(time), "{name} was not touched");
			}
		}
	}
	Ok(())
}

#[test]
fn only_changed_content_is_compressed_even_when_the_size_stays() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("folder");
	build(&root, &out, None)?;
	// Same length, different bytes: only the content hash can tell.
	write(&root, "Packages/p.txt", b"pockage")?;
	let report = build(&root, &out, None)?;
	assert_eq!(
		(report.compressed, report.reused),
		(1, report.entries - 1),
		"one file recompressed, the rest reused"
	);
	let fresh = tmp.path().join("fresh");
	build(&root, &fresh, None)?;
	assert_eq!(
		tree_hashes(&out),
		tree_hashes(&fresh),
		"identical to a build from scratch"
	);
	Ok(())
}

#[test]
fn changing_the_compression_level_rebuilds_every_file() -> TestResult {
	for key in [None, Some(key(4))] {
		let (tmp, root) = standard_project()?;
		// Compressible content, so that the levels really produce different
		// bytes.
		write(
			&root,
			"Assets/big.txt",
			&"compressible line of text\n".repeat(20_000).into_bytes(),
		)?;
		let out = tmp.path().join("folder");
		build_at_level(&root, &out, key.as_ref(), 1)?;
		let before = tree_hashes(&out);

		let report = build_at_level(&root, &out, key.as_ref(), 19)?;
		assert_eq!(
			(report.compressed, report.reused),
			(report.entries, 0),
			"every file is compressed again"
		);
		assert_ne!(tree_hashes(&out), before, "and the big file came out differently");

		let fresh = tmp.path().join("fresh");
		build_at_level(&root, &fresh, key.as_ref(), 19)?;
		assert_eq!(
			tree_hashes(&out),
			tree_hashes(&fresh),
			"identical to a build from scratch at the new level"
		);
		let again = build_at_level(&root, &out, key.as_ref(), 19)?;
		assert_eq!(
			again.compressed, 0,
			"and now the new level is the one that is remembered"
		);
	}
	Ok(())
}

#[test]
fn an_incremental_build_equals_a_fresh_one_after_many_changes() -> TestResult {
	for key in [None, Some(key(4))] {
		let (tmp, root) = standard_project()?;
		for n in 0..40u64 {
			let len = usize::try_from(3000 + n * 7)?;
			write(&root, &format!("Assets/many/f{n:02}.bin"), &random_bytes(len, n + 1))?;
		}
		let out = tmp.path().join("folder");
		build(&root, &out, key.as_ref())?;

		write(&root, "Assets/many/f05.bin", b"edited")?;
		std::fs::remove_file(root.join("Assets/many/f06.bin"))?;
		write(&root, "Assets/many/new.bin", &random_bytes(5000, 99))?;
		write(&root, "Settings/Game.toml", b"a = 2\n")?;
		let report = build(&root, &out, key.as_ref())?;
		assert_eq!(
			report.compressed, 3,
			"the edited, the new and the changed settings file"
		);
		assert_eq!(report.reused, report.entries - 3, "everything else was reused");

		let fresh = tmp.path().join("fresh");
		build(&root, &fresh, key.as_ref())?;
		assert_eq!(
			tree_hashes(&out),
			tree_hashes(&fresh),
			"identical to a build from scratch (encrypted: {})",
			key.is_some()
		);
	}
	Ok(())
}

#[test]
fn a_missing_data_file_is_made_again() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("folder");
	build(&root, &out, None)?;
	std::fs::remove_file(out.join("Packages/p.txt"))?;
	let report = build(&root, &out, None)?;
	assert_eq!(report.compressed, 1, "only the missing one");
	let fresh = tmp.path().join("fresh");
	build(&root, &fresh, None)?;
	assert_eq!(
		tree_hashes(&out),
		tree_hashes(&fresh),
		"identical to a build from scratch"
	);
	Ok(())
}

#[cfg(windows)]
#[test]
fn a_failing_build_changes_nothing() -> TestResult {
	use std::os::windows::fs::OpenOptionsExt;

	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("folder");
	build(&root, &out, None)?;
	write(&root, "Assets/Worlds/A.zenith", b"changed so it must be rebuilt")?;
	write(&root, "Settings/Game.toml", b"also changed\n")?;
	let before = tree_hashes(&out);

	// No sharing: nobody else can open the file while this handle lives.
	let guard = std::fs::OpenOptions::new()
		.read(true)
		.share_mode(0)
		.open(root.join("Packages/p.txt"))?;
	let result = build(&root, &out, None);
	drop(guard);

	assert!(result.is_err(), "an unreadable source fails the build");
	assert_eq!(tree_hashes(&out), before, "the old build is intact, index included");
	assert!(
		tree_hashes(&out).keys().all(|n| !n.ends_with(".zenith-tmp")),
		"no temporary files left"
	);
	assert!(build(&root, &out, None).is_ok(), "and the next build works");
	Ok(())
}

#[test]
fn a_damaged_data_file_of_the_previous_build_is_made_again() -> TestResult {
	for key in [None, Some(key(4))] {
		let (tmp, root) = standard_project()?;
		let out = tmp.path().join("folder");
		build(&root, &out, key.as_ref())?;
		let before = tree_hashes(&out);

		// The last byte is part of the footer checksum. The length is
		// unchanged, so only looking at the file can tell.
		let name = before
			.keys()
			.find(|n| *n != "build.index")
			.ok_or("the build has no data file")?
			.clone();
		let file = out.join(&name);
		let mut bytes = std::fs::read(&file)?;
		*bytes.last_mut().ok_or("empty data file")? ^= 0xff;
		std::fs::write(&file, &bytes)?;

		let report = build(&root, &out, key.as_ref())?;
		assert_eq!(
			(report.compressed, report.reused),
			(1, report.entries - 1),
			"only the damaged file is made again (encrypted: {})",
			key.is_some()
		);
		assert_eq!(tree_hashes(&out), before, "and the folder is exactly what it was");
	}
	Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
	std::fs::create_dir_all(to)?;
	for entry in std::fs::read_dir(from)? {
		let entry = entry?;
		let target = to.join(entry.file_name());
		if entry.file_type()?.is_dir() {
			copy_dir(&entry.path(), &target)?;
		} else {
			std::fs::copy(entry.path(), target)?;
		}
	}
	Ok(())
}

/// A data file of the build (any file that is not the index).
fn some_data_file(files: &std::collections::BTreeMap<String, String>) -> Result<String, Box<dyn std::error::Error>> {
	Ok(files
		.keys()
		.find(|name| !name.starts_with("build.index"))
		.ok_or("the build has no data file")?
		.clone())
}

#[test]
fn a_finished_build_leaves_no_index_markers_behind() -> TestResult {
	for key in [None, Some(key(4))] {
		let (tmp, root) = standard_project()?;
		let out = tmp.path().join("folder");
		build(&root, &out, key.as_ref())?;
		// A change, so that the second build goes through the whole commit.
		write(&root, "Packages/p.txt", b"changed")?;
		build(&root, &out, key.as_ref())?;
		let markers: Vec<String> = tree_hashes(&out)
			.into_keys()
			.filter(|n| n.starts_with("build.index"))
			.collect();
		assert_eq!(
			markers,
			vec!["build.index"],
			"only the index is left (encrypted: {})",
			key.is_some()
		);
	}
	Ok(())
}

#[test]
fn an_interrupted_commit_is_visible_and_the_next_build_repairs_it() -> TestResult {
	for key in [None, Some(key(4))] {
		let (tmp, root) = standard_project()?;
		let out = tmp.path().join("folder");
		build(&root, &out, key.as_ref())?;
		let finished = tree_hashes(&out);

		// What an interruption in the middle of a commit leaves: the old index
		// stepped aside, the new one waiting, and a data file that is half
		// replaced.
		std::fs::copy(out.join("build.index"), out.join("build.index.next"))?;
		std::fs::rename(out.join("build.index"), out.join("build.index.prev"))?;
		std::fs::write(out.join(some_data_file(&finished)?), b"half a file")?;

		assert!(
			matches!(Project::open(&out), Err(ZenithError::UnknownProjectFormat(_))),
			"a folder without build.index is plainly not a finished build"
		);
		let report = build(&root, &out, key.as_ref())?;
		assert_eq!(
			(report.compressed, report.reused),
			(report.entries, 0),
			"nothing of an interrupted build is trusted (encrypted: {})",
			key.is_some()
		);
		assert_eq!(
			tree_hashes(&out),
			finished,
			"and the folder is exactly what it should be"
		);
	}
	Ok(())
}

#[test]
fn a_first_build_that_was_interrupted_can_be_built_again() -> TestResult {
	for key in [None, Some(key(4))] {
		let (tmp, root) = standard_project()?;
		let reference = tmp.path().join("reference");
		build(&root, &reference, key.as_ref())?;

		// All data files are there, the index is still waiting under its
		// temporary name.
		let out = tmp.path().join("folder");
		copy_dir(&reference, &out)?;
		std::fs::rename(out.join("build.index"), out.join("build.index.next"))?;
		build(&root, &out, key.as_ref())?;
		assert_eq!(
			tree_hashes(&out),
			tree_hashes(&reference),
			"(encrypted: {})",
			key.is_some()
		);

		// Without the marker the same folder is somebody else's, and untouched.
		let foreign = tmp.path().join("foreign");
		copy_dir(&reference, &foreign)?;
		std::fs::remove_file(foreign.join("build.index"))?;
		let before = tree_hashes(&foreign);
		assert!(
			matches!(build(&root, &foreign, key.as_ref()), Err(ZenithError::NotABuildDir(_))),
			"a folder with files and no build.index at all is not ours"
		);
		assert_eq!(tree_hashes(&foreign), before, "and nothing was changed");
	}
	Ok(())
}

#[test]
fn leftovers_next_to_a_good_index_are_cleaned_up() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("folder");
	build(&root, &out, None)?;
	let finished = tree_hashes(&out);
	// A crash after the new index was installed, before the old one was
	// deleted, or before anything was replaced.
	std::fs::write(out.join("build.index.prev"), b"old")?;
	std::fs::write(out.join("build.index.next"), b"new")?;
	let report = build(&root, &out, None)?;
	assert_eq!(report.compressed, 0, "the good index is the previous build");
	assert_eq!(tree_hashes(&out), finished, "and the leftovers are gone");
	Ok(())
}

#[test]
fn an_entry_cannot_be_named_like_an_index_marker() -> TestResult {
	let (tmp, root) = standard_project()?;
	write(&root, "build.index.next", b"x")?;
	let result = Builder::new()
		.path(&root)
		.output(tmp.path().join("folder"))
		.format(BuildFormat::Fs)
		.zstd_compression(3)
		.include("build.index.next")
		.build();
	assert!(matches!(result, Err(ZenithError::InvalidBuildOutput(_))), "{result:?}");
	Ok(())
}
