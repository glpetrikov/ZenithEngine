//! Builds a Zenith project into a single archive file or a folder of files.
//!
//! See [`Builder`]. This crate only assembles the entry list and writes it
//! through `zenith_archive`; it knows nothing about script compilation or
//! about the format of settings files. Whatever should differ from the project
//! on disk (compiled scripts, patched settings) is prepared by the caller and
//! handed over with `add_file` / `add_bytes`.

#![deny(clippy::indexing_slicing, clippy::panic, clippy::todo, clippy::unimplemented)]

mod archive_build;
mod builder;
mod engine_exe;
mod folder_build;
mod hash_cache;
mod output;
mod plan;
mod web_runtime;

pub use builder::{BuildFormat, BuildReport, Builder};
pub use engine_exe::RenameExe;
pub use web_runtime::WebRuntime;
pub use zenith_archive::{BuildId, EncryptionKey};
pub use zenith_filesystem::ZPath;
