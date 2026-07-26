use ze_assets::CellId;
use ze_core::{Quat, Vec2, Vec3};
use ze_ecs::{
	Children, Collider, ColliderShape, EntitiesView, EntityId, Parent, RigidBody, RigidBodyType, Scene,
	TileGridPosition, TileWorldSettings, TilesRoot, Transform, ze_entity_id::ZeEntityId,
};
use ze_renderer::{Sprite, SpriteColorSettings, SpriteSettings, SpriteSize, TextureSource};

/// Finds the scene's singleton `TileWorldSettings` entity, if one exists.
/// Same linear-scan-for-a-singleton-component convention used for
/// `PhysicsSettings` elsewhere in this editor.
pub fn tile_world_settings_entity(scene: &Scene) -> Option<EntityId> {
	let world = scene.world();
	let mut matching_entity = None;

	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&TileWorldSettings>(entity).is_ok() {
				matching_entity = Some(entity);
				break;
			}
		}
	});

	matching_entity
}

/// Finds the singleton `TileWorldSettings` entity, creating one with the
/// default `tile_world_size` (1.0) if none exists yet -- lazy find-or-create,
/// same idiom `RenderSystem::toggle_debug_draw` uses for `PhysicsSettings`.
pub fn ensure_tile_world_settings(scene: &mut Scene) -> EntityId {
	if let Some(entity) = tile_world_settings_entity(scene) {
		return entity;
	}

	let entity = scene.create_entity("TileWorldSettings");
	scene.entity_mut(entity).add_component(TileWorldSettings::default());
	entity
}

pub fn tile_world_size(scene: &Scene) -> f32 {
	tile_world_settings_entity(scene)
		.and_then(|entity| {
			scene
				.world()
				.get::<&TileWorldSettings>(entity)
				.ok()
				.map(|settings| settings.tile_world_size)
		})
		.unwrap_or(1.0)
}

/// Defines the tile-grid world-origin convention (none existed before this
/// feature): tile `(col, row)`'s center sits at `(col, row) * tile_world_size`,
/// so the grid origin is cell `(0, 0)`'s center and `row` increases along `+Y`.
pub fn tile_world_position(col: i32, row: i32, tile_world_size: f32) -> Vec3 {
	Vec3::new(col as f32 * tile_world_size, row as f32 * tile_world_size, 0.0)
}

/// Linear scan for the entity (if any) whose `TileGridPosition` matches
/// `(col, row)` -- mirrors the singleton-lookup idiom used elsewhere in this
/// codebase (`physics_settings_entity`, `primary_camera_entity`), just
/// matching on a field instead of "any entity with this component".
pub fn find_tile_at(scene: &Scene, col: i32, row: i32) -> Option<EntityId> {
	let world = scene.world();
	let mut found = None;

	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			if let Ok(position) = world.get::<&TileGridPosition>(entity)
				&& position.col == col
				&& position.row == row
			{
				found = Some(entity);
				break;
			}
		}
	});

	found
}

/// Finds the scene's singleton `TilesRoot` entity, if one exists. Same
/// linear-scan convention as `tile_world_settings_entity`.
pub fn tiles_root_entity(scene: &Scene) -> Option<EntityId> {
	let world = scene.world();
	let mut matching_entity = None;

	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&TilesRoot>(entity).is_ok() {
				matching_entity = Some(entity);
				break;
			}
		}
	});

	matching_entity
}

/// Finds (or creates) the singleton `Tiles` root entity that newly-placed
/// tiles are parented under (flat -- no further sub-grouping) -- same lazy
/// find-or-create idiom as `ensure_tile_world_settings`. Purely a
/// Hierarchy-panel organization aid; tiles placed before this feature existed
/// are not retroactively reparented here.
pub fn ensure_tiles_root(scene: &mut Scene) -> EntityId {
	if let Some(entity) = tiles_root_entity(scene) {
		return entity;
	}

	let entity = scene.create_entity("Tiles");
	scene.entity_mut(entity).add_component(TilesRoot);
	entity
}

/// Parents `child` under `parent`, mirroring
/// `ZeroEditor/src/panels/hierarchy.rs`'s `attach_to_parent`/
/// `add_child_reference` (kept as a local copy rather than a shared export,
/// since that module's version also handles world-space-preserving
/// reparenting and cycle checks that tile parenting -- always a
/// freshly-created child with no existing parent/transform history --
/// doesn't need).
fn attach_to_parent(scene: &mut Scene, child: EntityId, parent: EntityId) {
	scene.world_mut().add_component(
		child,
		(Parent {
			id: ZeEntityId::from(parent),
		},),
	);

	if let Ok(mut children) = scene.world_mut().get::<&mut Children>(parent) {
		if !children.ids.iter().any(|id| EntityId::from(*id) == child) {
			children.ids.push(ZeEntityId::from(child));
		}
	} else {
		scene.world_mut().add_component(
			parent,
			(Children {
				ids: vec![ZeEntityId::from(child)],
			},),
		);
	}
}

pub struct PlaceTileParams<'a> {
	pub sheet_path: &'a str,
	pub cell_id: CellId,
	pub col: i32,
	pub row: i32,
	/// Written directly into the placed tile's `Sprite.settings.layer` --
	/// the existing render-order field sprites already have, reused here
	/// rather than adding a parallel concept. Higher draws on top; see
	/// wherever `SpriteSettings.layer` is otherwise consumed by the renderer.
	pub render_layer: i32,
	pub with_collision: bool,
}

