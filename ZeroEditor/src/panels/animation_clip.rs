use std::{collections::HashMap, path::Path};

use egui::{Color32, ColorImage, TextureHandle, TextureOptions, Ui};
use ze_assets::{
	AnimationClip, CellId, GridCell, TextureSheet, TextureSheetMode, UniformGridSheet, step_animation_frame,
};

use super::{
	EditorPanelContext, EditorSelection, Panel,
	texture_sheet_common::{
		DecodedImage, ViewportTransform, draw_checkerboard, load_or_reload_decoded_image, pick_uniform_grid_cell,
	},
};

/// Fixed screen-space gap between adjacent cells' rendered rects, matching
/// `texture_sheet.rs`'s `CELL_GAP` convention.
const CELL_GAP: f32 = 2.0;
const PREVIEW_SIZE: f32 = 96.0;

pub struct AnimationClipPanel {
	current_path: Option<std::path::PathBuf>,
	clip: Option<AnimationClip>,
	dirty: bool,
	status: Option<String>,
	sheet: Option<TextureSheet>,
	sheet_load_error: Option<String>,
	source_preview: Option<DecodedImage>,
	viewport: ViewportTransform,
	viewport_fitted: bool,
	preview_playing: bool,
	preview_frame_index: usize,
	preview_elapsed_secs: f32,
	frame_thumbnails: HashMap<CellId, TextureHandle>,
}

impl AnimationClipPanel {
	pub fn new() -> Self {
		Self {
			current_path: None,
			clip: None,
			dirty: false,
			status: None,
			sheet: None,
			sheet_load_error: None,
			source_preview: None,
			viewport: ViewportTransform {
				zoom: 1.0,
				pan: egui::Vec2::ZERO,
			},
			viewport_fitted: false,
			preview_playing: true,
			preview_frame_index: 0,
			preview_elapsed_secs: 0.0,
			frame_thumbnails: HashMap::new(),
		}
	}
}

impl Default for AnimationClipPanel {
	fn default() -> Self { Self::new() }
}

impl Panel for AnimationClipPanel {
	fn name(&self) -> &'static str { "Animation Clip" }

	// Mirrors `TextureSheetPanel`: this panel wraps its own content in a single
	// `ScrollArea`, so the dock framework's own wrapper is disabled to avoid a
	// double-scrolling region.
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

			let Some(clip) = self.clip.as_mut() else {
				ui.label("Select or create an .animationclip.json asset to edit it here.");
				return;
			};

			if let Some(error) = &self.sheet_load_error {
				ui.colored_label(Color32::YELLOW, error);
				return;
			}

			let Some(sheet) = self.sheet.as_ref() else {
				ui.label("Loading source texture sheet...");
				return;
			};
			let TextureSheetMode::UniformGrid(grid) = &sheet.mode else {
				ui.colored_label(
					Color32::YELLOW,
					"Source sheet must be a Uniform Grid sheet (Auto-pack sheets have no uniform cell layout).",
				);
				return;
			};

			let viewport_changed = show_animation_clip_viewport(
				ui,
				clip,
				grid,
				&assets_root,
				&mut self.source_preview,
				&mut self.viewport,
				&mut self.viewport_fitted,
			);
			if viewport_changed {
				self.dirty = true;
			}

			ui.separator();

			if let Some(preview) = &self.source_preview {
				let list_changed = show_frame_list(ui, clip, grid, preview, &mut self.frame_thumbnails);
				if list_changed {
					self.dirty = true;
					self.preview_frame_index = self.preview_frame_index.min(clip.frames.len().saturating_sub(1));
				}

				ui.separator();

				show_playback_preview(
					ui,
					clip,
					grid,
					preview,
					&mut self.frame_thumbnails,
					&mut self.preview_frame_index,
					&mut self.preview_elapsed_secs,
					&mut self.preview_playing,
				);

				ui.separator();
			}

			if show_clip_settings(ui, clip) {
				self.dirty = true;
			}
		});

		self.autosave_if_settled(ui.ctx());
	}
}

