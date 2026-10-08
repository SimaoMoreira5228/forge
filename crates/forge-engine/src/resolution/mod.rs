pub(crate) mod hooks;
pub mod sources;
pub(crate) mod transport;

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use forge_diagnostics::ForgeDiagnostic;

use crate::build::{Engine, Prepared};

fn validate_lock(prepared: &Prepared, lock: &forge_core::resolver::ForgeLock) -> Result<(), ForgeDiagnostic> {
	if lock.version != 1 {
		return Err(ForgeDiagnostic::error(
			101,
			format!("unsupported forge.lock version {}; expected 1", lock.version),
		));
	}
	lock.dependency_order().map_err(|e| ForgeDiagnostic::error(101, e))?;
	for request in &prepared.dependencies {
		if !lock.iter().any(|package| {
			package.name == request.name
				&& package.version == request.version
				&& package.source.as_deref() == Some(request.source.as_str())
				&& package.checksum.as_deref() == Some(request.checksum.as_str())
				&& package.dependencies.iter().collect::<BTreeSet<_>>()
					== request.dependencies.iter().collect::<BTreeSet<_>>()
		}) {
			return Err(ForgeDiagnostic::error(
				101,
				format!(
					"forge.lock does not match declared dependency `{}@{}`; resolve dependencies explicitly",
					request.name, request.version
				),
			));
		}
	}
	for requirement in &prepared.requirements {
		if !lock.iter().any(|package| {
			package.name == requirement.name
				&& forge_core::resolver::Version::parse(&package.version)
					.is_ok_and(|version| requirement.range.contains(&version))
		}) {
			return Err(ForgeDiagnostic::error(
				101,
				format!(
					"forge.lock does not satisfy requirement `{}` ({}); resolve dependencies explicitly",
					requirement.name, requirement.range
				),
			));
		}
	}
	Ok(())
}

impl Engine {
	pub fn dependency_lock(&self, offline: bool) -> Result<forge_core::resolver::ForgeLock, ForgeDiagnostic> {
		let _lock = self.shared_lock()?;
		let prepared = self.prepare_for_resolution(offline)?;
		self.dependency_lock_for(&prepared, false)
	}

	pub fn write_dependency_lock(&self, offline: bool) -> Result<std::path::PathBuf, ForgeDiagnostic> {
		let _lock = self.exclusive_lock()?;
		let prepared = self.prepare_for_resolution(offline)?;
		let lock = self.dependency_lock_for(&prepared, false)?;
		let text = lock.to_toml().map_err(|e| ForgeDiagnostic::error(101, e))?;
		let path = self.workspace.join("forge.lock");
		let temporary = self.workspace.join(format!(".forge.lock.tmp-{}", std::process::id()));
		let mut file = std::fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(&temporary)
			.map_err(|e| ForgeDiagnostic::error(8, format!("{}: {e}", temporary.display())))?;
		let result = (|| {
			file.write_all(text.as_bytes())?;
			file.sync_all()?;
			drop(file);
			std::fs::rename(&temporary, &path)
		})();
		if result.is_err() {
			let _ = std::fs::remove_file(&temporary);
		}
		result.map_err(|e| ForgeDiagnostic::error(8, format!("{}: {e}", path.display())))?;
		Ok(path)
	}

	pub fn sync_dependencies(&self, offline: bool) -> Result<Vec<forge_script::cells::FetchedSource>, ForgeDiagnostic> {
		let _lock = self.exclusive_lock()?;
		let prepared = self.prepare()?;
		self.fetch_sources(&prepared, offline)
	}

