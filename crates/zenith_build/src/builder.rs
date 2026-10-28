use std::{num::NonZeroUsize, path::PathBuf};

use zenith_archive::{BuildId, EncryptionKey, WriterOptions};
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{BackendKind, Project, ZPath, layout};

use crate::{
	archive_build,
	engine_exe::{self, EngineExe, RenameExe},
	folder_build, output, plan,
	web_runtime::WebRuntime,
};

/// Memory the compression threads of one build may use together, by the
/// estimate of `WriterOptions::thread_memory`.
const MEMORY_BUDGET: u64 = 2 * 1024 * 1024 * 1024;

/// What `build` produces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BuildFormat {
	/// One file (web, simple distribution). `output` is the file; by default
	/// `Build/game.zenithbuild` in the project.
	#[default]
	Archive,
	/// A folder with an index and one file per project file (Steam: an update
	/// only changes the files that changed). `output` is the folder; by default
	/// `Build/game/` in the project.
	Fs,
}

/// What a build did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildReport {
	/// Where the build was written.
	pub output: PathBuf,
	/// Files in the build.
	pub entries: usize,
	/// Files that were compressed. An archive compresses everything every time.
	pub compressed: usize,
	/// Files a folder build took over from the previous build in the same
	/// folder without reading them again: same content hash, same compression
	/// parameters, data file still there and intact.
	pub reused: usize,
	/// The identity of the build that was written: it changes with anything in
	/// the build, and the same project, settings and key give the same one.
	/// Keep it where the player cannot change it and open the build with
	/// `OpenOptions::expected_build_id`, and an older or another build is
	/// refused even though all its files are valid.
	pub build_id: BuildId,
	/// Things that went wrong after the build was complete: a stale data file
	/// of the previous build that could not be deleted. The build itself is
	/// valid.
	pub warnings: Vec<String>,
	/// The copy of the engine executable, if `engine_exe` was set.
	pub engine_exe: Option<PathBuf>,
	/// The files of the browser runtime that were copied (`web_runtime`).
	pub web_files: Vec<PathBuf>,
}

/// One step of the entry list. Steps apply in the order they were added; for
/// each path the last step that applies to it wins.
#[derive(Clone, Debug)]
pub enum Rule {
	Exclude(String),
	Include(String),
	AddFile { dest: ZPath, src: ZPath },
	AddBytes { dest: ZPath, bytes: Vec<u8> },
}

/// Builds a project into a distributable form.
///
/// The build is a virtual list of entries: the project on disk is neither
/// copied nor changed (apart from adding the output folder to `.gitignore`).
/// The base list is the manifest plus `Assets/`, `Settings/` and `Packages/`;
/// `Temp/`, `Trash/`, `Build/`, `.gitignore` and `.v1` never go in unless a
/// rule asks for them. Entries are sorted by path and no modification times are
/// stored, so the same project, settings and key give byte-identical output.
///
/// ```no_run
/// use zenith_build::{BuildFormat, Builder, ZPath};
///
/// # fn main() -> Result<(), zenith_error::ZenithError> {
/// // Scripts are compiled before the build (into Temp/Build/scripts.wasm). The
/// // builder knows nothing about the compiler or the format of the settings:
/// // the caller swaps the sources for the compiled file and prepares the
/// // changed settings itself.
/// let patched_settings = b"changed = true".to_vec();
/// let scripts = ZPath::parse("Assets://Scripts/scripts.wasm")?;
/// let compiled = ZPath::parse("Temp://Build/scripts.wasm")?;
/// let settings = ZPath::parse("Settings://Scripts.toml")?;
///
/// let builder = Builder::new().path("Game").output("dist/Game.zenithbuild");
/// let builder = builder.format(BuildFormat::Archive).zstd_compression(19);
/// let builder = builder.exclude("Assets/Scripts/**");
/// let builder = builder.add_file(scripts, compiled);
/// let builder = builder.add_bytes(settings, patched_settings);
/// builder.build()?;
/// # Ok(()) }
/// ```
#[derive(Clone, Debug)]
pub struct Builder {
	path: Option<PathBuf>,
	output: Option<PathBuf>,
	format: BuildFormat,
	compress: bool,
	level: i32,
	key: Option<EncryptionKey>,
	rules: Vec<Rule>,
	threads: usize,
	hash_cache: bool,
	engine_exe: Option<EngineExe>,
	web: Option<WebRuntime>,
}

