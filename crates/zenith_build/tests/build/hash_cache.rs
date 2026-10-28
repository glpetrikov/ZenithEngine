//! The hash cache of folder builds: when a file is not read again, and when it
//! must be.

use std::{
	fs::File,
	path::{Path, PathBuf},
	time::{Duration, SystemTime},
};

use zenith_build::{BuildFormat, BuildReport, Builder};
use zenith_error::ZenithError;

use crate::common::{TestResult, standard_project, write};

fn build(root: &Path, out: &Path, cache: bool) -> Result<BuildReport, ZenithError> {
	Builder::new()
		.path(root)
		.output(out)
		.format(BuildFormat::Fs)
		.zstd_compression(3)
		.hash_cache(cache)
		.build()
}

fn set_mtime(path: &Path, time: SystemTime) -> std::io::Result<()> {
	File::options().write(true).open(path)?.set_modified(time)
}

fn mtime(path: &Path) -> std::io::Result<SystemTime> {
	std::fs::metadata(path)?.modified()
}

/// Every file that goes into a build of the standard project.
fn project_files(root: &Path) -> Vec<PathBuf> {
	let mut out = vec![root.join("Game.zenithproject")];
	let mut stack: Vec<PathBuf> = ["Assets", "Settings", "Packages"]
		.iter()
		.map(|d| root.join(d))
		.collect();
	while let Some(dir) = stack.pop() {
		for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
			let path = entry.path();
			if path.is_dir() {
				stack.push(path);
			} else {
				out.push(path);
			}
		}
	}
	out
}

fn backdate(root: &Path) -> std::io::Result<SystemTime> {
	let long_ago = SystemTime::now() - Duration::from_secs(3600);
	for file in project_files(root) {
		set_mtime(&file, long_ago)?;
	}
	Ok(long_ago)
}

#[test]
fn an_unchanged_file_is_not_read_again_and_the_cache_can_be_turned_off() -> TestResult {
	let (tmp, root) = standard_project()?;
	let long_ago = backdate(&root)?;
	let out = tmp.path().join("folder");
	let first = build(&root, &out, true)?;
	assert_eq!(first.compressed, first.entries, "a first build compresses everything");
	assert!(
		root.join("Temp/Build/hash-cache.txt").is_file(),
		"the cache is kept in Temp/Build"
	);

	// The same length and the old modification time: only the content differs.
	let file = root.join("Packages/p.txt");
	std::fs::write(&file, b"pockage")?;
	set_mtime(&file, long_ago)?;
	let trusting = build(&root, &out, true)?;
	assert_eq!(
		(trusting.compressed, trusting.reused),
		(0, trusting.entries),
		"the cache trusts size and modification time, as make does"
	);

	let careful = build(&root, &out, false)?;
	assert_eq!(
		(careful.compressed, careful.reused),
		(1, careful.entries - 1),
		"with the cache off the file is read and the change is found"
	);
	Ok(())
}

#[test]
fn a_new_modification_time_or_size_makes_the_file_read_again() -> TestResult {
	let (tmp, root) = standard_project()?;
	let long_ago = backdate(&root)?;
	let out = tmp.path().join("folder");
	build(&root, &out, true)?;

	let file = root.join("Packages/p.txt");
	std::fs::write(&file, b"pockage")?;
	set_mtime(&file, long_ago + Duration::from_secs(10))?;
	let report = build(&root, &out, true)?;
	assert_eq!(
		(report.compressed, report.reused),
		(1, report.entries - 1),
		"another modification time"
	);

	write(&root, "Settings/Game.toml", b"a = 1000\n")?;
	set_mtime(&root.join("Settings/Game.toml"), long_ago)?;
	let report = build(&root, &out, true)?;
	assert_eq!(report.compressed, 1, "another size, even with the old time");
	Ok(())
}

#[test]
fn a_file_that_was_still_changing_when_it_was_recorded_is_not_trusted() -> TestResult {
	let (tmp, root) = standard_project()?;
	// A modification time in the future can never be older than the moment the
	// hash is recorded, which is what a write within the same clock step looks
	// like.
	let future = SystemTime::now() + Duration::from_secs(3600);
	let file = root.join("Packages/p.txt");
	set_mtime(&file, future)?;
	let out = tmp.path().join("folder");
	build(&root, &out, true)?;

	std::fs::write(&file, b"pockage")?;
	set_mtime(&file, future)?;
	assert_eq!(mtime(&file)?, future, "the time was put back exactly");
	let report = build(&root, &out, true)?;
	assert_eq!(
		report.compressed, 1,
		"the change is found although size and time are the same"
	);
	Ok(())
}

#[test]
fn files_that_left_the_build_leave_the_cache() -> TestResult {
	let (tmp, root) = standard_project()?;
	backdate(&root)?;
	let out = tmp.path().join("folder");
	build(&root, &out, true)?;
	let cache = root.join("Temp/Build/hash-cache.txt");
	let text = std::fs::read_to_string(&cache)?;
	assert!(text.contains("Packages/p.txt"), "a build file is in the cache: {text}");
	assert!(!text.contains("Temp/"), "nothing from Temp");

	std::fs::remove_file(root.join("Packages/p.txt"))?;
	build(&root, &out, true)?;
	assert!(
		!std::fs::read_to_string(&cache)?.contains("Packages/p.txt"),
		"a deleted file is forgotten"
	);
	assert!(
		!out.join("Temp").exists() && !out.join("hash-cache.txt").exists(),
		"and the cache is never part of the build"
	);
	Ok(())
}

#[test]
fn a_damaged_cache_is_the_same_as_none() -> TestResult {
	let (tmp, root) = standard_project()?;
	backdate(&root)?;
	let out = tmp.path().join("folder");
	build(&root, &out, true)?;
	std::fs::write(
		root.join("Temp/Build/hash-cache.txt"),
		"zenith-hash-cache 1\nnot a record\n\u{0}\u{1}",
	)?;
	let report = build(&root, &out, true)?;
	assert_eq!(
		(report.compressed, report.reused),
		(0, report.entries),
		"the files are read, found unchanged, and reused"
	);
	std::fs::write(root.join("Temp/Build/hash-cache.txt"), b"\xff\xfe binary")?;
	assert!(build(&root, &out, true).is_ok(), "even binary garbage");
	Ok(())
}
