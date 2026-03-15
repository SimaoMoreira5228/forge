use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, crate::package::Error>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
	pub name: String,
	pub version: String,
	pub source: PackageSource,
	pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PackageSource {
	Local {
		path: PathBuf,
	},
	Git {
		url: String,
		rev: String,
	},
	Http {
		url: String,
		sha256: Option<String>,
	},
	Registry {
		name: String,
		version: String,
	},
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Dependency {
	Git {
		name: String,
		url: String,
		version: String,
	},
	Http {
		url: String,
		sha256: Option<String>,
	},
	Local {
		name: String,
		path: String,
	},
	Registry {
		name: String,
		version: Option<String>,
	},
}

impl Dependency {
	pub fn name(&self) -> &str {
		match self {
			Dependency::Git { name, .. } => name,
			Dependency::Local { name, .. } => name,
			Dependency::Registry { name, .. } => name,
			Dependency::Http { .. } => "http",
		}
	}
}

pub struct PackageManager {
	packages_dir: PathBuf,
	cache_dir: PathBuf,
}

impl PackageManager {
	pub fn new() -> Self {
		let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
		let forge_dir = home.join(".forge");

		Self {
			packages_dir: forge_dir.join("packages"),
			cache_dir: forge_dir.join("cache"),
		}
	}

	pub fn with_custom_dirs(packages_dir: PathBuf, cache_dir: PathBuf) -> Self {
		Self { packages_dir, cache_dir }
	}

	pub fn packages_dir(&self) -> &Path {
		&self.packages_dir
	}

	pub fn cache_dir(&self) -> &Path {
		&self.cache_dir
	}

	pub fn fetch(&self, dep: &Dependency) -> Result<Package> {
		match dep {
			Dependency::Git { name, url, version } => self.fetch_git(name, url, version),
			Dependency::Http { url, sha256 } => self.fetch_http(url, sha256.as_deref()),
			Dependency::Local { name, path } => self.fetch_local(name, path),
			Dependency::Registry { name, version } => self.fetch_registry(name, version.as_deref()),
		}
	}

	fn fetch_git(&self, name: &str, url: &str, version: &str) -> Result<Package> {
		let package_path = self.packages_dir.join(name).join(version);

		if package_path.exists() {
			return Ok(Package {
				name: name.to_string(),
				version: version.to_string(),
				source: PackageSource::Git {
					url: url.to_string(),
					rev: version.to_string(),
				},
				path: package_path,
			});
		}

		Err(crate::package::Error::FetchFailed(format!(
			"Git dependency not found locally: {}@{} (run 'forge deps sync' to fetch)",
			name, version
		)))
	}

	fn fetch_http(&self, url: &str, _sha256: Option<&str>) -> Result<Package> {
		let filename = url.split('/').last().unwrap_or("download");

		let cache_path = self.cache_dir.join("downloads").join(filename);

		if !cache_path.exists() {
			return Err(crate::package::Error::FetchFailed(format!(
				"HTTP dependency not found in cache: {} (run 'forge deps sync' to download)",
				url
			)));
		}

		let package_name = filename.trim_end_matches(".tar.gz").trim_end_matches(".zip");

		Ok(Package {
			name: package_name.to_string(),
			version: "1.0.0".to_string(),
			source: PackageSource::Http {
				url: url.to_string(),
				sha256: None,
			},
			path: cache_path,
		})
	}

	fn fetch_local(&self, name: &str, path: &str) -> Result<Package> {
		let package_path = PathBuf::from(path);

		if !package_path.exists() {
			return Err(crate::package::Error::NotFound(format!(
				"Local dependency not found: {}",
				path
			)));
		}

		Ok(Package {
			name: name.to_string(),
			version: "local".to_string(),
			source: PackageSource::Local {
				path: package_path.clone(),
			},
			path: package_path,
		})
	}

	fn fetch_registry(&self, name: &str, version: Option<&str>) -> Result<Package> {
		let version = version.unwrap_or("latest");
		let package_path = self.packages_dir.join(name).join(version);

		if !package_path.exists() {
			return Err(crate::package::Error::NotFound(format!(
				"Registry package not found: {}@{} (run 'forge deps sync' to fetch)",
				name, version
			)));
		}

		Ok(Package {
			name: name.to_string(),
			version: version.to_string(),
			source: PackageSource::Registry {
				name: name.to_string(),
				version: version.to_string(),
			},
			path: package_path,
		})
	}

	pub fn list_local(&self) -> Vec<Package> {
		let mut packages = Vec::new();

		if !self.packages_dir.exists() {
			return packages;
		}

		if let Ok(entries) = std::fs::read_dir(&self.packages_dir) {
			for entry in entries.flatten() {
				let name = entry.file_name().to_string_lossy().to_string();
				if let Ok(versions) = std::fs::read_dir(entry.path()) {
					for version_entry in versions.flatten() {
						let version = version_entry.file_name().to_string_lossy().to_string();
						packages.push(Package {
							name: name.clone(),
							version,
							source: PackageSource::Local {
								path: version_entry.path(),
							},
							path: version_entry.path(),
						});
					}
				}
			}
		}

		packages
	}
}

impl Default for PackageManager {
	fn default() -> Self {
		Self::new()
	}
}
