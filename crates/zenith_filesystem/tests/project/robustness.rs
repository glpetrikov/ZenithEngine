//! Cases where an operation or an open must still work, or fail cleanly, when
//! something around the project is not as expected.

use std::{path::Path, sync::Arc};

use semver::VersionReq;
use zenith_archive::{ArchiveWriter, WriterOptions};
use zenith_error::ZenithError;
use zenith_filesystem::{Manifest, OpenOptions, Project, WarningKind};

use crate::common::{TestResult, new_project, z};

#[test]
fn restore_is_not_an_error_when_the_gitignore_is_damaged() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.ensure_ignored_dir(root.join("Out/sub"))?;
	project.register_scheme("outx", "Out")?;
	std::fs::write(root.join("Out/sub/f.txt"), b"x")?;
	let id = project.remove(&z("outx://sub")?)?;

	// The user (or a merge) leaves a half-edited block behind.
	std::fs::write(root.join(".gitignore"), "Temp/\n# zenith:begin\n")?;

	assert_eq!(
		project.restore(&id)?.to_string(),
		"Outx://sub",
		"the restore itself succeeded"
	);
	assert!(root.join("Out/sub/f.txt").is_file(), "the folder is back");
	assert!(
		project.trash_list()?.is_empty(),
		"the trash entry is gone, as after any restore"
	);
	assert!(
		project
			.warnings()
			.iter()
			.any(|w| w.kind == WarningKind::DamagedGitignore),
		"the damaged block is reported as a warning instead"
	);
	assert_eq!(
		std::fs::read_to_string(root.join(".gitignore"))?,
		"Temp/\n# zenith:begin\n",
		"and the file was left alone"
	);
	Ok(())
}

#[test]
fn the_same_warning_is_kept_once() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	std::fs::write(root.join(".gitignore"), "# zenith:begin\n")?;
	project.write(&z("Assets://a.txt")?, b"a")?;
	for n in 0..3 {
		project.move_to(&z("Assets://a.txt")?, &z("Assets://b.txt")?)?;
		project.move_to(&z("Assets://b.txt")?, &z("Assets://a.txt")?)?;
		assert_eq!(
			project
				.warnings()
				.iter()
				.filter(|w| w.kind == WarningKind::DamagedGitignore)
				.count(),
			1,
			"round {n}: every move re-checks the file, but the warning is not repeated"
		);
	}
	Ok(())
}

/// Rewrites the engine requirement of the project at `root`.
fn set_engine_version(root: &Path, requirement: &str) -> TestResult {
	let manifest = root.join("Game.zenithproject");
	let text = std::fs::read_to_string(&manifest)?;
	let from = text
		.lines()
		.find(|l| l.starts_with("engine_version"))
		.ok_or("no engine_version in the manifest")?;
	std::fs::write(
		&manifest,
		text.replace(from, &format!("engine_version = \"{requirement}\"")),
	)?;
	Ok(())
}

#[test]
fn a_manifest_for_another_engine_does_not_open() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	drop(project);
	assert!(Project::open(&root).is_ok(), "a new project matches this engine");

	set_engine_version(&root, "^99.0.0")?;
	let err = Project::open(&root).err().ok_or("it opened")?;
	assert!(
		matches!(&err, ZenithError::EngineVersionMismatch(required, _) if required.contains("99.0.0")),
		"{err:?}"
	);

	let project = OpenOptions::new().ignore_engine_version().open(&root)?;
	assert_eq!(
		project.manifest().name,
		"Game",
		"a tool that converts old projects can still read it"
	);
	Ok(())
}

#[test]
fn a_build_for_another_engine_does_not_open_either() -> TestResult {
	let mut manifest = Manifest::new("Game")?;
	manifest.engine_version = VersionReq::parse("^99.0.0")?;
	let mut writer = ArchiveWriter::new(Vec::new(), &WriterOptions::default())?;
	writer.add_bytes("Game.zenithproject", manifest.to_toml()?.as_bytes())?;
	let bytes = Arc::<[u8]>::from(writer.finish()?);

	let err = Project::open_archive_bytes(bytes.clone(), &OpenOptions::new())
		.err()
		.ok_or("it opened")?;
	assert!(matches!(err, ZenithError::EngineVersionMismatch(..)), "{err:?}");
	assert!(
		Project::open_archive_bytes(bytes, &OpenOptions::new().ignore_engine_version()).is_ok(),
		"unless the caller says it does not matter"
	);
	Ok(())
}