	fn dependency_lock_for(
		&self,
		prepared: &Prepared,
		use_existing: bool,
	) -> Result<forge_core::resolver::ForgeLock, ForgeDiagnostic> {
		if let Some(authority) = &prepared.imported_lock {
			if !use_existing || !prepared.requirements.is_empty() || !prepared.candidates.is_empty() {
				return Err(ForgeDiagnostic::error(
					101,
					format!(
						"dependencies are externally managed by `{authority}`; native dependency resolution is unavailable"
					),
				));
			}
			let lock = forge_core::resolver::ForgeLock::from_requests(prepared.dependencies.clone());
			validate_lock(prepared, &lock)?;
			return Ok(lock);
		}
		if !prepared.dependencies.is_empty() && (!prepared.requirements.is_empty() || !prepared.candidates.is_empty()) {
			return Err(ForgeDiagnostic::error(
				101,
				"exact dependency requests cannot be mixed with solver requirements or candidates",
			));
		}
		let path = self.workspace.join("forge.lock");
		if use_existing {
			match std::fs::read_to_string(&path) {
				Ok(text) => {
					let lock = forge_core::resolver::ForgeLock::parse(&text).map_err(|e| ForgeDiagnostic::error(101, e))?;
					validate_lock(prepared, &lock)?;
					return Ok(lock);
				}
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
				Err(e) => return Err(ForgeDiagnostic::error(8, format!("{}: {e}", path.display()))),
			}
		}
		if prepared.requirements.is_empty() && prepared.candidates.is_empty() {
			let lock = forge_core::resolver::ForgeLock::from_requests(prepared.dependencies.clone());
			validate_lock(prepared, &lock)?;
			return Ok(lock);
		}
		if use_existing {
			return Err(ForgeDiagnostic::error(101, "native dependency requirements need forge.lock")
				.with_help("resolve dependencies explicitly before building"));
		}
		let resolved = forge_core::solve(
			prepared.config.name.clone(),
			prepared.requirements.clone(),
			prepared.candidates.clone(),
		)
		.map_err(|error| ForgeDiagnostic::error(101, error.to_string()))?;
		let lock = forge_core::resolver::ForgeLock::from_resolved(&resolved);
		validate_lock(prepared, &lock)?;
		lock.sources().map_err(|error| ForgeDiagnostic::error(101, error))?;
		Ok(lock)
	}

	pub(crate) fn fetch_sources(
		&self,
		prepared: &Prepared,
		offline: bool,
	) -> Result<Vec<forge_script::cells::FetchedSource>, ForgeDiagnostic> {
		let lock = self.dependency_lock_for(prepared, true)?;
		if lock.packages.is_empty() {
			return Ok(Vec::new());
		}
		let store = crate::store::Store::open();
		let _lease = store.lock_shared("lease")?;
		let store = crate::resolution::sources::SourceStore::with_store(
			&self.workspace,
			store,
			prepared.config.source_mirrors.clone(),
			offline,
		);
		let patches = &prepared.config.local_patches;
		let mut roots = BTreeMap::new();
		for package in lock.iter() {
			if patches.contains_key(&package.name) {
				continue;
			}
			let source = package.source.clone().ok_or_else(|| {
				ForgeDiagnostic::error(101, format!("{}@{} has no source URL", package.name, package.version))
			})?;
			let (url, sha256, git_rev) = match source.strip_prefix("git+") {
				Some(rest) => {
					let (url, rev) = rest
						.split_once('#')
						.ok_or_else(|| ForgeDiagnostic::error(101, format!("git source `{source}` has no revision")))?;
					let url = url.split('?').next().unwrap_or(url).to_string();
					match prepared.config.git_patches.get(&url) {
						Some(patch) => (
							patch.git.clone().unwrap_or(url),
							String::new(),
							Some(patch.rev.clone().unwrap_or_else(|| rev.to_string())),
						),
						None => (url, String::new(), Some(rev.to_string())),
					}
				}
				None => (
					source,
					package.checksum.clone().ok_or_else(|| {
						ForgeDiagnostic::error(101, format!("{}@{} has no sha256 checksum", package.name, package.version))
					})?,
					None,
				),
			};
			let fetched = store.fetch(&crate::resolution::sources::SourcePackage {
				name: package.name.clone(),
				version: package.version.clone(),
				url,
				sha256,
				git_rev,
			})?;
			let relative = fetched.strip_prefix(&self.workspace).map_err(|_| {
				ForgeDiagnostic::error(101, format!("dependency source escaped workspace: {}", fetched.display()))
			})?;
			roots.insert(
				format!("{}@{}", package.name, package.version),
				relative.to_string_lossy().into_owned(),
			);
		}
		let mut packages = Vec::new();
		for package in lock.dependency_order().map_err(|e| ForgeDiagnostic::error(101, e))? {
			let root = match patches.get(&package.name) {
				Some(path) => path.to_string_lossy().into_owned(),
				None => {
					let key = format!("{}@{}", package.name, package.version);
					roots
						.get(&key)
						.ok_or_else(|| ForgeDiagnostic::error(101, format!("missing fetched dependency `{key}`")))?
						.clone()
				}
			};
			packages.push(forge_script::cells::FetchedSource {
				name: package.name.clone(),
				version: package.version.clone(),
				root,
				dependencies: package.dependencies.clone(),
			});
		}
		Ok(packages)
	}
}

#[cfg(test)]
mod tests {
	use forge_core::resolver::{DependencyRequest, DependencyRequirement, ForgeLock, Version, VersionRange};

	use super::*;

