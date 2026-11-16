//! Textures on the GPU, with their mipmaps made on the GPU.

use zenith_error::{ZResult, ZenithError};

use super::pipelines::{PipelineKey, Pipelines, SamplerKey};
use crate::assets::import::image::{Filter, ImageData, Pixels, TextureImport, Wrap};

/// A texture on the GPU, with the sampler its import settings ask for.
#[derive(Debug)]
pub struct GpuTexture {
	pub texture: wgpu::Texture,
	/// All of its mip levels.
	pub view: wgpu::TextureView,
	pub sampler: wgpu::Sampler,
	bytes: u64,
}

/// What [`GpuTexture::create`] makes a texture of.
#[derive(Clone, Copy, Debug)]
pub struct TexturePixels<'a> {
	pub width: u32,
	pub height: u32,
	pub format: wgpu::TextureFormat,
	/// Row by row from the top, in `format`.
	pub bytes: &'a [u8],
	pub mipmaps: bool,
	pub sampler: SamplerKey,
}

impl GpuTexture {
	/// Puts a decoded picture on the GPU: colors as sRGB, the rest as they
	/// are, light brighter than white as 16-bit floats.
	///
	/// # Errors
	/// `InvalidAsset` for a picture bigger than the GPU allows, or with fewer
	/// pixels than its size says.
	pub fn upload(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		pipelines: &mut Pipelines,
		label: &str,
		image: &ImageData,
		settings: &TextureImport,
	) -> ZResult<Self> {
		let halves: Vec<u16>;
		let (format, bytes) = match &image.pixels {
			Pixels::Rgba8(bytes) if image.usage.is_srgb() => (wgpu::TextureFormat::Rgba8UnormSrgb, bytes.as_slice()),
			Pixels::Rgba8(bytes) => (wgpu::TextureFormat::Rgba8Unorm, bytes.as_slice()),
			Pixels::Rgba32Float(floats) => {
				halves = floats
					.iter()
					.map(|&value| half::f16::from_f32(value).to_bits())
					.collect();
				(wgpu::TextureFormat::Rgba16Float, bytemuck::cast_slice(&halves))
			}
		};
		Self::create(
			device,
			queue,
			pipelines,
			label,
			&TexturePixels {
				width: image.width,
				height: image.height,
				format,
				bytes,
				mipmaps: settings.mipmaps,
				sampler: sampler_key(settings),
			},
		)
	}

	/// A texture of `pixels`.
	///
	/// # Errors
	/// As [`GpuTexture::upload`].
	pub fn create(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		pipelines: &mut Pipelines,
		label: &str,
		pixels: &TexturePixels<'_>,
	) -> ZResult<Self> {
		let TexturePixels {
			width,
			height,
			format,
			bytes,
			..
		} = *pixels;
		let max = device.limits().max_texture_dimension_2d;
		if width == 0 || height == 0 || width > max || height > max {
			return Err(ZenithError::InvalidAsset(format!(
				"{label}: a texture of {width}x{height} pixels; this GPU takes 1 to {max} a side"
			)));
		}
		let needed = u64::from(width) * u64::from(height) * pixel_size(format);
		if (bytes.len() as u64) < needed {
			return Err(ZenithError::InvalidAsset(format!(
				"{label}: {} bytes for {width}x{height} pixels",
				bytes.len()
			)));
		}
		Ok(Self::create_unchecked(device, queue, pipelines, label, pixels))
	}

	/// [`GpuTexture::create`] for pixels known to fit (the stand-ins).
	pub(crate) fn create_unchecked(
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		pipelines: &mut Pipelines,
		label: &str,
		pixels: &TexturePixels<'_>,
	) -> Self {
		let TexturePixels {
			width,
			height,
			format,
			bytes,
			mipmaps,
			sampler,
		} = *pixels;
		let pixel_size = pixel_size(format);
		let row = u64::from(width) * pixel_size;
		let levels = if mipmaps { mip_levels(width, height) } else { 1 };
		// COPY_SRC: it can be read back (a thumbnail in the editor, a test).
		let mut usage =
			wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC;
		if levels > 1 {
			usage |= wgpu::TextureUsages::RENDER_ATTACHMENT;
		}
		let size = wgpu::Extent3d {
			width,
			height,
			depth_or_array_layers: 1,
		};
		let texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some(label),
			size,
			mip_level_count: levels,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format,
			usage,
			view_formats: &[],
		});
		queue.write_texture(
			wgpu::TexelCopyTextureInfo {
				texture: &texture,
				mip_level: 0,
				origin: wgpu::Origin3d::ZERO,
				aspect: wgpu::TextureAspect::All,
			},
			bytes,
			wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: u32::try_from(row).ok(),
				rows_per_image: Some(height),
			},
			size,
		);
		if levels > 1 {
			generate_mipmaps(device, queue, pipelines, &texture, format, levels);
		}
		let bytes = (0..levels)
			.map(|level| u64::from((width >> level).max(1)) * u64::from((height >> level).max(1)) * pixel_size)
			.sum();
		Self {
			view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
			sampler: pipelines.sampler(&sampler),
			texture,
			bytes,
		}
	}

	/// What it takes of the memory of the GPU.
	pub const fn bytes(&self) -> u64 {
		self.bytes
	}
}

