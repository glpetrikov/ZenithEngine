use std::{
	fs, io,
	path::{Path, PathBuf},
	process::Command,
};

use ze_core::Context;
use ze_project::Project;

const EDITOR_ONLY_COMPONENT_TYPE: &str = "ze.editor.only";
const CAMERA_COMPONENT_TYPE: &str = "ze.renderer.camera";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildTarget {
	Linux,
	Windows,
}

impl BuildTarget {
	#[must_use]
	pub const fn host() -> Self {
		if cfg!(target_os = "windows") {
			Self::Windows
		} else {
			Self::Linux
		}
	}

	#[must_use]
	pub const fn target_triple(self) -> &'static str {
		match self {
			Self::Linux => "x86_64-unknown-linux-gnu",
			Self::Windows => {
				if cfg!(target_os = "linux") {
					"x86_64-pc-windows-gnu"
				} else {
					"x86_64-pc-windows-msvc"
				}
			}
		}
	}

	#[must_use]
	pub const fn binary_name(self) -> &'static str {
		match self {
			Self::Linux => "ZeroEngine",
			Self::Windows => "ZeroEngine.exe",
		}
	}

	#[must_use]
	pub const fn dotnet_rid(self) -> &'static str {
		match self {
			Self::Linux => "linux-x64",
			Self::Windows => "win-x64",
		}
	}
}

#[derive(Debug, Clone)]
pub struct BuildOptions {
	pub configuration: String,
	pub output_dir: PathBuf,
	pub zeroengine_binary: PathBuf,
	pub notice_path: PathBuf,
	pub api_source_dir: Option<PathBuf>,
	pub target: BuildTarget,
}

impl BuildOptions {
	pub fn release(output_dir: impl Into<PathBuf>, notice_path: impl Into<PathBuf>) -> Self {
		Self {
			configuration: "Release".to_string(),
			output_dir: output_dir.into(),
			zeroengine_binary: zeroengine_binary_path(BuildTarget::host()),
			notice_path: notice_path.into(),
			api_source_dir: None,
			target: BuildTarget::host(),
		}
	}

	#[must_use]
	pub fn debug(output_dir: impl Into<PathBuf>, notice_path: impl Into<PathBuf>) -> Self {
		Self {
			configuration: "Debug".to_string(),
			output_dir: output_dir.into(),
			zeroengine_binary: zeroengine_binary_path(BuildTarget::host()),
			notice_path: notice_path.into(),
			api_source_dir: None,
			target: BuildTarget::host(),
		}
	}

	#[must_use]
	pub fn with_zeroengine_binary(mut self, zeroengine_binary: impl Into<PathBuf>) -> Self {
		self.zeroengine_binary = zeroengine_binary.into();
		self
	}

	#[must_use]
	pub fn with_api_source_dir(mut self, api_source_dir: impl Into<PathBuf>) -> Self {
		self.api_source_dir = Some(api_source_dir.into());
		self
	}

	#[must_use]
	pub fn with_target(mut self, target: BuildTarget) -> Self {
		self.target = target;
		self.zeroengine_binary = zeroengine_binary_path(target);
		self
	}
}

#[derive(Debug, Clone)]
pub struct BuildOutput {
	pub output_dir: PathBuf,
	pub assets_pack: PathBuf,
	pub scripts_dll: PathBuf,
	pub scripts_deps: PathBuf,
	pub scripts_runtime_config: PathBuf,
	pub zeroengine_binary: PathBuf,
	pub notice: PathBuf,
}

pub fn build_project(project_file: impl AsRef<Path>, options: &BuildOptions) -> ze_core::Result<BuildOutput> {
	let project = Project::load(project_file)?;
	build_project_dist(&project, options)
}

