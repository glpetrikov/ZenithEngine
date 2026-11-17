//! The panels that show a world: the Viewport (the picture, the camera, the
//! transform gizmo), the Hierarchy and the list of worlds.

use dear_imgui_rs::{Condition, Key, MouseButton, Ui, WindowFlags};
use dear_imguizmo::{GuizmoExt, Mode, Operation};
use glam::{Mat4, Vec2, Vec3};

use crate::{
	camera::FlyInput,
	editor::Editor,
	scene_view::{self, GizmoOperation},
};

/// How far in front of the camera `F` puts the selected entity.
const FOCUS_DISTANCE: f64 = 6.0;

/// A size in pixels: at least one.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped to 1 and up first
const fn pixels(size: f32) -> u32 {
	size.max(1.0) as u32
}

/// What the keys ask of a flying camera.
fn fly_input(ui: &Ui) -> FlyInput {
	let key = |key: Key| if ui.is_key_down(key) { 1.0 } else { 0.0 };
	FlyInput {
		movement: Vec3::new(
			key(Key::D) - key(Key::A),
			key(Key::E) - key(Key::Q),
			key(Key::W) - key(Key::S),
		),
		look: Vec2::from(ui.io().mouse_delta()),
		fast: ui.is_key_down(Key::LeftShift),
		scroll: ui.io().mouse_wheel(),
	}
}

/// The Viewport: the world seen by the camera of the editor. Only this panel
/// gets the input of the game, and only while it has the focus.
pub fn viewport(editor: &mut Editor, ui: &Ui) {
	let mut focused = false;
	ui.window("Viewport")
		.size([640.0, 360.0], Condition::FirstUseEver)
		.position([30.0, 60.0], Condition::FirstUseEver)
		.flags(WindowFlags::NO_SCROLLBAR | WindowFlags::NO_SCROLL_WITH_MOUSE)
		.build(|| {
			focused = ui.is_window_focused();
			tools(editor, ui);
			let size = ui.content_region_avail();
			editor.scene.wanted = (pixels(size[0]), pixels(size[1]));
			let Some(texture) = editor.scene.texture else {
				ui.text_disabled(if editor.project.is_some() {
					"No world to draw: File > New World"
				} else {
					"No project is open"
				});
				return;
			};
			let corner = ui.cursor_screen_pos();
			ui.image(texture.texture_id(), size);
			let hovered = ui.is_item_hovered();
			steer(editor, ui, hovered);
			gizmo(editor, ui, corner, size);
		});
	editor.set_viewport_focus(focused);
}

/// The row of buttons over the picture.
fn tools(editor: &mut Editor, ui: &Ui) {
	for (label, operation) in [
		("Move", GizmoOperation::Move),
		("Rotate", GizmoOperation::Rotate),
		("Scale", GizmoOperation::Scale),
	] {
		if ui.radio_button(label, editor.scene.operation == operation) {
			editor.scene.operation = operation;
		}
		ui.same_line();
	}
	ui.checkbox("Local", &mut editor.scene.local);
	ui.same_line();
	ui.text_disabled(format!("Camera speed {:.1}", editor.scene.camera.speed));
}

/// Flying with the right mouse button held, the wheel to come closer, and the
/// keys for the gizmo (W, E, R) and for coming to the selected entity (F).
fn steer(editor: &mut Editor, ui: &Ui, hovered: bool) {
	let scene = &mut editor.scene;
	if hovered && ui.is_mouse_clicked(MouseButton::Right) {
		scene.flying = true;
	}
	if !ui.is_mouse_down(MouseButton::Right) {
		scene.flying = false;
	}
	if scene.flying {
		scene.camera.fly(&fly_input(ui), editor.dt);
		return;
	}
	if !hovered || ui.io().want_text_input() {
		return;
	}
	let wheel = ui.io().mouse_wheel();
	if wheel != 0.0 {
		scene.camera.dolly(wheel);
	}
	if ui.is_key_pressed(Key::W) {
		scene.operation = GizmoOperation::Move;
	}
	if ui.is_key_pressed(Key::E) {
		scene.operation = GizmoOperation::Rotate;
	}
	if ui.is_key_pressed(Key::R) {
		scene.operation = GizmoOperation::Scale;
	}
	if ui.is_key_pressed(Key::F)
		&& let (Some(project), Some(name)) = (&editor.project, &editor.active_world)
		&& let Some(entity) = scene.selection
		&& let Some(transform) = project
			.worlds
			.get(name)
			.and_then(|entry| entry.world.get_component::<zenith_components::Transform>(entity))
	{
		let position = transform.position;
		scene.camera.focus(position, FOCUS_DISTANCE);
	}
}

