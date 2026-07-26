use std::{
	collections::HashMap,
	sync::{Arc, atomic::Ordering},
	time::Instant,
};

use rfd::{MessageButtons, MessageDialog, MessageLevel};
use winit::{
	application::ApplicationHandler,
	event::{MouseScrollDelta, WindowEvent},
	event_loop::ActiveEventLoop,
	keyboard::PhysicalKey,
	window::{Window, WindowId},
};
use ze_animation::AnimationSystem;
use ze_assets::ResourceManager;
use ze_audio::AudioSystem;
use ze_core::{Result, Vec2, bail};
use ze_ecs::{EditorOnly, EntitiesView, PhysicsSettings, Scene, System, UiReferenceResolution, ViewportInfo, registry};
use ze_input::{Input, ZKeyCode, ZMouseCode};
use ze_physics::PhysicsSystem;
use ze_project::Project;
use ze_renderer::{
	CameraViewSystem, EditorCameraSystem, LateCameraViewSystem, RenderStatus, RenderSystem,
	register_renderer_components,
};
use ze_scripting_cs::{
	CursorApiCommand, SHUTDOWN_REQUESTED, ScriptingCursorGrabMode, ScriptingSceneLoadCommand, ScriptingSystem,
	drain_cursor_commands, drain_scripting_scene_load_commands, register_scripting_components,
};
use ze_ui::{UISystem, register_ui_components, ui_manager::UiManagerHandle};

const DEFAULT_SCENE_NAME: &str = "main";

#[derive(Debug)]
pub enum CustomEvents {
	Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingPrimaryCameraDialogState {
	Hidden,
	Shown,
}

pub struct App {
	runtime: tokio::runtime::Runtime,
	pub active_project: Option<Project>,
	pub window: Option<Arc<Window>>,
	scenes: HashMap<String, Scene>,
	active_scene: String,
	ui_manager: Option<UiManagerHandle>,
	renderer: Option<ze_renderer::Renderer>,
	focused: bool,
	occluded: bool,
	minimized: bool,
	last_frame_time: Instant,
	resources: ResourceManager,
	missing_primary_camera_dialog: MissingPrimaryCameraDialogState,
}
impl Default for App {
	fn default() -> Self { Self::new(None).expect("failed to initialize app") }
}

impl App {
	pub fn new(active_project: Option<Project>) -> Result<Self> {
		ze_log::debug!("[STARTUP 2a] Entered App::new()");

		let resources = active_project.as_ref().map_or_else(
			|| ResourceManager::for_runtime("Sandbox/assets"),
			|project| ResourceManager::new(project.asset_dir.clone()),
		);
		ze_log::debug!("[STARTUP 2b] Loading main scene...");
		let scene = load_main_scene(&resources, active_project.as_ref(), None)?;
		ze_log::debug!("[STARTUP 2c] Main scene loaded");

		if let Err(error) = resources.compile_game_shaders() {
			ze_log::error!("Failed to compile game shaders: {error:?}");
		}
		ze_log::debug!("[STARTUP 2d] Game shaders compiled");

		let active_scene = scene.name.clone();
		let mut scenes = HashMap::new();
		scenes.insert(scene.name.clone(), scene);

		let runtime = match tokio::runtime::Runtime::new() {
			Ok(r) => r,
			Err(e) => {
				ze_log::error!("Cannot create tokio runtime! error: {}", e);
				std::process::exit(1);
			}
		};

		Ok(Self {
			runtime,
			active_project,
			window: None,
			renderer: None,
			ui_manager: None,
			focused: true,
			occluded: false,
			minimized: false,
			scenes,
			active_scene,
			last_frame_time: Instant::now(),
			resources,
			missing_primary_camera_dialog: MissingPrimaryCameraDialogState::Hidden,
		})
	}

	pub fn add_scene(&mut self, scene: Scene) -> Result<()> {
		let name = scene.name.clone();
		if self.scenes.contains_key(&name) {
			bail!("scene `{name}` already exists");
		}

		self.scenes.insert(name, scene);
		Ok(())
	}

	pub fn set_active_scene(&mut self, name: impl Into<String>) -> Result<()> {
		let name = name.into();
		if !self.scenes.contains_key(&name) {
			bail!("scene `{name}` does not exist");
		}

		self.active_scene = name;
		Ok(())
	}

	pub fn active_scene(&self) -> Option<&Scene> { self.scenes.get(&self.active_scene) }

	pub fn active_scene_mut(&mut self) -> Option<&mut Scene> { self.scenes.get_mut(&self.active_scene) }

