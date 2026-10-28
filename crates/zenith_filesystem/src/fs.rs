//! The `Fs` backend: a project folder on disk.
//!
//! The root is opened once as a `cap_std::fs::Dir` and every operation goes
//! through it with a project-relative path. cap-std resolves paths inside the
//! directory handle, so `..`, absolute paths and symlinks that lead outside the
//! project cannot be followed: they fail, whatever the path parser let through.

use std::{
	io::{self, Write},
	path::{Path, PathBuf},
	sync::{Arc, PoisonError, RwLock},
	time::SystemTime,
};

use cap_std::{ambient_authority, fs::Dir};
use zenith_error::{ZResult, ZenithError};

use crate::{
	cache::{CacheConfig, CacheSettings, Stamp},
	core::{Backend, Core},
	gitignore,
	layout::{CACHE_CONFIG_FILE, DEFAULT_GITIGNORE, GITIGNORE, PROJECT_DIRS, TMP_SUFFIX, V1_FLAG, manifest_file_name},
	manifest::{Manifest, validate_project_name},
	options::{OpenOptions, WarningKind},
	types::{DirEntry, EntryKind, Metadata, ReadSeek},
};

/// Version 1 of the folder backend (structure flag `.v1`).
///
/// A cheap handle: the state is shared, so the file watcher thread can hold a
/// clone and call the same operations the owner does.
#[derive(Clone)]
pub struct FsV1(Arc<FsInner>);

impl std::ops::Deref for FsV1 {
	type Target = FsInner;

	fn deref(&self) -> &FsInner {
		&self.0
	}
}

pub struct FsInner {
	pub(crate) core: Core,
	pub(crate) dir: Dir,
	/// Canonical project root. Used for naming and path comparison only; files
	/// are never opened through it.
	pub(crate) root: PathBuf,
	/// Absolute, non-canonical root handed to the OS file watcher (notify takes
	/// plain paths). Events are mapped back to project-relative paths and the
	/// files are never opened through it either.
	pub(crate) watch_root: PathBuf,
	/// Folders listed in the managed `.gitignore` block (without slashes).
	pub(crate) managed: Arc<RwLock<Vec<String>>>,
}

/// The error of a move: `AlreadyExists` names the destination (someone else
/// created it in the meantime), anything else the source.
pub fn map_move_error(err: io::Error, from: &str, to: &str) -> ZenithError {
	if err.kind() == io::ErrorKind::AlreadyExists {
		ZenithError::ProjectAlreadyExists(to.to_string())
	} else {
		map_io(err, from)
	}
}

/// Translates an I/O error into the repo's error type, naming the path.
pub fn map_io(err: io::Error, rel: &str) -> ZenithError {
	match err.kind() {
		io::ErrorKind::NotFound => ZenithError::ProjectNotFound(rel.to_string()),
		io::ErrorKind::AlreadyExists => ZenithError::ProjectAlreadyExists(rel.to_string()),
		// cap-std reports a path that leaves the sandbox as PermissionDenied with
		// "a path led outside of the filesystem" and offers no other way to tell
		// it apart. The `escape` integration tests fail if a cap-std upgrade
		// changes the wording.
		io::ErrorKind::PermissionDenied if err.to_string().contains("outside") => {
			ZenithError::PathEscapesRoot(PathBuf::from(rel))
		}
		_ => ZenithError::Io(err),
	}
}

const fn system_time(t: cap_std::time::SystemTime) -> SystemTime {
	t.into_std()
}

fn kind_of(ft: cap_std::fs::FileType) -> EntryKind {
	if ft.is_symlink() {
		EntryKind::Symlink
	} else if ft.is_dir() {
		EntryKind::Dir
	} else {
		EntryKind::File
	}
}

/// The manifest of the project folder. The project name is the folder name: the
/// canonical path comes first (it has the real spelling of the name), the path
/// as given is the fallback for a folder reached through a symlink or junction
/// that has another name.
fn find_manifest(dir: &Dir, roots: [&Path; 2], given: &Path) -> ZResult<Manifest> {
	let mut first_error = None;
	let mut tried = None;
	for root in roots {
		let Some(name) = root.file_name().and_then(|n| n.to_str()) else {
			continue;
		};
		if tried == Some(name) {
			continue;
		}
		tried = Some(name);
		match read_manifest(dir, name) {
			Ok(manifest) => return Ok(manifest),
			Err(e) => {
				first_error.get_or_insert(e);
			}
		}
	}
	Err(first_error.unwrap_or_else(|| ZenithError::InvalidProjectPath(given.to_path_buf())))
}

