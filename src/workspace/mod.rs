use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::package::{Dependency, Package, PackageManager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
	pub name: String,
	pub version: String,
	pub root: PathBuf,
	pub deps: Vec<Dependency>,
}

impl Workspace {
	pub fn load(root: &Path) -> std::io::Result<Self> {
		let workspace_path = root.join("forge-workspace");

		if !workspace_path.exists() {
			return Ok(Workspace {
				name: "unnamed".to_string(),
				version: "0.0.0".to_string(),
				root: root.to_path_buf(),
				deps: Vec::new(),
			});
		}

		let content = std::fs::read_to_string(&workspace_path)?;

		serde_lua::from_str(&content).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
	}

	pub fn resolve_dependencies(&self, manager: &PackageManager) -> Vec<Package> {
		let mut packages = Vec::new();

		for dep in &self.deps {
			match manager.fetch(dep) {
				Ok(pkg) => packages.push(pkg),
				Err(e) => {
					log::warn!("Failed to fetch dependency {:?}: {}", dep.name(), e);
				}
			}
		}

		packages
	}
}

mod serde_lua {
	use crate::package::Dependency;
	use std::path::PathBuf;

	#[derive(Debug)]
	pub struct LuaWorkspace {
		pub name: String,
		pub version: String,
		pub deps: Vec<DepEntry>,
	}

	#[derive(Debug)]
	pub struct DepEntry {
		pub name: String,
		pub url: Option<String>,
		pub version: Option<String>,
		pub path: Option<String>,
	}

	pub fn from_str(s: &str) -> Result<super::Workspace, String> {
		let mut name = "unnamed".to_string();
		let mut version = "0.0.0".to_string();
		let mut deps = Vec::new();

		let mut in_workspace = false;
		let mut in_deps = false;

		for line in s.lines() {
			let line = line.trim();

			if line.starts_with("workspace") && line.contains("{") {
				in_workspace = true;
				continue;
			}

			if line == "}" && in_workspace && !in_deps {
				break;
			}

			if !in_workspace {
				continue;
			}

			if line.starts_with("deps") && line.contains("{") {
				in_deps = true;
				continue;
			}

			if line == "}" && in_deps {
				in_deps = false;
				continue;
			}

			if in_deps {
				if line.starts_with("git(") {
					if let Some(dep) = parse_git_dep(line) {
						deps.push(dep);
					}
				} else if line.starts_with("http(") {
					if let Some(dep) = parse_http_dep(line) {
						deps.push(dep);
					}
				} else if line.starts_with("local(") {
					if let Some(dep) = parse_local_dep(line) {
						deps.push(dep);
					}
				} else if line.contains("=") && !line.starts_with("--") {
					let parts: Vec<&str> = line.splitn(2, '=').collect();
					if parts.len() == 2 {
						let key = parts[0].trim();
						let value = parts[1].trim().trim_matches('"');

						match key {
							"name" => name = value.to_string(),
							"version" => version = value.to_string(),
							_ => {}
						}
					}
				}
			}
		}

		Ok(super::Workspace {
			name,
			version,
			root: PathBuf::new(),
			deps: deps.into_iter().map(|d| d.into()).collect(),
		})
	}

	fn parse_git_dep(line: &str) -> Option<DepEntry> {
		let content = line.trim_start_matches("git(").trim_end_matches("),").trim_end_matches(")");

		let mut name = None;
		let mut url = None;
		let mut version = None;

		for part in content.split(',') {
			let part = part.trim();
			if let Some((key, value)) = part.split_once('=') {
				let value = value.trim_matches('"');
				match key.trim() {
					"name" => name = Some(value.to_string()),
					"url" => url = Some(value.to_string()),
					"version" | "rev" | "tag" => version = Some(value.to_string()),
					_ => {}
				}
			}
		}

		if let (Some(name), Some(url)) = (name, url) {
			Some(DepEntry {
				name,
				url: Some(url),
				version,
				path: None,
			})
		} else {
			None
		}
	}

	fn parse_http_dep(line: &str) -> Option<DepEntry> {
		let content = line.trim_start_matches("http(").trim_end_matches("),").trim_end_matches(")");

		let mut url = None;
		let mut sha256 = None;

		for part in content.split(',') {
			let part = part.trim();
			if let Some((key, value)) = part.split_once('=') {
				let value = value.trim_matches('"');
				match key.trim() {
					"url" => url = Some(value.to_string()),
					"sha256" => sha256 = Some(value.to_string()),
					_ => {}
				}
			}
		}

		if let Some(url) = url {
			Some(DepEntry {
				name: "http".to_string(),
				url: Some(url),
				version: sha256,
				path: None,
			})
		} else {
			None
		}
	}

	fn parse_local_dep(line: &str) -> Option<DepEntry> {
		let content = line.trim_start_matches("local(").trim_end_matches("),").trim_end_matches(")");

		let mut name = None;
		let mut path = None;

		for part in content.split(',') {
			let part = part.trim();
			if let Some((key, value)) = part.split_once('=') {
				let value = value.trim_matches('"');
				match key.trim() {
					"name" => name = Some(value.to_string()),
					"path" => path = Some(value.to_string()),
					_ => {}
				}
			}
		}

		if let (Some(name), Some(path)) = (name, path) {
			Some(DepEntry {
				name,
				url: None,
				version: None,
				path: Some(path),
			})
		} else {
			None
		}
	}

	impl From<DepEntry> for Dependency {
		fn from(entry: DepEntry) -> Self {
			if entry.url.is_some() && entry.path.is_none() {
				Dependency::Git {
					name: entry.name,
					url: entry.url.unwrap(),
					version: entry.version.unwrap_or_else(|| "main".to_string()),
				}
			} else if entry.path.is_some() {
				Dependency::Local {
					name: entry.name,
					path: entry.path.unwrap(),
				}
			} else if entry.name == "http" {
				Dependency::Http {
					url: entry.url.unwrap_or_default(),
					sha256: entry.version,
				}
			} else {
				Dependency::Registry {
					name: entry.name,
					version: entry.version,
				}
			}
		}
	}
}
