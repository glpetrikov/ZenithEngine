//! The renderer. The plan is in `docs/renderer-plan.md`.
//!
//! A host (the runtime, the editor) makes a [`Renderer`] with
//! [`RenderSettings`], gives both to a world with [`install`], and keeps a
//! render schedule of its own ([`schedule`]), which it runs on the world every
//! frame:
//!
//! ```ignore
//! let renderer = Renderer::new(window, width, height, &settings).await?;
//! zenith_renderer::install(&mut world, renderer, settings);
//! let mut render = zenith_renderer::schedule();
//! // every frame
//! render.run(world.world_mut());
//! ```

pub mod assets;
pub mod cache;
pub mod components;
pub mod extract;
pub mod graph;
pub mod passes;
pub mod prepare;
pub mod render;
pub mod renderer;
pub mod settings;
pub mod shaders;
pub mod surface;
pub mod targets;

use bevy_ecs::{
	change_detection::DetectChanges,
	schedule::{IntoScheduleConfigs, Schedule, ScheduleLabel, SystemSet, common_conditions::resource_exists},
	system::{Res, ResMut},
	world::World,
};
pub use cache::{CacheSettings, GpuCache};
pub use components::register_components;
pub use prepare::ProjectRes;
pub use render::ClearColor;
pub use renderer::Renderer;
pub use settings::RenderSettings;
/// The wgpu the renderer is built on: a host uses this one, so the versions
/// match.
pub use wgpu;

/// The label of the render schedule.
#[derive(ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Render;

/// The steps of the render schedule, in this order.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderSet {
	/// Assets that changed on disk are read again.
	Assets,
	/// What the frame draws is copied out of the components.
	Extract,
	/// It is made ready for the GPU.
	Prepare,
	/// The graphs run and the frame is shown.
	Render,
	/// What was not used for long is let go.
	Cleanup,
}

/// A render schedule with the systems of the renderer. The host keeps it and
/// runs it on its world each frame, when it wants to draw.
pub fn schedule() -> Schedule {
	let mut schedule = Schedule::new(Render);
	add_systems(&mut schedule);
	schedule
}

/// Adds the systems of the renderer to `schedule`. They do nothing while the
/// world has no [`Renderer`] (on the web it comes later than the world).
pub fn add_systems(schedule: &mut Schedule) {
	schedule.configure_sets(
		(
			RenderSet::Assets,
			RenderSet::Extract,
			RenderSet::Prepare,
			RenderSet::Render,
			RenderSet::Cleanup,
		)
			.chain()
			.run_if(resource_exists::<Renderer>),
	);
	schedule.add_systems((
		cache::evict::begin_frame
			.in_set(RenderSet::Assets)
			.run_if(resource_exists::<GpuCache>),
		(
			extract::extract_cameras,
			extract::extract_meshes,
			extract::extract_sprites,
			extract::extract_lights,
		)
			.in_set(RenderSet::Extract),
		apply_settings.in_set(RenderSet::Prepare),
		(
			prepare::assets::load_shaders.run_if(resource_exists::<GpuCache>),
			prepare::assets::prepare_assets.run_if(resource_exists::<GpuCache>),
			targets::prepare_targets,
			prepare::views::prepare_views,
			prepare::draw_lists::prepare_draw_lists.run_if(resource_exists::<GpuCache>),
			prepare::sprite_lists::prepare_sprite_lists,
		)
			.chain()
			.after(apply_settings)
			.in_set(RenderSet::Prepare),
		render::render
			.in_set(RenderSet::Render)
			.run_if(resource_exists::<GpuCache>),
		cache::evict::evict
			.in_set(RenderSet::Cleanup)
			.run_if(resource_exists::<GpuCache>),
	));
}

/// Puts the resources of the renderer into `world`: it, its `settings` and a
/// [`GpuCache`] on its device.
pub fn insert_resources(world: &mut World, renderer: Renderer, settings: RenderSettings) {
	world.insert_resource(GpuCache::new(
		renderer.device(),
		renderer.queue(),
		CacheSettings::default(),
	));
	world.insert_resource(renderer);
	world.insert_resource(settings);
	world.init_resource::<ClearColor>();
	world.init_resource::<extract::ExtraViews>();
	world.init_resource::<targets::RenderTargets>();
	init_frame_resources(world);
}

/// The resources the systems of a frame expect to be there.
fn init_frame_resources(world: &mut World) {
	world.init_resource::<extract::ExtractedViews>();
	world.init_resource::<extract::ExtractedMeshes>();
	world.init_resource::<extract::ExtractedSprites>();
	world.init_resource::<extract::ExtractedLights>();
	world.init_resource::<prepare::PreparedViews>();
	world.init_resource::<prepare::DrawLists>();
	world.init_resource::<prepare::SpriteLists>();
	world.init_resource::<prepare::ReloadShaders>();
	world.init_resource::<passes::editor::GizmoLines>();
	world.init_resource::<passes::editor::EditorSelection>();
}