/// Removes what `FsV1::create` made: the folder if it did not exist before, its
/// (empty at the start) contents otherwise.
fn discard_new_project(path: &Path, existed: bool) {
	if !existed {
		let _ = std::fs::remove_dir_all(path);
		return;
	}
	for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
		let child = entry.path();
		let _ = if child.is_dir() {
			std::fs::remove_dir_all(&child)
		} else {
			std::fs::remove_file(&child)
		};
	}
}

fn read_manifest(dir: &Dir, name: &str) -> ZResult<Manifest> {
	let manifest_file = manifest_file_name(name);
	let text = dir.read_to_string(&manifest_file).map_err(|e| match e.kind() {
		io::ErrorKind::NotFound => ZenithError::InvalidProject(format!(
			"manifest {manifest_file} not found (the manifest is named after the project folder)"
		)),
		_ => ZenithError::Io(e),
	})?;
	let manifest = Manifest::parse(&text)?;
	if manifest.name != name {
		return Err(ZenithError::InvalidProject(format!(
			"manifest name {:?} does not match the folder name {name:?}",
			manifest.name
		)));
	}
	Ok(manifest)
}

impl FsV1 {
	/// Creates the folder layout of a new project and opens it.
	pub fn create(path: &Path, opts: &OpenOptions) -> ZResult<Self> {
		let name = path
			.file_name()
			.and_then(|n| n.to_str())
			.ok_or_else(|| ZenithError::InvalidProjectPath(path.to_path_buf()))?
			.to_string();
		validate_project_name(&name)?;
		let existed = path.exists();
		if existed && (!path.is_dir() || std::fs::read_dir(path)?.next().is_some()) {
			return Err(ZenithError::ProjectAlreadyExists(path.display().to_string()));
		}
		std::fs::create_dir_all(path)?;
		// A half-made project (the disk filled up) would make the next attempt
		// fail with "not empty", so it is taken back.
		if let Err(e) = Self::make_layout(path, &name) {
			discard_new_project(path, existed);
			return Err(e);
		}
		Self::open(path, opts)
	}

	fn make_layout(path: &Path, name: &str) -> ZResult<()> {
		let dir = Dir::open_ambient_dir(path, ambient_authority())?;
		for sub in PROJECT_DIRS {
			dir.create_dir_all(sub)?;
		}
		dir.write(GITIGNORE, DEFAULT_GITIGNORE)?;
		dir.write(V1_FLAG, b"")?;
		dir.write(manifest_file_name(name), Manifest::new(name)?.to_toml()?)?;
		Ok(())
	}

	/// Opens an existing project folder (the caller has seen the `.v1` flag).
	pub fn open(path: &Path, opts: &OpenOptions) -> ZResult<Self> {
		let root = std::fs::canonicalize(path).map_err(|_| ZenithError::InvalidProjectPath(path.to_path_buf()))?;
		let watch_root = std::path::absolute(path)?;
		let dir = Dir::open_ambient_dir(&root, ambient_authority())?;
		let manifest = find_manifest(&dir, [&root, &watch_root], path)?;

		let settings = match &opts.cache {
			CacheConfig::Manual(settings) => settings.clone(),
			CacheConfig::File => match dir.read_to_string(CACHE_CONFIG_FILE) {
				Ok(text) => CacheSettings::parse(&text)?,
				Err(e) if e.kind() == io::ErrorKind::NotFound => CacheSettings::default(),
				Err(e) => return Err(e.into()),
			},
		};

		let project = Self(Arc::new(FsInner {
			core: Core::new(manifest, settings, "fs", opts)?,
			dir,
			root,
			watch_root,
			managed: Arc::new(RwLock::new(Vec::new())),
		}));
		project.load_gitignore_state();
		Ok(project)
	}

	pub(crate) fn reverse(&self, rel: &str) -> Option<crate::zpath::ZPath> {
		self.core
			.schemes
			.read()
			.unwrap_or_else(PoisonError::into_inner)
			.reverse(rel)
	}

	// --- .gitignore -------------------------------------------------------

	fn read_gitignore(&self) -> ZResult<Option<String>> {
		match self.dir.read_to_string(GITIGNORE) {
			Ok(text) => Ok(Some(text)),
			Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
			Err(e) => Err(e.into()),
		}
	}

