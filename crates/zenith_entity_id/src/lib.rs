use std::fmt;

use bevy_ecs::entity::Entity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ZenithEntityId(u32);

impl From<Entity> for ZenithEntityId {
	fn from(entity: Entity) -> Self { Self(entity.index_u32()) }
}

impl fmt::Display for ZenithEntityId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.0.to_string()) }
}
