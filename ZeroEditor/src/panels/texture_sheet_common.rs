//! Shared viewport/preview helpers used by both the Texture Sheet panel
//! (`texture_sheet.rs`) and the "Create Texture Sheet" dialog
//! (`texture_sheet_create_dialog.rs`): raw-pixel-retaining image caching, a
//! self-contained 2D pan/zoom transform, a checkerboard background painter,
//! an eyedropper pixel sampler, and uniform-grid cell math.

use std::{
	path::{Path, PathBuf},
	time::SystemTime,
};

use egui::{Color32, ColorImage, TextureHandle, TextureOptions, Ui};
use image::RgbaImage;

/// A decoded source image cached alongside its uploaded GPU texture. Unlike
/// the plain `(PathBuf, SystemTime, TextureHandle)` caches used elsewhere in
/// this panel (e.g. Auto-pack file previews), this also retains the raw RGBA
/// pixel buffer so pixel-level operations (the eyedropper) don't need to
/// re-decode the file from disk on every click.
pub struct DecodedImage {
	pub path: PathBuf,
	pub modified: SystemTime,
	pub texture: TextureHandle,
	pub rgba: RgbaImage,
}

impl std::fmt::Debug for DecodedImage {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("DecodedImage")
			.field("path", &self.path)
			.field("modified", &self.modified)
			.field("rgba_dimensions", &self.rgba.dimensions())
			.finish_non_exhaustive()
	}
}

/// Loads (or reuses a cached) decoded image into `slot`, reloading whenever
/// `path` or its mtime changes. Returns `None` if the file can't be read or
/// decoded (the slot is left untouched in that case). The returned `bool` is
/// `true` exactly when a (re)decode happened this call -- callers that derive
/// other state from the pixels (e.g. recomputed grid trims) should redo that
/// work only when this is `true`, not on every call.
pub fn load_or_reload_decoded_image<'a>(
	ctx: &egui::Context,
	path: &Path,
	slot: &'a mut Option<DecodedImage>,
) -> Option<(&'a DecodedImage, bool)> {
	let modified = std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok()?;

	let needs_reload = slot
		.as_ref()
		.is_none_or(|decoded| decoded.path != path || decoded.modified != modified);

	if needs_reload {
		let image = image::open(path).ok()?;
		let rgba = image.to_rgba8();
		let (width, height) = rgba.dimensions();
		let color_image = ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
		let texture = ctx.load_texture(path.to_string_lossy(), color_image, TextureOptions::NEAREST);
		*slot = Some(DecodedImage {
			path: path.to_path_buf(),
			modified,
			texture,
			rgba,
		});
	}

	slot.as_ref().map(|decoded| (decoded, needs_reload))
}

/// Zoom is clamped to this range -- distinct from `scene.rs`'s 3D-camera
/// zoom constants, since this is a plain 2D screen-space scale factor, not an
/// orthographic half-size.
pub const MIN_SHEET_ZOOM: f32 = 0.05;
pub const MAX_SHEET_ZOOM: f32 = 64.0;
const SHEET_ZOOM_WHEEL_SCALE: f32 = 0.01;

/// A self-contained 2D pan/zoom transform for a viewport widget. Not
/// reusable from `scene.rs`'s `EditorCameraState` -- that one drives an ECS
/// camera entity and a GPU-rendered viewport texture via ray-casting; this is
/// plain screen-space arithmetic over a single static image.
#[derive(Debug, Clone, Copy)]
pub struct ViewportTransform {
	pub zoom: f32,
	pub pan: egui::Vec2,
}

impl ViewportTransform {
	/// A centered transform that fits `image_size` fully inside
	/// `viewport_rect`.
	pub fn fit(image_size: egui::Vec2, viewport_rect: egui::Rect) -> Self {
		let zoom = if image_size.x > 0.0 && image_size.y > 0.0 {
			(viewport_rect.width() / image_size.x)
				.min(viewport_rect.height() / image_size.y)
				.clamp(MIN_SHEET_ZOOM, MAX_SHEET_ZOOM)
		} else {
			1.0
		};
		let scaled = image_size * zoom;
		let pan = (viewport_rect.size() - scaled) * 0.5;
		Self { zoom, pan }
	}

	pub fn image_to_screen(&self, viewport_rect: egui::Rect, image_point: egui::Pos2) -> egui::Pos2 {
		viewport_rect.min + self.pan + image_point.to_vec2() * self.zoom
	}

	pub fn screen_to_image(&self, viewport_rect: egui::Rect, screen_point: egui::Pos2) -> egui::Pos2 {
		((screen_point - viewport_rect.min - self.pan) / self.zoom).to_pos2()
	}

	/// Drives pan (drag) and cursor-anchored zoom (scroll) from the
	/// viewport's own interaction `Response`.
	///
	/// The viewport sits inside an outer `egui::ScrollArea` (the rest of the
	/// panel's settings scroll below it), so scroll events consumed here for
	/// zoom are also zeroed out afterward -- otherwise the same wheel event
	/// would additionally scroll the outer area.
	pub fn apply_input(&mut self, ui: &Ui, response: &egui::Response, viewport_rect: egui::Rect) {
		if response.dragged() {
			self.pan += response.drag_delta();
		}

		if response.hovered() {
			let scroll = ui.input(|input| input.smooth_scroll_delta.y);
			if scroll != 0.0 {
				if let Some(pointer) = response.hover_pos() {
					let anchor = self.screen_to_image(viewport_rect, pointer);
					let zoom_factor = (-scroll * SHEET_ZOOM_WHEEL_SCALE).exp2();
					self.zoom = (self.zoom * zoom_factor).clamp(MIN_SHEET_ZOOM, MAX_SHEET_ZOOM);
					self.pan += pointer - self.image_to_screen(viewport_rect, anchor);
				}
				ui.input_mut(|input| input.smooth_scroll_delta.y = 0.0);
			}
		}
	}
}

