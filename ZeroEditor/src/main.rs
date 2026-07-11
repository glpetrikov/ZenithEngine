mod asset_hot_reload;
mod editor_workspace;
mod panels;
pub mod style;
mod undo_redo;

use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
	sync::{
		Arc,
		atomic::Ordering,
		mpsc::{self, Receiver, TryRecvError},
	},
	thread,
	time::{Duration, Instant},
};

use asset_hot_reload::{AssetHotReload, ScriptStatus, reload_managed_assembly};
use editor_workspace::{
	BuildRequest, BuildStatus, EditorRequest, EditorStatus, EditorWorkspace, SaveState, SaveStatus, SceneInfo,
};
use egui::{
	Context, RawInput, TextureId,
	epaint::{ClippedShape, Shape},
	pos2,
};
use egui_wgpu::{Renderer as EguiRenderer, RendererOptions, ScreenDescriptor};
use winit::{
	application::ApplicationHandler,
	event::{ElementState, MouseButton, WindowEvent},
	event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
	keyboard::PhysicalKey,
	window::{Window, WindowId},
};
use ze_app::load_project_scene;
use ze_assets::ResourceManager;
use ze_core::{Context as _, Quat, Vec3, anyhow, bail};
use ze_ecs::{EditorOnly, EntityId, Name, PhysicsSettings, SaveFile, Scene, Tag, Transform, shipyard::IntoIter};
use ze_physics::PhysicsSystem;
use ze_project::Project;
use ze_renderer::{Camera, CameraProjection, CompositeMode, RenderSystem, Renderer};
use ze_scripting_cs::{
	SHUTDOWN_REQUESTED, ScriptingSceneLoadCommand, ScriptingSystem, drain_scripting_scene_load_commands,
};

const PROJECT_SCENE_SCAN_INTERVAL: Duration = Duration::from_mins(1);
const EDITOR_ONLY_COMPONENT_TYPE: &str = "ze.editor.only";
const EDITOR_CAMERA_NAME: &str = "EditorCamera";
const PHYSICS_SETTINGS_NAME: &str = "PhysicsSettings";

pub struct EditorRenderFlow {
	egui_ctx: Context,
	workspace: EditorWorkspace,
	egui_renderer: EguiRenderer,
	viewport_texture_id: TextureId,
	viewport_generation: u64,
	pending_viewport_resolution: Option<[u32; 2]>,
	viewport_wants_game_input: bool,
	save_status: SaveStatus,
	window: Arc<Window>,
}

pub struct EditorRenderOutput {
	pub platform_output: egui::PlatformOutput,
	pub build_request: Option<BuildRequest>,
	pub menu_request: Option<EditorRequest>,
	pub project_saved: Option<Project>,
	pub project_save_error: Option<String>,
	pub project_config_changed: bool,
	pub close_requested: bool,
	// Rendered height of the title bar; used by winit to detect titlebar clicks
	pub titlebar_height: f32,
	// Rects of interactive titlebar elements; prevents drag_window() on buttons
	pub titlebar_interactive_rects: Vec<egui::Rect>,
}

#[derive(Debug, Clone)]
struct EditorRenderStatus {
	script: ScriptStatus,
	build: BuildStatus,
	build_message: Option<String>,
}

impl EditorRenderFlow {
	pub fn new(renderer: &Renderer, project_assets_root: Option<PathBuf>, window: Arc<Window>) -> Self {
		let mut egui_renderer =
			EguiRenderer::new(renderer.device(), renderer.surface_format(), RendererOptions::default());

		let viewport_texture_id = egui_renderer.register_native_texture(
			renderer.device(),
			renderer.viewport_texture_view(),
			egui_wgpu::wgpu::FilterMode::Linear,
		);

		let egui_ctx = Context::default();
		egui_ctx.set_fonts(style::get_fonts());
		egui_ctx.set_theme(egui::ThemePreference::Dark);
		egui_ctx.set_visuals(egui::Visuals::dark());
		egui_ctx.all_styles_mut(|style| *style = style::get_style());
		ze_log::init_editor(&egui_ctx);

		Self {
			egui_ctx,
			workspace: EditorWorkspace::new(viewport_texture_id, project_assets_root, window.clone()),
			egui_renderer,
			viewport_texture_id,
			viewport_generation: renderer.viewport_generation(),
			pending_viewport_resolution: None,
			viewport_wants_game_input: false,
			save_status: SaveStatus::saved(),
			window,
		}
	}

	pub fn context(&self) -> Context { self.egui_ctx.clone() }

	pub const fn mode(&self) -> editor_workspace::EditorMode { self.workspace.mode() }

	pub fn game_input_active(&self) -> bool { self.workspace.mode() == editor_workspace::EditorMode::Play }

	pub const fn mark_scene_loaded(&mut self) { self.save_status = SaveStatus::saved(); }

	pub const fn mark_save_started(&mut self) { self.save_status.state = SaveState::Saving; }

	pub fn mark_save_succeeded(&mut self) {
		self.workspace.mark_project_config_saved();
		self.save_status = SaveStatus {
			state: SaveState::Saved,
			last_successful_save: Some(Instant::now()),
		};
	}

	pub fn mark_project_config_saved(&mut self) {
		self.workspace.mark_project_config_saved();
		self.save_status.last_successful_save = Some(Instant::now());
		if self.save_status.state == SaveState::SaveFailed {
			self.save_status.state = SaveState::Saved;
		}
	}

	pub const fn mark_save_failed(&mut self) { self.save_status.state = SaveState::SaveFailed; }

	pub fn prepare_frame(&mut self, renderer: &mut Renderer) {
		renderer.set_composite_mode(CompositeMode::Editor);
		if let Some([width, height]) = self.pending_viewport_resolution.take() {
			renderer.set_viewport_size(width, height);
		}
		self.sync_viewport_texture(renderer);
	}

	fn sync_viewport_texture(&mut self, renderer: &Renderer) {
		let generation = renderer.viewport_generation();
		if generation != self.viewport_generation {
			self.egui_renderer.free_texture(&self.viewport_texture_id);
			self.viewport_texture_id = self.egui_renderer.register_native_texture(
				renderer.device(),
				renderer.viewport_texture_view(),
				egui_wgpu::wgpu::FilterMode::Linear,
			);
			self.viewport_generation = generation;
		}

		self.egui_renderer.update_egui_texture_from_wgpu_texture(
			renderer.device(),
			renderer.viewport_texture_view(),
			egui_wgpu::wgpu::FilterMode::Linear,
			self.viewport_texture_id,
		);
	}

