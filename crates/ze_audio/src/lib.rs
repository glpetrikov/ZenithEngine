use std::{
	collections::{HashMap, HashSet},
	io::Cursor,
};

use kira::{
	AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween,
	sound::{
		PlaybackState,
		static_sound::{StaticSoundData, StaticSoundHandle},
	},
};
use ze_assets::ResourceManager;
use ze_core::{Result, anyhow};
use ze_ecs::{AudioSource, EntitiesView, EntityId, Scene, System};
use ze_scripting_cs::{AudioApiCommand, drain_audio_api_commands, refresh_scripting_api_audio_playing_cache};

/// Owns the live `kira` audio backend, the decoded-clip cache, and one playback
/// handle per entity that's currently playing.
///
/// Kept outside the ECS entirely (like `ze_physics::PhysicsWorld` holds
/// rapier's live rigid-body state), since a playing sound handle isn't
/// serializable scene data -- the `AudioSource` component
/// (`ze_ecs::components`) only holds the inspector-editable config. Registered
/// as a `System` sibling to `PhysicsSystem`/`UISystem`, so a Scripts.dll
/// hot-reload -- which only touches the `ScriptingSystem` -- can never reach or
/// drop it; already-playing audio survives reload for free.
pub struct AudioSystem {
	manager: AudioManager,
	resources: ResourceManager,
	clip_cache: HashMap<String, StaticSoundData>,
	handles: HashMap<EntityId, PlayingSound>,
	auto_started: HashSet<EntityId>,
}

/// A live handle plus the volume last pushed to it, so `sync_volume_from_ecs`
/// can tell whether `AudioSource.volume` changed since we last looked (from a
/// `SetVolume` command, an Inspector edit, or a scene file reload) without kira
/// exposing a "current volume" getter to compare against directly.
struct PlayingSound {
	handle: StaticSoundHandle,
	applied_volume: f32,
}

impl AudioSystem {
	pub fn new(resources: ResourceManager) -> Result<Self> {
		let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
			.map_err(|error| anyhow!("failed to initialize audio backend: {error}"))?;

		Ok(Self {
			manager,
			resources,
			clip_cache: HashMap::new(),
			handles: HashMap::new(),
			auto_started: HashSet::new(),
		})
	}

	/// Starts (or restarts) playback for `entity`'s own `AudioSource`. A second
	/// `Play()` call on an already-playing entity retriggers cleanly rather
	/// than stacking a second overlapping voice, so each entity always has at
	/// most one live instance -- the thing that makes `Stop`/`SetVolume`/
	/// `IsPlaying` unambiguous per entity.
	fn play(&mut self, scene: &Scene, entity: EntityId) {
		self.stop(entity);

		let Ok(source) = scene.world().get::<&AudioSource>(entity) else {
			return;
		};
		let (clip_path, volume, looping) = (source.clip_path.clone(), source.volume, source.looping);

		let data = match cached_clip(&mut self.clip_cache, &self.resources, &clip_path) {
			Ok(data) => data,
			Err(error) => {
				ze_log::error!("failed to load audio clip `{clip_path}`: {error:?}");
				return;
			}
		};

		let mut data = data.volume(linear_to_decibels(volume));
		if looping {
			data = data.loop_region(0.0..);
		}

		match self.manager.play(data) {
			Ok(handle) => {
				self.handles.insert(
					entity,
					PlayingSound {
						handle,
						applied_volume: volume,
					},
				);
			}
			Err(error) => ze_log::error!("failed to play audio clip `{clip_path}`: {error:?}"),
		}
	}

	fn stop(&mut self, entity: EntityId) {
		if let Some(mut sound) = self.handles.remove(&entity) {
			sound.handle.stop(Tween::default());
		}
	}

	/// Just writes the new volume into the entity's `AudioSource` -- applying
	/// it to the live handle is `sync_volume_from_ecs`'s job, run
	/// unconditionally every tick, so a scripted `SetVolume` call and an
	/// Inspector/scene-file edit go through the exact same path instead
	/// of this method needing its own copy of the "push to kira" logic.
	fn set_volume(scene: &mut Scene, entity: EntityId, volume: f32) {
		if let Ok(mut source) = scene.world_mut().get::<&mut AudioSource>(entity) {
			source.volume = volume.clamp(0.0, 1.0);
		}
	}

