pub mod category;
mod dependency;

use std::{collections::HashMap, path::PathBuf, str::FromStr};

use dependency::Dependency;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::category::Category;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Author {
	pub name: String,
	pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PackageManifest {
	// Base
	pub name: String,             // "my_cool_package"
	pub display_name: String,     // "My Cool Package"
	pub version: semver::Version, // "0.0.1"
	pub description: String,      // "A description of my really cool package"
	pub dependencies: Vec<Dependency>,
	pub minimal_engine_version: semver::VersionReq, // "^0.1.0"
	#[serde(serialize_with = "serialize_license", deserialize_with = "deserialize_license")]
	pub license: spdx::Expression, // "MIT", "Apache-2.0", "GPL-3.0",

	// Provides
	#[serde(default)]
	pub themes: HashMap<String, PathBuf>, // ID + Path
	#[serde(default)]
	pub custom_colors: HashMap<String, String>, // ID + Hex

	// Additional
	#[serde(default)]
	pub repository: Option<String>, // "https://github.com/imcool/my_cool_package"
	#[serde(default)]
	pub categories: Vec<Category>, // ["Theme", "ImDontKnowCategoryOfThisThing"]
	#[serde(default)]
	pub authors: Vec<Author>, // [[Authors]] Name = "ImCool" Email = "imabsolutelycool@example.com"

	// Directories
	pub editor_cargo_toml: Option<PathBuf>, // "editor" with Cargo.toml and src.
	pub api_cargo_toml: Option<PathBuf>,    // "api" with Cargo.toml and src.
	pub wit: Option<Vec<PathBuf>>,          // "wit" folder(or custom folder) with .wit files for API bindings.
}

/// Serializes a `spdx::Expression` into a string.
///
/// # Errors
/// Returns a [`serde::ser::Error`] if the serializer fails to encode the
/// license string.
pub fn serialize_license<S: Serializer>(license: &spdx::Expression, serializer: S) -> Result<S::Ok, S::Error> {
	serializer.serialize_str(license.as_ref())
}

/// Deserializes a `spdx::Expression` from a string.
///
/// # Errors
/// Returns a [`serde::de::Error`] if the string is not a valid SPDX 2.1
/// expression.
pub fn deserialize_license<'de, D: Deserializer<'de>>(deserializer: D) -> Result<spdx::Expression, D::Error> {
	let raw = String::deserialize(deserializer)?;
	spdx::Expression::from_str(&raw).map_err(serde::de::Error::custom)
}
