//! The passes of the editor: `editor.grid`, `editor.gizmos` and
//! `editor.outline`.
//!
//! They draw lines over what the passes before drew, with a depth test, and
//! are in the graph [`GraphId::EDITOR`](crate::graph::GraphId::EDITOR), not in
//! the built-in one.
//!
//! The editor tells them what to draw through two resources:
//! [`GizmoLines`] (lines of its own: lights, cameras) and
//! [`EditorSelection`] (the entities that get an outline).

use bevy_ecs::{entity::Entity, resource::Resource};
use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Vec3};
use wgpu::util::DeviceExt;
use zenith_error::{ZResult, ZenithError};

use super::inputs::CameraData;
use crate::{
	cache::{LayoutId, PipelineId, PipelineKey},
	extract::ExtractedMeshes,
	graph::{
		ExecContext, InitContext, InputDesc, OutputDesc, ParamDesc, ParamMap, PassAction, PassCategory, PassDesc,
		PrepareContext, RenderPass, SlotKind, pass::data,
	},
	prepare::PreparedViews,
	shaders,
};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const OUTLINE_COLOR: [f32; 4] = [1.0, 0.65, 0.1, 1.0];
const GRID_COLOR: [f32; 3] = [0.5, 0.5, 0.5];
const AXIS_X_COLOR: [f32; 3] = [0.9, 0.25, 0.25];
const AXIS_Z_COLOR: [f32; 3] = [0.25, 0.4, 0.95];

/// A line the editor wants drawn, in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GizmoLine {
	pub from: DVec3,
	pub to: DVec3,
	pub color: [f32; 4],
}

/// The lines of the gizmos of this frame. The editor fills it before every
/// frame; the pass draws what is in it.
#[derive(Resource, Default, Debug)]
pub struct GizmoLines(pub Vec<GizmoLine>);

impl GizmoLines {
	pub fn line(&mut self, from: DVec3, to: DVec3, color: [f32; 4]) {
		self.0.push(GizmoLine { from, to, color });
	}

	/// A circle of `radius` around `center` in the plane that `axis_a` and
	/// `axis_b` (unit vectors) span.
	pub fn circle(&mut self, center: DVec3, axis_a: DVec3, axis_b: DVec3, radius: f64, color: [f32; 4]) {
		const SEGMENTS: usize = 32;
		let point = |i: usize| {
			let angle = i as f64 / SEGMENTS as f64 * std::f64::consts::TAU;
			center + (axis_a * angle.cos() + axis_b * angle.sin()) * radius
		};
		for i in 0..SEGMENTS {
			self.line(point(i), point(i + 1), color);
		}
	}

	/// The edges of a box of `corners` (as [`box_corners`] orders them).
	pub fn wire_box(&mut self, corners: &[DVec3; 8], color: [f32; 4]) {
		for (a, b) in BOX_EDGES {
			self.line(corners[a], corners[b], color);
		}
	}
}

/// The entities that get an outline.
#[derive(Resource, Default, Debug)]
pub struct EditorSelection(pub Vec<Entity>);

/// The pairs of corners (see [`box_corners`]) that are the edges of a box.
const BOX_EDGES: [(usize, usize); 12] = [
	(0, 1),
	(2, 3),
	(4, 5),
	(6, 7),
	(0, 2),
	(1, 3),
	(4, 6),
	(5, 7),
	(0, 4),
	(1, 5),
	(2, 6),
	(3, 7),
];

/// The corners of the box `min`..`max`, in bit order: bit 0 is x, bit 1 is y,
/// bit 2 is z (0 for `min`, 1 for `max`).
pub fn box_corners(min: Vec3, max: Vec3) -> [Vec3; 8] {
	std::array::from_fn(|i| {
		Vec3::new(
			if i & 1 == 0 { min.x } else { max.x },
			if i & 2 == 0 { min.y } else { max.y },
			if i & 4 == 0 { min.z } else { max.z },
		)
	})
}

/// A vertex of a line, relative to the camera.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
struct LineVertex {
	position: [f32; 3],
	color: [f32; 4],
}

impl LineVertex {
	const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
		array_stride: size_of::<Self>() as wgpu::BufferAddress,
		step_mode: wgpu::VertexStepMode::Vertex,
		attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
	};
}

