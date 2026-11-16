//! Pictures to `ImageData`.

use serde::{Deserialize, Serialize};
use zenith_error::{ZResult, ZenithError};

/// What the pixels of a texture are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureUsage {
	/// Colors, stored in sRGB: the GPU makes them linear when it reads them.
	#[default]
	Color,
	/// Directions of a normal map, read as they are.
	Normal,
	/// Light brighter than white (an `.hdr` sky), as 32-bit floats.
	Hdr,
	/// Anything else that is read as it is (roughness, a mask).
	Data,
}

impl TextureUsage {
	/// A guess from the name of a file: `normal` in it, an `.hdr`.
	pub fn guess(file_name: &str) -> Self {
		let extension = std::path::Path::new(file_name)
			.extension()
			.and_then(|extension| extension.to_str())
			.unwrap_or_default();
		if extension.eq_ignore_ascii_case("hdr") {
			Self::Hdr
		} else if file_name.to_ascii_lowercase().contains("normal") {
			Self::Normal
		} else {
			Self::Color
		}
	}

	/// Are the pixels sRGB colors?
	pub const fn is_srgb(self) -> bool {
		matches!(self, Self::Color)
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
	#[default]
	Linear,
	Nearest,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wrap {
	#[default]
	Repeat,
	Clamp,
	Mirror,
}

/// The import settings of a texture, kept in its container.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextureImport {
	/// The extension of the source (`png`, `jpg`, `hdr`, ...), for what reads
	/// it.
	pub source_format: String,
	pub usage: TextureUsage,
	pub mipmaps: bool,
	pub filter: Filter,
	pub wrap: Wrap,
}

impl Default for TextureImport {
	fn default() -> Self {
		Self::for_usage(TextureUsage::Color, "png")
	}
}

impl TextureImport {
	/// What suits a texture of `usage`: a sky is clamped and has no mipmaps
	/// of its own (the environment makes what it needs).
	pub fn for_usage(usage: TextureUsage, source_format: &str) -> Self {
		Self {
			source_format: source_format.to_ascii_lowercase(),
			usage,
			mipmaps: usage != TextureUsage::Hdr,
			filter: Filter::Linear,
			wrap: if usage == TextureUsage::Hdr {
				Wrap::Clamp
			} else {
				Wrap::Repeat
			},
		}
	}
}

/// The pixels of a texture, row by row from the top, four channels each.
#[derive(Clone, Debug, PartialEq)]
pub enum Pixels {
	Rgba8(Vec<u8>),
	Rgba32Float(Vec<f32>),
}

/// A decoded picture.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageData {
	pub width: u32,
	pub height: u32,
	pub usage: TextureUsage,
	pub pixels: Pixels,
}

/// Decodes the source of a texture.
///
/// # Errors
/// `ImageDecodeFailed` if it is not a picture `image` can read, `InvalidAsset`
/// for a picture of no size.
pub fn decode(source: &[u8], settings: &TextureImport) -> ZResult<ImageData> {
	let picture = image::load_from_memory(source).map_err(|e| ZenithError::ImageDecodeFailed(e.to_string()))?;
	let (width, height) = (picture.width(), picture.height());
	if width == 0 || height == 0 {
		return Err(ZenithError::InvalidAsset("the picture has no pixels".into()));
	}
	let pixels = if settings.usage == TextureUsage::Hdr {
		Pixels::Rgba32Float(picture.to_rgba32f().into_raw())
	} else {
		Pixels::Rgba8(picture.to_rgba8().into_raw())
	};
	Ok(ImageData {
		width,
		height,
		usage: settings.usage,
		pixels,
	})
}

/// Can `source` be decoded? Cheaper than [`decode`]: only the header is read.
///
/// # Errors
/// `ImageDecodeFailed` if it is not a picture `image` can read.
pub fn check(source: &[u8]) -> ZResult<(u32, u32)> {
	image::ImageReader::new(std::io::Cursor::new(source))
		.with_guessed_format()
		.map_err(|e| ZenithError::ImageDecodeFailed(e.to_string()))?
		.into_dimensions()
		.map_err(|e| ZenithError::ImageDecodeFailed(e.to_string()))
}

#[cfg(test)]
pub(crate) mod tests {
	use super::*;

	/// A PNG of `width` by `height` red pixels.
	pub fn png(width: u32, height: u32) -> Vec<u8> {
		let picture = image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 255]));
		let mut bytes = Vec::new();
		let written = picture.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png);
		assert!(written.is_ok(), "write the png: {written:?}");
		bytes
	}

	#[test]
	fn a_picture_is_decoded() -> ZResult<()> {
		let data = decode(&png(3, 2), &TextureImport::default())?;
		assert_eq!((data.width, data.height), (3, 2));
		assert_eq!(check(&png(3, 2))?, (3, 2));
		let Pixels::Rgba8(pixels) = data.pixels else {
			return Err(ZenithError::Other("8-bit pixels were expected".into()));
		};
		assert_eq!(pixels.len(), 3 * 2 * 4);
		assert_eq!(pixels.get(..4), Some([255, 0, 0, 255].as_slice()));
		Ok(())
	}

	#[test]
	fn an_hdr_texture_gets_floats() -> ZResult<()> {
		let data = decode(&png(1, 1), &TextureImport::for_usage(TextureUsage::Hdr, "png"))?;
		assert_eq!(data.pixels, Pixels::Rgba32Float(vec![1.0, 0.0, 0.0, 1.0]));
		Ok(())
	}

	#[test]
	fn what_is_not_a_picture_is_refused() {
		assert!(decode(b"not a picture", &TextureImport::default()).is_err());
		assert!(check(b"not a picture").is_err());
	}

	#[test]
	fn the_usage_is_guessed_from_the_name() {
		assert_eq!(TextureUsage::guess("cube-normal.png"), TextureUsage::Normal);
		assert_eq!(TextureUsage::guess("pure-sky.HDR"), TextureUsage::Hdr);
		assert_eq!(TextureUsage::guess("cube-diffuse.png"), TextureUsage::Color);
	}
}
