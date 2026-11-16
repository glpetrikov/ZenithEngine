//! The GPU cache on whatever adapter the machine has. The tests are skipped
//! where there is no adapter at all.

// A test that cannot go on has to stop: panicking is how it fails.
#![allow(clippy::expect_used, clippy::panic)]

use web_time::{Duration, Instant};
use zenith_filesystem::{Project, ZPath};
use zenith_renderer::{
	CacheSettings, GpuCache,
	assets::{
		AssetId, AssetRegistry,
		import::image::{Filter, TextureImport, TextureUsage, Wrap},
		json::MaterialData,
		ops,
	},
	cache::{AssetState, GpuMesh, PipelineKey, textures::GpuTexture},
	wgpu,
};

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
	let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
	let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
	pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
		label: Some("test device"),
		required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
		..Default::default()
	}))
	.ok()
}

fn cache(settings: CacheSettings) -> Option<GpuCache> {
	let Some((device, queue)) = device() else {
		eprintln!("no adapter: skipped");
		return None;
	};
	Some(GpuCache::new(&device, &queue, settings))
}

fn sample() -> (Project, AssetRegistry) {
	let project =
		Project::open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Sandbox/Test")).expect("open the sample project");
	let registry = AssetRegistry::scan(&project).expect("scan its assets");
	(project, registry)
}

fn id_of(registry: &AssetRegistry, path: &str) -> AssetId {
	registry
		.id_of(&ZPath::new(path).expect("a valid path"))
		.unwrap_or_else(|| panic!("{path} is not an asset: {:?}", registry.problems()))
}

/// A project on disk with nothing in it yet.
struct Scratch {
	dir: tempfile::TempDir,
	project: Project,
}

impl Scratch {
	fn new() -> Self {
		let dir = tempfile::tempdir().expect("a temporary folder");
		let project = Project::create(dir.path().join("Game")).expect("make a project");
		Self { dir, project }
	}

	/// Imports a PNG of `width` by `height` pixels of `rgba`, named `name`.
	fn texture(&self, name: &str, width: u32, height: u32, rgba: [u8; 4]) -> AssetId {
		let file = self.dir.path().join(name);
		image::RgbaImage::from_pixel(width, height, image::Rgba(rgba))
			.save(&file)
			.expect("write the picture");
		let mut registry = self.registry();
		ops::import_texture(&self.project, &mut registry, &file, &assets(), None)
			.expect("import the picture")
			.id
	}

	/// Writes a material with this base color texture.
	fn material(&self, name: &str, texture: Option<AssetId>) -> AssetId {
		let id = AssetId::generate().expect("an id");
		let material = MaterialData {
			base_color_texture: texture,
			..MaterialData::new(id)
		};
		let path = ZPath::new(&format!("Assets://{name}.zenithmaterial")).expect("a path");
		self.project
			.write(&path, material.to_text().expect("write it").as_bytes())
			.expect("write the material");
		id
	}

	fn registry(&self) -> AssetRegistry {
		AssetRegistry::scan(&self.project).expect("scan the assets")
	}
}

fn assets() -> ZPath {
	ZPath::new("Assets://").expect("a path")
}

#[test]
fn the_sample_cube_loads_with_its_material_and_its_textures() {
	let Some(mut cache) = cache(CacheSettings::default()) else {
		return;
	};
	let (project, registry) = sample();
	let cube = id_of(&registry, "Assets://Models/Cube/cube.zenithmodel");
	let sky = id_of(&registry, "Assets://Sky/pure-sky.zenithtexture");
	cache.request_mesh(cube);
	cache.request_texture(sky);
	assert_eq!(cache.state(cube), Some(AssetState::Loading));
	assert!(cache.mesh(cube).is_none(), "not drawn while it loads");
	assert_eq!(cache.load(&project, &registry, None), 2);

	let mesh = cache.mesh(cube).expect("the cube is ready");
	assert_eq!(cache.state(cube), Some(AssetState::Ready));
	let [submesh] = mesh.submeshes.as_slice() else {
		panic!("one mesh, not {}", mesh.submeshes.len());
	};
	assert_eq!(submesh.indices, 0..36);
	let material = submesh.material.expect("the cube has a material");

	cache.request_material(material);
	assert_eq!(cache.load(&project, &registry, None), 1, "the textures come with it");
	let gpu_material = cache.material(material).expect("the material is ready");
	assert_eq!(gpu_material.textures.len(), 2, "a color texture and a normal map");
	for &texture in &gpu_material.textures {
		assert_eq!(cache.state(texture), Some(AssetState::Ready));
	}
	let [color, normal] = [0, 1].map(|at| cache.texture(gpu_material.textures[at]).expect("loaded"));
	assert_eq!(color.texture.format(), wgpu::TextureFormat::Rgba8UnormSrgb);
	assert!(color.texture.mip_level_count() > 1, "with mipmaps");
	assert_eq!(normal.texture.format(), wgpu::TextureFormat::Rgba8Unorm);

	let sky = cache.texture(sky).expect("the sky is ready");
	assert_eq!(sky.texture.format(), wgpu::TextureFormat::Rgba16Float);
	assert_eq!(sky.texture.mip_level_count(), 1, "a sky has no mipmaps of its own");
	assert!(cache.memory() > 0);
}

