use std::{
	collections::BTreeSet,
	fs,
	fs::OpenOptions,
	io,
	path::{Component, Path, PathBuf},
	process::{Child, Command, Stdio},
	sync::mpsc::{Receiver, channel},
	thread,
	time::{Duration, Instant, SystemTime},
};

use notify::{
	Event, RecursiveMode, Watcher,
	event::{AccessKind, AccessMode, EventKind},
};
use ze_assets::AssetRef;
use ze_ecs::Scene;
use ze_scripting_cs::{ScriptingSystem, sync_scene_script_fields};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptStatus {
	Unavailable,
	UpToDate,
	Recompiling,
	#[expect(dead_code, reason = "keeps the status bar ready for a queued reload state")]
	ReloadPending,
	Failed,
}

#[derive(Debug, Default)]
pub struct AssetReloadSet {
	texture_assets: Vec<AssetRef>,
}

impl AssetReloadSet {
	pub fn texture_assets(&self) -> &[AssetRef] { &self.texture_assets }
}

pub struct AssetHotReload {
	assets_root: PathBuf,
	script_assembly_path: PathBuf,
	script_runtime_config_path: PathBuf,
	csproj_path: Option<PathBuf>,
	_watcher: Option<notify::RecommendedWatcher>,
	events: Option<Receiver<notify::Result<Event>>>,
	build_child: Option<Child>,
	script_status: ScriptStatus,
	last_successful_load_mtime: Option<SystemTime>,
	last_reload_attempt: Option<Instant>,
	last_source_change: Option<Instant>,
	script_build_paused: bool,
	script_rebuild_pending: bool,
	script_classes_dirty: bool,
	assembly_candidate_mtime: Option<SystemTime>,
}

const SCRIPT_RELOAD_DEBOUNCE_MS: u128 = 500;
const SCRIPT_SOURCE_DEBOUNCE: Duration = Duration::from_millis(500);

impl AssetHotReload {
	pub fn new(
		assets_root: impl Into<PathBuf>,
		script_assembly_path: impl Into<PathBuf>,
		script_runtime_config_path: impl Into<PathBuf>,
		csproj_path: Option<&Path>,
	) -> Self {
		let assets_root = absolute_path(assets_root.into());
		let script_assembly_path = absolute_path(script_assembly_path.into());
		let script_runtime_config_path = absolute_path(script_runtime_config_path.into());
		let csproj_path = csproj_path.map(absolute_path);
		let (watcher, events) = create_watcher(&assets_root, &script_assembly_path, csproj_path.as_deref());

		if let Some(events) = &events {
			let count = events.try_iter().count();
			ze_log::debug!("drained {count} initial file watcher event(s) during hot-reload setup");
		}

		let initial_mtime = modified_time(&script_assembly_path);
		let (build_child, script_status) = csproj_path.as_deref().map_or_else(
			|| (None, ScriptStatus::Unavailable),
			|csproj| {
				let child = spawn_dotnet_build(csproj);
				let status = match child {
					Some(_) => ScriptStatus::Recompiling,
					None => ScriptStatus::Failed,
				};
				(child, status)
			},
		);

		Self {
			assets_root,
			script_assembly_path,
			script_runtime_config_path,
			csproj_path,
			_watcher: watcher,
			events,
			build_child,
			script_status,
			last_successful_load_mtime: initial_mtime,
			last_reload_attempt: None,
			last_source_change: None,
			script_build_paused: false,
			script_rebuild_pending: false,
			script_classes_dirty: false,
			assembly_candidate_mtime: None,
		}
	}

	pub const fn script_status(&self) -> ScriptStatus { self.script_status }

	pub const fn take_script_classes_dirty(&mut self) -> bool {
		let dirty = self.script_classes_dirty;
		self.script_classes_dirty = false;
		dirty
	}

	pub fn pause_script_watcher_for_build(&mut self) -> io::Result<()> {
		self.script_build_paused = true;
		Self::kill_build_child(&mut self.build_child);
		self.drain_pending_watcher_events();

		wait_for_script_output_handles_to_release(&self.script_assembly_path, &self.script_runtime_config_path)
	}

	pub fn resume_script_watcher_after_build(&mut self) {
		self.drain_pending_watcher_events();
		self.script_build_paused = false;
		self.last_successful_load_mtime = modified_time(&self.script_assembly_path);
	}

