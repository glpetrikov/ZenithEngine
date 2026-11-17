//! The pool of transient textures the graphs draw into.
//!
//! A view takes what it needs, the textures go back when it is done, and the
//! next view (of this frame or the next) takes them again. A texture not
//! taken for a few frames is freed: a window made smaller, a camera gone.

use std::collections::HashMap;

/// What a texture of the pool is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureKey {
	pub format: wgpu::TextureFormat,
	pub width: u32,
	pub height: u32,
	pub usage: wgpu::TextureUsages,
	pub samples: u32,
}

impl TextureKey {
	/// What every texture of a graph can be used for: drawn into, read by a
	/// later pass, copied.
	pub const USAGE: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
		.union(wgpu::TextureUsages::TEXTURE_BINDING)
		.union(wgpu::TextureUsages::COPY_SRC)
		.union(wgpu::TextureUsages::COPY_DST);

	pub const fn new(format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
		Self {
			format,
			width,
			height,
			usage: Self::USAGE,
			samples: 1,
		}
	}
}

struct Pooled {
	texture: wgpu::Texture,
	view: wgpu::TextureView,
	last_used: u64,
	taken: bool,
}

/// The transient textures of the graphs.
pub struct TexturePool {
	textures: HashMap<TextureKey, Vec<Pooled>>,
	/// A texture not taken for more frames than this is freed.
	pub keep_frames: u64,
}

impl Default for TexturePool {
	fn default() -> Self {
		Self {
			textures: HashMap::new(),
			keep_frames: 3,
		}
	}
}

impl TexturePool {
	pub fn new() -> Self {
		Self::default()
	}

	/// A texture of `key` nobody has taken this view, made if there is none.
	pub fn acquire(
		&mut self,
		device: &wgpu::Device,
		key: TextureKey,
		frame: u64,
	) -> (wgpu::Texture, wgpu::TextureView) {
		let list = self.textures.entry(key).or_default();
		if let Some(free) = list.iter_mut().find(|pooled| !pooled.taken) {
			free.taken = true;
			free.last_used = frame;
			return (free.texture.clone(), free.view.clone());
		}
		let texture = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("Zenith Graph Texture"),
			size: wgpu::Extent3d {
				width: key.width.max(1),
				height: key.height.max(1),
				depth_or_array_layers: 1,
			},
			mip_level_count: 1,
			sample_count: key.samples,
			dimension: wgpu::TextureDimension::D2,
			format: key.format,
			usage: key.usage,
			view_formats: &[],
		});
		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		list.push(Pooled {
			texture: texture.clone(),
			view: view.clone(),
			last_used: frame,
			taken: true,
		});
		(texture, view)
	}

	/// Every texture is free again (a view is done: the commands of the next
	/// one come after its own in the encoder, so they may share).
	pub fn release_all(&mut self) {
		for pooled in self.textures.values_mut().flatten() {
			pooled.taken = false;
		}
	}

	/// Frees what was not taken for more than `keep_frames` frames.
	pub fn end_frame(&mut self, frame: u64) {
		let keep = self.keep_frames;
		for list in self.textures.values_mut() {
			list.retain(|pooled| frame.saturating_sub(pooled.last_used) <= keep);
		}
		self.textures.retain(|_, list| !list.is_empty());
	}

	/// How many textures the pool has.
	pub fn len(&self) -> usize {
		self.textures.values().map(Vec::len).sum()
	}

	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}
}
