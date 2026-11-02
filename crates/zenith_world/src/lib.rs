pub mod world_snapshot;

use bevy_ecs::{
	component::Component, entity::Entity, resource::Resource, schedule::Schedule, world::World as BevyWorld,
};
use tracing::instrument;
use zenith_components::Name;
use zenith_error::{IntoZResult, WrapErr, ZResult};
pub use zenith_input::Input;
use zenith_registry::ComponentRegistry;
use zenith_types::ecs::WorldType;

pub const WORLD_VERSION: u32 = 1;
pub const WORLD_EXTENSION: &str = "zenith";

#[derive(Resource, Clone, Copy)]
pub struct Time {
	pub delta: f32,
}

pub struct World {
	pub name: String,
	pub display_name: String,
	pub world_type: WorldType,
	#[allow(clippy::struct_field_names)]
	pub(crate) world: BevyWorld,
	pub(crate) registry: ComponentRegistry,
	schedule: Schedule,
}

impl World {
	#[instrument]
	pub fn new(name: &str) -> Self {
		let mut registry = ComponentRegistry::new();
		ComponentRegistry::register_defaults(&mut registry);
		Self::from_registry(name, registry)
	}

	#[instrument(skip(registry))]
	pub fn from_registry(name: &str, registry: ComponentRegistry) -> Self {
		let mut world = BevyWorld::new();
		world.insert_resource(Input::default());

		Self {
			name: name.to_string(),
			display_name: name.to_string(),
			world_type: WorldType::World,
			world,
			registry,
			schedule: Schedule::default(),
		}
	}
}

impl World {
	pub const fn world(&self) -> &BevyWorld {
		&self.world
	}
	pub const fn world_mut(&mut self) -> &mut BevyWorld {
		&mut self.world
	}
	pub const fn registry(&self) -> &ComponentRegistry {
		&self.registry
	}
	pub const fn registry_mut(&mut self) -> &mut ComponentRegistry {
		&mut self.registry
	}
	pub const fn schedule_mut(&mut self) -> &mut Schedule {
		&mut self.schedule
	}

	#[instrument(skip(self))]
	pub fn clear_world(&mut self) {
		// The input is the player's, not the level's: it outlives a level.
		let input = self.world.remove_resource::<Input>().unwrap_or_default();
		self.world = BevyWorld::new();
		self.world.insert_resource(input);
	}

	#[instrument(skip(self))]
	pub fn update_systems(&mut self, dt: f32) {
		self.world.insert_resource(Time { delta: dt });
		self.schedule.run(&mut self.world);
		// What only lasts a frame (new presses, mouse movement) is over now
		// that the systems have seen it.
		self.input_mut().end_frame(dt);
	}

	/// The input of the world: keyboard, mouse, gamepads and actions. Systems
	/// get it as `Res<Input>`.
	///
	/// # Panics
	/// If the resource was removed through [`Self::world_mut`].
	#[must_use]
	pub fn input(&self) -> &Input {
		self.world.resource::<Input>()
	}

	/// The input of the world, to feed it events.
	///
	/// # Panics
	/// If the resource was removed through [`Self::world_mut`].
	#[must_use]
	pub fn input_mut(&mut self) -> bevy_ecs::change_detection::Mut<'_, Input> {
		self.world.resource_mut::<Input>()
	}
}

impl World {
	#[instrument(skip(self))]
	pub fn create_entity(&mut self, name: &str) -> Entity {
		self.world.spawn(Name { name: name.to_string() }).id()
	}

	/// Destroys an entity and all of its components.
	///
	/// # Errors
	/// This function will return an error if the entity does not exist.
	#[instrument(skip(self))]
	pub fn destroy_entity(&mut self, entity: Entity) -> ZResult<()> {
		self.world
			.get_entity_mut(entity)
			.wrap_err_with(|| format!("cannot destroy entity {entity:?}: it doesn't exist"))
			.into_zresult()?
			.despawn();
		Ok(())
	}

	#[instrument(skip(self))]
	pub fn entity_exists(&self, entity: Entity) -> bool {
		self.world.entities().contains(entity)
	}