impl AnimationClipPanel {
	fn sync_from_selection(&mut self, context: &EditorPanelContext<'_>) {
		let Some(EditorSelection::Asset(path)) = context.selection.as_ref() else {
			return;
		};
		if !path
			.file_name()
			.and_then(|name| name.to_str())
			.is_some_and(|name| name.ends_with(&format!(".{}", ze_assets::ANIMATION_CLIP_EXTENSION)))
		{
			return;
		}
		if self.current_path.as_deref() == Some(path.as_path()) {
			return;
		}

		match AnimationClip::load(path) {
			Ok(clip) => {
				let sheet_result = context
					.project
					.map(|project| project.asset_dir.join(&clip.texture_sheet.path))
					.map(|sheet_path| TextureSheet::load(&sheet_path));

				self.current_path = Some(path.clone());
				self.dirty = false;
				self.source_preview = None;
				self.viewport_fitted = false;
				self.frame_thumbnails.clear();
				self.preview_frame_index = 0;
				self.preview_elapsed_secs = 0.0;
				self.status = None;

				match sheet_result {
					Some(Ok(sheet)) => {
						self.sheet = Some(sheet);
						self.sheet_load_error = None;
					}
					Some(Err(error)) => {
						self.sheet = None;
						self.sheet_load_error = Some(format!("Failed to load source texture sheet: {error}"));
					}
					None => {
						self.sheet = None;
						self.sheet_load_error = Some("No project loaded.".to_string());
					}
				}

				self.clip = Some(clip);
			}
			Err(error) => {
				self.status = Some(format!("Failed to load animation clip: {error}"));
			}
		}
	}

	fn show_header(&self, ui: &mut Ui) {
		let label = self
			.current_path
			.as_ref()
			.and_then(|path| path.file_name())
			.and_then(|name| name.to_str())
			.unwrap_or("No animation clip open");
		ui.label(egui::RichText::new(label).strong());
		if let Some(status) = &self.status {
			ui.colored_label(Color32::YELLOW, status);
		}
	}

	/// Same idle-based auto-save pattern as
	/// `TextureSheetPanel::autosave_if_settled` -- see its doc comment for why
	/// this fires once-per-settled-edit rather than on every intermediate
	/// frame of a drag/keystroke.
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
		let Some(clip) = &self.clip else {
			return;
		};

		match clip.save(&path) {
			Ok(()) => {
				self.dirty = false;
				self.status = None;
			}
			Err(error) => {
				self.status = Some(format!("Failed to save animation clip: {error}"));
			}
		}
	}
}

/// Allocates the pannable/zoomable viewport and draws the source sheet's grid
/// (reusing `texture_sheet_common`'s primitives, same visual treatment as
/// `TextureSheetPanel`). Unlike that panel's single-select behavior, clicking
/// a paintable cell here *appends* it to `clip.frames`, and every cell that
/// already appears in the sequence gets a small ordinal badge (a cell can
/// repeat, so all of its 1-based positions are shown, e.g. "1,4"). Returns
/// whether a frame was appended.
fn show_animation_clip_viewport(
	ui: &mut Ui,
	clip: &mut AnimationClip,
	grid: &UniformGridSheet,
	assets_root: &Path,
	source_preview: &mut Option<DecodedImage>,
	viewport: &mut ViewportTransform,
	viewport_fitted: &mut bool,
) -> bool {
	let viewport_height = (ui.available_height() * 0.45).clamp(200.0, 480.0);
	let (response, painter) = ui.allocate_painter(
		egui::vec2(ui.available_width(), viewport_height),
		egui::Sense::click_and_drag(),
	);
	let viewport_rect = response.rect;

	draw_checkerboard(&painter, viewport_rect, 16.0);

	if grid.texture.path.is_empty() {
		painter.text(
			viewport_rect.center(),
			egui::Align2::CENTER_CENTER,
			"Source sheet has no texture set.",
			egui::FontId::default(),
			Color32::from_gray(160),
		);
		return false;
	}

	let source_path = assets_root.join(&grid.texture.path);
	let Some((preview, _reloaded)) = load_or_reload_decoded_image(ui.ctx(), &source_path, source_preview) else {
		painter.text(
			viewport_rect.center(),
			egui::Align2::CENTER_CENTER,
			format!("Source texture not found: {}", source_path.display()),
			egui::FontId::default(),
			Color32::from_gray(160),
		);
		return false;
	};

	let image_size = egui::vec2(preview.rgba.width() as f32, preview.rgba.height() as f32);
	if !*viewport_fitted {
		*viewport = ViewportTransform::fit(image_size, viewport_rect);
		*viewport_fitted = true;
	}
	viewport.apply_input(ui, &response, viewport_rect);

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

	// 1-based ordinal positions this cell index occupies in `clip.frames`
	// (may be several, since a cell can repeat).
	let mut ordinals: HashMap<u32, Vec<usize>> = HashMap::new();
	for (position, frame) in clip.frames.iter().enumerate() {
		ordinals.entry(frame.0).or_default().push(position + 1);
	}

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
		let in_sequence = ordinals.contains_key(&(index as u32));
		let stroke_color = if in_sequence { Color32::YELLOW } else { Color32::GREEN };
		let trim_rect = egui::Rect::from_min_max(trim_min, trim_max).shrink(CELL_GAP);
		painter.rect_stroke(trim_rect, 0.0, (2.0, stroke_color), egui::StrokeKind::Inside);

		if let Some(positions) = ordinals.get(&(index as u32)) {
			let label = positions.iter().map(ToString::to_string).collect::<Vec<_>>().join(",");
			painter.text(
				trim_rect.left_top(),
				egui::Align2::LEFT_TOP,
				label,
				egui::FontId::monospace(11.0),
				Color32::YELLOW,
			);
		}
	}

	let mut changed = false;
	if response.clicked()
		&& let Some(pointer) = response.interact_pointer_pos()
	{
		let image_point = viewport.screen_to_image(viewport_rect, pointer);
		if let Some(index) = pick_uniform_grid_cell(image_point, cell_width, cell_height, cols, rows)
			&& grid.cells.get(index as usize).is_some_and(|cell| cell.trim.is_some())
		{
			clip.frames.push(CellId(index));
			changed = true;
		}
	}

	changed
}

