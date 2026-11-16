//! `AssetId`: the UUID of an asset.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zenith_error::{ZResult, ZenithError};

/// What a component or another asset refers to an asset by. It stays the same
/// when the file is moved or renamed, and when its source is replaced.
///
/// In text (JSON, a world file) it is the usual UUID string.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AssetId(Uuid);

impl AssetId {
	/// A new id, random (a version 4 UUID).
	///
	/// # Errors
	/// `Other` if the system random generator fails.
	pub fn generate() -> ZResult<Self> {
		let mut bytes = [0; 16];
		getrandom::getrandom(&mut bytes)
			.map_err(|e| ZenithError::Other(format!("no random bytes for an asset id: {e}")))?;
		Ok(Self(uuid::Builder::from_random_bytes(bytes).into_uuid()))
	}

	pub const fn from_bytes(bytes: [u8; 16]) -> Self {
		Self(Uuid::from_bytes(bytes))
	}

	pub const fn as_bytes(&self) -> &[u8; 16] {
		self.0.as_bytes()
	}

	pub const fn uuid(&self) -> Uuid {
		self.0
	}
}

impl From<Uuid> for AssetId {
	fn from(uuid: Uuid) -> Self {
		Self(uuid)
	}
}

impl fmt::Display for AssetId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.0.hyphenated().fmt(f)
	}
}

impl fmt::Debug for AssetId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "AssetId({})", self.0.hyphenated())
	}
}

impl FromStr for AssetId {
	type Err = ZenithError;

	fn from_str(text: &str) -> ZResult<Self> {
		Uuid::parse_str(text)
			.map(Self)
			.map_err(|e| ZenithError::InvalidAsset(format!("{text:?} is not an asset id: {e}")))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn new_ids_differ_and_are_random_uuids() -> ZResult<()> {
		let a = AssetId::generate()?;
		let b = AssetId::generate()?;
		assert_ne!(a, b);
		assert_eq!(a.uuid().get_version_num(), 4);
		Ok(())
	}

	#[test]
	fn an_id_is_a_uuid_string_in_text() -> ZResult<()> {
		let text = "8f0c6a43-58a4-4c1e-9a0b-2f1d3c4e5a6b";
		let id: AssetId = text.parse()?;
		assert_eq!(id.to_string(), text);
		assert_eq!(
			serde_json::to_string(&id).map_err(|e| ZenithError::Json(e.to_string()))?,
			format!("\"{text}\"")
		);
		assert!("not an id".parse::<AssetId>().is_err());
		Ok(())
	}
}
