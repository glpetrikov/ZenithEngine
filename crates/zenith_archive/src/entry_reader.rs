use std::{
	io::{self, Read, Seek, SeekFrom},
	sync::Arc,
};

use zenith_error::{ZResult, ZenithError};

use crate::{
	ctx::FileCtx,
	error::{to_u64, to_usize},
	layout::FileLayout,
};

/// Streaming reader over one entry. Decodes a chunk at a time and keeps only
/// the chunk it is currently in, so memory use is one chunk regardless of the
/// entry size.
pub struct EntryReader {
	ctx: Arc<FileCtx>,
	path: Arc<str>,
	layout: Arc<FileLayout>,
	pos: u64,
	cached: Option<(usize, Vec<u8>)>,
}

fn to_io(err: ZenithError) -> io::Error {
	match err {
		ZenithError::Io(e) => e,
		other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
	}
}

impl EntryReader {
	pub const fn new(ctx: Arc<FileCtx>, path: Arc<str>, layout: Arc<FileLayout>) -> Self {
		Self {
			ctx,
			path,
			layout,
			pos: 0,
			cached: None,
		}
	}

	/// Uncompressed size of the entry.
	#[must_use]
	pub fn size(&self) -> u64 {
		self.layout.size
	}

	/// Reads the whole entry. The buffer grows chunk by chunk instead of being
	/// sized from the (untrusted) declared size.
	///
	/// # Errors
	/// Fails if any chunk is damaged, truncated, or does not authenticate.
	pub fn read_all(&mut self) -> ZResult<Vec<u8>> {
		self.pos = 0;
		// A file of one chunk (most of them) is that chunk: no second buffer.
		if let [_] = self.layout.chunks.as_slice() {
			let data = self.ctx.read_chunk(&self.layout, &self.path, 0)?;
			self.pos = self.layout.size;
			return Ok(data);
		}
		let mut out = Vec::with_capacity(to_usize(self.layout.size.min(16 * 1024 * 1024))?);
		let mut idx = 0;
		while idx < self.layout.chunks.len() {
			out.extend_from_slice(&self.ctx.read_chunk(&self.layout, &self.path, idx)?);
			idx = idx.saturating_add(1);
		}
		self.pos = self.layout.size;
		Ok(out)
	}

	fn chunk(&mut self, idx: usize) -> ZResult<&[u8]> {
		if self.cached.as_ref().is_none_or(|(cached, _)| *cached != idx) {
			let data = self.ctx.read_chunk(&self.layout, &self.path, idx)?;
			self.cached = Some((idx, data));
		}
		Ok(self.cached.as_ref().map_or(&[][..], |(_, data)| data.as_slice()))
	}
}

impl Read for EntryReader {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		if buf.is_empty() || self.pos >= self.layout.size {
			return Ok(0);
		}
		let chunk_size = to_u64(self.ctx.header.chunk_size);
		let idx = to_usize(self.pos.div_euclid(chunk_size)).map_err(to_io)?;
		let within = to_usize(self.pos.rem_euclid(chunk_size)).map_err(to_io)?;
		let data = self.chunk(idx).map_err(to_io)?;
		let available = data
			.get(within..)
			.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "read position outside of chunk"))?;
		let n = available.len().min(buf.len());
		if let (Some(dst), Some(src)) = (buf.get_mut(..n), available.get(..n)) {
			dst.copy_from_slice(src);
		}
		self.pos = self.pos.saturating_add(to_u64(n));
		Ok(n)
	}
}

impl Seek for EntryReader {
	fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
		let target = match from {
			SeekFrom::Start(n) => Some(n),
			SeekFrom::End(delta) => self.layout.size.checked_add_signed(delta),
			SeekFrom::Current(delta) => self.pos.checked_add_signed(delta),
		};
		let target = target.ok_or_else(|| {
			io::Error::new(
				io::ErrorKind::InvalidInput,
				"seek to a negative or overflowing position",
			)
		})?;
		self.pos = target;
		Ok(target)
	}
}
