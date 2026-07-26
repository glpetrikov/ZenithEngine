use egui::{Mesh, PointerButton, Pos2, Rgba, Sense, TextureId, Ui, Vec2, epaint::Vertex};
use transform_gizmo_egui::{
	math::Transform as GizmoTransform,
	prelude::{Gizmo, GizmoConfig, GizmoInteraction, GizmoMode, GizmoOrientation, GizmoResult, mint},
};
use ze_core::{Mat4, Quat, Vec2 as ZeVec2, Vec3};
use ze_ecs::{Collider, ColliderShape, EntitiesView, EntityId, Inactive, SaveFile, Scene, Transform, fit_aspect};
use ze_renderer::{Camera, CameraProjection, Sprite, SpriteSize, reference_aspect};

use super::{EditorPanelContext, EditorSelection, Panel};
use crate::{
	editor_workspace::EditorMode,
	tilemap::{self, PlaceTileParams},
	undo_redo::SceneSnapshotCommand,
};

const MIN_EDITOR_ZOOM: f32 = 0.1;
const MAX_EDITOR_ZOOM: f32 = 1000.0;
const ZOOM_WHEEL_SCALE: f32 = 0.01;

#[derive(Debug, Clone)]
pub struct ViewportOutput {
	pub resolution: [u32; 2],
	/// The viewport image's on-screen rect, in logical (egui) points. Used to
	/// translate window-space pointer events into viewport-local coordinates
	/// for the yakui UI overlay, which is painted into a texture sized to
	/// `resolution`, not the full window.
	pub screen_rect: egui::Rect,
	pub hovered: bool,
	pub focused: bool,
	pub wants_game_input: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportMode {
	TwoDimensional,
	ThreeDimensional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewportTool {
	Select,
	PaintTiles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlacementMode {
	VisualOnly,
	VisualAndCollision,
}

#[derive(Debug)]
struct EditorCameraState {
	mode: ViewportMode,
	zoom: f32,
	yaw: f32,
	pitch: f32,
	mouse_sensitivity: f32,
	focus_point: Option<Vec3>,
	orbit_distance: f32,
}

impl Default for EditorCameraState {
	fn default() -> Self {
		Self {
			mode: ViewportMode::TwoDimensional,
			zoom: 10.0,
			yaw: 0.0,
			pitch: 0.0,
			mouse_sensitivity: 0.005,
			focus_point: None,
			orbit_distance: 10.0,
		}
	}
}

#[derive(Debug)]
pub struct ScenePanel {
	viewport_texture: TextureId,
	viewport_output: Option<ViewportOutput>,
	gizmo: Gizmo,
	gizmo_dragging: bool,
	camera: EditorCameraState,
	rmb_camera_active: bool,
	dragging_entity: Option<EntityId>,
	pending_scene_edit: Option<(SaveFile, bool)>,
	tool: ViewportTool,
	placement_mode: PlacementMode,
	stroke_start_cell: Option<(i32, i32)>,
	erase_stroke_start_cell: Option<(i32, i32)>,
	current_render_layer: i32,
}

impl ScenePanel {
	pub fn new(viewport_texture: TextureId) -> Self {
		Self {
			viewport_texture,
			viewport_output: None,
			gizmo: Gizmo::default(),
			gizmo_dragging: false,
			camera: EditorCameraState::default(),
			rmb_camera_active: false,
			dragging_entity: None,
			pending_scene_edit: None,
			tool: ViewportTool::Select,
			placement_mode: PlacementMode::VisualOnly,
			stroke_start_cell: None,
			erase_stroke_start_cell: None,
			current_render_layer: 0,
		}
	}

	fn begin_scene_edit(&mut self, context: &EditorPanelContext<'_>) {
		if self.pending_scene_edit.is_none() {
			self.pending_scene_edit = Some((context.scene.snapshot(), false));
		}
	}

	const fn mark_scene_edit_dirty(&mut self) {
		if let Some((_before, dirty)) = self.pending_scene_edit.as_mut() {
			*dirty = true;
		}
	}

	fn finish_scene_edit(&mut self, context: &mut EditorPanelContext<'_>) {
		let Some((before, dirty)) = self.pending_scene_edit.take() else {
			return;
		};
		if !dirty {
			return;
		}

		let after = context.scene.snapshot();
		context
			.undo_redo
			.record_executed(Box::new(SceneSnapshotCommand::new(before, after)));
	}
}

impl Panel for ScenePanel {
	fn name(&self) -> &'static str { "Scene" }

	fn show(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>) {
		egui::Frame::NONE.inner_margin(0.0).outer_margin(0.0).show(ui, |ui| {
			// The Scene tab opts out of dock background painting so the viewport can be
			// edge-to-edge.
			ui.painter()
				.rect_filled(ui.max_rect(), egui::CornerRadius::ZERO, crate::style::BG_BASE);
			ui.spacing_mut().item_spacing = Vec2::ZERO;

			self.show_toolbar(ui, context);

			let available_size = ui.available_size();
			let viewport_size = Vec2::new(available_size.x.max(1.0), available_size.y.max(1.0));
			let resolution = [viewport_size.x.round() as u32, viewport_size.y.round() as u32];

			let image_response = ui.image(egui::load::SizedTexture::new(self.viewport_texture, viewport_size));
			let input_response = ui.interact(
				image_response.rect,
				ui.id().with("viewport_input"),
				Sense::click_and_drag(),
			);

			if input_response.clicked() {
				input_response.request_focus();
			}

			if ui.rect_contains_pointer(image_response.rect)
				&& ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary))
			{
				input_response.request_focus();
			}

			if context.mode == EditorMode::Edit {
				self.apply_camera_controls(ui, image_response.rect, &input_response, context);

				if self.tool == ViewportTool::Select {
					let _gizmo_focused = self.show_gizmo(ui, image_response.rect, context);
					let clicked_in_viewport = ui.input(|input| input.pointer.button_clicked(PointerButton::Primary))
						&& ui.rect_contains_pointer(image_response.rect);
					let gizmo_held = self.gizmo_dragging
						&& ui.input(|input| input.pointer.button_down(PointerButton::Primary))
						&& ui.rect_contains_pointer(image_response.rect);

					if clicked_in_viewport && !gizmo_held {
						if let Some(position) = ui.input(|input| input.pointer.interact_pos()) {
							let picked_entity = pick_entity(context.scene, image_response.rect, position);
							*context.selection = picked_entity.map(EditorSelection::Entity);
						} else {
							*context.selection = None;
						}
					}
					self.update_2d_entity_drag(ui, image_response.rect, &input_response, gizmo_held, context);
					if self.pending_scene_edit.is_some()
						&& !ui.input(|input| input.pointer.button_down(PointerButton::Primary))
					{
						self.finish_scene_edit(context);
					}
				} else if self.tool == ViewportTool::PaintTiles && self.camera.mode == ViewportMode::TwoDimensional {
					self.handle_tile_paint(ui, image_response.rect, context);
					self.handle_tile_erase(ui, image_response.rect, context);
					draw_tile_grid_overlay(ui, image_response.rect, context.scene);
				}
			}

			let hovered = ui.rect_contains_pointer(image_response.rect);
			let focused = input_response.has_focus();
			let ctx = ui.ctx();
			let wants_game_input = (hovered || focused)
				&& !ctx.egui_wants_keyboard_input()
				&& (!ctx.egui_wants_pointer_input() || hovered);

			self.viewport_output = Some(ViewportOutput {
				resolution,
				screen_rect: image_response.rect,
				hovered,
				focused,
				wants_game_input,
			});
		});
	}

	fn set_viewport_texture(&mut self, texture: TextureId) { self.viewport_texture = texture; }

	fn take_viewport_output(&mut self) -> Option<ViewportOutput> { self.viewport_output.take() }

	fn clear_background(&self) -> bool { false }

	fn scroll_bars(&self) -> [bool; 2] { [false, false] }
}

impl ScenePanel {
	fn show_toolbar(&mut self, ui: &mut Ui, context: &EditorPanelContext<'_>) {
		ui.horizontal(|ui| {
			let two_dimensional = self.camera.mode == ViewportMode::TwoDimensional;
			if ui.selectable_label(two_dimensional, "2D").clicked() {
				self.camera.mode = ViewportMode::TwoDimensional;
			}
			if ui.selectable_label(!two_dimensional, "3D").clicked() {
				self.camera.mode = ViewportMode::ThreeDimensional;
			}

			ui.separator();

			if ui
				.selectable_label(self.tool == ViewportTool::Select, "Select")
				.clicked()
			{
				self.tool = ViewportTool::Select;
			}

			let brush_available = context.active_tile_brush.is_some();
			ui.add_enabled_ui(brush_available, |ui| {
				if ui
					.selectable_label(self.tool == ViewportTool::PaintTiles, "Paint Tiles")
					.clicked()
				{
					self.tool = ViewportTool::PaintTiles;
				}
			});
			if !brush_available && self.tool == ViewportTool::PaintTiles {
				self.tool = ViewportTool::Select;
			}

			if self.tool == ViewportTool::PaintTiles {
				ui.separator();
				let with_collision = self.placement_mode == PlacementMode::VisualAndCollision;
				if ui.selectable_label(!with_collision, "Visual Only").clicked() {
					self.placement_mode = PlacementMode::VisualOnly;
				}
				if ui.selectable_label(with_collision, "Visual + Collision").clicked() {
					self.placement_mode = PlacementMode::VisualAndCollision;
				}

				ui.separator();
				ui.label("Render Layer");
				ui.add(egui::DragValue::new(&mut self.current_render_layer));
			}
		});
	}

