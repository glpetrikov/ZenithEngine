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
/// tolerance). Kept at `0` (only a fully-invisible pixel is "background") --
/// a higher tolerance previously excluded faintly-visible antialiased edge
/// pixels (e.g. alpha 1-2) from the trim bbox, cropping a visible sliver off
/// round/filled shapes.
const ALPHA_VARIATION_TOLERANCE: u8 = 0;
/// Per-channel distance from `background_color` beyond which a pixel counts
/// as "real content" for cells without a meaningful alpha channel. Kept small
/// (rather than the much larger tolerance this used to have) so a subtly
/// antialiased edge pixel blended mostly-but-not-entirely toward the
/// background color still counts as content instead of being cropped away;
/// still nonzero to tolerate the `background_color` f32->u8 round-trip's
/// rounding noise (see `trim_cell`).
const BACKGROUND_CHANNEL_THRESHOLD: u8 = 2;

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
	/// Pixel offset of the grid's first cell's top-left corner from the
	/// source image's own `(0, 0)` -- lets a sheet with a margin/padding
	/// before its first row/column still line up cleanly with `cell_width x
	/// cell_height` cells instead of every cell being misaligned relative to
	/// the actual sprite content. `#[serde(default)]` so sheets saved before
	/// this field existed still deserialize, defaulting to no offset (i.e.
	/// unchanged behavior).
	#[serde(default)]
	pub origin_x: u32,
	#[serde(default)]
	pub origin_y: u32,
	pub background_color: [f32; 3],
	/// Row-major, `len() == cols * rows` -- see `uniform_grid_cols_rows` for
	/// how `cols`/`rows` are derived from `cell_width`/`cell_height`,
	/// `origin_x`/`origin_y`, and the source texture's pixel dimensions.
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
/// top-left corner), and `original_size` is the untrimmed cell dimensions --
/// normally the sheet's nominal `cell_width x cell_height`, but smaller for a
/// cell clamped to the source image's edge (see `clamp_cell_rect`) -- so a
/// sprite can still be positioned as if against the full, untrimmed cell.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct CellTrim {
	pub offset: (u32, u32),
	pub size: (u32, u32),
	pub original_size: (u32, u32),
	/// `true` if this trim was hand-adjusted in `ZeroEditor`'s Texture Sheet
	/// panel rather than produced by `compute_uniform_grid_trims`'s
	/// alpha/background-color detection. `#[serde(default)]` so sheets saved
	/// before this field existed still deserialize (defaulting to `false`,
	/// i.e. auto-detected). `recompute_uniform_grid_trims_preserving_manual`
	/// checks this to avoid silently overwriting a manual adjustment when
	/// re-trimming after an unrelated change (source texture reload,
	/// background-color edit, "Re-trim All").
	#[serde(default)]
	pub manual: bool,
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
	/// `(cols, rows)` given the full source texture's pixel dimensions -- see
	/// `uniform_grid_cols_rows` for the clamping rules.
	pub fn grid_dimensions(&self, source_width: u32, source_height: u32) -> (u32, u32) {
		uniform_grid_cols_rows(
			source_width,
			source_height,
			self.cell_width,
			self.cell_height,
			self.origin_x,
			self.origin_y,
		)
	}
}

