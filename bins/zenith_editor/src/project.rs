//! The project the editor has open and the worlds in it.

use std::{
	collections::BTreeMap,
	fmt,
	path::{Path, PathBuf},
};

use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{EntryKind, Project, ZPath};
use zenith_world::World;

/// The scheme of the folder with the worlds (`Assets/Worlds`).
const WORLDS_SCHEME: &str = "Worlds";
/// What a file of a world ends with.
pub const WORLD_EXTENSION: &str = ".zenith";
/// How deep below `Worlds/` the editor looks for worlds.
const MAX_DEPTH: usize = 16;
const LONGEST_NAME: usize = 64;

/// A world of the project, and whether it has changes that are not in its file.
pub struct WorldEntry {
	pub world: World,
	pub dirty: bool,
}

/// Something that went wrong with one world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldProblem {
	pub name: String,
	pub error: String,
}

/// What [`OpenProject::save_dirty`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveReport {
	/// Written, so no longer dirty.
	pub saved: Vec<String>,
	/// Not written: they stay dirty.
	pub failed: Vec<WorldProblem>,
}

/// Why a world cannot be created with the name it was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewWorldError {
	EmptyName,
	BadName(String),
	BadFolder(String),
	AlreadyExists(String),
}

impl fmt::Display for NewWorldError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::EmptyName => write!(f, "The world needs a name."),
			Self::BadName(why) => write!(f, "The name is not allowed: {why}."),
			Self::BadFolder(why) => write!(f, "The folder is not allowed: {why}."),
			Self::AlreadyExists(name) => write!(f, "There is a world called {name} already."),
		}
	}
}

impl std::error::Error for NewWorldError {}

/// A folder project and the worlds in it, by name: the path below `Worlds/`
/// without `.zenith` (`Level1`, `Caves/Deep`).
pub struct OpenProject {
	pub project: Project,
	pub path: PathBuf,
	pub worlds: BTreeMap<String, WorldEntry>,
	/// The worlds that were in `Worlds/` but could not be read.
	pub load_errors: Vec<WorldProblem>,
}

impl OpenProject {
	/// Opens the project in the folder and reads every world in `Worlds/`. A
	/// world that cannot be read is listed in `load_errors`, the others load.
	///
	/// # Errors
	/// The error of [`Project::open`], or of listing `Worlds/`.
	pub fn open(path: impl AsRef<Path>) -> ZResult<Self> {
		let path = path.as_ref().to_path_buf();
		let project = Project::open(&path)?;
		let mut names = Vec::new();
		scan(&project, "", 0, &mut names)?;
		let mut worlds = BTreeMap::new();
		let mut load_errors = Vec::new();
		for name in names {
			match load_world(&project, &name) {
				Ok(world) => {
					worlds.insert(name, WorldEntry { world, dirty: false });
				}
				Err(e) => load_errors.push(WorldProblem {
					name,
					error: e.to_string(),
				}),
			}
		}
		Ok(Self {
			project,
			path,
			worlds,
			load_errors,
		})
	}

	/// The names of the worlds with changes that are not saved.
	#[must_use]
	pub fn dirty_names(&self) -> Vec<String> {
		self.worlds
			.iter()
			.filter(|(_, entry)| entry.dirty)
			.map(|(name, _)| name.clone())
			.collect()
	}

	#[must_use]
	pub fn has_dirty(&self) -> bool {
		self.worlds.values().any(|entry| entry.dirty)
	}

	/// Says that every world has changes (for testing the saving, until the
	/// editor changes worlds itself).
	pub fn mark_all_dirty(&mut self) {
		for entry in self.worlds.values_mut() {
			entry.dirty = true;
		}
	}

	/// Writes the worlds that have changes, and only them. One that cannot be
	/// written does not stop the others; it stays dirty and is in the report.
	pub fn save_dirty(&mut self) -> SaveReport {
		let mut report = SaveReport::default();
		for (name, entry) in &mut self.worlds {
			if !entry.dirty {
				continue;
			}
			match write_world(&self.project, name, &mut entry.world) {
				Ok(()) => {
					entry.dirty = false;
					report.saved.push(name.clone());
				}
				Err(e) => report.failed.push(WorldProblem {
					name: name.clone(),
					error: e.to_string(),
				}),
			}
		}
		report
	}