/// Gives `renderer` and its `settings` to `world`. They outlive
/// `clear_world`, and so does the cache. The render systems are not the
/// world's: see [`schedule`].
pub fn install(world: &mut zenith_world::World, renderer: Renderer, settings: RenderSettings) {
	keep_resources(world);
	insert_resources(world.world_mut(), renderer, settings);
}

/// Makes what the renderer keeps survive `clear_world`.
fn keep_resources(world: &mut zenith_world::World) {
	world.keep_resource::<Renderer>();
	world.keep_resource::<RenderSettings>();
	world.keep_resource::<ClearColor>();
	world.keep_resource::<GpuCache>();
	// What the frame holds is made again every frame, but the systems expect
	// it to be there, and the views of the host and the targets are not the
	// world's.
	world.keep_resource::<extract::ExtractedViews>();
	world.keep_resource::<extract::ExtractedMeshes>();
	world.keep_resource::<extract::ExtractedSprites>();
	world.keep_resource::<extract::ExtractedLights>();
	world.keep_resource::<extract::ExtraViews>();
	world.keep_resource::<prepare::PreparedViews>();
	world.keep_resource::<prepare::DrawLists>();
	world.keep_resource::<prepare::SpriteLists>();
	world.keep_resource::<prepare::ReloadShaders>();
	world.keep_resource::<targets::RenderTargets>();
}

/// A change of [`RenderSettings`] reaches the renderer.
#[allow(clippy::needless_pass_by_value)] // a system takes its parameters by value
fn apply_settings(settings: Option<Res<RenderSettings>>, mut renderer: ResMut<Renderer>) {
	if let Some(settings) = settings
		&& settings.is_changed()
	{
		renderer.apply_settings(&settings);
	}
}

/// Gives `world` the project the assets are read from, and the list of its
/// assets. They outlive `clear_world` too. Call it again to read the assets
/// anew.
///
/// # Errors
/// The project's error if its assets cannot be listed.
pub fn install_project(
	world: &mut zenith_world::World,
	project: std::sync::Arc<zenith_filesystem::Project>,
) -> zenith_error::ZResult<()> {
	let registry = assets::AssetRegistry::scan(&project)?;
	world.keep_resource::<ProjectRes>();
	world.keep_resource::<assets::AssetRegistry>();
	world.world_mut().insert_resource(ProjectRes(project));
	world.world_mut().insert_resource(registry);
	Ok(())
}

/// The renderer and what it keeps for as long as the host lives.
///
/// That is its GPU, its cache, its targets and its views. [`uninstall`] takes
/// it out of one world and [`install_again`] puts it in another, so the editor
/// can draw the world it has open without the GPU and the loaded assets
/// starting over.
pub struct Installed {
	renderer: Renderer,
	settings: RenderSettings,
	cache: GpuCache,
	clear: ClearColor,
	targets: targets::RenderTargets,
	extra_views: extract::ExtraViews,
}

impl Installed {
	/// The renderer inside.
	pub const fn renderer(&self) -> &Renderer {
		&self.renderer
	}
}

/// Takes what [`install`] gave out of `world`. `None` if it has none of it.
pub fn uninstall(world: &mut zenith_world::World) -> Option<Installed> {
	let ecs = world.world_mut();
	let cache = ecs.remove_resource::<GpuCache>()?;
	let Some(renderer) = ecs.remove_resource::<Renderer>() else {
		ecs.insert_resource(cache);
		return None;
	};
	let installed = Installed {
		renderer,
		cache,
		settings: ecs.remove_resource::<RenderSettings>().unwrap_or_default(),
		clear: ecs.remove_resource::<ClearColor>().unwrap_or_default(),
		targets: ecs.remove_resource::<targets::RenderTargets>().unwrap_or_default(),
		extra_views: ecs.remove_resource::<extract::ExtraViews>().unwrap_or_default(),
	};
	// What the frame held is of no use to another world, and has buffers.
	ecs.remove_resource::<extract::ExtractedViews>();
	ecs.remove_resource::<extract::ExtractedMeshes>();
	ecs.remove_resource::<extract::ExtractedSprites>();
	ecs.remove_resource::<extract::ExtractedLights>();
	ecs.remove_resource::<prepare::PreparedViews>();
	ecs.remove_resource::<prepare::DrawLists>();
	ecs.remove_resource::<prepare::SpriteLists>();
	ecs.remove_resource::<passes::editor::GizmoLines>();
	ecs.remove_resource::<passes::editor::EditorSelection>();
	Some(installed)
}

/// Puts what [`uninstall`] took into `world`, as [`install`] would have. The
/// project of the world is not part of it: see [`install_project`].
pub fn install_again(world: &mut zenith_world::World, installed: Installed) {
	let Installed {
		renderer,
		settings,
		cache,
		clear,
		targets,
		extra_views,
	} = installed;
	keep_resources(world);
	let ecs = world.world_mut();
	ecs.insert_resource(cache);
	ecs.insert_resource(renderer);
	ecs.insert_resource(settings);
	ecs.insert_resource(clear);
	ecs.insert_resource(targets);
	ecs.insert_resource(extra_views);
	init_frame_resources(ecs);
}
