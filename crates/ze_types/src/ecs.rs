use serde::{Deserialize, Serialize};
use serde_json::Value;
use ze_entity_id::ZeEntityId;

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldType {
	#[default]
	World,
	Prefab,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SaveWorld {
	pub name: String,

	pub world_type: WorldType,

	pub version: String,

	pub entities: Vec<SavedEntity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SavedEntity {
	pub id: ZeEntityId,

	pub components: Vec<SavedComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SavedComponent {
	pub component_type: String,
	pub component_version: crate::Version,
	pub value: Value,
}
