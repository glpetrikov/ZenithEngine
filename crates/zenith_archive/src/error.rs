use zenith_error::{ZResult, ZenithError};

pub fn corrupt(msg: impl Into<String>) -> ZenithError {
	ZenithError::ArchiveCorrupt(msg.into())
}

pub fn limit(msg: impl Into<String>) -> ZenithError {
	ZenithError::ArchiveLimitExceeded(msg.into())
}

/// Lengths of in-memory buffers always fit into u64 on every supported target.
pub fn to_u64(n: usize) -> u64 {
	u64::try_from(n).unwrap_or(u64::MAX)
}

/// Sizes come from untrusted files, so a value that does not fit in memory is
/// reported as a limit violation instead of being truncated.
pub fn to_usize(n: u64) -> ZResult<usize> {
	usize::try_from(n).map_err(|_| limit("value does not fit into memory"))
}
