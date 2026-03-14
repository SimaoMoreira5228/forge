use globset::Glob;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub struct SourceResolver {
	project_root: PathBuf,
}

impl SourceResolver {
	pub fn new(project_root: PathBuf) -> Self {
		Self { project_root }
	}

	pub fn with_root(project_root: &Path) -> Self {
		Self {
			project_root: project_root.to_path_buf(),
		}
	}

	pub fn glob(&self, pattern: &str) -> Vec<PathBuf> {
		let full_pattern = if Path::new(pattern).is_absolute() {
			pattern.to_string()
		} else {
			self.project_root.join(pattern).to_string_lossy().to_string()
		};

		let glob = match Glob::new(&full_pattern) {
			Ok(g) => g,
			Err(e) => {
				log::warn!("Invalid glob pattern '{}': {}", pattern, e);
				return Vec::new();
			}
		};

		let matcher = glob.compile_matcher();
		let mut results = Vec::new();

		for entry in WalkDir::new(&self.project_root)
			.max_depth(10)
			.into_iter()
			.filter_map(|e| e.ok())
		{
			let path = entry.path();
			if path.is_file() {
				let path_str = path.to_string_lossy();
				if matcher.is_match(path) {
					results.push(path.to_path_buf());
				}
			}
		}

		results.sort();
		results.dedup();
		results
	}

	pub fn glob_recursive(&self, pattern: &str, max_depth: Option<usize>) -> Vec<PathBuf> {
		let base = if pattern.contains('*') || pattern.contains('?') {
			let parts: Vec<&str> = pattern.split(['/', '\\']).collect();
			let mut base_path = self.project_root.clone();
			for part in parts.iter().take_while(|p| !p.contains('*') && !p.contains('?')) {
				base_path = base_path.join(part);
			}
			base_path
		} else {
			self.project_root.join(pattern)
		};

		if !base.exists() {
			return Vec::new();
		}

		if base.is_file() {
			return vec![base];
		}

		let depth = max_depth.unwrap_or(10);
		let mut results = Vec::new();

		for entry in WalkDir::new(&base)
			.max_depth(depth)
			.follow_links(false)
			.into_iter()
			.filter_map(|e| e.ok())
		{
			if entry.file_type().is_file() {
				results.push(entry.path().to_path_buf());
			}
		}

		results.sort();
		results.dedup();
		results
	}

	pub fn resolve_sources(&self, patterns: &[String]) -> Vec<PathBuf> {
		let mut results = Vec::new();

		for pattern in patterns {
			if pattern.contains('*') || pattern.contains('?') {
				results.extend(self.glob(pattern));
			} else {
				let full_path = if Path::new(pattern).is_absolute() {
					PathBuf::from(pattern)
				} else {
					self.project_root.join(pattern)
				};

				if full_path.exists() {
					if full_path.is_file() {
						results.push(full_path);
					} else if full_path.is_dir() {
						for entry in WalkDir::new(&full_path).max_depth(10).into_iter().filter_map(|e| e.ok()) {
							if entry.file_type().is_file() {
								results.push(entry.path().to_path_buf());
							}
						}
					}
				}
			}
		}

		results.sort();
		results.dedup();
		results
	}

	pub fn resolve_includes(&self, dirs: &[String]) -> Vec<PathBuf> {
		let mut results = Vec::new();

		for dir in dirs {
			let full_path = if Path::new(dir).is_absolute() {
				PathBuf::from(dir)
			} else {
				self.project_root.join(dir)
			};

			if full_path.exists() && full_path.is_dir() {
				results.push(full_path);
			}
		}

		results.sort();
		results.dedup();
		results
	}

	pub fn to_absolute(&self, path: &str) -> PathBuf {
		let p = Path::new(path);
		if p.is_absolute() {
			p.to_path_buf()
		} else {
			self.project_root.join(path)
		}
	}

	pub fn relative(&self, path: &str) -> PathBuf {
		let p = Path::new(path);
		if p.is_absolute() {
			p.strip_prefix(&self.project_root).unwrap_or(p).to_path_buf()
		} else {
			p.to_path_buf()
		}
	}

	pub fn project_root(&self) -> &PathBuf {
		&self.project_root
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::env;

	#[test]
	fn test_source_resolver() {
		let root = env::current_dir().unwrap();
		let resolver = SourceResolver::new(root.clone());
		let files = resolver.glob("src/**/*.rs");
		println!("Found {} Rust files", files.len());
	}
}