pub fn build_project_dist(project: &Project, options: &BuildOptions) -> ze_core::Result<BuildOutput> {
	project.validate_structure()?;
	validate_included_scene_primary_cameras(project)?;

	// Ensure GameData.toml exists in the project assets dir so it gets packed
	// into assets.zepack via the staging step below.
	let game_data_path = project.game_data_path();
	if !game_data_path.is_file() {
		ze_project::GameData::default().save(&game_data_path)?;
	}

	build_scripts_assembly(project, &options.configuration, options.api_source_dir.as_deref())?;

	// Publish the project self-contained so hostfxr and all framework
	// assemblies are bundled into the dist. The output goes to a temp
	// directory to avoid cluttering the project tree.
	let publish_temp = std::env::temp_dir().join(format!("ze-publish-{}", std::process::id()));
	let publish_result = publish_self_contained_scripts(
		project,
		&options.configuration,
		options.api_source_dir.as_deref(),
		options.target.dotnet_rid(),
		&publish_temp,
	);
	if publish_result.is_err() {
		let _ = fs::remove_dir_all(&publish_temp);
	}
	publish_result?;

	let output_dir = &options.output_dir;
	replace_directory(output_dir)
		.with_context(|| format!("failed to create output directory {}", output_dir.display()))?;

	let staged_assets_dir = create_staged_assets_without_scripts(output_dir, &project.asset_dir, &project.scenes)
		.with_context(|| format!("failed to stage assets in {}", output_dir.display()))?;

	// Pack runtimeconfig.json and deps.json into the zepack so they are not
	// left as editable loose files on disk. At game startup they will be
	// extracted to a temp directory before the .NET runtime is initialized.
	copy_dotnet_configs_to_staging(project, &staged_assets_dir)?;

	let assets_pack_dir = output_dir.join("Game");
	let assets_pack = assets_pack_dir.join("assets.zepack");
	zepack::pack_directory(&staged_assets_dir, &assets_pack)?;
	remove_dir_if_exists(&staged_assets_dir)
		.with_context(|| format!("failed to remove staging directory {}", staged_assets_dir.display()))?;

	let scripts_dll = assets_pack_dir.join(
		project
			.scripts_dll
			.file_name()
			.ok_or_else(|| ze_core::anyhow!("scripts_dll path has no file name"))?,
	);
	copy_file(&project.scripts_dll, &scripts_dll).with_context(|| {
		format!(
			"failed to copy script assembly from {} to {}",
			project.scripts_dll.display(),
			scripts_dll.display()
		)
	})?;

	copy_dotnet_runtime_to_dist(project, output_dir, &publish_temp)?;
	let _ = fs::remove_dir_all(&publish_temp);

	let notice = output_dir.join("Engine").join("NOTICE");
	copy_file(&options.notice_path, &notice).with_context(|| {
		format!(
			"failed to copy NOTICE from {} to {}",
			options.notice_path.display(),
			notice.display()
		)
	})?;

	let source_runtime_binary = resolve_zeroengine_binary(&options.zeroengine_binary, options.target)?;
	let extension = source_runtime_binary
		.extension()
		.and_then(|extension| extension.to_str())
		.unwrap_or_default();
	let binary_name = executable_file_name(&project.game.name, extension);
	let zeroengine_binary = output_dir.join(binary_name);
	copy_runtime_binary(&source_runtime_binary, &zeroengine_binary).with_context(|| {
		format!(
			"failed to copy runtime binary from {} to {}",
			source_runtime_binary.display(),
			zeroengine_binary.display()
		)
	})?;

	Ok(BuildOutput {
		output_dir: output_dir.clone(),
		assets_pack,
		scripts_dll,
		scripts_deps: PathBuf::new(),
		scripts_runtime_config: PathBuf::new(),
		zeroengine_binary,
		notice,
	})
}

fn copy_dotnet_configs_to_staging(project: &Project, staged_assets_dir: &Path) -> ze_core::Result<()> {
	let config_staging = staged_assets_dir.join("dotnet-config");
	fs::create_dir_all(&config_staging)?;

	let project_runtime_config = project.runtime_config_path();
	copy_file(
		&project_runtime_config,
		&config_staging.join(
			project_runtime_config
				.file_name()
				.ok_or_else(|| ze_core::anyhow!("runtime config path has no file name"))?,
		),
	)?;

	let project_scripts_deps = project.scripts_dll.with_extension("deps.json");
	copy_file(
		&project_scripts_deps,
		&config_staging.join(
			project_scripts_deps
				.file_name()
				.ok_or_else(|| ze_core::anyhow!("scripts deps path has no file name"))?,
		),
	)?;

	Ok(())
}

