use std::{
	ffi::OsStr,
	fs,
	path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use toml::to_string;
use ze_core::{Vec3, anyhow, bail};

pub const PROJECT_FILE_NAME: &str = "ZEProject.toml";

const SPRITE_WESL_TEMPLATE: &str = "@group(0) @binding(0) var sprite_texture: texture_2d<f32>;
@group(0) @binding(1) var sprite_sampler: sampler;

struct Material {
    tint: vec4<f32>,
    params: vec4<f32>,
    uv_rect: vec4<f32>,
}

@group(0) @binding(2) var<uniform> material: Material;

@group(1) @binding(0) var<uniform> transform: mat4x4<f32>;
@group(2) @binding(0) var<uniform> view_projection: mat4x4<f32>;

struct Vertex {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) tex_coord: vec2<f32>,
}

@vertex
fn vs_main(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    out.position = view_projection * transform * vec4<f32>(vertex.position, 1.0);
    out.color = vertex.color;
    out.tex_coord = vec2<f32>(vertex.position.x + 0.5, -vertex.position.y + 0.5);
    return out;
}

fn rotate_tex_coord(tex_coord: vec2<f32>, angle: f32) -> vec2<f32> {
    let centered = tex_coord - vec2<f32>(0.5, 0.5);
    let sine = sin(angle);
    let cosine = cos(angle);
    return vec2<f32>(
        centered.x * cosine - centered.y * sine,
        centered.x * sine + centered.y * cosine,
    ) + vec2<f32>(0.5, 0.5);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let mode = material.params.x;
    let strength = material.params.y;
    let saturation_threshold = material.params.z;
    let texture_rotation = material.params.w;

    let tex_coord = clamp(rotate_tex_coord(in.tex_coord, texture_rotation), vec2<f32>(0.001, 0.001), vec2<f32>(0.999, 0.999));
    let sampled = textureSample(sprite_texture, sprite_sampler, tex_coord) * in.color;

    if mode < 0.5 {
        return sampled;
    }

    if mode < 1.5 {
        return sampled * material.tint;
    }

    let max_channel = max(sampled.r, max(sampled.g, sampled.b));
    let min_channel = min(sampled.r, min(sampled.g, sampled.b));
    let saturation = max_channel - min_channel;
    let grayscale_factor = 1.0 - smoothstep(0.0, saturation_threshold, saturation);
    let gray = dot(sampled.rgb, vec3<f32>(0.299, 0.587, 0.114));
    let tinted_gray = gray * material.tint.rgb;
    let mix_factor = clamp(grayscale_factor * strength, 0.0, 1.0);

    return vec4<f32>(mix(sampled.rgb, tinted_gray, mix_factor), sampled.a * material.tint.a);
}
";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Project {
	pub name: String,
	#[serde(default = "default_main_scene", alias = "start_scene")]
	pub main_scene: String,
	#[serde(default)]
	pub scenes: Vec<String>,
	#[serde(default = "default_asset_dir")]
	pub asset_dir: PathBuf,
	#[serde(default = "default_scripts_dll")]
	pub scripts_dll: PathBuf,
	#[serde(default)]
	pub physics: PhysicsSettings,
	#[serde(default)]
	pub game: GameSettings,
	#[serde(skip)]
	pub root_dir: PathBuf,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PhysicsSettings {
	#[serde(default = "default_gravity")]
	pub gravity: Vec3,
	#[serde(default = "default_physics_timestep")]
	pub timestep: f32,
	#[serde(default = "default_solver_iterations")]
	pub solver_iterations: u32,
	#[serde(default)]
	pub enable_debug_draw: bool,
}

