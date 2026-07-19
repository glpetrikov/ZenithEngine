use std::{
	borrow::Cow,
	fs,
	path::{Component, Path, PathBuf},
	sync::Arc,
};

use anyhow::{Result, anyhow, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod animation_clip;
pub use animation_clip::{ANIMATION_CLIP_EXTENSION, AnimationClip, step_animation_frame};

pub mod texture_sheet;
pub use texture_sheet::{
	AutoPackSheet, CellId, CellTrim, GridCell, ShelfPackedRect, TEXTURE_SHEET_EXTENSION, TextureSheet,
	TextureSheetMode, UniformGridSheet, compute_uniform_grid_trims, shelf_pack_preview,
};

const EMBEDDED_SPRITE_WGSL: &str = "\
@group(0) @binding(0)
var sprite_texture: texture_2d<f32>;

@group(0) @binding(1)
var sprite_sampler: sampler;

@group(1) @binding(0)
var<uniform> view_projection: mat4x4<f32>;

struct Vertex {
    @location(0)
    position: vec3<f32>,
    @location(1)
    color: vec4<f32>
}

// One row per sprite instance -- everything that used to live in a
// per-draw-call material/object uniform (transform, uv rect, tint, params,
// emissive) is now per-instance vertex data, so a whole texture group of
// sprites can be drawn with a single instanced draw call.
struct Instance {
    @location(2)
    model_0: vec4<f32>,
    @location(3)
    model_1: vec4<f32>,
    @location(4)
    model_2: vec4<f32>,
    @location(5)
    model_3: vec4<f32>,
    @location(6)
    uv_rect: vec4<f32>,
    @location(7)
    tint: vec4<f32>,
    @location(8)
    params: vec4<f32>,
    @location(9)
    emissive: vec4<f32>
}

struct VertexOutput {
    @builtin(position)
    position: vec4<f32>,
    @location(0)
    color: vec4<f32>,
    @location(1)
    tex_coord: vec2<f32>,
    @location(2)
    uv_rect: vec4<f32>,
    @location(3)
    tint: vec4<f32>,
    @location(4)
    params: vec4<f32>,
    @location(5)
    emissive: vec4<f32>
}

@vertex
fn vs_main(vertex: Vertex, instance: Instance) -> VertexOutput {
    let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);

    var out: VertexOutput;
    out.position = view_projection * model * vec4<f32>(vertex.position, 1.0);
    out.color = vertex.color;
    out.tex_coord = vec2<f32>(vertex.position.x + 0.5, -vertex.position.y + 0.5);
    out.uv_rect = instance.uv_rect;
    out.tint = instance.tint;
    out.params = instance.params;
    out.emissive = instance.emissive;
    return out;
}

fn rotate_tex_coord(tex_coord: vec2<f32>, angle: f32) -> vec2<f32> {
    let centered = tex_coord - vec2<f32>(0.5, 0.5);
    let sine = sin(angle);
    let cosine = cos(angle);
    return vec2<f32>(centered.x * cosine - centered.y * sine, centered.x * sine + centered.y * cosine) + vec2<f32>(0.5, 0.5);
}

struct FragmentOutput {
    @location(0) albedo: vec4<f32>,
    @location(1) emissive: vec4<f32>,
    @location(2) normal: vec4<f32>
}

@fragment
fn fs_main(in: VertexOutput) -> FragmentOutput {
    let mode = in.params.x;
    let strength = in.params.y;
    let saturation_threshold = in.params.z;
    let texture_rotation = in.params.w;
    let rotated = rotate_tex_coord(in.tex_coord, texture_rotation);
    let uv_offset = in.uv_rect.xy;
    let uv_scale = in.uv_rect.zw;
    let inset = uv_scale * 0.001;
    let tex_coord = clamp(uv_offset + rotated * uv_scale, uv_offset + inset, uv_offset + uv_scale - inset);
    let sampled = textureSample(sprite_texture, sprite_sampler, tex_coord) * in.color;

    var color: vec4<f32>;
    if mode < 0.5 {
        color = sampled;
    } else if mode < 1.5 {
        color = sampled * in.tint;
    } else {
        let max_channel = max(sampled.r, max(sampled.g, sampled.b));
        let min_channel = min(sampled.r, min(sampled.g, sampled.b));
        let saturation = max_channel - min_channel;
        let grayscale_factor = 1.0 - smoothstep(0.0, saturation_threshold, saturation);
        let gray = dot(sampled.rgb, vec3<f32>(0.299, 0.587, 0.114));
        let tinted_gray = gray * in.tint.rgb;
        let mix_factor = clamp(grayscale_factor * strength, 0.0, 1.0);
        color = vec4<f32>(mix(sampled.rgb, tinted_gray, mix_factor), sampled.a * in.tint.a);
    }

    var out: FragmentOutput;
    out.albedo = color;
    out.emissive = vec4<f32>(color.rgb * in.emissive.x, color.a);
    out.normal = vec4<f32>(0.5, 0.5, 1.0, 1.0);
    return out;
}
";

