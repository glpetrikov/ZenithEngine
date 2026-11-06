use zenith_error::ZenithError;
use zenith_filesystem::{Project, WarningKind};

use crate::common::{TestResult, new_project, z};

fn gitignore(root: &std::path::Path) -> std::io::Result<String> {
	std::fs::read_to_string(root.join(".gitignore"))
}

#[test]
fn a_folder_inside_the_project_is_created_and_listed() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.ensure_ignored_dir(root.join("Build"))?;
	assert!(root.join("Build").is_dir(), "the folder exists");
	assert_eq!(
		gitignore(&root)?,
		"Temp/\nTrash/\n.zenitheditor-backup/\n*.zenith-tmp\n# zenith:begin\n/Build/\n# zenith:end\n",
		"an anchored entry in a managed block; the original lines are untouched"
	);
	project.ensure_ignored_dir(root.join("Build"))?;
	project.ensure_ignored_dir(root.join("Out").join("Dist"))?;
	assert_eq!(
		gitignore(&root)?,
		"Temp/\nTrash/\n.zenitheditor-backup/\n*.zenith-tmp\n# zenith:begin\n/Build/\n/Out/Dist/\n# zenith:end\n",
		"repeating changes nothing, a nested folder is added to the same block"
	);
	assert!(root.join("Out/Dist").is_dir(), "parents are created");
	Ok(())
}

#[test]
fn a_folder_outside_the_project_is_only_created() -> TestResult {
	let (tmp, root, project) = new_project("Game")?;
	let before = gitignore(&root)?;
	let outside = tmp.path().join("builds").join("out");
	project.ensure_ignored_dir(&outside)?;
	assert!(outside.is_dir(), "created with parents");
	assert_eq!(gitignore(&root)?, before, ".gitignore is not touched");
	Ok(())
}

#[test]
fn without_a_gitignore_nothing_is_created() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	std::fs::remove_file(root.join(".gitignore"))?;
	project.ensure_ignored_dir(root.join("Build"))?;
	assert!(root.join("Build").is_dir(), "the folder is created");
	assert!(!root.join(".gitignore").exists(), "the file is not");
	project.sync_gitignore()?;
	assert!(!root.join(".gitignore").exists(), "sync does not create it either");
	Ok(())
}

#[test]
fn existing_lines_outside_the_block_are_respected() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	std::fs::write(root.join(".gitignore"), "Temp/\nTrash/\n/Build/\n")?;
	project.ensure_ignored_dir(root.join("Build"))?;
	assert_eq!(
		gitignore(&root)?,
		"Temp/\nTrash/\n/Build/\n",
		"no duplicate and no block"
	);
	project.ensure_ignored_dir(root.join("Temp").join("Build"))?;
	assert_eq!(
		gitignore(&root)?,
		"Temp/\nTrash/\n/Build/\n",
		"a folder under an ignored parent needs no entry"
	);
	Ok(())
}

#[test]
fn line_endings_and_final_newline_are_kept() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	std::fs::write(root.join(".gitignore"), "Temp/\r\nTrash/")?;
	project.ensure_ignored_dir(root.join("Build"))?;
	assert_eq!(
		gitignore(&root)?,
		"Temp/\r\nTrash/\r\n# zenith:begin\r\n/Build/\r\n# zenith:end",
		"CRLF and the missing final newline survive"
	);
	Ok(())
}

#[test]
fn removing_the_folder_through_the_project_drops_the_entry() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.register_scheme("build", "Build")?;
	project.ensure_ignored_dir(root.join("Build"))?;
	project.write(&z("build://out.bin")?, b"x")?;
	let id = project.remove(&z("build://")?);
	assert!(id.is_err(), "a scheme root cannot be removed, only what is inside");

	project.register_scheme("dist", "Dist")?;
	project.ensure_ignored_dir(root.join("Dist").join("win"))?;
	let id = project.remove(&z("dist://win")?)?;
	assert_eq!(
		gitignore(&root)?,
		"Temp/\nTrash/\n.zenitheditor-backup/\n*.zenith-tmp\n# zenith:begin\n/Build/\n# zenith:end\n",
		"the entry of the removed folder is gone, the other stays"
	);
	let entry: toml::Table =
		std::fs::read_to_string(root.join("Trash").join(id.as_str()).join("entry.toml"))?.parse()?;
	assert_eq!(
		entry["ignored"].as_bool(),
		Some(true),
		"entry.toml remembers the folder was ignored"
	);

	project.restore(&id)?;
	assert!(gitignore(&root)?.contains("/Dist/win/"), "restoring re-adds the entry");
	Ok(())
}

