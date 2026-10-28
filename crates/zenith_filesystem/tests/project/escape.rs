//! Nothing a project does may reach outside the project folder, whatever the
//! path says and whatever links exist inside it.
//!
//! Creating a symlink needs a privilege on Windows (developer mode or admin).
//! Rust has no way to mark a test as ignored at run time, so a test that
//! cannot create its link says so on stderr and passes without checking;
//! look for "SKIPPED" in the output when the escape coverage matters.

use std::path::Path;

use zenith_error::ZenithError;
use zenith_filesystem::{EntryKind, Project};

use crate::common::{TestResult, new_project, z};

#[cfg(unix)]
fn link_file(target: &Path, link: &Path) -> std::io::Result<()> {
	std::os::unix::fs::symlink(target, link)
}
#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) -> std::io::Result<()> {
	std::os::unix::fs::symlink(target, link)
}
#[cfg(windows)]
fn link_file(target: &Path, link: &Path) -> std::io::Result<()> {
	std::os::windows::fs::symlink_file(target, link)
}
#[cfg(windows)]
fn link_dir(target: &Path, link: &Path) -> std::io::Result<()> {
	std::os::windows::fs::symlink_dir(target, link)
}

fn skipped(what: &str, err: &std::io::Error) {
	eprintln!("SKIPPED {what}: cannot create the link here ({err})");
}

struct Setup {
	_tmp: tempfile::TempDir,
	outside: std::path::PathBuf,
	root: std::path::PathBuf,
	project: Project,
}

fn setup() -> Result<Setup, Box<dyn std::error::Error>> {
	let (tmp, root, project) = new_project("Game")?;
	let outside = tmp.path().join("outside");
	std::fs::create_dir(&outside)?;
	std::fs::write(outside.join("secret.txt"), b"secret")?;
	Ok(Setup {
		_tmp: tmp,
		outside,
		root,
		project,
	})
}

#[test]
fn symlink_to_a_file_outside_is_an_error() -> TestResult {
	let s = setup()?;
	if let Err(e) = link_file(&s.outside.join("secret.txt"), &s.root.join("Assets/leak.txt")) {
		{
			skipped("file link", &e);
			return Ok(());
		}
	}
	let leak = z("Assets://leak.txt")?;
	assert!(s.project.read_bytes(&leak).is_err(), "reading through the link fails");
	assert!(s.project.open_read(&leak).is_err(), "streaming through the link fails");
	assert!(
		s.project.write(&leak, b"overwritten").is_err(),
		"writing through the link fails"
	);
	assert_eq!(
		std::fs::read(s.outside.join("secret.txt"))?,
		b"secret",
		"the outside file is untouched"
	);
	assert!(s.project.is_symlink(&leak)?, "the link itself can still be inspected");
	Ok(())
}

#[test]
fn symlink_to_a_folder_outside_is_an_error() -> TestResult {
	let s = setup()?;
	if let Err(e) = link_dir(&s.outside, &s.root.join("Assets/leak")) {
		{
			skipped("folder link", &e);
			return Ok(());
		}
	}
	assert!(
		s.project.read_bytes(&z("Assets://leak/secret.txt")?).is_err(),
		"reading through the folder link fails"
	);
	assert!(s.project.list(&z("Assets://leak")?).is_err(), "listing it fails");
	assert!(
		s.project.write(&z("Assets://leak/new.txt")?, b"x").is_err(),
		"writing through it fails"
	);
	assert!(
		s.project.create_dir(&z("Assets://leak/sub")?).is_err(),
		"creating a folder through it fails"
	);
	assert!(
		s.project
			.copy(&z("Assets://leak/secret.txt")?, &z("Assets://stolen.txt")?)
			.is_err(),
		"copying out of it fails"
	);
	assert!(
		s.project.copy(&z("Settings://")?, &z("Assets://leak/copy")?).is_err(),
		"copying into it fails"
	);
	assert!(
		!s.outside.join("new.txt").exists() && !s.outside.join("sub").exists() && !s.outside.join("copy").exists(),
		"nothing was created outside"
	);
	assert!(!s.root.join("Assets/stolen.txt").exists(), "nothing was copied in");
	Ok(())
}

#[test]
fn escape_errors_are_reported_as_such() -> TestResult {
	let s = setup()?;
	if let Err(e) = link_file(&s.outside.join("secret.txt"), &s.root.join("Assets/leak.txt")) {
		{
			skipped("error kind", &e);
			return Ok(());
		}
	}
	assert!(
		matches!(
			s.project.read_bytes(&z("Assets://leak.txt")?),
			Err(ZenithError::PathEscapesRoot(_))
		),
		"an escaping link is reported as PathEscapesRoot"
	);
	Ok(())
}

#[test]
fn symlinks_that_stay_inside_work() -> TestResult {
	let s = setup()?;
	s.project.write(&z("Assets://real.txt")?, b"inside")?;
	if let Err(e) = link_file(Path::new("real.txt"), &s.root.join("Assets/alias.txt")) {
		{
			skipped("inner link", &e);
			return Ok(());
		}
	}
	assert_eq!(
		&*s.project.read_bytes(&z("Assets://alias.txt")?)?,
		b"inside",
		"reads follow an inner link"
	);
	assert!(s.project.is_symlink(&z("Assets://alias.txt")?)?, "is_symlink");
	s.project.write(&z("Assets://alias.txt")?, b"changed")?;
	assert_eq!(
		std::fs::read(s.root.join("Assets/real.txt"))?,
		b"changed",
		"writing through an inner link changes its target"
	);
	assert!(s.project.is_symlink(&z("Assets://alias.txt")?)?, "and keeps the link");
	assert!(
		!s.project.is_symlink(&z("Assets://real.txt")?)?,
		"a real file is not a link"
	);
	let kinds: Vec<_> = s
		.project
		.list(&z("Assets://")?)?
		.into_iter()
		.filter(|e| std::path::Path::new(&e.name).extension().is_some_and(|x| x == "txt"))
		.map(|e| (e.name, e.kind))
		.collect();
	assert_eq!(
		kinds,
		vec![
			("alias.txt".to_string(), EntryKind::Symlink),
			("real.txt".to_string(), EntryKind::File)
		],
		"list reports links as links"
	);
	Ok(())
}

#[cfg(windows)]
#[test]
fn junction_to_a_folder_outside_is_an_error() -> TestResult {
	let s = setup()?;
	// Junctions need no privilege, so this one runs everywhere on Windows.
	let status = std::process::Command::new("cmd")
		.args(["/C", "mklink", "/J"])
		.arg(s.root.join("Assets").join("junction"))
		.arg(&s.outside)
		.output()?;
	if !status.status.success() {
		{
			skipped(
				"junction",
				&std::io::Error::other(String::from_utf8_lossy(&status.stderr).into_owned()),
			);
			return Ok(());
		}
	}
	assert!(
		s.project.read_bytes(&z("Assets://junction/secret.txt")?).is_err(),
		"reading through a junction fails"
	);
	assert!(
		s.project.write(&z("Assets://junction/new.txt")?, b"x").is_err(),
		"writing through a junction fails"
	);
	assert!(!s.outside.join("new.txt").exists(), "nothing was created outside");
	Ok(())
}

#[test]
fn parser_rejects_traversal_early() {
	for text in [
		"Assets://../../outside/secret.txt",
		"Assets:///etc/passwd",
		"Assets://a/../../../x",
		"Assets://../..\\x",
	] {
		assert!(z(text).is_err(), "{text:?} is rejected before any file is touched");
	}
}
