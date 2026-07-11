use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use forge_diagnostics::{ForgeDiagnostic, codes};

const C_CELL: &str = include_str!("../../../prelude/std/c/cell.rhai");

pub struct StdCells {
	scripts: BTreeMap<String, String>,
}

impl StdCells {
	pub fn load(workspace: &Path, patches: &BTreeMap<String, PathBuf>) -> Result<Self, ForgeDiagnostic> {
		let mut scripts = BTreeMap::from([("c".to_string(), C_CELL.to_string())]);
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
		Ok(Self { scripts })
	}

	pub fn get(&self, cell: &str) -> Result<&String, ForgeDiagnostic> {
		self.scripts
			.get(cell)
			.ok_or_else(|| ForgeDiagnostic::error(codes::script::WRONG_TYPE, format!("no std cell `{cell}` exists")))
	}
}
