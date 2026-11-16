//! Letting go of what was not used for long.
//!
//! An `AssetId` is a `Copy` value in components, with nothing to count its
//! holders: the cache goes by when an asset was last asked for instead.

use std::collections::HashSet;

use bevy_ecs::system::ResMut;
use web_time::{Duration, Instant};

use super::{GpuCache, Slot, Table};
use crate::assets::AssetId;

impl GpuCache {
	/// While the cache is over its memory budget, evicts the assets not used
	/// for longer than [`CacheSettings::keep_unused`], the longest unused
	/// first. Pinned ones stay. Returns how many went, with the materials
	/// that used them.
	///
	/// [`CacheSettings::keep_unused`]: super::CacheSettings::keep_unused
	pub fn evict(&mut self) -> usize {
		if self.memory <= self.settings.memory_budget {
			return 0;
		}
		let (now, keep) = (self.now, self.settings.keep_unused);
		let mut old = Vec::new();
		old_ready(&self.meshes, now, keep, &self.pinned, &mut old);
		old_ready(&self.textures, now, keep, &self.pinned, &mut old);
		old_ready(&self.materials, now, keep, &self.pinned, &mut old);
		old.sort_unstable_by_key(|&(last_used, _)| last_used);

		let before = self.count();
		for (_, id) in old {
			if self.memory <= self.settings.memory_budget {
				break;
			}
			self.invalidate(id);
		}
		before - self.count()
	}

	/// Evicts every asset not asked for since the frame began (on a change of
	/// world), whatever the budget, with the materials that use it. Pinned
	/// ones stay, and so do the failed ones: they are not tried again on
	/// every change. Returns how many went.
	pub fn evict_unused(&mut self) -> usize {
		let mut gone = Vec::new();
		unused(&self.meshes, self.now, &self.pinned, &mut gone);
		unused(&self.textures, self.now, &self.pinned, &mut gone);
		unused(&self.materials, self.now, &self.pinned, &mut gone);
		let before = self.count();
		for id in gone {
			self.invalidate(id);
		}
		before - self.count()
	}

	/// How many assets the cache has, in any state.
	pub fn count(&self) -> usize {
		self.meshes.len() + self.textures.len() + self.materials.len()
	}
}

/// The ready assets of `table` not used for longer than `keep`, with when
/// they were last used.
fn old_ready<T>(
	table: &Table<T>,
	now: Instant,
	keep: Duration,
	pinned: &HashSet<AssetId>,
	old: &mut Vec<(Instant, AssetId)>,
) {
	old.extend(
		table
			.iter()
			.filter(|&(id, entry)| {
				matches!(entry.slot, Slot::Ready(_))
					&& !pinned.contains(id)
					&& now.saturating_duration_since(entry.last_used) > keep
			})
			.map(|(&id, entry)| (entry.last_used, id)),
	);
}

/// The assets of `table` not asked for since `now`, but the failed ones.
fn unused<T>(table: &Table<T>, now: Instant, pinned: &HashSet<AssetId>, gone: &mut Vec<AssetId>) {
	gone.extend(
		table
			.iter()
			.filter(|&(id, entry)| !matches!(entry.slot, Slot::Failed) && entry.last_used < now && !pinned.contains(id))
			.map(|(&id, _)| id),
	);
}

/// A frame of the cache begins (the first step of the render schedule).
pub fn begin_frame(mut cache: ResMut<GpuCache>) {
	cache.begin_frame(Instant::now());
}

/// The cache lets go of what was not used for long (the last step).
pub fn evict(mut cache: ResMut<GpuCache>) {
	cache.evict();
}
