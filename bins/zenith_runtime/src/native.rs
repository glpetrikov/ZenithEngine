//! What the native runtime does with projects. The browser has no folders to
//! open, so none of this exists there (see `Cargo.toml`).

use std::{
	path::{Path, PathBuf},
	sync::atomic::{AtomicBool, Ordering},
	time::Duration,
};

use zenith_build::{BuildFormat, Builder, EncryptionKey, RenameExe, ZPath};
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{OpenOptions, Project, ProjectEvent, WatchConfig, layout};
use zenith_world::World;

/// The project `--test` works on, relative to where `cargo run` was started.
const TEST_PROJECT: &str = "Sandbox/Test";
/// Where `--test` writes its builds, inside the test project.
const TEST_BUILDS: &str = "Sandbox/Test/Builds";
/// The output of `wasm-pack build --target web` and the page that starts it,
/// relative to where `cargo run` was started.
const WEB_PKG: &str = "bins/zenith_runtime/pkg";
const WEB_INDEX: &str = "bins/zenith_runtime/index.html";
/// How the browser runtime is made, from where `cargo run` was started.
const WASM_PACK: &str = "run `wasm-pack build bins/zenith_runtime --target web`";
/// The key of the encrypted test builds. A test key, not a secret.
const TEST_KEY: [u8; 32] = [0x5a; 32];

/// Set by Ctrl+C, SIGTERM or closing the console.
static QUIT: AtomicBool = AtomicBool::new(false);

/// Makes Ctrl+C (and SIGTERM, and closing the console) ask the game to end the
/// way closing the window does, so that it gets to clean up. The event loop
/// looks at [`quit_requested`] between events. Only the first call has an
/// effect.
pub fn quit_on_ctrl_c() {
	if let Err(e) = ctrlc::set_handler(|| QUIT.store(true, Ordering::SeqCst)) {
		zenith_log::warn!("Ctrl+C is not handled: {e}");
	}
}

/// Did the user ask the process to end from outside the window?
pub fn quit_requested() -> bool {
	QUIT.load(Ordering::SeqCst)
}

/// The project the runtime works with: with `--test` the test project (which
/// is built too), otherwise the build that lies next to the executable.
pub fn open_project() -> ZResult<Project> {
	if std::env::args().any(|arg| arg == "--test") {
		test_run()
	} else {
		open_game()
	}
}

/// How a build is opened: with the key the build wrote into this executable,
/// if it is encrypted.
fn build_options() -> OpenOptions {
	zenith_filesystem::embedded_key().map_or_else(OpenOptions::new, |key| OpenOptions::new().encryption_key(key))
}

/// Opens the build next to the executable: `game.zenithbuild` or `game/`,
/// beside it or in a `Build` folder beside it.
fn open_game() -> ZResult<Project> {
	let exe = std::env::current_exe()?;
	let folder = exe
		.parent()
		.ok_or_else(|| ZenithError::ProjectNotFound(format!("{} has no folder", exe.display())))?;
	let archive = format!("{}.{}", layout::DEFAULT_BUILD_NAME, layout::BUILD_FILE_EXTENSION);
	let mut tried = Vec::new();
	for base in [folder.to_path_buf(), folder.join(layout::DIR_BUILD)] {
		for name in [archive.as_str(), layout::DEFAULT_BUILD_NAME] {
			let candidate = base.join(name);
			if candidate.exists() {
				return build_options().open(candidate);
			}
			tried.push(candidate.display().to_string());
		}
	}
	Err(ZenithError::ProjectNotFound(format!(
		"no build next to the executable (looked for {})",
		tried.join(", ")
	)))
}

/// `--test`: opens the test project, watches it, writes a world and makes the
/// builds for the desktop and the browser, with and without encryption.
fn test_run() -> ZResult<Project> {
	let project = Project::open(TEST_PROJECT)?;
	watch(&project)?;

	let mut world = World::new("world");
	let entity = world.create_entity("Test");
	world.add_component(
		entity,
		zenith_components::Transform {
			position: glam::DVec3::new(10.0, -10.0, 3.0),
			orientation: glam::Quat::from_rotation_z(10.3),
			scale: glam::DVec3::new(1.0, 2.0, 1.0),
		},
	)?;
	project.write(&ZPath::new("Worlds/world.zenith")?, &world.snapshot_bytes()?)?;

	build_all()?;
	Ok(project)
}

fn watch(project: &Project) -> ZResult<()> {
	let watcher = project.watch(&WatchConfig::File)?;
	let events = watcher.events().clone();
	std::thread::spawn(move || {
		// The watcher stops when it is dropped, so this thread keeps it.
		let _watcher = watcher;
		while let Some(event) = events.recv_timeout(Duration::from_millis(500)) {
			match event {
				ProjectEvent::Change(change) => println!("{change:?}"),
				ProjectEvent::Rescan => {}
			}
		}
	});
	Ok(())
}

/// Was the `.wasm` of the browser runtime made before `exe`? The executable is
/// built again whenever the code changes, so an older `.wasm` was built from
/// older code. `false` when either cannot be looked at.
fn pkg_is_older_than(exe: &Path) -> bool {
	let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
	let Some(exe_time) = modified(exe) else {
		return false;
	};
	let Ok(entries) = std::fs::read_dir(WEB_PKG) else {
		return false;
	};
	entries
		.filter_map(Result::ok)
		.map(|entry| entry.path())
		.filter(|path| path.extension().is_some_and(|ext| ext == "wasm"))
		.filter_map(|path| modified(&path))
		.any(|wasm_time| wasm_time < exe_time)
}

/// `Builds/<Pc|Web>/<Plain|Encrypted>`. A PC build is a folder build with the
/// executable beside it; a web build is an archive with the browser runtime
/// beside it. Plain builds are not compressed, so their files can be read;
/// encrypted ones are compressed too. The executable and the browser files are
/// copied as they are in every case.
fn build_all() -> ZResult<()> {
	let exe = std::env::current_exe()?;
	let web = Path::new(WEB_PKG).is_dir();
	if !web {
		zenith_log::warn!("{WEB_PKG} does not exist ({WASM_PACK}): no web builds");
	} else if pkg_is_older_than(&exe) {
		// The web builds copy the browser runtime as it is: an old one does
		// not know what the engine does now.
		zenith_log::warn!("{WEB_PKG} is older than this runtime: the web builds get the old one ({WASM_PACK})");
	}
	for encrypted in [false, true] {
		let kind = if encrypted { "Encrypted" } else { "Plain" };
		let base = |platform: &str| PathBuf::from(TEST_BUILDS).join(platform).join(kind);
		let builder = || {
			let builder = Builder::new().path(TEST_PROJECT).zstd_compression(10);
			if encrypted {
				builder.encryption_key(EncryptionKey::new(TEST_KEY))
			} else {
				builder.compress(false)
			}
		};

		builder()
			.format(BuildFormat::Fs)
			.output(base("Pc").join(layout::DEFAULT_BUILD_NAME))
			.engine_exe(&exe)
			.rename_exe(RenameExe::ProjectName)
			.build()?;

		if web {
			builder()
				.format(BuildFormat::Archive)
				.output(base("Web").join(format!(
					"{}.{}",
					layout::DEFAULT_BUILD_NAME,
					layout::BUILD_FILE_EXTENSION
				)))
				.web_runtime(WEB_PKG)
				.web_index(WEB_INDEX)
				.build()?;
		}
	}
	Ok(())
}
