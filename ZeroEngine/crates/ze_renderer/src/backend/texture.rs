use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use image::GenericImageView;
use wgpu::util::DeviceExt;
use ze_assets::{AssetRef, ResourceManager};
use ze_core::Result;

use super::bind_group;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SpriteMaterialUniform {
	pub tint: [f32; 4],
	pub params: [f32; 4],
}

pub struct TextureResource {
	_texture: wgpu::Texture,
	view: wgpu::TextureView,
	sampler: wgpu::Sampler,
	width: u32,
	height: u32,
}

pub struct SpriteMaterial {
	pub bind_group: wgpu::BindGroup,
	_material_buffer: wgpu::Buffer,
}

pub struct TextureCache {
	textures: HashMap<AssetRef, TextureResource>,
	failed: std::collections::HashSet<AssetRef>,
	fallback: TextureResource,
}

struct RgbaTextureSource<'a> {
	label: &'a str,
	rgba: &'a [u8],
	width: u32,
	height: u32,
}

impl TextureResource {
	pub fn from_bytes(label: &str, bytes: &[u8], device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self> {
		let loaded_image = image::load_from_memory(bytes)?;
		let converted = loaded_image.to_rgba8();
		let size = loaded_image.dimensions();

		let source = RgbaTextureSource {
			label,
			rgba: &converted,
			width: size.0,
			height: size.1,
		};
		Ok(Self::from_rgba(&source, device, queue))
	}

	pub fn fallback(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
		const FALLBACK_RGBA: [u8; 16] = [255, 0, 255, 255, 32, 32, 32, 255, 32, 32, 32, 255, 255, 0, 255, 255];
		let source = RgbaTextureSource {
			label: "Fallback Texture",
			rgba: &FALLBACK_RGBA,
			width: 2,
			height: 2,
		};
		Self::from_rgba(&source, device, queue)
	}

	fn from_rgba(source: &RgbaTextureSource<'_>, device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
		let texture_size = wgpu::Extent3d {
			width: source.width,
			height: source.height,
			depth_or_array_layers: 1,
		};

		let texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some(source.label),
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba8UnormSrgb,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
			size: texture_size,
		});

		queue.write_texture(
			wgpu::TexelCopyTextureInfo {
				texture: &texture,
				mip_level: 0,
				origin: wgpu::Origin3d::ZERO,
				aspect: wgpu::TextureAspect::All,
			},
			source.rgba,
			wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: Some(source.width * 4),
				rows_per_image: Some(source.height),
			},
			texture_size,
		);

		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some(source.label),
			address_mode_u: wgpu::AddressMode::ClampToEdge,
			address_mode_v: wgpu::AddressMode::ClampToEdge,
			address_mode_w: wgpu::AddressMode::ClampToEdge,
			mag_filter: wgpu::FilterMode::Nearest,
			min_filter: wgpu::FilterMode::Nearest,
			..Default::default()
		});

		Self {
			_texture: texture,
			view,
			sampler,
			width: source.width,
			height: source.height,
		}
	}

	pub const fn dimensions(&self) -> (u32, u32) { (self.width, self.height) }
}

impl SpriteMaterial {
	pub fn new(
		label: &str,
		texture: &TextureResource,
		uniform: SpriteMaterialUniform,
		device: &wgpu::Device,
		layout: &wgpu::BindGroupLayout,
	) -> Self {
		let material_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
			label: Some("Sprite Material"),
			contents: bytemuck::bytes_of(&uniform),
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
		});

		let mut builder = bind_group::Builder::new(device);
		builder.set_layout(layout);
		builder.add_material(&texture.view, &texture.sampler, &material_buffer);
		let bind_group = builder.build(label);

		Self {
			bind_group,
			_material_buffer: material_buffer,
		}
	}
}

impl TextureCache {
	pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
		Self {
			textures: HashMap::new(),
			failed: std::collections::HashSet::new(),
			fallback: TextureResource::fallback(device, queue),
		}
	}

	pub fn invalidate(&mut self, asset: &AssetRef) -> bool {
		self.failed.remove(asset);
		self.textures.remove(asset).is_some()
	}

	pub fn invalidate_many<'a>(&mut self, assets: impl IntoIterator<Item = &'a AssetRef>) -> usize {
		let mut invalidated = 0;
		for asset in assets {
			if self.invalidate(asset) {
				invalidated += 1;
			}
		}
		invalidated
	}

	pub fn get_or_load(
		&mut self,
		asset: &AssetRef,
		resources: &ResourceManager,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
	) -> &TextureResource {
		// An empty path means "no texture assigned" (e.g. a freshly created
		// Sprite). This is a valid, expected state, not a load failure, so we
		// must not touch the filesystem or log anything for it.
		if asset.path.is_empty() {
			return &self.fallback;
		}

		// A previous attempt already failed for this asset. Retrying every
		// frame would spam the log with the same error without any chance of
		// succeeding, since nothing about the asset changes between frames.
		// invalidate() clears this so a real fix (e.g. re-importing the file)
		// gets picked up on the next access.
		if self.failed.contains(asset) {
			return &self.fallback;
		}

		if !self.textures.contains_key(asset) {
			let texture = resources
				.bytes(asset)
				.and_then(|bytes| TextureResource::from_bytes(&asset.path, bytes.as_ref(), device, queue));

			match texture {
				Ok(texture) => {
					self.textures.insert(asset.clone(), texture);
				}
				Err(error) => {
					ze_log::warn!("Failed to load texture `{}`: {error:?}", asset.path);
					self.failed.insert(asset.clone());
					return &self.fallback;
				}
			}
		}

		self.textures.get(asset).unwrap_or(&self.fallback)
	}
}
