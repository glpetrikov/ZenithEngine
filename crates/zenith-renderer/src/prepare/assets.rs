//! `prepare_assets`: asks the `GpuCache` for what the frame draws, loads some
//! of it and marks it as used.

use std::sync::Arc;

use bevy_ecs::{
	resource::Resource,
	system::{Res, ResMut},
};
use zenith_filesystem::Project;

use crate::{
	assets::AssetRegistry,
	cache::GpuCache,
	extract::{ExtractedMeshes, ExtractedSprites, ExtractedViews},
};

/// The project the assets are read from.
#[derive(Resource, Clone)]
pub struct ProjectRes(pub Arc<Project>);

/// Requests what the frame draws and loads some of it.
///
/// The cache loads what is missing within its time budget; what is not loaded
/// yet is drawn with a stand-in. Without a project or its registry nothing can
/// be loaded: everything is drawn with stand-ins.
#[allow(clippy::needless_pass_by_value)] // a system takes its parameters by value
pub fn prepare_assets(
	meshes: Res<ExtractedMeshes>,
	sprites: Res<ExtractedSprites>,
	views: Res<ExtractedViews>,
	project: Option<Res<ProjectRes>>,
	registry: Option<Res<AssetRegistry>>,
	mut cache: ResMut<GpuCache>,
) {
	for mesh in &meshes.0 {
		cache.request_mesh(mesh.mesh);
		cache.request_material(mesh.material);
	}
	for sprite in &sprites.0 {
		cache.request_texture(sprite.texture);
	}
	for view in &views.0 {
		if let Some((sky, _)) = view.environment {
			cache.request_texture(sky);
		}
	}
	if let (Some(project), Some(registry)) = (project, registry) {
		let budget = cache.settings().load_budget;
		cache.load(&project.0, &registry, Some(budget));
	}
}

/// Asks [`load_shaders`] to read the shaders of the project again (the editor
/// does it when a file changed).
#[derive(Resource, Default)]
pub struct ReloadShaders(pub bool);

/// Reads the shaders of the project into the cache.
///
/// Once, when the project is there, and again when [`ReloadShaders`] says so.
/// What it finds wrong is in the log; a missing shader of the engine is stood
/// in for by the engine's copy.
#[allow(clippy::needless_pass_by_value)] // a system takes its parameters by value
pub fn load_shaders(
	project: Option<Res<ProjectRes>>,
	reload: Option<ResMut<ReloadShaders>>,
	mut cache: ResMut<GpuCache>,
	mut loaded: bevy_ecs::system::Local<bool>,
) {
	let asked = reload.is_some_and(|mut reload| std::mem::take(&mut reload.0));
	let Some(project) = project else {
		return;
	};
	if *loaded && !asked {
		return;
	}
	*loaded = true;
	match crate::shaders::project::load_project_shaders(&project.0, cache.sources_mut()) {
		Ok(report) => {
			if !report.missing.is_empty() {
				zenith_log::warn!(
					"The project has no copy of {}: the engine's stands in",
					report.missing.join(", ")
				);
			}
			for (name, why) in &report.unreadable {
				zenith_log::warn!("Shader {name} cannot be read: {why}");
			}
		}
		Err(e) => zenith_log::warn!("The shaders of the project cannot be read: {e}"),
	}
}
