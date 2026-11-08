//! High dynamic range: the scene is drawn into a texture that can hold light
//! brighter than white, and a last pass maps it to what the screen can show.

/// The texture the scene is drawn into, and the pass that tone maps it onto
/// the screen.
pub struct HdrPipeline {
	pipeline: wgpu::RenderPipeline,
	bind_group: wgpu::BindGroup,
	layout: wgpu::BindGroupLayout,
	texture: wgpu::Texture,
	view: wgpu::TextureView,
	sampler: wgpu::Sampler,
}

impl HdrPipeline {
	/// The format of the scene: 16-bit floats, so a value is not stopped at
	/// `1.0` the way it is in the 8 bits of the screen.
	pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

	/// `output_format` is the format of the view `process` writes to.
	#[must_use]
	pub fn new(device: &wgpu::Device, width: u32, height: u32, output_format: wgpu::TextureFormat) -> Self {
		let (texture, view) = Self::create_texture(device, width, height);

		// One texel of the scene per pixel of the screen: nothing to filter.
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			address_mode_u: wgpu::AddressMode::ClampToEdge,
			address_mode_v: wgpu::AddressMode::ClampToEdge,
			address_mode_w: wgpu::AddressMode::ClampToEdge,
			mag_filter: wgpu::FilterMode::Nearest,
			min_filter: wgpu::FilterMode::Nearest,
			..Default::default()
		});

		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("HDR Bind Group Layout"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Float { filterable: false },
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
					count: None,
				},
			],
		});
		let bind_group = Self::create_bind_group(device, &layout, &view, &sampler);

		let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("HDR Pipeline Layout"),
			bind_group_layouts: &[Some(&layout)],
			immediate_size: 0,
		});
		let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/hdr.wgsl"));
		// No vertex buffer, no depth, no culling: the vertex shader makes one
		// triangle that covers the screen out of the vertex index.
		let pipeline = crate::render_pass::create_render_pipeline(
			device,
			&crate::render_pass::PipelineDesc {
				label: "HDR Pipeline",
				layout: &pipeline_layout,
				shader: &shader,
				vertex_layouts: &[],
				color_format: output_format,
				depth: None,
				cull_mode: None,
				vs_entry: "vs_main",
				fs_entry: "fs_main",
			},
		);

		Self {
			pipeline,
			bind_group,
			layout,
			texture,
			view,
			sampler,
		}
	}

	fn create_texture(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
		let texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("HDR Texture"),
			size: wgpu::Extent3d {
				width: width.max(1),
				height: height.max(1),
				depth_or_array_layers: 1,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: Self::FORMAT,
			// The scene is drawn into it, the tone mapping reads it.
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		});
		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		(texture, view)
	}

	fn create_bind_group(
		device: &wgpu::Device,
		layout: &wgpu::BindGroupLayout,
		view: &wgpu::TextureView,
		sampler: &wgpu::Sampler,
	) -> wgpu::BindGroup {
		device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("HDR Bind Group"),
			layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(view),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::Sampler(sampler),
				},
			],
		})
	}

	/// The size of the screen changed: the texture follows it.
	pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
		let (texture, view) = Self::create_texture(device, width, height);
		self.bind_group = Self::create_bind_group(device, &self.layout, &view, &self.sampler);
		self.texture = texture;
		self.view = view;
	}

	/// Where the scene is drawn.
	#[must_use]
	pub const fn view(&self) -> &wgpu::TextureView {
		&self.view
	}

	#[must_use]
	pub const fn texture(&self) -> &wgpu::Texture {
		&self.texture
	}

	/// Tone maps the scene onto `output`.
	pub fn process(&self, encoder: &mut wgpu::CommandEncoder, output: &wgpu::TextureView) {
		let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("HDR Tone Mapping Pass"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: output,
				resolve_target: None,
				depth_slice: None,
				ops: wgpu::Operations {
					load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
					store: wgpu::StoreOp::Store,
				},
			})],
			depth_stencil_attachment: None,
			occlusion_query_set: None,
			timestamp_writes: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&self.pipeline);
		pass.set_bind_group(0, &self.bind_group, &[]);
		pass.draw(0..3, 0..1);
	}
}
