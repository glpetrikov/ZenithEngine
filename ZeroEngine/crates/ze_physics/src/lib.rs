use std::{
	collections::{HashMap, HashSet},
	sync::mpsc::{Receiver, Sender, channel},
};

use rapier2d::prelude::*;
use ze_core::{Quat, Result, Vec2};
use ze_ecs::{
	Collider, ColliderShape, CollisionDetection, EntitiesView, EntityId, Inactive, PhysicsSettings, RigidBody,
	RigidBodyType, Scene, System, Transform,
};
use ze_scripting_cs::{
	RaycastHit2D, ScriptingApiCommand, ScriptingRuntimeHandle, clear_raycast_provider, drain_scripting_api_commands,
	refresh_scripting_api_velocity_cache, set_raycast_provider,
};

pub const DEFAULT_GRAVITY: Vec2 = Vec2::new(0.0, -9.81);
pub const DEFAULT_PHYSICS_TIMESTEP: f32 = 1.0 / 70.0;

pub struct PhysicsWorld {
	pipeline: PhysicsPipeline,
	gravity: Vector,
	integration_parameters: IntegrationParameters,
	island_manager: IslandManager,
	broad_phase: BroadPhaseBvh,
	narrow_phase: NarrowPhase,
	pub rigid_bodies: RigidBodySet,
	pub colliders: ColliderSet,
	impulse_joints: ImpulseJointSet,
	multibody_joints: MultibodyJointSet,
	ccd_solver: CCDSolver,
	entity_bodies: HashMap<EntityId, PhysicsBodyEntry>,
	entity_colliders: HashMap<EntityId, ColliderHandle>,
	collider_entities: HashMap<ColliderHandle, EntityId>,
	active_contact_pairs: HashSet<ColliderPairKey>,
	active_sensor_pairs: HashSet<ColliderPairKey>,
	collision_event_sender: Sender<CollisionEvent>,
	collision_event_receiver: Receiver<CollisionEvent>,
	contact_force_event_sender: Sender<ContactForceEvent>,
	contact_force_event_receiver: Receiver<ContactForceEvent>,
}

#[derive(Debug, Clone, Copy)]
struct PhysicsBodyEntry {
	handle: RigidBodyHandle,
	body_type: RigidBodyType,
}

type ColliderPairKey = (ColliderHandle, ColliderHandle);

impl PhysicsWorld {
	pub fn new() -> Self {
		let (collision_event_sender, collision_event_receiver) = channel();
		let (contact_force_event_sender, contact_force_event_receiver) = channel();

		Self {
			pipeline: PhysicsPipeline::new(),
			gravity: Vector::new(DEFAULT_GRAVITY.x, DEFAULT_GRAVITY.y),
			integration_parameters: IntegrationParameters::default(),
			island_manager: IslandManager::new(),
			broad_phase: BroadPhaseBvh::new(),
			narrow_phase: NarrowPhase::new(),
			rigid_bodies: RigidBodySet::new(),
			colliders: ColliderSet::new(),
			impulse_joints: ImpulseJointSet::new(),
			multibody_joints: MultibodyJointSet::new(),
			ccd_solver: CCDSolver::new(),
			entity_bodies: HashMap::new(),
			entity_colliders: HashMap::new(),
			collider_entities: HashMap::new(),
			active_contact_pairs: HashSet::new(),
			active_sensor_pairs: HashSet::new(),
			collision_event_sender,
			collision_event_receiver,
			contact_force_event_sender,
			contact_force_event_receiver,
		}
	}

	pub fn with_gravity(gravity: Vec2) -> Self {
		let mut world = Self::new();
		world.set_gravity(gravity);
		world
	}

	pub const fn gravity(&self) -> Vec2 { Vec2::new(self.gravity.x, self.gravity.y) }

	pub const fn set_gravity(&mut self, gravity: Vec2) { self.gravity = Vector::new(gravity.x, gravity.y); }

