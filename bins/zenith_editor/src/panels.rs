//! The panels and the dialogs of the editor.

use std::sync::LazyLock;

use dear_imgui_rs::{Condition, DockLayout, DockLayoutApply, DockNodeFlags, DockSplit, Ui, WindowFlags, WindowKey};
use web_time::{SystemTime, UNIX_EPOCH};
use zenith_input::{ZGamePad, ZGamePadAxis, ZStick};

use crate::{
	editor::{EMULATED_PAD, Editor},
	graph_panel, toolbar, viewport_panel,
};

const WELCOME: &str = "Welcome";
const NEW_WORLD: &str = "New World";
const QUIT: &str = "Unsaved changes";
const BACKUP: &str = "Unsaved changes found";
const PROBLEMS: &str = "Problems";
const LOOSE_MODELS: &str = "Models in Assets";
const DIALOGS: [&str; 6] = [WELCOME, NEW_WORLD, QUIT, BACKUP, PROBLEMS, LOOSE_MODELS];
const STATUSBAR_HEIGHT: f32 = 24.0;

/// A key of a panel: the layout of the dock and the saved layout find the
/// panel by it, so the title may change.
#[allow(clippy::expect_used)] // the names are fixed here and valid
fn panel_key(id: &str, title: &str) -> WindowKey {
	WindowKey::new(id, title).expect("the key of a panel is valid")
}

pub static VIEWPORT: LazyLock<WindowKey> = LazyLock::new(|| panel_key("viewport", "Viewport"));
pub static HIERARCHY: LazyLock<WindowKey> = LazyLock::new(|| panel_key("hierarchy", "Hierarchy"));
pub static WORLDS: LazyLock<WindowKey> = LazyLock::new(|| panel_key("worlds", "Worlds"));
pub static INPUT: LazyLock<WindowKey> = LazyLock::new(|| panel_key("input", "Input"));

/// The whole UI of one frame.
pub fn draw(editor: &mut Editor, ui: &Ui) {
	toolbar::draw_toolbar(editor, ui);
	dockspace(ui);
	statusbar(ui);
	viewport_panel::hierarchy(editor, ui);
	viewport_panel::viewport(editor, ui);
	input_debug(editor, ui);
	viewport_panel::worlds(editor, ui);
	graph_panel::draw(editor, ui);
	dialogs(editor, ui);
}

fn statusbar(ui: &Ui) {
	let viewport = ui.main_viewport();
	let pos = [
		viewport.pos()[0],
		viewport.pos()[1] + viewport.size()[1] - STATUSBAR_HEIGHT - 5.0,
	];
	let size = [viewport.size()[0], STATUSBAR_HEIGHT];
	ui.window("Statusbar")
		.position(pos, Condition::Always)
		.size(size, Condition::Always)
		.flags(
			WindowFlags::NO_TITLE_BAR
				| WindowFlags::NO_RESIZE
				| WindowFlags::NO_MOVE
				| WindowFlags::NO_COLLAPSE
				| WindowFlags::NO_SCROLLBAR
				| WindowFlags::NO_SAVED_SETTINGS
				| WindowFlags::NO_DOCKING
				| WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS,
		)
		.build(|| {
			ui.text_disabled("Zenith Engine 0.1.0");
			ui.same_line();
		});
}

