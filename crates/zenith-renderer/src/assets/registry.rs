//! `AssetRegistry`: UUID to path, by scanning the project.
//!
//! Every file under `Assets://` with the extension of an asset is read just
//! enough to know its id: the header of a container, the `uuid` of a JSON
//! file. A build is scanned the same way, through its project.

use std::collections::HashMap;

use bevy_ecs::resource::Resource;
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{EntryKind, Project, ZPath};

use super::{AssetId, AssetKind, container::Header, json};

/// Where an asset is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetEntry {
	pub path: ZPath,
	pub kind: AssetKind,
}

/// What is wrong with the assets of a project. Nothing here stops the game:
/// the asset is found at the other path, or not at all (then it is drawn with
/// a stand-in).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetProblem {
	/// Two files have one id: a file was copied outside the editor, which
	/// gives a copy an id of its own. `kept` is the one used.
	Duplicate { id: AssetId, kept: ZPath, ignored: ZPath },
	/// A file with the extension of an asset whose id cannot be read.
	Unreadable { path: ZPath, error: String },
}

impl AssetProblem {
	/// The file the problem is about.
	pub const fn path(&self) -> &ZPath {
		match self {
			Self::Duplicate { ignored, .. } => ignored,
			Self::Unreadable { path, .. } => path,
		}
	}
}

/// The assets of a project, by id and by path. A resource of the world.
#[derive(Resource, Clone, Debug, Default)]
pub struct AssetRegistry {
	assets: HashMap<AssetId, AssetEntry>,
	ids: HashMap<ZPath, AssetId>,
	problems: Vec<AssetProblem>,
}

/// Where the assets of a project are.
const ROOT: &str = "Assets://";

/// One asset file found by a scan.
struct Found {
	path: ZPath,
	kind: AssetKind,
	id: Result<AssetId, String>,
}

impl AssetRegistry {
	/// Finds every asset of `project`. Two files with one id: the one whose
	/// path comes first (in the order of the bytes of the path) is used.
	///
	/// # Errors
	/// An error of the project while listing a folder.
	pub fn scan(project: &Project) -> ZResult<Self> {
		let mut registry = Self::default();
		registry.rescan(project)?;
		Ok(registry)
	}

	/// Scans again. Of two files with one id, the one the registry already
	/// had keeps it: a copy made outside the editor does not take the place of
	/// the original.
	///
	/// # Errors
	/// An error of the project while listing a folder.
	pub fn rescan(&mut self, project: &Project) -> ZResult<()> {
		let mut found = Vec::new();
		let root = ZPath::new(ROOT)?;
		if project.exists(&root)? {
			walk(project, &root, &mut found)?;
		}
		found.sort_by(|a, b| a.path.cmp(&b.path));

		let old = std::mem::take(self);
		let known = |item: &Found| {
			item.id
				.as_ref()
				.is_ok_and(|id| old.assets.get(id).is_some_and(|entry| entry.path == item.path))
		};
		let (first, rest): (Vec<_>, Vec<_>) = found.into_iter().partition(known);
		for item in first.into_iter().chain(rest) {
			match item.id {
				Ok(id) => self.insert(item.path, item.kind, id),
				Err(error) => self.unreadable(item.path, error),
			}
		}
		Ok(())
	}

	pub fn get(&self, id: AssetId) -> Option<&AssetEntry> {
		self.assets.get(&id)
	}

	/// Where the asset is.
	pub fn path(&self, id: AssetId) -> Option<&ZPath> {
		self.assets.get(&id).map(|entry| &entry.path)
	}

	/// The asset at `path`. `None` for a file the registry does not use (not
	/// an asset, unreadable, or a duplicate).
	pub fn id_of(&self, path: &ZPath) -> Option<AssetId> {
		self.ids.get(path).copied()
	}

	pub fn len(&self) -> usize {
		self.assets.len()
	}

	pub fn is_empty(&self) -> bool {
		self.assets.is_empty()
	}

