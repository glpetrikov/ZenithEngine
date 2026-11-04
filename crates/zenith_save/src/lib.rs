//! Saves: the progress of a player, in named slots.
//!
//! A save is one file per slot, `Data://Saves/<slot>.zenithsave`, so it lives
//! in the data of the player (see
//! [`zenith_filesystem::userdata`]) and not in the game. It is a container of
//! the same format as a build: compressed, and encrypted if you give a key,
//! with a few things said about it in a header (the title, the version of the
//! data, when it was saved) and any number of named entries, such as the
//! snapshot of the world.
//!
//! Writing is safe against a crash. A save is built and checked in memory
//! first, and replaces the file atomically; the save that was there is kept
//! as `<slot>.zenithsave.bak` if it was good.
//!
//! A damaged save is an error: [`Saves::load`] gives `SaveDamaged` and never
//! quietly hands out something older. The backup is there when you want it:
//! [`Saves::load_backup`], [`Saves::restore_backup`], or
//! [`Saves::load_or_backup`] for a game that asks nothing of the player.
//!
//! ```no_run
//! use zenith_filesystem::Project;
//! use zenith_save::{Save, SaveMeta, Saves, WORLD_ENTRY};
//!
//! # fn main() -> Result<(), zenith_error::ZenithError> {
//! let project = Project::open("Game")?;
//! let saves = Saves::new(&project);
//!
//! let world_bytes = b"...".to_vec(); // World::snapshot_bytes()
//! let save = Save::new(SaveMeta::new("Level 3, before the boss", 1))
//! 	.with_entry(WORLD_ENTRY, world_bytes);
//! saves.save("slot1", &save)?;
//!
//! for slot in saves.slots()?.saves {
//! 	println!("{}: {}", slot.slot, slot.meta.title);
//! }
//! let loaded = saves.load("slot1")?;
//! # let _ = loaded; Ok(()) }
//! ```

// rustfmt (format_code_in_doc_comments) indents the examples with tabs, like all code here.
#![allow(clippy::tabs_in_doc_comments)]

use std::{
	collections::{BTreeMap, BTreeSet},
	sync::Arc,
};

mod base64;
mod sections;

pub use sections::{Loader, Saver, Section};
use serde::{Deserialize, Serialize};
use web_time::{SystemTime, UNIX_EPOCH};
use zenith_archive::{ArchiveReader, ArchiveWriter, EncryptionKey, WriterOptions, validate_entry_path};
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{Project, ZPath};

/// The format of the header. A save with a higher number is refused.
const FORMAT: u32 = 1;
/// The file with the header. A container of this format has to hold exactly
/// one root file with this extension, which is the one place for it.
const HEADER_FILE: &str = "Save.zenithproject";
const SAVES_DIR: &str = "Saves";
const SAVE_EXTENSION: &str = ".zenithsave";
const BACKUP_EXTENSION: &str = ".zenithsave.bak";
const LONGEST_SLOT_NAME: usize = 64;
/// Compression level of saves: they are written often, so fast.
const LEVEL: i32 = 3;

/// The name of the entry that holds the snapshot of the world, by convention.
pub const WORLD_ENTRY: &str = "world.zenith";

/// What a save says about itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveMeta {
	/// What to show in a list of saves.
	pub title: String,
	/// The version of the data in the save, for the game to compare: it is
	/// yours (the version of the world, say), the engine does not look at it.
	pub data_version: u32,
	/// The engine that wrote the save. Filled in by [`Saves::save`].
	pub engine_version: String,
	/// When it was written, milliseconds since 1970. Filled in by
	/// [`Saves::save`].
	pub saved_at_ms: u64,
	/// Anything else you want to show without opening the save: the level,
	/// the play time.
	pub extra: BTreeMap<String, String>,
}

impl SaveMeta {
	#[must_use]
	pub fn new(title: impl Into<String>, data_version: u32) -> Self {
		Self {
			title: title.into(),
			data_version,
			..Self::default()
		}
	}

	/// The same, with one more thing to show.
	#[must_use]
	pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
		self.extra.insert(key.into(), value.into());
		self
	}
}

/// What is in a save: its header and named entries (a file each).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Save {
	pub meta: SaveMeta,
	/// The data, by name (`/`-separated, like a path inside a build).
	pub entries: BTreeMap<String, Vec<u8>>,
	/// Set by [`Saves::load`]: this is the backup, because the save itself was
	/// damaged. Not written.
	pub from_backup: bool,
}

impl Save {
	#[must_use]
	pub fn new(meta: SaveMeta) -> Self {
		Self {
			meta,
			..Self::default()
		}
	}

	/// The same, with one more entry.
	#[must_use]
	pub fn with_entry(mut self, name: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
		self.entries.insert(name.into(), data.into());
		self
	}

