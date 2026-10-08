use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use forge_core::Catalog;
use forge_diagnostics::{ForgeDiagnostic, codes};
use forge_script::{ToolchainSelection, WorkspaceConfig};

use crate::store::hasher;

pub const EMBEDDED_CATALOG: &str = include_str!("../../../../prelude/toolchains/catalog.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogOrigin {
	Embedded,
	Workspace,
}

#[derive(Debug, Clone)]
pub struct ResolvedToolchain {
	pub name: String,
	pub root: PathBuf,
	pub bin_dir: PathBuf,
	pub path_dirs: Vec<PathBuf>,
	pub digest: String,
	pub coverage: Option<forge_core::toolchain::catalog::CoverageBackend>,
	pub worker: Option<forge_core::worker::WorkerProgram>,
}

pub const TOOLCHAIN_TOKEN: &str = "FORGE_TOOLCHAIN";

#[derive(Default)]
pub struct ToolchainPaths {
	pub bin: Vec<PathBuf>,
	pub read_only: Vec<PathBuf>,
	roots: BTreeMap<String, PathBuf>,
}

impl ToolchainPaths {
	pub fn of(toolchains: &BTreeMap<String, ResolvedToolchain>) -> Self {
		let mut paths = Self {
			bin: toolchains.values().flat_map(|t| t.path_dirs.iter().cloned()).collect(),
			read_only: toolchains.values().map(|t| t.root.clone()).collect(),
			roots: toolchains.values().map(|t| (t.id(), t.root.clone())).collect(),
		};
		paths.bin.sort();
		paths.bin.dedup();
		paths.read_only.sort();
		paths.read_only.dedup();
		paths
	}

	pub fn bin_refs(&self) -> Vec<&Path> {
		self.bin.iter().map(PathBuf::as_path).collect()
	}

	pub fn expand(&self, value: &str) -> String {
		let mut expanded = value.to_string();
		for (id, root) in &self.roots {
			expanded = expanded.replace(&format!("{TOOLCHAIN_TOKEN}/{id}/"), &format!("{}/", root.to_string_lossy()));
		}
		expanded
	}
}

impl ResolvedToolchain {
	pub fn id(&self) -> String {
		format!("{}@{}", self.name, &self.digest[..12.min(self.digest.len())])
	}

	pub fn reference(&self, binary: &Path) -> Result<String, ForgeDiagnostic> {
		let relative = binary.strip_prefix(&self.root).map_err(|_| {
			ForgeDiagnostic::error(
				codes::hermetic::TOOLCHAIN_MISMATCH,
				format!("tool `{}` is outside toolchain `{}`", binary.display(), self.name),
			)
		})?;
		Ok(format!(
			"{TOOLCHAIN_TOKEN}/{}/{}",
			self.id(),
			relative.to_string_lossy().replace('\\', "/")
		))
	}

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
	Err(not_found(toolchains, spec))
}

pub fn resolve_tool_reference(
	toolchains: &BTreeMap<String, ResolvedToolchain>,
	spec: &str,
) -> Result<String, ForgeDiagnostic> {
	if PathBuf::from(spec).is_absolute() {
		return resolve_tool_path(toolchains, spec).map(|path| path.to_string_lossy().into_owned());
	}
	for tool in toolchains.values() {
		if let Some(found) = tool.binary(spec) {
			return tool.reference(&found);
		}
	}
	Err(not_found(toolchains, spec))
}

