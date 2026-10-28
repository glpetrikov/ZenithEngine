//! A Zenith project: a folder on disk, or a build of one.
//!
//! [`Project`] has three backends behind one API:
//!
//! * `Fs`: the project folder (feature `fs`, on by default).
//! * `BuildArchive`: a build in a single file, for the web and simple
//!   distribution.
//! * `BuildFs`: a build as a folder of separate files, for Steam-style updates.
//!
//! Places inside a project are named with [`ZPath`]
//! (`Assets://textures/a.png`). A `ZPath` is only a value; this crate is the
//! one place that turns it into a real location.
//!
//! The `fs` feature gates everything that needs a real filesystem: the `Fs`
//! backend, the trash, `.gitignore` handling and the file watcher. Without it
//! only the read-only build backends remain, which is what a web build needs.

// A project is edited by users and read from untrusted builds: no panics and
// no unchecked indexing in library code.
#![deny(clippy::indexing_slicing, clippy::panic, clippy::todo, clippy::unimplemented)]

mod builds;
mod cache;
mod core;
mod manifest;
mod options;
mod project;
mod scheme;
mod trash_id;
mod tree;
mod types;
mod zpath;

#[cfg(feature = "fs")]
mod fs;
#[cfg(feature = "fs")]
mod gitignore;
#[cfg(feature = "fs")]
mod trash;
#[cfg(feature = "fs")]
mod watch;

pub mod layout;

pub use cache::{CacheConfig, CacheSettings};
#[cfg(feature = "fs")]
pub use fs::WalkEntry;
pub use manifest::{MANIFEST_VERSION, Manifest, validate_project_name};
pub use options::{DEFAULT_MAX_READ_SIZE, OpenOptions, ProjectWarning, WarningKind};
#[cfg(feature = "fs")]
pub use project::FsProject;
pub use project::{ArchiveProject, BackendKind, BuildFsProject, Project};
#[cfg(feature = "fs")]
pub use trash::{OnConflict, RestoreOptions, TrashEntry};
pub use trash_id::TrashId;
pub use types::{DirEntry, EntryKind, Metadata, ReadSeek};
#[cfg(feature = "fs")]
pub use watch::{ChangeKind, EventReceiver, FileChange, ProjectEvent, WatchConfig, WatchRule, Watcher};
/// Re-exported so callers need no direct dependency on the archive crate for
/// the common cases.
pub use zenith_archive::{BuildId, DirSource, EncryptionKey, MemDir};
pub use zpath::ZPath;