	/// An entry by name.
	#[must_use]
	pub fn entry(&self, name: &str) -> Option<&[u8]> {
		self.entries.get(name).map(Vec::as_slice)
	}
}

/// A save in the list of slots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveInfo {
	pub slot: String,
	pub meta: SaveMeta,
	/// The size of the file.
	pub size: u64,
	/// This is the backup, not the save itself.
	pub from_backup: bool,
	/// The slot has a backup file (the previous save), which may or may not
	/// be readable.
	pub has_backup: bool,
}

/// A slot whose save cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DamagedSlot {
	pub slot: String,
	/// What is wrong with it.
	pub error: String,
	/// The backup of the slot can be read: [`Saves::restore_backup`] brings it
	/// back.
	pub has_backup: bool,
}

/// Every slot there is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotList {
	/// The saves that can be read, the newest first.
	pub saves: Vec<SaveInfo>,
	/// The ones that cannot, by name.
	pub damaged: Vec<DamagedSlot>,
}

/// The header, as it is written (the `[save]` table of `Save.zenithproject`).
#[derive(Serialize, Deserialize)]
struct Header {
	save: HeaderTable,
}

#[derive(Serialize, Deserialize)]
struct HeaderTable {
	format: u32,
	engine_version: String,
	data_version: u32,
	saved_at_ms: u64,
	title: String,
	#[serde(default)]
	extra: BTreeMap<String, String>,
}

/// The slots of a project. Cheap: it only borrows the project.
pub struct Saves<'a> {
	project: &'a Project,
	key: Option<EncryptionKey>,
	compress: bool,
	compress_level: i32,
}

impl<'a> Saves<'a> {
	/// The saves of `project`, in its `Data://`.
	#[must_use]
	pub const fn new(project: &'a Project) -> Self {
		Self {
			project,
			key: None,
			compress: true,
			compress_level: LEVEL,
		}
	}

	/// Encrypts the saves, and reads them with this key. A key in a game that
	/// anyone can download stops nobody who looks for it, but it does keep a
	/// save from being edited with a text editor.
	#[must_use]
	pub fn encryption_key(mut self, key: EncryptionKey) -> Self {
		self.key = Some(key);
		self
	}

	/// Whether saves are compressed (default `true`). Off, the entries are
	/// stored as they are.
	#[must_use]
	pub const fn compress(mut self, compress: bool) -> Self {
		self.compress = compress;
		self
	}

	/// The zstd level, `1..=22` (default 3). A browser has one level and
	/// ignores it.
	#[must_use]
	pub const fn compression_level(mut self, level: i32) -> Self {
		self.compress_level = level;
		self
	}

	// --- the slots
	// ------------------------------------------------------------

	/// Writes `save` into the slot, replacing what is there. The old save, if
	/// it was good, becomes the backup. Returns what is listed for the slot.
	///
	/// # Errors
	/// `InvalidSaveSlot` for a bad name, `ArchiveInvalidPath` for a bad entry
	/// name, `ArchiveWrite` for a bad level, or the error of `Data://`.
	pub fn save(&self, slot: &str, save: &Save) -> ZResult<SaveInfo> {
		let main = Self::file(slot, SAVE_EXTENSION)?;
		let backup = Self::file(slot, BACKUP_EXTENSION)?;

		let mut meta = save.meta.clone();
		meta.engine_version = env!("CARGO_PKG_VERSION").to_string();
		meta.saved_at_ms = now_ms();
		let bytes = self.pack(&Self::header_of(&meta), &save.entries)?;
		// Look at it before it replaces anything: a save that cannot be read
		// back must not take the place of one that can.
		let read_back = self.unpack(&bytes)?;
		if read_back.meta != meta {
			return Err(ZenithError::SaveDamaged(format!(
				"{slot}: it did not read back as it was written"
			)));
		}

		// The save that is there is kept as the backup, but only if it is good:
		// a damaged one must not push out the good backup.
		if let Ok(old) = self.project.read_bytes(&main)
			&& self.unpack(&old).is_ok()
		{
			self.project.write(&backup, &old)?;
		}
		self.project.write(&main, &bytes)?;
		Ok(SaveInfo {
			slot: slot.to_string(),
			meta,
			size: bytes.len() as u64,
			from_backup: false,
			has_backup: self.project.exists(&backup)?,
		})
	}