#[test]
fn opening_does_not_edit_the_gitignore() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.ensure_ignored_dir(root.join("Build"))?;
	std::fs::remove_dir(root.join("Build"))?;
	let before = std::fs::read_to_string(root.join(".gitignore"))?;
	assert!(before.contains("/Build/"), "the entry is there: {before:?}");
	drop(project);

	let project = Project::open(&root)?;
	assert_eq!(
		std::fs::read_to_string(root.join(".gitignore"))?,
		before,
		"a stale entry is left alone by open"
	);
	project.sync_gitignore()?;
	assert!(
		!std::fs::read_to_string(root.join(".gitignore"))?.contains("/Build/"),
		"and removed by an explicit sync"
	);
	Ok(())
}

#[test]
fn a_folder_copy_that_cannot_finish_leaves_nothing_behind() -> TestResult {
	let (tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://dir/a.txt")?, b"a")?;
	project.write(&z("Assets://dir/sub/b.txt")?, b"b")?;
	if link_dir(
		&tmp.path().join("Game/Assets/Worlds"),
		&root.join("Assets/dir/sub/link"),
	)
	.is_none()
	{
		eprintln!("SKIPPED: cannot create a link here");
		return Ok(());
	}
	assert!(
		project.copy(&z("Assets://dir")?, &z("Assets://copy")?).is_err(),
		"a folder with a link in it is refused"
	);
	assert!(
		!root.join("Assets/copy").exists(),
		"and the half-made copy was removed again"
	);
	assert!(root.join("Assets/dir/sub/b.txt").is_file(), "the source is untouched");
	Ok(())
}

#[cfg(unix)]
#[test]
fn names_that_are_not_utf8_are_errors_not_gaps() -> TestResult {
	use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://dir/ok.txt")?, b"ok")?;
	std::fs::write(root.join("Assets/dir").join(OsStr::from_bytes(b"bad-\xff.txt")), b"x")?;

	assert_eq!(
		project.list(&z("Assets://dir")?)?.len(),
		1,
		"a listing cannot show it (a ZPath cannot name it)"
	);
	assert!(
		project.copy(&z("Assets://dir")?, &z("Assets://copy")?).is_err(),
		"but a copy does not pretend to be complete"
	);
	assert!(!root.join("Assets/copy").exists(), "and leaves nothing behind");
	assert!(
		project.walk_project(&|_| true).is_err(),
		"and a walk for a build fails instead of dropping the file"
	);
	Ok(())
}

#[test]
fn reading_into_memory_has_a_limit() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://big.bin")?, &[1u8; 100])?;
	project.write(&z("Assets://small.bin")?, &[1u8; 4])?;
	drop(project);

	let project = OpenOptions::new().max_read_size(10).open(&root)?;
	assert_eq!(
		project.read_bytes(&z("Assets://small.bin")?)?.len(),
		4,
		"under the limit"
	);
	assert!(
		matches!(
			project.read_bytes(&z("Assets://big.bin")?),
			Err(ZenithError::FileTooLarge(_))
		),
		"over the limit"
	);
	assert!(
		matches!(
			project.read_string(&z("Assets://big.bin")?),
			Err(ZenithError::FileTooLarge(_))
		),
		"read_string has the same limit"
	);
	let mut stream = project.open_read(&z("Assets://big.bin")?)?;
	let mut data = Vec::new();
	std::io::Read::read_to_end(&mut stream, &mut data)?;
	assert_eq!(data.len(), 100, "streaming is not limited");
	Ok(())
}

#[test]
fn windows_device_names_are_not_project_names() -> TestResult {
	let tmp = tempfile::tempdir()?;
	for name in ["con", "NUL", "Com1", "aux.Game"] {
		assert!(
			matches!(
				Project::create(tmp.path().join(name)),
				Err(ZenithError::BadProjectName(_))
			),
			"{name:?} is refused"
		);
	}
	Ok(())
}

/// A link (a junction on Windows, which needs no privilege) from `link` to
/// `target`. `None` if the system does not let us make one.
fn link_dir(target: &Path, link: &Path) -> Option<()> {
	#[cfg(unix)]
	{
		std::os::unix::fs::symlink(target, link).ok()
	}
	#[cfg(windows)]
	{
		let made = std::process::Command::new("cmd")
			.args(["/C", "mklink", "/J"])
			.arg(link)
			.arg(target)
			.output()
			.ok()?;
		made.status.success().then_some(())
	}
	#[cfg(not(any(unix, windows)))]
	{
		let _ = (target, link);
		None
	}
}

#[test]
fn a_project_reached_through_a_link_with_another_name_opens() -> TestResult {
	let tmp = tempfile::tempdir()?;
	// The project is called Game; its folder on disk is called Stuff, and the
	// path the user types goes through a link that is called Game.
	let original = tmp.path().join("Game");
	Project::create(&original)?;
	let real = tmp.path().join("Stuff");
	std::fs::rename(&original, &real)?;
	let alias = tmp.path().join("Game");
	if link_dir(&real, &alias).is_none() {
		eprintln!("SKIPPED: cannot create a link here");
		return Ok(());
	}

	let project = Project::open(&alias)?;
	assert_eq!(
		project.manifest().name,
		"Game",
		"the manifest is found under the link's name"
	);
	project.write(&z("Assets://a.txt")?, b"a")?;
	assert!(real.join("Assets/a.txt").is_file(), "and it is the same folder");
	Ok(())
}