impl Default for PhysicsSettings {
	fn default() -> Self {
		Self {
			gravity: default_gravity(),
			timestep: default_physics_timestep(),
			solver_iterations: default_solver_iterations(),
			enable_debug_draw: false,
		}
	}
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GameSettings {
	#[serde(default = "default_game_name")]
	pub name: String,
	#[serde(default = "default_game_version")]
	pub version: String,
	#[serde(default)]
	pub icon_path: Option<PathBuf>,
}

impl Default for GameSettings {
	fn default() -> Self {
		Self {
			name: default_game_name(),
			version: default_game_version(),
			icon_path: None,
		}
	}
}

impl Project {
	pub fn new(name: String, main_scene: String, asset_dir: PathBuf, scripts_dll: PathBuf) -> Project {
		let game_name = name.clone();
		let scenes = vec![main_scene.clone()];
		Self {
			name,
			main_scene,
			scenes,
			asset_dir,
			scripts_dll,
			physics: PhysicsSettings::default(),
			game: GameSettings {
				name: game_name,
				..GameSettings::default()
			},
			root_dir: PathBuf::new(),
		}
	}

	pub fn scripts_dir(&self) -> ze_core::Result<&Path> {
		let output_dir = self
			.scripts_dll
			.parent()
			.ok_or_else(|| anyhow!("scripts_dll does not have parent directory"))?;

		if output_dir.file_name() == Some(OsStr::new("bin")) {
			return output_dir
				.parent()
				.ok_or_else(|| anyhow!("scripts_dll bin directory does not have parent directory"));
		}

		Ok(output_dir)
	}

	pub fn csproj_path(&self) -> ze_core::Result<PathBuf> {
		Ok(self.scripts_dir()?.join(format!("{}.csproj", self.name)))
	}

	pub fn game_data_path(&self) -> PathBuf { self.asset_dir.join("GameData.toml") }

	pub fn runtime_config_path(&self) -> PathBuf {
		let runtime_config_name = self.scripts_dll.file_stem().and_then(|name| name.to_str()).map_or_else(
			|| "Scripts.runtimeconfig.json".to_string(),
			|name| format!("{name}.runtimeconfig.json"),
		);
		self.scripts_dll.with_file_name(runtime_config_name)
	}

	pub fn generate_csproj(&self) -> ze_core::Result<PathBuf> { self.generate_csproj_with_api_source(None) }

	pub fn generate_csproj_with_api_source(&self, api_source_dir: Option<&Path>) -> ze_core::Result<PathBuf> {
		let api_dir = match api_source_dir {
			Some(dir) => dir.to_path_buf(),
			None => ze_core::engine_api_dir()?,
		};

		let csproj_path = self.csproj_path()?;

		let assembly_name = self
			.scripts_dll
			.file_stem()
			.and_then(|name| name.to_str())
			.unwrap_or("Scripts");

		let csproj = format!(
			r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net10.0</TargetFramework>
    <RollForward>LatestMajor</RollForward>
    <Nullable>enable</Nullable>
    <ImplicitUsings>enable</ImplicitUsings>
    <AssemblyName>{assembly_name}</AssemblyName>
    <RootNamespace>{assembly_name}</RootNamespace>
    <GenerateRuntimeConfigurationFiles>true</GenerateRuntimeConfigurationFiles>
    <EnableDynamicLoading>true</EnableDynamicLoading>
    <OutputPath>bin/</OutputPath>
    <AppendTargetFrameworkToOutputPath>false</AppendTargetFrameworkToOutputPath>
    <AppendRuntimeIdentifierToOutputPath>false</AppendRuntimeIdentifierToOutputPath>
    <AllowUnsafeBlocks>true</AllowUnsafeBlocks>
    <InvariantGlobalization>true</InvariantGlobalization>
  </PropertyGroup>

  <ItemGroup>
    <Compile Include="{}/**/*.cs" Link="ZeroEngine/api/cs/public/%(RecursiveDir)%(Filename)%(Extension)" />
  </ItemGroup>

  <ItemGroup>
    <Watch Remove="bin/**" />
    <Watch Remove="obj/**" />
  </ItemGroup>
</Project>
"#,
			api_dir.display(),
		);

		fs::create_dir_all(
			csproj_path
				.parent()
				.ok_or_else(|| anyhow!("csproj path has no parent directory"))?,
		)?;

		// Only rewrite the csproj when its content has actually changed.
		// MSBuild uses file mtime to decide whether its incremental-build
		// cache is still valid.  Unconditionally writing the same bytes
		// bumps the mtime, which forces MSBuild to re-evaluate the project
		// and typically triggers a full rebuild — exactly the opposite of
		// the incremental behaviour we want during hot-reload iterations.
		let existing = fs::read_to_string(&csproj_path).unwrap_or_default();
		if existing == csproj {
			ze_log::debug!(
				"[ze_project] skipped csproj write (content unchanged): {}",
				csproj_path.display()
			);
		} else {
			ze_log::debug!("writing updated csproj: {}", csproj_path.display());
			fs::write(&csproj_path, csproj)?;
		}

		Ok(csproj_path)
	}

