//! The renderer without a window, on whatever adapter the machine has (a
//! software one, like lavapipe, will do). The tests are skipped where there is
//! no adapter at all.

// A test that cannot go on has to stop: panicking is how it fails.
#![allow(clippy::expect_used)]

use bevy_ecs::schedule::Schedule;
use zenith_renderer::{
	ClearColor, RenderSettings, Renderer, install,
	settings::{AlphaMode, Backend, PresentMode},
	wgpu,
};
use zenith_world::World;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
	let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
	let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
	pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
		label: Some("test device"),
		required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
		..Default::default()
	}))
	.ok()
}

/// A world with a renderer, and the render schedule that draws it.
fn world_with_renderer() -> Option<(World, Schedule)> {
	let Some((device, queue)) = device() else {
		eprintln!("no adapter: skipped");
		return None;
	};
	let mut world = World::new("w");
	install(
		&mut world,
		Renderer::from_device(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb),
		RenderSettings::default(),
	);
	Some((world, zenith_renderer::schedule()))
}

#[test]
fn a_renderer_without_a_surface_counts_frames_and_draws_nothing() {
	let Some((mut world, mut render)) = world_with_renderer() else {
		return;
	};
	render.run(world.world_mut());
	render.run(world.world_mut());
	let renderer = world.world().resource::<Renderer>();
	assert_eq!(renderer.frame(), 2, "every run of the render schedule is a frame");
	assert_eq!(renderer.surface_size(), None, "it was given no surface");
	assert_eq!(renderer.output_format(), wgpu::TextureFormat::Rgba8UnormSrgb);
}

#[test]
fn the_renderer_outlives_a_level() {
	let Some((mut world, mut render)) = world_with_renderer() else {
		return;
	};
	world.world_mut().resource_mut::<ClearColor>().0 = wgpu::Color::RED;
	render.run(world.world_mut());
	world.clear_world();
	assert!(world.world().contains_resource::<Renderer>(), "the renderer is kept");
	assert!(
		world.world().contains_resource::<RenderSettings>(),
		"so are its settings"
	);
	assert_eq!(
		world.world().resource::<ClearColor>().0,
		wgpu::Color::RED,
		"and the clear color"
	);
	render.run(world.world_mut());
	assert_eq!(
		world.world().resource::<Renderer>().frame(),
		2,
		"the schedule of the host still draws the new level"
	);
}

#[test]
fn a_change_of_the_settings_reaches_the_renderer() {
	let Some((mut world, mut render)) = world_with_renderer() else {
		return;
	};
	render.run(world.world_mut());
	let changed = RenderSettings {
		present_mode: PresentMode::Immediate,
		alpha_mode: AlphaMode::Transparent,
		backend: Backend::Vulkan,
		..RenderSettings::default()
	};
	*world.world_mut().resource_mut::<RenderSettings>() = changed.clone();
	render.run(world.world_mut());
	assert_eq!(world.world().resource::<Renderer>().settings(), &changed);
}

#[test]
fn the_render_systems_wait_for_a_renderer() {
	let mut world = World::new("w");
	// On the web the renderer comes after the world: until then, nothing runs.
	zenith_renderer::schedule().run(world.world_mut());
}
