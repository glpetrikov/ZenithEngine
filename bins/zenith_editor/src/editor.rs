//! The state of the editor and what it does, apart from how it is drawn.

use std::{
	path::{Path, PathBuf},
	time::{Duration, Instant},
};

use winit::event::{DeviceEvent, WindowEvent};
use zenith_error::ZResult;
use zenith_input::{Gamepads, Input, InputEvent, ZGamePad, ZGamePadAxis, ZGamepadKind};

use crate::{
	backup::{self, BackupInfo},
	project::{NewWorldError, OpenProject},
	recent::Recent,
};

/// The number the emulated gamepad has: far from the ones real pads get.
pub const EMULATED_PAD: usize = 1000;
/// How long a line of status stays in the menu bar.
const STATUS_TIME: Duration = Duration::from_secs(5);

/// The form of File → New World.
#[derive(Default)]
pub struct NewWorldForm {
	pub folder: String,
	pub name: String,
	pub error: Option<String>,
}

/// The gamepad that is made of widgets, for trying out the input without a
/// real one.
#[derive(Default)]
pub struct EmulatedPad {
	pub connected: bool,
	pub held: Vec<ZGamePad>,
	pub left: [f32; 2],
	pub right: [f32; 2],
	pub triggers: [f32; 2],
}

/// Asks the user for something; drawn as popups.
#[derive(Default)]
#[allow(clippy::struct_excessive_bools)] // one flag for each dialog, which is the point
pub struct Requests {
	pub new_world: bool,
	pub quit: bool,
	pub backup: bool,
	pub problems: bool,
}

#[allow(clippy::struct_excessive_bools)] // the flags of the state of the UI
pub struct Editor {
	pub project: Option<OpenProject>,
	recent_file: Option<PathBuf>,
	pub input: Input,
	gamepads: Option<Gamepads>,
	pub pad: EmulatedPad,
	/// The Viewport has the focus: it gets the keyboard, the mouse and the real
	/// gamepads.
	pub viewport_focused: bool,
	/// A backup of unsaved changes found in the project that was opened.
	pub backup_offer: Option<BackupInfo>,
	pub new_world: NewWorldForm,
	pub requests: Requests,
	/// Things that went wrong and that the user has to see.
	pub problems: Vec<String>,
	status: Option<(String, Instant)>,
	/// The editor is to end.
	pub exit: bool,
	pub welcome_open: bool,
	/// A dialog is open (set by the UI each frame).
	pub dialog_open: bool,
}

impl Editor {
	/// A new editor. It opens the project it had open last time, if it can.
	#[must_use]
	pub fn new(recent_file: Option<PathBuf>) -> Self {
		let gamepads = match Gamepads::new() {
			Ok(gamepads) => Some(gamepads),
			Err(e) => {
				zenith_log::warn!("No gamepads: {e}");
				None
			}
		};
		let mut editor = Self {
			project: None,
			recent_file,
			input: Input::default(),
			gamepads,
			pad: EmulatedPad::default(),
			viewport_focused: false,
			backup_offer: None,
			new_world: NewWorldForm::default(),
			requests: Requests::default(),
			problems: Vec::new(),
			status: None,
			exit: false,
			welcome_open: true,
			dialog_open: false,
		};
		let last = editor
			.recent_file
			.as_deref()
			.and_then(|file| Recent::load(file).last_project);
		if let Some(path) = last
			&& path.is_dir()
			&& let Err(e) = editor.open_project(&path)
		{
			editor.problem(format!("Cannot open the last project {}: {e}", path.display()));
		}
		editor
	}

	// --- project
	// ---------------------------------------------------------------

	/// Opens the project in `path` (a folder) in place of the one that is open,
	/// and remembers it for next time. The worlds that cannot be read are told
	/// to the user, and a backup of unsaved changes is offered. The caller
	/// makes sure that nothing is lost first: see
	/// [`Self::can_switch_project`].
	///
	/// # Errors
	/// The error of opening the project; the one that was open stays.
	pub fn open_project(&mut self, path: &Path) -> ZResult<()> {
		let open = OpenProject::open(path)?;
		for problem in &open.load_errors {
			self.problems
				.push(format!("The world {} cannot be read: {}", problem.name, problem.error));
		}
		self.backup_offer = match backup::find(&open.path) {
			Ok(info) => info,
			Err(e) => {
				self.problems
					.push(format!("The backup of unsaved changes cannot be read: {e}"));
				None
			}
		};
		self.requests.backup = self.backup_offer.is_some();
		self.requests.problems = !self.problems.is_empty();
		if let Some(file) = &self.recent_file
			&& let Err(e) = (Recent {
				last_project: Some(open.path.clone()),
			})
			.save(file)
		{
			zenith_log::warn!("Cannot remember the project: {e}");
		}
		self.say(format!("Opened {}", open.path.display()));
		self.project = Some(open);
		Ok(())
	}

