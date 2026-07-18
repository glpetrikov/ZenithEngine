use std::{
	path::{Path, PathBuf},
	time::SystemTime,
};

use egui::{Color32, ColorImage, TextureHandle, TextureOptions, Ui};
use ze_assets::{
	AssetRef, AutoPackSheet, CellId, CellTrim, TextureSheet, TextureSheetMode, UniformGridSheet,
	compute_uniform_grid_trims, shelf_pack_preview,
};
use ze_ecs::TileWorldSettings;

use super::{
	EditorPanelContext, EditorSelection, Panel,
	texture_sheet_common::{
		DecodedImage, ViewportTransform, draw_checkerboard, load_or_reload_decoded_image, pick_uniform_grid_cell,
		relativize_asset_path, sample_pixel,
	},
};
use crate::{tilemap, undo_redo::SceneSnapshotCommand};

/// A fixed logical layout width for the Auto-pack shelf-pack preview -- the
/// viewport now pans/zooms independently of the panel's on-screen width, so
/// this no longer needs to track `ui.available_width()`.
const AUTO_PACK_LAYOUT_WIDTH: u32 = 1024;

/// Fixed screen-space gap between adjacent cells' rendered rects (not scaled
/// by zoom, same convention as `draw_checkerboard`'s tile size) so cells read
/// as visually distinct tiles rather than a seamless image.
const CELL_GAP: f32 = 2.0;

/// The sheet + cell currently selected as the active tile-painting brush
/// (set from this panel, read by `ScenePanel`'s "Paint Tiles" tool). Only
/// `UniformGrid` cells can become a brush -- painting assumes a uniform
/// world-grid tile size, which an `AutoPack` sheet (a loose folder of
/// differently-sized images) can't provide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileBrush {
	/// The sheet's path relative to the project's asset root, matching the
	/// convention `TextureSource::SheetCell::sheet_path` (and `AssetRef::path`)
	/// already use -- not an absolute filesystem path.
	pub sheet_path: String,
	pub cell_id: CellId,
}

pub struct TextureSheetPanel {
	current_path: Option<PathBuf>,
	sheet: Option<TextureSheet>,
	dirty: bool,
	selected_cell: Option<CellId>,
	status: Option<String>,
	source_preview: Option<DecodedImage>,
	file_previews: Vec<(PathBuf, SystemTime, TextureHandle)>,
	viewport: ViewportTransform,
	viewport_fitted: bool,
	eyedropper_active: bool,
}

impl TextureSheetPanel {
	pub fn new() -> Self {
		Self {
			current_path: None,
			sheet: None,
			dirty: false,
			selected_cell: None,
			status: None,
			source_preview: None,
			file_previews: Vec::new(),
			viewport: ViewportTransform {
				zoom: 1.0,
				pan: egui::Vec2::ZERO,
			},
			viewport_fitted: false,
			eyedropper_active: false,
		}
	}
}

impl Default for TextureSheetPanel {
	fn default() -> Self { Self::new() }
}

impl Panel for TextureSheetPanel {
	fn name(&self) -> &'static str { "Texture Sheet" }

	// The dock framework wraps every tab's content in its own `ScrollArea` by
	// default (see `Panel::scroll_bars`'s default). This panel already wraps
	// its own content in a single `egui::ScrollArea::vertical()` below, so
	// leaving the default on produced two nested, independently-scrolling
	// regions -- disable the dock's own wrapper so there's exactly one.
	fn scroll_bars(&self) -> [bool; 2] { [false, false] }

