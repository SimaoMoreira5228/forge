use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use forge_core::Catalog;
use forge_diagnostics::{ForgeDiagnostic, codes};
use forge_script::{ToolchainSelection, WorkspaceConfig};

use std::collections::BTreeSet;

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
	pub digest: String,
}

impl ResolvedToolchain {
	pub fn binary(&self, name: &str) -> Option<PathBuf> {
		let suffix = std::env::consts::EXE_SUFFIX;
		let mut candidates = vec![self.bin_dir.join(name)];
		if !suffix.is_empty() {
			candidates.push(self.bin_dir.join(format!("{name}{suffix}")));
		}
		candidates.into_iter().find(|c| c.is_file())
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
	pub root: PathBuf,
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
			root: workspace.join(".forge").join("toolchains"),
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
		self.root.join(name).join(version)
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
				Ok(Self::finish(name, bin_dir))
			}
			ToolchainSelection::Version { version } => {
				let resolved = self.catalog.resolve(name, Some(version))?;
				let dir = self.install_dir(&resolved.name, &resolved.version);
				let bin_dir = installed_bin_dir(&dir).ok_or_else(|| {
					toolchain_error(name, format!("version {version} is not synced"))
						.with_help("run `forge toolchain sync` to download it")
				})?;
				Ok(Self::finish(name, bin_dir))
			}
			ToolchainSelection::Url { url, .. } => {
				let dir = self.install_dir(name, &Self::url_version(url));
				let bin_dir = installed_bin_dir(&dir).ok_or_else(|| {
					toolchain_error(name, "the pinned URL artifact is not synced".to_string())
						.with_help("run `forge toolchain sync` to download it")
				})?;
				Ok(Self::finish(name, bin_dir))
			}
		}
	}

	fn finish(name: &str, bin_dir: PathBuf) -> ResolvedToolchain {
		let digest = directory_digest(&bin_dir);
		ResolvedToolchain {
			name: name.to_string(),
			bin_dir,
			digest,
		}
	}
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

fn toolchain_error(name: &str, detail: String) -> ForgeDiagnostic {
	ForgeDiagnostic::error(codes::hermetic::TOOLCHAIN_MISMATCH, format!("toolchain `{name}`: {detail}"))
}

fn directory_digest(bin_dir: &Path) -> String {
	let mut hasher = blake3::Hasher::new();
	let mut files: Vec<PathBuf> = walkdir::WalkDir::new(bin_dir)
		.into_iter()
		.flatten()
		.filter(|e| e.file_type().is_file())
		.map(|e| e.path().to_path_buf())
		.collect();
	files.sort();
	for file in files {
		let rel = file.strip_prefix(bin_dir).unwrap_or(&file);
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
