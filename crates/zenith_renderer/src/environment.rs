//! The environment: a picture of the sky that is seen behind the scene and
//! mirrored in it.

use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{Project, ZPath};

/// A cube map: six square pictures, the faces of a cube seen from inside.
pub struct CubeTexture {
	pub texture: wgpu::Texture,
	pub view: wgpu::TextureView,
	pub sampler: wgpu::Sampler,
}

/// Makes cube maps out of equirectangular HDR pictures.
pub struct HdrLoader {
	layout: wgpu::BindGroupLayout,
	pipeline: wgpu::ComputePipeline,
}

impl HdrLoader {
	/// The format of the cube. Half floats are filterable everywhere (32-bit
	/// ones are not, without an extra feature) and can be written by a shader.
	const CUBE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

	#[must_use]
	pub fn new(device: &wgpu::Device) -> Self {
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("HdrLoader Bind Group Layout"),
			entries: &[
				// The picture. A 32-bit float texture: read with `textureLoad`.
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::COMPUTE,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Float { filterable: false },
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
				// The six faces, which the shader writes.
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::COMPUTE,
					ty: wgpu::BindingType::StorageTexture {
						access: wgpu::StorageTextureAccess::WriteOnly,
						format: Self::CUBE_FORMAT,
						view_dimension: wgpu::TextureViewDimension::D2Array,
					},
					count: None,
				},
			],
		});

		let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/equirectangular.wgsl"));
		let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("HdrLoader Pipeline Layout"),
			bind_group_layouts: &[Some(&layout)],
			immediate_size: 0,
		});
		let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
			label: Some("HdrLoader Pipeline"),
			layout: Some(&pipeline_layout),
			module: &shader,
			entry_point: Some("compute_equirect_to_cubemap"),
			compilation_options: wgpu::PipelineCompilationOptions::default(),
			cache: None,
		});

		Self { layout, pipeline }
	}

	/// ## Makes a cube map from the bytes of a Radiance `.hdr` picture
	///
	/// `dst_size` is the side of a face, in pixels.
	///
	/// ## Errors
	/// `ImageDecodeFailed` if the bytes are not an HDR picture, or it is too
	/// big for the device.
	pub fn from_equirectangular_bytes(
		&self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		data: &[u8],
		dst_size: u32,
		label: &str,
	) -> ZResult<CubeTexture> {
		let picture = image::load_from_memory_with_format(data, image::ImageFormat::Hdr)
			.map_err(|e| ZenithError::ImageDecodeFailed(format!("{label}: {e}")))?
			.to_rgba32f();
		let (width, height) = picture.dimensions();

		let max = device.limits().max_texture_dimension_2d;
		if width > max || height > max || dst_size > max {
			return Err(ZenithError::ImageDecodeFailed(format!(
				"{label}: {width}x{height} (cube {dst_size}) is more than the device can hold ({max})"
			)));
		}

		let src_size = wgpu::Extent3d {
			width,
			height,
			depth_or_array_layers: 1,
		};
		let src = device.create_texture(&wgpu::TextureDescriptor {
			label: Some(&format!("{label} (equirectangular)")),
			size: src_size,
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba32Float,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		queue.write_texture(
			wgpu::TexelCopyTextureInfo {
				texture: &src,
				mip_level: 0,
				origin: wgpu::Origin3d::ZERO,
				aspect: wgpu::TextureAspect::All,
			},
			bytemuck::cast_slice(&picture),
			wgpu::TexelCopyBufferLayout {
				offset: 0,
				// Four floats of four bytes a pixel.
				bytes_per_row: Some(width * 16),
				rows_per_image: Some(height),
			},
			src_size,
		);
		let src_view = src.create_view(&wgpu::TextureViewDescriptor::default());

		let texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some(label),
			size: wgpu::Extent3d {
				width: dst_size,
				height: dst_size,
				depth_or_array_layers: 6,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: Self::CUBE_FORMAT,
			usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		});
		// One texture, two ways to see it: as six layers to write to, and as a
		// cube to sample from.
		let dst_view = texture.create_view(&wgpu::TextureViewDescriptor {
			label: Some(label),
			dimension: Some(wgpu::TextureViewDimension::D2Array),
			array_layer_count: Some(6),
			..Default::default()
		});
		let view = texture.create_view(&wgpu::TextureViewDescriptor {
			label: Some(label),
			dimension: Some(wgpu::TextureViewDimension::Cube),
			array_layer_count: Some(6),
			..Default::default()
		});
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			address_mode_u: wgpu::AddressMode::ClampToEdge,
			address_mode_v: wgpu::AddressMode::ClampToEdge,
			address_mode_w: wgpu::AddressMode::ClampToEdge,
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			mipmap_filter: wgpu::MipmapFilterMode::Nearest,
			..Default::default()
		});

		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some(&format!("{label} Bind Group")),
			layout: &self.layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(&src_view),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::TextureView(&dst_view),
				},
			],
		});

		let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
			label: Some(&format!("{label} Encoder")),
		});
		{
			let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
				label: Some(&format!("{label} Pass")),
				timestamp_writes: None,
			});
			pass.set_pipeline(&self.pipeline);
			pass.set_bind_group(0, &bind_group, &[]);
			// The groups are 16 x 16 texels; the sixth of `z` is the sixth
			// face.
			let groups = dst_size.div_ceil(16);
			pass.dispatch_workgroups(groups, groups, 6);
		}
		queue.submit([encoder.finish()]);

		Ok(CubeTexture { texture, view, sampler })
	}
}

/// The sky of the scene: a cube map and the bind group the shaders read it by.
pub struct Environment {
	pub cube: CubeTexture,
	pub bind_group: wgpu::BindGroup,
}

impl Environment {
	/// The side of a face of the cube.
	const FACE_SIZE: u32 = 512;

	/// The layout of [`Environment::bind_group`]: a cube map and its sampler.
	#[must_use]
	pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
		device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("Environment Bind Group Layout"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Float { filterable: true },
						view_dimension: wgpu::TextureViewDimension::Cube,
						multisampled: false,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
					count: None,
				},
			],
		})
	}

	/// ## Loads the sky from an equirectangular `.hdr` of the project
	///
	/// ## Errors
	/// The project's error if the file cannot be read, `ImageDecodeFailed` if
	/// it is not a valid HDR picture.
	pub fn load(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		project: &Project,
		path: &ZPath,
		layout: &wgpu::BindGroupLayout,
	) -> ZResult<Self> {
		let bytes = project.read_bytes(path)?;
		let cube = HdrLoader::new(device).from_equirectangular_bytes(
			device,
			queue,
			&bytes,
			Self::FACE_SIZE,
			&path.to_string(),
		)?;

		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("Environment Bind Group"),
			layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(&cube.view),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::Sampler(&cube.sampler),
				},
			],
		});

		Ok(Self { cube, bind_group })
	}
}