	fn show(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>) {
		self.sync_from_selection(context);

		let assets_root = context.project.map(|project| project.asset_dir.clone());

		egui::ScrollArea::vertical().show(ui, |ui| {
			self.show_header(ui);

			let Some(assets_root) = assets_root else {
				ui.label("No project loaded.");
				return;
			};

			ui.separator();
			show_tilemap_settings(ui, context);
			ui.separator();

			let Some(sheet) = self.sheet.as_mut() else {
				ui.label("Select or create a .texturesheet.json asset to edit it here.");
				return;
			};

			// Precomputed once per frame so a cell click can set the active
			// brush immediately (see `show_uniform_grid_viewport_content`)
			// without re-deriving this on every click.
			let sheet_path = self
				.current_path
				.as_deref()
				.and_then(|path| relativize_asset_path(&assets_root, path));

			let viewport_changed = show_texture_sheet_viewport(
				ui,
				sheet,
				&assets_root,
				sheet_path.as_deref(),
				context.active_tile_brush,
				&mut self.source_preview,
				&mut self.file_previews,
				&mut self.viewport,
				&mut self.viewport_fitted,
				&mut self.selected_cell,
				&mut self.eyedropper_active,
			);
			if viewport_changed {
				self.dirty = true;
			}

			ui.separator();

			match &mut sheet.mode {
				TextureSheetMode::UniformGrid(grid) => {
					let (texture_changed, grid_changed) =
						show_uniform_grid_settings(ui, grid, &mut self.eyedropper_active);
					if texture_changed || grid_changed {
						self.dirty = true;
						self.source_preview = None;
					}
					if texture_changed {
						self.viewport_fitted = false;
					}

					ui.separator();
					show_uniform_grid_cell_inspector(ui, grid, self.selected_cell);
				}
				TextureSheetMode::AutoPack(pack) => {
					ui.label("Auto-pack sheets are a browsing convenience only -- selecting a cell here is");
					ui.label("identical to using that file directly. There is no real atlas/compositing, and");
					ui.label("auto-pack sheets cannot be used for tile painting (no uniform tile size).");
					ui.separator();

					if show_auto_pack_settings(ui, pack, &assets_root) {
						self.dirty = true;
					}

					ui.separator();
					show_auto_pack_cell_inspector(ui, pack, self.selected_cell);
				}
			}
		});

		self.autosave_if_settled(ui.ctx());
	}
}

impl TextureSheetPanel {
	fn sync_from_selection(&mut self, context: &EditorPanelContext<'_>) {
		let Some(EditorSelection::Asset(path)) = context.selection.as_ref() else {
			return;
		};
		if path.extension().and_then(|extension| extension.to_str()) != Some("json")
			|| !path
				.file_name()
				.and_then(|name| name.to_str())
				.is_some_and(|name| name.ends_with(&format!(".{}", ze_assets::TEXTURE_SHEET_EXTENSION)))
		{
			return;
		}
		if self.current_path.as_deref() == Some(path.as_path()) {
			return;
		}

		match TextureSheet::load(path) {
			Ok(sheet) => {
				self.sheet = Some(sheet);
				self.current_path = Some(path.clone());
				self.dirty = false;
				self.selected_cell = None;
				self.source_preview = None;
				self.file_previews.clear();
				self.viewport_fitted = false;
				self.eyedropper_active = false;
				self.status = None;
			}
			Err(error) => {
				self.status = Some(format!("Failed to load texture sheet: {error}"));
			}
		}
	}

	fn show_header(&self, ui: &mut Ui) {
		let label = self
			.current_path
			.as_ref()
			.and_then(|path| path.file_name())
			.and_then(|name| name.to_str())
			.unwrap_or("No sheet open");
		ui.label(egui::RichText::new(label).strong());
		if let Some(status) = &self.status {
			ui.colored_label(Color32::YELLOW, status);
		}
	}

	/// Writes the sheet to disk once a pending edit has "settled" -- nothing
	/// is currently being dragged (a `DragValue`/color-picker slider) or
	/// holding keyboard focus (a `DragValue` in type-to-edit mode, or the
	/// source-texture text field) anywhere in the whole panel. This fires
	/// exactly once right after the user finishes an interaction (drag
	/// release, tabbing away, a mode-combo/button click that never involved
	/// dragging in the first place) rather than on every intermediate frame
	/// of a still-in-progress drag or keystroke -- mirrors the "commit only
	/// once a change is finished" intent of `show_tilemap_settings`'s
	/// on-change trigger, generalized across widget types via this
	/// context-wide idle check instead of a single widget's own `.changed()`.
	fn autosave_if_settled(&mut self, ctx: &egui::Context) {
		if !self.dirty {
			return;
		}
		let settled = ctx.dragged_id().is_none() && ctx.memory(|memory| memory.focused().is_none());
		if !settled {
			return;
		}
		let Some(path) = self.current_path.clone() else {
			return;
		};
		let Some(sheet) = &self.sheet else {
			return;
		};

		match sheet.save(&path) {
			Ok(()) => {
				self.dirty = false;
				self.status = None;
			}
			Err(error) => {
				self.status = Some(format!("Failed to save texture sheet: {error}"));
			}
		}
	}
}