	/// Every asset, in no order.
	pub fn iter(&self) -> impl Iterator<Item = (AssetId, &AssetEntry)> {
		self.assets.iter().map(|(id, entry)| (*id, entry))
	}

	pub fn problems(&self) -> &[AssetProblem] {
		&self.problems
	}

	/// A file was made or changed (the watcher saw it, the editor wrote it):
	/// its id is read again. Returns the id, if the file is an asset that can
	/// be used.
	///
	/// # Errors
	/// `InvalidZPath` for a path the project cannot hold. A file that cannot
	/// be read is a problem of the registry, not an error.
	pub fn refresh(&mut self, project: &Project, path: &ZPath) -> ZResult<Option<AssetId>> {
		let Some(kind) = AssetKind::of_path(path) else {
			return Ok(None);
		};
		let id = read_id(project, path, kind);
		// What the file was before goes; if it keeps its id it comes back.
		self.forget(path);
		match id {
			Ok(id) => {
				self.insert(path.clone(), kind, id);
				Ok(self.id_of(path))
			}
			Err(e) => {
				self.unreadable(path.clone(), e.to_string());
				Ok(None)
			}
		}
	}

	/// A file is gone. If it held an id that another file has too, that file
	/// takes it.
	pub fn forget(&mut self, path: &ZPath) {
		self.problems.retain(|problem| problem.path() != path);
		let Some(id) = self.ids.remove(path) else {
			return;
		};
		if self.assets.get(&id).is_some_and(|entry| &entry.path == path) {
			self.assets.remove(&id);
			let heir = self.problems.iter().position(
				|problem| matches!(problem, AssetProblem::Duplicate { id: duplicate, .. } if *duplicate == id),
			);
			if let Some(AssetProblem::Duplicate { ignored, .. }) = heir.map(|at| self.problems.remove(at))
				&& let Some(kind) = AssetKind::of_path(&ignored)
			{
				zenith_log::info!("{ignored} has the id {id} now that {path} is gone");
				// A third copy is a duplicate of the heir now.
				for problem in &mut self.problems {
					if let AssetProblem::Duplicate {
						id: duplicate, kept, ..
					} = problem && *duplicate == id
					{
						kept.clone_from(&ignored);
					}
				}
				self.insert(ignored, kind, id);
			}
		}
	}

	/// A file was moved or renamed: it keeps its id, and what refers to it
	/// finds it at `to`.
	pub fn rename(&mut self, from: &ZPath, to: &ZPath) {
		if let Some(id) = self.ids.remove(from) {
			self.ids.insert(to.clone(), id);
			if let Some(entry) = self.assets.get_mut(&id)
				&& &entry.path == from
			{
				entry.path = to.clone();
			}
		}
		for problem in &mut self.problems {
			match problem {
				AssetProblem::Duplicate { kept, ignored, .. } => {
					for path in [kept, ignored] {
						if path == from {
							*path = to.clone();
						}
					}
				}
				AssetProblem::Unreadable { path, .. } => {
					if path == from {
						*path = to.clone();
					}
				}
			}
		}
	}

	/// Puts an asset in. An id the registry already has elsewhere makes a
	/// duplicate: the file it has keeps the id.
	pub(crate) fn insert(&mut self, path: ZPath, kind: AssetKind, id: AssetId) {
		if let Some(kept) = self.assets.get(&id).map(|entry| entry.path.clone())
			&& kept != path
		{
			zenith_log::warn!(
				"{path} has the id of {kept} ({id}): it was copied outside the editor, and is not used until it gets an id of its own"
			);
			self.problems.push(AssetProblem::Duplicate {
				id,
				kept,
				ignored: path,
			});
			return;
		}
		self.ids.insert(path.clone(), id);
		self.assets.insert(id, AssetEntry { path, kind });
	}

	fn unreadable(&mut self, path: ZPath, error: String) {
		zenith_log::warn!("{path} is not a usable asset: {error}");
		self.problems.push(AssetProblem::Unreadable { path, error });
	}
}

