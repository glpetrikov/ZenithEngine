//! Lights, shadows and the sky in the built-in graph, drawn into a texture on
//! whatever adapter the machine has (the tests are skipped where there is
//! none).

// A test that cannot go on has to stop: panicking is how it fails.
#![allow(clippy::expect_used)]

mod common;

use std::f32::consts::FRAC_PI_4;

use common::{SIZE, Scene, id_of};
use glam::{DVec3, Quat};
use zenith_components::Transform;
use zenith_renderer::{
	assets::AssetId,
	components::{DirectionalLight, Model, PointLight},
	extract::ExtraViews,
};

fn brightness(pixel: [u8; 4]) -> u32 {
	u32::from(pixel[0]) + u32::from(pixel[1]) + u32::from(pixel[2])
}

fn center(scene: &Scene) -> [u8; 4] {
	scene.pixel(SIZE / 2, SIZE / 2)
}

#[test]
fn a_directional_light_lights_the_face_it_shines_on() {
	let Some(mut scene) = Scene::new() else {
		return;
	};
	scene.spawn_cube(DVec3::ZERO);
	scene.render_frames(8);
	let dark = brightness(center(&scene));

	// The light travels along -Z, onto the face that looks at the camera.
	scene
		.world
		.world_mut()
		.spawn((DirectionalLight::default(), Transform::default()));
	scene.render_frames(2);
	let lit = brightness(center(&scene));
	assert!(lit > dark, "lit {lit}, dark {dark}");
}

#[test]
fn a_light_from_behind_leaves_the_face_to_the_ambient_light() {
	let Some(mut scene) = Scene::new() else {
		return;
	};
	scene.spawn_cube(DVec3::ZERO);
	scene.render_frames(8);
	let dark = brightness(center(&scene));

	// Shining along +Z: onto the back of the cube.
	let behind = Transform {
		orientation: Quat::from_rotation_y(std::f32::consts::PI),
		..Transform::default()
	};
	scene.world.world_mut().spawn((DirectionalLight::default(), behind));
	scene.render_frames(2);
	assert_eq!(brightness(center(&scene)), dark);
}

#[test]
fn a_point_light_near_the_face_lights_it() {
	let Some(mut scene) = Scene::new() else {
		return;
	};
	scene.spawn_cube(DVec3::ZERO);
	scene.render_frames(8);
	let dark = brightness(center(&scene));

	scene.world.world_mut().spawn((
		PointLight {
			range: 20.0,
			intensity: 40.0,
			..PointLight::default()
		},
		Transform {
			position: DVec3::new(0.0, 0.0, 2.5),
			..Transform::default()
		},
	));
	scene.render_frames(2);
	assert!(brightness(center(&scene)) > dark);
}

/// A wall behind a cube, and a light from the left that makes the cube's
/// shadow fall on the wall to the right of it.
fn shadow_scene() -> Option<Scene> {
	let mut scene = Scene::new()?;
	scene.spawn_cube(DVec3::ZERO);
	let wall = Model {
		mesh: scene.cube,
		material: scene.material,
	};
	scene.world.world_mut().spawn((
		wall,
		Transform {
			position: DVec3::new(0.0, 0.0, -1.5),
			scale: DVec3::new(10.0, 10.0, 0.2),
			..Transform::default()
		},
	));
	let light = Transform {
		orientation: Quat::from_rotation_y(-FRAC_PI_4),
		..Transform::default()
	};
	scene.world.world_mut().spawn((
		DirectionalLight {
			shadows: false,
			..DirectionalLight::default()
		},
		light,
	));
	Some(scene)
}

fn set_shadows(scene: &mut Scene, on: bool) {
	let mut lights = scene.world.world_mut().query::<&mut DirectionalLight>();
	for mut light in lights.iter_mut(scene.world.world_mut()) {
		light.shadows = on;
	}
}

#[test]
fn a_cube_casts_a_shadow_on_the_wall_behind_it() {
	let Some(mut scene) = shadow_scene() else {
		return;
	};
	scene.render_frames(8);
	// To the right of the cube, where its shadow falls with the light on.
	let (x, y) = (SIZE * 13 / 16, SIZE / 2);
	let without = brightness(scene.pixel(x, y));

	set_shadows(&mut scene, true);
	scene.render_frames(2);
	let with = brightness(scene.pixel(x, y));
	assert!(with < without, "in the shadow {with}, in the light {without}");

	// Far from the shadow the wall is lit the same with shadows.
	let far = brightness(scene.pixel(3, SIZE / 2));
	set_shadows(&mut scene, false);
	scene.render_frames(2);
	assert_eq!(far, brightness(scene.pixel(3, SIZE / 2)));
}

#[test]
fn the_sky_is_drawn_where_nothing_else_is() {
	let Some(mut scene) = Scene::new() else {
		return;
	};
	scene.spawn_cube(DVec3::ZERO);
	scene.render_frames(8);
	let background = scene.pixel(2, 2);
	let cube = center(&scene);

	let sky: AssetId = id_of(
		scene.world.world().resource::<zenith_renderer::assets::AssetRegistry>(),
		"Assets://Sky/pure-sky.zenithtexture",
	);
	scene.world.world_mut().resource_mut::<ExtraViews>().0[0].environment = Some((sky, 1.0));
	// The picture arrives over a few frames.
	scene.render_frames(8);
	assert_ne!(scene.pixel(2, 2), background, "the sky instead of the clear color");
	assert_eq!(scene.pixel(2, 2)[3], 255);
	assert_ne!(center(&scene), scene.pixel(2, 2), "the cube is still in front of it");
	assert_ne!(cube, [0; 4]);
}
