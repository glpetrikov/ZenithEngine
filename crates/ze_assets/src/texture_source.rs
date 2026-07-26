use std::collections::{HashMap, HashSet};

use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{AssetRef, CellId, CellTrim, ResourceManager, TextureSheet, TextureSheetMode, uniform_grid_cols_rows};

/// Where a texture's pixels come from: either a plain file, or a specific
/// cell of a `TextureSheet` asset. Shared by every consumer that draws a
/// texture (`ze_renderer::Sprite`, `ze_ui::UIImage`) so there is exactly one
/// texture-reference format and one resolution path in the engine, not a
/// separate one per renderer.
///
/// `#[serde(untagged)]` with disjoint field sets (`path`/`source` for `File`
/// vs `sheet_path`/`cell_id` for `SheetCell`) is what makes this backward
/// compatible: an existing scene's plain `{"path":...,"source":"Game"}`
/// texture value still deserializes as `File` with zero migration code, since
/// serde tries each variant's shape in turn.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum TextureSource {
	File(AssetRef),
	SheetCell { sheet_path: String, cell_id: CellId },
}

impl From<AssetRef> for TextureSource {
	fn from(asset: AssetRef) -> Self { Self::File(asset) }
}

/// Caches parsed `TextureSheet` JSON assets, keyed by their `AssetRef`
/// (including a failure negative-cache so a broken/missing sheet doesn't get
/// re-read from disk and re-logged every frame).
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

/// A `SheetCell` resolved down to "which grid cell of the eventual source
/// texture", deferred until that texture is actually loaded (and its pixel
/// dimensions known) since row-major decoding needs `cols = texture_width /
/// cell_width`.
pub enum PendingGridCell {
	None,
	Cell {
		cell_index: u32,
		cell_width: u32,
		cell_height: u32,
		origin_x: u32,
		origin_y: u32,
		trim: Option<CellTrim>,
	},
}

pub const FULL_UV_RECT: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// Resolves a `TextureSource` down to the `AssetRef` that should actually be
/// loaded as a GPU texture, plus (for `SheetCell` on a `UniformGrid` sheet)
/// enough info to compute the cell's UV rect once that texture's pixel
/// dimensions are known (see `resolve_uv_rect`).
pub fn resolve_texture_source(
	source: &TextureSource,
	sheet_cache: &mut SheetCache,
	resources: &ResourceManager,
) -> (AssetRef, PendingGridCell) {
	match source {
		TextureSource::File(asset_ref) => (asset_ref.clone(), PendingGridCell::None),
		TextureSource::SheetCell { sheet_path, cell_id } => {
			let sheet_ref = AssetRef::game(sheet_path.clone());
			let Some(sheet) = sheet_cache.get_or_load(&sheet_ref, resources) else {
				return (AssetRef::game(String::new()), PendingGridCell::None);
			};

			match &sheet.mode {
				TextureSheetMode::UniformGrid(grid) => {
					let trim = grid.cells.get(cell_id.0 as usize).and_then(|cell| cell.trim);
					(
						grid.texture.clone(),
						PendingGridCell::Cell {
							cell_index: cell_id.0,
							cell_width: grid.cell_width,
							cell_height: grid.cell_height,
							origin_x: grid.origin_x,
							origin_y: grid.origin_y,
							trim,
						},
					)
				}
				TextureSheetMode::AutoPack(pack) => {
					let file = pack
						.files
						.get(cell_id.0 as usize)
						.cloned()
						.unwrap_or_else(|| AssetRef::game(String::new()));
					(file, PendingGridCell::None)
				}
			}
		}
	}
}

