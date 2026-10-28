//! `scheme://path`: a value naming a place in a project. It reads nothing by
//! itself; only a `Project` turns it into a real location.

use std::{fmt, str::FromStr, sync::LazyLock};

use convert_case::{Boundary, Case, Converter};
use zenith_error::{ZResult, ZenithError};

fn bad(text: &str, why: &str) -> ZenithError {
	ZenithError::InvalidZPath(format!("{text:?}: {why}"))
}

/// Checks a project-relative path: `/`-separated, no empty, `.` or `..`
/// segments, no backslashes. `:` and control characters are refused too, so a
/// path can never turn into a drive-relative or absolute one on Windows.
/// The empty path is valid (the root of a scheme).
pub fn validate_relative(path: &str) -> Result<(), &'static str> {
	if path.is_empty() {
		return Ok(());
	}
	for segment in path.split('/') {
		if segment.is_empty() {
			return Err("empty segment (leading, trailing or doubled slash)");
		}
		if segment == "." || segment == ".." {
			return Err("`.` and `..` segments are not allowed");
		}
		if segment.chars().any(|c| c == '\\' || c == ':' || c.is_control()) {
			return Err("forbidden character (backslash, colon or control character)");
		}
	}
	Ok(())
}

/// Turns the scheme a person wrote into the one the project knows it by.
///
/// A scheme starts with a letter and goes on with letters, digits and
/// `_ - + .`, in whatever case style suits the writer: `assets`, `Assets`,
/// `ASSETS`, and `MyTextures`, `myTextures`, `my_textures`, `my-textures` and
/// `MY_TEXTURES` are the same scheme. The canonical form is `PascalCase`
/// (`Assets`, `MyTextures`), made with `convert_case`. Digits do not start a
/// new word: `Tex2D` and `tex2d` are both `Tex2d`. Single-letter words (`A-b`,
/// which would print as `AB` and read back as one word) are refused.
///
/// `None` if the spelling is not a scheme.
fn canonical_scheme(scheme: &str) -> Option<String> {
	static CONVERTER: LazyLock<Converter> = LazyLock::new(|| {
		Converter::new()
			.remove_boundaries(&Boundary::digits())
			.to_case(Case::Pascal)
	});

	let mut chars = scheme.chars();
	let spelled_right = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
		&& chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '.'));
	if !spelled_right {
		return None;
	}
	// Words come from the spelling (`A-b_c` is three of them: `ABC`).
	// PascalCase cannot always be read back the way it was made, though: `ABC`
	// reads as one word. A scheme whose printed form would mean another scheme
	// is refused, so that what is printed (and kept, in the trash for
	// instance) parses to the same scheme.
	let canonical = CONVERTER.convert(scheme);
	if CONVERTER.convert(&canonical) != canonical {
		return None;
	}
	canonical
		.starts_with(|c: char| c.is_ascii_uppercase())
		.then_some(canonical)
}

/// A scheme plus a path below its root, for example
/// `Assets://textures/ferris.png`.
///
/// The scheme can be written in any case style (`Assets://x`, `ASSETS://x`,
/// `MyTextures://x` and `my_textures://x`); it is stored, compared and printed
/// in its canonical form, `PascalCase` (`Assets://x`, `MyTextures://x`). The
/// path below it is a file path and keeps its case.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ZPath {
	scheme: String,
	path: String,
}

impl ZPath {
	/// Builds a `ZPath` from text: `Worlds/world.zenith` or
	/// `Worlds://world.zenith`. The scheme is the part before the first `/` (or
	/// before `://`) and may be written in any case style; `Worlds/` and
	/// `Worlds://` are the root of a scheme.
	///
	/// # Errors
	/// `InvalidScheme` for a malformed scheme name, `InvalidZPath` for a path
	/// that is absolute, empty-segmented, contains `..` or backslashes, or for
	/// text with no separator at all.
	pub fn new(text: &str) -> ZResult<Self> {
		let (scheme, path) = text
			.split_once("://")
			.or_else(|| text.split_once('/'))
			.ok_or_else(|| bad(text, "expected `Scheme/path` or `Scheme://path`"))?;
		Self::from_parts(scheme, path)
	}

	/// Builds a `ZPath` from its two parts.
	///
	/// # Errors
	/// Same as [`Self::new`].
	pub fn from_parts(scheme: &str, path: &str) -> ZResult<Self> {
		let scheme = canonical_scheme(scheme).ok_or_else(|| ZenithError::InvalidScheme(scheme.to_string()))?;
		validate_relative(path).map_err(|why| bad(path, why))?;
		Ok(Self {
			scheme,
			path: path.to_string(),
		})
	}

	/// Same as [`Self::new`].
	///
	/// # Errors
	/// Same as [`Self::new`].
	pub fn parse(text: &str) -> ZResult<Self> {
		Self::new(text)
	}

	#[must_use]
	pub fn scheme(&self) -> &str {
		&self.scheme
	}

	/// Path below the scheme root; empty for the root itself.
	#[must_use]
	pub fn path(&self) -> &str {
		&self.path
	}