/// The bytes of a pixel of `format` (one of the uncompressed formats the
/// cache makes).
fn pixel_size(format: wgpu::TextureFormat) -> u64 {
	u64::from(format.block_copy_size(None).unwrap_or(4))
}

/// How many mip levels a texture of this size has: down to one pixel.
pub const fn mip_levels(width: u32, height: u32) -> u32 {
	let side = if width > height { width } else { height };
	if side == 0 { 1 } else { side.bit_width() }
}

/// The sampler the import settings of a texture ask for.
pub const fn sampler_key(settings: &TextureImport) -> SamplerKey {
	let filter = match settings.filter {
		Filter::Linear => wgpu::FilterMode::Linear,
		Filter::Nearest => wgpu::FilterMode::Nearest,
	};
	SamplerKey {
		address_mode: match settings.wrap {
			Wrap::Repeat => wgpu::AddressMode::Repeat,
			Wrap::Clamp => wgpu::AddressMode::ClampToEdge,
			Wrap::Mirror => wgpu::AddressMode::MirrorRepeat,
		},
		mag_filter: filter,
		min_filter: filter,
		mipmap_filter: if settings.mipmaps && matches!(settings.filter, Filter::Linear) {
			wgpu::MipmapFilterMode::Linear
		} else {
			wgpu::MipmapFilterMode::Nearest
		},
		compare: None,
	}
}

/// Fills the levels below the first, each from the one above it. The GPU
/// does the sRGB right by itself: it reads linear values from an sRGB
/// texture and writes them back as sRGB.
fn generate_mipmaps(
	device: &wgpu::Device,
	queue: &wgpu::Queue,
	pipelines: &mut Pipelines,
	texture: &wgpu::Texture,
	format: wgpu::TextureFormat,
	levels: u32,
) {
	let shader = pipelines.shader("zenith.mipmap", include_str!("../shaders/mipmap.wgsl"));
	let layout = pipelines.layout(&[
		wgpu::BindGroupLayoutEntry {
			binding: 0,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture {
				sample_type: wgpu::TextureSampleType::Float { filterable: true },
				view_dimension: wgpu::TextureViewDimension::D2,
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
	]);
	let pipeline = pipelines.pipeline(&PipelineKey {
		layouts: vec![layout],
		..PipelineKey::new(shader, format)
	});
	let sampler = pipelines.sampler(&SamplerKey::LINEAR_CLAMP);
	let (Some(pipeline), Some(layout)) = (pipelines.get_pipeline(pipeline), pipelines.get_layout(layout)) else {
		return;
	};

	let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
		label: Some("Zenith Mipmaps"),
	});
	let level_view = |level: u32| {
		texture.create_view(&wgpu::TextureViewDescriptor {
			base_mip_level: level,
			mip_level_count: Some(1),
			..Default::default()
		})
	};
	for level in 1..levels {
		let source = level_view(level - 1);
		let target = level_view(level);
		let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("Zenith Mipmap Source"),
			layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(&source),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::Sampler(&sampler),
				},
			],
		});
		let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("Zenith Mipmap"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: &target,
				resolve_target: None,
				depth_slice: None,
				ops: wgpu::Operations {
					load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
					store: wgpu::StoreOp::Store,
				},
			})],
			depth_stencil_attachment: None,
			occlusion_query_set: None,
			timestamp_writes: None,
			multiview_mask: None,
		});
		pass.set_pipeline(pipeline);
		pass.set_bind_group(0, &bind_group, &[]);
		pass.draw(0..3, 0..1);
	}
	queue.submit([encoder.finish()]);
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::assets::import::image::TextureUsage;

	#[test]
	fn a_texture_has_mip_levels_down_to_one_pixel() {
		assert_eq!(mip_levels(1, 1), 1);
		assert_eq!(mip_levels(2, 1), 2);
		assert_eq!(mip_levels(64, 16), 7);
		assert_eq!(mip_levels(100, 3), 7, "100, 50, 25, 12, 6, 3, 1");
	}

	#[test]
	fn the_sampler_follows_the_import_settings() {
		let color = sampler_key(&TextureImport::for_usage(TextureUsage::Color, "png"));
		assert_eq!(color.address_mode, wgpu::AddressMode::Repeat);
		assert_eq!(color.mipmap_filter, wgpu::MipmapFilterMode::Linear);
		let sky = sampler_key(&TextureImport::for_usage(TextureUsage::Hdr, "hdr"));
		assert_eq!(sky.address_mode, wgpu::AddressMode::ClampToEdge);
		assert_eq!(sky.mipmap_filter, wgpu::MipmapFilterMode::Nearest, "no mipmaps");
		let pixel_art = sampler_key(&TextureImport {
			filter: Filter::Nearest,
			wrap: Wrap::Mirror,
			..TextureImport::default()
		});
		assert_eq!(pixel_art.mag_filter, wgpu::FilterMode::Nearest);
		assert_eq!(pixel_art.address_mode, wgpu::AddressMode::MirrorRepeat);
	}
}
