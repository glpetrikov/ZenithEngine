use std::{path::PathBuf, sync::Arc, time::Instant};

use egui::{Align, Layout, RichText, Sense, TextureId, Ui, WidgetText};
use egui_dock::{DockArea, DockState, NodeIndex, Style, TabStyle, TabViewer};
use winit::window::Window;
use ze_build::BuildTarget;
use ze_core::Vec2;
use ze_ecs::{EditorOnly, EntitiesView, PhysicsSettings, Scene};
use ze_project::{PhysicsSettings as ProjectPhysicsSettings, Project};
use ze_renderer::Camera;

use crate::{
	asset_hot_reload::ScriptStatus,
	panels::{
		ConsolePanel, EditorPanelContext, InspectorPanel, NoProjectPanel, Panel, SceneHierarchyPanel, ScenePanel,
		TextureSheetPanel, TileBrush, ViewportOutput, file_hierarchy::FileExplorer,
	},
	style::{ACCENT, BG_BASE, TEXT_MUTED, TEXT_PRIMARY},
	undo_redo::{SceneSnapshotCommand, UndoRedoBuffer},
};

fn abbreviate(name: &str) -> String {
	if name.len() <= 7 {
		return name.to_string();
	}
	let mut words = Vec::new();
	let mut current = String::new();
	let chars: Vec<char> = name.chars().collect();
	for (i, ch) in chars.iter().enumerate() {
		if ch.is_uppercase() {
			if !current.is_empty() {
				words.push(current);
				current = String::new();
			}
			current.push(*ch);
		} else if *ch == '_' || *ch == '-' || *ch == ' ' {
			if !current.is_empty() {
				words.push(current);
				current = String::new();
			}
		} else {
			if i > 0 && chars[i - 1].is_lowercase() && ch.is_uppercase() && !current.is_empty() {
				words.push(current);
				current = String::new();
			}
			current.push(*ch);
		}
	}
	if !current.is_empty() {
		words.push(current);
	}
	words
		.iter()
		.filter_map(|w| w.chars().next())
		.collect::<String>()
		.to_uppercase()
}