	pub fn load(path: impl AsRef<Path>) -> ze_core::Result<Self> {
		let path = path.as_ref();
		// Canonicalize so `base_dir` (and thus `asset_dir`/`scripts_dll`/`root_dir`) is
		// always absolute and symlink-resolved, matching canonicalized asset
		// selection paths elsewhere -- otherwise prefix-stripping an asset's
		// canonical path against a relative `asset_dir` (e.g. when launched with a
		// relative project path) silently fails everywhere it's used.
		let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
		let source = fs::read_to_string(&path)?;
		let mut project = toml::from_str::<Self>(&source)?;
		let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

		project.asset_dir = resolve_project_path(base_dir, &project.asset_dir);
		project.scripts_dll = resolve_project_path(base_dir, &project.scripts_dll);
		if let Some(icon_path) = project.game.icon_path.as_mut() {
			*icon_path = resolve_project_path(base_dir, icon_path);
		}
		project.root_dir = base_dir.to_path_buf();
		project.normalize_scene_settings();
		project.validate_structure()?;

		Ok(project)
	}

	pub fn load_from_dir(directory: impl AsRef<Path>) -> ze_core::Result<Self> {
		Self::load(directory.as_ref().join(PROJECT_FILE_NAME))
	}

	pub fn create_new(directory: impl AsRef<Path>, name: impl Into<String>) -> ze_core::Result<Self> {
		let directory = directory.as_ref();
		let name = name.into();
		let assets_dir = directory.join("assets");
		fs::create_dir_all(assets_dir.join("scenes"))?;
		fs::create_dir_all(assets_dir.join("scripts"))?;
		fs::create_dir_all(assets_dir.join("shaders"))?;
		let sprite_wesl_path = assets_dir.join("shaders").join("sprite.wesl");
		if !sprite_wesl_path.exists() {
			fs::write(&sprite_wesl_path, SPRITE_WESL_TEMPLATE)?;
		}
		fs::create_dir_all(assets_dir.join("textures"))?;

		let root_gitignore = directory.join(".gitignore");
		if !root_gitignore.exists() {
			fs::write(&root_gitignore, ".compiled/\n")?;
		}
		let root_zeignore = directory.join(".zeignore");
		if !root_zeignore.exists() {
			fs::write(&root_zeignore, ".compiled\n")?;
		}

		let assets_gitignore = assets_dir.join(".gitignore");
		if !assets_gitignore.exists() {
			fs::write(&assets_gitignore, "*.csproj\nbin/\nobj/\n")?;
		}
		let assets_zeignore = assets_dir.join(".zeignore");
		if !assets_zeignore.exists() {
			fs::write(&assets_zeignore, "*.csproj\nbin/\nobj/\n")?;
		}

		let mut project = Self::new(
			name.clone(),
			"Main".to_string(),
			assets_dir,
			directory.join("assets/bin").join(format!("{name}.dll")),
		);
		project.root_dir = directory.to_path_buf();
		project.game.name = name;
		project.normalize_scene_settings();
		project.save_to_project_file()?;
		project.generate_csproj()?;
		Ok(project)
	}

