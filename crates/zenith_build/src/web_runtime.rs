//! Copying the browser runtime (the output of `wasm-pack build --target web`)
//! next to a build.

use std::path::{Path, PathBuf};

use zenith_error::{ZResult, ZenithError};

use crate::engine_exe;

/// Folder next to the build that holds the runtime's `.js` and `.wasm`.
const PKG_DIR: &str = "pkg";
const INDEX_FILE: &str = "index.html";

/// The files that make the game run in a browser.
#[derive(Clone, Debug)]
pub struct WebRuntime {
	/// The `pkg` folder of `wasm-pack build --target web`.
	pub pkg: PathBuf,
	/// A page to use as `index.html`; a plain one that starts the runtime on a
	/// `<canvas id="zenith-canvas">` is written when there is none.
	pub index: Option<PathBuf>,
}

const fn bad(message: String) -> ZenithError {
	ZenithError::InvalidBuildOption(message)
}

/// What to copy where, worked out (and checked) before anything is built.
pub struct WebPlan {
	copies: Vec<(PathBuf, PathBuf)>,
	index: PathBuf,
	page: Page,
}

enum Page {
	Copy(PathBuf),
	Generate { script: String },
}

fn page_for(script: &str) -> String {
	format!(
		"<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"UTF-8\" />\n<title>Zenith Engine</title>\n<style>\nhtml, body {{ margin: 0; width: 100%; height: 100%; background: #303030; }}\ncanvas {{ width: 100%; height: 100%; }}\n</style>\n</head>\n<body>\n<canvas id=\"zenith-canvas\"></canvas>\n<script type=\"module\">\nimport init from \"./{PKG_DIR}/{script}\";\nawait init();\n</script>\n</body>\n</html>\n"
	)
}

/// Files below `dir` (not following links), sorted.
fn collect(dir: &Path, into: &mut Vec<PathBuf>) -> ZResult<()> {
	let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
	entries.sort_by_key(std::fs::DirEntry::file_name);
	for entry in entries {
		let kind = entry.file_type()?;
		if kind.is_dir() {
			collect(&entry.path(), into)?;
		} else if kind.is_file() {
			into.push(entry.path());
		}
	}
	Ok(())
}

fn has_extension(name: &str, extension: &str) -> bool {
	Path::new(name)
		.extension()
		.is_some_and(|e| e.eq_ignore_ascii_case(extension))
}

impl WebRuntime {
	/// The files to copy into `folder`.
	///
	/// # Errors
	/// `InvalidBuildOption` for a missing or unusable `pkg` folder or page.
	pub fn plan(&self, folder: &Path) -> ZResult<WebPlan> {
		let mut scripts = Vec::new();
		let mut copies = Vec::new();
		let mut wasm = 0usize;
		let mut entries: Vec<_> = std::fs::read_dir(&self.pkg)
			.map_err(|e| bad(format!("the web runtime folder {}: {e}", self.pkg.display())))?
			.collect::<Result<_, _>>()?;
		entries.sort_by_key(std::fs::DirEntry::file_name);
		let target = folder.join(PKG_DIR);
		for entry in entries {
			let kind = entry.file_type()?;
			let name = entry.file_name().to_string_lossy().into_owned();
			if kind.is_file() && has_extension(&name, "js") {
				scripts.push(name.clone());
				copies.push((entry.path(), target.join(&name)));
			} else if kind.is_file() && has_extension(&name, "wasm") {
				wasm += 1;
				copies.push((entry.path(), target.join(&name)));
			} else if kind.is_dir() && name == "snippets" {
				let mut files = Vec::new();
				collect(&entry.path(), &mut files)?;
				for file in files {
					let relative = file.strip_prefix(&self.pkg).map_err(|e| bad(e.to_string()))?;
					copies.push((file.clone(), target.join(relative)));
				}
			}
		}
		let [script] = scripts.as_slice() else {
			return Err(bad(format!(
				"{} must hold exactly one .js file (the output of `wasm-pack build --target web`), found {}",
				self.pkg.display(),
				scripts.len()
			)));
		};
		if wasm == 0 {
			return Err(bad(format!("{} holds no .wasm file", self.pkg.display())));
		}
		let page = if let Some(index) = &self.index {
			if !index.is_file() {
				return Err(bad(format!("the web page {} is not a file", index.display())));
			}
			Page::Copy(index.clone())
		} else {
			let safe = script
				.chars()
				.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
			if !safe {
				return Err(bad(format!("{script:?} cannot go into a generated page")));
			}
			Page::Generate { script: script.clone() }
		};
		Ok(WebPlan {
			copies,
			index: folder.join(INDEX_FILE),
			page,
		})
	}
}

impl WebPlan {
	/// Every file the plan writes.
	pub fn destinations(&self) -> impl Iterator<Item = &Path> {
		self.copies
			.iter()
			.map(|(_, to)| to.as_path())
			.chain(std::iter::once(self.index.as_path()))
	}

	/// Writes the files; returns where they went.
	pub fn write(&self) -> ZResult<Vec<PathBuf>> {
		let mut written = Vec::new();
		for (from, to) in &self.copies {
			if let Some(parent) = to.parent() {
				std::fs::create_dir_all(parent)?;
			}
			engine_exe::copy(from, to)?;
			written.push(to.clone());
		}
		match &self.page {
			Page::Copy(from) => engine_exe::copy(from, &self.index)?,
			Page::Generate { script } => std::fs::write(&self.index, page_for(script))?,
		}
		written.push(self.index.clone());
		Ok(written)
	}
}