const EDITOR_UNDO_HISTORY_LIMIT: usize = 32;
const DEFAULT_BOTTOM_PANEL_SHARE: f32 = 0.68;
const DEFAULT_RIGHT_COLUMN_SHARE: f32 = 0.75;
const DEFAULT_HIERARCHY_SHARE_IN_RIGHT_COLUMN: f32 = 0.45;
const SCENE_SELECTOR_VISIBLE_ROWS: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
	Edit,
	Play,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorRequest {
	OpenProject,
	NewProject,
	SaveProject,
	CreateScene(String),
	SwitchScene(String),
	EnterPlayMode,
	StopPlayMode,
	OpenTextureSheetAsset(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveState {
	Saved,
	Saving,
	UnsavedChanges,
	SaveFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildStatus {
	Idle,
	Running,
	Done,
	Failed,
}

#[derive(Debug, Clone, Copy)]
pub struct SaveStatus {
	pub state: SaveState,
	pub last_successful_save: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct EditorStatus {
	pub script: ScriptStatus,
	pub save: SaveStatus,
	pub build: BuildStatus,
	pub build_message: Option<String>,
}

impl SaveStatus {
	pub const fn saved() -> Self {
		Self {
			state: SaveState::Saved,
			last_successful_save: None,
		}
	}

	const fn with_state(self, state: SaveState) -> Self { Self { state, ..self } }
}

#[derive(Debug, Clone)]
pub struct SceneInfo {
	pub name: String,
	pub active: bool,
	pub dirty: bool,
}

#[derive(Debug, Clone)]
pub struct BuildRequest {
	pub output_dir: PathBuf,
	pub target: BuildTarget,
}

#[derive(Debug, Clone, Copy)]
enum WindowPanel {
	Scene,
	Hierarchy,
	Inspector,
	AssetBrowser,
	Files,
	Console,
	TextureSheet,
}

impl WindowPanel {
	const ALL: [Self; 7] = [
		Self::Scene,
		Self::Hierarchy,
		Self::Inspector,
		Self::AssetBrowser,
		Self::Files,
		Self::Console,
		Self::TextureSheet,
	];

	const fn menu_label(self) -> &'static str {
		match self {
			Self::Scene => "Scene",
			Self::Hierarchy => "Hierarchy",
			Self::Inspector => "Inspector",
			Self::AssetBrowser => "Asset Browser",
			Self::Files => "File Explorer",
			Self::Console => "Console",
			Self::TextureSheet => "Texture Sheet",
		}
	}

	const fn tab_name(self) -> &'static str {
		match self {
			Self::Scene => "Scene",
			Self::Hierarchy => "Hierarchy",
			Self::Inspector => "Inspector",
			Self::AssetBrowser | Self::Files => "File Explorer",
			Self::Console => "Console",
			Self::TextureSheet => "Texture Sheet",
		}
	}
}

pub struct EditorOutput {
	pub viewport: Option<ViewportOutput>,
	pub build_request: Option<BuildRequest>,
	pub menu_request: Option<EditorRequest>,
	pub project_saved: Option<Project>,
	pub project_save_error: Option<String>,
	pub project_config_changed: bool,
	// Exposed so the main loop can trigger event_loop.exit() on close
	pub close_requested: bool,
	// Rendered height of the title bar; used for winit-level titlebar hit-testing
	pub titlebar_height: f32,
	// Rects of interactive titlebar elements; used by winit to skip drag on buttons
	pub titlebar_interactive_rects: Vec<egui::Rect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsCategory {
	Project,
	Physics,
}

impl SettingsCategory {
	const ALL: [Self; 2] = [Self::Project, Self::Physics];

	const fn label(self) -> &'static str {
		match self {
			Self::Project => "Project",
			Self::Physics => "Physics",
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhysicsSettingsScope {
	ThisScene,
	AllScenesDefault,
}

impl PhysicsSettingsScope {
	const fn label(self) -> &'static str {
		match self {
			Self::ThisScene => "This Scene",
			Self::AllScenesDefault => "All Scenes (Default)",
		}
	}
}

#[derive(Debug)]
struct SceneCreatePrompt {
	name: String,
	error: Option<String>,
}

impl Default for SceneCreatePrompt {
	fn default() -> Self {
		Self {
			name: "NewScene".to_string(),
			error: None,
		}
	}
}

#[derive(Debug)]
struct BuildWindowState {
	project_root: PathBuf,
	output_dir: String,
	target: BuildTarget,
}

impl BuildWindowState {
	fn from_project(project: &Project) -> Self {
		Self {
			project_root: project.root_dir.clone(),
			output_dir: String::new(),
			target: BuildTarget::host(),
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildWindowVisibility {
	Closed,
	Open,
}

#[allow(clippy::struct_excessive_bools)]
pub struct EditorWorkspace {
	dock_state: DockState<Box<dyn Panel>>,
	project_assets_root: Option<std::path::PathBuf>,
	selection: Option<crate::panels::EditorSelection>,
	mode: EditorMode,
	undo_redo: UndoRedoBuffer,
	dock_layout_debug_logged: bool,
	scene_create_prompt: Option<SceneCreatePrompt>,
	settings_open: bool,
	settings_category: SettingsCategory,
	physics_settings_scope: PhysicsSettingsScope,
	settings_project_dirty: bool,
	build_window_visibility: BuildWindowVisibility,
	build_window: Option<BuildWindowState>,
	// Needed for window management (close, minimize, maximize, drag)
	window: Arc<Window>,
	// Set when the close button is clicked; read by the main loop
	close_requested: bool,
	// Rendered height of the title bar panel, stored after each frame
	titlebar_height: f32,
	// Rects of every interactive element in the title bar, gathered each frame
	titlebar_interactive_rects: Vec<egui::Rect>,
	// Set to true when the managed C# assembly is hot-reloaded; read and
	// cleared by the InspectorPanel to invalidate its script_classes_cache.
	script_classes_dirty: bool,
	// The sheet+cell selected in the Texture Sheet panel as the active tile
	// brush; read by ScenePanel's Paint Tiles tool.
	active_tile_brush: Option<TileBrush>,
}

impl EditorWorkspace {
	pub fn new(
		viewport_texture: TextureId,
		project_assets_root: Option<std::path::PathBuf>,
		window: Arc<Window>,
	) -> Self {
		let mut workspace = Self {
			dock_state: DockState::new(vec![Box::new(ScenePanel::new(viewport_texture)) as Box<dyn Panel>]),
			project_assets_root,
			selection: None,
			mode: EditorMode::Edit,
			undo_redo: UndoRedoBuffer::new(EDITOR_UNDO_HISTORY_LIMIT),
			dock_layout_debug_logged: false,
			scene_create_prompt: None,
			settings_open: false,
			settings_category: SettingsCategory::Project,
			physics_settings_scope: PhysicsSettingsScope::ThisScene,
			settings_project_dirty: false,
			build_window_visibility: BuildWindowVisibility::Closed,
			build_window: None,
			window,
			close_requested: false,
			titlebar_height: 0.0,
			titlebar_interactive_rects: Vec::new(),
			script_classes_dirty: false,
			active_tile_brush: None,
		};

		workspace.configure_default_layout();
		workspace
	}

	#[expect(dead_code, reason = "Extensibility hook for future editor panels.")]
	pub fn add_panel<T: Panel + 'static>(&mut self, panel: T) { self.dock_state.push_to_focused_leaf(Box::new(panel)); }

	pub const fn mode(&self) -> EditorMode { self.mode }

	pub const fn set_mode(&mut self, mode: EditorMode) { self.mode = mode; }

	pub const fn project_is_loaded(&self) -> bool { self.project_assets_root.is_some() }

	pub fn set_project_assets_root(&mut self, project_assets_root: Option<std::path::PathBuf>) {
		self.project_assets_root = project_assets_root;
	}

	pub const fn mark_script_classes_dirty(&mut self) { self.script_classes_dirty = true; }

	pub fn request_play(&self, menu_request: &mut Option<EditorRequest>) {
		if !self.project_is_loaded() {
			ze_log::warn!("cannot enter Play mode without a project loaded");
			return;
		}
		if self.mode == EditorMode::Play {
			return;
		}

		*menu_request = Some(EditorRequest::EnterPlayMode);
	}

	pub fn request_stop(&self, menu_request: &mut Option<EditorRequest>) {
		if !self.project_is_loaded() {
			ze_log::warn!("cannot stop Play mode because no project is loaded");
			return;
		}
		if self.mode == EditorMode::Edit {
			return;
		}

		*menu_request = Some(EditorRequest::StopPlayMode);
	}

	pub fn clear_selection(&mut self) { self.selection = None; }

	pub const fn mark_project_config_saved(&mut self) { self.settings_project_dirty = false; }

	pub fn show(
		&mut self,
		ui: &mut Ui,
		viewport_texture: TextureId,
		scene: &mut Scene,
		mut project: Option<&mut Project>,
		scenes: &[SceneInfo],
		status: &EditorStatus,
	) -> EditorOutput {
		let mut menu_request = None;
		let mut project_saved = None;
		let mut project_save_error = None;
		let mut project_config_changed = false;

		if ui.ctx().input(|input| input.key_pressed(egui::Key::Escape)) {
			let tag_popup_open =
				egui::containers::AreaState::load(ui.ctx(), egui::Id::new("zero_editor_tag_popup")).is_some();
			if !tag_popup_open {
				self.clear_selection();
			}
		}
		if ui
			.ctx()
			.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::Z))
		{
			self.undo_redo.undo(scene);
		}
		if ui
			.ctx()
			.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::Y))
		{
			self.undo_redo.redo(scene);
		}
		if self.mode == EditorMode::Edit
			&& ui
				.ctx()
				.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::S))
		{
			menu_request = Some(EditorRequest::SaveProject);
		}

		self.show_title_bar(
			ui,
			viewport_texture,
			scene,
			project.as_deref(),
			scenes,
			&mut menu_request,
		);
		self.show_top_bar(
			ui,
			viewport_texture,
			scene,
			project.as_deref(),
			scenes,
			&mut menu_request,
		);

		let build_status = status.build;
		self.show_status_bar(ui, status);

		for (_, panel) in self.dock_state.iter_all_tabs_mut() {
			panel.set_viewport_texture(viewport_texture);
		}

		let mut tab_viewer = EditorTabViewer {
			context: EditorPanelContext {
				scene,
				selection: &mut self.selection,
				mode: self.mode,
				undo_redo: &mut self.undo_redo,
				project_save_requested: &mut project_saved,
				editor_request: &mut menu_request,
				project: project.as_deref(),
				script_classes_dirty: &mut self.script_classes_dirty,
				active_tile_brush: &mut self.active_tile_brush,
			},
		};
		DockArea::new(&mut self.dock_state)
			.style(dock_style_from_egui(ui.style().as_ref()))
			.show_inside(ui, &mut tab_viewer);
		self.log_dock_layout_once();

		if let Some(EditorRequest::OpenTextureSheetAsset(path)) = &menu_request {
			let path = path.clone();
			self.selection = Some(crate::panels::EditorSelection::Asset(path));
			self.open_or_focus_panel(WindowPanel::TextureSheet, viewport_texture);
			menu_request = None;
		}

		if let Some(error) = self.save_dirty_project_settings_for_request(menu_request.as_ref(), &mut project_saved) {
			project_save_error = Some(error);
		}
		self.show_scene_create_prompt(ui, &mut menu_request);
		if self.show_settings_window(ui, scene, project.as_deref_mut(), scenes) {
			project_config_changed = true;
		}
		let (build_project_changed, requested_build) = self.show_build_window(ui, project, scenes, build_status);
		project_config_changed |= build_project_changed;
		let build_request = requested_build;

		let viewport = self
			.dock_state
			.iter_all_tabs_mut()
			.find_map(|(_, panel)| panel.take_viewport_output());

		let close_requested = std::mem::replace(&mut self.close_requested, false);
		EditorOutput {
			viewport,
			build_request,
			menu_request,
			project_saved,
			project_save_error,
			project_config_changed,
			close_requested,
			titlebar_height: self.titlebar_height,
			titlebar_interactive_rects: std::mem::take(&mut self.titlebar_interactive_rects),
		}
	}

	fn show_title_bar(
		&mut self,
		ui: &mut Ui,
		viewport_texture: TextureId,
		scene: &mut Scene,
		_project: Option<&Project>,
		scenes: &[SceneInfo],
		menu_request: &mut Option<EditorRequest>,
	) {
		// Clear interactive rects at the start of each frame
		self.titlebar_interactive_rects.clear();

		let panel_response = egui::Panel::top("title_bar")
			.min_size(22.0)
			.frame(
				egui::Frame::NONE
					.fill(BG_BASE)
					.inner_margin(egui::Margin::symmetric(4, 2)),
			)
			.show_inside(ui, |ui| {
				ui.horizontal_centered(|ui| {
					ui.add_space(4.0);

					let prev = ui.style_mut().visuals.button_frame;
					ui.style_mut().visuals.button_frame = false;

					let ze_menu = ui.menu_button(RichText::new("ZeroEngine").size(11.0), |ui| {
						ui.label("ZeroEngine 0.1.0");
						ui.separator();
						if ui.button("About").clicked() {
							ui.close();
						}
					});
					self.titlebar_interactive_rects.push(ze_menu.response.rect);
					let file_menu = ui.menu_button("File", |ui| {
						if ui.button("New Project").clicked() {
							*menu_request = Some(EditorRequest::NewProject);
							ui.close();
						}
						if ui.button("Open Project").clicked() {
							*menu_request = Some(EditorRequest::OpenProject);
							ui.close();
						}
						ui.separator();
						if ui
							.add_enabled(
								self.project_is_loaded() && self.mode == EditorMode::Edit,
								egui::Button::new("New Scene"),
							)
							.clicked()
						{
							self.begin_scene_create_prompt();
							ui.close();
						}
						ui.add_enabled_ui(self.project_is_loaded() && self.mode == EditorMode::Edit, |ui| {
							Self::show_open_scene_menu(ui, scenes, menu_request);
						});
						if ui
							.add_enabled(
								self.project_is_loaded() && self.mode == EditorMode::Edit,
								egui::Button::new("Save Project"),
							)
							.clicked()
						{
							*menu_request = Some(EditorRequest::SaveProject);
							ui.close();
						}
						ui.separator();
						if ui
							.add_enabled(self.project_is_loaded(), egui::Button::new("Build"))
							.clicked()
						{
							self.build_window_visibility = BuildWindowVisibility::Open;
							ui.close();
						}
						ui.separator();
						let _ = ui.button("Exit").clicked();
					});
					self.titlebar_interactive_rects.push(file_menu.response.rect);
					let edit_menu = ui.menu_button("Edit", |ui| {
						if ui.button("Undo").clicked() {
							self.undo_redo.undo(scene);
							ui.close();
						}
						if ui.button("Redo").clicked() {
							self.undo_redo.redo(scene);
							ui.close();
						}
						if ui
							.add_enabled(self.project_is_loaded(), egui::Button::new("Settings"))
							.clicked()
						{
							self.settings_open = true;
							ui.close();
						}
					});
					self.titlebar_interactive_rects.push(edit_menu.response.rect);
					let window_menu = ui.menu_button("Window", |ui| {
						ui.menu_button("Panel", |ui| {
							for panel in WindowPanel::ALL {
								if ui.button(panel.menu_label()).clicked() {
									self.open_or_focus_panel(panel, viewport_texture);
									ui.close();
								}
							}
						});
					});
					self.titlebar_interactive_rects.push(window_menu.response.rect);

					ui.style_mut().visuals.button_frame = prev;

					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						let button_size = egui::vec2(28.0, 22.0);

						let close_resp = ui.allocate_response(button_size, Sense::click());
						if close_resp.hovered() {
							ui.painter().rect_filled(
								close_resp.rect,
								egui::CornerRadius::ZERO,
								egui::Color32::from_rgb(196, 43, 28),
							);
						}
						ui.painter().text(
							close_resp.rect.center(),
							egui::Align2::CENTER_CENTER,
							"󰅖",
							egui::FontId::proportional(12.0),
							egui::Color32::WHITE,
						);
						if close_resp.clicked() {
							self.close_requested = true;
						}
						self.titlebar_interactive_rects.push(close_resp.rect);

						let max_resp = ui.add_sized(
							button_size,
							egui::Button::new(RichText::new("󰖯").size(12.0)).frame(false),
						);
						if max_resp.clicked() {
							self.window.set_maximized(true);
						}
						self.titlebar_interactive_rects.push(max_resp.rect);

						let min_resp = ui.add_sized(
							button_size,
							egui::Button::new(RichText::new("󰖰").size(12.0)).frame(false),
						);
						if min_resp.clicked() {
							self.window.set_minimized(true);
						}
						self.titlebar_interactive_rects.push(min_resp.rect);

						ui.allocate_ui(egui::vec2(ui.available_width(), ui.available_height()), |ui| {
							ui.centered_and_justified(|ui| {
								ui.label(RichText::new("ZeroEditor").size(11.0).color(TEXT_MUTED));
							});
						});
					});
				});
			});
		// Store the rendered panel height for winit-level titlebar hit-testing
		self.titlebar_height = panel_response.response.rect.height();
	}

	#[expect(
		clippy::needless_pass_by_ref_mut,
		reason = "play/stop routing requires future mutable access"
	)]
	fn show_top_bar(
		&mut self,
		ui: &mut Ui,
		_viewport_texture: TextureId,
		_scene: &Scene,
		project: Option<&Project>,
		scenes: &[SceneInfo],
		menu_request: &mut Option<EditorRequest>,
	) {
		#[allow(deprecated)]
		egui::TopBottomPanel::top("top_bar")
			.frame(
				egui::Frame::NONE
					.fill(BG_BASE)
					.inner_margin(egui::Margin::symmetric(6, 4)),
			)
			.show_inside(ui, |ui| {
				let total = ui.available_width();
				let play_w = 100.0;
				let left_w = ((total - play_w) / 2.0).max(10.0);

				ui.horizontal(|ui| {
					ui.allocate_ui(egui::vec2(left_w, ui.available_height()), |ui| {
						ui.horizontal_centered(|ui| {
							ui.label(RichText::new("ZE").strong());
							ui.separator();
							let project_name = project.map_or_else(|| "—".to_string(), |p| p.name.clone());
							ui.menu_button(abbreviate(&project_name), |ui| {
								ui.add_enabled(false, egui::Button::new(project_name));
							});
							self.show_scene_selector(ui, scenes, menu_request);
						});
					});

					ui.allocate_ui(egui::vec2(play_w, ui.available_height()), |ui| {
						ui.horizontal_centered(|ui| {
							let play_clicked = ui
								.add_enabled(
									self.mode == EditorMode::Edit && self.project_is_loaded(),
									egui::Button::new(RichText::new("󰐊").size(16.0)),
								)
								.clicked();
							let stop_clicked = ui
								.add_enabled(
									self.mode == EditorMode::Play && self.project_is_loaded(),
									egui::Button::new(RichText::new("󰓛").size(16.0)),
								)
								.clicked();
							if play_clicked {
								self.request_play(menu_request);
							}
							if stop_clicked {
								self.request_stop(menu_request);
							}
						});
					});
				});
			});
	}

	fn show_status_bar(&self, ui: &mut Ui, status: &EditorStatus) {
		#[allow(deprecated)]
		egui::TopBottomPanel::bottom("status_bar")
			.frame(
				egui::Frame::NONE
					.fill(BG_BASE)
					.inner_margin(egui::Margin::symmetric(8, 2))
					.rounding(egui::CornerRadius::same(10)),
			)
			.show_inside(ui, |ui| {
				ui.horizontal(|ui| {
					ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
						ui.add_space(6.0);
						ui.label(format!("ZeroEngine {}", env!("CARGO_PKG_VERSION")));
						ui.separator();
						show_save_status(ui, self.visible_save_status(status.save), self.project_is_loaded());
						ui.separator();
						show_script_status(ui, status.script);
						ui.separator();
						if let Some(msg) = &status.build_message {
							ui.label(msg);
							ui.separator();
						}
						ui.label(format!(
							"Undo: {} / Redo: {}",
							self.undo_redo.undo_depth(),
							self.undo_redo.redo_depth()
						));
					});
				});
			});
	}

	fn show_scene_selector(&self, ui: &mut Ui, scenes: &[SceneInfo], menu_request: &mut Option<EditorRequest>) {
		let active_scene = scenes
			.iter()
			.find(|scene| scene.active)
			.map_or("No scene", |scene| scene.name.as_str());
		let menu_height = ui.spacing().interact_size.y * SCENE_SELECTOR_VISIBLE_ROWS;
		ui.add_enabled_ui(self.project_is_loaded() && self.mode == EditorMode::Edit, |ui| {
			egui::ComboBox::from_id_salt("active_scene_selector")
				.selected_text(abbreviate(active_scene))
				.width(80.0)
				.height(menu_height)
				.show_ui(ui, |ui| {
					for scene_info in scenes {
						let label = if scene_info.dirty {
							format!("{} *", scene_info.name)
						} else {
							scene_info.name.clone()
						};
						if ui.selectable_label(scene_info.active, label).clicked() && !scene_info.active {
							*menu_request = Some(EditorRequest::SwitchScene(scene_info.name.clone()));
							ui.close();
						}
					}
				});
		});
	}

	fn show_open_scene_menu(ui: &mut Ui, scenes: &[SceneInfo], menu_request: &mut Option<EditorRequest>) {
		ui.menu_button("Open Scene", |ui| {
			if scenes.is_empty() {
				ui.label("No scenes in project");
				return;
			}

			for scene_info in scenes {
				let label = if scene_info.active {
					format!("{} (active)", scene_info.name)
				} else {
					scene_info.name.clone()
				};
				if ui.add_enabled(!scene_info.active, egui::Button::new(label)).clicked() {
					*menu_request = Some(EditorRequest::SwitchScene(scene_info.name.clone()));
					ui.close();
				}
			}
		});
	}

	fn show_settings_window(
		&mut self,
		ui: &Ui,
		scene: &mut Scene,
		mut project: Option<&mut Project>,
		scenes: &[SceneInfo],
	) -> bool {
		if !self.settings_open {
			return false;
		}

		let mut project_changed = false;
		let mut open = self.settings_open;
		egui::Window::new("Settings")
			.open(&mut open)
			.default_width(560.0)
			.default_height(380.0)
			.show(ui.ctx(), |ui| {
				ui.horizontal(|ui| {
					ui.vertical(|ui| {
						ui.set_width(140.0);
						for category in SettingsCategory::ALL {
							if ui
								.selectable_label(self.settings_category == category, category.label())
								.clicked()
							{
								self.settings_category = category;
							}
						}
					});
					ui.separator();
					ui.vertical(|ui| {
						ui.set_min_width(360.0);
						match self.settings_category {
							SettingsCategory::Project => {
								if let Some(project) = project.as_deref_mut() {
									if show_project_settings(ui, project, scenes) {
										self.settings_project_dirty = true;
										project_changed = true;
									}
								} else {
									ui.label("No project loaded.");
								}
							}
							SettingsCategory::Physics => {
								if self.show_physics_settings(ui, scene, project) {
									self.settings_project_dirty = true;
									project_changed = true;
								}
							}
						}
					});
				});
			});
		self.settings_open = open;
		project_changed
	}

	fn show_build_window(
		&mut self,
		ui: &Ui,
		project: Option<&mut Project>,
		scenes: &[SceneInfo],
		build_status: BuildStatus,
	) -> (bool, Option<BuildRequest>) {
		if self.build_window_visibility == BuildWindowVisibility::Closed {
			return (false, None);
		}

		if let Some(project) = project.as_ref() {
			self.sync_build_window_state(project);
		}

		let mut project_changed = false;
		let mut build_request = None;
		let mut open = true;
		egui::Window::new("Build")
			.open(&mut open)
			.default_width(520.0)
			.default_height(360.0)
			.show(ui.ctx(), |ui| {
				let Some(project) = project else {
					ui.label("No project loaded.");
					return;
				};

				ui.label("Scenes included in build");
				if show_project_scene_build_list(ui, project, scenes) {
					self.settings_project_dirty = true;
					project_changed = true;
				}

				ui.separator();

				if let Some(state) = self.build_window.as_mut() {
					ui.horizontal(|ui| {
						ui.label("Target");
						egui::ComboBox::from_id_salt("build_target")
							.selected_text(match state.target {
								BuildTarget::Linux => "Linux",
								BuildTarget::Windows => "Windows",
							})
							.show_ui(ui, |ui| {
								if ui
									.selectable_label(state.target == BuildTarget::Linux, "Linux")
									.clicked()
								{
									state.target = BuildTarget::Linux;
									ui.close();
								}
								if ui
									.selectable_label(state.target == BuildTarget::Windows, "Windows")
									.clicked()
								{
									state.target = BuildTarget::Windows;
									ui.close();
								}
							});
					});
				}

				ui.separator();

				let mut output_path = None;
				ui.horizontal(|ui| {
					ui.label("Output path");
					if let Some(state) = self.build_window.as_mut() {
						let output_dir_empty = state.output_dir.is_empty();
						let mut text_edit = egui::TextEdit::singleline(&mut state.output_dir);
						if output_dir_empty {
							text_edit = text_edit.hint_text("./dist");
						}
						ui.add(text_edit);
						output_path = Some(build_output_dir(project, &state.output_dir.clone()));
					}
				});

				ui.separator();

				let can_build = build_status != BuildStatus::Running;
				let target = self.build_window.as_ref().map_or(BuildTarget::host(), |s| s.target);
				if ui.add_enabled(can_build, egui::Button::new("Build")).clicked()
					&& let Some(output_dir) = output_path
				{
					build_request = Some(BuildRequest { output_dir, target });
				}
			});
		self.build_window_visibility = if open {
			BuildWindowVisibility::Open
		} else {
			BuildWindowVisibility::Closed
		};
		(project_changed, build_request)
	}

	fn sync_build_window_state(&mut self, project: &Project) {
		let needs_reset = self
			.build_window
			.as_ref()
			.is_none_or(|state| state.project_root != project.root_dir);
		if needs_reset {
			self.build_window = Some(BuildWindowState::from_project(project));
		}
	}

	fn show_physics_settings(&mut self, ui: &mut Ui, scene: &mut Scene, project: Option<&mut Project>) -> bool {
		ui.horizontal(|ui| {
			ui.label("Scope");
			ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
				egui::ComboBox::from_id_salt("physics_settings_scope")
					.selected_text(self.physics_settings_scope.label())
					.show_ui(ui, |ui| {
						for scope in [PhysicsSettingsScope::ThisScene, PhysicsSettingsScope::AllScenesDefault] {
							if ui
								.selectable_label(self.physics_settings_scope == scope, scope.label())
								.clicked()
							{
								self.physics_settings_scope = scope;
								ui.close();
							}
						}
					});
			});
		});
		ui.separator();

		match self.physics_settings_scope {
			PhysicsSettingsScope::ThisScene => {
				self.show_scene_physics_settings(ui, scene);
				false
			}
			PhysicsSettingsScope::AllScenesDefault => {
				let Some(project) = project else {
					ui.label("No project loaded.");
					return false;
				};
				let changed = show_project_physics_settings(ui, &mut project.physics);
				if changed {
					apply_project_physics_default_to_scene_helper(scene, &project.physics);
				}
				changed
			}
		}
	}

	fn show_scene_physics_settings(&mut self, ui: &mut Ui, scene: &mut Scene) {
		let Some(entity) = persistent_physics_settings_entity(scene) else {
			if ui.button("Create Scene Override").clicked() {
				let before = scene.snapshot();
				let entity = scene.create_entity("PhysicsSettings");
				scene.entity_mut(entity).add_component(PhysicsSettings::default());
				let after = scene.snapshot();
				self.undo_redo
					.record_executed(Box::new(SceneSnapshotCommand::new(before, after)));
			}
			return;
		};

		let Ok(settings_ref) = scene.world().get::<&PhysicsSettings>(entity) else {
			return;
		};
		let mut settings = settings_ref.clone();
		drop(settings_ref);

		if show_physics_settings_fields(ui, &mut settings) {
			let before = scene.snapshot();
			if let Ok(mut current) = scene.world_mut().get::<&mut PhysicsSettings>(entity) {
				**current = settings;
			}
			let after = scene.snapshot();
			self.undo_redo
				.record_executed(Box::new(SceneSnapshotCommand::new(before, after)));
		}
	}

	fn begin_scene_create_prompt(&mut self) { self.scene_create_prompt = Some(SceneCreatePrompt::default()); }

	fn show_scene_create_prompt(&mut self, ui: &Ui, menu_request: &mut Option<EditorRequest>) {
		let Some(prompt) = self.scene_create_prompt.as_mut() else {
			return;
		};

		let input_id = ui.id().with("editor_scene_create_name");
		let mut confirm = false;
		let mut cancel = false;

		egui::Window::new("New Scene")
			.id(ui.id().with("editor_scene_create_popup"))
			.collapsible(false)
			.resizable(false)
			.default_width(260.0)
			.anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
			.show(ui.ctx(), |ui| {
				ui.label("Scene name");
				let response = ui.add(
					egui::TextEdit::singleline(&mut prompt.name)
						.id(input_id)
						.desired_width(f32::INFINITY),
				);
				response.request_focus();

				if let Some(error) = &prompt.error {
					ui.colored_label(egui::Color32::YELLOW, error);
				}

				ui.horizontal(|ui| {
					if ui.button("Create").clicked() {
						confirm = true;
					}
					if ui.button("Cancel").clicked() {
						cancel = true;
					}
				});

				confirm |= response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
				cancel |= ui.input(|input| input.key_pressed(egui::Key::Escape));
			});

		if confirm {
			let name = prompt.name.trim().to_string();
			if name.is_empty() {
				prompt.error = Some("Scene name cannot be empty".to_string());
			} else {
				*menu_request = Some(EditorRequest::CreateScene(name));
				self.scene_create_prompt = None;
			}
		} else if cancel {
			self.scene_create_prompt = None;
		}
	}

	fn visible_save_status(&self, save_status: SaveStatus) -> SaveStatus {
		if (self.settings_project_dirty || self.has_unsaved_project_changes()) && save_status.state == SaveState::Saved
		{
			return save_status.with_state(SaveState::UnsavedChanges);
		}

		save_status
	}

	fn has_unsaved_project_changes(&self) -> bool {
		self.dock_state
			.iter_all_tabs()
			.any(|(_, panel)| panel.has_unsaved_project_changes())
	}

	fn save_dirty_project_settings_for_request(
		&mut self,
		menu_request: Option<&EditorRequest>,
		project_saved: &mut Option<Project>,
	) -> Option<String> {
		if !matches!(menu_request, Some(EditorRequest::SaveProject)) {
			return None;
		}

		match self.save_dirty_project_settings() {
			Ok(saved_project) => {
				*project_saved = saved_project;
				None
			}
			Err(error) => Some(error),
		}
	}

	fn save_dirty_project_settings(&mut self) -> Result<Option<Project>, String> {
		let mut saved_project = None;
		let mut errors = Vec::new();
		for (_, panel) in self.dock_state.iter_all_tabs_mut() {
			let Some(result) = panel.save_project_if_dirty() else {
				continue;
			};

			match result {
				Ok(project) => saved_project = Some(project),
				Err(error) => {
					ze_log::warn!("{error}");
					errors.push(error);
				}
			}
		}

		if errors.is_empty() {
			Ok(saved_project)
		} else {
			Err(errors.join("; "))
		}
	}

	fn configure_default_layout(&mut self) {
		let files_panel = self.create_files_panel();
		let surface = self.dock_state.main_surface_mut();

		// Split right column (Hierarchy) from root first so it spans full height
		let [center_node, right_node] = surface.split_right(
			NodeIndex::root(),
			DEFAULT_RIGHT_COLUMN_SHARE,
			vec![Box::new(SceneHierarchyPanel::new()) as Box<dyn Panel>],
		);

		// Split right column vertically: Hierarchy on top, Inspector below
		surface.split_below(
			right_node,
			DEFAULT_HIERARCHY_SHARE_IN_RIGHT_COLUMN,
			vec![Box::new(InspectorPanel::new()) as Box<dyn Panel>],
		);

		// Split bottom from center only — Files+Console go under the viewport,
		// not under the right column
		surface.split_below(
			center_node,
			DEFAULT_BOTTOM_PANEL_SHARE,
			vec![files_panel, Box::new(ConsolePanel::new()) as Box<dyn Panel>],
		);
	}

	const fn log_dock_layout_once(&mut self) {
		if self.dock_layout_debug_logged {
			return;
		}
		self.dock_layout_debug_logged = true;
	}

	fn open_or_focus_panel(&mut self, panel: WindowPanel, viewport_texture: TextureId) {
		if let Some(path) = self
			.dock_state
			.find_tab_from(|existing_panel| existing_panel.name() == panel.tab_name())
		{
			if let Err(error) = self.dock_state.set_active_tab(path) {
				ze_log::error!("failed to activate editor panel `{}`: {error:?}", panel.menu_label());
				return;
			}

			self.dock_state.set_focused_node_and_surface(path.node_path());
			return;
		}

		self.dock_state
			.push_to_focused_leaf(self.create_panel(panel, viewport_texture));
	}

	fn create_panel(&self, panel: WindowPanel, viewport_texture: TextureId) -> Box<dyn Panel> {
		match panel {
			WindowPanel::Scene => Box::new(ScenePanel::new(viewport_texture)),
			WindowPanel::Hierarchy => Box::new(SceneHierarchyPanel::new()),
			WindowPanel::Inspector => Box::new(InspectorPanel::new()),
			WindowPanel::AssetBrowser | WindowPanel::Files => self.create_files_panel(),
			WindowPanel::Console => Box::new(ConsolePanel::new()),
			WindowPanel::TextureSheet => Box::new(TextureSheetPanel::new()),
		}
	}

	fn create_files_panel(&self) -> Box<dyn Panel> {
		match &self.project_assets_root {
			Some(assets_root) => Box::new(FileExplorer::new(assets_root.clone())),
			None => Box::new(NoProjectPanel::new()),
		}
	}
}

