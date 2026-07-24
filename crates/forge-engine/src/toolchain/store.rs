use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use forge_core::Catalog;
use forge_diagnostics::{ForgeDiagnostic, codes};
use forge_script::{ToolchainSelection, WorkspaceConfig};

use crate::hasher;

pub const EMBEDDED_CATALOG: &str = include_str!("../../../../prelude/toolchains/catalog.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogOrigin {
	Embedded,
	Workspace,
}

#[derive(Debug, Clone)]
pub struct ResolvedToolchain {
	pub name: String,
	pub bin_dir: PathBuf,
	pub path_dirs: Vec<PathBuf>,
	pub digest: String,
}

impl ResolvedToolchain {
	pub fn binary(&self, name: &str) -> Option<PathBuf> {
		let suffix = std::env::consts::EXE_SUFFIX;
		for dir in &self.path_dirs {
			let direct = dir.join(name);
			if direct.is_file() {
				return Some(direct);
			}
			if !suffix.is_empty() {
				let shelled = dir.join(format!("{name}{suffix}"));
				if shelled.is_file() {
					return Some(shelled);
				}
			}
		}
		None
	}
}

pub fn resolve_tool_path(toolchains: &BTreeMap<String, ResolvedToolchain>, spec: &str) -> Result<PathBuf, ForgeDiagnostic> {
	let as_path = PathBuf::from(spec);
	if as_path.is_absolute() {
		return if as_path.is_file() {
			Ok(as_path)
		} else {
			Err(ForgeDiagnostic::error(
				codes::hermetic::TOOLCHAIN_MISMATCH,
				format!("tool `{spec}` does not exist"),
			))
		};
	}
	for tool in toolchains.values() {
		if let Some(found) = tool.binary(spec) {
			return Ok(found);
		}
	}
	let searched: Vec<String> = toolchains.values().map(|t| t.bin_dir.display().to_string()).collect();
	Err(ForgeDiagnostic::error(
		codes::hermetic::TOOLCHAIN_MISMATCH,
		format!("tool `{spec}` was not found in any configured toolchain"),
	)
	.with_help(format!(
		"searched: {}; add a [toolchains.<name>] section providing it",
		searched.join(", ")
	)))
}

pub struct ToolchainStore {
	pub store: crate::store::Store,
	pub catalog: Catalog,
	pub config: WorkspaceConfig,
	pub workspace_touched: BTreeSet<String>,
}

impl ToolchainStore {
	pub fn load(workspace: &Path, config: WorkspaceConfig) -> Result<Self, ForgeDiagnostic> {
		let mut catalog = Catalog::parse(EMBEDDED_CATALOG)?;
		let mut workspace_touched = BTreeSet::new();
		for file in &config.catalog_files {
			let absolute = workspace.join(file);
			let text = std::fs::read_to_string(&absolute).map_err(|e| {
				ForgeDiagnostic::error(
					codes::patch::PATCH_CONFLICT,
					format!("catalog file {}: {e}", absolute.display()),
				)
			})?;
			let overrides = Catalog::parse(&text)?;
			workspace_touched.extend(overrides.entry_names().cloned());
			catalog.merge_over(&overrides);
		}
		for name in config.toolchains.keys() {
			if matches!(config.toolchains[name], ToolchainSelection::Url { .. }) {
				workspace_touched.insert(name.clone());
			}
		}
		Ok(Self {
			store: crate::store::Store::open(),
			catalog,
			config,
			workspace_touched,
		})
	}

	pub fn origin_of(&self, name: &str) -> CatalogOrigin {
		if self.workspace_touched.contains(name) {
			CatalogOrigin::Workspace
		} else {
			CatalogOrigin::Embedded
		}
	}

	pub fn install_dir(&self, name: &str, version: &str) -> PathBuf {
		self.store.toolchains().join(name).join(version)
	}

	pub fn url_version(url: &str) -> String {
		blake3::hash(url.as_bytes()).to_hex()[..12].to_string()
	}

	pub fn configured(&self) -> impl Iterator<Item = (&String, &ToolchainSelection)> {
		self.config.toolchains.iter()
	}

	pub fn resolve_all(&self) -> Result<BTreeMap<String, ResolvedToolchain>, ForgeDiagnostic> {
		self.config
			.toolchains
			.iter()
			.map(|(name, selection)| Ok((name.clone(), self.resolve(name, selection)?)))
			.collect()
	}

