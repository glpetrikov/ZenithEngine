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
use ze_assets::ResourceManager;
use ze_core::{Result, Vec2, bail};
use ze_ecs::{EditorOnly, EntitiesView, PhysicsSettings, Scene, System, registry};
use ze_input::{Input, ZKeyCode, ZMouseCode};
use ze_physics::PhysicsSystem;
use ze_project::Project;
use ze_renderer::{Camera, EditorCameraSystem, RenderStatus, RenderSystem, register_renderer_components};
use ze_scripting_cs::{
	SHUTDOWN_REQUESTED, ScriptingSceneLoadCommand, ScriptingSystem, drain_scripting_scene_load_commands,
	register_scripting_components,
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
		let resources = active_project.as_ref().map_or_else(
			|| ResourceManager::for_runtime("Sandbox/assets"),
			|project| ResourceManager::new(project.asset_dir.clone()),
		);
		let scene = load_main_scene(&resources, active_project.as_ref(), None)?;

		if let Err(error) = resources.compile_game_shaders() {
			ze_log::error!("Failed to compile game shaders: {error:?}");
		}

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
		let Some(scene) = self.active_scene_mut() else {
			return Ok(());
		};

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
		let scene_name = self
			.active_project
			.as_ref()
			.map_or(DEFAULT_SCENE_NAME, |project| project.main_scene.as_str())
			.to_string();
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
	let scene_name = active_project.map_or(DEFAULT_SCENE_NAME, |project| project.main_scene.as_str());
	load_project_scene(resources, active_project, scene_name, ui_manager)
}

pub fn load_project_scene(
	resources: &ResourceManager,
	active_project: Option<&Project>,
	scene_name: &str,
	ui_manager: Option<UiManagerHandle>,
) -> Result<Scene> {
	validate_scene_name(scene_name)?;
	let mut registry = registry::ComponentRegistry::new();

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

	activate_runtime_camera(&mut scene);
	apply_project_physics_defaults(&mut scene, active_project);
	scene.add_system(EditorCameraSystem::new());
	let (scripts_path, runtime_config_path) = scripting_paths(active_project, resources);
	let scripting_system = ScriptingSystem::from_paths(scripts_path, runtime_config_path);
	let scripting_runtime = scripting_system.runtime();
	scene.add_system(scripting_system);
	scene.add_system(PhysicsSystem::with_scripting(scripting_runtime));
	scene.add_system(RenderSystem::new());
	if let Some(handle) = ui_manager {
		scene.add_system(UISystem::new(handle));
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

fn activate_runtime_camera(scene: &mut Scene) {
	if has_primary_runtime_camera(scene) {
		return;
	}

	let Some(camera) = first_runtime_camera(scene) else {
		return;
	};

	if let Ok(mut camera) = scene.world_mut().get::<&mut Camera>(camera) {
		camera.primary = true;
	}
}

fn has_primary_runtime_camera(scene: &Scene) -> bool {
	let world = scene.world();
	let mut found = false;
	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&EditorOnly>(entity).is_ok() {
				continue;
			}
			let Ok(camera) = world.get::<&Camera>(entity) else {
				continue;
			};
			if camera.primary {
				found = true;
				break;
			}
		}
	});
	found
}

fn first_runtime_camera(scene: &Scene) -> Option<ze_ecs::EntityId> {
	let world = scene.world();
	let mut camera = None;
	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&EditorOnly>(entity).is_ok() {
				continue;
			}
			if world.get::<&Camera>(entity).is_ok() {
				camera = Some(entity);
				break;
			}
		}
	});
	camera
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
	ze_log::error!("Scene `{name}` has no primary camera; exiting");
	let _ = MessageDialog::new()
		.set_level(MessageLevel::Error)
		.set_title("ZeroEngine")
		.set_description(format!(
			"Scene `{scene_name}` is missing a primary camera. Add a Camera component with Primary enabled before running the build."
		))
		.set_buttons(MessageButtons::Ok)
		.show();
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
		let attrs = Window::default_attributes()
			.with_title("ZeroEngine")
			.with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));

		// TODO: add asset manager and icon
		let window = match event_loop.create_window(attrs) {
			Ok(window) => Arc::new(window),
			Err(error) => {
				ze_log::error!("Failed to create window: {error}");
				event_loop.exit();
				return;
			}
		};

		// TODO: add cursor grab parameters in game settings
		let _ = window
			.set_cursor_grab(winit::window::CursorGrabMode::Locked)
			.or_else(|_| window.set_cursor_grab(winit::window::CursorGrabMode::Confined));

		window.set_cursor_visible(false);

		let renderer = self.runtime.block_on(ze_renderer::Renderer::new(window.clone()));
		match renderer {
			Ok(mut renderer) => {
				let ui_manager =
					UiManagerHandle::new(ze_ui::UiManager::new(renderer.device(), renderer.queue(), &window));
				renderer.set_ui_manager(ui_manager.clone());
				self.ui_manager = Some(ui_manager);
				self.renderer = Some(renderer);
			}
			Err(error) => {
				ze_log::error!("Failed to create renderer: {error:?}");
				event_loop.exit();
				return;
			}
		}

		self.window = Some(window);

		self.last_frame_time = Instant::now();
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

				if let Some(renderer) = &mut self.renderer
					&& !self.minimized
				{
					renderer.resize(size);
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
	use std::{collections::HashMap, time::Instant};

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
}
