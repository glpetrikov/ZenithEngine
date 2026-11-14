//! Opens a window and clears it with a color that slowly changes, through a
//! world and its render schedule:
//!
//! ```text
//! cargo run -p zenith-renderer --example clear
//! ```

use std::{sync::Arc, time::Instant};

use bevy_ecs::schedule::Schedule;
use winit::{
	application::ApplicationHandler,
	event::WindowEvent,
	event_loop::{ActiveEventLoop, EventLoop},
	window::{Window, WindowId},
};
use zenith_renderer::{ClearColor, RenderSettings, Renderer, install, wgpu};
use zenith_world::World;

struct App {
	/// The window, the world, and the render schedule, which is the app's.
	state: Option<(Arc<Window>, World, Schedule)>,
	start: Instant,
}

impl ApplicationHandler for App {
	fn resumed(&mut self, event_loop: &ActiveEventLoop) {
		if self.state.is_some() {
			return;
		}
		let window = match event_loop.create_window(Window::default_attributes().with_title("Zenith: clear")) {
			Ok(window) => Arc::new(window),
			Err(e) => {
				eprintln!("cannot open the window: {e}");
				event_loop.exit();
				return;
			}
		};
		let size = window.inner_size();
		let settings = RenderSettings::default();
		let renderer = match pollster::block_on(Renderer::new(Arc::clone(&window), size.width, size.height, &settings))
		{
			Ok(renderer) => renderer,
			Err(e) => {
				eprintln!("cannot open the GPU: {e}");
				event_loop.exit();
				return;
			}
		};
		if let Some(info) = renderer.adapter_info() {
			println!("{} ({:?}), {:?}", info.name, info.device_type, renderer.output_format());
		}
		let mut world = World::new("clear");
		install(&mut world, renderer, settings);
		window.request_redraw();
		self.state = Some((window, world, zenith_renderer::schedule()));
	}

	fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
		let Some((window, world, render)) = &mut self.state else {
			return;
		};
		match event {
			WindowEvent::CloseRequested => event_loop.exit(),
			WindowEvent::Resized(size) => {
				if let Some(mut renderer) = world.world_mut().get_resource_mut::<Renderer>() {
					renderer.resize_surface(size.width, size.height);
				}
			}
			WindowEvent::RedrawRequested => {
				let t = self.start.elapsed().as_secs_f64();
				if let Some(mut clear) = world.world_mut().get_resource_mut::<ClearColor>() {
					clear.0 = wgpu::Color {
						r: 0.5f64.mul_add(t.sin(), 0.5),
						g: 0.5f64.mul_add((t + 2.0).sin(), 0.5),
						b: 0.5f64.mul_add((t + 4.0).sin(), 0.5),
						a: 1.0,
					};
				}
				render.run(world.world_mut());
				window.request_redraw();
			}
			_ => {}
		}
	}
}

fn main() {
	let event_loop = match EventLoop::new() {
		Ok(event_loop) => event_loop,
		Err(e) => {
			eprintln!("cannot start the event loop: {e}");
			return;
		}
	};
	let mut app = App {
		state: None,
		start: Instant::now(),
	};
	if let Err(e) = event_loop.run_app(&mut app) {
		eprintln!("the event loop failed: {e}");
	}
}