/// The lines a pass draws, made by `prepare` for `exec`.
#[derive(Default)]
struct Lines {
	layout: Option<LayoutId>,
	pipeline: Option<PipelineId>,
	camera_group: Option<wgpu::BindGroup>,
	buffer: Option<wgpu::Buffer>,
	count: u32,
}

impl Lines {
	fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()> {
		self.layout = Some(ctx.cache.layout(&[wgpu::BindGroupLayoutEntry {
			binding: 0,
			visibility: wgpu::ShaderStages::VERTEX,
			ty: wgpu::BindingType::Buffer {
				ty: wgpu::BufferBindingType::Uniform,
				has_dynamic_offset: false,
				min_binding_size: wgpu::BufferSize::new(size_of::<crate::prepare::CameraUniform>() as u64),
			},
			count: None,
		}]));
		if ctx.cache.named_shader(shaders::LINES).is_none() {
			return Err(ZenithError::Other(format!("no shader {}", shaders::LINES)));
		}
		Ok(())
	}

	/// Gets ready to draw the lines `make` gives; it is told where the camera
	/// is.
	fn prepare(
		&mut self,
		ctx: &mut PrepareContext<'_>,
		make: impl FnOnce(&PrepareContext<'_>, DVec3) -> Vec<LineVertex>,
	) -> PassAction {
		self.pipeline = None;
		self.camera_group = None;
		self.buffer = None;
		self.count = 0;
		let shader = ctx.cache.named_shader(shaders::LINES);
		let position = ctx
			.world
			.get_resource::<PreparedViews>()
			.and_then(|views| views.get(ctx.view.key))
			.map(|view| view.position);
		let (Some(camera), Some(layout_id), Some(shader), Some(format), Some(position)) = (
			ctx.data::<CameraData>("camera").cloned(),
			self.layout,
			shader,
			ctx.texture_format("color"),
			position,
		) else {
			return PassAction::Skip;
		};
		let Some(layout) = ctx.cache.get_layout(layout_id).cloned() else {
			return PassAction::Skip;
		};
		let vertices = make(ctx, position);
		if vertices.is_empty() {
			return PassAction::Skip;
		}
		self.count = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
		// A new buffer every frame: the views of a frame are recorded one
		// after the other and submitted together, so one that is written
		// again would show the last view's lines in all of them.
		self.buffer = Some(ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some("Zenith Lines"),
			contents: bytemuck::cast_slice(&vertices),
			usage: wgpu::BufferUsages::VERTEX,
		}));
		self.camera_group = Some(ctx.bind_group(
			&layout,
			&[wgpu::BindGroupEntry {
				binding: 0,
				resource: camera.buffer.as_entire_binding(),
			}],
		));
		let key = PipelineKey {
			layouts: vec![layout_id],
			vertex_buffers: vec![LineVertex::LAYOUT],
			targets: vec![Some(wgpu::ColorTargetState {
				format,
				blend: Some(wgpu::BlendState::ALPHA_BLENDING),
				write_mask: wgpu::ColorWrites::ALL,
			})],
			depth_stencil: Some(wgpu::DepthStencilState {
				format: DEPTH_FORMAT,
				depth_write_enabled: Some(false),
				depth_compare: Some(wgpu::CompareFunction::LessEqual),
				stencil: wgpu::StencilState::default(),
				bias: wgpu::DepthBiasState::default(),
			}),
			primitive: wgpu::PrimitiveState {
				topology: wgpu::PrimitiveTopology::LineList,
				..wgpu::PrimitiveState::default()
			},
			..PipelineKey::new(shader, format)
		};
		self.pipeline = Some(ctx.cache.pipeline(&key));
		PassAction::Run
	}

	fn exec(&self, ctx: &mut ExecContext<'_>, label: &str) -> ZResult<()> {
		let cache = ctx.cache;
		let mut pass = ctx
			.begin_render_pass(label, &[("color", wgpu::Color::BLACK)], Some("depth"))
			.ok_or_else(|| ZenithError::Other(format!("{label}: the color or the depth is not bound")))?;
		let (Some(pipeline), Some(group), Some(buffer)) = (
			self.pipeline.and_then(|id| cache.get_pipeline(id)),
			&self.camera_group,
			&self.buffer,
		) else {
			return Ok(());
		};
		pass.set_pipeline(pipeline);
		pass.set_bind_group(0, group, &[]);
		pass.set_vertex_buffer(0, buffer.slice(..));
		pass.draw(0..self.count, 0..1);
		Ok(())
	}
}

