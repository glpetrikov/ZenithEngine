use std::{fs, path::Path};

use anyhow::Result;
use image::RgbaImage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::AssetRef;

pub const TEXTURE_SHEET_EXTENSION: &str = "texturesheet.json";
const TEXTURE_SHEET_VERSION: &str = "1";

/// Alpha values at or below this are treated as fully transparent when
/// deciding whether a cell has meaningful alpha variation (antialiased-edge
/// tolerance).
const ALPHA_VARIATION_TOLERANCE: u8 = 2;
/// Per-channel distance from `background_color` beyond which a pixel counts
/// as "real content" for cells without a meaningful alpha channel.
const BACKGROUND_CHANNEL_THRESHOLD: u8 = 8;

/// Identifies one cell in a `TextureSheet`: a row-major index into the grid
/// for `UniformGrid` sheets, or an index into `AutoPackSheet::files` for
/// `AutoPack` sheets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct CellId(pub u32);

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TextureSheet {
	pub version: String,
	pub mode: TextureSheetMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum TextureSheetMode {
	UniformGrid(UniformGridSheet),
	AutoPack(AutoPackSheet),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UniformGridSheet {
	pub texture: AssetRef,
	pub cell_width: u32,
	pub cell_height: u32,
	pub background_color: [f32; 3],
	/// Row-major, `len() == cols * rows` where `cols = floor(texture_width /
	/// cell_width)` and `rows = floor(texture_height / cell_height)`.
	pub cells: Vec<GridCell>,
}

/// `trim: None` means the cell is entirely background/transparent -- empty
/// space in the source texture, not a real tile. Editors should skip such
/// cells (not selectable/placeable).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GridCell {
	pub trim: Option<CellTrim>,
}

/// TexturePacker-style trim data: `offset`+`size` describe the trimmed rect
/// in cell-local pixel coordinates (relative to the cell's own untrimmed
/// top-left corner), and `original_size` is the untrimmed cell dimensions
/// (`cell_width` x `cell_height`) so a sprite can still be positioned as if
/// against the full, untrimmed cell.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct CellTrim {
	pub offset: (u32, u32),
	pub size: (u32, u32),
	pub original_size: (u32, u32),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutoPackSheet {
	/// Index into this list is the cell's `CellId`.
	pub files: Vec<AssetRef>,
}

impl TextureSheet {
	pub fn new_uniform_grid(sheet: UniformGridSheet) -> Self {
		Self {
			version: TEXTURE_SHEET_VERSION.to_string(),
			mode: TextureSheetMode::UniformGrid(sheet),
		}
	}

	pub fn new_auto_pack(sheet: AutoPackSheet) -> Self {
		Self {
			version: TEXTURE_SHEET_VERSION.to_string(),
			mode: TextureSheetMode::AutoPack(sheet),
		}
	}

	pub fn load(path: &Path) -> Result<Self> {
		let text = fs::read_to_string(path)?;
		Ok(serde_json::from_str(&text)?)
	}

	pub fn save(&self, path: &Path) -> Result<()> {
		let text = serde_json::to_string_pretty(self)?;
		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent)?;
		}
		fs::write(path, text)?;
		Ok(())
	}
}

impl UniformGridSheet {
	/// `(cols, rows)` given the full source texture's pixel dimensions.
	pub fn grid_dimensions(&self, source_width: u32, source_height: u32) -> (u32, u32) {
		(
			source_width / self.cell_width.max(1),
			source_height / self.cell_height.max(1),
		)
	}
}

/// Slices `image` into a `cols x rows` grid of `cell_width x cell_height`
/// cells (row-major) and computes a per-cell trim rect.
///
/// This is edit-time-only logic: computed once when a sheet is created or
/// its cell size / background color changes in ZeroEditor, then stored in
/// the sheet's JSON -- never recomputed at runtime load.
pub fn compute_uniform_grid_trims(
	image: &RgbaImage,
	cell_width: u32,
	cell_height: u32,
	background_color: [f32; 3],
) -> Vec<GridCell> {
	let cell_width = cell_width.max(1);
	let cell_height = cell_height.max(1);
	let cols = image.width() / cell_width;
	let rows = image.height() / cell_height;

	let mut cells = Vec::with_capacity((cols * rows) as usize);
	for row in 0..rows {
		for col in 0..cols {
			let origin_x = col * cell_width;
			let origin_y = row * cell_height;
			let trim = trim_cell(image, origin_x, origin_y, cell_width, cell_height, background_color);
			cells.push(GridCell { trim });
		}
	}
	cells
}

