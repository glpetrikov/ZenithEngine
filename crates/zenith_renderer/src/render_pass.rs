use crate::{render_target::RenderTargetHandle, texture::Texture};

/// A render pipeline with the name and the clear color of the pass that uses
/// it.
pub struct RenderPass {
	// TODO: add color for future imnodes render graph editor
	// TODO: add reads/writes and other things
	pub name: String,
	pub pipeline: wgpu::RenderPipeline,
	pub color_targets: Vec<RenderTargetHandle>,
	pub load_op: wgpu::LoadOp<wgpu::Color>,
}

impl RenderPass {
	#[must_use]
	pub fn new(name: &str, pipeline: wgpu::RenderPipeline) -> Self {
		Self {
			name: name.to_string(),
			pipeline,
			color_targets: vec![],
			load_op: wgpu::LoadOp::Clear(wgpu::Color {
				r: 0.1,
				g: 0.2,
				b: 0.5,
				a: 1.0,
			}),
		}
	}
}

/// What differs between the pipelines of the renderer; the rest of a pipeline
/// (triangle lists, counter-clockwise front faces, no blending) is the same.
pub struct PipelineDesc<'a> {
	pub label: &'a str,
	pub layout: &'a wgpu::PipelineLayout,
	pub shader: &'a wgpu::ShaderModule,
	pub vertex_layouts: &'a [wgpu::VertexBufferLayout<'a>],
	pub color_format: wgpu::TextureFormat,
	/// `None`: no depth buffer.
	pub depth: Option<DepthDesc>,
	/// `None`: both sides of a triangle are drawn.
	pub cull_mode: Option<wgpu::Face>,
	pub vs_entry: &'a str,
	pub fs_entry: &'a str,
}

#[derive(Clone, Copy)]
pub struct DepthDesc {
	pub write: bool,
	pub compare: wgpu::CompareFunction,
}

impl DepthDesc {
	/// Closer fragments hide those behind them, and are remembered.
	pub const SOLID: Self = Self {
		write: true,
		compare: wgpu::CompareFunction::Less,
	};
}

#[must_use]
pub fn create_render_pipeline(device: &wgpu::Device, desc: &PipelineDesc<'_>) -> wgpu::RenderPipeline {
	device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
		label: Some(desc.label),
		layout: Some(desc.layout),
		vertex: wgpu::VertexState {
			module: desc.shader,
			entry_point: Some(desc.vs_entry),
			buffers: &desc.vertex_layouts.iter().cloned().map(Some).collect::<Vec<_>>(),
			compilation_options: wgpu::PipelineCompilationOptions::default(),
		},
		fragment: Some(wgpu::FragmentState {
			module: desc.shader,
			entry_point: Some(desc.fs_entry),
			targets: &[Some(wgpu::ColorTargetState {
				format: desc.color_format,
				blend: Some(wgpu::BlendState::REPLACE),
				write_mask: wgpu::ColorWrites::ALL,
			})],
			compilation_options: wgpu::PipelineCompilationOptions::default(),
		}),
		primitive: wgpu::PrimitiveState {
			topology: wgpu::PrimitiveTopology::TriangleList,
			strip_index_format: None,
			front_face: wgpu::FrontFace::Ccw,
			cull_mode: desc.cull_mode,
			polygon_mode: wgpu::PolygonMode::Fill,
			unclipped_depth: false,
			conservative: false,
		},
		depth_stencil: desc.depth.map(|depth| wgpu::DepthStencilState {
			format: Texture::DEPTH_FORMAT,
			depth_write_enabled: Some(depth.write),
			depth_compare: Some(depth.compare),
			stencil: wgpu::StencilState::default(),
			bias: wgpu::DepthBiasState::default(),
		}),
		multisample: wgpu::MultisampleState {
			count: 1,
			mask: !0,
			alpha_to_coverage_enabled: false,
		},
		multiview_mask: None,
		cache: None, // TODO: Add Cache
	})
}