fn show_save_status(ui: &mut Ui, status: SaveStatus, project_loaded: bool) {
	let (rect, response) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
	ui.painter().rect_filled(
		rect,
		egui::CornerRadius::same(2),
		save_status_color(status.state, project_loaded),
	);
	response.on_hover_text(save_status_tooltip(status, project_loaded));
}

const fn save_state_label(state: SaveState) -> &'static str {
	match state {
		SaveState::Saved => "Saved",
		SaveState::Saving => "Saving...",
		SaveState::UnsavedChanges => "Unsaved changes",
		SaveState::SaveFailed => "Save failed",
	}
}

const fn save_status_color(state: SaveState, project_loaded: bool) -> egui::Color32 {
	if !project_loaded {
		return egui::Color32::from_rgb(130, 130, 136);
	}

	match state {
		SaveState::Saved => egui::Color32::from_rgb(80, 190, 110),
		SaveState::Saving | SaveState::UnsavedChanges => egui::Color32::from_rgb(230, 185, 70),
		SaveState::SaveFailed => egui::Color32::from_rgb(230, 80, 80),
	}
}

fn save_status_tooltip(status: SaveStatus, project_loaded: bool) -> String {
	let current = if project_loaded {
		save_state_label(status.state)
	} else {
		"No project loaded"
	};
	let last_save = status
		.last_successful_save
		.map_or_else(|| "no successful save yet".to_string(), relative_save_time);

	format!(
		"{current}\n{last_save}\nGreen: saved\nYellow: saving or unsaved changes\nRed: save failed\nGray: no project loaded"
	)
}

