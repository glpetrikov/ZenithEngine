//! Randomised damage. The seeds are fixed, so a failure can be replayed.
//!
//! * Whatever is done to a container, reading all of it either fails or gives
//!   back exactly what was written. (Checksums and the AEAD tag are what make
//!   this true; no single flipped byte is needed to see it break.)
//! * Whatever is done to the index with its checksums made valid again, the
//!   reader never panics and never returns a file of another size than the
//!   index states.

use std::{collections::BTreeMap, sync::Arc};

use zenith_archive::{ArchiveReader, EncryptionKey, FolderReader};

use crate::common::{FOOTER_LEN, TestResult, body_offset, build_archive, build_folder, fix_footer, key, sample_files};

struct Rng(u64);

impl Rng {
	const fn next(&mut self) -> u64 {
		self.0 ^= self.0 << 13;
		self.0 ^= self.0 >> 7;
		self.0 ^= self.0 << 17;
		self.0
	}

	fn below(&mut self, n: usize) -> usize {
		usize::try_from(self.next() % u64::try_from(n.max(1)).unwrap_or(1)).unwrap_or(0)
	}

	fn byte(&mut self) -> u8 {
		u8::try_from(self.next() & 0xff).unwrap_or(0)
	}
}

/// One to four random edits: flip a byte, overwrite a few, cut the end off, or
/// append garbage.
fn damage(bytes: &[u8], rng: &mut Rng) -> Vec<u8> {
	let mut out = bytes.to_vec();
	for _ in 0..=rng.below(4) {
		match rng.below(4) {
			0 => {
				let at = rng.below(out.len());
				if let Some(b) = out.get_mut(at) {
					*b ^= rng.byte().max(1);
				}
			}
			1 => {
				let at = rng.below(out.len());
				for offset in 0..=rng.below(8) {
					if let Some(b) = out.get_mut(at + offset) {
						*b = rng.byte();
					}
				}
			}
			2 => out.truncate(rng.below(out.len())),
			_ => (0..=rng.below(40)).for_each(|_| out.push(rng.byte())),
		}
	}
	out
}

/// Reads everything; `Some` only if every entry could be read.
fn read_archive(bytes: &[u8], k: Option<&EncryptionKey>) -> Option<BTreeMap<String, Vec<u8>>> {
	let reader = ArchiveReader::open(Arc::<[u8]>::from(bytes), k).ok()?;
	reader
		.entries()
		.iter()
		.map(|e| Some((e.path.clone(), reader.read_all(&e.path).ok()?)))
		.collect()
}

#[test]
fn damaged_archives_fail_or_come_back_exactly() -> TestResult {
	let files = sample_files();
	for (round, k) in [None, Some(key(9))].into_iter().enumerate() {
		let base = build_archive(&files, k.clone())?;
		let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (round as u64 + 1));
		for i in 0..1500 {
			let damaged = damage(&base, &mut rng);
			if damaged == base {
				continue;
			}
			if let Some(read) = read_archive(&damaged, k.as_ref()) {
				assert_eq!(
					read, files,
					"round {round}, iteration {i}: damaged bytes were accepted and changed the content"
				);
			}
		}
	}
	Ok(())
}

#[test]
fn damaged_folder_builds_fail_or_come_back_exactly() -> TestResult {
	let files = sample_files();
	for (round, k) in [None, Some(key(9))].into_iter().enumerate() {
		let base = build_folder(&files, k.clone())?;
		let names: Vec<String> = base.files.keys().cloned().collect();
		let mut rng = Rng(0xD1B5_4A32_D192_ED03 ^ (round as u64 + 1));
		for i in 0..1000 {
			let victim = names.get(rng.below(names.len())).ok_or("no files")?;
			let original = base.files.get(victim).ok_or("missing")?;
			let damaged = damage(original, &mut rng);
			if damaged.as_slice() == &**original {
				continue;
			}
			let mut folder = build_folder(&files, k.clone())?;
			folder.files.insert(victim.clone(), Arc::from(damaged));
			let Ok(reader) = FolderReader::open(folder.mem_dir(), k.as_ref()) else {
				continue;
			};
			let read: Option<BTreeMap<String, Vec<u8>>> = reader
				.entries()
				.iter()
				.map(|e| Some((e.path.clone(), reader.read_all(&e.path).ok()?)))
				.collect();
			if let Some(read) = read {
				assert_eq!(
					read, files,
					"round {round}, iteration {i}, file {victim}: damaged bytes were accepted and changed the content"
				);
			}
		}
	}
	Ok(())
}

