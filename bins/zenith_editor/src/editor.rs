//! The state of the editor and what it does, apart from how it is drawn.

use std::{
	path::{Path, PathBuf},
	time::{Duration, Instant},
};

use winit::event::{DeviceEvent, WindowEvent};
use zenith_error::ZResult;
use zenith_input::{Gamepads, Input, InputEvent, ZGamePad, ZGamePadAxis, ZGamepadKind};
use zenith_renderer::assets::{
	AssetRegistry,
	ops::{self, LooseModel},
};

use crate::{
	backup::{self, BackupInfo},
	project::{NewWorldError, OpenProject},
	recent::Recent,
	scene_view::SceneView,
	settings::{self, LooseModels},
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
	pub loose_models: bool,
}

#[allow(clippy::struct_excessive_bools)] // the flags of the state of the UI
pub struct Editor {
	pub project: Option<OpenProject>,
	/// What the Viewport shows.
	pub scene: SceneView,
	/// The world the Viewport and the Hierarchy show.
	pub active_world: Option<String>,
	/// The seconds the last frame took.
	pub dt: f32,
	recent_file: Option<PathBuf>,
	pub input: Input,
	gamepads: Option<Gamepads>,
	pub pad: EmulatedPad,
	/// The Viewport has the focus: it gets the keyboard, the mouse and the real
	/// gamepads.
	pub viewport_focused: bool,
	/// A backup of unsaved changes found in the project that was opened.
	pub backup_offer: Option<BackupInfo>,
	/// The model files put into `Assets/` by hand that the user is asked
	/// about.
	pub loose_models: Vec<LooseModel>,
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
			scene: SceneView::default(),
			active_world: None,
			dt: 0.0,
			recent_file,
			input: Input::default(),
			gamepads,
			pad: EmulatedPad::default(),
			viewport_focused: false,
			backup_offer: None,
			loose_models: Vec::new(),
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
		// The renderer goes out of the worlds that are about to be dropped.
		self.scene.release(self.project.as_mut());
		self.active_world = None;
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
		self.check_loose_models(false);
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

	// --- assets
	// ---------------------------------------------------------------

	/// File → Import: brings `files` (models, pictures) into `folder`, a
	/// folder of the project's `Assets/` on disk. A file that cannot come in
	/// is told to the user; the others still do.
	pub fn import_assets(&mut self, files: &[PathBuf], folder: &Path) {
		let Some(project) = &self.project else {
			self.say("Open a project first");
			return;
		};
		let project = &project.project;
		let target = match project.zpath_of(folder) {
			Some(target) if target.scheme() == "Assets" => target,
			Some(target) => {
				self.problem(format!(
					"{target} is not for assets (Assets/Worlds holds the worlds): pick another folder of Assets"
				));
				return;
			}
			None => {
				self.problem(format!(
					"{} is not a folder of the project's Assets: the files go there",
					folder.display()
				));
				return;
			}
		};
		let mut registry = match AssetRegistry::scan(project) {
			Ok(registry) => registry,
			Err(e) => {
				self.problem(format!("The assets of the project cannot be read: {e}"));
				return;
			}
		};
		let mut made = 0;
		let mut failed = Vec::new();
		for file in files {
			let result = if is_model(file) {
				ops::import_model(project, &mut registry, file, &target).map(|assets| assets.len())
			} else {
				ops::import_texture(project, &mut registry, file, &target, None).map(|_| 1)
			};
			match result {
				Ok(count) => made += count,
				Err(e) => failed.push(format!("{} is not imported: {e}", file.display())),
			}
		}
		let failures = failed.len();
		for message in failed {
			self.problem(message);
		}
		match failures {
			0 => self.say(format!("Imported {made} asset{} into {target}", plural(made))),
			_ => self.say(format!("Imported {made}, {failures} failed")),
		}
	}

	/// The name to offer for File → Export of the asset file `asset` (on disk,
	/// in the project): its name with the extension of its source.
	#[must_use]
	pub fn export_file_name(&self, asset: &Path) -> Option<String> {
		let project = &self.project.as_ref()?.project;
		ops::export_file_name(project, &project.zpath_of(asset)?).ok()
	}

	/// File → Export: writes what the asset file `asset` (on disk, in the
	/// project) was made from into `to`.
	pub fn export_asset(&mut self, asset: &Path, to: &Path) {
		let Some(project) = &self.project else {
			self.say("Open a project first");
			return;
		};
		let project = &project.project;
		let Some(path) = project.zpath_of(asset) else {
			self.problem(format!("{} is not a file of the project", asset.display()));
			return;
		};
		match ops::export_asset(project, &path, to) {
			Ok(()) => self.say(format!("Exported {path} to {}", to.display())),
			Err(e) => self.problem(format!("{path} is not exported: {e}")),
		}
	}

