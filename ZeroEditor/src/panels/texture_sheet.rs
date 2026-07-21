use std::{
	path::{Path, PathBuf},
	time::SystemTime,
};

use egui::{Color32, ColorImage, TextureHandle, TextureOptions, Ui};
use ze_assets::{
	AnimationClip, AssetRef, AutoPackSheet, CellId, CellTrim, TextureSheet, TextureSheetMode, UniformGridSheet,
	compute_uniform_grid_trims, recompute_uniform_grid_trims_preserving_manual, shelf_pack_preview,
};
use ze_ecs::TileWorldSettings;

use super::{
	EditorPanelContext, EditorSelection, Panel,
	texture_sheet_common::{
		DecodedImage, EYEDROPPER_TRANSPARENT_ALPHA_THRESHOLD, ViewportTransform, draw_checkerboard,
		load_or_reload_decoded_image, pick_uniform_grid_cell, relativize_asset_path, sample_pixel,
		scan_assets_with_extension, show_viewport_zoom_toolbar,
	},
};
use crate::{editor_workspace::EditorRequest, tilemap, undo_redo::SceneSnapshotCommand};

/// A fixed logical layout width for the Auto-pack shelf-pack preview -- the
/// viewport now pans/zooms independently of the panel's on-screen width, so
/// this no longer needs to track `ui.available_width()`.
const AUTO_PACK_LAYOUT_WIDTH: u32 = 1024;

/// Fixed screen-space gap between adjacent cells' rendered rects (not scaled
/// by zoom, same convention as `draw_checkerboard`'s tile size) so cells read
/// as visually distinct tiles rather than a seamless image.
const CELL_GAP: f32 = 2.0;

