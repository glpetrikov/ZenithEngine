//! The "Create Texture Sheet" modal, triggered by right-clicking an image in
//! the File Explorer. Lets the user confirm cell size / background color
//! with a live grid-detection preview before a `.texturesheet.json` is
//! written to disk under `<assets_root>/texture_sheets/`.

use std::path::{Path, PathBuf};

use egui::{Color32, Ui};
use ze_assets::{
	AssetRef, GridCell, TEXTURE_SHEET_EXTENSION, TextureSheet, TextureSheetMode, UniformGridSheet,
	compute_uniform_grid_trims,
};

use super::texture_sheet_common::{
	DecodedImage, EYEDROPPER_TRANSPARENT_ALPHA_THRESHOLD, load_or_reload_decoded_image, relativize_asset_path,
	sample_pixel,
};

const DEFAULT_CELL_SIZE: u32 = 32;
const DEFAULT_BACKGROUND_COLOR: [f32; 3] = [1.0, 0.0, 1.0];

pub enum TextureSheetCreateOutcome {
	Pending,
	Cancelled,
	Created(PathBuf),
}

#[derive(Debug)]
pub struct TextureSheetCreateDialog {
	source_image_path: PathBuf,
	cell_width: u32,
	cell_height: u32,
	background_color: [f32; 3],
	preview: Option<DecodedImage>,
	cells: Vec<GridCell>,
	last_grid_params: Option<(u32, u32, [f32; 3])>,
	eyedropper_active: bool,
	error: Option<String>,
}

impl TextureSheetCreateDialog {
	pub fn new(source_image_path: PathBuf, assets_root: &Path) -> Self {
		let (cell_width, cell_height, background_color) = seed_defaults(assets_root);
		Self {
			source_image_path,
			cell_width,
			cell_height,
			background_color,
			preview: None,
			cells: Vec::new(),
			last_grid_params: None,
			eyedropper_active: false,
			error: None,
		}
	}

	pub fn show(&mut self, ctx: &egui::Context, assets_root: &Path) -> TextureSheetCreateOutcome {
		let has_preview = load_or_reload_decoded_image(ctx, &self.source_image_path, &mut self.preview).is_some();

		if has_preview {
			let params = (self.cell_width, self.cell_height, self.background_color);
			if self.last_grid_params != Some(params) {
				let rgba = &self.preview.as_ref().expect("has_preview confirmed Some").rgba;
				self.cells =
					compute_uniform_grid_trims(rgba, self.cell_width, self.cell_height, 0, 0, self.background_color);
				self.last_grid_params = Some(params);
			}
		}

		let mut confirm = false;
		let mut cancel = false;

		egui::Window::new("Create Texture Sheet")
			.id(egui::Id::new("texture_sheet_create_dialog"))
			.collapsible(false)
			.resizable(false)
			.default_width(480.0)
			.anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
			.show(ctx, |ui| {
				if has_preview {
					self.show_preview(ui);
				} else {
					ui.colored_label(
						Color32::YELLOW,
						format!("Failed to load source image: {}", self.source_image_path.display()),
					);
				}

				ui.separator();

				ui.horizontal(|ui| {
					ui.label("Cell width");
					ui.add(egui::DragValue::new(&mut self.cell_width).range(1..=8192));
					ui.label("Cell height");
					ui.add(egui::DragValue::new(&mut self.cell_height).range(1..=8192));
				});
				ui.horizontal(|ui| {
					ui.label("Background color");
					ui.color_edit_button_rgb(&mut self.background_color);
					ui.toggle_value(&mut self.eyedropper_active, "Eyedropper");
				});

				if let Some(error) = &self.error {
					ui.colored_label(Color32::YELLOW, error);
				}

				ui.horizontal(|ui| {
					if ui.add_enabled(has_preview, egui::Button::new("OK")).clicked() {
						confirm = true;
					}
					if ui.button("Cancel").clicked() {
						cancel = true;
					}
				});

				cancel |= ui.input(|input| input.key_pressed(egui::Key::Escape));
			});

		if confirm {
			match self.create(assets_root) {
				Ok(path) => return TextureSheetCreateOutcome::Created(path),
				Err(error) => self.error = Some(error),
			}
		} else if cancel {
			return TextureSheetCreateOutcome::Cancelled;
		}

		TextureSheetCreateOutcome::Pending
	}

