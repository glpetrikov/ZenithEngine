//! The public `Project` type: one API over three backends.

use std::{path::Path, sync::Arc};

use zenith_archive::{BuildId, DirSource, FileSource, ReadAt, has_archive_magic};
use zenith_error::{ZResult, ZenithError};

use crate::{
	builds::{ArchiveV1, BuildFsV1},
	core::{Backend, Core},
	layout::is_trash_path,
	manifest::Manifest,
	options::{OpenOptions, ProjectWarning},
	trash_id::TrashId,
	types::{DirEntry, Metadata, ReadSeek},
	userdata::{DATA_SCHEME, UserStore, check_path},
	zpath::ZPath,
};
#[cfg(feature = "fs")]
use crate::{
	fs::{FsV1, WalkEntry, has_other_version_flag, has_v1_flag},
	layout::BUILD_INDEX_FILE,
	trash::{RestoreOptions, TrashEntry},
	watch::{WatchConfig, Watcher},
};

/// Which kind of project this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
	/// A project folder on disk; the only kind that can be changed.
	Fs,
	/// A build in one file (web, simple distribution). Read-only.
	BuildArchive,
	/// A build as a folder of separate files (Steam: updates touch only changed
	/// files). Read-only.
	BuildFs,
}

/// Folder backends by structure version (only 1 exists).
#[cfg(feature = "fs")]
pub enum FsProject {
	V1(FsV1),
}

/// Single-file build backends by format version.
pub enum ArchiveProject {
	V1(ArchiveV1),
}

/// Folder build backends by format version.
pub enum BuildFsProject {
	V1(BuildFsV1),
}

/// A project: a folder on disk, or a build of one. Thread-safe; every method
/// takes `&self`.
pub enum Project {
	#[cfg(feature = "fs")]
	Fs(FsProject),
	BuildArchive(ArchiveProject),
	BuildFs(BuildFsProject),
}

#[cfg(feature = "fs")]
mod fs_dir_source {
	use std::{io, sync::Arc};

	use cap_std::fs::Dir;
	use zenith_archive::{DirSource, FileSource, ReadAt};

	use crate::layout::BUILD_INDEX_FILE;

	/// A folder build on disk, read through a directory handle so a hostile
	/// build cannot point outside its folder.
	pub struct FsDirSource(pub Dir);

	impl FsDirSource {
		fn open(&self, name: &str) -> io::Result<Arc<dyn ReadAt>> {
			Ok(Arc::new(FileSource::new(self.0.open(name)?.into_std())?))
		}
	}

	impl DirSource for FsDirSource {
		fn open_index(&self) -> io::Result<Arc<dyn ReadAt>> {
			self.open(BUILD_INDEX_FILE)
		}

		fn open_data(&self, name: &str) -> io::Result<Arc<dyn ReadAt>> {
			self.open(name)
		}
	}
}

impl Project {
	// --- opening ------------------------------------------------------------

	/// Creates a new project folder with the standard layout and opens it.
	/// The folder name is the project name; the folder must not exist or be
	/// empty.
	///
	/// # Errors
	/// `BadProjectName`, `ProjectAlreadyExists`, or an I/O error.
	#[cfg(feature = "fs")]
	pub fn create(path: impl AsRef<Path>) -> ZResult<Self> {
		Ok(Self::Fs(FsProject::V1(FsV1::create(
			path.as_ref(),
			&OpenOptions::new(),
		)?)))
	}

	/// Opens a project folder, a single-file build or a folder build; the
	/// kind is detected from the path. Use [`OpenOptions`] for cache settings
	/// and an encryption key.
	///
	/// # Errors
	/// `UnknownProjectFormat` if the path is none of these; otherwise errors
	/// for damaged or unsupported projects and missing or wrong keys.
	pub fn open(path: impl AsRef<Path>) -> ZResult<Self> {
		Self::open_with_options(path.as_ref(), &OpenOptions::new())
	}

