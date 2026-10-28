//! Read cache shared by all backends (one instance per opened project).
//!
//! Only `read_bytes`/`read_string` use it. A file enters the cache on its
//! second request: the first only remembers the key, so a file that is read
//! once (a big asset during loading) never evicts anything useful.

use std::{
	collections::{BTreeMap, HashMap},
	sync::{Arc, Mutex, MutexGuard, PoisonError},
	time::SystemTime,
};

use serde::{Deserialize, Serialize};
use zenith_error::{ZResult, ZenithError};

use crate::layout::CACHE_CONFIG_FILE;

const MIB: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CacheSettings {
	/// The read on which a file enters the cache (2 = the second read).
	pub min_reads_before_cache: u32,
	/// Larger files are never cached.
	pub max_file_size: u64,
	/// Cached bytes in total; the least recently used files are dropped first.
	pub total_budget: u64,
	/// How many once-seen keys are remembered while waiting for a second read.
	pub max_tracked_keys: usize,
}

impl Default for CacheSettings {
	fn default() -> Self {
		Self {
			min_reads_before_cache: 2,
			max_file_size: 4 * MIB,
			total_budget: 256 * MIB,
			max_tracked_keys: 4096,
		}
	}
}

impl CacheSettings {
	/// Parses `Settings/CacheConfig.toml`. Missing keys keep their defaults;
	/// unknown keys are an error, because this file is owned by Zenith and a
	/// misspelled key would otherwise silently do nothing.
	///
	/// # Errors
	/// `InvalidSettings` for malformed text or unknown keys.
	pub fn parse(text: &str) -> ZResult<Self> {
		toml::from_str(text).map_err(|e| ZenithError::InvalidSettings(format!("{CACHE_CONFIG_FILE}: {e}")))
	}

	/// # Errors
	/// `TomlSer` if serialization fails.
	pub fn to_toml(&self) -> ZResult<String> {
		Ok(toml::to_string(self)?)
	}
}

/// Where cache settings come from.
#[derive(Clone, Debug, Default)]
pub enum CacheConfig {
	/// `Settings/CacheConfig.toml`; defaults if the file does not exist (it is
	/// not created).
	#[default]
	File,
	/// Explicit values, never written to disk.
	Manual(CacheSettings),
}

/// What identifies a version of a file on a mutable backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
	pub mtime: Option<SystemTime>,
	pub size: u64,
}

struct Entry {
	data: Arc<[u8]>,
	stamp: Option<Stamp>,
	tick: u64,
}

#[derive(Default)]
struct Inner {
	entries: HashMap<String, Entry>,
	/// tick -> key, oldest first.
	order: BTreeMap<u64, String>,
	bytes: u64,
	seen: HashMap<String, (u32, u64)>,
	seen_order: BTreeMap<u64, String>,
	tick: u64,
}

impl Inner {
	const fn next_tick(&mut self) -> u64 {
		self.tick = self.tick.wrapping_add(1);
		self.tick
	}

	fn remove(&mut self, key: &str) {
		if let Some(entry) = self.entries.remove(key) {
			self.order.remove(&entry.tick);
			self.bytes = self.bytes.saturating_sub(entry.data.len() as u64);
		}
		if let Some((_, tick)) = self.seen.remove(key) {
			self.seen_order.remove(&tick);
		}
	}
}

pub struct Cache {
	/// Distinguishes backends in the key (`fs`, `archive`, `folder`).
	backend: &'static str,
	settings: CacheSettings,
	inner: Mutex<Inner>,
}

impl Cache {
	pub fn new(backend: &'static str, settings: CacheSettings) -> Self {
		Self {
			backend,
			settings,
			inner: Mutex::new(Inner::default()),
		}
	}

	pub const fn settings(&self) -> &CacheSettings {
		&self.settings
	}

