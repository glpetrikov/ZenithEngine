use std::fmt;

use zenith_error::{ZResult, ZenithError};

/// Name of a trash entry's folder: `<unix milliseconds, 16 digits>-<8 hex>`.
///
/// The fixed-width timestamp makes plain string order equal deletion order
/// (barring a clock that goes backwards); the random suffix separates entries
/// deleted in the same millisecond. Only digits, lowercase hex and one dash:
/// safe as a file name on every filesystem, and impossible to use for path
/// traversal.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrashId(String);

const DIGITS: usize = 16;
const SUFFIX: usize = 8;

impl TrashId {
	/// Parses and validates an id.
	///
	/// # Errors
	/// `InvalidTrashId` if the text is not in the documented format.
	pub fn parse(text: &str) -> ZResult<Self> {
		let valid = text.len() == DIGITS + 1 + SUFFIX
			&& text
				.get(..DIGITS)
				.is_some_and(|d| d.bytes().all(|b| b.is_ascii_digit()))
			&& text.get(DIGITS..=DIGITS) == Some("-")
			&& text
				.get(DIGITS + 1..)
				.is_some_and(|s| s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
		if valid {
			Ok(Self(text.to_string()))
		} else {
			Err(ZenithError::InvalidTrashId(text.to_string()))
		}
	}

	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}

	/// Milliseconds since the Unix epoch encoded in the id.
	#[must_use]
	pub fn timestamp_ms(&self) -> u64 {
		self.0.get(..DIGITS).and_then(|d| d.parse().ok()).unwrap_or(0)
	}

	/// A fresh id for `now_ms`; `salt` varies the suffix.
	#[cfg(any(feature = "fs", test))]
	pub(crate) fn new(now_ms: u64, salt: u32) -> Self {
		Self(format!("{now_ms:016}-{salt:08x}"))
	}
}

impl fmt::Display for TrashId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(&self.0)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn format_and_order() -> ZResult<()> {
		let a = TrashId::new(1_764_547_200_123, 0xa1b2_c3d4);
		assert_eq!(a.as_str(), "0001764547200123-a1b2c3d4", "documented format");
		assert_eq!(TrashId::parse(a.as_str())?, a, "parse round trip");
		assert_eq!(a.timestamp_ms(), 1_764_547_200_123, "timestamp");
		let later = TrashId::new(1_764_547_200_124, 0);
		let much_later = TrashId::new(10_000_000_000_000, 0);
		assert!(a < later && later < much_later, "string order is time order");
		Ok(())
	}

	#[test]
	fn rejects_anything_else() {
		for bad in [
			"",
			"../etc",
			"0001764547200123-a1b2c3d",
			"0001764547200123-a1b2c3d4x",
			"0001764547200123_a1b2c3d4",
			"000176454720012-aa1b2c3d4",
			"0001764547200123-A1B2C3D4",
			"0001764547200123-a1b2c3g4",
			"0001764547200123/a1b2c3d4",
		] {
			assert!(TrashId::parse(bad).is_err(), "{bad:?} must be rejected");
		}
	}
}
