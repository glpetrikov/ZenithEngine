use zenith_build::{BuildFormat, Builder};
use zenith_error::ZenithError;

use crate::common::{TestResult, standard_project, write};

fn gitignore(root: &std::path::Path) -> Result<String, std::io::Error> {
	std::fs::read_to_string(root.join(".gitignore"))
}

#[test]
fn default_outputs_live_in_build_and_are_ignored() -> TestResult {
	let (_tmp, root) = standard_project()?;
	let file = Builder::new().path(&root).zstd_compression(3).build()?.output;
	assert_eq!(
		file,
		root.join("Build").join("game.zenithbuild"),
		"default archive path"
	);
	assert!(file.is_file(), "written");
	assert_eq!(
		gitignore(&root)?,
		"Temp/\nTrash/\n.zenitheditor-backup/\n*.zenith-tmp\n# zenith:begin\n/Build/\n# zenith:end\n",
		"Build/ went into the managed .gitignore block"
	);

	let folder = Builder::new()
		.path(&root)
		.format(BuildFormat::Fs)
		.zstd_compression(3)
		.build()?
		.output;
	assert_eq!(folder, root.join("Build").join("game"), "default folder path");
	assert!(folder.join("build.index").is_file(), "index written");
	assert_eq!(gitignore(&root)?.matches("/Build/").count(), 1, "and listed once");
	Ok(())
}

#[test]
fn a_folder_build_straight_in_the_project_root_is_ignored_itself() -> TestResult {
	let (_tmp, root) = standard_project()?;
	let out = root.join("dist");
	Builder::new()
		.path(&root)
		.output(&out)
		.format(BuildFormat::Fs)
		.zstd_compression(3)
		.build()?;
	assert!(out.join("build.index").is_file(), "written");
	assert!(
		gitignore(&root)?.contains("/dist/"),
		"the build folder is ignored, not only its (non-existent) parent: {}",
		gitignore(&root)?
	);
	Ok(())
}

#[test]
fn the_parent_of_a_custom_output_is_created_and_listed_when_inside() -> TestResult {
	let (tmp, root) = standard_project()?;
	let inside = root.join("Out").join("Dist").join("g.zenithbuild");
	Builder::new().path(&root).output(&inside).zstd_compression(3).build()?;
	assert!(inside.is_file(), "written, parents created");
	assert!(
		gitignore(&root)?.contains("/Out/Dist/"),
		"the parent folder is ignored: {}",
		gitignore(&root)?
	);

	let before = gitignore(&root)?;
	let outside = tmp.path().join("elsewhere").join("deeper").join("g.zenithbuild");
	Builder::new()
		.path(&root)
		.output(&outside)
		.zstd_compression(3)
		.build()?;
	assert!(outside.is_file(), "written outside the project");
	assert_eq!(
		gitignore(&root)?,
		before,
		".gitignore is not touched for a folder outside the project"
	);
	Ok(())
}

#[test]
fn outputs_that_overlap_the_inputs_are_refused() -> TestResult {
	let (tmp, root) = standard_project()?;
	let refused = |out: std::path::PathBuf, format: BuildFormat| -> Result<(), Box<dyn std::error::Error>> {
		let existed = out.exists();
		let result = Builder::new()
			.path(&root)
			.output(&out)
			.format(format)
			.zstd_compression(3)
			.build();
		assert!(
			matches!(result, Err(ZenithError::InvalidBuildOutput(_))),
			"{} ({format:?}): {result:?}",
			out.display()
		);
		assert_eq!(out.exists(), existed, "{} must not be created", out.display());
		Ok(())
	};
	for dir in ["Assets", "Settings", "Packages"] {
		refused(root.join(dir).join("out.zenithbuild"), BuildFormat::Archive)?;
		refused(root.join(dir).join("out"), BuildFormat::Fs)?;
	}
	refused(root.join("Assets"), BuildFormat::Fs)?;
	refused(root.join("Game.zenithproject"), BuildFormat::Archive)?;
	refused(root.clone(), BuildFormat::Fs)?;
	refused(tmp.path().to_path_buf(), BuildFormat::Fs)?;
	refused(root.join("sub").join("..").join(".."), BuildFormat::Fs)?;
	Ok(())
}

#[test]
fn an_output_that_a_rule_makes_an_input_is_refused() -> TestResult {
	let (_tmp, root) = standard_project()?;
	let docs = root.join("Docs").join("readme.txt");
	let result = Builder::new().path(&root).output(&docs).include("Docs/**").build();
	assert!(
		matches!(result, Err(ZenithError::InvalidBuildOutput(_))),
		"the output is a file the build reads: {result:?}"
	);
	assert_eq!(std::fs::read(&docs)?, b"docs", "and it was not overwritten");

	// Build/** pulled into a folder build whose output is below Build/.
	let result = Builder::new()
		.path(&root)
		.format(BuildFormat::Fs)
		.zstd_compression(3)
		.build();
	assert!(result.is_ok(), "the first folder build is fine");
	let result = Builder::new()
		.path(&root)
		.format(BuildFormat::Fs)
		.include("Build/**")
		.build();
	assert!(
		matches!(result, Err(ZenithError::InvalidBuildOutput(_))),
		"a folder build that includes its own output: {result:?}"
	);
	let result = Builder::new()
		.path(&root)
		.include("Build/**")
		.zstd_compression(3)
		.build();
	assert!(
		result.is_ok(),
		"an archive may include the old folder build, it is not its own output"
	);
	Ok(())
}

