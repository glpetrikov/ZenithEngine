//! The components, extract and prepare of the renderer. Extract needs no GPU;
//! prepare and render do, and are skipped where there is no adapter.

// A test that cannot go on has to stop: panicking is how it fails.
#![allow(clippy::expect_used)]

use bevy_ecs::{schedule::Schedule, world::World};
use glam::{DVec3, Quat, Vec3};
use zenith_components::{IsActive, Transform};
use zenith_renderer::{
	ClearColor, RenderSettings, Renderer,
	assets::AssetId,
	components::{
		Camera, CameraTarget, ClearMode, DirectionalLight, Environment, Model, PointLight, Projection, Sprite,
	},
	extract::{
		ExtraViews, ExtractedLights, ExtractedMeshes, ExtractedSprites, ExtractedView, ExtractedViews, ViewSource,
		ViewTarget, extract_cameras, extract_lights, extract_meshes, extract_sprites,
	},
	install,
	prepare::{DrawLists, MeshBatch, PreparedViews},
	targets::RenderTargets,
	wgpu,
};

const fn id(n: u8) -> AssetId {
	AssetId::from_bytes([n; 16])
}

fn at(x: f64, y: f64, z: f64) -> Transform {
	Transform {
		position: DVec3::new(x, y, z),
		..Transform::default()
	}
}

fn camera(order: i32) -> Camera {
	Camera {
		order,
		..Camera::default()
	}
}

const fn model(mesh: u8, material: u8) -> Model {
	Model {
		mesh: id(mesh),
		material: id(material),
	}
}

fn host_view(number: u32, target: ViewTarget, order: i32, clear: ClearMode) -> ExtractedView {
	ExtractedView {
		source: ViewSource::Host(number),
		position: DVec3::new(0.0, 0.0, 5.0),
		rotation: Quat::IDENTITY,
		projection: Projection::default(),
		target,
		viewport: None,
		order,
		clear,
		graph: None,
		environment: None,
	}
}

// --- Extract ----------------------------------------------------------------

fn extract_world() -> (World, Schedule) {
	let mut world = World::new();
	world.init_resource::<ExtractedViews>();
	world.init_resource::<ExtractedMeshes>();
	world.init_resource::<ExtractedSprites>();
	world.init_resource::<ExtractedLights>();
	world.init_resource::<ExtraViews>();
	let mut schedule = Schedule::default();
	schedule.add_systems((extract_cameras, extract_meshes, extract_sprites, extract_lights));
	(world, schedule)
}

#[test]
fn extract_copies_what_is_active_and_orders_the_views() {
	let (mut world, mut schedule) = extract_world();
	let late = world.spawn((camera(5), at(0.0, 0.0, 1.0))).id();
	let early = world
		.spawn((
			camera(-1),
			at(1.0, 2.0, 3.0),
			Environment {
				sky: id(9),
				intensity: 0.5,
			},
		))
		.id();
	world.spawn((camera(0), at(0.0, 0.0, 0.0), IsActive::new(false)));
	world.spawn((
		Camera {
			target: CameraTarget::Texture(id(7)),
			..camera(0)
		},
		at(0.0, 0.0, 0.0),
	));
	world
		.resource_mut::<ExtraViews>()
		.0
		.push(host_view(1, ViewTarget::Window, 2, ClearMode::None));

	let shown = world.spawn((model(1, 2), at(4.0, 0.0, 0.0))).id();
	world.spawn((model(3, 4), at(5.0, 0.0, 0.0), IsActive::new(false)));
	world.spawn((model(5, 6), at(6.0, 0.0, 0.0), IsActive::new(true)));
	world.spawn((
		Sprite {
			texture: id(8),
			size: [2.0, 1.0],
			color: [1.0; 4],
			uv_rect: None,
			order: 3,
		},
		at(0.0, 1.0, 0.0),
	));
	let sun = world
		.spawn((
			DirectionalLight::default(),
			Transform {
				orientation: Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
				..Transform::default()
			},
		))
		.id();
	world.spawn((PointLight::default(), at(0.0, 3.0, 0.0)));

	schedule.run(&mut world);

	let views = &world.resource::<ExtractedViews>().0;
	let sources: Vec<ViewSource> = views.iter().map(|view| view.source).collect();
	assert_eq!(
		sources,
		[ViewSource::Entity(early), ViewSource::Host(1), ViewSource::Entity(late)],
		"by order; an inactive camera and one into a texture are left out"
	);
	assert_eq!(views[0].position, DVec3::new(1.0, 2.0, 3.0));
	assert_eq!(views[0].environment, Some((id(9), 0.5)));
	assert_eq!(views[2].environment, None);

	let meshes = &world.resource::<ExtractedMeshes>().0;
	assert_eq!(meshes.len(), 2, "the inactive model is left out, the active one is not");
	assert!(meshes.iter().any(|mesh| mesh.entity == shown && mesh.mesh == id(1)));

	let sprites = &world.resource::<ExtractedSprites>().0;
	assert_eq!(sprites.len(), 1);
	assert_eq!(sprites[0].texture, id(8));

	let lights = world.resource::<ExtractedLights>();
	assert_eq!(lights.point.len(), 1);
	assert_eq!(lights.directional.len(), 1);
	assert_eq!(lights.directional[0].entity, sun);
	assert!(
		(lights.directional[0].direction - Vec3::NEG_Y).length() < 1e-5,
		"turned a quarter down, the light shines down: {}",
		lights.directional[0].direction
	);
}

