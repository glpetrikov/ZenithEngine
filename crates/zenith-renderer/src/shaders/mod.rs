//! The shaders of the renderer: WESL, found by name.
//!
//! A pass asks for a shader by name (`Zenith/standard`) and does not care
//! where its text comes from. The engine keeps a copy of its own shaders
//! ([`ENGINE`]) to write them into a project ([`project`]) and to stand in for
//! one that is missing or broken. [`ShaderSources`] holds the text the cache
//! compiles from: the engine's copies, then what the project has over them.
//! [`compile`](compile::compile) turns WESL into checked WGSL.

pub mod compile;
pub mod project;

use std::collections::BTreeMap;

/// The standard material.
pub const STANDARD: &str = "Zenith/standard";
/// The tone mapping of an HDR picture.
pub const TONEMAP: &str = "Zenith/tonemap";
/// The depth of the meshes seen from a light.
pub const SHADOW: &str = "Zenith/shadow";
/// The sky.
pub const SKY: &str = "Zenith/sky";
/// Sprites.
pub const SPRITE: &str = "Zenith/sprite";
/// Lines (the grid, the gizmos and the outline of the editor).
pub const LINES: &str = "Zenith/lines";
/// The structs the shaders of the passes share.
pub const COMMON: &str = "Zenith/common";
/// What stands in for a shader of a material that is missing or broken.
pub const MAGENTA: &str = "Zenith/magenta";

/// The engine's copies, by name.
pub const ENGINE: [(&str, &str); 8] = [
	(COMMON, include_str!("common.wesl")),
	(STANDARD, include_str!("standard.wesl")),
	(TONEMAP, include_str!("tonemap.wesl")),
	(SHADOW, include_str!("shadow.wesl")),
	(SKY, include_str!("sky.wesl")),
	(SPRITE, include_str!("sprite.wesl")),
	(LINES, include_str!("lines.wesl")),
	(MAGENTA, include_str!("magenta.wesl")),
];

/// The first line of a copy of an engine shader reads
/// `// zenith-shader: <name> <version>`.
const HEADER: &str = "// zenith-shader:";

/// The version a shader's first line names, if it names one.
pub fn version_of(source: &str) -> Option<u32> {
	let line = source.lines().next()?.strip_prefix(HEADER)?;
	line.split_whitespace().nth(1)?.parse().ok()
}

/// Whether `name` is a shader of the engine.
pub fn is_engine(name: &str) -> bool {
	ENGINE.iter().any(|&(engine, _)| engine == name)
}

/// The text of shaders, by name. A change of it counts ([`Self::version`]),
/// so whoever compiled from it knows to do it again.
#[derive(Clone, Debug)]
pub struct ShaderSources {
	sources: BTreeMap<String, String>,
	version: u64,
}

impl Default for ShaderSources {
	/// The engine's copies.
	fn default() -> Self {
		Self::engine()
	}
}

impl ShaderSources {
	/// The engine's own copies, and nothing else.
	pub fn engine() -> Self {
		Self {
			sources: ENGINE
				.iter()
				.map(|&(name, source)| (name.to_owned(), source.to_owned()))
				.collect(),
			version: 0,
		}
	}

	/// The text of the shader `name`.
	pub fn get(&self, name: &str) -> Option<&str> {
		self.sources.get(name).map(String::as_str)
	}

	/// Puts the text of `name` in (a shader of the project, a change on disk).
	pub fn set(&mut self, name: impl Into<String>, source: impl Into<String>) {
		self.sources.insert(name.into(), source.into());
		self.version += 1;
	}

	/// Takes `name` out. Returns whether it was there.
	pub fn remove(&mut self, name: &str) -> bool {
		let removed = self.sources.remove(name).is_some();
		if removed {
			self.version += 1;
		}
		removed
	}

	/// The names there are, in order.
	pub fn names(&self) -> impl Iterator<Item = &str> {
		self.sources.keys().map(String::as_str)
	}

	/// The names with their text.
	pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
		self.sources
			.iter()
			.map(|(name, source)| (name.as_str(), source.as_str()))
	}

	/// Goes up with every change.
	pub const fn version(&self) -> u64 {
		self.version
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_copy_of_the_engine_names_itself_and_a_version() {
		for (name, source) in ENGINE {
			assert_eq!(
				source.lines().next(),
				Some(format!("{HEADER} {name} 1").as_str()),
				"the first line of {name}"
			);
			assert_eq!(version_of(source), Some(1));
		}
	}

	#[test]
	fn a_change_of_the_sources_is_counted() {
		let mut sources = ShaderSources::engine();
		let before = sources.version();
		sources.set("Mine/toon", "// text");
		assert!(sources.version() > before);
		assert!(!sources.remove("nothing"));
		assert!(sources.remove("Mine/toon"));
	}

	#[test]
	fn a_version_line_is_read_or_there_is_none() {
		assert_eq!(version_of("// zenith-shader: A/b 7\nfn x() {}"), Some(7));
		assert_eq!(version_of("fn x() {}"), None);
		assert_eq!(version_of("// zenith-shader: A/b\n"), None);
	}
}
