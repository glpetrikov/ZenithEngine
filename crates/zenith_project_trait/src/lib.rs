use std::path::Path;

use zenith_error::ZResult;
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
	///
	/// # Errors
	/// Returns `Err` if `path` is missing either the version marker file or
	/// the manifest, or if the manifest exists but fails to parse.
	fn open(path: &Path, name: &str) -> ZResult<Self>
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
	///
	/// # Errors
	/// Returns [`ZenithError::InvalidProjectPath`] if `path.join(name)`
	/// already exists, and [`ZenithError::BadProjectName`] if `name` fails
	/// validation.
	fn create(path: &Path, name: &str) -> ZResult<Self>
	where
		Self: Sized;

	/// Loads the world at `path`.
	///
	/// # Errors
	/// Returns `Err` if no world exists at `path`, or if the file exists but
	/// fails to deserialize.
	fn load_world(&self, path: &WorldPath) -> ZResult<World>;
	/// Loads the world at `path` with the given registry.
	///
	/// # Errors
	/// Returns `Err` under the same conditions as [`Self::load_world`], plus
	/// if the world references a component type not present in `registry`.
	fn load_world_with_registry(&self, path: &WorldPath, registry: ComponentRegistry) -> ZResult<World>;
	/// Saves `world` to `path`, overwriting any existing file there.
	///
	/// # Errors
	/// Returns `Err` if `world` fails to serialize, or if writing to `path`
	/// fails (e.g. permissions, disk space).
	fn save_world(&self, path: &WorldPath, world: &mut World) -> ZResult<()>;
	// TODO: add atomic_save_world
	/// Copies the world at `path` to `new_path`, leaving the original in place.
	///
	/// # Errors
	/// Returns `Err` if no world exists at `path`, or if the copy fails
	/// (e.g. `new_path`'s parent directory is missing, or I/O fails).
	fn copy_world(&self, path: &WorldPath, new_path: &WorldPath) -> ZResult<()>;
	/// Moves (and can rename) the world at `path` to `new_path`.
	///
	/// # Errors
	/// Returns `Err` under the same conditions as [`Self::copy_world`], or if
	/// the move/rename itself fails partway through.
	fn move_world(&self, path: &WorldPath, new_path: &WorldPath) -> ZResult<()>;
	/// Returns whether a world exists at `path`.
	///
	/// # Errors
	/// Returns `Err` if the existence check itself fails (e.g. I/O or
	/// permission errors) — as distinct from `Ok(false)` for a world that
	/// simply isn't there.
	fn has_world(&self, path: &WorldPath) -> ZResult<bool>;
	/// Moves the world at `path` to the trash.
	///
	/// # Errors
	/// Returns `Err` if no world exists at `path`, or if it cannot be moved
	/// to the trash (e.g. I/O failure).
	fn delete_world(&self, path: &WorldPath) -> ZResult<()>;
	/// Lists every world in the project. See [`WorldPath`] for entry
	/// fallibility notes.
	fn list_worlds(&self) -> Vec<ZResult<WorldPath>>;
}
