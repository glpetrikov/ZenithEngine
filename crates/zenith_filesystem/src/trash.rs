//! The project trash: `Trash/<TrashId>/entry.toml` +
//! `Trash/<TrashId>/files/<name>`.
//!
//! There is no index: the state of the trash is whatever is on disk, so it
//! cannot drift out of sync. `remove` never deletes anything for good; only
//! `purge`/`purge_all` do.
//!
//! Crash safety by ordering: `remove` creates the entry folder and writes
//! `entry.toml` before moving the object, `restore` moves the object back
//! before deleting the entry folder. A crash in between leaves either an entry
//! without an object (skipped by `trash_list`, cleaned by `purge_all`) or an
//! object that is still restorable.

use std::{
	hash::{BuildHasher, Hasher},
	io,
	sync::atomic::{AtomicU64, Ordering},
	time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use zenith_error::{ZResult, ZenithError};

use crate::{
	core::Backend,
	fs::{FsV1, map_io, map_move_error},
	layout::DIR_TRASH,
	trash_id::TrashId,
	types::EntryKind,
	zpath::ZPath,
};

#[derive(Serialize, Deserialize)]
struct EntryFile {
	/// Original location as a `ZPath`, so the trash survives moving the project
	/// folder.
	original: String,
	deleted_at_ms: u64,
	kind: String,
	size: u64,
	/// The folder was listed in the managed `.gitignore` block; restoring it
	/// re-adds the entry.
	#[serde(default)]
	ignored: bool,
}

/// One item in the trash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashEntry {
	pub id: TrashId,
	pub original: ZPath,
	pub deleted_at: SystemTime,
	/// `File` or `Dir`.
	pub kind: EntryKind,
	/// Bytes; for a folder the sum of its files.
	pub size: u64,
}

/// What to do when the restore target already exists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OnConflict {
	/// Return an error and change nothing.
	#[default]
	Fail,
	/// Restore under a new name with a numeric suffix (`ferris-1.png`).
	KeepBoth,
	/// The existing object goes to the trash first, so nothing is lost for
	/// good.
	Overwrite,
}

#[derive(Clone, Debug, Default)]
pub struct RestoreOptions {
	/// Restore here instead of the original location.
	pub to: Option<ZPath>,
	pub on_conflict: OnConflict,
}

/// Milliseconds since the epoch, never repeating within the process: two
/// removals in the same millisecond still get ids in deletion order.
fn now_ms() -> u64 {
	static LAST: AtomicU64 = AtomicU64::new(0);
	let now = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
	let mut last = LAST.load(Ordering::Relaxed);
	loop {
		let next = now.max(last.saturating_add(1));
		match LAST.compare_exchange_weak(last, next, Ordering::Relaxed, Ordering::Relaxed) {
			Ok(_) => return next,
			Err(seen) => last = seen,
		}
	}
}

fn random_suffix() -> u32 {
	let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
	hasher.write_u128(
		SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap_or(Duration::ZERO)
			.as_nanos(),
	);
	u32::try_from(hasher.finish() & 0xffff_ffff).unwrap_or(0)
}

fn entry_dir(id: &TrashId) -> String {
	format!("{DIR_TRASH}/{id}")
}

impl FsV1 {
	fn dir_size(&self, rel: &str) -> u64 {
		self.list(rel).map_or(0, |entries| {
			entries
				.iter()
				.map(|e| {
					let child = format!("{rel}/{}", e.name);
					match e.kind {
						EntryKind::Dir => self.dir_size(&child),
						_ => self.dir.symlink_metadata(&child).map_or(0, |m| m.len()),
					}
				})
				.sum()
		})
	}