	fn apply_camera_controls(
		&mut self,
		ui: &Ui,
		viewport: egui::Rect,
		input_response: &egui::Response,
		context: &mut EditorPanelContext<'_>,
	) {
		self.camera.zoom = sanitize_zoom(self.camera.zoom);
		sync_camera_projection(context.scene, self.camera.mode, self.camera.zoom);
		self.handle_focus_shortcut(ui, input_response, context);

		let secondary_down = ui.input(|input| input.pointer.button_down(PointerButton::Secondary));
		if !secondary_down {
			self.rmb_camera_active = false;
		} else if ui.rect_contains_pointer(viewport)
			&& ui.input(|input| input.pointer.button_pressed(PointerButton::Secondary))
		{
			self.rmb_camera_active = true;
		}

		if self.camera.mode == ViewportMode::TwoDimensional {
			self.apply_2d_camera_controls(ui, viewport, context);
		} else {
			self.apply_3d_camera_controls(ui, context);
		}
	}

	fn apply_2d_camera_controls(&mut self, ui: &Ui, viewport: egui::Rect, context: &mut EditorPanelContext<'_>) {
		let scroll_delta = ui.input(|input| input.smooth_scroll_delta.y);
		if ui.rect_contains_pointer(viewport) && scroll_delta != 0.0 {
			let pointer = ui.input(|input| input.pointer.hover_pos());
			let anchor_before_zoom =
				pointer.and_then(|pointer| cursor_world_on_2d_plane(context.scene, viewport, pointer, 0.0));
			let zoom_factor = (-scroll_delta * ZOOM_WHEEL_SCALE).exp2();
			self.camera.zoom = sanitize_zoom(self.camera.zoom * zoom_factor);
			sync_camera_projection(context.scene, self.camera.mode, self.camera.zoom);
			if let (Some(pointer), Some(anchor_before_zoom)) = (pointer, anchor_before_zoom)
				&& let Some(anchor_after_zoom) = cursor_world_on_2d_plane(context.scene, viewport, pointer, 0.0)
			{
				move_primary_camera(context.scene, anchor_before_zoom - anchor_after_zoom);
			}
		}

		// Right-click is repurposed for tile erase while Paint Tiles is the
		// active tool (see `handle_tile_erase`), so the camera must not also
		// pan out from under the cursor on the same right-click-drag.
		let pan_active = self.rmb_camera_active
			&& self.tool != ViewportTool::PaintTiles
			&& !ui.input(|input| input.pointer.button_down(PointerButton::Primary));

		if pan_active {
			let delta = ui.input(|input| input.pointer.delta());
			let target_aspect = reference_aspect(context.scene);
			let fitted_size = fitted_viewport(context.scene, viewport).size();
			let world_per_point = ZeVec2::new(
				self.camera.zoom * target_aspect / fitted_size.x,
				self.camera.zoom / fitted_size.y,
			);
			move_primary_camera(
				context.scene,
				Vec3::new(-delta.x * world_per_point.x, delta.y * world_per_point.y, 0.0),
			);
		}
	}

