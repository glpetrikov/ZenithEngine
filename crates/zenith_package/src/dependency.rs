use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Dependency {
	pub name: String,
	pub version: semver::VersionReq,
	#[serde(flatten)]
	pub source: DependencySource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields, rename_all = "PascalCase")]
pub enum DependencySource {
	Git {
		git: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		branch: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		tag: Option<String>,
		#[serde(skip_serializing_if = "Option::is_none")]
		rev: Option<String>,
	},
	Path {
		path: PathBuf,
	},
}
