pub mod camera_view;
pub mod components;
pub mod definitions;
pub mod entity;
pub mod registry;
pub mod scene;
pub mod system;
pub mod ze_entity_id;

pub use camera_view::*;
pub use components::*;
pub use definitions::*;
pub use entity::*;
pub use registry::*;
pub use scene::*;
pub use schemars::{self, JsonSchema};
pub use serde::{self, Deserialize, Serialize};
pub use shipyard::{self, Component, EntitiesView, EntityId, *};
pub use system::*;
use ze_core::{Quat, Result, Vec3};

pub fn test() -> Result<()> {
	let mut scene = Scene::new("Test Scene");

	let entity1 = scene.create_entity("Entity1");

	scene.entity_mut(entity1).add_component(Transform {
		position: Vec3::new(1.0, 1.0, 1.0),
		scale: Vec3::new(1.0, 1.0, 1.0),
		rotation: Quat::from_rotation_z(90.0_f32.to_radians()),
	});

	scene.entity_mut(entity1).add_component(Inactive);

	let entity2 = scene.create_entity("Entity2");

	scene.entity_mut(entity2).add_component(Transform {
		position: Vec3::new(-1.0, -1.0, 1.0),
		scale: Vec3::new(1.0, 1.0, 1.0),
		rotation: Quat::from_rotation_z(-90.0_f32.to_radians()),
	});

	let scene_directory = "Sandbox/assets/scenes";

	scene.save(&scene_directory.into(), "main")?;

	let loaded_scene = Scene::from_name(scene_directory.into(), "main")?;

	let name = loaded_scene.world().get::<&Name>(entity1)?;
	let transform = loaded_scene.world().get::<&Transform>(entity1)?;

	println!("{:?}", *name);
	println!("{:?}", *transform);

	Ok(())
}