/// The transform gizmo (`ImGuizmo`) over the picture, for the selected entity.
/// It is drawn by the UI, not by the renderer, and needs only the matrices of
/// the camera.
fn gizmo(editor: &mut Editor, ui: &Ui, corner: [f32; 2], size: [f32; 2]) {
	let (Some(project), Some(name)) = (&editor.project, editor.active_world.clone()) else {
		return;
	};
	let Some(model) = editor.scene.selected_matrix(project, &name) else {
		return;
	};
	let camera = &editor.scene.camera;
	let view = camera.view_matrix().to_cols_array();
	let projection = camera
		.gizmo_projection((pixels(size[0]), pixels(size[1])))
		.to_cols_array();
	let mut matrix = model.to_cols_array();
	let operation = match editor.scene.operation {
		GizmoOperation::Move => Operation::TRANSLATE,
		GizmoOperation::Rotate => Operation::ROTATE,
		GizmoOperation::Scale => Operation::SCALE,
	};
	let mode = if editor.scene.local || editor.scene.operation == GizmoOperation::Scale {
		Mode::Local
	} else {
		Mode::World
	};
	let guizmo = ui.guizmo();
	guizmo.set_rect(corner[0], corner[1], size[0], size[1]);
	guizmo.set_drawlist_window();
	guizmo.enable(!editor.scene.flying);
	let used = guizmo
		.manipulate_config(&view, &projection, &mut matrix)
		.operation(operation)
		.mode(mode)
		.build();
	if used && let Some(project) = &mut editor.project {
		editor
			.scene
			.apply_matrix(project, &name, Mat4::from_cols_array(&matrix));
	}
}

/// The entities of the world that is shown; a click selects one.
pub fn hierarchy(editor: &mut Editor, ui: &Ui) {
	ui.window("Hierarchy")
		.size([260.0, 300.0], Condition::FirstUseEver)
		.position([30.0, 60.0], Condition::FirstUseEver)
		.build(|| {
			let (Some(project), Some(name)) = (&mut editor.project, &editor.active_world) else {
				ui.text_disabled("No world is open");
				return;
			};
			let Some(entry) = project.worlds.get_mut(name) else {
				return;
			};
			ui.text(name);
			ui.separator();
			let entities = scene_view::entities(&mut entry.world);
			if entities.is_empty() {
				ui.text_disabled("Nothing in this world yet");
			}
			for (entity, label) in entities {
				let selected = editor.scene.selection == Some(entity);
				if ui
					.selectable_config(format!("{label}##{}", entity.to_bits()))
					.selected(selected)
					.build()
				{
					editor.scene.selection = Some(entity);
				}
			}
		});
}

/// The worlds of the project, `*` marks the ones with changes. A click shows
/// one in the Viewport.
pub fn worlds(editor: &mut Editor, ui: &Ui) {
	ui.window("Worlds")
		.size([260.0, 300.0], Condition::FirstUseEver)
		.position([30.0, 440.0], Condition::FirstUseEver)
		.build(|| {
			let Some(project) = &editor.project else {
				ui.text_disabled("No project is open");
				return;
			};
			ui.text(project.path.display().to_string());
			ui.separator();
			if project.worlds.is_empty() {
				ui.text_disabled("No worlds yet: File > New World");
			}
			let mut chosen = None;
			for (name, entry) in &project.worlds {
				let label = if entry.dirty { format!("* {name}") } else { name.clone() };
				let shown = editor.active_world.as_deref() == Some(name.as_str());
				if ui.selectable_config(format!("{label}##world")).selected(shown).build() {
					chosen = Some(name.clone());
				}
			}
			if chosen.is_some() && chosen != editor.active_world {
				editor.active_world = chosen;
			}
		});
}
