//! A symlink among the entries is an error, never silently skipped.
//!
//! Creating a symlink needs a privilege on Windows, and Rust cannot mark a test
//! ignored at run time, so a test that cannot create its link prints "SKIPPED"
//! and passes without checking. The junction test needs no privilege.

use std::path::Path;

use zenith_build::{Builder, ZPath};
use zenith_error::ZenithError;

use crate::common::{TestResult, standard_project};

#[cfg(unix)]
fn link_file(target: &Path, link: &Path) -> std::io::Result<()> {
	std::os::unix::fs::symlink(target, link)
}
#[cfg(windows)]
fn link_file(target: &Path, link: &Path) -> std::io::Result<()> {
	std::os::windows::fs::symlink_file(target, link)
}

#[test]
fn a_file_symlink_in_the_base_list_is_an_error() -> TestResult {
	let (tmp, root) = standard_project()?;
	if let Err(e) = link_file(&root.join("Assets/Worlds/A.zenith"), &root.join("Assets/alias.zenith")) {
		eprintln!("SKIPPED file symlink: cannot create the link here ({e})");
		return Ok(());
	}
	let out = tmp.path().join("b.zenithbuild");
	let result = Builder::new().path(&root).output(&out).build();
	assert!(
		matches!(result, Err(ZenithError::SymlinkInBuild(ref p)) if p == "Assets/alias.zenith"),
		"link among the entries"
	);
	assert!(!out.exists(), "nothing was written");

	let excluded = Builder::new()
		.path(&root)
		.output(&out)
		.exclude("Assets/alias.zenith")
		.zstd_compression(3)
		.build();
	assert!(excluded.is_ok(), "a link the rules leave out is not an entry");
	Ok(())
}

#[test]
fn add_file_with_a_symlink_source_is_an_error() -> TestResult {
	let (tmp, root) = standard_project()?;
	if let Err(e) = link_file(&root.join("Packages/p.txt"), &root.join("Temp/link.txt")) {
		eprintln!("SKIPPED add_file symlink: cannot create the link here ({e})");
		return Ok(());
	}
	let out = tmp.path().join("b.zenithbuild");
	let result = Builder::new()
		.path(&root)
		.output(&out)
		.add_file(ZPath::parse("Assets://x.txt")?, ZPath::parse("Temp://link.txt")?)
		.build();
	assert!(matches!(result, Err(ZenithError::SymlinkInBuild(_))), "{result:?}");
	Ok(())
}

#[cfg(windows)]
#[test]
fn a_junction_in_the_base_list_is_an_error() -> TestResult {
	let (tmp, root) = standard_project()?;
	let made = std::process::Command::new("cmd")
		.args(["/C", "mklink", "/J"])
		.arg(root.join("Assets").join("junction"))
		.arg(root.join("Packages"))
		.output()?;
	if !made.status.success() {
		eprintln!("SKIPPED junction: {}", String::from_utf8_lossy(&made.stderr));
		return Ok(());
	}
	let out = tmp.path().join("b.zenithbuild");
	let result = Builder::new().path(&root).output(&out).build();
	assert!(
		matches!(result, Err(ZenithError::SymlinkInBuild(ref p)) if p == "Assets/junction"),
		"{result:?}"
	);
	assert!(!out.exists(), "nothing was written");
	Ok(())
}
