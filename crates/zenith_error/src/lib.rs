use std::path::PathBuf;

// Re-exported so binaries (editor, runtime, cli) call zenith_error::install()
// once at startup instead of adding color-eyre as a direct dependency
// themselves. Library crates should never call this -- it's a global
// panic/eyre hook, valid once per process.
pub use color_eyre::install;
pub use eyre::{Report, WrapErr, bail, ensure, eyre};
pub use thiserror::Error;

/// `ZenithEngine` Errors
#[derive(Debug, Error)]
pub enum ZenithError {
	// === Project Errors ===
	#[error("Unsupported project version")]
	UnsupportedProjectVersion,
	#[error("Invalid project: {0}")]
	InvalidProject(String),
	#[error("Invalid project path: {0}")]
	InvalidProjectPath(PathBuf),
	#[error("Bad project name: {0}")]
	BadProjectName(String),
	#[error("Path escapes root: {0}")]
	PathEscapesRoot(PathBuf),

	// === World Errors ===
	#[error("Invalid world path: {0}")]
	InvalidWorldPath(PathBuf),
	#[error("Invalid world: {0}")]
	InvalidWorld(String),

	// === ECS Errors ===
	#[error("failed to load component while restoring snapshot: {0}")]
	ComponentLoadFailed(String),
	#[error("unknown component type: {0}")]
	UnknownComponentType(String),

	// === Errors From Other Crates Or Std ===
	#[error("I/O error: {0}")]
	Io(#[from] std::io::Error),

	#[error("Semver error: {0}")]
	Semver(#[from] semver::Error),

	#[error("Toml deserialization error: {0}")]
	TomlDe(#[from] toml::de::Error),
	#[error("Toml serialization error: {0}")]
	TomlSer(#[from] toml::ser::Error),

	#[error("JSON error: {0}")]
	Json(#[from] serde_json::Error),

	#[error("YAML error: {0}")]
	YAML(#[from] serde_saphyr::Error),
	#[error("YAML Serialization error: {0}")]
	YAMLSerialization(#[from] serde_saphyr::SerializeError),

	#[error("{0}")]
	Other(String),
}

// Public API boundary: documented, matchable variants.
pub type ZPubResult<T> = std::result::Result<T, ZenithError>;

// Internal use: free-form propagation with .wrap_err() context.
pub type ZResult<T> = eyre::Result<T>;

pub trait IntoPubResult<T> {
	fn into_pub_result(self) -> ZPubResult<T>;
}

impl<T> IntoPubResult<T> for ZResult<T> {
	fn into_pub_result(self) -> ZPubResult<T> {
		self.map_err(|report| {
			report
				.downcast::<ZenithError>()
				.unwrap_or_else(|report| ZenithError::Other(report.to_string()))
		})
	}
}
