use std::fmt;

use bevy_ecs::entity::Entity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ZeEntityId(String);

impl From<Entity> for ZeEntityId {
	fn from(entity: Entity) -> Self { Self(entity.to_string().replace('v', ":")) }
}

impl fmt::Display for ZeEntityId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}
