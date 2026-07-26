use std::path::PathBuf;

pub enum ShaderSource {
	Path(PathBuf),
	Source(String),
}

#[allow(dead_code)]
pub struct Pipeline {
	pub render_pipeline: wgpu::RenderPipeline,
	pub name: String,
}

pub struct Builder<'a> {
	shader_source: Option<ShaderSource>,
	vertex_entry: String,
	fragment_entry: String,
	pixel_formats: Vec<wgpu::TextureFormat>,
	blend_states: Vec<Option<wgpu::BlendState>>,
	vertex_buffer_layouts: Vec<wgpu::VertexBufferLayout<'static>>,
	topology: wgpu::PrimitiveTopology,
	polygon_mode: wgpu::PolygonMode,
	cull_mode: Option<wgpu::Face>,
	depth_write_enabled: bool,
	depth_compare: wgpu::CompareFunction,
	depth_stencil_enabled: bool,
	name: String,
	bind_group_layouts: Vec<Option<&'a wgpu::BindGroupLayout>>,
	device: &'a wgpu::Device,
}

#[allow(dead_code)]
impl<'a> Builder<'a> {
	// TODO: add shader_from_filepath(path, fs_main, vs_main) and
	// shader_from_source(source, fs_main, vs_main)
	pub fn new(device: &'a wgpu::Device) -> Self {
		Self {
			shader_source: None,
			vertex_entry: "vs_main".to_string(),
			fragment_entry: "fs_main".to_string(),
			pixel_formats: vec![wgpu::TextureFormat::Bgra8UnormSrgb],
			blend_states: vec![Some(wgpu::BlendState::ALPHA_BLENDING)],
			vertex_buffer_layouts: vec![],
			topology: wgpu::PrimitiveTopology::TriangleList,
			polygon_mode: wgpu::PolygonMode::Fill,
			cull_mode: None, // Some(wgpu::Face::Back),
			depth_write_enabled: true,
			depth_compare: wgpu::CompareFunction::Less,
			depth_stencil_enabled: true,
			name: "Unnamed Pipeline".to_string(),
			bind_group_layouts: vec![],
			device,
		}
	}

	pub fn with_name(mut self, name: impl Into<String>) -> Self {
		self.name = name.into();
		self
	}

	pub fn with_shader_path(mut self, path: impl Into<PathBuf>) -> Self {
		self.shader_source = Some(ShaderSource::Path(path.into()));
		self
	}

	pub fn with_shader_source(mut self, source: impl Into<String>) -> Self {
		self.shader_source = Some(ShaderSource::Source(source.into()));
		self
	}

	pub fn with_shader(
		mut self,
		path: impl Into<PathBuf>,
		vertex_entry: impl Into<String>,
		fragment_entry: impl Into<String>,
	) -> Self {
		self.shader_source = Some(ShaderSource::Path(path.into()));
		self.vertex_entry = vertex_entry.into();
		self.fragment_entry = fragment_entry.into();
		self
	}

	pub fn with_vertex_entry(mut self, entry: impl Into<String>) -> Self {
		self.vertex_entry = entry.into();
		self
	}

	pub fn with_fragment_entry(mut self, entry: impl Into<String>) -> Self {
		self.fragment_entry = entry.into();
		self
	}

	pub fn with_pixel_format(mut self, format: wgpu::TextureFormat) -> Self {
		self.pixel_formats = vec![format];
		self.blend_states.resize(1, Some(wgpu::BlendState::ALPHA_BLENDING));
		self
	}

	pub fn with_pixel_formats(mut self, formats: impl Into<Vec<wgpu::TextureFormat>>) -> Self {
		self.pixel_formats = formats.into();
		self.blend_states
			.resize(self.pixel_formats.len(), Some(wgpu::BlendState::ALPHA_BLENDING));
		self
	}

	/// Fills every attachment's blend state with the same value. For
	/// per-attachment control (e.g. a `GBuffer` pass where some attachments
	/// blend and others don't), use `with_blend_states` instead.
	pub fn with_blend_state(mut self, blend: Option<wgpu::BlendState>) -> Self {
		self.blend_states = vec![blend; self.pixel_formats.len().max(1)];
		self
	}

	pub fn with_blend_states(mut self, blends: impl Into<Vec<Option<wgpu::BlendState>>>) -> Self {
		self.blend_states = blends.into();
		self
	}

	pub fn with_vertex_buffer_layout(mut self, layout: wgpu::VertexBufferLayout<'static>) -> Self {
		self.vertex_buffer_layouts.push(layout);
		self
	}

