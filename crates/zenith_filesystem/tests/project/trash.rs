use zenith_error::ZenithError;
use zenith_filesystem::{EntryKind, OnConflict, OpenOptions, Project, RestoreOptions};

use crate::common::{TestResult, names, new_project, z};

#[test]
fn remove_moves_into_the_documented_layout() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://tex/ferris.png")?, b"12345")?;
	let id = project.remove(&z("Assets://tex/ferris.png")?)?;

	assert!(!root.join("Assets/tex/ferris.png").exists(), "the file left its place");
	let entry_dir = root.join("Trash").join(id.as_str());
	assert_eq!(names(&entry_dir), vec!["entry.toml", "files"], "entry folder layout");
	assert_eq!(
		std::fs::read(entry_dir.join("files/ferris.png"))?,
		b"12345",
		"the object keeps its name"
	);

	let entry: toml::Table = std::fs::read_to_string(entry_dir.join("entry.toml"))?.parse()?;
	assert_eq!(
		entry["original"].as_str(),
		Some("Assets://tex/ferris.png"),
		"original is a ZPath, not a disk path"
	);
	assert_eq!(entry["kind"].as_str(), Some("file"), "kind");
	assert_eq!(entry["size"].as_integer(), Some(5), "size");
	assert_eq!(entry["ignored"].as_bool(), Some(false), "ignored flag");
	assert!(
		entry["deleted_at_ms"]
			.as_integer()
			.is_some_and(|ms| ms > 1_600_000_000_000),
		"timestamp in ms"
	);
	Ok(())
}

#[test]
fn list_is_newest_first_and_find_filters() -> TestResult {
	let (_tmp, _root, project) = new_project("Game")?;
	for name in ["a", "b", "c"] {
		project.write(&z(&format!("Assets://{name}"))?, name.as_bytes())?;
	}
	let (a, b, c) = (
		project.remove(&z("Assets://a")?)?,
		project.remove(&z("Assets://b")?)?,
		project.remove(&z("Assets://c")?)?,
	);
	let listed: Vec<_> = project.trash_list()?.into_iter().map(|e| e.id).collect();
	assert_eq!(listed, vec![c, b, a], "newest first, even within one millisecond");

	project.write(&z("Assets://a")?, b"again")?;
	let a2 = project.remove(&z("Assets://a")?)?;
	let found: Vec<_> = project
		.trash_find(&z("Assets://a")?)?
		.into_iter()
		.map(|e| e.id)
		.collect();
	assert_eq!(found.len(), 2, "both removals of Assets://a");
	assert_eq!(found.first(), Some(&a2), "newest first");
	assert!(
		project.trash_find(&z("Assets://zzz")?)?.is_empty(),
		"nothing for an unknown path"
	);
	assert_eq!(
		project.trash_list()?.first().map(|e| e.kind),
		Some(EntryKind::File),
		"kind is reported"
	);
	Ok(())
}

#[test]
fn restore_puts_things_back_and_recreates_parents() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://deep/er/f.txt")?, b"x")?;
	let id = project.remove(&z("Assets://deep/er/f.txt")?)?;
	project.remove(&z("Assets://deep")?)?;
	assert!(!root.join("Assets/deep").exists(), "the parents are gone too");

	let restored = project.restore(&id)?;
	assert_eq!(
		restored.to_string(),
		"Assets://deep/er/f.txt",
		"the restored location is returned"
	);
	assert_eq!(
		std::fs::read(root.join("Assets/deep/er/f.txt"))?,
		b"x",
		"missing parents were created"
	);
	assert!(
		!root.join("Trash").join(id.as_str()).exists(),
		"the entry folder is deleted after a restore"
	);
	assert!(
		matches!(project.restore(&id), Err(ZenithError::TrashEntryNotFound(_))),
		"an id works once"
	);
	Ok(())
}

#[test]
fn folders_survive_a_round_trip_with_their_size() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://dir/a")?, b"123")?;
	project.write(&z("Assets://dir/sub/b")?, b"4567")?;
	let id = project.remove(&z("Assets://dir")?)?;
	let entry = project.trash_list()?.into_iter().next().ok_or("no entry")?;
	assert_eq!(
		(entry.kind, entry.size),
		(EntryKind::Dir, 7),
		"a folder entry sums its files"
	);
	project.restore(&id)?;
	assert_eq!(
		std::fs::read(root.join("Assets/dir/sub/b"))?,
		b"4567",
		"contents are intact"
	);
	Ok(())
}

#[test]
fn restore_last_takes_the_newest() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	assert!(
		matches!(project.restore_last(), Err(ZenithError::TrashEntryNotFound(_))),
		"empty trash"
	);
	project.write(&z("Assets://old")?, b"old")?;
	project.write(&z("Assets://new")?, b"new")?;
	project.remove(&z("Assets://old")?)?;
	project.remove(&z("Assets://new")?)?;
	assert_eq!(
		project.restore_last()?.to_string(),
		"Assets://new",
		"the newest comes back"
	);
	assert!(
		root.join("Assets/new").exists() && !root.join("Assets/old").exists(),
		"and only it"
	);
	Ok(())
}

