use dear_imgui_rs::Ui;
use zenith_renderer::assets::{AssetKind, ops::MODEL_SOURCES};

use crate::editor::{Editor, PICTURE_EXTENSIONS};

/// File → Import: the files, then the folder of `Assets/` they go into.
fn import(editor: &mut Editor) {
	let Some(assets) = editor.project.as_ref().map(|p| p.path.join("Assets")) else {
		return;
	};
	let Some(files) = rfd::FileDialog::new()
		.set_title("Import")
		.add_filter(
			"Models and pictures",
			&[&MODEL_SOURCES[..], &PICTURE_EXTENSIONS[..]].concat(),
		)
		.add_filter("Models", &MODEL_SOURCES)
		.add_filter("Pictures", &PICTURE_EXTENSIONS)
		.pick_files()
	else {
		return;
	};
	if let Some(folder) = rfd::FileDialog::new()
		.set_title("Import into")
		.set_directory(&assets)
		.pick_folder()
	{
		editor.import_assets(&files, &folder);
	}
}

/// File → Export: an asset of the project, then where its source goes.
fn export(editor: &mut Editor) {
	let Some(assets) = editor.project.as_ref().map(|p| p.path.join("Assets")) else {
		return;
	};
	let Some(asset) = rfd::FileDialog::new()
		.set_title("Export")
		.set_directory(&assets)
		.add_filter("Assets", &AssetKind::ALL.map(AssetKind::extension))
		.pick_file()
	else {
		return;
	};
	let mut save = rfd::FileDialog::new().set_title("Export to");
	if let Some(name) = editor.export_file_name(&asset) {
		save = save.set_file_name(name);
	}
	if let Some(to) = save.save_file() {
		editor.export_asset(&asset, &to);
	}
}

/// File → Replace Source: an asset of the project, then the file that takes the
/// place of what it was made from.
fn replace_source(editor: &mut Editor) {
	let Some(assets) = editor.project.as_ref().map(|p| p.path.join("Assets")) else {
		return;
	};
	let Some(asset) = rfd::FileDialog::new()
		.set_title("Replace the source of")
		.set_directory(&assets)
		.add_filter(
			"Models and pictures",
			&[AssetKind::Model.extension(), AssetKind::Texture.extension()],
		)
		.pick_file()
	else {
		return;
	};
	let Some(source) = rfd::FileDialog::new()
		.set_title("Take the new source from")
		.add_filter(
			"Models and pictures",
			&[&MODEL_SOURCES[..], &PICTURE_EXTENSIONS[..]].concat(),
		)
		.pick_file()
	else {
		return;
	};
	editor.replace_source(&asset, &source);
}

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
			if ui.menu_item_enabled_selected("Import...", None::<&str>, false, has_project) {
				import(editor);
			}
			if ui.menu_item_enabled_selected("Export...", None::<&str>, false, has_project) {
				export(editor);
			}
			if ui.menu_item_enabled_selected("Replace Source...", None::<&str>, false, has_project) {
				replace_source(editor);
			}
			if ui.menu_item_enabled_selected("Import Loose Models...", None::<&str>, false, has_project) {
				editor.check_loose_models(true);
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
