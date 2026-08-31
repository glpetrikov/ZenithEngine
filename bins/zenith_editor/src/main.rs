mod theme;
mod toolbar;

use dear_app::{
	AppConfig, Application, DockingConfig, FrameContext, InitContext, RunError,
	imgui::{Condition, DockNodeFlags, FontSource, WindowFlags},
	run,
	wgpu::PresentMode,
};
use theme::zenith_theme;

// TODO: write manual window creation, wgpu rendering, etc. instead of using the
// dear-app

struct EditorApp {
	clicks: u32,
	color_rgba: [f32; 4],
}

impl Application for EditorApp {
	fn configure_imgui(&mut self, context: &mut InitContext<'_>) -> Result<(), RunError> {
		let ctx = context.imgui();
		zenith_theme().apply_to_context(ctx);

		let font_data = include_bytes!("../assets/fonts/Inter-Regular.ttf");
		unsafe {
			ctx.font_atlas()
				.add_font(&[FontSource::ttf_data_with_size(font_data, 18.0)]);
		}

		Ok(())
	}
	fn frame(&mut self, context: &mut FrameContext<'_>) -> Result<(), RunError> {
		let ui = context.ui();
		let viewport = ui.main_viewport();

		toolbar::draw_toolbar(ui);

		let menu_bar_height = ui.frame_height();
		let workspace_pos = [viewport.pos()[0], viewport.pos()[1] + menu_bar_height];
		let workspace_size = [viewport.size()[0], viewport.size()[1] - menu_bar_height];

		ui.window("ZenithDockspace")
			.position(workspace_pos, Condition::Always)
			.size(workspace_size, Condition::Always)
			.flags(
				WindowFlags::NO_TITLE_BAR
					| WindowFlags::NO_COLLAPSE
					| WindowFlags::NO_RESIZE
					| WindowFlags::NO_MOVE
					| WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS
					| WindowFlags::NO_NAV_FOCUS,
			)
			.build(|| {
				let content_size = ui.content_region_avail();
				let _ = ui
					.dockspace()
					.current_window(content_size)
					.flags(DockNodeFlags::empty())
					.build();
			});

		ui.window("Window")
			.size([200.0, 100.0], Condition::FirstUseEver)
			.position([30.0, 30.0], Condition::FirstUseEver)
			.build(|| {
				if ui.button("Click me") {
					self.clicks += 1;
				}
				ui.same_line();
				ui.text_colored([0.5, 0.5, 0.5, 0.5], format!("Clicks: {}", self.clicks));
			});

		ui.window("Window 2")
			.size([350.0, 100.0], Condition::FirstUseEver)
			.position([300.0, 300.0], Condition::FirstUseEver)
			.build(|| {
				ui.color_edit4("Color", &mut self.color_rgba);
			});

		Ok(())
	}
}

fn main() -> Result<(), RunError> {
	let app = EditorApp {
		clicks: 0,
		color_rgba: [1.0, 1.0, 1.0, 1.0],
	};

	run(
		AppConfig {
			window_title: "Zenith Editor".to_string(),
			window_size: (1280.0, 720.0),
			present_mode: PresentMode::Fifo,
			clear_color: [0.1, 0.1, 0.1, 1.0],
			docking: DockingConfig::ApplicationManaged {
				dockspace_flags: DockNodeFlags::empty(),
			},
			..Default::default()
		},
		app,
	)
}
