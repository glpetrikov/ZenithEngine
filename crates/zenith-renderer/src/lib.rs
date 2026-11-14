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
pub mod surface;
pub mod targets;

use bevy_ecs::{
	change_detection::DetectChanges,
	schedule::{IntoScheduleConfigs, Schedule, ScheduleLabel, SystemSet, common_conditions::resource_exists},
	system::{Res, ResMut},
	world::World,
};
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
		apply_settings.in_set(RenderSet::Prepare),
		render::render.in_set(RenderSet::Render),
	));
}

/// Puts the resources of the renderer into `world`.
pub fn insert_resources(world: &mut World, renderer: Renderer, settings: RenderSettings) {
	world.insert_resource(renderer);
	world.insert_resource(settings);
	world.init_resource::<ClearColor>();
}

/// Gives `renderer` and its `settings` to `world`. They outlive
/// `clear_world`. The render systems are not the world's: see [`schedule`].
pub fn install(world: &mut zenith_world::World, renderer: Renderer, settings: RenderSettings) {
	world.keep_resource::<Renderer>();
	world.keep_resource::<RenderSettings>();
	world.keep_resource::<ClearColor>();
	insert_resources(world.world_mut(), renderer, settings);
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
