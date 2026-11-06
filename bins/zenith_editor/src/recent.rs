//! What the editor remembers between runs: the project it had open.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zenith_error::{ZResult, ZenithError};

/// Where it is kept: `Editor.toml` in the folder of the engine in the data
/// folder of the user. `None` if the system has no such folder.
#[must_use]
pub fn default_file() -> Option<PathBuf> {
	dirs::data_local_dir().map(|dir| dir.join("ZenithEngine").join("Editor.toml"))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recent {
	/// The project that was open when the editor was closed.
	#[serde(default)]
	pub last_project: Option<PathBuf>,
}

impl Recent {
	/// Reads the file. One that is missing, or cannot be understood, gives an
	/// empty state (the editor starts without a project), the second with a
	/// line in the log.
	#[must_use]
	pub fn load(file: &Path) -> Self {
		let text = match std::fs::read_to_string(file) {
			Ok(text) => text,
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
			Err(e) => {
				zenith_log::warn!("Cannot read {}: {e}", file.display());
				return Self::default();
			}
		};
		toml::from_str(&text).unwrap_or_else(|e| {
			zenith_log::warn!("Cannot understand {}: {e}", file.display());
			Self::default()
		})
	}

	/// Writes the file (and its folder), replacing it whole.
	///
	/// # Errors
	/// `Toml` if the state cannot be written, or the error of writing the file.
	pub fn save(&self, file: &Path) -> ZResult<()> {
		let text = toml::to_string(self).map_err(|e| ZenithError::Toml(e.to_string()))?;
		if let Some(folder) = file.parent() {
			std::fs::create_dir_all(folder)?;
		}
		let temp = file.with_extension("toml.zenith-tmp");
		std::fs::write(&temp, text)?;
		std::fs::rename(&temp, file)?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	type TestResult = Result<(), Box<dyn std::error::Error>>;

	#[test]
	fn the_last_project_comes_back() -> TestResult {
		let tmp = tempfile::tempdir()?;
		let file = tmp.path().join("deep").join("Editor.toml");
		assert_eq!(Recent::load(&file), Recent::default(), "no file, nothing remembered");
		let recent = Recent {
			last_project: Some(tmp.path().join("My Game")),
		};
		recent.save(&file)?;
		assert_eq!(Recent::load(&file), recent);
		assert!(
			!file.with_extension("toml.zenith-tmp").exists(),
			"no temporary file is left"
		);
		Ok(())
	}

	#[test]
	fn a_damaged_file_means_nothing_remembered() -> TestResult {
		let tmp = tempfile::tempdir()?;
		let file = tmp.path().join("Editor.toml");
		std::fs::write(&file, "last_project = [not toml")?;
		assert_eq!(Recent::load(&file), Recent::default());
		std::fs::write(&file, "something_else = 1\n")?;
		assert_eq!(Recent::load(&file), Recent::default(), "unknown fields do not matter");
		Ok(())
	}

	#[test]
	fn the_default_file_is_below_the_data_folder_and_not_a_fixed_place() {
		if let (Some(file), Some(data)) = (default_file(), dirs::data_local_dir()) {
			assert!(file.starts_with(data));
			assert!(file.ends_with("ZenithEngine/Editor.toml"));
		}
	}
}