	fn apply_3d_camera_controls(&mut self, ui: &Ui, context: &mut EditorPanelContext<'_>) {
		if !self.rmb_camera_active {
			return;
		}

		let pointer_delta = ui.input(|input| input.pointer.delta());
		self.camera.yaw = pointer_delta.x.mul_add(-self.camera.mouse_sensitivity, self.camera.yaw);
		self.camera.pitch = pointer_delta
			.y
			.mul_add(-self.camera.mouse_sensitivity, self.camera.pitch)
			.clamp(-std::f32::consts::FRAC_PI_2 + 0.01, std::f32::consts::FRAC_PI_2 - 0.01);

		let rotation = Quat::from_rotation_y(self.camera.yaw) * Quat::from_rotation_x(self.camera.pitch);
		if let Some(focus_point) = self.camera.focus_point {
			let forward = rotation * Vec3::NEG_Z;
			let position = focus_point - forward * self.camera.orbit_distance;
			set_primary_camera_transform(context.scene, position, rotation);
		} else {
			set_primary_camera_rotation(context.scene, rotation);
		}
	}

	fn handle_focus_shortcut(
		&mut self,
		ui: &Ui,
		input_response: &egui::Response,
		context: &mut EditorPanelContext<'_>,
	) {
		let wants_focus = ui.input(|input| input.key_pressed(egui::Key::F));
		if !wants_focus
			|| (!input_response.hovered() && !input_response.has_focus())
			|| ui.ctx().egui_wants_keyboard_input()
		{
			return;
		}

		let Some(EditorSelection::Entity(entity)) = context.selection.as_ref() else {
			return;
		};
		let Some(target) = entity_focus_point(context.scene, *entity) else {
			ze_log::warn!(
				"failed to focus selected entity {}; world transform is unavailable",
				entity.index()
			);
			return;
		};

		match self.camera.mode {
			ViewportMode::TwoDimensional => {
				focus_primary_camera_2d(context.scene, target);
			}
			ViewportMode::ThreeDimensional => {
				self.camera.focus_point = Some(target);
				self.camera.orbit_distance =
					camera_distance_to_focus(context.scene, target).unwrap_or(self.camera.zoom);
				let rotation = Quat::from_rotation_y(self.camera.yaw) * Quat::from_rotation_x(self.camera.pitch);
				let forward = rotation * Vec3::NEG_Z;
				let position = target - forward * self.camera.orbit_distance;
				set_primary_camera_transform(context.scene, position, rotation);
			}
		}
	}

	/// Handles the "Paint Tiles" tool: click places one tile at the cell
	/// under the cursor, click-drag fills every cell in the dragged
	/// rectangle (inclusive both ends) in one go. Reuses the same
	/// begin/mark-dirty/finish undo-batching idiom as `update_2d_entity_drag`
	/// and `show_gizmo` in this file, so a whole drag stroke becomes one undo
	/// entry -- `tilemap::place_tile` only reports a change when it actually
	/// replaces/creates a tile, so re-entering an already-painted cell mid-drag
	/// doesn't spuriously dirty the batch.
	fn handle_tile_paint(&mut self, ui: &Ui, viewport: egui::Rect, context: &mut EditorPanelContext<'_>) {
		let Some(brush) = context.active_tile_brush.clone() else {
			self.stroke_start_cell = None;
			return;
		};

		let left_down = ui.input(|input| input.pointer.button_down(PointerButton::Primary));
		if !left_down {
			if self.pending_scene_edit.is_some() {
				self.finish_scene_edit(context);
			}
			self.stroke_start_cell = None;
			return;
		}

		if !ui.rect_contains_pointer(viewport) {
			return;
		}
		let Some(pointer) = ui.input(|input| input.pointer.interact_pos().or_else(|| input.pointer.hover_pos())) else {
			return;
		};
		let Some(world_position) = cursor_world_on_2d_plane(context.scene, viewport, pointer, 0.0) else {
			return;
		};

		let tile_size = tilemap::tile_world_size(context.scene).max(f32::EPSILON);
		let current_cell = world_to_cell(world_position, tile_size);

		if self.stroke_start_cell.is_none() {
			self.begin_scene_edit(context);
		}
		let start_cell = *self.stroke_start_cell.get_or_insert(current_cell);

		let with_collision = self.placement_mode == PlacementMode::VisualAndCollision;
		let render_layer = self.current_render_layer;
		for row in start_cell.1.min(current_cell.1)..=start_cell.1.max(current_cell.1) {
			for col in start_cell.0.min(current_cell.0)..=start_cell.0.max(current_cell.0) {
				let placed = tilemap::place_tile(
					context.scene,
					&PlaceTileParams {
						sheet_path: &brush.sheet_path,
						cell_id: brush.cell_id,
						col,
						row,
						render_layer,
						with_collision,
					},
				);
				if placed {
					self.mark_scene_edit_dirty();
				}
			}
		}
	}