fn relative_save_time(saved_at: Instant) -> String {
	let elapsed = saved_at.elapsed();
	let seconds = elapsed.as_secs();
	if seconds < 5 {
		return "last save just now".to_string();
	}
	if seconds < 60 {
		return format!("last save {seconds}s ago");
	}

	let minutes = seconds / 60;
	if minutes < 60 {
		return format!("last save {minutes}m ago");
	}

	let hours = minutes / 60;
	format!("last save {hours}h ago")
}

fn show_script_status(ui: &mut Ui, status: ScriptStatus) {
	let (rect, response) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
	ui.painter()
		.rect_filled(rect, egui::CornerRadius::same(2), script_status_color(status));
	response.on_hover_text(format!(
		"{}\n\nGreen: scripts are ready\nYellow: compiling\nRed: compile error\nGray: no scripts project or unknown",
		script_status_label(status)
	));
}

const fn section_separator_color() -> egui::Color32 { egui::Color32::from_rgb(54, 54, 54) }

fn dock_style_from_egui(style: &egui::Style) -> Style {
	let mut dock_style = Style::from_egui(style);

	dock_style.main_surface_border_stroke = egui::Stroke::NONE;
	dock_style.tab_bar.bg_fill = BG_BASE;
	dock_style.tab_bar.hline_color = section_separator_color();
	dock_style.tab.tab_body.bg_fill = BG_BASE;
	dock_style.tab.tab_body.stroke = egui::Stroke::NONE;

	for tab in [
		&mut dock_style.tab.active,
		&mut dock_style.tab.focused,
		&mut dock_style.tab.hovered,
		&mut dock_style.tab.active_with_kb_focus,
		&mut dock_style.tab.inactive_with_kb_focus,
		&mut dock_style.tab.focused_with_kb_focus,
	] {
		tab.text_color = TEXT_PRIMARY;
		tab.outline_color = ACCENT;
	}

	dock_style.tab.inactive.bg_fill = BG_BASE;
	dock_style.tab.inactive.text_color = TEXT_PRIMARY;
	dock_style.tab.inactive.outline_color = BG_BASE;
	dock_style.separator.width = 2.0;
	dock_style.separator.color_idle = egui::Color32::from_rgb(64, 64, 64);
	dock_style.separator.color_hovered = ACCENT;
	dock_style.separator.color_dragged = ACCENT;
	dock_style.overlay.selection_color = ACCENT.linear_multiply(0.5);
	dock_style.overlay.button_color = ACCENT;
	dock_style.overlay.button_border_stroke = egui::Stroke::new(1.0, ACCENT);

	dock_style
}