	pub fn add_system<S>(&mut self, system: S)
	where
		S: System + 'static,
	{
		let Some(scene) = self.active_scene_mut() else {
			return;
		};

		scene.add_system(system);
	}

	pub fn update_active_scene_systems(&mut self, dt: f32) -> Result<()> {
		// `CameraViewSystem` (runs inside `scene.update_systems`, between
		// `PhysicsSystem` and `UISystem`) needs the viewport size to rebuild
		// `ActiveCameraView`, but only `App` owns the `Renderer`. Stash it as a
		// unique each tick rather than threading a `&Renderer` through the
		// `System` trait.
		let viewport = self.renderer.as_ref().map(|renderer| {
			let size = renderer.viewport_size();
			Vec2::new(size.width as f32, size.height as f32)
		});

		// Reference/design resolution the ScreenSpaceOverlay UI was authored
		// against, so `UISystem` can scale the canvas by viewport_width /
		// reference_width. Sourced from project settings; falls back to the
		// 1920x1080 default when no project is loaded.
		let reference_resolution =
			self.active_project
				.as_ref()
				.map_or_else(UiReferenceResolution::default, |project| UiReferenceResolution {
					size: Vec2::new(project.ui.reference_width, project.ui.reference_height),
				});

		let Some(scene) = self.active_scene_mut() else {
			return Ok(());
		};

		if let Some(size) = viewport {
			scene.world().add_unique(ViewportInfo { size });
		}
		scene.world().add_unique(reference_resolution);

		scene.with_system_mut::<PhysicsSystem, _>(|physics, scene| {
			physics.remove_inactive_entities(scene);
		});
		scene.update_systems(dt)?;
		self.apply_scripting_scene_load_commands();
		Ok(())
	}

	fn apply_scripting_scene_load_commands(&mut self) {
		for command in drain_scripting_scene_load_commands() {
			if let Err(error) = self.apply_scripting_scene_load_command(command) {
				ze_log::error!("failed to apply script scene command: {error:?}");
			}
		}
	}

	fn apply_scripting_scene_load_command(&mut self, command: ScriptingSceneLoadCommand) -> Result<()> {
		match command {
			ScriptingSceneLoadCommand::Load { name } => self.load_and_activate_scene(&name),
			ScriptingSceneLoadCommand::LoadMain => self.load_main_scene(),
			ScriptingSceneLoadCommand::Reload => self.reload_active_scene(),
		}
	}

	fn load_main_scene(&mut self) -> Result<()> {
		let scene_name = resolve_main_scene_name(&self.resources, self.active_project.as_ref());
		self.load_and_activate_scene(&scene_name)
	}

	fn load_and_activate_scene(&mut self, scene_name: &str) -> Result<()> {
		let scene = load_project_scene(
			&self.resources,
			self.active_project.as_ref(),
			scene_name,
			self.ui_manager.clone(),
		)?;
		let loaded_name = scene.name.clone();
		self.scenes.insert(loaded_name.clone(), scene);
		self.active_scene = loaded_name;
		self.missing_primary_camera_dialog = MissingPrimaryCameraDialogState::Hidden;
		Ok(())
	}