/// Normalizes a `PendingGridCell` into a `uv_rect` (`[0,0,1,1]` for `None`,
/// i.e. sample the whole texture) using the now-loaded texture's pixel
/// dimensions, plus two size hints for a caller that sizes itself
/// automatically (e.g. `SpriteSize::Auto`): the "effective" size (the trimmed
/// cell size, i.e. the actual visible footprint) and the "nominal" size (the
/// untrimmed grid cell -- or, for `None`, the same as `texture_dimensions`).
/// Auto-sizing must scale against *both*: using the nominal size alone would
/// ignore trimming entirely, but using the effective size alone (as an
/// earlier version of this function did) loses the untrimmed cell as a
/// common reference point, so an animation's differently-trimmed frames each
/// silently renormalize their own aspect ratio instead of shrinking/growing
/// relative to one shared, stable size.
pub fn resolve_uv_rect(pending: PendingGridCell, texture_dimensions: (u32, u32)) -> ([f32; 4], (u32, u32), (u32, u32)) {
	let PendingGridCell::Cell {
		cell_index,
		cell_width,
		cell_height,
		origin_x,
		origin_y,
		trim,
	} = pending
	else {
		return (FULL_UV_RECT, texture_dimensions, texture_dimensions);
	};

	let (texture_width, texture_height) = texture_dimensions;
	let (cols, _rows) = uniform_grid_cols_rows(
		texture_width,
		texture_height,
		cell_width,
		cell_height,
		origin_x,
		origin_y,
	);
	let cols = cols.max(1);
	let col = cell_index % cols;
	let row = cell_index / cols;
	let cell_origin = (origin_x + col * cell_width, origin_y + row * cell_height);

	let (offset, size) = trim.map_or(((0, 0), (cell_width, cell_height)), |trim| (trim.offset, trim.size));
	let pixel_x = cell_origin.0 + offset.0;
	let pixel_y = cell_origin.1 + offset.1;

	let uv_rect = [
		pixel_x as f32 / texture_width.max(1) as f32,
		pixel_y as f32 / texture_height.max(1) as f32,
		size.0 as f32 / texture_width.max(1) as f32,
		size.1 as f32 / texture_height.max(1) as f32,
	];

	(uv_rect, size, (cell_width, cell_height))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{AutoPackSheet, GridCell, UniformGridSheet};

	#[test]
	fn file_source_resolves_to_full_uv_rect_unchanged() {
		let (uv_rect, effective, nominal) = resolve_uv_rect(PendingGridCell::None, (64, 32));
		assert_eq!(uv_rect, FULL_UV_RECT);
		assert_eq!(effective, (64, 32));
		assert_eq!(nominal, (64, 32));
	}

	#[test]
	fn grid_cell_resolves_to_normalized_trimmed_uv_rect() {
		// A 2x1 grid of 32x32 cells in a 64x32 source texture; cell 1's trim
		// starts 4px into its own cell and is 20x24.
		let pending = PendingGridCell::Cell {
			cell_index: 1,
			cell_width: 32,
			cell_height: 32,
			origin_x: 0,
			origin_y: 0,
			trim: Some(CellTrim {
				offset: (4, 2),
				size: (20, 24),
				original_size: (32, 32),
				manual: false,
			}),
		};

		let (uv_rect, effective, nominal) = resolve_uv_rect(pending, (64, 32));
		assert_eq!(effective, (20, 24));
		assert_eq!(nominal, (32, 32));
		// cell 1 origin = (32, 0) + trim offset (4, 2) = (36, 2), normalized by 64x32.
		assert!((uv_rect[0] - 36.0 / 64.0).abs() < 1e-6);
		assert!((uv_rect[1] - 2.0 / 32.0).abs() < 1e-6);
		assert!((uv_rect[2] - 20.0 / 64.0).abs() < 1e-6);
		assert!((uv_rect[3] - 24.0 / 32.0).abs() < 1e-6);
	}

	#[test]
	fn grid_cell_resolves_uv_rect_with_grid_origin_offset() {
		// Same 2x1 grid of 32x32 cells, but the grid itself starts 8px into
		// the source texture (e.g. a sheet with a margin before its first
		// cell) -- cell 1's pixel origin must shift by that same offset.
		let pending = PendingGridCell::Cell {
			cell_index: 1,
			cell_width: 32,
			cell_height: 32,
			origin_x: 8,
			origin_y: 0,
			trim: None,
		};

		let (uv_rect, effective, nominal) = resolve_uv_rect(pending, (72, 32));
		assert_eq!(effective, (32, 32));
		assert_eq!(nominal, (32, 32));
		// cell 1 origin = grid origin (8, 0) + cell (32, 0) = (40, 0), normalized by
		// 72x32.
		assert!((uv_rect[0] - 40.0 / 72.0).abs() < 1e-6);
		assert!((uv_rect[1] - 0.0).abs() < 1e-6);
	}

	#[test]
	fn sheet_cell_on_uniform_grid_resolves_texture_and_pending_cell() {
		let tempdir = std::env::temp_dir().join(format!("ze_assets_test_{}", std::process::id()));
		std::fs::create_dir_all(&tempdir).unwrap();
		let sheet = TextureSheet::new_uniform_grid(UniformGridSheet {
			texture: AssetRef::game("textures/sheet.png"),
			cell_width: 16,
			cell_height: 16,
			origin_x: 0,
			origin_y: 0,
			background_color: [0.0, 0.0, 0.0],
			cells: vec![
				GridCell { trim: None },
				GridCell {
					trim: Some(CellTrim {
						offset: (1, 1),
						size: (14, 14),
						original_size: (16, 16),
						manual: false,
					}),
				},
			],
		});
		sheet.save(&tempdir.join("test.texturesheet.json")).unwrap();

		let resources = ResourceManager::new(&tempdir);
		let mut sheet_cache = SheetCache::new();
		let source = TextureSource::SheetCell {
			sheet_path: "test.texturesheet.json".to_string(),
			cell_id: CellId(1),
		};

		let (asset_ref, pending) = resolve_texture_source(&source, &mut sheet_cache, &resources);
		assert_eq!(asset_ref, AssetRef::game("textures/sheet.png"));
		let PendingGridCell::Cell {
			cell_index,
			cell_width,
			cell_height,
			trim,
			..
		} = pending
		else {
			panic!("expected a resolved grid cell");
		};
		assert_eq!(cell_index, 1);
		assert_eq!(cell_width, 16);
		assert_eq!(cell_height, 16);
		assert_eq!(trim.unwrap().offset, (1, 1));

		std::fs::remove_dir_all(&tempdir).ok();
	}

	#[test]
	fn sheet_cell_on_auto_pack_resolves_to_the_files_own_asset_ref() {
		let tempdir = std::env::temp_dir().join(format!("ze_assets_test_autopack_{}", std::process::id()));
		std::fs::create_dir_all(&tempdir).unwrap();
		let sheet = TextureSheet::new_auto_pack(AutoPackSheet {
			files: vec![AssetRef::game("a.png"), AssetRef::game("b.png")],
		});
		sheet.save(&tempdir.join("palette.texturesheet.json")).unwrap();

		let resources = ResourceManager::new(&tempdir);
		let mut sheet_cache = SheetCache::new();
		let source = TextureSource::SheetCell {
			sheet_path: "palette.texturesheet.json".to_string(),
			cell_id: CellId(1),
		};

		let (asset_ref, pending) = resolve_texture_source(&source, &mut sheet_cache, &resources);
		assert_eq!(asset_ref, AssetRef::game("b.png"));
		assert!(matches!(pending, PendingGridCell::None));

		std::fs::remove_dir_all(&tempdir).ok();
	}
}