	/// Opening another project would lose nothing. For the menu of projects.
	#[allow(dead_code)] // the menu of projects is not there yet
	#[must_use]
	pub fn can_switch_project(&self) -> bool {
		self.project.as_ref().is_none_or(|p| !p.has_dirty())
	}

	pub fn has_dirty(&self) -> bool {
		self.project.as_ref().is_some_and(OpenProject::has_dirty)
	}

	// --- saving
	// ---------------------------------------------------------------

	/// Ctrl+S: writes every world that has changes.
	pub fn save_all(&mut self) {
		let Some(project) = &mut self.project else {
			self.say("There is no project to save");
			return;
		};
		let report = project.save_dirty();
		for failed in &report.failed {
			self.problems
				.push(format!("The world {} is not saved: {}", failed.name, failed.error));
		}
		self.requests.problems |= !report.failed.is_empty();
		match (report.saved.len(), report.failed.len()) {
			(0, 0) => self.say("Nothing to save"),
			(saved, 0) => self.say(format!("Saved {saved} world{}", if saved == 1 { "" } else { "s" })),
			(saved, failed) => self.say(format!("Saved {saved}, {failed} failed")),
		}
	}

	/// The user closes the editor (the window, Escape): ends at once, or asks
	/// first if there are changes.
	pub fn request_exit(&mut self) {
		if self.has_dirty() {
			self.requests.quit = true;
		} else {
			self.exit = true;
		}
	}

	/// "Save all" in the question on exit: ends if everything was written.
	pub fn save_and_exit(&mut self) {
		self.save_all();
		if !self.has_dirty() {
			self.exit = true;
		}
	}

	/// The editor is stopped from outside (Ctrl+C, SIGTERM): the changes that
	/// are not saved go to the backup, and the editor ends.
	pub fn backup_and_exit(&mut self) {
		if let Some(project) = &mut self.project {
			match backup::write(project) {
				Ok(names) if names.is_empty() => {}
				Ok(names) => zenith_log::info!("Unsaved worlds went to the backup: {}", names.join(", ")),
				Err(e) => zenith_log::error!("The unsaved worlds could not go to the backup: {e}"),
			}
		}
		self.exit = true;
	}

	/// "Restore" on the offer of the backup.
	pub fn accept_backup(&mut self) {
		self.backup_offer = None;
		let Some(project) = &mut self.project else { return };
		match backup::restore(project) {
			Ok(report) => {
				for failed in &report.failed {
					self.problems.push(format!(
						"The world {} stays in the backup, it cannot be restored: {}",
						failed.name, failed.error
					));
				}
				self.requests.problems |= !report.failed.is_empty();
				self.say(format!("Restored {} from the backup", report.restored.len()));
			}
			Err(e) => {
				self.problems.push(format!("The backup cannot be restored: {e}"));
				self.requests.problems = true;
			}
		}
	}

	/// "Discard" on the offer of the backup.
	pub fn discard_backup(&mut self) {
		self.backup_offer = None;
		let Some(project) = &self.project else { return };
		if let Err(e) = backup::discard(&project.path) {
			self.problems.push(format!("The backup cannot be deleted: {e}"));
			self.requests.problems = true;
		}
	}

	/// "Create" in File → New World. `true` if it was made (the form closes).
	pub fn create_world(&mut self) -> bool {
		let Some(project) = &mut self.project else {
			self.new_world.error = Some("Open a project first.".to_string());
			return false;
		};
		match project.create_world(&self.new_world.folder, &self.new_world.name) {
			Ok(name) => {
				self.say(format!("Created {name}, not saved yet"));
				self.new_world = NewWorldForm::default();
				true
			}
			Err(e) => {
				self.new_world.error = Some(match e {
					NewWorldError::EmptyName => "The world needs a name.".to_string(),
					other => other.to_string(),
				});
				false
			}
		}
	}

	pub fn problem(&mut self, message: String) {
		zenith_log::warn!("{message}");
		self.problems.push(message);
		self.requests.problems = true;
	}

	// --- status
	// ---------------------------------------------------------------

	pub fn say(&mut self, message: impl Into<String>) {
		self.status = Some((message.into(), Instant::now()));
	}

	/// The line to show in the menu bar, while it is fresh.
	#[must_use]
	pub fn status(&self) -> Option<&str> {
		self.status
			.as_ref()
			.filter(|(_, since)| since.elapsed() < STATUS_TIME)
			.map(|(message, _)| message.as_str())
	}

	// --- input
	// ---------------------------------------------------------------

	/// Only the Viewport gets the keyboard and the mouse: the rest of the
	/// editor does not turn into game input.
	pub fn window_event(&mut self, event: &WindowEvent) {
		if self.viewport_focused {
			self.input.handle_window_event(event);
		}
	}

