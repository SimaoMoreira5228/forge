use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use forge_diagnostics::{ForgeDiagnostic, codes};

pub struct FileLock {
	_file: File,
	path: PathBuf,
}

impl FileLock {
	pub fn exclusive(path: &Path, purpose: &str) -> Result<Self, ForgeDiagnostic> {
		Self::acquire(path, purpose, true)
	}

	pub fn shared(path: &Path, purpose: &str) -> Result<Self, ForgeDiagnostic> {
		Self::acquire(path, purpose, false)
	}

	pub fn try_exclusive(path: &Path) -> Result<Option<Self>, ForgeDiagnostic> {
		let file = open(path)?;
		match file.try_lock() {
			Ok(()) => Ok(Some(Self {
				_file: file,
				path: path.to_path_buf(),
			})),
			Err(TryLockError::WouldBlock) => Ok(None),
			Err(TryLockError::Error(e)) => Err(io_error(path, e)),
		}
	}

	pub fn path(&self) -> &Path {
		&self.path
	}

	fn acquire(path: &Path, purpose: &str, exclusive: bool) -> Result<Self, ForgeDiagnostic> {
		let file = open(path)?;
		let attempt = if exclusive { file.try_lock() } else { file.try_lock_shared() };
		match attempt {
			Ok(()) => {}
			Err(TryLockError::WouldBlock) => {
				eprintln!("waiting: another forge process holds the {purpose} lock ({})", path.display());
				let locked = if exclusive { file.lock() } else { file.lock_shared() };
				locked.map_err(|e| io_error(path, e))?;
			}
			Err(TryLockError::Error(e)) => return Err(io_error(path, e)),
		}
		Ok(Self {
			_file: file,
			path: path.to_path_buf(),
		})
	}
}

fn open(path: &Path) -> Result<File, ForgeDiagnostic> {
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
	}
	OpenOptions::new()
		.create(true)
		.truncate(false)
		.read(true)
		.write(true)
		.open(path)
		.map_err(|e| io_error(path, e))
}

fn io_error(path: &Path, error: std::io::Error) -> ForgeDiagnostic {
	ForgeDiagnostic::error(
		codes::hermetic::HERMETIC_VIOLATION,
		format!("lock `{}`: {error}", path.display()),
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn exclusive_lock_excludes_second_holder() {
		let path = std::env::temp_dir().join(format!("forge-lock-{}", std::process::id()));
		let _ = std::fs::remove_file(&path);
		let held = FileLock::exclusive(&path, "test").unwrap();
		assert!(
			FileLock::try_exclusive(&path).unwrap().is_none(),
			"second holder must not acquire"
		);
		drop(held);
		assert!(
			FileLock::try_exclusive(&path).unwrap().is_some(),
			"released lock is acquirable"
		);
		let _ = std::fs::remove_file(&path);
	}
}
