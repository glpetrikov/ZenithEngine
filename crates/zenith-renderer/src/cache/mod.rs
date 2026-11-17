//! `GpuCache`: every GPU resource that is kept between frames, one for all
//! the graphs and cameras.
//!
//! * Shaders, bind group layouts, samplers and pipelines ([`pipelines`]) are
//!   made once and found again by what they are.
//! * Meshes, textures and materials are found by their [`AssetId`]. Each is
//!   `Loading` (not drawn), `Ready`, or `Failed` (drawn with a stand-in, see
//!   [`fallback`]). Asking for one ([`GpuCache::request_mesh`], ...) marks it
//!   used; [`GpuCache::load`] reads what was asked for, within a time budget;
//!   what was not used for long goes when the cache is over its memory budget
//!   ([`evict`]).
//!
//! A failed asset is warned about once and not tried again until its file
//! changes ([`GpuCache::invalidate`]) or [`GpuCache::reload_failed`].

pub mod evict;
pub mod fallback;
pub mod materials;
pub mod meshes;
pub mod pipelines;
pub mod textures;

use std::collections::{HashMap, HashSet, VecDeque};

use bevy_ecs::resource::Resource;
pub use fallback::Fallbacks;
pub use materials::{GpuMaterial, MATERIAL_GROUP, MaterialUniform};
pub use meshes::{GpuMesh, GpuSubMesh};
pub use pipelines::{LayoutId, PipelineId, PipelineKey, Pipelines, SamplerKey, ShaderId};
pub use textures::GpuTexture;
use web_time::{Duration, Instant};
use zenith_error::ZResult;
use zenith_filesystem::Project;

use crate::{
	assets::{AssetId, AssetKind, AssetRegistry, load},
	shaders::ShaderSources,
};

/// Where an asset is in the cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetState {
	/// Asked for, not loaded yet: not drawn.
	Loading,
	Ready,
	/// It could not be loaded: drawn with a stand-in.
	Failed,
}

/// How the cache keeps and loads assets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheSettings {
	/// An asset not used for this long may be evicted (the editor keeps them
	/// longer than a game).
	pub keep_unused: Duration,
	/// Assets are evicted only while the cache takes more of the GPU than
	/// this, in bytes. A soft limit: what is used stays.
	pub memory_budget: u64,
	/// How long a frame may spend loading. At least one asset is loaded a
	/// frame, whatever it takes.
	pub load_budget: Duration,
}

impl Default for CacheSettings {
	fn default() -> Self {
		Self {
			keep_unused: Duration::from_secs(30),
			memory_budget: 512 * 1024 * 1024,
			load_budget: Duration::from_millis(4),
		}
	}
}

/// What a cached asset has in common.
trait GpuAsset {
	/// What it is called in the log.
	const WHAT: &'static str;
	fn bytes(&self) -> u64;
}

impl GpuAsset for GpuMesh {
	const WHAT: &'static str = "model";

	fn bytes(&self) -> u64 {
		Self::bytes(self)
	}
}

impl GpuAsset for GpuTexture {
	const WHAT: &'static str = "texture";

	fn bytes(&self) -> u64 {
		Self::bytes(self)
	}
}

impl GpuAsset for GpuMaterial {
	const WHAT: &'static str = "material";

	fn bytes(&self) -> u64 {
		Self::bytes(self)
	}
}

enum Slot<T> {
	Loading,
	Ready(T),
	Failed,
}

struct Entry<T> {
	slot: Slot<T>,
	last_used: Instant,
}

impl<T> Entry<T> {
	const fn state(&self) -> AssetState {
		match self.slot {
			Slot::Loading => AssetState::Loading,
			Slot::Ready(_) => AssetState::Ready,
			Slot::Failed => AssetState::Failed,
		}
	}
}

type Table<T> = HashMap<AssetId, Entry<T>>;

/// The GPU resources kept between frames. A resource of the world.
#[derive(Resource)]
pub struct GpuCache {
	device: wgpu::Device,
	queue: wgpu::Queue,
	pipelines: Pipelines,
	material_layout_id: LayoutId,
	material_layout: wgpu::BindGroupLayout,
	fallback: Fallbacks,
	meshes: Table<GpuMesh>,
	textures: Table<GpuTexture>,
	materials: Table<GpuMaterial>,
	/// What was asked for and is not loaded yet, in the order it was asked.
	pending: VecDeque<(AssetKind, AssetId)>,
	/// The materials that use a texture: they go when it does.
	users: HashMap<AssetId, HashSet<AssetId>>,
	pinned: HashSet<AssetId>,
	settings: CacheSettings,
	/// The time of the frame ([`GpuCache::begin_frame`]).
	now: Instant,
	/// What the ready assets take of the GPU, in bytes.
	memory: u64,
	/// Where the shaders by name come from.
	sources: ShaderSources,
}

