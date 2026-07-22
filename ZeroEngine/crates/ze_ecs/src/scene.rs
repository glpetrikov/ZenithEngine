use std::{
	fs,
	path::{Path, PathBuf},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shipyard::{Component, EntitiesView, EntityId, World};
use ze_core::{Result, Vec3, anyhow};

use crate::{
	components::{
		Animator, AudioSource, Children, Collider, EditorOnly, Inactive, Name, Parent, PhysicsSettings, RigidBody, Tag,
		TileGridPosition, TileWorldSettings, TilesRoot, Transform, TransformInheritance,
	},
	definitions::{SaveFile, SceneType},
	entity::Entity,
	registry::ComponentRegistry,
	system::System,
};

pub const SCENE_VERSION: &str = "0.1.0";
pub const SCENE_EXTENSION: &str = "zescene.json";

pub struct Scene {
	pub name: String,
	pub display_name: String,
	pub scene_type: SceneType,
	pub(crate) world: World,
	pub(crate) registry: ComponentRegistry,
	systems: Vec<Box<dyn System>>,
}

impl Scene {
	pub fn new(name: &str) -> Self {
		let mut registry = ComponentRegistry::new();
		Self::register_defaults(&mut registry);

		Self {
			name: name.to_string(),
			display_name: name.to_string(),
			scene_type: SceneType::Scene,
			world: World::new(),
			registry,
			systems: Vec::new(),
		}
	}

	pub fn from_registry(name: &str, registry: ComponentRegistry) -> Self {
		Self {
			name: name.to_string(),
			display_name: name.to_string(),
			scene_type: SceneType::Scene,
			world: World::new(),
			registry,
			systems: Vec::new(),
		}
	}
}

impl Scene {
	pub const fn world(&self) -> &World { &self.world }

	pub const fn world_mut(&mut self) -> &mut World { &mut self.world }

	pub const fn registry(&self) -> &ComponentRegistry { &self.registry }

	pub const fn registry_mut(&mut self) -> &mut ComponentRegistry { &mut self.registry }

	pub fn clear_world(&mut self) { self.world = World::new(); }
}

impl Scene {
	pub fn add_system<T>(&mut self, system: T)
	where
		T: System + 'static,
	{
		self.systems.push(Box::new(system));
	}

	pub fn update_systems(&mut self, dt: f32) -> Result<()> {
		let mut systems = std::mem::take(&mut self.systems);
		let mut result = Ok(());

		for system in &mut systems {
			if let Err(error) = system.update(self, dt) {
				result = Err(error);
				break;
			}
		}

		self.systems = systems;
		result
	}

	pub fn update_system<T>(&mut self, dt: f32) -> Result<bool>
	where
		T: System + 'static,
	{
		let mut systems = std::mem::take(&mut self.systems);
		let mut result = Ok(false);

		for system in &mut systems {
			let Some(system) = system.as_any_mut().downcast_mut::<T>() else {
				continue;
			};

			result = system.update(self, dt).map(|()| true);
			break;
		}

		self.systems = systems;
		result
	}

	pub fn with_system_mut<T, R>(&mut self, f: impl FnOnce(&mut T, &Self) -> R) -> Option<R>
	where
		T: System + 'static,
	{
		let mut systems = std::mem::take(&mut self.systems);
		let mut f = Some(f);
		let mut output = None;

		for system in &mut systems {
			let Some(system) = system.as_any_mut().downcast_mut::<T>() else {
				continue;
			};

			output = Some(f.take().expect("system callback was already consumed")(system, self));
			break;
		}

		self.systems = systems;
		output
	}
}

impl Scene {
	pub fn register_component<T>(&mut self, type_id: &str)
	where
		T: Component + Clone + Serialize + for<'de> Deserialize<'de> + JsonSchema + 'static,
	{
		self.registry.register::<T>(type_id);
	}

	pub fn register_defaults(registry: &mut ComponentRegistry) {
		registry.register::<Name>("ze.name");
		registry.register::<Tag>("ze.tag");
		registry.register::<Parent>("ze.parent");
		registry.register::<TransformInheritance>("ze.transform_inheritance");
		registry.register::<Children>("ze.children");
		registry.register::<Inactive>("ze.inactive");
		registry.register::<EditorOnly>("ze.editor.only");
		registry.register::<Transform>("ze.transform");
		registry.register::<RigidBody>("ze.physics_2d.rigidbody");
		registry.register::<Collider>("ze.physics_2d.collider");
		registry.register::<PhysicsSettings>("ze.physics_2d.settings");
		registry.register::<AudioSource>("ze.audio.source");
		registry.register::<Animator>("ze.anim.animator");
		registry.register::<TileGridPosition>("ze.tile.grid_position");
		registry.register::<TileWorldSettings>("ze.tile.settings");
		registry.register::<TilesRoot>("ze.tile.root");
	}
}

impl Scene {
	pub fn create_entity(&mut self, name: &str) -> EntityId {
		self.world.add_entity((Name { name: name.to_string() },))
	}

	pub fn destroy_entity(&mut self, entity: EntityId) {
		let mut entities = self.entity_tree_for_delete(entity);
		entities.reverse();
		for entity in entities {
			self.cleanup_entity_links_for_delete(entity);
			self.world.delete_entity(entity);
		}
	}

	/// Spawns a new entity that is a component-for-component copy of
	/// `template`.
	///
	/// Every registered component on the template is serialized and re-loaded
	/// onto the fresh entity, so the clone starts life with identical component
	/// values. Hierarchy links (`Parent`/`Children`) are deliberately skipped
	/// so the clone is a free-standing root rather than silently sharing the
	/// template's parent or being registered as a child that nothing points to.
	///
	/// Returns `None` when the template has no serializable components (e.g. it
	/// was already destroyed), so callers can distinguish a real clone from a
	/// no-op.
	pub fn clone_entity(&mut self, template: EntityId) -> Option<EntityId> {
		let saved = self.registry.save_entity(template, &self.world);
		if saved.components.is_empty() {
			return None;
		}

		let new_entity = self.world.add_entity((Name {
			name: "Entity".to_string(),
		},));
		for component in saved.components {
			if component.component_type == "ze.parent" || component.component_type == "ze.children" {
				continue;
			}
			// Best-effort: a component that fails to round-trip just doesn't make
			// it onto the clone rather than aborting the whole copy.
			let _ = self.registry.load_component(new_entity, &mut self.world, component);
		}

		Some(new_entity)
	}

	pub fn descendant_entities(&self, root: EntityId) -> Vec<EntityId> {
		let mut entities = vec![root];
		let mut index = 0usize;

		while let Some(parent) = entities.get(index).copied() {
			index += 1;
			for child in self.children_of(parent) {
				if !entities.contains(&child) {
					entities.push(child);
				}
			}
		}

		entities
	}

	fn entity_tree_for_delete(&self, root: EntityId) -> Vec<EntityId> { self.descendant_entities(root) }

	fn children_of(&self, parent: EntityId) -> Vec<EntityId> {
		let mut children = Vec::new();

		if let Ok(component) = self.world.get::<&Children>(parent) {
			for id in &component.ids {
				let child = EntityId::from(*id);
				if !children.contains(&child) {
					children.push(child);
				}
			}
		}

		let all_entities = self
			.world
			.run(|entities: EntitiesView| entities.iter().collect::<Vec<_>>());
		for candidate in all_entities {
			let is_child = self
				.world
				.get::<&Parent>(candidate)
				.is_ok_and(|component| EntityId::from(component.id) == parent);
			if is_child && !children.contains(&candidate) {
				children.push(candidate);
			}
		}

		children
	}

	fn cleanup_entity_links_for_delete(&mut self, entity: EntityId) {
		let all_entities = self
			.world
			.run(|entities: EntitiesView| entities.iter().collect::<Vec<_>>());

		for candidate in all_entities.iter().copied() {
			if candidate == entity {
				continue;
			}

			let parent_points_to_deleted = self
				.world
				.get::<&Parent>(candidate)
				.is_ok_and(|component| EntityId::from(component.id) == entity);
			if parent_points_to_deleted {
				let _ = self.world.remove::<(Parent,)>(candidate);
			}

			self.remove_child_reference(candidate, entity);
		}
	}

	fn remove_child_reference(&mut self, parent: EntityId, child: EntityId) {
		let remove_children_component = if let Ok(mut children_component) = self.world.get::<&mut Children>(parent) {
			children_component.ids.retain(|id| EntityId::from(*id) != child);
			children_component.ids.is_empty()
		} else {
			false
		};

		if remove_children_component {
			let _ = self.world.remove::<(Children,)>(parent);
		}
	}

	pub const fn entity_mut(&mut self, entity: EntityId) -> Entity<'_> { Entity::new(entity, self) }

	pub(crate) fn add_component<T>(&mut self, entity: EntityId, component: T)
	where
		T: Component + Clone + Serialize + for<'de> Deserialize<'de> + JsonSchema + 'static,
	{
		self.world.add_component(entity, (component,));
	}

	pub fn world_transform(&self, entity: EntityId) -> Option<Transform> {
		let mut visited = Vec::new();
		self.world_transform_inner(entity, &mut visited)
	}

	pub fn set_world_transform(&mut self, entity: EntityId, world_transform: Transform) -> Result<()> {
		let local_transform = self
			.local_transform_from_world(entity, world_transform)
			.ok_or_else(|| anyhow!("failed to resolve local transform for entity {}", entity.index()))?;

		let mut current = self.world.get::<&mut Transform>(entity)?;
		**current = local_transform;
		Ok(())
	}
}

impl Scene {
	pub fn snapshot(&self) -> SaveFile {
		self.world.run(|entities: EntitiesView| SaveFile {
			version: SCENE_VERSION.to_string(),
			name: self.display_name.clone(),
			scene_type: self.scene_type,
			entities: entities
				.iter()
				.map(|entity| self.registry.save_entity(entity, &self.world))
				.collect(),
		})
	}

	pub fn restore_snapshot(&mut self, snapshot: SaveFile) -> Result<()> {
		self.display_name = if snapshot.name.is_empty() {
			self.name.clone()
		} else {
			snapshot.name.clone()
		};
		self.scene_type = snapshot.scene_type;
		self.clear_world();
		let mut world = World::new();

		for saved_entity in &snapshot.entities {
			let entity = EntityId::from(saved_entity.id);
			world.spawn(entity);
		}

		for saved_entity in snapshot.entities {
			let entity = EntityId::from(saved_entity.id);

			for component in saved_entity.components {
				self.registry
					.load_component(entity, &mut world, component)
					.map_err(|err| anyhow!("Cannot restore scene snapshot: {err:?}"))?;
			}
		}

		self.world = world;
		Ok(())
	}

	pub fn from_name(directory: PathBuf, name: &str) -> Result<Self> {
		let mut registry = ComponentRegistry::new();
		Self::register_defaults(&mut registry);
		Self::from_name_with_registry(directory, name, registry)
	}

	pub fn from_name_with_registry(directory: PathBuf, name: &str, registry: ComponentRegistry) -> Result<Self> {
		let mut local_registry = registry; // ?
		Self::register_defaults(&mut local_registry);
		let path = scene_path(directory, name);
		Self::from_path_with_registry(&path, local_registry)
	}

	pub fn from_path(path: &PathBuf) -> Result<Self> {
		let registry = ComponentRegistry::new();
		Self::from_path_with_registry(path, registry)
	}

	pub fn from_path_with_registry(path: &PathBuf, registry: ComponentRegistry) -> Result<Self> {
		let json_text = fs::read_to_string(path)?;
		let json_value: serde_json::Value = serde_json::from_str(&json_text)?;

		let schema = Self::schema_value();

		jsonschema::validate(&schema, &json_value).map_err(|err| anyhow!("Invalid scene file: {err}"))?;

		let save_file: SaveFile = serde_json::from_value(json_value)?;

		let name = path
			.file_name()
			.and_then(|name| name.to_str())
			.and_then(|name| name.strip_suffix(".zescene.json"))
			.unwrap_or("")
			.to_string();
		let display_name = if save_file.name.is_empty() {
			name.clone()
		} else {
			save_file.name.clone()
		};
		let scene_type = save_file.scene_type;

		let mut world = World::new();

		for saved_entity in &save_file.entities {
			let entity = EntityId::from(saved_entity.id);
			world.spawn(entity);
		}

		for saved_entity in save_file.entities {
			let entity = EntityId::from(saved_entity.id);

			for component in saved_entity.components {
				registry
					.load_component(entity, &mut world, component)
					.map_err(|err| anyhow!("Cannot load scene: {err:?}"))?;
			}
		}

		Ok(Self {
			name,
			display_name,
			scene_type,
			world,
			registry,
			systems: Vec::new(),
		})
	}

	pub fn save(&self, directory: &PathBuf, file_name: &str) -> Result<()> {
		fs::create_dir_all(directory)?;

		let path = scene_path(directory, file_name);

		let save_file = self.snapshot();
		let json = serde_json::to_string_pretty(&save_file)?;
		fs::write(path, json)?;

		Ok(())
	}

	pub fn schema_value() -> serde_json::Value {
		serde_json::to_value(schemars::schema_for!(SaveFile)).expect("scene schema serialization failed")
	}
}

impl Scene {
	fn world_transform_inner(&self, entity: EntityId, visited: &mut Vec<EntityId>) -> Option<Transform> {
		if visited.contains(&entity) {
			return None;
		}
		visited.push(entity);

		let local = self.world.get::<&Transform>(entity).ok()?.clone();
		let Some(parent) = self.parent_entity(entity) else {
			visited.pop();
			return Some(local);
		};

		let parent_world = self.world_transform_inner(parent, visited)?;
		visited.pop();
		Some(compose_transform(&parent_world, &local, inheritance_for(self, entity)))
	}

	fn local_transform_from_world(&self, entity: EntityId, world: Transform) -> Option<Transform> {
		let Some(parent) = self.parent_entity(entity) else {
			return Some(world);
		};

		let parent_world = self.world_transform(parent)?;
		let inheritance = inheritance_for(self, entity);
		Some(decompose_transform(&parent_world, &world, inheritance))
	}

	fn parent_entity(&self, entity: EntityId) -> Option<EntityId> {
		let parent = self.world.get::<&Parent>(entity).ok()?;
		let parent = EntityId::from(parent.id);
		self.world.get::<&Transform>(parent).ok()?;
		Some(parent)
	}
}

fn scene_path(directory: impl AsRef<Path>, name: &str) -> PathBuf {
	directory.as_ref().join(format!("{name}.zescene.json"))
}

fn inheritance_for(scene: &Scene, entity: EntityId) -> TransformInheritance {
	scene
		.world()
		.get::<&TransformInheritance>(entity)
		.ok()
		.map(|inheritance| **inheritance)
		.unwrap_or_default()
}

fn compose_transform(parent_world: &Transform, local: &Transform, inheritance: TransformInheritance) -> Transform {
	let mut position = local.position;
	if inheritance.inherit_scale {
		position *= parent_world.scale;
	}
	if inheritance.inherit_rotation {
		position = parent_world.rotation * position;
	}
	if inheritance.inherit_position {
		position += parent_world.position;
	}

	let rotation = if inheritance.inherit_rotation {
		parent_world.rotation * local.rotation
	} else {
		local.rotation
	};

	let scale = if inheritance.inherit_scale {
		parent_world.scale * local.scale
	} else {
		local.scale
	};

	Transform {
		position,
		scale,
		rotation,
	}
}

fn decompose_transform(parent_world: &Transform, world: &Transform, inheritance: TransformInheritance) -> Transform {
	let mut position = world.position;
	if inheritance.inherit_position {
		position -= parent_world.position;
	}
	if inheritance.inherit_rotation {
		position = parent_world.rotation.conjugate() * position;
	}
	if inheritance.inherit_scale {
		position *= safe_inverse_scale(parent_world.scale);
	}

	let rotation = if inheritance.inherit_rotation {
		parent_world.rotation.conjugate() * world.rotation
	} else {
		world.rotation
	};

	let scale = if inheritance.inherit_scale {
		world.scale * safe_inverse_scale(parent_world.scale)
	} else {
		world.scale
	};

	Transform {
		position,
		scale,
		rotation,
	}
}

fn safe_inverse_scale(scale: Vec3) -> Vec3 {
	Vec3::new(
		1.0 / scale.x.abs().max(f32::EPSILON).copysign(scale.x),
		1.0 / scale.y.abs().max(f32::EPSILON).copysign(scale.y),
		1.0 / scale.z.abs().max(f32::EPSILON).copysign(scale.z),
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ze_entity_id::ZeEntityId;

	fn parent_child_scene() -> (Scene, EntityId, EntityId) {
		let mut scene = Scene::new("Parent Child Test");
		let parent = scene.create_entity("Parent");
		let child = scene.create_entity("Child");

		scene.world_mut().add_component(
			child,
			(Parent {
				id: ZeEntityId::from(parent),
			},),
		);
		scene.world_mut().add_component(
			parent,
			(Children {
				ids: vec![ZeEntityId::from(child)],
			},),
		);

		(scene, parent, child)
	}

	#[test]
	fn clone_entity_copies_components_onto_a_new_entity() {
		let mut scene = Scene::new("Clone Test");
		let template = scene.create_entity("Template");
		scene.world_mut().add_component(
			template,
			(Transform {
				position: Vec3::new(3.0, 4.0, 0.0),
				..Transform::default()
			},),
		);

		let clone = scene.clone_entity(template).expect("template has components to clone");

		assert_ne!(clone, template);
		let cloned_transform = scene.world().get::<&Transform>(clone).expect("clone has a transform");
		assert!((cloned_transform.position.x - 3.0).abs() < f32::EPSILON);
		assert!((cloned_transform.position.y - 4.0).abs() < f32::EPSILON);
		// Original stays intact.
		assert!(scene.world().get::<&Name>(template).is_ok());
	}

	#[test]
	fn clone_entity_skips_parent_and_children_links() {
		let (mut scene, parent, child) = parent_child_scene();

		let clone = scene.clone_entity(child).expect("child has components to clone");

		assert!(scene.world().get::<&Parent>(clone).is_err());
		assert!(scene.world().get::<&Children>(parent).is_ok());
	}

	#[test]
	fn deleting_parent_deletes_child() {
		let (mut scene, parent, child) = parent_child_scene();

		scene.destroy_entity(parent);

		assert!(scene.world().get::<&Name>(parent).is_err());
		assert!(scene.world().get::<&Name>(child).is_err());
	}

	#[test]
	fn deleting_parent_deletes_descendants() {
		let (mut scene, parent, child) = parent_child_scene();
		let grandchild = scene.create_entity("Grandchild");
		scene.world_mut().add_component(
			grandchild,
			(Parent {
				id: ZeEntityId::from(child),
			},),
		);
		scene.world_mut().add_component(
			child,
			(Children {
				ids: vec![ZeEntityId::from(grandchild)],
			},),
		);

		scene.destroy_entity(parent);

		assert!(scene.world().get::<&Name>(parent).is_err());
		assert!(scene.world().get::<&Name>(child).is_err());
		assert!(scene.world().get::<&Name>(grandchild).is_err());
	}

	#[test]
	fn deleting_child_removes_parent_children_link() {
		let (mut scene, parent, child) = parent_child_scene();

		scene.destroy_entity(child);

		assert!(scene.world().get::<&Children>(parent).is_err());
	}

	#[test]
	fn snapshot_includes_scene_metadata() {
		let mut scene = Scene::new("internal_name");
		scene.display_name = "Display Name".to_string();
		scene.scene_type = SceneType::Prefab;

		let snapshot = scene.snapshot();

		assert_eq!(snapshot.name, "Display Name");
		assert_eq!(snapshot.scene_type, SceneType::Prefab);
	}

	#[test]
	fn legacy_snapshot_defaults_scene_metadata() {
		let snapshot = serde_json::json!({
			"version": SCENE_VERSION,
			"entities": []
		});

		let schema = Scene::schema_value();
		jsonschema::validate(&schema, &snapshot).expect("legacy scene metadata should be optional in schema");
		let save_file: SaveFile = serde_json::from_value(snapshot).expect("legacy scene metadata should be optional");

		assert_eq!(save_file.name, "");
		assert_eq!(save_file.scene_type, SceneType::Scene);
	}
}