fn input_debug(editor: &mut Editor, ui: &Ui) {
	ui.window(&*INPUT)
		.size([380.0, 460.0], Condition::FirstUseEver)
		.position([700.0, 60.0], Condition::FirstUseEver)
		.build(|| {
			ui.text(if editor.viewport_focused {
				"Viewport focused: input is read"
			} else {
				"Focus the Viewport to send input"
			});
			ui.separator();

			let keys: Vec<String> = editor.input.pressed_keys().map(|k| k.to_string()).collect();
			ui.text(format!(
				"Keys: {}",
				if keys.is_empty() {
					"-".to_string()
				} else {
					keys.join(" ")
				}
			));
			let delta = editor.input.mouse_delta();
			ui.text(format!("Mouse moved: {:.1}, {:.1}", delta.x, delta.y));
			let text = editor.input.text();
			if !text.is_empty() {
				ui.text(format!("Typed: {text}"));
			}

			ui.separator();
			ui.text("Gamepads");
			let pads = editor.input.list_gamepads();
			if pads.is_empty() {
				ui.text_disabled("none connected");
			}
			for info in &pads {
				let Some(pad) = editor.input.gamepad(info.id) else {
					continue;
				};
				ui.text(format!("{}: {}", info.id, info.name));
				let held: Vec<&str> = ZGamePad::ALL
					.iter()
					.filter(|b| pad.is_pressed(**b))
					.map(|b| b.name())
					.collect();
				let left = pad.stick(ZStick::Left);
				let right = pad.stick(ZStick::Right);
				ui.text(format!(
					"  held: {}  left: {:.2}, {:.2}  right: {:.2}, {:.2}  triggers: {:.2}, {:.2}",
					if held.is_empty() {
						"-".to_string()
					} else {
						held.join(" ")
					},
					left.x,
					left.y,
					right.x,
					right.y,
					pad.axis(ZGamePadAxis::LeftTrigger),
					pad.axis(ZGamePadAxis::RightTrigger),
				));
			}

			ui.separator();
			emulated_pad(editor, ui);
		});
}

/// A gamepad made of widgets. It works whether or not the Viewport has the
/// focus: pressing its buttons is the point.
fn emulated_pad(editor: &mut Editor, ui: &Ui) {
	let mut connected = editor.pad.connected;
	if ui.checkbox("Emulated gamepad", &mut connected) {
		editor.set_pad_connected(connected);
	}
	if !editor.pad.connected {
		return;
	}
	ui.text_disabled(format!("Gamepad {EMULATED_PAD}: hold a button to press it"));
	for (i, button) in ZGamePad::ALL.iter().enumerate() {
		if i % 4 != 0 {
			ui.same_line();
		}
		ui.button(button.name());
		let held = ui.is_item_active();
		editor.set_pad_button(*button, held);
	}
	let sticks = [
		("Left", ZGamePadAxis::LeftStickX, ZGamePadAxis::LeftStickY),
		("Right", ZGamePadAxis::RightStickX, ZGamePadAxis::RightStickY),
	];
	for (index, (name, x_axis, y_axis)) in sticks.into_iter().enumerate() {
		let mut value = if index == 0 { editor.pad.left } else { editor.pad.right };
		let mut changed = ui.slider_f32(format!("{name} X"), &mut value[0], -1.0, 1.0);
		changed |= ui.slider_f32(format!("{name} Y"), &mut value[1], -1.0, 1.0);
		if ui.button(format!("Center {name}")) {
			value = [0.0, 0.0];
			changed = true;
		}
		if changed {
			if index == 0 {
				editor.pad.left = value;
			} else {
				editor.pad.right = value;
			}
			editor.set_pad_axis(x_axis, value[0]);
			editor.set_pad_axis(y_axis, value[1]);
		}
	}
	for (index, (name, axis)) in [
		("Left trigger", ZGamePadAxis::LeftTrigger),
		("Right trigger", ZGamePadAxis::RightTrigger),
	]
	.into_iter()
	.enumerate()
	{
		let mut value = editor.pad.triggers[index];
		if ui.slider_f32(name, &mut value, 0.0, 1.0) {
			editor.pad.triggers[index] = value;
			editor.set_pad_axis(axis, value);
		}
	}
}

// --- dialogs
// -------------------------------------------------------------------

/// Opens the dialog that was asked for (one at a time) and draws the ones
/// that are open.
fn dialogs(editor: &mut Editor, ui: &Ui) {
	let open = DIALOGS.iter().any(|name| ui.is_popup_open(*name));
	editor.dialog_open = open;
	if !open {
		if editor.requests.backup {
			editor.requests.backup = false;
			ui.open_popup(BACKUP);
		} else if editor.requests.problems {
			editor.requests.problems = false;
			ui.open_popup(PROBLEMS);
		} else if editor.requests.loose_models {
			editor.requests.loose_models = false;
			ui.open_popup(LOOSE_MODELS);
		} else if editor.requests.quit {
			editor.requests.quit = false;
			ui.open_popup(QUIT);
		} else if editor.requests.new_world {
			editor.requests.new_world = false;
			ui.open_popup(NEW_WORLD);
		} else if editor.welcome_open {
			editor.welcome_open = false;
			ui.open_popup(WELCOME);
		}
	}
	// A dialog is drawn under the docked panels unless it is brought to the
	// front every frame: the panels take the front back when they are drawn.
	for name in DIALOGS {
		if ui.is_popup_open(name) {
			ui.set_window_focus(Some(name));
		}
	}
	welcome(ui);
	new_world(editor, ui);
	quit(editor, ui);
	backup_offer(editor, ui);
	problems(editor, ui);
	loose_models(editor, ui);
}