/// The global tile world-size control (per-scene, per the spec's "single
/// float on a scene-level settings entity is fine for the jam" note). On
/// change, updates the persisted `TileWorldSettings` and rescales every
/// painted tile in one before/after scene-snapshot undo entry -- a
/// straightforward on-change trigger, not a per-frame system.
fn show_tilemap_settings(ui: &mut Ui, context: &mut EditorPanelContext<'_>) {
	ui.label(egui::RichText::new("Tilemap").strong());

	let current = tilemap::tile_world_size(context.scene);
	let mut new_value = current;
	ui.horizontal(|ui| {
		ui.label("Tile world size");
		let changed = ui
			.add(egui::DragValue::new(&mut new_value).speed(0.05).range(0.01..=1000.0))
			.changed();

		if changed && new_value > 0.0 {
			let before = context.scene.snapshot();
			let entity = tilemap::ensure_tile_world_settings(context.scene);
			if let Ok(mut settings) = context.scene.world_mut().get::<&mut TileWorldSettings>(entity) {
				settings.tile_world_size = new_value;
			}
			tilemap::rescale_tiles(context.scene, new_value);
			let after = context.scene.snapshot();
			context
				.undo_redo
				.record_executed(Box::new(SceneSnapshotCommand::new(before, after)));
		}
	});
}

/// Allocates the pannable/zoomable viewport, draws its mode-specific content
/// (source image + grid outlines, or Auto-pack thumbnails) inside a
/// checkerboard background, and overlays the mode `ComboBox` on top. Returns
/// whether anything happened that should mark the sheet dirty (an eyedropper
/// pick, or a mode switch).
#[allow(clippy::too_many_arguments)]
fn show_texture_sheet_viewport(
	ui: &mut Ui,
	sheet: &mut TextureSheet,
	assets_root: &Path,
	sheet_path: Option<&str>,
	active_tile_brush: &mut Option<TileBrush>,
	source_preview: &mut Option<DecodedImage>,
	file_previews: &mut Vec<(PathBuf, SystemTime, TextureHandle)>,
	viewport: &mut ViewportTransform,
	viewport_fitted: &mut bool,
	selected_cell: &mut Option<CellId>,
	eyedropper_active: &mut bool,
) -> bool {
	let viewport_height = (ui.available_height() * 0.55).clamp(240.0, 600.0);
	let (response, painter) = ui.allocate_painter(
		egui::vec2(ui.available_width(), viewport_height),
		egui::Sense::click_and_drag(),
	);
	let viewport_rect = response.rect;

	draw_checkerboard(&painter, viewport_rect, 16.0);

	let mut changed = false;
	match &mut sheet.mode {
		TextureSheetMode::UniformGrid(grid) => {
			changed |= show_uniform_grid_viewport_content(
				ui,
				&painter,
				&response,
				viewport_rect,
				grid,
				assets_root,
				sheet_path,
				active_tile_brush,
				source_preview,
				viewport,
				viewport_fitted,
				selected_cell,
				eyedropper_active,
			);
		}
		TextureSheetMode::AutoPack(pack) => {
			show_auto_pack_viewport_content(
				ui,
				&painter,
				&response,
				viewport_rect,
				pack,
				assets_root,
				file_previews,
				viewport,
				viewport_fitted,
				selected_cell,
			);
		}
	}

	changed |= show_mode_overlay(
		ui,
		viewport_rect,
		sheet,
		source_preview,
		file_previews,
		viewport_fitted,
		selected_cell,
	);

	show_active_brush_indicator(ui, viewport_rect, sheet_path, active_tile_brush.as_ref());

	changed
}