#[test]
fn read_string_uses_the_cache_and_notices_changes() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	let path = z("Assets://note.txt")?;
	project.write(&path, b"first")?;
	for _ in 0..3 {
		assert_eq!(project.read_string(&path)?, "first", "every read, cached or not");
	}
	std::fs::write(root.join("Assets/note.txt"), b"second!")?;
	assert_eq!(project.read_string(&path)?, "second!", "a change from outside is seen");
	project.write(&path, &[0xff, 0xfe])?;
	assert!(
		matches!(project.read_string(&path), Err(ZenithError::InvalidUtf8(_))),
		"binary data is an error, however often it is read"
	);
	assert!(
		matches!(project.read_string(&path), Err(ZenithError::InvalidUtf8(_))),
		"also from the cache"
	);
	Ok(())
}

/// Runs `first` and `second` at the same moment on two threads.
fn at_the_same_time<A: Send, B: Send>(
	first: impl FnOnce() -> A + Send,
	second: impl FnOnce() -> B + Send,
) -> Result<(A, B), Box<dyn std::error::Error>> {
	let start = std::sync::Barrier::new(2);
	std::thread::scope(|scope| {
		let a = scope.spawn(|| {
			start.wait();
			first()
		});
		let b = scope.spawn(|| {
			start.wait();
			second()
		});
		Ok((
			a.join().map_err(|_| "the first thread panicked")?,
			b.join().map_err(|_| "the second thread panicked")?,
		))
	})
}

#[test]
fn of_two_moves_to_the_same_name_only_one_wins() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	for round in 0..300 {
		let (a, b, dest) = (
			z(&format!("Assets://a{round}.txt"))?,
			z(&format!("Assets://b{round}.txt"))?,
			z(&format!("Assets://d{round}.txt"))?,
		);
		std::fs::write(root.join(format!("Assets/a{round}.txt")), b"from a")?;
		std::fs::write(root.join(format!("Assets/b{round}.txt")), b"from b")?;
		let (first, second) = at_the_same_time(|| project.move_to(&a, &dest), || project.move_to(&b, &dest))?;

		let (winner, loser_source, loser_content, error) = match (first, second) {
			(Ok(()), Err(e)) => ("from a", format!("Assets/b{round}.txt"), "from b", e),
			(Err(e), Ok(())) => ("from b", format!("Assets/a{round}.txt"), "from a", e),
			(first, second) => {
				return Err(format!("round {round}: both or neither moved: {first:?} / {second:?}").into());
			}
		};
		assert!(
			matches!(error, ZenithError::ProjectAlreadyExists(_)),
			"round {round}: {error:?}"
		);
		assert_eq!(
			std::fs::read_to_string(root.join(format!("Assets/d{round}.txt")))?,
			winner,
			"round {round}: the winner's file is intact, nothing was written over it"
		);
		assert_eq!(
			std::fs::read_to_string(root.join(&loser_source))?,
			loser_content,
			"round {round}: the loser keeps its file"
		);
	}
	Ok(())
}

#[test]
fn of_two_copies_to_the_same_name_only_one_wins() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	for round in 0..300 {
		let (a, b, dest) = (
			z(&format!("Assets://a{round}.txt"))?,
			z(&format!("Assets://b{round}.txt"))?,
			z(&format!("Assets://d{round}.txt"))?,
		);
		std::fs::write(root.join(format!("Assets/a{round}.txt")), b"from a")?;
		std::fs::write(root.join(format!("Assets/b{round}.txt")), b"from b")?;
		let (first, second) = at_the_same_time(|| project.copy(&a, &dest), || project.copy(&b, &dest))?;

		let (winner, error) = match (first, second) {
			(Ok(()), Err(e)) => ("from a", e),
			(Err(e), Ok(())) => ("from b", e),
			(first, second) => {
				return Err(format!("round {round}: both or neither copied: {first:?} / {second:?}").into());
			}
		};
		assert!(
			matches!(error, ZenithError::ProjectAlreadyExists(_)),
			"round {round}: {error:?}"
		);
		assert_eq!(
			std::fs::read_to_string(root.join(format!("Assets/d{round}.txt")))?,
			winner,
			"round {round}"
		);
		let leftovers: Vec<String> = crate::common::names(&root.join("Assets"))
			.into_iter()
			.filter(|n| n.ends_with(".zenith-tmp"))
			.collect();
		assert!(
			leftovers.is_empty(),
			"round {round}: the loser's temporary file is gone: {leftovers:?}"
		);
	}
	Ok(())
}
