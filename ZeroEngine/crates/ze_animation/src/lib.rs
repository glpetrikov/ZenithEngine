use std::collections::{HashMap, HashSet};

use ze_assets::{AnimationClip, AssetRef, ResourceManager, step_animation_frame};
use ze_core::Result;
use ze_ecs::{Animator, EntitiesView, EntityId, Scene, System};
use ze_renderer::{Sprite, TextureSource};
use ze_scripting_cs::{AnimatorApiCommand, drain_animator_api_commands};

/// Caches parsed `AnimationClip` JSON assets, mirroring `ze_renderer`'s
/// `SheetCache` shape (including a failure negative-cache so a broken/missing
/// clip doesn't get re-read from disk and re-logged every frame).
struct AnimationClipCache {
	clips: HashMap<AssetRef, AnimationClip>,
	failed: HashSet<AssetRef>,
}

impl AnimationClipCache {
	fn new() -> Self {
		Self {
			clips: HashMap::new(),
			failed: HashSet::new(),
		}
	}

	fn get_or_load(&mut self, clip_ref: &AssetRef, resources: &ResourceManager) -> Option<&AnimationClip> {
		if self.failed.contains(clip_ref) {
			return None;
		}

		if !self.clips.contains_key(clip_ref) {
			match load_clip(clip_ref, resources) {
				Ok(clip) => {
					self.clips.insert(clip_ref.clone(), clip);
				}
				Err(error) => {
					ze_log::warn!("Failed to load animation clip `{}`: {error:?}", clip_ref.path);
					self.failed.insert(clip_ref.clone());
					return None;
				}
			}
		}

		self.clips.get(clip_ref)
	}
}

fn load_clip(clip_ref: &AssetRef, resources: &ResourceManager) -> Result<AnimationClip> {
	let text = resources.string(clip_ref)?;
	Ok(serde_json::from_str(&text)?)
}

/// Ticks every entity with both `Animator` and `Sprite`: applies queued
/// `SetState` commands from C# scripts, advances playback, and writes the
/// resulting cell into `Sprite.texture` as a `TextureSource::SheetCell`.
pub struct AnimationSystem {
	resources: ResourceManager,
	cache: AnimationClipCache,
}

impl AnimationSystem {
	pub fn new(resources: ResourceManager) -> Self {
		Self {
			resources,
			cache: AnimationClipCache::new(),
		}
	}
}

impl System for AnimationSystem {
	fn name(&self) -> &'static str { "AnimationSystem" }

	fn update(&mut self, scene: &mut Scene, dt: f32) -> Result<()> {
		for command in drain_animator_api_commands() {
			let AnimatorApiCommand::SetState { entity, state } = command;
			if let Ok(mut animator) = scene.world().get::<&mut Animator>(entity) {
				animator.set_state(state);
			}
		}

		let world = scene.world();
		let mut entities = Vec::new();
		world.run(|view: EntitiesView| {
			for entity in view.iter() {
				entities.push(entity);
			}
		});

		for entity in entities {
			self.tick_entity(world, entity, dt);
		}

		Ok(())
	}

	fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}

impl AnimationSystem {
	fn tick_entity(&mut self, world: &ze_ecs::World, entity: EntityId, dt: f32) {
		let Ok(animator) = world.get::<&Animator>(entity) else {
			return;
		};
		if !animator.playing {
			return;
		}
		let Some(state) = animator.states.iter().find(|s| s.name == animator.current_state) else {
			return;
		};
		let clip_ref = AssetRef::game(state.clip_path.clone());
		drop(animator);

		let Some(clip) = self.cache.get_or_load(&clip_ref, &self.resources) else {
			return;
		};
		if clip.frames.is_empty() {
			return;
		}

		let (frame_count, frame_duration_ms, loop_animation) =
			(clip.frames.len(), clip.frame_duration_ms, clip.loop_animation);

		let Ok(mut animator) = world.get::<&mut Animator>(entity) else {
			return;
		};
		let mut frame_index = animator.current_frame_index as usize;
		let mut elapsed_secs = animator.elapsed_secs;
		step_animation_frame(
			&mut frame_index,
			&mut elapsed_secs,
			dt,
			frame_count,
			frame_duration_ms,
			loop_animation,
		);
		animator.current_frame_index = frame_index as u32;
		animator.elapsed_secs = elapsed_secs;
		drop(animator);

		let cell_id = clip.frames[frame_index];
		let sheet_path = clip.texture_sheet.path.clone();
		if let Ok(mut sprite) = world.get::<&mut Sprite>(entity) {
			sprite.texture = TextureSource::SheetCell { sheet_path, cell_id };
		}
	}
}