	pub fn add_2d_force(&mut self, entity: EntityId, force: Vec2) {
		let Some(entry) = self.entity_bodies.get(&entity).copied() else {
			return;
		};
		let Some(body) = self.rigid_bodies.get_mut(entry.handle) else {
			return;
		};

		body.add_force(Vector::new(force.x, force.y), true);
	}

	pub fn add_2d_impulse(&mut self, entity: EntityId, impulse: Vec2) {
		let Some(entry) = self.entity_bodies.get(&entity).copied() else {
			return;
		};
		let Some(body) = self.rigid_bodies.get_mut(entry.handle) else {
			return;
		};

		body.apply_impulse(Vector::new(impulse.x, impulse.y), true);
	}

	pub fn reset_forces(&mut self) {
		for (_, body) in self.rigid_bodies.iter_mut() {
			body.reset_forces(false);
		}
	}

	pub fn body_velocities(&self) -> Vec<(EntityId, f32, f32)> {
		self.entity_bodies
			.iter()
			.filter_map(|(entity, entry)| {
				self.rigid_bodies.get(entry.handle).map(|body| {
					let velocity = body.linvel();
					(*entity, velocity.x, velocity.y)
				})
			})
			.collect()
	}

	/// Casts a ray through the world and returns the closest hit, if any.
	///
	/// `direction` doesn't need to be a unit vector: it's normalized here so `max_distance`
	/// always means world-space units to the caller, regardless of how they built it.
	pub fn raycast(&self, origin: Vec2, direction: Vec2, max_distance: f32) -> Option<RaycastHit2D> {
		if max_distance <= 0.0 {
			return None;
		}

		let direction = direction.try_normalize()?;
		let ray = Ray::new(Vector::new(origin.x, origin.y), Vector::new(direction.x, direction.y));

		// rapier 0.32's QueryPipeline is a borrowed view derived on demand from the broad-phase,
		// unlike older rapier versions where a separate QueryPipeline had to be synced manually
		// after every step. `broad_phase` is already current here since `step()` updates it.
		let query_pipeline = self.broad_phase.as_query_pipeline(
			self.narrow_phase.query_dispatcher(),
			&self.rigid_bodies,
			&self.colliders,
			QueryFilter::default(),
		);

		let (collider_handle, intersection) = query_pipeline.cast_ray_and_get_normal(&ray, max_distance, true)?;
		let entity = self.entity_for_collider(collider_handle)?;
		let hit_point = ray.point_at(intersection.time_of_impact);

		Some(RaycastHit2D {
			point: Vec2::new(hit_point.x, hit_point.y),
			normal: Vec2::new(intersection.normal.x, intersection.normal.y),
			entity,
		})
	}

	pub fn step(&mut self, dt: f32) -> Vec<CollisionEvent> {
		self.integration_parameters.dt = dt.max(0.0);

		let event_handler = ChannelEventCollector::new(
			self.collision_event_sender.clone(),
			self.contact_force_event_sender.clone(),
		);

		self.pipeline.step(
			self.gravity,
			&self.integration_parameters,
			&mut self.island_manager,
			&mut self.broad_phase,
			&mut self.narrow_phase,
			&mut self.rigid_bodies,
			&mut self.colliders,
			&mut self.impulse_joints,
			&mut self.multibody_joints,
			&mut self.ccd_solver,
			&(),
			&event_handler,
		);

		self.contact_force_event_receiver.try_iter().for_each(drop);
		self.collision_event_receiver.try_iter().collect()
	}

