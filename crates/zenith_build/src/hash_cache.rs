//! Remembers the content hash of project files between folder builds, so that
//! rebuilding a project that has not changed does not read every file again.
//!
//! The cache lives in `Temp/Build/hash-cache.txt`, which is never shipped and
//! never in version control. It is a plain text file: a line per file with its
//! size, modification time, the time the hash was recorded, the hash (BLAKE3,
//! hex) and the project-relative path. A missing or damaged file is an empty
//! cache.
//!
//! A hash is used instead of reading the file when the size and the
//! modification time are what they were when it was recorded **and** the file
//! was already at least [`SETTLE`] old at that moment. The second condition is
//! the one `git` and `make` need as well: file systems keep modification times
//! in coarse steps, so a file written twice within one step looks unchanged.
//!
//! What this cannot notice is a file whose content changed while its size and
//! modification time were put back. Turn the cache off
//! (`Builder::hash_cache(false)`) for builds from tools that do that.

use std::{
	collections::HashMap,
	time::{Duration, SystemTime, UNIX_EPOCH},
};

use zenith_error::ZResult;
use zenith_filesystem::{Project, ZPath};

const FILE: &str = "Temp://Build/hash-cache.txt";
const HEADER: &str = "zenith-hash-cache 1";
/// How long a file must have been unchanged when its hash was recorded.
const SETTLE: Duration = Duration::from_secs(2);

fn nanos(time: SystemTime) -> Option<u128> {
	time.duration_since(UNIX_EPOCH).ok().map(|d| d.as_nanos())
}

/// What is known about one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
	size: u64,
	modified_ns: u128,
	recorded_ns: u128,
	hash: [u8; 32],
}

impl Record {
	/// The text line of this record for `path` (see the module documentation).
	fn line(&self, path: &str) -> String {
		format!(
			"{}\t{}\t{}\t{}\t{path}\n",
			self.size,
			self.modified_ns,
			self.recorded_ns,
			blake3::Hash::from_bytes(self.hash).to_hex()
		)
	}

	/// The record for a file seen now. `None` if its modification time is
	/// before the epoch (then there is nothing to compare later).
	pub fn new(size: u64, modified: SystemTime, hash: [u8; 32]) -> Option<Self> {
		Some(Self {
			size,
			modified_ns: nanos(modified)?,
			recorded_ns: nanos(SystemTime::now())?,
			hash,
		})
	}
}

#[derive(Default)]
pub struct HashCache {
	records: HashMap<String, Record>,
}

impl HashCache {
	/// The cache of `project`; empty if there is none or it cannot be read.
	pub fn load(project: &Project) -> Self {
		let text = ZPath::parse(FILE)
			.ok()
			.and_then(|path| project.read_string(&path).ok())
			.unwrap_or_default();
		Self::parse(&text)
	}

	fn parse(text: &str) -> Self {
		let mut lines = text.lines();
		if lines.next() != Some(HEADER) {
			return Self::default();
		}
		let records = lines
			.filter_map(|line| {
				let mut fields = line.splitn(5, '\t');
				let size = fields.next()?.parse().ok()?;
				let modified_ns = fields.next()?.parse().ok()?;
				let recorded_ns = fields.next()?.parse().ok()?;
				let hash = *blake3::Hash::from_hex(fields.next()?).ok()?.as_bytes();
				let path = fields.next()?;
				Some((
					path.to_string(),
					Record {
						size,
						modified_ns,
						recorded_ns,
						hash,
					},
				))
			})
			.collect();
		Self { records }
	}

	/// The hash of `rel`, if it can be trusted for a file of this size and
	/// modification time.
	pub fn lookup(&self, rel: &str, size: u64, modified: SystemTime) -> Option<[u8; 32]> {
		let record = self.records.get(rel)?;
		let modified_ns = nanos(modified)?;
		let settled = modified_ns.saturating_add(SETTLE.as_nanos());
		(record.size == size && record.modified_ns == modified_ns && record.recorded_ns >= settled)
			.then_some(record.hash)
	}

	/// Writes the cache: the records of this one that `keep` accepts (files
	/// that are still part of the build) with `fresh` ones replacing them.
	///
	/// # Errors
	/// The project's error if `Temp/Build/hash-cache.txt` cannot be written.
	pub fn save(&self, project: &Project, fresh: &[(String, Record)], keep: impl Fn(&str) -> bool) -> ZResult<()> {
		let mut records: HashMap<&str, &Record> = self
			.records
			.iter()
			.filter(|(path, _)| keep(path))
			.map(|(path, record)| (path.as_str(), record))
			.collect();
		for (path, record) in fresh {
			records.insert(path.as_str(), record);
		}
		let mut sorted: Vec<_> = records.into_iter().collect();
		sorted.sort_by_key(|(path, _)| *path);

		let mut text = String::from(HEADER);
		text.push('\n');
		for (path, record) in sorted {
			text.push_str(&record.line(path));
		}
		project.write(&ZPath::parse(FILE)?, text.as_bytes())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn long_ago() -> SystemTime {
		SystemTime::now() - Duration::from_secs(3600)
	}

	#[test]
	fn a_settled_file_with_the_same_stat_is_trusted() {
		let modified = long_ago();
		let record = Record::new(10, modified, [7; 32]);
		let cache = HashCache {
			records: HashMap::from([("a".to_string(), record.unwrap_or_else(|| unreachable!()))]),
		};
		assert_eq!(cache.lookup("a", 10, modified), Some([7; 32]), "same size and time");
		assert_eq!(cache.lookup("a", 11, modified), None, "another size");
		assert_eq!(
			cache.lookup("a", 10, modified + Duration::from_secs(1)),
			None,
			"another modification time"
		);
		assert_eq!(cache.lookup("b", 10, modified), None, "another file");
	}

	#[test]
	fn a_file_changed_just_before_it_was_recorded_is_not_trusted() {
		let modified = SystemTime::now();
		let record = Record::new(10, modified, [7; 32]);
		let cache = HashCache {
			records: HashMap::from([("a".to_string(), record.unwrap_or_else(|| unreachable!()))]),
		};
		assert_eq!(
			cache.lookup("a", 10, modified),
			None,
			"it could have been written again within the same clock step"
		);
	}

	#[test]
	fn text_round_trip_and_damage() {
		let modified = long_ago();
		let record = Record::new(5, modified, [9; 32]).unwrap_or_else(|| unreachable!());
		let mut text = String::from(HEADER);
		text.push('\n');
		text.push_str(&record.line("Assets/a b.txt"));
		text.push_str("garbage line\n");
		let parsed = HashCache::parse(&text);
		assert_eq!(parsed.records.len(), 1, "the good line is read, the bad one skipped");
		assert_eq!(
			parsed.records.get("Assets/a b.txt"),
			Some(&record),
			"spaces in paths survive"
		);
		assert!(
			HashCache::parse("not a cache\n1\t2\t3\t4\tx\n").records.is_empty(),
			"wrong header"
		);
		assert!(HashCache::parse("").records.is_empty(), "empty file");
	}
}
