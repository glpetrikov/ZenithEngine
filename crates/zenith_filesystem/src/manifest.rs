//! `<Name>.zenithproject`: the project manifest.
//!
//! ```toml
//! [project]
//! name = "Test"
//! project_version = "0.1.0"
//! engine_version = "^0.1.0"
//! version = 1          # manifest format, unrelated to the `.v1` folder flag
//! ```

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

impl Manifest {
	/// A manifest for a new project: version 0.1.0 for engines `^<this
	/// version>`.
	///
	/// # Errors
	/// `BadProjectName` or a semver error.
	pub fn new(name: &str) -> ZResult<Self> {
		validate_project_name(name)?;
		Ok(Self {
			name: name.to_string(),
			project_version: Version::new(0, 1, 0),
			engine_version: VersionReq::parse(&format!("^{}", env!("CARGO_PKG_VERSION")))?,
			version: MANIFEST_VERSION,
		})
	}

	/// Parses manifest text.
	///
	/// # Errors
	/// `TomlDe` for malformed text, `UnsupportedProjectVersion` for another
	/// manifest format version, `BadProjectName` for an unusable name.
	pub fn parse(text: &str) -> ZResult<Self> {
		let file: File = toml::from_str(text)?;
		if file.project.version != MANIFEST_VERSION {
			return Err(ZenithError::UnsupportedProjectVersion);
		}
		validate_project_name(&file.project.name)?;
		Ok(file.project)
	}

	/// Serializes the manifest.
	///
	/// # Errors
	/// `TomlSer` if serialization fails.
	pub fn to_toml(&self) -> ZResult<String> {
		Ok(toml::to_string(&File { project: self.clone() })?)
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