/// A small read-only overlay in the viewport's top-right corner showing which
/// cell (if any) of *this* sheet is currently the active paint brush -- lets
/// the user confirm/switch brushes by clicking cells without ever needing to
/// scroll down to the (now-removed) separate confirm button.
fn show_active_brush_indicator(
	ui: &mut Ui,
	viewport_rect: egui::Rect,
	sheet_path: Option<&str>,
	active_tile_brush: Option<&TileBrush>,
) {
	let Some(sheet_path) = sheet_path else {
		return;
	};
	let Some(brush) = active_tile_brush else {
		return;
	};
	if brush.sheet_path != sheet_path {
		return;
	}

	let overlay_rect = egui::Rect::from_min_size(
		egui::pos2(viewport_rect.max.x - 168.0, viewport_rect.min.y + 8.0),
		egui::vec2(160.0, 20.0),
	);
	let mut overlay_ui = ui.new_child(
		egui::UiBuilder::new()
			.max_rect(overlay_rect)
			.layout(egui::Layout::right_to_left(egui::Align::Min)),
	);
	egui::Frame::popup(overlay_ui.style()).show(&mut overlay_ui, |ui| {
		ui.label(format!("Painting: Cell #{}", brush.cell_id.0));
	});
}

#[allow(clippy::too_many_arguments)]
fn show_uniform_grid_viewport_content(
	ui: &Ui,
	painter: &egui::Painter,
	response: &egui::Response,
	viewport_rect: egui::Rect,
	grid: &mut UniformGridSheet,
	assets_root: &Path,
	sheet_path: Option<&str>,
	active_tile_brush: &mut Option<TileBrush>,
	source_preview: &mut Option<DecodedImage>,
	viewport: &mut ViewportTransform,
	viewport_fitted: &mut bool,
	selected_cell: &mut Option<CellId>,
	eyedropper_active: &mut bool,
) -> bool {
	if grid.texture.path.is_empty() {
		painter.text(
			viewport_rect.center(),
			egui::Align2::CENTER_CENTER,
			"No source texture set.",
			egui::FontId::default(),
			Color32::from_gray(160),
		);
		return false;
	}

	let source_path = assets_root.join(&grid.texture.path);
	let Some((preview, reloaded)) = load_or_reload_decoded_image(ui.ctx(), &source_path, source_preview) else {
		painter.text(
			viewport_rect.center(),
			egui::Align2::CENTER_CENTER,
			format!("Source texture not found: {}", source_path.display()),
			egui::FontId::default(),
			Color32::from_gray(160),
		);
		return false;
	};

	if reloaded {
		// Trim recomputation is edit-time-only and happens here (once per
		// source-texture/cell-size/background-color change), never at
		// runtime load -- see `compute_uniform_grid_trims`.
		grid.cells =
			compute_uniform_grid_trims(&preview.rgba, grid.cell_width, grid.cell_height, grid.background_color);
	}

	let image_size = egui::vec2(preview.rgba.width() as f32, preview.rgba.height() as f32);
	if !*viewport_fitted {
		*viewport = ViewportTransform::fit(image_size, viewport_rect);
		*viewport_fitted = true;
	}
	viewport.apply_input(ui, response, viewport_rect);

	let dest_min = viewport.image_to_screen(viewport_rect, egui::pos2(0.0, 0.0));
	let dest_max = viewport.image_to_screen(viewport_rect, image_size.to_pos2());
	painter.image(
		preview.texture.id(),
		egui::Rect::from_min_max(dest_min, dest_max),
		egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
		Color32::WHITE,
	);

	let cell_width = grid.cell_width.max(1);
	let cell_height = grid.cell_height.max(1);
	let cols = (preview.rgba.width() / cell_width).max(1);
	let rows = (preview.rgba.height() / cell_height).max(1);

	for (index, cell) in grid.cells.iter().enumerate() {
		let col = index as u32 % cols;
		let row = index as u32 / cols;
		let cell_min = viewport.image_to_screen(
			viewport_rect,
			egui::pos2((col * cell_width) as f32, (row * cell_height) as f32),
		);
		let cell_max = viewport.image_to_screen(
			viewport_rect,
			egui::pos2(((col + 1) * cell_width) as f32, ((row + 1) * cell_height) as f32),
		);
		painter.rect_stroke(
			egui::Rect::from_min_max(cell_min, cell_max).shrink(CELL_GAP),
			0.0,
			(1.0, Color32::from_gray(90)),
			egui::StrokeKind::Inside,
		);

		let Some(trim) = cell.trim else {
			continue;
		};
		let trim_min = viewport.image_to_screen(
			viewport_rect,
			egui::pos2(
				(col * cell_width + trim.offset.0) as f32,
				(row * cell_height + trim.offset.1) as f32,
			),
		);
		let trim_max = viewport.image_to_screen(
			viewport_rect,
			egui::pos2(
				(col * cell_width + trim.offset.0 + trim.size.0) as f32,
				(row * cell_height + trim.offset.1 + trim.size.1) as f32,
			),
		);
		let is_selected = *selected_cell == Some(CellId(index as u32));
		let stroke_color = if is_selected { Color32::YELLOW } else { Color32::GREEN };
		painter.rect_stroke(
			egui::Rect::from_min_max(trim_min, trim_max).shrink(CELL_GAP),
			0.0,
			(2.0, stroke_color),
			egui::StrokeKind::Inside,
		);
	}

	let mut changed = false;
	if response.clicked()
		&& let Some(pointer) = response.interact_pointer_pos()
	{
		let image_point = viewport.screen_to_image(viewport_rect, pointer);
		if *eyedropper_active {
			if let Some(color) = sample_pixel(&preview.rgba, image_point) {
				grid.background_color = color;
				grid.cells = compute_uniform_grid_trims(&preview.rgba, grid.cell_width, grid.cell_height, color);
				*eyedropper_active = false;
				changed = true;
			}
		} else if let Some(index) = pick_uniform_grid_cell(image_point, cell_width, cell_height, cols, rows) {
			*selected_cell = Some(CellId(index));
			// Selecting a paintable cell immediately becomes the active
			// brush -- no separate confirm step, so switching brushes mid
			// painting-session never requires scrolling down to a button.
			if grid.cells.get(index as usize).is_some_and(|cell| cell.trim.is_some())
				&& let Some(sheet_path) = sheet_path
			{
				*active_tile_brush = Some(TileBrush {
					sheet_path: sheet_path.to_string(),
					cell_id: CellId(index),
				});
			}
		}
	}

	changed
}