#[test]
fn wrong_kinds_of_existing_outputs() -> TestResult {
	let (tmp, root) = standard_project()?;
	let folder = tmp.path().join("is-a-folder");
	std::fs::create_dir(&folder)?;
	let result = Builder::new().path(&root).output(&folder).build();
	assert!(
		matches!(result, Err(ZenithError::InvalidBuildOutput(_))),
		"an archive cannot replace a folder: {result:?}"
	);

	let file = tmp.path().join("is-a-file");
	std::fs::write(&file, b"x")?;
	let result = Builder::new().path(&root).output(&file).format(BuildFormat::Fs).build();
	assert!(
		matches!(result, Err(ZenithError::NotABuildDir(_))),
		"a folder build cannot replace a file: {result:?}"
	);
	assert_eq!(std::fs::read(&file)?, b"x", "the file is untouched");
	Ok(())
}

#[test]
fn a_successful_build_leaves_no_temporary_files() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("t");
	std::fs::create_dir(&out)?;
	Builder::new()
		.path(&root)
		.output(out.join("a.zenithbuild"))
		.zstd_compression(3)
		.build()?;
	Builder::new()
		.path(&root)
		.output(out.join("f"))
		.format(BuildFormat::Fs)
		.zstd_compression(3)
		.build()?;
	write(&root, "Assets/new.txt", b"x")?;
	Builder::new()
		.path(&root)
		.output(out.join("f"))
		.format(BuildFormat::Fs)
		.zstd_compression(3)
		.build()?;
	let mut left = Vec::new();
	let mut stack = vec![out];
	while let Some(dir) = stack.pop() {
		for entry in std::fs::read_dir(dir)?.flatten() {
			let path = entry.path();
			if path.is_dir() {
				stack.push(path);
			} else if path.to_string_lossy().ends_with(".zenith-tmp") {
				left.push(path);
			}
		}
	}
	assert!(left.is_empty(), "temporary files left behind: {left:?}");
	Ok(())
}

#[test]
fn the_default_name_does_not_depend_on_the_project_name() -> TestResult {
	// A runtime opens the build under a fixed name, whatever the project is
	// called.
	let tmp = tempfile::tempdir()?;
	for name in ["Alpha", "Totally Different"] {
		let root = tmp.path().join(name);
		zenith_filesystem::Project::create(&root)?;
		let file = Builder::new().path(&root).zstd_compression(3).build()?.output;
		assert_eq!(file, root.join("Build").join("game.zenithbuild"), "archive for {name}");
		let folder = Builder::new()
			.path(&root)
			.format(BuildFormat::Fs)
			.zstd_compression(3)
			.build()?
			.output;
		assert_eq!(folder, root.join("Build").join("game"), "folder for {name}");

		for path in [&file, &folder] {
			let project = zenith_filesystem::Project::open(path)?;
			assert_eq!(
				project.manifest().name,
				name,
				"the project name still comes from the manifest"
			);
		}
	}
	Ok(())
}

#[test]
fn a_build_never_replaces_the_files_that_make_a_project() -> TestResult {
	let (_tmp, root) = standard_project()?;
	for rel in [".v1", ".gitignore", ".git/x.zenithbuild", "Trash/x.zenithbuild"] {
		let out = root.join(rel);
		let before = std::fs::read(&out).ok();
		let result = Builder::new().path(&root).output(&out).zstd_compression(3).build();
		assert!(
			matches!(result, Err(ZenithError::InvalidBuildOutput(_))),
			"{rel}: {result:?}"
		);
		assert_eq!(std::fs::read(&out).ok(), before, "{rel} is untouched");
	}
	Ok(())
}

#[test]
fn a_file_named_like_a_temporary_file_cannot_go_into_a_folder_build() -> TestResult {
	let (tmp, root) = standard_project()?;
	write(&root, "Assets/a.txt.zenith-tmp", b"x")?;
	let out = tmp.path().join("folder");
	let result = Builder::new()
		.path(&root)
		.output(&out)
		.format(BuildFormat::Fs)
		.zstd_compression(3)
		.build();
	assert!(matches!(result, Err(ZenithError::InvalidBuildOutput(_))), "{result:?}");
	assert!(!out.exists(), "and nothing was written");
	Ok(())
}