	pub(crate) fn open_with_options(path: &Path, opts: &OpenOptions) -> ZResult<Self> {
		let meta = std::fs::metadata(path).map_err(|_| ZenithError::InvalidProjectPath(path.to_path_buf()))?;
		if meta.is_file() {
			let src = FileSource::open(path)?;
			if !has_archive_magic(&src) {
				return Err(ZenithError::UnknownProjectFormat(path.to_path_buf()));
			}
			return Ok(Self::BuildArchive(ArchiveProject::V1(ArchiveV1::open(src, opts)?)));
		}
		Self::open_folder(path, opts)
	}

	#[cfg(feature = "fs")]
	fn open_folder(path: &Path, opts: &OpenOptions) -> ZResult<Self> {
		match (has_v1_flag(path), path.join(BUILD_INDEX_FILE).is_file()) {
			(true, false) => Ok(Self::Fs(FsProject::V1(FsV1::open(path, opts)?))),
			(false, true) => {
				let dir = cap_std::fs::Dir::open_ambient_dir(path, cap_std::ambient_authority())?;
				Ok(Self::BuildFs(BuildFsProject::V1(BuildFsV1::open(
					fs_dir_source::FsDirSource(dir),
					opts,
				)?)))
			}
			(false, false) if has_other_version_flag(path) => Err(ZenithError::UnsupportedProjectVersion),
			_ => Err(ZenithError::UnknownProjectFormat(path.to_path_buf())),
		}
	}

	#[cfg(not(feature = "fs"))]
	fn open_folder(path: &Path, _opts: &OpenOptions) -> ZResult<Self> {
		Err(ZenithError::Unsupported(format!(
			"opening the folder {} needs the `fs` feature; pass a build with open_archive_bytes or open_build_dir",
			path.display()
		)))
	}

	/// Opens a single-file build that is already in memory (a download in a
	/// browser, an embedded resource). Loading it in parts over the network is
	/// not supported.
	///
	/// # Errors
	/// Errors for a damaged build, an unsupported version, or a missing or
	/// wrong key.
	pub fn open_archive_bytes(bytes: Arc<[u8]>, opts: &OpenOptions) -> ZResult<Self> {
		Ok(Self::BuildArchive(ArchiveProject::V1(ArchiveV1::open(bytes, opts)?)))
	}

	/// Opens a single-file build from any positional source.
	///
	/// # Errors
	/// Same as [`Self::open_archive_bytes`].
	pub fn open_archive(src: impl ReadAt + 'static, opts: &OpenOptions) -> ZResult<Self> {
		Ok(Self::BuildArchive(ArchiveProject::V1(ArchiveV1::open(src, opts)?)))
	}

	/// Opens a folder build whose files come from `source` (needed where the
	/// build is not a plain directory, for example fetched files in a browser).
	///
	/// # Errors
	/// Same as [`Self::open_archive_bytes`].
	pub fn open_build_dir(source: impl DirSource + 'static, opts: &OpenOptions) -> ZResult<Self> {
		Ok(Self::BuildFs(BuildFsProject::V1(BuildFsV1::open(source, opts)?)))
	}

	// --- common -------------------------------------------------------------

	fn backend(&self) -> &dyn Backend {
		match self {
			#[cfg(feature = "fs")]
			Self::Fs(FsProject::V1(p)) => p,
			Self::BuildArchive(ArchiveProject::V1(p)) => p,
			Self::BuildFs(BuildFsProject::V1(p)) => p,
		}
	}

	fn core(&self) -> &Core {
		self.backend().core()
	}

	fn resolve(&self, path: &ZPath) -> ZResult<String> {
		self.core().resolve(path)
	}

	#[must_use]
	pub const fn kind(&self) -> BackendKind {
		match self {
			#[cfg(feature = "fs")]
			Self::Fs(_) => BackendKind::Fs,
			Self::BuildArchive(_) => BackendKind::BuildArchive,
			Self::BuildFs(_) => BackendKind::BuildFs,
		}
	}

	/// The identity of a build; `None` for a project folder. See
	/// [`OpenOptions::expected_build_id`].
	#[must_use]
	pub fn build_id(&self) -> Option<BuildId> {
		self.backend().build_id()
	}

