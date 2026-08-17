use std::collections::BTreeMap;

use bevy_ecs::{component, entity::Entity, world};
use serde_json::Value;
use tracing::instrument;
use ze_types::{
	Deserialize, Serialize,
	ecs::{SavedComponent, SavedEntity},
};

type SaveComponentFn = Box<dyn Fn(Entity, &world::World) -> Option<Value>>;
type LoadComponentFn = Box<dyn Fn(Entity, &mut world::World, Value) -> Result<(), Box<dyn std::error::Error>>>;

pub struct ComponentRegistry {
	components: BTreeMap<String, ComponentRegistration>,
}

struct ComponentRegistration {
	component_type: String,
	component_version: ze_types::Version,
	save: SaveComponentFn,
	load: LoadComponentFn,
	display_name: Option<String>,
}

impl ComponentRegistry {
	pub const fn new() -> Self {
		Self {
			components: BTreeMap::new(),
		}
	}

	#[instrument(skip(self))]
	pub fn register<T>(
		&mut self,
		component_type: impl Into<String> + std::fmt::Debug,
		component_version: ze_types::Version,
	) where
		T: component::Component + Clone + Serialize + for<'de> Deserialize<'de> + 'static,
	{
		self.register_inner::<T>(component_type.into(), component_version, None);
	}

	#[instrument(skip(self))]
	pub fn register_with_display_name<T>(
		&mut self,
		component_type: impl Into<String> + std::fmt::Debug,
		component_version: ze_types::Version,
		display_name: impl Into<String> + std::fmt::Debug,
	) where
		T: component::Component + Clone + Serialize + for<'de> Deserialize<'de> + 'static,
	{
		self.register_inner::<T>(component_type.into(), component_version, Some(display_name.into()));
	}

	#[instrument(skip(self))]
	fn register_inner<T>(
		&mut self,
		component_type: String,
		component_version: ze_types::Version,
		display_name: Option<String>,
	) where
		T: component::Component + Clone + Serialize + for<'de> Deserialize<'de> + 'static,
	{
		let save: SaveComponentFn = Box::new(|entity: Entity, world: &world::World| {
			world
				.get::<T>(entity)
				.and_then(|component| serde_json::to_value(component).ok())
		});

		let load: LoadComponentFn = Box::new(
			|entity: Entity, world: &mut world::World, value: Value| -> Result<(), Box<dyn std::error::Error>> {
				let component: T = serde_json::from_value(value)?;
				world.entity_mut(entity).insert(component);
				Ok(())
			},
		);

		self.components.insert(
			component_type.clone(),
			ComponentRegistration {
				component_type,
				component_version,
				save,
				load,
				display_name,
			},
		);
	}

	#[instrument(skip(self, save, load))]
	pub fn register_custom(
		&mut self,
		component_type: impl Into<String> + std::fmt::Debug,
		component_version: ze_types::Version,
		save: SaveComponentFn,
		load: LoadComponentFn,
	) {
		let component_type = component_type.into();
		self.components.insert(
			component_type.clone(),
			ComponentRegistration {
				component_type,
				component_version,
				save,
				load,
				display_name: None,
			},
		);
	}

	#[instrument(skip(self, world))]
	pub fn save_entity(&self, entity: Entity, world: &world::World) -> SavedEntity {
		let components = self
			.components
			.values()
			.filter_map(|registration| {
				(registration.save)(entity, world).map(|value| SavedComponent {
					component_type: registration.component_type.clone(),
					component_version: registration.component_version.clone(),
					value,
				})
			})
			.collect();

		SavedEntity {
			id: entity.into(),
			components,
		}
	}

	#[instrument(skip(self, world, component), fields(component_type = %component.component_type))]
	pub fn load_component(
		&self,
		entity: Entity,
		world: &mut world::World,
		component: SavedComponent,
	) -> Result<(), Box<dyn std::error::Error>> {
		let registration = self
			.components
			.get(&component.component_type)
			.ok_or_else(|| format!("unknown component type: {}", component.component_type))?;

		(registration.load)(entity, world, component.value)
	}

	#[instrument(skip(self))]
	pub fn display_name(&self, type_id: &str) -> Option<&str> {
		self.components
			.get(type_id)
			.and_then(|registration| registration.display_name.as_deref())
	}

	#[instrument(skip(self))]
	pub fn is_registered(&self, type_id: &str) -> bool { self.components.contains_key(type_id) }

	pub fn registered_types(&self) -> impl Iterator<Item = &str> { self.components.keys().map(String::as_str) }
}

impl Default for ComponentRegistry {
	fn default() -> Self { Self::new() }
}
