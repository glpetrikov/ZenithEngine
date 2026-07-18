use std::collections::{HashMap, HashSet};

use ze_assets::{AssetRef, ResourceManager, TextureSheet};
use ze_core::Result;

/// Caches parsed `TextureSheet` JSON assets, mirroring `TextureCache`'s
/// shape (including a failure negative-cache so a broken/missing sheet
/// doesn't get re-read from disk and re-logged every frame).
pub struct SheetCache {
	sheets: HashMap<AssetRef, TextureSheet>,
	failed: HashSet<AssetRef>,
}

impl SheetCache {
	pub fn new() -> Self {
		Self {
			sheets: HashMap::new(),
			failed: HashSet::new(),
		}
	}

	pub fn get_or_load(&mut self, sheet_ref: &AssetRef, resources: &ResourceManager) -> Option<&TextureSheet> {
		if self.failed.contains(sheet_ref) {
			return None;
		}

		if !self.sheets.contains_key(sheet_ref) {
			let sheet = load_sheet(sheet_ref, resources);

			match sheet {
				Ok(sheet) => {
					self.sheets.insert(sheet_ref.clone(), sheet);
				}
				Err(error) => {
					ze_log::warn!("Failed to load texture sheet `{}`: {error:?}", sheet_ref.path);
					self.failed.insert(sheet_ref.clone());
					return None;
				}
			}
		}

		self.sheets.get(sheet_ref)
	}

	pub fn invalidate(&mut self, asset: &AssetRef) -> bool {
		self.failed.remove(asset);
		self.sheets.remove(asset).is_some()
	}

	pub fn invalidate_many<'a>(&mut self, assets: impl IntoIterator<Item = &'a AssetRef>) -> usize {
		assets.into_iter().filter(|asset| self.invalidate(asset)).count()
	}
}

impl Default for SheetCache {
	fn default() -> Self { Self::new() }
}

fn load_sheet(sheet_ref: &AssetRef, resources: &ResourceManager) -> Result<TextureSheet> {
	let text = resources.string(sheet_ref)?;
	Ok(serde_json::from_str(&text)?)
}
