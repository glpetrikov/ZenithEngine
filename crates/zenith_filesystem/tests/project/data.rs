//! `Data://`: the data of the player, kept outside the project.

use std::sync::Arc;

use zenith_archive::{ArchiveWriter, WriterOptions};
use zenith_error::ZenithError;
use zenith_filesystem::{DirStore, EntryKind, Manifest, MemStore, OpenOptions, Project, UserStore, userdata};

use crate::common::{TestResult, names, new_project, z};

fn build() -> Result<Arc<[u8]>, Box<dyn std::error::Error>> {
	let options = WriterOptions {
		compress: false,
		chunk_size_log2: 12,
		..WriterOptions::default()
	};
	let mut writer = ArchiveWriter::new(Vec::new(), &options)?;
	// Entries go in the order of their paths.
	writer.add_bytes("Assets/a.txt", b"a")?;
	writer.add_bytes("Game.zenithproject", Manifest::new("Game")?.to_toml()?.as_bytes())?;
	Ok(Arc::from(writer.finish()?))
}

/// A project folder whose `Data://` is in memory (the default is the real data
/// folder of whoever runs the tests).
fn project() -> Result<(tempfile::TempDir, std::path::PathBuf, Project), Box<dyn std::error::Error>> {
	let (tmp, root, project) = new_project("Game")?;
	project.set_user_data(Arc::new(MemStore::new()));
	Ok((tmp, root, project))
}

#[test]
fn data_is_written_and_read_like_a_project_file() -> TestResult {
	let (_tmp, root, project) = project()?;
	let file = z("Data://Saves/slot1.bin")?;
	assert!(!project.exists(&file)?, "nothing yet");
	project.write(&file, b"progress")?;
	assert!(project.exists(&file)?, "written");
	assert_eq!(&*project.read_bytes(&file)?, b"progress", "read back");
	assert_eq!(project.read_string(&file).ok().as_deref(), Some("progress"), "as text");
	assert_eq!(project.metadata(&file)?.size, 8, "size");
	assert_eq!(project.metadata(&file)?.kind, EntryKind::File, "kind");
	let mut text = String::new();
	std::io::Read::read_to_string(&mut project.open_read(&file)?, &mut text)?;
	assert_eq!(text, "progress", "streamed");

	project.write(&file, b"more progress")?;
	assert_eq!(&*project.read_bytes(&file)?, b"more progress", "replaced");
	let listed: Vec<String> = project.list(&z("Data://Saves")?)?.into_iter().map(|e| e.name).collect();
	assert_eq!(listed, ["slot1.bin"], "listed");
	let root_names: Vec<String> = project.list(&z("Data://")?)?.into_iter().map(|e| e.name).collect();
	assert_eq!(root_names, ["Saves"], "the root lists the folder");

	assert!(project.delete(&file)?, "deleted");
	assert!(!project.delete(&file)?, "only once");
	assert!(!project.exists(&file)?, "gone");
	assert!(
		matches!(project.read_bytes(&file), Err(ZenithError::ProjectNotFound(_))),
		"not found"
	);
	assert!(
		names(&root).iter().all(|n| n != "Data"),
		"nothing of it is in the project folder"
	);
	Ok(())
}

#[test]
fn data_works_for_builds_which_are_read_only() -> TestResult {
	let store = Arc::new(MemStore::new());
	let project = Project::open_archive_bytes(build()?, &OpenOptions::new().user_data(store.clone()))?;
	assert!(
		matches!(project.write(&z("Assets://b.txt")?, b"x"), Err(ZenithError::ReadOnly)),
		"the build itself cannot be written"
	);
	project.write(&z("Data://Input.toml")?, b"x = 1")?;
	assert_eq!(
		&*project.read_bytes(&z("Data://Input.toml")?)?,
		b"x = 1",
		"but Data:// can"
	);
	assert_eq!(
		store.read("Input.toml")?,
		Some(b"x = 1".to_vec()),
		"it went to the store"
	);
	project.create_dir(&z("Data://Saves")?)?;
	Ok(())
}

#[test]
fn data_can_be_moved_to_another_store_later() -> TestResult {
	let first = Arc::new(MemStore::new());
	let second = Arc::new(MemStore::new());
	let project = Project::open_archive_bytes(build()?, &OpenOptions::new().user_data(first.clone()))?;
	project.write(&z("Data://a")?, b"1")?;
	project.set_user_data(second.clone());
	project.write(&z("Data://b")?, b"2")?;
	assert_eq!(
		first.read("a")?,
		Some(b"1".to_vec()),
		"the first store keeps what it got"
	);
	assert_eq!(
		second.read("b")?,
		Some(b"2".to_vec()),
		"the second gets what came after"
	);
	assert_eq!(second.read("a")?, None, "nothing is moved");
	project.flush_data()?;
	Ok(())
}

