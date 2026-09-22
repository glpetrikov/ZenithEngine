use zenith_error::{ZResult, ZenithError};

use crate::{pick_surface_format, render_target::RenderTargetHandle};

pub struct RenderPass {
	// TODO: add color for future imnodes render graph editor
	// TODO: add reads/writes and other things
	pub name: String,
	pub pipeline: wgpu::RenderPipeline,
	pub color_targets: Vec<RenderTargetHandle>,
	pub load_op: wgpu::LoadOp<wgpu::Color>,
}

impl RenderPass {
	/// Create new render pass
	///
	/// # Errors
	/// return error if cannot find supported surface format.
	pub fn new(name: &str, device: &wgpu::Device, surface_capability: &wgpu::SurfaceCapabilities) -> ZResult<Self> {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some(name),
			source: wgpu::ShaderSource::Wgsl(include_str!("triangle.wgsl").into()),
		});

		let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some(name),
			bind_group_layouts: &[],
			immediate_size: 0,
		});

		let formats = pick_surface_format(surface_capability)
			.ok_or_else(|| ZenithError::NoSuitableSurfaceFormat("no supported surface format".into()))?;

		let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some(name),
			layout: Some(&render_pipeline_layout),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_main"),
				buffers: &[],
				compilation_options: wgpu::PipelineCompilationOptions::default(),
			},
			fragment: Some(wgpu::FragmentState {
				module: &shader,
				entry_point: Some("fs_main"),
				targets: &[Some(wgpu::ColorTargetState {
					format: formats.view,
					blend: Some(wgpu::BlendState::REPLACE),
					write_mask: wgpu::ColorWrites::ALL,
				})],
				compilation_options: wgpu::PipelineCompilationOptions::default(),
			}),
			primitive: wgpu::PrimitiveState {
				topology: wgpu::PrimitiveTopology::TriangleList,
				strip_index_format: None,
				front_face: wgpu::FrontFace::Ccw,
				cull_mode: Some(wgpu::Face::Back),
				polygon_mode: wgpu::PolygonMode::Fill,
				unclipped_depth: false,
				conservative: false,
			},
			depth_stencil: None,
			multisample: wgpu::MultisampleState {
				count: 1,
				mask: !0,
				alpha_to_coverage_enabled: false,
			},
			multiview_mask: None,
			cache: None, // TODO: Add Cache
		});

		Ok(Self {
			name: name.to_string(),
			pipeline: render_pipeline,
			color_targets: vec![],
			load_op: wgpu::LoadOp::Clear(wgpu::Color {
				r: 0.1,
				g: 0.2,
				b: 0.5,
				a: 1.0,
			}),
		})
	}
}
