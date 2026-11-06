use dear_imgui_rs::Ui;

use crate::editor::Editor;

pub fn draw_toolbar(editor: &mut Editor, ui: &Ui) {
	ui.main_menu_bar(|| {
		ui.text("ZenithEngine");
		ui.separator_vertical();
		let has_project = editor.project.is_some();
		ui.menu("File", || {
			ui.menu_item("New Project");
			if ui.menu_item("Open Project") {
				rfd::FileDialog::new()
					.set_title("Open Project")
					.pick_folder()
					.map(|path| editor.open_project(path.as_path()));
			}
			ui.separator();
			if ui.menu_item_enabled_selected("New World...", None::<&str>, false, has_project) {
				editor.new_world = crate::editor::NewWorldForm::default();
				editor.requests.new_world = true;
			}
			if ui.menu_item_enabled_selected("Save All", Some("Ctrl+S"), false, has_project) {
				editor.save_all();
			}
			ui.separator();
			if ui.menu_item("Quit") {
				editor.request_exit();
			}
		});
		ui.menu("Edit", || {
			ui.menu_item("Undo");
			ui.menu_item("Redo");
		});
		ui.menu("Window", || {});
		ui.menu("Debug", || {
			if ui.menu_item_enabled_selected("Mark all worlds changed", None::<&str>, false, has_project)
				&& let Some(project) = &mut editor.project
			{
				project.mark_all_dirty();
			}
		});

		ui.menu("Help", || {
			ui.text_link_open_url(
				"Report an Issue...",
				"https://github.com/glpetrikov/ZenithEngine/issues/new",
			);
		});

		if let Some(status) = editor.status() {
			ui.separator_vertical();
			ui.text_disabled(status);
		}
	});
}