	/// Handles the "Paint Tiles" tool's erase gesture: right-click removes the
	/// tile at the cell under the cursor, right-click-drag clears every cell
	/// in the dragged rectangle (inclusive both ends) in one go. Mirrors
	/// `handle_tile_paint`'s begin/mark-dirty/finish undo-batching idiom,
	/// using its own `erase_stroke_start_cell` so a paint-drag and an
	/// erase-drag never share/clobber each other's stroke-start state.
	fn handle_tile_erase(&mut self, ui: &Ui, viewport: egui::Rect, context: &mut EditorPanelContext<'_>) {
		let right_down = ui.input(|input| input.pointer.button_down(PointerButton::Secondary));
		if !right_down {
			if self.pending_scene_edit.is_some() {
				self.finish_scene_edit(context);
			}
			self.erase_stroke_start_cell = None;
			return;
		}

		if !ui.rect_contains_pointer(viewport) {
			return;
		}
		let Some(pointer) = ui.input(|input| input.pointer.interact_pos().or_else(|| input.pointer.hover_pos())) else {
			return;
		};
		let Some(world_position) = cursor_world_on_2d_plane(context.scene, viewport, pointer, 0.0) else {
			return;
		};

		let tile_size = tilemap::tile_world_size(context.scene).max(f32::EPSILON);
		let current_cell = world_to_cell(world_position, tile_size);

		if self.erase_stroke_start_cell.is_none() {
			self.begin_scene_edit(context);
		}
		let start_cell = *self.erase_stroke_start_cell.get_or_insert(current_cell);

		for row in start_cell.1.min(current_cell.1)..=start_cell.1.max(current_cell.1) {
			for col in start_cell.0.min(current_cell.0)..=start_cell.0.max(current_cell.0) {
				if tilemap::erase_tile(context.scene, col, row) {
					self.mark_scene_edit_dirty();
				}
			}
		}
	}

	fn update_2d_entity_drag(
		&mut self,
		ui: &Ui,
		viewport: egui::Rect,
		input_response: &egui::Response,
		gizmo_active: bool,
		context: &mut EditorPanelContext<'_>,
	) {
		if self.camera.mode != ViewportMode::TwoDimensional || gizmo_active {
			self.dragging_entity = None;
			return;
		}

		let left_down = ui.input(|input| input.pointer.button_down(PointerButton::Primary));
		if !left_down {
			if self.pending_scene_edit.is_some() {
				self.finish_scene_edit(context);
			}
			self.dragging_entity = None;
			return;
		}

		if self.dragging_entity.is_none()
			&& input_response.drag_started_by(PointerButton::Primary)
			&& let (Some(EditorSelection::Entity(selected)), Some(pointer)) =
				(context.selection.as_ref(), input_response.interact_pointer_pos())
			&& pick_entity(context.scene, viewport, pointer).is_some_and(|picked| picked == *selected)
		{
			self.dragging_entity = Some(*selected);
		}

		let Some(entity) = self.dragging_entity else {
			return;
		};
		let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) else {
			return;
		};
		let Some(world_position) = pointer_world_on_entity_z(context.scene, viewport, pointer, entity) else {
			return;
		};

		if self.pending_scene_edit.is_none() {
			self.begin_scene_edit(context);
		}
		self.mark_scene_edit_dirty();

		if let Some(mut transform) = entity_world_transform(context.scene, entity) {
			transform.position.x = world_position.x;
			transform.position.y = world_position.y;
			if let Err(error) = context.scene.set_world_transform(entity, transform) {
				ze_log::warn!("failed to move entity {} in world space: {error:?}", entity.index());
			}
		}
	}

	fn show_gizmo(&mut self, ui: &Ui, viewport: egui::Rect, context: &mut EditorPanelContext<'_>) -> bool {
		let Some(EditorSelection::Entity(entity)) = context.selection.as_ref() else {
			self.gizmo_dragging = false;
			return false;
		};
		let entity = *entity;
		let Some(camera) = primary_camera(context.scene, reference_aspect(context.scene)) else {
			self.gizmo_dragging = false;
			return false;
		};
		let Some(transform) = entity_world_transform(context.scene, entity) else {
			self.gizmo_dragging = false;
			return false;
		};

		self.gizmo.update_config(GizmoConfig {
			view_matrix: row_matrix_from_mat4(camera.view),
			projection_matrix: row_matrix_from_mat4(camera.projection),
			viewport: fitted_viewport(context.scene, viewport),
			modes: GizmoMode::all(),
			orientation: GizmoOrientation::Local,
			..*self.gizmo.config()
		});

		let gizmo_transform = gizmo_transform_from_engine(&transform);
		let interact_result = interact_gizmo_in_viewport(&mut self.gizmo, ui, viewport, &[gizmo_transform]);
		self.gizmo_dragging =
			interact_result.is_some() && ui.input(|input| input.pointer.button_down(PointerButton::Primary));

		if let Some((_result, new_transforms)) = interact_result {
			if self.pending_scene_edit.is_none() {
				self.begin_scene_edit(context);
			}
			if let Some(new_transform) = new_transforms.first() {
				self.mark_scene_edit_dirty();
				apply_world_gizmo_transform(context.scene, entity, *new_transform);
			}
			return true;
		}

		if self.gizmo_dragging {
			self.gizmo_dragging = false;
		}
		if self.pending_scene_edit.is_some() {
			self.finish_scene_edit(context);
		}
		self.gizmo_dragging = false;
		false
	}
}