#[allow(clippy::too_many_arguments)]
fn show_auto_pack_viewport_content(
	ui: &Ui,
	painter: &egui::Painter,
	response: &egui::Response,
	viewport_rect: egui::Rect,
	pack: &AutoPackSheet,
	assets_root: &Path,
	file_previews: &mut Vec<(PathBuf, SystemTime, TextureHandle)>,
	viewport: &mut ViewportTransform,
	viewport_fitted: &mut bool,
	selected_cell: &mut Option<CellId>,
) {
	if pack.files.is_empty() {
		painter.text(
			viewport_rect.center(),
			egui::Align2::CENTER_CENTER,
			"No files in this palette yet.",
			egui::FontId::default(),
			Color32::from_gray(160),
		);
		return;
	}

	let dimensions: Vec<(u32, u32)> = pack
		.files
		.iter()
		.map(|file| image::image_dimensions(assets_root.join(&file.path)).unwrap_or((1, 1)))
		.collect();
	let packed = shelf_pack_preview(&dimensions, AUTO_PACK_LAYOUT_WIDTH);
	let content_size = egui::vec2(
		AUTO_PACK_LAYOUT_WIDTH as f32,
		packed.iter().map(|rect| rect.y + rect.height).max().unwrap_or(0) as f32,
	);

	if !*viewport_fitted {
		*viewport = ViewportTransform::fit(content_size, viewport_rect);
		*viewport_fitted = true;
	}
	viewport.apply_input(ui, response, viewport_rect);

	for placed in &packed {
		let file = &pack.files[placed.file_index];
		let min = viewport.image_to_screen(viewport_rect, egui::pos2(placed.x as f32, placed.y as f32));
		let max = viewport.image_to_screen(
			viewport_rect,
			egui::pos2((placed.x + placed.width) as f32, (placed.y + placed.height) as f32),
		);
		let rect = egui::Rect::from_min_max(min, max).shrink(CELL_GAP);

		if let Some(texture) = load_or_get_cached_texture(ui, &assets_root.join(&file.path), file_previews) {
			painter.image(
				texture.id(),
				rect,
				egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
				Color32::WHITE,
			);
		}

		let is_selected = *selected_cell == Some(CellId(placed.file_index as u32));
		let stroke_color = if is_selected {
			Color32::YELLOW
		} else {
			Color32::from_gray(120)
		};
		painter.rect_stroke(rect, 0.0, (2.0, stroke_color), egui::StrokeKind::Inside);
	}

	if response.clicked()
		&& let Some(pointer) = response.interact_pointer_pos()
	{
		let image_point = viewport.screen_to_image(viewport_rect, pointer);
		let clicked = packed.iter().find(|placed| {
			let min = egui::pos2(placed.x as f32, placed.y as f32);
			let max = egui::pos2((placed.x + placed.width) as f32, (placed.y + placed.height) as f32);
			egui::Rect::from_min_max(min, max).contains(image_point)
		});
		if let Some(placed) = clicked {
			*selected_cell = Some(CellId(placed.file_index as u32));
		}
	}
}

