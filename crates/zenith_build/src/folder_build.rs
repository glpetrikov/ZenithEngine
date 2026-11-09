//! `BuildFormat::Fs`: a folder with `build.index` and one data file per entry.
//!
//! Order matters for updates and for crashes:
//!
//! 1. Every entry is *staged*, in parallel: its source is hashed, and unless
//!    the previous build already holds exactly that content with the same
//!    compression parameters (and its data file is still there and its header,
//!    footer and chunk table still check out) it is compressed into a temporary
//!    file. A failure here deletes the temporary files and leaves the folder as
//!    it was.
//! 2. The new index is encoded (still nothing in the folder changed).
//! 3. The commit (see `commit`): the new index is written as
//!    `build.index.next`, the old one steps aside as `build.index.prev`, staged
//!    files replace the data files (except those whose bytes came out
//!    identical: modification time kept, so an updater sees nothing to fetch),
//!    and the new index becomes `build.index`. A folder cannot change all at
//!    once, but until the last step it has no `build.index`, so an interruption
//!    leaves something plainly unfinished that the next build recognises and
//!    repairs, never an old index over a mixture of files.
//! 4. Only then are data files that the old index listed and the new one does
//!    not deleted; one that cannot be deleted is reported as a warning, since
//!    the build is complete. Files in the folder that are not part of the build
//!    are never deleted.

use std::{
	collections::{HashMap, HashSet},
	fs::File,
	io::{self, BufWriter, Read, Write},
	path::{Path, PathBuf},
	sync::{
		Arc, Mutex, PoisonError,
		atomic::{AtomicBool, AtomicUsize, Ordering},
	},
};

use zenith_archive::{
	BuildId, DirSource, EncryptionKey, FileSource, FolderEntry, FolderIndexEntry, FolderReader, FolderWriter,
	INDEX_FILE_NAME, ReadAt, WriterOptions, folder_is_encrypted,
};
use zenith_error::{ZResult, ZenithError};
use zenith_filesystem::{Project, layout::TMP_SUFFIX};

use crate::{
	archive_build::temp_path,
	hash_cache::{HashCache, Record},
	plan::{Plan, Source, open_source},
};

/// What a folder build did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
	/// Files that were compressed.
	pub compressed: usize,
	/// Files taken over from the previous build without touching them.
	pub reused: usize,
	/// Problems after the build was complete (see `BuildReport::warnings`).
	pub warnings: Vec<String>,
}

/// The index a build is about to install, written next to the old one.
const NEXT_INDEX: &str = "build.index.next";
/// The old index while the data files are being replaced.
const PREV_INDEX: &str = "build.index.prev";

/// An index of the output folder (`build.index`, or one of the two files above)
/// and the data files next to it, for the archive reader. The names it opens
/// come from that index, which the reader has checked.
struct OutputDir {
	root: PathBuf,
	index: &'static str,
}

impl DirSource for OutputDir {
	fn open_index(&self) -> io::Result<Arc<dyn ReadAt>> {
		Ok(Arc::new(FileSource::open(self.root.join(self.index))?))
	}

	fn open_data(&self, name: &str) -> io::Result<Arc<dyn ReadAt>> {
		Ok(Arc::new(FileSource::open(self.root.join(name))?))
	}
}

/// The id of the folder build that was just written. Reading its index back
/// also proves that it can be opened with this key.
pub fn build_id(output: &Path, key: Option<&EncryptionKey>) -> ZResult<BuildId> {
	let source = OutputDir {
		root: output.to_path_buf(),
		index: INDEX_FILE_NAME,
	};
	Ok(FolderReader::open(source, key)?.build_id())
}

/// What the output folder holds before a build.
enum Previous {
	/// Nothing, or an empty folder.
	Nothing,
	/// A complete build whose index could be read.
	Build(FolderReader),
	/// A build that was interrupted while its data files were being replaced:
	/// there is no `build.index`, but the index it was about to install, the
	/// one it had moved aside, or both. The data files are a mixture of two
	/// builds, so none is reused; these are the names of the files that are
	/// ours to overwrite.
	Interrupted(HashSet<String>),
}