fn publish_self_contained_scripts(
	project: &Project,
	configuration: &str,
	api_source_dir: Option<&Path>,
	rid: &str,
	publish_output_dir: &Path,
) -> ze_core::Result<()> {
	let project_path = project.generate_csproj_with_api_source(api_source_dir)?;

	// The generated csproj hardcodes `<OutputPath>bin/</OutputPath>` (with
	// `Append{TargetFramework,RuntimeIdentifier}ToOutputPath` both false), so
	// it resolves to the same `assets/bin/` directory regardless of
	// configuration/RID/self-contained-ness. `dotnet publish` runs a normal
	// Build first and writes *that* intermediate output to `OutputPath`
	// before copying the publish-ready result to `-o` -- so without
	// overriding `OutputPath` here, this self-contained/RID-specific
	// publish's own internal build step would silently overwrite the
	// framework-dependent `Sandbox.dll`/`Sandbox.runtimeconfig.json` that
	// `build_scripts_assembly` just produced in `assets/bin/`, in place,
	// every time a dist build runs.
	let build_output_dir = publish_output_dir.join("build");
	let status = Command::new("dotnet")
		.arg("publish")
		.arg(&project_path)
		.arg("--configuration")
		.arg(configuration)
		.arg("--self-contained")
		.arg("true")
		.arg("-r")
		.arg(rid)
		.arg("--nologo")
		.arg("-o")
		.arg(publish_output_dir)
		.arg(format!(
			"-p:OutputPath={}{}",
			build_output_dir.display(),
			std::path::MAIN_SEPARATOR
		))
		.arg("-p:EnableDynamicLoading=true")
		.status()
		.map_err(|error| {
			io::Error::new(
				error.kind(),
				format!("failed to run dotnet publish for {}: {error}", project_path.display()),
			)
		})?;

	// Publishing with -r/--self-contained leaves a self-contained-flavored
	// restore graph (RID-specific assets, `includedFrameworks` instead of
	// `frameworks` in runtimeconfig.json) cached in the project's obj/
	// directory. `dotnet build` (in `build_scripts_assembly`) shares that
	// obj/ directory, so without clearing it here, the *next* plain build of
	// this project would silently pick up the cached self-contained restore
	// and regenerate `Sandbox.runtimeconfig.json` -- the one actually shipped
	// and loaded via `hostfxr_initialize_for_runtime_config` at runtime -- in
	// the self-contained shape, which that hosting API does not support
	// ("Initialization for self-contained components is not supported").
	if let Some(project_dir) = project_path.parent() {
		let _ = fs::remove_dir_all(project_dir.join("obj"));
	}

	if !status.success() {
		ze_core::bail!("dotnet publish failed for {}", project_path.display());
	}

	Ok(())
}

fn copy_dotnet_runtime_to_dist(project: &Project, output_dir: &Path, publish_dir: &Path) -> ze_core::Result<()> {
	if !publish_dir.exists() {
		ze_core::bail!("self-contained publish output not found at {}", publish_dir.display());
	}

	let dotnet_dir = output_dir.join("dotnet");
	fs::create_dir_all(&dotnet_dir)?;

	let mut copied_any = false;
	for entry in fs::read_dir(publish_dir)? {
		let entry = entry?;
		let path = entry.path();
		let file_name = entry.file_name();

		// Skip the game assembly — it is placed in Game/ separately.
		if file_name == project.scripts_dll.file_name().unwrap_or_default() {
			continue;
		}

		// Skip config files — they go into assets.zepack.
		if file_name
			.to_str()
			.is_some_and(|name| name.ends_with(".runtimeconfig.json") || name.ends_with(".deps.json"))
		{
			continue;
		}

		// Skip debug symbols.
		if path.extension().is_some_and(|ext| ext == "pdb") {
			continue;
		}

		// Skip XML documentation files.
		if path.extension().is_some_and(|ext| ext == "xml") {
			continue;
		}

		// Skip satellite resource directories (culture-specific assemblies).
		if path.is_dir()
			&& file_name
				.to_str()
				.is_some_and(|name| name.len() == 2 || (name.len() == 5 && name.contains('-')))
		{
			continue;
		}

		copy_recursive(&path, &dotnet_dir.join(&file_name))?;
		copied_any = true;
	}

	if !copied_any {
		ze_core::bail!(
			"self-contained publish output at {} contained no runtime files to copy into {}",
			publish_dir.display(),
			dotnet_dir.display()
		);
	}

	Ok(())
}

fn copy_recursive(source: &Path, target: &Path) -> io::Result<()> {
	if source.is_dir() {
		fs::create_dir_all(target)?;
		for entry in fs::read_dir(source)? {
			let entry = entry?;
			copy_recursive(&entry.path(), &target.join(entry.file_name()))?;
		}
	} else {
		let _ = fs::copy(source, target)?;
	}
	Ok(())
}

fn ensure_rustup_target(triple: &str) -> ze_core::Result<()> {
	let output = Command::new("rustup")
		.args(["target", "list", "--installed"])
		.output()
		.map_err(|error| io::Error::new(error.kind(), format!("failed to run rustup target list: {error}")))?;

	let installed = String::from_utf8_lossy(&output.stdout);
	if !installed.lines().any(|line| line.trim() == triple) {
		ze_log::info!("adding rustup target: {triple}");
		let status = Command::new("rustup")
			.args(["target", "add", triple])
			.status()
			.map_err(|error| io::Error::new(error.kind(), format!("failed to run rustup target add: {error}")))?;
		if !status.success() {
			ze_core::bail!("rustup target add {triple} failed");
		}
	}
	Ok(())
}