/// The mode `ComboBox`, drawn as a floating overlay in the viewport's
/// top-left corner (a viewport control, like a 3D viewport's shading-mode
/// dropdown, rather than a page-level settings row). Actually switching mode
/// converts the sheet in place to a fresh default of the newly selected mode
/// -- there's no meaningful way to preserve a Uniform Grid's cells when
/// switching to Auto-pack's file-list shape or vice versa.
#[allow(clippy::too_many_arguments)]
fn show_mode_overlay(
	ui: &mut Ui,
	viewport_rect: egui::Rect,
	sheet: &mut TextureSheet,
	source_preview: &mut Option<DecodedImage>,
	file_previews: &mut Vec<(PathBuf, SystemTime, TextureHandle)>,
	viewport_fitted: &mut bool,
	selected_cell: &mut Option<CellId>,
) -> bool {
	let mut changed = false;
	let current_label = match &sheet.mode {
		TextureSheetMode::UniformGrid(_) => "Uniform Grid",
		TextureSheetMode::AutoPack(_) => "Auto-pack",
	};

	// A `child_ui` (not an `egui::Area`) so this stays inside the panel's own
	// scroll-clipped layer: an `Area` is a separate top-level layer that
	// ignores the parent `ScrollArea`'s clip rect, so anchoring one to
	// `viewport_rect` (which moves as the panel scrolls) made the overlay
	// drift away from the viewport instead of scrolling together with it.
	let overlay_rect = egui::Rect::from_min_size(viewport_rect.min + egui::vec2(8.0, 8.0), egui::vec2(160.0, 20.0));
	let mut overlay_ui = ui.new_child(
		egui::UiBuilder::new()
			.max_rect(overlay_rect)
			.layout(egui::Layout::left_to_right(egui::Align::Min)),
	);
	egui::Frame::popup(overlay_ui.style()).show(&mut overlay_ui, |ui| {
		egui::ComboBox::from_id_salt("texture_sheet_mode_combo")
			.selected_text(current_label)
			.show_ui(ui, |ui| {
				let is_grid = matches!(sheet.mode, TextureSheetMode::UniformGrid(_));
				if ui.selectable_label(is_grid, "Uniform Grid").clicked() && !is_grid {
					sheet.mode = TextureSheetMode::UniformGrid(UniformGridSheet {
						texture: AssetRef::game(String::new()),
						cell_width: 32,
						cell_height: 32,
						background_color: [1.0, 0.0, 1.0],
						cells: Vec::new(),
					});
					*source_preview = None;
					*selected_cell = None;
					*viewport_fitted = false;
					changed = true;
				}
				if ui.selectable_label(!is_grid, "Auto-pack").clicked() && is_grid {
					sheet.mode = TextureSheetMode::AutoPack(AutoPackSheet { files: Vec::new() });
					file_previews.clear();
					*selected_cell = None;
					*viewport_fitted = false;
					changed = true;
				}
			});
	});

	changed
}