	pub fn poll(&mut self, scene: &mut Scene, _game_running: bool) -> AssetReloadSet {
		self.reap_build_child();

		if self.script_build_paused {
			self.drain_pending_watcher_events();
			return AssetReloadSet::default();
		}

		let mut texture_assets = BTreeSet::new();
		let mut script_source_changed = false;
		let mut script_output_close_write = false;

		if let Some(events) = &self.events {
			for event in events.try_iter() {
				let Ok(event) = event else {
					ze_log::error!("asset file watcher error: {event:?}");
					continue;
				};
				if !is_reload_event(event.kind) {
					continue;
				}
				let is_close_write = matches!(event.kind, EventKind::Access(AccessKind::Close(AccessMode::Write)));
				for path in &event.paths {
					let path = absolute_path(path);
					if let Some(asset_path) = self.game_asset_path(&path)
						&& is_texture_asset(&asset_path)
					{
						texture_assets.insert(AssetRef::game(asset_path));
					}
					script_source_changed |= is_script_source(&path);
					if is_close_write {
						script_output_close_write |= self.is_script_assembly(&path);
						script_output_close_write |= self.is_script_runtime_config(&path);
					}
				}
			}
		}

		if script_source_changed {
			self.script_status = ScriptStatus::Recompiling;
			self.script_rebuild_pending = true;
		}

		if self.script_rebuild_pending
			&& self.build_child.is_none()
			&& self
				.last_source_change
				.is_none_or(|t| t.elapsed() >= SCRIPT_SOURCE_DEBOUNCE)
		{
			self.last_source_change = Some(Instant::now());
			if let Some(csproj_path) = self.csproj_path.as_deref() {
				if let Some(child) = spawn_dotnet_build(csproj_path) {
					self.build_child = Some(child);
					self.script_rebuild_pending = false;
				} else {
					self.script_status = ScriptStatus::Failed;
				}
			}
		}

		let debounce_ms = self.last_reload_attempt.map_or(9999, |t| t.elapsed().as_millis());
		let enough_time_since_last_attempt = debounce_ms >= SCRIPT_RELOAD_DEBOUNCE_MS;

		if script_output_close_write && enough_time_since_last_attempt {
			let current_mtime = modified_time(&self.script_assembly_path);
			if current_mtime.is_some() && current_mtime != self.last_successful_load_mtime {
				self.reload_scripts_now(scene);
			} else {
				self.last_reload_attempt = Some(Instant::now());
			}
			self.assembly_candidate_mtime = None;
		}

		// Cross-platform fallback: after a successful build, settle-check the
		// assembly mtime directly. Required on Windows where the notify backend
		// (ReadDirectoryChangesW) does not emit Close(Write) events.
		if self.build_child.is_none() && self.script_status == ScriptStatus::Recompiling {
			let current_mtime = modified_time(&self.script_assembly_path);
			if current_mtime.is_some() && current_mtime != self.last_successful_load_mtime {
				if self.assembly_candidate_mtime == current_mtime {
					if enough_time_since_last_attempt {
						self.assembly_candidate_mtime = None;
						self.reload_scripts_now(scene);
					}
				} else {
					self.assembly_candidate_mtime = current_mtime;
				}
			}
		}

		if self.build_child.is_none()
			&& self.script_status == ScriptStatus::Recompiling
			&& modified_time(&self.script_assembly_path) == self.last_successful_load_mtime
		{
			self.script_status = ScriptStatus::UpToDate;
		}

		AssetReloadSet {
			texture_assets: texture_assets.into_iter().collect(),
		}
	}

	fn drain_pending_watcher_events(&self) {
		let Some(events) = &self.events else {
			return;
		};

		for event in events.try_iter() {
			if let Err(error) = event {
				ze_log::error!("asset file watcher error while paused: {error:?}");
			}
		}
	}

	fn game_asset_path(&self, path: &Path) -> Option<String> {
		let path = absolute_path(path);
		let relative = path.strip_prefix(&self.assets_root).ok()?;
		asset_path_string(relative)
	}

	fn is_script_assembly(&self, path: &Path) -> bool { path == self.script_assembly_path }

	fn is_script_runtime_config(&self, path: &Path) -> bool { path == self.script_runtime_config_path }