pub fn switch_viewport_camera(scene: &mut Scene, mode: EditorMode) {
	let editor_camera = find_editor_camera(scene);
	let game_camera = find_game_camera(scene, editor_camera);

	match mode {
		EditorMode::Edit => {
			if let Some(game_camera) = game_camera {
				set_camera_primary(scene, game_camera, false);
			}
			if let Some(editor_camera) = editor_camera {
				set_camera_primary(scene, editor_camera, true);
			} else {
				ze_log::warn!("no editor viewport camera found in scene `{}`", scene.name);
			}
		}
		EditorMode::Play => {
			if let Some(game_camera) = game_camera {
				if let Some(editor_camera) = editor_camera {
					set_camera_primary(scene, editor_camera, false);
				}
				set_camera_primary(scene, game_camera, true);
			} else {
				ze_log::warn!(
					"no primary game camera found in scene `{}`; keeping the editor camera active",
					scene.name
				);
				if let Some(editor_camera) = editor_camera {
					set_camera_primary(scene, editor_camera, true);
				}
			}
		}
	}
}

fn find_editor_camera(scene: &Scene) -> Option<ze_ecs::EntityId> {
	let world = scene.world();
	let mut camera = None;

	world.run(|entities: ze_ecs::EntitiesView| {
		for entity in entities.iter() {
			let Ok(_) = world.get::<&Camera>(entity) else {
				continue;
			};

			if !is_editor_camera(scene, entity) {
				continue;
			}

			camera = Some(entity);
			break;
		}
	});

	camera
}