/// Returns `(texture_changed, grid_changed)` -- kept distinct so the caller
/// only re-fits the viewport (resetting pan/zoom) when the source texture
/// itself changes, not on routine cell-size/background-color edits.
fn show_uniform_grid_settings(ui: &mut Ui, grid: &mut UniformGridSheet, eyedropper_active: &mut bool) -> (bool, bool) {
	let mut texture_changed = false;
	let mut grid_changed = false;

	ui.horizontal(|ui| {
		ui.label("Source texture");
		let mut texture_path = grid.texture.path.clone();
		if ui.text_edit_singleline(&mut texture_path).changed() {
			grid.texture = AssetRef::game(texture_path);
			texture_changed = true;
		}
	});

	ui.horizontal(|ui| {
		ui.label("Cell width");
		let mut width = grid.cell_width;
		if ui.add(egui::DragValue::new(&mut width).range(1..=8192)).changed() {
			grid.cell_width = width;
			grid_changed = true;
		}
		ui.label("Cell height");
		let mut height = grid.cell_height;
		if ui.add(egui::DragValue::new(&mut height).range(1..=8192)).changed() {
			grid.cell_height = height;
			grid_changed = true;
		}
	});

	ui.horizontal(|ui| {
		ui.label("Background color");
		if ui.color_edit_button_rgb(&mut grid.background_color).changed() {
			grid_changed = true;
		}
		ui.toggle_value(eyedropper_active, "Eyedropper");
	});

	(texture_changed, grid_changed)
}

fn show_uniform_grid_cell_inspector(ui: &mut Ui, grid: &UniformGridSheet, selected_cell: Option<CellId>) {
	let Some(cell_id) = selected_cell else {
		ui.label("No cell selected.");
		return;
	};
	let Some(cell) = grid.cells.get(cell_id.0 as usize) else {
		ui.label("Selected cell is out of range for the current grid.");
		return;
	};

	ui.label(format!("Cell #{}", cell_id.0));
	match cell.trim {
		Some(CellTrim {
			offset,
			size,
			original_size,
		}) => {
			ui.label(format!("Trim offset: {offset:?}"));
			ui.label(format!("Trim size: {size:?}"));
			ui.label(format!("Untrimmed cell size: {original_size:?}"));
		}
		None => {
			ui.label("Empty cell (background/transparent) -- not placeable.");
		}
	}
}

fn show_auto_pack_settings(ui: &mut Ui, pack: &mut AutoPackSheet, assets_root: &Path) -> bool {
	let mut changed = false;

	if ui.button("Add File...").clicked()
		&& let Some(picked) = rfd::FileDialog::new()
			.add_filter("Images", &["png", "jpg", "jpeg", "webp"])
			.set_directory(assets_root)
			.pick_file()
		&& let Some(relative) = relativize_asset_path(assets_root, &picked)
	{
		pack.files.push(AssetRef::game(relative));
		changed = true;
	}

	let mut remove_index = None;
	for (index, file) in pack.files.iter().enumerate() {
		ui.horizontal(|ui| {
			ui.label(format!("{index}: {}", file.path));
			if ui.button("Remove").clicked() {
				remove_index = Some(index);
			}
		});
	}
	if let Some(index) = remove_index {
		pack.files.remove(index);
		changed = true;
	}

	changed
}

fn show_auto_pack_cell_inspector(ui: &mut Ui, pack: &AutoPackSheet, selected_cell: Option<CellId>) {
	let Some(cell_id) = selected_cell else {
		ui.label("No file selected.");
		return;
	};
	let Some(file) = pack.files.get(cell_id.0 as usize) else {
		ui.label("Selected file is out of range for the current palette.");
		return;
	};

	ui.label(format!("File #{}: {}", cell_id.0, file.path));
}

fn load_or_get_cached_texture<'a>(
	ui: &Ui,
	path: &Path,
	cache: &'a mut Vec<(PathBuf, SystemTime, TextureHandle)>,
) -> Option<&'a TextureHandle> {
	let modified = std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok()?;

	let needs_load = cache
		.iter()
		.position(|(cached_path, cached_time, _)| cached_path == path && *cached_time == modified)
		.is_none();

	if needs_load {
		cache.retain(|(cached_path, ..)| cached_path != path);
		let image = image::open(path).ok()?;
		let rgba = image.to_rgba8();
		let (width, height) = rgba.dimensions();
		let color_image = ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
		let texture = ui
			.ctx()
			.load_texture(path.to_string_lossy(), color_image, TextureOptions::LINEAR);
		cache.push((path.to_path_buf(), modified, texture));
	}

	cache
		.iter()
		.find(|(cached_path, ..)| cached_path == path)
		.map(|(_, _, texture)| texture)
}