	/// Reads the save of the slot. A save that cannot be read is an error, even
	/// when the slot has a backup that can: what the player gets back is for
	/// the game to decide (see [`Self::load_backup`] and
	/// [`Self::restore_backup`]).
	///
	/// # Errors
	/// `SaveNotFound` for a slot with no save, `SaveDamaged` (the message says
	/// what is wrong) for a save that cannot be read, `SaveVersionMismatch`
	/// for a save of a newer engine, `ArchiveEncryptedNoKey` and
	/// `ArchiveWrongKey` for key problems, `InvalidSaveSlot` for a bad name.
	pub fn load(&self, slot: &str) -> ZResult<Save> {
		self.read_slot(slot, SAVE_EXTENSION).map(|(save, _)| save)
	}

	/// Reads the backup of the slot (the save from before the last one), with
	/// `from_backup` set.
	///
	/// # Errors
	/// As [`Self::load`], for the backup: `SaveNotFound` if there is none.
	pub fn load_backup(&self, slot: &str) -> ZResult<Save> {
		self.read_slot(slot, BACKUP_EXTENSION).map(|(save, _)| save)
	}

	/// The save if it can be read, the backup if not (`from_backup` says
	/// which, and the log says that it happened). For a game that would rather
	/// give the player older progress than an error. A problem of the setup,
	/// such as a wrong key or a save of a newer engine, is still an error:
	/// the backup would fail the same way, or be older than it should be.
	///
	/// # Errors
	/// The error of [`Self::load`] when the backup cannot be used either.
	pub fn load_or_backup(&self, slot: &str) -> ZResult<Save> {
		match self.load(slot) {
			Err(error @ (ZenithError::SaveNotFound(_) | ZenithError::SaveDamaged(_))) => match self.load_backup(slot) {
				Ok(save) => {
					zenith_log::warn!("The save in slot {slot} cannot be used ({error}): using its backup");
					Ok(save)
				}
				Err(backup_error) if is_setup_error(&backup_error) => Err(backup_error),
				Err(_) => Err(error),
			},
			other => other,
		}
	}

	/// Puts the backup in place of the save of the slot (which may be damaged
	/// or missing, or simply not the one the player wants). The backup stays as
	/// it is.
	///
	/// # Errors
	/// `SaveNotFound` if there is no backup, `SaveDamaged` if it cannot be
	/// read, or the error of `Data://`.
	pub fn restore_backup(&self, slot: &str) -> ZResult<SaveInfo> {
		let (save, size) = self.read_slot(slot, BACKUP_EXTENSION)?;
		let backup = self
			.read_file(slot, BACKUP_EXTENSION)?
			.ok_or_else(|| ZenithError::SaveNotFound(slot.to_string()))?;
		self.project.write(&Self::file(slot, SAVE_EXTENSION)?, &backup)?;
		Ok(SaveInfo {
			slot: slot.to_string(),
			meta: save.meta,
			size,
			from_backup: false,
			has_backup: true,
		})
	}

	/// What the slot says about its save, without the entries; an error in the
	/// cases of [`Self::load`].
	///
	/// # Errors
	/// As [`Self::load`].
	pub fn info(&self, slot: &str) -> ZResult<SaveInfo> {
		let (save, size) = self.read_slot(slot, SAVE_EXTENSION)?;
		Ok(SaveInfo {
			slot: slot.to_string(),
			meta: save.meta,
			size,
			from_backup: false,
			has_backup: self.project.exists(&Self::file(slot, BACKUP_EXTENSION)?)?,
		})
	}

	/// One of the two files of a slot, read and understood, and its size.
	fn read_slot(&self, slot: &str, extension: &str) -> ZResult<(Save, u64)> {
		let is_backup = extension == BACKUP_EXTENSION;
		let Some(bytes) = self.read_file(slot, extension)? else {
			return Err(ZenithError::SaveNotFound(slot.to_string()));
		};
		match self.unpack(&bytes) {
			Ok(mut save) => {
				save.from_backup = is_backup;
				Ok((save, bytes.len() as u64))
			}
			Err(e) if is_setup_error(&e) => Err(e),
			Err(e) => Err(ZenithError::SaveDamaged(format!(
				"{slot}{}: {e}",
				if is_backup { " (backup)" } else { "" }
			))),
		}
	}

	/// Whether the slot has a save or a backup of one (damaged or not).
	///
	/// # Errors
	/// `InvalidSaveSlot` for a bad name, or the error of `Data://`.
	pub fn exists(&self, slot: &str) -> ZResult<bool> {
		for extension in [SAVE_EXTENSION, BACKUP_EXTENSION] {
			if self.project.exists(&Self::file(slot, extension)?)? {
				return Ok(true);
			}
		}
		Ok(false)
	}

	/// Deletes the save and its backup for good; `false` if there was nothing.
	///
	/// # Errors
	/// `InvalidSaveSlot` for a bad name, or the error of `Data://`.
	pub fn delete(&self, slot: &str) -> ZResult<bool> {
		let mut deleted = false;
		for extension in [SAVE_EXTENSION, BACKUP_EXTENSION] {
			deleted |= self.project.delete(&Self::file(slot, extension)?)?;
		}
		Ok(deleted)
	}