	fn render(
		&mut self,
		renderer: &mut Renderer,
		scene: &mut Scene,
		project: Option<&mut Project>,
		scenes: &[SceneInfo],
		has_dirty_scenes: bool,
		input: RawInput,
		status: &EditorRenderStatus,
	) -> EditorRenderOutput {
		renderer.set_composite_mode(CompositeMode::Editor);
		self.sync_viewport_texture(renderer);
		self.refresh_save_status(has_dirty_scenes);
		let mut build_request = None;
		let mut menu_request = None;
		let mut project_saved = None;
		let mut project_save_error = None;
		let mut project_config_changed = false;
		let mut close_requested = false;
		let mut titlebar_height = 0.0;
		let mut titlebar_interactive_rects = Vec::new();
		let mut project = project;

		let full_output = self.egui_ctx.run_ui(input, |ui| {
			let editor_status = EditorStatus {
				script: status.script,
				save: self.save_status,
				build: status.build,
				build_message: status.build_message.clone(),
			};
			let output = self.workspace.show(
				ui,
				self.viewport_texture_id,
				scene,
				project.as_deref_mut(),
				scenes,
				&editor_status,
			);
			build_request = output.build_request;
			menu_request = output.menu_request;
			project_saved = output.project_saved;
			project_save_error = output.project_save_error;
			project_config_changed = output.project_config_changed;
			close_requested = output.close_requested;
			titlebar_height = output.titlebar_height;
			titlebar_interactive_rects = output.titlebar_interactive_rects;
			if let Some(viewport) = output.viewport {
				self.pending_viewport_resolution = Some(viewport.resolution);
				self.viewport_wants_game_input =
					viewport.wants_game_input || self.workspace.mode() == editor_workspace::EditorMode::Play;
				let _ = (viewport.hovered, viewport.focused);
			}
		});

		let egui::FullOutput {
			platform_output,
			textures_delta,
			mut shapes,
			pixels_per_point,
			..
		} = full_output;

		outline_text_shapes(&mut shapes);
		let paint_jobs = self.egui_ctx.tessellate(shapes, pixels_per_point);
		for (texture_id, delta) in &textures_delta.set {
			self.egui_renderer
				.update_texture(renderer.device(), renderer.queue(), *texture_id, delta);
		}

		let surface_size = renderer.surface_size();
		let screen_descriptor = ScreenDescriptor {
			size_in_pixels: [surface_size.width.max(1), surface_size.height.max(1)],
			pixels_per_point,
		};

		renderer.present_editor(|device, queue, encoder, surface_view| {
			let callback_buffers =
				self.egui_renderer
					.update_buffers(device, queue, encoder, &paint_jobs, &screen_descriptor);

			{
				let render_pass_descriptor = egui_wgpu::wgpu::RenderPassDescriptor {
					label: Some("Editor Pass"),
					color_attachments: &[Some(egui_wgpu::wgpu::RenderPassColorAttachment {
						view: surface_view,
						resolve_target: None,
						depth_slice: None,
						ops: egui_wgpu::wgpu::Operations {
							load: egui_wgpu::wgpu::LoadOp::Clear(egui_wgpu::wgpu::Color::BLACK),
							store: egui_wgpu::wgpu::StoreOp::Store,
						},
					})],
					depth_stencil_attachment: None,
					occlusion_query_set: None,
					timestamp_writes: None,
					multiview_mask: None,
				};
				let mut render_pass = encoder.begin_render_pass(&render_pass_descriptor).forget_lifetime();
				self.egui_renderer
					.render(&mut render_pass, &paint_jobs, &screen_descriptor);
			}

			callback_buffers
		});

		for texture_id in &textures_delta.free {
			self.egui_renderer.free_texture(texture_id);
		}

		EditorRenderOutput {
			platform_output,
			build_request,
			menu_request,
			project_saved,
			project_save_error,
			project_config_changed,
			close_requested,
			titlebar_height,
			titlebar_interactive_rects,
		}
	}

	fn refresh_save_status(&mut self, has_dirty_scenes: bool) {
		if self.save_status.state == SaveState::Saving {
			return;
		}
		if self.save_status.state == SaveState::SaveFailed {
			return;
		}

		self.save_status.state = if has_dirty_scenes {
			SaveState::UnsavedChanges
		} else {
			SaveState::Saved
		};
	}
}

struct EditorSceneManager {
	scenes: BTreeMap<String, EditorSceneEntry>,
	active_scene: String,
	play_snapshot: Option<EditorPlaySnapshot>,
}

struct EditorSceneEntry {
	scene: Scene,
	saved_fingerprint: Option<String>,
}

struct EditorPlaySnapshot {
	active_scene: String,
	scenes: BTreeMap<String, EditorSceneSnapshot>,
}

struct EditorSceneSnapshot {
	scene: SaveFile,
	saved_fingerprint: Option<String>,
}

impl EditorSceneManager {
	fn from_scene(scene: Scene, mark_saved: bool) -> Self {
		let active_scene = scene.name.clone();
		let mut manager = Self {
			scenes: BTreeMap::new(),
			active_scene,
			play_snapshot: None,
		};
		manager.insert_scene(scene, mark_saved);
		manager
	}

	fn load_project(resources: &ResourceManager, project: &Project) -> ze_core::Result<Self> {
		let mut manager = Self {
			scenes: BTreeMap::new(),
			active_scene: project.main_scene.clone(),
			play_snapshot: None,
		};

		for scene_name in available_project_scene_names(project)? {
			let existed = project_scene_path(project, &scene_name).is_file();
			let mut scene = load_project_scene(resources, Some(project), &scene_name)
				.with_context(|| format!("failed to load project scene `{scene_name}`"))?;
			ensure_editor_runtime_entities(&mut scene, Some(project));
			manager.insert_scene(scene, existed);
		}

		if !manager.scenes.contains_key(&manager.active_scene) {
			let scene_name = project.main_scene.clone();
			let existed = project_scene_path(project, &scene_name).is_file();
			let mut scene = load_project_scene(resources, Some(project), &scene_name)
				.with_context(|| format!("failed to load main scene `{scene_name}`"))?;
			ensure_editor_runtime_entities(&mut scene, Some(project));
			manager.insert_scene(scene, existed);
		}

		Ok(manager)
	}

	fn active_scene_mut(&mut self) -> Option<&mut Scene> {
		self.scenes.get_mut(&self.active_scene).map(|entry| &mut entry.scene)
	}

	fn insert_scene(&mut self, scene: Scene, mark_saved: bool) {
		let name = scene.name.clone();
		let saved_fingerprint = mark_saved.then(|| scene_fingerprint(&scene));
		self.scenes.insert(
			name,
			EditorSceneEntry {
				scene,
				saved_fingerprint,
			},
		);
	}

	fn set_active_scene(&mut self, name: &str) -> ze_core::Result<()> {
		if !self.scenes.contains_key(name) {
			bail!("scene `{name}` is not loaded");
		}

		self.active_scene = name.to_string();
		Ok(())
	}

	fn enter_play_mode(&mut self) -> ze_core::Result<()> {
		if self.play_snapshot.is_some() {
			return Ok(());
		}

		let scenes = self
			.scenes
			.iter()
			.map(|(name, entry)| {
				(
					name.clone(),
					EditorSceneSnapshot {
						scene: entry.scene.snapshot(),
						saved_fingerprint: entry.saved_fingerprint.clone(),
					},
				)
			})
			.collect();

		self.play_snapshot = Some(EditorPlaySnapshot {
			active_scene: self.active_scene.clone(),
			scenes,
		});
		let Some(active_scene) = self.scenes.get_mut(&self.active_scene).map(|entry| &mut entry.scene) else {
			bail!("cannot enter Play mode because no active scene is loaded");
		};
		editor_workspace::switch_viewport_camera(active_scene, editor_workspace::EditorMode::Play);
		Ok(())
	}

	fn stop_play_mode(&mut self) -> ze_core::Result<()> {
		let Some(snapshot) = self.play_snapshot.take() else {
			return Ok(());
		};

		self.scenes
			.retain(|scene_name, _entry| snapshot.scenes.contains_key(scene_name));

		for (scene_name, scene_snapshot) in snapshot.scenes {
			let Some(entry) = self.scenes.get_mut(&scene_name) else {
				ze_log::error!("cannot restore Play mode snapshot for missing scene `{scene_name}`");
				continue;
			};
			entry.scene.restore_snapshot(scene_snapshot.scene)?;
			reset_scene_runtime_systems(&mut entry.scene);
			entry.saved_fingerprint = scene_snapshot.saved_fingerprint;
		}

		self.active_scene = if self.scenes.contains_key(&snapshot.active_scene) {
			snapshot.active_scene
		} else {
			self.scenes.keys().next().cloned().unwrap_or_else(|| "main".to_string())
		};

		if let Some(active_scene) = self.active_scene_mut() {
			editor_workspace::switch_viewport_camera(active_scene, editor_workspace::EditorMode::Edit);
		}

		Ok(())
	}

	fn apply_scripting_scene_load_commands(&mut self, resources: &ResourceManager, project: Option<&Project>) -> bool {
		let mut applied_scene_load = false;
		for command in drain_scripting_scene_load_commands() {
			match self.apply_scripting_scene_load_command(resources, project, command) {
				Ok(()) => applied_scene_load = true,
				Err(error) => ze_log::error!("failed to apply script scene command in editor Play mode: {error:?}"),
			}
		}
		applied_scene_load
	}

	fn apply_scripting_scene_load_command(
		&mut self,
		resources: &ResourceManager,
		project: Option<&Project>,
		command: ScriptingSceneLoadCommand,
	) -> ze_core::Result<()> {
		match command {
			ScriptingSceneLoadCommand::Load { name } => self.load_and_activate_scene(resources, project, &name),
			ScriptingSceneLoadCommand::LoadMain => {
				let scene_name = project
					.map_or("main", |project| project.main_scene.as_str())
					.to_string();
				self.load_and_activate_scene(resources, project, &scene_name)
			}
			ScriptingSceneLoadCommand::Reload => {
				let scene_name = self.active_scene.clone();
				self.load_and_activate_scene(resources, project, &scene_name)
			}
		}
	}

