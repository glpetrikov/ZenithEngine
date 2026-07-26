use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shipyard::{Component, EntityId, advanced::get_component::Ref};
use ze_core::Result;

pub struct Entity<'a> {
	id: EntityId,
	scene: &'a mut crate::Scene,
}

impl<'a> Entity<'a> {
	pub(crate) const fn new(id: EntityId, scene: &'a mut crate::Scene) -> Self { Self { id, scene } }

	pub const fn id(&self) -> EntityId { self.id }

	pub fn add_component<T>(&mut self, component: T) -> &mut Self
	where
		T: Component + Clone + Serialize + for<'de> Deserialize<'de> + JsonSchema + 'static,
	{
		self.scene.add_component(self.id, component);
		self
	}

	pub fn remove_component<T>(&mut self) -> Result<()>
	where
		T: Component + Clone + Serialize + for<'de> Deserialize<'de> + JsonSchema + 'static,
	{
		let _ = self.scene.world.remove::<(T,)>(self.id);
		Ok(())
	}

	pub fn has_component<T>(&self) -> bool
	where
		T: Component + Clone + Serialize + for<'de> Deserialize<'de> + JsonSchema + 'static,
	{
		self.scene.world.get::<&T>(self.id).is_ok()
	}

	pub fn get_component<T>(&self) -> Result<Ref<'_, &T>>
	where
		T: Component + Clone + Serialize + for<'de> Deserialize<'de> + JsonSchema + 'static,
	{
		Ok(self.scene.world.get::<&T>(self.id)?)
	}
}
