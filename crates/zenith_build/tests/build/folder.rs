use std::{collections::BTreeSet, path::Path, time::SystemTime};

use zenith_archive::{FolderWriter, WriterOptions};
use zenith_build::{BuildFormat, BuildReport, Builder, EncryptionKey};
use zenith_error::ZenithError;
use zenith_filesystem::Project;

use crate::common::{TestResult, key, standard_project, tree_hashes, write};

fn build(root: &Path, out: &Path, key: Option<&EncryptionKey>) -> Result<BuildReport, ZenithError> {
	let mut b = Builder::new()
		.path(root)
		.output(out)
		.format(BuildFormat::Fs)
		.zstd_compression(3);
	if let Some(key) = key {
		b = b.encryption_key(key.clone());
	}
	b.build()
}

fn mtimes(dir: &Path) -> std::collections::BTreeMap<String, SystemTime> {
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

/// Names that differ (changed content, added, or removed) between two
/// snapshots.
fn changed(
	before: &std::collections::BTreeMap<String, String>,
	after: &std::collections::BTreeMap<String, String>,
) -> BTreeSet<String> {
	before
		.keys()
		.chain(after.keys())
		.filter(|name| before.get(*name) != after.get(*name))
		.cloned()
		.collect()
}

#[test]
fn changing_one_file_changes_only_its_data_file_and_the_index() -> TestResult {
	for key in [None, Some(key(4))] {
		let (tmp, root) = standard_project()?;
		let out = tmp.path().join("folder");
		build(&root, &out, key.as_ref())?;
		let before = tree_hashes(&out);
		let before_times = mtimes(&out);
		let total = before.len();
		assert!(total >= 7, "one data file per entry plus the index: {total}");

		std::thread::sleep(std::time::Duration::from_millis(30));
		write(&root, "Assets/Worlds/A.zenith", b"world a, edited")?;
		build(&root, &out, key.as_ref())?;
		let after = tree_hashes(&out);

		let writer = FolderWriter::new(&WriterOptions {
			key: key.clone(),
			..WriterOptions::default()
		})?;
		let edited = writer.data_file_name("Assets/Worlds/A.zenith")?;
		assert_eq!(
			changed(&before, &after),
			BTreeSet::from([edited.clone(), "build.index".to_string()]),
			"only the edited file's data file and the index differ (encrypted: {})",
			key.is_some()
		);
		let same = before
			.iter()
			.filter(|(name, hash)| after.get(*name) == Some(*hash))
			.count();
		assert_eq!(same, total - 2, "every other file has the same content hash");

		let after_times = mtimes(&out);
		for (name, time) in &before_times {
			if *name != edited && name != "build.index" {
				assert_eq!(
					after_times.get(name),
					Some(time),
					"{name} was not rewritten (modification time kept)"
				);
			}
		}
	}
	Ok(())
}

#[test]
fn removing_a_file_deletes_only_its_data_file() -> TestResult {
	for key in [None, Some(key(4))] {
		let (tmp, root) = standard_project()?;
		let out = tmp.path().join("folder");
		build(&root, &out, key.as_ref())?;
		write(&out, "my-notes.txt", b"not part of the build")?;
		let before = tree_hashes(&out);

		std::fs::remove_file(root.join("Assets/Worlds/A.zenith"))?;
		write(&root, "Assets/Added.txt", b"new file")?;
		build(&root, &out, key.as_ref())?;
		let after = tree_hashes(&out);

		let writer = FolderWriter::new(&WriterOptions {
			key: key.clone(),
			..WriterOptions::default()
		})?;
		let removed = writer.data_file_name("Assets/Worlds/A.zenith")?;
		let added = writer.data_file_name("Assets/Added.txt")?;
		assert_eq!(
			changed(&before, &after),
			BTreeSet::from([removed.clone(), added, "build.index".to_string()]),
			"one deleted, one new, and the index"
		);
		assert!(!out.join(&removed).exists(), "the stale data file is gone");
		assert_eq!(
			std::fs::read(out.join("my-notes.txt"))?,
			b"not part of the build",
			"other files in the folder are never deleted"
		);
		if key.is_some() {
			let dir = removed.split_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
			let still_used = after.keys().any(|n| n.starts_with(&format!("{dir}/")));
			assert_eq!(
				out.join(&dir).exists(),
				still_used,
				"an emptied fan-out folder is removed, a used one stays"
			);
		}
	}
	Ok(())
}

#[test]
fn a_foreign_folder_is_refused_and_left_alone() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("someone-elses");
	write(&out, "photo.jpg", b"precious")?;
	write(&out, "docs/a.txt", b"precious too")?;
	let before = tree_hashes(&out);
	let result = build(&root, &out, None);
	assert!(matches!(result, Err(ZenithError::NotABuildDir(_))), "{result:?}");
	assert_eq!(tree_hashes(&out), before, "nothing changed");

	let empty = tmp.path().join("empty");
	std::fs::create_dir(&empty)?;
	assert!(build(&root, &empty, None).is_ok(), "an empty folder is fine");
	assert!(
		build(&root, &tmp.path().join("new").join("deeper"), None).is_ok(),
		"a missing folder is created with parents"
	);
	Ok(())
}

#[test]
fn a_foreign_file_with_the_name_of_a_new_data_file_is_not_overwritten() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("folder");
	build(&root, &out, None)?;
	write(&out, "Assets/Added.txt", b"foreign file")?;
	write(&root, "Assets/Added.txt", b"our file")?;
	let before = tree_hashes(&out);
	let result = build(&root, &out, None);
	assert!(matches!(result, Err(ZenithError::InvalidBuildOutput(_))), "{result:?}");
	assert_eq!(
		tree_hashes(&out),
		before,
		"nothing changed, the old build is still complete"
	);
	Ok(())
}

#[test]
fn an_unreadable_previous_build_stops_the_build() -> TestResult {
	let (tmp, root) = standard_project()?;
	let refused = |out: &Path, key: Option<&EncryptionKey>| -> Result<(), Box<dyn std::error::Error>> {
		let before = tree_hashes(out);
		let result = build(&root, out, key);
		assert!(
			matches!(result, Err(ZenithError::PreviousBuildUnreadable(_))),
			"{result:?}"
		);
		assert_eq!(tree_hashes(out), before, "the folder is not cleaned or changed");
		Ok(())
	};

	let encrypted = tmp.path().join("enc");
	build(&root, &encrypted, Some(&key(1)))?;
	refused(&encrypted, Some(&key(2)))?;
	refused(&encrypted, None)?;
	build(&root, &encrypted, Some(&key(1)))?;

	let plain = tmp.path().join("plain");
	build(&root, &plain, None)?;
	refused(&plain, Some(&key(1)))?;

	let damaged = tmp.path().join("damaged");
	build(&root, &damaged, None)?;
	let mut index = std::fs::read(damaged.join("build.index"))?;
	let middle = index.len() / 2;
	index[middle] ^= 1;
	std::fs::write(damaged.join("build.index"), index)?;
	refused(&damaged, None)?;
	Ok(())
}

#[test]
fn the_folder_build_opens_as_a_project() -> TestResult {
	let (tmp, root) = standard_project()?;
	let out = tmp.path().join("folder");
	build(&root, &out, None)?;
	let project = Project::open(&out)?;
	assert_eq!(project.manifest().name, "Game", "manifest");
	assert_eq!(
		project.read_string(&zenith_build::ZPath::parse("Packages://p.txt")?)?,
		"package",
		"content"
	);
	Ok(())
}