	fn fixture(name: &str) -> (Engine, Prepared) {
		let dir = std::env::temp_dir().join(format!("forge-authority-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		let prepared = Prepared {
			config: forge_script::WorkspaceConfig::parse("").unwrap(),
			graph: forge_core::BuildGraph::new(),
			decls: BTreeMap::new(),
			dependencies: Vec::new(),
			requirements: Vec::new(),
			candidates: Vec::new(),
			imported_lock: None,
		};
		(Engine::open(&dir), prepared)
	}

	#[test]
	fn lock_validation_rejects_unknown_versions_and_stale_exact_edges() {
		let (engine, mut prepared) = fixture("validation");
		prepared.dependencies = vec![
			DependencyRequest::new("top", "1.0.0", "top", "sha").with_dependencies(["leaf 1.0.0", "other 1.0.0"]),
			DependencyRequest::new("leaf", "1.0.0", "leaf", "sha"),
			DependencyRequest::new("other", "1.0.0", "other", "sha"),
		];
		let mut lock = ForgeLock::from_requests(prepared.dependencies.clone());
		lock.packages[0].dependencies.reverse();
		assert!(validate_lock(&prepared, &lock).is_ok());
		lock.version = 2;
		assert!(
			validate_lock(&prepared, &lock)
				.unwrap_err()
				.to_string()
				.contains("unsupported forge.lock version")
		);
		lock.version = 1;
		lock.packages[0].dependencies.pop();
		std::fs::write(engine.workspace.join("forge.lock"), lock.to_toml().unwrap()).unwrap();
		assert!(
			engine
				.fetch_sources(&prepared, false)
				.unwrap_err()
				.to_string()
				.contains("does not match")
		);
		prepared.dependencies[0].dependencies.push("missing".into());
		assert!(engine.dependency_lock_for(&prepared, false).is_err());
		std::fs::remove_dir_all(engine.workspace).unwrap();
	}

	#[test]
	fn exact_requests_cannot_be_silently_dropped_by_the_solver() {
		let (engine, mut prepared) = fixture("mixture");
		prepared
			.dependencies
			.push(DependencyRequest::new("demo", "1.0.0", "demo", "sha"));
		prepared
			.requirements
			.push(DependencyRequirement::new("demo", VersionRange::full()));
		std::fs::write(
			engine.workspace.join("forge.lock"),
			ForgeLock::from_requests(prepared.dependencies.clone()).to_toml().unwrap(),
		)
		.unwrap();
		for use_existing in [false, true] {
			assert!(
				engine
					.dependency_lock_for(&prepared, use_existing)
					.unwrap_err()
					.to_string()
					.contains("cannot be mixed")
			);
		}
		prepared.requirements.clear();
		prepared
			.candidates
			.push(forge_core::PackageCandidate::new("demo", Version::new(1, 0, 0)));
		assert!(
			engine
				.dependency_lock_for(&prepared, false)
				.unwrap_err()
				.to_string()
				.contains("cannot be mixed")
		);
		std::fs::remove_dir_all(engine.workspace).unwrap();
	}

	#[test]
	fn public_lock_write_replaces_atomically_and_sync_uses_local_patches() {
		let (engine, _) = fixture("operations");
		std::fs::write(
			engine.workspace.join("FORGE_ROOT"),
			"[discovery]\ninclude = [\".\"]\n[patch.local.demo]\npath = \"vendor/demo\"\n",
		)
		.unwrap();
		let script = engine.workspace.join("FORGE.rhai");
		std::fs::write(&script, "dependency_require(\"demo\", \"1.0.0\", \"2.0.0\");\ndependency_candidate(\"demo\", \"1.0.0\", \"https://example.invalid/demo\", \"sha\", []);").unwrap();
		assert!(
			engine
				.sync_dependencies(false)
				.unwrap_err()
				.to_string()
				.contains("need forge.lock")
		);
		assert_eq!(engine.dependency_lock(false).unwrap().packages.len(), 1);
		let path = engine.workspace.join("forge.lock");
		assert!(!path.exists());
		std::fs::write(&path, "previous lock").unwrap();
		let snapshot = engine.workspace.join("previous.lock");
		std::fs::hard_link(&path, &snapshot).unwrap();
		assert_eq!(engine.write_dependency_lock(false).unwrap(), path);
		assert_eq!(std::fs::read_to_string(&snapshot).unwrap(), "previous lock");
		let locked = std::fs::read_to_string(&path).unwrap();
		assert_eq!(ForgeLock::parse(&locked).unwrap().packages[0].name, "demo");
		let sources = engine.sync_dependencies(false).unwrap();
		assert_eq!(sources.len(), 1);
		assert_eq!(sources[0].root, "vendor/demo");
		assert!(!engine.workspace.join("forge-out/deps/demo").exists());
		std::fs::write(&script, "dependency_require(\"missing\", \"1.0.0\", \"2.0.0\");").unwrap();
		assert!(engine.write_dependency_lock(false).is_err());
		assert_eq!(std::fs::read_to_string(&path).unwrap(), locked);
		std::fs::remove_dir_all(engine.workspace).unwrap();
	}

	#[test]
	fn only_explicit_lock_operations_enable_resolution() {
		let (engine, _) = fixture("resolution-boundary");
		std::fs::write(engine.workspace.join("FORGE_ROOT"), "[discovery]\ninclude = [\".\"]\n").unwrap();
		let script = engine.workspace.join("FORGE.rhai");
		std::fs::write(&script, r#"if !resolving_dependencies { fetch("disabled"); }"#).unwrap();
		assert!(engine.dependency_lock(false).unwrap().packages.is_empty());
		assert!(engine.write_dependency_lock(false).is_ok());
		for result in [engine.prepare().map(|_| ()), engine.sync_dependencies(false).map(|_| ())] {
			assert!(
				result
					.unwrap_err()
					.to_string()
					.contains(forge_script::rhai_rt::UNRESOLVED_FETCH)
			);
		}
		std::fs::write(&script, r#"if resolving_dependencies { fetch("file:///metadata"); }"#).unwrap();
		assert!(engine.prepare().is_ok());
		assert!(engine.sync_dependencies(false).is_ok());
		assert!(engine.dependency_lock(false).is_err());
		assert!(engine.write_dependency_lock(false).is_err());
		std::fs::remove_dir_all(engine.workspace).unwrap();
	}

	#[test]
	fn imported_pins_ignore_native_lock_even_when_empty() {
		let (engine, mut prepared) = fixture("imported");
		std::fs::write(engine.workspace.join("forge.lock"), "invalid lock").unwrap();
		prepared.imported_lock = Some("external.lock".into());
		assert!(engine.dependency_lock_for(&prepared, true).unwrap().packages.is_empty());
		assert!(engine.fetch_sources(&prepared, false).unwrap().is_empty());
		assert!(
			engine
				.dependency_lock_for(&prepared, false)
				.unwrap_err()
				.to_string()
				.contains("externally managed")
		);
		prepared
			.dependencies
			.push(DependencyRequest::new("demo", "1.0.0", "https://example.invalid/demo", "sha"));
		prepared.config.local_patches.insert("demo".into(), "vendor/demo".into());
		let fetched = engine.fetch_sources(&prepared, false).unwrap();
		assert_eq!(fetched.len(), 1);
		assert_eq!(fetched[0].name, "demo");
		assert_eq!(fetched[0].root, "vendor/demo");
		prepared.dependencies.clear();
		prepared
			.requirements
			.push(DependencyRequirement::new("demo", VersionRange::full()));
		assert!(engine.fetch_sources(&prepared, false).is_err());
		std::fs::remove_dir_all(engine.workspace).unwrap();
	}

	#[test]
	fn native_builds_require_a_matching_lock_for_requirements() {
		let (engine, mut prepared) = fixture("native");
		let request = DependencyRequest::new("demo", "1.0.0", "git+https://example.invalid/demo#abc", "");
		prepared.dependencies.push(request.clone());
		assert_eq!(engine.dependency_lock_for(&prepared, true).unwrap().packages[0].name, "demo");
		prepared.dependencies.clear();
		prepared.requirements.push(DependencyRequirement::new(
			"demo",
			VersionRange::singleton(Version::new(1, 0, 0)),
		));
		assert!(
			engine
				.dependency_lock_for(&prepared, true)
				.unwrap_err()
				.to_string()
				.contains("need forge.lock")
		);
		let lock = ForgeLock::from_requests([request]);
		let path = engine.workspace.join("forge.lock");
		std::fs::write(&path, lock.to_toml().unwrap()).unwrap();
		assert!(engine.dependency_lock_for(&prepared, true).is_ok());
		prepared.requirements[0].range = VersionRange::singleton(Version::new(2, 0, 0));
		assert!(
			engine
				.dependency_lock_for(&prepared, true)
				.unwrap_err()
				.to_string()
				.contains("does not satisfy")
		);
		prepared.requirements[0].name = "missing".into();
		assert!(engine.dependency_lock_for(&prepared, true).is_err());
		prepared.requirements.clear();
		prepared.dependencies.push(DependencyRequest::new(
			"missing",
			"1.0.0",
			"https://example.invalid/missing",
			"sha",
		));
		assert!(
			engine
				.dependency_lock_for(&prepared, true)
				.unwrap_err()
				.to_string()
				.contains("does not match")
		);
		std::fs::remove_dir_all(engine.workspace).unwrap();
	}
}
