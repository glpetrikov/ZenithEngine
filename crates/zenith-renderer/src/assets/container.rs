//! The binary container of models and textures.
//!
//! | Offset | Size | Field |
//! | --- | --- | --- |
//! | 0 | 4 | magic: `ZMDL` (model) or `ZTEX` (texture) |
//! | 4 | 4 | format version, `u32` little-endian |
//! | 8 | 16 | the [`AssetId`] |
//! | 24 | 4 | length of the settings, `u32` little-endian |
//! | 28 | that length | the import settings, JSON (UTF-8) |
//! | after them | to the end | the source file, byte for byte |
//!
//! The header ([`HEADER_LEN`] bytes) is read on its own, without the rest of
//! the file: the registry needs nothing more. The id sits at a fixed place, so
//! a copy gets a new one by writing 16 bytes ([`set_id`]).

use std::io::Read;

use serde::{Serialize, de::DeserializeOwned};
use zenith_error::{ZResult, ZenithError};

use super::{AssetId, AssetKind, refuse_lfs_pointer};

/// The version of the container this engine writes and reads.
pub const FORMAT_VERSION: u32 = 1;
/// The size of the header.
pub const HEADER_LEN: usize = 28;
/// Import settings longer than this are refused: they are a few hundred
/// bytes, and a broken file must not make the engine allocate gigabytes.
const MAX_SETTINGS_LEN: u32 = 1 << 20;
const ID_AT: usize = 8;

const fn magic(kind: AssetKind) -> Option<&'static [u8; 4]> {
	match kind {
		AssetKind::Model => Some(b"ZMDL"),
		AssetKind::Texture => Some(b"ZTEX"),
		AssetKind::Material | AssetKind::RenderGraph => None,
	}
}

const fn bad(message: String) -> ZenithError {
	ZenithError::InvalidAsset(message)
}

/// The header of a container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
	pub kind: AssetKind,
	pub version: u32,
	pub id: AssetId,
	/// How many bytes of settings follow the header.
	pub settings_len: u32,
}

impl Header {
	/// Reads the header from the first [`HEADER_LEN`] bytes of `bytes`.
	///
	/// # Errors
	/// `InvalidAsset` for a file that is too short, is not a container (a Git
	/// LFS pointer is named as such), was made by a newer engine, or claims
	/// settings too long to be true.
	pub fn parse(bytes: &[u8]) -> ZResult<Self> {
		refuse_lfs_pointer(bytes, &"the asset")?;
		let header: &[u8; HEADER_LEN] = bytes
			.first_chunk()
			.ok_or_else(|| bad(format!("{} bytes are too few for an asset", bytes.len())))?;
		let (magic_bytes, rest) = header.split_at(4);
		let kind = [AssetKind::Model, AssetKind::Texture]
			.into_iter()
			.find(|kind| magic(*kind).is_some_and(|m| m == magic_bytes))
			.ok_or_else(|| bad("not an asset container (unknown magic)".into()))?;
		let (version, rest) = rest.split_at(4);
		let (id, length) = rest.split_at(16);
		let version = u32::from_le_bytes(version.try_into().map_err(|_| bad("short header".into()))?);
		if version > FORMAT_VERSION {
			return Err(bad(format!(
				"the asset is of version {version}, this engine reads up to {FORMAT_VERSION}: it was made by a newer engine"
			)));
		}
		let id = AssetId::from_bytes(id.try_into().map_err(|_| bad("short header".into()))?);
		let settings_len = u32::from_le_bytes(length.try_into().map_err(|_| bad("short header".into()))?);
		if settings_len > MAX_SETTINGS_LEN {
			return Err(bad(format!("the asset claims {settings_len} bytes of settings")));
		}
		Ok(Self {
			kind,
			version,
			id,
			settings_len,
		})
	}

	/// Reads the header from the start of `reader`, and nothing more.
	///
	/// # Errors
	/// An I/O error, or those of [`Header::parse`].
	pub fn read(reader: &mut dyn Read) -> ZResult<Self> {
		let mut bytes = Vec::with_capacity(HEADER_LEN);
		reader.take(HEADER_LEN as u64).read_to_end(&mut bytes)?;
		Self::parse(&bytes)
	}
}

/// A whole container: its id, its settings and its source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container<S> {
	pub kind: AssetKind,
	pub id: AssetId,
	pub settings: S,
	pub source: Vec<u8>,
}

/// Makes the bytes of a container.
///
/// # Errors
/// `InvalidAsset` if `kind` is not a binary asset, `Json` if the settings
/// cannot be written.
pub fn write<S: Serialize>(kind: AssetKind, id: AssetId, settings: &S, source: &[u8]) -> ZResult<Vec<u8>> {
	let magic = magic(kind).ok_or_else(|| bad(format!("a {kind:?} is JSON, not a container")))?;
	let settings = serde_json::to_vec_pretty(settings).map_err(|e| ZenithError::Json(e.to_string()))?;
	let settings_len = u32::try_from(settings.len())
		.ok()
		.filter(|len| *len <= MAX_SETTINGS_LEN)
		.ok_or_else(|| bad("the import settings are too long".into()))?;
	let mut bytes = Vec::with_capacity(HEADER_LEN + settings.len() + source.len());
	bytes.extend_from_slice(magic);
	bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
	bytes.extend_from_slice(id.as_bytes());
	bytes.extend_from_slice(&settings_len.to_le_bytes());
	bytes.extend_from_slice(&settings);
	bytes.extend_from_slice(source);
	Ok(bytes)
}

