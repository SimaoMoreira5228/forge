use std::path::Path;
use std::time::Duration;

#[derive(Debug)]
pub struct CacheGC {
	pub max_size: u64,
	pub max_age: Duration,
	pub keep_recent: usize,
}

impl CacheGC {
	pub fn new(max_size: u64, max_age: Duration, keep_recent: usize) -> Self {
		Self {
			max_size,
			max_age,
			keep_recent,
		}
	}

	pub fn default_for_forge() -> Self {
		Self {
			max_size: 10 * 1024 * 1024 * 1024,               // 10GB
			max_age: Duration::from_secs(30 * 24 * 60 * 60), // 30 days
			keep_recent: 100,
		}
	}

	pub fn run(&self, cache_path: &Path) -> GCResult {
		let mut result = GCResult::default();

		if !cache_path.exists() {
			return result;
		}

		let now = std::time::SystemTime::now();

		// Collect all artifacts with their metadata
		let mut artifacts: Vec<ArtifactInfo> = Vec::new();

		if let Ok(entries) = std::fs::read_dir(cache_path) {
			for entry in entries.flatten() {
				let path = entry.path();
				if path.is_file() {
					if let Ok(metadata) = std::fs::metadata(&path) {
						let size = metadata.len();
						let modified = metadata.modified().ok();
						let accessed = metadata.accessed().ok();

						let age_days = modified
							.map(|m| now.duration_since(m).map(|d| d.as_secs() / (24 * 60 * 60)).unwrap_or(0))
							.unwrap_or(0);

						artifacts.push(ArtifactInfo {
							path: path.clone(),
							size,
							modified,
							accessed,
							age_days,
						});

						result.total_size += size;
						result.total_files += 1;
					}
				}
			}
		}

		// Sort by access time (oldest first)
		artifacts.sort_by(|a, b| a.accessed.cmp(&b.accessed));

		// Remove oldest until under size limit
		let target_size = self.max_size;
		let mut current_size = result.total_size;

		for artifact in &artifacts {
			if current_size <= target_size {
				break;
			}

			// Keep recent files
			if result.files_removed < self.keep_recent as u64 {
				result.files_removed += 1;
				continue;
			}

			// Check age limit
			if artifact.age_days < self.max_age.as_secs() / (24 * 60 * 60) {
				continue;
			}

			if std::fs::remove_file(&artifact.path).is_ok() {
				current_size -= artifact.size;
				result.freed_size += artifact.size;
				result.files_removed += 1;
			}
		}

		// Also clean up empty directories
		self.clean_empty_dirs(cache_path);

		result
	}

	fn clean_empty_dirs(&self, cache_path: &Path) {
		if let Ok(entries) = std::fs::read_dir(cache_path) {
			for entry in entries.flatten() {
				let path = entry.path();
				if path.is_dir() {
					if let Ok(mut entries) = std::fs::read_dir(&path) {
						if entries.next().is_none() {
							let _ = std::fs::remove_dir(&path);
						}
					}
				}
			}
		}
	}
}

#[derive(Debug, Default)]
pub struct GCResult {
	pub total_files: u64,
	pub total_size: u64,
	pub files_removed: u64,
	pub freed_size: u64,
}

#[derive(Debug)]
struct ArtifactInfo {
	path: std::path::PathBuf,
	size: u64,
	modified: Option<std::time::SystemTime>,
	accessed: Option<std::time::SystemTime>,
	age_days: u64,
}
