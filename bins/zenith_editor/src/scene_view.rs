//! What the Viewport shows: the open world drawn by the renderer into a
//! texture, seen by the camera of the editor, with the grid, the gizmos of
//! lights and cameras, and the outline and transform gizmo of what is
//! selected.
//!
//! The renderer is made once and lives in the world that is shown (it is a
//! resource of it). Showing another world moves it there, with its cache of
//! loaded assets.

use std::{path::PathBuf, sync::Arc};

use bevy_ecs::{entity::Entity, schedule::Schedule};
use dear_imgui_wgpu::ExternalTextureId;
use glam::{DVec3, Mat4, Vec3};
use zenith_components::{Name, Transform};
use zenith_filesystem::{ChangeKind, Project, ProjectEvent, WatchConfig, WatchRule, Watcher, ZPath};
use zenith_renderer::{
	Installed, RenderSettings, Renderer,
	assets::AssetRegistry,
	cache::GpuCache,
	components::{Camera, ClearMode, DirectionalLight, PointLight, Projection},
	extract::{ExtraViews, ExtractedView, ViewSource, ViewTarget},
	graph::GraphId,
	install, install_again, install_project,
	passes::editor::{EditorSelection, GizmoLines},
	prepare::ReloadShaders,
	shaders::project::install_engine_shaders,
	targets::{RenderTargetId, RenderTargets},
	uninstall, wgpu,
};

use crate::{camera::EditorCamera, gfx::Gfx, project::OpenProject};

/// The format the world is drawn in: what the UI shows is sRGB.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// The number of the editor's view among the views of the host.
const EDITOR_VIEW: u32 = 1_000_000;
const CLEAR: [f32; 4] = [0.1, 0.1, 0.12, 1.0];
const LIGHT_COLOR: [f32; 4] = [1.0, 0.9, 0.3, 1.0];
const CAMERA_COLOR: [f32; 4] = [0.8, 0.8, 0.8, 1.0];
/// The size a gizmo of a light or a camera has when it is not selected, in
/// units.
const ICON_SIZE: f64 = 0.3;

/// What the transform gizmo does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GizmoOperation {
	#[default]
	Move,
	Rotate,
	Scale,
}

/// The state of the Viewport.
pub struct SceneView {
	pub camera: EditorCamera,
	pub selection: Option<Entity>,
	pub operation: GizmoOperation,
	/// The gizmo works in the axes of the entity, not of the world.
	pub local: bool,
	/// The right mouse button is held on the picture: the camera flies.
	pub flying: bool,
	/// The size of the panel in pixels, set while it is drawn.
	pub wanted: (u32, u32),
	/// What the UI draws; `None` until there is a world to draw.
	pub texture: Option<ExternalTextureId>,
	/// The renderer while no world holds it.
	free: Option<Installed>,
	/// The world that holds the renderer.
	home: Option<String>,
	schedule: Schedule,
	target: Option<RenderTargetId>,
	texture_size: (u32, u32),
	/// The project the watcher and the engine's shaders were set up for.
	prepared_for: Option<PathBuf>,
	watcher: Option<Watcher>,
}

impl Default for SceneView {
	fn default() -> Self {
		Self {
			camera: EditorCamera::default(),
			selection: None,
			operation: GizmoOperation::default(),
			local: false,
			flying: false,
			wanted: (1, 1),
			texture: None,
			free: None,
			home: None,
			schedule: zenith_renderer::schedule(),
			target: None,
			texture_size: (0, 0),
			prepared_for: None,
			watcher: None,
		}
	}
}

/// The texture the world was drawn into.
struct Shown {
	view: wgpu::TextureView,
	/// It is not the one the UI was told about.
	resized: bool,
}

/// What changed in the project that the renderer has to know.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Changes {
	pub shaders: bool,
	/// Assets that were written over.
	pub assets: Vec<ZPath>,
	/// Assets that came or went: the list of them is read again.
	pub rescan: bool,
}