#[test]
fn conflicts_fail_keep_both_or_overwrite() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://tex/ferris.png")?, b"v1")?;
	let id = project.remove(&z("Assets://tex/ferris.png")?)?;
	project.write(&z("Assets://tex/ferris.png")?, b"v2")?;

	assert!(
		matches!(project.restore(&id), Err(ZenithError::ProjectAlreadyExists(_))),
		"Fail is the default"
	);
	assert_eq!(
		std::fs::read(root.join("Assets/tex/ferris.png"))?,
		b"v2",
		"and changes nothing"
	);
	assert_eq!(project.trash_list()?.len(), 1, "the entry is still in the trash");

	let both = project.restore_with(
		&id,
		&RestoreOptions {
			to: None,
			on_conflict: OnConflict::KeepBoth,
		},
	)?;
	assert_eq!(
		both.to_string(),
		"Assets://tex/ferris-1.png",
		"a numeric suffix before the extension"
	);
	assert_eq!(
		std::fs::read(root.join("Assets/tex/ferris-1.png"))?,
		b"v1",
		"restored under the new name"
	);
	assert_eq!(
		std::fs::read(root.join("Assets/tex/ferris.png"))?,
		b"v2",
		"the existing file is untouched"
	);

	let id2 = project.remove(&z("Assets://tex/ferris-1.png")?)?;
	let over = project.restore_with(
		&id2,
		&RestoreOptions {
			to: Some(z("Assets://tex/ferris.png")?),
			on_conflict: OnConflict::Overwrite,
		},
	)?;
	assert_eq!(
		over.to_string(),
		"Assets://tex/ferris.png",
		"restored to the requested place"
	);
	assert_eq!(
		std::fs::read(root.join("Assets/tex/ferris.png"))?,
		b"v1",
		"the restored file won"
	);
	let displaced = project.trash_find(&z("Assets://tex/ferris.png")?)?;
	assert_eq!(
		displaced.len(),
		1,
		"the overwritten file went to the trash, nothing is lost for good"
	);
	let displaced_id = displaced.first().map(|e| e.id.clone()).ok_or("no entry")?;
	assert_eq!(
		std::fs::read(root.join("Trash").join(displaced_id.as_str()).join("files/ferris.png"))?,
		b"v2",
		"with its content"
	);
	Ok(())
}

#[test]
fn restore_to_another_place() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://a.txt")?, b"a")?;
	let id = project.remove(&z("Assets://a.txt")?)?;
	let at = project.restore_with(
		&id,
		&RestoreOptions {
			to: Some(z("Settings://elsewhere/a.txt")?),
			on_conflict: OnConflict::Fail,
		},
	)?;
	assert_eq!(at.to_string(), "Settings://elsewhere/a.txt", "returned location");
	assert_eq!(
		std::fs::read(root.join("Settings/elsewhere/a.txt"))?,
		b"a",
		"restored there"
	);
	Ok(())
}

#[test]
fn a_scheme_that_is_gone_blocks_restore_but_not_restore_with_to() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.register_scheme("textures", "Assets/Textures")?;
	project.write(&z("textures://a.png")?, b"png")?;
	let id = project.remove(&z("textures://a.png")?)?;
	drop(project);

	let reopened = Project::open(&root)?;
	assert!(
		matches!(reopened.restore(&id), Err(ZenithError::SchemeNotRegistered(s)) if s == "Textures"),
		"the original scheme is not registered any more"
	);
	let at = reopened.restore_with(
		&id,
		&RestoreOptions {
			to: Some(z("Assets://a.png")?),
			on_conflict: OnConflict::Fail,
		},
	)?;
	assert_eq!(at.to_string(), "Assets://a.png", "an explicit target still works");
	Ok(())
}

#[test]
fn purge_is_the_only_way_to_delete_for_good() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	for n in ["a", "b", "c"] {
		project.write(&z(&format!("Assets://{n}"))?, b"x")?;
	}
	let a = project.remove(&z("Assets://a")?)?;
	project.remove(&z("Assets://b")?)?;
	project.remove(&z("Assets://c")?)?;

	project.purge(&a)?;
	assert!(!root.join("Trash").join(a.as_str()).exists(), "purge deletes the entry");
	assert_eq!(project.trash_list()?.len(), 2, "others stay");
	assert!(
		matches!(project.purge(&a), Err(ZenithError::TrashEntryNotFound(_))),
		"second purge"
	);

	// Something that is not an entry belongs to whoever put it there; an
	// entry folder that is damaged (named like one, nothing inside) is ours.
	std::fs::create_dir(root.join("Trash/junk"))?;
	std::fs::write(root.join("Trash/stray.txt"), b"x")?;
	std::fs::create_dir(root.join("Trash/0000000000000001-aaaaaaaa"))?;
	project.purge_all()?;
	assert_eq!(
		names(&root.join("Trash")),
		vec!["junk", "stray.txt"],
		"purge_all deletes the entries, damaged ones too, and nothing else"
	);
	Ok(())
}