	pub fn register_entity(
		&mut self,
		entity: EntityId,
		rigid_body: &RigidBody,
		collider: &Collider,
		transform: &Transform,
	) {
		if self.entity_bodies.contains_key(&entity) {
			return;
		}

		let body = RigidBodyBuilder::new(to_rapier_body_type(rigid_body.body_type))
			.translation(Vector::new(transform.position.x, transform.position.y))
			.rotation(transform.rotation.to_euler(ze_core::glam::EulerRot::XYZ).2)
			.gravity_scale(if rigid_body.use_gravity {
				rigid_body.gravity_scale
			} else {
				0.0
			})
			.linear_damping(rigid_body.linear_damping)
			.angular_damping(rigid_body.angular_damping)
			.locked_axes(to_locked_axes(rigid_body))
			.ccd_enabled(matches!(rigid_body.collision_detection, CollisionDetection::Continuous))
			.build();

		let body_handle = self.rigid_bodies.insert(body);
		let collider_handle = self.colliders.insert_with_parent(
			build_collider(collider, rigid_body.mass, transform.scale.truncate()),
			body_handle,
			&mut self.rigid_bodies,
		);

		self.entity_bodies.insert(
			entity,
			PhysicsBodyEntry {
				handle: body_handle,
				body_type: rigid_body.body_type,
			},
		);
		self.entity_colliders.insert(entity, collider_handle);
		self.collider_entities.insert(collider_handle, entity);
	}

	pub fn remove_entity(&mut self, entity: EntityId) {
		let Some(entry) = self.entity_bodies.remove(&entity) else {
			return;
		};
		let collider = self.entity_colliders.remove(&entity);
		if let Some(collider) = collider {
			self.collider_entities.remove(&collider);
			self.active_contact_pairs
				.retain(|pair| pair.0 != collider && pair.1 != collider);
			self.active_sensor_pairs
				.retain(|pair| pair.0 != collider && pair.1 != collider);
		}

		self.rigid_bodies.remove(
			entry.handle,
			&mut self.island_manager,
			&mut self.colliders,
			&mut self.impulse_joints,
			&mut self.multibody_joints,
			true,
		);
	}

	pub fn entity_for_collider(&self, collider: ColliderHandle) -> Option<EntityId> {
		self.collider_entities.get(&collider).copied()
	}

	pub fn collider_is_sensor(&self, collider: ColliderHandle) -> bool {
		self.colliders
			.get(collider)
			.is_some_and(rapier2d::geometry::Collider::is_sensor)
	}

	pub fn sync_from_ecs(&mut self, scene: &Scene) {
		let transforms = self
			.entity_bodies
			.iter()
			.filter(|(_, entry)| is_kinematic(entry.body_type))
			.filter_map(|(entity, _)| {
				scene
					.world_transform(*entity)
					.map(|transform| (*entity, transform.position.x, transform.position.y, transform.rotation))
			})
			.collect::<Vec<_>>();

		for (entity, x, y, rotation) in transforms {
			let Some(entry) = self.entity_bodies.get(&entity).copied() else {
				continue;
			};
			let Some(body) = self.rigid_bodies.get_mut(entry.handle) else {
				continue;
			};

			let angle = rotation.to_euler(ze_core::glam::EulerRot::XYZ).2;
			body.set_position(Pose::new(Vector::new(x, y), angle), true);
		}
	}

	pub fn sync_to_ecs(&self, scene: &mut Scene) -> Result<()> {
		let updates = self
			.entity_bodies
			.iter()
			.filter_map(|(entity, entry)| {
				self.rigid_bodies.get(entry.handle).map(|body| {
					let position = body.translation();
					(*entity, position.x, position.y, body.rotation().angle())
				})
			})
			.collect::<Vec<_>>();

		for (entity, x, y, angle) in updates {
			let Some(mut transform) = scene.world_transform(entity) else {
				continue;
			};
			transform.position.x = x;
			transform.position.y = y;
			transform.rotation = Quat::from_rotation_z(angle);
			scene.set_world_transform(entity, transform)?;
		}

		Ok(())
	}
}

impl Default for PhysicsWorld {
	fn default() -> Self { Self::new() }
}

pub struct PhysicsSystem {
	world: PhysicsWorld,
	accumulator: f32,
	scripting: Option<ScriptingRuntimeHandle>,
}

impl PhysicsSystem {
	pub fn new() -> Self {
		Self {
			world: PhysicsWorld::new(),
			accumulator: 0.0,
			scripting: None,
		}
	}

