use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};

#[derive(Component, Serialize, Deserialize, Default, Clone)]
pub struct Camera {
	pub projection: glam::Mat4,
}

pub enum PowerMode {
	LowPerformance,
	HighPerformance,
}

pub struct SurfaceFormats {
	pub storage: wgpu::TextureFormat,
	pub view: wgpu::TextureFormat,
}