fn sanitize_zoom(zoom: f32) -> f32 {
	if zoom.is_finite() {
		zoom.clamp(MIN_EDITOR_ZOOM, MAX_EDITOR_ZOOM)
	} else {
		ze_log::warn!("editor camera zoom became invalid; resetting to default");
		10.0
	}
}

#[derive(Debug, Clone, Copy)]
struct CameraMatrices {
	view: Mat4,
	projection: Mat4,
}

/// The sub-rect of `viewport` (in the same egui point units) that the camera
/// actually renders into once letterboxed/pillarboxed to the project's fixed
/// reference aspect ratio -- see `ze_renderer::reference_aspect` and
/// `ze_ecs::fit_aspect`, which this mirrors exactly. Editor screen<->world
/// conversions (picking, dragging, panning, the gizmo, the tile grid overlay)
/// must go through this instead of the raw panel rect, or they drift from
/// what the fixed-reference-aspect render/gameplay path actually shows
/// whenever the panel isn't exactly at the reference aspect ratio -- which is
/// normal for a docked viewport.
fn fitted_viewport(scene: &Scene, viewport: egui::Rect) -> egui::Rect {
	let target_aspect = reference_aspect(scene);
	let (offset, size) = fit_aspect(ZeVec2::new(viewport.width(), viewport.height()), target_aspect);
	egui::Rect::from_min_size(
		egui::pos2(viewport.left() + offset.x, viewport.top() + offset.y),
		egui::vec2(size.x, size.y),
	)
}

fn primary_camera(scene: &Scene, aspect: f32) -> Option<CameraMatrices> {
	let world = scene.world();
	let mut camera_matrices = None;

	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&Inactive>(entity).is_ok() {
				continue;
			}

			let Ok(camera) = world.get::<&Camera>(entity) else {
				continue;
			};

			if !camera.primary {
				continue;
			}

			let Some(transform) = scene.world_transform(entity) else {
				continue;
			};
			camera_matrices = Some(camera_matrices_from_components(&transform, &camera, aspect));
			break;
		}
	});

	camera_matrices
}

fn primary_camera_entity(scene: &Scene) -> Option<EntityId> {
	let world = scene.world();
	let mut primary = None;

	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&Inactive>(entity).is_ok() {
				continue;
			}

			let Ok(camera) = world.get::<&Camera>(entity) else {
				continue;
			};

			if camera.primary {
				primary = Some(entity);
				break;
			}
		}
	});

	primary
}

fn sync_camera_projection(scene: &mut Scene, mode: ViewportMode, zoom: f32) {
	let Some(entity) = primary_camera_entity(scene) else {
		return;
	};

	if let Ok(mut camera) = scene.world_mut().get::<&mut Camera>(entity) {
		camera.projection = match mode {
			ViewportMode::TwoDimensional => CameraProjection::Orthographic {
				size: zoom,
				near: -1000.0,
				far: 1000.0,
			},
			ViewportMode::ThreeDimensional => CameraProjection::Perspective {
				fov_y_radians: 60.0_f32.to_radians(),
				near: 0.01,
				far: 1000.0,
			},
		};
	}

	if mode == ViewportMode::TwoDimensional
		&& let Ok(mut transform) = scene.world_mut().get::<&mut Transform>(entity)
	{
		transform.rotation = Quat::IDENTITY;
		if transform.position.z.abs() < f32::EPSILON {
			transform.position.z = 10.0;
		}
	}
}

fn move_primary_camera(scene: &mut Scene, delta: Vec3) {
	let Some(entity) = primary_camera_entity(scene) else {
		return;
	};
	let Ok(mut transform) = scene.world_mut().get::<&mut Transform>(entity) else {
		return;
	};
	transform.position += delta;
}

fn set_primary_camera_rotation(scene: &mut Scene, rotation: Quat) {
	let Some(entity) = primary_camera_entity(scene) else {
		return;
	};
	let Ok(mut transform) = scene.world_mut().get::<&mut Transform>(entity) else {
		return;
	};
	transform.rotation = rotation;
}

fn set_primary_camera_transform(scene: &mut Scene, position: Vec3, rotation: Quat) {
	let Some(entity) = primary_camera_entity(scene) else {
		return;
	};
	let Ok(mut transform) = scene.world_mut().get::<&mut Transform>(entity) else {
		return;
	};
	transform.position = position;
	transform.rotation = rotation;
}

fn focus_primary_camera_2d(scene: &mut Scene, target: Vec3) {
	let Some(entity) = primary_camera_entity(scene) else {
		return;
	};
	let Ok(mut transform) = scene.world_mut().get::<&mut Transform>(entity) else {
		return;
	};
	transform.position.x = target.x;
	transform.position.y = target.y;
}

fn camera_distance_to_focus(scene: &Scene, target: Vec3) -> Option<f32> {
	let entity = primary_camera_entity(scene)?;
	let transform = scene.world().get::<&Transform>(entity).ok()?;
	let distance = transform.position.distance(target);
	Some(distance.clamp(1.0, 1000.0))
}

fn camera_matrices_from_components(transform: &Transform, camera: &Camera, aspect: f32) -> CameraMatrices {
	let camera_transform = Mat4::from_scale_rotation_translation(Vec3::ONE, transform.rotation, transform.position);
	let view = camera_transform.inverse();
	let projection = match camera.projection {
		CameraProjection::Orthographic { size, near, far } => {
			let half_height = size * 0.5;
			let half_width = half_height * aspect;
			Mat4::orthographic_rh(-half_width, half_width, -half_height, half_height, near, far)
		}
		CameraProjection::Perspective {
			fov_y_radians,
			near,
			far,
		} => Mat4::perspective_rh(fov_y_radians, aspect, near, far),
	};

	CameraMatrices { view, projection }
}