	/// Every slot: the saves that can be read (newest first) and the ones that
	/// cannot.
	///
	/// # Errors
	/// `ArchiveEncryptedNoKey` and `ArchiveWrongKey` for key problems, or the
	/// error of `Data://`.
	pub fn slots(&self) -> ZResult<SlotList> {
		let folder = ZPath::from_parts(zenith_filesystem::DATA_SCHEME, SAVES_DIR)?;
		let entries = match self.project.list(&folder) {
			Ok(entries) => entries,
			Err(ZenithError::ProjectNotFound(_)) => return Ok(SlotList::default()),
			Err(e) => return Err(e),
		};
		let slots: BTreeSet<String> = entries
			.into_iter()
			.filter_map(|entry| {
				// The longer ending first: `x.zenithsave.bak` also ends in
				// `.bak` only, but be exact about it.
				entry
					.name
					.strip_suffix(BACKUP_EXTENSION)
					.or_else(|| entry.name.strip_suffix(SAVE_EXTENSION))
					.map(str::to_string)
			})
			.collect();

		let mut list = SlotList::default();
		for slot in slots {
			match self.info(&slot) {
				Ok(info) => list.saves.push(info),
				Err(error @ (ZenithError::SaveNotFound(_) | ZenithError::SaveDamaged(_))) => {
					// Only the backup is left, or the save cannot be read.
					let has_backup = self.read_slot(&slot, BACKUP_EXTENSION).is_ok();
					let error = match error {
						ZenithError::SaveNotFound(_) => "the save is missing".to_string(),
						other => other.to_string(),
					};
					list.damaged.push(DamagedSlot {
						slot,
						error,
						has_backup,
					});
				}
				Err(e) => return Err(e),
			}
		}
		list.saves.sort_by(|a, b| {
			b.meta
				.saved_at_ms
				.cmp(&a.meta.saved_at_ms)
				.then_with(|| a.slot.cmp(&b.slot))
		});
		Ok(list)
	}

	// --- inside
	// ---------------------------------------------------------------

	/// `Data://Saves/<slot><extension>`. A slot name is made of letters,
	/// digits, spaces, `_`, `-` and `.`, up to 64 of them, and does not begin
	/// or end with a space or a dot.
	fn file(slot: &str, extension: &str) -> ZResult<ZPath> {
		let bad = |why: &str| ZenithError::InvalidSaveSlot(format!("{slot:?}: {why}"));
		if slot.is_empty() || slot.chars().count() > LONGEST_SLOT_NAME {
			return Err(bad("it must be 1 to 64 characters"));
		}
		if !slot
			.chars()
			.all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.'))
		{
			return Err(bad("only letters, digits, spaces, `_`, `-` and `.` are allowed"));
		}
		if slot.starts_with(['.', ' ']) || slot.ends_with(['.', ' ']) {
			return Err(bad("it cannot begin or end with a space or a dot"));
		}
		ZPath::from_parts(
			zenith_filesystem::DATA_SCHEME,
			&format!("{SAVES_DIR}/{slot}{extension}"),
		)
		.map_err(|e| bad(&e.to_string()))
	}

	/// The bytes of a slot's file; `None` if there is none.
	fn read_file(&self, slot: &str, extension: &str) -> ZResult<Option<Arc<[u8]>>> {
		let path = Self::file(slot, extension)?;
		if !self.project.exists(&path)? {
			return Ok(None);
		}
		self.project.read_bytes(&path).map(Some)
	}

	fn header_of(meta: &SaveMeta) -> Header {
		Header {
			save: HeaderTable {
				format: FORMAT,
				engine_version: meta.engine_version.clone(),
				data_version: meta.data_version,
				saved_at_ms: meta.saved_at_ms,
				title: meta.title.clone(),
				extra: meta.extra.clone(),
			},
		}
	}

	/// The container for a header and entries.
	fn pack(&self, header: &Header, entries: &BTreeMap<String, Vec<u8>>) -> ZResult<Vec<u8>> {
		let mut files: BTreeMap<&str, Vec<u8>> = BTreeMap::new();
		for (name, data) in entries {
			check_entry_name(name)?;
			files.insert(name, data.clone());
		}
		let header = toml::to_string(header).map_err(|e| ZenithError::Toml(e.to_string()))?;
		files.insert(HEADER_FILE, header.into_bytes());

		let options = WriterOptions {
			compress: self.compress,
			compression_level: self.compress_level,
			key: self.key.clone(),
			..WriterOptions::default()
		};
		// A container wants its entries in the order of their paths, which is
		// the order of the map.
		let mut writer = ArchiveWriter::new(Vec::new(), &options)?;
		for (path, data) in &files {
			writer.add_bytes(path, data)?;
		}
		writer.finish()
	}