const EMBEDDED_SPRITE_WGSL_BYTES: &[u8] = EMBEDDED_SPRITE_WGSL.as_bytes();

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssetSource {
	Engine,
	Game,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetRef {
	pub source: AssetSource,
	pub path: String,
}

impl AssetRef {
	pub fn engine(path: impl Into<String>) -> Self {
		Self {
			source: AssetSource::Engine,
			path: path.into(),
		}
	}

	pub fn game(path: impl Into<String>) -> Self {
		Self {
			source: AssetSource::Game,
			path: path.into(),
		}
	}
}

#[derive(Debug, Clone)]
pub struct ResourceManager {
	game_assets_root: PathBuf,
	game_pack: Option<Arc<zepack::SmartZepack>>,
}

impl ResourceManager {
	pub fn new(game_assets_root: impl Into<PathBuf>) -> Self {
		Self {
			game_assets_root: game_assets_root.into(),
			game_pack: None,
		}
	}

	pub fn for_runtime(fallback_assets_root: impl Into<PathBuf>) -> Self {
		let fallback_assets_root = fallback_assets_root.into();
		let exe_dir = std::env::current_exe()
			.ok()
			.and_then(|path| path.parent().map(Path::to_path_buf));

		if let Some(package_path) = exe_dir.as_ref().and_then(|dir| {
			let game_path = dir.join("Game").join("assets.zepack");
			if game_path.exists() {
				Some(game_path)
			} else {
				let legacy_path = dir.join("assets.zepack");
				if legacy_path.exists() { Some(legacy_path) } else { None }
			}
		}) && let Ok(package) = zepack::SmartZepack::open(&package_path)
			&& let Ok(game_assets_root) = package.materialize_to_temp()
		{
			return Self {
				game_assets_root,
				game_pack: Some(Arc::new(package)),
			};
		}

		if exe_dir.as_ref().is_some_and(|dir| path_contains_target(dir)) {
			return Self::new(fallback_assets_root);
		}

		if let Some(exe_assets_root) = exe_dir.map(|dir| dir.join("assets"))
			&& exe_assets_root.exists()
		{
			return Self::new(exe_assets_root);
		}

		Self::new(fallback_assets_root)
	}

	pub fn game_assets_root(&self) -> &Path { &self.game_assets_root }

	pub const fn has_game_pack(&self) -> bool { self.game_pack.is_some() }

	pub fn bytes(&self, asset: &AssetRef) -> Result<Cow<'static, [u8]>> {
		match asset.source {
			AssetSource::Engine => Ok(Cow::Borrowed(self.engine_bytes(&asset.path)?)),
			AssetSource::Game => Ok(Cow::Owned(self.game_bytes(&asset.path)?)),
		}
	}

	pub fn string(&self, asset: &AssetRef) -> Result<Cow<'static, str>> {
		match asset.source {
			AssetSource::Engine => Ok(Cow::Borrowed(self.engine_string(&asset.path)?)),
			AssetSource::Game => Ok(Cow::Owned(self.game_string(&asset.path)?)),
		}
	}

	pub fn game_bytes(&self, path: &str) -> Result<Vec<u8>> {
		if let Some(package) = &self.game_pack
			&& package.contains(path)
		{
			return Ok(package.read_file(path)?);
		}

		Ok(fs::read(self.resolve_game_path(path)?)?)
	}

	pub fn game_string(&self, path: &str) -> Result<String> { Ok(String::from_utf8(self.game_bytes(path)?)?) }

	pub fn engine_bytes(&self, path: &str) -> Result<&'static [u8]> {
		match path {
			"shaders/engine/sprite.wgsl" | "shaders/sprite.wgsl" => Ok(EMBEDDED_SPRITE_WGSL_BYTES),
			_ => bail!("unknown engine asset: {path}"),
		}
	}

	pub fn engine_string(&self, path: &str) -> Result<&'static str> {
		match path {
			"shaders/engine/sprite.wgsl" | "shaders/sprite.wgsl" => Ok(EMBEDDED_SPRITE_WGSL),
			_ => bail!("unknown engine asset: {path}"),
		}
	}

	pub fn compile_game_shaders(&self) -> Result<()> {
		if self.game_pack.is_some() {
			return Ok(());
		}

		let source_root = self.game_assets_root.join("shaders");
		let target_root = self.game_assets_root.join(".compiled").join("shaders");

		if !source_root.exists() {
			return Ok(());
		}

		fs::create_dir_all(&target_root)?;

		let compiler = wesl::Wesl::new(&source_root);
		compile_wesl_directory(&compiler, &source_root, &target_root, &source_root)
	}

	fn resolve_game_path(&self, path: &str) -> Result<PathBuf> {
		let relative = Path::new(path);

		if relative.is_absolute() {
			bail!("asset path must be relative: {path}");
		}

		for component in relative.components() {
			if matches!(
				component,
				Component::ParentDir | Component::RootDir | Component::Prefix(_)
			) {
				bail!("asset path cannot leave assets root: {path}");
			}
		}

		let resolved = self
			.resolve_compiled_game_shader_path(relative)
			.unwrap_or_else(|| self.game_assets_root.join(relative));
		if !resolved.exists() {
			return Err(anyhow!("asset not found: {}", resolved.display()));
		}

		Ok(resolved)
	}

	fn resolve_compiled_game_shader_path(&self, relative: &Path) -> Option<PathBuf> {
		if relative.extension().and_then(|extension| extension.to_str()) != Some("wgsl") {
			return None;
		}

		let shader_path = relative.strip_prefix(Path::new("shaders")).ok()?;
		Some(
			self.game_assets_root
				.join(".compiled")
				.join("shaders")
				.join(shader_path),
		)
	}
}