	fn reload_active_scene(&mut self) -> Result<()> {
		let scene_name = self.active_scene.clone();
		self.load_and_activate_scene(&scene_name)
	}
}

pub fn load_main_scene(
	resources: &ResourceManager,
	active_project: Option<&Project>,
	ui_manager: Option<UiManagerHandle>,
) -> Result<Scene> {
	let scene_name = resolve_main_scene_name(resources, active_project);
	load_project_scene(resources, active_project, &scene_name, ui_manager)
}

/// Picks the scene to boot into. When a `Project` is loaded (editor, or a
/// standalone launch given an explicit project path) its `main_scene` is
/// authoritative. Otherwise -- the normal path for a shipped/packaged
/// standalone build, which runs with no `Project` at all -- fall back to the
/// `main_scene` baked into `GameData.toml` at build time (see
/// `ze_build::build_project_dist`), rather than a hardcoded scene name.
fn resolve_main_scene_name(resources: &ResourceManager, active_project: Option<&Project>) -> String {
	if let Some(project) = active_project {
		return project.main_scene.clone();
	}

	let game_data_path = resources.game_assets_root().join("GameData.toml");
	ze_project::GameData::load(&game_data_path)
		.map(|game_data| game_data.main_scene)
		.unwrap_or_else(|_| DEFAULT_SCENE_NAME.to_string())
}

pub fn load_project_scene(
	resources: &ResourceManager,
	active_project: Option<&Project>,
	scene_name: &str,
	ui_manager: Option<UiManagerHandle>,
) -> Result<Scene> {
	validate_scene_name(scene_name)?;
	let mut registry = registry::ComponentRegistry::new();

	// `AudioSource` (unlike UI/scripting components) is registered inside
	// `Scene::register_defaults` itself, alongside
	// `RigidBody`/`Collider`/`PhysicsSettings` -- see ze_ecs/src/components.rs for
	// why.
	Scene::register_defaults(&mut registry);
	register_renderer_components(&mut registry);
	register_scripting_components(&mut registry);
	register_ui_components(&mut registry);

	let scene_path = resources
		.game_assets_root()
		.join(format!("scenes/{scene_name}.zescene.json"));
	let mut scene = if scene_path.exists() {
		Scene::from_path_with_registry(&scene_path, registry).map_err(|error| {
			ze_core::anyhow!(
				"failed to load scene `{scene_name}` from {}: {error:?}",
				scene_path.display()
			)
		})?
	} else {
		ze_log::warn!(
			"scene `{scene_name}` not found at {}, starting with an empty scene",
			scene_path.display()
		);
		Scene::from_registry(scene_name, registry)
	};
	let _entity_count = scene
		.world()
		.run(|entities: ze_ecs::EntitiesView| entities.iter().count());

	apply_project_physics_defaults(&mut scene, active_project);
	scene.add_system(EditorCameraSystem::new());
	// CameraViewSystem must run before ScriptingSystem so that C# scripts
	// calling GetMouseWorldPosition() read a fresh ActiveCameraView instead
	// of a stale / missing one.  Running before PhysicsSystem means the
	// view-projection is based on the previous frame's camera transform,
	// which is acceptable for cursor picking. UISystem needs a fresher view
	// than that (see LateCameraViewSystem below), so this pass alone is not
	// sufficient for World Space UI projection.
	scene.add_system(CameraViewSystem::new());
	let (scripts_path, runtime_config_path) = scripting_paths(active_project, resources);
	ze_log::debug!(
		"[STARTUP 2e] Initializing scripting system (assembly: {}, runtimeconfig: {})",
		scripts_path.display(),
		runtime_config_path.display()
	);
	let scripting_system = ScriptingSystem::from_paths(scripts_path, runtime_config_path);
	ze_log::debug!("[STARTUP 2f] Scripting system initialized");
	let scripting_runtime = scripting_system.runtime();
	scene.add_system(scripting_system);
	scene.add_system(PhysicsSystem::with_scripting(scripting_runtime));
	scene.add_system(AnimationSystem::new(resources.clone()));
	// Re-refresh ActiveCameraView after simulation has moved things this
	// tick, so UISystem's WorldSpace projection (below) isn't one tick behind
	// -- see LateCameraViewSystem's doc comment for why that lag matters.
	scene.add_system(LateCameraViewSystem::new());
	if let Some(handle) = ui_manager {
		scene.add_system(UISystem::new(handle, resources.clone()));
	}
	scene.add_system(RenderSystem::new());
	// Non-fatal on failure (e.g. no audio device in CI/headless) -- audio is a
	// jam nice-to-have, not something the rest of the engine should depend on.
	match AudioSystem::new(resources.clone()) {
		Ok(audio) => scene.add_system(audio),
		Err(error) => ze_log::error!("failed to initialize audio system: {error:?}"),
	}
	Ok(scene)
}

fn apply_project_physics_defaults(scene: &mut Scene, active_project: Option<&Project>) {
	if physics_settings_entity(scene).is_some() {
		return;
	}

	let Some(project) = active_project else {
		return;
	};

	let entity = scene.create_entity("PhysicsSettings");
	scene
		.entity_mut(entity)
		.add_component(EditorOnly)
		.add_component(project_physics_settings(&project.physics));
}

const fn project_physics_settings(settings: &ze_project::PhysicsSettings) -> PhysicsSettings {
	PhysicsSettings {
		gravity: Vec2::new(settings.gravity.x, settings.gravity.y),
		enable_debug_draw: settings.enable_debug_draw,
		physics_timestep: settings.timestep,
	}
}

fn physics_settings_entity(scene: &Scene) -> Option<ze_ecs::EntityId> {
	let world = scene.world();
	let mut found = None;
	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&PhysicsSettings>(entity).is_ok() {
				found = Some(entity);
				break;
			}
		}
	});
	found
}

