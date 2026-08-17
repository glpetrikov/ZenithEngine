pub mod ecs;
// pub mod physics;

pub use glam::{
	BVec4, Quat, Vec2, Vec3,
	bool::{BVec2, BVec3},
};
pub use semver::{BuildMetadata, Prerelease, Version};
pub use serde::{Deserialize, Deserializer, Serialize, Serializer}; // Maybe this should be deleted.

// for serde
pub const fn default_true() -> bool { true }
pub const fn default_false() -> bool { false }