	#[must_use]
	pub fn manifest(&self) -> &Manifest {
		&self.core().manifest
	}

	/// Things that went wrong without failing an operation, oldest first
	/// (for example a damaged Zenith block in `.gitignore`). Also logged.
	#[must_use]
	pub fn warnings(&self) -> Vec<ProjectWarning> {
		self.core().warnings()
	}

	/// The cache settings in effect.
	#[must_use]
	pub fn cache_settings(&self) -> &crate::cache::CacheSettings {
		self.core().cache.settings()
	}

	/// Adds a scheme for this project handle. Not persisted.
	///
	/// # Errors
	/// `InvalidScheme`/`InvalidZPath` for a bad name or folder,
	/// `SchemeAlreadyRegistered` for a name in use.
	pub fn register_scheme(&self, name: &str, root: &str) -> ZResult<()> {
		self.core()
			.schemes
			.write()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.register(name, root)
	}

	// --- reading ------------------------------------------------------------

	// --- the data of the player (`Data://`)
	// -----------------------------------

	/// The path inside `Data://`, if `path` is one (checked: a name that is
	/// safe everywhere, the root only if `allow_root`).
	fn data_path(path: &ZPath, allow_root: bool) -> ZResult<Option<&str>> {
		if path.scheme() != DATA_SCHEME {
			return Ok(None);
		}
		check_path(path.path(), allow_root)?;
		Ok(Some(path.path()))
	}

	/// Where `Data://` is kept.
	#[must_use]
	pub fn user_data(&self) -> Arc<dyn UserStore> {
		self.core()
			.user_data
			.read()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.clone()
	}

	/// Keeps `Data://` somewhere else from now on (nothing is moved). For a
	/// browser, which opens its store once the project has been read, since it
	/// is named after the project.
	pub fn set_user_data(&self, store: Arc<dyn UserStore>) {
		*self
			.core()
			.user_data
			.write()
			.unwrap_or_else(std::sync::PoisonError::into_inner) = store;
	}

	/// Waits until what was written to `Data://` is safe (a browser writes in
	/// the background). Call it before the game ends.
	///
	/// # Errors
	/// An I/O error.
	pub fn flush_data(&self) -> ZResult<()> {
		self.user_data().flush()
	}

	/// Deletes a file or a folder of `Data://` for good; `false` if there was
	/// nothing. (A file of the project is never deleted for good: `remove`
	/// moves it to the trash.)
	///
	/// # Errors
	/// `Unsupported` for a path that is not in `Data://`, `InvalidZPath` for a
	/// bad name.
	pub fn delete(&self, path: &ZPath) -> ZResult<bool> {
		if let Some(rel) = Self::data_path(path, false)? {
			return self.user_data().remove(rel);
		}
		Err(ZenithError::Unsupported(
			"only Data:// is deleted for good; `remove` moves a project file to the trash".into(),
		))
	}

	/// Replaces a file atomically: it is the old file or the new one, never
	/// half of each. Missing folders are made.
	///
	/// `Data://` can be written for any project or build. A file of the
	/// project can be written in a project folder only.
	///
	/// # Errors
	/// `ReadOnly` for a build, `ReadOnlyScheme` for `Trash://`,
	/// `ProjectWrongKind` if the path is a folder.
	pub fn write(&self, path: &ZPath, data: &[u8]) -> ZResult<()> {
		if let Some(rel) = Self::data_path(path, false)? {
			return self.user_data().write(rel, data);
		}
		self.write_project_file(path, data)
	}

	/// Creates a folder and its parents.
	///
	/// # Errors
	/// `ReadOnly` for a build (`Data://` can always be written),
	/// `ReadOnlyScheme` for `Trash://`.
	pub fn create_dir(&self, path: &ZPath) -> ZResult<()> {
		if let Some(rel) = Self::data_path(path, true)? {
			return self.user_data().create_dir(rel);
		}
		self.create_project_dir(path)
	}