	fn reload_scripts_now(&mut self, scene: &mut Scene) {
		self.assembly_candidate_mtime = None;
		self.last_reload_attempt = Some(Instant::now());
		match reload_managed_assembly(scene, &self.script_assembly_path, &self.script_runtime_config_path) {
			Ok(()) => {
				self.script_status = ScriptStatus::UpToDate;
				self.last_successful_load_mtime = modified_time(&self.script_assembly_path);
				self.script_classes_dirty = true;
				ze_log::debug!(
					"C# script assembly `{}` reloaded successfully",
					self.script_assembly_path.display()
				);
			}
			Err(error) => {
				self.script_status = ScriptStatus::Failed;
				ze_log::error!(
					"failed to hot reload managed script assembly `{}`: {error:?}",
					self.script_assembly_path.display()
				);
			}
		}
	}

	fn reap_build_child(&mut self) {
		let Some(child) = &mut self.build_child else {
			return;
		};

		match child.try_wait() {
			Ok(Some(status)) => {
				if status.success() {
					if modified_time(&self.script_assembly_path) == self.last_successful_load_mtime {
						self.script_status = ScriptStatus::UpToDate;
					}
				} else {
					ze_log::warn!("dotnet build exited with status {status}");
					if self.script_status == ScriptStatus::Recompiling {
						self.script_status = ScriptStatus::Failed;
					}
				}
				self.build_child = None;
			}
			Ok(None) => {}
			Err(error) => {
				ze_log::error!("failed to poll dotnet build: {error}");
				self.build_child = None;
				if self.script_status == ScriptStatus::Recompiling {
					self.script_status = ScriptStatus::Failed;
				}
			}
		}
	}

	fn kill_build_child(child: &mut Option<Child>) {
		if let Some(mut c) = child.take() {
			let _ = c.kill();
			let _ = c.wait();
		}
	}
}

impl Drop for AssetHotReload {
	fn drop(&mut self) { Self::kill_build_child(&mut self.build_child); }
}

fn create_watcher(
	assets_root: &Path,
	script_assembly_path: &Path,
	csproj_path: Option<&Path>,
) -> (
	Option<notify::RecommendedWatcher>,
	Option<Receiver<notify::Result<Event>>>,
) {
	let (sender, receiver) = channel();
	let Ok(mut watcher) = notify::recommended_watcher(sender) else {
		ze_log::error!("failed to create asset file watcher");
		return (None, None);
	};

	let mut watched = false;
	if assets_root.is_dir() {
		match watcher.watch(assets_root, RecursiveMode::Recursive) {
			Ok(()) => {
				watched = true;
			}
			Err(error) => {
				ze_log::error!(
					"failed to watch project assets directory {}: {error}",
					assets_root.display()
				);
			}
		}
	}

	if let Some(output_dir) = script_assembly_path.parent()
		&& output_dir.is_dir()
		&& !path_starts_with(output_dir, assets_root)
	{
		match watcher.watch(output_dir, RecursiveMode::NonRecursive) {
			Ok(()) => watched = true,
			Err(error) => ze_log::error!(
				"failed to watch script output directory {}: {error}",
				output_dir.display()
			),
		}
	}

	if let Some(script_root) = csproj_path.and_then(Path::parent)
		&& script_root.is_dir()
		&& !path_starts_with(script_root, assets_root)
	{
		match watcher.watch(script_root, RecursiveMode::Recursive) {
			Ok(()) => watched = true,
			Err(error) => ze_log::error!(
				"failed to watch script source directory {}: {error}",
				script_root.display()
			),
		}
	}

	if watched {
		(Some(watcher), Some(receiver))
	} else {
		(None, None)
	}
}

const fn is_reload_event(kind: EventKind) -> bool {
	matches!(
		kind,
		EventKind::Any
			| EventKind::Create(_)
			| EventKind::Modify(_)
			| EventKind::Remove(_)
			| EventKind::Other
			| EventKind::Access(AccessKind::Close(AccessMode::Write))
	)
}

fn is_texture_asset(path: &str) -> bool {
	matches!(
		Path::new(path).extension().and_then(|extension| extension.to_str()),
		Some("jpeg" | "jpg" | "png" | "webp")
	)
}

fn is_script_source(path: &Path) -> bool {
	matches!(
		path.extension().and_then(|extension| extension.to_str()),
		Some("cs" | "csproj")
	) && !path
		.components()
		.any(|component| matches!(component.as_os_str().to_str(), Some("bin" | "obj")))
}

