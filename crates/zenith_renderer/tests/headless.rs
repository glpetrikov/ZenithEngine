//! Draws the scene without a window, on whatever adapter the machine has (a
//! software one, like lavapipe, will do), and looks at the pixels.
//!
//! The tests are skipped where there is no adapter at all. To look at the
//! frames, set `ZENITH_RENDER_DUMP` to a folder: the PNGs are written there.

// A test that cannot go on has to stop: panicking is how it fails.
#![allow(clippy::expect_used)]

use glam::{Vec2, Vec3};
use zenith_filesystem::Project;
use zenith_renderer_old::{camera::CameraInput, renderer::Renderer};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

struct Gpu {
	device: wgpu::Device,
	queue: wgpu::Queue,
}

fn gpu() -> Option<Gpu> {
	let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
	let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
	println!("adapter: {:?}", adapter.get_info());
	let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
		label: Some("test device"),
		required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
		..Default::default()
	}))
	.ok()?;
	Some(Gpu { device, queue })
}

fn project() -> Project {
	Project::open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Sandbox/Test")).expect("open the sample project")
}

fn renderer(gpu: &Gpu) -> Renderer {
	Renderer::new(&gpu.device, &gpu.queue, FORMAT, WIDTH, HEIGHT, &project(), "test").expect("create the renderer")
}

/// Draws one frame and returns its pixels (RGBA, row by row).
fn frame(gpu: &Gpu, renderer: &Renderer) -> Vec<u8> {
	let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
		label: Some("target"),
		size: wgpu::Extent3d {
			width: WIDTH,
			height: HEIGHT,
			depth_or_array_layers: 1,
		},
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: FORMAT,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
		view_formats: &[],
	});
	let view = target.create_view(&wgpu::TextureViewDescriptor::default());

	let bytes_per_row = WIDTH * 4;
	assert_eq!(
		bytes_per_row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT,
		0,
		"the rows must be aligned"
	);
	let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
		label: Some("readback"),
		size: u64::from(bytes_per_row * HEIGHT),
		usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
		mapped_at_creation: false,
	});

	let mut encoder = gpu
		.device
		.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
	renderer.render(&mut encoder, &view, "test pass");
	encoder.copy_texture_to_buffer(
		target.as_image_copy(),
		wgpu::TexelCopyBufferInfo {
			buffer: &readback,
			layout: wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: Some(bytes_per_row),
				rows_per_image: Some(HEIGHT),
			},
		},
		wgpu::Extent3d {
			width: WIDTH,
			height: HEIGHT,
			depth_or_array_layers: 1,
		},
	);
	gpu.queue.submit([encoder.finish()]);

	readback
		.slice(..)
		.map_async(wgpu::MapMode::Read, |r| r.expect("map the readback buffer"));
	gpu.device
		.poll(wgpu::PollType::wait_indefinitely())
		.expect("wait for the GPU");
	let pixels = readback
		.slice(..)
		.get_mapped_range()
		.expect("read the mapped range")
		.to_vec();
	readback.unmap();
	pixels
}

fn dump(name: &str, pixels: &[u8]) {
	if let Ok(dir) = std::env::var("ZENITH_RENDER_DUMP") {
		std::fs::create_dir_all(&dir).expect("create the dump folder");
		image::save_buffer(
			format!("{dir}/{name}.png"),
			pixels,
			WIDTH,
			HEIGHT,
			image::ColorType::Rgba8,
		)
		.expect("write the png");
	}
}

const fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
	let i = ((y * WIDTH + x) * 4) as usize;
	[pixels[i], pixels[i + 1], pixels[i + 2]]
}

fn distinct_colors(pixels: &[u8]) -> usize {
	let mut seen = std::collections::HashSet::new();
	for p in pixels.as_chunks::<4>().0 {
		seen.insert([p[0], p[1], p[2]]);
	}
	seen.len()
}

