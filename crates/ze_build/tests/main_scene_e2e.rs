//! End-to-end regression test for the standalone-build "wrong starting scene"
//! bug: a shipped standalone game runs with no `Project`/`ZEProject.toml` on
//! disk (see `ResourceManager::for_runtime`), so `main_scene` must survive
//! into the packaged `assets.zepack` via `GameData.toml`. This builds a copy
//! of the real `Sandbox` project with a non-default main scene and inspects
//! the packaged output directly.

use std::{
	path::{Path, PathBuf},
	process::Command,
};

use ze_build::{BuildOptions, BuildTarget, build_project_dist};
use ze_project::Project;

fn workspace_root() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.parent() // crates/
		.and_then(Path::parent) // ZeroEngine/ (inner)
		.and_then(Path::parent) // workspace root
		.expect("ze_build crate has a workspace root three levels up")
		.to_path_buf()
}

/// `OtherScene` has no camera in the real Sandbox project (build validation
/// requires the main scene to have one), so this injects a minimal camera
/// entity into a scratch copy of the scene -- the real `Sandbox/assets` tree
/// is never touched.
fn add_camera_entity(scene_path: &Path) {
	let source = std::fs::read_to_string(scene_path).expect("scene file should be readable");
	let mut scene: serde_json::Value = serde_json::from_str(&source).expect("scene file should be valid JSON");

	let camera_entity: serde_json::Value = serde_json::json!({
		"id": { "index": 100, "gen": 0 },
		"components": [
			{
				"component_type": "ze.name",
				"value": { "name": "TestCamera" }
			},
			{
				"component_type": "ze.renderer.camera",
				"value": {
					"clear_color": [0.12, 0.12, 0.13, 1.0],
					"primary": true,
					"projection": { "Orthographic": { "far": 1000.0, "near": -1000.0, "size": 10.0 } }
				}
			},
			{
				"component_type": "ze.transform",
				"value": {
					"position": [0.0, 0.0, 10.0],
					"rotation": [0.0, 0.0, 0.0, 1.0],
					"scale": [1.0, 1.0, 1.0]
				}
			}
		]
	});

	scene["entities"]
		.as_array_mut()
		.expect("scene should have an entities array")
		.push(camera_entity);

	std::fs::write(scene_path, serde_json::to_string_pretty(&scene).unwrap()).expect("failed to write scene file");
}

#[test]
fn build_project_dist_bakes_configured_main_scene_into_packaged_game_data() {
	let root = workspace_root();

	let scratch_root = std::env::temp_dir().join(format!("ze-main-scene-e2e-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&scratch_root);
	std::fs::create_dir_all(&scratch_root).expect("failed to create scratch root");
	let project_dir = scratch_root.join("Sandbox");
	let output_dir = scratch_root.join("dist");

	let copy_status = Command::new("cp")
		.arg("-r")
		.arg(root.join("Sandbox"))
		.arg(&project_dir)
		.status()
		.expect("failed to spawn `cp`");
	assert!(copy_status.success(), "copying Sandbox project failed");

	add_camera_entity(&project_dir.join("assets/scenes/OtherScene.zescene.json"));

	let mut project =
		Project::load(project_dir.join("ZEProject.toml")).expect("copied Sandbox/ZEProject.toml should load");

	// Point main_scene at a scene other than the default `main` to prove the
	// built standalone follows whatever's *configured*, not a hardcoded name.
	project
		.set_main_scene("OtherScene")
		.expect("OtherScene should be a valid, already-listed scene");
	assert_eq!(project.main_scene, "OtherScene");

	let notice_path = root.join("NOTICE");
	let options = BuildOptions::debug(output_dir.clone(), notice_path).with_target(BuildTarget::host());

	let cleanup = || {
		let _ = std::fs::remove_dir_all(&scratch_root);
	};

	let output = match build_project_dist(&project, &options) {
		Ok(output) => output,
		Err(error) => {
			cleanup();
			panic!("build_project_dist failed: {error:?}");
		}
	};

	let pack = match zepack::SmartZepack::open(&output.assets_pack) {
		Ok(pack) => pack,
		Err(error) => {
			cleanup();
			panic!("failed to open packaged assets.zepack: {error:?}");
		}
	};

	if !pack.contains("GameData.toml") {
		cleanup();
		panic!("packaged assets.zepack should contain GameData.toml");
	}

	let game_data_bytes = match pack.read_file("GameData.toml") {
		Ok(bytes) => bytes,
		Err(error) => {
			cleanup();
			panic!("GameData.toml should be readable from the pack: {error:?}");
		}
	};

	let extracted_path = output_dir.join("GameData.toml");
	std::fs::write(&extracted_path, &game_data_bytes).expect("failed to write extracted GameData.toml");
	let game_data = ze_project::GameData::load(&extracted_path).expect("GameData.toml should parse as GameData");

	cleanup();

	assert_eq!(
		game_data.main_scene, "OtherScene",
		"packaged GameData.toml should carry the project's configured main_scene, not the default"
	);
}