	pub const fn with_topology(mut self, topology: wgpu::PrimitiveTopology) -> Self {
		self.topology = topology;
		self
	}

	pub const fn with_polygon_mode(mut self, polygon_mode: wgpu::PolygonMode) -> Self {
		self.polygon_mode = polygon_mode;
		self
	}

	pub const fn with_cull_mode(mut self, cull_mode: Option<wgpu::Face>) -> Self {
		self.cull_mode = cull_mode;
		self
	}

	pub const fn with_depth_write_enabled(mut self, enabled: bool) -> Self {
		self.depth_write_enabled = enabled;
		self
	}

	pub const fn with_depth_compare(mut self, compare: wgpu::CompareFunction) -> Self {
		self.depth_compare = compare;
		self
	}

	pub const fn with_depth_stencil_enabled(mut self, enabled: bool) -> Self {
		self.depth_stencil_enabled = enabled;
		self
	}

	pub fn with_bind_group_layout(mut self, layout: &'a wgpu::BindGroupLayout) -> Self {
		self.bind_group_layouts.push(Some(layout));
		self
	}

	pub fn build(self) -> ze_core::Result<Pipeline> {
		let source_code = match self.shader_source {
			Some(ShaderSource::Path(_path)) => {
				ze_core::bail!("file-based shader loading is not supported in release builds");
			}
			Some(ShaderSource::Source(source)) => source,
			None => {
				ze_core::bail!("Pipeline `{}` has no shader source", self.name);
			}
		};

		let shader_module_name = format!("{} Shader", self.name);

		let shader_module_descriptor = wgpu::ShaderModuleDescriptor {
			label: Some(&shader_module_name),
			source: wgpu::ShaderSource::Wgsl(source_code.into()),
		};

		let shader_module = self.device.create_shader_module(shader_module_descriptor);

		let pipeline_layout_name = format!("{} Pipeline Layout", self.name);

		let pipeline_layout_descriptor = wgpu::PipelineLayoutDescriptor {
			label: Some(pipeline_layout_name.as_str()),
			bind_group_layouts: &self.bind_group_layouts,
			immediate_size: 0,
		};
		let pipeline_layout = self.device.create_pipeline_layout(&pipeline_layout_descriptor);

		let render_targets: Vec<Option<wgpu::ColorTargetState>> = self
			.pixel_formats
			.iter()
			.zip(self.blend_states.iter())
			.map(|(format, blend)| {
				Some(wgpu::ColorTargetState {
					format: *format,
					blend: *blend,
					write_mask: wgpu::ColorWrites::ALL,
				})
			})
			.collect();

		let depth_stencil = self.depth_stencil_enabled.then_some(wgpu::DepthStencilState {
			format: wgpu::TextureFormat::Depth32Float,
			depth_write_enabled: Some(self.depth_write_enabled),
			depth_compare: Some(self.depth_compare),
			stencil: wgpu::StencilState::default(),
			bias: wgpu::DepthBiasState::default(),
		});

		let render_pipeline_name = format!("{} Render Pipeline", self.name);

		let render_pipeline_descriptor = wgpu::RenderPipelineDescriptor {
			label: Some(render_pipeline_name.as_str()),
			layout: Some(&pipeline_layout),
			vertex: wgpu::VertexState {
				module: &shader_module,
				entry_point: Some(&self.vertex_entry),
				buffers: &self.vertex_buffer_layouts,
				compilation_options: wgpu::PipelineCompilationOptions::default(),
			},
			fragment: Some(wgpu::FragmentState {
				module: &shader_module,
				entry_point: Some(&self.fragment_entry),
				targets: &render_targets,
				compilation_options: wgpu::PipelineCompilationOptions::default(),
			}),
			primitive: wgpu::PrimitiveState {
				topology: self.topology,
				strip_index_format: None,
				front_face: wgpu::FrontFace::Ccw,
				cull_mode: self.cull_mode,
				polygon_mode: self.polygon_mode,
				unclipped_depth: false,
				conservative: false,
			},
			depth_stencil,
			multisample: wgpu::MultisampleState {
				count: 1,
				mask: !0,
				alpha_to_coverage_enabled: false,
			},
			multiview_mask: None,
			cache: None,
		};

		let render_pipeline = self.device.create_render_pipeline(&render_pipeline_descriptor);

		let pipeline = Pipeline {
			render_pipeline,
			name: self.name.clone(),
		};

		Ok(pipeline)
	}
}