fn executable_file_name(game_name: &str, extension: &str) -> String {
	let mut name = sanitize_file_stem(game_name);
	if name.is_empty() {
		name.push_str("ZeroEngineGame");
	}
	if !extension.is_empty() {
		name.push('.');
		name.push_str(extension);
	}
	name
}

fn sanitize_file_stem(value: &str) -> String {
	value
		.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
				character
			} else {
				'_'
			}
		})
		.collect::<String>()
		.trim_matches('_')
		.to_string()
}

fn zeroengine_binary_path(target: BuildTarget) -> PathBuf {
	match runtime_binary_mode() {
		RuntimeBinaryMode::Dev { workspace_root } => workspace_runtime_binary_path(&workspace_root, target),
		RuntimeBinaryMode::Release { executable_dir } => executable_dir.join(target.binary_name()),
	}
}

enum RuntimeBinaryMode {
	Dev { workspace_root: PathBuf },
	Release { executable_dir: PathBuf },
}

fn runtime_binary_mode() -> RuntimeBinaryMode {
	let current_exe = std::env::current_exe().ok();
	let executable_dir = current_exe.as_deref().and_then(Path::parent).map_or_else(
		|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
		Path::to_path_buf,
	);

	match option_env!("ZEROENGINE_EDITOR_RUNTIME_MODE") {
		Some("dev") => {
			let workspace_root = find_cargo_workspace_from(&executable_dir).unwrap_or_else(|| {
				panic!(
					"ZEROENGINE_EDITOR_RUNTIME_MODE=dev but no cargo workspace found from executable path {}",
					executable_dir.display()
				)
			});
			return RuntimeBinaryMode::Dev { workspace_root };
		}
		Some("release" | "packaged") => return RuntimeBinaryMode::Release { executable_dir },
		_ => {}
	}

	find_cargo_workspace_from(&executable_dir).map_or(RuntimeBinaryMode::Release { executable_dir }, |workspace_root| {
		RuntimeBinaryMode::Dev { workspace_root }
	})
}

fn current_build_profile() -> &'static str { option_env!("PROFILE").unwrap_or("debug") }

fn workspace_runtime_binary_path(workspace_root: &Path, target: BuildTarget) -> PathBuf {
	workspace_root
		.join("target")
		.join(target.target_triple())
		.join(current_build_profile())
		.join(target.binary_name())
}

fn resolve_zeroengine_binary(path: &Path, target: BuildTarget) -> ze_core::Result<PathBuf> {
	if is_editor_binary(path) {
		ze_core::bail!(
			"refusing to package editor binary as game runtime: {}; expected {}",
			path.display(),
			target.binary_name()
		);
	}

	if is_zeroengine_binary(path, target) {
		match runtime_binary_mode() {
			RuntimeBinaryMode::Dev { workspace_root } => {
				build_zeroengine_runtime_binary(&workspace_root, target)?;
				let built_runtime = workspace_runtime_binary_path(&workspace_root, target);
				if built_runtime.is_file() {
					return Ok(built_runtime);
				}
				ze_core::bail!(
					"ZeroEngine runtime binary was not produced at {}",
					built_runtime.display()
				);
			}
			RuntimeBinaryMode::Release { executable_dir } => {
				if path.is_file() {
					return Ok(path.to_path_buf());
				}
				ze_core::bail!(
					"ZeroEngine runtime binary not found next to editor at {}; release editor builds do not invoke cargo",
					executable_dir.join(target.binary_name()).display()
				);
			}
		}
	}

	if path.is_file() {
		return Ok(path.to_path_buf());
	}

	ze_core::bail!(
		"ZeroEngine runtime binary not found at {}; build the `ZeroEngine` package first or pass BuildOptions::with_zeroengine_binary",
		path.display()
	);
}

pub fn build_dev_runtime(target: BuildTarget) -> ze_core::Result<()> {
	let current_exe = std::env::current_exe()
		.map_err(|error| io::Error::new(error.kind(), format!("failed to get current executable path: {error}")))?;
	let executable_dir = current_exe
		.parent()
		.ok_or_else(|| io::Error::other("current executable path has no parent directory"))?;
	let workspace_root = find_cargo_workspace_from(executable_dir).ok_or_else(|| {
		io::Error::other(
			"no cargo workspace found from executable path; set ZEROENGINE_EDITOR_RUNTIME_MODE=dev in a workspace checkout",
		)
	})?;
	build_zeroengine_runtime_binary(&workspace_root, target)
}

