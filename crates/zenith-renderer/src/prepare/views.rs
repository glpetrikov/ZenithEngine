//! `prepare_views`: the matrices and the uniform buffer of every view.
//!
//! The position of a view is a `DVec3`; the GPU never sees it. Everything is
//! drawn relative to the camera ([`InstanceData`](super::InstanceData) holds
//! the model matrix relative to it), so the view matrix has no translation.
//!
//! Each view has a buffer of its own: `queue.write_buffer` takes effect before
//! any command of the frame runs, so one buffer for all the views would give
//! every one of them the data of the last.

use std::{collections::HashMap, mem};

use bevy_ecs::{
	resource::Resource,
	system::{Res, ResMut},
};
use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Mat4, Quat};

use crate::{
	extract::{ExtractedView, ExtractedViews, ViewTarget},
	graph::{ViewKey, viewport_pixels},
	renderer::Renderer,
	targets::RenderTargets,
};

/// A view as the shaders read it (the `camera` slot of the graph).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct CameraUniform {
	/// World (relative to the camera) to view space.
	pub view: [[f32; 4]; 4],
	pub proj: [[f32; 4]; 4],
	pub view_proj: [[f32; 4]; 4],
	pub inv_view_proj: [[f32; 4]; 4],
	/// The size of the viewport in pixels, then near and far.
	pub viewport: [f32; 4],
}

/// A view of the frame, ready.
pub struct PreparedView {
	pub key: ViewKey,
	pub position: DVec3,
	pub rotation: Quat,
	/// The size of the viewport in pixels.
	pub size: (u32, u32),
	pub uniform: CameraUniform,
	pub buffer: wgpu::Buffer,
}

/// The views of the frame, by key.
#[derive(Resource, Default)]
pub struct PreparedViews {
	views: HashMap<ViewKey, PreparedView>,
}

impl PreparedViews {
	pub fn get(&self, key: ViewKey) -> Option<&PreparedView> {
		self.views.get(&key)
	}

	pub fn len(&self) -> usize {
		self.views.len()
	}

	pub fn is_empty(&self) -> bool {
		self.views.is_empty()
	}
}

/// The size of what `target` draws into, if it exists and has one.
pub fn target_size(target: ViewTarget, renderer: &Renderer, targets: Option<&RenderTargets>) -> Option<(u32, u32)> {
	match target {
		ViewTarget::Window => renderer.surface_size(),
		ViewTarget::Target(_) | ViewTarget::Texture { .. } => targets?.resolve(target).map(|target| target.size),
	}
}

/// The uniform of `view` for a viewport of `size` pixels.
pub fn camera_uniform(view: &ExtractedView, size: (u32, u32)) -> CameraUniform {
	let view_matrix = Mat4::from_quat(view.rotation.inverse());
	let proj = view.projection.matrix(size);
	let view_proj = proj * view_matrix;
	CameraUniform {
		view: view_matrix.to_cols_array_2d(),
		proj: proj.to_cols_array_2d(),
		view_proj: view_proj.to_cols_array_2d(),
		inv_view_proj: view_proj.inverse().to_cols_array_2d(),
		viewport: [
			size.0 as f32,
			size.1 as f32,
			view.projection.near(),
			view.projection.far(),
		],
	}
}

/// Makes a [`PreparedView`] of every extracted view that has a place to draw
/// into. The buffer of a view lives on from frame to frame.
#[allow(clippy::needless_pass_by_value)] // a system takes its parameters by value
pub fn prepare_views(
	extracted: Res<ExtractedViews>,
	renderer: Res<Renderer>,
	targets: Option<Res<RenderTargets>>,
	mut prepared: ResMut<PreparedViews>,
) {
	let mut old = mem::take(&mut prepared.views);
	for view in &extracted.0 {
		let Some(target_size) = target_size(view.target, &renderer, targets.as_deref()) else {
			continue;
		};
		let [_, _, width, height] = viewport_pixels(target_size, view.viewport);
		let uniform = camera_uniform(view, (width, height));
		let key = view.key();
		let buffer = old.remove(&key).map_or_else(
			|| {
				renderer.device().create_buffer(&wgpu::BufferDescriptor {
					label: Some("Zenith Camera"),
					size: size_of::<CameraUniform>() as u64,
					usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
					mapped_at_creation: false,
				})
			},
			|view| view.buffer,
		);
		renderer.queue().write_buffer(&buffer, 0, bytemuck::bytes_of(&uniform));
		prepared.views.insert(
			key,
			PreparedView {
				key,
				position: view.position,
				rotation: view.rotation,
				size: (width, height),
				uniform,
				buffer,
			},
		);
	}
}
