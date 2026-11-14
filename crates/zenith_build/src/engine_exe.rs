//! Copying the engine's executable next to a build.

use std::{
	ffi::OsString,
	path::{Path, PathBuf},
};

use zenith_archive::EncryptionKey;
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::layout::TMP_SUFFIX;

use crate::output::resolve_for_compare;

/// What the copy of the engine executable is called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameExe {
	/// The name of the project (from its manifest).
	ProjectName,
	/// This name, without an extension: the extension of the original is kept
	/// (`Game` becomes `Game.exe` for `zenith_engine.exe`).
	Name(String),
}

/// An engine executable to copy and what to call the copy.
#[derive(Clone, Debug)]
pub struct EngineExe {
	pub source: PathBuf,
	pub rename: Option<RenameExe>,
}

const fn bad(message: String) -> ZenithError {
	ZenithError::InvalidBuildOption(message)
}

/// A file name that is safe on every platform.
fn check_name(name: &str) -> ZResult<()> {
	let hostile = name.is_empty()
		|| name.ends_with(['.', ' '])
		|| name
			.chars()
			.any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\u{5c}' | '|' | '?' | '*'));
	if hostile {
		return Err(bad(format!("{name:?} cannot be the name of the engine executable")));
	}
	Ok(())
}

impl EngineExe {
	/// The file the copy will be, in `folder`. Checks the source and the name,
	/// so a bad option is reported before anything is built.
	pub fn destination(&self, folder: &Path, project_name: &str) -> ZResult<PathBuf> {
		let meta = std::fs::metadata(&self.source)
			.map_err(|e| bad(format!("the engine executable {}: {e}", self.source.display())))?;
		if !meta.is_file() {
			return Err(bad(format!(
				"the engine executable {} is not a file",
				self.source.display()
			)));
		}
		let original = self
			.source
			.file_name()
			.ok_or_else(|| bad(format!("{} has no file name", self.source.display())))?;
		let file_name = match &self.rename {
			None => original.to_os_string(),
			Some(rename) => {
				let stem = match rename {
					RenameExe::ProjectName => project_name,
					RenameExe::Name(name) => name,
				};
				check_name(stem)?;
				let mut name = OsString::from(stem);
				if let Some(ext) = Path::new(original).extension() {
					name.push(".");
					name.push(ext);
				}
				name
			}
		};
		let destination = folder.join(file_name);
		if resolve_for_compare(&destination)? == resolve_for_compare(&self.source)? {
			return Err(bad(format!(
				"the engine executable {} would be copied onto itself",
				self.source.display()
			)));
		}
		Ok(destination)
	}
}

/// Copies `source` to `destination` through a temporary file, so that a failure
/// never leaves half an executable under the final name. Permissions (the
/// executable bit) are copied with the content.
pub fn copy(source: &Path, destination: &Path) -> ZResult<()> {
	copy_then(source, destination, |_| Ok(()))
}

/// Copies a program of the engine (its executable, its `.wasm`) like
/// [`copy`], and writes `key` into the key slot of the copy, so the program
/// opens the build it is given with. `None` leaves the slot of the copy empty.
///
/// # Errors
/// `InvalidBuildOption` if there is a key and the program has no slot (it
/// was made by an engine older than the slot), and the errors of the copy.
pub fn copy_with_key(source: &Path, destination: &Path, key: Option<&EncryptionKey>) -> ZResult<()> {
	copy_then(source, destination, |temp| {
		let mut bytes = std::fs::read(temp)?;
		let found = zenith_archive::key_slot::write_key(&mut bytes, key)?;
		if found {
			std::fs::write(temp, &bytes)?;
		} else if key.is_some() {
			return Err(no_slot(source));
		}
		Ok(())
	})
}

/// Checks, before anything is built, that the program can be given the key
/// of an encrypted build.
///
/// # Errors
/// `InvalidBuildOption` if it cannot be read or has no slot for the key.
pub fn check_key_slot(program: &Path) -> ZResult<()> {
	let bytes = std::fs::read(program).map_err(|e| bad(format!("{}: {e}", program.display())))?;
	if zenith_archive::key_slot::has_slot(&bytes)? {
		Ok(())
	} else {
		Err(no_slot(program))
	}
}

fn no_slot(program: &Path) -> ZenithError {
	bad(format!(
		"{} has no place for the key of an encrypted build: it was made by an older engine",
		program.display()
	))
}

/// Copies `source` to a temporary file, lets `change` work on it, and gives it
/// its final name.
fn copy_then(source: &Path, destination: &Path, change: impl FnOnce(&Path) -> ZResult<()>) -> ZResult<()> {
	let mut temp = destination.as_os_str().to_os_string();
	temp.push(TMP_SUFFIX);
	let temp = PathBuf::from(temp);
	let result = std::fs::copy(source, &temp)
		.map_err(ZenithError::from)
		.and_then(|_| change(&temp))
		.and_then(|()| Ok(std::fs::rename(&temp, destination)?));
	if result.is_err() {
		// The error that matters is the first one.
		let _ = std::fs::remove_file(&temp);
	}
	result
}