fn build_zeroengine_runtime_binary(workspace_root: &Path, target: BuildTarget) -> ze_core::Result<()> {
	let triple = target.target_triple();
	ensure_rustup_target(triple)?;

	let profile = current_build_profile();
	let mut cmd = Command::new("cargo");
	cmd.arg("build").arg("-p").arg("ZeroEngine").arg("--target").arg(triple);
	match profile {
		"debug" => {} // default cargo profile, no extra flags
		"release" => {
			cmd.arg("--release");
		}
		_ => {
			cmd.arg("--profile").arg(profile);
		}
	}
	let status = cmd.current_dir(workspace_root).status().map_err(|error| {
		io::Error::new(
			error.kind(),
			format!("failed to run cargo build -p ZeroEngine ({profile}) for {triple}: {error}"),
		)
	})?;

	if !status.success() {
		ze_core::bail!("cargo build -p ZeroEngine ({profile}) for {triple} failed");
	}

	Ok(())
}

fn validate_included_scene_primary_cameras(project: &Project) -> ze_core::Result<()> {
	let missing_scenes = scenes_without_primary_camera(project)?;
	if missing_scenes.is_empty() {
		return Ok(());
	}

	ze_log::warn!(
		"scenes included in build without a primary runtime camera: {}",
		missing_scenes.join(", ")
	);
	Ok(())
}

fn scenes_without_primary_camera(project: &Project) -> ze_core::Result<Vec<String>> {
	let mut scenes_without_primary_camera = Vec::new();

	for scene_name in &project.scenes {
		let scene_path = project_scene_path(project, scene_name);
		if !scene_path.is_file() {
			ze_log::warn!(
				"scene `{scene_name}` is included in build but was not found at {}; skipping primary camera validation",
				scene_path.display()
			);
			continue;
		}

		let mut scene = read_scene_json(&scene_path)?;
		let has_primary_camera = scene_has_primary_runtime_camera(&mut scene).map_err(|error| {
			ze_core::anyhow!(
				"failed to validate primary camera in scene `{scene_name}` at {}: {error}",
				scene_path.display()
			)
		})?;
		if !has_primary_camera {
			scenes_without_primary_camera.push(scene_name.clone());
		}
	}

	Ok(scenes_without_primary_camera)
}

fn project_scene_path(project: &Project, scene_name: &str) -> PathBuf {
	project
		.asset_dir
		.join("scenes")
		.join(format!("{scene_name}.zescene.json"))
}

fn read_scene_json(path: &Path) -> ze_core::Result<serde_json::Value> {
	let json_text = fs::read_to_string(path)?;
	serde_json::from_str(&json_text)
		.map_err(|error| ze_core::anyhow!("failed to parse scene file {}: {error}", path.display()))
}

fn find_cargo_workspace_from(start: &Path) -> Option<PathBuf> {
	for directory in start.ancestors() {
		let manifest = directory.join("Cargo.toml");
		if is_cargo_workspace_manifest(&manifest) {
			return Some(directory.to_path_buf());
		}
	}

	None
}

fn is_cargo_workspace_manifest(path: &Path) -> bool {
	fs::read_to_string(path).is_ok_and(|source| source.lines().any(|line| line.trim() == "[workspace]"))
}

fn is_zeroengine_binary(path: &Path, target: BuildTarget) -> bool {
	path.file_name()
		.and_then(|name| name.to_str())
		.is_some_and(|name| name == target.binary_name())
}

fn is_editor_binary(path: &Path) -> bool {
	path.file_name()
		.and_then(|name| name.to_str())
		.is_some_and(|name| matches!(name, "ZeroEditor" | "ZeroEditor.exe"))
}

pub fn build_scripts_assembly(
	project: &Project,
	configuration: &str,
	api_source_dir: Option<&Path>,
) -> ze_core::Result<()> {
	let project_path = project.generate_csproj_with_api_source(api_source_dir)?;
	ze_log::info!("building C# scripts assembly from {}", project_path.display());

	let status = Command::new("dotnet")
		.arg("build")
		.arg(&project_path)
		.arg("--configuration")
		.arg(configuration)
		.arg("--nologo")
		.status()
		.map_err(|error| {
			io::Error::new(
				error.kind(),
				format!("failed to run dotnet build for {}: {error}", project_path.display()),
			)
		})?;

	if !status.success() {
		ze_core::bail!("dotnet build failed for {}", project_path.display());
	}

	Ok(())
}

fn replace_directory(path: &Path) -> io::Result<()> {
	match fs::remove_dir_all(path) {
		Ok(()) => {}
		Err(error) if error.kind() == io::ErrorKind::NotFound => {}
		Err(error) => return Err(error),
	}

	fs::create_dir_all(path)
}

fn remove_dir_if_exists(path: &Path) -> io::Result<()> {
	match fs::remove_dir_all(path) {
		Ok(()) => Ok(()),
		Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
		Err(error) => Err(error),
	}
}

