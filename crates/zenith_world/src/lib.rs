pub mod world_io;

use bevy_ecs::{
	component::Component, entity::Entity, resource::Resource, schedule::Schedule, world::World as BevyWorld,
};
use tracing::instrument;
use zenith_components::{ExcludeFromBuild, IsActive, Name, Transform};
use zenith_error::{IntoPubResult, WrapErr, ZPubResult};
use zenith_registry::ComponentRegistry;
use zenith_types::ecs::WorldType;

pub const WORLD_VERSION: &str = "0.1.0";
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
		Self::register_defaults(&mut registry);
		Self::from_registry(name, registry)
	}

	#[instrument(skip(registry))]
	pub fn from_registry(name: &str, registry: ComponentRegistry) -> Self {
		let world = BevyWorld::new();

		Self {
			name: name.to_string(),
			display_name: name.to_string(),
			world_type: WorldType::World,
			world,
			registry,
			schedule: Schedule::default(),
		}
	}

	#[instrument(skip(registry))]
	pub fn register_defaults(registry: &mut ComponentRegistry) {
		registry.register::<Name>(
			"zenith_engine.name",
			zenith_types::Version {
				major: 1,
				minor: 0,
				patch: 0,
				pre: zenith_types::Prerelease::EMPTY,
				build: zenith_types::BuildMetadata::EMPTY,
			},
		);
		registry.register::<IsActive>(
			"zenith_engine.is_active",
			zenith_types::Version {
				major: 1,
				minor: 0,
				patch: 0,
				pre: zenith_types::Prerelease::EMPTY,
				build: zenith_types::BuildMetadata::EMPTY,
			},
		);
		registry.register::<ExcludeFromBuild>(
			"zenith_engine.exclude_from_build",
			zenith_types::Version {
				major: 1,
				minor: 0,
				patch: 0,
				pre: zenith_types::Prerelease::EMPTY,
				build: zenith_types::BuildMetadata::EMPTY,
			},
		);
		registry.register::<Transform>(
			"zenith_engine.transform.v1",
			zenith_types::Version {
				major: 1,
				minor: 1,
				patch: 0,
				pre: zenith_types::Prerelease::EMPTY,
				build: zenith_types::BuildMetadata::EMPTY,
			},
		);
	}
}

impl World {
	pub const fn world(&self) -> &BevyWorld { &self.world }
	pub const fn world_mut(&mut self) -> &mut BevyWorld { &mut self.world }
	pub const fn registry(&self) -> &ComponentRegistry { &self.registry }
	pub const fn registry_mut(&mut self) -> &mut ComponentRegistry { &mut self.registry }
	pub const fn schedule_mut(&mut self) -> &mut Schedule { &mut self.schedule }

	#[instrument(skip(self))]
	pub fn clear_world(&mut self) { self.world = BevyWorld::new(); }

	#[instrument(skip(self))]
	pub fn update_systems(&mut self, dt: f32) {
		self.world.insert_resource(Time { delta: dt });
		self.schedule.run(&mut self.world);
	}
}

impl World {
	#[instrument(skip(self))]
	pub fn create_entity(&mut self, name: &str) -> Entity { self.world.spawn(Name { name: name.to_string() }).id() }

	#[instrument(skip(self))]
	pub fn destroy_entity(&mut self, entity: Entity) -> ZPubResult<()> {
		self.world
			.get_entity_mut(entity)
			.wrap_err_with(|| format!("cannot destroy entity {entity:?}: it doesn't exist"))
			.into_pub_result()?
			.despawn();
		Ok(())
	}

	#[instrument(skip(self))]
	pub fn entity_exists(&self, entity: Entity) -> bool { self.world.entities().contains(entity) }

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

	#[instrument(skip(self, component))]
	pub fn add_component<T>(&mut self, entity: Entity, component: T) -> ZPubResult<()>
	where
		T: Component,
	{
		self.world
			.get_entity_mut(entity)
			.wrap_err_with(|| format!("cannot add component to {entity:?}: it doesn't exist"))
			.into_pub_result()?
			.insert(component);
		Ok(())
	}

	#[instrument(skip(self))]
	pub fn remove_component<T>(&mut self, entity: Entity) -> ZPubResult<()>
	where
		T: Component,
	{
		self.world
			.get_entity_mut(entity)
			.wrap_err_with(|| format!("cannot remove component from {entity:?}: it doesn't exist"))
			.into_pub_result()?
			.remove::<T>();
		Ok(())
	}

	#[instrument(skip(self))]
	pub fn take_component<T>(&mut self, entity: Entity) -> ZPubResult<Option<T>>
	where
		T: Component,
	{
		Ok(self
			.world
			.get_entity_mut(entity)
			.wrap_err_with(|| format!("cannot take component from {entity:?}: it doesn't exist"))
			.into_pub_result()?
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