fn entity_world_transform(scene: &Scene, entity: EntityId) -> Option<Transform> { scene.world_transform(entity) }

fn entity_focus_point(scene: &Scene, entity: EntityId) -> Option<Vec3> {
	entity_world_transform(scene, entity).map(|transform| transform.position)
}

fn apply_world_gizmo_transform(scene: &mut Scene, entity: EntityId, gizmo_transform: GizmoTransform) {
	let transform = Transform {
		position: Vec3::new(
			gizmo_transform.translation.x as f32,
			gizmo_transform.translation.y as f32,
			gizmo_transform.translation.z as f32,
		),
		scale: Vec3::new(
			gizmo_transform.scale.x as f32,
			gizmo_transform.scale.y as f32,
			gizmo_transform.scale.z as f32,
		),
		rotation: Quat::from_xyzw(
			gizmo_transform.rotation.v.x as f32,
			gizmo_transform.rotation.v.y as f32,
			gizmo_transform.rotation.v.z as f32,
			gizmo_transform.rotation.s as f32,
		),
	};

	if let Err(error) = scene.set_world_transform(entity, transform) {
		ze_log::warn!(
			"failed to apply world transform to entity {}: {error:?}",
			entity.index()
		);
	}
}

fn interact_gizmo_in_viewport(
	gizmo: &mut Gizmo,
	ui: &Ui,
	viewport: egui::Rect,
	targets: &[GizmoTransform],
) -> Option<(GizmoResult, Vec<GizmoTransform>)> {
	let pointer = ui.input(|input| input.pointer.hover_pos());
	let pointer_in_viewport = pointer.is_some_and(|position| viewport.contains(position));
	let primary_down = ui.input(|input| input.pointer.button_down(PointerButton::Primary));

	let (cursor_pos, hovered, drag_started) = pointer.map_or((Pos2::ZERO, false, false), |cursor_pos| {
		if pointer_in_viewport {
			let interaction = ui.interact(
				egui::Rect::from_center_size(cursor_pos, Vec2::splat(1.0)),
				ui.id().with("_viewport_gizmo_interaction"),
				Sense::click_and_drag(),
			);
			(
				cursor_pos,
				interaction.hovered(),
				interaction.hovered() && ui.input(|input| input.pointer.button_pressed(PointerButton::Primary)),
			)
		} else {
			(cursor_pos, false, false)
		}
	});

	let result = gizmo.update(
		GizmoInteraction {
			cursor_pos: (cursor_pos.x, cursor_pos.y),
			hovered,
			drag_started,
			dragging: primary_down,
		},
		targets,
	);

	draw_gizmo(ui, viewport, gizmo);
	result
}

fn draw_gizmo(ui: &Ui, viewport: egui::Rect, gizmo: &Gizmo) {
	let draw_data = gizmo.draw();
	egui::Painter::new(ui.ctx().clone(), ui.layer_id(), viewport).add(Mesh {
		indices: draw_data.indices,
		vertices: draw_data
			.vertices
			.into_iter()
			.zip(draw_data.colors)
			.map(|(pos, [r, g, b, a])| Vertex {
				pos: pos.into(),
				uv: Pos2::default(),
				color: Rgba::from_rgba_premultiplied(r, g, b, a).into(),
			})
			.collect(),
		..Default::default()
	});
}

fn gizmo_transform_from_engine(transform: &Transform) -> GizmoTransform {
	GizmoTransform::from_scale_rotation_translation(
		mint::Vector3 {
			x: f64::from(transform.scale.x),
			y: f64::from(transform.scale.y),
			z: f64::from(transform.scale.z),
		},
		mint::Quaternion {
			v: mint::Vector3 {
				x: f64::from(transform.rotation.x),
				y: f64::from(transform.rotation.y),
				z: f64::from(transform.rotation.z),
			},
			s: f64::from(transform.rotation.w),
		},
		mint::Vector3 {
			x: f64::from(transform.position.x),
			y: f64::from(transform.position.y),
			z: f64::from(transform.position.z),
		},
	)
}

fn row_matrix_from_mat4(matrix: Mat4) -> mint::RowMatrix4<f64> {
	let columns = matrix.to_cols_array_2d();
	mint::RowMatrix4 {
		x: row_from_columns(columns, 0),
		y: row_from_columns(columns, 1),
		z: row_from_columns(columns, 2),
		w: row_from_columns(columns, 3),
	}
}

fn row_from_columns(columns: [[f32; 4]; 4], row: usize) -> mint::Vector4<f64> {
	mint::Vector4 {
		x: f64::from(columns[0][row]),
		y: f64::from(columns[1][row]),
		z: f64::from(columns[2][row]),
		w: f64::from(columns[3][row]),
	}
}

fn pick_entity(scene: &Scene, viewport: egui::Rect, pointer: egui::Pos2) -> Option<EntityId> {
	let viewport = fitted_viewport(scene, viewport);
	let camera = primary_camera(scene, reference_aspect(scene))?;
	let ray = pointer_ray(camera, viewport, pointer)?;
	let world = scene.world();
	let mut picked = None;

	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&Inactive>(entity).is_ok() {
				continue;
			}

			let Some(transform) = scene.world_transform(entity) else {
				continue;
			};

			let mut distance = if let Ok(sprite) = world.get::<&Sprite>(entity)
				&& let Some(sprite_distance) = intersect_sprite_bounds(ray, &transform, &sprite.size)
			{
				Some(sprite_distance)
			} else {
				None
			};

			if let Ok(collider) = world.get::<&Collider>(entity)
				&& let Some(collider_distance) = intersect_collider_bounds(ray, &transform, &collider.shape)
			{
				distance = Some(distance.map_or(collider_distance, |current| current.min(collider_distance)));
			}

			let Some(distance) = distance else {
				continue;
			};

			if picked.is_none_or(|(_, picked_distance)| distance < picked_distance) {
				picked = Some((entity, distance));
			}
		}
	});

	picked.map(|(entity, _)| entity)
}

