use serde::{Deserialize, Deserializer, de};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
	pub r: f32,
	pub g: f32,
	pub b: f32,
	pub a: f32,
}

impl Color {
	pub const WHITE: Self = Self {
		r: 1.0,
		g: 1.0,
		b: 1.0,
		a: 1.0,
	};
	pub const BLACK: Self = Self {
		r: 0.0,
		g: 0.0,
		b: 0.0,
		a: 1.0,
	};
	pub const RED: Self = Self {
		r: 1.0,
		g: 0.0,
		b: 0.0,
		a: 1.0,
	};
	pub const GREEN: Self = Self {
		r: 0.0,
		g: 1.0,
		b: 0.0,
		a: 1.0,
	};
	pub const BLUE: Self = Self {
		r: 0.0,
		g: 0.0,
		b: 1.0,
		a: 1.0,
	};
	pub const TRANSPARENT: Self = Self {
		r: 0.0,
		g: 0.0,
		b: 0.0,
		a: 0.0,
	};

	pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self { Self { r, g, b, a } }
	pub const fn rgb(r: f32, g: f32, b: f32) -> Self { Self { r, g, b, a: 1.0 } }
	pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self { Self { r, g, b, a } }

	pub fn new_u8(r: u8, g: u8, b: u8, a: u8) -> Self {
		Self {
			r: byte_channel_to_float(r),
			g: byte_channel_to_float(g),
			b: byte_channel_to_float(b),
			a: byte_channel_to_float(a),
		}
	}

	pub fn rgb_u8(r: u8, g: u8, b: u8) -> Self {
		Self {
			r: byte_channel_to_float(r),
			g: byte_channel_to_float(g),
			b: byte_channel_to_float(b),
			a: 1.0,
		}
	}

	pub fn rgba_u8(r: u8, g: u8, b: u8, a: u8) -> Self { Self::new_u8(r, g, b, a) }
}

pub fn deserialize_rgba<'de, D>(deserializer: D) -> Result<[f32; 4], D::Error>
where
	D: Deserializer<'de>,
{
	RgbaInput::deserialize(deserializer).and_then(|color| color.into_rgba().map_err(de::Error::custom))
}

pub fn deserialize_optional_rgba<'de, D>(deserializer: D) -> Result<Option<[f32; 4]>, D::Error>
where
	D: Deserializer<'de>,
{
	Option::<RgbaInput>::deserialize(deserializer)?
		.map(|color| color.into_rgba().map_err(de::Error::custom))
		.transpose()
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RgbaInput {
	Byte([u8; 4]),
	Float([f32; 4]),
}

impl RgbaInput {
	fn into_rgba(self) -> Result<[f32; 4], &'static str> {
		match self {
			Self::Byte(color) => Ok([
				byte_channel_to_float(color[0]),
				byte_channel_to_float(color[1]),
				byte_channel_to_float(color[2]),
				byte_channel_to_float(color[3]),
			]),
			Self::Float(color) => {
				validate_float_rgba(color)?;
				Ok(color)
			}
		}
	}
}

fn byte_channel_to_float(channel: u8) -> f32 { f32::from(channel) / 255.0 }

fn validate_float_rgba(color: [f32; 4]) -> Result<(), &'static str> {
	if color
		.iter()
		.all(|channel| channel.is_finite() && *channel >= 0.0 && *channel <= 1.0)
	{
		Ok(())
	} else {
		Err("float color channels must be finite values in the 0.0..=1.0 range")
	}
}