	fn write_atomic(&self, rel: &str, data: &[u8]) -> ZResult<()> {
		let tmp = tmp_name(rel);
		// Flushed to disk before the rename: after a power loss the old file or
		// the new one is there, never an empty one.
		let written = self.dir.create(&tmp).and_then(|mut file| {
			file.write_all(data)?;
			file.sync_all()
		});
		if let Err(e) = written {
			let _ = self.dir.remove_file(&tmp);
			return Err(map_io(e, rel));
		}
		self.dir.rename(&tmp, &self.dir, rel).map_err(|e| {
			let _ = self.dir.remove_file(&tmp);
			map_io(e, rel)
		})
	}

	/// Reads the managed block without changing the file. Opening a project
	/// must not edit the working tree: it is done by tools that only read it (a
	/// build on a CI machine). Stale entries are cleaned up by
	/// `sync_gitignore`, which `remove`, `move_to` and the watcher call.
	fn load_gitignore_state(&self) {
		let entries = self
			.read_gitignore()
			.and_then(|text| text.as_deref().map(gitignore::managed_entries).transpose());
		match entries {
			Ok(entries) => {
				*self.managed.write().unwrap_or_else(PoisonError::into_inner) = entries.unwrap_or_default();
			}
			Err(e) => {
				self.refresh_managed(None);
				self.warn_gitignore(&e);
			}
		}
	}

	fn refresh_managed(&self, text: Option<&str>) {
		let entries = text
			.and_then(|t| gitignore::managed_entries(t).ok())
			.unwrap_or_default();
		*self.managed.write().unwrap_or_else(PoisonError::into_inner) = entries;
	}

	/// Removes block entries whose folder is gone, and an emptied block.
	/// A missing `.gitignore` is not an error and is not created.
	pub fn sync_gitignore(&self) -> ZResult<()> {
		let Some(text) = self.read_gitignore()? else {
			self.refresh_managed(None);
			return Ok(());
		};
		let exists = |path: &str| self.dir.metadata(path).is_ok_and(|m| m.is_dir());
		let result = gitignore::remove_missing(&text, exists);
		match result {
			Ok(Some(new_text)) => {
				self.write_atomic(GITIGNORE, new_text.as_bytes())?;
				self.refresh_managed(Some(&new_text));
				Ok(())
			}
			Ok(None) => {
				self.refresh_managed(Some(&text));
				Ok(())
			}
			Err(e) => {
				self.refresh_managed(None);
				Err(e)
			}
		}
	}

	/// `sync_gitignore` for places where the operation itself already
	/// succeeded: a damaged block becomes a warning, not a failure.
	pub(crate) fn sync_gitignore_lenient(&self) {
		if let Err(e) = self.sync_gitignore() {
			self.warn_gitignore(&e);
		}
	}

	/// `ensure_ignored_dir` for the same places (a `restore` that has already
	/// put the folder back).
	pub(crate) fn ensure_ignored_dir_lenient(&self, path: &Path) {
		if let Err(e) = self.ensure_ignored_dir(path) {
			self.warn_gitignore(&e);
		}
	}

	fn warn_gitignore(&self, err: &ZenithError) {
		match err {
			ZenithError::InvalidGitignore(_) => self.core.warn(
				WarningKind::DamagedGitignore,
				format!("{GITIGNORE} was left untouched: {err}"),
			),
			_ => self
				.core
				.warn(WarningKind::GitignoreIo, format!("could not update {GITIGNORE}: {err}")),
		}
	}

	/// Creates `path` (and parents). If it lies inside the project and the
	/// project has a `.gitignore`, adds an anchored entry to the managed
	/// block. A folder outside the project is only created.
	pub fn ensure_ignored_dir(&self, path: &Path) -> ZResult<()> {
		let absolute = std::path::absolute(path)?;
		match self.project_relative(&absolute) {
			Some(rel) if !rel.is_empty() => {
				self.dir.create_dir_all(&rel).map_err(|e| map_io(e, &rel))?;
				let Some(text) = self.read_gitignore()? else {
					return Ok(());
				};
				if gitignore::ignores(&text, &rel) {
					// Covered already (by the block or by the user's own
					// lines); still validate the block so damage is
					// reported the same way.
					gitignore::managed_entries(&text)?;
					return Ok(());
				}
				if let Some(new_text) = gitignore::add_entry(&text, &rel)? {
					self.write_atomic(GITIGNORE, new_text.as_bytes())?;
					self.refresh_managed(Some(&new_text));
				}
				Ok(())
			}
			Some(_) => Ok(()),
			None => {
				std::fs::create_dir_all(&absolute)?;
				Ok(())
			}
		}
	}

