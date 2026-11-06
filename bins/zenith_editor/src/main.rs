mod app;
mod backup;
mod editor;
mod gfx;
mod panels;
mod project;
mod recent;
mod theme;
mod toolbar;

use std::sync::atomic::Ordering;

use zenith_error::{IntoZResult, ZInternalResult, eyre};

fn main() -> ZInternalResult<()> {
	zenith_error::install().into_zresult()?;
	let log_dir = dirs::data_local_dir()
		.ok_or_else(|| eyre!("could not determine local data directory"))?
		.join("ZenithEngine")
		.join("Logs");

	let log_guard = zenith_log::init(log_dir, zenith_log::Rotation::MINUTELY, 10)?;

	// Ctrl+C (and SIGTERM, closing the console) end the editor through its
	// loop, so that the changes are put in the backup and everything is closed
	// in order.
	if let Err(e) = ctrlc::set_handler(|| app::QUIT.store(true, Ordering::SeqCst)) {
		zenith_log::warn!("Ctrl+C is not handled: {e}");
	}

	app::run(log_guard)
}