	/// Looks for model files put into `Assets/` by hand. What happens to them
	/// is the setting of the project (see [`LooseModels`]); `asked` (File →
	/// Import Loose Models) always asks.
	pub fn check_loose_models(&mut self, asked: bool) {
		let Some(project) = &self.project else { return };
		let found = match ops::loose_models(&project.project) {
			Ok(found) => found,
			Err(e) => {
				self.problem(format!("Assets/ cannot be searched for models: {e}"));
				return;
			}
		};
		let setting = if asked {
			LooseModels::Ask
		} else {
			settings::loose_models(&project.project)
		};
		if !found.iter().any(|model| model.taken_by.is_none()) {
			if asked {
				self.say("No models to import in Assets");
			}
			return;
		}
		match setting {
			LooseModels::Never => {}
			LooseModels::Ask => {
				self.loose_models = found;
				self.requests.loose_models = true;
			}
			LooseModels::Always => {
				self.loose_models = found;
				self.import_loose_models();
			}
		}
	}

	/// "Import" in the question about the loose models: each that is not in
	/// the way of an asset becomes one, and its file goes to the trash.
	pub fn import_loose_models(&mut self) {
		let models = std::mem::take(&mut self.loose_models);
		let Some(project) = &self.project else { return };
		let project = &project.project;
		let mut registry = match AssetRegistry::scan(project) {
			Ok(registry) => registry,
			Err(e) => {
				self.problem(format!("The assets of the project cannot be read: {e}"));
				return;
			}
		};
		let mut imported = 0;
		let mut failed = Vec::new();
		for model in models.iter().filter(|model| model.taken_by.is_none()) {
			match ops::import_loose_model(project, &mut registry, &model.path) {
				Ok(_) => imported += 1,
				Err(e) => failed.push(format!("{} is not imported: {e}", model.path)),
			}
		}
		let failures = failed.len();
		for message in failed {
			self.problem(message);
		}
		match failures {
			0 => self.say(format!(
				"Imported {imported} model{}, the files are in the trash",
				plural(imported)
			)),
			_ => self.say(format!("Imported {imported} models, {failures} failed")),
		}
	}

