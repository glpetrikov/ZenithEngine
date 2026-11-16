//! Shaders, bind group layouts, samplers and render pipelines: made once,
//! found again by what they are, never evicted.

use std::{
	collections::HashMap,
	hash::{DefaultHasher, Hash, Hasher},
};

/// A shader of the cache, by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShaderId(u32);

/// A bind group layout of the cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayoutId(u32);

/// A render pipeline of the cache. It stops being valid when its shader is
/// made again (hot reload): [`Pipelines::get_pipeline`] then gives `None`, and
/// the pass asks for it again by its key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PipelineId(u32);

/// Everything that makes a render pipeline. Two passes asking with equal keys
/// share one pipeline.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PipelineKey {
	pub shader: ShaderId,
	pub vertex_entry: &'static str,
	/// `None`: no fragment stage (a depth-only pass).
	pub fragment_entry: Option<&'static str>,
	/// The bind groups, in the order of their `@group`.
	pub layouts: Vec<LayoutId>,
	pub vertex_buffers: Vec<wgpu::VertexBufferLayout<'static>>,
	pub targets: Vec<Option<wgpu::ColorTargetState>>,
	pub depth_stencil: Option<wgpu::DepthStencilState>,
	pub primitive: wgpu::PrimitiveState,
	pub multisample: wgpu::MultisampleState,
}

impl PipelineKey {
	/// A pipeline of `shader` with one color target of `format`, nothing else
	/// (no vertex buffers, no bind groups, no depth): the rest is filled in
	/// with struct update syntax.
	pub fn new(shader: ShaderId, format: wgpu::TextureFormat) -> Self {
		Self {
			shader,
			vertex_entry: "vs_main",
			fragment_entry: Some("fs_main"),
			layouts: Vec::new(),
			vertex_buffers: Vec::new(),
			targets: vec![Some(format.into())],
			depth_stencil: None,
			primitive: wgpu::PrimitiveState::default(),
			multisample: wgpu::MultisampleState::default(),
		}
	}
}

/// What makes a sampler. `wgpu::SamplerDescriptor` has floats and cannot be a
/// key; the level of detail is not clamped here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SamplerKey {
	pub address_mode: wgpu::AddressMode,
	pub mag_filter: wgpu::FilterMode,
	pub min_filter: wgpu::FilterMode,
	pub mipmap_filter: wgpu::MipmapFilterMode,
	/// A comparison sampler (shadows).
	pub compare: Option<wgpu::CompareFunction>,
}

impl SamplerKey {
	/// Linear filtering, clamped at the edges.
	pub const LINEAR_CLAMP: Self = Self {
		address_mode: wgpu::AddressMode::ClampToEdge,
		mag_filter: wgpu::FilterMode::Linear,
		min_filter: wgpu::FilterMode::Linear,
		mipmap_filter: wgpu::MipmapFilterMode::Linear,
		compare: None,
	};
}

struct Shader {
	id: ShaderId,
	source_hash: u64,
	module: wgpu::ShaderModule,
}

/// The pipelines and what they are made of.
pub struct Pipelines {
	device: wgpu::Device,
	shaders: HashMap<String, Shader>,
	shader_names: Vec<String>,
	layout_ids: HashMap<Vec<wgpu::BindGroupLayoutEntry>, LayoutId>,
	layouts: Vec<wgpu::BindGroupLayout>,
	pipeline_layouts: HashMap<Vec<LayoutId>, wgpu::PipelineLayout>,
	samplers: HashMap<SamplerKey, wgpu::Sampler>,
	ids: HashMap<PipelineKey, PipelineId>,
	made: HashMap<PipelineId, wgpu::RenderPipeline>,
	next_pipeline: u32,
}

impl Pipelines {
	pub(crate) fn new(device: wgpu::Device) -> Self {
		Self {
			device,
			shaders: HashMap::new(),
			shader_names: Vec::new(),
			layout_ids: HashMap::new(),
			layouts: Vec::new(),
			pipeline_layouts: HashMap::new(),
			samplers: HashMap::new(),
			ids: HashMap::new(),
			made: HashMap::new(),
			next_pipeline: 0,
		}
	}