fn welcome(ui: &Ui) {
	if let Some(_token) = ui.begin_modal_popup(WELCOME) {
		ui.text("Welcome to Zenith Editor!");
		ui.spacing();
		if ui.button("OK") {
			ui.close_current_popup();
		}
	}
}

fn new_world(editor: &mut Editor, ui: &Ui) {
	if let Some(_token) = ui.begin_modal_popup(NEW_WORLD) {
		ui.text("A new empty world. It is saved with the others (Ctrl+S).");
		ui.spacing();
		ui.input_text("Name", &mut editor.new_world.name).build();
		ui.input_text("Folder (optional)", &mut editor.new_world.folder).build();
		ui.text_disabled("Inside Worlds/, such as Caves or Caves/Deep");
		if let Some(error) = &editor.new_world.error {
			ui.text_colored([1.0, 0.4, 0.4, 1.0], error);
		}
		ui.spacing();
		if ui.button("Create") && editor.create_world() {
			ui.close_current_popup();
		}
		ui.same_line();
		if ui.button("Cancel") {
			editor.new_world = crate::editor::NewWorldForm::default();
			ui.close_current_popup();
		}
	}
}

fn quit(editor: &mut Editor, ui: &Ui) {
	if let Some(_token) = ui.begin_modal_popup(QUIT) {
		ui.text("These worlds have changes that are not saved:");
		if let Some(project) = &editor.project {
			for name in project.dirty_names() {
				ui.text(format!("  {name}"));
			}
		}
		ui.spacing();
		if ui.button("Save all and quit") {
			editor.save_and_exit();
			ui.close_current_popup();
		}
		ui.same_line();
		if ui.button("Quit without saving") {
			editor.exit = true;
			ui.close_current_popup();
		}
		ui.same_line();
		if ui.button("Cancel") {
			ui.close_current_popup();
		}
	}
}

fn backup_offer(editor: &mut Editor, ui: &Ui) {
	if let Some(_token) = ui.begin_modal_popup(BACKUP) {
		if let Some(info) = &editor.backup_offer {
			ui.text(format!(
				"The editor was stopped {} with changes that were not saved:",
				age(info.saved_at_ms)
			));
			for name in &info.worlds {
				ui.text(format!("  {name}"));
			}
		}
		ui.spacing();
		ui.text_wrapped(
			"Restore puts them in the project and deletes the backup. Later keeps the backup for the next time.",
		);
		ui.spacing();
		if ui.button("Restore") {
			editor.accept_backup();
			ui.close_current_popup();
		}
		ui.same_line();
		if ui.button("Discard") {
			editor.discard_backup();
			ui.close_current_popup();
		}
		ui.same_line();
		if ui.button("Later") {
			editor.backup_offer = None;
			ui.close_current_popup();
		}
	}
}

fn problems(editor: &mut Editor, ui: &Ui) {
	if let Some(_token) = ui.begin_modal_popup(PROBLEMS) {
		for message in &editor.problems {
			ui.text_wrapped(message);
		}
		ui.spacing();
		if ui.button("OK") {
			editor.problems.clear();
			ui.close_current_popup();
		}
	}
}

