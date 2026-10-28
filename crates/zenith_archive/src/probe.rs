//! Cheap header inspection, so a caller can decide whether a key is needed
//! before handing one to a reader (which is strict about unneeded keys).

use zenith_error::ZResult;

use crate::{
	ctx::read_header,
	format::Kind,
	read_at::{ReadAt, read_exact_at},
};

/// First bytes of an archive file: checked without any other validation, so
/// "this is not an archive at all" can be told apart from "this archive is
/// damaged".
#[must_use]
pub fn has_archive_magic(src: &dyn ReadAt) -> bool {
	let mut magic = [0u8; 8];
	read_exact_at(src, 0, &mut magic).is_ok() && magic == Kind::Archive.magic()
}

/// Whether the archive is encrypted, read from its (verified) header.
///
/// # Errors
/// Fails if the header is damaged or from an unsupported version.
pub fn archive_is_encrypted(src: &dyn ReadAt) -> ZResult<bool> {
	Ok(read_header(src, Kind::Archive)?.encrypted)
}

/// Whether a folder build is encrypted, read from the header of `build.index`.
///
/// # Errors
/// Fails if the header is damaged or from an unsupported version.
pub fn folder_is_encrypted(index: &dyn ReadAt) -> ZResult<bool> {
	Ok(read_header(index, Kind::FolderIndex)?.encrypted)
}