fn copy_file(source: &Path, target: &Path) -> io::Result<()> {
	if let Some(parent) = target.parent() {
		fs::create_dir_all(parent)?;
	}

	fs::copy(source, target).map(|_| ())
}

fn copy_runtime_binary(source: &Path, target: &Path) -> io::Result<()> {
	if let Some(parent) = target.parent() {
		fs::create_dir_all(parent)?;
	}

	match fs::remove_file(target) {
		Ok(()) => {}
		Err(error) if error.kind() == io::ErrorKind::NotFound => {}
		Err(error) => return Err(error),
	}

	fs::copy(source, target)?;
	let source_metadata = fs::metadata(source)?;
	let target_metadata = fs::metadata(target)?;
	if source_metadata.len() != target_metadata.len() {
		return Err(io::Error::other(format!(
			"copied runtime binary size mismatch: {} has {} bytes, {} has {} bytes",
			source.display(),
			source_metadata.len(),
			target.display(),
			target_metadata.len()
		)));
	}

	Ok(())
}

fn create_staged_assets_without_scripts(
	output_dir: &Path,
	assets_dir: &Path,
	included_scenes: &[String],
) -> io::Result<PathBuf> {
	let staged_assets_dir = output_dir.join(".staged-assets");
	remove_dir_if_exists(&staged_assets_dir)?;
	fs::create_dir_all(&staged_assets_dir)?;
	copy_directory_excluding_names(
		assets_dir,
		&staged_assets_dir,
		&["scripts", ".compiled"],
		included_scenes,
	)?;
	Ok(staged_assets_dir)
}

fn copy_directory_excluding_names(
	source: &Path,
	target: &Path,
	excluded_names: &[&str],
	included_scenes: &[String],
) -> io::Result<()> {
	for entry in fs::read_dir(source)? {
		let entry = entry?;
		let source_path = entry.path();
		let name = entry.file_name();

		if excluded_names
			.iter()
			.any(|excluded| name == std::ffi::OsStr::new(excluded))
		{
			continue;
		}

		let target_path = target.join(&name);
		if source_path.is_dir() {
			fs::create_dir_all(&target_path)?;
			copy_directory_excluding_names(&source_path, &target_path, excluded_names, included_scenes)?;
		} else {
			copy_asset_file(&source_path, &target_path, included_scenes)?;
		}
	}

	Ok(())
}

fn copy_asset_file(source: &Path, target: &Path, included_scenes: &[String]) -> io::Result<()> {
	if is_scene_file(source) {
		if !scene_file_is_included(source, included_scenes) {
			return Ok(());
		}
		copy_filtered_scene_file(source, target)
	} else {
		copy_file(source, target)
	}
}

fn is_scene_file(path: &Path) -> bool {
	path.file_name()
		.and_then(|name| name.to_str())
		.is_some_and(|name| name.ends_with(".zescene.json"))
}

fn scene_file_is_included(path: &Path, included_scenes: &[String]) -> bool {
	let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
		return false;
	};
	let Some(scene_name) = file_name.strip_suffix(".zescene.json") else {
		return false;
	};

	included_scenes.iter().any(|included| included == scene_name)
}

fn copy_filtered_scene_file(source: &Path, target: &Path) -> io::Result<()> {
	let json_text = fs::read_to_string(source)?;
	let mut json_value: serde_json::Value = serde_json::from_str(&json_text).map_err(|error| {
		ze_log::warn!("failed to parse scene file {}: {error}", source.display());
		io::Error::new(
			io::ErrorKind::InvalidData,
			format!("failed to parse scene file {}: {error}", source.display()),
		)
	})?;

	filter_editor_only_entities(&mut json_value).map_err(|error| {
		ze_log::warn!(
			"failed to filter editor-only entities from {}: {error}",
			source.display()
		);
		io::Error::new(
			io::ErrorKind::InvalidData,
			format!(
				"failed to filter editor-only entities from {}: {error}",
				source.display()
			),
		)
	})?;
	ensure_primary_runtime_camera(&mut json_value).map_err(|error| {
		ze_log::warn!("failed to activate a runtime camera in {}: {error}", source.display());
		io::Error::new(
			io::ErrorKind::InvalidData,
			format!("failed to activate a runtime camera in {}: {error}", source.display()),
		)
	})?;

	write_json_file(target, &json_value)
}

fn filter_editor_only_entities(json_value: &mut serde_json::Value) -> Result<(), String> {
	let Some(entities) = json_value.get_mut("entities").and_then(serde_json::Value::as_array_mut) else {
		return Err("scene file is missing an entities array".to_string());
	};

	entities.retain(|entity| !entity_is_editor_only(entity));
	Ok(())
}