/// A line from `from` to `to`, seen from `camera`.
fn push(vertices: &mut Vec<LineVertex>, camera: DVec3, from: DVec3, to: DVec3, color: [f32; 4]) {
	for point in [from, to] {
		vertices.push(LineVertex {
			position: (point - camera).as_vec3().to_array(),
			color,
		});
	}
}

fn pass_description(name: &'static str, display: &'static str) -> PassDesc {
	PassDesc::new(name, display, PassCategory::Debug)
		.input(InputDesc::read("camera", SlotKind::data(data::CAMERA)))
		.input(InputDesc::write("color", SlotKind::Color))
		.input(InputDesc::write("depth", SlotKind::Depth))
		.output(OutputDesc::pass_through("color", SlotKind::Color, "color"))
		.output(OutputDesc::pass_through("depth", SlotKind::Depth, "depth"))
}

/// `editor.grid`: a grid on the ground (the plane `y = 0`), brightest at the
/// camera and gone far from it, with the X and Z axes in color.
pub struct Grid {
	extent: i64,
	lines: Lines,
}

impl Default for Grid {
	fn default() -> Self {
		Self {
			extent: DEFAULT_EXTENT,
			lines: Lines::default(),
		}
	}
}

const DEFAULT_EXTENT: i64 = 60;

impl RenderPass for Grid {
	fn describe() -> PassDesc {
		pass_description("editor.grid", "Grid").param(
			ParamDesc::int("extent", DEFAULT_EXTENT)
				.range(1.0, 500.0)
				.tooltip("How far from the camera the lines go, in units"),
		)
	}

	fn set_params(&mut self, params: &ParamMap) -> ZResult<()> {
		self.extent = params.int("extent").unwrap_or(DEFAULT_EXTENT).clamp(1, 500);
		Ok(())
	}

	fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()> {
		self.lines.init(ctx)
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		let extent = self.extent;
		self.lines.prepare(ctx, |_, camera| grid_lines(camera, extent))
	}

	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()> {
		self.lines.exec(ctx, "editor grid")
	}
}

/// The lines of the grid around `camera`, one unit apart.
fn grid_lines(camera: DVec3, extent: i64) -> Vec<LineVertex> {
	let mut vertices = Vec::new();
	let (cx, cz) = (camera.x.floor() as i64, camera.z.floor() as i64);
	let reach = extent as f64;
	let fade = |distance: f64| (1.0 - distance / reach).clamp(0.0, 1.0) as f32;
	for step in -extent..=extent {
		for (axis_is_x, base) in [(true, cx), (false, cz)] {
			let at = base + step;
			let along = if axis_is_x { cz } else { cx };
			let line_position = at as f64;
			let distance = (line_position - if axis_is_x { camera.x } else { camera.z }).abs();
			let alpha = fade(distance) * if at % 10 == 0 { 0.6 } else { 0.25 };
			if alpha <= 0.0 {
				continue;
			}
			let rgb = match (at, axis_is_x) {
				// The line x = 0 is the Z axis; the line z = 0 is the X axis.
				(0, true) => AXIS_Z_COLOR,
				(0, false) => AXIS_X_COLOR,
				_ => GRID_COLOR,
			};
			let color = [rgb[0], rgb[1], rgb[2], if at == 0 { 0.9 } else { alpha }];
			let (start, end) = ((along - extent) as f64, (along + extent) as f64);
			let (from, to) = if axis_is_x {
				(
					DVec3::new(line_position, 0.0, start),
					DVec3::new(line_position, 0.0, end),
				)
			} else {
				(
					DVec3::new(start, 0.0, line_position),
					DVec3::new(end, 0.0, line_position),
				)
			};
			push(&mut vertices, camera, from, to, color);
		}
	}
	vertices
}