/// Computes `(cols, rows)` for a `UniformGrid` sheet's cell layout: how many
/// `cell_width x cell_height` cells fit into a `source_width x
/// source_height` image starting at `(origin_x, origin_y)`.
///
/// Uses floor division along an axis where at least one full cell already
/// fits (the common case -- this correctly ignores an intentional
/// trailing/leading margin narrower than one cell without inventing a spurious
/// partial cell for it), but clamps up to `1` instead of truncating to `0`
/// when the origin-adjusted source is nonempty yet narrower/shorter than a
/// single cell. Without that clamp, increasing `cell_width`/`cell_height`
/// past what an axis can fully contain made the *entire* grid's cell list
/// come back empty (`cols` or `rows` floors to `0`, so `cols * rows == 0`)
/// instead of yielding one edge-clamped cell -- see `clamp_cell_rect` for how
/// that cell's own pixel bounds are then clamped to whatever is actually
/// available.
pub fn uniform_grid_cols_rows(
	source_width: u32,
	source_height: u32,
	cell_width: u32,
	cell_height: u32,
	origin_x: u32,
	origin_y: u32,
) -> (u32, u32) {
	let cell_width = cell_width.max(1);
	let cell_height = cell_height.max(1);
	let available_width = source_width.saturating_sub(origin_x);
	let available_height = source_height.saturating_sub(origin_y);

	let cols = if available_width == 0 {
		0
	} else {
		(available_width / cell_width).max(1)
	};
	let rows = if available_height == 0 {
		0
	} else {
		(available_height / cell_height).max(1)
	};
	(cols, rows)
}

/// Clamps a nominal `cell_width x cell_height` cell at grid position `(col,
/// row)` -- pixel origin `(origin_x + col * cell_width, origin_y + row *
/// cell_height)` -- to whatever pixels are actually available in a
/// `source_width x source_height` image. Returns `(cell_origin_x,
/// cell_origin_y, actual_width, actual_height)`; the latter two never exceed
/// `cell_width`/`cell_height` but may be smaller for an edge cell that
/// `uniform_grid_cols_rows` rounded up to `1` rather than dropping.
fn clamp_cell_rect(
	source_width: u32,
	source_height: u32,
	cell_width: u32,
	cell_height: u32,
	origin_x: u32,
	origin_y: u32,
	col: u32,
	row: u32,
) -> (u32, u32, u32, u32) {
	let cell_origin_x = origin_x + col * cell_width;
	let cell_origin_y = origin_y + row * cell_height;
	let actual_width = cell_width.min(source_width.saturating_sub(cell_origin_x));
	let actual_height = cell_height.min(source_height.saturating_sub(cell_origin_y));
	(cell_origin_x, cell_origin_y, actual_width, actual_height)
}

/// Slices `image` into a `cols x rows` grid of `cell_width x cell_height`
/// cells (row-major, starting at `(origin_x, origin_y)`) and computes a
/// per-cell trim rect.
///
/// This is edit-time-only logic: computed once when a sheet is created or
/// its cell size / origin / background color changes in ZeroEditor, then
/// stored in the sheet's JSON -- never recomputed at runtime load.
pub fn compute_uniform_grid_trims(
	image: &RgbaImage,
	cell_width: u32,
	cell_height: u32,
	origin_x: u32,
	origin_y: u32,
	background_color: [f32; 3],
) -> Vec<GridCell> {
	let cell_width = cell_width.max(1);
	let cell_height = cell_height.max(1);
	let (cols, rows) = uniform_grid_cols_rows(
		image.width(),
		image.height(),
		cell_width,
		cell_height,
		origin_x,
		origin_y,
	);

	let mut cells = Vec::with_capacity((cols * rows) as usize);
	for row in 0..rows {
		for col in 0..cols {
			let (cell_origin_x, cell_origin_y, actual_width, actual_height) = clamp_cell_rect(
				image.width(),
				image.height(),
				cell_width,
				cell_height,
				origin_x,
				origin_y,
				col,
				row,
			);
			let trim = trim_cell(
				image,
				cell_origin_x,
				cell_origin_y,
				actual_width,
				actual_height,
				background_color,
			);
			cells.push(GridCell { trim });
		}
	}
	cells
}