fn ensure_primary_runtime_camera(json_value: &mut serde_json::Value) -> Result<(), String> {
	let Some(entities) = json_value.get_mut("entities").and_then(serde_json::Value::as_array_mut) else {
		return Err("scene file is missing an entities array".to_string());
	};

	if entities.iter().any(entity_has_primary_camera) {
		return Ok(());
	}

	for entity in entities {
		if let Some(camera) = camera_component_mut(entity) {
			camera["value"]["primary"] = serde_json::Value::Bool(true);
			break;
		}
	}

	Ok(())
}

fn scene_has_primary_runtime_camera(json_value: &mut serde_json::Value) -> Result<bool, String> {
	filter_editor_only_entities(json_value)?;
	ensure_primary_runtime_camera(json_value)?;
	let Some(entities) = json_value.get("entities").and_then(serde_json::Value::as_array) else {
		return Err("scene file is missing an entities array".to_string());
	};

	Ok(entities.iter().any(entity_has_primary_camera))
}

fn entity_is_editor_only(entity: &serde_json::Value) -> bool {
	let Some(components) = entity.get("components").and_then(serde_json::Value::as_array) else {
		return false;
	};

	components.iter().any(|component| {
		component
			.get("component_type")
			.and_then(serde_json::Value::as_str)
			.is_some_and(|component_type| component_type == EDITOR_ONLY_COMPONENT_TYPE)
	})
}

fn entity_has_primary_camera(entity: &serde_json::Value) -> bool {
	entity
		.get("components")
		.and_then(serde_json::Value::as_array)
		.is_some_and(|components| {
			components.iter().any(|component| {
				component
					.get("component_type")
					.and_then(serde_json::Value::as_str)
					.is_some_and(|component_type| component_type == CAMERA_COMPONENT_TYPE)
					&& component
						.get("value")
						.and_then(|value| value.get("primary"))
						.and_then(serde_json::Value::as_bool)
						.unwrap_or(false)
			})
		})
}

fn camera_component_mut(entity: &mut serde_json::Value) -> Option<&mut serde_json::Value> {
	entity
		.get_mut("components")?
		.as_array_mut()?
		.iter_mut()
		.find(|component| {
			component
				.get("component_type")
				.and_then(serde_json::Value::as_str)
				.is_some_and(|component_type| component_type == CAMERA_COMPONENT_TYPE)
		})
}

fn write_json_file(path: &Path, json_value: &serde_json::Value) -> io::Result<()> {
	let json = serde_json::to_string_pretty(json_value).map_err(|error| {
		io::Error::new(
			io::ErrorKind::InvalidData,
			format!("failed to serialize filtered scene {}: {error}", path.display()),
		)
	})?;

	copy_text_file(path, &json)
}