/// `editor.gizmos`: the lines in [`GizmoLines`].
#[derive(Default)]
pub struct Gizmos {
	lines: Lines,
}

impl RenderPass for Gizmos {
	fn describe() -> PassDesc {
		pass_description("editor.gizmos", "Gizmos")
	}

	fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()> {
		self.lines.init(ctx)
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		self.lines.prepare(ctx, |ctx, camera| {
			let mut vertices = Vec::new();
			if let Some(lines) = ctx.world.get_resource::<GizmoLines>() {
				for line in &lines.0 {
					push(&mut vertices, camera, line.from, line.to, line.color);
				}
			}
			vertices
		})
	}

	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()> {
		self.lines.exec(ctx, "editor gizmos")
	}
}

/// `editor.outline`: the box around each selected model.
#[derive(Default)]
pub struct Outline {
	lines: Lines,
}

impl RenderPass for Outline {
	fn describe() -> PassDesc {
		pass_description("editor.outline", "Outline")
	}

	fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()> {
		self.lines.init(ctx)
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		// The cache is borrowed for the shader and the pipeline later, so the
		// boxes are found first.
		let mut boxes: Vec<[DVec3; 8]> = Vec::new();
		if let (Some(selection), Some(meshes)) = (
			ctx.world.get_resource::<EditorSelection>(),
			ctx.world.get_resource::<ExtractedMeshes>(),
		) {
			for mesh in meshes.0.iter().filter(|mesh| selection.0.contains(&mesh.entity)) {
				let Some(gpu) = ctx.cache.mesh(mesh.mesh) else {
					continue;
				};
				let (min, max) = (Vec3::from(gpu.bounds.0), Vec3::from(gpu.bounds.1));
				boxes.push(
					box_corners(min, max)
						.map(|corner| mesh.position + (mesh.rotation * (corner * mesh.scale)).as_dvec3()),
				);
			}
		}
		self.lines.prepare(ctx, |_, camera| {
			let mut vertices = Vec::new();
			for corners in &boxes {
				for (a, b) in BOX_EDGES {
					push(&mut vertices, camera, corners[a], corners[b], OUTLINE_COLOR);
				}
			}
			vertices
		})
	}

	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()> {
		self.lines.exec(ctx, "editor outline")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_box_has_eight_corners_and_twelve_edges_that_join_corners_one_bit_apart() {
		let corners = box_corners(Vec3::ZERO, Vec3::ONE);
		assert_eq!(corners[0], Vec3::ZERO);
		assert_eq!(corners[7], Vec3::ONE);
		for (a, b) in BOX_EDGES {
			assert_eq!((a ^ b).count_ones(), 1, "{a} {b}");
		}
		let mut seen = BOX_EDGES.to_vec();
		seen.sort_unstable();
		seen.dedup();
		assert_eq!(seen.len(), 12);
	}

	#[test]
	fn the_grid_has_a_line_each_way_for_every_unit_and_colors_the_axes() {
		let vertices = grid_lines(DVec3::new(0.5, 3.0, 0.5), 4);
		// 9 lines each way, 2 vertices each (the ones that fade to nothing are
		// left out: none at this extent).
		assert!(vertices.len() <= 9 * 2 * 2);
		assert!(vertices.iter().any(|v| v.color[..3] == AXIS_X_COLOR));
		assert!(vertices.iter().any(|v| v.color[..3] == AXIS_Z_COLOR));
		// Relative to the camera: the ground is 3 units below it.
		assert!(vertices.iter().all(|v| (v.position[1] + 3.0).abs() < 1e-6));
	}

	#[test]
	fn gizmo_lines_make_circles_and_boxes() {
		let mut lines = GizmoLines::default();
		lines.circle(DVec3::ZERO, DVec3::X, DVec3::Y, 2.0, [1.0; 4]);
		assert_eq!(lines.0.len(), 32);
		assert!(lines.0.iter().all(|l| (l.from.length() - 2.0).abs() < 1e-9));
		lines.0.clear();
		lines.wire_box(&box_corners(Vec3::ZERO, Vec3::ONE).map(Vec3::as_dvec3), [1.0; 4]);
		assert_eq!(lines.0.len(), 12);
	}
}
