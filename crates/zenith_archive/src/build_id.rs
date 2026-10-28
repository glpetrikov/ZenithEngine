//! The identity of one build.

use std::fmt;

use sha2::{Digest, Sha256};
use zenith_error::{ZResult, ZenithError};

use crate::format::HEADER_LEN;

/// Names one build: 32 bytes that change whenever anything in the build does.
///
/// It is SHA-256 over the container header and the stored index (the archive's
/// index, or `build.index` of a folder build), and the index lists every file
/// with its size and, for each chunk or data file, what pins its exact bytes
/// (the chunk tags of an encrypted build, the content hashes of a folder
/// build). The same project, settings and key give the same id; any change
/// gives another. An old build is not a *damaged* build, so its files are all
/// valid; the only way to tell it from the current one is to know what the
/// current one is. Keep the id of a release somewhere the player cannot change
/// (inside the program, or in a signed manifest) and open the build with
/// `OpenOptions::expected_build_id`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuildId([u8; 32]);

impl BuildId {
	#[must_use]
	pub const fn from_bytes(bytes: [u8; 32]) -> Self {
		Self(bytes)
	}

	#[must_use]
	pub const fn as_bytes(&self) -> &[u8; 32] {
		&self.0
	}

	/// The id of a container, from its header and the index as stored.
	pub(crate) fn of(header: &[u8; HEADER_LEN], stored_index: &[u8]) -> Self {
		let mut hasher = Sha256::new();
		hasher.update(b"zenith-build-id-v1");
		hasher.update(header);
		hasher.update(stored_index);
		let digest = hasher.finalize();
		let mut id = [0u8; 32];
		for (b, d) in id.iter_mut().zip(digest.iter()) {
			*b = *d;
		}
		Self(id)
	}

	/// Lowercase hex, 64 digits.
	#[must_use]
	pub fn to_hex(&self) -> String {
		use fmt::Write;
		self.0.iter().fold(String::with_capacity(64), |mut text, byte| {
			let _ = write!(text, "{byte:02x}");
			text
		})
	}

	/// Parses 64 hex digits (either case).
	///
	/// # Errors
	/// `InvalidSettings` if the text is not 64 hex digits.
	pub fn from_hex(text: &str) -> ZResult<Self> {
		let bad = || ZenithError::InvalidSettings(format!("a build id is 64 hex digits, got {text:?}"));
		let digits = text.as_bytes();
		if digits.len() != 64 {
			return Err(bad());
		}
		let mut id = [0u8; 32];
		let (pairs, _) = digits.as_chunks::<2>();
		for (slot, [hi, lo]) in id.iter_mut().zip(pairs) {
			let value = |d: u8| char::from(d).to_digit(16).and_then(|v| u8::try_from(v).ok());
			match (value(*hi), value(*lo)) {
				(Some(hi), Some(lo)) => *slot = hi << 4 | lo,
				_ => return Err(bad()),
			}
		}
		Ok(Self(id))
	}
}

impl fmt::Display for BuildId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.to_hex())
	}
}

impl fmt::Debug for BuildId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "BuildId({})", self.to_hex())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hex_round_trip_and_errors() -> ZResult<()> {
		let id = BuildId::from_bytes(core::array::from_fn(|i| u8::try_from(i * 7).unwrap_or(0)));
		assert_eq!(BuildId::from_hex(&id.to_hex())?, id, "round trip");
		assert_eq!(BuildId::from_hex(&id.to_hex().to_uppercase())?, id, "either case");
		assert_eq!(format!("{id}"), id.to_hex(), "Display is the hex");
		for bad in [
			"",
			"ab",
			&"zz".repeat(32),
			&"a".repeat(65),
			&format!("{}é", "a".repeat(62)),
		] {
			assert!(BuildId::from_hex(bad).is_err(), "{bad:?} is not a build id");
		}
		Ok(())
	}

	#[test]
	fn the_id_depends_on_the_header_and_the_index() {
		let (a, b) = ([1u8; HEADER_LEN], [2u8; HEADER_LEN]);
		assert_eq!(BuildId::of(&a, b"index"), BuildId::of(&a, b"index"), "deterministic");
		assert_ne!(BuildId::of(&a, b"index"), BuildId::of(&a, b"index!"), "another index");
		assert_ne!(BuildId::of(&a, b"index"), BuildId::of(&b, b"index"), "another header");
	}
}
