//! The built-in graph (input camera, input meshes, opaque, tonemap, output)
//! draws a world into a texture, on whatever adapter the machine has. The
//! tests are skipped where there is no adapter at all.

// A test that cannot go on has to stop: panicking is how it fails.
#![allow(clippy::expect_used)]

mod common;

use common::{SIZE, Scene};
use glam::{DVec3, Quat};
use zenith_renderer::{
	Renderer,
	assets::AssetId,
	components::{ClearMode, Projection},
	extract::{ExtraViews, ExtractedView, ViewSource, ViewTarget},
	targets::RenderTargets,
};

#[test]
fn the_built_in_graph_compiles_with_the_passes_of_the_engine() {
	let Some(scene) = Scene::new() else {
		return;
	};
	let renderer = scene.world.world().resource::<Renderer>();
	let entry = renderer
		.graphs()
		.get(zenith_renderer::graph::GraphId::BUILTIN)
		.expect("the built-in graph is there");
	assert!(entry.compiled.is_some(), "{:?}", entry.diagnostics);
	assert_eq!(entry.diagnostics, [], "no warnings");
}

#[test]
fn an_empty_world_shows_the_color_the_scene_is_cleared_to() {
	let Some(mut scene) = Scene::new() else {
		return;
	};
	scene.render_frames(2);
	let [r, g, b, a] = scene.pixel(2, 2);
	assert_eq!(a, 255);
	// The blue of the graph (0.1, 0.2, 0.5), through the tone mapping.
	assert!(b > g && g > r, "blue: {r} {g} {b}");
}

#[test]
fn a_cube_is_drawn_over_the_background() {
	let Some(mut scene) = Scene::new() else {
		return;
	};
	scene.spawn_cube(DVec3::ZERO);
	// The mesh, the material and its textures arrive over a few frames.
	scene.render_frames(8);
	let background = scene.pixel(2, 2);
	let center = scene.pixel(SIZE / 2, SIZE / 2);
	assert_ne!(center, background, "the cube is in the middle");
	assert_eq!(scene.pixel(SIZE - 3, SIZE - 3), background, "and not in the corner");
}

#[test]
fn a_cube_behind_the_camera_is_not_seen() {
	let Some(mut scene) = Scene::new() else {
		return;
	};
	scene.spawn_cube(DVec3::new(0.0, 0.0, 10.0));
	scene.render_frames(8);
	assert_eq!(scene.pixel(SIZE / 2, SIZE / 2), scene.pixel(2, 2));
}

fn view(number: u32, target: ViewTarget, order: i32, viewport: Option<[f32; 4]>, clear: ClearMode) -> ExtractedView {
	ExtractedView {
		source: ViewSource::Host(number),
		position: DVec3::new(0.0, 0.0, 4.0),
		rotation: Quat::IDENTITY,
		projection: Projection::default(),
		target,
		viewport,
		order,
		clear,
		graph: None,
		environment: None,
	}
}

#[test]
fn two_cameras_share_a_target_and_a_third_draws_into_a_texture() {
	let Some(mut scene) = Scene::new() else {
		return;
	};
	scene.world.world_mut().resource_mut::<ExtraViews>().0.clear();
	scene.spawn_cube(DVec3::ZERO);
	let red = ClearMode::Color([1.0, 0.0, 0.0, 1.0]);
	let green = ClearMode::Color([0.0, 1.0, 0.0, 1.0]);
	let texture = AssetId::from_bytes([7; 16]);
	{
		let target = scene.target;
		let mut extra = scene.world.world_mut().resource_mut::<ExtraViews>();
		// The left half is cleared red by its camera, the right half green.
		extra
			.0
			.push(view(1, ViewTarget::Target(target), 0, Some([0.0, 0.0, 0.5, 1.0]), red));
		extra.0.push(view(
			2,
			ViewTarget::Target(target),
			1,
			Some([0.5, 0.0, 0.5, 1.0]),
			green,
		));
		// The third camera has a texture of its own, with a different size.
		extra.0.push(view(
			3,
			ViewTarget::Texture {
				id: texture,
				size: (32, 32),
			},
			2,
			None,
			ClearMode::Color([0.0, 0.0, 1.0, 1.0]),
		));
	}
	scene.render_frames(8);
	// Each camera draws the cube into the middle of its own half; the corners
	// of the halves show the background.
	let background = scene.pixel(2, 2);
	assert_ne!(scene.pixel(SIZE / 4, SIZE / 2), background, "the cube in the left half");
	assert_ne!(
		scene.pixel(SIZE * 3 / 4, SIZE / 2),
		background,
		"the cube in the right half"
	);
	assert_eq!(scene.pixel(SIZE - 3, 2), background);
	let id = scene
		.world
		.world()
		.resource::<RenderTargets>()
		.texture_target(texture)
		.expect("the texture has a target");
	let size = scene
		.world
		.world()
		.resource::<RenderTargets>()
		.get(id)
		.expect("it exists")
		.size;
	assert_eq!(size, (32, 32));
	// The cube is drawn into the texture too: the middle is not the corner.
	scene.target = id;
	assert_ne!(scene.pixel(16, 16), scene.pixel(1, 1));
}
