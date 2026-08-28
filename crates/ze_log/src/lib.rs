use std::path::Path;

pub use tracing::{
	debug, debug_span, error, error_span, info, info_span, instrument, trace, trace_span, warn, warn_span,
};
// Re-exported so callers configure rotation without adding
// tracing-appender as a direct dependency themselves.
pub use tracing_appender::rolling::Rotation;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use ze_error::{ZPubResult, ZResult, into_pub_result};

/// Must stay alive for the process lifetime -- dropping it flushes and
/// stops the non-blocking file writer thread.
pub struct LogGuard {
	_file_guard: tracing_appender::non_blocking::WorkerGuard,
}

/// Public API boundary: matchable ZeroError, per the internal/public split.
pub fn init(logs_dir: impl AsRef<Path>, rotation: Rotation, max_files: usize) -> ZPubResult<LogGuard> {
	into_pub_result(init_inner(logs_dir, rotation, max_files))
}

fn init_inner(logs_dir: impl AsRef<Path>, rotation: Rotation, max_files: usize) -> ZResult<LogGuard> {
	std::fs::create_dir_all(&logs_dir)?;

	let file_appender = tracing_appender::rolling::Builder::new()
		.rotation(rotation)
		.filename_prefix("zeroengine")
		.filename_suffix("log")
		.max_log_files(max_files)
		.build(logs_dir)?;

	let (non_blocking, file_guard) = tracing_appender::non_blocking(file_appender);

	let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

	let registry = tracing_subscriber::registry()
		.with(filter)
		.with(tracing_subscriber::fmt::layer())
		.with(
			tracing_subscriber::fmt::layer()
				.with_writer(non_blocking)
				.with_ansi(false),
		)
		.with(tracing_error::ErrorLayer::default());

	#[cfg(feature = "profiling")]
	let registry = registry.with(tracing_tracy::TracyLayer::default());

	registry.init();

	Ok(LogGuard {
		_file_guard: file_guard,
	})
}