fn loose_models(editor: &mut Editor, ui: &Ui) {
	if let Some(_token) = ui.begin_modal_popup(LOOSE_MODELS) {
		ui.text("These model files are in Assets but are not assets yet:");
		for model in editor.loose_models.iter().filter(|model| model.taken_by.is_none()) {
			ui.text(format!("  {}", model.path));
		}
		let skipped: Vec<_> = editor
			.loose_models
			.iter()
			.filter_map(|model| Some((&model.path, model.taken_by.as_ref()?)))
			.collect();
		if !skipped.is_empty() {
			ui.spacing();
			ui.text_disabled("Left alone, the asset they would become is there already:");
			for (path, taken_by) in skipped {
				ui.text_disabled(format!("  {path} ({taken_by})"));
			}
		}
		ui.spacing();
		ui.text_wrapped(
			"Import makes a .zenithmodel next to each, with its materials and textures, and moves the file to the trash of the project, where it can be restored from. The files it uses (.mtl, .bin, pictures) stay.",
		);
		ui.spacing();
		if ui.button("Import") {
			editor.import_loose_models();
			ui.close_current_popup();
		}
		ui.same_line();
		if ui.button("Not now") {
			editor.loose_models.clear();
			ui.close_current_popup();
		}
		ui.same_line();
		if ui.button("Never for this project") {
			editor.never_import_loose_models();
			ui.close_current_popup();
		}
	}
}

/// "5 minutes ago", for the time a backup was made.
fn age(saved_at_ms: u64) -> String {
	let now = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX));
	let seconds = now.saturating_sub(saved_at_ms) / 1000;
	match seconds {
		0..=59 => "just now".to_string(),
		60..=3599 => format!("{} minutes ago", seconds / 60),
		3600..=86_399 => format!("{} hours ago", seconds / 3600),
		_ => format!("{} days ago", seconds / 86_400),
	}
}

fn dockspace(ui: &Ui) {
	let viewport = ui.main_viewport();
	let menu_bar_height = ui.frame_height();
	let pos = [viewport.pos()[0], viewport.pos()[1] + menu_bar_height];
	let size = [
		viewport.size()[0],
		viewport.size()[1] - menu_bar_height - STATUSBAR_HEIGHT - 5.0,
	];
	ui.window("ZenithDockspace")
		.position(pos, Condition::Always)
		.size(size, Condition::Always)
		.flags(
			WindowFlags::NO_TITLE_BAR
				| WindowFlags::NO_COLLAPSE
				| WindowFlags::NO_RESIZE
				| WindowFlags::NO_MOVE
				| WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS
				| WindowFlags::NO_NAV_FOCUS,
		)
		.build(|| {
			let content = ui.content_region_avail();
			// The root ID is needed: without it no dockspace is made, and the
			// docked panels hang on a node that follows no window.
			let root = ui.get_id("ZenithDockspaceRoot");
			// The first start (no saved layout): Hierarchy on the left, Input
			// on the right, the Viewport in the middle and the Worlds under it.
			let layout = DockLayout::split(
				DockSplit::Right,
				0.2,
				DockLayout::tabs([&*INPUT]),
				DockLayout::split(
					DockSplit::Left,
					0.2,
					DockLayout::tabs([&*HIERARCHY]),
					DockLayout::split(
						DockSplit::Down,
						0.25,
						DockLayout::tabs([&*WORLDS]),
						DockLayout::tabs([&*VIEWPORT]),
					),
				),
			);
			let _ = ui
				.dockspace()
				.current_window(content)
				.root_id(root)
				.layout(&layout, DockLayoutApply::IfMissing)
				.flags(DockNodeFlags::empty())
				.build();
		});
}