	pub fn validate_structure(&self) -> ze_core::Result<()> {
		if self.name.trim().is_empty() {
			bail!("project name cannot be empty");
		}
		if self.game.name.trim().is_empty() {
			bail!("game name cannot be empty");
		}
		if !self.asset_dir.is_dir() {
			bail!("project assets directory does not exist: {}", self.asset_dir.display());
		}
		validate_scene_name(&self.main_scene)?;
		if self.scenes.is_empty() {
			bail!("project must contain at least one scene");
		}
		for scene in &self.scenes {
			validate_scene_name(scene)?;
		}
		if !self.scenes.iter().any(|scene| scene == &self.main_scene) {
			bail!("main scene `{}` is not listed in project scenes", self.main_scene);
		}

		Ok(())
	}

	pub fn add_scene(&mut self, scene: impl Into<String>) -> ze_core::Result<bool> {
		let scene = scene.into().trim().to_string();
		validate_scene_name(&scene)?;
		if self.scenes.iter().any(|existing| existing == &scene) {
			return Ok(false);
		}

		self.scenes.push(scene);
		self.scenes.sort();
		Ok(true)
	}

	pub fn set_main_scene(&mut self, scene: impl Into<String>) -> ze_core::Result<()> {
		let scene = scene.into().trim().to_string();
		validate_scene_name(&scene)?;
		if !self.scenes.iter().any(|existing| existing == &scene) {
			self.scenes.push(scene.clone());
			self.scenes.sort();
		}
		self.main_scene = scene;
		Ok(())
	}

	pub fn sync_scenes_from_disk(&mut self) -> ze_core::Result<bool> {
		self.normalize_scene_settings();

		let mut scenes = scene_names_from_directory(&self.asset_dir.join("scenes"))?;
		if !scenes.iter().any(|scene| scene == &self.main_scene) {
			scenes.push(self.main_scene.clone());
		}
		scenes.sort();
		scenes.dedup();

		let changed = self.scenes != scenes;
		self.scenes = scenes;
		Ok(changed)
	}

	pub fn scene_names_from_disk(&self) -> ze_core::Result<Vec<String>> {
		scene_names_from_directory(&self.asset_dir.join("scenes"))
	}

	pub fn save(&self, path: impl AsRef<Path>) -> ze_core::Result<()> {
		let path = path.as_ref();

		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent)?;
		}

		let mut project = self.clone();
		project.normalize_scene_settings();
		let source = to_string(&project.serializable_at(path.parent().unwrap_or_else(|| Path::new("."))))?;
		fs::write(path, source)?;

		Ok(())
	}

	pub fn save_to_project_file(&self) -> ze_core::Result<()> {
		let path = if self.root_dir.as_os_str().is_empty() {
			PathBuf::from(PROJECT_FILE_NAME)
		} else {
			self.root_dir.join(PROJECT_FILE_NAME)
		};
		self.save(path)
	}

	fn serializable_at(&self, base_dir: &Path) -> Self {
		let mut project = self.clone();
		project.normalize_scene_settings();
		project.asset_dir = path_relative_to(base_dir, &project.asset_dir);
		project.scripts_dll = path_relative_to(base_dir, &project.scripts_dll);
		if let Some(icon_path) = project.game.icon_path.as_mut() {
			*icon_path = path_relative_to(base_dir, icon_path);
		}
		project
	}

	fn normalize_scene_settings(&mut self) {
		self.main_scene = normalized_scene_name(&self.main_scene).unwrap_or_else(default_main_scene);
		if self.scenes.is_empty() {
			self.scenes.push(self.main_scene.clone());
		}

		self.scenes = self
			.scenes
			.iter()
			.filter_map(|scene| normalized_scene_name(scene))
			.collect();
		if !self.scenes.iter().any(|scene| scene == &self.main_scene) {
			self.scenes.push(self.main_scene.clone());
		}
		self.scenes.sort();
		self.scenes.dedup();
	}
}

fn resolve_project_path(base_dir: &Path, path: &Path) -> PathBuf {
	if path.is_absolute() {
		return path.to_path_buf();
	}

	base_dir.join(path)
}

fn default_main_scene() -> String { "main".to_string() }

fn default_asset_dir() -> PathBuf { PathBuf::from("assets") }

fn default_scripts_dll() -> PathBuf { PathBuf::from("assets/bin/Scripts.dll") }

