//! Containers and the chunk codec of Zenith builds.
//!
//! Two containers share one codec:
//!
//! * [`ArchiveReader`] / `ArchiveWriter`: the whole build in one file.
//! * [`FolderReader`] / `FolderWriter`: `build.index` plus one self-contained
//!   data file per project file, so a game update only touches changed files.
//!
//! Data is cut into fixed-size chunks; every chunk is an independent zstd
//! frame, encrypted with AES-256-GCM-SIV when a key is given. Reading one
//! chunk never needs its neighbours.
//!
//! Builds are untrusted input: every size and offset is validated with
//! checked arithmetic and limits before memory is allocated, and no malformed
//! file may cause a panic.
//!
//! The readers do not depend on the `write` feature, so the crate builds for
//! `wasm32-unknown-unknown` without it (the zstd C library needs a C compiler
//! there; decoding uses the pure-Rust `ruzstd`).
//!
//! # Keys
//!
//! An [`EncryptionKey`] is 32 random bytes. Never derive it from a password:
//! each header carries a `key_check` value, which lets anyone holding a build
//! test guesses offline at full speed.

// Checked arithmetic and no slice indexing are part of the format's safety
// story. Unit tests are exempt from the arithmetic lint only; they still avoid
// indexing.
#![cfg_attr(not(test), deny(clippy::arithmetic_side_effects))]
#![deny(clippy::indexing_slicing, clippy::panic, clippy::todo, clippy::unimplemented)]

mod archive;
mod build_id;
mod crypto;
mod ctx;
mod entry_reader;
mod error;
mod folder;
mod format;
mod layout;
mod path;
mod probe;
mod read_at;

#[cfg(feature = "write")]
mod archive_write;
#[cfg(feature = "write")]
mod folder_write;
#[cfg(feature = "write")]
mod write;

pub use archive::{ArchiveEntry, ArchiveReader};
#[cfg(feature = "write")]
pub use archive_write::{ArchiveWriter, EntrySealer, SealedEntry};
pub use build_id::BuildId;
pub use crypto::EncryptionKey;
pub use entry_reader::EntryReader;
pub use folder::{DirSource, FolderEntry, FolderReader, MemDir};
#[cfg(feature = "write")]
pub use folder_write::{DataFileInfo, FolderIndexEntry, FolderWriter};
pub use format::{
	DEFAULT_CHUNK_SIZE_LOG2, FORMAT_VERSION, INDEX_FILE_NAME, MANIFEST_EXTENSION, MAX_CHUNK_SIZE_LOG2, MAX_ENTRIES,
	MAX_PATH_LEN, MIN_CHUNK_SIZE_LOG2,
};
pub use path::{is_windows_reserved_name, validate_entry_path, validate_entry_set};
pub use probe::{archive_is_encrypted, folder_is_encrypted, has_archive_magic};
pub use read_at::{FileSource, ReadAt};
#[cfg(feature = "write")]
pub use write::WriterOptions;