#[test]
fn the_block_disappears_with_its_last_folder() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.register_scheme("build", "Build")?;
	project.ensure_ignored_dir(root.join("Build").join("x"))?;
	project.remove(&z("build://x")?)?;
	assert_eq!(
		gitignore(&root)?,
		"Temp/\nTrash/\n.zenitheditor-backup/\n*.zenith-tmp\n",
		"an empty block is removed entirely"
	);
	Ok(())
}

#[test]
fn moving_a_folder_through_the_project_syncs_too() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.register_scheme("out", "Out")?;
	project.ensure_ignored_dir(root.join("Out").join("a"))?;
	project.move_to(&z("out://a")?, &z("Assets://moved")?)?;
	assert_eq!(
		gitignore(&root)?,
		"Temp/\nTrash/\n.zenitheditor-backup/\n*.zenith-tmp\n",
		"the old location is no longer listed"
	);
	Ok(())
}

#[test]
fn an_explicit_sync_cleans_up_stale_entries_and_open_leaves_the_file_alone() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.ensure_ignored_dir(root.join("Build"))?;
	std::fs::remove_dir(root.join("Build"))?;
	project.sync_gitignore()?;
	assert_eq!(
		gitignore(&root)?,
		"Temp/\nTrash/\n.zenitheditor-backup/\n*.zenith-tmp\n",
		"explicit sync"
	);

	project.ensure_ignored_dir(root.join("Build"))?;
	drop(project);
	std::fs::remove_dir(root.join("Build"))?;
	let stale = gitignore(&root)?;
	let _reopened = Project::open(&root)?;
	assert_eq!(
		gitignore(&root)?,
		stale,
		"open only reads: a tool that opens a project edits nothing"
	);
	Ok(())
}

#[test]
fn a_damaged_block_never_blocks_opening() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	drop(project);
	let damaged = "Temp/\n# zenith:begin\n/Build/\n";
	std::fs::write(root.join(".gitignore"), damaged)?;

	let project = Project::open(&root)?;
	let warnings = project.warnings();
	assert_eq!(warnings.len(), 1, "one warning");
	assert_eq!(
		warnings.first().map(|w| w.kind),
		Some(WarningKind::DamagedGitignore),
		"of the right kind"
	);
	assert!(
		warnings.first().is_some_and(|w| w.message.contains("zenith:begin")),
		"that says what is wrong"
	);
	assert_eq!(gitignore(&root)?, damaged, "the file is not changed");

	assert!(
		matches!(project.sync_gitignore(), Err(ZenithError::InvalidGitignore(_))),
		"explicit sync reports it"
	);
	assert!(
		matches!(
			project.ensure_ignored_dir(root.join("Build")),
			Err(ZenithError::InvalidGitignore(_))
		),
		"ensure_ignored_dir too"
	);
	assert_eq!(gitignore(&root)?, damaged, "still unchanged");
	assert!(
		root.join("Build").is_dir(),
		"the folder itself is still created before the failure"
	);

	project.write(&z("Assets://still-works.txt")?, b"x")?;
	let id = project.remove(&z("Assets://still-works.txt")?)?;
	assert_eq!(
		project
			.warnings()
			.iter()
			.filter(|w| w.kind == WarningKind::DamagedGitignore)
			.count(),
		1,
		"operations that sync record a warning instead of failing (once, however often they run)"
	);
	project.restore(&id)?;
	Ok(())
}