/// What is in the output folder: nothing, a build whose index we can read, or
/// what an interrupted build left.
fn inspect(output: &Path, options: &WriterOptions) -> ZResult<Previous> {
	let meta = match std::fs::symlink_metadata(output) {
		Ok(meta) => meta,
		Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Previous::Nothing),
		Err(e) => return Err(e.into()),
	};
	if !meta.is_dir() {
		return Err(ZenithError::NotABuildDir(output.to_path_buf()));
	}
	if std::fs::read_dir(output)?.next().is_none() {
		return Ok(Previous::Nothing);
	}
	let index = output.join(INDEX_FILE_NAME);
	if !index.is_file() {
		return interrupted(output, options).ok_or_else(|| ZenithError::NotABuildDir(output.to_path_buf()));
	}

	let unreadable = |why: String| ZenithError::PreviousBuildUnreadable(format!("{}: {why}", index.display()));
	let was_encrypted = folder_is_encrypted(&FileSource::open(&index)?).map_err(|e| unreadable(e.to_string()))?;
	if was_encrypted != options.key.is_some() {
		return Err(unreadable(format!(
			"the existing build is {}, this one is {}; build into another folder",
			if was_encrypted { "encrypted" } else { "not encrypted" },
			if options.key.is_some() {
				"encrypted"
			} else {
				"not encrypted"
			},
		)));
	}
	let reader = FolderReader::open(
		OutputDir {
			root: output.to_path_buf(),
			index: INDEX_FILE_NAME,
		},
		options.key.as_ref(),
	)
	.map_err(|e| unreadable(e.to_string()))?;
	Ok(Previous::Build(reader))
}

/// The remains of an interrupted commit, if the folder holds any. An index that
/// cannot be read (damaged, or made with another key) tells nothing about
/// names.
fn interrupted(output: &Path, options: &WriterOptions) -> Option<Previous> {
	let mut found = false;
	let mut names = HashSet::new();
	for index in [NEXT_INDEX, PREV_INDEX] {
		if !output.join(index).is_file() {
			continue;
		}
		found = true;
		let source = OutputDir {
			root: output.to_path_buf(),
			index,
		};
		if let Ok(reader) = FolderReader::open(source, options.key.as_ref()) {
			names.extend(reader.entries().iter().map(|entry| entry.data_name.clone()));
		}
	}
	found.then_some(Previous::Interrupted(names))
}

/// Writes a file and flushes it to disk, so that renaming it into place cannot
/// leave a half-written file after a power loss.
fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
	let mut file = File::create(path)?;
	file.write_all(bytes)?;
	file.sync_all()
}

/// Whether two files hold the same bytes.
fn same_content(a: &Path, b: &Path) -> io::Result<bool> {
	let (mut fa, mut fb) = (File::open(a)?, File::open(b)?);
	if fa.metadata()?.len() != fb.metadata()?.len() {
		return Ok(false);
	}
	let (mut ba, mut bb) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
	loop {
		let n = fa.read(&mut ba)?;
		if n == 0 {
			return Ok(true);
		}
		fb.read_exact(bb.get_mut(..n).ok_or_else(|| io::Error::other("buffer"))?)?;
		if ba.get(..n) != bb.get(..n) {
			return Ok(false);
		}
	}
}

/// Removes now-empty folders from `start` up to (not including) `stop`.
fn prune_empty_parents(start: &Path, stop: &Path) {
	let mut dir = start.parent();
	while let Some(current) = dir {
		if current == stop || !current.starts_with(stop) || std::fs::remove_dir(current).is_err() {
			break;
		}
		dir = current.parent();
	}
}

/// BLAKE3 over the source content.
///
/// The hash only has to tell "same content" from "different content" and has
/// to be cheap, because it is the whole cost of rebuilding an untouched
/// project. BLAKE3 hashes about 3.5 times faster than SHA-256 here (6.5 against
/// 1.7 GB/s measured on 256 MiB, SHA-256 with hardware support), and is as
/// collision resistant (256 bits). No key is needed: with encryption the
/// index that holds the hash is encrypted too.
fn hash_source(project: &Project, source: &Source) -> ZResult<[u8; 32]> {
	let mut hasher = blake3::Hasher::new();
	hasher.update_reader(open_source(project, source)?)?;
	Ok(*hasher.finalize().as_bytes())
}

/// Hashes what passes through while the compressor reads it, so the hash in
/// the index is that of the bytes that were actually compressed even if the
/// file changed since the first pass.
struct Tee<R> {
	inner: R,
	hasher: blake3::Hasher,
}