fn find_game_camera(scene: &Scene, editor_camera: Option<ze_ecs::EntityId>) -> Option<ze_ecs::EntityId> {
	let world = scene.world();
	let mut primary_camera = None;
	let mut fallback_camera = None;

	world.run(|entities: ze_ecs::EntitiesView| {
		for entity in entities.iter() {
			let Ok(camera) = world.get::<&Camera>(entity) else {
				continue;
			};

			if editor_camera.is_some_and(|editor| editor == entity) || is_editor_camera(scene, entity) {
				continue;
			}

			if fallback_camera.is_none() {
				fallback_camera = Some(entity);
			}
			if camera.primary {
				primary_camera = Some(entity);
				break;
			}
		}
	});

	primary_camera.or(fallback_camera)
}

fn is_editor_camera(scene: &Scene, entity: ze_ecs::EntityId) -> bool {
	let world = scene.world();
	world.get::<&EditorOnly>(entity).is_ok()
}

fn set_camera_primary(scene: &mut Scene, entity: ze_ecs::EntityId, primary: bool) {
	if let Ok(mut camera) = scene.world_mut().get::<&mut Camera>(entity) {
		camera.primary = primary;
	}
}

fn show_project_settings(ui: &mut Ui, project: &mut Project, scenes: &[SceneInfo]) -> bool {
	let mut changed = false;
	ui.horizontal(|ui| {
		ui.label("Name");
		changed |= ui.text_edit_singleline(&mut project.name).changed();
	});
	ui.horizontal(|ui| {
		ui.label("Version");
		changed |= ui.text_edit_singleline(&mut project.game.version).changed();
	});

	let scene_names = project_scene_names(project, scenes);

	egui::ComboBox::from_label("Main scene")
		.selected_text(&project.main_scene)
		.show_ui(ui, |ui| {
			for scene_name in &scene_names {
				if ui
					.selectable_label(project.main_scene == *scene_name, scene_name)
					.clicked()
				{
					if let Err(error) = project.set_main_scene(scene_name.clone()) {
						ze_log::warn!("failed to set main scene `{scene_name}` from Settings: {error:?}");
					} else {
						changed = true;
					}
					ui.close();
				}
			}
		});

	ui.separator();
	ui.label("Scenes included in build");
	changed |= show_project_scene_build_list(ui, project, scenes);

	changed
}