impl Default for Builder {
	fn default() -> Self {
		Self::new()
	}
}

impl Builder {
	#[must_use]
	pub const fn new() -> Self {
		Self {
			path: None,
			output: None,
			format: BuildFormat::Archive,
			compress: true,
			level: 9,
			key: None,
			rules: Vec::new(),
			threads: 0,
			hash_cache: true,
			engine_exe: None,
			web: None,
		}
	}

	/// The project folder. Used as given: `~` is not expanded.
	#[must_use]
	pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
		self.path = Some(path.into());
		self
	}

	/// Where to write the build: a plain filesystem path (not a `ZPath`), a
	/// file for `Archive` and a folder for `Fs`. It may not lie inside
	/// `Assets/`, `Settings/` or `Packages/`, may not be a file the build
	/// reads, and may not contain the project folder.
	#[must_use]
	pub fn output(mut self, output: impl Into<PathBuf>) -> Self {
		self.output = Some(output.into());
		self
	}

	#[must_use]
	pub const fn format(mut self, format: BuildFormat) -> Self {
		self.format = format;
		self
	}

	/// Whether files are compressed (default `true`). Without it they are
	/// stored as they are: the build is bigger, but a file can be read in a hex
	/// dump or a text editor (unless the build is encrypted), which helps in
	/// tests. `zstd_compression` is then ignored. Switching it re-writes every
	/// file of a folder build once.
	#[must_use]
	pub const fn compress(mut self, compress: bool) -> Self {
		self.compress = compress;
		self
	}

	/// zstd level, `1..=22` (default 9). Anything else is reported by `build`.
	/// The level changes build time and size, not how fast the game reads it.
	#[must_use]
	pub const fn zstd_compression(mut self, level: i32) -> Self {
		self.level = level;
		self
	}

	/// How many threads compress at the same time. `0` (the default) uses one
	/// per processor, fewer if the memory zstd needs at this level and chunk
	/// size would not fit in two gigabytes. The output does not depend on it.
	#[must_use]
	pub const fn threads(mut self, threads: usize) -> Self {
		self.threads = threads;
		self
	}

	/// Whether a folder build may take the hash of a file it has seen before
	/// from `Temp/Build/hash-cache.txt` when its size and modification time
	/// have not changed (default `true`), instead of reading it. Turn it off
	/// for builds from tools that restore modification times. Archive builds
	/// do not hash.
	#[must_use]
	pub const fn hash_cache(mut self, on: bool) -> Self {
		self.hash_cache = on;
		self
	}

	/// Encrypts the build with AES-256-GCM-SIV. Without a key nothing is
	/// encrypted.
	///
	/// * **`Fs` builds: keep the same key for every update of a game.** The
	///   names of the data files and the nonces of every chunk are derived from
	///   the key, so a new key renames and re-encrypts every file: an update
	///   would have to rebuild and re-download everything instead of the
	///   changed files. A build into a folder that holds a build made with
	///   another key is refused.
	/// * **Web builds: better not to encrypt.** Decryption in the browser is
	///   noticeably slow (a WebAssembly measurement gave about 45 MB/s on
	///   incompressible data and 77 to 162 MB/s on text and binaries), and the
	///   key must be available to the client to decrypt anything, so it
	///   protects the files from nobody who matters.
	///
	/// The key is 32 random bytes (`EncryptionKey::generate`), never derived
	/// from a password: the header carries a value that lets anyone test key
	/// guesses offline at full speed.
	#[must_use]
	pub fn encryption_key(mut self, key: EncryptionKey) -> Self {
		self.key = Some(key);
		self
	}

	/// Copies the engine's executable next to the build, into the folder that
	/// holds `output`, so that the build can be run or shipped as it is. The
	/// path is used as given (`~` is not expanded); a missing file is reported
	/// by `build` before anything is written. An existing copy is replaced.
	#[must_use]
	pub fn engine_exe(mut self, source: impl Into<PathBuf>) -> Self {
		self.engine_exe = Some(EngineExe {
			source: source.into(),
			rename: None,
		});
		self
	}

	/// Gives the copy of the engine executable another name (see
	/// [`RenameExe`]); without it the file keeps its name. Does nothing unless
	/// `engine_exe` is set.
	#[must_use]
	pub fn rename_exe(mut self, rename: RenameExe) -> Self {
		if let Some(exe) = &mut self.engine_exe {
			exe.rename = Some(rename);
		}
		self
	}

	/// Copies the browser runtime next to the build, into the folder that holds
	/// `output`: `pkg/` with the `.js` and `.wasm` from `pkg_dir` (the output
	/// of `wasm-pack build --target web`) and an `index.html` that starts it
	/// on a `<canvas id="zenith-canvas">`. Use [`Self::web_index`] for your
	/// own page, which can load `./pkg/<name>.js`. The files are copied as
	/// they are: never compressed or encrypted. A web game wants the `Archive`
	/// format.
	#[must_use]
	pub fn web_runtime(mut self, pkg_dir: impl Into<PathBuf>) -> Self {
		let index = self.web.take().and_then(|web| web.index);
		self.web = Some(WebRuntime {
			pkg: pkg_dir.into(),
			index,
		});
		self
	}

	/// Uses this page as `index.html` of the web runtime (see
	/// [`Self::web_runtime`], which must be set too).
	#[must_use]
	pub fn web_index(mut self, page: impl Into<PathBuf>) -> Self {
		if let Some(web) = &mut self.web {
			web.index = Some(page.into());
		}
		self
	}

	/// Removes the files matching `glob` (relative to the project root, `/`
	/// separators, for example `Assets/Scripts/**`) from the entry list.
	#[must_use]
	pub fn exclude(mut self, glob: impl Into<String>) -> Self {
		self.rules.push(Rule::Exclude(glob.into()));
		self
	}

	/// Adds the project files matching `glob` (relative to the project root),
	/// including files removed earlier and files outside the base list.
	#[must_use]
	pub fn include(mut self, glob: impl Into<String>) -> Self {
		self.rules.push(Rule::Include(glob.into()));
		self
	}

	/// Puts the project file `src` into the build at `dest`. An entry already
	/// at `dest` is replaced. Both resolve through the project's built-in
	/// schemes.
	#[must_use]
	pub fn add_file(mut self, dest: ZPath, src: ZPath) -> Self {
		self.rules.push(Rule::AddFile { dest, src });
		self
	}

	/// Puts `bytes` into the build at `dest`, replacing an entry already there.
	/// This is how changed settings are supplied: the caller prepares the new
	/// content.
	#[must_use]
	pub fn add_bytes(mut self, dest: ZPath, bytes: impl Into<Vec<u8>>) -> Self {
		self.rules.push(Rule::AddBytes {
			dest,
			bytes: bytes.into(),
		});
		self
	}

	/// Threads to run: what was asked for, or one per processor limited by the
	/// memory budget. `states` is how many zstd states a thread needs.
	fn thread_count(&self, options: &WriterOptions, states: u64) -> usize {
		if self.threads > 0 {
			return self.threads;
		}
		let cpus = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
		let per_thread = options.thread_memory().saturating_mul(states).max(1);
		let by_memory = usize::try_from(MEMORY_BUDGET / per_thread).unwrap_or(usize::MAX);
		cpus.min(by_memory).max(1)
	}

	/// Produces the build and reports what it did.
	///
	/// # Errors
	/// * `InvalidBuildOption`: missing `path`, a level outside `1..=22`, a bad
	///   glob.
	/// * `InvalidBuildOutput`: an output that overlaps what the build reads.
	/// * `NotABuildDir`: an `Fs` output folder that is not empty and holds no
	///   build.
	/// * `PreviousBuildUnreadable`: the build already in an `Fs` output folder
	///   cannot be read (other key, damaged); nothing is changed.
	/// * `SymlinkInBuild`: a symlink among the entries.
	/// * Project, archive and I/O errors.
	pub fn build(self) -> ZResult<BuildReport> {
		if self.compress && !(1..=22).contains(&self.level) {
			return Err(ZenithError::InvalidBuildOption(format!(
				"zstd compression level must be in 1..=22, got {}",
				self.level
			)));
		}
		let path = self
			.path
			.as_deref()
			.ok_or_else(|| ZenithError::InvalidBuildOption("the project path is not set".into()))?;
		let compiled = plan::compile_rules(&self.rules)?;

		let project = Project::open(path)?;
		if project.kind() != BackendKind::Fs {
			return Err(ZenithError::InvalidBuildOption(format!(
				"{} is a build, not a project folder",
				path.display()
			)));
		}
		let root = std::fs::canonicalize(path)?;
		let plain_root = std::path::absolute(path)?;
		let plan = plan::make_plan(&project, &self.rules, &compiled, self.format, self.key.is_some())?;

		let name = project.manifest().name.clone();
		let output_path = match &self.output {
			Some(path) => std::path::absolute(path)?,
			None => output::default_output(&plain_root, self.format),
		};
		output::check_output(&root, &output_path, &name, &plan, self.format)?;

		let parent = output_path.parent().ok_or_else(|| {
			ZenithError::InvalidBuildOutput(format!("{} has no parent folder", output_path.display()))
		})?;
		// The folder that holds the output is ignored. A folder build that sits
		// straight in the project root is its own folder: ignore that, or the
		// whole build would show up in version control. (A single file in the
		// root cannot be listed: the managed block only holds folders.)
		let in_root = output::resolve_for_compare(parent)? == root;
		let ignored = if self.format == BuildFormat::Fs && in_root {
			output_path.as_path()
		} else {
			parent
		};
		let exe = match &self.engine_exe {
			Some(exe) => {
				let destination = exe.destination(parent, &name)?;
				output::check_output(&root, &destination, &name, &plan, BuildFormat::Archive)?;
				Some((exe.source.as_path(), destination))
			}
			None => None,
		};
		let web = match &self.web {
			Some(web) => {
				let web_plan = web.plan(parent)?;
				for destination in web_plan.destinations() {
					output::check_output(&root, destination, &name, &plan, BuildFormat::Archive)?;
				}
				Some(web_plan)
			}
			None => None,
		};
		project.ensure_ignored_dir(ignored)?;

		let options = WriterOptions {
			compress: self.compress,
			compression_level: self.level,
			key: self.key.clone(),
			..WriterOptions::default()
		};
		let entries = plan.entries.len();
		let (compressed, reused, warnings) = match self.format {
			BuildFormat::Archive => {
				// Whole small files and chunks of big ones are compressed at
				// the same time; both need their own zstd state.
				let threads = self.thread_count(&options, 2);
				let options = WriterOptions { threads, ..options };
				archive_build::write_archive(&project, &plan, &output_path, &options)?;
				(entries, 0, Vec::new())
			}
			BuildFormat::Fs => {
				// One file per thread, so each writer compresses alone.
				let threads = self.thread_count(&options, 1);
				let stats =
					folder_build::write_folder(&project, &plan, &output_path, &options, threads, self.hash_cache)?;
				(stats.compressed, stats.reused, stats.warnings)
			}
		};
		let build_id = match self.format {
			BuildFormat::Archive => archive_build::build_id(&output_path, self.key.as_ref())?,
			BuildFormat::Fs => folder_build::build_id(&output_path, self.key.as_ref())?,
		};
		let engine_exe = match exe {
			Some((source, destination)) => {
				engine_exe::copy(source, &destination)?;
				Some(destination)
			}
			None => None,
		};
		let web_files = match web {
			Some(web) => web.write()?,
			None => Vec::new(),
		};
		Ok(BuildReport {
			output: output_path,
			entries,
			compressed,
			reused,
			build_id,
			warnings,
			engine_exe,
			web_files,
		})
	}
}

/// The base list is defined once, in the project crate.
pub const fn base_dirs() -> &'static [&'static str] {
	layout::BASE_BUILD_DIRS
}
