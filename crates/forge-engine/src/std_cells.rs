use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use forge_diagnostics::{ForgeDiagnostic, codes};

const MANIFEST_TOML: &str = include_str!("../../../prelude/std/manifest.toml");

include!(concat!(env!("OUT_DIR"), "/embedded_cells.rs"));

#[derive(serde::Deserialize, Default)]
struct Manifest {
	cells: BTreeMap<String, CellEntry>,
}

#[derive(serde::Deserialize)]
struct CellEntry {
	extensions: Vec<String>,
	#[serde(default)]
	preferred_toolchain: String,
	#[serde(default)]
	fallback_toolchains: Vec<String>,
	#[serde(default)]
	standard: String,
}

pub struct StdCells {
	scripts: BTreeMap<String, String>,
	workspace_scripts: Vec<String>,
	ext_to_cell: BTreeMap<String, String>,
	toolchain_map: BTreeMap<String, Vec<String>>,
	standard_map: BTreeMap<String, String>,
}

impl StdCells {
	pub fn load(workspace: &Path, patches: &BTreeMap<String, PathBuf>) -> Result<Self, ForgeDiagnostic> {
		let mut scripts: BTreeMap<String, String> = EMBEDDED_CELLS
			.iter()
			.map(|(name, script)| (name.to_string(), script.to_string()))
			.collect();
		for (cell, relative) in patches {
			let absolute = workspace.join(relative);
			let text = std::fs::read_to_string(&absolute).map_err(|e| {
				ForgeDiagnostic::error(
					codes::patch::PATCH_CONFLICT,
					format!("std cell patch `{}`: cannot read {}: {e}", cell, absolute.display()),
				)
			})?;
			scripts.insert(cell.clone(), text);
		}
		let manifest: Manifest = toml::from_str(MANIFEST_TOML)
			.map_err(|e| ForgeDiagnostic::error(codes::script::PARSE_ERROR, format!("invalid std manifest: {e}")))?;
		let mut ext_to_cell = BTreeMap::new();
		let mut toolchain_map = BTreeMap::new();
		let mut standard_map = BTreeMap::new();
		for (cell, entry) in &manifest.cells {
			for ext in &entry.extensions {
				ext_to_cell.insert(ext.clone(), cell.clone());
			}
			let mut candidates = Vec::new();
			if !entry.preferred_toolchain.is_empty() {
				candidates.push(entry.preferred_toolchain.clone());
			}
			for fallback in &entry.fallback_toolchains {
				candidates.push(fallback.clone());
			}
			toolchain_map.insert(cell.clone(), candidates);
			if !entry.standard.is_empty() {
				standard_map.insert(cell.clone(), entry.standard.clone());
			}
		}
		Ok(Self {
			scripts,
			workspace_scripts: EMBEDDED_WORKSPACE_SCRIPTS
				.iter()
				.map(|(_, script)| script.to_string())
				.collect(),
			ext_to_cell,
			toolchain_map,
			standard_map,
		})
	}

	pub fn workspace_scripts(&self) -> &[String] {
		&self.workspace_scripts
	}

	pub fn get(&self, cell: &str) -> Result<&String, ForgeDiagnostic> {
		self.scripts
			.get(cell)
			.ok_or_else(|| ForgeDiagnostic::error(codes::script::WRONG_TYPE, format!("no std cell `{cell}` exists")))
	}

	pub fn cell_for_extension(&self, ext: &str) -> Option<&str> {
		self.ext_to_cell.get(ext).map(|s| s.as_str())
	}

	pub fn supported_languages(&self) -> String {
		let mut by_cell: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
		for (ext, cell) in &self.ext_to_cell {
			by_cell.entry(cell.as_str()).or_default().push(ext.as_str());
		}
		by_cell
			.iter()
			.map(|(cell, exts)| format!("{} ({})", cell, exts.join(", ")))
			.collect::<Vec<_>>()
			.join(", ")
	}

	pub fn available_cells(&self) -> Vec<String> {
		self.scripts.keys().cloned().collect()
	}

	pub fn toolchain_candidates(&self, cell: &str) -> &[String] {
		self.toolchain_map.get(cell).map_or(&[], |v| v.as_slice())
	}

	pub fn standard_for(&self, cell: &str) -> Option<&str> {
		self.standard_map.get(cell).map(|s| s.as_str())
	}
}