	pub fn device_event(&mut self, event: &DeviceEvent) {
		if self.viewport_focused {
			self.input.handle_device_event(event);
		}
	}

	/// Tells whether the Viewport has the focus. Losing it lets go of the keys.
	pub fn set_viewport_focus(&mut self, focused: bool) {
		if focused != self.viewport_focused {
			self.viewport_focused = focused;
			self.input.apply(InputEvent::Focus(focused));
		}
	}

	/// Ends a frame of input: the real gamepads are read (while the Viewport
	/// has the focus), then what happened in the frame becomes the past.
	pub fn end_frame(&mut self, dt: f32) {
		if self.viewport_focused
			&& let Some(gamepads) = &mut self.gamepads
		{
			gamepads.poll(&mut self.input);
		}
		self.input.end_frame(dt);
	}

	/// The emulated gamepad is plugged in or out.
	pub fn set_pad_connected(&mut self, connected: bool) {
		if connected == self.pad.connected {
			return;
		}
		self.pad = EmulatedPad {
			connected,
			..EmulatedPad::default()
		};
		self.input.apply(if connected {
			InputEvent::GamepadConnected {
				id: EMULATED_PAD,
				name: "Emulated gamepad".to_string(),
				kind: ZGamepadKind::Generic,
				ff_supported: false,
				mapped: true,
			}
		} else {
			InputEvent::GamepadDisconnected { id: EMULATED_PAD }
		});
	}

	/// A button of the emulated gamepad is held or let go.
	pub fn set_pad_button(&mut self, button: ZGamePad, held: bool) {
		if self.pad.held.contains(&button) == held {
			return;
		}
		if held {
			self.pad.held.push(button);
		} else {
			self.pad.held.retain(|b| *b != button);
		}
		self.input.apply(InputEvent::GamepadButton {
			id: EMULATED_PAD,
			button,
			pressed: held,
		});
	}

	/// An axis of the emulated gamepad.
	pub fn set_pad_axis(&mut self, axis: ZGamePadAxis, value: f32) {
		self.input.apply(InputEvent::GamepadAxis {
			id: EMULATED_PAD,
			axis,
			value,
		});
	}
}

#[cfg(test)]
mod tests {
	use zenith_filesystem::Project;

	use super::*;

	type TestResult = Result<(), Box<dyn std::error::Error>>;

	/// An editor with a project in a folder with the worlds `A` and `B` saved.
	fn editor_with_project() -> Result<(tempfile::TempDir, Editor, PathBuf), Box<dyn std::error::Error>> {
		let tmp = tempfile::tempdir()?;
		let path = tmp.path().join("Game");
		Project::create(&path)?;
		let remembered = tmp.path().join("Editor.toml");
		let mut seed = OpenProject::open(&path)?;
		seed.create_world("", "A")?;
		seed.create_world("", "B")?;
		seed.save_dirty();
		let mut editor = Editor::new(Some(remembered));
		editor.open_project(&path)?;
		Ok((tmp, editor, path))
	}

	fn mark(editor: &mut Editor, name: &str) {
		if let Some(entry) = editor.project.as_mut().and_then(|p| p.worlds.get_mut(name)) {
			entry.dirty = true;
		}
	}

	#[test]
	fn a_project_that_was_open_opens_again() -> TestResult {
		let (tmp, editor, path) = editor_with_project()?;
		assert_eq!(editor.project.as_ref().map(|p| p.worlds.len()), Some(2));
		let again = Editor::new(Some(tmp.path().join("Editor.toml")));
		assert_eq!(
			again.project.as_ref().map(|p| p.path.clone()),
			Some(path),
			"it was remembered"
		);
		assert!(Editor::new(Some(tmp.path().join("nothing.toml"))).project.is_none());
		Ok(())
	}

	#[test]
	fn save_all_writes_only_what_changed_and_says_so() -> TestResult {
		let (_tmp, mut editor, _) = editor_with_project()?;
		editor.save_all();
		assert_eq!(editor.status(), Some("Nothing to save"));
		mark(&mut editor, "B");
		editor.save_all();
		assert_eq!(editor.status(), Some("Saved 1 world"));
		assert!(!editor.has_dirty());
		Ok(())
	}

	#[test]
	fn closing_asks_only_when_something_would_be_lost() -> TestResult {
		let (_tmp, mut editor, _) = editor_with_project()?;
		editor.request_exit();
		assert!(editor.exit && !editor.requests.quit, "nothing is lost: it ends");

		let (_tmp, mut editor, _) = editor_with_project()?;
		mark(&mut editor, "A");
		editor.request_exit();
		assert!(!editor.exit && editor.requests.quit, "it asks");
		editor.save_and_exit();
		assert!(editor.exit && !editor.has_dirty(), "saved, then it ends");
		Ok(())
	}