fn scripting_paths(
	active_project: Option<&Project>,
	resources: &ResourceManager,
) -> (std::path::PathBuf, std::path::PathBuf) {
	if let Some(project) = active_project {
		return (project.scripts_dll.clone(), project.runtime_config_path());
	}

	if let Some(paths) = find_dist_scripts(resources) {
		return paths;
	}

	let scripts_dir = resources.game_assets_root().join("scripts").join("bin");
	if let Some((scripts_dll, runtime_config)) = find_scripts_in_dir(&scripts_dir) {
		return (scripts_dll, runtime_config);
	}

	(
		scripts_dir.join("Scripts.dll"),
		scripts_dir.join("Scripts.runtimeconfig.json"),
	)
}

fn find_dist_scripts(resources: &ResourceManager) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
	let exe_dir = std::env::current_exe()
		.ok()
		.and_then(|path| path.parent().map(std::path::Path::to_path_buf))?;

	// New layout: assembly is in Game/ subdirectory.
	let game_dir = exe_dir.join("Game");
	let assembly_path = find_only_dll(&game_dir)?;

	// Try to stage runtimeconfig.json and deps.json from the materialized
	// zepack contents. At build time these files are packed into
	// assets.zepack and extracted to a temp directory at startup.
	if let Some(staged) = stage_dotnet_configs_from_pack(&assembly_path, resources) {
		return Some(staged);
	}

	// Legacy layout: look for config next to the assembly directly.
	let runtime_config = assembly_path.with_extension("runtimeconfig.json");
	if runtime_config.exists() {
		return Some((assembly_path, runtime_config));
	}

	// Also search exe_dir root for legacy layouts.
	if let Some((legacy_assembly, legacy_config)) = find_assembly_with_config(&exe_dir) {
		return Some((legacy_assembly, legacy_config));
	}

	None
}

fn stage_dotnet_configs_from_pack(
	assembly_path: &std::path::Path,
	resources: &ResourceManager,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
	// Check if the materialized zepack contains dotnet-config files.
	let pack_root = resources.game_assets_root();
	let stem = assembly_path.file_stem()?.to_str()?;
	let config_filename = format!("{stem}.runtimeconfig.json");
	let config_source = pack_root.join("dotnet-config").join(&config_filename);
	if !config_source.exists() {
		return None;
	}

	let nanos = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |d| d.as_nanos());
	let staging_dir = std::env::temp_dir().join(format!("zeroengine-dotnet-{}-{nanos}", std::process::id()));
	std::fs::create_dir_all(&staging_dir).ok()?;

	let assembly_name = assembly_path.file_name()?;
	let staged_assembly = staging_dir.join(assembly_name);
	std::fs::copy(assembly_path, &staged_assembly).ok()?;

	let staged_config = staging_dir.join(&config_filename);
	std::fs::copy(&config_source, &staged_config).ok()?;

	let deps_filename = format!("{stem}.deps.json");
	let deps_source = pack_root.join("dotnet-config").join(&deps_filename);
	if deps_source.exists() {
		let _ = std::fs::copy(&deps_source, staging_dir.join(&deps_filename));
	}

	Some((staged_assembly, staged_config))
}

fn find_assembly_with_config(dir: &std::path::Path) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
	let entries = std::fs::read_dir(dir).ok()?;
	for entry in entries.filter_map(std::result::Result::ok) {
		let path = entry.path();
		if path.extension().and_then(|ext| ext.to_str()) != Some("dll") {
			continue;
		}
		let runtime_config = path.with_extension("runtimeconfig.json");
		if runtime_config.exists() {
			return Some((path, runtime_config));
		}
	}
	None
}

fn find_only_dll(dir: &std::path::Path) -> Option<std::path::PathBuf> {
	let entries = std::fs::read_dir(dir).ok()?;
	for entry in entries.filter_map(std::result::Result::ok) {
		let path = entry.path();
		if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("dll") {
			return Some(path);
		}
	}
	None
}

fn find_scripts_in_dir(dir: &std::path::Path) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
	find_assembly_with_config(dir)
}

fn validate_scene_name(scene_name: &str) -> Result<()> {
	if scene_name.trim().is_empty() {
		bail!("scene name cannot be empty");
	}
	if scene_name != scene_name.trim() {
		bail!("scene name `{scene_name}` must not have leading or trailing whitespace");
	}
	if scene_name == "." || scene_name == ".." || scene_name.contains('/') || scene_name.contains('\\') {
		bail!("scene name `{scene_name}` must be a file stem, not a path");
	}
	if scene_name.ends_with(".zescene.json") {
		bail!("scene name `{scene_name}` must not include the scene file extension");
	}

	Ok(())
}

