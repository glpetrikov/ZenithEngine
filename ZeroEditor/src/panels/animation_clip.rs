use std::{collections::HashMap, path::Path};

use egui::{Color32, ColorImage, TextureHandle, TextureOptions, Ui};
use ze_assets::{
	AnimationClip, CellId, CellTrim, GridCell, TextureSheet, TextureSheetMode, UniformGridSheet, step_animation_frame,
};

use super::{
	EditorPanelContext, EditorSelection, Panel,
	texture_sheet_common::{
		DecodedImage, ViewportTransform, draw_checkerboard, load_or_reload_decoded_image, pick_uniform_grid_cell,
		show_viewport_zoom_toolbar,
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

			if handle_frame_keyboard_nav(ui, clip, &mut self.preview_frame_index, &mut self.preview_playing) {
				self.dirty = true;
			}

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
				let list_changed = show_frame_list(
					ui,
					clip,
					grid,
					preview,
					&mut self.frame_thumbnails,
					&mut self.preview_frame_index,
				);
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

/// Up/Down steps the currently selected/scrubbed frame (`preview_frame_index`
/// doubles as both the playback-preview position and the frame-list
/// selection -- see `show_frame_list`) -- Up/Down rather than Left/Right
/// because the frame list is a vertically-stacked column of rows, not a
/// horizontal strip; Left/Right are also accepted as a secondary binding.
/// Delete removes the selected frame from the sequence. Guarded on no widget
/// currently holding keyboard focus, so these don't fire while e.g. a
/// `DragValue` or the source-sheet text field is mid-edit -- same guard
/// `autosave_if_settled` uses. Returns whether `clip.frames` changed (a
/// Delete), not whether the selection moved.
fn handle_frame_keyboard_nav(
	ui: &Ui,
	clip: &mut AnimationClip,
	preview_frame_index: &mut usize,
	preview_playing: &mut bool,
) -> bool {
	if clip.frames.is_empty() || ui.memory(|memory| memory.focused().is_some()) {
		return false;
	}

	if ui.input(|input| input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::ArrowRight)) {
		*preview_playing = false;
		*preview_frame_index = (*preview_frame_index + 1).min(clip.frames.len() - 1);
	}
	if ui.input(|input| input.key_pressed(egui::Key::ArrowUp) || input.key_pressed(egui::Key::ArrowLeft)) {
		*preview_playing = false;
		*preview_frame_index = preview_frame_index.saturating_sub(1);
	}
	if ui.input(|input| input.key_pressed(egui::Key::Delete)) {
		clip.frames.remove(*preview_frame_index);
		*preview_frame_index = (*preview_frame_index).min(clip.frames.len().saturating_sub(1));
		return true;
	}

	false
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
	let origin_x = grid.origin_x;
	let origin_y = grid.origin_y;
	let (cols, rows) = grid.grid_dimensions(preview.rgba.width(), preview.rgba.height());

	if grid.cells.is_empty() {
		painter.text(
			viewport_rect.center(),
			egui::Align2::CENTER_CENTER,
			"Source sheet has no cells for its current grid settings\n(check cell width/height and grid origin \
			 against the source texture's size in the Texture Sheet panel).",
			egui::FontId::default(),
			Color32::from_gray(160),
		);
	}

	// 1-based ordinal positions this cell index occupies in `clip.frames`
	// (may be several, since a cell can repeat).
	let mut ordinals: HashMap<u32, Vec<usize>> = HashMap::new();
	for (position, frame) in clip.frames.iter().enumerate() {
		ordinals.entry(frame.0).or_default().push(position + 1);
	}

	for (index, cell) in grid.cells.iter().enumerate() {
		let col = index as u32 % cols.max(1);
		let row = index as u32 / cols.max(1);
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
		if let Some(index) =
			pick_uniform_grid_cell(image_point, cell_width, cell_height, origin_x, origin_y, cols, rows)
			&& grid.cells.get(index as usize).is_some_and(|cell| cell.trim.is_some())
		{
			clip.frames.push(CellId(index));
			changed = true;
		}
	}

	show_viewport_zoom_toolbar(ui, viewport_rect, viewport, viewport_fitted, false);

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
		let x = grid.origin_x + col * cell_width + trim.offset.0;
		let y = grid.origin_y + row * cell_height + trim.offset.1;
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

/// Maps a `CellTrim`'s cell-local offset/size onto `target_rect`, scaled by
/// the cell's `original_size` -- so a cell that's trimmed smaller than its
/// neighbors lands at its true relative position and size within
/// `target_rect` instead of independently stretching its tightly-cropped
/// thumbnail to fill the whole rect. The latter would silently normalize
/// away exactly the position/size differences onion skinning (and any
/// frame-to-frame comparison) exists to show.
fn trim_rect_within(target_rect: egui::Rect, trim: &CellTrim) -> egui::Rect {
	let (original_width, original_height) = trim.original_size;
	let scale_x = target_rect.width() / original_width.max(1) as f32;
	let scale_y = target_rect.height() / original_height.max(1) as f32;
	let min = target_rect.min + egui::vec2(trim.offset.0 as f32 * scale_x, trim.offset.1 as f32 * scale_y);
	let size = egui::vec2(trim.size.0 as f32 * scale_x, trim.size.1 as f32 * scale_y);
	egui::Rect::from_min_size(min, size)
}

/// Draws `cell_id`'s cropped sprite inside `target_rect` at its true
/// relative position/size (via `trim_rect_within`), tinted by `tint` -- used
/// for both the current frame and onion-skin ghosts in the playback preview,
/// so all of them share one common coordinate space instead of each
/// independently filling `target_rect`. Does nothing for an empty
/// (background-only) cell.
#[allow(clippy::too_many_arguments)]
fn draw_cell_sprite(
	ui: &Ui,
	cell_id: CellId,
	grid: &UniformGridSheet,
	cols: u32,
	preview: &DecodedImage,
	frame_thumbnails: &mut HashMap<CellId, TextureHandle>,
	target_rect: egui::Rect,
	tint: Color32,
) {
	let Some(trim) = grid.cells.get(cell_id.0 as usize).and_then(|cell| cell.trim) else {
		return;
	};
	let Some(texture) = get_or_create_thumbnail(ui, cell_id, grid, cols, preview, frame_thumbnails) else {
		return;
	};
	ui.painter().image(
		texture.id(),
		trim_rect_within(target_rect, &trim),
		egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
		tint,
	);
}

/// The ordered frame-sequence list: one row per `clip.frames` entry with a
/// drag handle (reorder by dragging), cropped thumbnail, ordinal index,
/// ▲/▼ reorder (swap-with-neighbor -- kept alongside dragging for
/// single-step precision), a Duplicate button (inserts a copy right after,
/// for holding a frame longer without touching `frame_duration_ms`), and a
/// Remove button. Clicking a row (or its ordinal label) sets
/// `selected_frame`, which doubles as the playback preview's scrub position
/// -- see `handle_frame_keyboard_nav` and `show_playback_preview`.
fn show_frame_list(
	ui: &mut Ui,
	clip: &mut AnimationClip,
	grid: &UniformGridSheet,
	preview: &DecodedImage,
	frame_thumbnails: &mut HashMap<CellId, TextureHandle>,
	selected_frame: &mut usize,
) -> bool {
	ui.label(egui::RichText::new("Frames").strong());

	let (cols, _rows) = grid.grid_dimensions(preview.rgba.width(), preview.rgba.height());
	let cols = cols.max(1);

	if clip.frames.is_empty() {
		ui.label("Click a cell in the viewport above to add it to the sequence.");
		return false;
	}

	let mut changed = false;
	let mut remove_index = None;
	let mut swap_with_next = None;
	let mut duplicate_index = None;
	let mut drag_from = None;
	let mut drag_to = None;

	for (index, cell_id) in clip.frames.clone().iter().enumerate() {
		let is_selected = *selected_frame == index;
		let drag_handle_id = ui.id().with("animation_clip_frame_drag").with(index);

		let row = ui.horizontal(|ui| {
			ui.dnd_drag_source(drag_handle_id, index, |ui| {
				ui.label("⠿").on_hover_text("Drag to reorder");
			});

			if ui.selectable_label(is_selected, format!("{}.", index + 1)).clicked() {
				*selected_frame = index;
			}

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
			if ui.small_button("Duplicate").clicked() {
				duplicate_index = Some(index);
			}
			if ui.small_button("Remove").clicked() {
				remove_index = Some(index);
			}
		});

		if is_selected {
			ui.painter().rect_stroke(
				row.response.rect,
				2.0,
				(1.5, ui.visuals().selection.stroke.color),
				egui::StrokeKind::Inside,
			);
		}

		if row.response.dnd_hover_payload::<usize>().is_some() {
			let rect = row.response.rect;
			let insert_before = ui
				.input(|input| input.pointer.hover_pos())
				.is_some_and(|pointer| pointer.y < rect.center().y);
			let line_y = if insert_before { rect.top() } else { rect.bottom() };
			ui.painter().hline(
				rect.x_range(),
				line_y,
				egui::Stroke::new(2.0, ui.visuals().strong_text_color()),
			);

			if let Some(dragged_index) = row.response.dnd_release_payload::<usize>() {
				drag_from = Some(*dragged_index);
				drag_to = Some(if insert_before { index } else { index + 1 });
			}
		}
	}

	if let (Some(from), Some(to)) = (drag_from, drag_to)
		&& to != from
		&& to != from + 1
	{
		let moved = clip.frames.remove(from);
		let to = if to > from { to - 1 } else { to };
		clip.frames.insert(to, moved);
		*selected_frame = to;
		changed = true;
	}
	if let Some(index) = swap_with_next {
		clip.frames.swap(index, index + 1);
		*selected_frame = if *selected_frame == index {
			index + 1
		} else if *selected_frame == index + 1 {
			index
		} else {
			*selected_frame
		};
		changed = true;
	}
	if let Some(index) = duplicate_index {
		clip.frames.insert(index + 1, clip.frames[index]);
		*selected_frame = index + 1;
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
		let (cols, _rows) = grid.grid_dimensions(preview.rgba.width(), preview.rgba.height());
		let cols = cols.max(1);
		let cell_id = clip.frames[*preview_frame_index];

		let (rect, _response) = ui.allocate_exact_size(egui::vec2(PREVIEW_SIZE, PREVIEW_SIZE), egui::Sense::hover());
		ui.painter().rect_filled(rect, 0.0, Color32::from_gray(30));
		let full_rect = rect.shrink(4.0);

		// Onion skinning: faint ghosts of the neighboring frames, shown only
		// while paused/scrubbing -- during playback they'd just read as
		// constant motion blur rather than a spacing aid. Previous/next are
		// tinted red/blue (the conventional onion-skinning color split) so
		// direction of motion stays legible at a glance.
		if !*preview_playing && clip.frames.len() > 1 {
			const ONION_ALPHA: u8 = 130;
			if *preview_frame_index > 0 {
				draw_cell_sprite(
					ui,
					clip.frames[*preview_frame_index - 1],
					grid,
					cols,
					preview,
					frame_thumbnails,
					full_rect,
					Color32::from_rgba_unmultiplied(255, 80, 80, ONION_ALPHA),
				);
			}
			if *preview_frame_index + 1 < clip.frames.len() {
				draw_cell_sprite(
					ui,
					clip.frames[*preview_frame_index + 1],
					grid,
					cols,
					preview,
					frame_thumbnails,
					full_rect,
					Color32::from_rgba_unmultiplied(80, 140, 255, ONION_ALPHA),
				);
			}
		}

		draw_cell_sprite(
			ui,
			cell_id,
			grid,
			cols,
			preview,
			frame_thumbnails,
			full_rect,
			Color32::WHITE,
		);

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
