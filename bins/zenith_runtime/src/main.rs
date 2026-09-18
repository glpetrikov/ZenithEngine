use zenith_error::ZInternalResult;
use zenith_log::LogGuard;
use zenith_runtime::run;

fn main() -> ZInternalResult<()> {
	zenith_error::install()?;

	#[allow(unused)]
	let log_guard: LogGuard;
	#[cfg(not(target_arch = "wasm32"))]
	{
		let log_dir = dirs::data_local_dir()
			.unwrap_or_else(|| {
				eprintln!("could not determine local data directory");
				std::process::exit(1)
			})
			.join("ZenithEngine")
			.join("Logs");

		#[allow(unused)]
		{
			log_guard = zenith_log::init(log_dir, zenith_log::Rotation::MINUTELY, 10)?;
		}
	}

	run()
}