#[test]
fn a_lost_asset_gets_a_stand_in_and_is_not_tried_again_until_it_changes() {
	let Some(mut cache) = cache(CacheSettings::default()) else {
		return;
	};
	let scratch = Scratch::new();
	let texture = scratch.texture("red.png", 4, 4, [255, 0, 0, 255]);
	let registry = scratch.registry();

	let unknown = AssetId::generate().expect("an id");
	cache.request_mesh(unknown);
	cache.load(&scratch.project, &registry, None);
	assert_eq!(cache.state(unknown), Some(AssetState::Failed));
	assert!(
		cache
			.mesh(unknown)
			.is_some_and(|mesh| std::ptr::eq(mesh, std::ptr::from_ref(&cache.fallback().cube))),
		"a lost model is the cube"
	);

	let path = ZPath::new("Assets://red.zenithtexture").expect("a path");
	let good = scratch.project.read_bytes(&path).expect("read it");
	scratch.project.write(&path, b"broken").expect("break it");
	cache.request_texture(texture);
	cache.load(&scratch.project, &registry, None);
	assert_eq!(cache.state(texture), Some(AssetState::Failed));
	assert!(
		cache
			.texture(texture)
			.is_some_and(|t| std::ptr::eq(t, std::ptr::from_ref(&cache.fallback().checker))),
		"a broken texture is the checker"
	);

	scratch.project.write(&path, &good).expect("mend it");
	cache.request_texture(texture);
	assert_eq!(cache.pending(), 0, "a failed asset is not asked for again");
	cache.invalidate(texture);
	cache.request_texture(texture);
	cache.load(&scratch.project, &registry, None);
	assert_eq!(cache.state(texture), Some(AssetState::Ready), "its file changed");

	cache.reload_failed();
	assert_eq!(cache.pending(), 1, "the lost model is tried again by hand");
}

#[test]
fn a_changed_texture_takes_the_materials_that_use_it_along() {
	let Some(mut cache) = cache(CacheSettings::default()) else {
		return;
	};
	let scratch = Scratch::new();
	let texture = scratch.texture("bricks.png", 8, 8, [200, 100, 50, 255]);
	let material = scratch.material("Bricks", Some(texture));
	let plain = scratch.material("Plain", None);
	let registry = scratch.registry();

	cache.request_material(material);
	cache.request_material(plain);
	cache.load(&scratch.project, &registry, None);
	assert_eq!(cache.state(material), Some(AssetState::Ready));
	assert_eq!(
		cache.state(texture),
		Some(AssetState::Ready),
		"loaded with its material"
	);
	assert_eq!(cache.state(plain), Some(AssetState::Ready));

	cache.invalidate(texture);
	assert_eq!(cache.state(texture), None);
	assert_eq!(cache.state(material), None, "it used the texture");
	assert_eq!(cache.state(plain), Some(AssetState::Ready), "it did not");

	cache.request_material(material);
	cache.load(&scratch.project, &registry, None);
	assert_eq!(cache.state(material), Some(AssetState::Ready));
	assert_eq!(cache.state(texture), Some(AssetState::Ready));
}