	fn load_and_activate_scene(
		&mut self,
		resources: &ResourceManager,
		project: Option<&Project>,
		scene_name: &str,
	) -> ze_core::Result<()> {
		let mut scene = load_project_scene(resources, project, scene_name)?;
		ensure_editor_runtime_entities(&mut scene, project);
		let loaded_name = scene.name.clone();
		self.insert_scene(
			scene,
			project_scene_path_if_loaded(project, &loaded_name).is_some_and(|path| path.is_file()),
		);
		self.active_scene = loaded_name;
		Ok(())
	}

	fn scene_infos(&self) -> Vec<SceneInfo> {
		self.scenes
			.iter()
			.map(|(name, entry)| SceneInfo {
				name: name.clone(),
				active: name == &self.active_scene,
				dirty: entry.is_dirty(),
			})
			.collect()
	}

	fn has_dirty_scenes(&self) -> bool { self.scenes.values().any(EditorSceneEntry::is_dirty) }

	fn save_all_dirty_scenes(&mut self, project: &Project) -> ze_core::Result<usize> {
		let scene_directory = project_scenes_dir(project);
		let mut saved = 0usize;

		for (scene_name, entry) in &mut self.scenes {
			if !entry.is_dirty() {
				continue;
			}

			let scene_path = project_scene_path(project, scene_name);
			save_editor_scene(&entry.scene, &scene_directory, scene_name)
				.with_context(|| format!("failed to save scene `{scene_name}` to `{}`", scene_path.display()))?;
			entry.saved_fingerprint = Some(scene_fingerprint(&entry.scene));
			saved += 1;
		}

		Ok(saved)
	}

	fn load_missing_project_scenes(
		&mut self,
		resources: &ResourceManager,
		project: &Project,
		scene_names: &[String],
	) -> ze_core::Result<usize> {
		let mut loaded = 0usize;
		for scene_name in scene_names {
			if self.scenes.contains_key(scene_name) {
				continue;
			}
			if !project_scene_path(project, scene_name).is_file() {
				continue;
			}

			let mut scene = load_project_scene(resources, Some(project), scene_name)
				.with_context(|| format!("failed to load discovered scene `{scene_name}`"))?;
			ensure_editor_runtime_entities(&mut scene, Some(project));
			self.insert_scene(scene, true);
			loaded += 1;
		}

		Ok(loaded)
	}

	fn retain_project_scenes(&mut self, project: &Project, scene_names: &[String]) {
		self.scenes.retain(|name, entry| {
			scene_names.iter().any(|scene| scene == name)
				|| project.scenes.iter().any(|scene| scene == name)
				|| entry.is_dirty()
		});
		if self.scenes.contains_key(&self.active_scene) {
			return;
		}

		self.active_scene = if self.scenes.contains_key(&project.main_scene) {
			project.main_scene.clone()
		} else {
			self.scenes
				.keys()
				.next()
				.cloned()
				.unwrap_or_else(|| project.main_scene.clone())
		};
	}
}

impl EditorSceneEntry {
	fn is_dirty(&self) -> bool {
		let current = scene_fingerprint(&self.scene);
		self.saved_fingerprint.as_ref() != Some(&current)
	}
}

#[allow(clippy::struct_excessive_bools)]
struct EditorApp {
	runtime: tokio::runtime::Runtime,
	active_project: Option<Project>,
	project_config_dirty: bool,
	last_scene_file_scan: Instant,
	build_status: BuildStatus,
	build_message: Option<String>,
	build_receiver: Option<Receiver<BuildEvent>>,
	window: Option<Arc<Window>>,
	renderer: Option<Renderer>,
	editor_flow: Option<EditorRenderFlow>,
	egui_state: Option<egui_winit::State>,
	resources: ResourceManager,
	scene_manager: Option<EditorSceneManager>,
	occluded: bool,
	minimized: bool,
	last_frame: Instant,
	asset_hot_reload: AssetHotReload,
	script_hot_reload_build_pause: ScriptHotReloadBuildPause,
	// Rendered height of the title bar, updated every frame from egui
	titlebar_height: f32,
	// Last known cursor position in logical pixels
	last_mouse_pos: egui::Pos2,
	// Interactive rects from the title bar; used to block drag on buttons
	titlebar_interactive_rects: Vec<egui::Rect>,
}

enum EditorBuildResult {
	Succeeded,
	Failed { error: String },
}