	/// Moves a file or folder into the trash.
	pub fn remove(&self, rel: &str, original: &ZPath) -> ZResult<TrashId> {
		if self
			.core
			.schemes
			.read()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.is_root(rel)
		{
			return Err(ZenithError::Unsupported(format!("removing the scheme root {original}")));
		}
		let meta = self.dir.symlink_metadata(rel).map_err(|e| map_io(e, rel))?;
		let (kind, size) = if meta.is_dir() {
			("dir", self.dir_size(rel))
		} else {
			("file", meta.len())
		};
		let ignored = meta.is_dir()
			&& self
				.managed
				.read()
				.unwrap_or_else(std::sync::PoisonError::into_inner)
				.iter()
				.any(|entry| entry == rel);
		let name = rel.rsplit_once('/').map_or(rel, |(_, name)| name);

		self.dir.create_dir_all(DIR_TRASH)?;
		let (id, base) = self.new_entry_dir()?;
		let record = EntryFile {
			original: original.to_string(),
			deleted_at_ms: id.timestamp_ms(),
			kind: kind.to_string(),
			size,
			ignored,
		};
		let moved = (|| -> ZResult<()> {
			let text = toml::to_string(&record).map_err(|e| ZenithError::Toml(e.to_string()))?;
			self.dir.write(format!("{base}/entry.toml"), text)?;
			self.dir.create_dir(format!("{base}/files"))?;
			self.dir
				.rename(rel, &self.dir, format!("{base}/files/{name}"))
				.map_err(|e| map_io(e, rel))
		})();
		if let Err(e) = moved {
			let _ = self.dir.remove_dir_all(&base);
			return Err(e);
		}
		self.core.cache.invalidate_prefix(rel);
		self.sync_gitignore_lenient();
		Ok(id)
	}

	/// `create_dir` is atomic, so a clash with another entry (or process) is
	/// detected and retried with a new suffix.
	fn new_entry_dir(&self) -> ZResult<(TrashId, String)> {
		for _ in 0..32 {
			let id = TrashId::new(now_ms(), random_suffix());
			let base = entry_dir(&id);
			match self.dir.create_dir(&base) {
				Ok(()) => return Ok((id, base)),
				Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
				Err(e) => return Err(e.into()),
			}
		}
		Err(ZenithError::Other("could not allocate a unique trash id".into()))
	}

	fn load_entry(&self, id: &TrashId) -> ZResult<(TrashEntry, bool, String)> {
		let base = entry_dir(id);
		let text = self
			.dir
			.read_to_string(format!("{base}/entry.toml"))
			.map_err(|_| ZenithError::TrashEntryNotFound(id.to_string()))?;
		let record: EntryFile = toml::from_str(&text).map_err(|e| ZenithError::Toml(e.to_string()))?;
		let mut objects = self
			.dir
			.read_dir(format!("{base}/files"))
			.map_err(|_| ZenithError::TrashEntryNotFound(id.to_string()))?;
		let object = objects
			.next()
			.ok_or_else(|| ZenithError::TrashEntryNotFound(id.to_string()))??;
		if objects.next().is_some() {
			return Err(ZenithError::TrashEntryNotFound(format!("{id}: more than one object")));
		}
		let name = object
			.file_name()
			.into_string()
			.map_err(|_| ZenithError::TrashEntryNotFound(id.to_string()))?;
		let kind = match record.kind.as_str() {
			"dir" => EntryKind::Dir,
			"file" => EntryKind::File,
			other => {
				return Err(ZenithError::InvalidSettings(format!(
					"{base}/entry.toml: unknown kind {other:?}"
				)));
			}
		};
		let entry = TrashEntry {
			id: id.clone(),
			original: ZPath::parse(&record.original)?,
			deleted_at: UNIX_EPOCH + Duration::from_millis(record.deleted_at_ms),
			kind,
			size: record.size,
		};
		Ok((entry, record.ignored, format!("{base}/files/{name}")))
	}

	/// Entries with an object, newest first. Damaged entries are skipped.
	pub fn trash_list(&self) -> ZResult<Vec<TrashEntry>> {
		let children = match self.dir.read_dir(DIR_TRASH) {
			Ok(children) => children,
			Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
			Err(e) => return Err(e.into()),
		};
		let mut entries = Vec::new();
		for child in children {
			let Ok(child) = child else { continue };
			let Some(id) = child.file_name().to_str().and_then(|n| TrashId::parse(n).ok()) else {
				continue;
			};
			if let Ok((entry, ..)) = self.load_entry(&id) {
				entries.push(entry);
			}
		}
		entries.sort_by(|a, b| b.id.cmp(&a.id));
		Ok(entries)
	}

	/// Entries whose original location is `path`.
	pub fn trash_find(&self, path: &ZPath) -> ZResult<Vec<TrashEntry>> {
		Ok(self.trash_list()?.into_iter().filter(|e| &e.original == path).collect())
	}

