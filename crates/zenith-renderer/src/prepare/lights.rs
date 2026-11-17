//! The lights of a view, as the shaders read them (`Lights` of
//! `standard.wgsl`).
//!
//! Positions are relative to the camera, like everything the GPU sees. There
//! is room for a few lights: the nearest point lights are kept, the first
//! directional ones.

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Vec3};

use crate::extract::ExtractedLights;

/// The most directional lights a view is lit by.
pub const MAX_DIRECTIONAL: usize = 4;
/// The most point lights a view is lit by.
pub const MAX_POINT: usize = 16;

/// A directional light on the GPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct DirectionalGpu {
	/// The way the light travels (xyz).
	pub direction: [f32; 4],
	/// Color times illuminance; w is 1 for the light that casts the shadow.
	pub color: [f32; 4],
}

/// A point light on the GPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct PointGpu {
	/// Relative to the camera; w is the range.
	pub position: [f32; 4],
	/// Color times intensity.
	pub color: [f32; 4],
}

/// The uniform buffer of the lights of a view.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LightsUniform {
	/// The ambient light that does not come from the environment (rgb).
	pub ambient: [f32; 4],
	/// The number of directional lights, then of point lights.
	pub counts: [u32; 4],
	pub directional: [DirectionalGpu; MAX_DIRECTIONAL],
	pub point: [PointGpu; MAX_POINT],
}

impl Default for LightsUniform {
	fn default() -> Self {
		Self::zeroed()
	}
}

impl LightsUniform {
	/// The lights of the frame as a camera at `camera` sees them.
	#[allow(clippy::cast_possible_truncation)] // at most 16 and 4 lights
	pub fn build(lights: &ExtractedLights, camera: DVec3, ambient: [f32; 3]) -> Self {
		let mut uniform = Self {
			ambient: [ambient[0], ambient[1], ambient[2], 0.0],
			..Self::zeroed()
		};
		let mut shadow_given = false;
		for (slot, light) in uniform.directional.iter_mut().zip(&lights.directional) {
			let cast = light.shadows && !shadow_given;
			shadow_given |= cast;
			let direction = light.direction.normalize_or_zero();
			let color = Vec3::from(light.color) * light.illuminance;
			*slot = DirectionalGpu {
				direction: [direction.x, direction.y, direction.z, 0.0],
				color: [color.x, color.y, color.z, f32::from(u8::from(cast))],
			};
		}
		uniform.counts[0] = lights.directional.len().min(MAX_DIRECTIONAL) as u32;

		// The nearest point lights, when there are more than there is room for.
		let mut points: Vec<(f64, usize)> = lights
			.point
			.iter()
			.enumerate()
			.map(|(index, light)| ((light.position - camera).length_squared(), index))
			.collect();
		points.sort_by(|a, b| a.0.total_cmp(&b.0));
		for (slot, &(_, index)) in uniform.point.iter_mut().zip(&points) {
			let light = &lights.point[index];
			let position = (light.position - camera).as_vec3();
			let color = Vec3::from(light.color) * light.intensity;
			*slot = PointGpu {
				position: [position.x, position.y, position.z, light.range],
				color: [color.x, color.y, color.z, 0.0],
			};
		}
		uniform.counts[1] = points.len().min(MAX_POINT) as u32;
		uniform
	}

	/// The way the light that casts the shadow travels.
	pub fn shadow_direction(&self) -> Option<Vec3> {
		self.directional
			.iter()
			.take(self.counts[0] as usize)
			.find(|light| light.color[3] > 0.5)
			.map(|light| Vec3::new(light.direction[0], light.direction[1], light.direction[2]))
	}
}

#[cfg(test)]
mod tests {
	use bevy_ecs::entity::Entity;

	use super::*;
	use crate::extract::{ExtractedDirectionalLight, ExtractedPointLight};

	fn sun(shadows: bool) -> ExtractedDirectionalLight {
		ExtractedDirectionalLight {
			entity: Entity::PLACEHOLDER,
			direction: Vec3::NEG_Y,
			color: [1.0, 0.5, 0.0],
			illuminance: 2.0,
			shadows,
		}
	}

	fn lamp(x: f64) -> ExtractedPointLight {
		ExtractedPointLight {
			entity: Entity::PLACEHOLDER,
			position: DVec3::new(x, 0.0, 0.0),
			color: [1.0; 3],
			intensity: 1.0,
			range: 10.0,
		}
	}

	#[test]
	fn the_first_light_that_asks_for_a_shadow_casts_it() {
		let lights = ExtractedLights {
			directional: vec![sun(false), sun(true), sun(true)],
			point: vec![],
		};
		let uniform = LightsUniform::build(&lights, DVec3::ZERO, [0.1; 3]);
		assert_eq!(uniform.counts[0], 3);
		let flags: Vec<f32> = uniform.directional.iter().map(|light| light.color[3]).collect();
		assert_eq!(flags, [0.0, 1.0, 0.0, 0.0]);
		assert_eq!(uniform.directional[0].color[..3], [2.0, 1.0, 0.0]);
		assert_eq!(uniform.shadow_direction(), Some(Vec3::NEG_Y));
	}

	#[test]
	fn point_lights_are_relative_to_the_camera_and_the_nearest_are_kept() {
		let lights = ExtractedLights {
			directional: vec![],
			point: (0..20).map(|i| lamp(100.0 - f64::from(i))).collect(),
		};
		let uniform = LightsUniform::build(&lights, DVec3::new(100.0, 0.0, 0.0), [0.0; 3]);
		assert_eq!(uniform.counts[1], 16);
		assert_eq!(uniform.point[0].position[0], 0.0, "the nearest is at the camera");
		assert_eq!(uniform.point[15].position[0], -15.0, "16 of 20, the farthest 4 dropped");
	}
}