enum BuildEvent {
	Stage { message: String },
	Result(EditorBuildResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScriptHotReloadBuildPause {
	NotPaused,
	Paused,
}

impl EditorApp {
	fn new(mut active_project: Option<Project>) -> Self {
		if let Some(project) = active_project.as_mut()
			&& let Err(error) = project.save_to_project_file()
		{
			ze_log::warn!("failed to persist project scene list at startup: {error:?}");
		}
		let resources = active_project.as_ref().map_or_else(
			|| ResourceManager::for_runtime("assets"),
			|project| ResourceManager::new(project.asset_dir.clone()),
		);
		let asset_hot_reload = create_asset_hot_reload(&resources, active_project.as_ref());
		let runtime = match tokio::runtime::Runtime::new() {
			Ok(runtime) => runtime,
			Err(error) => {
				ze_log::error!("failed to create tokio runtime for editor: {error}");
				std::process::exit(1);
			}
		};
		Self {
			runtime,
			active_project,
			project_config_dirty: false,
			last_scene_file_scan: Instant::now(),
			build_status: BuildStatus::Idle,
			build_receiver: None,
			build_message: None,
			window: None,
			renderer: None,
			editor_flow: None,
			egui_state: None,
			asset_hot_reload,
			resources,
			scene_manager: None,
			occluded: false,
			minimized: false,
			last_frame: Instant::now(),
			script_hot_reload_build_pause: ScriptHotReloadBuildPause::NotPaused,
			titlebar_height: 0.0,
			last_mouse_pos: egui::Pos2::ZERO,
			titlebar_interactive_rects: Vec::new(),
		}
	}
}

impl ApplicationHandler for EditorApp {
	fn resumed(&mut self, event_loop: &ActiveEventLoop) {
		let attributes = Window::default_attributes()
			.with_title("ZeroEditor")
			.with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
			.with_decorations(false);
		let window = Arc::new(match event_loop.create_window(attributes) {
			Ok(window) => window,
			Err(error) => {
				ze_log::error!("failed to create editor window: {error}");
				event_loop.exit();
				return;
			}
		});
		let mut renderer = match self.runtime.block_on(Renderer::new(window.clone())) {
			Ok(renderer) => renderer,
			Err(error) => {
				ze_log::error!("failed to create renderer: {error:?}");
				event_loop.exit();
				return;
			}
		};
		renderer.set_composite_mode(CompositeMode::Editor);

		let scene_manager = match &self.active_project {
			Some(project) => match EditorSceneManager::load_project(&self.resources, project) {
				Ok(manager) => manager,
				Err(error) => {
					ze_log::error!("failed to load project scenes; opening empty editor scene: {error:?}");
					EditorSceneManager::from_scene(create_default_editor_scene(), true)
				}
			},
			None => EditorSceneManager::from_scene(create_default_editor_scene(), true),
		};
		let mut editor_flow = EditorRenderFlow::new(
			&renderer,
			self.active_project.as_ref().map(|project| project.asset_dir.clone()),
			window.clone(),
		);
		editor_flow.mark_scene_loaded();
		let egui_state = egui_winit::State::new(
			editor_flow.context(),
			egui::ViewportId::ROOT,
			window.as_ref(),
			Some(window.scale_factor() as f32),
			Some(winit::window::Theme::Dark),
			None,
		);

		self.renderer = Some(renderer);
		self.editor_flow = Some(editor_flow);
		self.egui_state = Some(egui_state);
		self.scene_manager = Some(scene_manager);
		self.window = Some(window);
		self.last_frame = Instant::now();
	}

	fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
		event_loop.set_control_flow(ControlFlow::Poll);
		if let Some(window) = &self.window
			&& !self.occluded
			&& !self.minimized
		{
			window.request_redraw();
		}
	}

	#[allow(clippy::too_many_lines)]
	fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
		let Some(window) = self.window.clone() else {
			return;
		};

		let mut egui_consumed = false;
		if let Some(egui_state) = &mut self.egui_state {
			let response = egui_state.on_window_event(&window, &event);
			egui_consumed = response.consumed;
			if response.repaint {
				window.request_redraw();
			}
		}
		let game_input_active = self
			.editor_flow
			.as_ref()
			.is_some_and(EditorRenderFlow::game_input_active);
		if should_forward_game_input(&event, game_input_active, egui_consumed) {
			forward_game_input(&event);
		}

		match event {
			WindowEvent::CloseRequested => event_loop.exit(),
			WindowEvent::Occluded(occluded) => {
				self.occluded = occluded;
				if !occluded {
					window.request_redraw();
				}
			}
			WindowEvent::Resized(size) => {
				self.minimized = size.width == 0 || size.height == 0;
				if self.minimized {
				} else {
					if let Some(renderer) = &mut self.renderer {
						renderer.resize(size);
					}
					window.request_redraw();
				}
			}
			WindowEvent::ScaleFactorChanged { scale_factor: _, .. } => {
				let size = window.inner_size();
				self.minimized = size.width == 0 || size.height == 0;
				if !self.minimized {
					if let Some(renderer) = &mut self.renderer {
						renderer.resize(size);
					}
					window.request_redraw();
				}
			}
			WindowEvent::RedrawRequested => {
				self.sync_project_scene_files_if_due();
				self.poll_build_worker();
				let build_status = self.build_status;
				let build_message = self.build_message.clone();
				let (
					build_request,
					menu_request,
					project_saved,
					project_save_error,
					project_config_changed,
					close_requested,
					titlebar_height,
					titlebar_interactive_rects,
				) = {
					let (Some(renderer), Some(editor_flow), Some(egui_state), Some(scene_manager), asset_hot_reload) = (
						&mut self.renderer,
						&mut self.editor_flow,
						&mut self.egui_state,
						&mut self.scene_manager,
						&mut self.asset_hot_reload,
					) else {
						return;
					};
					let now = Instant::now();
					let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
					self.last_frame = now;

					editor_flow.prepare_frame(renderer);
					let game_running = editor_flow.mode() == editor_workspace::EditorMode::Play;
					if !update_editor_scene_before_render(
						scene_manager,
						&self.resources,
						self.active_project.as_ref(),
						asset_hot_reload,
						renderer,
						game_running,
						dt,
					) {
						return;
					}

					let scene_infos = scene_manager.scene_infos();
					let has_dirty_scenes = scene_manager.has_dirty_scenes();
					let Some(scene) = scene_manager.active_scene_mut() else {
						ze_log::error!("cannot render editor because no active scene is loaded");
						return;
					};

					scene.with_system_mut::<RenderSystem, _>(|render_system, scene| {
						let _ = render_system.render(scene, renderer, &self.resources);
					});
					renderer.flush_game_pass();

					let input = egui_state.take_egui_input(&window);

					if asset_hot_reload.take_script_classes_dirty() {
						editor_flow.workspace.mark_script_classes_dirty();
					}

					let editor_render_status = EditorRenderStatus {
						script: asset_hot_reload.script_status(),
						build: build_status,
						build_message,
					};
					let output = editor_flow.render(
						renderer,
						scene,
						self.active_project.as_mut(),
						&scene_infos,
						has_dirty_scenes,
						input,
						&editor_render_status,
					);
					let EditorRenderOutput {
						platform_output,
						build_request,
						menu_request,
						project_saved,
						project_save_error,
						project_config_changed,
						close_requested,
						titlebar_height,
						titlebar_interactive_rects,
					} = output;
					egui_state.handle_platform_output(&window, platform_output);
					(
						build_request,
						menu_request,
						project_saved,
						project_save_error,
						project_config_changed,
						close_requested,
						titlebar_height,
						titlebar_interactive_rects,
					)
				};

				if project_config_changed {
					self.project_config_dirty = true;
				}
				if let Some(project) = project_saved {
					self.apply_saved_project(project);
					if !matches!(menu_request, Some(EditorRequest::SaveProject)) {
						self.mark_project_config_saved();
					}
				}
				if let Some(menu_request) = menu_request {
					match menu_request {
						EditorRequest::SaveProject => {
							self.save_project_from_request(project_save_error);
						}
						EditorRequest::OpenProject => self.open_project_from_dialog(),
						EditorRequest::NewProject => self.create_project_from_dialog(),
						EditorRequest::CreateScene(scene_name) => self.create_and_open_scene(&scene_name),
						EditorRequest::SwitchScene(scene_name) => self.switch_scene(&scene_name),
						EditorRequest::EnterPlayMode => self.enter_play_mode(),
						EditorRequest::StopPlayMode => self.stop_play_mode(),
					}
				}
				if let Some(build_request) = build_request {
					self.start_build_distributable(build_request.output_dir);
				}
				if close_requested {
					event_loop.exit();
					return;
				}
				if SHUTDOWN_REQUESTED.swap(false, Ordering::SeqCst) {
					let is_playing = self
						.editor_flow
						.as_ref()
						.is_some_and(|f| f.mode() == editor_workspace::EditorMode::Play);
					if is_playing {
						self.stop_play_mode();
					}
				}
				self.titlebar_height = titlebar_height;
				self.titlebar_interactive_rects = titlebar_interactive_rects;
				ze_input::Input::update_globally(ze_input::Input::late_update);
			}
			WindowEvent::CursorMoved { position, .. } => {
				// Store cursor position for titlebar hit-testing on mouse down
				self.last_mouse_pos = pos2(position.x as f32, position.y as f32);
			}
			WindowEvent::MouseInput {
				state: ElementState::Pressed,
				button: MouseButton::Left,
				..
			} => {
				// Start drag only when the press is inside the titlebar
				// AND not over any interactive element (menu buttons, window controls)
				if self.last_mouse_pos.y < self.titlebar_height
					&& !self
						.titlebar_interactive_rects
						.iter()
						.any(|r| r.contains(self.last_mouse_pos))
					&& let Some(window) = &self.window
				{
					let _ = window.drag_window();
				}
			}
			_ => {}
		}
	}
}

impl EditorApp {
	fn save_project(&mut self) -> ze_core::Result<()> {
		self.sync_project_scene_list()?;

		let project = self
			.active_project
			.as_ref()
			.ok_or_else(|| anyhow!("cannot save scenes without a project loaded"))?
			.clone();

		let scene_manager = self
			.scene_manager
			.as_mut()
			.ok_or_else(|| anyhow!("cannot save scenes because no scene manager is loaded"))?;
		scene_manager.save_all_dirty_scenes(&project)?;

		let project = self
			.active_project
			.as_ref()
			.ok_or_else(|| anyhow!("cannot save project config without a project loaded"))?
			.clone();

		if self.project_config_dirty {
			project.save_to_project_file()?;
			self.project_config_dirty = false;
		}
		Ok(())
	}

	fn save_project_from_request(&mut self, project_save_error: Option<String>) {
		self.mark_save_started();
		let project_config_failed = project_save_error.is_some_and(|error| {
			ze_log::error!("failed to save project config: {error}");
			true
		});

		let scene_failed = if let Err(error) = self.save_project() {
			ze_log::error!("failed to save project scenes: {error:?}");
			true
		} else {
			false
		};

		if project_config_failed || scene_failed {
			self.mark_save_failed();
		} else {
			self.mark_save_succeeded();
		}
	}

