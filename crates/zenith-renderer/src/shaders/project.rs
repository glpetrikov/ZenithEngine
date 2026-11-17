//! The shaders as files of a project: `Shaders://` (`Assets/Shaders`).
//!
//! A new or an older project gets the engine's copies written
//! ([`install_engine_shaders`]); the renderer reads what the project has
//! ([`load_project_shaders`]). A file is a shader by its path in `Shaders://`
//! without the extension (`Zenith/standard`, `Effects/toon`), the same path
//! WESL imports it by. `.wesl` is the extension; a `.wgsl` file is read too
//! (plain WGSL is valid WESL), and `.wesl` wins when both are there.

use zenith_error::ZResult;
use zenith_filesystem::{EntryKind, Project, ZPath};

use super::{ENGINE, ShaderSources, is_engine, version_of};

/// The root of the shaders of a project.
const ROOT: &str = "Shaders://";
/// What a shader file ends with, the one that wins first.
const EXTENSIONS: [&str; 2] = ["wesl", "wgsl"];

/// What [`install_engine_shaders`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstallReport {
	/// The shaders that were not in the project and now are.
	pub written: Vec<String>,
	/// The shaders the project has an older copy of than the engine's. They
	/// are not touched: the project's copy may have been changed on purpose.
	pub newer_in_engine: Vec<String>,
}

fn path_of(name: &str) -> ZResult<ZPath> {
	ZPath::new(&format!("{ROOT}{name}.wesl"))
}

/// Writes the engine's shaders that the project lacks.
///
/// They go into `Shaders://Zenith/`. A copy that is there is left alone; one
/// that is older than the engine's is named in the report, for the editor to
/// offer a replacement.
///
/// # Errors
/// The project cannot be written (a build is read-only).
pub fn install_engine_shaders(project: &Project) -> ZResult<InstallReport> {
	let mut report = InstallReport::default();
	for (name, source) in ENGINE {
		let path = path_of(name)?;
		if project.exists(&path)? {
			let has = version_of(&project.read_string(&path)?);
			if has.is_none_or(|has| version_of(source).is_some_and(|engine| engine > has)) {
				report.newer_in_engine.push(name.to_owned());
			}
		} else {
			project.write(&path, source.as_bytes())?;
			report.written.push(name.to_owned());
		}
	}
	Ok(report)
}

/// Writes all the shaders of the engine again, over what is there ("restore
/// the engine's shaders").
///
/// # Errors
/// The project cannot be written.
pub fn restore_engine_shaders(project: &Project) -> ZResult<()> {
	for (name, source) in ENGINE {
		project.write(&path_of(name)?, source.as_bytes())?;
	}
	Ok(())
}

/// What [`load_project_shaders`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadReport {
	/// The shaders read from the project.
	pub loaded: Vec<String>,
	/// Shaders of the engine the project does not have: the engine's copy
	/// stands in for them.
	pub missing: Vec<String>,
	/// Files that could not be read, with why.
	pub unreadable: Vec<(String, String)>,
}

/// Reads the shaders of the project into `sources`, over the engine's copies
/// (a copy the project does not have stays).
///
/// A shader that was in `sources` from an earlier load and is not in the
/// project any more is taken out (the engine's copy comes back for an engine
/// shader).
///
/// # Errors
/// `Shaders://` cannot be listed for another reason than not being there.
pub fn load_project_shaders(project: &Project, sources: &mut ShaderSources) -> ZResult<LoadReport> {
	let mut files = Vec::new();
	if project.exists(&ZPath::new(ROOT)?)? {
		collect(project, ROOT, "", &mut files)?;
	}
	// `.wesl` first, so it wins over a `.wgsl` of the same name.
	files.sort_by_key(|(name, extension, _)| (name.clone(), EXTENSIONS.iter().position(|e| e == extension)));
	files.dedup_by(|later, first| later.0 == first.0);

	let mut report = LoadReport::default();
	let before: Vec<String> = sources.names().map(str::to_owned).collect();
	let mut fresh = ShaderSources::engine();
	for (name, _, path) in files {
		match project.read_string(&path) {
			Ok(text) => {
				fresh.set(name.clone(), text);
				report.loaded.push(name);
			}
			Err(e) => report.unreadable.push((name, e.to_string())),
		}
	}
	for (name, _) in ENGINE {
		if !report.loaded.iter().any(|loaded| loaded == name) {
			report.missing.push(name.to_owned());
		}
	}
	// Into `sources` only what differs, so a reload that changed nothing does
	// not make the cache compile everything again.
	for name in &before {
		if fresh.get(name).is_none() {
			sources.remove(name);
		}
	}
	for (name, text) in fresh.iter() {
		if sources.get(name) != Some(text) {
			sources.set(name, text);
		}
	}
	Ok(report)
}

/// `(name, extension, path)` of every shader file under `dir`.
fn collect(project: &Project, dir: &str, prefix: &str, out: &mut Vec<(String, String, ZPath)>) -> ZResult<()> {
	let folder = ZPath::new(dir)?;
	for entry in project.list(&folder)? {
		let path = folder.join(&entry.name)?;
		match entry.kind {
			EntryKind::Dir => {
				let sub = format!("{dir}{}{}", if dir.ends_with('/') { "" } else { "/" }, entry.name);
				collect(project, &sub, &format!("{prefix}{}/", entry.name), out)?;
			}
			EntryKind::File | EntryKind::Symlink => {
				if let Some((stem, extension)) = entry.name.rsplit_once('.')
					&& EXTENSIONS.contains(&extension)
				{
					out.push((format!("{prefix}{stem}"), extension.to_owned(), path));
				}
			}
		}
	}
	Ok(())
}

/// The names of the shaders of the engine, for a message.
pub fn engine_names() -> impl Iterator<Item = &'static str> {
	ENGINE.iter().map(|&(name, _)| name).filter(|name| is_engine(name))
}
