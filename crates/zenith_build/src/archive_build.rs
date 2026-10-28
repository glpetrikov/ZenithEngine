//! `BuildFormat::Archive`: one file, written next to its final name and
//! renamed into place, so a failed build never leaves half an archive there.
//!
//! The entries are compressed on several threads and appended in path order.
//! Small files are compressed whole, a window of them at a time, one file per
//! thread; a big file is cut into chunks that are compressed side by side. A
//! chunk depends only on its own bytes, so the archive is byte for byte the
//! same as a single thread would write.

use std::{
	fs::File,
	io::{BufWriter, Write},
	path::{Path, PathBuf},
	sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use zenith_archive::{
	ArchiveReader, ArchiveWriter, BuildId, EncryptionKey, EntrySealer, FileSource, SealedEntry, WriterOptions,
};
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{Project, layout::TMP_SUFFIX};

use crate::plan::{Plan, Source, open_source};

/// Files up to this size are compressed whole, on one thread each. (Smaller in
/// the unit tests, so that they can cross the limits with little data.)
const SMALL_ENTRY: u64 = if cfg!(test) { 64 * 1024 } else { 8 * 1024 * 1024 };
/// Most bytes of small files compressed at the same time, so that memory stays
/// bounded however many small files there are.
const WINDOW: u64 = if cfg!(test) { 512 * 1024 } else { 64 * 1024 * 1024 };

pub fn temp_path(target: &Path) -> PathBuf {
	let mut name = target
		.file_name()
		.map(std::ffi::OsStr::to_os_string)
		.unwrap_or_default();
	name.push(TMP_SUFFIX);
	target.with_file_name(name)
}

/// `options.threads` is how many threads compress at the same time.
pub fn write_archive(project: &Project, plan: &Plan, output: &Path, options: &WriterOptions) -> ZResult<()> {
	let tmp = temp_path(output);
	let result =
		write_to(project, plan, &tmp, options).and_then(|()| std::fs::rename(&tmp, output).map_err(Into::into));
	if result.is_err() {
		let _ = std::fs::remove_file(&tmp);
	}
	result
}

/// The id of the archive that was just written. Reading it back also proves
/// that what is in the output can be opened with this key.
pub fn build_id(output: &Path, key: Option<&EncryptionKey>) -> ZResult<BuildId> {
	Ok(ArchiveReader::open(FileSource::open(output)?, key)?.build_id())
}

/// An entry waiting in a window.
type Item<'a> = (&'a String, &'a Source);

fn write_to(project: &Project, plan: &Plan, tmp: &Path, options: &WriterOptions) -> ZResult<()> {
	let mut writer = ArchiveWriter::new(BufWriter::new(File::create(tmp)?), options)?;
	let threads = options.threads.max(1);
	// Sealers compress whole small files; they work alone, the writer's own
	// threads are for the big ones.
	let single = WriterOptions {
		threads: 1,
		..options.clone()
	};
	let mut sealers = (0..threads)
		.map(|_| EntrySealer::new(&single))
		.collect::<ZResult<Vec<_>>>()?;

	let entries: Vec<Item<'_>> = plan.entries.iter().collect();
	let mut next = 0;
	while let Some(&(path, source)) = entries.get(next) {
		let window = if threads > 1 {
			small_window(project, entries.get(next..).unwrap_or_default())?
		} else {
			&[]
		};
		if window.is_empty() {
			writer.add(path, open_source(project, source)?)?;
			next = next.saturating_add(1);
			continue;
		}
		let sealed = seal_window(project, &mut sealers, window)?;
		for (&(path, _), entry) in window.iter().zip(sealed) {
			writer.add_sealed(path, entry)?;
		}
		next = next.saturating_add(window.len());
	}

	let mut file = writer.finish()?;
	file.flush()?;
	// On disk before the rename, so a power loss never leaves a half-written
	// archive under the real name.
	file.get_ref().sync_all()?;
	Ok(())
}

/// How many bytes an entry has (as far as the plan knows).
fn size_of(project: &Project, source: &Source) -> ZResult<u64> {
	Ok(match source {
		Source::Project { size, .. } => *size,
		Source::Zpath(path) => project.metadata(path)?.size,
		Source::Bytes(bytes) => u64::try_from(bytes.len()).unwrap_or(u64::MAX),
	})
}

/// The run of small entries at the start of `rest`, as many as fit in a window.
/// Empty if the first entry is big.
fn small_window<'a, 'b>(project: &Project, rest: &'b [Item<'a>]) -> ZResult<&'b [Item<'a>]> {
	let mut bytes = 0u64;
	let mut count = 0usize;
	for &(_, source) in rest {
		let size = size_of(project, source)?;
		if size > SMALL_ENTRY || (count > 0 && bytes.saturating_add(size) > WINDOW) {
			break;
		}
		bytes = bytes.saturating_add(size);
		count = count.saturating_add(1);
	}
	Ok(rest.get(..count).unwrap_or_default())
}