impl Changes {
	/// Sorts a change of a file of the project.
	pub fn add(&mut self, kind: &ChangeKind, path: &ZPath, rules: &[String]) {
		let is = |name: &str| rules.iter().any(|rule| rule == name);
		if is("shaders") {
			self.shaders = true;
		}
		if is("assets") {
			match kind {
				ChangeKind::Modified => self.assets.push(path.clone()),
				ChangeKind::Created | ChangeKind::Removed | ChangeKind::Renamed { .. } => self.rescan = true,
			}
		}
	}
}

fn watch_rules() -> Vec<WatchRule> {
	vec![
		WatchRule {
			name: "shaders".to_string(),
			patterns: vec!["Assets/Shaders/**".to_string()],
			exclude: Vec::new(),
		},
		WatchRule {
			name: "assets".to_string(),
			patterns: vec!["Assets/**".to_string()],
			exclude: vec!["Assets/Shaders/**".to_string(), "Assets/Worlds/**".to_string()],
		},
	]
}

impl SceneView {
	/// Takes the renderer out of the world that holds it, before the project
	/// is closed (with its worlds).
	pub fn release(&mut self, project: Option<&mut OpenProject>) {
		if let (Some(home), Some(project)) = (self.home.take(), project)
			&& let Some(entry) = project.worlds.get_mut(&home)
		{
			self.free = uninstall(&mut entry.world);
		}
		self.selection = None;
		self.prepared_for = None;
		self.watcher = None;
	}

	/// Draws the world `name` of `project` into the texture of the Viewport,
	/// and lets the UI see it.
	pub fn render(&mut self, gfx: &mut Gfx, project: &mut OpenProject, name: &str) {
		let Some(shown) = self.draw(gfx.device(), gfx.queue(), project, name) else {
			return;
		};
		if shown.resized || self.texture.is_none() {
			match self.texture {
				Some(texture) => gfx.update_texture(texture, &shown.view),
				None => self.texture = gfx.register_texture(&shown.view),
			}
		}
	}

	/// Draws the world into the texture, and says which it is. `None`: there
	/// is no such world.
	fn draw(
		&mut self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		project: &mut OpenProject,
		name: &str,
	) -> Option<Shown> {
		if !self.hold(device, queue, project, name) {
			return None;
		}
		self.prepare_project(project);
		let changes = self.changes();
		let entry = project.worlds.get_mut(name)?;
		let world = &mut entry.world;
		if self.selection.is_some_and(|entity| !world.entity_exists(entity)) {
			self.selection = None;
		}
		apply_changes(world, &changes, &project.project);

		let size = self.size_for(device);
		let (id, view) = {
			let mut targets = world.world_mut().resource_mut::<RenderTargets>();
			let id = *self.target.get_or_insert_with(|| targets.create(device, size, FORMAT));
			targets.resize(device, id, size);
			(id, targets.get(id)?.view.clone())
		};
		let resized = size != self.texture_size;
		self.texture_size = size;

		let ecs = world.world_mut();
		ecs.resource_mut::<ExtraViews>().0 = vec![ExtractedView {
			source: ViewSource::Host(EDITOR_VIEW),
			position: self.camera.position,
			rotation: self.camera.rotation(),
			projection: self.camera.projection,
			target: ViewTarget::Target(id),
			viewport: None,
			order: 0,
			clear: ClearMode::Color(CLEAR),
			graph: Some(GraphId::EDITOR),
			environment: None,
		}];
		ecs.resource_mut::<EditorSelection>().0 = self.selection.into_iter().collect();
		let mut lines = GizmoLines::default();
		gizmo_lines(ecs, self.selection, &mut lines);
		*ecs.resource_mut::<GizmoLines>() = lines;
		self.schedule.run(ecs);
		Some(Shown { view, resized })
	}

