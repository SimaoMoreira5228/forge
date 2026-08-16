use std::collections::BTreeMap;

use forge_diagnostics::ForgeDiagnostic;

use crate::builder::{Engine, Prepared};

impl Engine {
	pub fn dependency_lock(&self) -> Result<forge_core::resolver::ForgeLock, ForgeDiagnostic> {
		let _lock = self.shared_lock()?;
		let prepared = self.prepare()?;
		self.dependency_lock_for(&prepared, false)
	}

	fn dependency_lock_for(
		&self,
		prepared: &Prepared,
		use_existing: bool,
	) -> Result<forge_core::resolver::ForgeLock, ForgeDiagnostic> {
		let path = self.workspace.join("forge.lock");
		if use_existing && path.is_file() {
			let text =
				std::fs::read_to_string(&path).map_err(|e| ForgeDiagnostic::error(8, format!("{}: {e}", path.display())))?;
			return forge_core::resolver::ForgeLock::parse(&text).map_err(|e| ForgeDiagnostic::error(101, e));
		}
		if prepared.requirements.is_empty() && prepared.candidates.is_empty() {
			return Ok(forge_core::resolver::ForgeLock::from_requests(prepared.dependencies.clone()));
		}
		let resolved = forge_core::solve(
			prepared.config.name.clone(),
			prepared.requirements.clone(),
			prepared.candidates.clone(),
		)
		.map_err(|error| ForgeDiagnostic::error(101, error.to_string()))?;
		let lock = forge_core::resolver::ForgeLock::from_resolved(&resolved);
		lock.sources().map_err(|error| ForgeDiagnostic::error(101, error))?;
		Ok(lock)
	}

	pub(crate) fn fetch_sources(
		&self,
		prepared: &Prepared,
	) -> Result<Vec<forge_script::cells::FetchedSource>, ForgeDiagnostic> {
		if prepared.dependencies.is_empty() && prepared.requirements.is_empty() && prepared.candidates.is_empty() {
			return Ok(Vec::new());
		}
		let lock = self.dependency_lock_for(prepared, true)?;
		if lock.packages.is_empty() {
			return Ok(Vec::new());
		}
		let store = crate::source_store::SourceStore::open(&self.workspace, prepared.config.source_mirrors.clone());
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
			let fetched = store.fetch(&crate::source_store::SourcePackage {
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