fn show_missing_primary_camera_dialog(scene_name: &str) {
	let name = if scene_name.is_empty() { "unnamed" } else { scene_name };
	ze_log::error!("Scene `{name}` has no usable camera; exiting");
	let _ = MessageDialog::new()
		.set_level(MessageLevel::Error)
		.set_title("ZeroEngine")
		.set_description(format!(
			"Scene `{scene_name}` has no usable camera. Add a Camera component to the scene -- it doesn't need to be \
			 marked Primary, the engine will use it automatically unless another camera is explicitly marked Primary."
		))
		.set_buttons(MessageButtons::Ok)
		.show();
}

/// Drains cursor visibility/grab requests queued by C# scripts and applies them
/// to the game window. `CursorGrabMode::Locked` falls back to `Confined` where
/// the platform can't lock (e.g. some X11 setups), matching the startup grab.
fn apply_cursor_commands(window: &Window) {
	for command in drain_cursor_commands() {
		match command {
			CursorApiCommand::SetVisible(visible) => {
				ze_log::debug!("[cursor] set_cursor_visible({visible})");
				window.set_cursor_visible(visible);
			}
			CursorApiCommand::SetGrabMode(mode) => {
				ze_log::debug!("[cursor] applying grab mode {mode:?}");
				let result = match mode {
					ScriptingCursorGrabMode::None => window.set_cursor_grab(winit::window::CursorGrabMode::None),
					ScriptingCursorGrabMode::Confined => {
						window.set_cursor_grab(winit::window::CursorGrabMode::Confined)
					}
					ScriptingCursorGrabMode::Locked => window
						.set_cursor_grab(winit::window::CursorGrabMode::Locked)
						.or_else(|_| window.set_cursor_grab(winit::window::CursorGrabMode::Confined)),
				};
				if let Err(error) = result {
					ze_log::warn!("[cursor] set_cursor_grab({mode:?}) failed: {error:?}");
				}
			}
		}
	}
}

fn exit_for_missing_primary_camera(
	event_loop: &ActiveEventLoop,
	dialog_state: &mut MissingPrimaryCameraDialogState,
	scene_name: &str,
) {
	if *dialog_state == MissingPrimaryCameraDialogState::Hidden {
		*dialog_state = MissingPrimaryCameraDialogState::Shown;
		show_missing_primary_camera_dialog(scene_name);
	}
	event_loop.exit();
}