fn show_project_scene_build_list(ui: &mut Ui, project: &mut Project, scenes: &[SceneInfo]) -> bool {
	let mut changed = false;
	let scene_names = project_scene_names(project, scenes);
	if scene_names.is_empty() {
		ui.label("No scenes found.");
		return changed;
	}

	for scene_name in scene_names {
		let is_main_scene = scene_name == project.main_scene;
		let mut included = project.scenes.iter().any(|scene| scene == &scene_name);
		let response = ui.add_enabled(
			!is_main_scene,
			egui::Checkbox::new(&mut included, build_scene_label(&scene_name, is_main_scene)),
		);
		if !response.changed() {
			continue;
		}

		if included {
			match project.add_scene(scene_name) {
				Ok(scene_changed) => changed |= scene_changed,
				Err(error) => ze_log::warn!("failed to include scene in build from Settings: {error:?}"),
			}
		} else if let Some(index) = project.scenes.iter().position(|scene| scene == &scene_name) {
			project.scenes.remove(index);
			changed = true;
		}
	}

	changed
}

fn project_scene_names(project: &Project, scenes: &[SceneInfo]) -> Vec<String> {
	let mut scene_names = scenes.iter().map(|scene| scene.name.clone()).collect::<Vec<_>>();
	scene_names.extend(project.scenes.iter().cloned());
	scene_names.sort();
	scene_names.dedup();
	scene_names
}