	/// Makes a new empty world, in memory and dirty: its file appears when it
	/// is saved. `folder` is optional and below `Worlds/`. Returns its name.
	///
	/// # Errors
	/// [`NewWorldError`] for a name that is missing, not allowed, or taken.
	pub fn create_world(&mut self, folder: &str, name: &str) -> Result<String, NewWorldError> {
		let name = name.trim();
		let folder = folder.trim().trim_matches('/');
		if name.is_empty() {
			return Err(NewWorldError::EmptyName);
		}
		check_segment(name).map_err(|why| NewWorldError::BadName(why.to_string()))?;
		if !folder.is_empty() {
			for segment in folder.split('/') {
				check_segment(segment).map_err(|why| NewWorldError::BadFolder(why.to_string()))?;
			}
		}
		let key = if folder.is_empty() {
			name.to_string()
		} else {
			format!("{folder}/{name}")
		};
		let on_disk = world_path(&key)
			.and_then(|path| self.project.exists(&path))
			.unwrap_or(false);
		if self.worlds.contains_key(&key) || on_disk {
			return Err(NewWorldError::AlreadyExists(key));
		}
		self.worlds.insert(
			key.clone(),
			WorldEntry {
				world: World::new(name),
				dirty: true,
			},
		);
		Ok(key)
	}
}

/// One part of a name or folder: letters, digits, spaces, `_` and `-`, up to
/// 64, not beginning or ending with a space.
///
/// # Errors
/// What is wrong with it.
pub fn check_segment(segment: &str) -> Result<(), &'static str> {
	if segment.is_empty() || segment.chars().count() > LONGEST_NAME {
		return Err("it must be 1 to 64 characters");
	}
	if !segment
		.chars()
		.all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-'))
	{
		return Err("only letters, digits, spaces, `_` and `-` are allowed");
	}
	if segment.starts_with(' ') || segment.ends_with(' ') {
		return Err("it cannot begin or end with a space");
	}
	Ok(())
}

/// A name of a world as a whole (`Caves/Deep`): every part must be fine.
///
/// # Errors
/// What is wrong with it.
pub fn check_world_key(key: &str) -> Result<(), &'static str> {
	if key.is_empty() {
		return Err("it is empty");
	}
	key.split('/').try_for_each(check_segment)
}

/// `Worlds://<name>.zenith`.
///
/// # Errors
/// `InvalidPath` for a name the project does not take.
pub fn world_path(name: &str) -> ZResult<ZPath> {
	ZPath::from_parts(WORLDS_SCHEME, &format!("{name}{WORLD_EXTENSION}"))
}

/// Reads one world of the project.
///
/// # Errors
/// The error of reading it, or `Yaml` if it is not a world.
pub fn load_world(project: &Project, name: &str) -> ZResult<World> {
	let bytes = project.read_bytes(&world_path(name)?)?;
	let mut world = World::new(name.rsplit('/').next().unwrap_or(name));
	world.restore_bytes(&bytes)?;
	Ok(world)
}

fn write_world(project: &Project, name: &str, world: &mut World) -> ZResult<()> {
	let bytes = world.snapshot_bytes()?;
	if let Some((folder, _)) = name.rsplit_once('/') {
		let folder = ZPath::from_parts(WORLDS_SCHEME, folder)?;
		if !project.exists(&folder)? {
			project.create_dir(&folder)?;
		}
	}
	project.write(&world_path(name)?, &bytes)
}