	/// `path` relative to the project root, `/`-separated, if it is inside.
	fn project_relative(&self, absolute: &Path) -> Option<String> {
		// Resolve the deepest existing ancestor so symlinks and `\\?\` prefixes
		// compare equal to the canonical root.
		let mut existing = absolute;
		let mut tail = Vec::new();
		while !existing.exists() {
			tail.push(existing.file_name()?.to_os_string());
			existing = existing.parent()?;
		}
		let mut resolved = std::fs::canonicalize(existing).ok()?;
		resolved.extend(tail.iter().rev());
		let rel = resolved.strip_prefix(&self.root).ok()?;
		let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
		Some(parts?.join("/"))
	}

	// --- writing ----------------------------------------------------------

	/// Atomically replaces a file: written next to the target, then renamed.
	pub fn write(&self, rel: &str, data: &[u8]) -> ZResult<()> {
		if let Some((parent, _)) = rel.rsplit_once('/') {
			self.dir.create_dir_all(parent).map_err(|e| map_io(e, rel))?;
		}
		if self.dir.metadata(rel).is_ok_and(|m| m.is_dir()) {
			return Err(ZenithError::ProjectWrongKind(format!("{rel} is a folder")));
		}
		if self.dir.symlink_metadata(rel).is_ok_and(|m| m.is_symlink()) {
			// Renaming a temporary file over a link would replace the link
			// itself. Write through it instead: cap-std follows it inside the
			// project and refuses it when it leads outside.
			self.dir.write(rel, data).map_err(|e| map_io(e, rel))?;
		} else {
			self.write_atomic(rel, data)?;
		}
		self.core.cache.invalidate(rel);
		Ok(())
	}

	pub fn create_dir(&self, rel: &str) -> ZResult<()> {
		self.dir.create_dir_all(rel).map_err(|e| map_io(e, rel))
	}

	/// Copies a file or a folder tree. The destination must not exist. A folder
	/// that cannot be copied completely (it holds a symlink, or a file whose
	/// name is not valid UTF-8) is not copied at all: what was made is taken
	/// back.
	///
	/// A destination that appears while the copy runs is never replaced: a file
	/// is put in place with a link that fails if the name is taken, a folder is
	/// created with `create_dir`.
	pub fn copy(&self, from: &str, to: &str) -> ZResult<()> {
		check_not_inside(from, to)?;
		if self.dir.symlink_metadata(to).is_ok() {
			return Err(ZenithError::ProjectAlreadyExists(to.to_string()));
		}
		if let Some((parent, _)) = to.rsplit_once('/') {
			self.dir.create_dir_all(parent).map_err(|e| map_io(e, to))?;
		}
		self.copy_tree(from, to)
	}

	fn copy_tree(&self, from: &str, to: &str) -> ZResult<()> {
		let meta = self.dir.symlink_metadata(from).map_err(|e| map_io(e, from))?;
		if meta.is_symlink() {
			return Err(ZenithError::Unsupported(format!("copying the symlink {from}")));
		}
		if meta.is_dir() {
			// `create_dir` fails if `to` exists, so a folder that is there
			// after it succeeded was made by this call, and only then is it
			// taken back.
			self.dir.create_dir(to).map_err(|e| map_io(e, to))?;
			let copied = self.list_complete(from).and_then(|entries| {
				entries.iter().try_for_each(|entry| {
					self.copy_tree(&format!("{from}/{}", entry.name), &format!("{to}/{}", entry.name))
				})
			});
			if copied.is_err() {
				let _ = self.dir.remove_dir_all(to);
			}
			copied
		} else {
			let tmp = tmp_name(to);
			self.dir.copy(from, &self.dir, &tmp).map_err(|e| map_io(e, from))?;
			self.rename_no_replace(&tmp, to).map_err(|e| {
				let _ = self.dir.remove_file(&tmp);
				map_move_error(e, to, to)
			})
		}
	}