/// Strips the trailing `.texturesheet.json` suffix (and any directory
/// components) off a project-relative asset path, for a readable sheet-picker
/// label -- `file_stem` alone would only strip `.json`, leaving
/// `.texturesheet`.
fn display_name_for_relative_path(relative: &str) -> &str {
	let file_name = Path::new(relative)
		.file_name()
		.and_then(|name| name.to_str())
		.unwrap_or(relative);
	file_name
		.strip_suffix(&format!(".{}", ze_assets::TEXTURE_SHEET_EXTENSION))
		.unwrap_or(file_name)
}

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
	/// The cell a right-click context menu was opened on -- persisted (rather
	/// than recomputed from the current hover position) because the menu's
	/// contents are redrawn every frame it stays open, including frames after
	/// the pointer has moved off that cell.
	context_menu_cell: Option<CellId>,
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
			context_menu_cell: None,
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
			let Some(assets_root) = assets_root else {
				self.show_header(ui);
				ui.label("No project loaded.");
				return;
			};

			self.show_sheet_picker(ui, context.selection, &assets_root);
			self.show_header(ui);

			ui.separator();
			show_tilemap_settings(ui, context);
			ui.separator();

			let Some(sheet) = self.sheet.as_mut() else {
				ui.label("Pick a texture sheet above, or create one from the File Explorer's");
				ui.label("\"Create Texture Sheet\" option, to edit it here.");
				return;
			};

			// Precomputed once per frame so a cell click can set the active
			// brush immediately (see `show_uniform_grid_viewport_content`)
			// without re-deriving this on every click.
			let sheet_path = self
				.current_path
				.as_deref()
				.and_then(|path| relativize_asset_path(&assets_root, path));

			let mut open_animation_clip = None;
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
				&mut self.context_menu_cell,
				&mut self.status,
				&mut open_animation_clip,
			);
			if viewport_changed {
				self.dirty = true;
			}
			if let Some(dest) = open_animation_clip {
				*context.selection = Some(EditorSelection::Asset(dest.clone()));
				*context.editor_request = Some(EditorRequest::OpenAnimationClipAsset(dest));
			}

			ui.separator();

			match &mut sheet.mode {
				TextureSheetMode::UniformGrid(grid) => {
					let (texture_changed, grid_changed) =
						show_uniform_grid_settings(ui, grid, &mut self.eyedropper_active, self.source_preview.as_ref());
					if texture_changed || grid_changed {
						self.dirty = true;
						self.source_preview = None;
					}
					if texture_changed {
						self.viewport_fitted = false;
					}

					ui.separator();
					if show_uniform_grid_cell_inspector(ui, grid, self.selected_cell, self.source_preview.as_ref()) {
						self.dirty = true;
					}
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

		self.open_sheet(path.clone());
	}

	/// Loads `path` into the panel, replacing whatever sheet (if any) is
	/// currently open. Shared by both the selection-driven flow (double-click
	/// in the File Explorer, or the "Create Texture Sheet" dialog routing
	/// through `EditorRequest::OpenTextureSheetAsset`) and the in-panel sheet
	/// picker, so both paths reset the same editing state.
	fn open_sheet(&mut self, path: PathBuf) {
		match TextureSheet::load(&path) {
			Ok(sheet) => {
				self.sheet = Some(sheet);
				self.current_path = Some(path);
				self.dirty = false;
				self.selected_cell = None;
				self.source_preview = None;
				self.file_previews.clear();
				self.viewport_fitted = false;
				self.eyedropper_active = false;
				self.context_menu_cell = None;
				self.status = None;
			}
			Err(error) => {
				self.status = Some(format!("Failed to load texture sheet: {error}"));
			}
		}
	}

	/// A combo box listing every `.texturesheet.json` asset in the project,
	/// letting the user open one directly instead of hunting for it in the
	/// File Explorer. Options are rescanned from disk every time this runs
	/// (cheap -- it's an occasional directory walk, not a hot path), which is
	/// also how the list notices a sheet that was deleted/moved externally.
	fn show_sheet_picker(&mut self, ui: &mut Ui, selection: &mut Option<EditorSelection>, assets_root: &Path) {
		let options = scan_assets_with_extension(assets_root, ze_assets::TEXTURE_SHEET_EXTENSION);
		let current_relative = self
			.current_path
			.as_deref()
			.and_then(|path| relativize_asset_path(assets_root, path));

		ui.horizontal(|ui| {
			ui.label("Texture Sheet:");
			let selected_text = current_relative
				.as_deref()
				.map_or("(pick a sheet)", display_name_for_relative_path);

			egui::ComboBox::from_id_salt("texture_sheet_picker")
				.selected_text(selected_text)
				.show_ui(ui, |ui| {
					if options.is_empty() {
						ui.label("No texture sheets in this project yet.");
					}
					for option in &options {
						let display = display_name_for_relative_path(option);
						let is_current = current_relative.as_deref() == Some(option.as_str());
						if ui.selectable_label(is_current, display).clicked() && !is_current {
							let path = assets_root.join(option);
							*selection = Some(EditorSelection::Asset(path.clone()));
							self.open_sheet(path);
						}
					}
				});
		});
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
	context_menu_cell: &mut Option<CellId>,
	status: &mut Option<String>,
	open_animation_clip: &mut Option<PathBuf>,
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
				context_menu_cell,
				status,
				open_animation_clip,
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
	show_viewport_zoom_toolbar(ui, viewport_rect, viewport, viewport_fitted, true);

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

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
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
	context_menu_cell: &mut Option<CellId>,
	status: &mut Option<String>,
	open_animation_clip: &mut Option<PathBuf>,
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
		// source-texture/cell-size/origin/background-color change), never at
		// runtime load -- see `compute_uniform_grid_trims`. Uses the
		// manual-preserving variant so a hand-adjusted trim (see
		// `show_uniform_grid_cell_inspector`) survives a texture-file reload.
		grid.cells = recompute_uniform_grid_trims_preserving_manual(
			&preview.rgba,
			grid.cell_width,
			grid.cell_height,
			grid.origin_x,
			grid.origin_y,
			grid.background_color,
			&grid.cells,
		);
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
	let origin_x = grid.origin_x;
	let origin_y = grid.origin_y;
	let (cols, rows) = grid.grid_dimensions(preview.rgba.width(), preview.rgba.height());

	// Precomputed once per frame (rather than re-deriving inside the click
	// handler below) so both the hover-highlight drawing and the right-click
	// context-menu-target logic can share the same "which cell is the
	// pointer over right now" answer.
	let hovered_cell_index = response
		.hover_pos()
		.map(|pointer| viewport.screen_to_image(viewport_rect, pointer))
		.and_then(|image_point| {
			pick_uniform_grid_cell(image_point, cell_width, cell_height, origin_x, origin_y, cols, rows)
		});

	for (index, cell) in grid.cells.iter().enumerate() {
		let col = index as u32 % cols.max(1);
		let row = index as u32 / cols.max(1);
		let is_selected = *selected_cell == Some(CellId(index as u32));
		let is_hovered = !is_selected && hovered_cell_index == Some(index as u32);

		let cell_min = viewport.image_to_screen(
			viewport_rect,
			egui::pos2(
				(origin_x + col * cell_width) as f32,
				(origin_y + row * cell_height) as f32,
			),
		);
		let cell_max = viewport.image_to_screen(
			viewport_rect,
			egui::pos2(
				(origin_x + (col + 1) * cell_width) as f32,
				(origin_y + (row + 1) * cell_height) as f32,
			),
		);
		let cell_border_color = if is_hovered {
			Color32::from_gray(200)
		} else {
			Color32::from_gray(90)
		};
		painter.rect_stroke(
			egui::Rect::from_min_max(cell_min, cell_max).shrink(CELL_GAP),
			0.0,
			(1.0, cell_border_color),
			egui::StrokeKind::Inside,
		);
		painter.text(
			cell_min + egui::vec2(3.0, 1.0),
			egui::Align2::LEFT_TOP,
			index.to_string(),
			egui::FontId::monospace(36.0),
			Color32::from_gray(190),
		);

		let Some(trim) = cell.trim else {
			continue;
		};
		let trim_min = viewport.image_to_screen(
			viewport_rect,
			egui::pos2(
				(origin_x + col * cell_width + trim.offset.0) as f32,
				(origin_y + row * cell_height + trim.offset.1) as f32,
			),
		);
		let trim_max = viewport.image_to_screen(
			viewport_rect,
			egui::pos2(
				(origin_x + col * cell_width + trim.offset.0 + trim.size.0) as f32,
				(origin_y + row * cell_height + trim.offset.1 + trim.size.1) as f32,
			),
		);
		let stroke_color = if is_selected {
			Color32::YELLOW
		} else if is_hovered {
			Color32::from_rgb(120, 200, 255)
		} else {
			Color32::GREEN
		};
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
			// `sample_pixel` returns `None` for a click outside the image's
			// own bounds (e.g. on the checkerboard margin around a source
			// image smaller than the viewport) -- previously silent, which
			// looked identical to a stuck/non-functional eyedropper. Every
			// eyedropper click now gets a status message one way or another.
			match sample_pixel(&preview.rgba, image_point) {
				Some((color, alpha)) if alpha > EYEDROPPER_TRANSPARENT_ALPHA_THRESHOLD => {
					grid.background_color = color;
					grid.cells = recompute_uniform_grid_trims_preserving_manual(
						&preview.rgba,
						grid.cell_width,
						grid.cell_height,
						grid.origin_x,
						grid.origin_y,
						color,
						&grid.cells,
					);
					*eyedropper_active = false;
					*status = None;
					changed = true;
				}
				Some(_) => {
					*status = Some(
						"That pixel is transparent -- background color needs a visible pixel. Pick a \
						 solid-colored pixel instead."
							.to_string(),
					);
				}
				None => {
					*status = Some("Click inside the source image to sample a background color.".to_string());
				}
			}
		} else if let Some(index) =
			pick_uniform_grid_cell(image_point, cell_width, cell_height, origin_x, origin_y, cols, rows)
		{
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

	if response.secondary_clicked() {
		*context_menu_cell = hovered_cell_index
			.filter(|&index| grid.cells.get(index as usize).is_some_and(|cell| cell.trim.is_some()))
			.map(CellId);
	}
	response.context_menu(|ui| {
		let Some(cell_id) = *context_menu_cell else {
			ui.label("No cell here.");
			return;
		};
		ui.label(format!("Cell #{}", cell_id.0));
		ui.separator();
		if let Some(sheet_path) = sheet_path
			&& ui.button("Create Animation Clip starting here").clicked()
		{
			match create_animation_clip_from_cell(assets_root, sheet_path, cell_id) {
				Ok(dest) => {
					*open_animation_clip = Some(dest);
					*status = None;
				}
				Err(error) => *status = Some(error),
			}
			ui.close();
		}
	});

	changed
}

/// Creates a new `.animationclip.json` under `<assets_root>/animation_clips/`
/// sourced from `sheet_relative_path` and seeded with a single starting
/// frame -- the "Create Animation Clip starting here" context-menu
/// shortcut's fast path. Unlike `file_hierarchy.rs`'s "Create Animation Clip"
/// flow, this skips the name-prompt dialog entirely (auto-named after the
/// sheet, with the same numeric-suffix collision handling), since a
/// right-click shortcut should be lower-friction than the full flow it
/// stands in for.
fn create_animation_clip_from_cell(
	assets_root: &Path,
	sheet_relative_path: &str,
	starting_cell: CellId,
) -> Result<PathBuf, String> {
	let mut clip = AnimationClip::new(AssetRef::game(sheet_relative_path.to_string()));
	clip.frames.push(starting_cell);

	let dir = assets_root.join("animation_clips");
	let stem = display_name_for_relative_path(sheet_relative_path);
	let mut dest = dir.join(format!("{stem}.{}", ze_assets::ANIMATION_CLIP_EXTENSION));
	let mut suffix = 2;
	while dest.exists() {
		dest = dir.join(format!("{stem}_{suffix}.{}", ze_assets::ANIMATION_CLIP_EXTENSION));
		suffix += 1;
	}

	clip.save(&dest)
		.map_err(|error| format!("Failed to save animation clip: {error}"))?;
	Ok(dest.canonicalize().unwrap_or(dest))
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
						origin_x: 0,
						origin_y: 0,
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
fn show_uniform_grid_settings(
	ui: &mut Ui,
	grid: &mut UniformGridSheet,
	eyedropper_active: &mut bool,
	source_preview: Option<&DecodedImage>,
) -> (bool, bool) {
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
		ui.label("Grid origin X")
			.on_hover_text("Pixel offset of the grid's first cell from the source image's own (0, 0) -- use this if the sheet has a margin/padding before its first row/column.");
		let mut origin_x = grid.origin_x;
		if ui.add(egui::DragValue::new(&mut origin_x).range(0..=8192)).changed() {
			grid.origin_x = origin_x;
			grid_changed = true;
		}
		ui.label("Y");
		let mut origin_y = grid.origin_y;
		if ui.add(egui::DragValue::new(&mut origin_y).range(0..=8192)).changed() {
			grid.origin_y = origin_y;
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

	let non_empty = grid.cells.iter().filter(|cell| cell.trim.is_some()).count();
	ui.horizontal(|ui| {
		ui.label(format!("{non_empty} / {} non-empty cells", grid.cells.len()));
		if let Some(preview) = source_preview
			&& ui
				.button("Re-trim All")
				.on_hover_text(
					"Force-recompute trim for every cell (useful after changing background color if a cell's \
					 trim didn't refresh automatically). Manually-adjusted trims are left untouched.",
				)
				.clicked()
		{
			grid.cells = recompute_uniform_grid_trims_preserving_manual(
				&preview.rgba,
				grid.cell_width,
				grid.cell_height,
				grid.origin_x,
				grid.origin_y,
				grid.background_color,
				&grid.cells,
			);
			grid_changed = true;
		}
	});

	(texture_changed, grid_changed)
}

/// Shows the selected cell's trim data, editable via `DragValue`s so the
/// user can manually resize/reposition the crop box when auto-detection
/// doesn't match what they want (crop tighter/looser, or shift to include/
/// exclude specific pixels). Any edit here marks the trim `manual`, which
/// `recompute_uniform_grid_trims_preserving_manual` then treats as an
/// override to keep rather than silently overwrite on the next
/// background-color change, texture reload, or "Re-trim All". Returns
/// whether anything changed (the caller marks the sheet dirty on `true`).
fn show_uniform_grid_cell_inspector(
	ui: &mut Ui,
	grid: &mut UniformGridSheet,
	selected_cell: Option<CellId>,
	source_preview: Option<&DecodedImage>,
) -> bool {
	let Some(cell_id) = selected_cell else {
		ui.label("No cell selected.");
		return false;
	};
	let cell_width = grid.cell_width.max(1);
	let cell_height = grid.cell_height.max(1);
	let origin_x = grid.origin_x;
	let origin_y = grid.origin_y;
	let background_color = grid.background_color;
	let Some(existing_trim) = grid.cells.get(cell_id.0 as usize).map(|cell| cell.trim) else {
		ui.label("Selected cell is out of range for the current grid.");
		return false;
	};

	ui.label(format!("Cell #{}", cell_id.0));

	let mut changed = false;
	if let Some(mut trim) = existing_trim {
		if trim.manual {
			ui.colored_label(Color32::YELLOW, "Manual override");
		} else {
			ui.label("Auto-detected");
		}

		let mut edited = false;
		ui.horizontal(|ui| {
			ui.label("Trim offset X");
			edited |= ui
				.add(egui::DragValue::new(&mut trim.offset.0).range(0..=cell_width - 1))
				.changed();
			ui.label("Y");
			edited |= ui
				.add(egui::DragValue::new(&mut trim.offset.1).range(0..=cell_height - 1))
				.changed();
		});
		ui.horizontal(|ui| {
			ui.label("Trim size W");
			edited |= ui
				.add(egui::DragValue::new(&mut trim.size.0).range(1..=cell_width.saturating_sub(trim.offset.0).max(1)))
				.changed();
			ui.label("H");
			edited |= ui
				.add(egui::DragValue::new(&mut trim.size.1).range(1..=cell_height.saturating_sub(trim.offset.1).max(1)))
				.changed();
		});

		ui.label(format!("Untrimmed cell size: {:?}", trim.original_size));

		let reset_to_auto = trim.manual
			&& source_preview.is_some()
			&& ui
				.button("Reset to Auto")
				.on_hover_text("Discard the manual override and recompute this cell's trim from the source pixels")
				.clicked();

		if edited {
			// Re-clamp after the edit: `DragValue::range` is evaluated
			// against the *previous* frame's values, so shrinking `offset`
			// right after `size` was already at its old max could otherwise
			// leave offset+size past the cell bounds.
			trim.offset.0 = trim.offset.0.min(cell_width - 1);
			trim.offset.1 = trim.offset.1.min(cell_height - 1);
			trim.size.0 = trim.size.0.clamp(1, cell_width.saturating_sub(trim.offset.0).max(1));
			trim.size.1 = trim.size.1.clamp(1, cell_height.saturating_sub(trim.offset.1).max(1));
			trim.manual = true;
			if let Some(cell) = grid.cells.get_mut(cell_id.0 as usize) {
				cell.trim = Some(trim);
			}
			changed = true;
		} else if reset_to_auto && let Some(preview) = source_preview {
			let recomputed = compute_uniform_grid_trims(
				&preview.rgba,
				cell_width,
				cell_height,
				origin_x,
				origin_y,
				background_color,
			);
			if let Some(cell) = grid.cells.get_mut(cell_id.0 as usize) {
				cell.trim = recomputed
					.get(cell_id.0 as usize)
					.and_then(|recomputed_cell| recomputed_cell.trim);
			}
			changed = true;
		}
	} else {
		ui.label("Empty cell (background/transparent) -- not placeable.");
		if source_preview.is_some()
			&& ui
				.button("Add Manual Trim")
				.on_hover_text(
					"Force this cell to be placeable with a manually-sized crop box, overriding auto-detection",
				)
				.clicked()
			&& let Some(cell) = grid.cells.get_mut(cell_id.0 as usize)
		{
			cell.trim = Some(CellTrim {
				offset: (0, 0),
				size: (cell_width, cell_height),
				original_size: (cell_width, cell_height),
				manual: true,
			});
			changed = true;
		}
	}

	changed
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