#[test]
fn a_material_with_a_lost_texture_is_drawn_with_the_checker_in_its_place() {
	let Some(mut cache) = cache(CacheSettings::default()) else {
		return;
	};
	let scratch = Scratch::new();
	let lost = AssetId::generate().expect("an id");
	let material = scratch.material("Lost", Some(lost));
	let registry = scratch.registry();
	cache.request_material(material);
	cache.load(&scratch.project, &registry, None);
	assert_eq!(
		cache.state(material),
		Some(AssetState::Ready),
		"the material itself is fine"
	);
	assert_eq!(cache.state(lost), Some(AssetState::Failed));
}

#[test]
fn what_was_not_used_for_long_goes_while_the_cache_is_over_its_budget() {
	let Some(mut cache) = cache(CacheSettings {
		memory_budget: 0,
		keep_unused: Duration::from_secs(30),
		..CacheSettings::default()
	}) else {
		return;
	};
	let scratch = Scratch::new();
	let old = scratch.texture("old.png", 16, 16, [255; 4]);
	let pinned = scratch.texture("pinned.png", 16, 16, [0, 0, 0, 255]);
	let registry = scratch.registry();
	let start = Instant::now();

	cache.begin_frame(start);
	cache.request_texture(old);
	cache.request_texture(pinned);
	cache.pin(pinned);
	cache.load(&scratch.project, &registry, None);
	let both = cache.memory();

	cache.begin_frame(start + Duration::from_secs(10));
	assert_eq!(cache.evict(), 0, "used not long ago");

	cache.begin_frame(start + Duration::from_secs(60));
	assert_eq!(cache.evict(), 1);
	assert_eq!(cache.state(old), None);
	assert_eq!(cache.state(pinned), Some(AssetState::Ready), "pinned");
	assert!(cache.memory() < both);

	cache.settings_mut().memory_budget = u64::MAX;
	cache.request_texture(old);
	cache.load(&scratch.project, &registry, None);
	cache.begin_frame(start + Duration::from_secs(600));
	assert_eq!(cache.evict(), 0, "under the budget nothing goes");

	cache.begin_frame(start + Duration::from_secs(601));
	cache.request_texture(pinned);
	assert_eq!(
		cache.evict_unused(),
		1,
		"a change of world takes what this frame did not ask for"
	);
	assert_eq!(cache.state(old), None);
	cache.unpin(pinned);
	cache.begin_frame(start + Duration::from_secs(602));
	assert_eq!(cache.evict_unused(), 1);
	assert_eq!(cache.memory(), 0);
}

#[test]
fn the_load_budget_spreads_the_loading_over_frames() {
	let Some(mut cache) = cache(CacheSettings::default()) else {
		return;
	};
	let scratch = Scratch::new();
	let textures = ["a.png", "b.png", "c.png"].map(|name| scratch.texture(name, 2, 2, [9; 4]));
	let registry = scratch.registry();
	for texture in textures {
		cache.request_texture(texture);
	}
	assert_eq!(
		cache.load(&scratch.project, &registry, Some(Duration::ZERO)),
		1,
		"one at least"
	);
	assert_eq!(cache.pending(), 2);
	assert_eq!(cache.load(&scratch.project, &registry, None), 2);
}

#[test]
fn pipelines_are_shared_and_made_again_with_their_shader() {
	let Some(mut cache) = cache(CacheSettings::default()) else {
		return;
	};
	let format = wgpu::TextureFormat::Rgba8UnormSrgb;
	let shader = cache.shader("test.mesh", MESH_SHADER);
	assert_eq!(cache.shader("test.mesh", MESH_SHADER), shader, "the same source");
	let key = PipelineKey {
		layouts: vec![cache.layout(&[]), cache.material_layout()],
		vertex_buffers: vec![GpuMesh::LAYOUT],
		..PipelineKey::new(shader, format)
	};
	let pipeline = cache.pipeline(&key);
	assert_eq!(cache.pipeline(&key), pipeline, "one pipeline for one key");
	assert!(cache.get_pipeline(pipeline).is_some());
	let count = cache.pipelines().pipeline_count();

	let changed = MESH_SHADER.replace("material.base_color", "material.base_color * 0.5");
	assert_eq!(cache.shader("test.mesh", &changed), shader, "hot reload keeps the id");
	assert!(
		cache.get_pipeline(pipeline).is_none(),
		"its pipelines went with the old source"
	);
	let again = cache.pipeline(&key);
	assert_ne!(again, pipeline);
	assert_eq!(cache.pipelines().pipeline_count(), count);

	assert_eq!(cache.layout(&[]), cache.layout(&[]), "one layout for one description");
}