	#[instrument(skip(self))]
	pub fn clone_entity(&mut self, template: Entity) -> Option<Entity> {
		let saved = self.registry.save_entity(template, &self.world);
		if saved.components.is_empty() {
			return None;
		}

		let new_entity = self
			.world
			.spawn(Name {
				name: "Entity".to_string(),
			})
			.id();
		for component in saved.components {
			let _ = self.registry.load_component(new_entity, &mut self.world, component);
		}

		Some(new_entity)
	}

	/// Adds a component to an entity.
	///
	/// # Errors
	/// This function will return an error if the entity does not exist.
	#[instrument(skip(self, component))]
	pub fn add_component<T>(&mut self, entity: Entity, component: T) -> ZResult<()>
	where
		T: Component,
	{
		self.world
			.get_entity_mut(entity)
			.wrap_err_with(|| format!("cannot add component to {entity:?}: it doesn't exist"))
			.into_zresult()?
			.insert(component);
		Ok(())
	}

	/// Removes a component from an entity.
	///
	/// # Errors
	/// This function will return an error if the entity does not exist.
	#[instrument(skip(self))]
	pub fn remove_component<T>(&mut self, entity: Entity) -> ZResult<()>
	where
		T: Component,
	{
		self.world
			.get_entity_mut(entity)
			.wrap_err_with(|| format!("cannot remove component from {entity:?}: it doesn't exist"))
			.into_zresult()?
			.remove::<T>();
		Ok(())
	}

	/// Removes a component from an entity and returns it, if it exists.
	///
	/// # Errors
	/// This function will return an error if the entity does not exist.
	#[instrument(skip(self))]
	pub fn take_component<T>(&mut self, entity: Entity) -> ZResult<Option<T>>
	where
		T: Component,
	{
		Ok(self
			.world
			.get_entity_mut(entity)
			.wrap_err_with(|| format!("cannot take component from {entity:?}: it doesn't exist"))
			.into_zresult()?
			.take::<T>())
	}

	#[instrument(skip(self))]
	pub fn get_component<T>(&self, entity: Entity) -> Option<&T>
	where
		T: Component,
	{
		self.world.get::<T>(entity)
	}

	#[instrument(skip(self))]
	pub fn get_component_mut<T>(&mut self, entity: Entity) -> Option<bevy_ecs::change_detection::Mut<'_, T>>
	where
		T: Component<Mutability = bevy_ecs::component::Mutable>,
	{
		self.world.get_mut::<T>(entity)
	}

	#[instrument(skip(self))]
	pub fn has_component<T>(&self, entity: Entity) -> bool
	where
		T: Component,
	{
		self.world.get::<T>(entity).is_some()
	}
}

#[cfg(test)]
mod tests {
	use bevy_ecs::{prelude::IntoScheduleConfigs, system::Res};
	use zenith_input::{InputEvent, ZKeyCode};

	use super::*;

	#[derive(Resource, Default)]
	struct Seen {
		just_pressed: bool,
	}

	#[allow(clippy::needless_pass_by_value)] // a system takes its parameters by value
	fn watch(input: Res<Input>, mut seen: bevy_ecs::system::ResMut<Seen>) {
		seen.just_pressed = input.is_key_just_pressed(ZKeyCode::Space);
	}

	#[test]
	fn systems_see_the_input_and_the_frame_ends_after_them() {
		let mut world = World::new("w");
		world.world_mut().init_resource::<Seen>();
		world.schedule_mut().add_systems(watch.into_configs());

		world.input_mut().apply(InputEvent::KeyDown(ZKeyCode::Space));
		world.update_systems(0.016);
		assert!(
			world.world().resource::<Seen>().just_pressed,
			"the system sees the press of this frame"
		);
		assert!(
			world.input().is_key_pressed(ZKeyCode::Space),
			"and the key is still held"
		);
		assert!(
			!world.input().is_key_just_pressed(ZKeyCode::Space),
			"but the press is over"
		);
		world.update_systems(0.016);
		assert!(
			!world.world().resource::<Seen>().just_pressed,
			"the next frame sees no press"
		);
	}

	#[test]
	fn the_input_outlives_a_level() {
		let mut world = World::new("w");
		world.input_mut().apply(InputEvent::KeyDown(ZKeyCode::W));
		world.clear_world();
		assert!(
			world.input().is_key_pressed(ZKeyCode::W),
			"clearing the world keeps the input"
		);
	}
}
