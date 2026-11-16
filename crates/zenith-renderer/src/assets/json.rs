//! Assets that are JSON: materials, render graphs, and later prefabs and
//! scenes. Every one has `uuid`, `type` and `version` at the top; the rest is
//! its own.

use serde::{Deserialize, Serialize};
use zenith_error::{ZResult, ZenithError};

use super::{AssetId, AssetKind};

fn json_error(e: &serde_json::Error) -> ZenithError {
	ZenithError::Json(e.to_string())
}

/// What every JSON asset starts with.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct JsonHeader {
	pub uuid: AssetId,
	#[serde(rename = "type")]
	pub kind: String,
	#[serde(default)]
	pub version: u32,
}

/// The id of a JSON asset of `kind`.
///
/// # Errors
/// `Json` for text that is not JSON or has no `uuid`, `InvalidAsset` if its
/// `type` is not that of `kind`.
pub fn read_id(text: &str, kind: AssetKind) -> ZResult<AssetId> {
	let header: JsonHeader = serde_json::from_str(text).map_err(|e| json_error(&e))?;
	if header.kind != kind.type_name() {
		return Err(ZenithError::InvalidAsset(format!(
			"a {:?} was expected, the file says it is a {:?}",
			kind.type_name(),
			header.kind
		)));
	}
	Ok(header.uuid)
}

/// The text of a JSON asset with another id; the rest stays as it was, in the
/// same order.
///
/// # Errors
/// `Json` for text that is not a JSON object.
pub fn set_id(text: &str, id: AssetId) -> ZResult<String> {
	let mut value: serde_json::Value = serde_json::from_str(text).map_err(|e| json_error(&e))?;
	let object = value
		.as_object_mut()
		.ok_or_else(|| ZenithError::Json("an asset is a JSON object".into()))?;
	object.insert("uuid".into(), serde_json::Value::String(id.to_string()));
	to_text(&value)
}

/// Pretty JSON with a newline at the end, the way every asset is written.
pub(crate) fn to_text(value: &impl Serialize) -> ZResult<String> {
	let mut text = serde_json::to_string_pretty(value).map_err(|e| json_error(&e))?;
	text.push('\n');
	Ok(text)
}

/// How a material lets what is behind it through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
	#[default]
	Opaque,
	/// Pixels with less alpha than `alpha_cutoff` are not drawn.
	Mask,
	/// Blended with what is behind it.
	Blend,
}

/// A material: how a surface looks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MaterialData {
	pub uuid: AssetId,
	#[serde(rename = "type")]
	pub kind: String,
	pub version: u32,
	/// The shader that draws it. Only `standard` for now.
	pub shader: String,
	/// Linear RGBA, multiplied with the texture.
	pub base_color: [f32; 4],
	pub base_color_texture: Option<AssetId>,
	pub normal_texture: Option<AssetId>,
	pub roughness: f32,
	pub metallic: f32,
	pub blend: BlendMode,
	pub alpha_cutoff: f32,
	pub double_sided: bool,
}

impl Default for MaterialData {
	fn default() -> Self {
		Self::new(AssetId::from_bytes([0; 16]))
	}
}

impl MaterialData {
	pub const VERSION: u32 = 1;

	/// A plain white material.
	pub fn new(uuid: AssetId) -> Self {
		Self {
			uuid,
			kind: AssetKind::Material.type_name().into(),
			version: Self::VERSION,
			shader: "standard".into(),
			base_color: [1.0; 4],
			base_color_texture: None,
			normal_texture: None,
			roughness: 0.5,
			metallic: 0.0,
			blend: BlendMode::Opaque,
			alpha_cutoff: 0.5,
			double_sided: false,
		}
	}

	/// # Errors
	/// `Json` for text that is not a material, `InvalidAsset` for one of
	/// another type or a newer version.
	pub fn parse(text: &str) -> ZResult<Self> {
		read_id(text, AssetKind::Material)?;
		let material: Self = serde_json::from_str(text).map_err(|e| json_error(&e))?;
		if material.version > Self::VERSION {
			return Err(ZenithError::InvalidAsset(format!(
				"the material is of version {}, this engine reads up to {}",
				material.version,
				Self::VERSION
			)));
		}
		Ok(material)
	}

	/// # Errors
	/// `Json` if it cannot be written (it always can).
	pub fn to_text(&self) -> ZResult<String> {
		to_text(self)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn id(byte: u8) -> AssetId {
		AssetId::from_bytes([byte; 16])
	}

	#[test]
	fn a_material_is_written_and_read_back() -> ZResult<()> {
		let material = MaterialData {
			base_color_texture: Some(id(2)),
			roughness: 0.8,
			..MaterialData::new(id(1))
		};
		let text = material.to_text()?;
		assert!(
			text.starts_with("{\n  \"uuid\": \"01010101-"),
			"the id comes first: {text}"
		);
		assert!(text.contains("\"type\": \"material\""), "{text}");
		assert_eq!(MaterialData::parse(&text)?, material);
		assert_eq!(read_id(&text, AssetKind::Material)?, id(1));
		Ok(())
	}

	#[test]
	fn what_a_material_leaves_out_is_the_default() -> ZResult<()> {
		let text = format!(
			"{{ \"uuid\": \"{}\", \"type\": \"material\", \"metallic\": 1.0 }}",
			id(5)
		);
		let material = MaterialData::parse(&text)?;
		assert!((material.metallic - 1.0).abs() < f32::EPSILON);
		assert_eq!(material.shader, "standard");
		Ok(())
	}

	#[test]
	fn another_type_or_a_newer_version_is_refused() {
		let graph = format!("{{ \"uuid\": \"{}\", \"type\": \"render_graph\" }}", id(5));
		assert!(MaterialData::parse(&graph).is_err());
		let newer = format!("{{ \"uuid\": \"{}\", \"type\": \"material\", \"version\": 99 }}", id(5));
		assert!(MaterialData::parse(&newer).is_err());
		assert!(
			read_id("{ \"type\": \"material\" }", AssetKind::Material).is_err(),
			"no id"
		);
	}

	#[test]
	fn a_new_id_keeps_the_rest_in_its_order() -> ZResult<()> {
		let text = MaterialData::new(id(1)).to_text()?;
		let copied = set_id(&text, id(9))?;
		assert_eq!(read_id(&copied, AssetKind::Material)?, id(9));
		assert_eq!(
			copied.replace(&id(9).to_string(), &id(1).to_string()),
			text,
			"only the id changed"
		);
		Ok(())
	}
}
