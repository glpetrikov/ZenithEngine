//! The graph of the editor: the grid, the gizmos and the outline are drawn
//! over the scene, and the renderer can be moved from one world to another.
//! Skipped where there is no adapter.

// A test that cannot go on has to stop: panicking is how it fails.
#![allow(clippy::expect_used)]

mod common;

use common::Scene;
use glam::DVec3;
use zenith_renderer::{
	graph::GraphId,
	install_again,
	passes::editor::{EditorSelection, GizmoLines},
	uninstall,
};

fn different(a: &[[u8; 4]], b: &[[u8; 4]]) -> usize {
	a.iter().zip(b).filter(|(a, b)| a != b).count()
}

/// A scene with a cube, seen from above by the built-in graph, and its image.
fn plain() -> Option<(Scene, Vec<[u8; 4]>)> {
	let mut scene = Scene::new()?;
	scene.spawn_cube(DVec3::new(0.0, 0.5, 0.0));
	scene.look_from_above(None);
	scene.render_frames(30);
	let image = scene.image();
	Some((scene, image))
}

#[test]
fn the_editor_graph_draws_a_grid_under_the_scene() {
	let Some((mut scene, plain)) = plain() else {
		return;
	};
	scene.look_from_above(Some(GraphId::EDITOR));
	scene.render_frames(2);
	let with_grid = scene.image();
	assert!(different(&plain, &with_grid) > 50, "the lines of the grid are there");
	// The cube hides the lines behind it.
	let middle = (common::SIZE * common::SIZE / 2 + common::SIZE / 2) as usize;
	assert_eq!(plain[middle], with_grid[middle], "the cube is in front of the grid");
}

#[test]
fn a_selected_model_gets_a_box_of_its_own_color() {
	let Some((mut scene, _)) = plain() else {
		return;
	};
	scene.look_from_above(Some(GraphId::EDITOR));
	scene.render_frames(2);
	let without = scene.image();

	let entity = {
		let world = scene.world.world_mut();
		let mut meshes = world.query::<(bevy_ecs::entity::Entity, &zenith_renderer::components::Model)>();
		meshes.iter(world).next().expect("the cube").0
	};
	scene.world.world_mut().resource_mut::<EditorSelection>().0.push(entity);
	scene.render_frames(2);
	let with = scene.image();
	assert!(different(&without, &with) > 10, "the edges of the box");
	assert!(
		with.iter().any(|[r, g, b, _]| *r > 150 && *r > *g && *g > *b),
		"orange pixels"
	);
}

#[test]
fn lines_of_the_editor_are_drawn_where_it_puts_them() {
	let Some((mut scene, _)) = plain() else {
		return;
	};
	scene.look_from_above(Some(GraphId::EDITOR));
	scene.render_frames(2);
	let without = scene.image();
	scene.world.world_mut().resource_mut::<GizmoLines>().line(
		DVec3::new(-3.0, 1.5, 1.0),
		DVec3::new(3.0, 1.5, 1.0),
		[0.0, 1.0, 0.0, 1.0],
	);
	scene.render_frames(2);
	let with = scene.image();
	assert!(different(&without, &with) >= 10, "a line across the picture");
	assert!(
		with.iter().any(|[r, g, b, _]| *g > 150 && *r < 100 && *b < 100),
		"green pixels"
	);
}

#[test]
fn the_renderer_moves_to_another_world_with_its_cache() {
	let Some((mut scene, before)) = plain() else {
		return;
	};
	let installed = uninstall(&mut scene.world).expect("it was installed");
	assert!(uninstall(&mut scene.world).is_none(), "taken out");
	install_again(&mut scene.world, installed);
	scene.render_frames(2);
	assert_eq!(scene.image(), before, "the same picture: the assets stayed loaded");
}
