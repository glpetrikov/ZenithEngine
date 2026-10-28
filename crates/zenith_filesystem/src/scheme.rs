//! Scheme registry: maps scheme names to folders inside the project.

use std::collections::BTreeMap;

use zenith_error::{ZResult, ZenithError};

use crate::{
	layout::{BUILTIN_SCHEMES, DIR_GIT, GITIGNORE, MANIFEST_EXTENSION, V1_FLAG},
	zpath::{ZPath, validate_relative},
};

#[derive(Clone, Debug)]
pub struct Schemes {
	roots: BTreeMap<String, String>,
}

impl Schemes {
	pub fn with_builtin() -> Self {
		let roots = BUILTIN_SCHEMES
			.iter()
			.map(|(name, root)| ((*name).to_string(), (*root).to_string()))
			.collect();
		Self { roots }
	}

	pub fn register(&mut self, name: &str, root: &str) -> ZResult<()> {
		// Reuse ZPath's syntax check for the name.
		let name = ZPath::from_parts(name, "")?.scheme().to_string();
		let name = name.as_str();
		if root.is_empty() {
			return Err(ZenithError::InvalidZPath(format!(
				"scheme {name:?} needs a folder, not the project root"
			)));
		}
		validate_relative(root).map_err(|why| ZenithError::InvalidZPath(format!("{root:?}: {why}")))?;
		// A scheme is a way to write into the project. It may not reach the
		// files that make the folder a project, or the repository (a hook in
		// `.git/hooks` runs programs).
		let top = root.split('/').next().unwrap_or(root);
		if top == V1_FLAG || top == GITIGNORE || top == DIR_GIT || top.ends_with(&format!(".{MANIFEST_EXTENSION}")) {
			return Err(ZenithError::InvalidZPath(format!(
				"scheme {name:?}: {root:?} is not a folder a scheme can point at ({top} is managed by the engine or git)"
			)));
		}
		if self.roots.contains_key(name) {
			return Err(ZenithError::SchemeAlreadyRegistered(name.to_string()));
		}
		self.roots.insert(name.to_string(), root.to_string());
		Ok(())
	}

	/// Project-relative path of a `ZPath` (`/`-separated, never empty).
	pub fn resolve(&self, path: &ZPath) -> ZResult<String> {
		let root = self
			.roots
			.get(path.scheme())
			.ok_or_else(|| ZenithError::SchemeNotRegistered(path.scheme().to_string()))?;
		Ok(if path.path().is_empty() {
			root.clone()
		} else {
			format!("{root}/{}", path.path())
		})
	}

	/// Whether `rel` is exactly the root folder of some scheme.
	#[cfg(feature = "fs")]
	pub fn is_root(&self, rel: &str) -> bool {
		self.roots.values().any(|root| root == rel)
	}

	/// The `ZPath` of a project-relative path, using the scheme with the
	/// longest root (`Worlds://`, not `Assets://`). `None` if no scheme covers
	/// it.
	#[cfg(any(feature = "fs", test))]
	pub fn reverse(&self, rel: &str) -> Option<ZPath> {
		let (name, root) = self
			.roots
			.iter()
			.filter(|(_, root)| {
				rel == root.as_str() || rel.strip_prefix(root.as_str()).is_some_and(|r| r.starts_with('/'))
			})
			.max_by_key(|(_, root)| root.len())?;
		let rest = rel.get(root.len()..)?.trim_start_matches('/');
		ZPath::from_parts(name, rest).ok()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::layout::is_trash_path;

	#[test]
	fn resolves_and_reverses() -> ZResult<()> {
		let mut s = Schemes::with_builtin();
		assert_eq!(
			s.resolve(&ZPath::parse("Worlds://A.zenith")?)?,
			"Assets/Worlds/A.zenith",
			"worlds"
		);
		assert_eq!(s.resolve(&ZPath::parse("Assets://")?)?, "Assets", "scheme root");
		assert_eq!(
			s.reverse("Assets/Worlds/A.zenith").map(|p| p.to_string()),
			Some("Worlds://A.zenith".into()),
			"longest root wins"
		);
		assert_eq!(
			s.reverse("Assets/x").map(|p| p.to_string()),
			Some("Assets://x".into()),
			"assets"
		);
		assert_eq!(s.reverse("Other/x"), None, "uncovered path");
		assert_eq!(s.reverse("AssetsX/y"), None, "a prefix is not a parent folder");
		assert!(
			matches!(
				s.resolve(&ZPath::parse("nope://x")?),
				Err(ZenithError::SchemeNotRegistered(_))
			),
			"unknown scheme"
		);

		s.register("textures", "Assets/Textures")?;
		assert_eq!(
			s.resolve(&ZPath::parse("textures://a.png")?)?,
			"Assets/Textures/a.png",
			"custom scheme"
		);
		assert!(
			matches!(
				s.register("textures", "X"),
				Err(ZenithError::SchemeAlreadyRegistered(_))
			),
			"duplicate"
		);
		assert!(s.register("bad", "").is_err(), "empty root");
		assert!(s.register("bad", "../x").is_err(), "escaping root");
		for reserved in [".v1", ".gitignore", ".git", ".git/hooks", "Game.zenithproject"] {
			assert!(
				s.register("reserved", reserved).is_err(),
				"{reserved:?} is not a folder a scheme can point at"
			);
		}
		assert!(
			s.register("gitlike", ".github/workflows").is_ok(),
			"only the exact name .git is reserved, not names that start with it"
		);
		assert!(s.register("1bad", "X").is_err(), "bad name");
		assert!(s.register("my tex", "X").is_err(), "bad name");
		s.register("MyTextures", "Assets/My")?;
		assert_eq!(
			s.resolve(&ZPath::parse("my_textures://a")?)?,
			"Assets/My/a",
			"any spelling of the name finds it"
		);
		assert!(
			matches!(
				s.register("my-textures", "Y"),
				Err(ZenithError::SchemeAlreadyRegistered(_))
			),
			"another spelling of a used name is a duplicate"
		);
		assert!(is_trash_path(&s.resolve(&ZPath::parse("Trash://x")?)?), "trash scheme");
		s.register("sneaky", "Trash/x")?;
		assert!(
			is_trash_path(&s.resolve(&ZPath::parse("sneaky://y")?)?),
			"a scheme rooted inside the trash is still the trash"
		);
		Ok(())
	}
}
