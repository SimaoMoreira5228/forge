use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use forge_core::Catalog;
use forge_diagnostics::{ForgeDiagnostic, codes};
use forge_script::{ToolchainSelection, WorkspaceConfig};

use crate::hasher;

const EMBEDDED_CATALOG: &str = include_str!("../../../prelude/toolchains/catalog.toml");

#[derive(Debug, Clone)]
pub struct ResolvedToolchain {
	pub name: String,
	pub bin_dir: PathBuf,
	pub digest: String,
}

impl ResolvedToolchain {
	pub fn binary(&self, name: &str) -> Option<PathBuf> {
		let candidate = self.bin_dir.join(name);
		candidate.is_file().then_some(candidate)
	}
}

pub struct ToolchainStore {
	root: PathBuf,
	catalog: Catalog,
	config: WorkspaceConfig,
}

impl ToolchainStore {
	pub fn new(workspace: &Path, config: WorkspaceConfig) -> Self {
		Self {
			root: workspace.join(".forge").join("toolchains"),
			catalog: Catalog::parse(EMBEDDED_CATALOG).unwrap_or_default(),
			config,
		}
	}

	pub fn catalog(&self) -> &Catalog {
		&self.catalog
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
				let dir = self.root.join(&resolved.name).join(&resolved.version);
				let bin_dir = pick_bin_dir(&dir);
				if !bin_dir.exists() {
					return Err(toolchain_error(
						name,
						format!("version {version} is not synced (expected {})", bin_dir.display()),
					)
					.with_help("run `forge toolchain sync` to download it"));
				}
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
	let bin = path.join("bin");
	if bin.is_dir() { bin } else { path.to_path_buf() }
}

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