	/// Moves `from` to `to` and fails with `AlreadyExists` if `to` is there,
	/// also if it appeared after the caller looked. A file is moved by linking
	/// it to the new name (which fails if the name is taken, atomically) and
	/// then removing the old name; where the file system has no hard links it
	/// is renamed after a last look. A folder is renamed: where a rename may
	/// replace something, that is only an empty folder.
	pub(crate) fn rename_no_replace(&self, from: &str, to: &str) -> io::Result<()> {
		if self.dir.symlink_metadata(from)?.is_file() {
			match self.dir.hard_link(from, &self.dir, to) {
				Ok(()) => {
					return self.dir.remove_file(from).inspect_err(|_| {
						// Both names must not be left behind.
						let _ = self.dir.remove_file(to);
					});
				}
				Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(e),
				// No hard links here (FAT, some network drives).
				Err(_) => {}
			}
		}
		if self.dir.symlink_metadata(to).is_ok() {
			return Err(io::ErrorKind::AlreadyExists.into());
		}
		self.dir.rename(from, &self.dir, to)
	}

	/// Moves a file or folder. The destination must not exist; one that appears
	/// while the move runs is not replaced (see [`Self::rename_no_replace`]).
	pub fn rename(&self, from: &str, to: &str) -> ZResult<()> {
		check_not_inside(from, to)?;
		if self.dir.symlink_metadata(from).is_err() {
			return Err(ZenithError::ProjectNotFound(from.to_string()));
		}
		if self.dir.symlink_metadata(to).is_ok() {
			return Err(ZenithError::ProjectAlreadyExists(to.to_string()));
		}
		if let Some((parent, _)) = to.rsplit_once('/') {
			self.dir.create_dir_all(parent).map_err(|e| map_io(e, to))?;
		}
		self.rename_no_replace(from, to)
			.map_err(|e| map_move_error(e, from, to))?;
		self.core.cache.invalidate_prefix(from);
		self.core.cache.invalidate_prefix(to);
		Ok(())
	}

	/// Writes `Settings/CacheConfig.toml` from the settings in effect.
	pub fn save_cache_config(&self) -> ZResult<()> {
		self.dir.create_dir_all(crate::layout::DIR_SETTINGS)?;
		let text = self.core.cache.settings().to_toml()?;
		self.write_atomic(CACHE_CONFIG_FILE, text.as_bytes())
	}
}

/// One entry found by [`FsV1::walk`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkEntry {
	/// Project-relative path with `/` separators.
	pub path: String,
	/// What the entry itself is; links are reported, never followed.
	pub kind: EntryKind,
	/// Size in bytes of a file; 0 for folders and links.
	pub size: u64,
	/// Modification time of a file, if the file system has one.
	pub modified: Option<SystemTime>,
}

impl FsV1 {
	/// Lists everything under the project root, depth first, each folder sorted
	/// by name. A folder is entered only if `descend` returns true for it, so
	/// callers can skip trees they will not use (`Trash/`, `.git/`). A file
	/// whose name is not valid UTF-8 is an error: it cannot be named in a
	/// path, and silently leaving it out would drop it from a build.
	pub fn walk(&self, descend: &dyn Fn(&str) -> bool) -> ZResult<Vec<WalkEntry>> {
		let mut out = Vec::new();
		self.walk_dir(".", "", descend, &mut out)?;
		Ok(out)
	}

	fn walk_dir(
		&self,
		dir: &str,
		prefix: &str,
		descend: &dyn Fn(&str) -> bool,
		out: &mut Vec<WalkEntry>,
	) -> ZResult<()> {
		for entry in self.list_complete(dir)? {
			let path = if prefix.is_empty() {
				entry.name.clone()
			} else {
				format!("{prefix}/{}", entry.name)
			};
			let (size, modified) = if entry.kind == EntryKind::File {
				let meta = self.dir.symlink_metadata(&path).map_err(|e| map_io(e, &path))?;
				(meta.len(), meta.modified().ok().map(system_time))
			} else {
				(0, None)
			};
			out.push(WalkEntry {
				path: path.clone(),
				kind: entry.kind,
				size,
				modified,
			});
			if entry.kind == EntryKind::Dir && descend(&path) {
				self.walk_dir(&path, &path, descend, out)?;
			}
		}
		Ok(())
	}
}

impl FsV1 {
	/// The entries of a folder, sorted by name, and how many were left out
	/// because their names are not valid UTF-8 (a `ZPath` cannot name them).
	/// Temporary files of an atomic write are never listed.
	fn read_entries(&self, rel: &str) -> ZResult<(Vec<DirEntry>, usize)> {
		let mut out = Vec::new();
		let mut skipped = 0usize;
		for entry in self.dir.read_dir(rel).map_err(|e| map_io(e, rel))? {
			let entry = entry?;
			let Some(name) = entry.file_name().to_str().map(str::to_string) else {
				skipped = skipped.saturating_add(1);
				continue;
			};
			if name.ends_with(TMP_SUFFIX) && name.starts_with('.') {
				continue;
			}
			out.push(DirEntry {
				name,
				kind: kind_of(entry.file_type()?),
			});
		}
		out.sort_by(|a, b| a.name.cmp(&b.name));
		Ok((out, skipped))
	}