#[derive(Debug, Clone, Copy)]
struct Ray {
	origin: Vec3,
	direction: Vec3,
}

fn pointer_ray(camera: CameraMatrices, viewport: egui::Rect, pointer: egui::Pos2) -> Option<Ray> {
	let x = ((pointer.x - viewport.left()) / viewport.width())
		.clamp(0.0, 1.0)
		.mul_add(2.0, -1.0);
	let y = ((pointer.y - viewport.top()) / viewport.height())
		.clamp(0.0, 1.0)
		.mul_add(-2.0, 1.0);
	let inverse_view_projection = (camera.projection * camera.view).inverse();
	let near = inverse_view_projection.project_point3(Vec3::new(x, y, -1.0));
	let far = inverse_view_projection.project_point3(Vec3::new(x, y, 1.0));
	let direction = far - near;

	(direction.length_squared() > f32::EPSILON).then_some(Ray {
		origin: near,
		direction: direction.normalize(),
	})
}

fn pointer_world_on_entity_z(
	scene: &Scene,
	viewport: egui::Rect,
	pointer: egui::Pos2,
	entity: EntityId,
) -> Option<Vec3> {
	let transform = entity_world_transform(scene, entity)?;
	let viewport = fitted_viewport(scene, viewport);
	let camera = primary_camera(scene, reference_aspect(scene))?;
	let ray = pointer_ray(camera, viewport, pointer)?;

	if ray.direction.z.abs() <= f32::EPSILON {
		return None;
	}

	let distance = (transform.position.z - ray.origin.z) / ray.direction.z;
	(distance >= 0.0).then_some(ray.origin + ray.direction * distance)
}

fn cursor_world_on_2d_plane(scene: &Scene, viewport: egui::Rect, pointer: egui::Pos2, plane_z: f32) -> Option<Vec3> {
	let viewport = fitted_viewport(scene, viewport);
	let camera = primary_camera(scene, reference_aspect(scene))?;
	let ray = pointer_ray(camera, viewport, pointer)?;

	if ray.direction.z.abs() <= f32::EPSILON {
		return None;
	}

	let distance = (plane_z - ray.origin.z) / ray.direction.z;
	(distance >= 0.0).then_some(ray.origin + ray.direction * distance)
}

/// Inverse of `tilemap::tile_world_position`: rounds a world-space point to
/// the nearest tile-grid cell for the given tile world size.
fn world_to_cell(world_position: Vec3, tile_world_size: f32) -> (i32, i32) {
	(
		(world_position.x / tile_world_size).round() as i32,
		(world_position.y / tile_world_size).round() as i32,
	)
}

/// Inverse of `pointer_ray`'s screen-to-NDC mapping: projects a world-space
/// point through the camera's view-projection matrix and back into screen
/// space. Used by `draw_tile_grid_overlay` to place grid lines/highlights,
/// since (unlike `pointer_ray`/`cursor_world_on_2d_plane`) nothing in this
/// file previously needed to go from world space back to screen space.
fn world_to_screen(camera: CameraMatrices, viewport: egui::Rect, world: Vec3) -> egui::Pos2 {
	let ndc = (camera.projection * camera.view).project_point3(world);
	egui::pos2(
		ndc.x.mul_add(0.5, 0.5).mul_add(viewport.width(), viewport.left()),
		(1.0 - ndc.y.mul_add(0.5, 0.5)).mul_add(viewport.height(), viewport.top()),
	)
}

