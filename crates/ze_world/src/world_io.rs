use std::{collections::HashMap, fs, path::Path};

use bevy_ecs::entity::Entity;
use tracing::instrument;
use ze_entity_id::ZeEntityId;
use ze_error::{WrapErr, ZResult as Result, eyre};
use ze_registry::ComponentRegistry;
use ze_types::ecs::SaveWorld;

use crate::{WORLD_EXTENSION, WORLD_VERSION, World};

impl World {
	#[instrument(skip(self))]
	pub fn snapshot(&mut self) -> SaveWorld {
		let mut query = self.world.query::<Entity>();
		let entities = query
			.iter(&self.world)
			.map(|entity| self.registry.save_entity(entity, &self.world))
			.collect();

		SaveWorld {
			name: self.display_name.clone(),
			world_type: self.world_type.clone(),
			version: WORLD_VERSION.to_string(),
			entities,
		}
	}

	#[instrument(skip(self, snapshot))]
	pub fn restore_snapshot(&mut self, snapshot: SaveWorld) -> Result<()> {
		self.display_name = if snapshot.name.is_empty() {
			self.name.clone()
		} else {
			snapshot.name.clone()
		};
		self.world_type = snapshot.world_type;
		self.clear_world();

		let mut remap: HashMap<ZeEntityId, Entity> = HashMap::with_capacity(snapshot.entities.len());
		for saved_entity in &snapshot.entities {
			let new_entity = self.world.spawn_empty().id();
			remap.insert(saved_entity.id.clone(), new_entity);
		}

		for saved_entity in snapshot.entities {
			let new_entity = remap[&saved_entity.id];
			for component in saved_entity.components {
				self.registry
					.load_component(new_entity, &mut self.world, component)
					.map_err(|error| eyre!("failed to load component while restoring snapshot: {error}"))?;
			}
		}

		Ok(())
	}

	#[instrument]
	pub fn from_path(path: impl AsRef<Path> + std::fmt::Debug) -> Result<Self> {
		let registry = ComponentRegistry::new();
		Self::from_path_with_registry(path, registry)
	}

	#[instrument(skip(registry))]
	pub fn from_path_with_registry(
		path: impl AsRef<Path> + std::fmt::Debug,
		mut registry: ComponentRegistry,
	) -> Result<Self> {
		// register_defaults() is idempotent (plain map insert), so calling
		// it here unconditionally is safe even if the caller already
		// registered some/all defaults themselves -- this way the function
		// is correct by construction rather than depending on every caller
		// remembering to do it first.
		Self::register_defaults(&mut registry);

		let path = path.as_ref();
		let yaml_text =
			fs::read_to_string(path).wrap_err_with(|| format!("failed to read world file at {}", path.display()))?;
		let save_file: SaveWorld = serde_saphyr::from_str(&yaml_text).wrap_err("invalid world file")?;

		let name = path
			.file_name()
			.and_then(|name| name.to_str())
			.and_then(|name| name.strip_suffix(&format!(".{WORLD_EXTENSION}")))
			.unwrap_or_default()
			.to_string();

		let mut world = Self::from_registry(&name, registry);
		world.restore_snapshot(save_file)?;
		Ok(world)
	}

	#[instrument(skip(self))]
	pub fn save(&mut self, directory: impl AsRef<Path> + std::fmt::Debug, file_name: &str) -> Result<()> {
		let directory = directory.as_ref();
		fs::create_dir_all(directory)?;

		let path = scene_path(directory, file_name);
		let yaml = serde_saphyr::to_string(&self.snapshot()).wrap_err("failed to serialize world to YAML")?;
		fs::write(path, yaml)?;
		Ok(())
	}
}

fn scene_path(directory: &Path, name: &str) -> std::path::PathBuf {
	directory.join(format!("{name}.{WORLD_EXTENSION}"))
}