	/// A container opened, and its header read; the entries are not.
	fn open_container(&self, bytes: &[u8]) -> ZResult<(ArchiveReader, SaveMeta)> {
		let reader = ArchiveReader::open(Arc::<[u8]>::from(bytes), self.key.as_ref())?;
		let header = reader.read_all(HEADER_FILE)?;
		let header =
			std::str::from_utf8(&header).map_err(|e| ZenithError::ArchiveCorrupt(format!("the header: {e}")))?;
		let Header { save: table } = toml::from_str(header).map_err(|e| ZenithError::Toml(e.to_string()))?;
		if table.format > FORMAT {
			return Err(ZenithError::SaveVersionMismatch(format!(
				"it is of format {}, this engine reads up to {FORMAT} (it was written by engine {})",
				table.format, table.engine_version
			)));
		}
		let meta = SaveMeta {
			title: table.title,
			data_version: table.data_version,
			engine_version: table.engine_version,
			saved_at_ms: table.saved_at_ms,
			extra: table.extra,
		};
		Ok((reader, meta))
	}

	/// The header and entries of a container.
	fn unpack(&self, bytes: &[u8]) -> ZResult<Save> {
		let (reader, meta) = self.open_container(bytes)?;
		let mut entries = BTreeMap::new();
		for entry in reader.entries() {
			if entry.path != HEADER_FILE {
				entries.insert(entry.path.clone(), reader.read_all(&entry.path)?);
			}
		}
		Ok(Save {
			meta,
			entries,
			from_backup: false,
		})
	}

	/// Opens a file of a slot for reading by part: the header is read, the
	/// entries wait until they are asked for.
	fn open_slot(&self, slot: &str, extension: &str) -> ZResult<Loader> {
		let is_backup = extension == BACKUP_EXTENSION;
		let Some(bytes) = self.read_file(slot, extension)? else {
			return Err(ZenithError::SaveNotFound(slot.to_string()));
		};
		match self.open_container(&bytes) {
			Ok((reader, meta)) => Ok(Loader::new(slot, reader, meta, is_backup)),
			Err(e) if is_setup_error(&e) => Err(e),
			Err(e) => Err(ZenithError::SaveDamaged(format!(
				"{slot}{}: {e}",
				if is_backup { " (backup)" } else { "" }
			))),
		}
	}

	/// Starts a save of the slot made of sections (see [`Saver`]). Nothing is
	/// written until [`Saver::finish`].
	#[must_use]
	pub fn saver(&self, slot: &str, meta: SaveMeta) -> Saver<'_, 'a> {
		Saver::new(self, slot, meta)
	}

	/// Opens the save of the slot for reading by section.
	///
	/// # Errors
	/// As [`Self::load`], for the header; a damaged section is found when it is
	/// asked for, so the others can still be read.
	pub fn loader(&self, slot: &str) -> ZResult<Loader> {
		self.open_slot(slot, SAVE_EXTENSION)
	}

	/// The same for the backup of the slot (`from_backup` is set).
	///
	/// # Errors
	/// As [`Self::load_backup`].
	pub fn loader_backup(&self, slot: &str) -> ZResult<Loader> {
		self.open_slot(slot, BACKUP_EXTENSION)
	}
}

/// Problems with the setup, not with the file: the next copy of the save
/// would not read either, so there is no use falling back to the backup.
const fn is_setup_error(error: &ZenithError) -> bool {
	matches!(
		error,
		ZenithError::ArchiveEncryptedNoKey
			| ZenithError::ArchiveWrongKey
			| ZenithError::ArchiveNotEncrypted
			| ZenithError::SaveVersionMismatch(_)
	)
}

fn check_entry_name(name: &str) -> ZResult<()> {
	validate_entry_path(name)?;
	// The header's place, and the one thing a container of this format has
	// only one of.
	let is_root_manifest = !name.contains('/') && name.to_ascii_lowercase().ends_with(".zenithproject");
	if is_root_manifest {
		return Err(ZenithError::ArchiveInvalidPath(format!(
			"{name}: a root file ending in .zenithproject is the header of the save"
		)));
	}
	Ok(())
}