	pub fn with_scripting(scripting: ScriptingRuntimeHandle) -> Self {
		Self {
			world: PhysicsWorld::new(),
			accumulator: 0.0,
			scripting: Some(scripting),
		}
	}

	pub const fn world(&self) -> &PhysicsWorld { &self.world }

	pub const fn world_mut(&mut self) -> &mut PhysicsWorld { &mut self.world }

	pub fn reset(&mut self) {
		self.world = PhysicsWorld::new();
		self.accumulator = 0.0;
	}

	pub fn remove_inactive_entities(&mut self, scene: &Scene) {
		let inactive_entities = self
			.world
			.entity_bodies
			.keys()
			.copied()
			.filter(|entity| scene.world().get::<&Inactive>(*entity).is_ok())
			.collect::<Vec<_>>();

		for entity in inactive_entities {
			self.world.remove_entity(entity);
		}
	}

	fn sync_scene_bodies(&mut self, scene: &Scene) {
		let ecs_world = scene.world();
		let mut active_entities = HashSet::new();
		let mut entities_to_register = Vec::new();

		ecs_world.run(|entities: EntitiesView| {
			for entity in entities.iter() {
				if ecs_world.get::<&Inactive>(entity).is_ok() {
					continue;
				}

				let Ok((_, rigid_body, collider)) = ecs_world.get::<(&Transform, &RigidBody, &Collider)>(entity) else {
					continue;
				};

				let Some(world_transform) = scene.world_transform(entity) else {
					continue;
				};

				active_entities.insert(entity);
				entities_to_register.push((entity, rigid_body.clone(), collider.clone(), world_transform));
			}
		});

		let removed_entities = self
			.world
			.entity_bodies
			.keys()
			.copied()
			.filter(|entity| !active_entities.contains(entity))
			.collect::<Vec<_>>();
		for entity in removed_entities {
			self.world.remove_entity(entity);
		}

		for (entity, rigid_body, collider, transform) in entities_to_register {
			self.world.register_entity(entity, &rigid_body, &collider, &transform);
		}
	}
}

impl Default for PhysicsSystem {
	fn default() -> Self { Self::new() }
}

impl System for PhysicsSystem {
	fn name(&self) -> &'static str { "PhysicsSystem" }

	fn update(&mut self, scene: &mut Scene, dt: f32) -> Result<()> {
		let settings = scene_physics_settings(scene);
		let fixed_dt = settings.physics_timestep.max(f32::EPSILON);
		self.sync_scene_bodies(scene);

		self.accumulator += dt.max(0.0);
		let steps = (self.accumulator / fixed_dt) as usize;
		for _ in 0..steps {
			self.world.set_gravity(settings.gravity);
			self.apply_scripting_api_commands();

			self.world.sync_from_ecs(scene);
			let collision_events = self.world.step(fixed_dt);
			self.world.reset_forces();

			self.world.sync_to_ecs(scene)?;

			if let Some(scripting) = self.scripting.clone() {
				refresh_scripting_api_velocity_cache(self.world.body_velocities());

				// SAFETY: `&self.world` stays valid for the duration of this call frame, and the
				// provider is always cleared before returning (even on error), so no dangling
				// pointer can outlive it -- e.g. across a later `PhysicsSystem::reset()`.
				unsafe {
					set_raycast_provider((&raw const self.world).cast::<()>(), raycast_via_context);
				}
				let fixed_update_result = scripting.fixed_update(scene, fixed_dt);
				clear_raycast_provider();
				fixed_update_result?;

				self.apply_scripting_api_commands();
				self.dispatch_script_collision_events(&scripting, collision_events);
				self.apply_scripting_api_commands();
			}

			self.accumulator -= fixed_dt;
		}

		Ok(())
	}

	fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}

