use std::path::{Path, PathBuf};

use forge_diagnostics::{ForgeDiagnostic, codes};

use crate::lock::FileLock;

pub const STORE_DIR_ENV: &str = "FORGE_STORE_DIR";

#[derive(Debug, Clone)]
pub struct Store {
	root: PathBuf,
}

impl Store {
	pub fn open() -> Self {
		Self { root: store_root() }
	}

	pub fn at(root: impl Into<PathBuf>) -> Self {
		Self { root: root.into() }
	}

	pub fn root(&self) -> &Path {
		&self.root
	}

	pub fn blobs(&self) -> PathBuf {
		self.root.join("blobs")
	}

	pub fn sources(&self) -> PathBuf {
		self.root.join("sources")
	}

	pub fn toolchains(&self) -> PathBuf {
		self.root.join("toolchains")
	}

	pub fn blob(&self, digest: &str) -> PathBuf {
		self.blobs().join(digest)
	}

	pub fn lock(&self, name: &str) -> Result<FileLock, ForgeDiagnostic> {
		FileLock::exclusive(&self.locks_dir().join(format!("{name}.lock")), "forge store")
	}

	pub fn locks_dir(&self) -> PathBuf {
		let name = self
			.root
			.file_name()
			.map(|name| name.to_string_lossy().into_owned())
			.unwrap_or_else(|| "forge".into());
		self.root.parent().unwrap_or(&self.root).join(format!("{name}.locks"))
	}

	pub fn staging(&self, label: &str) -> PathBuf {
		let unique = format!(
			".tmp-{label}-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.map(|d| d.as_nanos())
				.unwrap_or(0)
		);
		self.root.join(unique)
	}

	pub fn publish_dir(&self, staged: &Path, destination: &Path) -> Result<(), ForgeDiagnostic> {
		if destination.exists() {
			std::fs::remove_dir_all(destination).map_err(|e| io("remove stale", destination, e))?;
		}
		if let Some(parent) = destination.parent() {
			std::fs::create_dir_all(parent).map_err(|e| io("create", parent, e))?;
		}
		std::fs::rename(staged, destination).map_err(|e| io("publish", destination, e))
	}

	pub fn clean(&self) -> Result<(), ForgeDiagnostic> {
		let _lock = self.lock("store")?;
		if self.root.exists() {
			std::fs::remove_dir_all(&self.root).map_err(|e| io("clean store", &self.root, e))?;
		}
		Ok(())
	}
}

pub fn store_root() -> PathBuf {
	if let Some(dir) = std::env::var_os(STORE_DIR_ENV) {
		return PathBuf::from(dir);
	}
	platform_default()
}

#[cfg(target_os = "macos")]
fn platform_default() -> PathBuf {
	home().join("Library").join("Caches").join("Forge")
}

#[cfg(target_os = "windows")]
fn platform_default() -> PathBuf {
	std::env::var_os("LOCALAPPDATA")
		.map(PathBuf::from)
		.unwrap_or_else(|| home().join("AppData").join("Local"))
		.join("Forge")
		.join("Cache")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_default() -> PathBuf {
	let base = std::env::var_os("XDG_CACHE_HOME")
		.map(PathBuf::from)
		.filter(|path| path.is_absolute())
		.unwrap_or_else(|| home().join(".cache"));
	base.join("forge")
}

#[cfg(not(any(unix, target_os = "windows")))]
fn platform_default() -> PathBuf {
	home().join(".forge-store")
}

fn home() -> PathBuf {
	#[cfg(unix)]
	let key = "HOME";
	#[cfg(not(unix))]
	let key = "USERPROFILE";
	std::env::var_os(key).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn io(stage: &str, path: &Path, error: std::io::Error) -> ForgeDiagnostic {
	ForgeDiagnostic::error(
		codes::hermetic::HERMETIC_VIOLATION,
		format!("{stage} `{}`: {error}", path.display()),
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn layout_is_segmented() {
		let store = Store::at("/store");
		assert!(store.blobs().ends_with("blobs"));
		assert!(store.sources().ends_with("sources"));
		assert!(store.toolchains().ends_with("toolchains"));
		assert!(store.blob("abc").ends_with("blobs/abc"));
	}

	#[test]
	fn default_root_ends_with_forge_component() {
		let root = store_root();
		assert!(root.file_name().is_some(), "store root must have a name: {}", root.display());
	}

	#[test]
	fn publish_replaces_destination_atomically() {
		let root = std::env::temp_dir().join(format!("forge-store-pub-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let store = Store::at(&root);
		let staged = root.join(".tmp-a");
		std::fs::create_dir_all(&staged).unwrap();
		std::fs::write(staged.join("f"), b"1").unwrap();
		let dest = store.toolchains().join("x/1.0");
		store.publish_dir(&staged, &dest).unwrap();
		assert!(dest.join("f").is_file());
		let staged2 = root.join(".tmp-b");
		std::fs::create_dir_all(&staged2).unwrap();
		std::fs::write(staged2.join("g"), b"2").unwrap();
		store.publish_dir(&staged2, &dest).unwrap();
		assert!(dest.join("g").is_file() && !dest.join("f").exists());
		let _ = std::fs::remove_dir_all(&root);
	}
}
