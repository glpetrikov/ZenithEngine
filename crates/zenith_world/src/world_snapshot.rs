use std::collections::HashMap;

use bevy_ecs::entity::Entity;
use tracing::instrument;
use zenith_entity_id::ZenithEntityId;
use zenith_error::{ZPubResult, ZenithError};
use zenith_types::ecs::SaveWorld;

use crate::{WORLD_VERSION, World};

impl World {
	#[instrument(skip(self))]
	pub fn snapshot(&mut self) -> SaveWorld {
		let mut query = self.world.query::<Entity>();
		let entities = query
			.iter(&self.world)
			.map(|entity| self.registry.save_entity(entity, &self.world))
			.collect();

		SaveWorld {
			name: self.display_name.clone(),
			world_type: self.world_type.clone(),
			version: WORLD_VERSION,
			entities,
		}
	}

	#[instrument(skip(self, snapshot))]
	pub fn restore_snapshot(&mut self, snapshot: SaveWorld) -> ZPubResult<()> {
		self.display_name = if snapshot.name.is_empty() {
			self.name.clone()
		} else {
			snapshot.name.clone()
		};
		self.world_type = snapshot.world_type;
		self.clear_world();

		let mut remap: HashMap<ZenithEntityId, Entity> = HashMap::with_capacity(snapshot.entities.len());
		for saved_entity in &snapshot.entities {
			let new_entity = self.world.spawn_empty().id();
			remap.insert(saved_entity.id.clone(), new_entity);
		}

		for saved_entity in snapshot.entities {
			let new_entity = remap[&saved_entity.id];
			for component in saved_entity.components {
				self.registry
					.load_component(new_entity, &mut self.world, component)
					.map_err(|error| ZenithError::ComponentLoadFailed(error.to_string()))?;
			}
		}

		Ok(())
	}
}
