use bevy_ecs::component::Component;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

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
	pub position: Vec3,
	pub orientation: Quat,
	pub scale: Vec3,
}

impl Default for Transform {
	fn default() -> Self {
		Self {
			position: Vec3::ZERO,
			orientation: Quat::IDENTITY,
			scale: Vec3::ONE,
		}
	}
}

// TODO: add anywhere "enabled" flag
