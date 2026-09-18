use tracing_subscriber::{fmt::format::Pretty, layer::SubscriberExt, util::SubscriberInitExt};
use tracing_web::{MakeWebConsoleWriter, performance_layer};
use zenith_error::{IntoZResult, WrapErr, ZInternalResult, ZResult};

/// Initializes the logger for wasm targets.
///
/// # Errors
/// Currently infallible, but returns `ZResult` to match `init`'s signature.
pub fn init_web() -> ZResult<()> { init_web_inner().wrap_err("Cannot initilize logger").into_zresult() }

fn init_web_inner() -> ZInternalResult<()> {
	let fmt_layer = tracing_subscriber::fmt::layer()
		.with_ansi(false)
		.without_time()
		.with_writer(MakeWebConsoleWriter::new());
	let perf_layer = performance_layer().with_details_from_fields(Pretty::default());

	tracing_subscriber::registry()
		.with(fmt_layer)
		.with(perf_layer)
		.with(tracing_error::ErrorLayer::default())
		.init();

	Ok(())
}