fn spawn_dotnet_build(csproj_path: &Path) -> Option<Child> {
	let mut cmd = Command::new("dotnet");
	cmd.arg("build")
		.arg("--nologo")
		.arg(csproj_path)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped());

	let mut child = match cmd.spawn() {
		Ok(child) => child,
		Err(error) => {
			ze_log::error!("failed to start dotnet build for {}: {error}", csproj_path.display());
			return None;
		}
	};

	let stdout = child.stdout.take();
	let stderr = child.stderr.take();

	thread::Builder::new()
		.name("dotnet-build-out".into())
		.spawn(move || read_build_output(stdout))
		.ok();
	thread::Builder::new()
		.name("dotnet-build-err".into())
		.spawn(move || read_build_output(stderr))
		.ok();

	Some(child)
}

fn read_build_output(stream: Option<impl io::Read + Send + 'static>) {
	use io::BufRead;
	let Some(reader) = stream else {
		return;
	};
	for line in io::BufReader::new(reader).lines() {
		let Ok(line) = line else {
			break;
		};
		if line.contains(": error CS") || line.contains("Build FAILED") {
			ze_log::error!("{line}");
		} else if line.contains(": warning CS") {
			ze_log::warn!("{line}");
		}
	}
}

const SCRIPT_OUTPUT_LOCK_RELEASE_TIMEOUT: Duration = Duration::from_secs(10);
const SCRIPT_OUTPUT_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(50);

fn wait_for_script_output_handles_to_release(assembly_path: &Path, runtime_config_path: &Path) -> io::Result<()> {
	let deps_path = assembly_path.with_extension("deps.json");
	let pdb_path = assembly_path.with_extension("pdb");
	let output_paths = [
		assembly_path,
		runtime_config_path,
		deps_path.as_path(),
		pdb_path.as_path(),
	];
	let deadline = Instant::now() + SCRIPT_OUTPUT_LOCK_RELEASE_TIMEOUT;

	loop {
		let Some(error) = first_locked_output_path(&output_paths) else {
			return Ok(());
		};

		if Instant::now() >= deadline {
			return Err(error);
		}

		thread::sleep(SCRIPT_OUTPUT_LOCK_POLL_INTERVAL);
	}
}

fn first_locked_output_path(paths: &[&Path]) -> Option<io::Error> {
	for path in paths {
		if !path.exists() {
			continue;
		}

		if let Err(error) = open_script_output_for_build(path) {
			return Some(io::Error::new(
				error.kind(),
				format!(
					"timed out waiting for script hot reload to release {}: {error}",
					path.display()
				),
			));
		}
	}

	None
}

fn open_script_output_for_build(path: &Path) -> io::Result<()> {
	OpenOptions::new().read(true).write(true).open(path).map(drop)
}

pub fn reload_managed_assembly(
	scene: &mut Scene,
	assembly_path: &Path,
	runtime_config_path: &Path,
) -> ze_core::Result<()> {
	let runtime = scene
		.with_system_mut::<ScriptingSystem, _>(|scripting, _scene| {
			scripting.reload_from_paths(assembly_path.to_path_buf(), runtime_config_path.to_path_buf())?;
			Ok(scripting.runtime())
		})
		.unwrap_or_else(|| Err(ze_core::anyhow!("scene does not contain a C# scripting system")))?;

	let pruned = sync_scene_script_fields(scene, &runtime)?;
	if pruned > 0 {
		ze_log::debug!("removed {pruned} stale C# script field value(s) after hot reload");
	}

	Ok(())
}

fn asset_path_string(path: &Path) -> Option<String> {
	let mut parts = Vec::new();
	for component in path.components() {
		let Component::Normal(part) = component else {
			return None;
		};
		parts.push(part.to_str()?);
	}

	(!parts.is_empty()).then(|| parts.join("/"))
}

fn path_starts_with(path: &Path, root: &Path) -> bool { path.strip_prefix(root).is_ok() }

fn modified_time(path: &Path) -> Option<SystemTime> { fs::metadata(path).and_then(|metadata| metadata.modified()).ok() }

fn absolute_path(path: impl AsRef<Path>) -> PathBuf {
	let path = path.as_ref();
	if path.is_absolute() {
		return path.to_path_buf();
	}

	std::env::current_dir().map_or_else(|_| path.to_path_buf(), |directory| directory.join(path))
}
