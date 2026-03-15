mod manager;

pub use manager::{Dependency, Package, PackageManager, PackageSource};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("Failed to fetch package: {0}")]
	FetchFailed(String),

	#[error("Package not found: {0}")]
	NotFound(String),

	#[error("Invalid dependency specification: {0}")]
	InvalidDep(String),

	#[error("IO error: {0}")]
	Io(#[from] std::io::Error),
}