	fn start_build_distributable(&mut self, output_dir: PathBuf) {
		if self.build_status == BuildStatus::Running {
			ze_log::warn!("build already running");
			return;
		}

		if self.active_project.is_none() {
			ze_log::warn!("cannot build distributable without a project loaded");
			return;
		}
		if let Err(error) = self.save_project_before_build() {
			self.build_status = BuildStatus::Failed;
			ze_log::error!("failed to save project before distributable build: {error:?}");
			return;
		}

		if let Some(project) = self.active_project.as_ref() {
			Self::append_build_path_to_gitignore(&output_dir, &project.root_dir);
		}

		let Some(project) = self.active_project.clone() else {
			ze_log::warn!("cannot build distributable without a project loaded");
			return;
		};
		self.script_hot_reload_build_pause = match self.asset_hot_reload.pause_script_watcher_for_build() {
			Ok(()) => ScriptHotReloadBuildPause::Paused,
			Err(error) => {
				self.build_status = BuildStatus::Failed;
				ze_log::error!("failed to pause script hot reload before distributable build: {error}");
				return;
			}
		};
		let options = build_distributable_options(output_dir, Some(&project));
		let (sender, receiver) = mpsc::channel();
		let thread = thread::Builder::new()
			.name("zeroeditor-build".to_string())
			.spawn(move || {
				let _ = sender.send(BuildEvent::Stage {
					message: "Building scripts...".to_string(),
				});

				let result = match ze_build::build_project_dist(&project, &options) {
					Ok(_) => EditorBuildResult::Succeeded,
					Err(error) => EditorBuildResult::Failed {
						error: format!("{error:?}"),
					},
				};
				let _ = sender.send(BuildEvent::Result(result));
			});

		match thread {
			Ok(_) => {
				self.build_status = BuildStatus::Running;
				self.build_receiver = Some(receiver);
			}
			Err(error) => {
				self.resume_script_hot_reload_after_build();
				self.build_status = BuildStatus::Failed;
				self.build_receiver = None;
				ze_log::error!("failed to start distributable build thread: {error}");
			}
		}
	}

	fn poll_build_worker(&mut self) {
		let egui_ctx = self.editor_flow.as_ref().map(|f| f.egui_ctx.clone());
		let Some(receiver) = self.build_receiver.take() else {
			return;
		};

		match receiver.try_recv() {
			Ok(BuildEvent::Stage { message }) => {
				self.build_message = Some(message);
				if let Some(ctx) = &egui_ctx {
					ctx.request_repaint();
				}
				self.build_receiver = Some(receiver);
			}
			Ok(BuildEvent::Result(EditorBuildResult::Succeeded)) => {
				self.resume_script_hot_reload_after_build();
				// Force immediate reload of the managed assembly so new C# script
				// classes are visible in the inspector's Add Component popup.
				if let Some(sm) = &mut self.scene_manager
					&& let Some(scene) = sm.active_scene_mut()
					&& let Some(project) = self.active_project.as_ref()
				{
					if let Err(error) =
						reload_managed_assembly(scene, &project.scripts_dll, &project.runtime_config_path())
					{
						ze_log::error!("failed to reload managed assembly after distributable build: {error:?}");
					} else if let Some(editor_flow) = &mut self.editor_flow {
						editor_flow.workspace.mark_script_classes_dirty();
					}
				}
				self.build_message = None;
				self.build_status = BuildStatus::Done;
			}
			Ok(BuildEvent::Result(EditorBuildResult::Failed { error })) => {
				self.resume_script_hot_reload_after_build();
				self.build_message = None;
				self.build_status = BuildStatus::Failed;
				ze_log::error!("failed to build distributable: {error}");
			}
			Err(TryRecvError::Empty) => {
				self.build_receiver = Some(receiver);
			}
			Err(TryRecvError::Disconnected) => {
				self.resume_script_hot_reload_after_build();
				self.build_message = None;
				self.build_status = BuildStatus::Failed;
				ze_log::error!("distributable build worker stopped before reporting a result");
			}
		}
	}

	fn resume_script_hot_reload_after_build(&mut self) {
		if self.script_hot_reload_build_pause != ScriptHotReloadBuildPause::Paused {
			return;
		}

		self.asset_hot_reload.resume_script_watcher_after_build();
		self.script_hot_reload_build_pause = ScriptHotReloadBuildPause::NotPaused;
	}

	fn append_build_path_to_gitignore(output_dir: &std::path::Path, project_root: &std::path::Path) {
		let normalized = output_dir.components().collect::<std::path::PathBuf>();
		let Ok(relative) = normalized.strip_prefix(project_root) else {
			return;
		};
		let Some(top) = relative.components().next() else {
			return;
		};
		let Some(top_str) = top.as_os_str().to_str() else {
			return;
		};
		let gitignore_path = project_root.join(".gitignore");
		let existing = std::fs::read_to_string(&gitignore_path).unwrap_or_default();
		let normalized_top = top_str.trim_matches('/');
		for line in existing.lines() {
			let trimmed = line.trim().trim_matches('/');
			if trimmed == normalized_top {
				return;
			}
		}
		let line = format!("/{top_str}/\n");
		let updated = if existing.is_empty() || existing.ends_with('\n') {
			format!("{existing}{line}")
		} else {
			format!("{existing}\n{line}")
		};
		if let Err(error) = std::fs::write(&gitignore_path, updated) {
			ze_log::warn!("failed to update .gitignore: {error:?}");
		}
	}

	fn save_project_before_build(&mut self) -> ze_core::Result<()> {
		self.mark_save_started();
		match self.save_project() {
			Ok(()) => {
				self.mark_save_succeeded();
				Ok(())
			}
			Err(error) => {
				self.mark_save_failed();
				Err(error)
			}
		}
	}

	fn sync_project_scene_list(&mut self) -> ze_core::Result<()> {
		let Some(project) = self.active_project.as_mut() else {
			return Ok(());
		};
		let mut changed = false;
		if !project.scenes.iter().any(|scene| scene == &project.main_scene) {
			let main_scene = project.main_scene.clone();
			changed |= project.add_scene(main_scene)?;
		}
		if changed {
			self.project_config_dirty = true;
		}
		Ok(())
	}

	fn sync_project_scene_files_if_due(&mut self) {
		if self.last_scene_file_scan.elapsed() < PROJECT_SCENE_SCAN_INTERVAL {
			return;
		}
		self.last_scene_file_scan = Instant::now();

		if let Err(error) = self.sync_project_scene_files() {
			ze_log::warn!("failed to sync project scenes from disk: {error:?}");
		}
	}

	fn sync_project_scene_files(&mut self) -> ze_core::Result<()> {
		let Some(project) = self.active_project.as_mut() else {
			return Ok(());
		};

		let scene_names = available_project_scene_names(project)?;

		let Some(scene_manager) = self.scene_manager.as_mut() else {
			return Ok(());
		};
		scene_manager.retain_project_scenes(project, &scene_names);
		scene_manager.load_missing_project_scenes(&self.resources, project, &scene_names)?;
		Ok(())
	}

	fn open_project_from_dialog(&mut self) {
		let Some(path) = rfd::FileDialog::new()
			.add_filter("ZeroEngine Project", &["toml"])
			.set_file_name(ze_project::PROJECT_FILE_NAME)
			.pick_file()
		else {
			return;
		};

		self.load_project(&path);
	}

	fn create_project_from_dialog(&mut self) {
		let Some(directory) = rfd::FileDialog::new().pick_folder() else {
			return;
		};
		let name = directory
			.file_name()
			.and_then(|name| name.to_str())
			.unwrap_or("ZeroEngineGame")
			.to_string();

		match Project::create_new(&directory, name) {
			Ok(project) => {
				let scene_path = project.asset_dir.join("scenes/Main.zescene.json");
				let scene_json = r#"{"version":"0.1.0","name":"Main","scene_type":"Scene","entities":[]}"#.to_string();
				if let Err(error) = std::fs::write(&scene_path, scene_json) {
					ze_log::warn!("failed to create default scene file: {error:?}");
				}
				match ze_build::build_scripts_assembly(&project, "Debug", None) {
					Ok(()) => {
						ze_log::debug!("initial C# script assembly built successfully");
					}
					Err(error) => {
						ze_log::warn!("initial C# script build failed: {error:?}");
					}
				}
				let scripts_dll = project.scripts_dll.clone();
				let runtime_config_path = project.runtime_config_path();
				self.apply_project(project);

				// Force the scripting runtime to reload from the freshly built
				// Scripts.dll so that any user-added script types (e.g. Test.cs)
				// are immediately discoverable in the Add Component dropdown.
				if let Some(scene) = self.scene_manager.as_mut().and_then(|m| m.active_scene_mut()) {
					if let Err(error) = reload_managed_assembly(scene, &scripts_dll, &runtime_config_path) {
						ze_log::warn!("initial C# script assembly refresh failed: {error:?}");
					} else {
						if let Some(editor_flow) = &mut self.editor_flow {
							editor_flow.workspace.mark_script_classes_dirty();
						}
						ze_log::debug!("initial C# script assembly loaded and discovery complete");
					}
				}
			}
			Err(error) => ze_log::error!("failed to create project in {}: {error:?}", directory.display()),
		}
	}