impl<R: Read> Read for Tee<R> {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		let n = self.inner.read(buf)?;
		self.hasher.update(buf.get(..n).unwrap_or_default());
		Ok(n)
	}
}

/// One entry, ready to be committed.
enum Staged {
	/// The previous build already has it.
	Reused(FolderIndexEntry),
	/// Compressed into `tmp`; `replace` is false if the existing file has the
	/// same bytes.
	Fresh {
		entry: FolderIndexEntry,
		tmp: PathBuf,
		replace: bool,
	},
}

struct Job<'a> {
	path: &'a str,
	source: &'a Source,
	name: &'a str,
	target: PathBuf,
}

struct Shared<'a> {
	project: &'a Project,
	options: &'a WriterOptions,
	level: u8,
	chunk_log2: u8,
	previous: HashMap<&'a str, &'a FolderEntry>,
	/// The build that is in the output folder now, if any.
	reader: Option<&'a FolderReader>,
	/// Hashes remembered from earlier builds; `None` if the cache is off.
	cache: Option<&'a HashCache>,
	/// Hashes worked out in this build, to be remembered for the next.
	fresh: Mutex<Vec<(String, Record)>>,
}

impl Shared<'_> {
	/// Whether the previous build's data file for `path` is still there and
	/// agrees with its index: same length, valid header and footer, chunk table
	/// that checks out. (The chunks themselves are not read: that would cost as
	/// much as compressing again.)
	fn previous_file_is_sound(&self, path: &str) -> bool {
		self.reader.is_some_and(|reader| reader.open_entry(path).is_ok())
	}

	/// The content hash of a source: remembered from an earlier build if the
	/// file's size and modification time allow it, read from the file
	/// otherwise. The flag says which.
	fn source_hash(&self, source: &Source) -> ZResult<([u8; 32], bool)> {
		if let (
			Some(cache),
			Source::Project {
				rel,
				size,
				modified: Some(modified),
			},
		) = (self.cache, source)
			&& let Some(hash) = cache.lookup(rel, *size, *modified)
		{
			return Ok((hash, true));
		}
		Ok((hash_source(self.project, source)?, false))
	}

	/// Keeps `hash` for the next build, if the source is a project file with a
	/// modification time and the cache is on.
	fn remember(&self, source: &Source, hash: [u8; 32]) {
		if self.cache.is_none() {
			return;
		}
		if let Source::Project {
			rel,
			size,
			modified: Some(modified),
		} = source
			&& let Some(record) = Record::new(*size, *modified, hash)
		{
			self.fresh
				.lock()
				.unwrap_or_else(PoisonError::into_inner)
				.push((rel.clone(), record));
		}
	}

	fn stage(&self, job: &Job<'_>, writer: &mut Option<FolderWriter>) -> ZResult<Staged> {
		let (hash, from_cache) = self.source_hash(job.source)?;

		if let Some(old) = self.previous.get(job.path)
			&& old.content_hash == hash
			&& old.compression_level == self.level
			&& old.chunk_size_log2 == self.chunk_log2
			&& old.data_name == job.name
			&& self.previous_file_is_sound(job.path)
		{
			if !from_cache {
				self.remember(job.source, hash);
			}
			return Ok(Staged::Reused(FolderIndexEntry {
				path: old.path.clone(),
				size: old.size,
				data_len: old.data_len,
				content_hash: old.content_hash,
				compression_level: old.compression_level,
				chunk_size_log2: old.chunk_size_log2,
			}));
		}

		if writer.is_none() {
			*writer = Some(FolderWriter::new(self.options)?);
		}
		let writer = writer.as_mut().ok_or_else(|| ZenithError::Other("writer".into()))?;

		let tmp = temp_path(&job.target);
		if let Some(parent) = job.target.parent() {
			std::fs::create_dir_all(parent)?;
		}
		let staged = (|| -> ZResult<Staged> {
			let mut out = BufWriter::new(File::create(&tmp)?);
			let mut source = Tee {
				inner: open_source(self.project, job.source)?,
				hasher: blake3::Hasher::new(),
			};
			// The hash of what was compressed goes into the data file as well
			// as the index, so the two belong together.
			let info =
				writer.write_data_file(&mut out, job.path, &mut source, |tee| *tee.hasher.finalize().as_bytes())?;
			out.flush()?;
			drop(out);
			let replace = !(job.target.is_file() && same_content(&tmp, &job.target)?);
			// What was compressed is what the index records. A remembered hash
			// that this contradicts was stale (the file changed without its
			// size or time changing); the one just worked out replaces it. A
			// hash read from the file is remembered unless the file changed
			// between the two reads.
			let content_hash = info.content_hash;
			if from_cache {
				if content_hash != hash {
					self.remember(job.source, content_hash);
				}
			} else if content_hash == hash {
				self.remember(job.source, hash);
			}
			Ok(Staged::Fresh {
				entry: FolderIndexEntry {
					path: job.path.to_string(),
					size: info.size,
					data_len: info.data_len,
					content_hash,
					compression_level: self.level,
					chunk_size_log2: self.chunk_log2,
				},
				tmp: tmp.clone(),
				replace,
			})
		})();
		if staged.is_err() {
			let _ = std::fs::remove_file(&tmp);
		}
		staged
	}
}

