use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::{fs::File, io::BufReader, path::Path, time::SystemTime};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildCache {
	pub rule_hashes: DashMap<String, String>,
	#[serde(default)]
	pub mtimes: DashMap<String, SystemTime>,
	#[serde(default)]
	pub file_hashes: DashMap<String, String>,
	#[serde(default)]
	pub artifact_metadata: DashMap<String, ArtifactMetadata>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ArtifactMetadata {
	pub size: u64,
	pub created: SystemTime,
	pub compressed: bool,
	pub dependencies: Vec<String>,
}

impl BuildCache {
	pub fn new() -> Self {
		Self {
			rule_hashes: DashMap::new(),
			mtimes: DashMap::new(),
			file_hashes: DashMap::new(),
			artifact_metadata: DashMap::new(),
		}
	}

	pub fn load(path: &Path) -> Self {
		if let Ok(file) = File::open(path)
			&& let Ok(cache) = serde_json::from_reader::<_, BuildCache>(BufReader::new(file))
		{
			return cache;
		}
		Self::new()
	}

	pub fn validate_and_clean(&self, project_path: &Path) {
		let mut stale_files = Vec::new();

		for entry in self.file_hashes.iter() {
			let file_path = project_path.join(entry.key());
			if let Ok(metadata) = std::fs::metadata(&file_path) {
				if let Ok(modified) = metadata.modified() {
					if let Some(cached_mtime) = self.mtimes.get(entry.key()) {
						if modified > *cached_mtime.value() {
							stale_files.push(entry.key().clone());
						}
					} else {
						stale_files.push(entry.key().clone());
					}
				}
			} else {
				stale_files.push(entry.key().clone());
			}
		}

		for file in stale_files {
			log::debug!("Removing stale cache entry for: {}", file);
			self.file_hashes.remove(&file);
			self.mtimes.remove(&file);
		}
	}

	pub fn save(&self, path: &Path) -> anyhow::Result<()> {
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)?;
		}
		let file = File::create(path)?;
		serde_json::to_writer_pretty(file, self)?;
		Ok(())
	}
}