	fn ensure_workspace_config(root_dir: &Path) {
		let zeignore_path = root_dir.join(".zeignore");
		if !zeignore_path.exists() {
			let content = "/target\n/bin\n.zepack\n";
			if let Err(error) = std::fs::write(&zeignore_path, content) {
				ze_log::warn!("failed to create .zeignore: {error}");
			} else {
				ze_log::debug!("created .zeignore in project workspace");
			}
		}

		let gitignore_path = root_dir.join(".gitignore");
		if !gitignore_path.exists() {
			let content = "\
# ZeroEngine
/target
/bin
*.dll
*.pdb

# IDE
.idea/
.vs/
.vscode/
*.swp
*.swo

# OS
thumbs.db
.DS_Store
";
			if let Err(error) = std::fs::write(&gitignore_path, content) {
				ze_log::warn!("failed to create .gitignore: {error}");
			} else {
				ze_log::debug!("created .gitignore in project workspace");
			}
		}
	}

	fn load_project(&mut self, project_path: &std::path::Path) {
		match Project::load(project_path) {
			Ok(project) => self.apply_project(project),
			Err(error) => ze_log::error!("failed to load project {}: {error:?}", project_path.display()),
		}
	}

	fn apply_project(&mut self, project: Project) {
		Self::ensure_workspace_config(&project.root_dir);
		if let Err(error) = project.save_to_project_file() {
			ze_log::warn!("failed to persist project scene list: {error:?}");
		}
		self.resources = ResourceManager::new(project.asset_dir.clone());
		self.asset_hot_reload = create_asset_hot_reload(&self.resources, Some(&project));
		let scene_manager = match EditorSceneManager::load_project(&self.resources, &project) {
			Ok(manager) => manager,
			Err(error) => {
				ze_log::error!("failed to reload project scenes; opening empty editor scene: {error:?}");
				EditorSceneManager::from_scene(create_default_editor_scene(), true)
			}
		};
		if let Some(editor_flow) = &mut self.editor_flow {
			editor_flow.workspace = EditorWorkspace::new(
				editor_flow.viewport_texture_id,
				Some(project.asset_dir.clone()),
				editor_flow.window.clone(),
			);
			editor_flow.mark_scene_loaded();
		}
		self.scene_manager = Some(scene_manager);
		self.active_project = Some(project);
		self.project_config_dirty = false;
		self.last_scene_file_scan = Instant::now();
	}

	fn apply_saved_project(&mut self, project: Project) {
		self.resources = ResourceManager::new(project.asset_dir.clone());
		self.asset_hot_reload = create_asset_hot_reload(&self.resources, Some(&project));
		if let Some(editor_flow) = &mut self.editor_flow {
			editor_flow
				.workspace
				.set_project_assets_root(Some(project.asset_dir.clone()));
		}
		self.active_project = Some(project);
		self.project_config_dirty = false;
		self.last_scene_file_scan = Instant::now();
	}

	fn create_scene(&mut self, scene_name: &str) -> ze_core::Result<()> {
		if let Some(scene_manager) = self.scene_manager.as_ref()
			&& scene_manager.scenes.contains_key(scene_name)
		{
			bail!("scene `{scene_name}` is already loaded");
		}
		let project = self
			.active_project
			.as_mut()
			.ok_or_else(|| anyhow!("cannot create scene `{scene_name}` without a project loaded"))?;
		if project_scene_path(project, scene_name).exists() {
			bail!("scene `{scene_name}` already exists; open it instead");
		}
		let mut validation_project = project.clone();
		if !validation_project.add_scene(scene_name.to_string())? {
			bail!("scene `{scene_name}` is already registered in the project");
		}

		let scene_directory = project_scenes_dir(project);
		let scene = Scene::new(scene_name);
		save_editor_scene(&scene, &scene_directory, scene_name)
			.with_context(|| format!("failed to create scene file `{scene_name}`"))?;
		project.add_scene(scene_name.to_string())?;
		project.save_to_project_file()?;
		self.project_config_dirty = false;
		self.last_scene_file_scan = Instant::now();

		let mut scene = load_project_scene(&self.resources, Some(project), scene_name)?;
		ensure_editor_runtime_entities(&mut scene, Some(project));

		let scene_manager = self
			.scene_manager
			.as_mut()
			.ok_or_else(|| anyhow!("cannot create scene `{scene_name}` because no scene manager is loaded"))?;
		scene_manager.insert_scene(scene, true);
		Ok(())
	}

	fn create_and_open_scene(&mut self, scene_name: &str) {
		match self.create_scene(scene_name) {
			Ok(()) => self.switch_scene(scene_name),
			Err(error) => ze_log::error!("failed to create scene `{scene_name}`: {error:?}"),
		}
	}

	fn switch_scene(&mut self, scene_name: &str) {
		if let Some(scene_manager) = &mut self.scene_manager
			&& let Err(error) = scene_manager.set_active_scene(scene_name)
		{
			ze_log::error!("failed to switch active scene to `{scene_name}`: {error:?}");
			return;
		}

		// main_scene is only changed by the explicit "Set as Main Scene" UI action.
		self.project_config_dirty = false;
		self.last_scene_file_scan = Instant::now();
	}

	fn enter_play_mode(&mut self) {
		let Some(scene_manager) = self.scene_manager.as_mut() else {
			ze_log::warn!("cannot enter Play mode because no scene manager is loaded");
			return;
		};

		// Validate that the active scene has a primary camera before entering play
		// mode.
		if let Some(scene) = scene_manager.active_scene_mut() {
			let camera_missing = scene
				.world()
				.borrow::<ze_ecs::View<Camera>>()
				.is_ok_and(|cameras| cameras.iter().next().is_none());
			if camera_missing {
				let _ = rfd::MessageDialog::new()
					.set_title("Missing Primary Camera")
					.set_description("Cannot start simulation: Current scene has no Primary Camera component!")
					.set_level(rfd::MessageLevel::Warning)
					.show();
				return;
			}
		}

		if let Err(error) = scene_manager.enter_play_mode() {
			ze_log::error!("failed to enter Play mode: {error:?}");
			return;
		}
		if let Some(editor_flow) = &mut self.editor_flow {
			editor_flow.workspace.set_mode(editor_workspace::EditorMode::Play);
		}
	}

	fn stop_play_mode(&mut self) {
		let Some(scene_manager) = self.scene_manager.as_mut() else {
			ze_log::warn!("cannot stop Play mode because no scene manager is loaded");
			return;
		};
		if let Err(error) = scene_manager.stop_play_mode() {
			ze_log::error!("failed to stop Play mode: {error:?}");
		}
		if let Some(editor_flow) = &mut self.editor_flow {
			editor_flow.workspace.clear_selection();
			editor_flow.workspace.set_mode(editor_workspace::EditorMode::Edit);
		}
	}

	const fn mark_save_started(&mut self) {
		if let Some(editor_flow) = &mut self.editor_flow {
			editor_flow.mark_save_started();
		}
	}

	fn mark_save_succeeded(&mut self) {
		if let Some(editor_flow) = &mut self.editor_flow {
			editor_flow.mark_save_succeeded();
		}
	}

	fn mark_project_config_saved(&mut self) {
		if let Some(editor_flow) = &mut self.editor_flow {
			editor_flow.mark_project_config_saved();
		}
	}

	const fn mark_save_failed(&mut self) {
		if let Some(editor_flow) = &mut self.editor_flow {
			editor_flow.mark_save_failed();
		}
	}
}

fn update_editor_scene_before_render(
	scene_manager: &mut EditorSceneManager,
	resources: &ResourceManager,
	active_project: Option<&Project>,
	asset_hot_reload: &mut AssetHotReload,
	renderer: &mut Renderer,
	game_running: bool,
	dt: f32,
) -> bool {
	let Some(scene) = scene_manager.active_scene_mut() else {
		ze_log::error!("cannot update editor because no active scene is loaded");
		return false;
	};

	let asset_reload = asset_hot_reload.poll(scene, game_running);
	renderer.invalidate_textures(asset_reload.texture_assets());

	if game_running {
		if let Err(error) = scene.update_system::<ScriptingSystem>(dt) {
			ze_log::error!("failed to update scripting system: {error:?}");
		}
		if let Err(error) = scene.update_system::<PhysicsSystem>(dt) {
			ze_log::error!("failed to update physics system: {error:?}");
		}
		if scene_manager.apply_scripting_scene_load_commands(resources, active_project)
			&& let Some(scene) = scene_manager.active_scene_mut()
		{
			editor_workspace::switch_viewport_camera(scene, editor_workspace::EditorMode::Play);
		}
	}

	true
}

