//! Saves made of sections: values with names, grouped by what they describe.
//!
//! ```no_run
//! use zenith_filesystem::Project;
//! use zenith_save::{SaveMeta, Saves};
//!
//! # fn main() -> Result<(), zenith_error::ZenithError> {
//! let project = Project::open("Game")?;
//! let saves = Saves::new(&project);
//!
//! let mut saver = saves.saver("slot1", SaveMeta::new("Level 3", 1));
//! saver.begin("player")?;
//! saver.save_i32("score", 32)?;
//! saver.save_string("name", "Ada")?;
//! saver.end()?;
//! saver.finish()?;
//!
//! let loader = saves.loader("slot1")?;
//! let player = loader.section("player")?;
//! let score = player.load_i32("score")?;
//! let lives = player.load_i32_or("lives", 3)?;
//! # let _ = (score, lives); Ok(()) }
//! ```
//!
//! A section is a TOML file inside the container of the save
//! (`Sections/<name>.toml`): its version, and its values. The sections are
//! independent, so a damaged one is found by its name when it is asked for and
//! the others can still be read.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use toml::{Table, Value};
use zenith_archive::ArchiveReader;
use zenith_error::{ZResult, ZenithError};

use crate::{HEADER_FILE, Save, SaveInfo, SaveMeta, Saves, base64, check_entry_name, is_setup_error};

const SECTIONS_DIR: &str = "Sections";
const SECTION_EXTENSION: &str = ".toml";
const LONGEST_NAME: usize = 64;

/// A section as it is written.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SectionFile {
	section: SectionHead,
	#[serde(default)]
	values: Table,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SectionHead {
	version: u32,
}

/// Names of sections and of values: 1 to 64 of letters, digits, `_` and `-`.
fn check_name(what: &str, name: &str) -> ZResult<()> {
	let fine = !name.is_empty()
		&& name.chars().count() <= LONGEST_NAME
		&& name
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
	if fine {
		Ok(())
	} else {
		Err(ZenithError::InvalidSaveData(format!(
			"{what} {name:?}: it must be 1 to 64 of ASCII letters, digits, `_` and `-`"
		)))
	}
}

/// Whether a file is in the folder of the sections, whatever the case: on a
/// system that ignores it `sections/x` would be the same folder.
fn in_sections_folder(path: &str) -> bool {
	path.split_once('/')
		.is_some_and(|(folder, _)| folder.eq_ignore_ascii_case(SECTIONS_DIR))
}

fn section_path(name: &str) -> String {
	format!("{SECTIONS_DIR}/{name}{SECTION_EXTENSION}")
}

fn invalid(message: impl Into<String>) -> ZenithError {
	ZenithError::InvalidSaveData(message.into())
}

// --- writing
// ---------------------------------------------------------------

struct Open {
	name: String,
	version: u32,
	values: Table,
}

/// A save being written, section by section. Nothing reaches the disk until
/// [`Self::finish`], and then all at once: a save that is not finished, or
/// fails on the way, leaves the slot as it was.
pub struct Saver<'s, 'a> {
	saves: &'s Saves<'a>,
	slot: String,
	meta: SaveMeta,
	entries: BTreeMap<String, Vec<u8>>,
	sections: BTreeMap<String, SectionFile>,
	open: Option<Open>,
}

