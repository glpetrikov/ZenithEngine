mod banner;
mod hooks;

use winit::event_loop::{ControlFlow, EventLoop};
use ze_app::{App, CustomEvents};
use ze_project::Project;

pub fn run() {
	ze_core::ensure_large_main_thread_stack();

	ze_log::debug!("[STARTUP 1] Entered zeroengine::run()");

	ze_log::init(); // TODO: add file logging
	ze_log::debug!("[STARTUP 2] ze_log initialized");

	ze_log::trace!("Setting up panic hook");
	hooks::panic_hook();
	ze_log::debug!("[STARTUP 3] Panic hook installed");

	// ===================================================
	// Creating Event Loop
	// ===================================================
	ze_log::debug!("[STARTUP 4] Creating event loop...");
	let event_loop = EventLoop::<CustomEvents>::with_user_event()
		.build()
		.expect("Cannot create Event Loop");
	let event_loop_proxy = event_loop.create_proxy();
	ze_log::debug!("[STARTUP 5] Event loop created");

	ze_log::trace!("Event Loop setting control flow");
	event_loop.set_control_flow(ControlFlow::Poll);

	banner::render_banner();

	// ===================================================
	// Setup Ctrl+C hook
	// ===================================================
	hooks::ctrlc_hook(&event_loop_proxy);

	// ===================================================
	// Running App
	// ===================================================
	ze_log::debug!("[STARTUP 6] Creating App...");
	let active_project = std::env::args().nth(1).map_or_else(
		|| None,
		|project_path| match Project::load(&project_path) {
			Ok(project) => Some(project),
			Err(error) => {
				ze_log::error!("Failed to load project `{project_path}`: {error:?}");
				std::process::exit(1);
			}
		},
	);

	let mut app = match App::new(active_project) {
		Ok(app) => app,
		Err(error) => {
			ze_log::error!("Failed to create app: {error:?}");
			std::process::exit(1);
		}
	};
	ze_log::debug!("[STARTUP 7] App created successfully");

	ze_log::debug!("[STARTUP 8] Entering event loop");
	event_loop.run_app(&mut app).expect("Cannot run Application");
}