/// Reads a whole container, which must be of `kind`.
///
/// # Errors
/// Those of [`Header::parse`]; `InvalidAsset` for another kind or settings cut
/// short; `Json` for settings that are not what `S` expects.
pub fn read<S: DeserializeOwned>(bytes: &[u8], kind: AssetKind) -> ZResult<Container<S>> {
	let header = Header::parse(bytes)?;
	if header.kind != kind {
		return Err(bad(format!("a {:?} was expected, this is a {:?}", kind, header.kind)));
	}
	let settings_end = HEADER_LEN + header.settings_len as usize;
	let settings = bytes
		.get(HEADER_LEN..settings_end)
		.ok_or_else(|| bad("the settings of the asset are cut short".into()))?;
	let settings = serde_json::from_slice(settings).map_err(|e| ZenithError::Json(e.to_string()))?;
	Ok(Container {
		kind,
		id: header.id,
		settings,
		source: bytes.get(settings_end..).unwrap_or_default().to_vec(),
	})
}

/// Gives the container in `bytes` another id, in place.
///
/// # Errors
/// Those of [`Header::parse`].
pub fn set_id(bytes: &mut [u8], id: AssetId) -> ZResult<()> {
	Header::parse(bytes)?;
	if let Some(place) = bytes.get_mut(ID_AT..ID_AT + 16) {
		place.copy_from_slice(id.as_bytes());
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use serde::Deserialize;

	use super::*;

	#[derive(Debug, PartialEq, Serialize, Deserialize)]
	struct Settings {
		scale: f32,
	}

	fn id(byte: u8) -> AssetId {
		AssetId::from_bytes([byte; 16])
	}

	#[test]
	fn a_container_is_read_back() -> ZResult<()> {
		let bytes = write(AssetKind::Model, id(7), &Settings { scale: 2.0 }, b"v 1 2 3\n")?;
		assert_eq!(bytes.get(..4), Some(b"ZMDL".as_slice()));

		let header = Header::read(&mut bytes.as_slice())?;
		assert_eq!(header.kind, AssetKind::Model);
		assert_eq!(header.id, id(7));
		assert_eq!(header.version, FORMAT_VERSION);

		let container: Container<Settings> = read(&bytes, AssetKind::Model)?;
		assert_eq!(container.id, id(7));
		assert_eq!(container.settings, Settings { scale: 2.0 });
		assert_eq!(container.source, b"v 1 2 3\n");
		Ok(())
	}

	#[test]
	fn the_header_alone_is_enough() -> ZResult<()> {
		let bytes = write(AssetKind::Texture, id(3), &Settings { scale: 1.0 }, &[0; 10_000])?;
		let header = Header::parse(bytes.get(..HEADER_LEN).unwrap_or_default())?;
		assert_eq!((header.kind, header.id), (AssetKind::Texture, id(3)));
		Ok(())
	}

	#[test]
	fn a_copy_gets_another_id_and_keeps_the_rest() -> ZResult<()> {
		let mut bytes = write(AssetKind::Model, id(1), &Settings { scale: 1.0 }, b"source")?;
		let before = bytes.clone();
		set_id(&mut bytes, id(2))?;
		assert_eq!(Header::parse(&bytes)?.id, id(2));
		assert_eq!(bytes.get(HEADER_LEN..), before.get(HEADER_LEN..), "the rest stays");
		Ok(())
	}

	#[test]
	fn what_is_not_a_good_container_is_refused() -> ZResult<()> {
		assert!(Header::parse(b"ZMDL").is_err(), "too short");
		assert!(Header::parse(&[0; 40]).is_err(), "no magic");
		assert!(
			Header::parse(b"version https://git-lfs.github.com/spec/v1\noid sha256:00").is_err(),
			"a Git LFS pointer"
		);

		let mut newer = write(AssetKind::Model, id(1), &Settings { scale: 1.0 }, b"")?;
		if let Some(version) = newer.get_mut(4..8) {
			version.copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
		}
		assert!(Header::parse(&newer).is_err(), "a newer engine made it");

		let mut huge = write(AssetKind::Model, id(1), &Settings { scale: 1.0 }, b"")?;
		if let Some(length) = huge.get_mut(24..28) {
			length.copy_from_slice(&u32::MAX.to_le_bytes());
		}
		assert!(Header::parse(&huge).is_err(), "settings too long to be true");

		let model = write(AssetKind::Model, id(1), &Settings { scale: 1.0 }, b"")?;
		assert!(read::<Settings>(&model, AssetKind::Texture).is_err(), "another kind");
		assert!(
			read::<Settings>(model.get(..HEADER_LEN + 3).unwrap_or_default(), AssetKind::Model).is_err(),
			"settings cut short"
		);
		assert!(write(AssetKind::Material, id(1), &Settings { scale: 1.0 }, b"").is_err());
		Ok(())
	}
}
