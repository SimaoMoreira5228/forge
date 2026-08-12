use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use forge_diagnostics::{ForgeDiagnostic, codes};
use serde::{Deserialize, Serialize};

use crate::hasher;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionProof {
	pub action: String,
	pub component: String,
	pub key: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub toolchain: Option<String>,
	pub inputs: Vec<(String, String)>,
	pub outputs: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proof {
	pub version: u32,
	pub digest: String,
	pub entries: Vec<ActionProof>,
}

#[derive(Debug, Clone)]
pub struct ReplayDivergence {
	pub action: String,
	pub detail: String,
}

#[derive(Debug, Clone)]
pub struct Divergence {
	pub action: String,
	pub detail: String,
}

impl Proof {
	pub const VERSION: u32 = 1;

	pub fn seal(entries: Vec<ActionProof>) -> Result<Self, ForgeDiagnostic> {
		let digest = digest_of(&entries)?;
		Ok(Self {
			version: Self::VERSION,
			digest,
			entries,
		})
	}

	pub fn load(path: &Path) -> Result<Self, ForgeDiagnostic> {
		let text = std::fs::read_to_string(path)
			.map_err(|e| ForgeDiagnostic::error(codes::inputs::MISSING_INPUT, format!("{}: {e}", path.display())))?;
		serde_json::from_str(&text)
			.map_err(|e| ForgeDiagnostic::error(codes::script::PARSE_ERROR, format!("{}: {e}", path.display())))
	}

	pub fn write(&self, path: &Path) -> Result<(), ForgeDiagnostic> {
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent).map_err(|e| ForgeDiagnostic::error(8, format!("{}: {e}", parent.display())))?;
		}
		let bytes = serde_json::to_vec_pretty(self).map_err(|e| ForgeDiagnostic::error(8, format!("proof encode: {e}")))?;
		std::fs::write(path, bytes).map_err(|e| ForgeDiagnostic::error(8, format!("{}: {e}", path.display())))
	}

	pub fn verify(&self, workspace: &Path) -> Result<Vec<Divergence>, ForgeDiagnostic> {
		let mut divergences = Vec::new();
		if self.version != Self::VERSION {
			divergences.push(Divergence {
				action: String::new(),
				detail: format!("proof version {} is not {}\n", self.version, Self::VERSION),
			});
		}
		if self.digest != digest_of(&self.entries)? {
			divergences.push(Divergence {
				action: String::new(),
				detail: "proof digest does not match its entries\n".to_string(),
			});
		}
		let produced: Vec<&str> = self
			.entries
			.iter()
			.flat_map(|entry| entry.outputs.iter().map(|(path, _)| path.as_str()))
			.collect();
		for entry in &self.entries {
			for (label, recorded) in [("input", &entry.inputs), ("output", &entry.outputs)] {
				for (path, expected) in recorded {
					if label == "input" && is_generated(path, &produced) {
						continue;
					}
					match hasher::hash_path(workspace, Path::new(path)) {
						Ok(actual) if actual == *expected => {}
						Ok(actual) => divergences.push(Divergence {
							action: entry.action.clone(),
							detail: format!("{label} `{path}`: recorded {expected}, found {actual}"),
						}),
						Err(e) => divergences.push(Divergence {
							action: entry.action.clone(),
							detail: format!("{label} `{path}`: {e}"),
						}),
					}
				}
			}
		}
		Ok(divergences)
	}
}

pub fn compare(recorded: &Proof, fresh: &Proof) -> Vec<ReplayDivergence> {
	let mut divergences = Vec::new();
	let replays: BTreeMap<&str, &ActionProof> = fresh.entries.iter().map(|entry| (entry.action.as_str(), entry)).collect();
	for entry in &recorded.entries {
		let Some(replayed) = replays.get(entry.action.as_str()) else {
			divergences.push(ReplayDivergence {
				action: entry.action.clone(),
				detail: "action was not replayed".to_string(),
			});
			continue;
		};
		if replayed.key != entry.key {
			divergences.push(ReplayDivergence {
				action: entry.action.clone(),
				detail: "inputs or toolchain differ from the proof".to_string(),
			});
		}
		let outputs: BTreeMap<&str, &str> = entry
			.outputs
			.iter()
			.map(|(path, hash)| (path.as_str(), hash.as_str()))
			.collect();
		for (path, hash) in &replayed.outputs {
			match outputs.get(path.as_str()) {
				Some(expected) if *expected == hash => {}
				Some(expected) => divergences.push(ReplayDivergence {
					action: entry.action.clone(),
					detail: format!("output `{path}`: proof {expected}, replay {hash}"),
				}),
				None => divergences.push(ReplayDivergence {
					action: entry.action.clone(),
					detail: format!("output `{path}` was not in the proof"),
				}),
			}
		}
	}
	let proven: BTreeSet<&str> = recorded.entries.iter().map(|entry| entry.action.as_str()).collect();
	for entry in &fresh.entries {
		if !proven.contains(entry.action.as_str()) {
			divergences.push(ReplayDivergence {
				action: entry.action.clone(),
				detail: "action was not in the proof".to_string(),
			});
		}
	}
	divergences
}

impl Proof {
	pub fn check_seal(&self) -> Result<(), ForgeDiagnostic> {
		if self.digest == digest_of(&self.entries)? {
			return Ok(());
		}
		Err(ForgeDiagnostic::error(
			codes::script::PARSE_ERROR,
			"proof digest does not match its entries",
		))
	}
}

fn is_generated(path: &str, produced: &[&str]) -> bool {
	produced
		.iter()
		.any(|output| path == *output || path.strip_prefix(output).is_some_and(|rest| rest.starts_with('/')))
}

fn digest_of(entries: &[ActionProof]) -> Result<String, ForgeDiagnostic> {
	let bytes = serde_json::to_vec(entries).map_err(|e| ForgeDiagnostic::error(8, format!("proof encode: {e}")))?;
	Ok(hasher::hex(blake3::hash(&bytes).as_bytes()))
}

pub fn hash_records(workspace: &Path, paths: &[PathBuf]) -> Result<Vec<(String, String)>, ForgeDiagnostic> {
	let mut records = Vec::with_capacity(paths.len());
	for path in paths {
		let hash = hasher::hash_path(workspace, path)
			.map_err(|e| ForgeDiagnostic::error(codes::inputs::MISSING_INPUT, format!("{}: {e}", path.display())))?;
		records.push((path.to_string_lossy().into_owned(), hash));
	}
	Ok(records)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn generated_inputs_are_not_reverified() {
		let produced = ["forge-out/build/x", "forge-out/lib/a.rlib"];
		assert!(is_generated("forge-out/build/x", &produced));
		assert!(is_generated("forge-out/build/x/output", &produced));
		assert!(!is_generated("forge-out/build/xy", &produced));
		assert!(!is_generated("crates/forge/src/main.rs", &produced));
	}
}