/// A shader that reads everything a mesh and a material give: making a
/// pipeline of it checks that the layouts of the cache are what the
/// documentation of `materials` says.
const MESH_SHADER: &str = "
struct Material {
    base_color: vec4f,
    roughness: f32,
    metallic: f32,
    alpha_cutoff: f32,
    flags: u32,
}
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_color_texture: texture_2d<f32>;
@group(1) @binding(2) var base_color_sampler: sampler;
@group(1) @binding(3) var normal_texture: texture_2d<f32>;
@group(1) @binding(4) var normal_sampler: sampler;

struct Vertex {
    @location(0) position: vec3f,
    @location(1) uv: vec2f,
    @location(2) normal: vec3f,
    @location(3) tangent: vec3f,
    @location(4) bitangent: vec3f,
}
struct Out {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
    @location(1) normal: vec3f,
}

@vertex
fn vs_main(v: Vertex) -> Out {
    var out: Out;
    out.position = vec4f(v.position, 1.0);
    out.uv = v.uv;
    out.normal = v.normal + v.tangent + v.bitangent;
    return out;
}

@fragment
fn fs_main(in: Out) -> @location(0) vec4f {
    let color = textureSample(base_color_texture, base_color_sampler, in.uv) * material.base_color;
    let normal = textureSample(normal_texture, normal_sampler, in.uv).xyz;
    if (material.flags & 1u) != 0u && color.a < material.alpha_cutoff {
        discard;
    }
    return vec4f(color.rgb * (material.roughness + material.metallic) + normal * 0.0 + in.normal * 0.0, color.a);
}
";

#[test]
fn mipmaps_average_the_light_not_the_srgb_numbers() {
	let Some(mut cache) = cache(CacheSettings::default()) else {
		return;
	};
	// Two columns of white, two of black: the 1x1 level is half the light,
	// which sRGB writes as 188, not 128.
	let pixels: Vec<u8> = (0..4 * 4)
		.flat_map(|i| if i % 4 < 2 { [255; 4] } else { [0, 0, 0, 255] })
		.collect();
	let settings = TextureImport {
		filter: Filter::Linear,
		wrap: Wrap::Repeat,
		..TextureImport::for_usage(TextureUsage::Color, "png")
	};
	let image = zenith_renderer::assets::import::image::ImageData {
		width: 4,
		height: 4,
		usage: TextureUsage::Color,
		pixels: zenith_renderer::assets::import::image::Pixels::Rgba8(pixels),
	};
	let (device, queue) = (cache.device().clone(), cache.queue().clone());
	let texture = GpuTexture::upload(&device, &queue, cache.pipelines_mut(), "columns", &image, &settings)
		.expect("upload the texture");
	assert_eq!(texture.texture.mip_level_count(), 3);
	let [r, g, b, a] = read_pixel(&device, &queue, &texture.texture, 2);
	assert!((186..=190).contains(&r), "red {r}");
	assert_eq!((r, g, b), (r, r, r), "gray");
	assert_eq!(a, 255);
}

/// The top left pixel of `level` of an `Rgba8` texture.
fn read_pixel(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, level: u32) -> [u8; 4] {
	let buffer = device.create_buffer(&wgpu::BufferDescriptor {
		label: Some("read back"),
		size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
		usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
		mapped_at_creation: false,
	});
	let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
	encoder.copy_texture_to_buffer(
		wgpu::TexelCopyTextureInfo {
			texture,
			mip_level: level,
			origin: wgpu::Origin3d::ZERO,
			aspect: wgpu::TextureAspect::All,
		},
		wgpu::TexelCopyBufferInfo {
			buffer: &buffer,
			layout: wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
				rows_per_image: Some(1),
			},
		},
		wgpu::Extent3d {
			width: 1,
			height: 1,
			depth_or_array_layers: 1,
		},
	);
	queue.submit([encoder.finish()]);
	buffer.map_async(wgpu::MapMode::Read, .., |result| result.expect("map the buffer"));
	device
		.poll(wgpu::PollType::wait_indefinitely())
		.expect("wait for the GPU");
	let bytes = buffer.get_mapped_range(..4).expect("read the buffer");
	[bytes[0], bytes[1], bytes[2], bytes[3]]
}
