//! The built-in graph (input camera, input meshes, opaque, tonemap, output)
//! draws a world into a texture, on whatever adapter the machine has. The
//! tests are skipped where there is no adapter at all.

// A test that cannot go on has to stop: panicking is how it fails.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use bevy_ecs::schedule::Schedule;
use glam::{DVec3, Quat};
use zenith_components::Transform;
use zenith_filesystem::{Project, ZPath};
use zenith_renderer::{
	ProjectRes, RenderSettings, Renderer,
	assets::{AssetId, AssetRegistry},
	components::{ClearMode, Model, Projection},
	extract::{ExtraViews, ExtractedView, ViewSource, ViewTarget},
	install,
	targets::{RenderTargetId, RenderTargets},
	wgpu,
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const SIZE: u32 = 64;

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

fn id_of(registry: &AssetRegistry, path: &str) -> AssetId {
	registry
		.id_of(&ZPath::new(path).expect("a valid path"))
		.unwrap_or_else(|| panic!("{path} is not an asset: {:?}", registry.problems()))
}

/// A world with the sample project, a renderer, a target, and one view that
/// looks at the origin from 4 units away.
struct Scene {
	world: zenith_world::World,
	render: Schedule,
	target: RenderTargetId,
	cube: AssetId,
	material: AssetId,
}

impl Scene {
	fn new() -> Option<Self> {
		let Some((device, queue)) = device() else {
			eprintln!("no adapter: skipped");
			return None;
		};
		let project =
			Project::open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Sandbox/Test")).expect("open the sample");
		let registry = AssetRegistry::scan(&project).expect("scan its assets");
		let cube = id_of(&registry, "Assets://Models/Cube/cube.zenithmodel");
		let material = id_of(&registry, "Assets://Models/Cube/Brick.zenithmaterial");

		let mut world = zenith_world::World::new("w");
		install(
			&mut world,
			Renderer::from_device(device.clone(), queue, FORMAT),
			RenderSettings::default(),
		);
		let target = world
			.world_mut()
			.resource_mut::<RenderTargets>()
			.create(&device, (SIZE, SIZE), FORMAT);
		world.world_mut().insert_resource(ProjectRes(Arc::new(project)));
		world.world_mut().insert_resource(registry);
		world.world_mut().resource_mut::<ExtraViews>().0.push(ExtractedView {
			source: ViewSource::Host(1),
			position: DVec3::new(0.0, 0.0, 4.0),
			rotation: Quat::IDENTITY,
			projection: Projection::default(),
			target: ViewTarget::Target(target),
			viewport: None,
			order: 0,
			clear: ClearMode::default(),
			graph: None,
			environment: None,
		});
		Some(Self {
			world,
			render: zenith_renderer::schedule(),
			target,
			cube,
			material,
		})
	}

	fn spawn_cube(&mut self, position: DVec3) {
		let model = Model {
			mesh: self.cube,
			material: self.material,
		};
		let transform = Transform {
			position,
			..Transform::default()
		};
		self.world.world_mut().spawn((model, transform));
	}

	/// Frames until the assets have loaded (the cache loads a few a frame).
	fn render_frames(&mut self, frames: usize) {
		for _ in 0..frames {
			self.render.run(self.world.world_mut());
		}
	}

	fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
		let renderer = self.world.world().resource::<Renderer>();
		let target = self
			.world
			.world()
			.resource::<RenderTargets>()
			.get(self.target)
			.expect("the target");
		let device = renderer.device();
		let buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("read back"),
			size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
			usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
			mapped_at_creation: false,
		});
		let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
		let mut source = target.texture.as_image_copy();
		source.origin = wgpu::Origin3d { x, y, z: 0 };
		encoder.copy_texture_to_buffer(
			source,
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
}

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