#[test]
fn an_index_with_valid_checksums_never_makes_the_reader_panic_or_lie_about_sizes() -> TestResult {
	let files = sample_files();
	let base = build_archive(&files, None)?;
	let (body, end) = (body_offset(&base), base.len() - FOOTER_LEN);
	let mut rng = Rng(0x2545_F491_4F6C_DD1D);
	for i in 0..3000 {
		let mut bytes = base.clone();
		for _ in 0..=rng.below(3) {
			let at = body + rng.below(end - body);
			if let Some(b) = bytes.get_mut(at) {
				*b = rng.byte();
			}
		}
		fix_footer(&mut bytes, false);
		let Ok(reader) = ArchiveReader::open(Arc::<[u8]>::from(bytes), None) else {
			continue;
		};
		for entry in reader.entries() {
			if let Ok(data) = reader.read_all(&entry.path) {
				assert_eq!(
					u64::try_from(data.len())?,
					entry.size,
					"iteration {i}: {} came back with another size than the index states",
					entry.path
				);
			}
		}
	}
	Ok(())
}

#[test]
fn a_folder_index_with_valid_checksums_never_makes_the_reader_panic_or_lie_about_sizes() -> TestResult {
	let files = sample_files();
	let base = build_folder(&files, None)?;
	let index = base.files.get("build.index").ok_or("no index")?.to_vec();
	let (body, end) = (body_offset(&index), index.len() - FOOTER_LEN);
	let mut rng = Rng(0x8F1B_BCDC_A5A5_5A5A);
	for i in 0..3000 {
		let mut bytes = index.clone();
		for _ in 0..=rng.below(3) {
			let at = body + rng.below(end - body);
			if let Some(b) = bytes.get_mut(at) {
				*b = rng.byte();
			}
		}
		fix_footer(&mut bytes, false);
		let mut folder = build_folder(&files, None)?;
		folder.files.insert("build.index".to_string(), Arc::from(bytes));
		let Ok(reader) = FolderReader::open(folder.mem_dir(), None) else {
			continue;
		};
		for entry in reader.entries() {
			if let Ok(data) = reader.read_all(&entry.path) {
				assert_eq!(
					u64::try_from(data.len())?,
					entry.size,
					"iteration {i}: {} came back with another size than the index states",
					entry.path
				);
			}
		}
	}
	Ok(())
}

#[test]
fn a_data_file_with_valid_checksums_never_makes_the_reader_panic_or_lie_about_sizes() -> TestResult {
	let files = sample_files();
	let base = build_folder(&files, None)?;
	let names: Vec<String> = base.files.keys().filter(|n| *n != "build.index").cloned().collect();
	let mut rng = Rng(0x0123_4567_89AB_CDEF);
	for i in 0..3000 {
		let victim = names.get(rng.below(names.len())).ok_or("no data files")?;
		let mut bytes = base.files.get(victim).ok_or("missing")?.to_vec();
		// The tail of a data file is the chunk table and the footer.
		let (body, end) = (body_offset(&bytes), bytes.len() - FOOTER_LEN);
		for _ in 0..=rng.below(2) {
			let at = body + rng.below((end - body).max(1));
			if let Some(b) = bytes.get_mut(at) {
				*b = rng.byte();
			}
		}
		fix_footer(&mut bytes, false);
		let mut folder = build_folder(&files, None)?;
		folder.files.insert(victim.clone(), Arc::from(bytes));
		let reader = FolderReader::open(folder.mem_dir(), None)?;
		for entry in reader.entries() {
			if let Ok(data) = reader.read_all(&entry.path) {
				assert_eq!(
					u64::try_from(data.len())?,
					entry.size,
					"iteration {i}, {victim}: {} came back with another size than the index states",
					entry.path
				);
			}
		}
	}
	Ok(())
}
