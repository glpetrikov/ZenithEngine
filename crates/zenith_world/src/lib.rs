pub mod world_snapshot;

use std::any::TypeId;

use bevy_ecs::{
	component::{Component, ComponentId},
	entity::Entity,
	query::Without,
	resource::{IsResource, Resource},
	schedule::Schedule,
	world::World as BevyWorld,
};
use tracing::instrument;
use zenith_components::Name;
pub use zenith_components::Time;
use zenith_error::{IntoZResult, WrapErr, ZResult};
pub use zenith_input::Input;
use zenith_registry::ComponentRegistry;
use zenith_types::ecs::WorldType;

pub const WORLD_VERSION: u32 = 1;
pub const WORLD_EXTENSION: &str = "zenith";

pub struct World {
	pub name: String,
	pub display_name: String,
	pub world_type: WorldType,
	#[allow(clippy::struct_field_names)]
	pub(crate) world: BevyWorld,
	pub(crate) registry: ComponentRegistry,
	schedule: Schedule,
	/// The resources that outlive [`World::clear_world`].
	kept: Vec<TypeId>,
	/// The resources bevy puts into every world for itself: they outlive
	/// [`World::clear_world`] too.
	builtin: Vec<ComponentId>,
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
		let builtin = world.resource_entities().iter().map(|(id, _)| id).collect();
		world.insert_resource(Input::default());
		world.insert_resource(Time::default());

		let mut this = Self {
			name: name.to_string(),
			display_name: name.to_string(),
			world_type: WorldType::World,
			world,
			registry,
			schedule: Schedule::default(),
			kept: Vec::new(),
			builtin,
		};
		// The input is the player's, not the level's: it outlives a level. So
		// does the clock.
		this.keep_resource::<Input>();
		this.keep_resource::<Time>();
		this
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
	/// The schedule of the game, which [`World::update_systems`] runs. Other
	/// schedules (the renderer's) belong to whoever runs them, on
	/// [`World::world_mut`].
	pub const fn schedule_mut(&mut self) -> &mut Schedule {
		&mut self.schedule
	}

	/// Makes the resource `T` outlive [`World::clear_world`]. For what belongs
	/// to the program and not to the level (the input, the renderer). Asking
	/// twice is the same as once.
	#[instrument(skip(self))]
	pub fn keep_resource<T: Resource>(&mut self) {
		let id = TypeId::of::<T>();
		if !self.kept.contains(&id) {
			self.kept.push(id);
		}
	}

	/// Empties the world: every entity and every resource goes, except those
	/// kept with [`World::keep_resource`].
	///
	/// The world is emptied in place, not made again: the systems of a
	/// schedule belong to the world they first ran on and cannot run on another
	/// one. This holds for the schedules others keep too.
	#[instrument(skip(self))]
	pub fn clear_world(&mut self) {
		self.world.despawn_all::<Without<IsResource>>();

		let components = self.world.components();
		let keep: Vec<ComponentId> = self
			.kept
			.iter()
			.filter_map(|id| components.get_id(*id))
			.chain(self.builtin.iter().copied())
			.collect();
		let doomed: Vec<(ComponentId, Entity)> = self
			.world
			.resource_entities()
			.iter()
			.filter(|(id, _)| !keep.contains(id))
			.collect();
		for (id, entity) in doomed {
			if let Ok(mut entity) = self.world.get_entity_mut(entity) {
				entity.remove_by_id(id);
			}
		}
		self.world.clear_non_send();

		// Whatever happened to them, a world has an input and a clock.
		self.world.init_resource::<Input>();
		self.world.init_resource::<Time>();
	}

	/// Runs the game for a frame of `dt` seconds: sets [`Time`], runs the
	/// systems, and ends the frame of the input.
	#[instrument(skip(self))]
	pub fn update_systems(&mut self, dt: f32) {
		self.world.insert_resource(Time { dt });
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

	#[derive(Resource)]
	struct Kept(u32);

	#[derive(Resource)]
	struct Level;

	#[test]
	fn kept_resources_outlive_a_level_and_the_others_do_not() {
		let mut world = World::new("w");
		world.keep_resource::<Kept>();
		world.keep_resource::<Kept>();
		world.world_mut().insert_resource(Kept(7));
		world.world_mut().insert_resource(Level);
		world.clear_world();
		assert_eq!(
			world.world().get_resource::<Kept>().map(|k| k.0),
			Some(7),
			"a kept resource is moved into the new world"
		);
		assert!(
			!world.world().contains_resource::<Level>(),
			"a resource of the level goes with it"
		);
		world.world_mut().remove_resource::<Kept>();
		world.clear_world();
		assert!(
			!world.world().contains_resource::<Kept>(),
			"a kept resource that was removed does not come back"
		);
	}

	#[derive(Resource, Default)]
	struct Drawn(u32);

	#[allow(clippy::needless_pass_by_value)] // a system takes its parameters by value
	fn draw(mut drawn: bevy_ecs::system::ResMut<Drawn>) {
		drawn.0 += 1;
	}

	#[allow(clippy::needless_pass_by_value)] // a system takes its parameters by value
	fn play(mut drawn: bevy_ecs::system::ResMut<Drawn>) {
		drawn.0 += 100;
	}

	/// A schedule of someone else's (like the renderer's, which the runtime
	/// keeps) and the schedule of the game both go on running on a cleared
	/// world.
	#[test]
	fn the_schedules_run_after_a_level_is_cleared() {
		let mut world = World::new("w");
		let mut other = Schedule::default();
		other.add_systems(draw.into_configs());
		world.keep_resource::<Drawn>();
		world.world_mut().init_resource::<Drawn>();
		world.schedule_mut().add_systems(play.into_configs());
		other.run(world.world_mut());
		world.update_systems(0.016);
		let entity = world.create_entity("gone");
		world.clear_world();
		assert!(!world.entity_exists(entity), "the entities of the level are gone");
		other.run(world.world_mut());
		world.update_systems(0.016);
		assert_eq!(
			world.world().resource::<Drawn>().0,
			202,
			"both schedules still run on the cleared world"
		);
	}

	#[derive(Resource, Default)]
	struct Clock(f32);

	#[allow(clippy::needless_pass_by_value)] // a system takes its parameters by value
	fn tick(time: Res<Time>, mut clock: bevy_ecs::system::ResMut<Clock>) {
		clock.0 += time.dt;
	}

	#[test]
	fn systems_see_the_time_of_the_frame() {
		let mut world = World::new("w");
		assert_eq!(world.world().resource::<Time>().dt, 0.0, "a new world has a clock");
		world.world_mut().init_resource::<Clock>();
		world.schedule_mut().add_systems(tick.into_configs());
		world.update_systems(0.25);
		world.update_systems(0.5);
		assert!(
			(world.world().resource::<Clock>().0 - 0.75).abs() < f32::EPSILON,
			"the systems saw both frames"
		);
		world.clear_world();
		assert!(world.world().contains_resource::<Time>(), "the clock outlives a level");
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