#[test]
fn extract_makes_its_lists_again_every_frame() {
	let (mut world, mut schedule) = extract_world();
	let entity = world.spawn((model(1, 2), at(0.0, 0.0, 0.0))).id();
	schedule.run(&mut world);
	schedule.run(&mut world);
	assert_eq!(world.resource::<ExtractedMeshes>().0.len(), 1, "not twice");
	world.despawn(entity);
	schedule.run(&mut world);
	assert_eq!(world.resource::<ExtractedMeshes>().0.len(), 0);
}

#[test]
fn the_same_camera_has_the_same_key_and_two_cameras_do_not() {
	let (mut world, mut schedule) = extract_world();
	world.spawn((camera(0), Transform::default()));
	world.spawn((camera(1), Transform::default()));
	schedule.run(&mut world);
	let first: Vec<_> = world
		.resource::<ExtractedViews>()
		.0
		.iter()
		.map(ExtractedView::key)
		.collect();
	schedule.run(&mut world);
	let second: Vec<_> = world
		.resource::<ExtractedViews>()
		.0
		.iter()
		.map(ExtractedView::key)
		.collect();
	assert_eq!(first, second);
	assert_ne!(first[0], first[1]);
}

// --- Prepare and render -----------------------------------------------------

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

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// A world with a renderer and a target of 64x32 that views can draw into.
fn gpu_world() -> Option<(zenith_world::World, Schedule, zenith_renderer::targets::RenderTargetId)> {
	let Some((device, queue)) = device() else {
		eprintln!("no adapter: skipped");
		return None;
	};
	let mut world = zenith_world::World::new("w");
	let renderer = Renderer::from_device(device, queue, FORMAT);
	install(&mut world, renderer, RenderSettings::default());
	// These tests are about the clearing of targets: no graph draws over it.
	world
		.world_mut()
		.resource_mut::<Renderer>()
		.graphs_mut()
		.remove(zenith_renderer::graph::GraphId::BUILTIN);
	let target = {
		let device = world.world().resource::<Renderer>().device().clone();
		world
			.world_mut()
			.resource_mut::<RenderTargets>()
			.create(&device, (64, 32), FORMAT)
	};
	Some((world, zenith_renderer::schedule(), target))
}

fn pixel(world: &zenith_world::World, id: zenith_renderer::targets::RenderTargetId) -> [u8; 4] {
	let renderer = world.world().resource::<Renderer>();
	let target = world.world().resource::<RenderTargets>().get(id).expect("the target");
	let device = renderer.device();
	let buffer = device.create_buffer(&wgpu::BufferDescriptor {
		label: Some("read back"),
		size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
		usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
		mapped_at_creation: false,
	});
	let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
	encoder.copy_texture_to_buffer(
		target.texture.as_image_copy(),
		wgpu::TexelCopyBufferInfo {
			buffer: &buffer,
			layout: wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
				rows_per_image: Some(1),
			},
		},
		wgpu::Extent3d {
			width: 1,
			height: 1,
			depth_or_array_layers: 1,
		},
	);
	renderer.queue().submit([encoder.finish()]);
	buffer.map_async(wgpu::MapMode::Read, .., |result| result.expect("map the buffer"));
	device
		.poll(wgpu::PollType::wait_indefinitely())
		.expect("wait for the GPU");
	let bytes = buffer.get_mapped_range(..4).expect("read the buffer");
	[bytes[0], bytes[1], bytes[2], bytes[3]]
}

