use bevy_reflect::Reflect;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use shipyard::Component;
use ze_core::{Quat, Vec2, Vec3};

use crate::ze_entity_id::ZeEntityId;

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Name {
	pub name: String,
}

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Tag {
	pub tag: String,
}

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Transform {
	#[schemars(with = "[f32; 3]")]
	pub position: Vec3,

	#[schemars(with = "[f32; 3]")]
	pub scale: Vec3,

	#[schemars(with = "[f32; 4]")]
	pub rotation: Quat,
}

impl Default for Transform {
	fn default() -> Self {
		Self {
			position: Vec3::ZERO,
			scale: Vec3::ONE,
			rotation: Quat::IDENTITY,
		}
	}
}

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Parent {
	#[schemars(with = "(u64, u16)")]
	pub id: ZeEntityId,
}

#[derive(Component, Reflect, Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct TransformInheritance {
	#[serde(default = "default_true")]
	pub inherit_position: bool,
	#[serde(default = "default_true")]
	pub inherit_rotation: bool,
	#[serde(default = "default_true")]
	pub inherit_scale: bool,
}

impl Default for TransformInheritance {
	fn default() -> Self {
		Self {
			inherit_position: true,
			inherit_rotation: true,
			inherit_scale: true,
		}
	}
}

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Children {
	#[schemars(with = "Vec<(u64, u16)>")]
	pub ids: Vec<ZeEntityId>,
}

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Inactive;

#[derive(Component, Reflect, Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct EditorOnly;

#[allow(clippy::struct_excessive_bools)]
#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RigidBody {
	pub body_type: RigidBodyType,
	#[serde(default = "default_true")]
	pub use_gravity: bool,
	pub gravity_scale: f32,
	pub linear_damping: f32,
	pub angular_damping: f32,
	#[serde(default)]
	pub mass: Option<f32>,
	#[serde(default)]
	pub freeze_position_x: bool,
	#[serde(default)]
	pub freeze_position_y: bool,
	#[serde(default)]
	pub freeze_rotation_x: bool,
	#[serde(default)]
	pub freeze_rotation_y: bool,
	#[serde(default)]
	pub freeze_rotation_z: bool,
	#[serde(default)]
	pub collision_detection: CollisionDetection,
}

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PhysicsSettings {
	#[schemars(with = "[f32; 2]")]
	pub gravity: Vec2,
	pub enable_debug_draw: bool,
	#[serde(default = "default_physics_timestep")]
	pub physics_timestep: f32,
}

impl Default for PhysicsSettings {
	fn default() -> Self {
		Self {
			gravity: Vec2::new(0.0, -9.81),
			enable_debug_draw: false,
			physics_timestep: default_physics_timestep(),
		}
	}
}

fn default_physics_timestep() -> f32 { 1.0 / 70.0 }

impl Default for RigidBody {
	fn default() -> Self {
		Self {
			body_type: RigidBodyType::Dynamic,
			use_gravity: true,
			gravity_scale: 1.0,
			linear_damping: 0.05,
			angular_damping: 0.0,
			mass: None,
			freeze_position_x: false,
			freeze_position_y: false,
			freeze_rotation_x: false,
			freeze_rotation_y: false,
			freeze_rotation_z: false,
			collision_detection: CollisionDetection::Discrete,
		}
	}
}

const fn default_true() -> bool { true }

#[derive(Reflect, Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub enum RigidBodyType {
	Static,
	Dynamic,
	KinematicPositionBased,
	KinematicVelocityBased,
}

#[derive(Reflect, Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum CollisionDetection {
	#[default]
	Discrete,
	Continuous,
}

#[derive(Component, Reflect, Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Collider {
	pub shape: ColliderShape,
	pub friction: f32,
	pub restitution: f32,
	pub density: f32,
	pub is_sensor: bool,
}

impl Default for Collider {
	fn default() -> Self {
		Self {
			shape: ColliderShape::Box {
				half_extents: Vec2::splat(0.5),
			},
			friction: 0.5,
			restitution: 0.0,
			density: 1.0,
			is_sensor: false,
		}
	}
}

#[derive(Reflect, Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum ColliderShape {
	Box {
		#[schemars(with = "[f32; 2]")]
		half_extents: Vec2,
	},
	Circle {
		radius: f32,
	},
	Capsule {
		half_height: f32,
		radius: f32,
	},
	ConvexPolygon {
		#[schemars(with = "Vec<[f32; 2]>")]
		points: Vec<Vec2>,
	},
}