fn default_gravity() -> Vec3 { Vec3::new(0.0, -9.81, 0.0) }

fn default_physics_timestep() -> f32 { 1.0 / 70.0 }

const fn default_solver_iterations() -> u32 { 8 }

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct GameData {
	#[serde(default)]
	pub tags: Vec<String>,
}

impl GameData {
	pub fn load(path: impl AsRef<Path>) -> ze_core::Result<Self> {
		let source = fs::read_to_string(path.as_ref())?;
		Ok(toml::from_str(&source)?)
	}

	pub fn save(&self, path: impl AsRef<Path>) -> ze_core::Result<()> {
		if let Some(parent) = path.as_ref().parent() {
			fs::create_dir_all(parent)?;
		}
		fs::write(path.as_ref(), to_string(self)?)?;
		Ok(())
	}

	pub fn load_or_create(path: impl AsRef<Path>) -> Self {
		if path.as_ref().is_file() {
			Self::load(path).unwrap_or_default()
		} else {
			let data = GameData {
				tags: vec![
					"Untagged".to_string(),
					"Camera".to_string(),
					"Player".to_string(),
					"Enemy".to_string(),
					"StaticGeometry".to_string(),
				],
			};
			let _ = data.save(path);
			data
		}
	}
}

fn default_game_name() -> String { "ZeroEngine Game".to_string() }

fn default_game_version() -> String { "0.1.0".to_string() }

fn path_relative_to(base_dir: &Path, path: &Path) -> PathBuf {
	if let Ok(relative) = path.strip_prefix(base_dir)
		&& !relative.as_os_str().is_empty()
	{
		return relative.to_path_buf();
	}

	path.to_path_buf()
}

fn scene_names_from_directory(directory: &Path) -> ze_core::Result<Vec<String>> {
	let entries = match fs::read_dir(directory) {
		Ok(entries) => entries,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
		Err(error) => return Err(error.into()),
	};

	let mut scenes = Vec::new();
	for entry in entries {
		let entry = entry?;
		let path = entry.path();
		if !path.is_file() {
			continue;
		}
		let Some(scene) = scene_name_from_path(&path) else {
			continue;
		};
		validate_scene_name(&scene)?;
		scenes.push(scene);
	}

	scenes.sort();
	scenes.dedup();
	Ok(scenes)
}

fn scene_name_from_path(path: &Path) -> Option<String> {
	let file_name = path.file_name()?.to_str()?;
	file_name
		.strip_suffix(".zescene.json")
		.map(std::string::ToString::to_string)
}

fn normalized_scene_name(scene: &str) -> Option<String> {
	let scene = scene.trim();
	(!scene.is_empty()).then(|| scene.to_string())
}