impl GpuCache {
	/// A cache on `device`, with its stand-ins made.
	pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, settings: CacheSettings) -> Self {
		let mut pipelines = Pipelines::new(device.clone());
		let material_layout_id = pipelines.layout(&materials::layout_entries());
		let material_layout = pipelines.get_layout(material_layout_id).cloned().unwrap_or_else(|| {
			device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
				label: Some("Zenith Material"),
				entries: &materials::layout_entries(),
			})
		});
		let fallback = Fallbacks::new(device, queue, &mut pipelines, &material_layout);
		Self {
			device: device.clone(),
			queue: queue.clone(),
			pipelines,
			material_layout_id,
			material_layout,
			fallback,
			meshes: HashMap::new(),
			textures: HashMap::new(),
			materials: HashMap::new(),
			pending: VecDeque::new(),
			users: HashMap::new(),
			pinned: HashSet::new(),
			settings,
			now: Instant::now(),
			memory: 0,
			sources: ShaderSources::default(),
		}
	}

	pub const fn settings(&self) -> &CacheSettings {
		&self.settings
	}

	pub const fn settings_mut(&mut self) -> &mut CacheSettings {
		&mut self.settings
	}

	pub const fn device(&self) -> &wgpu::Device {
		&self.device
	}

	pub const fn queue(&self) -> &wgpu::Queue {
		&self.queue
	}

	/// The stand-ins of lost assets.
	pub const fn fallback(&self) -> &Fallbacks {
		&self.fallback
	}

	/// A new frame begins at `now`: what is asked for from here on is used in
	/// it.
	pub const fn begin_frame(&mut self, now: Instant) {
		self.now = now;
	}

	/// What the ready assets take of the GPU, in bytes (the stand-ins are not
	/// counted).
	pub const fn memory(&self) -> u64 {
		self.memory
	}

	// --- Assets -----------------------------------------------------------

	/// The model `id` is needed this frame.
	pub fn request_mesh(&mut self, id: AssetId) {
		if Self::mark(&mut self.meshes, id, self.now) {
			self.pending.push_back((AssetKind::Model, id));
		}
	}

	/// The texture `id` is needed this frame.
	pub fn request_texture(&mut self, id: AssetId) {
		if Self::mark(&mut self.textures, id, self.now) {
			self.pending.push_back((AssetKind::Texture, id));
		}
	}

	/// The material `id` is needed this frame, and so are its textures.
	pub fn request_material(&mut self, id: AssetId) {
		if Self::mark(&mut self.materials, id, self.now) {
			self.pending.push_back((AssetKind::Material, id));
		} else if let Some(Entry {
			slot: Slot::Ready(material),
			..
		}) = self.materials.get(&id)
		{
			for texture in &material.textures {
				if let Some(entry) = self.textures.get_mut(texture) {
					entry.last_used = self.now;
				}
			}
		}
	}

	/// Marks `id` used at `now`. `true`: it was not there, and is `Loading`
	/// now.
	fn mark<T>(table: &mut Table<T>, id: AssetId, now: Instant) -> bool {
		if let Some(entry) = table.get_mut(&id) {
			entry.last_used = now;
			return false;
		}
		table.insert(
			id,
			Entry {
				slot: Slot::Loading,
				last_used: now,
			},
		);
		true
	}

	/// How many assets wait to be loaded.
	pub fn pending(&self) -> usize {
		self.pending.len()
	}

	/// Loads what was asked for, in the order it was asked, until `budget`
	/// is spent (`None`: all of it). Returns how many were loaded. What did
	/// not fit stays `Loading` for the next call.
	pub fn load(&mut self, project: &Project, registry: &AssetRegistry, budget: Option<Duration>) -> usize {
		let start = Instant::now();
		let mut loaded = 0;
		while let Some((kind, id)) = self.pending.pop_front() {
			if loaded > 0 && budget.is_some_and(|budget| start.elapsed() >= budget) {
				self.pending.push_front((kind, id));
				break;
			}
			let waiting = match kind {
				AssetKind::Model => self.meshes.get(&id).map(Entry::state),
				AssetKind::Texture => self.textures.get(&id).map(Entry::state),
				AssetKind::Material => self.materials.get(&id).map(Entry::state),
				AssetKind::RenderGraph => None,
			};
			// Loaded on the way (a texture of a material), or let go since.
			if waiting != Some(AssetState::Loading) {
				continue;
			}
			match kind {
				AssetKind::Model => {
					let mesh = self.make_mesh(project, registry, id);
					settle(&mut self.meshes, &mut self.memory, id, mesh);
				}
				AssetKind::Texture => self.load_texture(project, registry, id),
				AssetKind::Material => {
					let material = self.make_material(project, registry, id);
					settle(&mut self.materials, &mut self.memory, id, material);
				}
				AssetKind::RenderGraph => {}
			}
			loaded += 1;
		}
		loaded
	}

	fn make_mesh(&self, project: &Project, registry: &AssetRegistry, id: AssetId) -> ZResult<GpuMesh> {
		let data = load::mesh(project, registry, id)?;
		GpuMesh::upload(&self.device, &label(registry, id), &data)
	}

	fn load_texture(&mut self, project: &Project, registry: &AssetRegistry, id: AssetId) {
		let texture = load::texture(project, registry, id).and_then(|(image, settings)| {
			GpuTexture::upload(
				&self.device,
				&self.queue,
				&mut self.pipelines,
				&label(registry, id),
				&image,
				&settings,
			)
		});
		settle(&mut self.textures, &mut self.memory, id, texture);
	}

	fn make_material(&mut self, project: &Project, registry: &AssetRegistry, id: AssetId) -> ZResult<GpuMaterial> {
		let data = load::material(project, registry, id)?;
		for texture in [data.base_color_texture, data.normal_texture].into_iter().flatten() {
			self.users.entry(texture).or_default().insert(id);
			Self::mark(&mut self.textures, texture, self.now);
			if self.textures.get(&texture).map(Entry::state) == Some(AssetState::Loading) {
				self.load_texture(project, registry, texture);
			}
		}
		let ready = |texture: Option<AssetId>| match self.textures.get(&texture?)?.slot {
			Slot::Ready(ref texture) => Some(texture),
			_ => None,
		};
		let base_color = match data.base_color_texture {
			None => &self.fallback.white,
			Some(_) => ready(data.base_color_texture).unwrap_or(&self.fallback.checker),
		};
		let normal = ready(data.normal_texture).unwrap_or(&self.fallback.flat_normal);
		Ok(GpuMaterial::create(
			&self.device,
			&self.material_layout,
			&label(registry, id),
			&data,
			base_color,
			normal,
		))
	}

	/// Where `id` is in the cache, if it was asked for.
	pub fn state(&self, id: AssetId) -> Option<AssetState> {
		self.meshes
			.get(&id)
			.map(Entry::state)
			.or_else(|| self.textures.get(&id).map(Entry::state))
			.or_else(|| self.materials.get(&id).map(Entry::state))
	}

	/// The model `id` to draw: the cube if it failed, `None` while it loads
	/// or if it was never asked for.
	pub fn mesh(&self, id: AssetId) -> Option<&GpuMesh> {
		match self.meshes.get(&id)?.slot {
			Slot::Ready(ref mesh) => Some(mesh),
			Slot::Failed => Some(&self.fallback.cube),
			Slot::Loading => None,
		}
	}

	/// The texture `id`: the checker if it failed, `None` while it loads or
	/// if it was never asked for.
	pub fn texture(&self, id: AssetId) -> Option<&GpuTexture> {
		match self.textures.get(&id)?.slot {
			Slot::Ready(ref texture) => Some(texture),
			Slot::Failed => Some(&self.fallback.checker),
			Slot::Loading => None,
		}
	}

	/// The material `id`: the standard one if it failed, `None` while it
	/// loads or if it was never asked for.
	pub fn material(&self, id: AssetId) -> Option<&GpuMaterial> {
		match self.materials.get(&id)?.slot {
			Slot::Ready(ref material) => Some(material),
			Slot::Failed => Some(&self.fallback.material),
			Slot::Loading => None,
		}
	}

	/// The file of `id` changed: it is dropped, with the materials that use
	/// it, and loaded again when it is next asked for.
	pub fn invalidate(&mut self, id: AssetId) {
		self.remove(id);
		for material in self.users.remove(&id).unwrap_or_default() {
			self.remove(material);
		}
	}

	fn remove(&mut self, id: AssetId) {
		let freed = take(&mut self.meshes, id) + take(&mut self.textures, id) + take(&mut self.materials, id);
		self.memory = self.memory.saturating_sub(freed);
	}

	/// The assets that failed are tried again (a manual reload).
	pub fn reload_failed(&mut self) {
		retry(&mut self.meshes, AssetKind::Model, &mut self.pending);
		retry(&mut self.textures, AssetKind::Texture, &mut self.pending);
		retry(&mut self.materials, AssetKind::Material, &mut self.pending);
	}

	/// `id` is never evicted (preloading). It still has to be asked for to be
	/// loaded.
	pub fn pin(&mut self, id: AssetId) {
		self.pinned.insert(id);
	}

	pub fn unpin(&mut self, id: AssetId) {
		self.pinned.remove(&id);
	}

	// --- Pipelines --------------------------------------------------------

	/// The shaders, layouts, samplers and pipelines.
	pub const fn pipelines(&self) -> &Pipelines {
		&self.pipelines
	}

	pub const fn pipelines_mut(&mut self) -> &mut Pipelines {
		&mut self.pipelines
	}

	/// The shaders by name.
	pub const fn sources(&self) -> &ShaderSources {
		&self.sources
	}

	pub const fn sources_mut(&mut self) -> &mut ShaderSources {
		&mut self.sources
	}

	/// The shader `name` from [`GpuCache::sources`], made the first time it
	/// is asked for (and again if its text changed). `None`: no such shader.
	pub fn named_shader(&mut self, name: &str) -> Option<ShaderId> {
		let source = self.sources.get(name)?;
		Some(self.pipelines.shader(name, source))
	}

	/// See [`Pipelines::shader`].
	pub fn shader(&mut self, name: &str, source: &str) -> ShaderId {
		self.pipelines.shader(name, source)
	}

	/// See [`Pipelines::layout`].
	pub fn layout(&mut self, entries: &[wgpu::BindGroupLayoutEntry]) -> LayoutId {
		self.pipelines.layout(entries)
	}

	pub fn get_layout(&self, id: LayoutId) -> Option<&wgpu::BindGroupLayout> {
		self.pipelines.get_layout(id)
	}

	/// The layout of bind group 1, the material ([`materials`]).
	pub const fn material_layout(&self) -> LayoutId {
		self.material_layout_id
	}

	/// See [`Pipelines::sampler`].
	pub fn sampler(&mut self, key: &SamplerKey) -> wgpu::Sampler {
		self.pipelines.sampler(key)
	}

	/// See [`Pipelines::pipeline`].
	pub fn pipeline(&mut self, key: &PipelineKey) -> PipelineId {
		self.pipelines.pipeline(key)
	}

	pub fn get_pipeline(&self, id: PipelineId) -> Option<&wgpu::RenderPipeline> {
		self.pipelines.get_pipeline(id)
	}
}