	/// Appends a path (one or more segments).
	///
	/// # Errors
	/// `InvalidZPath` if `segment` is not a valid relative path.
	pub fn join(&self, segment: &str) -> ZResult<Self> {
		if segment.is_empty() {
			return Err(bad(segment, "cannot join an empty path"));
		}
		let joined = if self.path.is_empty() {
			segment.to_string()
		} else {
			format!("{}/{segment}", self.path)
		};
		Self::from_parts(&self.scheme, &joined)
	}

	/// The path without its last segment; `None` at the scheme root.
	#[must_use]
	pub fn parent(&self) -> Option<Self> {
		if self.path.is_empty() {
			return None;
		}
		let parent = self.path.rsplit_once('/').map_or("", |(parent, _)| parent);
		Some(Self {
			scheme: self.scheme.clone(),
			path: parent.to_string(),
		})
	}

	/// Last segment; `None` at the scheme root.
	#[must_use]
	pub fn file_name(&self) -> Option<&str> {
		if self.path.is_empty() {
			None
		} else {
			Some(self.path.rsplit_once('/').map_or(self.path.as_str(), |(_, name)| name))
		}
	}
}

impl fmt::Display for ZPath {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}://{}", self.scheme, self.path)
	}
}

impl FromStr for ZPath {
	type Err = ZenithError;

	fn from_str(text: &str) -> Result<Self, Self::Err> {
		Self::parse(text)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_and_prints() -> ZResult<()> {
		let p = ZPath::parse("Assets://textures/ferris.png")?;
		assert_eq!(p.scheme(), "Assets", "scheme");
		assert_eq!(p.path(), "textures/ferris.png", "path");
		assert_eq!(p.to_string(), "Assets://textures/ferris.png", "display round trip");
		assert_eq!(p.file_name(), Some("ferris.png"), "file name");
		assert_eq!(
			p.parent().map(|p| p.to_string()),
			Some("Assets://textures".to_string()),
			"parent"
		);
		assert_eq!(ZPath::parse("Assets://")?.parent(), None, "root has no parent");
		assert_eq!(
			p.join("x.txt").map(|p| p.to_string()).ok(),
			Some("Assets://textures/ferris.png/x.txt".into()),
			"join"
		);
		Ok(())
	}

	#[test]
	fn the_scheme_can_be_written_in_any_case_style() -> ZResult<()> {
		for text in ["Assets://a/B", "Assets://a/B", "ASSETS://a/B"] {
			let p = ZPath::parse(text)?;
			assert_eq!(p.scheme(), "Assets", "{text}");
			assert_eq!(
				p.to_string(),
				"Assets://a/B",
				"printed canonically; the path keeps its case"
			);
		}
		for text in [
			"MyTextures",
			"myTextures",
			"my_textures",
			"my-textures",
			"MY_TEXTURES",
			"My-Textures",
		] {
			assert_eq!(ZPath::from_parts(text, "x")?.scheme(), "MyTextures", "{text}");
		}
		for text in ["URLScheme", "aBc", "x1y", "My.Pack"] {
			let once = ZPath::from_parts(text, "")?;
			assert_eq!(
				ZPath::from_parts(once.scheme(), "")?.scheme(),
				once.scheme(),
				"{text}: stable"
			);
		}
		for text in ["A-b_c", "a-b", "x_Y"] {
			assert!(
				ZPath::from_parts(text, "").is_err(),
				"{text}: would not read back the same"
			);
		}
		assert_eq!(
			ZPath::from_parts("Tex2D", "")?.scheme(),
			"Tex2d",
			"digits do not split words"
		);
		assert_eq!(ZPath::from_parts("svn+SSH", "")?.scheme(), "Svn+ssh");
		assert_eq!(ZPath::parse("Assets://a")?, ZPath::parse("Assets://a")?, "equal values");
		let again = ZPath::parse(&ZPath::parse("MyTextures://a")?.to_string())?;
		assert_eq!(again.scheme(), "MyTextures", "canonical form is stable");
		Ok(())
	}

	#[test]
	fn a_single_slash_works_like_the_separator() -> ZResult<()> {
		let short = ZPath::new("Worlds/world.zenith")?;
		assert_eq!(short, ZPath::new("Worlds://world.zenith")?, "both spellings");
		assert_eq!(short.to_string(), "Worlds://world.zenith", "printed with the separator");
		assert_eq!(ZPath::new("my_textures/a/B.png")?.to_string(), "MyTextures://a/B.png");
		assert_eq!(ZPath::new("Worlds/")?.path(), "", "the root of a scheme");
		for text in ["Worlds", "Worlds//x", "/x", "Worlds/../x", "Worlds/a/", "1a/x"] {
			assert!(ZPath::new(text).is_err(), "{text:?} must be rejected");
		}
		Ok(())
	}

	#[test]
	fn rejects_bad_paths() {
		for text in [
			"Assets://../x",
			"Assets://a/../b",
			"Assets://a/./b",
			"Assets:///abs",
			"Assets://a//b",
			"Assets://a/",
			"Assets://a\\b",
			"Assets://C:/x",
			"Assets://a\0b",
			"no-separator",
			"://x",
			"1abc://x",
			"_a://x",
			"as sets://x",
			"é://x",
		] {
			assert!(ZPath::parse(text).is_err(), "{text:?} must be rejected");
		}
		assert!(
			ZPath::from_parts("assets", "a").and_then(|p| p.join("../b")).is_err(),
			"join rejects traversal"
		);
	}
}
