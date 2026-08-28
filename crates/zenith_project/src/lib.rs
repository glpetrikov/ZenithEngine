use std::path::Path;

use zenith_error::{ZPubResult, ZenithError};
use zenith_project_trait::ProjectTrait;
use zenith_project_v1::ProjectV1;
use zenith_registry::ComponentRegistry;
use zenith_types::{ZENITH_PROJECT_EXTENSION, paths::WorldPath};
use zenith_world::World;

pub enum ProjectVersion {
	V1(ProjectV1),
}

pub struct Project {
	pub state: ProjectVersion,
}

impl ProjectTrait for Project {
	fn name(&self) -> &str {
		match &self.state {
			ProjectVersion::V1(p) => p.name(),
		}
	}
	fn engine_version(&self) -> &zenith_types::VersionReq {
		match &self.state {
			ProjectVersion::V1(p) => p.engine_version(),
		}
	}

	fn open(path: &Path, name: &str) -> ZPubResult<Self> {
		if !std::fs::exists(path)? {
			return Err(ZenithError::InvalidProjectPath(path.to_path_buf()));
		}

		if !std::fs::exists(path.join(format!("{name}.{ZENITH_PROJECT_EXTENSION}")))? {
			return Err(ZenithError::InvalidProject(format!(
				"Project file ({name}.{ZENITH_PROJECT_EXTENSION}) not found"
			)));
		}
		if std::fs::exists(path.join(".v1"))? {
			ProjectV1::open(path, name).map(|p| Self {
				state: ProjectVersion::V1(p),
			})
		} else {
			Err(ZenithError::InvalidProject(
				"Project Version file (.v1) not found".to_string(),
			))
		}
	}
	fn create(path: &Path, name: &str) -> ZPubResult<Self> {
		ProjectV1::create(path, name).map(|p| Self {
			state: ProjectVersion::V1(p),
		})
	}

	fn load_world(&self, path: &WorldPath) -> ZPubResult<World> {
		match &self.state {
			ProjectVersion::V1(p) => p.load_world(path),
		}
	}
	fn load_world_with_registry(&self, path: &WorldPath, registry: ComponentRegistry) -> ZPubResult<World> {
		match &self.state {
			ProjectVersion::V1(p) => p.load_world_with_registry(path, registry),
		}
	}
	fn save_world(&self, path: &WorldPath, world: &mut World) -> ZPubResult<()> {
		match &self.state {
			ProjectVersion::V1(p) => p.save_world(path, world),
		}
	}
	fn copy_world(&self, path: &WorldPath, new_path: &WorldPath) -> ZPubResult<()> {
		match &self.state {
			ProjectVersion::V1(p) => p.copy_world(path, new_path),
		}
	}
	fn move_world(&self, path: &WorldPath, new_path: &WorldPath) -> ZPubResult<()> {
		match &self.state {
			ProjectVersion::V1(p) => p.move_world(path, new_path),
		}
	}
	fn has_world(&self, path: &WorldPath) -> ZPubResult<bool> {
		match &self.state {
			ProjectVersion::V1(p) => p.has_world(path),
		}
	}
	fn delete_world(&self, path: &WorldPath) -> ZPubResult<()> {
		match &self.state {
			ProjectVersion::V1(p) => p.delete_world(path),
		}
	}
	fn list_worlds(&self) -> Vec<ZPubResult<WorldPath>> {
		match &self.state {
			ProjectVersion::V1(p) => p.list_worlds(),
		}
	}
}
