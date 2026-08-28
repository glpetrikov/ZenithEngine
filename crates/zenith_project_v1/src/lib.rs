use std::{
	fs::File,
	io::{Read, Write},
	path::{Path, PathBuf},
};

use zenith_error::{ZPubResult, ZenithError};
use zenith_project_trait::ProjectTrait;
use zenith_registry::ComponentRegistry;
use zenith_types::{
	DEFAULT_PROJECT_DIRS, PROJECT_GITIGNORE, ZENITH_PROJECT_EXTENSION, ZENITH_VERSION, ZENITH_WORLD_EXTENSION,
	ecs::SaveWorld,
	paths::WorldPath,
	project::{Project, ZenithProject},
};
use zenith_world::World;

pub struct ProjectV1 {
	pub name: String,
	pub path: PathBuf,
	pub engine_version: zenith_types::VersionReq,
}

impl ProjectTrait for ProjectV1 {
	fn name(&self) -> &str { &self.name }
	fn engine_version(&self) -> &zenith_types::VersionReq { &self.engine_version }

	fn open(path: &Path, name: &str) -> ZPubResult<Self> {
		let mut zenith_project_file = File::open(path.join(format!("{name}.{ZENITH_PROJECT_EXTENSION}")))?;
		let mut buf = String::new();
		zenith_project_file.read_to_string(&mut buf)?;
		let zenith_project: ZenithProject = toml::from_str(&buf)?;

		// === Checking Default Directories Exist ===
		for dir in DEFAULT_PROJECT_DIRS {
			if !std::fs::exists(path.join(dir))? {
				std::fs::create_dir_all(path.join(dir))?;
			}
		}
		if !std::fs::exists(path.join(".gitignore"))? {
			let mut file = std::fs::File::create(path.join(".gitignore"))?;
			file.write_all(PROJECT_GITIGNORE)?;
		}

		Ok(Self {
			name: zenith_project.project.name,
			path: path.to_path_buf(),
			engine_version: zenith_project.project.engine_version,
		})
	}
	fn create(path: &Path, name: &str) -> ZPubResult<Self> {
		// === Checking existence of project directory ===
		if !std::fs::exists(path)? {
			return Err(ZenithError::InvalidProjectPath(path.to_path_buf()));
		}

		// === Validating project name ===
		if !is_valid_project_name(name) {
			return Err(ZenithError::BadProjectName(name.to_string()));
		}

		// === Create the project directory ===
		let project_root = path.join(name);
		std::fs::create_dir(&project_root)?;

		// === Create the file of version ===
		std::fs::File::create(project_root.join(".v1"))?;

		// === Create the project file ===
		let engine_version = zenith_types::VersionReq::parse(ZENITH_VERSION)?;
		let mut zenith_project_file =
			std::fs::File::create(project_root.join(format!("{}.{}", name, zenith_types::ZENITH_PROJECT_EXTENSION)))?;
		zenith_project_file.write_all(
			toml::to_string(&zenith_types::project::ZenithProject {
				project: Project {
					name: name.to_string(),
					description: None,
					project_version: zenith_types::Version::new(0, 1, 0),
					engine_version: engine_version.clone(),
					version: 1,
				},
			})?
			.as_bytes(),
		)?;

		// === Create the gitignore file ===
		let mut gitignore_file = std::fs::File::create(project_root.join(".gitignore"))?;
		gitignore_file.write_all(PROJECT_GITIGNORE)?;

		// === Creating Assets, Assets/Worlds, Settings, Packages, Temp, Trash
		// directories ===
		for dir in DEFAULT_PROJECT_DIRS {
			std::fs::create_dir_all(project_root.join(dir))?;
		}

		Ok(Self {
			name: name.to_string(),
			path: project_root,
			engine_version,
		})
	}

