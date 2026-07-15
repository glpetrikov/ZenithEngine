use std::collections::BTreeMap;

use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shipyard::{Component, EntityId, World};

use crate::{SavedComponent, SavedEntity};

type SaveComponentFn = Box<dyn Fn(EntityId, &World) -> Option<Value>>;
type LoadComponentFn = Box<dyn Fn(EntityId, &mut World, Value) -> Result<(), Box<dyn std::error::Error>>>;

pub struct ComponentRegistry {
	components: BTreeMap<String, ComponentRegistration>,
}

struct ComponentRegistration {
	component_type: String,
	schema: Value,
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

	pub fn register<T>(&mut self, component_type: impl Into<String>)
	where
		T: Component + Clone + Serialize + for<'de> Deserialize<'de> + JsonSchema + 'static,
	{
		let component_type = component_type.into();

		let schema = serde_json::to_value(schema_for!(T)).expect("component schema serialization failed");

		let save = Box::new(|entity: EntityId, world: &World| {
			world
				.get::<&T>(entity)
				.ok()
				.and_then(|component| serde_json::to_value(*component).ok())
		});

		let load = Box::new(
			|entity: EntityId, world: &mut World, value: Value| -> Result<(), Box<dyn std::error::Error>> {
				let component: T = serde_json::from_value(value)?;
				world.add_component(entity, (component,));
				Ok(())
			},
		);

		let registration = ComponentRegistration {
			component_type: component_type.clone(),
			schema,
			save,
			load,
			display_name: None,
		};

		self.components.insert(component_type, registration);
	}

	pub fn register_with_display_name<T>(&mut self, component_type: impl Into<String>, display_name: impl Into<String>)
	where
		T: Component + Clone + Serialize + for<'de> Deserialize<'de> + JsonSchema + 'static,
	{
		let component_type = component_type.into();

		let schema = serde_json::to_value(schema_for!(T)).expect("component schema serialization failed");

		let save = Box::new(|entity: EntityId, world: &World| {
			world
				.get::<&T>(entity)
				.ok()
				.and_then(|component| serde_json::to_value(*component).ok())
		});

		let load = Box::new(
			|entity: EntityId, world: &mut World, value: Value| -> Result<(), Box<dyn std::error::Error>> {
				let component: T = serde_json::from_value(value)?;
				world.add_component(entity, (component,));
				Ok(())
			},
		);

		let registration = ComponentRegistration {
			component_type: component_type.clone(),
			schema,
			save,
			load,
			display_name: Some(display_name.into()),
		};

		self.components.insert(component_type, registration);
	}

	pub fn register_custom(
		&mut self,
		component_type: impl Into<String>,
		schema: Value,
		save: SaveComponentFn,
		load: LoadComponentFn,
	) {
		let component_type = component_type.into();
		self.components.insert(
			component_type.clone(),
			ComponentRegistration {
				component_type,
				schema,
				save,
				load,
				display_name: None,
			},
		);
	}

	pub fn save_entity(&self, entity: EntityId, world: &World) -> SavedEntity {
		let mut components = Vec::new();

		for registration in self.components.values() {
			if let Some(value) = (registration.save)(entity, world) {
				components.push(SavedComponent {
					component_type: registration.component_type.clone(),
					value,
				});
			}
		}

		SavedEntity {
			id: entity.into(),
			components,
		}
	}

	pub fn load_component(
		&self,
		entity: EntityId,
		world: &mut World,
		component: SavedComponent,
	) -> Result<(), Box<dyn std::error::Error>> {
		let registration = self
			.components
			.get(&component.component_type)
			.ok_or_else(|| format!("unknown component type: {}", component.component_type))?;

		jsonschema::validate(&registration.schema, &component.value)
			.map_err(|err| format!("invalid component {}: {}", component.component_type, err))?;

		(registration.load)(entity, world, component.value)
	}

	pub fn component_schema(&self, type_id: &str) -> Option<&Value> {
		self.components.get(type_id).map(|component| &component.schema)
	}

	pub fn component_schemas(&self) -> BTreeMap<String, Value> {
		self.components
			.iter()
			.map(|(type_id, registration)| (type_id.clone(), registration.schema.clone()))
			.collect()
	}

	pub fn display_name(&self, type_id: &str) -> Option<&str> {
		self.components
			.get(type_id)
			.and_then(|registration| registration.display_name.as_deref())
	}
}

impl Default for ComponentRegistry {
	fn default() -> Self { Self::new() }
}