/// The tests that make a UI context take turns: there is one at a time.
#[cfg(test)]
pub static UI_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
	use dear_imgui_rs::MouseButton;
	use dear_imgui_wgpu::{WgpuInitInfo, WgpuRenderer, wgpu};

	use super::*;

	type TestResult = Result<(), Box<dyn std::error::Error>>;

	fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
		let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
		pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
	}

	#[test]
	fn the_menu_opens_when_it_is_clicked() -> TestResult {
		let _one_context = UI_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
		let Some((device, queue)) = gpu() else {
			return Ok(());
		};
		let mut context = dear_imgui_rs::Context::create();
		let _ = context.set_ini_filename(None::<std::path::PathBuf>);
		let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
		context.io_mut().set_config_flags(flags);
		let mut backend = WgpuRenderer::new(
			WgpuInitInfo::new(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb),
			&mut context,
		)?;
		let mut editor = Editor::new(None);
		editor.graph.init(&context);
		editor.welcome_open = false;

		let mut opened_at = None;
		let mut run =
			|context: &mut dear_imgui_rs::Context, editor: &mut Editor| -> Result<bool, Box<dyn std::error::Error>> {
				context.io_mut().set_display_size([1280.0, 720.0]);
				context.io_mut().set_delta_time(1.0 / 60.0);
				let frame = context.try_begin_frame()?;
				draw(editor, frame.ui());
				let open = frame.ui().is_popup_open_with_flags(
					"",
					dear_imgui_rs::PopupQueryFlags::ANY_POPUP_ID | dear_imgui_rs::PopupQueryFlags::ANY_POPUP_LEVEL,
				);
				let pending = frame.render(backend.renderer_consumer()?);
				drop(backend.reconcile_frame(pending)?);
				Ok(open)
			};
		for _ in 0..5 {
			run(&mut context, &mut editor)?;
		}
		// Click along the menu bar until a menu opens.
		for x in (40..400).step_by(10) {
			context.io_mut().add_mouse_pos_event([x as f32, 10.0]);
			run(&mut context, &mut editor)?;
			context.io_mut().add_mouse_button_event(MouseButton::Left, true);
			run(&mut context, &mut editor)?;
			context.io_mut().add_mouse_button_event(MouseButton::Left, false);
			if run(&mut context, &mut editor)? {
				opened_at = Some(x);
				break;
			}
		}
		assert!(opened_at.is_some(), "no menu opened by a click in the menu bar");
		editor.graph.shutdown();
		backend.shutdown(&mut context)?;
		Ok(())
	}
}

#[cfg(test)]
mod welcome_tests {
	use dear_imgui_rs::MouseButton;
	use dear_imgui_wgpu::{WgpuInitInfo, WgpuRenderer, wgpu};

	use super::*;

	#[test]
	fn the_welcome_dialog_can_be_closed_with_its_button() -> Result<(), Box<dyn std::error::Error>> {
		let _one_context = UI_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
		let Some(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()
		else {
			return Ok(());
		};
		let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
		let mut context = dear_imgui_rs::Context::create();
		let _ = context.set_ini_filename(None::<std::path::PathBuf>);
		let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
		context.io_mut().set_config_flags(flags);
		let mut backend = WgpuRenderer::new(
			WgpuInitInfo::new(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb),
			&mut context,
		)?;
		let mut editor = Editor::new(None);
		editor.graph.init(&context);
		let any = dear_imgui_rs::PopupQueryFlags::ANY_POPUP_ID | dear_imgui_rs::PopupQueryFlags::ANY_POPUP_LEVEL;
		let mut run =
			|context: &mut dear_imgui_rs::Context, editor: &mut Editor| -> Result<bool, Box<dyn std::error::Error>> {
				context.io_mut().set_display_size([1280.0, 720.0]);
				context.io_mut().set_delta_time(1.0 / 60.0);
				let frame = context.try_begin_frame()?;
				draw(editor, frame.ui());
				let open = frame.ui().is_popup_open_with_flags("", any);
				let pending = frame.render(backend.renderer_consumer()?);
				drop(backend.reconcile_frame(pending)?);
				Ok(open)
			};
		for _ in 0..5 {
			run(&mut context, &mut editor)?;
		}
		assert!(run(&mut context, &mut editor)?, "the welcome dialog is open");
		let mut closed = false;
		'search: for y in (250..450).step_by(10) {
			for x in (500..800).step_by(10) {
				context.io_mut().add_mouse_pos_event([x as f32, y as f32]);
				run(&mut context, &mut editor)?;
				context.io_mut().add_mouse_button_event(MouseButton::Left, true);
				run(&mut context, &mut editor)?;
				context.io_mut().add_mouse_button_event(MouseButton::Left, false);
				if !run(&mut context, &mut editor)? {
					closed = true;
					break 'search;
				}
			}
		}
		assert!(closed, "no click closed the welcome dialog");
		editor.graph.shutdown();
		backend.shutdown(&mut context)?;
		Ok(())
	}
}
