use dear_app::imgui::Ui;

pub fn draw_toolbar(ui: &Ui) {
	ui.main_menu_bar(|| {
		ui.text("ZenithEngine");
		ui.separator_vertical();
		ui.menu("File", || {
			ui.menu_item("New Project");
			ui.menu_item("Open Project");
			ui.separator();
			if ui.menu_item("Quit") {
				// TODO: add Event Manager
			}
		});
		ui.menu("Edit", || {
			ui.menu_item("Undo");
			ui.menu_item("Redo");
		});
		ui.menu("Window", || {});

		ui.menu("Help", || {
			ui.text_link_open_url(
				"Report an Issue...",
				"https://github.com/glpetrikov/ZenithEngine/issues/new",
			);
		});
	});
}