	pub fn resolve(&self, name: &str, selection: &ToolchainSelection) -> Result<ResolvedToolchain, ForgeDiagnostic> {
		match selection {
			ToolchainSelection::Path { path } => {
				let bin_dir = pick_bin_dir(path);
				if !bin_dir.exists() {
					return Err(toolchain_error(name, format!("path `{}` does not exist", bin_dir.display())));
				}
				Ok(Self::finish(name, path, bin_dir))
			}
			ToolchainSelection::Version { version } => {
				let resolved = self.catalog.resolve(name, Some(version))?;
				let dir = self.install_dir(&resolved.name, &resolved.version);
				self.synced_root(name, version, &dir)
			}
			ToolchainSelection::Url { url, .. } => {
				let dir = self.install_dir(name, &Self::url_version(url));
				self.synced_root(name, &format!("url:{}", Self::url_version(url)), &dir)
			}
		}
	}

	fn synced_root(&self, name: &str, version_display: &str, dir: &Path) -> Result<ResolvedToolchain, ForgeDiagnostic> {
		if !dir.join(INSTALL_MARKER).exists() {
			return Err(toolchain_error(name, format!("version {version_display} is not synced"))
				.with_help("run `forge toolchain sync` to download it"));
		}
		if find_bin_dir(dir).is_none() && !any_file_under(dir) {
			return Err(toolchain_error(name, format!("install at {} is empty", dir.display())));
		}
		let bin_dir = installed_bin_dir(dir).unwrap_or_else(|| dir.to_path_buf());
		Ok(Self::finish(name, dir, bin_dir))
	}

	fn finish(name: &str, root: &Path, bin_dir: PathBuf) -> ResolvedToolchain {
		let mut path_dirs = vec![bin_dir.clone()];
		path_dirs.extend(nested_bin_dirs(root));
		path_dirs.sort();
		path_dirs.dedup();
		let digest = directory_digest(root, &path_dirs);
		ResolvedToolchain {
			name: name.to_string(),
			bin_dir,
			path_dirs,
			digest,
		}
	}
}

pub fn nested_bin_dirs(root: &Path) -> Vec<PathBuf> {
	let mut dirs: Vec<PathBuf> = walkdir::WalkDir::new(root)
		.into_iter()
		.flatten()
		.filter(|e| e.file_type().is_dir() && e.file_name() != "bin" && e.path().join("bin").is_dir())
		.map(|e| e.path().join("bin"))
		.collect();
	dirs.sort_by_key(|p| p.components().count());
	dirs
}

fn pick_bin_dir(path: &Path) -> PathBuf {
	if let Some(bin) = find_bin_dir(path) {
		return bin;
	}
	path.to_path_buf()
}

pub fn installed_bin_dir(install_root: &Path) -> Option<PathBuf> {
	let marker = std::fs::read_to_string(install_root.join(INSTALL_MARKER)).ok();
	if let Some(relative) = marker.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(PathBuf::from) {
		let candidate = install_root.join(relative);
		if candidate.is_dir() {
			return Some(candidate);
		}
	}
	find_bin_dir(install_root)
}

fn find_bin_dir(root: &Path) -> Option<PathBuf> {
	let mut candidates: Vec<PathBuf> = walkdir::WalkDir::new(root)
		.into_iter()
		.flatten()
		.filter(|e| e.file_type().is_dir() && e.path().join("bin").is_dir())
		.map(|e| e.path().join("bin"))
		.collect();
	candidates.sort_by_key(|p| p.components().count());
	candidates
		.into_iter()
		.find(|bin| std::fs::read_dir(bin).is_ok_and(|entries| entries.flatten().count() > 0))
}

pub const INSTALL_MARKER: &str = ".forge-install.txt";

fn any_file_under(root: &Path) -> bool {
	walkdir::WalkDir::new(root)
		.into_iter()
		.flatten()
		.any(|e| e.file_type().is_file())
}

fn toolchain_error(name: &str, detail: String) -> ForgeDiagnostic {
	ForgeDiagnostic::error(codes::hermetic::TOOLCHAIN_MISMATCH, format!("toolchain `{name}`: {detail}"))
}

fn directory_digest(root: &Path, bin_dirs: &[PathBuf]) -> String {
	let mut hasher = blake3::Hasher::new();
	let mut files: Vec<PathBuf> = bin_dirs
		.iter()
		.flat_map(|dir| walkdir::WalkDir::new(dir).into_iter().flatten())
		.filter(|e| e.file_type().is_file())
		.map(|e| e.path().to_path_buf())
		.collect();
	files.sort();
	files.dedup();
	for file in files {
		let rel = file.strip_prefix(root).unwrap_or(&file);
		hasher.update(rel.to_string_lossy().as_bytes());
		hasher.update(&[0]);
		if let Ok(bytes) = hasher_digest_file(&file) {
			hasher.update(bytes.as_slice());
		}
		hasher.update(&[1]);
	}
	hasher.finalize().to_hex()[..24].to_string()
}

fn hasher_digest_file(path: &Path) -> std::io::Result<Vec<u8>> {
	Ok(hasher::hash_file(path)?.to_vec())
}
