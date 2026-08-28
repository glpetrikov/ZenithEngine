use serde::{Deserialize, Serialize};

use crate::ZENITH_VERSION;

#[derive(Serialize, Deserialize, Default)]
pub struct ZenithProject {
	pub project: Project, // for the [project] in the project's toml file
}

#[derive(Serialize, Deserialize)]
pub struct Project {
	pub name: String,
	pub description: Option<String>,
	pub project_version: semver::Version,
	pub engine_version: semver::VersionReq,
	pub version: u32,
}
impl Default for Project {
	fn default() -> Self {
		Self {
			name: "Default".to_string(),
			description: None,
			project_version: semver::Version::new(0, 1, 0),
			engine_version: semver::VersionReq::parse(ZENITH_VERSION).unwrap_or_default(),
			version: 1,
		}
	}
}
