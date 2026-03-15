use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Toolchain {
	pub id: String,
	pub name: String,
	pub version: String,
	pub path: PathBuf,
	pub fingerprint: String,
}

impl Toolchain {
	pub fn new(id: impl Into<String>, name: impl Into<String>, version: impl Into<String>, path: PathBuf) -> Self {
		let version_str = version.into();
		let fingerprint = Self::compute_fingerprint(&path, &version_str);
		Self {
			id: id.into(),
			name: name.into(),
			version: version_str,
			path,
			fingerprint,
		}
	}

	fn compute_fingerprint(path: &PathBuf, version: &str) -> String {
		use blake3::Hasher;
		let mut hasher = Hasher::new();
		hasher.update(path.to_string_lossy().as_bytes());
		hasher.update(version.as_bytes());

		// Add file modification time for extra stability
		if let Ok(meta) = std::fs::metadata(path) {
			if let Ok(mtime) = meta.modified() {
				hasher.update(&mtime.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs().to_le_bytes());
			}
		}

		hasher.finalize().to_hex().to_string()[..16].to_string()
	}

	pub fn detect(name: &str) -> Option<Self> {
		let path = which(name)?;
		let version = Self::get_version(&path)?;

		Some(Self::new(format!("system-{}", name), name, version, path))
	}

	fn get_version(path: &PathBuf) -> Option<String> {
		let output = std::process::Command::new(path).arg("--version").output().ok()?;

		let version_str = String::from_utf8_lossy(&output.stdout);

		// Extract version number (e.g., "gcc (Ubuntu 11.4.0-1ubuntu1~22.04) 11.4.0" -> "11.4.0")
		let version = version_str.lines().next()?.rsplit(' ').next()?;

		Some(version.to_string())
	}
}

fn which(name: &str) -> Option<PathBuf> {
	std::env::var_os("PATH").and_then(|path| {
		std::env::split_paths(&path)
			.filter_map(|dir| {
				let exe = dir.join(name);
				if exe.exists() { Some(exe) } else { None }
			})
			.next()
	})
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolchainFingerprint {
	pub components: HashMap<String, String>,
}

impl ToolchainFingerprint {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn add_toolchain(&mut self, toolchain: &Toolchain) {
		self.components.insert(toolchain.name.clone(), toolchain.fingerprint.clone());
	}

	pub fn compute_hash(&self) -> String {
		use blake3::Hasher;
		let mut hasher = Hasher::new();

		let mut keys: Vec<_> = self.components.keys().collect();
		keys.sort();

		for key in keys {
			hasher.update(key.as_bytes());
			hasher.update(self.components[key].as_bytes());
		}

		hasher.finalize().to_hex().to_string()
	}
}