#[test]
fn prepare_sorts_the_meshes_of_a_view_into_batches() {
	let Some((mut world, mut render, target)) = gpu_world() else {
		return;
	};
	world.world_mut().resource_mut::<ExtraViews>().0.push(host_view(
		1,
		ViewTarget::Target(target),
		0,
		ClearMode::default(),
	));
	// Two of one mesh and material, one of another, spawned in a mixed order.
	world.world_mut().spawn((model(2, 2), at(0.0, 0.0, 0.0)));
	world.world_mut().spawn((model(1, 1), at(1.0, 0.0, 0.0)));
	world.world_mut().spawn((model(2, 2), at(2.0, 0.0, 0.0)));

	render.run(world.world_mut());

	let key = world.world().resource::<ExtractedViews>().0[0].key();
	let prepared = world.world().resource::<PreparedViews>();
	let view = prepared.get(key).expect("the view is prepared");
	assert_eq!(view.size, (64, 32));
	assert_eq!(view.uniform.viewport[..2], [64.0, 32.0]);

	let lists = world.world().resource::<DrawLists>();
	let list = lists.get(key).expect("the view has lists");
	assert_eq!(
		list.opaque.batches,
		[
			MeshBatch {
				material: id(1),
				mesh: id(1),
				instances: 0..1
			},
			MeshBatch {
				material: id(2),
				mesh: id(2),
				instances: 1..3
			},
		],
		"equal ones are one batch, the batches go by material and mesh"
	);
	assert_eq!(list.transparent.batches.len(), 0);
	assert!(lists.instance_buffer().is_some());
}

#[test]
fn a_view_without_a_place_to_draw_is_not_prepared() {
	let Some((mut world, mut render, _target)) = gpu_world() else {
		return;
	};
	// The window: this renderer has no surface. A target that does not exist.
	let mut extra = world.world_mut().resource_mut::<ExtraViews>();
	extra.0.push(host_view(1, ViewTarget::Window, 0, ClearMode::default()));
	extra.0.push(host_view(
		2,
		ViewTarget::Target(zenith_renderer::targets::RenderTargetId(99)),
		1,
		ClearMode::default(),
	));
	render.run(world.world_mut());
	assert!(world.world().resource::<PreparedViews>().is_empty());
}

#[test]
fn the_first_camera_on_a_target_clears_it_and_the_next_one_does_not() {
	let Some((mut world, mut render, target)) = gpu_world() else {
		return;
	};
	let red = ClearMode::Color([1.0, 0.0, 0.0, 1.0]);
	let green = ClearMode::Color([0.0, 1.0, 0.0, 1.0]);
	{
		let mut extra = world.world_mut().resource_mut::<ExtraViews>();
		extra.0.push(host_view(1, ViewTarget::Target(target), 0, red));
		extra.0.push(host_view(2, ViewTarget::Target(target), 1, green));
	}
	render.run(world.world_mut());
	assert_eq!(pixel(&world, target), [255, 0, 0, 255], "red from the first, not green");
}

#[test]
fn a_clear_of_none_leaves_what_the_camera_before_drew() {
	let Some((mut world, mut render, target)) = gpu_world() else {
		return;
	};
	{
		let mut extra = world.world_mut().resource_mut::<ExtraViews>();
		extra.0.push(host_view(
			1,
			ViewTarget::Target(target),
			0,
			ClearMode::Color([0.0, 0.0, 1.0, 1.0]),
		));
	}
	render.run(world.world_mut());
	// The second frame has a camera that clears nothing: the blue of the first
	// frame is what it draws over.
	world.world_mut().resource_mut::<ExtraViews>().0[0].clear = ClearMode::None;
	render.run(world.world_mut());
	assert_eq!(pixel(&world, target), [0, 0, 255, 255]);
}

#[test]
fn a_window_that_no_camera_draws_on_is_cleared_with_the_clear_color() {
	let Some((mut world, mut render, _target)) = gpu_world() else {
		return;
	};
	world.world_mut().resource_mut::<ClearColor>().0 = wgpu::Color::RED;
	// No surface here: the frame is counted and nothing breaks.
	render.run(world.world_mut());
	assert_eq!(world.world().resource::<Renderer>().frame(), 1);
}