#[test]
fn the_scene_is_drawn() {
	let Some(gpu) = gpu() else {
		eprintln!("no adapter: skipped");
		return;
	};
	let mut renderer = renderer(&gpu);
	renderer.update(0.0, &CameraInput::default());
	let pixels = frame(&gpu, &renderer);
	dump("scene", &pixels);

	// Many colors: lit and textured bricks, not just the clear color.
	assert!(distinct_colors(&pixels) > 200, "the frame looks empty");
}

#[test]
fn the_camera_turns_and_moves() {
	let Some(gpu) = gpu() else {
		return;
	};
	let mut renderer = renderer(&gpu);
	renderer.update(0.0, &CameraInput::default());
	let before = frame(&gpu, &renderer);

	let yaw = renderer.camera().yaw;
	let pitch = renderer.camera().pitch;
	renderer.update(
		0.5,
		&CameraInput {
			forward: 1.0,
			look: Vec2::new(100.0, -50.0),
			..Default::default()
		},
	);
	assert!(
		renderer.camera().yaw > yaw,
		"the mouse to the right turns the camera to the right"
	);
	assert!(renderer.camera().pitch > pitch, "the mouse up looks up");
	let after = frame(&gpu, &renderer);
	dump("camera-moved", &after);
	assert_ne!(before, after, "the view did not change");
}

/// The sky of the sample is brightest around its sun, which is at a longitude
/// of -75 degrees and 12 degrees above the horizon.
#[test]
fn the_sky_and_the_sun_are_seen() {
	let Some(gpu) = gpu() else {
		return;
	};
	let mut renderer = renderer(&gpu);

	// High above the cubes, looking at the sun.
	renderer.camera_mut().position = Vec3::new(0.0, 30.0, 0.0);
	renderer.camera_mut().yaw = (-75.0_f32).to_radians();
	renderer.camera_mut().pitch = 12.0_f32.to_radians();
	renderer.update(0.0, &CameraInput::default());
	let toward_the_sun = frame(&gpu, &renderer);
	dump("sky-sun", &toward_the_sun);

	// The sun is much brighter than white, and tone mapping leaves it white.
	let (cx, cy) = (WIDTH / 2, HEIGHT / 2);
	let centre = pixel(&toward_the_sun, cx, cy);
	assert!(centre.iter().all(|&c| c > 245), "the sun is not white: {centre:?}");

	// Looking straight up: the sky is blue, not the clear color of the scene.
	renderer.camera_mut().pitch = 80.0_f32.to_radians();
	renderer.update(0.0, &CameraInput::default());
	let up = frame(&gpu, &renderer);
	dump("sky-up", &up);
	let zenith = pixel(&up, cx, cy);
	assert!(zenith[2] > zenith[0], "the zenith should be blue: {zenith:?}");

	// Looking down at the ground of the sky: darker than the glow of the sun.
	renderer.camera_mut().position = Vec3::new(0.0, 60.0, 0.0);
	renderer.camera_mut().pitch = (-80.0_f32).to_radians();
	renderer.update(0.0, &CameraInput::default());
	let down = frame(&gpu, &renderer);
	dump("sky-down", &down);
	let sum = |p: [u8; 3]| p.iter().map(|&c| u32::from(c)).sum::<u32>();
	assert!(
		sum(pixel(&down, 5, 5)) < sum(centre),
		"the ground is brighter than the sun"
	);
}

#[test]
fn a_cube_close_up_reflects_the_sky() {
	let Some(gpu) = gpu() else {
		return;
	};
	let mut renderer = renderer(&gpu);
	renderer.camera_mut().position = Vec3::new(-1.0, 1.5, 4.0);
	renderer.camera_mut().yaw = (-80.0_f32).to_radians();
	renderer.camera_mut().pitch = (-15.0_f32).to_radians();
	renderer.update(0.0, &CameraInput::default());
	let pixels = frame(&gpu, &renderer);
	dump("closeup", &pixels);
	assert!(distinct_colors(&pixels) > 500);
}