impl PhysicsSystem {
	fn apply_scripting_api_commands(&mut self) {
		for command in drain_scripting_api_commands() {
			match command {
				ScriptingApiCommand::Add2DForce { entity, x, y } => {
					self.world.add_2d_force(entity, Vec2::new(x, y));
				}
				ScriptingApiCommand::Add2DImpulse { entity, x, y } => {
					self.world.add_2d_impulse(entity, Vec2::new(x, y));
				}
			}
		}
	}

	fn dispatch_script_collision_events(
		&mut self,
		scripting: &ScriptingRuntimeHandle,
		collision_events: Vec<CollisionEvent>,
	) {
		let previous_contact_pairs = self.world.active_contact_pairs.clone();
		let previous_sensor_pairs = self.world.active_sensor_pairs.clone();
		let mut contact_events = Vec::new();
		let mut sensor_events = Vec::new();

		for event in collision_events {
			if event.sensor() {
				sensor_events.push(event);
			} else {
				contact_events.push(event);
			}
		}

		for event in contact_events {
			self.dispatch_contact_event(scripting, event.collider1(), event.collider2(), event.started());
		}

		let contact_stays = self
			.world
			.active_contact_pairs
			.iter()
			.copied()
			.filter(|pair| previous_contact_pairs.contains(pair))
			.collect::<Vec<_>>();

		for (collider1, collider2) in contact_stays {
			self.dispatch_contact_stay(scripting, collider1, collider2);
		}

		for event in sensor_events {
			self.dispatch_sensor_event(scripting, event.collider1(), event.collider2(), event.started());
		}

		let sensor_stays = self
			.world
			.active_sensor_pairs
			.iter()
			.copied()
			.filter(|pair| previous_sensor_pairs.contains(pair))
			.collect::<Vec<_>>();

		for (collider1, collider2) in sensor_stays {
			self.dispatch_sensor_stay(scripting, collider1, collider2);
		}
	}

	fn dispatch_contact_event(
		&mut self,
		scripting: &ScriptingRuntimeHandle,
		collider1: ColliderHandle,
		collider2: ColliderHandle,
		started: bool,
	) {
		let Some((entity1, entity2)) = self.entities_for_pair(collider1, collider2) else {
			return;
		};

		let pair = collider_pair_key(collider1, collider2);

		if started {
			self.world.active_contact_pairs.insert(pair);
			scripting.on_contact_enter(entity1, entity2);
			scripting.on_contact_enter(entity2, entity1);
		} else {
			self.world.active_contact_pairs.remove(&pair);
			scripting.on_contact_exit(entity1, entity2);
			scripting.on_contact_exit(entity2, entity1);
		}
	}

	fn dispatch_contact_stay(
		&self,
		scripting: &ScriptingRuntimeHandle,
		collider1: ColliderHandle,
		collider2: ColliderHandle,
	) {
		let Some((entity1, entity2)) = self.entities_for_pair(collider1, collider2) else {
			return;
		};

		scripting.on_contact_stay(entity1, entity2);
		scripting.on_contact_stay(entity2, entity1);
	}

	fn dispatch_sensor_event(
		&mut self,
		scripting: &ScriptingRuntimeHandle,
		collider1: ColliderHandle,
		collider2: ColliderHandle,
		started: bool,
	) {
		let pair = collider_pair_key(collider1, collider2);

		if started {
			self.world.active_sensor_pairs.insert(pair);
			self.dispatch_sensor_enter(scripting, collider1, collider2);
		} else {
			self.world.active_sensor_pairs.remove(&pair);
			self.dispatch_sensor_exit(scripting, collider1, collider2);
		}
	}

	fn dispatch_sensor_stay(
		&self,
		scripting: &ScriptingRuntimeHandle,
		collider1: ColliderHandle,
		collider2: ColliderHandle,
	) {
		self.dispatch_sensor_callbacks(
			scripting,
			collider1,
			collider2,
			ScriptingRuntimeHandle::on_contact_stay,
			ScriptingRuntimeHandle::on_sensor_stay,
		);
	}

