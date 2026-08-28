use std::path::Path;

use zenith_error::ZPubResult;
use zenith_registry::ComponentRegistry;
use zenith_types::paths::WorldPath;
use zenith_world::World;

/// Interface for a loaded Zenith project, implemented separately per
/// on-disk project format version (e.g. `ProjectV1`, `ProjectV2`).
pub trait ProjectTrait {
	/// Returns the project name.
	fn name(&self) -> &str;
	/// Returns the engine version required by the project.
	fn engine_version(&self) -> &zenith_types::VersionReq;

	/// Opens the project located at `path`. The directory must contain both
	/// a `.v*` version marker file and a `*Name*.zenithproject` manifest —
	/// these are normally created by [`Self::create`], and their absence is
	/// an error (the directory isn't recognized as a project at all).
	/// Other expected folders (e.g. `Assets`, `Settings`, or gitignored `Temp`)
	/// are recreated automatically if missing — this covers a project whose
	/// structure was manually edited/pruned after creation, not the
	/// manifest/marker files themselves, which are never silently regenerated.
	fn open(path: &Path, name: &str) -> ZPubResult<Self>
	where
		Self: Sized;
	/// Creates a new project named `name` inside `path`.
	///
	/// `path` must already exist and denote the *parent* directory — this is
	/// the opposite of [`Self::open`], which takes the project's own root.
	/// The resulting project directory, `path.join(name)`, must not already
	/// exist; if it does, this returns [`ZenithError::InvalidProjectPath`]
	/// unless the code creating it explicitly reports `AlreadyExists`. Fails
	/// with `BadProjectName` if `name` doesn't pass validation (see
	/// `is_valid_project_name`).
	fn create(path: &Path, name: &str) -> ZPubResult<Self>
	where
		Self: Sized;

	/// Loads the world at `path`.
	fn load_world(&self, path: &WorldPath) -> ZPubResult<World>;
	/// Loads the world at `path` with the given registry.
	fn load_world_with_registry(&self, path: &WorldPath, registry: ComponentRegistry) -> ZPubResult<World>;
	/// Saves `world` to `path`, overwriting any existing file there.
	fn save_world(&self, path: &WorldPath, world: &mut World) -> ZPubResult<()>;
	// TODO: add atomic_save_world
	/// Copies the world at `path` to `new_path`, leaving the original in place.
	fn copy_world(&self, path: &WorldPath, new_path: &WorldPath) -> ZPubResult<()>;
	/// Moves (and can rename) the world at `path` to `new_path`.
	fn move_world(&self, path: &WorldPath, new_path: &WorldPath) -> ZPubResult<()>;
	/// Returns whether a world exists at `path`.
	fn has_world(&self, path: &WorldPath) -> ZPubResult<bool>;
	/// Moves the world at `path` to the trash.
	fn delete_world(&self, path: &WorldPath) -> ZPubResult<()>;
	/// Lists every world in the project. See [`WorldPath`] for entry
	/// fallibility notes.
	fn list_worlds(&self) -> Vec<ZPubResult<WorldPath>>;
}