/// The name of `id` in labels and the log: its path.
fn label(registry: &AssetRegistry, id: AssetId) -> String {
	registry
		.get(id)
		.map_or_else(|| id.to_string(), |entry| entry.path.to_string())
}

/// Puts what loading `id` gave into its entry (if it is still there).
fn settle<T: GpuAsset>(table: &mut Table<T>, memory: &mut u64, id: AssetId, result: ZResult<T>) {
	let Some(entry) = table.get_mut(&id) else {
		return;
	};
	entry.slot = match result {
		Ok(asset) => {
			*memory += asset.bytes();
			Slot::Ready(asset)
		}
		Err(e) => {
			zenith_log::warn!("The {} {id} cannot be used, a stand-in is drawn instead: {e}", T::WHAT);
			Slot::Failed
		}
	};
}

/// Takes `id` out of `table`; what it took of the GPU.
fn take<T: GpuAsset>(table: &mut Table<T>, id: AssetId) -> u64 {
	match table.remove(&id) {
		Some(Entry {
			slot: Slot::Ready(asset),
			..
		}) => asset.bytes(),
		_ => 0,
	}
}

fn retry<T>(table: &mut Table<T>, kind: AssetKind, pending: &mut VecDeque<(AssetKind, AssetId)>) {
	for (&id, entry) in table.iter_mut() {
		if matches!(entry.slot, Slot::Failed) {
			entry.slot = Slot::Loading;
			pending.push_back((kind, id));
		}
	}
}
