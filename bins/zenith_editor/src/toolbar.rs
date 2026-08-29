use dear_app::imgui::Ui;

pub fn draw_titlebar(ui: &Ui) {
	ui.main_menu_bar(|| {
		ui.text("ZenithEngine");
		ui.separator_vertical();
		ui.menu("File", || {
			ui.menu_item("New");
			ui.menu_item("Open");

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
	});
}