fn now_ms() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use zenith_filesystem::{DirStore, MemStore};

	use super::*;

	type TestResult = Result<(), Box<dyn std::error::Error>>;

	fn project() -> Result<(tempfile::TempDir, Project), Box<dyn std::error::Error>> {
		let tmp = tempfile::tempdir()?;
		let project = Project::create(tmp.path().join("Game"))?;
		project.set_user_data(Arc::new(MemStore::new()));
		Ok((tmp, project))
	}

	fn world(text: &str) -> Save {
		Save::new(SaveMeta::new(text, 7).with_extra("level", "3")).with_entry(WORLD_ENTRY, text.as_bytes().to_vec())
	}

	fn damage(project: &Project, slot: &str, extension: &str) -> TestResult {
		project.write(&Saves::file(slot, extension)?, b"this is not a save")?;
		Ok(())
	}

	#[test]
	fn a_save_comes_back_as_it_was_written() -> TestResult {
		let (_tmp, project) = project()?;
		let saves = Saves::new(&project);
		let info = saves.save(
			"slot1",
			&world("first").with_entry("extra/thumbnail.png", vec![1, 2, 3]),
		)?;
		assert_eq!(info.meta.title, "first", "title");
		assert_eq!(
			info.meta.engine_version,
			env!("CARGO_PKG_VERSION"),
			"the engine is filled in"
		);
		assert!(info.meta.saved_at_ms > 1_600_000_000_000, "and the time");

		let loaded = saves.load("slot1")?;
		assert_eq!(loaded.meta, info.meta, "the header");
		assert_eq!(loaded.meta.data_version, 7, "the version of the game's data");
		assert_eq!(loaded.meta.extra.get("level").map(String::as_str), Some("3"), "extra");
		assert_eq!(loaded.entry(WORLD_ENTRY), Some(&b"first"[..]), "the world");
		assert_eq!(
			loaded.entry("extra/thumbnail.png"),
			Some(&[1u8, 2, 3][..]),
			"another entry"
		);
		assert!(!loaded.from_backup, "not from the backup");
		assert!(saves.exists("slot1")?, "exists");
		assert!(!saves.exists("slot2")?, "another slot does not");
		Ok(())
	}

	#[test]
	fn a_save_goes_to_the_data_of_the_player_not_to_the_project() -> TestResult {
		let tmp = tempfile::tempdir()?;
		let project = Project::create(tmp.path().join("Game"))?;
		let player = tmp.path().join("player");
		project.set_user_data(Arc::new(DirStore::new(&player)));
		Saves::new(&project).save("slot1", &world("x"))?;
		assert!(
			player.join("Saves").join("slot1.zenithsave").is_file(),
			"a file in the data folder"
		);
		let names: Vec<String> = std::fs::read_dir(player.join("Saves"))?
			.flatten()
			.filter_map(|e| e.file_name().into_string().ok())
			.collect();
		assert_eq!(names, ["slot1.zenithsave"], "and no temporary file left beside it");
		assert!(
			!tmp.path().join("Game").join("Saves").exists(),
			"nothing in the project"
		);
		Ok(())
	}

	#[test]
	fn the_previous_save_is_kept_as_a_backup_and_used_when_the_save_is_damaged() -> TestResult {
		let (_tmp, project) = project()?;
		let saves = Saves::new(&project);
		saves.save("slot1", &world("one"))?;
		assert!(
			!project.exists(&Saves::file("slot1", BACKUP_EXTENSION)?)?,
			"the first save has nothing to back up"
		);
		saves.save("slot1", &world("two"))?;
		assert_eq!(saves.load("slot1")?.entry(WORLD_ENTRY), Some(&b"two"[..]), "the newest");

		damage(&project, "slot1", SAVE_EXTENSION)?;
		// A damaged save is an error, backup or no backup.
		let error = saves.load("slot1").err();
		assert!(
			matches!(error, Some(ZenithError::SaveDamaged(_))),
			"load is an error: {error:?}"
		);
		assert!(
			error.is_some_and(|e| e.to_string().contains("slot1")),
			"which says which slot"
		);
		assert!(
			matches!(saves.info("slot1"), Err(ZenithError::SaveDamaged(_))),
			"so is info"
		);

		// The backup is there for whoever asks for it.
		let backup = saves.load_backup("slot1")?;
		assert!(backup.from_backup, "it says it is the backup");
		assert_eq!(backup.entry(WORLD_ENTRY), Some(&b"one"[..]), "with the older progress");
		let fallback = saves.load_or_backup("slot1")?;
		assert!(fallback.from_backup, "load_or_backup falls back");
		assert_eq!(fallback.entry(WORLD_ENTRY), Some(&b"one"[..]), "to the same save");
		assert_eq!(saves.load_or_backup("slot1")?.meta, backup.meta, "the header too");

		// A damaged save must not replace the good backup.
		saves.save("slot1", &world("three"))?;
		assert_eq!(
			saves.load("slot1")?.entry(WORLD_ENTRY),
			Some(&b"three"[..]),
			"saved again"
		);
		assert!(!saves.load("slot1")?.from_backup, "and it is the save, not the backup");
		damage(&project, "slot1", SAVE_EXTENSION)?;
		assert_eq!(
			saves.load_backup("slot1")?.entry(WORLD_ENTRY),
			Some(&b"one"[..]),
			"the backup is still the good one from before the damage"
		);

		// Bringing the backup back makes it the save again.
		let restored = saves.restore_backup("slot1")?;
		assert_eq!(restored.meta.title, "one", "what was restored");
		assert!(!restored.from_backup, "it is the save now");
		assert_eq!(
			saves.load("slot1")?.entry(WORLD_ENTRY),
			Some(&b"one"[..]),
			"readable again"
		);
		assert!(saves.info("slot1")?.has_backup, "and the backup is still there");
		assert!(
			matches!(saves.restore_backup("nope"), Err(ZenithError::SaveNotFound(_))),
			"nothing to restore"
		);
		Ok(())
	}

	#[test]
	fn nothing_and_everything_damaged_are_told_apart() -> TestResult {
		let (_tmp, project) = project()?;
		let saves = Saves::new(&project);
		assert!(
			matches!(saves.load("nope"), Err(ZenithError::SaveNotFound(_))),
			"an empty slot"
		);
		assert!(
			matches!(saves.load_backup("nope"), Err(ZenithError::SaveNotFound(_))),
			"no backup either"
		);
		damage(&project, "bad", SAVE_EXTENSION)?;
		assert!(
			matches!(saves.load("bad"), Err(ZenithError::SaveDamaged(_))),
			"a damaged save, no backup"
		);
		assert!(
			matches!(saves.load_backup("bad"), Err(ZenithError::SaveNotFound(_))),
			"there is nothing to fall back to"
		);
		assert!(
			matches!(saves.load_or_backup("bad"), Err(ZenithError::SaveDamaged(_))),
			"so the error stays"
		);
		damage(&project, "worse", SAVE_EXTENSION)?;
		damage(&project, "worse", BACKUP_EXTENSION)?;
		assert!(
			matches!(saves.load_backup("worse"), Err(ZenithError::SaveDamaged(_))),
			"both damaged"
		);
		assert!(
			matches!(saves.load_or_backup("worse"), Err(ZenithError::SaveDamaged(_))),
			"also for the fallback"
		);
		assert!(
			matches!(saves.load_or_backup("nope"), Err(ZenithError::SaveNotFound(_))),
			"an empty slot is still empty"
		);
		Ok(())
	}

	#[test]
	fn the_list_is_newest_first_and_names_the_damaged() -> TestResult {
		let (_tmp, project) = project()?;
		let saves = Saves::new(&project);
		assert_eq!(saves.slots()?, SlotList::default(), "nothing yet");
		saves.save("old", &world("old"))?;
		std::thread::sleep(std::time::Duration::from_millis(5));
		saves.save("new", &world("new"))?;
		damage(&project, "broken", SAVE_EXTENSION)?;
		saves.save("saved twice", &world("a"))?;
		saves.save("saved twice", &world("b"))?;
		damage(&project, "saved twice", SAVE_EXTENSION)?;
		saves.save("only backup", &world("x"))?;
		saves.save("only backup", &world("y"))?;
		project.delete(&Saves::file("only backup", SAVE_EXTENSION)?)?;

		let list = saves.slots()?;
		let order: Vec<&str> = list.saves.iter().map(|s| s.slot.as_str()).collect();
		assert_eq!(order, ["new", "old"], "the readable ones, the newest first");
		assert!(list.saves.iter().all(|s| s.size > 0 && !s.has_backup), "with sizes");
		let damaged: Vec<(&str, bool)> = list.damaged.iter().map(|d| (d.slot.as_str(), d.has_backup)).collect();
		assert_eq!(
			damaged,
			[("broken", false), ("only backup", true), ("saved twice", true)],
			"the others by name, with whether the backup can bring them back"
		);
		assert!(
			list.damaged.iter().all(|d| !d.error.is_empty()),
			"each says what is wrong"
		);
		assert_eq!(
			list.damaged
				.iter()
				.find(|d| d.slot == "only backup")
				.map(|d| d.error.as_str()),
			Some("the save is missing"),
			"a slot with only a backup"
		);
		Ok(())
	}

	#[test]
	fn deleting_takes_the_backup_too() -> TestResult {
		let (_tmp, project) = project()?;
		let saves = Saves::new(&project);
		saves.save("slot1", &world("one"))?;
		saves.save("slot1", &world("two"))?;
		assert!(saves.delete("slot1")?, "deleted");
		assert!(!saves.exists("slot1")?, "nothing is left, not even the backup");
		assert!(!saves.delete("slot1")?, "only once");
		assert!(
			matches!(saves.load("slot1"), Err(ZenithError::SaveNotFound(_))),
			"empty"
		);
		Ok(())
	}

	#[test]
	fn encrypted_saves_need_their_key_and_are_not_replaced_by_the_backup_of_a_wrong_one() -> TestResult {
		let (_tmp, project) = project()?;
		let key = EncryptionKey::new([7; 32]);
		let secret = Saves::new(&project).encryption_key(key);
		secret.save("slot1", &world("hidden"))?;
		secret.save("slot1", &world("hidden too"))?;
		assert_eq!(
			secret.load("slot1")?.entry(WORLD_ENTRY),
			Some(&b"hidden too"[..]),
			"with the key"
		);

		let raw = project.read_bytes(&Saves::file("slot1", SAVE_EXTENSION)?)?;
		assert!(!raw.windows(6).any(|w| w == b"hidden"), "the text is not in the file");
		assert!(
			matches!(
				Saves::new(&project).load("slot1"),
				Err(ZenithError::ArchiveEncryptedNoKey)
			),
			"no key"
		);
		let wrong = Saves::new(&project).encryption_key(EncryptionKey::new([8; 32]));
		assert!(
			matches!(wrong.load("slot1"), Err(ZenithError::ArchiveWrongKey)),
			"a wrong key"
		);
		assert!(
			!secret.load("slot1")?.from_backup,
			"a key problem was not taken for damage, so nothing fell back"
		);
		Ok(())
	}

	#[test]
	fn compression_makes_a_save_smaller_and_is_optional() -> TestResult {
		let (_tmp, project) = project()?;
		let big = Save::new(SaveMeta::new("big", 1)).with_entry(WORLD_ENTRY, "entity: 1\n".repeat(20_000));
		let packed = Saves::new(&project).save("packed", &big)?;
		let stored = Saves::new(&project).compress(false).save("stored", &big)?;
		assert!(packed.size * 4 < stored.size, "{} against {}", packed.size, stored.size);
		assert_eq!(
			Saves::new(&project).load("stored")?.entries,
			big.entries,
			"a stored save reads too"
		);
		assert_eq!(
			Saves::new(&project).load("packed")?.entries,
			big.entries,
			"and a compressed one"
		);
		Ok(())
	}

	#[test]
	fn slot_and_entry_names_are_checked() -> TestResult {
		let (_tmp, project) = project()?;
		let saves = Saves::new(&project);
		for bad in [
			"",
			" ",
			".hidden",
			"trailing.",
			"a/b",
			"a\\b",
			"x:y",
			"CON",
			&"n".repeat(65),
			"tab\t",
		] {
			assert!(saves.save(bad, &world("x")).is_err(), "{bad:?} is not a slot name");
		}
		for good in ["slot1", "Quick Save", "auto-save_2", "Сохранение", "v1.2"] {
			assert!(saves.save(good, &world("x")).is_ok(), "{good:?} is");
		}
		for bad in ["", "/a", "../a", "Save.zenithproject", "Other.ZenithProject", "a//b"] {
			let save = Save::new(SaveMeta::new("x", 1)).with_entry(bad, vec![1]);
			assert!(saves.save("slot", &save).is_err(), "{bad:?} is not an entry name");
		}
		let nested = Save::new(SaveMeta::new("x", 1)).with_entry("dir/file.zenithproject", vec![1]);
		assert!(
			saves.save("slot", &nested).is_ok(),
			"a manifest-like name below the root is fine"
		);
		Ok(())
	}

	#[test]
	fn a_save_of_a_newer_format_is_refused_not_guessed_at() -> TestResult {
		let (_tmp, project) = project()?;
		let saves = Saves::new(&project);
		saves.save("slot1", &world("old"))?;
		let mut header = Saves::header_of(&world("future").meta);
		header.save.format = FORMAT + 1;
		let bytes = saves.pack(&header, &BTreeMap::new())?;
		project.write(&Saves::file("slot1", SAVE_EXTENSION)?, &bytes)?;
		assert!(
			matches!(saves.load("slot1"), Err(ZenithError::SaveVersionMismatch(_))),
			"refused, and the older backup is not quietly put in its place"
		);
		Ok(())
	}

	#[test]
	fn a_save_that_does_not_read_back_replaces_nothing() -> TestResult {
		let (_tmp, project) = project()?;
		let saves = Saves::new(&project);
		saves.save("slot1", &world("good"))?;
		// A level outside 1..=22 makes the writer fail before anything is
		// stored.
		let broken = Saves::new(&project).compression_level(99);
		assert!(broken.save("slot1", &world("never")).is_err(), "the save fails");
		assert_eq!(
			saves.load("slot1")?.entry(WORLD_ENTRY),
			Some(&b"good"[..]),
			"and the old one is untouched"
		);
		Ok(())
	}
}