	/// # Errors
	/// `SchemeNotRegistered`, or an I/O error.
	pub fn exists(&self, path: &ZPath) -> ZResult<bool> {
		if let Some(rel) = Self::data_path(path, true)? {
			return Ok(self.user_data().metadata(rel)?.is_some());
		}
		self.backend().exists(&self.resolve(path)?)
	}

	/// Entries of a folder, sorted by name.
	///
	/// # Errors
	/// `ProjectNotFound` if it does not exist.
	pub fn list(&self, path: &ZPath) -> ZResult<Vec<DirEntry>> {
		if let Some(rel) = Self::data_path(path, true)? {
			return self.user_data().list(rel);
		}
		self.backend().list(&self.resolve(path)?)
	}

	/// # Errors
	/// `ProjectNotFound` if it does not exist.
	pub fn metadata(&self, path: &ZPath) -> ZResult<Metadata> {
		if let Some(rel) = Self::data_path(path, true)? {
			return self
				.user_data()
				.metadata(rel)?
				.ok_or_else(|| ZenithError::ProjectNotFound(path.to_string()));
		}
		self.backend().metadata(&self.resolve(path)?)
	}

	/// Whether the path itself is a symlink (always false in builds).
	///
	/// # Errors
	/// `ProjectNotFound` if it does not exist.
	pub fn is_symlink(&self, path: &ZPath) -> ZResult<bool> {
		if Self::data_path(path, true)?.is_some() {
			return Ok(false);
		}
		self.backend().is_symlink(&self.resolve(path)?)
	}

	/// A file of `Data://`, within the size limit.
	fn read_data(&self, path: &ZPath, rel: &str) -> ZResult<Vec<u8>> {
		if rel.is_empty() {
			return Err(ZenithError::ProjectWrongKind(format!("{path} is a folder")));
		}
		let data = self
			.user_data()
			.read(rel)?
			.ok_or_else(|| ZenithError::ProjectNotFound(path.to_string()))?;
		let limit = self.core().max_read_size;
		if data.len() as u64 > limit {
			return Err(ZenithError::FileTooLarge(format!(
				"{path} is {} bytes, the limit is {limit} (OpenOptions::max_read_size)",
				data.len()
			)));
		}
		Ok(data)
	}

	/// Reads a whole file, through the cache (`Trash://` is never cached).
	///
	/// # Errors
	/// `ProjectNotFound`, `ProjectWrongKind` for a folder, `PathEscapesRoot`
	/// for a link that leaves the project, or an I/O error.
	pub fn read_bytes(&self, path: &ZPath) -> ZResult<Arc<[u8]>> {
		if let Some(rel) = Self::data_path(path, true)? {
			return Ok(Arc::from(self.read_data(path, rel)?));
		}
		let backend = self.backend();
		let rel = backend.core().resolve(path)?;
		if is_trash_path(&rel) {
			return Ok(Arc::from(Self::read_limited(backend, &rel)?));
		}
		// Stamp first: if the file changes while we read, the stored stamp is
		// older than the data and the next lookup reloads.
		let stamp = backend.stamp(&rel);
		if let Some(hit) = backend.core().cache.get(&rel, stamp) {
			return Ok(hit);
		}
		let data: Arc<[u8]> = Arc::from(Self::read_limited(backend, &rel)?);
		backend.core().cache.offer(&rel, &data, stamp);
		Ok(data)
	}

	/// Reads a file unless it is bigger than `max_read_size`. The size comes
	/// from the file system or the build's index, so a hostile build cannot
	/// make the reader allocate more than the limit.
	fn read_limited(backend: &dyn Backend, rel: &str) -> ZResult<Vec<u8>> {
		let limit = backend.core().max_read_size;
		let size = backend.metadata(rel)?.size;
		if size > limit {
			return Err(ZenithError::FileTooLarge(format!(
				"{rel} is {size} bytes, the limit is {limit} (OpenOptions::max_read_size, or stream it with open_read)"
			)));
		}
		backend.read_raw(rel)
	}