/// Every asset file under `folder`. Links are not followed.
fn walk(project: &Project, folder: &ZPath, found: &mut Vec<Found>) -> ZResult<()> {
	for entry in project.list(folder)? {
		let path = folder.join(&entry.name)?;
		match entry.kind {
			EntryKind::Dir => walk(project, &path, found)?,
			EntryKind::File => {
				if let Some(kind) = AssetKind::of_path(&path) {
					let id = read_id(project, &path, kind).map_err(|e| e.to_string());
					found.push(Found { path, kind, id });
				}
			}
			EntryKind::Symlink => {}
		}
	}
	Ok(())
}

/// The id of the asset file at `path`, reading as little of it as it can.
///
/// # Errors
/// An error of the project, or `InvalidAsset` / `Json` for a file that is not
/// an asset of `kind`.
pub fn read_id(project: &Project, path: &ZPath, kind: AssetKind) -> ZResult<AssetId> {
	if kind.is_binary() {
		let header = Header::read(&mut *project.open_read(path)?)?;
		if header.kind != kind {
			return Err(ZenithError::InvalidAsset(format!(
				"{path} holds a {:?}, its extension says {kind:?}",
				header.kind
			)));
		}
		Ok(header.id)
	} else {
		json::read_id(&project.read_string(path)?, kind)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::assets::{container, json::MaterialData};

	const NO_PROBLEMS: &[AssetProblem] = &[];

	fn project() -> ZResult<(tempfile::TempDir, Project)> {
		let dir = tempfile::tempdir()?;
		let project = Project::create(dir.path().join("Game"))?;
		Ok((dir, project))
	}

	#[allow(clippy::expect_used)] // the paths of the tests are written by hand
	fn path(text: &str) -> ZPath {
		ZPath::new(text).expect("a valid path")
	}

	fn id(byte: u8) -> AssetId {
		AssetId::from_bytes([byte; 16])
	}

	fn write_model(project: &Project, at: &str, id: AssetId) -> ZResult<()> {
		project.write(&path(at), &container::write(AssetKind::Model, id, &(), b"v 0 0 0")?)
	}

	fn write_material(project: &Project, at: &str, id: AssetId) -> ZResult<()> {
		project.write(&path(at), MaterialData::new(id).to_text()?.as_bytes())
	}

	#[test]
	fn the_assets_of_a_project_are_found() -> ZResult<()> {
		let (_dir, project) = project()?;
		write_model(&project, "Assets://Models/Cube.zenithmodel", id(1))?;
		write_material(&project, "Assets://Materials/Red.zenithmaterial", id(2))?;
		project.write(&path("Assets://Models/cube.obj"), b"v 0 0 0")?;

		let registry = AssetRegistry::scan(&project)?;
		assert_eq!(registry.len(), 2, "the .obj is not an asset");
		assert_eq!(registry.path(id(1)), Some(&path("Assets://Models/Cube.zenithmodel")));
		assert_eq!(registry.get(id(2)).map(|e| e.kind), Some(AssetKind::Material));
		assert_eq!(
			registry.id_of(&path("Assets://Materials/Red.zenithmaterial")),
			Some(id(2))
		);
		assert_eq!(registry.problems(), NO_PROBLEMS);
		Ok(())
	}

	#[test]
	fn a_project_without_assets_has_none() -> ZResult<()> {
		let (_dir, project) = project()?;
		assert!(AssetRegistry::scan(&project)?.is_empty());
		Ok(())
	}

	#[test]
	fn of_two_files_with_one_id_the_first_or_the_known_one_is_used() -> ZResult<()> {
		let (_dir, project) = project()?;
		write_model(&project, "Assets://B.zenithmodel", id(1))?;
		let mut registry = AssetRegistry::scan(&project)?;

		// A copy made outside the editor, at a path that comes first.
		write_model(&project, "Assets://A.zenithmodel", id(1))?;
		registry.rescan(&project)?;
		assert_eq!(
			registry.path(id(1)),
			Some(&path("Assets://B.zenithmodel")),
			"the known one"
		);
		assert_eq!(
			registry.problems(),
			&[AssetProblem::Duplicate {
				id: id(1),
				kept: path("Assets://B.zenithmodel"),
				ignored: path("Assets://A.zenithmodel"),
			}]
		);
		assert_eq!(registry.id_of(&path("Assets://A.zenithmodel")), None);

		// A cold scan knows neither: the first path.
		let cold = AssetRegistry::scan(&project)?;
		assert_eq!(cold.path(id(1)), Some(&path("Assets://A.zenithmodel")));
		Ok(())
	}

	#[test]
	fn when_the_original_goes_the_duplicate_takes_its_id() -> ZResult<()> {
		let (_dir, project) = project()?;
		write_model(&project, "Assets://A.zenithmodel", id(1))?;
		write_model(&project, "Assets://B.zenithmodel", id(1))?;
		let mut registry = AssetRegistry::scan(&project)?;
		registry.forget(&path("Assets://A.zenithmodel"));
		assert_eq!(registry.path(id(1)), Some(&path("Assets://B.zenithmodel")));
		assert_eq!(registry.problems(), NO_PROBLEMS);

		// Three copies: the third is a duplicate of the one that took the id.
		write_model(&project, "Assets://C.zenithmodel", id(1))?;
		let mut registry = AssetRegistry::scan(&project)?;
		registry.forget(&path("Assets://A.zenithmodel"));
		assert_eq!(
			registry.problems(),
			&[AssetProblem::Duplicate {
				id: id(1),
				kept: path("Assets://B.zenithmodel"),
				ignored: path("Assets://C.zenithmodel"),
			}]
		);
		Ok(())
	}

	#[test]
	fn a_moved_file_keeps_its_id() -> ZResult<()> {
		let (_dir, project) = project()?;
		write_model(&project, "Assets://A.zenithmodel", id(1))?;
		let mut registry = AssetRegistry::scan(&project)?;
		registry.rename(&path("Assets://A.zenithmodel"), &path("Assets://Moved/A.zenithmodel"));
		assert_eq!(registry.path(id(1)), Some(&path("Assets://Moved/A.zenithmodel")));
		assert_eq!(registry.id_of(&path("Assets://A.zenithmodel")), None);

		// Moved outside the editor: the watcher sees it made at one place and
		// gone at the other.
		let mut registry = AssetRegistry::scan(&project)?;
		project.move_to(&path("Assets://A.zenithmodel"), &path("Assets://Other.zenithmodel"))?;
		registry.refresh(&project, &path("Assets://Other.zenithmodel"))?;
		registry.forget(&path("Assets://A.zenithmodel"));
		assert_eq!(registry.path(id(1)), Some(&path("Assets://Other.zenithmodel")));
		assert_eq!(registry.problems(), NO_PROBLEMS);
		Ok(())
	}

	#[test]
	fn a_file_that_is_not_an_asset_is_a_problem_not_an_error() -> ZResult<()> {
		let (_dir, project) = project()?;
		project.write(
			&path("Assets://Broken.zenithmodel"),
			b"version https://git-lfs.github.com/spec/v1\n",
		)?;
		project.write(&path("Assets://Wrong.zenithmaterial"), b"{ \"uuid\": 5 }")?;
		write_model(&project, "Assets://Liar.zenithtexture", id(3))?;
		let mut registry = AssetRegistry::scan(&project)?;
		assert!(registry.is_empty());
		assert_eq!(registry.problems().len(), 3, "{:?}", registry.problems());

		// Fixed: the problem goes.
		write_material(&project, "Assets://Wrong.zenithmaterial", id(4))?;
		assert_eq!(
			registry.refresh(&project, &path("Assets://Wrong.zenithmaterial"))?,
			Some(id(4))
		);
		assert_eq!(registry.problems().len(), 2);
		Ok(())
	}
}
