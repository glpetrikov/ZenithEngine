//! The `render` system: runs the graph of every view, submits and presents.
//!
//! The views come in the order the extract made (by `order`). The first one
//! on a target clears it, the others draw over it. A view runs the graph of
//! its camera, or the main one, or the built-in one; when none of them runs,
//! the target is only cleared.

use std::collections::HashSet;

use bevy_ecs::{change_detection::Mut, resource::Resource, world::World};

use crate::{
	cache::GpuCache,
	components::ClearMode,
	extract::{ExtractedViews, ViewTarget},
	graph::{Frame, ViewInfo},
	prepare::views::PreparedViews,
	renderer::Renderer,
	surface::SurfaceFrame,
	targets::RenderTargets,
};

/// The color of a window that no camera draws on.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ClearColor(pub wgpu::Color);

impl Default for ClearColor {
	fn default() -> Self {
		Self(wgpu::Color {
			r: 0.1,
			g: 0.1,
			b: 0.1,
			a: 1.0,
		})
	}
}

/// Draws a frame: every view into its target, then the window is shown.
pub fn render(world: &mut World) {
	world.resource_scope(|world, mut renderer: Mut<Renderer>| {
		world.resource_scope(|world, mut cache: Mut<GpuCache>| {
			draw(world, &mut renderer, &mut cache);
		});
	});
}

fn draw(world: &World, renderer: &mut Renderer, cache: &mut GpuCache) {
	let window = renderer.begin_frame();
	let window_clear = world
		.get_resource::<ClearColor>()
		.map_or_else(|| ClearColor::default().0, |color| color.0);
	let (Some(views), Some(prepared)) = (
		world.get_resource::<ExtractedViews>(),
		world.get_resource::<PreparedViews>(),
	) else {
		finish(renderer, window, window_clear);
		return;
	};
	let targets = world.get_resource::<RenderTargets>();

	let parts = renderer.parts();
	let mut encoder = parts.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
		label: Some("Zenith Frame"),
	});
	let mut cleared: HashSet<ViewTarget> = HashSet::new();

	for view in &views.0 {
		let Some(prepared) = prepared.get(view.key()) else {
			continue;
		};
		let (target, format, size) = match view.target {
			ViewTarget::Window => match (&window, parts.surface_size) {
				(Some(frame), Some(size)) => (&frame.view, parts.output_format, size),
				_ => continue,
			},
			ViewTarget::Target(id) => match targets.and_then(|targets| targets.get(id)) {
				Some(target) => (&target.view, target.format, target.size),
				None => continue,
			},
		};
		// Only the first camera on a target clears it.
		let first = cleared.insert(view.target);
		let clear = match view.clear {
			ClearMode::Color(color) if first => Some(color_of(color)),
			_ => None,
		};
		let info = ViewInfo {
			key: prepared.key,
			target,
			target_format: format,
			target_size: size,
			viewport: view.viewport,
			clear,
		};
		match parts.graphs.resolve(view.graph) {
			Some((_, graph)) => {
				let mut frame = Frame {
					device: parts.device,
					queue: parts.queue,
					cache,
					world,
					pool: &mut *parts.pool,
					encoder: &mut encoder,
					number: parts.frame,
				};
				graph.run(&mut frame, &info);
			}
			None => {
				if let Some(color) = clear {
					clear_target(&mut encoder, target, color);
				}
			}
		}
	}

	if let Some(frame) = &window
		&& !cleared.contains(&ViewTarget::Window)
	{
		clear_target(&mut encoder, &frame.view, window_clear);
	}
	parts.queue.submit([encoder.finish()]);
	if let Some(frame) = window {
		renderer.present(frame);
	}
}

/// Presents a frame that nothing drew into: it is cleared.
fn finish(renderer: &Renderer, window: Option<SurfaceFrame>, color: wgpu::Color) {
	let Some(frame) = window else {
		return;
	};
	let mut encoder = renderer
		.device()
		.create_command_encoder(&wgpu::CommandEncoderDescriptor {
			label: Some("Zenith Frame"),
		});
	clear_target(&mut encoder, &frame.view, color);
	renderer.queue().submit([encoder.finish()]);
	renderer.present(frame);
}

fn color_of(color: [f32; 4]) -> wgpu::Color {
	wgpu::Color {
		r: f64::from(color[0]),
		g: f64::from(color[1]),
		b: f64::from(color[2]),
		a: f64::from(color[3]),
	}
}

fn clear_target(encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, color: wgpu::Color) {
	encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
		label: Some("Clear"),
		color_attachments: &[Some(wgpu::RenderPassColorAttachment {
			view: target,
			resolve_target: None,
			depth_slice: None,
			ops: wgpu::Operations {
				load: wgpu::LoadOp::Clear(color),
				store: wgpu::StoreOp::Store,
			},
		})],
		depth_stencil_attachment: None,
		occlusion_query_set: None,
		timestamp_writes: None,
		multiview_mask: None,
	});
}
