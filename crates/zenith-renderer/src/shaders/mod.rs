//! The shaders of the engine, and where a pass finds a shader by name.
//!
//! A pass asks for a shader by name (`Zenith/standard`) and does not care
//! where its text comes from. [`ShaderSources`] gives the engine's own copies
//! until the shaders live in the project (stage 10 of the plan), which then
//! gives the project's copy first.

use std::collections::HashMap;

/// The standard material.
pub const STANDARD: &str = "Zenith/standard";
/// The tone mapping of an HDR picture.
pub const TONEMAP: &str = "Zenith/tonemap";

/// The engine's copies, by name.
const ENGINE: [(&str, &str); 2] = [
	(STANDARD, include_str!("standard.wgsl")),
	(TONEMAP, include_str!("tonemap.wgsl")),
];

/// The text of shaders, by name.
#[derive(Clone, Debug)]
pub struct ShaderSources {
	sources: HashMap<String, String>,
}

impl Default for ShaderSources {
	fn default() -> Self {
		Self {
			sources: ENGINE
				.iter()
				.map(|&(name, source)| (name.to_owned(), source.to_owned()))
				.collect(),
		}
	}
}

impl ShaderSources {
	/// The text of the shader `name`.
	pub fn get(&self, name: &str) -> Option<&str> {
		self.sources.get(name).map(String::as_str)
	}

	/// Puts the text of `name` in (a shader of the project, a change on disk).
	pub fn set(&mut self, name: impl Into<String>, source: impl Into<String>) {
		self.sources.insert(name.into(), source.into());
	}

	/// The names there are.
	pub fn names(&self) -> impl Iterator<Item = &str> {
		self.sources.keys().map(String::as_str)
	}
}