/// Compresses the entries of a window, one per thread at a time, and returns
/// them in the window's order. The first error (in that order) is the result.
fn seal_window(project: &Project, sealers: &mut [EntrySealer], window: &[Item<'_>]) -> ZResult<Vec<SealedEntry>> {
	let next = AtomicUsize::new(0);
	let failed = AtomicBool::new(false);
	let (next, failed) = (&next, &failed);

	let mut done: Vec<(usize, ZResult<SealedEntry>)> = std::thread::scope(|scope| {
		let workers: Vec<_> = sealers
			.iter_mut()
			.take(window.len())
			.map(|sealer| {
				scope.spawn(move || {
					let mut results = Vec::new();
					while !failed.load(Ordering::Relaxed) {
						let i = next.fetch_add(1, Ordering::Relaxed);
						let Some(&(path, source)) = window.get(i) else {
							break;
						};
						let sealed = open_source(project, source).and_then(|src| sealer.seal(path, src));
						if sealed.is_err() {
							failed.store(true, Ordering::Relaxed);
						}
						results.push((i, sealed));
					}
					results
				})
			})
			.collect();
		workers
			.into_iter()
			.flat_map(|worker| {
				worker.join().unwrap_or_else(|_| {
					vec![(
						usize::MAX,
						Err(ZenithError::Other("a build worker thread panicked".into())),
					)]
				})
			})
			.collect()
	});
	done.sort_by_key(|(i, _)| *i);
	done.into_iter().map(|(_, result)| result).collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{builder::BuildFormat, plan::make_plan};

	/// Bytes that do not compress, from a seed.
	fn noise(len: usize, seed: u64) -> Vec<u8> {
		let mut x = seed | 1;
		(0..len)
			.map(|_| {
				x ^= x << 13;
				x ^= x >> 7;
				x ^= x << 17;
				u8::try_from(x & 0xff).unwrap_or(0)
			})
			.collect()
	}

	/// Many files below, around and above the limits of the test build, so that
	/// windows end in different places and big files sit between them.
	fn build_with(threads: usize, key: Option<zenith_archive::EncryptionKey>) -> ZResult<Vec<u8>> {
		let tmp = tempfile::tempdir()?;
		let root = tmp.path().join("Game");
		let project = Project::create(&root)?;
		for n in 0..70usize {
			let len = match n % 7 {
				0 => 0,
				1 => 100,
				2 => 20_000,
				3 => SMALL_ENTRY as usize,
				4 => SMALL_ENTRY as usize + 1,
				5 => 150_000,
				_ => 40_000 + n,
			};
			std::fs::write(root.join(format!("Assets/f{n:02}.bin")), noise(len, n as u64 + 1))?;
		}
		let plan = make_plan(&project, &[], &Vec::new(), BuildFormat::Archive, key.is_some())?;
		let options = WriterOptions {
			compress: true,
			compression_level: 3,
			chunk_size_log2: 14,
			key,
			threads,
		};
		let out = tmp.path().join("out.zenithbuild");
		write_archive(&project, &plan, &out, &options)?;
		Ok(std::fs::read(out)?)
	}

	#[test]
	fn the_archive_does_not_depend_on_the_number_of_threads() -> ZResult<()> {
		for key in [None, Some(zenith_archive::EncryptionKey::new([5; 32]))] {
			let single = build_with(1, key.clone())?;
			for threads in [2, 3, 8] {
				assert!(
					build_with(threads, key.clone())? == single,
					"{threads} threads wrote other bytes (encrypted: {})",
					key.is_some()
				);
			}
		}
		Ok(())
	}
}
