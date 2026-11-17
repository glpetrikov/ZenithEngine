//! The shaders of a project: written by the engine, read back, and drawn with.
//! The drawing tests are skipped where there is no adapter.

// A test that cannot go on has to stop: panicking is how it fails.
#![allow(clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::Scene;
use zenith_filesystem::{Project, ZPath};
use zenith_renderer::{
	ProjectRes,
	prepare::ReloadShaders,
	shaders::{
		ENGINE, ShaderSources, TONEMAP,
		project::{install_engine_shaders, load_project_shaders, restore_engine_shaders},
	},
};

fn new_project(dir: &tempfile::TempDir) -> Project {
	Project::create(dir.path().join("P")).expect("make a project")
}

fn tonemap_path() -> ZPath {
	ZPath::new("Shaders://Zenith/tonemap.wesl").expect("a path")
}

#[test]
fn the_engine_writes_its_shaders_once_and_leaves_changes_alone() {
	let dir = tempfile::tempdir().expect("a folder");
	let project = new_project(&dir);

	let report = install_engine_shaders(&project).expect("install");
	assert_eq!(report.written.len(), ENGINE.len());
	assert!(project.exists(&tonemap_path()).expect("exists"));

	// A copy that was changed on purpose is not written over.
	project
		.write(&tonemap_path(), b"// zenith-shader: Zenith/tonemap 1\n// mine\n")
		.expect("write");
	let again = install_engine_shaders(&project).expect("install again");
	assert_eq!(again.written, Vec::<String>::new());
	assert!(again.newer_in_engine.is_empty(), "same version: nothing newer");
	assert!(project.read_string(&tonemap_path()).expect("read").contains("// mine"));

	// An older version is reported, and still not touched.
	project
		.write(&tonemap_path(), b"// zenith-shader: Zenith/tonemap 0\n")
		.expect("write");
	let older = install_engine_shaders(&project).expect("install over older");
	assert_eq!(older.newer_in_engine, [TONEMAP]);

	restore_engine_shaders(&project).expect("restore");
	assert!(
		!project
			.read_string(&tonemap_path())
			.expect("read")
			.contains("version 0")
	);
	assert!(
		project
			.read_string(&tonemap_path())
			.expect("read")
			.starts_with("// zenith-shader: Zenith/tonemap 1")
	);
}

#[test]
fn a_project_is_read_with_its_own_shaders_over_the_engines() {
	let dir = tempfile::tempdir().expect("a folder");
	let project = new_project(&dir);
	install_engine_shaders(&project).expect("install");
	project
		.write(
			&ZPath::new("Shaders://Effects/toon.wesl").expect("path"),
			b"// a shader of the game\n",
		)
		.expect("write");
	project
		.write(
			&ZPath::new("Shaders://Effects/plain.wgsl").expect("path"),
			b"// plain wgsl\n",
		)
		.expect("write");
	project
		.write(&ZPath::new("Shaders://notes.txt").expect("path"), b"not a shader")
		.expect("write");

	let mut sources = ShaderSources::engine();
	let report = load_project_shaders(&project, &mut sources).expect("load");
	assert!(report.missing.is_empty(), "{:?}", report.missing);
	assert_eq!(sources.get("Effects/toon"), Some("// a shader of the game\n"));
	assert_eq!(sources.get("Effects/plain"), Some("// plain wgsl\n"));
	assert!(sources.get("notes").is_none());

	// A shader taken out of the project is gone after the next read; an
	// engine shader taken out comes back as the engine's copy.
	project
		.remove(&ZPath::new("Shaders://Effects/toon.wesl").expect("path"))
		.expect("remove");
	project.remove(&tonemap_path()).expect("remove");
	let report = load_project_shaders(&project, &mut sources).expect("load again");
	assert!(sources.get("Effects/toon").is_none());
	assert_eq!(report.missing, [TONEMAP]);
	assert!(sources.get(TONEMAP).is_some());
}

#[test]
fn a_project_without_shaders_draws_with_the_engines() {
	let dir = tempfile::tempdir().expect("a folder");
	let project = new_project(&dir);
	let mut sources = ShaderSources::engine();
	let report = load_project_shaders(&project, &mut sources).expect("load");
	assert_eq!(report.loaded, Vec::<String>::new());
	assert_eq!(report.missing.len(), ENGINE.len());
}

/// A scene whose project is a fresh one that has the engine's shaders.
fn scene_with_project() -> Option<(Scene, Arc<Project>, tempfile::TempDir)> {
	let mut scene = Scene::new()?;
	let dir = tempfile::tempdir().expect("a folder");
	let project = Arc::new(new_project(&dir));
	install_engine_shaders(&project).expect("install");
	scene
		.world
		.world_mut()
		.insert_resource(ProjectRes(Arc::clone(&project)));
	Some((scene, project, dir))
}

fn reload(scene: &mut Scene) {
	scene.world.world_mut().resource_mut::<ReloadShaders>().0 = true;
	scene.render_frames(2);
}

#[test]
fn a_changed_shader_of_the_project_draws_with_the_change() {
	let Some((mut scene, project, _dir)) = scene_with_project() else {
		return;
	};
	scene.render_frames(2);
	let [r, _, b, _] = scene.pixel(2, 2);
	assert!(b > r, "the blue background first: {r} {b}");

	let changed = zenith_renderer::shaders::ENGINE
		.iter()
		.find(|(name, _)| *name == TONEMAP)
		.expect("the engine has it")
		.1
		.replace(
			"return vec4<f32>(mapped, texel.a);",
			"return vec4<f32>(1.0, 0.0, 0.0, 1.0);",
		);
	project.write(&tonemap_path(), changed.as_bytes()).expect("write");
	reload(&mut scene);
	assert_eq!(scene.pixel(2, 2), [255, 0, 0, 255], "red now");
}

#[test]
fn a_broken_shader_keeps_the_last_one_that_worked_and_a_broken_first_one_the_engines() {
	let Some((mut scene, project, _dir)) = scene_with_project() else {
		return;
	};
	scene.render_frames(2);
	let good = scene.pixel(2, 2);

	project
		.write(
			&tonemap_path(),
			b"// zenith-shader: Zenith/tonemap 1\nthis is not a shader",
		)
		.expect("write");
	reload(&mut scene);
	assert_eq!(scene.pixel(2, 2), good, "the last version that compiled");

	// A project whose copy is broken from the start: the engine's stands in.
	let Some((mut fresh, fresh_project, _dir2)) = scene_with_project() else {
		return;
	};
	fresh_project
		.write(&tonemap_path(), b"broken from the start")
		.expect("write");
	fresh.render_frames(3);
	assert_eq!(fresh.pixel(2, 2), good);
}
