use std::{
	collections::HashMap,
	ffi::OsStr,
	fmt,
	fs::{self, File},
	io::{self, Cursor, Read, Seek, SeekFrom, Write},
	path::{Component, Path, PathBuf},
	time::{SystemTime, UNIX_EPOCH},
};

const MAGIC: &[u8; 10] = b"ZEPACK0.1\0";
const INDEX_HEADER_LEN: u64 = 18;
const TAR_BLOCK_LEN: usize = 512;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
	Io(io::Error),
	InvalidArchive(&'static str),
	InvalidPath(String),
	FileNotFound(String),
	MemoryLimit(effective_limits::Error),
	Ruzstd(ruzstd::decoding::errors::FrameDecoderError),
	Utf8(std::string::FromUtf8Error),
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Io(error) => write!(f, "{error}"),
			Self::InvalidArchive(message) => write!(f, "invalid zepack archive: {message}"),
			Self::InvalidPath(path) => write!(f, "invalid asset path: {path}"),
			Self::FileNotFound(path) => write!(f, "file not found in zepack archive: {path}"),
			Self::MemoryLimit(error) => write!(f, "failed to query memory limit: {error}"),
			Self::Ruzstd(error) => write!(f, "failed to decode zepack compressed stream: {error}"),
			Self::Utf8(error) => write!(f, "{error}"),
		}
	}
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
	fn from(error: io::Error) -> Self { Self::Io(error) }
}

impl From<effective_limits::Error> for Error {
	fn from(error: effective_limits::Error) -> Self { Self::MemoryLimit(error) }
}

impl From<ruzstd::decoding::errors::FrameDecoderError> for Error {
	fn from(error: ruzstd::decoding::errors::FrameDecoderError) -> Self { Self::Ruzstd(error) }
}

impl From<std::string::FromUtf8Error> for Error {
	fn from(error: std::string::FromUtf8Error) -> Self { Self::Utf8(error) }
}

#[derive(Debug, Clone)]
pub struct ZepackIndex {
	files: HashMap<String, IndexedFile>,
	total_uncompressed_size: u64,
}

#[derive(Debug, Clone, Copy)]
struct IndexedFile {
	tar_offset: u64,
	size: u64,
}

#[derive(Debug)]
pub struct ZepackArchive {
	path: PathBuf,
	index: ZepackIndex,
	data_offset: u64,
}

#[derive(Debug)]
pub struct SmartZepack {
	archive: ZepackArchive,
	mode: SmartMode,
}

#[derive(Debug)]
enum SmartMode {
	Memory(HashMap<String, Vec<u8>>),
	Disk { root: TempDir },
}

#[derive(Debug)]
struct TempDir {
	path: PathBuf,
}

impl Drop for TempDir {
	fn drop(&mut self) { let _ = fs::remove_dir_all(&self.path); }
}

impl ZepackIndex {
	pub fn contains(&self, path: &str) -> bool { self.files.contains_key(path) }

	pub const fn total_uncompressed_size(&self) -> u64 { self.total_uncompressed_size }

	pub fn files(&self) -> impl Iterator<Item = &str> { self.files.keys().map(String::as_str) }
}

impl ZepackArchive {
	pub fn open(path: impl AsRef<Path>) -> Result<Self> {
		let path = path.as_ref().to_path_buf();
		let mut file = File::open(&path)?;
		let mut magic = [0; 10];
		file.read_exact(&mut magic)?;
		if &magic != MAGIC {
			return Err(Error::InvalidArchive("bad magic"));
		}

		let index_len = read_u64_from(&mut file)?;
		let mut index_bytes = vec![0; usize_from_u64(index_len)?];
		file.read_exact(&mut index_bytes)?;
		let index = ZepackIndex::decode(&index_bytes)?;

		Ok(Self {
			path,
			index,
			data_offset: INDEX_HEADER_LEN + index_len,
		})
	}

	pub const fn index(&self) -> &ZepackIndex { &self.index }

	pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
		let path = normalize_archive_path(path)?;
		let Some(entry) = self.index.files.get(&path).copied() else {
			return Err(Error::FileNotFound(path));
		};

		let mut file = File::open(&self.path)?;
		file.seek(SeekFrom::Start(self.data_offset))?;
		let mut decoder = ruzstd::decoding::StreamingDecoder::new(file)?;
		copy_exact(&mut decoder, io::sink(), entry.tar_offset)?;

		let mut bytes = vec![0; usize_from_u64(entry.size)?];
		decoder.read_exact(&mut bytes)?;
		Ok(bytes)
	}
}

impl SmartZepack {
	pub fn open(path: impl AsRef<Path>) -> Result<Self> {
		let archive = ZepackArchive::open(path)?;
		let memory_limit = effective_limits::memory_limit()?;

		let mode = if archive.index.total_uncompressed_size() <= memory_limit / 3 {
			let mut files = HashMap::new();
			for path in archive.index.files() {
				files.insert(path.to_string(), archive.read_file(path)?);
			}
			SmartMode::Memory(files)
		} else {
			SmartMode::Disk {
				root: TempDir::new("zepack")?,
			}
		};

		Ok(Self { archive, mode })
	}