	/// Makes sure the renderer is in the world `name`. `false`: there is no
	/// such world.
	fn hold(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, project: &mut OpenProject, name: &str) -> bool {
		if self.home.as_deref() == Some(name) && project.worlds.contains_key(name) {
			return true;
		}
		if !project.worlds.contains_key(name) {
			return false;
		}
		if let Some(old) = self.home.take()
			&& let Some(entry) = project.worlds.get_mut(&old)
		{
			self.free = uninstall(&mut entry.world);
		}
		let Some(entry) = project.worlds.get_mut(name) else {
			return false;
		};
		match self.free.take() {
			Some(installed) => install_again(&mut entry.world, installed),
			None => install(
				&mut entry.world,
				Renderer::from_device(device.clone(), queue.clone(), FORMAT),
				RenderSettings::default(),
			),
		}
		if let Err(e) = install_project(&mut entry.world, Arc::clone(&project.project)) {
			zenith_log::warn!("The assets of the project cannot be listed for drawing: {e}");
		}
		entry.world.world_mut().resource_mut::<ReloadShaders>().0 = true;
		// A schedule is made for one world: another one needs its own.
		self.schedule = zenith_renderer::schedule();
		self.home = Some(name.to_string());
		self.selection = None;
		true
	}

	/// The first time for a project: its shaders are written, and its files
	/// are watched.
	fn prepare_project(&mut self, project: &OpenProject) {
		if self.prepared_for.as_deref() == Some(project.path.as_path()) {
			return;
		}
		self.prepared_for = Some(project.path.clone());
		match install_engine_shaders(&project.project) {
			Ok(report) => {
				if !report.written.is_empty() {
					zenith_log::info!(
						"Wrote the engine's shaders into the project: {}",
						report.written.join(", ")
					);
				}
				for name in &report.newer_in_engine {
					zenith_log::info!("The engine has a newer version of the shader {name} than the project");
				}
			}
			Err(e) => zenith_log::warn!("The engine's shaders cannot be written into the project: {e}"),
		}
		self.watcher = match project.project.watch(&WatchConfig::Manual(watch_rules())) {
			Ok(watcher) => Some(watcher),
			Err(e) => {
				zenith_log::warn!("The files of the project are not watched: {e}");
				None
			}
		};
	}

	/// What changed in the project since the last frame.
	fn changes(&self) -> Changes {
		let mut changes = Changes::default();
		let Some(watcher) = &self.watcher else {
			return changes;
		};
		for event in watcher.events().try_iter() {
			match event {
				ProjectEvent::Change(change) => changes.add(&change.kind, &change.path, &change.rules),
				ProjectEvent::Rescan => {
					changes.shaders = true;
					changes.rescan = true;
				}
			}
		}
		changes
	}

	/// The size of the panel, in what the GPU can make.
	fn size_for(&self, device: &wgpu::Device) -> (u32, u32) {
		let max = device.limits().max_texture_dimension_2d;
		(self.wanted.0.clamp(1, max), self.wanted.1.clamp(1, max))
	}

	/// The model matrix of the selected entity for the gizmo, relative to the
	/// camera, and the size of what it is drawn over.
	pub fn selected_matrix(&self, project: &OpenProject, name: &str) -> Option<Mat4> {
		let entity = self.selection?;
		let world = &project.worlds.get(name)?.world;
		let transform = world.get_component::<Transform>(entity)?;
		Some(Mat4::from_scale_rotation_translation(
			transform.scale.as_vec3(),
			transform.orientation,
			(transform.position - self.camera.position).as_vec3(),
		))
	}

	/// The gizmo moved the entity: `model` is its new matrix (relative to the
	/// camera). Marks the world as changed.
	pub fn apply_matrix(&self, project: &mut OpenProject, name: &str, model: Mat4) {
		let Some(entity) = self.selection else { return };
		let Some(entry) = project.worlds.get_mut(name) else {
			return;
		};
		let (scale, orientation, translation) = model.to_scale_rotation_translation();
		let Some(mut transform) = entry.world.get_component_mut::<Transform>(entity) else {
			return;
		};
		transform.position = self.camera.position + translation.as_dvec3();
		transform.orientation = orientation.normalize();
		transform.scale = scale.as_dvec3();
		entry.dirty = true;
	}
}