fn reset_scene_runtime_systems(scene: &mut Scene) {
	scene.with_system_mut::<PhysicsSystem, _>(|physics, _scene| physics.reset());
	scene.with_system_mut::<ScriptingSystem, _>(|scripting, _scene| scripting.reset());
}

fn project_scenes_dir(project: &Project) -> PathBuf { project.asset_dir.join("scenes") }

fn available_project_scene_names(project: &Project) -> ze_core::Result<Vec<String>> {
	let mut scene_names = project.scene_names_from_disk()?;
	scene_names.extend(project.scenes.iter().cloned());
	if !scene_names.iter().any(|scene| scene == &project.main_scene) {
		scene_names.push(project.main_scene.clone());
	}
	scene_names.sort();
	scene_names.dedup();
	Ok(scene_names)
}

fn project_scene_path(project: &Project, scene_name: &str) -> PathBuf {
	project_scenes_dir(project).join(format!("{scene_name}.zescene.json"))
}

fn project_scene_path_if_loaded(project: Option<&Project>, scene_name: &str) -> Option<PathBuf> {
	project.map(|project| project_scene_path(project, scene_name))
}

fn ensure_editor_runtime_entities(scene: &mut Scene, project: Option<&Project>) {
	ensure_editor_camera(scene);
	ensure_editor_physics_settings(scene, project);
	editor_workspace::switch_viewport_camera(scene, editor_workspace::EditorMode::Edit);
}

fn ensure_editor_camera(scene: &mut Scene) {
	let editor_camera = editor_camera_entity(scene).unwrap_or_else(|| scene.create_entity(EDITOR_CAMERA_NAME));
	ensure_editor_only_marker(scene, editor_camera);
	if scene.world().get::<&Tag>(editor_camera).is_err() {
		scene.entity_mut(editor_camera).add_component(Tag {
			tag: EDITOR_CAMERA_NAME.to_string(),
		});
	}
	if scene.world().get::<&Camera>(editor_camera).is_err() {
		scene.entity_mut(editor_camera).add_component(default_editor_camera());
	}
	if scene.world().get::<&Transform>(editor_camera).is_err() {
		scene
			.entity_mut(editor_camera)
			.add_component(default_editor_camera_transform());
	}
}

fn editor_camera_entity(scene: &Scene) -> Option<EntityId> {
	let world = scene.world();
	let mut marked_camera = None;
	let mut named_candidate = None;

	world.run(|entities: ze_ecs::EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&Camera>(entity).is_ok() && world.get::<&EditorOnly>(entity).is_ok() {
				marked_camera = Some(entity);
				break;
			}

			if named_candidate.is_none() && is_editor_camera_named_entity(scene, entity) {
				named_candidate = Some(entity);
			}
		}
	});

	marked_camera.or(named_candidate)
}

fn is_editor_camera_named_entity(scene: &Scene, entity: EntityId) -> bool {
	let world = scene.world();
	world
		.get::<&Name>(entity)
		.is_ok_and(|name| name.name == EDITOR_CAMERA_NAME)
		|| world.get::<&Tag>(entity).is_ok_and(|tag| tag.tag == EDITOR_CAMERA_NAME)
}

const fn default_editor_camera() -> Camera {
	Camera {
		projection: CameraProjection::Orthographic {
			size: 17.411_005,
			near: -1000.0,
			far: 1000.0,
		},
		primary: true,
		clear_color: [0.12, 0.12, 0.13, 1.0],
	}
}

const fn default_editor_camera_transform() -> Transform {
	Transform {
		position: Vec3::new(-1.395_359_5, -0.582_343_5, 5.0),
		rotation: Quat::IDENTITY,
		scale: Vec3::ONE,
	}
}

fn ensure_editor_only_marker(scene: &mut Scene, entity: EntityId) {
	if scene.world().get::<&EditorOnly>(entity).is_err() {
		scene.entity_mut(entity).add_component(EditorOnly);
	}
}

fn ensure_editor_physics_settings(scene: &mut Scene, project: Option<&Project>) {
	if persistent_physics_settings_entity(scene).is_some() {
		return;
	}

	let physics_settings = project.map_or_else(PhysicsSettings::default, |project| PhysicsSettings {
		gravity: ze_core::Vec2::new(project.physics.gravity.x, project.physics.gravity.y),
		enable_debug_draw: project.physics.enable_debug_draw,
		physics_timestep: project.physics.timestep,
	});

	let entity = physics_settings_entity(scene).unwrap_or_else(|| scene.create_entity(PHYSICS_SETTINGS_NAME));
	ensure_editor_only_marker(scene, entity);
	if scene.world().get::<&Tag>(entity).is_err() {
		scene.entity_mut(entity).add_component(Tag {
			tag: PHYSICS_SETTINGS_NAME.to_string(),
		});
	}
	scene.entity_mut(entity).add_component(physics_settings);
}

fn physics_settings_entity(scene: &Scene) -> Option<EntityId> {
	let world = scene.world();
	let mut found = None;
	world.run(|entities: ze_ecs::EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&PhysicsSettings>(entity).is_ok() {
				found = Some(entity);
				break;
			}
		}
	});
	found
}

fn persistent_physics_settings_entity(scene: &Scene) -> Option<EntityId> {
	let world = scene.world();
	let mut found = None;
	world.run(|entities: ze_ecs::EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&PhysicsSettings>(entity).is_ok() && world.get::<&EditorOnly>(entity).is_err() {
				found = Some(entity);
				break;
			}
		}
	});
	found
}

fn create_asset_hot_reload(resources: &ResourceManager, active_project: Option<&Project>) -> AssetHotReload {
	if let Some(project) = active_project
		&& let Err(error) = project.generate_csproj()
	{
		ze_log::error!("failed to generate csproj: {error}");
	}

	let script_library_path = active_project.map_or_else(
		|| resources.game_assets_root().join("bin/Scripts.dll"),
		|project| project.scripts_dll.clone(),
	);
	let runtime_config_path = active_project.map_or_else(
		|| script_library_path.with_file_name("Scripts.runtimeconfig.json"),
		Project::runtime_config_path,
	);
	let csproj_path = active_project.and_then(|project| project.csproj_path().ok());

	AssetHotReload::new(
		resources.game_assets_root(),
		script_library_path,
		runtime_config_path,
		csproj_path.as_deref(),
	)
}

fn build_distributable_options(output_dir: PathBuf, project: Option<&ze_project::Project>) -> ze_build::BuildOptions {
	let notice_path = resolve_notice_path(project);
	let api_source_dir = project.and_then(resolve_api_source_dir);
	// Match the .NET configuration to the Cargo build profile so script builds
	// use the same optimization level as the editor (Debug during development,
	// Release when doing a release build of the editor).
	let is_release = option_env!("PROFILE").is_some_and(|p| p == "release");
	let mut options = if is_release {
		ze_build::BuildOptions::release(output_dir, notice_path)
	} else {
		ze_build::BuildOptions::debug(output_dir, notice_path)
	};
	if let Some(api_dir) = api_source_dir {
		options = options.with_api_source_dir(api_dir);
	}
	options
}

fn resolve_notice_path(project: Option<&ze_project::Project>) -> PathBuf {
	let candidates: [Option<PathBuf>; 4] = [
		project.map(|p| p.root_dir.clone()),
		std::env::current_dir().ok(),
		std::env::current_exe()
			.ok()
			.and_then(|exe| exe.parent().map(Path::to_path_buf)),
		ze_core::resolve_engine_root().ok(),
	];
	for start in candidates.iter().flatten() {
		let path = start.join("NOTICE");
		if path.is_file() {
			return path;
		}
	}
	// Fall back to current_dir/NOTICE for the error message if it does not exist.
	std::env::current_dir().map_or_else(|_| PathBuf::from("NOTICE"), |dir| dir.join("NOTICE"))
}