impl ApplicationHandler<CustomEvents> for App {
	fn resumed(&mut self, event_loop: &ActiveEventLoop) {
		ze_log::debug!("[STARTUP 3a] Creating window...");

		let attrs = Window::default_attributes()
			.with_title("ZeroEngine")
			.with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));

		// TODO: add asset manager and icon
		let window = match event_loop.create_window(attrs) {
			Ok(window) => Arc::new(window),
			Err(error) => {
				ze_log::error!("[STARTUP 3a] Failed to create window: {error}");
				event_loop.exit();
				return;
			}
		};
		ze_log::debug!("[STARTUP 3b] Window created");

		// TODO: add cursor grab parameters in game settings
		if let Err(error) = window
			.set_cursor_grab(winit::window::CursorGrabMode::Locked)
			.or_else(|_| window.set_cursor_grab(winit::window::CursorGrabMode::Confined))
		{
			ze_log::warn!("[cursor] startup set_cursor_grab failed: {error:?}");
		}

		window.set_cursor_visible(false);

		ze_log::debug!("[STARTUP 3c] Initializing renderer...");
		let renderer = self.runtime.block_on(ze_renderer::Renderer::new(window.clone()));
		match renderer {
			Ok(mut renderer) => {
				ze_log::debug!("[STARTUP 3d] Renderer initialized");
				let ui_manager =
					UiManagerHandle::new(ze_ui::UiManager::new(renderer.device(), renderer.queue(), &window));
				renderer.set_ui_manager(ui_manager.clone());
				self.ui_manager = Some(ui_manager.clone());
				self.renderer = Some(renderer);

				// The initial scene was loaded in `App::new()`, before the window (and
				// therefore the UI manager) existed, so `load_project_scene` was called
				// with `ui_manager: None` and never added a `UISystem` -- scenes loaded
				// after this point via scripting scene-load commands already pass
				// `self.ui_manager`, so only the startup scene needs patching up here.
				let resources = self.resources.clone();
				if let Some(scene) = self.active_scene_mut()
					&& scene.with_system_mut::<UISystem, _>(|_, _| ()).is_none()
				{
					scene.add_system(UISystem::new(ui_manager, resources));
				}
			}
			Err(error) => {
				ze_log::error!("[STARTUP 3c] Failed to create renderer: {error:?}");
				event_loop.exit();
				return;
			}
		}

		self.window = Some(window);

		// Ensure ViewportInfo is available immediately so that any systems
		// running before the first `about_to_wait()` tick (e.g. a Resized
		// event delivered during the same loop iteration) see a valid
		// viewport size.
		if let Some(renderer) = &self.renderer {
			let size = renderer.viewport_size();
			if let Some(scene) = self.active_scene_mut() {
				scene.world().add_unique(ViewportInfo {
					size: Vec2::new(size.width as f32, size.height as f32),
				});
			}
		}

		self.last_frame_time = Instant::now();
		ze_log::debug!("[STARTUP 3e] First frame setup complete — entering main loop");
	}
	fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
		ze_log::trace!("App update");

		event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

		let now = Instant::now();
		let dt = now.duration_since(self.last_frame_time).as_secs_f32().min(0.05);
		self.last_frame_time = now;

		let window = self.window.clone();
		if let Some(window) = window
			&& !self.occluded
			&& !self.minimized
		{
			if let Err(error) = self.update_active_scene_systems(dt) {
				ze_log::error!("System update failed: {error:?}");
			}
			apply_cursor_commands(&window);
			window.request_redraw();
		}
		if SHUTDOWN_REQUESTED.swap(false, Ordering::SeqCst) {
			event_loop.exit();
			return;
		}
		Input::update_globally(Input::late_update);
	}

	fn user_event(&mut self, event_loop: &ActiveEventLoop, event: CustomEvents) {
		match event {
			CustomEvents::Shutdown => {
				event_loop.exit();
			}
		}
	}

	fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
		ze_log::trace!("window event");

		// Forward events to yakui-winit for UI input tracking
		if let Some(ref handle) = self.ui_manager {
			handle.borrow_mut().handle_window_event(&event);
		}

		match event {
			// === WINDOW STATE =============
			WindowEvent::Resized(size) => {
				self.minimized = size.width == 0 || size.height == 0;

				if !self.minimized {
					if let Some(renderer) = &mut self.renderer {
						renderer.resize(size);
					}
					// Keep ViewportInfo in sync so that CameraViewSystem and
					// any C# scripts calling GetMouseWorldPosition see the
					// updated viewport size before the next about_to_wait()
					// tick.
					if let Some(renderer) = &self.renderer {
						let vs = renderer.viewport_size();
						if let Some(scene) = self.active_scene_mut() {
							scene.world().add_unique(ViewportInfo {
								size: Vec2::new(vs.width as f32, vs.height as f32),
							});
						}
					}
				}
			}
			WindowEvent::Focused(focused) => {
				self.focused = focused;

				if !focused {
					Input::update_globally(Input::reset);
				}
			}
			WindowEvent::Occluded(occluded) => {
				self.occluded = occluded;
				// Occlusion (e.g. minimized, covered by another window) can happen
				// without a matching Focused(false) on some window managers, and is
				// another point where key-release events can silently get lost.
				if occluded {
					Input::update_globally(Input::reset);
				}
			}
			WindowEvent::CloseRequested => {
				ze_log::debug!("Exiting...");
				event_loop.exit();
			}
			// === INPUT ==============
			WindowEvent::KeyboardInput { event: key_event, .. } => {
				if let PhysicalKey::Code(code) = key_event.physical_key {
					Input::update_globally(|i| {
						i.set_key(ZKeyCode::from(code), key_event.state.is_pressed());
					});
				}
			}
			WindowEvent::MouseInput { state, button, .. } => {
				Input::update_globally(|i| {
					i.set_mouse_button(ZMouseCode::from(button), state.is_pressed());
				});
			}
			WindowEvent::MouseWheel { delta, .. } => {
				let wheel_delta = match delta {
					MouseScrollDelta::LineDelta(_, y) => y,
					MouseScrollDelta::PixelDelta(position) => position.y as f32 / 120.0,
				};
				Input::update_globally(|i| {
					i.add_mouse_wheel_delta(wheel_delta);
				});
			}
			WindowEvent::CursorMoved { position, .. } => {
				Input::update_globally(|i| {
					i.set_mouse_pos(position.x as f32, position.y as f32);
				});
			}
			// === RENDER ==============
			WindowEvent::RedrawRequested => {
				ze_log::trace!("RedrawRequested");

				let missing_primary_camera = if let Some(renderer) = &mut self.renderer {
					let Some(scene) = self.scenes.get_mut(&self.active_scene) else {
						return;
					};

					let render_status = scene.with_system_mut::<RenderSystem, _>(|render_system, scene| {
						render_system.render(scene, renderer, &self.resources)
					});
					render_status.and_then(|status| match status {
						RenderStatus::Rendered => None,
						RenderStatus::MissingPrimaryCamera { scene_name } => Some(scene_name),
					})
				} else {
					None
				};
				if let Some(scene_name) = missing_primary_camera {
					exit_for_missing_primary_camera(event_loop, &mut self.missing_primary_camera_dialog, &scene_name);
				}
			}
			_ => {}
		}
	}

	fn device_event(
		&mut self,
		_event_loop: &ActiveEventLoop,
		_device_id: winit::event::DeviceId,
		event: winit::event::DeviceEvent,
	) {
		if let winit::event::DeviceEvent::MouseMotion { delta } = event {
			Input::update_globally(|input| {
				input.add_mouse_delta(delta.0 as f32, delta.1 as f32);
			});
		}
	}
}