	pub fn contains(&self, path: &str) -> bool {
		normalize_archive_path(path)
			.ok()
			.is_some_and(|path| self.archive.index.contains(&path))
	}

	pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
		let path = normalize_archive_path(path)?;
		match &self.mode {
			SmartMode::Memory(files) => files
				.get(&path)
				.cloned()
				.ok_or_else(|| Error::FileNotFound(path.clone())),
			SmartMode::Disk { root } => {
				let target = root.path.join(&path);
				if target.exists() {
					return Ok(fs::read(target)?);
				}

				let bytes = self.archive.read_file(&path)?;
				if let Some(parent) = target.parent() {
					fs::create_dir_all(parent)?;
				}
				fs::write(&target, &bytes)?;
				Ok(bytes)
			}
		}
	}

	pub fn materialize_to_temp(&self) -> Result<PathBuf> {
		let root = TempDir::new("zepack-materialized")?;
		for path in self.archive.index.files() {
			let bytes = self.read_file(path)?;
			let target = root.path.join(path);
			if let Some(parent) = target.parent() {
				fs::create_dir_all(parent)?;
			}
			fs::write(target, bytes)?;
		}

		let path = root.path.clone();
		std::mem::forget(root);
		Ok(path)
	}
}

impl TempDir {
	fn new(prefix: &str) -> Result<Self> {
		let nanos = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.map_or(0, |duration| duration.as_nanos());
		let path = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
		fs::create_dir_all(&path)?;
		Ok(Self { path })
	}
}

pub fn pack_directory(input_dir: impl AsRef<Path>, output_file: impl AsRef<Path>) -> Result<()> {
	pack_directory_filtered(input_dir, output_file, |_| true)
}

pub fn pack_directory_excluding_names(
	input_dir: impl AsRef<Path>,
	output_file: impl AsRef<Path>,
	excluded_names: &[&str],
) -> Result<()> {
	pack_directory_filtered(input_dir, output_file, |path| {
		!path
			.components()
			.any(|component| matches!(component, Component::Normal(name) if excluded_names.iter().any(|excluded| name == OsStr::new(excluded))))
	})
}

pub fn unpack_file(input_file: impl AsRef<Path>, output_dir: impl AsRef<Path>) -> Result<()> {
	let archive = ZepackArchive::open(input_file)?;
	fs::create_dir_all(output_dir.as_ref())?;

	let mut file = File::open(&archive.path)?;
	file.seek(SeekFrom::Start(archive.data_offset))?;
	let decoder = ruzstd::decoding::StreamingDecoder::new(file)?;
	let mut tar_archive = tar::Archive::new(decoder);
	tar_archive.unpack(output_dir)?;
	Ok(())
}

fn pack_directory_filtered(
	input_dir: impl AsRef<Path>,
	output_file: impl AsRef<Path>,
	include_path: impl Fn(&Path) -> bool,
) -> Result<()> {
	let input_dir = input_dir.as_ref();
	let mut files = Vec::new();
	collect_files(input_dir, input_dir, &include_path, &mut files)?;
	files.sort_by(|left, right| left.0.cmp(&right.0));

	let mut tar_bytes = Vec::new();
	let mut index = ZepackIndex {
		files: HashMap::new(),
		total_uncompressed_size: 0,
	};

	for (archive_path, source_path) in files {
		let bytes = fs::read(&source_path)?;
		let size = u64::try_from(bytes.len()).map_err(|_| Error::InvalidArchive("file is too large"))?;
		let tar_offset =
			u64::try_from(tar_bytes.len()).map_err(|_| Error::InvalidArchive("tar stream is too large"))? + 512;

		let mut header = tar::Header::new_gnu();
		header.set_path(&archive_path)?;
		header.set_size(size);
		header.set_mode(0o644);
		header.set_cksum();

		tar_bytes.extend_from_slice(header.as_bytes());
		tar_bytes.extend_from_slice(&bytes);
		write_tar_padding(&mut tar_bytes, size);

		index.files.insert(archive_path, IndexedFile { tar_offset, size });
		index.total_uncompressed_size = index.total_uncompressed_size.saturating_add(size);
	}

	tar_bytes.extend_from_slice(&[0; TAR_BLOCK_LEN * 2]);

	let index_bytes = index.encode()?;
	let output_file = output_file.as_ref();
	if let Some(parent) = output_file.parent()
		&& !parent.as_os_str().is_empty()
	{
		fs::create_dir_all(parent)?;
	}

	let mut output = File::create(output_file)?;
	output.write_all(MAGIC)?;
	output.write_all(
		&u64::try_from(index_bytes.len())
			.map_err(|_| Error::InvalidArchive("index is too large"))?
			.to_le_bytes(),
	)?;
	output.write_all(&index_bytes)?;
	output.write_all(&ruzstd_compress(&tar_bytes))?;
	Ok(())
}