	/// The shader `name` with the WGSL `source`. The same name with the same
	/// source is the same shader. Another source makes it again under the same
	/// id (hot reload), and the pipelines made with the old one are dropped.
	///
	/// A source that does not compile is an error of the device (the built-in
	/// shaders are tested).
	pub fn shader(&mut self, name: &str, source: &str) -> ShaderId {
		let source_hash = {
			let mut hasher = DefaultHasher::new();
			source.hash(&mut hasher);
			hasher.finish()
		};
		if let Some(shader) = self.shaders.get(name)
			&& shader.source_hash == source_hash
		{
			return shader.id;
		}
		let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some(name),
			source: wgpu::ShaderSource::Wgsl(source.into()),
		});
		let id = if let Some(old) = self.shaders.get(name) {
			let id = old.id;
			self.drop_pipelines_of(id);
			id
		} else {
			let id = ShaderId(u32::try_from(self.shader_names.len()).unwrap_or(u32::MAX));
			self.shader_names.push(name.to_owned());
			id
		};
		self.shaders.insert(
			name.to_owned(),
			Shader {
				id,
				source_hash,
				module,
			},
		);
		id
	}

	fn module(&self, id: ShaderId) -> Option<&wgpu::ShaderModule> {
		let name = self.shader_names.get(id.0 as usize)?;
		self.shaders.get(name).map(|shader| &shader.module)
	}

	fn drop_pipelines_of(&mut self, shader: ShaderId) {
		let pipelines = &mut self.made;
		self.ids.retain(|key, id| {
			let keep = key.shader != shader;
			if !keep {
				pipelines.remove(id);
			}
			keep
		});
	}

	/// The bind group layout with `entries`, made the first time it is asked
	/// for.
	pub fn layout(&mut self, entries: &[wgpu::BindGroupLayoutEntry]) -> LayoutId {
		if let Some(&id) = self.layout_ids.get(entries) {
			return id;
		}
		let id = LayoutId(u32::try_from(self.layouts.len()).unwrap_or(u32::MAX));
		self.layouts
			.push(self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
				label: Some("Zenith Bind Group Layout"),
				entries,
			}));
		self.layout_ids.insert(entries.to_vec(), id);
		id
	}

	pub fn get_layout(&self, id: LayoutId) -> Option<&wgpu::BindGroupLayout> {
		self.layouts.get(id.0 as usize)
	}

	/// The sampler `key` describes.
	pub fn sampler(&mut self, key: &SamplerKey) -> wgpu::Sampler {
		self.samplers
			.entry(*key)
			.or_insert_with(|| {
				self.device.create_sampler(&wgpu::SamplerDescriptor {
					label: Some("Zenith Sampler"),
					address_mode_u: key.address_mode,
					address_mode_v: key.address_mode,
					address_mode_w: key.address_mode,
					mag_filter: key.mag_filter,
					min_filter: key.min_filter,
					mipmap_filter: key.mipmap_filter,
					compare: key.compare,
					..Default::default()
				})
			})
			.clone()
	}

	/// The pipeline `key` describes, made the first time it is asked for.
	///
	/// A key with a shader that is not of this cache, or one the shader does
	/// not fit, is an error of the device: wgpu reports it through its error
	/// handler, as it does any bad pipeline.
	pub fn pipeline(&mut self, key: &PipelineKey) -> PipelineId {
		if let Some(&id) = self.ids.get(key) {
			return id;
		}
		let id = PipelineId(self.next_pipeline);
		self.next_pipeline += 1;
		let pipeline = self.make_pipeline(key);
		self.made.insert(id, pipeline);
		self.ids.insert(key.clone(), id);
		id
	}

	fn make_pipeline(&mut self, key: &PipelineKey) -> wgpu::RenderPipeline {
		if !self.pipeline_layouts.contains_key(&key.layouts) {
			let layouts: Vec<Option<&wgpu::BindGroupLayout>> =
				key.layouts.iter().map(|&id| self.get_layout(id)).collect();
			let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
				label: Some("Zenith Pipeline Layout"),
				bind_group_layouts: &layouts,
				immediate_size: 0,
			});
			self.pipeline_layouts.insert(key.layouts.clone(), layout);
		}
		let layout = self.pipeline_layouts.get(&key.layouts);
		let name = self.shader_names.get(key.shader.0 as usize).map_or("?", String::as_str);
		let Some(module) = self.module(key.shader) else {
			// Not a shader of this cache: wgpu reports the error of an empty
			// module, the way it reports any other bad pipeline.
			let empty = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
				label: Some("missing shader"),
				source: wgpu::ShaderSource::Wgsl("".into()),
			});
			return self.create(key, layout, &empty, name);
		};
		self.create(key, layout, module, name)
	}

	fn create(
		&self,
		key: &PipelineKey,
		layout: Option<&wgpu::PipelineLayout>,
		module: &wgpu::ShaderModule,
		name: &str,
	) -> wgpu::RenderPipeline {
		let buffers: Vec<Option<wgpu::VertexBufferLayout<'static>>> =
			key.vertex_buffers.iter().cloned().map(Some).collect();
		self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some(name),
			layout,
			vertex: wgpu::VertexState {
				module,
				entry_point: Some(key.vertex_entry),
				compilation_options: wgpu::PipelineCompilationOptions::default(),
				buffers: &buffers,
			},
			primitive: key.primitive,
			depth_stencil: key.depth_stencil.clone(),
			multisample: key.multisample,
			fragment: key.fragment_entry.map(|entry| wgpu::FragmentState {
				module,
				entry_point: Some(entry),
				compilation_options: wgpu::PipelineCompilationOptions::default(),
				targets: &key.targets,
			}),
			multiview_mask: None,
			cache: None,
		})
	}

	/// The pipeline `id`, unless its shader was made again since.
	pub fn get_pipeline(&self, id: PipelineId) -> Option<&wgpu::RenderPipeline> {
		self.made.get(&id)
	}

	/// How many pipelines there are.
	pub fn pipeline_count(&self) -> usize {
		self.made.len()
	}
}
