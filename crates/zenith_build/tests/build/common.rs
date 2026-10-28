use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
	sync::Arc,
};

use sha2::{Digest, Sha256};
use zenith_archive::{ArchiveReader, EncryptionKey};
use zenith_filesystem::Project;

pub type TestResult = Result<(), Box<dyn std::error::Error>>;

pub const fn key(seed: u8) -> EncryptionKey {
	EncryptionKey::new([seed; 32])
}

pub fn hex(bytes: &[u8]) -> String {
	bytes
		.iter()
		.flat_map(|b| [b >> 4, b & 0x0f])
		.map(|n| char::from_digit(u32::from(n), 16).unwrap_or('0'))
		.collect()
}

pub fn sha256(bytes: &[u8]) -> String {
	hex(&Sha256::digest(bytes))
}

/// Deterministic incompressible bytes (xorshift).
pub fn random_bytes(len: usize, seed: u64) -> Vec<u8> {
	let mut x = seed | 1;
	(0..len)
		.map(|_| {
			x ^= x << 13;
			x ^= x >> 7;
			x ^= x << 17;
			(x & 0xff) as u8
		})
		.collect()
}

pub fn write(root: &Path, rel: &str, bytes: &[u8]) -> std::io::Result<()> {
	let path = root.join(rel);
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent)?;
	}
	std::fs::write(path, bytes)
}

/// A project named `Game` with files in and outside the base list.
pub fn standard_project() -> Result<(tempfile::TempDir, PathBuf), Box<dyn std::error::Error>> {
	let tmp = tempfile::tempdir()?;
	let root = tmp.path().join("Game");
	Project::create(&root)?;
	write(&root, "Assets/Worlds/A.zenith", b"world a")?;
	write(&root, "Assets/tex/a.png", &random_bytes(10_000, 1))?;
	write(&root, "Assets/Scripts/lib.rs", b"fn main() {}\n")?;
	write(&root, "Settings/Game.toml", b"a = 1\n")?;
	write(&root, "Packages/p.txt", b"package")?;
	write(&root, "Temp/junk.txt", b"junk")?;
	write(&root, "Docs/readme.txt", b"docs")?;
	Ok((tmp, root))
}

/// Every file below `dir`: relative path to the SHA-256 of its content.
pub fn tree_hashes(dir: &Path) -> BTreeMap<String, String> {
	fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
		let Ok(entries) = std::fs::read_dir(dir) else { return };
		for entry in entries.flatten() {
			let path = entry.path();
			if path.is_dir() {
				walk(base, &path, out);
			} else if let (Ok(rel), Ok(bytes)) = (path.strip_prefix(base), std::fs::read(&path)) {
				out.insert(rel.to_string_lossy().replace('\\', "/"), sha256(&bytes));
			}
		}
	}
	let mut out = BTreeMap::new();
	walk(dir, dir, &mut out);
	out
}

/// Paths and contents of an archive build.
pub fn archive_contents(
	file: &Path,
	key: Option<&EncryptionKey>,
) -> Result<BTreeMap<String, Vec<u8>>, Box<dyn std::error::Error>> {
	let reader = ArchiveReader::open(Arc::<[u8]>::from(std::fs::read(file)?), key)?;
	reader
		.entries()
		.iter()
		.map(|e| Ok((e.path.clone(), reader.read_all(&e.path)?)))
		.collect()
}

pub fn archive_paths(file: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
	Ok(archive_contents(file, None)?.into_keys().collect())
}
