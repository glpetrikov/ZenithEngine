use std::path::{Path, PathBuf};

use crate::{Context, Result, bail};

pub fn resolve_engine_root() -> Result<PathBuf> {
	if cfg!(debug_assertions) {
		if let Some(root) = find_workspace_root_from_exe() {
			return Ok(root);
		}
		if let Some(root) = find_workspace_root_from_cargo_manifest_dir() {
			return Ok(root);
		}
	}

	let exe = std::env::current_exe().context("failed to get current executable path")?;
	let exe_dir = exe
		.parent()
		.context("failed to get directory containing the executable")?;

	if !exe_dir.join("api/cs").is_dir() {
		bail!(
			"expected engine asset directory api/cs next to executable at {}, \
			 but it was not found. Make sure the engine distribution includes \
			 the api/cs directory.",
			exe_dir.display()
		);
	}

	Ok(exe_dir.to_path_buf())
}

pub fn engine_api_dir() -> Result<PathBuf> {
	let root = resolve_engine_root()?;
	let dev_path = root.join("ZeroEngine/api/cs");
	if dev_path.is_dir() {
		return Ok(dev_path);
	}
	Ok(root.join("api/cs"))
}

fn is_workspace_root(path: &Path) -> bool { path.join("Cargo.toml").is_file() && path.join("ZeroEngine").is_dir() }

fn find_workspace_root_from_exe() -> Option<PathBuf> {
	let exe = std::env::current_exe().ok()?;
	let exe_dir = exe.parent()?;
	exe_dir.ancestors().find(|a| is_workspace_root(a)).map(PathBuf::from)
}

fn find_workspace_root_from_cargo_manifest_dir() -> Option<PathBuf> {
	let manifest = std::env::var("CARGO_MANIFEST_DIR").ok()?;
	Path::new(&manifest)
		.ancestors()
		.find(|a| is_workspace_root(a))
		.map(PathBuf::from)
}