	fn dispatch_sensor_enter(
		&self,
		scripting: &ScriptingRuntimeHandle,
		collider1: ColliderHandle,
		collider2: ColliderHandle,
	) {
		self.dispatch_sensor_callbacks(
			scripting,
			collider1,
			collider2,
			ScriptingRuntimeHandle::on_contact_enter,
			ScriptingRuntimeHandle::on_sensor_enter,
		);
	}

	fn dispatch_sensor_exit(
		&self,
		scripting: &ScriptingRuntimeHandle,
		collider1: ColliderHandle,
		collider2: ColliderHandle,
	) {
		self.dispatch_sensor_callbacks(
			scripting,
			collider1,
			collider2,
			ScriptingRuntimeHandle::on_contact_exit,
			ScriptingRuntimeHandle::on_sensor_exit,
		);
	}

	fn dispatch_sensor_callbacks(
		&self,
		scripting: &ScriptingRuntimeHandle,
		collider1: ColliderHandle,
		collider2: ColliderHandle,
		contact_callback: fn(&ScriptingRuntimeHandle, EntityId, EntityId),
		sensor_callback: fn(&ScriptingRuntimeHandle, EntityId, EntityId),
	) {
		let Some((entity1, entity2)) = self.entities_for_pair(collider1, collider2) else {
			return;
		};

		let collider1_is_sensor = self.world.collider_is_sensor(collider1);
		let collider2_is_sensor = self.world.collider_is_sensor(collider2);

		match (collider1_is_sensor, collider2_is_sensor) {
			(true, false) => {
				contact_callback(scripting, entity1, entity2);
				sensor_callback(scripting, entity2, entity1);
			}
			(false, true) => {
				contact_callback(scripting, entity2, entity1);
				sensor_callback(scripting, entity1, entity2);
			}
			(true, true) => {
				contact_callback(scripting, entity1, entity2);
				contact_callback(scripting, entity2, entity1);
				sensor_callback(scripting, entity1, entity2);
				sensor_callback(scripting, entity2, entity1);
			}
			(false, false) => {}
		}
	}

	fn entities_for_pair(&self, collider1: ColliderHandle, collider2: ColliderHandle) -> Option<(EntityId, EntityId)> {
		Some((
			self.world.entity_for_collider(collider1)?,
			self.world.entity_for_collider(collider2)?,
		))
	}
}

/// Trampoline registered with `ze_scripting_cs::set_raycast_provider`; see the safety note at
/// its call site in `PhysicsSystem::update` for why the raw pointer is sound here.
fn raycast_via_context(context: *const (), origin: Vec2, direction: Vec2, max_distance: f32) -> Option<RaycastHit2D> {
	let world = unsafe { &*context.cast::<PhysicsWorld>() };
	world.raycast(origin, direction, max_distance)
}

fn collider_pair_key(collider1: ColliderHandle, collider2: ColliderHandle) -> ColliderPairKey {
	if collider1.into_raw_parts() <= collider2.into_raw_parts() {
		(collider1, collider2)
	} else {
		(collider2, collider1)
	}
}

fn scene_physics_settings(scene: &Scene) -> PhysicsStepSettings {
	let world = scene.world();
	let mut settings = PhysicsStepSettings::default();

	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			let Ok(physics_settings) = world.get::<&PhysicsSettings>(entity) else {
				continue;
			};

			settings.gravity = physics_settings.gravity;
			settings.physics_timestep = physics_settings.physics_timestep;
			break;
		}
	});

	settings
}

#[derive(Clone, Copy)]
struct PhysicsStepSettings {
	gravity: Vec2,
	physics_timestep: f32,
}

impl Default for PhysicsStepSettings {
	fn default() -> Self {
		Self {
			gravity: DEFAULT_GRAVITY,
			physics_timestep: DEFAULT_PHYSICS_TIMESTEP,
		}
	}
}

