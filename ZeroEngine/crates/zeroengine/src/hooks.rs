use ze_app::CustomEvents;

pub fn ctrlc_hook(event_loop_proxy: &winit::event_loop::EventLoopProxy<CustomEvents>) {
	let ctrlc_proxy = event_loop_proxy.clone();

	match ctrlc::set_handler(move || {
		println!("\nReceived Ctrl+C, shutting down...");

		let _ = ctrlc_proxy.send_event(CustomEvents::Shutdown);
	}) {
		Ok(()) => {}
		Err(e) => {
			ze_log::error!("Cannot Setup ctrlc hook, error: {e:?}");
		}
	}
}

pub fn panic_hook() {
	const RED_BOLD: &str = "\x1b[1;31m";
	const RESET: &str = "\x1b[0m";

	std::panic::set_hook(Box::new(|panic_info| {
		let location = panic_info.location().map_or_else(
			|| "unknown".into(),
			|l| format!("{}:{}:{}", l.file(), l.line(), l.column()),
		);

		let message = panic_info
			.payload()
			.downcast_ref::<&str>()
			.copied()
			.or_else(|| panic_info.payload().downcast_ref::<String>().map(String::as_str))
			.unwrap_or("unknown panic");

		println!("{RED_BOLD}ZEROENGINE FATAL ERROR{RESET} at {location}: {message}");
	}));
}
