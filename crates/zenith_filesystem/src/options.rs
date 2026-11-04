//! How a project is opened.

use std::{path::Path, sync::Arc};

use zenith_archive::{BuildId, EncryptionKey};
use zenith_error::ZResult;

use crate::{cache::CacheConfig, project::Project, userdata::UserStore};

/// Settings for [`Project::open`]-style calls. Build with the chain methods:
///
/// ```no_run
/// use zenith_filesystem::{CacheConfig, OpenOptions};
///
/// # fn main() -> Result<(), zenith_error::ZenithError> {
/// let options = OpenOptions::new().cache_config(CacheConfig::File);
/// let project = options.open("Game")?;
/// # Ok(()) }
/// ```
#[derive(Clone, Debug)]
pub struct OpenOptions {
	pub(crate) cache: CacheConfig,
	pub(crate) key: Option<EncryptionKey>,
	pub(crate) max_read_size: u64,
	pub(crate) ignore_engine_version: bool,
	pub(crate) expected_build_id: Option<BuildId>,
	pub(crate) user_data: Option<Arc<dyn UserStore>>,
}

/// Default for [`OpenOptions::max_read_size`].
pub const DEFAULT_MAX_READ_SIZE: u64 = 1024 * 1024 * 1024;

impl Default for OpenOptions {
	fn default() -> Self {
		Self {
			cache: CacheConfig::default(),
			key: None,
			max_read_size: DEFAULT_MAX_READ_SIZE,
			ignore_engine_version: false,
			expected_build_id: None,
			user_data: None,
		}
	}
}

impl OpenOptions {
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// The largest file `read_bytes` and `read_string` load into memory
	/// (default 1 GiB; use `open_read` to stream bigger ones). A build states
	/// the size of every file in its index, and a hostile index can state a
	/// size that no memory holds; this is checked before anything is read.
	#[must_use]
	pub const fn max_read_size(mut self, bytes: u64) -> Self {
		self.max_read_size = bytes;
		self
	}

	/// The build this must be, when opening a build (a single file or a
	/// folder). Any other build fails with `BuildIdMismatch`, a complete older
	/// release of the game included, which the files of a build cannot tell
	/// from the current one themselves. Keep the id of a release where the
	/// player cannot change it (in the program, or in a signed manifest);
	/// `BuildReport::build_id` and [`Project::build_id`] give it. A project
	/// folder has no id and ignores this.
	#[must_use]
	pub const fn expected_build_id(mut self, id: BuildId) -> Self {
		self.expected_build_id = Some(id);
		self
	}

	/// Opens a project or build even if its manifest is for another engine
	/// version (the default is `EngineVersionMismatch`). For tools that have to
	/// read an old project in order to convert it.
	#[must_use]
	pub const fn ignore_engine_version(mut self) -> Self {
		self.ignore_engine_version = true;
		self
	}

	/// Where the data of the player (`Data://`) is kept. By default it is the
	/// user's data folder on a computer (`<data folder>/<project name>`) and
	/// memory, which is gone with the process, where there is no folder (a
	/// browser: give it an `IdbStore`). See [`UserStore`].
	#[must_use]
	pub fn user_data(mut self, store: Arc<dyn UserStore>) -> Self {
		self.user_data = Some(store);
		self
	}

	/// Where cache settings come from (default: `CacheConfig::File`).
	#[must_use]
	pub const fn cache_config(mut self, config: CacheConfig) -> Self {
		self.cache = config;
		self
	}

	/// Key for encrypted builds. A build that is not encrypted ignores it;
	/// an encrypted one fails to open without it, or with a wrong one.
	#[must_use]
	pub fn encryption_key(mut self, key: EncryptionKey) -> Self {
		self.key = Some(key);
		self
	}

	/// Opens the project at `path`: its kind (project folder, single-file
	/// build or folder build) is detected from what is there.
	///
	/// # Errors
	/// `UnknownProjectFormat` if `path` is none of the three, and errors for a
	/// damaged project or build, an unsupported version, or a missing or
	/// wrong key.
	pub fn open(self, path: impl AsRef<Path>) -> ZResult<Project> {
		Project::open_with_options(path.as_ref(), &self)
	}
}

/// Something worth knowing that did not stop an operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectWarning {
	pub kind: WarningKind,
	pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarningKind {
	/// The Zenith block in `.gitignore` is damaged, so it was left untouched.
	DamagedGitignore,
	/// `.gitignore` could not be read or written.
	GitignoreIo,
}