fn build_scene_label(scene_name: &str, is_main_scene: bool) -> String {
	if is_main_scene {
		format!("{scene_name} (main)")
	} else {
		scene_name.to_string()
	}
}

fn show_project_physics_settings(ui: &mut Ui, settings: &mut ProjectPhysicsSettings) -> bool {
	let mut edited = project_physics_settings_to_ecs(settings);
	if !show_physics_settings_fields(ui, &mut edited) {
		return false;
	}

	settings.gravity.x = edited.gravity.x;
	settings.gravity.y = edited.gravity.y;
	settings.timestep = edited.physics_timestep;
	settings.enable_debug_draw = edited.enable_debug_draw;
	true
}

fn show_physics_settings_fields(ui: &mut Ui, settings: &mut PhysicsSettings) -> bool {
	let mut changed = false;
	ui.horizontal(|ui| {
		ui.label("Gravity");
		ui.label("X");
		changed |= ui
			.add(egui::DragValue::new(&mut settings.gravity.x).speed(0.05))
			.changed();
		ui.label("Y");
		changed |= ui
			.add(egui::DragValue::new(&mut settings.gravity.y).speed(0.05))
			.changed();
	});
	changed |= ui
		.checkbox(&mut settings.enable_debug_draw, "Enable debug draw")
		.changed();
	ui.horizontal(|ui| {
		ui.label("Physics timestep");
		changed |= ui
			.add(egui::DragValue::new(&mut settings.physics_timestep).speed(0.001))
			.changed();
	});
	changed
}

const fn project_physics_settings_to_ecs(settings: &ProjectPhysicsSettings) -> PhysicsSettings {
	PhysicsSettings {
		gravity: Vec2::new(settings.gravity.x, settings.gravity.y),
		enable_debug_draw: settings.enable_debug_draw,
		physics_timestep: settings.timestep,
	}
}

fn apply_project_physics_default_to_scene_helper(scene: &mut Scene, settings: &ProjectPhysicsSettings) {
	if persistent_physics_settings_entity(scene).is_some() {
		return;
	}

	let Some(entity) = physics_settings_entity(scene) else {
		return;
	};
	if scene.world().get::<&EditorOnly>(entity).is_err() {
		return;
	}
	if let Ok(mut current) = scene.world_mut().get::<&mut PhysicsSettings>(entity) {
		**current = project_physics_settings_to_ecs(settings);
	}
}

fn build_output_dir(project: &Project, output_dir: &str) -> PathBuf {
	let trimmed = output_dir.trim();
	let path = if trimmed.is_empty() {
		PathBuf::from("dist")
	} else {
		PathBuf::from(trimmed)
	};
	if path.is_absolute() {
		path
	} else {
		project.root_dir.join(path)
	}
}

fn persistent_physics_settings_entity(scene: &Scene) -> Option<ze_ecs::EntityId> {
	let world = scene.world();
	let mut matching_entity = None;
	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&PhysicsSettings>(entity).is_ok() && world.get::<&EditorOnly>(entity).is_err() {
				matching_entity = Some(entity);
				break;
			}
		}
	});
	matching_entity
}

fn physics_settings_entity(scene: &Scene) -> Option<ze_ecs::EntityId> {
	let world = scene.world();
	let mut matching_entity = None;
	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&PhysicsSettings>(entity).is_ok() {
				matching_entity = Some(entity);
				break;
			}
		}
	});
	matching_entity
}

const fn script_status_color(status: ScriptStatus) -> egui::Color32 {
	match status {
		ScriptStatus::Unavailable => egui::Color32::from_rgb(130, 130, 136),
		ScriptStatus::UpToDate => egui::Color32::from_rgb(80, 190, 110),
		ScriptStatus::Recompiling | ScriptStatus::ReloadPending => egui::Color32::from_rgb(230, 185, 70),
		ScriptStatus::Failed => egui::Color32::from_rgb(230, 80, 80),
	}
}

const fn script_status_label(status: ScriptStatus) -> &'static str {
	match status {
		ScriptStatus::Unavailable => "Scripts: unavailable",
		ScriptStatus::UpToDate => "Ready",
		ScriptStatus::Recompiling | ScriptStatus::ReloadPending => "Compiling",
		ScriptStatus::Failed => "Error",
	}
}

struct EditorTabViewer<'a> {
	context: EditorPanelContext<'a>,
}

impl TabViewer for EditorTabViewer<'_> {
	type Tab = Box<dyn Panel>;

	fn title(&mut self, tab: &mut Self::Tab) -> WidgetText { tab.name().into() }

	fn ui(&mut self, ui: &mut Ui, tab: &mut Self::Tab) { tab.show(ui, &mut self.context); }

	fn tab_style_override(&self, tab: &Self::Tab, global_style: &TabStyle) -> Option<TabStyle> {
		if tab.name() != "Scene" {
			return None;
		}

		let mut style = global_style.clone();
		// The viewport texture should meet the dock body edge without an editor frame
		// around it.
		style.tab_body.inner_margin = egui::Margin::ZERO;
		style.tab_body.stroke = egui::Stroke::NONE;
		style.tab_body.corner_radius = egui::CornerRadius::ZERO;
		for tab in [
			&mut style.active,
			&mut style.focused,
			&mut style.hovered,
			&mut style.active_with_kb_focus,
			&mut style.inactive,
			&mut style.inactive_with_kb_focus,
			&mut style.focused_with_kb_focus,
		] {
			tab.outline_color = tab.bg_fill;
		}
		Some(style)
	}

	fn clear_background(&self, tab: &Self::Tab) -> bool { tab.clear_background() }

	fn scroll_bars(&self, tab: &Self::Tab) -> [bool; 2] { tab.scroll_bars() }
}