#[test]
fn an_entry_without_an_object_is_skipped_and_cleaned() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://a")?, b"x")?;
	let id = project.remove(&z("Assets://a")?)?;
	std::fs::remove_file(root.join("Trash").join(id.as_str()).join("files/a"))?;
	assert!(
		project.trash_list()?.is_empty(),
		"no object, no entry (as after a crash during remove)"
	);
	assert!(
		matches!(project.restore(&id), Err(ZenithError::TrashEntryNotFound(_))),
		"cannot be restored"
	);
	project.purge_all()?;
	assert!(names(&root.join("Trash")).is_empty(), "purge_all cleans it");
	Ok(())
}

#[test]
fn a_file_named_entry_toml_is_no_problem() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://entry.toml")?, b"mine")?;
	let id = project.remove(&z("Assets://entry.toml")?)?;
	project.restore(&id)?;
	assert_eq!(
		std::fs::read(root.join("Assets/entry.toml"))?,
		b"mine",
		"the object lives under files/, away from the metadata"
	);
	Ok(())
}

#[test]
fn the_trash_survives_moving_the_project_folder() -> TestResult {
	let (tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://a.txt")?, b"a")?;
	let id = project.remove(&z("Assets://a.txt")?)?;
	drop(project);

	let elsewhere = tmp.path().join("deeper");
	std::fs::create_dir(&elsewhere)?;
	let moved = elsewhere.join("Game");
	std::fs::rename(&root, &moved)?;
	let project = Project::open(&moved)?;
	project.restore(&id)?;
	assert_eq!(
		std::fs::read(moved.join("Assets/a.txt"))?,
		b"a",
		"the original is a ZPath, so a new location changes nothing"
	);
	Ok(())
}

#[test]
fn scheme_roots_cannot_be_removed() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	for root_path in ["Assets://", "Worlds://", "Settings://"] {
		assert!(
			matches!(project.remove(&z(root_path)?), Err(ZenithError::Unsupported(_))),
			"{root_path} must not be removable"
		);
	}
	assert!(root.join("Assets/Worlds").is_dir(), "folders are intact");
	assert!(
		matches!(
			project.remove(&z("Assets://nothing")?),
			Err(ZenithError::ProjectNotFound(_))
		),
		"missing path"
	);
	Ok(())
}

#[test]
fn the_trash_scheme_is_read_only() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	project.write(&z("Assets://a.txt")?, b"text")?;
	let id = project.remove(&z("Assets://a.txt")?)?;
	let inside = z(&format!("Trash://{id}/files/a.txt"))?;

	assert!(project.exists(&inside)?, "exists");
	assert_eq!(project.metadata(&inside)?.size, 4, "metadata");
	assert_eq!(project.list(&z(&format!("Trash://{id}"))?)?.len(), 2, "list");
	assert_eq!(project.read_string(&inside)?, "text", "read_string");
	assert_eq!(&*project.read_bytes(&inside)?, b"text", "read_bytes");
	assert!(project.open_read(&inside).is_ok(), "open_read");
	project.copy(&inside, &z("Assets://copied.txt")?)?;
	assert_eq!(
		std::fs::read(root.join("Assets/copied.txt"))?,
		b"text",
		"copy out of the trash works"
	);

	let readonly = |r: Result<(), ZenithError>| matches!(r, Err(ZenithError::ReadOnlyScheme(_)));
	assert!(readonly(project.write(&inside, b"x")), "write");
	assert!(readonly(project.create_dir(&z("Trash://new")?)), "create_dir");
	assert!(readonly(project.remove(&inside).map(|_| ())), "remove");
	assert!(
		readonly(project.copy(&z("Assets://copied.txt")?, &z("Trash://x")?)),
		"copy into the trash"
	);
	assert!(
		readonly(project.move_to(&inside, &z("Assets://m.txt")?)),
		"move out of the trash"
	);
	assert!(
		readonly(project.move_to(&z("Assets://copied.txt")?, &z("Trash://m.txt")?)),
		"move into the trash"
	);

	project.register_scheme("sneaky", "Trash")?;
	assert!(
		readonly(project.write(&z("sneaky://x")?, b"x")),
		"another scheme rooted in the trash is read-only too"
	);
	Ok(())
}

#[test]
fn removing_and_restoring_invalidate_the_cache() -> TestResult {
	let (_tmp, root, project) = new_project("Game")?;
	let path = z("Assets://cached.txt")?;
	project.write(&path, b"one")?;
	for _ in 0..3 {
		project.read_bytes(&path)?;
	}
	let id = project.remove(&path)?;
	assert!(
		matches!(project.read_bytes(&path), Err(ZenithError::ProjectNotFound(_))),
		"a removed file is not served from the cache"
	);
	project.restore(&id)?;
	std::fs::write(root.join("Assets/cached.txt"), b"two!")?;
	assert_eq!(&*project.read_bytes(&path)?, b"two!", "a restored file is read fresh");
	let _ = OpenOptions::new();
	Ok(())
}
