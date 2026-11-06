//! The panels and the dialogs of the editor.

use dear_imgui_rs::{Condition, DockNodeFlags, Ui, WindowFlags};
use web_time::{SystemTime, UNIX_EPOCH};
use zenith_input::{ZGamePad, ZGamePadAxis, ZStick};

use crate::{
	editor::{EMULATED_PAD, Editor},
	toolbar,
};

const WELCOME: &str = "Welcome";
const NEW_WORLD: &str = "New World";
const QUIT: &str = "Unsaved changes";
const BACKUP: &str = "Unsaved changes found";
const PROBLEMS: &str = "Problems";
const DIALOGS: [&str; 5] = [WELCOME, NEW_WORLD, QUIT, BACKUP, PROBLEMS];
const STATUSBAR_HEIGHT: f32 = 24.0;

/// The whole UI of one frame.
pub fn draw(editor: &mut Editor, ui: &Ui) {
	toolbar::draw_toolbar(editor, ui);
	dockspace(ui);
	statusbar(ui);
	hierarhy(ui);
	viewport(editor, ui);
	input_debug(editor, ui);
	worlds(editor, ui);
	dialogs(editor, ui);
}

fn hierarhy(ui: &Ui) {
	ui.window("Hierarchy")
		.size([260.0, 300.0], Condition::FirstUseEver)
		.position([30.0, 60.0], Condition::FirstUseEver)
		.build(|| {
			if let Some(_node) = ui.tree_node("Hierarchy") {
				ui.selectable("Entity1");
			}
		});
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

/// The place the render will go. It is the only panel that gets the input of
/// the game, and only while it has the focus.
fn viewport(editor: &mut Editor, ui: &Ui) {
	let mut focused = false;
	ui.window("Viewport")
		.size([640.0, 360.0], Condition::FirstUseEver)
		.position([30.0, 60.0], Condition::FirstUseEver)
		.build(|| {
			focused = ui.is_window_focused();
			ui.text_disabled("The render goes here");
			if focused {
				ui.text_disabled("Focused: the keyboard and the mouse go to the game");
			}
		});
	editor.set_viewport_focus(focused);
}

fn input_debug(editor: &mut Editor, ui: &Ui) {
	ui.window("Input")
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

/// The worlds of the project, `*` marks the ones with changes.
fn worlds(editor: &Editor, ui: &Ui) {
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
			for (name, entry) in &project.worlds {
				ui.text(if entry.dirty { format!("* {name}") } else { name.clone() });
			}
		});
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
	welcome(ui);
	new_world(editor, ui);
	quit(editor, ui);
	backup_offer(editor, ui);
	problems(editor, ui);
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
			let _ = ui
				.dockspace()
				.current_window(content)
				.flags(DockNodeFlags::empty())
				.build();
		});
}