fn trim_cell(
	image: &RgbaImage,
	origin_x: u32,
	origin_y: u32,
	cell_width: u32,
	cell_height: u32,
	background_color: [f32; 3],
) -> Option<CellTrim> {
	let background = [
		(background_color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
		(background_color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
		(background_color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
	];

	let has_alpha_variation = cell_has_alpha_variation(image, origin_x, origin_y, cell_width, cell_height);

	let mut min_x = cell_width;
	let mut min_y = cell_height;
	let mut max_x = 0u32;
	let mut max_y = 0u32;
	let mut found = false;

	for local_y in 0..cell_height {
		for local_x in 0..cell_width {
			let pixel = image.get_pixel(origin_x + local_x, origin_y + local_y);
			let is_content = if has_alpha_variation {
				pixel[3] > ALPHA_VARIATION_TOLERANCE
			} else {
				channel_diff(pixel[0], background[0]) > BACKGROUND_CHANNEL_THRESHOLD
					|| channel_diff(pixel[1], background[1]) > BACKGROUND_CHANNEL_THRESHOLD
					|| channel_diff(pixel[2], background[2]) > BACKGROUND_CHANNEL_THRESHOLD
			};

			if is_content {
				found = true;
				min_x = min_x.min(local_x);
				min_y = min_y.min(local_y);
				max_x = max_x.max(local_x);
				max_y = max_y.max(local_y);
			}
		}
	}

	if !found {
		return None;
	}

	Some(CellTrim {
		offset: (min_x, min_y),
		size: (max_x - min_x + 1, max_y - min_y + 1),
		original_size: (cell_width, cell_height),
	})
}

fn cell_has_alpha_variation(
	image: &RgbaImage,
	origin_x: u32,
	origin_y: u32,
	cell_width: u32,
	cell_height: u32,
) -> bool {
	let mut first: Option<u8> = None;
	for local_y in 0..cell_height {
		for local_x in 0..cell_width {
			let alpha = image.get_pixel(origin_x + local_x, origin_y + local_y)[3];
			match first {
				None => first = Some(alpha),
				Some(value) if value != alpha => return true,
				Some(_) => {}
			}
		}
	}
	false
}

fn channel_diff(a: u8, b: u8) -> u8 { a.abs_diff(b) }

/// One rect placed by `shelf_pack_preview`, in preview-canvas pixel
/// coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShelfPackedRect {
	pub file_index: usize,
	pub x: u32,
	pub y: u32,
	pub width: u32,
	pub height: u32,
}

/// Shelf-packs `file_dimensions` (sort by height descending, place
/// left-to-right until `max_row_width` is hit, then start a new row) purely
/// to lay out the Auto-pack preview grid in the ZeroEditor panel.
///
/// This does **not** composite the source files into a single GPU
/// texture/atlas -- it only computes where to draw each file's own preview
/// thumbnail in the panel. Selecting an Auto-pack cell always resolves to
/// one of the original files (functionally identical to `TextureSource::File`);
/// there is no real atlas backing this layout. The result is never
/// persisted -- recompute on every panel open.
pub fn shelf_pack_preview(file_dimensions: &[(u32, u32)], max_row_width: u32) -> Vec<ShelfPackedRect> {
	let mut order: Vec<usize> = (0..file_dimensions.len()).collect();
	order.sort_by_key(|&index| std::cmp::Reverse(file_dimensions[index].1));

	let mut rects = Vec::with_capacity(file_dimensions.len());
	let mut cursor_x = 0u32;
	let mut cursor_y = 0u32;
	let mut row_height = 0u32;

	for index in order {
		let (width, height) = file_dimensions[index];

		if cursor_x > 0 && cursor_x + width > max_row_width.max(1) {
			cursor_x = 0;
			cursor_y += row_height;
			row_height = 0;
		}

		rects.push(ShelfPackedRect {
			file_index: index,
			x: cursor_x,
			y: cursor_y,
			width,
			height,
		});

		cursor_x += width;
		row_height = row_height.max(height);
	}

	rects
}

#[cfg(test)]
mod tests {
	use image::Rgba;

	use super::*;

	fn make_image(width: u32, height: u32, fill: Rgba<u8>) -> RgbaImage { RgbaImage::from_pixel(width, height, fill) }

	fn set_rect(image: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, color: Rgba<u8>) {
		for dy in 0..h {
			for dx in 0..w {
				image.put_pixel(x + dx, y + dy, color);
			}
		}
	}

	#[test]
	fn fully_transparent_cell_has_no_trim() {
		// 2x1 grid of 8x8 cells; cell 0 is fully transparent (alpha path).
		let mut image = make_image(16, 8, Rgba([0, 0, 0, 0]));
		set_rect(&mut image, 8, 0, 8, 8, Rgba([255, 0, 0, 255]));

		let cells = compute_uniform_grid_trims(&image, 8, 8, [0.0, 0.0, 0.0]);
		assert_eq!(cells.len(), 2);
		assert!(cells[0].trim.is_none());
		assert!(cells[1].trim.is_some());
	}

	#[test]
	fn fully_background_cell_has_no_trim() {
		// No alpha variation anywhere (opaque background_color path) --
		// one cell is pure background, the other has a distinct square.
		let background = Rgba([10, 20, 30, 255]);
		let mut image = make_image(16, 8, background);
		set_rect(&mut image, 8, 0, 8, 8, Rgba([200, 200, 200, 255]));

		let cells = compute_uniform_grid_trims(&image, 8, 8, [10.0 / 255.0, 20.0 / 255.0, 30.0 / 255.0]);
		assert_eq!(cells.len(), 2);
		assert!(cells[0].trim.is_none());
		assert!(cells[1].trim.is_some());
	}

	#[test]
	fn alpha_path_trims_to_exact_bbox_of_opaque_pixels() {
		// Cell has alpha variation (some fully-transparent padding), so the
		// alpha>2 branch is used regardless of background_color.
		let mut image = make_image(16, 16, Rgba([0, 0, 0, 0]));
		set_rect(&mut image, 4, 6, 5, 3, Rgba([255, 255, 255, 255]));

		let cells = compute_uniform_grid_trims(&image, 16, 16, [1.0, 0.0, 1.0]);
		let trim = cells[0].trim.expect("expected a trim rect");
		assert_eq!(trim.offset, (4, 6));
		assert_eq!(trim.size, (5, 3));
		assert_eq!(trim.original_size, (16, 16));
	}

	#[test]
	fn background_distance_path_trims_to_exact_bbox_when_fully_opaque() {
		// Fully opaque everywhere (no alpha variation), so the
		// background_color-distance branch is used.
		let background = Rgba([0, 0, 0, 255]);
		let mut image = make_image(10, 10, background);
		set_rect(&mut image, 2, 3, 4, 5, Rgba([255, 255, 255, 255]));

		let cells = compute_uniform_grid_trims(&image, 10, 10, [0.0, 0.0, 0.0]);
		let trim = cells[0].trim.expect("expected a trim rect");
		assert_eq!(trim.offset, (2, 3));
		assert_eq!(trim.size, (4, 5));
		assert_eq!(trim.original_size, (10, 10));
	}

	#[test]
	fn shelf_pack_preview_avoids_overlaps_and_respects_row_width() {
		let dimensions = [(10, 20), (15, 10), (8, 8), (30, 5)];
		let rects = shelf_pack_preview(&dimensions, 25);

		assert_eq!(rects.len(), dimensions.len());
		for rect in &rects {
			assert!(rect.x + rect.width <= 30, "rect exceeds a sane bound: {rect:?}");
		}

		for (i, a) in rects.iter().enumerate() {
			for b in rects.iter().skip(i + 1) {
				let overlap_x = a.x < b.x + b.width && b.x < a.x + a.width;
				let overlap_y = a.y < b.y + b.height && b.y < a.y + a.height;
				assert!(!(overlap_x && overlap_y), "rects overlap: {a:?} vs {b:?}");
			}
		}
	}

	#[test]
	fn round_trips_uniform_grid_sheet_through_json() {
		let sheet = TextureSheet::new_uniform_grid(UniformGridSheet {
			texture: AssetRef::game("textures/sheet.png"),
			cell_width: 16,
			cell_height: 16,
			background_color: [1.0, 0.0, 1.0],
			cells: vec![
				GridCell { trim: None },
				GridCell {
					trim: Some(CellTrim {
						offset: (1, 2),
						size: (14, 12),
						original_size: (16, 16),
					}),
				},
			],
		});

		let json = serde_json::to_string(&sheet).expect("serialize");
		let round_tripped: TextureSheet = serde_json::from_str(&json).expect("deserialize");

		match round_tripped.mode {
			TextureSheetMode::UniformGrid(grid) => {
				assert_eq!(grid.cell_width, 16);
				assert_eq!(grid.cells.len(), 2);
				assert!(grid.cells[0].trim.is_none());
				assert_eq!(grid.cells[1].trim.unwrap().offset, (1, 2));
			}
			TextureSheetMode::AutoPack(_) => panic!("expected UniformGrid mode"),
		}
	}

	#[test]
	fn round_trips_auto_pack_sheet_through_json() {
		let sheet = TextureSheet::new_auto_pack(AutoPackSheet {
			files: vec![AssetRef::game("a.png"), AssetRef::game("b.png")],
		});

		let json = serde_json::to_string(&sheet).expect("serialize");
		let round_tripped: TextureSheet = serde_json::from_str(&json).expect("deserialize");

		match round_tripped.mode {
			TextureSheetMode::AutoPack(pack) => assert_eq!(pack.files.len(), 2),
			TextureSheetMode::UniformGrid(_) => panic!("expected AutoPack mode"),
		}
	}
}