/// While the Paint Tiles tool is active in 2D mode and the scene isn't
/// playing (gated at the call site, mirroring `handle_tile_paint`/
/// `handle_tile_erase`'s own gating), draws a light grid spaced by
/// `tilemap::tile_world_size` across the visible viewport, plus a
/// highlighted cell under the cursor -- so the user can see cell boundaries
/// and exactly where a click would place a tile before committing. Purely a
/// visual aid, painted on top of the GPU-rendered `ui.image(...)` viewport
/// the same way `draw_gizmo` overlays its mesh; a tile's footprint is
/// `tile_world_size` centered on `(col, row) * tile_world_size` (see
/// `tilemap::tile_world_position`), so grid lines fall at half-integer
/// multiples of `tile_world_size`, not whole-integer ones.
fn draw_tile_grid_overlay(ui: &Ui, viewport: egui::Rect, scene: &Scene) {
	const MAX_GRID_LINES: i32 = 500;

	let Some(camera) = primary_camera(scene, reference_aspect(scene)) else {
		return;
	};
	let fitted = fitted_viewport(scene, viewport);
	let tile_size = tilemap::tile_world_size(scene).max(f32::EPSILON);

	// World-space bounds of the visible viewport (its four corners projected
	// onto the Z=0 plane), so only grid lines actually on-screen get drawn.
	let corners = [
		viewport.left_top(),
		viewport.right_top(),
		viewport.left_bottom(),
		viewport.right_bottom(),
	];
	let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
	let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
	for corner in corners {
		let Some(world) = cursor_world_on_2d_plane(scene, viewport, corner, 0.0) else {
			return;
		};
		min_x = min_x.min(world.x);
		min_y = min_y.min(world.y);
		max_x = max_x.max(world.x);
		max_y = max_y.max(world.y);
	}

	let first_col = (min_x / tile_size).floor() as i32 - 1;
	let last_col = ((max_x / tile_size).ceil() as i32 + 1).min(first_col + MAX_GRID_LINES);
	let first_row = (min_y / tile_size).floor() as i32 - 1;
	let last_row = ((max_y / tile_size).ceil() as i32 + 1).min(first_row + MAX_GRID_LINES);

	let painter = ui.painter_at(viewport);
	let line_color = egui::Color32::from_white_alpha(24);

	let mut col = first_col;
	while col <= last_col {
		let x = (col as f32 - 0.5) * tile_size;
		let top = world_to_screen(camera, fitted, Vec3::new(x, min_y, 0.0));
		let bottom = world_to_screen(camera, fitted, Vec3::new(x, max_y, 0.0));
		painter.line_segment([top, bottom], (1.0, line_color));
		col += 1;
	}

	let mut row = first_row;
	while row <= last_row {
		let y = (row as f32 - 0.5) * tile_size;
		let left = world_to_screen(camera, fitted, Vec3::new(min_x, y, 0.0));
		let right = world_to_screen(camera, fitted, Vec3::new(max_x, y, 0.0));
		painter.line_segment([left, right], (1.0, line_color));
		row += 1;
	}

	if let Some(pointer) = ui.input(|input| input.pointer.hover_pos())
		&& ui.rect_contains_pointer(viewport)
		&& let Some(world) = cursor_world_on_2d_plane(scene, viewport, pointer, 0.0)
	{
		let (col, row) = world_to_cell(world, tile_size);
		let cell_min = Vec3::new((col as f32 - 0.5) * tile_size, (row as f32 - 0.5) * tile_size, 0.0);
		let cell_max = Vec3::new((col as f32 + 0.5) * tile_size, (row as f32 + 0.5) * tile_size, 0.0);
		let screen_min = world_to_screen(camera, fitted, cell_min);
		let screen_max = world_to_screen(camera, fitted, cell_max);
		let rect = egui::Rect::from_two_pos(screen_min, screen_max);

		painter.rect_filled(rect, 0.0, egui::Color32::from_rgba_unmultiplied(120, 210, 255, 40));
		painter.rect_stroke(
			rect,
			0.0,
			(2.0, egui::Color32::from_rgb(120, 210, 255)),
			egui::StrokeKind::Inside,
		);
	}
}

fn intersect_sprite_bounds(ray: Ray, transform: &Transform, size: &SpriteSize) -> Option<f32> {
	let [width, height] = sprite_pick_size(size);
	let half_extents = (transform.scale.abs() * Vec3::new(width, height, 0.1)) * 0.5;
	let min = transform.position - half_extents;
	let max = transform.position + half_extents;
	intersect_aabb(ray, min, max)
}

fn intersect_collider_bounds(ray: Ray, transform: &Transform, shape: &ColliderShape) -> Option<f32> {
	let (min, max) = collider_local_bounds(shape)?;
	let corners = [
		transform_local_point(transform, ZeVec2::new(min.x, min.y)),
		transform_local_point(transform, ZeVec2::new(max.x, min.y)),
		transform_local_point(transform, ZeVec2::new(max.x, max.y)),
		transform_local_point(transform, ZeVec2::new(min.x, max.y)),
	];
	let min = Vec3::new(
		corners.iter().map(|point| point.x).fold(f32::INFINITY, f32::min),
		corners.iter().map(|point| point.y).fold(f32::INFINITY, f32::min),
		transform.position.z - 0.05,
	);
	let max = Vec3::new(
		corners.iter().map(|point| point.x).fold(f32::NEG_INFINITY, f32::max),
		corners.iter().map(|point| point.y).fold(f32::NEG_INFINITY, f32::max),
		transform.position.z + 0.05,
	);
	intersect_aabb(ray, min, max)
}

fn collider_local_bounds(shape: &ColliderShape) -> Option<(ZeVec2, ZeVec2)> {
	match shape {
		ColliderShape::Box { half_extents } => Some((-*half_extents, *half_extents)),
		ColliderShape::Circle { radius } => {
			let extent = ZeVec2::splat(*radius);
			Some((-extent, extent))
		}
		ColliderShape::Capsule { half_height, radius } => {
			let half_extents = ZeVec2::new(*radius, *half_height + *radius);
			Some((-half_extents, half_extents))
		}
		ColliderShape::ConvexPolygon { points } => {
			let mut min = ZeVec2::splat(f32::INFINITY);
			let mut max = ZeVec2::splat(f32::NEG_INFINITY);

			for point in points {
				min = min.min(*point);
				max = max.max(*point);
			}

			(min.is_finite() && max.is_finite()).then_some((min, max))
		}
	}
}

fn transform_local_point(transform: &Transform, point: ZeVec2) -> Vec3 {
	let scaled = Vec3::new(point.x * transform.scale.x, point.y * transform.scale.y, 0.0);
	transform.position + transform.rotation * scaled
}

const fn sprite_pick_size(size: &SpriteSize) -> [f32; 2] {
	match size {
		SpriteSize::Auto => [1.0, 1.0],
		SpriteSize::Custom { width, height } => [*width, *height],
	}
}

fn intersect_aabb(ray: Ray, min: Vec3, max: Vec3) -> Option<f32> {
	let inverse_direction = Vec3::new(
		1.0 / ray.direction.x.abs().max(f32::EPSILON).copysign(ray.direction.x),
		1.0 / ray.direction.y.abs().max(f32::EPSILON).copysign(ray.direction.y),
		1.0 / ray.direction.z.abs().max(f32::EPSILON).copysign(ray.direction.z),
	);
	let t1 = (min - ray.origin) * inverse_direction;
	let t2 = (max - ray.origin) * inverse_direction;
	let t_min = t1.min(t2);
	let t_max = t1.max(t2);
	let near = t_min.x.max(t_min.y).max(t_min.z);
	let far = t_max.x.min(t_max.y).min(t_max.z);

	(far >= near.max(0.0)).then_some(near.max(0.0))
}
