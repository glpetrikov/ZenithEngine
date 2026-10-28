//! The folder layout of a project, defined once. Other crates (the build tool
//! in particular) take these names from here instead of repeating them.

/// Extension of the project manifest file (`<ProjectName>.zenithproject`).
pub const MANIFEST_EXTENSION: &str = "zenithproject";
/// Empty flag file marking the folder-structure version 1.
pub const V1_FLAG: &str = ".v1";
pub const GITIGNORE: &str = ".gitignore";
/// Written into a new project's `.gitignore`.
pub const DEFAULT_GITIGNORE: &str = "Temp/\nTrash/\n*.zenith-tmp\n";
/// Ending of the temporary file next to a file that is being replaced
/// atomically (`write`, archive and folder builds).
pub const TMP_SUFFIX: &str = ".zenith-tmp";

pub const DIR_ASSETS: &str = "Assets";
pub const DIR_WORLDS: &str = "Assets/Worlds";
pub const DIR_PACKAGES: &str = "Packages";
pub const DIR_SETTINGS: &str = "Settings";
pub const DIR_TEMP: &str = "Temp";
pub const DIR_TRASH: &str = "Trash";
pub const DIR_GIT: &str = ".git";
/// Not created with the project; appears when something asks for it.
pub const DIR_BUILD: &str = "Build";

/// Name of a build when the builder is not told where to put it, whatever the
/// project is called: `Build/game.zenithbuild` or `Build/game/`. A runtime can
/// open it with a fixed name.
pub const DEFAULT_BUILD_NAME: &str = "game";
/// Extension of a single-file build.
pub const BUILD_FILE_EXTENSION: &str = "zenithbuild";

/// Folders `Project::create` makes, parents before children.
pub const PROJECT_DIRS: &[&str] = &[DIR_ASSETS, DIR_WORLDS, DIR_PACKAGES, DIR_SETTINGS, DIR_TEMP, DIR_TRASH];

/// Folders that go into a build next to the manifest.
pub const BASE_BUILD_DIRS: &[&str] = &[DIR_ASSETS, DIR_SETTINGS, DIR_PACKAGES];

/// Built-in schemes and the project folders they point at.
pub const BUILTIN_SCHEMES: &[(&str, &str)] = &[
	("Assets", DIR_ASSETS),
	("Worlds", DIR_WORLDS),
	("Settings", DIR_SETTINGS),
	("Packages", DIR_PACKAGES),
	("Temp", DIR_TEMP),
	("Trash", DIR_TRASH),
];

/// Never reported by the watcher, whatever the rules say.
pub const ALWAYS_IGNORED_DIRS: &[&str] = &[DIR_TEMP, DIR_TRASH, DIR_BUILD, DIR_GIT];

pub const CACHE_CONFIG_FILE: &str = "Settings/CacheConfig.toml";
pub const BUILD_CONFIG_FILE: &str = "Settings/Build.toml";

/// Index file of a folder build (re-exported from the archive crate).
pub use zenith_archive::INDEX_FILE_NAME as BUILD_INDEX_FILE;

/// Debounce window of the file watcher.
pub const WATCH_DEBOUNCE_MS: u64 = 200;
/// Events the watcher queue holds before it collapses into a `Rescan`.
pub const WATCH_QUEUE_CAPACITY: usize = 1024;

/// Name of the manifest for a project folder name.
#[must_use]
pub fn manifest_file_name(project_name: &str) -> String {
	format!("{project_name}.{MANIFEST_EXTENSION}")
}

/// Whether a project-relative path lies in the trash.
#[must_use]
pub fn is_trash_path(rel: &str) -> bool {
	rel == DIR_TRASH || rel.strip_prefix(DIR_TRASH).is_some_and(|rest| rest.starts_with('/'))
}
