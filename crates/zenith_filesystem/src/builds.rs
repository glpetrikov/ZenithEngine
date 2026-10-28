//! Read-only backends: a single-file build (`BuildArchive`) and a folder of
//! separate files (`BuildFs`). Paths inside a build are relative to the
//! project root and `/`-separated, so a `ZPath` resolves exactly as on `Fs`.

use zenith_archive::{
	ArchiveReader, BuildId, DirSource, FolderReader, ReadAt, archive_is_encrypted, folder_is_encrypted,
};
use zenith_error::{ZResult, ZenithError};

use crate::{
	cache::{CacheConfig, CacheSettings, Stamp},
	core::{Backend, Core},
	layout::{CACHE_CONFIG_FILE, MANIFEST_EXTENSION, manifest_file_name},
	manifest::Manifest,
	options::OpenOptions,
	tree::Tree,
	types::{DirEntry, EntryKind, Metadata, ReadSeek},
};

/// The manifest and the cache settings are small text files; a bigger one in a
/// build is a sign of a hostile index.
const MAX_CONFIG_SIZE: u64 = 1024 * 1024;

fn check_config_size(files: &[(String, u64)], path: &str) -> ZResult<()> {
	match files.iter().find(|(p, _)| p == path) {
		Some((_, size)) if *size > MAX_CONFIG_SIZE => Err(ZenithError::FileTooLarge(format!(
			"{path} in the build is {size} bytes, the limit for a settings file is {MAX_CONFIG_SIZE}"
		))),
		_ => Ok(()),
	}
}

/// Manifest and cache settings of a build, found by name inside it.
fn load_common(
	files: &[(String, u64)],
	read: &dyn Fn(&str) -> ZResult<Vec<u8>>,
	opts: &OpenOptions,
) -> ZResult<(Manifest, CacheSettings)> {
	let suffix = format!(".{MANIFEST_EXTENSION}");
	let manifest_file = files
		.iter()
		.map(|(path, _)| path.as_str())
		.find(|path| !path.contains('/') && path.ends_with(&suffix))
		.ok_or_else(|| ZenithError::InvalidProject("the build has no manifest".into()))?;
	check_config_size(files, manifest_file)?;
	let text = String::from_utf8(read(manifest_file)?)
		.map_err(|_| ZenithError::InvalidProject("the manifest is not valid UTF-8".into()))?;
	let manifest = Manifest::parse(&text)?;
	if manifest_file != manifest_file_name(&manifest.name) {
		return Err(ZenithError::InvalidProject(format!(
			"manifest file {manifest_file} does not match the project name {}",
			manifest.name
		)));
	}
	let settings = match &opts.cache {
		CacheConfig::Manual(settings) => settings.clone(),
		CacheConfig::File => {
			if files.iter().any(|(path, _)| path == CACHE_CONFIG_FILE) {
				check_config_size(files, CACHE_CONFIG_FILE)?;
				CacheSettings::parse(&String::from_utf8_lossy(&read(CACHE_CONFIG_FILE)?))?
			} else {
				CacheSettings::default()
			}
		}
	};
	Ok((manifest, settings))
}

fn not_found(rel: &str) -> ZenithError {
	ZenithError::ProjectNotFound(rel.to_string())
}

macro_rules! build_backend {
	($(#[$doc:meta])* $name:ident, $reader:ty, $tag:literal) => {
		$(#[$doc])*
		pub struct $name {
			core: Core,
			reader: $reader,
			tree: Tree,
		}

		impl $name {
			fn assemble(reader: $reader, opts: &OpenOptions) -> ZResult<Self> {
				if let Some(expected) = opts.expected_build_id
					&& expected != reader.build_id()
				{
					return Err(ZenithError::BuildIdMismatch(
						expected.to_hex(),
						reader.build_id().to_hex(),
					));
				}
				let files: Vec<(String, u64)> =
					reader.entries().iter().map(|e| (e.path.clone(), e.size)).collect();
				let (manifest, settings) = load_common(&files, &|rel| reader.read_all(rel), opts)?;
				let tree = Tree::from_files(files.iter().map(|(p, s)| (p.as_str(), *s)));
				Ok(Self { core: Core::new(manifest, settings, $tag, opts)?, reader, tree })
			}

			fn file(&self, rel: &str) -> ZResult<()> {
				match self.tree.metadata(rel) {
					Some(m) if m.kind == EntryKind::File => Ok(()),
					Some(_) => Err(ZenithError::ProjectWrongKind(format!("{rel} is a folder"))),
					None => Err(not_found(rel)),
				}
			}
		}

		impl Backend for $name {
			fn core(&self) -> &Core {
				&self.core
			}

			fn exists(&self, rel: &str) -> ZResult<bool> {
				Ok(self.tree.metadata(rel).is_some())
			}

			fn list(&self, rel: &str) -> ZResult<Vec<DirEntry>> {
				let children = self.tree.children(rel).ok_or_else(|| not_found(rel))?;
				Ok(children.map(|(name, kind)| DirEntry { name: name.to_string(), kind }).collect())
			}

			fn metadata(&self, rel: &str) -> ZResult<Metadata> {
				self.tree.metadata(rel).ok_or_else(|| not_found(rel))
			}

			fn is_symlink(&self, rel: &str) -> ZResult<bool> {
				self.metadata(rel).map(|_| false)
			}

			fn read_raw(&self, rel: &str) -> ZResult<Vec<u8>> {
				self.file(rel)?;
				self.reader.read_all(rel)
			}

			fn open_read(&self, rel: &str) -> ZResult<Box<dyn ReadSeek + Send>> {
				self.file(rel)?;
				Ok(Box::new(self.reader.open_entry(rel)?))
			}

			fn stamp(&self, _rel: &str) -> Option<Stamp> {
				// A build never changes, so a cached copy never goes stale.
				None
			}

			fn build_id(&self) -> Option<BuildId> {
				Some(self.reader.build_id())
			}
		}
	};
}

build_backend!(
	/// Version 1 of the single-file build backend.
	ArchiveV1,
	ArchiveReader,
	"archive"
);
build_backend!(
	/// Version 1 of the folder build backend.
	BuildFsV1,
	FolderReader,
	"folder"
);

impl ArchiveV1 {
	pub fn open(src: impl ReadAt + 'static, opts: &OpenOptions) -> ZResult<Self> {
		// A key nobody needs is ignored here; the reader itself stays strict.
		let key = if archive_is_encrypted(&src)? {
			opts.key.as_ref()
		} else {
			None
		};
		Self::assemble(ArchiveReader::open(src, key)?, opts)
	}
}

impl BuildFsV1 {
	pub fn open(source: impl DirSource + 'static, opts: &OpenOptions) -> ZResult<Self> {
		let key = if folder_is_encrypted(&*source.open_index()?)? {
			opts.key.as_ref()
		} else {
			None
		};
		Self::assemble(FolderReader::open(source, key)?, opts)
	}
}
