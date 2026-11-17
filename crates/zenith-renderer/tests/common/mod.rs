//! What the tests that draw a world into a texture share: a device (the tests
//! are skipped where there is no adapter), the sample project and a view that
//! looks at the origin from 4 units away.

// A test that cannot go on has to stop: panicking is how it fails.
#![allow(clippy::expect_used, clippy::panic, dead_code)]

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

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const SIZE: u32 = 64;

pub fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
	let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
	let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
	pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
		label: Some("test device"),
		required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
		..Default::default()
	}))
	.ok()
}

pub fn id_of(registry: &AssetRegistry, path: &str) -> AssetId {
	registry
		.id_of(&ZPath::new(path).expect("a valid path"))
		.unwrap_or_else(|| panic!("{path} is not an asset: {:?}", registry.problems()))
}

/// A world with the sample project, a renderer, a target, and one view that
/// looks at the origin from 4 units away.
pub struct Scene {
	pub world: zenith_world::World,
	pub render: Schedule,
	pub target: RenderTargetId,
	pub cube: AssetId,
	pub material: AssetId,
}

impl Scene {
	pub fn new() -> Option<Self> {
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

	pub fn spawn_cube(&mut self, position: DVec3) {
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
	pub fn render_frames(&mut self, frames: usize) {
		for _ in 0..frames {
			self.render.run(self.world.world_mut());
		}
	}

	pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
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

impl Scene {
	/// Every pixel of the target, row by row.
	pub fn image(&self) -> Vec<[u8; 4]> {
		let renderer = self.world.world().resource::<Renderer>();
		let target = self
			.world
			.world()
			.resource::<RenderTargets>()
			.get(self.target)
			.expect("the target");
		let device = renderer.device();
		// A row of SIZE pixels is a multiple of the alignment of the copy.
		let row = SIZE * 4;
		assert_eq!(row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0, "the rows are padded");
		let buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("read back"),
			size: u64::from(row * SIZE),
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
					bytes_per_row: Some(row),
					rows_per_image: Some(SIZE),
				},
			},
			wgpu::Extent3d {
				width: SIZE,
				height: SIZE,
				depth_or_array_layers: 1,
			},
		);
		renderer.queue().submit([encoder.finish()]);
		buffer.map_async(wgpu::MapMode::Read, .., |result| result.expect("map the buffer"));
		device
			.poll(wgpu::PollType::wait_indefinitely())
			.expect("wait for the GPU");
		let bytes = buffer.get_mapped_range(..).expect("read the buffer");
		bytes.as_chunks::<4>().0.to_vec()
	}

	/// Looks at the origin from above and behind, with `graph`.
	pub fn look_from_above(&mut self, graph: Option<zenith_renderer::graph::GraphId>) {
		let mut extra = self.world.world_mut().resource_mut::<ExtraViews>();
		extra.0[0].position = DVec3::new(0.0, 2.0, 4.0);
		extra.0[0].rotation = Quat::from_rotation_x(-0.45);
		extra.0[0].graph = graph;
	}
}