	fn show_preview(&mut self, ui: &mut Ui) {
		let Some(preview) = self.preview.as_ref() else {
			return;
		};

		let source_size = preview.texture.size();
		let max_width = ui.available_width().min(440.0);
		let scale = max_width / source_size[0] as f32;
		let preview_size = egui::vec2(source_size[0] as f32 * scale, source_size[1] as f32 * scale);

		let image_response = ui.image(egui::load::SizedTexture::new(preview.texture.id(), preview_size));
		let origin = image_response.rect.min;
		let painter = ui.painter_at(image_response.rect);

		let cell_width = self.cell_width.max(1);
		let cell_height = self.cell_height.max(1);
		let (cols, _rows) = ze_assets::uniform_grid_cols_rows(
			source_size[0] as u32,
			source_size[1] as u32,
			cell_width,
			cell_height,
			0,
			0,
		);
		let cols = cols.max(1);

		for (index, cell) in self.cells.iter().enumerate() {
			let Some(trim) = cell.trim else {
				continue;
			};
			let col = index as u32 % cols;
			let row = index as u32 / cols;
			let cell_min = origin
				+ egui::vec2(
					col as f32 * cell_width as f32 * scale,
					row as f32 * cell_height as f32 * scale,
				);
			let trim_rect = egui::Rect::from_min_size(
				cell_min + egui::vec2(trim.offset.0 as f32 * scale, trim.offset.1 as f32 * scale),
				egui::vec2(trim.size.0 as f32 * scale, trim.size.1 as f32 * scale),
			);
			painter.rect_stroke(trim_rect, 0.0, (2.0, Color32::GREEN), egui::StrokeKind::Inside);
		}

		let interact = ui.interact(
			image_response.rect,
			ui.id().with("texture_sheet_create_eyedropper"),
			egui::Sense::click(),
		);
		if self.eyedropper_active
			&& interact.clicked()
			&& let Some(pointer) = interact.interact_pointer_pos()
		{
			let image_point = egui::pos2((pointer.x - origin.x) / scale, (pointer.y - origin.y) / scale);
			if let Some((color, alpha)) = sample_pixel(&preview.rgba, image_point) {
				if alpha <= EYEDROPPER_TRANSPARENT_ALPHA_THRESHOLD {
					self.error = Some(
						"That pixel is transparent -- pick a solid-colored pixel for the background color.".to_string(),
					);
				} else {
					self.background_color = color;
					self.eyedropper_active = false;
					self.error = None;
				}
			}
		}
	}

	fn create(&self, assets_root: &Path) -> Result<PathBuf, String> {
		let relative = relativize_asset_path(assets_root, &self.source_image_path)
			.ok_or_else(|| "Source image is not inside the project's asset directory.".to_string())?;

		let sheet = TextureSheet::new_uniform_grid(UniformGridSheet {
			texture: AssetRef::game(relative),
			cell_width: self.cell_width,
			cell_height: self.cell_height,
			origin_x: 0,
			origin_y: 0,
			background_color: self.background_color,
			cells: self.cells.clone(),
		});

		let stem = self
			.source_image_path
			.file_stem()
			.and_then(|stem| stem.to_str())
			.unwrap_or("texture_sheet");
		let dir = assets_root.join("texture_sheets");

		let mut dest = dir.join(format!("{stem}.{TEXTURE_SHEET_EXTENSION}"));
		let mut suffix = 2;
		while dest.exists() {
			dest = dir.join(format!("{stem}_{suffix}.{TEXTURE_SHEET_EXTENSION}"));
			suffix += 1;
		}

		sheet
			.save(&dest)
			.map_err(|error| format!("Failed to save texture sheet: {error}"))?;

		Ok(dest.canonicalize().unwrap_or(dest))
	}
}

/// Seeds `cell_width`/`cell_height`/`background_color` from the most
/// recently modified `.texturesheet.json` under `texture_sheets/` (matching
/// the last sheet the user created), or a sensible fallback if none exists.
fn seed_defaults(assets_root: &Path) -> (u32, u32, [f32; 3]) {
	let fallback = (DEFAULT_CELL_SIZE, DEFAULT_CELL_SIZE, DEFAULT_BACKGROUND_COLOR);

	let dir = assets_root.join("texture_sheets");
	let Ok(entries) = std::fs::read_dir(&dir) else {
		return fallback;
	};

	let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
	for entry in entries.flatten() {
		let path = entry.path();
		let is_sheet = path
			.file_name()
			.and_then(|name| name.to_str())
			.is_some_and(|name| name.ends_with(&format!(".{TEXTURE_SHEET_EXTENSION}")));
		if !is_sheet {
			continue;
		}
		let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
			continue;
		};
		if newest.as_ref().is_none_or(|(time, _)| modified > *time) {
			newest = Some((modified, path));
		}
	}

	let Some((_, path)) = newest else {
		return fallback;
	};

	match TextureSheet::load(&path).map(|sheet| sheet.mode) {
		Ok(TextureSheetMode::UniformGrid(grid)) => (grid.cell_width, grid.cell_height, grid.background_color),
		_ => fallback,
	}
}
