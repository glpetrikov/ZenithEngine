//! The backup of the changes that were not saved: `.zenitheditor-backup` in the
//! folder of the project.
//!
//! The editor writes it when it is stopped from outside (Ctrl+C), and offers it
//! when the project is opened next: the worlds in it go to the project and the
//! backup is deleted. It holds the worlds that were dirty, as files, in the
//! layout of the project (`Worlds/Caves/Deep.zenith`), and `Backup.toml` which
//! lists them and says when it was made. `Backup.toml` is written last, so a
//! backup that has it is complete.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use web_time::{SystemTime, UNIX_EPOCH};
use zenith_error::{ZResult, ZenithError};

use crate::project::{OpenProject, WORLD_EXTENSION, WorldEntry, WorldProblem, check_world_key, world_path};

/// The folder of the backup, in the folder of the project.
pub const BACKUP_DIR: &str = zenith_filesystem::layout::DIR_EDITOR_BACKUP;
const MANIFEST: &str = "Backup.toml";
const WORLDS_DIR: &str = "Worlds";
const TEMP_SUFFIX: &str = ".zenith-tmp";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Manifest {
	saved_at_ms: u64,
	worlds: Vec<String>,
}

/// What a backup holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupInfo {
	/// When it was made, milliseconds since 1970.
	pub saved_at_ms: u64,
	/// The worlds in it, by name.
	pub worlds: Vec<String>,
}

/// What [`restore`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreReport {
	/// In the project now (written, not dirty); gone from the backup.
	pub restored: Vec<String>,
	/// Still only in the backup.
	pub failed: Vec<WorldProblem>,
}

fn dir(project: &Path) -> PathBuf {
	project.join(BACKUP_DIR)
}

fn world_file(project: &Path, name: &str) -> PathBuf {
	dir(project).join(WORLDS_DIR).join(format!("{name}{WORLD_EXTENSION}"))
}

fn now_ms() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
}

/// A file replaced whole: written beside it, then renamed over it.
fn write_file(file: &Path, bytes: &[u8]) -> ZResult<()> {
	if let Some(folder) = file.parent() {
		std::fs::create_dir_all(folder)?;
	}
	let mut temp = file.as_os_str().to_owned();
	temp.push(TEMP_SUFFIX);
	let temp = PathBuf::from(temp);
	std::fs::write(&temp, bytes)?;
	std::fs::rename(&temp, file)?;
	Ok(())
}

fn write_manifest(project: &Path, worlds: Vec<String>) -> ZResult<()> {
	let text = toml::to_string(&Manifest {
		saved_at_ms: now_ms(),
		worlds,
	})
	.map_err(|e| ZenithError::Toml(e.to_string()))?;
	write_file(&dir(project).join(MANIFEST), text.as_bytes())
}

fn read_manifest(project: &Path) -> ZResult<Option<Manifest>> {
	let text = match std::fs::read_to_string(dir(project).join(MANIFEST)) {
		Ok(text) => text,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
		Err(e) => return Err(e.into()),
	};
	toml::from_str(&text)
		.map(Some)
		.map_err(|e| ZenithError::Toml(format!("{BACKUP_DIR}/{MANIFEST}: {e}")))
}

/// The backup in the folder of the project, if there is one.
///
/// # Errors
/// `Toml` if `Backup.toml` cannot be understood, or the error of reading it.
pub fn find(project: &Path) -> ZResult<Option<BackupInfo>> {
	Ok(read_manifest(project)?.map(|manifest| BackupInfo {
		saved_at_ms: manifest.saved_at_ms,
		worlds: manifest
			.worlds
			.into_iter()
			.filter(|w| check_world_key(w).is_ok())
			.collect(),
	}))
}

