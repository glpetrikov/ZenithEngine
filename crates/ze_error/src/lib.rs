// Re-exported so binaries (editor, runtime, cli) call ze_error::install()
// once at startup instead of adding color-eyre as a direct dependency
// themselves. Library crates should never call this -- it's a global
// panic/eyre hook, valid once per process.
pub use color_eyre::install;
pub use eyre::{Report, WrapErr, bail, ensure, eyre};
pub use thiserror::Error;

/// `ZeroEngine` Errors
#[derive(Debug, Error)]
pub enum ZeroError {
	#[error("I/O error: {0}")]
	Io(#[from] std::io::Error),
	#[error("{0}")]
	Other(String),
}

// Public API boundary: documented, matchable variants.
pub type ZPubResult<T> = std::result::Result<T, ZeroError>;

// Internal use: free-form propagation with .wrap_err() context.
pub type ZResult<T> = eyre::Result<T>;

pub trait IntoPubResult<T> {
	fn into_pub_result(self) -> ZPubResult<T>;
}

impl<T> IntoPubResult<T> for ZResult<T> {
	fn into_pub_result(self) -> ZPubResult<T> {
		self.map_err(|report| {
			report
				.downcast::<ZeroError>()
				.unwrap_or_else(|report| ZeroError::Other(report.to_string()))
		})
	}
}