#[cfg(test)]
mod tests {
	use std::{collections::HashMap, fs, path::PathBuf, time::Instant};

	use ze_ecs::{Collider, Inactive, RigidBody, Transform};

	use super::*;

	#[test]
	fn standalone_update_removes_inactive_physics_bodies() -> Result<()> {
		let mut scene = Scene::new("Inactive Runtime Physics Test");
		let entity = scene.create_entity("Body");
		scene
			.entity_mut(entity)
			.add_component(Transform::default())
			.add_component(RigidBody::default())
			.add_component(Collider::default());
		scene.add_system(PhysicsSystem::new());

		let mut scenes = HashMap::new();
		scenes.insert(scene.name.clone(), scene);
		let mut app = App {
			runtime: tokio::runtime::Runtime::new()?,
			active_project: None,
			window: None,
			renderer: None,
			ui_manager: None,
			focused: true,
			occluded: false,
			minimized: false,
			scenes,
			active_scene: "Inactive Runtime Physics Test".to_string(),
			last_frame_time: Instant::now(),
			resources: ResourceManager::new("assets"),
			missing_primary_camera_dialog: MissingPrimaryCameraDialogState::Hidden,
		};

		app.update_active_scene_systems(ze_physics::DEFAULT_PHYSICS_TIMESTEP)?;
		let scene = app.active_scene_mut().expect("test scene should be active");
		scene.with_system_mut::<PhysicsSystem, _>(|physics, _scene| {
			assert_eq!(physics.world().rigid_bodies.len(), 1);
			assert_eq!(physics.world().colliders.len(), 1);
		});

		scene.entity_mut(entity).add_component(Inactive);
		app.update_active_scene_systems(ze_physics::DEFAULT_PHYSICS_TIMESTEP)?;
		let scene = app.active_scene_mut().expect("test scene should be active");
		scene.with_system_mut::<PhysicsSystem, _>(|physics, _scene| {
			assert_eq!(physics.world().rigid_bodies.len(), 0);
			assert_eq!(physics.world().colliders.len(), 0);
		});

		Ok(())
	}

