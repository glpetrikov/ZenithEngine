use std::collections::{HashMap, HashSet};

use ze_assets::{AssetRef, ResourceManager, SheetCache, TextureSource, resolve_texture_source, resolve_uv_rect};

struct LoadedUiTexture {
	texture_id: yakui::TextureId,
	width: u32,
	height: u32,
}

/// Decodes and uploads `UIImage` textures to the GPU, registers each with
/// yakui's wgpu integration once, and caches the resulting `yakui::TextureId`
/// (registration is one-time -- see `yakui_wgpu::YakuiWgpu::add_texture` --
/// so re-registering every frame would leak GPU textures).
///
/// This decodes bytes independently of `ze_renderer`'s own `TextureCache`
/// rather than sharing it: `ze_ui` cannot depend on `ze_renderer` (the
/// dependency already runs the other way, `ze_renderer -> ze_ui`, for
/// `UiManagerHandle`), so a second small decode step is unavoidable. What is
/// shared is the actual texture-reference format and cell/UV resolution math
/// (`ze_assets::{TextureSource, SheetCache, resolve_texture_source,
/// resolve_uv_rect}`) -- the same code `Sprite` uses -- so a `UIImage`
/// pointed at a `TextureSheet` cell resolves identically to a `Sprite`
/// pointed at the same cell.
pub struct UiImageTextureCache {
	textures: HashMap<AssetRef, LoadedUiTexture>,
	failed: HashSet<AssetRef>,
	sheet_cache: SheetCache,
}

impl UiImageTextureCache {
	pub fn new() -> Self {
		Self {
			textures: HashMap::new(),
			failed: HashSet::new(),
			sheet_cache: SheetCache::new(),
		}
	}

	/// Resolves a `TextureSource` to a yakui texture id plus the normalized
	/// UV rect to sample (the whole texture, or a specific sheet cell's
	/// trimmed rect). Returns `None` for an unassigned texture (an empty
	/// path -- the default for a freshly added `UIImage`) or a load failure
	/// (already logged once, on first failure).
	pub fn resolve(
		&mut self,
		source: &TextureSource,
		resources: &ResourceManager,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		wgpu_context: &mut yakui_wgpu::YakuiWgpu,
	) -> Option<(yakui::TextureId, [f32; 4])> {
		let (asset_ref, pending) = resolve_texture_source(source, &mut self.sheet_cache, resources);

		if asset_ref.path.is_empty() || self.failed.contains(&asset_ref) {
			return None;
		}

		if !self.textures.contains_key(&asset_ref) {
			let loaded = resources
				.bytes(&asset_ref)
				.ok()
				.and_then(|bytes| decode_texture(&asset_ref.path, bytes.as_ref(), device, queue));

			let Some((view, width, height)) = loaded else {
				ze_log::warn!("Failed to load UI image texture `{}`", asset_ref.path);
				self.failed.insert(asset_ref);
				return None;
			};

			let texture_id = wgpu_context.add_texture(
				view,
				wgpu::FilterMode::Nearest,
				wgpu::FilterMode::Nearest,
				wgpu::MipmapFilterMode::Nearest,
				wgpu::AddressMode::ClampToEdge,
			);
			self.textures.insert(
				asset_ref.clone(),
				LoadedUiTexture {
					texture_id,
					width,
					height,
				},
			);
		}

		let loaded = self.textures.get(&asset_ref)?;
		let (uv_rect, _effective_dimensions, _nominal_dimensions) =
			resolve_uv_rect(pending, (loaded.width, loaded.height));
		Some((loaded.texture_id, uv_rect))
	}
}

impl Default for UiImageTextureCache {
	fn default() -> Self { Self::new() }
}

/// Decodes image bytes and uploads them as an `Rgba8UnormSrgb` texture,
/// mirroring `ze_renderer`'s own `TextureResource::from_bytes` (kept as a
/// separate, small implementation here -- see the module doc for why this
/// can't simply call into `ze_renderer` instead).
fn decode_texture(
	label: &str,
	bytes: &[u8],
	device: &wgpu::Device,
	queue: &wgpu::Queue,
) -> Option<(wgpu::TextureView, u32, u32)> {
	let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
	let (width, height) = (rgba.width(), rgba.height());

	let size = wgpu::Extent3d {
		width,
		height,
		depth_or_array_layers: 1,
	};

	let texture = device.create_texture(&wgpu::TextureDescriptor {
		label: Some(label),
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: wgpu::TextureFormat::Rgba8UnormSrgb,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
		size,
	});

	queue.write_texture(
		wgpu::TexelCopyTextureInfo {
			texture: &texture,
			mip_level: 0,
			origin: wgpu::Origin3d::ZERO,
			aspect: wgpu::TextureAspect::All,
		},
		&rgba,
		wgpu::TexelCopyBufferLayout {
			offset: 0,
			bytes_per_row: Some(width * 4),
			rows_per_image: Some(height),
		},
		size,
	);

	let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
	Some((view, width, height))
}