/// Places one tile at `(col, row)`, replacing any tile already there (no
/// stacking). Returns `false` (a no-op) if an identical tile -- same
/// sheet/cell/render-layer/collision setting -- already occupies the cell,
/// so repainting the same cell mid-drag doesn't spuriously mark the paint
/// stroke's undo batch dirty. The new tile is parented under `Tiles` (see
/// `ensure_tiles_root`) purely to keep the Hierarchy panel navigable.
pub fn place_tile(scene: &mut Scene, params: &PlaceTileParams<'_>) -> bool {
	if let Some(existing) = find_tile_at(scene, params.col, params.row) {
		if tile_matches(scene, existing, params) {
			return false;
		}
		scene.destroy_entity(existing);
	}

	let size = tile_world_size(scene);
	let entity = scene.create_entity("Tile");
	scene.entity_mut(entity).add_component(Transform {
		position: tile_world_position(params.col, params.row, size),
		scale: Vec3::new(size, size, 1.0),
		rotation: Quat::IDENTITY,
	});
	// Forced 1x1 Custom size (not Auto): the tile's on-screen world size must
	// come entirely from Transform.scale so a non-square trimmed cell doesn't
	// break grid alignment, and so rescaling is a pure function of
	// (col, row, tile_world_size).
	scene.entity_mut(entity).add_component(Sprite {
		texture: TextureSource::SheetCell {
			sheet_path: params.sheet_path.to_string(),
			cell_id: params.cell_id,
		},
		size: SpriteSize::Custom {
			width: 1.0,
			height: 1.0,
		},
		color: SpriteColorSettings::default(),
		settings: SpriteSettings {
			layer: params.render_layer,
			..SpriteSettings::default()
		},
		glow_strength: 0.0,
	});
	scene.entity_mut(entity).add_component(TileGridPosition {
		col: params.col,
		row: params.row,
	});
	if params.with_collision {
		// `ze_physics::sync_scene_bodies` only registers an entity with rapier
		// when it has `Transform` + `RigidBody` + `Collider` together (see
		// `ze_physics/src/lib.rs`'s `get::<(&Transform, &RigidBody, &Collider)>`)
		// -- a `Collider` with no paired `RigidBody` is stored in the scene and
		// shows up in the Inspector, but is silently skipped by physics and
		// never actually collides with anything. Tiles are static geometry, so
		// the paired body is `RigidBodyType::Static`.
		scene.entity_mut(entity).add_component(RigidBody {
			body_type: RigidBodyType::Static,
			..RigidBody::default()
		});
		scene.entity_mut(entity).add_component(Collider {
			shape: ColliderShape::Box {
				half_extents: Vec2::splat(size / 2.0),
			},
			..Collider::default()
		});
	}

	let tiles_root = ensure_tiles_root(scene);
	attach_to_parent(scene, entity, tiles_root);

	true
}

/// Erases the tile (if any) at `(col, row)`, mirroring `place_tile`'s
/// bool-return convention: returns `false` (a no-op) if the cell was already
/// empty, so repeatedly erasing an already-cleared cell mid-drag doesn't
/// spuriously mark the erase stroke's undo batch dirty.
pub fn erase_tile(scene: &mut Scene, col: i32, row: i32) -> bool {
	let Some(existing) = find_tile_at(scene, col, row) else {
		return false;
	};
	scene.destroy_entity(existing);
	true
}

fn tile_matches(scene: &Scene, entity: EntityId, params: &PlaceTileParams<'_>) -> bool {
	let world = scene.world();
	let Ok(sprite) = world.get::<&Sprite>(entity) else {
		return false;
	};
	let matches_texture = matches!(
		&sprite.texture,
		TextureSource::SheetCell { sheet_path, cell_id }
			if sheet_path == params.sheet_path && *cell_id == params.cell_id
	);
	let matches_layer = sprite.settings.layer == params.render_layer;
	let has_collider = world.get::<&Collider>(entity).is_ok();

	matches_texture && matches_layer && has_collider == params.with_collision
}

/// Rewrites every `TileGridPosition` entity's `Transform.position`/`scale`
/// (and `Collider.half_extents`, if it has a box collider) to match
/// `new_tile_world_size`. Editor-time, on-change operation -- called
/// directly from the tile-size setting's UI when it changes, not a
/// per-frame system.
pub fn rescale_tiles(scene: &mut Scene, new_tile_world_size: f32) {
	let mut tiles: Vec<(EntityId, i32, i32)> = Vec::new();
	{
		let world = scene.world();
		world.run(|entities: EntitiesView| {
			for entity in entities.iter() {
				if let Ok(position) = world.get::<&TileGridPosition>(entity) {
					tiles.push((entity, position.col, position.row));
				}
			}
		});
	}

	for (entity, col, row) in tiles {
		if let Ok(mut transform) = scene.world_mut().get::<&mut Transform>(entity) {
			transform.position = tile_world_position(col, row, new_tile_world_size);
			transform.scale = Vec3::new(new_tile_world_size, new_tile_world_size, transform.scale.z);
		}
		if let Ok(mut collider) = scene.world_mut().get::<&mut Collider>(entity)
			&& let ColliderShape::Box { half_extents } = &mut collider.shape
		{
			*half_extents = Vec2::splat(new_tile_world_size / 2.0);
		}
	}
}
