use super::{EditorPanelContext, Panel};

#[derive(Debug)]
pub struct ConsolePanel;

impl ConsolePanel {
	pub const fn new() -> Self { Self }
}

impl Panel for ConsolePanel {
	fn name(&self) -> &'static str { "Console" }

	fn show(&mut self, ui: &mut egui::Ui, _context: &mut EditorPanelContext<'_>) { ze_log::show_editor_console(ui); }
}