	/// Re-applies `AudioSource.volume` to every currently-playing handle whose
	/// entity's volume has changed since it was last applied -- the same "sync
	/// live state from the ECS every tick" guarantee
	/// `PhysicsWorld::sync_from_ecs` gives `Transform` for kinematic bodies, so
	/// an Inspector or scene-file edit takes effect immediately, not just
	/// scripted `SetVolume` calls.
	fn sync_volume_from_ecs(&mut self, scene: &Scene) {
		let world = scene.world();
		for (&entity, sound) in &mut self.handles {
			let Ok(source) = world.get::<&AudioSource>(entity) else {
				continue;
			};

			let volume = source.volume.clamp(0.0, 1.0);
			if (volume - sound.applied_volume).abs() > f32::EPSILON {
				sound.handle.set_volume(linear_to_decibels(volume), Tween::default());
				sound.applied_volume = volume;
			}
		}
	}

	/// Stops every currently-playing instance. Public so editor-only callers
	/// (e.g. restoring the pre-Play-mode snapshot on `Stop`) can silence audio
	/// without going through the scripting command queue.
	pub fn stop_all(&mut self) {
		for (_, mut sound) in self.handles.drain() {
			sound.handle.stop(Tween::default());
		}
		self.auto_started.clear();
	}

	/// Plays any `AudioSource` with `play_on_start` the first time this system
	/// sees it, so a sound like background music can be entirely data-driven
	/// -- no script needed at all.
	fn auto_start_new_entities(&mut self, scene: &Scene) {
		let world = scene.world();
		let mut to_start = Vec::new();

		world.run(|entities: EntitiesView| {
			for entity in entities.iter() {
				let Ok(source) = world.get::<&AudioSource>(entity) else {
					continue;
				};
				if source.play_on_start && !self.auto_started.contains(&entity) {
					to_start.push(entity);
				}
			}
		});

		for entity in to_start {
			self.auto_started.insert(entity);
			self.play(scene, entity);
		}
	}

	/// Drops handles for sounds that finished on their own (a non-looping clip
	/// playing out), so `handles` doesn't grow unboundedly over a long play
	/// session with many one-shot SFX across many entities.
	fn prune_finished(&mut self) {
		self.handles
			.retain(|_, sound| sound.handle.state() != PlaybackState::Stopped);
	}
}

impl System for AudioSystem {
	fn name(&self) -> &'static str { "AudioSystem" }

	fn update(&mut self, scene: &mut Scene, _dt: f32) -> Result<()> {
		self.auto_start_new_entities(scene);

		for command in drain_audio_api_commands() {
			match command {
				AudioApiCommand::Play { entity } => self.play(scene, entity),
				AudioApiCommand::Stop { entity } => self.stop(entity),
				AudioApiCommand::SetVolume { entity, volume } => Self::set_volume(scene, entity, volume),
			}
		}

		self.sync_volume_from_ecs(scene);
		self.prune_finished();
		refresh_scripting_api_audio_playing_cache(
			self.handles
				.iter()
				.map(|(&entity, sound)| (entity, sound.handle.state().is_advancing())),
		);

		Ok(())
	}

	fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}

/// Decodes a clip on first use and caches it by its game-relative asset path
/// (e.g. `"sounds/jump.wav"`) so repeated plays (a jump mashed many times)
/// don't re-read and re-decode from the zepack archive every call.
/// `StaticSoundData` is cheap to clone -- the decoded samples are `Arc`-shared
/// -- so caching the decoded form, not just the raw bytes, is what
/// actually avoids repeat decode cost.
fn cached_clip(
	cache: &mut HashMap<String, StaticSoundData>,
	resources: &ResourceManager,
	path: &str,
) -> Result<StaticSoundData> {
	if let Some(data) = cache.get(path) {
		return Ok(data.clone());
	}

	let bytes = resources.game_bytes(path)?;
	let data = StaticSoundData::from_cursor(Cursor::new(bytes))
		.map_err(|error| anyhow!("failed to decode audio clip `{path}`: {error}"))?;
	cache.insert(path.to_string(), data.clone());
	Ok(data)
}

/// `kira` volumes are in decibels; our C#-facing API and `AudioSource.volume`
/// use a linear 0.0-1.0 scale (matching how UI colors etc. are exposed), so
/// calls are converted here. This is the exact inverse of
/// `Decibels::as_amplitude`.
fn linear_to_decibels(volume: f32) -> Decibels {
	let volume = volume.clamp(0.0, 1.0);
	if volume <= 0.0 {
		Decibels::SILENCE
	} else {
		Decibels(20.0 * volume.log10())
	}
}