/// Writes the dirty worlds of the project to the backup. A backup that is
/// already there keeps the worlds this one does not replace. Nothing dirty
/// writes nothing. Returns the names of the worlds written.
///
/// # Errors
/// The error of writing a file; worlds before it are in the backup but not
/// listed, so they are not offered.
pub fn write(open: &mut OpenProject) -> ZResult<Vec<String>> {
	let mut written = Vec::new();
	for (name, entry) in &mut open.worlds {
		if entry.dirty {
			let bytes = entry.world.snapshot_bytes()?;
			write_file(&world_file(&open.path, name), &bytes)?;
			written.push(name.clone());
		}
	}
	if written.is_empty() {
		return Ok(written);
	}
	let mut listed = find(&open.path)
		.ok()
		.flatten()
		.map(|info| info.worlds)
		.unwrap_or_default();
	for name in &written {
		if !listed.contains(name) {
			listed.push(name.clone());
		}
	}
	listed.sort();
	write_manifest(&open.path, listed)?;
	Ok(written)
}

/// Puts the worlds of the backup in the project: each is written to its file
/// (and so is not dirty) and leaves the backup. One that cannot be used stays
/// in the backup and is in the report. The backup is deleted when it is empty.
///
/// # Errors
/// The error of reading or rewriting `Backup.toml`.
pub fn restore(open: &mut OpenProject) -> ZResult<RestoreReport> {
	let mut report = RestoreReport::default();
	let Some(info) = find(&open.path)? else {
		return Ok(report);
	};
	for name in info.worlds {
		let result = (|| -> ZResult<()> {
			let file = world_file(&open.path, &name);
			let bytes = std::fs::read(&file)?;
			// Understood before it takes the place of anything.
			let mut world = zenith_world::World::new(name.rsplit('/').next().unwrap_or(&name));
			world.restore_bytes(&bytes)?;
			if let Some((folder, _)) = name.rsplit_once('/') {
				let folder = zenith_filesystem::ZPath::from_parts("Worlds", folder)?;
				if !open.project.exists(&folder)? {
					open.project.create_dir(&folder)?;
				}
			}
			open.project.write(&world_path(&name)?, &bytes)?;
			open.worlds.insert(name.clone(), WorldEntry { world, dirty: false });
			std::fs::remove_file(&file)?;
			Ok(())
		})();
		match result {
			Ok(()) => report.restored.push(name),
			Err(e) => report.failed.push(WorldProblem {
				name,
				error: e.to_string(),
			}),
		}
	}
	if report.failed.is_empty() {
		discard(&open.path)?;
	} else {
		write_manifest(&open.path, report.failed.iter().map(|p| p.name.clone()).collect())?;
	}
	Ok(report)
}

