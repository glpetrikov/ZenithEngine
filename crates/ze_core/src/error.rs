#[derive(Debug)]
pub enum ZeroError {
	WindowCreationFailed(String),
	Unknown(String),
}

impl std::fmt::Display for ZeroError {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		match self {
			Self::WindowCreationFailed(s) => write!(f, "Window creation failed: {s}"),
			Self::Unknown(s) => write!(f, "Unknown error: {s}"),
		}
	}
}

impl std::error::Error for ZeroError {}

pub type ZResult<T> = Result<T, ZeroError>;