/// Stages every job on a pool of threads. Which thread handles which file has
/// no effect on the result: a data file depends only on its own content and the
/// parameters.
fn stage_all(shared: &Shared<'_>, jobs: &[Job<'_>], threads: usize) -> ZResult<Vec<Staged>> {
	let threads = threads.clamp(1, jobs.len().max(1));
	let next = AtomicUsize::new(0);
	let failed = AtomicBool::new(false);

	let mut results: Vec<(usize, ZResult<Staged>)> = std::thread::scope(|scope| {
		let workers: Vec<_> = (0..threads)
			.map(|_| {
				scope.spawn(|| {
					let mut writer = None;
					let mut done = Vec::new();
					while !failed.load(Ordering::Relaxed) {
						let i = next.fetch_add(1, Ordering::Relaxed);
						let Some(job) = jobs.get(i) else { break };
						let result = shared.stage(job, &mut writer);
						if result.is_err() {
							failed.store(true, Ordering::Relaxed);
						}
						done.push((i, result));
					}
					done
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

	results.sort_by_key(|(i, _)| *i);
	if results.iter().any(|(_, r)| r.is_err()) {
		let mut first_error = None;
		for (_, result) in results {
			match result {
				Ok(Staged::Fresh { tmp, .. }) => {
					let _ = std::fs::remove_file(tmp);
				}
				Ok(Staged::Reused(_)) => {}
				Err(e) => {
					first_error.get_or_insert(e);
				}
			}
		}
		return Err(first_error.unwrap_or_else(|| ZenithError::Other("build failed".into())));
	}
	results.into_iter().map(|(_, r)| r).collect()
}

/// `threads` files are compressed at the same time; `hash_cache` says whether
/// hashes of unchanged files are taken from `Temp/Build/hash-cache.txt`.
pub fn write_folder(
	project: &Project,
	plan: &Plan,
	output: &Path,
	options: &WriterOptions,
	threads: usize,
	hash_cache: bool,
) -> ZResult<Stats> {
	write_folder_with_threads(project, plan, output, options, threads, hash_cache)
}

fn write_folder_with_threads(
	project: &Project,
	plan: &Plan,
	output: &Path,
	options: &WriterOptions,
	threads: usize,
	hash_cache: bool,
) -> ZResult<Stats> {
	let previous = inspect(output, options)?;
	let build = match &previous {
		Previous::Build(reader) => Some(reader),
		Previous::Nothing | Previous::Interrupted(_) => None,
	};
	let previous_entries: &[FolderEntry] = build.map_or(&[], FolderReader::entries);
	// The data files that belong to what was there before, and may be replaced
	// or deleted.
	let previous_names: Vec<&str> = match &previous {
		Previous::Interrupted(names) => names.iter().map(String::as_str).collect(),
		_ => previous_entries.iter().map(|entry| entry.data_name.as_str()).collect(),
	};
	let probe = FolderWriter::new(options)?;
	// 0 marks files that are stored without compression.
	let level = if options.compress {
		u8::try_from(options.compression_level)
			.map_err(|_| ZenithError::InvalidBuildOption("zstd compression level out of range".into()))?
	} else {
		0
	};

	let mut names = Vec::with_capacity(plan.entries.len());
	for path in plan.entries.keys() {
		names.push(probe.data_file_name(path)?);
	}
	// A data file is staged under `<name>.zenith-tmp`, and the index goes
	// through `build.index.next` and `build.index.prev`; an entry with one of
	// these names (unencrypted builds name data files after the entries) would
	// collide with them.
	if let Some(name) = names
		.iter()
		.find(|name| name.ends_with(TMP_SUFFIX) || *name == NEXT_INDEX || *name == PREV_INDEX)
	{
		return Err(ZenithError::InvalidBuildOutput(format!(
			"{name} is a name a folder build keeps for its own files ({TMP_SUFFIX}, {NEXT_INDEX}, {PREV_INDEX})"
		)));
	}
	// A file that is not one of the old build's own must never be overwritten.
	let old: HashSet<&str> = previous_names.iter().copied().collect();
	for name in &names {
		if !old.contains(name.as_str()) && output.join(name).symlink_metadata().is_ok() {
			return Err(ZenithError::InvalidBuildOutput(format!(
				"{name} already exists in {} and is not part of the previous build",
				output.display()
			)));
		}
	}

	std::fs::create_dir_all(output)?;
	let jobs: Vec<Job<'_>> = plan
		.entries
		.iter()
		.zip(&names)
		.map(|((path, source), name)| Job {
			path,
			source,
			name,
			target: output.join(name),
		})
		.collect();
	let cache = hash_cache.then(|| HashCache::load(project));
	let shared = Shared {
		project,
		options,
		level,
		chunk_log2: options.chunk_size_log2,
		previous: previous_entries
			.iter()
			.map(|entry| (entry.path.as_str(), entry))
			.collect(),
		reader: build,
		cache: cache.as_ref(),
		fresh: Mutex::new(Vec::new()),
	};
	let staged = stage_all(&shared, &jobs, threads)?;

	// Nothing in the folder has changed yet, so the index is encoded (and can
	// still fail) before the first data file is replaced.
	let index: Vec<FolderIndexEntry> = staged
		.iter()
		.map(|item| match item {
			Staged::Reused(entry) | Staged::Fresh { entry, .. } => entry.clone(),
		})
		.collect();
	let index_bytes = match probe.encode_index(&index) {
		Ok(bytes) => bytes,
		Err(e) => {
			discard(&staged);
			return Err(e);
		}
	};

	// Commit. From here on the old build is being replaced.
	let mut stats = Stats::default();
	if let Err(e) = commit(&jobs, &staged, &index_bytes, output, &mut stats) {
		discard(&staged);
		return Err(e);
	}

	let current: HashSet<&str> = names.iter().map(String::as_str).collect();
	for stale in previous_names.iter().filter(|name| !current.contains(**name)) {
		let file = output.join(stale);
		match std::fs::remove_file(&file) {
			Ok(()) => prune_empty_parents(&file, output),
			Err(e) if e.kind() == io::ErrorKind::NotFound => {}
			// The new build is complete and its index in place; a stale file
			// that cannot go (the game is running from it) is not worth
			// reporting the whole build as failed.
			Err(e) => stats.warnings.push(format!(
				"could not delete {}, a file of the previous build that is no longer part of it: {e}",
				file.display()
			)),
		}
	}

	if let Some(cache) = &cache {
		let fresh = shared.fresh.into_inner().unwrap_or_else(PoisonError::into_inner);
		let in_build: HashSet<&str> = plan
			.entries
			.values()
			.filter_map(|source| match source {
				Source::Project { rel, .. } => Some(rel.as_str()),
				_ => None,
			})
			.collect();
		if let Err(e) = cache.save(project, &fresh, |rel| in_build.contains(rel)) {
			stats.warnings.push(format!(
				"could not save Temp/Build/hash-cache.txt (the next build reads every file again): {e}"
			));
		}
	}
	Ok(stats)
}

/// Deletes the temporary files of staged entries that were not moved into place
/// (those that were are gone already).
fn discard(staged: &[Staged]) {
	for item in staged {
		if let Staged::Fresh { tmp, .. } = item {
			let _ = std::fs::remove_file(tmp);
		}
	}
}

/// Moves the staged data files into place and installs the new index.
///
/// The files of a folder build cannot all be replaced at once, so the order is
/// chosen to make an interruption visible and repairable instead of leaving an
/// old index over a mixture of old and new files:
///
/// 1. the new index is written next to the old one (`build.index.next`);
/// 2. the old index steps aside (`build.index.prev`): from now on the folder
///    has no `build.index`, so it is plainly not a finished build;
/// 3. the data files are replaced;
/// 4. the new index takes the name `build.index`, and the old one is deleted.
///
/// If a step fails before any data file was replaced the old index is put back
/// and nothing has changed. After that the folder is left as it is (without an
/// index), and the next build recognises the remains (see [`Previous`]).
fn commit(jobs: &[Job<'_>], staged: &[Staged], index_bytes: &[u8], output: &Path, stats: &mut Stats) -> ZResult<()> {
	let index = output.join(INDEX_FILE_NAME);
	let next = output.join(NEXT_INDEX);
	let prev = output.join(PREV_INDEX);
	let incomplete = |why: io::Error| {
		ZenithError::BuildIncomplete(format!(
			"{}: {why}. It has no build.index; build again to repair it",
			output.display()
		))
	};

	if let Err(e) = write_synced(&next, index_bytes) {
		let _ = std::fs::remove_file(&next);
		return Err(e.into());
	}
	let had_index = index.is_file();
	if had_index && let Err(e) = std::fs::rename(&index, &prev) {
		let _ = std::fs::remove_file(&next);
		return Err(e.into());
	}

	let mut moved = false;
	for (job, item) in jobs.iter().zip(staged) {
		let step = match item {
			Staged::Reused(_) => {
				stats.reused += 1;
				Ok(())
			}
			Staged::Fresh { tmp, replace, .. } => {
				stats.compressed += 1;
				if *replace {
					std::fs::rename(tmp, &job.target).inspect(|()| moved = true)
				} else {
					// Same bytes: keep the file as it is, modification time
					// included.
					std::fs::remove_file(tmp)
				}
			}
		};
		if let Err(e) = step {
			if moved {
				return Err(incomplete(e));
			}
			// Nothing of the old build has been touched yet.
			if had_index {
				let _ = std::fs::rename(&prev, &index);
			}
			let _ = std::fs::remove_file(&next);
			return Err(e.into());
		}
	}

	std::fs::rename(&next, &index).map_err(incomplete)?;
	if let Err(e) = std::fs::remove_file(&prev)
		&& e.kind() != io::ErrorKind::NotFound
	{
		stats.warnings.push(format!(
			"could not delete {}, the old index; the next build removes it: {e}",
			prev.display()
		));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use super::*;
	use crate::{builder::BuildFormat, plan::make_plan};

	fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
		let mut out = BTreeMap::new();
		let mut stack = vec![dir.to_path_buf()];
		while let Some(current) = stack.pop() {
			for entry in std::fs::read_dir(current).into_iter().flatten().flatten() {
				let path = entry.path();
				if path.is_dir() {
					stack.push(path);
				} else if let (Ok(rel), Ok(bytes)) = (path.strip_prefix(dir), std::fs::read(&path)) {
					out.insert(rel.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"), bytes);
				}
			}
		}
		out
	}

	/// The thread count must never show in the output.
	#[test]
	fn output_does_not_depend_on_the_number_of_threads() -> ZResult<()> {
		let tmp = tempfile::tempdir()?;
		let root = tmp.path().join("Game");
		let project = Project::create(&root)?;
		for n in 0..30usize {
			let text = format!(
				"file {n}
"
			)
			.repeat(200 + n * 13);
			std::fs::write(root.join(format!("Assets/f{n:02}.txt")), text)?;
		}
		let plan = make_plan(&project, &[], &Vec::new(), BuildFormat::Fs, false)?;
		let options = WriterOptions {
			compression_level: 3,
			..WriterOptions::default()
		};

		let mut outputs = Vec::new();
		for threads in [1, 2, 3, 8, 64] {
			let out = tmp.path().join(format!("out-{threads}"));
			let stats = write_folder_with_threads(&project, &plan, &out, &options, threads, false)?;
			assert_eq!(
				stats.compressed,
				plan.entries.len(),
				"{threads} threads compress everything on a first build"
			);
			outputs.push(tree(&out));
		}
		for other in outputs.iter().skip(1) {
			assert!(
				*other == *outputs.first().unwrap_or(other),
				"the output differs between thread counts"
			);
		}
		Ok(())
	}
}