fn ruzstd_compress(input: &[u8]) -> Vec<u8> {
	ruzstd::encoding::compress_to_vec(input, ruzstd::encoding::CompressionLevel::Fastest)
}

fn collect_files(
	root: &Path,
	directory: &Path,
	include_path: &impl Fn(&Path) -> bool,
	files: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
	for entry in fs::read_dir(directory)? {
		let entry = entry?;
		let source_path = entry.path();
		let relative = source_path
			.strip_prefix(root)
			.map_err(|_| Error::InvalidArchive("failed to build relative asset path"))?;

		if !include_path(relative) {
			continue;
		}

		if source_path.is_dir() {
			collect_files(root, &source_path, include_path, files)?;
		} else {
			files.push((archive_path_string(relative)?, source_path));
		}
	}

	Ok(())
}

fn archive_path_string(path: &Path) -> Result<String> {
	let mut parts = Vec::new();
	for component in path.components() {
		let Component::Normal(part) = component else {
			return Err(Error::InvalidPath(path.display().to_string()));
		};
		let Some(part) = part.to_str() else {
			return Err(Error::InvalidPath(path.display().to_string()));
		};
		parts.push(part);
	}
	normalize_archive_path(&parts.join("/"))
}

fn normalize_archive_path(path: &str) -> Result<String> {
	let path = Path::new(path);
	if path.is_absolute() {
		return Err(Error::InvalidPath(path.display().to_string()));
	}

	let mut parts = Vec::new();
	for component in path.components() {
		match component {
			Component::Normal(part) => {
				let Some(part) = part.to_str() else {
					return Err(Error::InvalidPath(path.display().to_string()));
				};
				parts.push(part);
			}
			Component::CurDir => {}
			_ => return Err(Error::InvalidPath(path.display().to_string())),
		}
	}

	Ok(parts.join("/"))
}

fn write_tar_padding(output: &mut Vec<u8>, size: u64) {
	let padding = (TAR_BLOCK_LEN as u64 - size % TAR_BLOCK_LEN as u64) % TAR_BLOCK_LEN as u64;
	output.resize(output.len() + usize::try_from(padding).unwrap_or(0), 0);
}

fn copy_exact(mut reader: impl Read, mut writer: impl Write, len: u64) -> Result<()> {
	let mut limited = reader.by_ref().take(len);
	io::copy(&mut limited, &mut writer)?;
	if limited.limit() != 0 {
		return Err(Error::InvalidArchive("compressed stream ended early"));
	}
	Ok(())
}

fn read_u64_from(reader: &mut impl Read) -> Result<u64> {
	let mut bytes = [0; 8];
	reader.read_exact(&mut bytes)?;
	Ok(u64::from_le_bytes(bytes))
}

fn read_u32(input: &mut Cursor<&[u8]>) -> Result<u32> {
	let mut bytes = [0; 4];
	input.read_exact(&mut bytes)?;
	Ok(u32::from_le_bytes(bytes))
}

fn read_u64(input: &mut Cursor<&[u8]>) -> Result<u64> {
	let mut bytes = [0; 8];
	input.read_exact(&mut bytes)?;
	Ok(u64::from_le_bytes(bytes))
}

fn usize_from_u64(value: u64) -> Result<usize> {
	usize::try_from(value).map_err(|_| Error::InvalidArchive("value does not fit in memory"))
}

impl ZepackIndex {
	fn encode(&self) -> Result<Vec<u8>> {
		let mut output = Vec::new();
		output.extend_from_slice(&self.total_uncompressed_size.to_le_bytes());
		output.extend_from_slice(
			&u32::try_from(self.files.len())
				.map_err(|_| Error::InvalidArchive("too many files"))?
				.to_le_bytes(),
		);

		let mut files = self.files.iter().collect::<Vec<_>>();
		files.sort_by(|left, right| left.0.cmp(right.0));
		for (path, entry) in files {
			output.extend_from_slice(
				&u32::try_from(path.len())
					.map_err(|_| Error::InvalidArchive("path is too long"))?
					.to_le_bytes(),
			);
			output.extend_from_slice(path.as_bytes());
			output.extend_from_slice(&entry.tar_offset.to_le_bytes());
			output.extend_from_slice(&entry.size.to_le_bytes());
		}

		Ok(output)
	}

	fn decode(bytes: &[u8]) -> Result<Self> {
		let mut input = Cursor::new(bytes);
		let total_uncompressed_size = read_u64(&mut input)?;
		let file_count = read_u32(&mut input)?;
		let mut files = HashMap::new();

		for _ in 0..file_count {
			let path_len = read_u32(&mut input)?;
			let mut path = vec![0; usize::try_from(path_len).map_err(|_| Error::InvalidArchive("path is too long"))?];
			input.read_exact(&mut path)?;
			let path = normalize_archive_path(&String::from_utf8(path)?)?;
			let tar_offset = read_u64(&mut input)?;
			let size = read_u64(&mut input)?;
			files.insert(path, IndexedFile { tar_offset, size });
		}

		Ok(Self {
			files,
			total_uncompressed_size,
		})
	}
}