macro_rules! savers {
	($($(#[$doc:meta])* $name:ident($ty:ty) => $convert:expr;)*) => {$(
		$(#[$doc])*
		///
		/// # Errors
		/// `InvalidSaveData` if no section is open, the name is bad, or the
		/// name is already in the section.
		pub fn $name(&mut self, key: &str, value: $ty) -> ZResult<()> {
			let value: ZResult<Value> = ($convert)(value);
			self.put(key, value?)
		}
	)*};
}

impl<'s, 'a> Saver<'s, 'a> {
	pub(crate) fn new(saves: &'s Saves<'a>, slot: &str, meta: SaveMeta) -> Self {
		Self {
			saves,
			slot: slot.to_string(),
			meta,
			entries: BTreeMap::new(),
			sections: BTreeMap::new(),
			open: None,
		}
	}

	/// Begins the section `name`, of version 1.
	///
	/// # Errors
	/// As [`Self::begin_versioned`].
	pub fn begin(&mut self, name: &str) -> ZResult<()> {
		self.begin_versioned(name, 1)
	}

	/// Begins the section `name`, of the `version` of its data (from 1), which
	/// [`Section::get_version`] gives back when the save is read: the game
	/// moves old data to the new form by it.
	///
	/// # Errors
	/// `InvalidSaveData` if a section is still open, the name is bad or taken,
	/// or the version is 0.
	pub fn begin_versioned(&mut self, name: &str, version: u32) -> ZResult<()> {
		if let Some(open) = &self.open {
			return Err(invalid(format!(
				"cannot begin {name:?}: the section {:?} is not ended",
				open.name
			)));
		}
		check_name("section", name)?;
		if version == 0 {
			return Err(invalid(format!("section {name:?}: the version starts at 1")));
		}
		if self.sections.contains_key(name) {
			return Err(invalid(format!("section {name:?} is in the save already")));
		}
		self.open = Some(Open {
			name: name.to_string(),
			version,
			values: Table::new(),
		});
		Ok(())
	}

	/// Ends the section that is open.
	///
	/// # Errors
	/// `InvalidSaveData` if none is open.
	pub fn end(&mut self) -> ZResult<()> {
		let open = self.open.take().ok_or_else(|| invalid("end: no section is open"))?;
		self.sections.insert(
			open.name,
			SectionFile {
				section: SectionHead { version: open.version },
				values: open.values,
			},
		);
		Ok(())
	}

	fn put(&mut self, key: &str, value: Value) -> ZResult<()> {
		let open = self
			.open
			.as_mut()
			.ok_or_else(|| invalid(format!("{key:?}: no section is open")))?;
		check_name("value", key)?;
		if open.values.contains_key(key) {
			return Err(invalid(format!("{key:?} is in the section {:?} already", open.name)));
		}
		open.values.insert(key.to_string(), value);
		Ok(())
	}

	savers! {
		/// Saves a `bool` as `key` in the open section.
		save_bool(bool) => |v| Ok(Value::Boolean(v));
		/// Saves an `i32` as `key` in the open section.
		save_i32(i32) => |v| Ok(Value::Integer(i64::from(v)));
		/// Saves a `u32` as `key` in the open section.
		save_u32(u32) => |v| Ok(Value::Integer(i64::from(v)));
		/// Saves an `i64` as `key` in the open section.
		save_i64(i64) => |v| Ok(Value::Integer(v));
		/// Saves a `u64` as `key` in the open section. TOML holds integers up to
		/// `i64::MAX`: a larger one is `InvalidSaveData`.
		save_u64(u64) => |v| i64::try_from(v)
			.map(Value::Integer)
			.map_err(|_| invalid(format!("{v} does not fit in a save: the largest integer is {}", i64::MAX)));
		/// Saves an `f32` as `key` in the open section.
		save_f32(f32) => |v| Ok(Value::Float(f64::from(v)));
		/// Saves an `f64` as `key` in the open section.
		save_f64(f64) => |v| Ok(Value::Float(v));
		/// Saves text as `key` in the open section.
		save_string(&str) => |v: &str| Ok(Value::String(v.to_string()));
		/// Saves bytes as `key` in the open section (kept as base64 text).
		save_bytes(&[u8]) => |v: &[u8]| Ok(Value::String(base64::encode(v)));
	}

	/// Saves anything `serde` can write as `key` in the open section: a struct
	/// becomes a table, a list an array.
	///
	/// # Errors
	/// `InvalidSaveData` as for the other values, `Toml` if the value cannot be
	/// written as TOML.
	pub fn save<T: Serialize + ?Sized>(&mut self, key: &str, value: &T) -> ZResult<()> {
		let value = Value::try_from(value).map_err(|e| ZenithError::Toml(e.to_string()))?;
		self.put(key, value)
	}

	/// A whole section from one value, in one call: its fields are the values
	/// (a struct, or a map). No section may be open.
	///
	/// # Errors
	/// `InvalidSaveData` for a bad or taken name, a version of 0, an open
	/// section, or a value that is not a table; `Toml` if it cannot be written.
	pub fn save_section<T: Serialize + ?Sized>(&mut self, name: &str, version: u32, value: &T) -> ZResult<()> {
		self.begin_versioned(name, version)?;
		let Some(mut open) = self.open.take() else {
			return Err(invalid("the section did not open"));
		};
		let Value::Table(values) = Value::try_from(value).map_err(|e| ZenithError::Toml(e.to_string()))? else {
			return Err(invalid(format!("section {name:?}: it must be a struct or a map")));
		};
		for key in values.keys() {
			check_name("value", key)?;
		}
		open.values = values;
		self.open = Some(open);
		self.end()
	}

	/// Adds a file that is not a section (the snapshot of the world, a
	/// thumbnail) under `name`. See [`Save::entries`].
	///
	/// # Errors
	/// `ArchiveInvalidPath` for a bad name or one in `Sections/`, and
	/// `InvalidSaveData` if it is taken.
	pub fn entry(&mut self, name: &str, data: impl Into<Vec<u8>>) -> ZResult<()> {
		check_entry_name(name)?;
		// Any case: on a system that ignores it `sections/x` would be the same
		// folder.
		if in_sections_folder(name) {
			return Err(ZenithError::ArchiveInvalidPath(format!(
				"{name}: {SECTIONS_DIR}/ is for the sections"
			)));
		}
		if self.entries.contains_key(name) {
			return Err(invalid(format!("the file {name:?} is in the save already")));
		}
		self.entries.insert(name.to_string(), data.into());
		Ok(())
	}

	/// Writes the save to its slot ([`Saves::save`]: atomically, the old one
	/// becomes the backup).
	///
	/// # Errors
	/// `InvalidSaveData` if a section is not ended, or the errors of
	/// [`Saves::save`].
	pub fn finish(self) -> ZResult<SaveInfo> {
		if let Some(open) = &self.open {
			return Err(invalid(format!("the section {:?} is not ended", open.name)));
		}
		let mut save = Save {
			meta: self.meta,
			entries: self.entries,
			from_backup: false,
		};
		for (name, file) in &self.sections {
			let text = toml::to_string(file).map_err(|e| ZenithError::Toml(e.to_string()))?;
			save.entries.insert(section_path(name), text.into_bytes());
		}
		self.saves.save(&self.slot, &save)
	}
}

// --- reading
// ---------------------------------------------------------------

/// A save opened for reading by part: the header is read, a section or file
/// is read when it is asked for.
pub struct Loader {
	slot: String,
	reader: ArchiveReader,
	meta: SaveMeta,
	from_backup: bool,
}

impl Loader {
	pub(crate) fn new(slot: &str, reader: ArchiveReader, meta: SaveMeta, from_backup: bool) -> Self {
		Self {
			slot: slot.to_string(),
			reader,
			meta,
			from_backup,
		}
	}

	/// What the save says about itself.
	#[must_use]
	pub const fn meta(&self) -> &SaveMeta {
		&self.meta
	}

	/// This is the backup of the slot, not the save itself.
	#[must_use]
	pub const fn from_backup(&self) -> bool {
		self.from_backup
	}

	/// The names of the sections, in alphabetical order. Files in `Sections/`
	/// that are not named like a section (a hand-made or damaged container) are
	/// left out.
	#[must_use]
	pub fn sections(&self) -> Vec<String> {
		let prefix = format!("{SECTIONS_DIR}/");
		let mut names: Vec<String> = self
			.reader
			.entries()
			.iter()
			.filter_map(|e| {
				e.path
					.strip_prefix(&prefix)
					.and_then(|rest| rest.strip_suffix(SECTION_EXTENSION))
					.filter(|name| check_name("section", name).is_ok())
					.map(str::to_string)
			})
			.collect();
		names.sort();
		names
	}

	/// Whether the save has the section.
	#[must_use]
	pub fn has_section(&self, name: &str) -> bool {
		self.reader.find(&section_path(name)).is_some()
	}

	/// The names of the files that are not sections.
	#[must_use]
	pub fn entries(&self) -> Vec<String> {
		self.reader
			.entries()
			.iter()
			.filter(|e| e.path != HEADER_FILE && !in_sections_folder(&e.path))
			.map(|e| e.path.clone())
			.collect()
	}

	/// A file that is not a section.
	///
	/// # Errors
	/// `SaveKeyNotFound` if there is none, `SaveDamaged` if it cannot be read.
	pub fn entry(&self, name: &str) -> ZResult<Vec<u8>> {
		if name == HEADER_FILE || in_sections_folder(name) || self.reader.find(name).is_none() {
			return Err(ZenithError::SaveKeyNotFound(format!("the file {name}")));
		}
		self.read(name, name)
	}

	/// Reads and understands one section.
	///
	/// # Errors
	/// `SaveSectionNotFound` if there is none, `SaveDamaged` (the message names
	/// the section) if it cannot be read, `InvalidSaveData` for a bad name, or
	/// the key errors of an encrypted save.
	pub fn section(&self, name: &str) -> ZResult<Section> {
		check_name("section", name)?;
		let path = section_path(name);
		if self.reader.find(&path).is_none() {
			return Err(ZenithError::SaveSectionNotFound(name.to_string()));
		}
		let bytes = self.read(&path, &format!("section {name}"))?;
		let damaged = |why: String| ZenithError::SaveDamaged(format!("{}: section {name}: {why}", self.slot));
		let text = std::str::from_utf8(&bytes).map_err(|e| damaged(e.to_string()))?;
		let file: SectionFile = toml::from_str(text).map_err(|e| damaged(e.to_string()))?;
		if file.section.version == 0 {
			return Err(damaged("the version starts at 1".to_string()));
		}
		Ok(Section {
			name: name.to_string(),
			version: file.section.version,
			values: file.values,
		})
	}

	fn read(&self, path: &str, what: &str) -> ZResult<Vec<u8>> {
		self.reader.read_all(path).map_err(|e| {
			if is_setup_error(&e) {
				e
			} else {
				ZenithError::SaveDamaged(format!("{}: {what}: {e}", self.slot))
			}
		})
	}
}

/// The values of one section.
#[derive(Clone, Debug)]
pub struct Section {
	name: String,
	version: u32,
	values: Table,
}

macro_rules! getters {
	($($load:ident, $load_or:ident, $ty:ty, $via:ident, $what:literal;)*) => {$(
		#[doc = concat!("The ", $what, " saved as `key`.")]
		///
		/// # Errors
		/// `SaveKeyNotFound` if there is no such value, `SaveWrongType` if it is
		/// not of this type (or the integer does not fit).
		pub fn $load(&self, key: &str) -> ZResult<$ty> {
			self.$via(key, $what)
		}

		#[doc = concat!("The ", $what, " saved as `key`, or `default` if there is no such value.")]
		///
		/// # Errors
		/// `SaveWrongType` if the value is there but not of this type.
		pub fn $load_or(&self, key: &str, default: $ty) -> ZResult<$ty> {
			if self.has(key) {
				self.$load(key)
			} else {
				Ok(default)
			}
		}
	)*};
}

impl Section {
	/// The name of the section.
	#[must_use]
	pub fn name(&self) -> &str {
		&self.name
	}

	/// The version the section was saved with ([`Saver::begin_versioned`]; 1
	/// for [`Saver::begin`]).
	#[must_use]
	pub const fn get_version(&self) -> u32 {
		self.version
	}

	/// Whether the section has a value `key`.
	#[must_use]
	pub fn has(&self, key: &str) -> bool {
		self.values.contains_key(key)
	}

	/// The names of the values.
	#[must_use]
	pub fn keys(&self) -> Vec<&str> {
		self.values.keys().map(String::as_str).collect()
	}

	fn raw(&self, key: &str) -> ZResult<&Value> {
		self.values
			.get(key)
			.ok_or_else(|| ZenithError::SaveKeyNotFound(format!("{}.{key}", self.name)))
	}

	fn wrong(&self, key: &str, want: &str, found: &Value) -> ZenithError {
		ZenithError::SaveWrongType(format!("{}.{key} is {}, not {want}", self.name, found.type_str()))
	}

	fn integer<T: TryFrom<i64>>(&self, key: &str, want: &str) -> ZResult<T> {
		match self.raw(key)? {
			Value::Integer(n) => T::try_from(*n).map_err(|_| {
				ZenithError::SaveWrongType(format!("{}.{key} is {n}, which does not fit {want}", self.name))
			}),
			other => Err(self.wrong(key, want, other)),
		}
	}

	fn float(&self, key: &str, want: &str) -> ZResult<f64> {
		match self.raw(key)? {
			Value::Float(f) => Ok(*f),
			// `speed = 1` in a save edited by hand is a number all the same.
			#[allow(clippy::cast_precision_loss)] // only integers above 2^53 lose anything
			Value::Integer(n) => Ok(*n as f64),
			other => Err(self.wrong(key, want, other)),
		}
	}

	#[allow(clippy::cast_possible_truncation)] // the nearest f32 is what was asked for
	fn single(&self, key: &str, want: &str) -> ZResult<f32> {
		self.float(key, want).map(|f| f as f32)
	}

	fn boolean(&self, key: &str, want: &str) -> ZResult<bool> {
		match self.raw(key)? {
			Value::Boolean(b) => Ok(*b),
			other => Err(self.wrong(key, want, other)),
		}
	}

	getters! {
		load_bool, load_bool_or, bool, boolean, "bool";
		load_i32, load_i32_or, i32, integer, "i32";
		load_u32, load_u32_or, u32, integer, "u32";
		load_i64, load_i64_or, i64, integer, "i64";
		load_u64, load_u64_or, u64, integer, "u64";
		load_f32, load_f32_or, f32, single, "f32";
		load_f64, load_f64_or, f64, float, "f64";
	}

	/// The text saved as `key`.
	///
	/// # Errors
	/// `SaveKeyNotFound`, or `SaveWrongType` if it is not text.
	pub fn load_string(&self, key: &str) -> ZResult<String> {
		match self.raw(key)? {
			Value::String(s) => Ok(s.clone()),
			other => Err(self.wrong(key, "text", other)),
		}
	}

	/// The text saved as `key`, or `default` if there is no such value.
	///
	/// # Errors
	/// `SaveWrongType` if the value is there but is not text.
	pub fn load_string_or(&self, key: &str, default: &str) -> ZResult<String> {
		if self.has(key) {
			self.load_string(key)
		} else {
			Ok(default.to_string())
		}
	}

	/// The bytes saved as `key`.
	///
	/// # Errors
	/// `SaveKeyNotFound`, or `SaveWrongType` if it is not bytes
	/// ([`Saver::save_bytes`]).
	pub fn load_bytes(&self, key: &str) -> ZResult<Vec<u8>> {
		let Value::String(text) = self.raw(key)? else {
			return Err(self.wrong(key, "bytes", self.raw(key)?));
		};
		base64::decode(text)
			.ok_or_else(|| ZenithError::SaveWrongType(format!("{}.{key} is text, which is not bytes", self.name)))
	}

	/// The bytes saved as `key`, or `default` if there is no such value.
	///
	/// # Errors
	/// `SaveWrongType` if the value is there but is not bytes.
	pub fn load_bytes_or(&self, key: &str, default: &[u8]) -> ZResult<Vec<u8>> {
		if self.has(key) {
			self.load_bytes(key)
		} else {
			Ok(default.to_vec())
		}
	}

	/// Anything `serde` can read, saved as `key` ([`Saver::save`]).
	///
	/// # Errors
	/// `SaveKeyNotFound`, or `SaveWrongType` if the value is not of this shape.
	pub fn load<T: DeserializeOwned>(&self, key: &str) -> ZResult<T> {
		self.raw(key)?
			.clone()
			.try_into()
			.map_err(|e| ZenithError::SaveWrongType(format!("{}.{key}: {e}", self.name)))
	}

	/// The whole section as one value (a struct or a map), the other half of
	/// [`Saver::save_section`].
	///
	/// # Errors
	/// `SaveWrongType` if the values do not fit the shape.
	pub fn load_all<T: DeserializeOwned>(&self) -> ZResult<T> {
		Value::Table(self.values.clone())
			.try_into()
			.map_err(|e| ZenithError::SaveWrongType(format!("{}: {e}", self.name)))
	}
}

#[cfg(test)]
mod tests;