/// Paints a fixed-screen-space-tile checkerboard across `rect`, signaling
/// transparency the same way other image editors conventionally do. Tile
/// size is not scaled by zoom, so it always reads as a flat page-level
/// background rather than part of the zoomed content.
pub fn draw_checkerboard(painter: &egui::Painter, rect: egui::Rect, tile_size: f32) {
	let dark = Color32::from_gray(45);
	let light = Color32::from_gray(60);
	painter.rect_filled(rect, 0.0, dark);

	let tile_size = tile_size.max(1.0);
	let cols = (rect.width() / tile_size).ceil() as i32;
	let rows = (rect.height() / tile_size).ceil() as i32;
	for row in 0..rows {
		for col in 0..cols {
			if (row + col) % 2 == 0 {
				continue;
			}
			let min = rect.min + egui::vec2(col as f32 * tile_size, row as f32 * tile_size);
			let tile_rect = egui::Rect::from_min_size(min, egui::vec2(tile_size, tile_size)).intersect(rect);
			painter.rect_filled(tile_rect, 0.0, light);
		}
	}
}

/// Bounds-checked RGB sample at `image_point`, as `background_color`'s
/// `0.0..=1.0` per-channel convention (alpha is ignored).
pub fn sample_pixel(rgba: &RgbaImage, image_point: egui::Pos2) -> Option<[f32; 3]> {
	if image_point.x < 0.0 || image_point.y < 0.0 {
		return None;
	}
	let x = image_point.x as u32;
	let y = image_point.y as u32;
	if x >= rgba.width() || y >= rgba.height() {
		return None;
	}
	let pixel = rgba.get_pixel(x, y);
	Some([
		f32::from(pixel[0]) / 255.0,
		f32::from(pixel[1]) / 255.0,
		f32::from(pixel[2]) / 255.0,
	])
}

/// Inverse of the row-major `col = index % cols; row = index / cols` layout
/// used to lay out a `UniformGrid` sheet's cells. Returns `None` outside the
/// `cols x rows` grid.
pub fn pick_uniform_grid_cell(
	image_point: egui::Pos2,
	cell_width: u32,
	cell_height: u32,
	cols: u32,
	rows: u32,
) -> Option<u32> {
	if image_point.x < 0.0 || image_point.y < 0.0 {
		return None;
	}
	let col = (image_point.x / cell_width.max(1) as f32) as u32;
	let row = (image_point.y / cell_height.max(1) as f32) as u32;
	if col >= cols || row >= rows {
		return None;
	}
	Some(row * cols + col)
}

/// Strips `assets_root` off `absolute`, returning a `/`-joined asset-relative
/// path (the convention `AssetRef::path`/`TextureSource::SheetCell::sheet_path`
/// already use), or `None` if `absolute` isn't under `assets_root` (both
/// paths must be in the same absolute/canonical form for this to succeed).
pub fn relativize_asset_path(assets_root: &Path, absolute: &Path) -> Option<String> {
	let relative = absolute.strip_prefix(assets_root).ok()?;
	let mut parts = Vec::new();
	for component in relative.components() {
		let std::path::Component::Normal(part) = component else {
			return None;
		};
		parts.push(part.to_str()?);
	}
	(!parts.is_empty()).then(|| parts.join("/"))
}

/// Recursively scans `assets_root` for files whose name ends with `extension`
/// (e.g. `"animationclip.json"`), returning project-relative path strings
/// (the `AssetRef::path` convention), sorted for a stable combo-box order.
/// No caching -- this backs an occasionally-shown picker, not a hot path.
fn scan_assets_with_extension(assets_root: &Path, extension: &str) -> Vec<String> {
	fn visit(dir: &Path, assets_root: &Path, extension: &str, out: &mut Vec<String>) {
		let Ok(entries) = std::fs::read_dir(dir) else {
			return;
		};
		for entry in entries.flatten() {
			let path = entry.path();
			if path.is_dir() {
				visit(&path, assets_root, extension, out);
			} else if path
				.file_name()
				.and_then(|name| name.to_str())
				.is_some_and(|name| name.ends_with(extension))
				&& let Some(relative) = relativize_asset_path(assets_root, &path)
			{
				out.push(relative);
			}
		}
	}

	let mut paths = Vec::new();
	visit(assets_root, assets_root, extension, &mut paths);
	paths.sort();
	paths
}

/// A combo box listing every asset under `assets_root` matching `extension`,
/// backing e.g. the Animation Clip panel's source-sheet field and the
/// Animator Inspector's per-state clip picker -- there's no generic
/// "pick an asset" widget elsewhere in this codebase (`Sprite.texture` is
/// edited via a raw text field), so this is deliberately minimal: no
/// thumbnails, just relative paths. Returns `true` if `selected` changed.
pub fn asset_picker(
	ui: &mut Ui,
	id_source: impl std::hash::Hash,
	assets_root: &Path,
	extension: &str,
	selected: &mut String,
) -> bool {
	let mut changed = false;
	let options = scan_assets_with_extension(assets_root, extension);
	let selected_text = if selected.is_empty() {
		"(none)"
	} else {
		selected.as_str()
	};

	egui::ComboBox::from_id_salt(id_source)
		.selected_text(selected_text)
		.show_ui(ui, |ui| {
			for option in &options {
				if ui.selectable_label(selected == option, option).clicked() && selected != option {
					*selected = option.clone();
					changed = true;
				}
			}
		});

	changed
}