	#[test]
	fn stopped_from_outside_puts_the_changes_in_the_backup_and_the_next_start_offers_them() -> TestResult {
		let (tmp, mut editor, path) = editor_with_project()?;
		mark(&mut editor, "A");
		editor.project.as_mut().ok_or("no project")?.create_world("", "Fresh")?;
		editor.backup_and_exit();
		assert!(editor.exit, "it ends");
		assert!(path.join(backup::BACKUP_DIR).is_dir(), "the backup is there");

		let mut next = Editor::new(Some(tmp.path().join("Editor.toml")));
		let offer = next.backup_offer.clone().ok_or("no offer")?;
		assert_eq!(offer.worlds, ["A", "Fresh"], "the unsaved worlds are offered");
		assert!(next.requests.backup, "and the user is asked");
		assert!(!next.has_dirty(), "nothing is dirty until it is restored");

		next.accept_backup();
		assert!(next.backup_offer.is_none());
		assert!(!path.join(backup::BACKUP_DIR).exists(), "the backup is deleted");
		let names: Vec<String> = next
			.project
			.as_ref()
			.ok_or("no project")?
			.worlds
			.keys()
			.cloned()
			.collect();
		assert_eq!(names, ["A", "B", "Fresh"], "Fresh is a world of the project now");
		assert!(path.join("Assets/Worlds/Fresh.zenith").is_file());
		Ok(())
	}

	#[test]
	fn the_backup_can_be_discarded() -> TestResult {
		let (tmp, mut editor, path) = editor_with_project()?;
		mark(&mut editor, "A");
		editor.backup_and_exit();
		let mut next = Editor::new(Some(tmp.path().join("Editor.toml")));
		assert!(next.backup_offer.is_some());
		next.discard_backup();
		assert!(!path.join(backup::BACKUP_DIR).exists());
		assert!(
			Editor::new(Some(tmp.path().join("Editor.toml"))).backup_offer.is_none(),
			"not offered again"
		);
		Ok(())
	}

	#[test]
	fn nothing_dirty_leaves_no_backup() -> TestResult {
		let (_tmp, mut editor, path) = editor_with_project()?;
		editor.backup_and_exit();
		assert!(editor.exit);
		assert!(!path.join(backup::BACKUP_DIR).exists());
		Ok(())
	}

	#[test]
	fn a_new_world_is_created_from_the_form_or_the_form_says_why_not() -> TestResult {
		let (_tmp, mut editor, _) = editor_with_project()?;
		editor.new_world.name = "A".into();
		assert!(!editor.create_world(), "taken");
		assert!(editor.new_world.error.as_deref().is_some_and(|e| e.contains("already")));
		editor.new_world.name = String::new();
		assert!(!editor.create_world());
		assert_eq!(editor.new_world.error.as_deref(), Some("The world needs a name."));
		editor.new_world.name = "C".into();
		editor.new_world.folder = "Caves".into();
		assert!(editor.create_world(), "made");
		assert!(
			editor.new_world.error.is_none() && editor.new_world.name.is_empty(),
			"the form is clear"
		);
		assert_eq!(
			editor.project.as_ref().map(OpenProject::dirty_names),
			Some(vec!["Caves/C".to_string()])
		);
		Ok(())
	}

	#[test]
	fn the_keyboard_is_for_the_viewport_only() {
		let mut editor = Editor::new(None);
		let event = winit::event::WindowEvent::Focused(false);
		editor.window_event(&event);
		assert!(!editor.viewport_focused);
		editor.set_viewport_focus(true);
		editor.set_viewport_focus(false);
		assert!(!editor.viewport_focused, "focus goes and comes back");
	}

	#[test]
	fn the_emulated_gamepad_appears_and_presses_buttons() {
		let mut editor = Editor::new(None);
		assert_eq!(editor.input.list_gamepads().len(), 0);
		editor.set_pad_connected(true);
		assert_eq!(editor.input.list_gamepads().len(), 1);
		editor.set_pad_button(ZGamePad::South, true);
		editor.set_pad_axis(ZGamePadAxis::LeftStickX, 1.0);
		assert!(editor.input.is_gamepad_pressed(ZGamePad::South, EMULATED_PAD));
		assert!(editor.input.gamepad_axis(ZGamePadAxis::LeftStickX, EMULATED_PAD) > 0.5);
		editor.set_pad_button(ZGamePad::South, false);
		assert!(!editor.input.is_gamepad_pressed(ZGamePad::South, EMULATED_PAD));
		editor.set_pad_connected(false);
		assert_eq!(editor.input.list_gamepads().len(), 0, "plugged out");
	}
}