fn path_contains_target(path: &Path) -> bool {
	path.components()
		.any(|component| matches!(component, Component::Normal(name) if name == std::ffi::OsStr::new("target")))
}

fn compile_wesl_directory(
	compiler: &wesl::Wesl<wesl::StandardResolver>,
	source_root: &Path,
	target_root: &Path,
	directory: &Path,
) -> Result<()> {
	for entry in fs::read_dir(directory)? {
		let entry = entry?;
		let source_path = entry.path();

		if source_path.is_dir() {
			compile_wesl_directory(compiler, source_root, target_root, &source_path)?;
			continue;
		}

		if source_path.extension().and_then(|extension| extension.to_str()) != Some("wesl") {
			continue;
		}

		let relative = source_path.strip_prefix(source_root)?;
		let module_path = wesl_module_path(relative)?;
		let compiled = compiler.compile(&module_path.parse()?)?.to_string();

		let mut target_path = target_root.join(relative);
		target_path.set_extension("wgsl");
		if let Some(parent) = target_path.parent() {
			fs::create_dir_all(parent)?;
		}
		fs::write(target_path, compiled)?;
	}

	Ok(())
}

fn wesl_module_path(relative: &Path) -> Result<String> {
	let mut module = String::from("package");

	for component in relative.with_extension("").components() {
		let Component::Normal(part) = component else {
			bail!("invalid WESL module path: {}", relative.display());
		};
		let Some(part) = part.to_str() else {
			bail!("non-utf8 WESL module path: {}", relative.display());
		};
		module.push_str("::");
		module.push_str(part);
	}

	Ok(module)
}

pub fn copy_game_assets_to_target(
	source_assets_dir: impl AsRef<Path>,
	target_assets_dir: impl AsRef<Path>,
) -> Result<()> {
	fn copy_dir(source: &Path, target: &Path) -> Result<()> {
		fs::create_dir_all(target)?;

		for entry in fs::read_dir(source)? {
			let entry = entry?;
			let source_path = entry.path();
			let target_path = target.join(entry.file_name());

			if source_path.is_dir() {
				copy_dir(&source_path, &target_path)?;
			} else {
				if let Some(parent) = target_path.parent() {
					fs::create_dir_all(parent)?;
				}
				fs::copy(&source_path, &target_path)?;
			}
		}

		Ok(())
	}

	copy_dir(source_assets_dir.as_ref(), target_assets_dir.as_ref())
}