	fn lock(&self) -> MutexGuard<'_, Inner> {
		self.inner.lock().unwrap_or_else(PoisonError::into_inner)
	}

	/// The key is built from the backend and the resolved path inside it, not
	/// from the `ZPath` text, so `Assets://Worlds/A` and `Worlds://A` share an
	/// entry.
	fn key(&self, rel: &str) -> String {
		format!("{}:{rel}", self.backend)
	}

	/// A cached copy, if it is still valid. `current` is the file's present
	/// stamp on a mutable backend (`None` on immutable builds): a changed
	/// stamp drops the entry.
	pub fn get(&self, rel: &str, current: Option<Stamp>) -> Option<Arc<[u8]>> {
		let key = self.key(rel);
		let mut inner = self.lock();
		let stale = inner.entries.get(&key).is_some_and(|e| e.stamp != current);
		if stale {
			inner.remove(&key);
			return None;
		}
		let tick = inner.next_tick();
		let (old_tick, data) = {
			let entry = inner.entries.get_mut(&key)?;
			let old = std::mem::replace(&mut entry.tick, tick);
			(old, entry.data.clone())
		};
		inner.order.remove(&old_tick);
		inner.order.insert(tick, key);
		drop(inner);
		Some(data)
	}

	/// Called after a cache miss was read from the backend.
	pub fn offer(&self, rel: &str, data: &Arc<[u8]>, stamp: Option<Stamp>) {
		self.offer_lazy(rel, data.len(), || data.clone(), stamp);
	}

	/// `offer` for a caller that does not have the bytes as an `Arc` yet:
	/// `make` is called only if this read puts the file into the cache, so the
	/// first read of a file, which only leaves a note, copies nothing.
	pub fn offer_lazy(&self, rel: &str, len: usize, make: impl FnOnce() -> Arc<[u8]>, stamp: Option<Stamp>) {
		let size = len as u64;
		if size > self.settings.max_file_size || size > self.settings.total_budget {
			return;
		}
		let key = self.key(rel);
		let mut inner = self.lock();
		if inner.entries.contains_key(&key) {
			return;
		}
		let reads = inner.seen.get(&key).map_or(1, |(count, _)| count.saturating_add(1));
		inner.remove(&key);
		let tick = inner.next_tick();
		if reads >= self.settings.min_reads_before_cache {
			inner.order.insert(tick, key.clone());
			inner.bytes = inner.bytes.saturating_add(size);
			inner.entries.insert(
				key,
				Entry {
					data: make(),
					stamp,
					tick,
				},
			);
			while inner.bytes > self.settings.total_budget {
				let Some((_, oldest)) = inner.order.pop_first() else {
					break;
				};
				if let Some(entry) = inner.entries.remove(&oldest) {
					inner.bytes = inner.bytes.saturating_sub(entry.data.len() as u64);
				}
			}
		} else {
			inner.seen.insert(key.clone(), (reads, tick));
			inner.seen_order.insert(tick, key);
			while inner.seen.len() > self.settings.max_tracked_keys {
				let Some((_, oldest)) = inner.seen_order.pop_first() else {
					break;
				};
				inner.seen.remove(&oldest);
			}
		}
		drop(inner);
	}

	/// Drops one file.
	#[cfg(feature = "fs")]
	pub fn invalidate(&self, rel: &str) {
		let key = self.key(rel);
		self.lock().remove(&key);
	}

	/// Drops a file or everything below a folder.
	#[cfg(any(feature = "fs", test))]
	pub fn invalidate_prefix(&self, rel: &str) {
		let exact = self.key(rel);
		let below = format!("{exact}/");
		let mut inner = self.lock();
		let doomed: Vec<String> = inner
			.entries
			.keys()
			.chain(inner.seen.keys())
			.filter(|k| **k == exact || k.starts_with(&below))
			.cloned()
			.collect();
		for key in doomed {
			inner.remove(&key);
		}
	}

	#[cfg(test)]
	pub fn cached_bytes(&self) -> u64 {
		self.lock().bytes
	}

	#[cfg(test)]
	pub fn is_cached(&self, rel: &str) -> bool {
		self.lock().entries.contains_key(&self.key(rel))
	}

	#[cfg(test)]
	pub fn tracked(&self) -> usize {
		self.lock().seen.len()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn data(n: usize) -> Arc<[u8]> {
		Arc::from(vec![0u8; n])
	}

	fn cache(settings: CacheSettings) -> Cache {
		Cache::new("t", settings)
	}

	#[test]
	fn second_read_enters_the_cache() {
		let c = cache(CacheSettings::default());
		assert!(c.get("a", None).is_none(), "nothing cached yet");
		c.offer("a", &data(10), None);
		assert!(!c.is_cached("a"), "first read only remembers the key");
		assert_eq!(c.tracked(), 1, "the key is tracked");
		c.offer("a", &data(10), None);
		assert!(c.is_cached("a"), "second read caches");
		assert_eq!(c.tracked(), 0, "tracking ends once cached");
		assert_eq!(c.get("a", None).map(|d| d.len()), Some(10), "hit");
	}

	#[test]
	fn limits_are_enforced() {
		let c = cache(CacheSettings {
			max_file_size: 100,
			total_budget: 250,
			max_tracked_keys: 2,
			min_reads_before_cache: 1,
		});
		c.offer("huge", &data(101), None);
		assert!(!c.is_cached("huge"), "over max_file_size is never cached");
		c.offer("a", &data(100), None);
		c.offer("b", &data(100), None);
		assert!(c.get("a", None).is_some(), "touch a so b becomes the oldest");
		c.offer("c", &data(100), None);
		assert!(
			c.is_cached("a") && c.is_cached("c") && !c.is_cached("b"),
			"LRU evicts the least recently used"
		);
		assert!(c.cached_bytes() <= 250, "budget holds");

		let t = cache(CacheSettings {
			max_tracked_keys: 2,
			..CacheSettings::default()
		});
		for k in ["x", "y", "z"] {
			t.offer(k, &data(1), None);
		}
		assert_eq!(t.tracked(), 2, "tracked keys are bounded");
		t.offer("x", &data(1), None);
		assert!(
			!t.is_cached("x"),
			"the oldest tracked key was forgotten, so this is a first read again"
		);
	}

	#[test]
	fn changed_stamp_invalidates() {
		let c = cache(CacheSettings {
			min_reads_before_cache: 1,
			..CacheSettings::default()
		});
		let s1 = Stamp { mtime: None, size: 1 };
		let s2 = Stamp { mtime: None, size: 2 };
		c.offer("a", &data(1), Some(s1));
		assert!(c.get("a", Some(s1)).is_some(), "same stamp hits");
		assert!(c.get("a", Some(s2)).is_none(), "different stamp misses");
		assert!(!c.is_cached("a"), "and drops the entry");
	}

	#[test]
	fn invalidate_prefix_drops_a_subtree_only() {
		let c = cache(CacheSettings {
			min_reads_before_cache: 1,
			..CacheSettings::default()
		});
		for k in ["dir/a", "dir/sub/b", "dirx/c", "dir"] {
			c.offer(k, &data(1), None);
		}
		c.invalidate_prefix("dir");
		assert!(
			!c.is_cached("dir") && !c.is_cached("dir/a") && !c.is_cached("dir/sub/b"),
			"subtree dropped"
		);
		assert!(c.is_cached("dirx/c"), "a sibling with the same prefix survives");
	}

	#[test]
	fn settings_file_format() -> ZResult<()> {
		let s = CacheSettings::parse("max_file_size = 10\n")?;
		assert_eq!(s.max_file_size, 10, "given key");
		assert_eq!(
			s.total_budget,
			CacheSettings::default().total_budget,
			"missing key keeps its default"
		);
		assert_eq!(CacheSettings::parse(&s.to_toml()?)?, s, "round trip");
		assert!(
			CacheSettings::parse("max_file_sise = 10\n").is_err(),
			"a misspelled key is an error"
		);
		assert!(CacheSettings::parse("max_file_size = 'x'").is_err(), "wrong type");
		Ok(())
	}
}
