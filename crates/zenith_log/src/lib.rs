#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use native::{LogGuard, init};
pub use tracing::{
	debug, debug_span, error, error_span, info, info_span, instrument, trace, trace_span, warn, warn_span,
};
#[cfg(not(target_arch = "wasm32"))]
pub use tracing_appender::rolling::Rotation;
#[cfg(target_arch = "wasm32")]
pub use web::init_web;
