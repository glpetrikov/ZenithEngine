use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zenith_error::{ZResult, ZenithError};

#[derive(Debug, Clone)]
pub enum ColorValue {
	Literal(csscolorparser::Color),
	Reference(String),
}

impl<'de> Deserialize<'de> for ColorValue {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let s = String::deserialize(deserializer)?;
		if s.starts_with('$') {
			return Ok(Self::Reference(s));
		}
		s.parse::<csscolorparser::Color>()
			.map(ColorValue::Literal)
			.map_err(serde::de::Error::custom)
	}
}

impl Serialize for ColorValue {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		match self {
			Self::Literal(color) => serializer.serialize_str(&color.to_css_hex()),
			Self::Reference(name) => serializer.serialize_str(name),
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Theme {
	pub name: String,
	pub based_on: Option<String>,
	pub palette: HashMap<String, csscolorparser::Color>,
	pub colors: HashMap<String, ColorValue>,
	pub description: String,
}

impl Theme {
	/// Resolves all color references in the theme's `colors` map, replacing any
	/// references with their corresponding literal colors from the `palette`.
	///
	/// # Errors
	/// Returns `Err` if any color reference in `colors` does not have a
	/// matching entry in the `palette`.
	pub fn resolve_colors(&self) -> ZResult<HashMap<String, csscolorparser::Color>> {
		self.colors
			.iter()
			.map(|(key, value)| {
				let color = match value {
					ColorValue::Literal(c) => c.clone(),
					ColorValue::Reference(name) => {
						let palette_key = name.strip_prefix('$').unwrap_or(name.as_str());
						self.palette.get(palette_key).cloned().ok_or_else(|| {
							ZenithError::UnknownPaletteReference(name.clone(), palette_key.to_string())
						})?
					}
				};
				Ok((key.clone(), color))
			})
			.collect()
	}
}