fn resolve_api_source_dir(project: &ze_project::Project) -> Option<PathBuf> {
	// Try dev workspace layout: engine root / ZeroEngine/api/cs
	if let Ok(engine_root) = ze_core::resolve_engine_root() {
		let api_dir_dev = engine_root.join("ZeroEngine/api/cs");
		if api_dir_dev.is_dir() {
			return Some(api_dir_dev);
		}
	}
	// Try project-root-relative api/cs
	let project_api = project.root_dir.join("ZeroEngine/api/cs");
	if project_api.is_dir() {
		return Some(project_api);
	}
	// Try current directory
	if let Ok(cwd) = std::env::current_dir() {
		let cwd_api = cwd.join("ZeroEngine/api/cs");
		if cwd_api.is_dir() {
			return Some(cwd_api);
		}
	}
	// Try bundled api/cs next to the executable (dist layout)
	if let Ok(exe) = std::env::current_exe()
		&& let Some(exe_dir) = exe.parent()
	{
		let bundled_api = exe_dir.join("api/cs");
		if bundled_api.is_dir() {
			return Some(bundled_api);
		}
	}
	None
}

fn outline_text_shapes(shapes: &mut [ClippedShape]) {
	for clipped_shape in shapes {
		let shape = std::mem::replace(&mut clipped_shape.shape, Shape::Noop);
		clipped_shape.shape = outlined_shape(shape);
	}
}

fn outlined_shape(shape: Shape) -> Shape {
	match shape {
		Shape::Text(text) => {
			let mut shapes = Vec::with_capacity(TEXT_OUTLINE_OFFSETS.len() + 1);
			for offset in TEXT_OUTLINE_OFFSETS {
				let mut outline = text.clone();
				outline.pos += egui::vec2(offset[0], offset[1]);
				outline.fallback_color = egui::Color32::BLACK;
				outline.override_text_color = Some(egui::Color32::BLACK);
				shapes.push(Shape::Text(outline));
			}
			shapes.push(Shape::Text(text));
			Shape::Vec(shapes)
		}
		Shape::Vec(shapes) => Shape::Vec(shapes.into_iter().map(outlined_shape).collect()),
		other => other,
	}
}

const TEXT_OUTLINE_OFFSETS: [[f32; 2]; 4] = [[0.0, -1.0], [-1.0, 0.0], [1.0, 0.0], [0.0, 1.0]];

fn scene_fingerprint(scene: &Scene) -> String { format!("{:?}", editor_persistent_snapshot(scene)) }

fn editor_persistent_snapshot(scene: &Scene) -> SaveFile {
	let mut snapshot = scene.snapshot();
	snapshot.entities.retain(|entity| {
		!entity
			.components
			.iter()
			.any(|component| component.component_type == EDITOR_ONLY_COMPONENT_TYPE)
	});
	snapshot
}

fn save_editor_scene(scene: &Scene, directory: &PathBuf, file_name: &str) -> ze_core::Result<()> {
	std::fs::create_dir_all(directory)?;
	let path = directory.join(format!("{file_name}.zescene.json"));
	let save_file = editor_persistent_snapshot(scene);
	let json = serde_json::to_string_pretty(&save_file)?;
	std::fs::write(path, json)?;
	Ok(())
}

fn main() {
	let event_loop = match EventLoop::new() {
		Ok(event_loop) => event_loop,
		Err(error) => {
			ze_log::error!("failed to create event loop: {error}");
			return;
		}
	};
	let active_project = std::env::args().nth(1).map_or_else(
		|| None,
		|project_path| match Project::load(&project_path) {
			Ok(project) => Some(project),
			Err(error) => {
				ze_log::warn!(
					"failed to load project {} at startup; opening editor without a project: {error:?}",
					project_path
				);
				None
			}
		},
	);

	let mut app = EditorApp::new(active_project);
	if let Err(error) = event_loop.run_app(&mut app) {
		ze_log::error!("failed to run editor app: {error}");
	}
}

fn create_default_editor_scene() -> Scene {
	let mut registry = ze_ecs::ComponentRegistry::new();
	Scene::register_defaults(&mut registry);
	ze_renderer::register_renderer_components(&mut registry);

	let mut scene = Scene::from_registry("Untitled", registry);
	ensure_editor_runtime_entities(&mut scene, None);
	scene.add_system(ze_renderer::EditorCameraSystem::new());
	scene.add_system(RenderSystem::new());
	scene
}

fn forward_game_input(event: &WindowEvent) -> bool {
	match event {
		WindowEvent::KeyboardInput { event, .. } => {
			if let PhysicalKey::Code(code) = event.physical_key {
				ze_input::Input::update_globally(|input| {
					input.set_key(ze_input::ZKeyCode::from(code), event.state == ElementState::Pressed);
				});
			}
			true
		}
		WindowEvent::MouseInput { state, button, .. } => {
			ze_input::Input::update_globally(|input| {
				input.set_mouse_button(ze_input::ZMouseCode::from(*button), *state == ElementState::Pressed);
			});
			true
		}
		WindowEvent::CursorMoved { position, .. } => {
			ze_input::Input::update_globally(|input| {
				input.set_mouse_pos(position.x as f32, position.y as f32);
			});
			true
		}
		WindowEvent::MouseWheel { delta, .. } => {
			let wheel_delta = match delta {
				winit::event::MouseScrollDelta::LineDelta(_, y) => *y,
				winit::event::MouseScrollDelta::PixelDelta(position) => position.y as f32,
			};
			ze_input::Input::update_globally(|input| {
				input.add_mouse_wheel_delta(wheel_delta);
			});
			true
		}
		WindowEvent::CursorLeft { .. } => {
			ze_input::Input::update_globally(ze_input::Input::reset);
			true
		}
		_ => false,
	}
}

const fn should_forward_game_input(event: &WindowEvent, game_input_active: bool, egui_consumed: bool) -> bool {
	if !game_input_active {
		return false;
	}

	if matches!(event, WindowEvent::KeyboardInput { .. }) {
		return true;
	}

	!egui_consumed
}

#[cfg(test)]
mod tests {
	use std::fs;

	use ze_project::PhysicsSettings as ProjectPhysicsSettings;

	use super::*;

	#[test]
	fn save_all_dirty_scenes_writes_inactive_loaded_scenes() -> ze_core::Result<()> {
		let root_dir = std::env::temp_dir().join(format!("zeroeditor-save-all-scenes-{}", std::process::id()));
		if root_dir.exists() {
			fs::remove_dir_all(&root_dir)?;
		}
		fs::create_dir_all(root_dir.join("assets").join("scenes"))?;

		let project = Project {
			name: "SaveAllScenes".to_string(),
			main_scene: "First".to_string(),
			scenes: vec!["First".to_string(), "Second".to_string()],
			asset_dir: root_dir.join("assets"),
			scripts_dll: root_dir
				.join("assets")
				.join("scripts")
				.join("bin")
				.join("SaveAllScenes.dll"),
			physics: ProjectPhysicsSettings::default(),
			game: ze_project::GameSettings::default(),
			root_dir: root_dir.clone(),
		};

		let mut manager = EditorSceneManager::from_scene(Scene::new("First"), true);
		manager.insert_scene(Scene::new("Second"), true);
		manager.active_scene = "First".to_string();

		let first_scene = &mut manager
			.scenes
			.get_mut("First")
			.expect("first scene should be loaded")
			.scene;
		let first_entity = first_scene.create_entity("FirstDirty");
		first_scene.entity_mut(first_entity).add_component(Tag {
			tag: "FirstDirty".to_string(),
		});

		let second_scene = &mut manager
			.scenes
			.get_mut("Second")
			.expect("second scene should be loaded")
			.scene;
		let second_entity = second_scene.create_entity("SecondDirty");
		second_scene.entity_mut(second_entity).add_component(Tag {
			tag: "SecondDirty".to_string(),
		});

		let saved = manager.save_all_dirty_scenes(&project)?;
		assert_eq!(saved, 2);
		assert!(fs::read_to_string(project_scene_path(&project, "First"))?.contains("FirstDirty"));
		assert!(fs::read_to_string(project_scene_path(&project, "Second"))?.contains("SecondDirty"));
		assert_eq!(manager.save_all_dirty_scenes(&project)?, 0);

		fs::remove_dir_all(root_dir)?;
		Ok(())
	}
}
