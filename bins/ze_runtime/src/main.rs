use ze_log::*;

#[instrument]
fn main() -> ze_error::ZPubResult<()> {
	ze_error::into_pub_result(ze_error::install())?;
	let _log_guard = ze_log::init("Logs", ze_log::Rotation::MINUTELY, 10)?;

	info!("runtime starting up");
	warn!(reason = "test warning", "something to look at");

	// Force an error through the eyre path to see wrap_err/downcast working.
	std::thread::spawn(|| {
		if let Err(report) = try_something() {
			error!(error = %report, "something failed");
		}
	});

	info!("runtime shutting down");
	panic!()
}
#[instrument]
fn try_something() -> ze_error::ZResult<()> {
	use ze_error::WrapErr;
	std::fs::read_to_string("this_file_does_not_exist.txt").wrap_err("failed to read config file")?;
	Ok(())
}