/// The names of the worlds in `Worlds/<folder>`, below it too.
fn scan(project: &Project, folder: &str, depth: usize, out: &mut Vec<String>) -> ZResult<()> {
	if depth > MAX_DEPTH {
		return Ok(());
	}
	let path = ZPath::from_parts(WORLDS_SCHEME, folder)?;
	let entries = match project.list(&path) {
		Ok(entries) => entries,
		// A project with no worlds yet has no folder for them.
		Err(ZenithError::ProjectNotFound(_)) => return Ok(()),
		Err(e) => return Err(e),
	};
	for entry in entries {
		let relative = if folder.is_empty() {
			entry.name.clone()
		} else {
			format!("{folder}/{}", entry.name)
		};
		match entry.kind {
			EntryKind::Dir => scan(project, &relative, depth + 1, out)?,
			EntryKind::File => {
				if let Some(name) = relative.strip_suffix(WORLD_EXTENSION)
					&& check_world_key(name).is_ok()
				{
					out.push(name.to_string());
				}
			}
			EntryKind::Symlink => {}
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	type TestResult = Result<(), Box<dyn std::error::Error>>;

	pub fn project_with(worlds: &[&str]) -> Result<(tempfile::TempDir, OpenProject), Box<dyn std::error::Error>> {
		let tmp = tempfile::tempdir()?;
		let path = tmp.path().join("Game");
		Project::create(&path)?;
		let mut open = OpenProject::open(&path)?;
		for name in worlds {
			let (folder, leaf) = name.rsplit_once('/').unwrap_or(("", name));
			open.create_world(folder, leaf)?;
		}
		let report = open.save_dirty();
		assert_eq!(report.failed, Vec::new());
		let open = OpenProject::open(&path)?;
		Ok((tmp, open))
	}

	#[test]
	fn a_new_project_has_no_worlds() -> TestResult {
		let (_tmp, open) = project_with(&[])?;
		assert!(open.worlds.is_empty() && open.load_errors.is_empty() && !open.has_dirty());
		Ok(())
	}

	#[test]
	fn new_worlds_are_dirty_until_saved_and_found_after() -> TestResult {
		let (tmp, mut open) = project_with(&[])?;
		assert_eq!(open.create_world("", "Level1")?, "Level1");
		assert_eq!(open.create_world("Caves", "Deep")?, "Caves/Deep");
		assert_eq!(open.dirty_names(), ["Caves/Deep", "Level1"], "in order, both dirty");
		assert!(
			!open.path.join("Assets/Worlds/Level1.zenith").exists(),
			"nothing is on disk before saving"
		);

		let report = open.save_dirty();
		assert_eq!(report.saved, ["Caves/Deep", "Level1"]);
		assert_eq!(report.failed, Vec::new());
		assert!(!open.has_dirty(), "saved ones are not dirty");
		assert!(
			open.path.join("Assets/Worlds/Caves/Deep.zenith").is_file(),
			"in the folder"
		);

		let again = OpenProject::open(tmp.path().join("Game"))?;
		let names: Vec<&str> = again.worlds.keys().map(String::as_str).collect();
		assert_eq!(names, ["Caves/Deep", "Level1"], "both are found");
		Ok(())
	}

	#[test]
	fn only_dirty_worlds_are_written() -> TestResult {
		let (_tmp, mut open) = project_with(&["A", "B"])?;
		assert!(!open.has_dirty());
		assert_eq!(open.save_dirty(), SaveReport::default(), "nothing to do");

		let a = open.path.join("Assets/Worlds/A.zenith");
		let b = open.path.join("Assets/Worlds/B.zenith");
		std::fs::write(&a, "changed by someone else")?;
		std::fs::write(&b, "changed by someone else")?;
		if let Some(entry) = open.worlds.get_mut("A") {
			entry.dirty = true;
		}
		let report = open.save_dirty();
		assert_eq!(report.saved, ["A"]);
		assert_ne!(std::fs::read_to_string(&a)?, "changed by someone else", "A is written");
		assert_eq!(
			std::fs::read_to_string(&b)?,
			"changed by someone else",
			"B is not touched"
		);
		Ok(())
	}

	#[test]
	fn a_world_that_cannot_be_read_is_reported_and_the_others_load() -> TestResult {
		let (tmp, open) = project_with(&["Good"])?;
		std::fs::write(open.path.join("Assets/Worlds/Bad.zenith"), "this: [is not a world")?;
		std::fs::write(open.path.join("Assets/Worlds/notes.txt"), "not a world file")?;
		std::fs::write(open.path.join("Assets/Worlds/bad name!.zenith"), "x")?;
		let again = OpenProject::open(tmp.path().join("Game"))?;
		assert_eq!(again.worlds.keys().collect::<Vec<_>>(), ["Good"]);
		assert_eq!(again.load_errors.len(), 1, "{:?}", again.load_errors);
		assert_eq!(again.load_errors[0].name, "Bad");
		Ok(())
	}

	#[test]
	fn names_are_checked() -> TestResult {
		let (_tmp, mut open) = project_with(&["Taken"])?;
		assert_eq!(open.create_world("", "  "), Err(NewWorldError::EmptyName));
		for bad in ["a/b", "a.b", "..", "a\\b", "x:y", &"n".repeat(65)] {
			assert!(
				matches!(open.create_world("", bad), Err(NewWorldError::BadName(_))),
				"{bad:?} as a name"
			);
		}
		for bad in ["..", "a//b", "a/../b", "a.b", "C:"] {
			assert!(
				matches!(open.create_world(bad, "W"), Err(NewWorldError::BadFolder(_))),
				"{bad:?} as a folder"
			);
		}
		assert_eq!(
			open.create_world("", "Taken"),
			Err(NewWorldError::AlreadyExists("Taken".into())),
			"the name of a world that is open"
		);
		assert_eq!(open.create_world("/Sub/", " Fine ")?, "Sub/Fine", "trimmed");
		assert_eq!(
			open.create_world("Sub", "Fine"),
			Err(NewWorldError::AlreadyExists("Sub/Fine".into())),
			"and of one that was just made"
		);
		// A file that is there but did not load still holds its name.
		std::fs::write(open.path.join("Assets/Worlds/Broken.zenith"), "{")?;
		assert_eq!(
			open.create_world("", "Broken"),
			Err(NewWorldError::AlreadyExists("Broken".into())),
			"a file that is on disk"
		);
		Ok(())
	}

	#[test]
	fn mark_all_dirty_makes_everything_dirty() -> TestResult {
		let (_tmp, mut open) = project_with(&["A", "B"])?;
		open.mark_all_dirty();
		assert_eq!(open.dirty_names(), ["A", "B"]);
		Ok(())
	}

	#[test]
	fn world_keys_are_checked_as_a_whole() {
		assert!(check_world_key("Caves/Deep").is_ok());
		for bad in ["", "/a", "a/", "a//b", "../a", "a/..", "a.b"] {
			assert!(check_world_key(bad).is_err(), "{bad:?}");
		}
	}
}
