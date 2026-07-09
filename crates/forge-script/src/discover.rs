use std::path::{Path, PathBuf};

use forge_diagnostics::{ForgeDiagnostic, codes};

use crate::workspace::Discovery;

#[derive(Debug, Clone)]
pub struct PackageSource {
	pub package: String,
	pub file: PathBuf,
}

const CONFIG_FILE_TOML: &str = "FORGE.toml";
const CONFIG_FILE_RHAI: &str = "FORGE.rhai";

const ALWAYS_SKIP: &[&str] = &["forge-out", ".forge", ".git", "target", "node_modules"];

pub fn discover_packages(workspace: &Path, discovery: &Discovery) -> Result<Vec<PackageSource>, ForgeDiagnostic> {
	let mut found = Vec::new();
	let include = if discovery.include.is_empty() {
		vec![PathBuf::from(".")]
	} else {
		discovery.include.clone()
	};
	for root_dir in include {
		walk(workspace.join(root_dir), workspace, discovery, &mut found)?;
	}
	found.sort_by(|a, b| a.package.cmp(&b.package));
	if found.is_empty() {
		return Err(ForgeDiagnostic::error(
			codes::targets::UNKNOWN_TARGET,
			"no FORGE.toml or FORGE.rhai found under the configured discovery paths",
		)
		.with_help("add a [discovery] section to FORGE_ROOT or create a FORGE.toml"));
	}
	Ok(found)
}

fn config_file_in(dir: &Path) -> Option<PathBuf> {
	let toml = dir.join(CONFIG_FILE_TOML);
	if toml.is_file() {
		return Some(toml);
	}
	let rhai = dir.join(CONFIG_FILE_RHAI);
	rhai.is_file().then_some(rhai)
}

fn walk(
	dir: PathBuf,
	workspace: &Path,
	discovery: &Discovery,
	found: &mut Vec<PackageSource>,
) -> Result<(), ForgeDiagnostic> {
	if let Some(file) = config_file_in(&dir) {
		let package = dir
			.strip_prefix(workspace)
			.map(|p| p.to_string_lossy().into_owned())
			.unwrap_or_default();
		found.push(PackageSource { package, file });
		return Ok(());
	}

	let entries = std::fs::read_dir(&dir)
		.map_err(|e| ForgeDiagnostic::error(codes::inputs::MISSING_INPUT, format!("cannot read {}: {e}", dir.display())))?;
	for entry in entries.flatten() {
		let path = entry.path();
		if !entry.file_type().is_ok_and(|t| t.is_dir()) {
			continue;
		}
		let name = entry.file_name().to_string_lossy().into_owned();
		if ALWAYS_SKIP.contains(&name.as_str()) || discovery.exclude.iter().any(|x| x == &name) {
			continue;
		}
		walk(path, workspace, discovery, found)?;
	}
	Ok(())
}
