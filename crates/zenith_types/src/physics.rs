use crate::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RigidBodyType {
	Static,
	Dynamic,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum CollisionDetection {
	#[default]
	Discrete,
	Continuous,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PhysicsMaterial {
	pub friction: f32,
	pub friction_combine_rule: CombineMode,
	pub restitution: f32,
	pub restitution_combine_rule: CombineMode,
}

// NOTE: If a dependency on rapier2d/3d appears, replace it with the built-in
// rapier type
/// GeometricMean > ClampedSum > Max > Multiply > Min > Average
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum CombineMode {
	Average,
	Min,
	Multiply,
	Max,
	ClampedSum,
	GeometricMean,
}