	/// Reads a UTF-8 text file, through the cache.
	///
	/// # Errors
	/// Same as [`Self::read_bytes`], plus `InvalidUtf8`.
	pub fn read_string(&self, path: &ZPath) -> ZResult<String> {
		let invalid = || ZenithError::InvalidUtf8(path.to_string());
		if let Some(rel) = Self::data_path(path, true)? {
			return String::from_utf8(self.read_data(path, rel)?).map_err(|_| invalid());
		}
		let backend = self.backend();
		let core = backend.core();
		let rel = core.resolve(path)?;
		if is_trash_path(&rel) {
			return String::from_utf8(Self::read_limited(backend, &rel)?).map_err(|_| invalid());
		}
		let stamp = backend.stamp(&rel);
		if let Some(hit) = core.cache.get(&rel, stamp) {
			return std::str::from_utf8(&hit).map(str::to_owned).map_err(|_| invalid());
		}
		// The bytes become the string without a copy; they are copied for the
		// cache only if this read puts the file there.
		let text = String::from_utf8(Self::read_limited(backend, &rel)?).map_err(|_| invalid())?;
		core.cache
			.offer_lazy(&rel, text.len(), || Arc::from(text.as_bytes()), stamp);
		Ok(text)
	}

	/// Streaming read access. Not cached.
	///
	/// # Errors
	/// Same as [`Self::read_bytes`].
	pub fn open_read(&self, path: &ZPath) -> ZResult<Box<dyn ReadSeek + Send>> {
		if let Some(rel) = Self::data_path(path, true)? {
			// A file of the player's data is read whole; it is small.
			return Ok(Box::new(std::io::Cursor::new(self.read_data(path, rel)?)));
		}
		self.backend().open_read(&self.resolve(path)?)
	}
}

#[cfg(feature = "fs")]
impl Project {
	fn fs_only(&self, what: &str) -> ZResult<&FsV1> {
		match self {
			Self::Fs(FsProject::V1(fs)) => Ok(fs),
			_ => Err(ZenithError::Unsupported(what.to_string())),
		}
	}

	// --- changing (project folders only) ------------------------------------

	/// The folder backend and the resolved path, for an operation that changes
	/// files: builds are read-only, and the trash can only be changed through
	/// the trash operations.
	fn writable(&self, path: &ZPath) -> ZResult<(&FsV1, String)> {
		match self {
			Self::Fs(FsProject::V1(fs)) => {
				if path.scheme() == DATA_SCHEME {
					return Err(ZenithError::Unsupported(
						"Data:// is not part of the project: use write and delete".into(),
					));
				}
				let rel = fs.core.resolve(path)?;
				if is_trash_path(&rel) {
					return Err(ZenithError::ReadOnlyScheme(path.scheme().to_string()));
				}
				Ok((fs, rel))
			}
			_ => Err(ZenithError::ReadOnly),
		}
	}

	fn write_project_file(&self, path: &ZPath, data: &[u8]) -> ZResult<()> {
		let (fs, rel) = self.writable(path)?;
		fs.write(&rel, data)
	}

	fn create_project_dir(&self, path: &ZPath) -> ZResult<()> {
		let (fs, rel) = self.writable(path)?;
		fs.create_dir(&rel)
	}

	/// Moves a file or folder to the trash and returns its id. Nothing is
	/// deleted for good; see [`Self::restore`] and [`Self::purge`].
	///
	/// # Errors
	/// `ReadOnly` for builds, `ReadOnlyScheme` for `Trash://`,
	/// `ProjectNotFound`, `Unsupported` for a scheme root.
	pub fn remove(&self, path: &ZPath) -> ZResult<TrashId> {
		let (fs, rel) = self.writable(path)?;
		fs.remove(&rel, path)
	}

	/// Copies a file or folder inside this project, possibly between schemes.
	/// The source may be in `Trash://`; the destination must not exist.
	///
	/// # Errors
	/// `ReadOnly` for builds, `ProjectAlreadyExists`, `ProjectNotFound`.
	pub fn copy(&self, from: &ZPath, to: &ZPath) -> ZResult<()> {
		let (fs, to_rel) = self.writable(to)?;
		if from.scheme() == DATA_SCHEME {
			return Err(ZenithError::Unsupported(
				"Data:// is not part of the project: read the file and write it".into(),
			));
		}
		let from_rel = fs.core.resolve(from)?;
		fs.copy(&from_rel, &to_rel)
	}