/// Tells the renderer of the world what changed in the project.
fn apply_changes(world: &mut zenith_world::World, changes: &Changes, project: &Arc<Project>) {
	if changes.rescan {
		if let Err(e) = install_project(world, Arc::clone(project)) {
			zenith_log::warn!("The assets of the project cannot be listed again: {e}");
		}
		// A file that came back may be an asset that was dropped for being
		// broken: ask for the ones that failed again.
		world.world_mut().resource_mut::<GpuCache>().reload_failed();
	}
	if changes.shaders {
		world.world_mut().resource_mut::<ReloadShaders>().0 = true;
	}
	if !changes.assets.is_empty() {
		let ids: Vec<_> = {
			let registry = world.world().resource::<AssetRegistry>();
			changes.assets.iter().filter_map(|path| registry.id_of(path)).collect()
		};
		let mut cache = world.world_mut().resource_mut::<GpuCache>();
		for id in ids {
			cache.invalidate(id);
		}
	}
}

/// The entities of a world, for the Hierarchy: those with a place.
pub fn entities(world: &mut zenith_world::World) -> Vec<(Entity, String)> {
	let ecs = world.world_mut();
	let mut found: Vec<(Entity, String)> = ecs
		.query::<(Entity, Option<&Name>, &Transform)>()
		.iter(ecs)
		.map(|(entity, name, _)| {
			let label = name.map_or_else(|| format!("Entity {}", entity.index()), |name| name.name.clone());
			(entity, label)
		})
		.collect();
	found.sort_by_key(|(entity, _)| entity.index());
	found
}

/// The lines for the lights and cameras of the world: a small mark each, and
/// the reach of the one that is selected.
pub fn gizmo_lines(world: &mut bevy_ecs::world::World, selected: Option<Entity>, lines: &mut GizmoLines) {
	type Row<'a> = (
		Entity,
		&'a Transform,
		Option<&'a DirectionalLight>,
		Option<&'a PointLight>,
		Option<&'a Camera>,
	);
	let mut query = world.query::<Row<'_>>();
	for (entity, transform, directional, point, camera) in query.iter(world) {
		let (position, rotation) = (transform.position, transform.orientation);
		let is_selected = selected == Some(entity);
		if directional.is_some() {
			cross(lines, position, ICON_SIZE, LIGHT_COLOR);
			let forward = (rotation * Vec3::NEG_Z).as_dvec3();
			lines.line(position, position + forward * 2.0, LIGHT_COLOR);
			let side = (rotation * Vec3::X).as_dvec3();
			for offset in [side * 0.25, -side * 0.25] {
				lines.line(position + offset, position + offset + forward * 2.0, LIGHT_COLOR);
			}
		}
		if let Some(light) = point {
			cross(lines, position, ICON_SIZE, LIGHT_COLOR);
			if is_selected {
				let range = f64::from(light.range);
				lines.circle(position, DVec3::X, DVec3::Y, range, LIGHT_COLOR);
				lines.circle(position, DVec3::Y, DVec3::Z, range, LIGHT_COLOR);
				lines.circle(position, DVec3::Z, DVec3::X, range, LIGHT_COLOR);
			}
		}
		if let Some(camera) = camera {
			camera_frustum(lines, position, rotation, &camera.projection);
		}
	}
}

/// Three lines through `center`, along the axes.
fn cross(lines: &mut GizmoLines, center: DVec3, size: f64, color: [f32; 4]) {
	for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
		lines.line(center - axis * size, center + axis * size, color);
	}
}

/// The shape of what a camera sees, a few units long.
fn camera_frustum(lines: &mut GizmoLines, position: DVec3, rotation: glam::Quat, projection: &Projection) {
	const LENGTH: f32 = 2.0;
	const ASPECT: f32 = 16.0 / 9.0;
	let (half_height, apex) = match *projection {
		Projection::Perspective { fov_y, .. } => ((fov_y * 0.5).tan() * LENGTH, true),
		Projection::Orthographic { height, .. } => (height * 0.5, false),
	};
	let half_width = half_height * ASPECT;
	let at = |x: f32, y: f32, z: f32| position + (rotation * Vec3::new(x, y, z)).as_dvec3();
	let far = [
		at(-half_width, -half_height, -LENGTH),
		at(half_width, -half_height, -LENGTH),
		at(half_width, half_height, -LENGTH),
		at(-half_width, half_height, -LENGTH),
	];
	for i in 0..4 {
		lines.line(far[i], far[(i + 1) % 4], CAMERA_COLOR);
		let from = if apex {
			position
		} else {
			// Straight sides: the same corner on the near plane.
			at(
				[-half_width, half_width, half_width, -half_width][i],
				[-half_height, -half_height, half_height, half_height][i],
				0.0,
			)
		};
		lines.line(from, far[i], CAMERA_COLOR);
	}
}

