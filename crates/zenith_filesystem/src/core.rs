//! State every backend shares, and the operations every backend offers.

use std::sync::{Mutex, PoisonError, RwLock};

use semver::Version;
use zenith_archive::BuildId;
use zenith_error::{ZResult, ZenithError};

#[cfg(feature = "fs")]
use crate::options::WarningKind;
use crate::{
	cache::{Cache, CacheSettings, Stamp},
	manifest::Manifest,
	options::{OpenOptions, ProjectWarning},
	scheme::Schemes,
	types::{DirEntry, Metadata, ReadSeek},
	zpath::ZPath,
};

/// The manifest names the engine versions the project is for. Until there are
/// migrations, a project (or a build) of another engine version does not open.
fn check_engine_version(manifest: &Manifest) -> ZResult<()> {
	let current = Version::parse(env!("CARGO_PKG_VERSION")).map_err(|e| ZenithError::Version(e.to_string()))?;
	if manifest.engine_version.matches(&current) {
		Ok(())
	} else {
		Err(ZenithError::EngineVersionMismatch(
			manifest.engine_version.to_string(),
			current.to_string(),
		))
	}
}

pub struct Core {
	pub manifest: Manifest,
	pub schemes: RwLock<Schemes>,
	pub cache: Cache,
	/// Largest file `read_bytes`/`read_string` load into memory.
	pub max_read_size: u64,
	warnings: Mutex<Vec<ProjectWarning>>,
}

impl Core {
	/// # Errors
	/// `EngineVersionMismatch` if the manifest is for another engine version
	/// (unless `opts` says to ignore that).
	pub fn new(
		manifest: Manifest,
		settings: CacheSettings,
		backend_tag: &'static str,
		opts: &OpenOptions,
	) -> ZResult<Self> {
		if !opts.ignore_engine_version {
			check_engine_version(&manifest)?;
		}
		Ok(Self {
			manifest,
			schemes: RwLock::new(Schemes::with_builtin()),
			cache: Cache::new(backend_tag, settings),
			max_read_size: opts.max_read_size,
			warnings: Mutex::new(Vec::new()),
		})
	}

	/// Project-relative path of a `ZPath`.
	pub fn resolve(&self, path: &ZPath) -> ZResult<String> {
		self.schemes
			.read()
			.unwrap_or_else(PoisonError::into_inner)
			.resolve(path)
	}

	/// Records a warning and logs it. The same warning is kept (and logged)
	/// once, however often the operation that causes it runs.
	#[cfg(feature = "fs")]
	pub fn warn(&self, kind: WarningKind, message: String) {
		let mut warnings = self.warnings.lock().unwrap_or_else(PoisonError::into_inner);
		if warnings.iter().any(|w| w.kind == kind && w.message == message) {
			return;
		}
		zenith_log::warn!("{message}");
		warnings.push(ProjectWarning { kind, message });
	}

	pub fn warnings(&self) -> Vec<ProjectWarning> {
		self.warnings.lock().unwrap_or_else(PoisonError::into_inner).clone()
	}
}

/// What the read side of every backend can do. Paths are project-relative,
/// `/`-separated, already resolved from a `ZPath`.
pub trait Backend: Send + Sync {
	fn core(&self) -> &Core;
	fn exists(&self, rel: &str) -> ZResult<bool>;
	fn list(&self, rel: &str) -> ZResult<Vec<DirEntry>>;
	fn metadata(&self, rel: &str) -> ZResult<Metadata>;
	fn is_symlink(&self, rel: &str) -> ZResult<bool>;
	/// Reads a file without touching the cache.
	fn read_raw(&self, rel: &str) -> ZResult<Vec<u8>>;
	fn open_read(&self, rel: &str) -> ZResult<Box<dyn ReadSeek + Send>>;
	/// Version stamp for cache validation; `None` for immutable builds.
	fn stamp(&self, rel: &str) -> Option<Stamp>;
	/// The identity of a build; `None` for a project folder.
	fn build_id(&self) -> Option<BuildId> {
		None
	}
}
