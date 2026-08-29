mod toolbar;

use dear_app::{
	AppConfig, DockingConfig, RunError, Theme,
	imgui::{Condition, DockNodeFlags, WindowFlags},
	run_ui,
	wgpu::PresentMode,
};

struct State {
	clicks: u32,
	color_rgba: [f32; 4],
}

fn main() -> Result<(), RunError> {
	let mut state = State {
		clicks: 0,
		color_rgba: [1.0, 1.0, 1.0, 1.0],
	};

	run_ui(
		AppConfig {
			window_title: "Zenith Editor".to_string(),
			window_size: (1280.0, 720.0),
			present_mode: PresentMode::Fifo,
			clear_color: [0.1, 0.1, 0.1, 1.0],
			theme: Some(Theme::Dark),
			docking: DockingConfig::FullViewport {
				dockspace_flags: DockNodeFlags::empty(),
				host_window_flags: WindowFlags::NO_TITLE_BAR
					| WindowFlags::NO_COLLAPSE
					| WindowFlags::NO_RESIZE
					| WindowFlags::NO_MOVE
					| WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS
					| WindowFlags::NO_NAV_FOCUS,
				host_window_name: "ZenithDockspace".to_string(),
			},
			..Default::default()
		},
		move |ui| {
			toolbar::draw_titlebar(ui);

			ui.window("Window")
				.size([200.0, 100.0], Condition::FirstUseEver)
				.position([30.0, 30.0], Condition::FirstUseEver)
				.build(|| {
					if ui.button("Click me") {
						state.clicks += 1;
					}
					ui.same_line();
					ui.text_colored([0.5, 0.5, 0.5, 0.5], format!("Clicks: {}", state.clicks));
				});
			ui.window("Window 2")
				.size([350.0, 100.0], Condition::FirstUseEver)
				.position([300.0, 300.0], Condition::FirstUseEver)
				.build(|| {
					ui.color_edit4("My RGBA Input", &mut state.color_rgba);
				});
		},
	)?;
	Ok(())
}