#[cfg(test)]
mod tests {
	use bevy_ecs::world::World;
	use zenith_filesystem::Project;

	use super::*;

	type TestResult = Result<(), Box<dyn std::error::Error>>;

	fn zpath(text: &str) -> ZPath {
		// A path that is wrong in a test is a failure of it.
		#[allow(clippy::panic)]
		ZPath::new(text).unwrap_or_else(|e| panic!("{text}: {e}"))
	}

	/// A GPU without a window; `None` where there is no adapter.
	fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
		let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
		pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
			label: Some("test device"),
			required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
			..Default::default()
		}))
		.ok()
	}

	/// A project with the worlds `A` and `B`.
	fn two_worlds() -> Result<(tempfile::TempDir, OpenProject), Box<dyn std::error::Error>> {
		let tmp = tempfile::tempdir()?;
		let path = tmp.path().join("Game");
		Project::create(&path)?;
		let mut project = OpenProject::open(&path)?;
		project.create_world("", "A")?;
		project.create_world("", "B")?;
		Ok((tmp, project))
	}

	#[test]
	fn the_renderer_follows_the_world_that_is_shown_and_the_project_gets_the_engines_shaders() -> TestResult {
		let Some((device, queue)) = gpu() else {
			return Ok(());
		};
		let (_tmp, mut project) = two_worlds()?;
		let mut view = SceneView::default();
		let shown = view.draw(&device, &queue, &mut project, "A").ok_or("A is not drawn")?;
		assert!(shown.resized, "the UI has not seen the texture yet");
		assert!(project.worlds["A"].world.world().contains_resource::<Renderer>());
		assert!(project.project.exists(&zpath("Shaders://Zenith/lines.wesl"))?);

		let shown = view
			.draw(&device, &queue, &mut project, "A")
			.ok_or("A is drawn again")?;
		assert!(!shown.resized, "the same texture");

		view.draw(&device, &queue, &mut project, "B").ok_or("B is not drawn")?;
		assert!(project.worlds["B"].world.world().contains_resource::<Renderer>());
		assert!(
			!project.worlds["A"].world.world().contains_resource::<Renderer>(),
			"it moved"
		);
		assert!(view.draw(&device, &queue, &mut project, "Nowhere").is_none());

		// Letting go gives the renderer back for the next project.
		view.release(Some(&mut project));
		assert!(!project.worlds["B"].world.world().contains_resource::<Renderer>());
		Ok(())
	}

	#[test]
	fn the_panel_gets_the_size_it_asks_for_and_the_gizmo_moves_the_selected_entity() -> TestResult {
		let Some((device, queue)) = gpu() else {
			return Ok(());
		};
		let (_tmp, mut project) = two_worlds()?;
		let mut view = SceneView {
			wanted: (320, 200),
			..SceneView::default()
		};
		view.draw(&device, &queue, &mut project, "A").ok_or("not drawn")?;
		let targets = project.worlds["A"].world.world().resource::<RenderTargets>();
		let target = view.target.and_then(|id| targets.get(id)).ok_or("no target")?;
		assert_eq!(target.size, (320, 200));
		view.wanted = (0, 99_999_999);
		let shown = view.draw(&device, &queue, &mut project, "A").ok_or("not drawn")?;
		assert!(shown.resized);
		assert_eq!(view.texture_size.0, 1, "at least one pixel");
		assert!(view.texture_size.1 <= device.limits().max_texture_dimension_2d);

		let entity = project.worlds.get_mut("A").ok_or("no A")?.world.create_entity("Box");
		project.worlds.get_mut("A").ok_or("no A")?.world.add_component(
			entity,
			Transform {
				position: DVec3::new(1.0, 2.0, 3.0),
				..Transform::default()
			},
		)?;
		view.selection = Some(entity);
		let model = view.selected_matrix(&project, "A").ok_or("no matrix")?;
		let (_, _, relative) = model.to_scale_rotation_translation();
		let expected = (DVec3::new(1.0, 2.0, 3.0) - view.camera.position).as_vec3();
		assert!((relative - expected).length() < 1e-5, "relative to the camera");

		// A new world is dirty until it is saved.
		project.save_dirty();
		assert!(!project.worlds["A"].dirty);
		view.apply_matrix(&mut project, "A", Mat4::from_translation(expected + Vec3::X));
		let moved = project.worlds["A"]
			.world
			.get_component::<Transform>(entity)
			.ok_or("no transform")?
			.position;
		assert!((moved - DVec3::new(2.0, 2.0, 3.0)).length() < 1e-5, "{moved}");
		assert!(project.worlds["A"].dirty, "the world has changes to save");
		Ok(())
	}

	#[test]
	fn changes_are_sorted_by_what_they_ask_of_the_renderer() {
		let mut changes = Changes::default();
		changes.add(
			&ChangeKind::Modified,
			&zpath("Shaders://Zenith/tonemap.wesl"),
			&["shaders".to_string()],
		);
		changes.add(
			&ChangeKind::Modified,
			&zpath("Assets://Models/Cube/cube.zenithmodel"),
			&["assets".to_string()],
		);
		assert!(changes.shaders && !changes.rescan);
		assert_eq!(changes.assets.len(), 1);
		changes.add(
			&ChangeKind::Created,
			&zpath("Assets://Models/new.zenithmodel"),
			&["assets".to_string()],
		);
		assert!(changes.rescan, "a new file: the list is read again");
		assert_eq!(changes.assets.len(), 1, "and it is not a change of an old one");
	}

	#[test]
	fn the_watch_rules_keep_shaders_and_worlds_out_of_the_assets() {
		let rules = watch_rules();
		let assets = rules.iter().find(|rule| rule.name == "assets");
		assert!(assets.is_some_and(|rule| rule.exclude.iter().any(|glob| glob.contains("Worlds"))));
		assert!(assets.is_some_and(|rule| rule.exclude.iter().any(|glob| glob.contains("Shaders"))));
	}

	#[test]
	fn lights_and_cameras_get_lines_and_a_selected_point_light_shows_its_range() {
		let mut world = World::new();
		let light = world
			.spawn((
				Transform::default(),
				PointLight {
					range: 5.0,
					..PointLight::default()
				},
			))
			.id();
		world.spawn((Transform::default(), DirectionalLight::default()));
		world.spawn((Transform::default(), Camera::default()));

		let mut lines = GizmoLines::default();
		gizmo_lines(&mut world, None, &mut lines);
		let plain = lines.0.len();
		assert!(plain > 0);
		assert!(lines.0.iter().all(|line| line.from.is_finite() && line.to.is_finite()));

		let mut lines = GizmoLines::default();
		gizmo_lines(&mut world, Some(light), &mut lines);
		assert_eq!(lines.0.len(), plain + 3 * 32, "three circles of the range");
	}

	#[test]
	fn the_hierarchy_lists_what_has_a_place_by_name() {
		let mut world = zenith_world::World::new("w");
		let named = world.create_entity("Sun");
		let _ = world.add_component(named, Transform::default());
		let unnamed = world.world_mut().spawn(Transform::default()).id();
		world.create_entity("Not placed");
		let listed = entities(&mut world);
		assert_eq!(listed.len(), 2, "only what has a Transform");
		assert!(listed.iter().any(|(entity, label)| *entity == named && label == "Sun"));
		assert!(
			listed
				.iter()
				.any(|(entity, label)| *entity == unnamed && label.starts_with("Entity "))
		);
	}
}
