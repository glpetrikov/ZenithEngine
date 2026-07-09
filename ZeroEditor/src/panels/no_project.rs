use egui::Ui;

use super::{EditorPanelContext, Panel};

#[derive(Debug, Default)]
pub struct NoProjectPanel;

impl NoProjectPanel {
	pub const fn new() -> Self { Self }
}

impl Panel for NoProjectPanel {
	fn name(&self) -> &'static str { "File Explorer" }

	fn show(&mut self, ui: &mut Ui, _context: &mut EditorPanelContext<'_>) {
		ui.vertical_centered(|ui| {
			ui.add_space(12.0);
			ui.heading("No project loaded");
			ui.label("Use File -> Open Project to load a ZEProject.toml.");
		});
	}
}
