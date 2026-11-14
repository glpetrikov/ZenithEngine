use bevy_ecs::{component::Component, resource::Resource};
use glam::{DVec3, Quat};
use serde::{Deserialize, Serialize};

/// The time of the frame. The world sets it before its systems run; a system
/// reads it as `Res<Time>`.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Time {
	/// Seconds since the frame before.
	pub dt: f32,
}

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Name {
	pub name: String,
}

#[derive(Component, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct IsActive {
	active: bool,
}

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ExcludeFromBuild;

#[derive(Component, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Transform {
	pub position: DVec3,
	pub orientation: Quat,
	pub scale: DVec3,
}

impl Default for Transform {
	fn default() -> Self {
		Self {
			position: DVec3::ZERO,
			orientation: Quat::IDENTITY,
			scale: DVec3::ONE,
		}
	}
}

// TODO: add anywhere "enabled" flag