fn validate_scene_name(scene: &str) -> ze_core::Result<()> {
	if scene.trim().is_empty() {
		bail!("scene name cannot be empty");
	}
	if scene != scene.trim() {
		bail!("scene name `{scene}` must not have leading or trailing whitespace");
	}
	if scene == "." || scene == ".." || scene.contains('/') || scene.contains('\\') {
		bail!("scene name `{scene}` must be a file stem, not a path");
	}
	if scene.ends_with(".zescene.json") {
		bail!("scene name `{scene}` must not include the scene file extension");
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn unique_test_dir(name: &str) -> PathBuf {
		let nanos = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.expect("system clock is before UNIX_EPOCH")
			.as_nanos();
		std::env::temp_dir().join(format!("zeroengine_{name}_{nanos}"))
	}

	#[test]
	fn csproj_path_uses_script_source_dir_when_dll_is_in_bin() -> ze_core::Result<()> {
		let project = Project::new(
			"Sandbox".to_string(),
			"main".to_string(),
			PathBuf::from("assets"),
			PathBuf::from("assets/bin/Sandbox.dll"),
		);

		assert_eq!(project.csproj_path()?, PathBuf::from("assets/Sandbox.csproj"));
		Ok(())
	}

	#[test]
	fn csproj_path_uses_dll_parent_when_dll_is_not_in_bin() -> ze_core::Result<()> {
		let project = Project::new(
			"Sandbox".to_string(),
			"main".to_string(),
			PathBuf::from("assets"),
			PathBuf::from("assets/Sandbox.dll"),
		);

		assert_eq!(project.csproj_path()?, PathBuf::from("assets/Sandbox.csproj"));
		Ok(())
	}

	#[test]
	fn legacy_start_scene_loads_as_main_scene() -> ze_core::Result<()> {
		let mut project = toml::from_str::<Project>(
			r#"
name = "Sandbox"
start_scene = "level_one"
"#,
		)?;

		project.normalize_scene_settings();

		assert_eq!(project.main_scene, "level_one");
		assert_eq!(project.scenes, vec!["level_one"]);
		Ok(())
	}

	#[test]
	fn serialized_project_uses_main_scene_and_scenes() -> ze_core::Result<()> {
		let project = Project::new(
			"Sandbox".to_string(),
			"main".to_string(),
			PathBuf::from("assets"),
			PathBuf::from("assets/bin/Sandbox.dll"),
		);

		let source = toml::to_string(&project)?;

		assert!(source.contains("main_scene = \"main\""));
		assert!(source.contains("scenes = [\"main\"]"));
		assert!(!source.contains("start_scene"));
		Ok(())
	}

	#[test]
	fn sync_scenes_from_disk_adds_scene_files() -> ze_core::Result<()> {
		let root = unique_test_dir("scene_scan");
		let assets = root.join("assets");
		let scenes = assets.join("scenes");
		fs::create_dir_all(&scenes)?;
		fs::write(scenes.join("main.zescene.json"), "{}")?;
		fs::write(scenes.join("LevelOne.zescene.json"), "{}")?;
		fs::write(scenes.join("notes.txt"), "")?;

		let mut project = Project::new(
			"Sandbox".to_string(),
			"main".to_string(),
			assets,
			root.join("assets/bin/Sandbox.dll"),
		);
		project.root_dir = root.clone();

		assert!(project.sync_scenes_from_disk()?);
		assert_eq!(project.scenes, vec!["LevelOne", "main"]);

		let _ = fs::remove_dir_all(root);
		Ok(())
	}

	#[test]
	fn save_preserves_scene_selection() -> ze_core::Result<()> {
		let root = unique_test_dir("project_save_selection");
		let assets = root.join("assets");
		let scenes = assets.join("scenes");
		fs::create_dir_all(&scenes)?;
		fs::write(scenes.join("main.zescene.json"), "{}")?;
		fs::write(scenes.join("Menu.zescene.json"), "{}")?;

		let mut project = Project::new(
			"Sandbox".to_string(),
			"main".to_string(),
			assets,
			root.join("assets/bin/Sandbox.dll"),
		);
		project.root_dir = root.clone();
		project.save_to_project_file()?;

		let source = fs::read_to_string(root.join(PROJECT_FILE_NAME))?;
		assert!(source.contains("main_scene = \"main\""));
		assert!(source.contains("scenes = [\"main\"]"));
		assert!(!source.contains("Menu"));

		let _ = fs::remove_dir_all(root);
		Ok(())
	}

	#[test]
	fn load_preserves_scene_selection() -> ze_core::Result<()> {
		let root = unique_test_dir("project_load_selection");
		let assets = root.join("assets");
		let scenes = assets.join("scenes");
		fs::create_dir_all(&scenes)?;
		fs::write(scenes.join("main.zescene.json"), "{}")?;
		fs::write(scenes.join("Menu.zescene.json"), "{}")?;

		let mut project = Project::new(
			"Sandbox".to_string(),
			"main".to_string(),
			assets,
			root.join("assets/bin/Sandbox.dll"),
		);
		project.root_dir = root.clone();
		project.save_to_project_file()?;

		let loaded = Project::load(root.join(PROJECT_FILE_NAME))?;
		assert_eq!(loaded.scenes, vec!["main"]);
		assert_eq!(loaded.scene_names_from_disk()?, vec!["Menu", "main"]);

		let _ = fs::remove_dir_all(root);
		Ok(())
	}
}
