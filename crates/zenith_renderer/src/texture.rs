use image::GenericImageView;
use zenith_error::{ZResult, ZenithError};

pub struct Texture {
	pub texture: wgpu::Texture,
	pub view: wgpu::TextureView,
	pub sampler: wgpu::Sampler,
}

impl Texture {
	/// The format of every depth buffer in the renderer.
	pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

	/// ## Creates a depth buffer of the size of the surface
	///
	/// It has to be made again when the surface changes its size: a depth
	/// buffer smaller or bigger than the color attachment is an error.
	#[must_use]
	pub fn create_depth_texture(device: &wgpu::Device, width: u32, height: u32, label: &str) -> Self {
		let size = wgpu::Extent3d {
			width: width.max(1),
			height: height.max(1),
			depth_or_array_layers: 1,
		};
		let texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some(label),
			size,
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: Self::DEPTH_FORMAT,
			// RENDER_ATTACHMENT: the passes draw into it. TEXTURE_BINDING: a
			// later pass (a shadow, a fog) can read it.
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		});

		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			address_mode_u: wgpu::AddressMode::ClampToEdge,
			address_mode_v: wgpu::AddressMode::ClampToEdge,
			address_mode_w: wgpu::AddressMode::ClampToEdge,
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			mipmap_filter: wgpu::MipmapFilterMode::Nearest,
			compare: Some(wgpu::CompareFunction::LessEqual),
			lod_min_clamp: 0.0,
			lod_max_clamp: 100.0,
			..Default::default()
		});

		Self { texture, view, sampler }
	}

	/// ## Creates a texture from bytes
	///
	/// `is_normal_map` says what the pixels are. Colors are stored in sRGB and
	/// the GPU turns them linear when it reads them; the pixels of a normal map
	/// are *directions*, which must be read as they are.
	///
	/// ## Errors
	/// if image can't make image from bytes
	pub fn from_bytes(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		bytes: &[u8],
		label: &str,
		is_normal_map: bool,
	) -> ZResult<Self> {
		let img = image::load_from_memory(bytes).map_err(|e| ZenithError::ImageDecodeFailed(e.to_string()))?;
		Self::from_image(device, queue, &img, Some(label), is_normal_map)
	}

	/// ## Creates a texture from `image`'s `Image`
	///
	/// ## Errors
	/// Empty
	pub fn from_image(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		img: &image::DynamicImage,
		label: Option<&str>,
		is_normal_map: bool,
	) -> ZResult<Self> {
		let rgba = img.to_rgba8();
		let (width, height) = img.dimensions();
		Ok(Self::from_rgba8(
			device,
			queue,
			&rgba,
			width,
			height,
			label,
			is_normal_map,
		))
	}

	/// A texture of one color. It stands in for a texture a material does not
	/// have: white for a color, `(0.5, 0.5, 1.0)` (the normal does not tilt)
	/// for a normal map.
	#[must_use]
	pub fn from_color(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		rgba: [u8; 4],
		label: &str,
		is_normal_map: bool,
	) -> Self {
		Self::from_rgba8(device, queue, &rgba, 1, 1, Some(label), is_normal_map)
	}

	fn from_rgba8(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		rgba: &[u8],
		width: u32,
		height: u32,
		label: Option<&str>,
		is_normal_map: bool,
	) -> Self {
		let size = wgpu::Extent3d {
			width,
			height,
			depth_or_array_layers: 1,
		};
		let texture = device.create_texture(&wgpu::TextureDescriptor {
			label,
			size,
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: if is_normal_map {
				wgpu::TextureFormat::Rgba8Unorm
			} else {
				wgpu::TextureFormat::Rgba8UnormSrgb
			},
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});

		queue.write_texture(
			wgpu::TexelCopyTextureInfo {
				aspect: wgpu::TextureAspect::All,
				texture: &texture,
				mip_level: 0,
				origin: wgpu::Origin3d::ZERO,
			},
			rgba,
			wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: Some(4 * width),
				rows_per_image: Some(height),
			},
			size,
		);

		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			address_mode_u: wgpu::AddressMode::Repeat,
			address_mode_v: wgpu::AddressMode::Repeat,
			address_mode_w: wgpu::AddressMode::Repeat,
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			mipmap_filter: wgpu::MipmapFilterMode::Nearest,
			..Default::default()
		});

		Self { texture, view, sampler }
	}
}