const fn to_rapier_body_type(body_type: RigidBodyType) -> rapier2d::dynamics::RigidBodyType {
	match body_type {
		RigidBodyType::Static => rapier2d::dynamics::RigidBodyType::Fixed,
		RigidBodyType::Dynamic => rapier2d::dynamics::RigidBodyType::Dynamic,
		RigidBodyType::KinematicPositionBased => rapier2d::dynamics::RigidBodyType::KinematicPositionBased,
		RigidBodyType::KinematicVelocityBased => rapier2d::dynamics::RigidBodyType::KinematicVelocityBased,
	}
}

const fn is_kinematic(body_type: RigidBodyType) -> bool {
	matches!(
		body_type,
		RigidBodyType::KinematicPositionBased | RigidBodyType::KinematicVelocityBased
	)
}

fn to_locked_axes(rigid_body: &RigidBody) -> LockedAxes {
	let mut axes = LockedAxes::empty();

	axes.set(LockedAxes::TRANSLATION_LOCKED_X, rigid_body.freeze_position_x);
	axes.set(LockedAxes::TRANSLATION_LOCKED_Y, rigid_body.freeze_position_y);
	axes.set(LockedAxes::ROTATION_LOCKED_X, rigid_body.freeze_rotation_x);
	axes.set(LockedAxes::ROTATION_LOCKED_Y, rigid_body.freeze_rotation_y);
	axes.set(LockedAxes::ROTATION_LOCKED_Z, rigid_body.freeze_rotation_z);

	axes
}

fn build_collider(collider: &Collider, mass: Option<f32>, scale: Vec2) -> rapier2d::geometry::Collider {
	let scale = scale.abs().max(Vec2::splat(f32::EPSILON));
	let builder = match collider.shape {
		ColliderShape::Box { half_extents } => {
			let half_extents = half_extents * scale;
			ColliderBuilder::cuboid(half_extents.x, half_extents.y)
		}
		ColliderShape::Circle { radius } => ColliderBuilder::ball(radius * scale.max_element()),
		ColliderShape::Capsule { half_height, radius } => {
			ColliderBuilder::capsule_y(half_height * scale.y, radius * scale.max_element())
		}
		ColliderShape::ConvexPolygon { ref points } => {
			let scaled_points = points
				.iter()
				.map(|point| Vector::new(point.x * scale.x, point.y * scale.y))
				.collect::<Vec<_>>();

			ColliderBuilder::convex_hull(&scaled_points).unwrap_or_else(|| ColliderBuilder::cuboid(0.5, 0.5))
		}
	};

	let builder = builder
		.restitution(collider.restitution)
		.friction(collider.friction)
		.sensor(collider.is_sensor)
		.active_events(ActiveEvents::COLLISION_EVENTS);

	let builder = if let Some(mass) = mass {
		builder.mass(mass)
	} else {
		builder.density(collider.density)
	};

	builder.build()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn inactive_entities_do_not_participate_in_physics() -> Result<()> {
		let mut scene = Scene::new("Inactive Physics Test");
		let entity = scene.create_entity("Body");
		scene
			.entity_mut(entity)
			.add_component(Transform::default())
			.add_component(RigidBody::default())
			.add_component(Collider::default())
			.add_component(Inactive);

		let mut physics = PhysicsSystem::new();
		physics.update(&mut scene, DEFAULT_PHYSICS_TIMESTEP)?;
		assert_eq!(physics.world().rigid_bodies.len(), 0);
		assert_eq!(physics.world().colliders.len(), 0);

		scene.entity_mut(entity).remove_component::<Inactive>()?;
		physics.update(&mut scene, DEFAULT_PHYSICS_TIMESTEP)?;
		assert_eq!(physics.world().rigid_bodies.len(), 1);
		assert_eq!(physics.world().colliders.len(), 1);

		scene.entity_mut(entity).add_component(Inactive);
		physics.update(&mut scene, DEFAULT_PHYSICS_TIMESTEP)?;
		assert_eq!(physics.world().rigid_bodies.len(), 0);
		assert_eq!(physics.world().colliders.len(), 0);
		Ok(())
	}
}
