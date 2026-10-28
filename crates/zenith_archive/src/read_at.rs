//! Positional reading. Containers are read through this trait so the same
//! reader works on files (native) and on downloaded bytes (web).

use std::{fs::File, io, path::Path, sync::Arc};

use zenith_error::ZResult;

use crate::error::{corrupt, to_u64};

/// Random-access source of bytes. Implementations must be usable from several
/// threads at once, so reads take `&self` and carry their own offset.
pub trait ReadAt: Send + Sync {
	/// Total length of the source in bytes.
	fn len(&self) -> u64;

	/// Reads up to `buf.len()` bytes starting at `offset`; returns 0 at or past
	/// the end.
	///
	/// # Errors
	/// Propagates the underlying I/O error.
	fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize>;

	fn is_empty(&self) -> bool {
		self.len() == 0
	}
}

impl<T: ReadAt + ?Sized> ReadAt for Arc<T> {
	fn len(&self) -> u64 {
		(**self).len()
	}

	fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
		(**self).read_at(offset, buf)
	}
}

/// Data that is already in memory (a downloaded build, a test fixture).
impl ReadAt for Arc<[u8]> {
	fn len(&self) -> u64 {
		to_u64(<[u8]>::len(self))
	}

	fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
		let Ok(start) = usize::try_from(offset) else {
			return Ok(0);
		};
		let Some(rest) = self.get(start..) else {
			return Ok(0);
		};
		let n = rest.len().min(buf.len());
		if let (Some(dst), Some(src)) = (buf.get_mut(..n), rest.get(..n)) {
			dst.copy_from_slice(src);
			Ok(n)
		} else {
			Ok(0)
		}
	}
}

/// A file read without moving a shared cursor where the platform allows it.
pub struct FileSource {
	#[cfg(any(unix, windows))]
	file: File,
	// No positional read on this target: serialize seek+read behind a lock.
	#[cfg(not(any(unix, windows)))]
	file: std::sync::Mutex<File>,
	len: u64,
}

impl FileSource {
	/// Wraps an open file.
	///
	/// # Errors
	/// Fails if the file length cannot be read.
	pub fn new(file: File) -> io::Result<Self> {
		let len = file.metadata()?.len();
		Ok(Self {
			#[cfg(any(unix, windows))]
			file,
			#[cfg(not(any(unix, windows)))]
			file: std::sync::Mutex::new(file),
			len,
		})
	}

	/// Opens a file for reading.
	///
	/// # Errors
	/// Fails if the file cannot be opened.
	pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
		Self::new(File::open(path)?)
	}
}

impl ReadAt for FileSource {
	fn len(&self) -> u64 {
		self.len
	}

	#[cfg(unix)]
	fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
		std::os::unix::fs::FileExt::read_at(&self.file, buf, offset)
	}

	#[cfg(windows)]
	fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
		std::os::windows::fs::FileExt::seek_read(&self.file, buf, offset)
	}

	#[cfg(not(any(unix, windows)))]
	fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
		use std::io::{Read, Seek, SeekFrom};
		let mut file = self.file.lock().map_err(|_| io::Error::other("file lock poisoned"))?;
		file.seek(SeekFrom::Start(offset))?;
		file.read(buf)
	}
}

/// Fills `buf` completely or fails: a short source means a truncated container.
pub fn read_exact_at(src: &dyn ReadAt, offset: u64, buf: &mut [u8]) -> ZResult<()> {
	let end = offset
		.checked_add(to_u64(buf.len()))
		.ok_or_else(|| corrupt("offset overflow"))?;
	if end > src.len() {
		return Err(corrupt("read past the end of the file (truncated?)"));
	}
	let mut offset = offset;
	let mut rest = buf;
	while !rest.is_empty() {
		let n = src.read_at(offset, rest)?;
		if n == 0 {
			return Err(corrupt("unexpected end of file"));
		}
		let (_, tail) = std::mem::take(&mut rest)
			.split_at_mut_checked(n)
			.ok_or_else(|| corrupt("source returned more bytes than requested"))?;
		rest = tail;
		offset = offset
			.checked_add(to_u64(n))
			.ok_or_else(|| corrupt("offset overflow"))?;
	}
	Ok(())
}
