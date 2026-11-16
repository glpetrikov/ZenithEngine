//! What the editor keeps for a project: `Settings://Editor.toml`, part of the
//! project (unlike `recent.rs`, which is the user's).
//!
//! ```toml
//! [assets]
//! # What happens to the .obj, .gltf and .glb files put into Assets/ by hand
//! # when the project opens: "ask", "always" (imported) or "never".
//! loose_models = "ask"
//! ```

use serde::{Deserialize, Serialize};
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{Project, ZPath};

const FILE: &str = "Settings://Editor.toml";

/// What happens to the model files put into `Assets/` by hand when the
/// project opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LooseModels {
	/// The user is asked which to import.
	#[default]
	Ask,
	/// They are imported, and their files go to the trash.
	Always,
	/// Nothing happens; File → Import Loose Models still does it.
	Never,
}

#[derive(Default, Deserialize)]
struct File {
	#[serde(default)]
	assets: Assets,
}

#[derive(Default, Deserialize)]
struct Assets {
	#[serde(default)]
	loose_models: LooseModels,
}

/// The setting of the project. A file that is not there is the default; one
/// that cannot be read is too, with a warning.
pub fn loose_models(project: &Project) -> LooseModels {
	let read = || -> ZResult<Option<LooseModels>> {
		let path = ZPath::new(FILE)?;
		if !project.exists(&path)? {
			return Ok(None);
		}
		let text = String::from_utf8_lossy(&project.read_bytes(&path)?).into_owned();
		let file: File = toml::from_str(&text).map_err(|e| ZenithError::Other(e.to_string()))?;
		Ok(Some(file.assets.loose_models))
	};
	read()
		.unwrap_or_else(|e| {
			zenith_log::warn!("{FILE} cannot be read, the defaults are used: {e}");
			None
		})
		.unwrap_or_default()
}

/// Writes the setting, keeping the other settings of the file (its comments
/// are lost: the file is written anew).
///
/// # Errors
/// An error of the project, or a file that is not TOML (it is not replaced).
pub fn set_loose_models(project: &Project, value: LooseModels) -> ZResult<()> {
	let path = ZPath::new(FILE)?;
	let mut table: toml::Table = if project.exists(&path)? {
		toml::from_str(&String::from_utf8_lossy(&project.read_bytes(&path)?))
			.map_err(|e| ZenithError::Other(format!("{FILE}: {e}")))?
	} else {
		toml::Table::new()
	};
	let assets = table
		.entry("assets")
		.or_insert_with(|| toml::Value::Table(toml::Table::new()));
	let toml::Value::Table(assets) = assets else {
		return Err(ZenithError::Other(format!("{FILE}: `assets` is not a table")));
	};
	assets.insert(
		"loose_models".into(),
		toml::Value::try_from(value).map_err(|e| ZenithError::Other(e.to_string()))?,
	);
	let text = toml::to_string_pretty(&table).map_err(|e| ZenithError::Other(e.to_string()))?;
	project.write(&path, text.as_bytes())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_setting_is_ask_until_it_is_set_and_the_rest_of_the_file_stays() -> ZResult<()> {
		let tmp = tempfile::tempdir()?;
		let project = Project::create(tmp.path().join("Game"))?;
		assert_eq!(loose_models(&project), LooseModels::Ask, "no file");

		let path = ZPath::new(FILE)?;
		project.write(&path, b"[window]\nwidth = 3\n")?;
		set_loose_models(&project, LooseModels::Never)?;
		assert_eq!(loose_models(&project), LooseModels::Never);
		let text = String::from_utf8_lossy(&project.read_bytes(&path)?).into_owned();
		assert!(text.contains("width = 3"), "kept: {text}");

		project.write(&path, b"not toml [")?;
		assert_eq!(loose_models(&project), LooseModels::Ask, "a broken file is the default");
		assert!(
			set_loose_models(&project, LooseModels::Always).is_err(),
			"and it is not replaced"
		);
		Ok(())
	}
}