	/// `list` for operations that must see everything (a copy, a walk for a
	/// build): a name that cannot be represented is an error, not a gap.
	fn list_complete(&self, rel: &str) -> ZResult<Vec<DirEntry>> {
		let (entries, skipped) = self.read_entries(rel)?;
		if skipped > 0 {
			return Err(ZenithError::Unsupported(format!(
				"{rel} holds {skipped} entry/entries whose names are not valid UTF-8"
			)));
		}
		Ok(entries)
	}
}

fn check_not_inside(from: &str, to: &str) -> ZResult<()> {
	if from == to || to.strip_prefix(from).is_some_and(|rest| rest.starts_with('/')) {
		return Err(ZenithError::ProjectWrongKind(format!(
			"cannot place {from} inside itself ({to})"
		)));
	}
	Ok(())
}

/// Sibling temp name for atomic writes: same folder, so the final rename never
/// crosses a filesystem boundary.
pub fn tmp_name(rel: &str) -> String {
	use std::{
		hash::{BuildHasher, Hasher},
		sync::atomic::{AtomicU64, Ordering},
	};
	static COUNTER: AtomicU64 = AtomicU64::new(0);
	let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
	hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
	let suffix = format!("{:08x}", hasher.finish() & 0xffff_ffff);
	match rel.rsplit_once('/') {
		Some((parent, name)) => format!("{parent}/.{name}.{suffix}{TMP_SUFFIX}"),
		None => format!(".{rel}.{suffix}{TMP_SUFFIX}"),
	}
}

impl Backend for FsV1 {
	fn core(&self) -> &Core {
		&self.core
	}

	fn exists(&self, rel: &str) -> ZResult<bool> {
		self.dir.try_exists(rel).map_err(|e| map_io(e, rel))
	}

	fn list(&self, rel: &str) -> ZResult<Vec<DirEntry>> {
		Ok(self.read_entries(rel)?.0)
	}

	fn metadata(&self, rel: &str) -> ZResult<Metadata> {
		let meta = self.dir.metadata(rel).map_err(|e| map_io(e, rel))?;
		Ok(Metadata {
			kind: if meta.is_dir() { EntryKind::Dir } else { EntryKind::File },
			size: if meta.is_dir() { 0 } else { meta.len() },
		})
	}

	fn is_symlink(&self, rel: &str) -> ZResult<bool> {
		Ok(self.dir.symlink_metadata(rel).map_err(|e| map_io(e, rel))?.is_symlink())
	}

	fn read_raw(&self, rel: &str) -> ZResult<Vec<u8>> {
		if self.dir.metadata(rel).is_ok_and(|m| m.is_dir()) {
			return Err(ZenithError::ProjectWrongKind(format!("{rel} is a folder")));
		}
		self.dir.read(rel).map_err(|e| map_io(e, rel))
	}

	fn open_read(&self, rel: &str) -> ZResult<Box<dyn ReadSeek + Send>> {
		if self.dir.metadata(rel).is_ok_and(|m| m.is_dir()) {
			return Err(ZenithError::ProjectWrongKind(format!("{rel} is a folder")));
		}
		Ok(Box::new(self.dir.open(rel).map_err(|e| map_io(e, rel))?.into_std()))
	}

	fn stamp(&self, rel: &str) -> Option<Stamp> {
		let meta = self.dir.metadata(rel).ok()?;
		Some(Stamp {
			mtime: meta.modified().ok().map(system_time),
			size: meta.len(),
		})
	}
}

/// Whether `path` is a project folder with the structure flag. Used to route
/// `open` before anything else is read.
pub fn has_v1_flag(path: &Path) -> bool {
	path.join(V1_FLAG).is_file()
}

/// Other `.vN` flags: a project from a newer structure version.
pub fn has_other_version_flag(path: &Path) -> bool {
	std::fs::read_dir(path).is_ok_and(|entries| {
		entries.flatten().any(|e| {
			e.file_name().to_str().is_some_and(|n| {
				n != V1_FLAG
					&& n.len() > 2 && n.starts_with(".v")
					&& n.get(2..).is_some_and(|d| d.bytes().all(|b| b.is_ascii_digit()))
			})
		})
	})
}