	/// Moves a file or folder inside this project, possibly between schemes.
	/// The destination must not exist. (`move` is a keyword, hence the name.)
	///
	/// # Errors
	/// `ReadOnly` for builds, `ReadOnlyScheme` if either side is in `Trash://`,
	/// `ProjectAlreadyExists`, `ProjectNotFound`.
	pub fn move_to(&self, from: &ZPath, to: &ZPath) -> ZResult<()> {
		let (fs, from_rel) = self.writable(from)?;
		let (_, to_rel) = self.writable(to)?;
		fs.rename(&from_rel, &to_rel)?;
		fs.sync_gitignore_lenient();
		Ok(())
	}

	/// Writes the cache settings in effect to `Settings/CacheConfig.toml`.
	///
	/// # Errors
	/// `ReadOnly` for builds.
	pub fn save_cache_config(&self) -> ZResult<()> {
		match self {
			Self::Fs(FsProject::V1(fs)) => fs.save_cache_config(),
			_ => Err(ZenithError::ReadOnly),
		}
	}

	// --- project-relative access (for tools that treat the folder as a whole)
	// ---

	/// The project-relative, `/`-separated path a `ZPath` stands for (for
	/// example `Worlds://A.zenith` is `Assets/Worlds/A.zenith`).
	///
	/// # Errors
	/// `SchemeNotRegistered` for an unknown scheme.
	pub fn project_path(&self, path: &ZPath) -> ZResult<String> {
		self.resolve(path)
	}

	/// Lists every entry under the project root, including places no scheme
	/// covers (the manifest, `Build/`, ...). Links are reported, not followed.
	/// A folder is entered only if `descend` accepts its project-relative path.
	///
	/// # Errors
	/// `Unsupported` for builds, or an I/O error.
	pub fn walk_project(&self, descend: &dyn Fn(&str) -> bool) -> ZResult<Vec<WalkEntry>> {
		self.fs_only("walking the project folder")?.walk(descend)
	}

	/// Streaming read of a project-relative path (see [`Self::walk_project`]).
	/// Not cached.
	///
	/// # Errors
	/// `Unsupported` for builds, `InvalidZPath` if `rel` is not a plain
	/// relative path, otherwise as [`Self::open_read`].
	pub fn open_project_file(&self, rel: &str) -> ZResult<Box<dyn ReadSeek + Send>> {
		let fs = self.fs_only("reading project files by path")?;
		crate::zpath::validate_relative(rel).map_err(|why| ZenithError::InvalidZPath(format!("{rel:?}: {why}")))?;
		fs.open_read(rel)
	}

	// --- trash --------------------------------------------------------------

	/// Trash entries, newest first. Entries whose object is missing are
	/// skipped.
	///
	/// # Errors
	/// `Unsupported` for builds.
	pub fn trash_list(&self) -> ZResult<Vec<TrashEntry>> {
		self.fs_only("the trash")?.trash_list()
	}

	/// Entries removed from exactly `path`.
	///
	/// # Errors
	/// `Unsupported` for builds.
	pub fn trash_find(&self, path: &ZPath) -> ZResult<Vec<TrashEntry>> {
		self.fs_only("the trash")?.trash_find(path)
	}

	/// Puts an entry back where it was. Missing parent folders are created.
	///
	/// # Errors
	/// `TrashEntryNotFound`, `SchemeNotRegistered` if the original scheme is
	/// gone, `ProjectAlreadyExists` if the place is taken, `Unsupported` for
	/// builds.
	pub fn restore(&self, id: &TrashId) -> ZResult<ZPath> {
		self.fs_only("the trash")?.restore(id)
	}

	/// Restores the newest entry.
	///
	/// # Errors
	/// `TrashEntryNotFound` if the trash is empty; otherwise as
	/// [`Self::restore`].
	pub fn restore_last(&self) -> ZResult<ZPath> {
		self.fs_only("the trash")?.restore_last()
	}

