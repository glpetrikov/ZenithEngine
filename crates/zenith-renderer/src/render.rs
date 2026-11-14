//! The `render` system: runs the graph of every view, submits and presents.
//!
//! There is no graph yet: the window is cleared with [`ClearColor`].

use bevy_ecs::{
	resource::Resource,
	system::{Res, ResMut},
};

use crate::renderer::Renderer;

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

/// Draws a frame into the surface.
#[allow(clippy::needless_pass_by_value)] // a system takes its parameters by value
pub fn render(mut renderer: ResMut<Renderer>, clear: Res<ClearColor>) {
	let Some(frame) = renderer.begin_frame() else {
		return;
	};
	let mut encoder = renderer
		.device()
		.create_command_encoder(&wgpu::CommandEncoderDescriptor {
			label: Some("Zenith Frame"),
		});
	encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
		label: Some("Clear"),
		color_attachments: &[Some(wgpu::RenderPassColorAttachment {
			view: &frame.view,
			resolve_target: None,
			depth_slice: None,
			ops: wgpu::Operations {
				load: wgpu::LoadOp::Clear(clear.0),
				store: wgpu::StoreOp::Store,
			},
		})],
		depth_stencil_attachment: None,
		occlusion_query_set: None,
		timestamp_writes: None,
		multiview_mask: None,
	});
	renderer.queue().submit([encoder.finish()]);
	renderer.present(frame);
}