	/// "Never for this project" in the question about the loose models.
	pub fn never_import_loose_models(&mut self) {
		self.loose_models.clear();
		let Some(project) = &self.project else { return };
		match settings::set_loose_models(&project.project, LooseModels::Never) {
			Ok(()) => self.say("Models in Assets are not imported on their own any more"),
			Err(e) => self.problem(format!("The setting is not saved: {e}")),
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

	/// The world the Viewport shows: the one that was chosen, else the first.
	pub fn shown_world(&mut self) -> Option<String> {
		let project = self.project.as_ref()?;
		if self
			.active_world
			.as_ref()
			.is_none_or(|name| !project.worlds.contains_key(name))
		{
			self.active_world = project.worlds.keys().next().cloned();
		}
		self.active_world.clone()
	}

	/// Draws the world into the texture of the Viewport, before the UI that
	/// shows it.
	pub fn render_viewport(&mut self, gfx: &mut crate::gfx::Gfx) {
		let Some(name) = self.shown_world() else { return };
		if let Some(project) = &mut self.project {
			self.scene.render(gfx, project, &name);
		}
	}

	/// Edit → Replace Source: puts another file into the model or picture
	/// `asset` (on disk, in the project). What uses it gets the new one: the
	/// renderer sees the file change.
	pub fn replace_source(&mut self, asset: &Path, source: &Path) {
		let Some(project) = &self.project else {
			self.say("Open a project first");
			return;
		};
		let project = &project.project;
		let Some(path) = project.zpath_of(asset) else {
			self.problem(format!("{} is not a file of the project", asset.display()));
			return;
		};
		match ops::replace_source(project, &path, source) {
			Ok(()) => self.say(format!("{path} now comes from {}", source.display())),
			Err(e) => self.problem(format!("The source of {path} is not replaced: {e}")),
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

/// The pictures File → Import takes; the models are [`ops::MODEL_SOURCES`].
pub const PICTURE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "hdr"];

fn is_model(file: &Path) -> bool {
	file.file_name()
		.and_then(|name| name.to_str())
		.is_some_and(ops::is_model_source)
}

const fn plural(count: usize) -> &'static str {
	if count == 1 { "" } else { "s" }
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
	fn import_brings_files_into_a_folder_of_assets_and_export_takes_the_source_out() -> TestResult {
		let (tmp, mut editor, path) = editor_with_project()?;
		let models = path.join("Assets/Models");
		std::fs::create_dir_all(&models)?;
		let square = tmp.path().join("square.obj");
		std::fs::write(&square, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")?;
		let broken = tmp.path().join("broken.png");
		std::fs::write(&broken, b"not a picture")?;

		editor.import_assets(&[square.clone(), broken], &models);
		assert!(models.join("square.zenithmodel").is_file(), "the model came in");
		assert_eq!(editor.status(), Some("Imported 1, 1 failed"));
		assert_eq!(editor.problems.len(), 1, "the picture that is not one is told");

		editor.import_assets(std::slice::from_ref(&square), tmp.path());
		assert_eq!(editor.problems.len(), 2, "a folder outside the project is refused");
		editor.import_assets(std::slice::from_ref(&square), &path.join("Assets/Worlds"));
		assert!(
			editor.problems.last().is_some_and(|p| p.contains("worlds")),
			"and the folder of the worlds"
		);

		let into_root = tmp.path().join("tri.obj");
		std::fs::write(
			&into_root,
			"v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
",
		)?;
		editor.import_assets(&[into_root], &path.join("Assets"));
		assert!(
			path.join("Assets/tri.zenithmodel").is_file(),
			"Assets itself is a folder of Assets"
		);

		let asset = models.join("square.zenithmodel");
		assert_eq!(editor.export_file_name(&asset).as_deref(), Some("square.obj"));
		let out = tmp.path().join("out.obj");
		editor.export_asset(&asset, &out);
		assert_eq!(std::fs::read(&out)?, std::fs::read(&square)?, "the source comes out");
		Ok(())
	}

	#[test]
	fn replacing_a_source_changes_the_asset_and_keeps_its_place() -> TestResult {
		let (tmp, mut editor, path) = editor_with_project()?;
		let models = path.join("Assets/Models");
		std::fs::create_dir_all(&models)?;
		let square = tmp.path().join("square.obj");
		std::fs::write(
			&square,
			"v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
",
		)?;
		editor.import_assets(std::slice::from_ref(&square), &models);
		let asset = models.join("square.zenithmodel");
		let before = std::fs::read(&asset)?;

		let bigger = tmp.path().join("bigger.obj");
		std::fs::write(
			&bigger,
			"v 0 0 0
v 2 0 0
v 0 2 0
v 2 2 0
f 1 2 3
f 2 4 3
",
		)?;
		editor.replace_source(&asset, &bigger);
		assert!(editor.problems.is_empty(), "{:?}", editor.problems);
		assert_ne!(std::fs::read(&asset)?, before, "the asset holds the new source");
		let name = editor.export_file_name(&asset);
		assert_eq!(name.as_deref(), Some("square.obj"), "it is the same asset");

		editor.replace_source(&tmp.path().join("outside.zenithmodel"), &bigger);
		assert_eq!(editor.problems.len(), 1, "a file outside the project is refused");
		Ok(())
	}

	#[test]
	fn models_put_into_assets_are_asked_about_imported_or_left_as_the_project_says() -> TestResult {
		let (tmp, _, path) = editor_with_project()?;
		let remembered = tmp.path().join("Editor.toml");
		std::fs::write(path.join("Assets/tri.obj"), "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")?;

		let mut editor = Editor::new(Some(remembered.clone()));
		assert!(editor.requests.loose_models, "asked on open");
		assert_eq!(editor.loose_models.len(), 1);
		editor.never_import_loose_models();
		assert_eq!(editor.loose_models, []);

		let mut editor = Editor::new(Some(remembered.clone()));
		assert!(!editor.requests.loose_models, "never: not asked again");
		editor.check_loose_models(true);
		assert!(editor.requests.loose_models, "but the menu still asks");
		editor.loose_models.clear();

		let project = Project::open(&path)?;
		settings::set_loose_models(&project, LooseModels::Always)?;
		let editor = Editor::new(Some(remembered));
		assert!(!editor.requests.loose_models, "always: not asked");
		assert!(path.join("Assets/tri.zenithmodel").is_file(), "imported");
		assert!(!path.join("Assets/tri.obj").exists(), "the file is in the trash");
		assert_eq!(editor.status(), Some("Imported 1 model, the files are in the trash"));
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