/// Crops the cell's trimmed rect out of the decoded source image and uploads
/// it as a small cached thumbnail texture (one upload per distinct `CellId`
/// actually used in the clip, not per frame-list row).
fn get_or_create_thumbnail<'a>(
	ui: &Ui,
	cell_id: CellId,
	grid: &UniformGridSheet,
	cols: u32,
	preview: &DecodedImage,
	cache: &'a mut HashMap<CellId, TextureHandle>,
) -> Option<&'a TextureHandle> {
	if !cache.contains_key(&cell_id) {
		let cell: &GridCell = grid.cells.get(cell_id.0 as usize)?;
		let trim = cell.trim?;
		let cell_width = grid.cell_width.max(1);
		let cell_height = grid.cell_height.max(1);
		let col = cell_id.0 % cols;
		let row = cell_id.0 / cols;
		let x = col * cell_width + trim.offset.0;
		let y = row * cell_height + trim.offset.1;
		let (w, h) = trim.size;
		if x + w > preview.rgba.width() || y + h > preview.rgba.height() {
			return None;
		}
		let cropped = image::imageops::crop_imm(&preview.rgba, x, y, w, h).to_image();
		let color_image = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &cropped);
		let texture = ui.ctx().load_texture(
			format!("animation_clip_thumbnail_{}", cell_id.0),
			color_image,
			TextureOptions::NEAREST,
		);
		cache.insert(cell_id, texture);
	}

	cache.get(&cell_id)
}

/// The ordered frame-sequence list: one row per `clip.frames` entry with a
/// cropped thumbnail, ordinal index, ▲/▼ reorder (swap-with-neighbor, simpler
/// and less error-prone in egui than drag-reordering), and a Remove button.
/// Same list-editing shape as `TextureSheetPanel`'s Auto-pack file list.
fn show_frame_list(
	ui: &mut Ui,
	clip: &mut AnimationClip,
	grid: &UniformGridSheet,
	preview: &DecodedImage,
	frame_thumbnails: &mut HashMap<CellId, TextureHandle>,
) -> bool {
	ui.label(egui::RichText::new("Frames").strong());

	let cell_width = grid.cell_width.max(1);
	let cols = (preview.rgba.width() / cell_width).max(1);

	if clip.frames.is_empty() {
		ui.label("Click a cell in the viewport above to add it to the sequence.");
		return false;
	}

	let mut changed = false;
	let mut remove_index = None;
	let mut swap_with_next = None;

	for (index, cell_id) in clip.frames.clone().iter().enumerate() {
		ui.horizontal(|ui| {
			ui.label(format!("{}.", index + 1));

			if let Some(texture) = get_or_create_thumbnail(ui, *cell_id, grid, cols, preview, frame_thumbnails) {
				ui.add(egui::Image::new((texture.id(), egui::vec2(32.0, 32.0))));
			} else {
				ui.label("(empty cell)");
			}

			ui.label(format!("Cell #{}", cell_id.0));

			if ui.small_button("▲").clicked() && index > 0 {
				swap_with_next = Some(index - 1);
			}
			if ui.small_button("▼").clicked() && index + 1 < clip.frames.len() {
				swap_with_next = Some(index);
			}
			if ui.small_button("Remove").clicked() {
				remove_index = Some(index);
			}
		});
	}

	if let Some(index) = swap_with_next {
		clip.frames.swap(index, index + 1);
		changed = true;
	}
	if let Some(index) = remove_index {
		clip.frames.remove(index);
		changed = true;
	}

	changed
}

