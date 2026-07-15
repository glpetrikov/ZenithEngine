use bevy_reflect::Reflect;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[reflect(opaque)]
pub struct ZeEntityId {
	pub index: u64,
	pub generation: u16,
}

impl From<shipyard::EntityId> for ZeEntityId {
	fn from(id: shipyard::EntityId) -> Self {
		Self {
			index: id.index(),
			generation: id.r#gen(),
		}
	}
}

impl From<ZeEntityId> for shipyard::EntityId {
	fn from(ze_id: ZeEntityId) -> Self { Self::new_from_index_and_gen(ze_id.index, ze_id.generation) }
}

impl Serialize for ZeEntityId {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		(self.index, self.generation).serialize(serializer)
	}
}

impl<'de> Deserialize<'de> for ZeEntityId {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		let (index, generation) = <(u64, u16)>::deserialize(deserializer)?;
		Ok(Self { index, generation })
	}
}
