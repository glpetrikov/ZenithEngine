//! 2D: sprites drawn by an orthographic camera in the built-in graph, into a
//! texture, on whatever adapter the machine has (skipped where there is none).

// A test that cannot go on has to stop: panicking is how it fails.
#![allow(clippy::expect_used)]

mod common;

use common::{SIZE, Scene, id_of};
use glam::DVec3;
use zenith_components::Transform;
use zenith_renderer::{
	assets::{AssetId, AssetRegistry},
	components::{Projection, Sprite},
	extract::ExtraViews,
};

/// A scene seen from z = 10 by a camera that shows 4 units of height.
fn scene_2d() -> Option<(Scene, AssetId)> {
	let scene = Scene::new()?;
	let texture = id_of(
		scene.world.world().resource::<AssetRegistry>(),
		"Assets://Models/Cube/cube-diffuse.zenithtexture",
	);
	let mut scene = scene;
	{
		let mut extra = scene.world.world_mut().resource_mut::<ExtraViews>();
		extra.0[0].position = DVec3::new(0.0, 0.0, 10.0);
		extra.0[0].projection = Projection::Orthographic {
			height: 4.0,
			near: 0.1,
			far: 100.0,
		};
	}
	Some((scene, texture))
}

const fn sprite(texture: AssetId, color: [f32; 4], order: i32, size: f32) -> Sprite {
	Sprite {
		texture,
		size: [size, size],
		color,
		uv_rect: None,
		order,
	}
}

#[test]
fn a_sprite_is_drawn_where_it_stands_and_nowhere_else() {
	let Some((mut scene, texture)) = scene_2d() else {
		return;
	};
	scene.render_frames(3);
	let background = scene.pixel(2, 2);

	// 2 by 2 units of the 4 by 4 the camera sees: the middle of the picture.
	scene
		.world
		.world_mut()
		.spawn((sprite(texture, [1.0; 4], 0, 2.0), Transform::default()));
	// The texture arrives over a few frames.
	scene.render_frames(30);
	assert_ne!(scene.pixel(SIZE / 2, SIZE / 2), background, "in the middle");
	assert_ne!(scene.pixel(SIZE * 5 / 8, SIZE * 3 / 8), background, "inside the quad");
	assert_eq!(scene.pixel(2, 2), background, "the corner is not covered");
	assert_eq!(scene.pixel(SIZE - 3, SIZE - 3), background);
}

#[test]
fn a_sprite_that_is_moved_moves_on_the_screen() {
	let Some((mut scene, texture)) = scene_2d() else {
		return;
	};
	let entity = scene
		.world
		.world_mut()
		.spawn((
			sprite(texture, [1.0; 4], 0, 1.0),
			Transform {
				position: DVec3::new(-1.0, 0.0, 0.0),
				..Transform::default()
			},
		))
		.id();
	scene.render_frames(30);
	let background = scene.pixel(SIZE - 3, SIZE / 2);
	assert_ne!(scene.pixel(SIZE / 4, SIZE / 2), background, "on the left");
	assert_eq!(scene.pixel(SIZE * 3 / 4, SIZE / 2), background, "not on the right");

	scene
		.world
		.world_mut()
		.entity_mut(entity)
		.get_mut::<Transform>()
		.expect("it has a transform")
		.position
		.x = 1.0;
	scene.render_frames(2);
	assert_eq!(scene.pixel(SIZE / 4, SIZE / 2), background, "gone from the left");
	assert_ne!(scene.pixel(SIZE * 3 / 4, SIZE / 2), background, "on the right now");
}

#[test]
fn the_sprite_with_the_higher_order_is_on_top() {
	let Some((mut scene, texture)) = scene_2d() else {
		return;
	};
	let red = [1.0, 0.0, 0.0, 1.0];
	let blue = [0.0, 0.0, 1.0, 1.0];
	// At one depth: only the order tells which is on top.
	let first = scene
		.world
		.world_mut()
		.spawn((sprite(texture, red, 0, 2.0), Transform::default()))
		.id();
	scene
		.world
		.world_mut()
		.spawn((sprite(texture, blue, 1, 2.0), Transform::default()));
	scene.render_frames(30);
	let [r, _, b, _] = scene.pixel(SIZE / 2, SIZE / 2);
	assert!(b > r, "blue on top: {r} {b}");

	let mut order = |entity: bevy_ecs::entity::Entity, value: i32| {
		scene
			.world
			.world_mut()
			.entity_mut(entity)
			.get_mut::<Sprite>()
			.expect("it is a sprite")
			.order = value;
	};
	order(first, 2);
	scene.render_frames(2);
	let [r, _, b, _] = scene.pixel(SIZE / 2, SIZE / 2);
	assert!(r > b, "red on top now: {r} {b}");
}

#[test]
fn a_part_of_the_texture_can_be_cut_out() {
	let Some((mut scene, texture)) = scene_2d() else {
		return;
	};
	let whole = scene
		.world
		.world_mut()
		.spawn((sprite(texture, [1.0; 4], 0, 2.0), Transform::default()))
		.id();
	scene.render_frames(30);
	let before = scene.pixel(SIZE / 2, SIZE / 2);
	let mut sprite = scene.world.world_mut().entity_mut(whole);
	let mut sprite = sprite.get_mut::<Sprite>().expect("it is a sprite");
	// A tiny part of a corner of the texture, stretched over the quad.
	sprite.uv_rect = Some([0.0, 0.0, 0.01, 0.01]);
	scene.render_frames(2);
	assert_ne!(scene.pixel(SIZE / 2, SIZE / 2), before);
}