	/// Restores with a different target and/or conflict handling.
	///
	/// # Errors
	/// As [`Self::restore`].
	pub fn restore_with(&self, id: &TrashId, options: &RestoreOptions) -> ZResult<ZPath> {
		self.fs_only("the trash")?.restore_with(id, options)
	}

	/// Deletes one trash entry for good. The only ways to delete anything for
	/// good are this and [`Self::purge_all`].
	///
	/// # Errors
	/// `TrashEntryNotFound`, `Unsupported` for builds.
	pub fn purge(&self, id: &TrashId) -> ZResult<()> {
		self.fs_only("the trash")?.purge(id)
	}

	/// Empties the trash: every entry, damaged ones included. Other files in
	/// `Trash/` (not named like an entry) are left alone.
	///
	/// # Errors
	/// `Unsupported` for builds.
	pub fn purge_all(&self) -> ZResult<()> {
		self.fs_only("the trash")?.purge_all()
	}

	// --- output folders and .gitignore
	// ----------------------------------------

	/// Creates a folder (with parents). If it is inside the project and the
	/// project has a `.gitignore`, an anchored entry (`/Build/`) goes into the
	/// Zenith-managed block; a folder outside the project is only created.
	///
	/// # Errors
	/// `InvalidGitignore` for a damaged block (the file is not changed),
	/// `Unsupported` for builds, or an I/O error.
	pub fn ensure_ignored_dir(&self, path: impl AsRef<Path>) -> ZResult<()> {
		self.fs_only("output folders")?.ensure_ignored_dir(path.as_ref())
	}

	/// Drops managed `.gitignore` entries whose folder no longer exists, and
	/// the block itself once empty. A missing `.gitignore` is not an error.
	///
	/// # Errors
	/// `InvalidGitignore` for a damaged block (the file is not changed),
	/// `Unsupported` for builds.
	pub fn sync_gitignore(&self) -> ZResult<()> {
		self.fs_only("`.gitignore` handling")?.sync_gitignore()
	}

	// --- watching -----------------------------------------------------------

	/// Starts watching the project folder for changes.
	///
	/// # Errors
	/// `Unsupported` for builds, `InvalidSettings` for bad rules, `Watch` if
	/// the OS watcher cannot start.
	pub fn watch(&self, config: &WatchConfig) -> ZResult<Watcher> {
		self.fs_only("watching files")?.watch(config)
	}
}

/// Without the `fs` feature only builds exist, and builds are read-only.
#[cfg(not(feature = "fs"))]
impl Project {
	/// Always `ReadOnly`.
	///
	/// # Errors
	/// `ReadOnly`.
	pub const fn save_cache_config(&self) -> ZResult<()> {
		Err(ZenithError::ReadOnly)
	}

	// Without the `fs` feature there are only builds: nothing of the project
	// can be written (`Data://` can, see `Project::write`).
	#[allow(clippy::unused_self)] // the same signature as with the `fs` feature
	const fn write_project_file(&self, _path: &ZPath, _data: &[u8]) -> ZResult<()> {
		Err(ZenithError::ReadOnly)
	}

	#[allow(clippy::unused_self)] // the same signature as with the `fs` feature
	const fn create_project_dir(&self, _path: &ZPath) -> ZResult<()> {
		Err(ZenithError::ReadOnly)
	}

	/// Always `ReadOnly`.
	///
	/// # Errors
	/// `ReadOnly`.
	pub const fn remove(&self, _path: &ZPath) -> ZResult<TrashId> {
		Err(ZenithError::ReadOnly)
	}

	/// Always `ReadOnly`.
	///
	/// # Errors
	/// `ReadOnly`.
	pub const fn copy(&self, _from: &ZPath, _to: &ZPath) -> ZResult<()> {
		Err(ZenithError::ReadOnly)
	}

	/// Always `ReadOnly`.
	///
	/// # Errors
	/// `ReadOnly`.
	pub const fn move_to(&self, _from: &ZPath, _to: &ZPath) -> ZResult<()> {
		Err(ZenithError::ReadOnly)
	}
}