	/// Restores the newest entry.
	pub fn restore_last(&self) -> ZResult<ZPath> {
		let newest = self
			.trash_list()?
			.into_iter()
			.next()
			.ok_or_else(|| ZenithError::TrashEntryNotFound("the trash is empty".into()))?;
		self.restore_with(&newest.id, &RestoreOptions::default())
	}

	/// Restores to the original location (`on_conflict = Fail`).
	pub fn restore(&self, id: &TrashId) -> ZResult<ZPath> {
		self.restore_with(id, &RestoreOptions::default())
	}

	pub fn restore_with(&self, id: &TrashId, options: &RestoreOptions) -> ZResult<ZPath> {
		let (entry, ignored, object) = self.load_entry(id)?;
		let mut target = options.to.clone().unwrap_or_else(|| entry.original.clone());
		let mut target_rel = self.core.resolve(&target)?;
		if crate::layout::is_trash_path(&target_rel) {
			return Err(ZenithError::ReadOnlyScheme(target.scheme().to_string()));
		}
		if target.file_name().is_none() {
			return Err(ZenithError::Unsupported("restoring onto a scheme root".into()));
		}

		if self.dir.symlink_metadata(&target_rel).is_ok() {
			match options.on_conflict {
				OnConflict::Fail => return Err(ZenithError::ProjectAlreadyExists(target.to_string())),
				OnConflict::Overwrite => {
					self.remove(&target_rel, &target)?;
				}
				OnConflict::KeepBoth => {
					target = self.free_name(&target)?;
					target_rel = self.core.resolve(&target)?;
				}
			}
		}

		if let Some((parent, _)) = target_rel.rsplit_once('/') {
			self.dir.create_dir_all(parent).map_err(|e| map_io(e, &target_rel))?;
		}
		self.rename_no_replace(&object, &target_rel)
			.map_err(|e| map_move_error(e, &object, &target_rel))?;
		// The object is back; only now is the entry folder disposable.
		let _ = self.dir.remove_dir_all(entry_dir(id));
		self.core.cache.invalidate_prefix(&target_rel);
		// The restore is done; a damaged `.gitignore` must not turn it into an
		// error (the entry is gone, a retry would find nothing).
		if ignored {
			self.ensure_ignored_dir_lenient(&self.root.join(&target_rel));
		}
		Ok(target)
	}

	/// `name-1.ext`, `name-2.ext`, ... until nothing is there.
	fn free_name(&self, target: &ZPath) -> ZResult<ZPath> {
		let name = target.file_name().unwrap_or_default();
		let (stem, ext) = match name.rfind('.') {
			Some(dot) if dot > 0 => name.split_at(dot),
			_ => (name, ""),
		};
		for n in 1..10_000 {
			let candidate = format!("{stem}-{n}{ext}");
			let path = match target.parent() {
				Some(parent) if !parent.path().is_empty() => parent.join(&candidate)?,
				Some(parent) => ZPath::from_parts(parent.scheme(), &candidate)?,
				None => break,
			};
			if self.dir.symlink_metadata(self.core.resolve(&path)?).is_err() {
				return Ok(path);
			}
		}
		Err(ZenithError::ProjectAlreadyExists(target.to_string()))
	}

	/// Deletes one entry for good.
	pub fn purge(&self, id: &TrashId) -> ZResult<()> {
		let base = entry_dir(id);
		if self.dir.symlink_metadata(&base).is_err() {
			return Err(ZenithError::TrashEntryNotFound(id.to_string()));
		}
		self.dir.remove_dir_all(&base).map_err(|e| map_io(e, &base))
	}

	/// Empties the trash, damaged entries included. Only what is named like an
	/// entry (a `TrashId`) is deleted: a file someone else keeps in `Trash/` is
	/// not ours to delete for good.
	pub fn purge_all(&self) -> ZResult<()> {
		let children = match self.dir.read_dir(DIR_TRASH) {
			Ok(children) => children,
			Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
			Err(e) => return Err(e.into()),
		};
		for child in children {
			let child = child?;
			let name = child.file_name();
			let Some(name) = name.to_str().filter(|name| TrashId::parse(name).is_ok()) else {
				continue;
			};
			let path = format!("{DIR_TRASH}/{name}");
			if child.file_type()?.is_dir() {
				self.dir.remove_dir_all(&path).map_err(|e| map_io(e, &path))?;
			} else {
				self.dir.remove_file(&path).map_err(|e| map_io(e, &path))?;
			}
		}
		Ok(())
	}
}