/// A small fixed-size area that actually loops the clip at its configured
/// frame duration, so the user can validate a clip looks right without
/// running the game. Shares `step_animation_frame` with the runtime
/// `AnimationSystem` (`ze_assets::step_animation_frame`) so preview and
/// runtime playback never drift out of sync.
#[allow(clippy::too_many_arguments)]
fn show_playback_preview(
	ui: &mut Ui,
	clip: &AnimationClip,
	grid: &UniformGridSheet,
	preview: &DecodedImage,
	frame_thumbnails: &mut HashMap<CellId, TextureHandle>,
	preview_frame_index: &mut usize,
	preview_elapsed_secs: &mut f32,
	preview_playing: &mut bool,
) {
	ui.label(egui::RichText::new("Preview").strong());

	if clip.frames.is_empty() {
		ui.label("Add at least one frame to preview playback.");
		return;
	}

	if *preview_playing {
		let dt = ui.input(|input| input.stable_dt);
		step_animation_frame(
			preview_frame_index,
			preview_elapsed_secs,
			dt,
			clip.frames.len(),
			clip.frame_duration_ms,
			clip.loop_animation,
		);
		ui.ctx().request_repaint();
	}
	*preview_frame_index = (*preview_frame_index).min(clip.frames.len() - 1);

	ui.horizontal(|ui| {
		let cell_width = grid.cell_width.max(1);
		let cols = (preview.rgba.width() / cell_width).max(1);
		let cell_id = clip.frames[*preview_frame_index];

		let (rect, _response) = ui.allocate_exact_size(egui::vec2(PREVIEW_SIZE, PREVIEW_SIZE), egui::Sense::hover());
		ui.painter().rect_filled(rect, 0.0, Color32::from_gray(30));
		if let Some(texture) = get_or_create_thumbnail(ui, cell_id, grid, cols, preview, frame_thumbnails) {
			ui.painter().image(
				texture.id(),
				rect.shrink(4.0),
				egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
				Color32::WHITE,
			);
		}

		ui.vertical(|ui| {
			let label = if *preview_playing { "Pause" } else { "Play" };
			if ui.button(label).clicked() {
				*preview_playing = !*preview_playing;
			}
			ui.label(format!("Frame {} / {}", *preview_frame_index + 1, clip.frames.len()));
		});
	});
}

/// `frame_duration_ms`/loop settings. Changing the source sheet after
/// creation is not exposed here -- the sheet is fixed at creation time (see
/// `file_hierarchy.rs`'s "Create Animation Clip" flow), avoiding needing to
/// handle "existing frame indices now invalid for a different sheet"
/// migration logic.
fn show_clip_settings(ui: &mut Ui, clip: &mut AnimationClip) -> bool {
	let mut changed = false;

	ui.horizontal(|ui| {
		ui.label("Source sheet");
		ui.label(&clip.texture_sheet.path);
	});

	ui.horizontal(|ui| {
		ui.label("Frame duration (ms)");
		if ui
			.add(
				egui::DragValue::new(&mut clip.frame_duration_ms)
					.speed(1.0)
					.range(1.0..=60_000.0),
			)
			.changed()
		{
			changed = true;
		}
	});

	if ui.checkbox(&mut clip.loop_animation, "Loop").changed() {
		changed = true;
	}

	changed
}