/// Deletes the backup.
///
/// # Errors
/// The error of deleting the folder.
pub fn discard(project: &Path) -> ZResult<()> {
	match std::fs::remove_dir_all(dir(project)) {
		Ok(()) => Ok(()),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(e) => Err(e.into()),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::project::OpenProject;

	type TestResult = Result<(), Box<dyn std::error::Error>>;

	fn open_with(names: &[&str]) -> Result<(tempfile::TempDir, OpenProject), Box<dyn std::error::Error>> {
		let tmp = tempfile::tempdir()?;
		let path = tmp.path().join("Game");
		zenith_filesystem::Project::create(&path)?;
		let mut open = OpenProject::open(&path)?;
		for name in names {
			let (folder, leaf) = name.rsplit_once('/').unwrap_or(("", name));
			open.create_world(folder, leaf)?;
		}
		open.save_dirty();
		Ok((tmp, open))
	}

	fn dirty(open: &mut OpenProject, names: &[&str]) {
		for name in names {
			if let Some(entry) = open.worlds.get_mut(*name) {
				entry.dirty = true;
			}
		}
	}

	#[test]
	fn the_dirty_worlds_are_backed_up_and_the_project_is_not_touched() -> TestResult {
		let (_tmp, mut open) = open_with(&["A", "Caves/B", "C"])?;
		let before = std::fs::read(open.path.join("Assets/Worlds/A.zenith"))?;
		assert!(find(&open.path)?.is_none(), "no backup yet");
		assert!(write(&mut open)?.is_empty(), "nothing is dirty, nothing is written");
		assert!(find(&open.path)?.is_none());

		dirty(&mut open, &["A", "Caves/B"]);
		assert_eq!(write(&mut open)?, ["A", "Caves/B"]);
		let info = find(&open.path)?.ok_or("no backup")?;
		assert_eq!(info.worlds, ["A", "Caves/B"]);
		assert!(info.saved_at_ms > 1_600_000_000_000);
		assert!(world_file(&open.path, "Caves/B").is_file());
		assert!(!world_file(&open.path, "C").exists(), "C was not dirty");
		assert_eq!(
			std::fs::read(open.path.join("Assets/Worlds/A.zenith"))?,
			before,
			"the project is as it was"
		);
		assert!(open.has_dirty(), "and the worlds are still dirty");
		Ok(())
	}

	#[test]
	fn restoring_puts_the_worlds_in_the_project_and_deletes_the_backup() -> TestResult {
		let (tmp, mut open) = open_with(&["A"])?;
		dirty(&mut open, &["A"]);
		open.create_world("New", "W")?;
		write(&mut open)?;

		let mut next = OpenProject::open(tmp.path().join("Game"))?;
		assert!(!next.worlds.contains_key("New/W"), "W was only in memory");
		let report = restore(&mut next)?;
		assert_eq!(report.restored, ["A", "New/W"]);
		assert_eq!(report.failed, Vec::new());
		assert!(
			next.path.join("Assets/Worlds/New/W.zenith").is_file(),
			"W is in the project now"
		);
		assert!(!next.has_dirty(), "and not dirty: it is written");
		assert!(!next.path.join(BACKUP_DIR).exists(), "the backup is gone");
		assert!(find(&next.path)?.is_none());
		assert_eq!(OpenProject::open(&next.path)?.worlds.len(), 2, "a fresh open sees both");
		Ok(())
	}

	#[test]
	fn a_second_backup_keeps_what_the_first_had() -> TestResult {
		let (_tmp, mut open) = open_with(&["A", "B"])?;
		dirty(&mut open, &["A"]);
		write(&mut open)?;
		for entry in open.worlds.values_mut() {
			entry.dirty = false;
		}
		dirty(&mut open, &["B"]);
		write(&mut open)?;
		assert_eq!(find(&open.path)?.ok_or("none")?.worlds, ["A", "B"], "both are listed");
		Ok(())
	}

	#[test]
	fn a_broken_world_stays_in_the_backup_and_the_others_come_back() -> TestResult {
		let (_tmp, mut open) = open_with(&["A", "B"])?;
		dirty(&mut open, &["A", "B"]);
		write(&mut open)?;
		std::fs::write(world_file(&open.path, "A"), "this: [is not a world")?;
		let report = restore(&mut open)?;
		assert_eq!(report.restored, ["B"]);
		assert_eq!(report.failed.len(), 1);
		assert_eq!(report.failed[0].name, "A");
		assert_eq!(find(&open.path)?.ok_or("none")?.worlds, ["A"], "A is still offered");
		discard(&open.path)?;
		assert!(find(&open.path)?.is_none(), "discarded");
		discard(&open.path)?;
		Ok(())
	}

	#[test]
	fn a_manifest_cannot_point_outside_the_backup() -> TestResult {
		let (_tmp, open) = open_with(&[])?;
		std::fs::create_dir_all(dir(&open.path))?;
		std::fs::write(
			dir(&open.path).join(MANIFEST),
			"saved_at_ms = 1\nworlds = [\"../../Evil\", \"Fine\", \"a/../b\"]\n",
		)?;
		assert_eq!(find(&open.path)?.ok_or("none")?.worlds, ["Fine"], "only good names");
		std::fs::write(dir(&open.path).join(MANIFEST), "this is = not toml [")?;
		assert!(
			matches!(find(&open.path), Err(ZenithError::Toml(_))),
			"a damaged manifest is an error"
		);
		Ok(())
	}
}