/// `cell_width`/`cell_height` here are the cell's *actual* pixel dimensions
/// (already clamped by the caller for an edge cell -- see
/// `clamp_cell_rect`), not necessarily the sheet's nominal configured size.
fn trim_cell(
	image: &RgbaImage,
	cell_origin_x: u32,
	cell_origin_y: u32,
	cell_width: u32,
	cell_height: u32,
	background_color: [f32; 3],
) -> Option<CellTrim> {
	let background = [
		(background_color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
		(background_color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
		(background_color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
	];

	let has_alpha_variation = cell_has_alpha_variation(image, cell_origin_x, cell_origin_y, cell_width, cell_height);

	let mut min_x = cell_width;
	let mut min_y = cell_height;
	let mut max_x = 0u32;
	let mut max_y = 0u32;
	let mut found = false;

	for local_y in 0..cell_height {
		for local_x in 0..cell_width {
			let pixel = image.get_pixel(cell_origin_x + local_x, cell_origin_y + local_y);
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
		manual: false,
	})
}

/// Same as `compute_uniform_grid_trims`, but preserves manual overrides.
///
/// Keeps `existing_cells`' trim in place for any cell whose trim has
/// `manual: true` instead of overwriting it with a fresh auto-detected one --
/// so a hand-adjusted crop box survives an unrelated re-trim (source texture
/// reload, background-color/origin change, `ZeroEditor`'s "Re-trim All"
/// button). A manual trim is only kept if its `original_size` still matches
/// that cell's *actual* current dimensions (see `clamp_cell_rect` -- an edge
/// cell's actual size can be smaller than the sheet's nominal `cell_width x
/// cell_height`) -- if the grid's own layout changed enough to resize this
/// specific cell, the override no longer applies to the same cell bounds and
/// falls back to auto-detection like any other cell.
pub fn recompute_uniform_grid_trims_preserving_manual(
	image: &RgbaImage,
	cell_width: u32,
	cell_height: u32,
	origin_x: u32,
	origin_y: u32,
	background_color: [f32; 3],
	existing_cells: &[GridCell],
) -> Vec<GridCell> {
	let cell_width = cell_width.max(1);
	let cell_height = cell_height.max(1);
	let (cols, _rows) = uniform_grid_cols_rows(
		image.width(),
		image.height(),
		cell_width,
		cell_height,
		origin_x,
		origin_y,
	);
	let mut cells = compute_uniform_grid_trims(image, cell_width, cell_height, origin_x, origin_y, background_color);

	for (index, cell) in cells.iter_mut().enumerate() {
		if let Some(existing_trim) = existing_cells.get(index).and_then(|existing| existing.trim)
			&& existing_trim.manual
		{
			let col = index as u32 % cols.max(1);
			let row = index as u32 / cols.max(1);
			let (_, _, actual_width, actual_height) = clamp_cell_rect(
				image.width(),
				image.height(),
				cell_width,
				cell_height,
				origin_x,
				origin_y,
				col,
				row,
			);
			if existing_trim.original_size == (actual_width, actual_height) {
				cell.trim = Some(existing_trim);
			}
		}
	}

	cells
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

		let cells = compute_uniform_grid_trims(&image, 8, 8, 0, 0, [0.0, 0.0, 0.0]);
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

		let cells = compute_uniform_grid_trims(&image, 8, 8, 0, 0, [10.0 / 255.0, 20.0 / 255.0, 30.0 / 255.0]);
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

		let cells = compute_uniform_grid_trims(&image, 16, 16, 0, 0, [1.0, 0.0, 1.0]);
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

		let cells = compute_uniform_grid_trims(&image, 10, 10, 0, 0, [0.0, 0.0, 0.0]);
		let trim = cells[0].trim.expect("expected a trim rect");
		assert_eq!(trim.offset, (2, 3));
		assert_eq!(trim.size, (4, 5));
		assert_eq!(trim.original_size, (10, 10));
	}

	#[test]
	fn alpha_path_includes_faintly_visible_antialiased_edge_pixels() {
		// A ring of alpha=1 pixels around an opaque square simulates a
		// antialiased edge tapering almost to nothing at its outermost row/col.
		// These must still count as content -- excluding them (the old
		// tolerance of 2) cropped a visible sliver off round/filled shapes.
		let mut image = make_image(10, 10, Rgba([0, 0, 0, 0]));
		set_rect(&mut image, 3, 3, 4, 4, Rgba([255, 255, 255, 1]));
		set_rect(&mut image, 4, 4, 2, 2, Rgba([255, 255, 255, 255]));

		let cells = compute_uniform_grid_trims(&image, 10, 10, 0, 0, [1.0, 0.0, 1.0]);
		let trim = cells[0].trim.expect("expected a trim rect");
		assert_eq!(trim.offset, (3, 3));
		assert_eq!(trim.size, (4, 4));
	}

	#[test]
	fn background_distance_path_includes_subtly_antialiased_edge_pixels() {
		// A ring blended just 3 units off background color simulates an
		// antialiased edge over a solid background_color. The old threshold
		// of 8 misclassified this as background, cropping a visible sliver.
		let background = Rgba([0, 0, 0, 255]);
		let mut image = make_image(10, 10, background);
		set_rect(&mut image, 3, 3, 4, 4, Rgba([3, 3, 3, 255]));
		set_rect(&mut image, 4, 4, 2, 2, Rgba([255, 255, 255, 255]));

		let cells = compute_uniform_grid_trims(&image, 10, 10, 0, 0, [0.0, 0.0, 0.0]);
		let trim = cells[0].trim.expect("expected a trim rect");
		assert_eq!(trim.offset, (3, 3));
		assert_eq!(trim.size, (4, 4));
	}

	#[test]
	fn grid_origin_offset_shifts_cell_grid_without_dropping_content() {
		// 12x8 image; a 2px margin before the first cell, then a 2x1 grid of
		// 5x6 cells. Without the origin offset, cell math would start reading
		// from (0,0) and misalign every cell relative to the actual content.
		let mut image = make_image(12, 8, Rgba([0, 0, 0, 0]));
		set_rect(&mut image, 2, 2, 5, 6, Rgba([255, 0, 0, 255])); // cell 0's content
		set_rect(&mut image, 7, 2, 5, 6, Rgba([0, 255, 0, 255])); // cell 1's content

		let cells = compute_uniform_grid_trims(&image, 5, 6, 2, 2, [0.0, 0.0, 0.0]);
		assert_eq!(cells.len(), 2);
		let trim0 = cells[0].trim.expect("cell 0 should have content");
		assert_eq!(trim0.offset, (0, 0));
		assert_eq!(trim0.size, (5, 6));
		let trim1 = cells[1].trim.expect("cell 1 should have content");
		assert_eq!(trim1.offset, (0, 0));
		assert_eq!(trim1.size, (5, 6));
	}

	#[test]
	fn cell_bigger_than_image_yields_one_clamped_cell_instead_of_none() {
		// A single 6x6 opaque image with a cell size (8x8) that doesn't fully
		// fit -- floor division alone (6 / 8 == 0) would previously drop the
		// grid's only cell entirely instead of clamping it to what's available.
		let mut image = make_image(6, 6, Rgba([0, 0, 0, 0]));
		set_rect(&mut image, 1, 1, 3, 3, Rgba([255, 255, 255, 255]));

		let cells = compute_uniform_grid_trims(&image, 8, 8, 0, 0, [0.0, 0.0, 0.0]);
		assert_eq!(cells.len(), 1, "the one cell that fits (clamped) must not be dropped");
		let trim = cells[0].trim.expect("expected a trim rect");
		assert_eq!(trim.offset, (1, 1));
		assert_eq!(trim.size, (3, 3));
		assert_eq!(
			trim.original_size,
			(6, 6),
			"clamped cell's original_size should reflect its actual footprint"
		);
	}

	#[test]
	fn cell_size_exceeding_only_one_axis_still_clamps_that_axis_only() {
		// 20x6 image, cell size 8x8: cols = 20/8 = 2 (floor, unaffected), but
		// rows = 6/8 floors to 0 without the clamp -- both cells in the single
		// row must still appear, clamped to the image's actual height.
		let image = make_image(20, 6, Rgba([10, 10, 10, 255]));
		let (cols, rows) = uniform_grid_cols_rows(image.width(), image.height(), 8, 8, 0, 0);
		assert_eq!((cols, rows), (2, 1));

		let cells = compute_uniform_grid_trims(&image, 8, 8, 0, 0, [10.0 / 255.0, 10.0 / 255.0, 10.0 / 255.0]);
		assert_eq!(cells.len(), 2);
	}

	#[test]
	fn preserving_recompute_keeps_manual_trim_but_refreshes_auto_ones() {
		// 2x1 grid of 8x8 cells; cell 1 gets a hand-adjusted trim that
		// deliberately does not match what auto-detection would produce.
		let mut image = make_image(16, 8, Rgba([0, 0, 0, 0]));
		set_rect(&mut image, 0, 0, 8, 8, Rgba([255, 0, 0, 255]));
		set_rect(&mut image, 8, 0, 8, 8, Rgba([0, 255, 0, 255]));

		let existing = vec![
			GridCell { trim: None },
			GridCell {
				trim: Some(CellTrim {
					offset: (1, 1),
					size: (2, 2),
					original_size: (8, 8),
					manual: true,
				}),
			},
		];

		let cells = recompute_uniform_grid_trims_preserving_manual(&image, 8, 8, 0, 0, [0.0, 0.0, 0.0], &existing);

		// Cell 0 had no manual override, so it's freshly auto-detected.
		let cell0_trim = cells[0].trim.expect("cell 0 should auto-detect a trim");
		assert_eq!(cell0_trim.offset, (0, 0));
		assert!(!cell0_trim.manual);

		// Cell 1's manual override survives untouched.
		let cell1_trim = cells[1].trim.expect("cell 1 should keep its manual trim");
		assert_eq!(cell1_trim.offset, (1, 1));
		assert_eq!(cell1_trim.size, (2, 2));
		assert!(cell1_trim.manual);
	}

	#[test]
	fn preserving_recompute_drops_manual_trim_when_cell_size_changes() {
		let mut image = make_image(8, 8, Rgba([0, 0, 0, 0]));
		set_rect(&mut image, 0, 0, 8, 8, Rgba([255, 0, 0, 255]));

		let existing = vec![GridCell {
			trim: Some(CellTrim {
				offset: (1, 1),
				size: (2, 2),
				original_size: (8, 8),
				manual: true,
			}),
		}];

		// Cell size changed from 8x8 to 4x4 -- the manual trim (sized for an
		// 8x8 cell) no longer applies, so it should fall back to auto-detect.
		let cells = recompute_uniform_grid_trims_preserving_manual(&image, 4, 4, 0, 0, [0.0, 0.0, 0.0], &existing);
		let trim = cells[0].trim.expect("expected an auto-detected trim");
		assert!(!trim.manual);
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
			origin_x: 0,
			origin_y: 0,
			background_color: [1.0, 0.0, 1.0],
			cells: vec![
				GridCell { trim: None },
				GridCell {
					trim: Some(CellTrim {
						offset: (1, 2),
						size: (14, 12),
						original_size: (16, 16),
						manual: true,
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
				let trim = grid.cells[1].trim.expect("cell 1 should have a trim");
				assert_eq!(trim.offset, (1, 2));
				assert!(trim.manual);
			}
			TextureSheetMode::AutoPack(_) => panic!("expected UniformGrid mode"),
		}
	}

	#[test]
	fn deserializes_pre_manual_field_json_as_non_manual() {
		// Sheets saved before `CellTrim::manual` existed have no such key --
		// `#[serde(default)]` must still let them load, defaulting to `false`.
		let json = r#"{
			"version": "1",
			"mode": {
				"UniformGrid": {
					"texture": { "source": "Game", "path": "textures/sheet.png" },
					"cell_width": 16,
					"cell_height": 16,
					"background_color": [1.0, 0.0, 1.0],
					"cells": [
						{ "trim": { "offset": [1, 2], "size": [14, 12], "original_size": [16, 16] } }
					]
				}
			}
		}"#;

		let sheet: TextureSheet = serde_json::from_str(json).expect("deserialize pre-manual-field JSON");
		match sheet.mode {
			TextureSheetMode::UniformGrid(grid) => {
				assert!(!grid.cells[0].trim.expect("cell 0 should have a trim").manual);
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