fn copy_text_file(target: &Path, contents: &str) -> io::Result<()> {
	if let Some(parent) = target.parent() {
		fs::create_dir_all(parent)?;
	}

	fs::write(target, contents)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn filters_editor_only_entities_from_scene_value() {
		let mut scene = serde_json::json!({
			"version": "0.1.0",
			"entities": [
				{
					"id": { "index": 0, "gen": 0 },
					"components": [
						{ "component_type": "ze.editor.only", "value": null },
						{ "component_type": "ze.transform", "value": {} }
					]
				},
				{
					"id": { "index": 1, "gen": 0 },
					"components": [
						{ "component_type": "ze.transform", "value": {} }
					]
				}
			]
		});

		assert!(filter_editor_only_entities(&mut scene).is_ok());

		assert!(scene.get("entities").and_then(serde_json::Value::as_array).is_some());
		let Some(entities) = scene.get("entities").and_then(serde_json::Value::as_array) else {
			return;
		};

		assert_eq!(entities.len(), 1);
		assert_eq!(
			entities[0]["id"],
			serde_json::json!({
				"index": 1,
				"gen": 0
			})
		);
	}

	#[test]
	fn scene_file_inclusion_uses_project_scene_names() {
		let included_scenes = vec!["main".to_string(), "Credits".to_string()];

		assert!(scene_file_is_included(
			Path::new("assets/scenes/main.zescene.json"),
			&included_scenes
		));
		assert!(scene_file_is_included(
			Path::new("assets/scenes/Credits.zescene.json"),
			&included_scenes
		));
		assert!(!scene_file_is_included(
			Path::new("assets/scenes/Menu.zescene.json"),
			&included_scenes
		));
	}

	#[test]
	fn filtered_scene_promotes_remaining_camera_when_editor_camera_was_primary() {
		let mut scene = serde_json::json!({
			"entities": [
				{
					"components": [
						{ "component_type": "ze.editor.only", "value": null },
						{ "component_type": "ze.renderer.camera", "value": { "primary": true } }
					]
				},
				{
					"components": [
						{ "component_type": "ze.renderer.camera", "value": { "primary": false } }
					]
				}
			]
		});

		filter_editor_only_entities(&mut scene).expect("editor-only filtering should succeed");
		ensure_primary_runtime_camera(&mut scene).expect("runtime camera activation should succeed");

		let entities = scene["entities"].as_array().expect("entities should remain an array");
		assert_eq!(entities.len(), 1);
		assert!(entity_has_primary_camera(&entities[0]));
	}

	#[test]
	fn filtered_scene_preserves_existing_runtime_primary_camera() {
		let mut scene = serde_json::json!({
			"entities": [
				{
					"components": [
						{ "component_type": "ze.renderer.camera", "value": { "primary": true } }
					]
				},
				{
					"components": [
						{ "component_type": "ze.renderer.camera", "value": { "primary": false } }
					]
				}
			]
		});

		ensure_primary_runtime_camera(&mut scene).expect("runtime camera activation should succeed");

		let entities = scene["entities"].as_array().expect("entities should remain an array");
		assert!(entity_has_primary_camera(&entities[0]));
		assert!(!entity_has_primary_camera(&entities[1]));
	}

	#[test]
	fn primary_camera_validation_accepts_promotable_runtime_camera() {
		let mut scene = serde_json::json!({
			"entities": [
				{
					"components": [
						{ "component_type": "ze.editor.only", "value": null },
						{ "component_type": "ze.renderer.camera", "value": { "primary": true } }
					]
				},
				{
					"components": [
						{ "component_type": "ze.renderer.camera", "value": { "primary": false } }
					]
				}
			]
		});

		let has_primary_camera =
			scene_has_primary_runtime_camera(&mut scene).expect("primary camera validation should succeed");

		assert!(has_primary_camera);
	}

	#[test]
	fn primary_camera_validation_ignores_editor_only_camera_without_runtime_camera() {
		let mut scene = serde_json::json!({
			"entities": [
				{
					"components": [
						{ "component_type": "ze.editor.only", "value": null },
						{ "component_type": "ze.renderer.camera", "value": { "primary": true } }
					]
				}
			]
		});

		let has_primary_camera =
			scene_has_primary_runtime_camera(&mut scene).expect("primary camera validation should succeed");

		assert!(!has_primary_camera);
	}

	#[test]
	fn primary_camera_validation_detects_runtime_primary_camera() {
		let mut scene = serde_json::json!({
			"entities": [
				{
					"components": [
						{ "component_type": "ze.renderer.camera", "value": { "primary": true } }
					]
				}
			]
		});

		let has_primary_camera =
			scene_has_primary_runtime_camera(&mut scene).expect("primary camera validation should succeed");

		assert!(has_primary_camera);
	}

	#[test]
	fn primary_camera_validation_detects_camera_on_mixed_component_entity() {
		let mut scene = serde_json::json!({
			"entities": [
				{
					"components": [
						{ "component_type": "ze.name", "value": { "name": "Ball" } },
						{ "component_type": "ze.physics_2d.rigidbody", "value": { "body_type": "Dynamic" } },
						{ "component_type": "ze.physics_2d.collider", "value": { "shape": { "Circle": { "radius": 0.5 } } } },
						{ "component_type": "ze.scripting.script", "value": { "path": "Sandbox.Script", "enabled": true, "fields": {} } },
						{ "component_type": "ze.renderer.camera", "value": { "primary": true } },
						{ "component_type": "ze.transform", "value": {} }
					]
				}
			]
		});

		let has_primary_camera = scene_has_primary_runtime_camera(&mut scene)
			.expect("primary camera validation should accept mixed entities");

		assert!(has_primary_camera);
	}

	#[test]
	fn runtime_binary_copy_replaces_existing_output_file() -> io::Result<()> {
		let test_dir = std::env::temp_dir().join(format!("ze-build-runtime-copy-{}", std::process::id()));
		if test_dir.exists() {
			fs::remove_dir_all(&test_dir)?;
		}
		fs::create_dir_all(&test_dir)?;

		let host_target = BuildTarget::host();
		let source = test_dir
			.join("target")
			.join(host_target.target_triple())
			.join(current_build_profile())
			.join(host_target.binary_name());
		let target = test_dir.join("dist").join(host_target.binary_name());
		fs::create_dir_all(source.parent().expect("source should have parent"))?;
		fs::create_dir_all(target.parent().expect("target should have parent"))?;
		fs::write(&source, b"fresh runtime binary")?;
		fs::write(&target, b"stale")?;

		copy_runtime_binary(&source, &target)?;

		assert_eq!(fs::read(&target)?, b"fresh runtime binary");
		fs::remove_dir_all(test_dir)?;
		Ok(())
	}
}