	fn load_world(&self, path: &WorldPath) -> ZPubResult<World> {
		let mut registry = ComponentRegistry::new();
		ComponentRegistry::register_defaults(&mut registry);
		Self::load_world_with_registry(self, path, registry)
	}
	fn load_world_with_registry(
		&self,
		path: &WorldPath,
		registry: zenith_registry::ComponentRegistry,
	) -> ZPubResult<World> {
		let yaml_string = std::fs::read_to_string(self.path.join("Assets").join(path.as_path()))?;
		let save_file: SaveWorld = serde_saphyr::from_str(&yaml_string)?;
		let name = path
			.as_path()
			.file_name()
			.and_then(|name| name.to_str())
			.and_then(|name| name.strip_suffix(&format!(".{ZENITH_WORLD_EXTENSION}")))
			.ok_or_else(|| ZenithError::InvalidWorldPath(path.as_path().to_path_buf()))?
			.to_string();

		let mut world = World::from_registry(&name, registry);
		world.restore_snapshot(save_file)?;
		Ok(world)
	}
	fn save_world(&self, path: &WorldPath, world: &mut World) -> ZPubResult<()> {
		let destination = self.path.join("Assets").join(path.as_path());
		if let Some(parent) = destination.parent() {
			std::fs::create_dir_all(parent)?;
		}

		let yaml = serde_saphyr::to_string(&world.snapshot())?;
		std::fs::write(destination, yaml)?;

		Ok(())
	}
	fn copy_world(&self, path: &WorldPath, new_path: &WorldPath) -> ZPubResult<()> {
		let destination = self.path.join("Assets").join(new_path.as_path());
		if let Some(parent) = destination.parent() {
			std::fs::create_dir_all(parent)?;
		}
		std::fs::copy(self.path.join("Assets").join(path.as_path()), destination)?;
		Ok(())
	}

	fn move_world(&self, path: &WorldPath, new_path: &WorldPath) -> ZPubResult<()> {
		let destination = self.path.join("Assets").join(new_path.as_path());
		if let Some(parent) = destination.parent() {
			std::fs::create_dir_all(parent)?;
		}
		std::fs::rename(self.path.join("Assets").join(path.as_path()), destination)?;
		Ok(())
	}
	fn has_world(&self, path: &WorldPath) -> ZPubResult<bool> {
		Ok(std::fs::exists(self.path.join("Assets").join(path.as_path()))?)
	}
	fn delete_world(&self, path: &WorldPath) -> ZPubResult<()> {
		let source = self.path.join("Assets").join(path.as_path());
		let destination = self.path.join("Trash").join(path.as_path());

		if let Some(parent) = destination.parent() {
			std::fs::create_dir_all(parent)?;
		}

		std::fs::rename(source, destination)?;
		Ok(())
	}
	fn list_worlds(&self) -> Vec<ZPubResult<WorldPath>> {
		zenith_types::walk_files(&self.path.join("Assets").join("Worlds"))
			.into_iter()
			.map(|entry| {
				// walk_files is rooted at Assets/Worlds, so entries come back
				// relative to that root — WorldPath requires the leading
				// "Worlds" segment itself, so it has to be re-added here.
				entry.and_then(|relative| WorldPath::new(Path::new("Worlds").join(relative)))
			})
			.collect()
	}
}

const WINDOWS_RESERVED: &[&str] = &[
	"CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
	"LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn is_valid_project_name(name: &str) -> bool {
	let charset_ok = !name.is_empty()
		&& name.chars().count() <= 64
		&& name
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');

	let no_leading_or_trailing_dot = !name.starts_with('.') && !name.ends_with('.');
	let no_double_dot = !name.contains("..");

	// Windows reserves a device name by its base name — the part before the
	// first dot — regardless of what follows, so "CON.txt" is just as
	// reserved as bare "CON".
	let base_name = name.split('.').next().unwrap_or(name);
	let windows_reserved = WINDOWS_RESERVED
		.iter()
		.any(|reserved| base_name.eq_ignore_ascii_case(reserved));

	charset_ok && no_leading_or_trailing_dot && no_double_dot && !windows_reserved
}
