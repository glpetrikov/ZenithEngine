mod animation_clip;
mod console;
pub mod file_hierarchy;
mod hierarchy;
mod inspector;
mod no_project;
mod scene;
mod texture_sheet;
mod texture_sheet_common;
mod texture_sheet_create_dialog;

use std::path::PathBuf;

pub use animation_clip::AnimationClipPanel;
pub use console::ConsolePanel;
use egui::{TextureId, Ui};
pub use hierarchy::SceneHierarchyPanel;
pub use inspector::InspectorPanel;
pub use no_project::NoProjectPanel;
pub use scene::{ScenePanel, ViewportOutput};
pub use texture_sheet::{TextureSheetPanel, TileBrush};
use ze_ecs::{EntityId, Scene};
use ze_project::Project;

use crate::{
	editor_workspace::{EditorMode, EditorRequest},
	undo_redo::UndoRedoBuffer,
};

#[derive(Debug, Clone)]
pub enum EditorSelection {
	Entity(EntityId),
	Asset(PathBuf),
}

#[expect(dead_code, reason = "passes save request from panels to caller")]
pub struct EditorPanelContext<'a> {
	pub selection: &'a mut Option<EditorSelection>,
	pub scene: &'a mut Scene,
	pub mode: EditorMode,
	pub undo_redo: &'a mut UndoRedoBuffer,
	pub project_save_requested: &'a mut Option<Project>,
	pub editor_request: &'a mut Option<EditorRequest>,
	pub project: Option<&'a Project>,
	pub script_classes_dirty: &'a mut bool,
	/// The sheet+cell currently selected as the active tile-painting brush,
	/// set by `TextureSheetPanel` and read by `ScenePanel`'s Paint Tiles tool.
	pub active_tile_brush: &'a mut Option<TileBrush>,
}

pub trait Panel {
	fn name(&self) -> &'static str;

	fn show(&mut self, ui: &mut Ui, context: &mut EditorPanelContext<'_>);

	fn set_viewport_texture(&mut self, _texture: TextureId) {}

	fn take_viewport_output(&mut self) -> Option<ViewportOutput> { None }

	fn save_project_if_dirty(&mut self) -> Option<Result<Project, String>> { None }

	fn has_unsaved_project_changes(&self) -> bool { false }

	fn clear_background(&self) -> bool { true }

	fn scroll_bars(&self) -> [bool; 2] { [true, true] }
}