fn not_found(toolchains: &BTreeMap<String, ResolvedToolchain>, spec: &str) -> ForgeDiagnostic {
	let searched: Vec<String> = toolchains.values().map(|t| t.bin_dir.display().to_string()).collect();
	ForgeDiagnostic::error(
		codes::hermetic::TOOLCHAIN_MISMATCH,
		format!("tool `{spec}` was not found in any configured toolchain"),
	)
	.with_help(format!(
		"searched: {}; add a [toolchains.<name>] section providing it",
		searched.join(", ")
	))
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
				let mut tool = Self::finish(name, &bin_dir, bin_dir.clone());
				tool.coverage = self.catalog.get(name).and_then(|entry| entry.coverage.clone());
				tool.worker = self.catalog.get(name).and_then(|entry| entry.worker.clone());
				Ok(tool)
			}
			ToolchainSelection::Version { version } => {
				let resolved = self.catalog.resolve(name, Some(version))?;
				let dir = self.install_dir(&resolved.name, &resolved.version);
				let mut tool = self.synced_root(name, version, &dir)?;
				tool.coverage = resolved.entry.coverage.clone();
				tool.worker = resolved.entry.worker.clone();
				Ok(tool)
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
			root: root.to_path_buf(),
			bin_dir,
			path_dirs,
			digest,
			coverage: None,
			worker: None,
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

#[cfg(test)]
mod tests {
	use super::*;

	fn toolchain(name: &str, root: &str, digest: &str) -> ResolvedToolchain {
		ResolvedToolchain {
			name: name.to_string(),
			root: root.into(),
			bin_dir: root.into(),
			path_dirs: vec![root.into()],
			digest: digest.to_string(),
			coverage: None,
			worker: None,
		}
	}

	#[test]
	fn a_reference_names_the_toolchain_and_the_binary_not_where_the_store_lives() {
		let here = toolchain("rust", "/store/one/toolchains/rust/1.98.0", "abcdef0123456789");
		let there = toolchain("rust", "/elsewhere/toolchains/rust/1.98.0", "abcdef0123456789");
		let binary = |tool: &ResolvedToolchain| tool.root.join("bin/rustc");

		assert_eq!(
			here.reference(&binary(&here)).unwrap(),
			there.reference(&binary(&there)).unwrap()
		);
		assert_eq!(
			here.reference(&binary(&here)).unwrap(),
			"FORGE_TOOLCHAIN/rust@abcdef012345/bin/rustc"
		);
	}

	#[test]
	fn a_reference_keeps_distinguishing_the_things_a_key_must_distinguish() {
		let rust = toolchain("rust", "/store/toolchains/rust/1.98.0", "abcdef0123456789");
		let clang = toolchain("clang", "/store/toolchains/clang/1.98.0", "abcdef0123456789");
		let rebuilt = toolchain("rust", "/store/toolchains/rust/1.98.0", "ffffffffffffffff");
		let other_binary = toolchain("rust", "/store/toolchains/rust/1.98.0", "abcdef0123456789");

		let reference = |tool: &ResolvedToolchain, binary: &str| tool.reference(&PathBuf::from(binary)).unwrap();
		let base = reference(&rust, "/store/toolchains/rust/1.98.0/bin/rustc");

		assert_ne!(
			base,
			reference(&clang, "/store/toolchains/clang/1.98.0/bin/rustc"),
			"a different toolchain is a different key"
		);
		assert_ne!(
			base,
			reference(&rebuilt, "/store/toolchains/rust/1.98.0/bin/rustc"),
			"a changed toolchain digest is a different key"
		);
		assert_ne!(
			base,
			reference(&other_binary, "/store/toolchains/rust/1.98.0/bin/cargo"),
			"a different binary is a different key"
		);
	}

	#[test]
	fn a_binary_outside_its_toolchain_has_no_reference() {
		let tool = toolchain("gcc", "/store/toolchains/gcc/14", "abcdef0123456789");
		assert!(tool.reference(Path::new("/usr/bin/cc")).is_err());
	}

	#[test]
	fn expanding_a_reference_returns_the_binary_to_run() {
		let toolchains = BTreeMap::from([(
			"rust".to_string(),
			toolchain("rust", "/store/one/toolchains/rust/1.98.0", "abcdef0123456789"),
		)]);
		let paths = ToolchainPaths::of(&toolchains);

		assert_eq!(
			paths.expand("FORGE_TOOLCHAIN/rust@abcdef012345/bin/rustc"),
			"/store/one/toolchains/rust/1.98.0/bin/rustc"
		);
		assert_eq!(
			paths.expand("-L dependency=FORGE_TOOLCHAIN/rust@abcdef012345/lib"),
			"-L dependency=/store/one/toolchains/rust/1.98.0/lib"
		);
		assert_eq!(paths.expand("rustc"), "rustc", "a bare tool name is left alone");
	}

	#[test]
	fn a_reference_only_expands_for_the_toolchain_it_names() {
		let toolchains = BTreeMap::from([
			(
				"rust".to_string(),
				toolchain("rust", "/store/one/rust", "abcdef0123456789abcdef01"),
			),
			(
				"clang".to_string(),
				toolchain("clang", "/store/one/clang", "fedcba9876543210fedcba98"),
			),
		]);
		let paths = ToolchainPaths::of(&toolchains);

		assert_eq!(
			paths.expand("FORGE_TOOLCHAIN/rust@abcdef012345/bin/rustc"),
			"/store/one/rust/bin/rustc"
		);
		assert_eq!(
			paths.expand("FORGE_TOOLCHAIN/clang@fedcba987654/bin/clang"),
			"/store/one/clang/bin/clang"
		);
	}

	#[test]
	fn a_reference_is_resolved_from_the_toolchain_that_actually_provides_the_binary() {
		let root = std::env::temp_dir().join(format!("forge-toolref-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(root.join("bin")).unwrap();
		std::fs::write(root.join("bin/rustc"), b"tool").unwrap();
		let mut rust = toolchain("rust", &root.to_string_lossy(), "abcdef0123456789abcdef01");
		rust.bin_dir = root.join("bin");
		rust.path_dirs = vec![rust.bin_dir.clone()];
		let toolchains = BTreeMap::from([("rust".to_string(), rust)]);

		assert_eq!(
			resolve_tool_reference(&toolchains, "rustc").unwrap(),
			"FORGE_TOOLCHAIN/rust@abcdef012345/bin/rustc".to_string()
		);
		assert!(resolve_tool_reference(&toolchains, "no-such-tool").is_err());
		assert_eq!(resolve_tool_path(&toolchains, "rustc").unwrap(), root.join("bin/rustc"));
		assert_eq!(
			resolve_tool_reference(&toolchains, &root.join("bin/rustc").to_string_lossy()).unwrap(),
			root.join("bin/rustc").to_string_lossy()
		);
		assert!(resolve_tool_reference(&toolchains, "/no/such/binary").is_err());

		let _ = std::fs::remove_dir_all(&root);
	}
}
