use std::collections::HashMap;

use bevy_ecs::entity::Entity;
use tracing::instrument;
use zenith_entity_id::ZenithEntityId;
use zenith_error::{ZResult, ZenithError};
use zenith_types::ecs::SaveWorld;

use crate::{WORLD_VERSION, World};

impl World {
	#[instrument(skip(self))]
	pub fn snapshot(&mut self) -> SaveWorld {
		let mut query = self.world.query::<Entity>();
		let entities = query
			.iter(&self.world)
			.map(|entity| self.registry.save_entity(entity, &self.world))
			// Entities with nothing to save are the engine's own bookkeeping
			// (or hold nothing): writing them would add new empty entities with
			// every save and load.
			.filter(|saved| !saved.components.is_empty())
			.collect();

		SaveWorld {
			name: self.display_name.clone(),
			world_type: self.world_type.clone(),
			version: WORLD_VERSION,
			entities,
		}
	}

	/// The snapshot as the bytes of a world file (YAML, UTF-8): what
	/// `Project::write` takes.
	///
	/// # Errors
	/// `Yaml` if a component cannot be written.
	pub fn snapshot_bytes(&mut self) -> ZResult<Vec<u8>> {
		let text = serde_saphyr::to_string(&self.snapshot()).map_err(|e| ZenithError::Yaml(e.to_string()))?;
		Ok(text.into_bytes())
	}

	/// Restores the world from the bytes of a world file (see
	/// [`Self::snapshot_bytes`]).
	///
	/// # Errors
	/// `Yaml` if the bytes are not a world file, otherwise as
	/// [`Self::restore_snapshot`].
	pub fn restore_bytes(&mut self, bytes: &[u8]) -> ZResult<()> {
		let snapshot = serde_saphyr::from_slice(bytes).map_err(|e| ZenithError::Yaml(e.to_string()))?;
		self.restore_snapshot(snapshot)
	}

	/// restores the world from a snapshot, replacing all existing entities and
	/// components.
	///
	/// # Errors
	/// This function will return an error if the snapshot is invalid or if
	/// there is an error
	#[instrument(skip(self, snapshot))]
	pub fn restore_snapshot(&mut self, snapshot: SaveWorld) -> ZResult<()> {
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

#[cfg(test)]
mod tests {
	use zenith_components::Name;

	use super::*;

	#[test]
	fn a_world_survives_being_turned_into_bytes() -> ZResult<()> {
		let mut world = World::new("world");
		world.create_entity("Test");
		let bytes = world.snapshot_bytes()?;
		assert!(std::str::from_utf8(&bytes).is_ok(), "a text file");

		let mut restored = World::new("other");
		restored.restore_bytes(&bytes)?;
		let mut query = restored.world_mut().query::<&Name>();
		let names: Vec<_> = query.iter(restored.world()).map(|n| format!("{n:?}")).collect();
		assert_eq!(names.len(), 1, "the entity came back: {names:?}");
		assert!(names[0].contains("Test"), "with its name: {names:?}");
		assert_eq!(restored.snapshot_bytes()?, bytes, "and writes the same bytes again");

		assert!(restored.restore_bytes(b"not: [a world").is_err(), "garbage is an error");
		Ok(())
	}
}
