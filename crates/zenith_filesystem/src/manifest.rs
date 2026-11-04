//! `<Name>.zenithproject`: the project manifest.
//!
//! ```toml
//! [project]
//! name = "Test"
//! project_version = "0.1.0"
//! engine_version = "^0.1.0"
//! version = 1          # manifest format, unrelated to the `.v1` folder flag
//! user_data = "Studio/Test"   # optional, see below
//! ```
//!
//! `user_data` is the folder of the data of the player (`Data://`) below the
//! data folder of the system, a name or a path of names. Without it the folder
//! is called like the project. Choose it once and keep it: the saves of the
//! players stay where they are when the game is renamed, and several games of
//! one studio can share a folder name above their own.

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use zenith_archive::is_windows_reserved_name;
use zenith_error::{ZResult, ZenithError};

/// The only manifest format version this crate reads and writes.
pub const MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
	pub name: String,
	pub project_version: Version,
	pub engine_version: VersionReq,
	pub version: u32,
	/// The folder of the data of the player, below the data folder of the
	/// system (`Studio/Game`); the project name when absent.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub user_data: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct File {
	project: Manifest,
}

/// A project name is also a folder and file name, so it must be a plain,
/// portable name.
///
/// # Errors
/// `BadProjectName` if it is empty, too long, has a separator, a colon or a
/// control character, starts with a dot, ends with a dot or space, or is a
/// Windows device name (`CON`, `NUL`, `COM1`, ...).
pub fn validate_project_name(name: &str) -> ZResult<()> {
	let bad = name.is_empty()
		|| name.len() > 128
		|| name.starts_with('.')
		|| name.ends_with(['.', ' '])
		|| is_windows_reserved_name(name)
		|| name
			.chars()
			.any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control());
	if bad {
		Err(ZenithError::BadProjectName(name.to_string()))
	} else {
		Ok(())
	}
}

/// The folder of the data of the player must be a plain, portable path of
/// names: no `..`, nothing a platform cannot name.
///
/// # Errors
/// `InvalidSettings` describing the problem.
pub fn validate_user_data_folder(folder: &str) -> ZResult<()> {
	zenith_archive::validate_entry_path(folder)
		.map_err(|e| ZenithError::InvalidSettings(format!("user_data {folder:?} in the manifest: {e}")))
}

impl Manifest {
	/// The folder of the data of the player: `user_data` of the manifest, the
	/// project name without it.
	#[must_use]
	pub fn user_data_folder(&self) -> &str {
		self.user_data.as_deref().unwrap_or(&self.name)
	}

	/// A manifest for a new project: version 0.1.0 for engines `^<this
	/// version>`.
	///
	/// # Errors
	/// `BadProjectName` or `Version`.
	pub fn new(name: &str) -> ZResult<Self> {
		validate_project_name(name)?;
		Ok(Self {
			name: name.to_string(),
			project_version: Version::new(0, 1, 0),
			engine_version: VersionReq::parse(&format!("^{}", env!("CARGO_PKG_VERSION")))
				.map_err(|e| ZenithError::Version(e.to_string()))?,
			version: MANIFEST_VERSION,
			user_data: None,
		})
	}

	/// Parses manifest text.
	///
	/// # Errors
	/// `Toml` for malformed text, `UnsupportedProjectVersion` for another
	/// manifest format version, `BadProjectName` for an unusable name,
	/// `InvalidSettings` for an unusable `user_data`.
	pub fn parse(text: &str) -> ZResult<Self> {
		let file: File = toml::from_str(text).map_err(|e| ZenithError::Toml(e.to_string()))?;
		if file.project.version != MANIFEST_VERSION {
			return Err(ZenithError::UnsupportedProjectVersion);
		}
		validate_project_name(&file.project.name)?;
		if let Some(folder) = &file.project.user_data {
			validate_user_data_folder(folder)?;
		}
		Ok(file.project)
	}

	/// Serializes the manifest.
	///
	/// # Errors
	/// `Toml` if serialization fails.
	pub fn to_toml(&self) -> ZResult<String> {
		toml::to_string(&File { project: self.clone() }).map_err(|e| ZenithError::Toml(e.to_string()))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_the_sandbox_manifest() -> ZResult<()> {
		let m = Manifest::parse(
			"[project]\nname = \"Test\"\nproject_version = \"0.1.0\"\nengine_version = \"^0.1.0\"\nversion = 1\n",
		)?;
		assert_eq!(m.name, "Test", "name");
		assert_eq!(m.project_version, Version::new(0, 1, 0), "project version");
		assert!(m.engine_version.matches(&Version::new(0, 1, 5)), "engine requirement");
		assert_eq!(Manifest::parse(&m.to_toml()?)?, m, "serialization round trip");
		Ok(())
	}

	#[test]
	fn the_folder_of_the_data_of_the_player_defaults_to_the_name() -> ZResult<()> {
		let plain = Manifest::new("Game")?;
		assert_eq!(plain.user_data_folder(), "Game", "the project name");
		assert!(
			!plain.to_toml()?.contains("user_data"),
			"nothing is written for the default"
		);

		let mut chosen = Manifest::new("Game")?;
		chosen.user_data = Some("Studio/Zenith Game".to_string());
		assert_eq!(chosen.user_data_folder(), "Studio/Zenith Game", "the chosen one");
		assert_eq!(Manifest::parse(&chosen.to_toml()?)?, chosen, "it survives a round trip");

		let text = |folder: &str| {
			format!(
				"[project]\nname = \"Game\"\nproject_version = \"0.1.0\"\nengine_version = \"^0.1.0\"\nversion = 1\nuser_data = \"{folder}\"\n"
			)
		};
		for bad in ["", "..", "a/../b", "/abs", "a//b", "C:/x", "a\\b", "CON", "x."] {
			assert!(
				matches!(Manifest::parse(&text(bad)), Err(ZenithError::InvalidSettings(_))),
				"{bad:?} must be refused"
			);
		}
		Ok(())
	}

	#[test]
	fn rejects_bad_input() {
		assert!(
			matches!(
				Manifest::parse(
					"[project]\nname = \"T\"\nproject_version = \"0.1.0\"\nengine_version = \"^0.1.0\"\nversion = 2\n"
				),
				Err(ZenithError::UnsupportedProjectVersion)
			),
			"unknown manifest version"
		);
		assert!(Manifest::parse("[project]\nname = \"T\"\n").is_err(), "missing fields");
		assert!(Manifest::parse("not toml [").is_err(), "garbage");
		for name in [
			"", ".hidden", "a/b", "a\\b", "a:b", "x.", "x ", "CON?", "con", "NUL.Game", "Com1",
		] {
			assert!(validate_project_name(name).is_err(), "{name:?} must be rejected");
		}
		assert!(validate_project_name("My Game 2").is_ok(), "spaces inside are fine");
	}
}