	// Regression test for the fix making `Physics.Raycast()`/`GetHoveredEntity()`
	// work from a script's regular `OnUpdate`, not just `OnFixedUpdate` -- the
	// native raycast provider used to be registered only for the duration of
	// `PhysicsSystem`'s fixed-update step, so calling it from `OnUpdate` (the
	// natural place for mouse-hover/click picking) silently always missed.
	// Drives `RaycastFromUpdateProbe.cs` (Sandbox/assets/scripts/), which casts
	// a fixed ray every `OnUpdate` tick and reports hit/miss + a tick counter
	// via its own `Transform.Position`, read back through the real ECS (no
	// scripting-side command queue involved, so nothing here can be confused
	// with `PhysicsSystem`'s own consumption of `ScriptingApiCommand`s).
	// Requires `dotnet build Sandbox/assets/Sandbox.csproj` to have been run at
	// least once.
	#[test]
	fn raycast_from_regular_update_works_across_multiple_ticks() -> Result<()> {
		use std::{collections::BTreeMap, path::Path};

		use ze_core::Vec3;
		use ze_ecs::{ColliderShape, RigidBodyType, Tag};
		use ze_scripting_cs::{Script, Scripts};

		let mut scene = Scene::new("Raycast From Update Test");
		register_scripting_components(scene.registry_mut());

		let target = scene.create_entity("RaycastTarget");
		scene
			.entity_mut(target)
			.add_component(Transform {
				position: Vec3::new(0.0, -3.0, 0.0),
				..Transform::default()
			})
			.add_component(RigidBody {
				body_type: RigidBodyType::Static,
				..RigidBody::default()
			})
			.add_component(Collider {
				shape: ColliderShape::Box {
					half_extents: Vec2::new(1.0, 1.0),
				},
				..Collider::default()
			});
		scene.world_mut().add_component(
			target,
			(Tag {
				tag: "RaycastTarget".to_string(),
			},),
		);

		let probe = scene.create_entity("Probe");
		scene.entity_mut(probe).add_component(Transform::default());
		scene.entity_mut(probe).add_component(Scripts {
			list: vec![Script {
				path: "RaycastFromUpdateProbe".to_string(),
				enabled: true,
				fields: BTreeMap::new(),
			}],
		});

		let assembly_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Sandbox/assets/bin/Sandbox.dll");
		let runtime_config_path =
			Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Sandbox/assets/bin/Sandbox.runtimeconfig.json");
		let scripting_system = ScriptingSystem::from_paths(assembly_path, runtime_config_path);
		let runtime = scripting_system.runtime();
		let mut physics_system = PhysicsSystem::with_scripting(runtime.clone());

		// Deliberately smaller than the physics fixed timestep, so most ticks
		// accumulate without an actual physics step running -- proving the
		// raycast provider survives *between* steps, not just during them.
		let dt = ze_physics::DEFAULT_PHYSICS_TIMESTEP / 3.0;

		let mut hits_after_warmup = 0;
		let mut misses_after_warmup = 0;
		for tick in 1..=15i32 {
			// Same per-frame order as production (`ze_app`'s system registration):
			// scripting's regular `OnUpdate` runs before `PhysicsSystem`'s tick.
			runtime.update(&mut scene, dt)?;
			physics_system.update(&mut scene, dt)?;

			let position = scene
				.world_transform(probe)
				.expect("probe should have a transform")
				.position;
			assert_eq!(
				position.y as i32, tick,
				"probe should report exactly once per OnUpdate tick"
			);

			// A handful of ticks' grace for the very first physics step to land.
			if tick >= 4 {
				if (position.x - 1.0).abs() < f32::EPSILON {
					hits_after_warmup += 1;
				} else {
					misses_after_warmup += 1;
				}
			}
		}

		assert_eq!(
			misses_after_warmup,
			0,
			"Physics.Raycast() from OnUpdate should hit the target on every tick once physics has run at \
			 least once, got {misses_after_warmup} misses out of {} post-warmup ticks",
			hits_after_warmup + misses_after_warmup
		);
		assert!(hits_after_warmup > 0);

		Ok(())
	}

	#[test]
	fn resolve_main_scene_name_prefers_project_over_game_data() {
		let dir = std::env::temp_dir().join(format!("ze_app-main-scene-project-{}", std::process::id()));
		let _ = fs::remove_dir_all(&dir);
		fs::create_dir_all(&dir).unwrap();
		fs::write(dir.join("GameData.toml"), "main_scene = \"FromGameData\"\n").unwrap();

		let mut project = ze_project::Project::new(
			"Test".to_string(),
			"FromProject".to_string(),
			PathBuf::from("assets"),
			PathBuf::from("assets/bin/Scripts.dll"),
		);
		project.scenes.push("FromProject".to_string());

		let resources = ResourceManager::new(&dir);
		assert_eq!(resolve_main_scene_name(&resources, Some(&project)), "FromProject");

		let _ = fs::remove_dir_all(&dir);
	}

	#[test]
	fn resolve_main_scene_name_falls_back_to_packaged_game_data_without_project() {
		// This is the shipped-standalone-game code path: `ResourceManager::for_runtime`
		// finds `Game/assets.zepack` next to the executable with no `Project`/
		// `ZEProject.toml` in sight, so `GameData.toml` (baked in at build time
		// from the editor's main-scene setting) must be consulted instead of a
		// hardcoded scene name.
		let dir = std::env::temp_dir().join(format!("ze_app-main-scene-gamedata-{}", std::process::id()));
		let _ = fs::remove_dir_all(&dir);
		fs::create_dir_all(&dir).unwrap();
		fs::write(dir.join("GameData.toml"), "main_scene = \"MainMenu\"\n").unwrap();

		let resources = ResourceManager::new(&dir);
		assert_eq!(resolve_main_scene_name(&resources, None), "MainMenu");

		let _ = fs::remove_dir_all(&dir);
	}

	#[test]
	fn resolve_main_scene_name_defaults_when_no_project_and_no_game_data() {
		let dir = std::env::temp_dir().join(format!("ze_app-main-scene-none-{}", std::process::id()));
		let _ = fs::remove_dir_all(&dir);
		fs::create_dir_all(&dir).unwrap();

		let resources = ResourceManager::new(&dir);
		assert_eq!(resolve_main_scene_name(&resources, None), DEFAULT_SCENE_NAME);

		let _ = fs::remove_dir_all(&dir);
	}
}