#[test]
fn data_stays_out_of_the_project() -> TestResult {
	let (_tmp, _root, project) = project()?;
	assert!(
		matches!(
			project.register_scheme("data", "Saves"),
			Err(ZenithError::SchemeAlreadyRegistered(_))
		),
		"the scheme cannot be taken for something else, in any spelling"
	);
	assert!(
		matches!(project.remove(&z("Data://x")?), Err(ZenithError::Unsupported(_))),
		"no trash for Data://"
	);
	project.write(&z("Data://x")?, b"1")?;
	assert!(
		matches!(
			project.copy(&z("Data://x")?, &z("Assets://y")?),
			Err(ZenithError::Unsupported(_))
		),
		"not copied into the project"
	);
	assert!(
		matches!(
			project.copy(&z("Assets://y")?, &z("Data://x")?),
			Err(ZenithError::Unsupported(_))
		),
		"nor out of it"
	);
	assert!(
		matches!(project.delete(&z("Assets://x")?), Err(ZenithError::Unsupported(_))),
		"project files are not deleted for good"
	);
	Ok(())
}

#[test]
fn bad_names_are_refused() -> TestResult {
	let (_tmp, _root, project) = project()?;
	for bad in [
		"Data://",
		"Data://a.zenith-tmp",
		"Data://Saves/slot.zenith-tmp",
		"Data://CON",
		"Data://a.",
		"Data://a b ",
	] {
		assert!(
			matches!(project.write(&z(bad)?, b"x"), Err(ZenithError::InvalidZPath(_))),
			"{bad} must be refused"
		);
	}
	assert!(
		project.write(&z("Data://Saves/Slot 1.bin")?, b"x").is_ok(),
		"ordinary names are fine"
	);
	Ok(())
}

#[test]
fn the_size_limit_holds_for_data_too() -> TestResult {
	let store = Arc::new(MemStore::new());
	let project = Project::open_archive_bytes(build()?, &OpenOptions::new().max_read_size(4).user_data(store))?;
	project.write(&z("Data://big")?, b"123456789")?;
	assert!(
		matches!(project.read_bytes(&z("Data://big")?), Err(ZenithError::FileTooLarge(_))),
		"too big to load"
	);
	project.write(&z("Data://small")?, b"1234")?;
	assert!(project.read_bytes(&z("Data://small")?).is_ok(), "at the limit is fine");
	Ok(())
}

#[test]
fn by_default_data_is_in_the_data_folder_of_the_user_and_nothing_is_made_early() -> TestResult {
	let name = "ZenithTestNoSuchGame7F3A91";
	let Some(expected) = userdata::default_data_dir(name) else {
		return Ok(()); // a system with no data folder
	};
	assert!(!expected.exists(), "the test needs the folder to be absent");
	let tmp = tempfile::tempdir()?;
	let root = tmp.path().join(name);
	let project = Project::create(&root)?;
	assert!(!project.exists(&z("Data://anything")?)?, "nothing there");
	assert!(project.list(&z("Data://")?)?.is_empty(), "the root is empty");
	assert!(!expected.exists(), "reading made no folder");
	// Not written here: that would leave a folder in the data folder of
	// whoever runs the tests. The folder store is tested on its own.
	assert!(
		format!("{:?}", project.user_data()).contains(name),
		"the folder is named after the project"
	);
	Ok(())
}

#[test]
fn the_manifest_can_choose_the_folder_of_the_data() -> TestResult {
	let name = "ZenithTestChosenFolderA1B2";
	let tmp = tempfile::tempdir()?;
	let root = tmp.path().join(name);
	Project::create(&root)?;
	let manifest = root.join(format!("{name}.zenithproject"));
	let text = std::fs::read_to_string(&manifest)?;
	std::fs::write(
		&manifest,
		format!("{text}user_data = \"ZenithTestStudio/Other Name\"\n"),
	)?;

	let project = Project::open(&root)?;
	assert_eq!(
		project.manifest().user_data_folder(),
		"ZenithTestStudio/Other Name",
		"read from the manifest"
	);
	let store = format!("{:?}", project.user_data());
	assert!(
		store.contains("ZenithTestStudio") && store.contains("Other Name"),
		"the store is in that folder: {store}"
	);
	assert!(
		!store.contains(name),
		"and not in the one named after the project: {store}"
	);
	Ok(())
}

#[test]
fn a_folder_store_is_used_through_the_project() -> TestResult {
	let tmp = tempfile::tempdir()?;
	let saves = tmp.path().join("player");
	let project =
		Project::open_archive_bytes(build()?, &OpenOptions::new().user_data(Arc::new(DirStore::new(&saves))))?;
	project.write(&z("Data://Saves/one.bin")?, b"1")?;
	assert_eq!(
		std::fs::read(saves.join("Saves").join("one.bin"))?,
		b"1",
		"a real file in the folder"
	);
	Ok(())
}
