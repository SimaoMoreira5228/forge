use std::path::{Path, PathBuf};
use std::process::Command;

use forge_core::OutputKind;
use forge_diagnostics::{ForgeDiagnostic, codes};

use crate::builder::Engine;
use crate::cas::Cas;
use crate::proof::Proof;

impl Engine {
	pub fn time_travel(&self, proof_path: &Path) -> Result<(usize, usize), ForgeDiagnostic> {
		let _lock = self.exclusive_lock()?;
		let proof = Proof::load(proof_path)?;
		let cas = Cas::open(&self.out_dir());
		let mut restored = 0;
		let mut unavailable = 0;
		for entry in &proof.entries {
			if !cas.contains(&entry.key) {
				unavailable += 1;
				continue;
			}
			let outputs: Vec<(PathBuf, OutputKind)> = entry
				.outputs
				.iter()
				.filter_map(|(path, _)| {
					cas.published_kind(&entry.key, Path::new(path))
						.map(|kind| (PathBuf::from(path), kind))
				})
				.collect();
			if outputs.is_empty() {
				unavailable += 1;
				continue;
			}
			cas.restore(&entry.key, &outputs, &self.workspace)?;
			restored += 1;
		}
		Ok((restored, unavailable))
	}
}

pub fn record_revision(workspace: &Path, out_dir: &Path) -> Result<(), ForgeDiagnostic> {
	let Some(revision) = git(workspace, &["rev-parse", "--verify", "HEAD"]) else {
		return Ok(());
	};
	let proofs = out_dir.join("proofs");
	std::fs::create_dir_all(&proofs).map_err(|e| io_error(&proofs, e))?;
	std::fs::copy(out_dir.join("forge.proof"), proofs.join(format!("{revision}.proof")))
		.map(|_| ())
		.map_err(|e| io_error(&proofs, e))
}

pub fn revision_proof_path(workspace: &Path, to: &str) -> Result<PathBuf, ForgeDiagnostic> {
	let revision = git(workspace, &["rev-parse", "--verify", &format!("{to}^{{commit}}")])
		.ok_or_else(|| ForgeDiagnostic::error(8, format!("cannot resolve revision `{to}`")))?;
	Ok(workspace.join("forge-out/proofs").join(format!("{revision}.proof")))
}

fn git(workspace: &Path, args: &[&str]) -> Option<String> {
	let output = Command::new("git").arg("-C").arg(workspace).args(args).output().ok()?;
	if !output.status.success() {
		return None;
	}
	let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
	(!text.is_empty()).then_some(text)
}

fn io_error(path: &Path, error: std::io::Error) -> ForgeDiagnostic {
	ForgeDiagnostic::error(codes::hermetic::HERMETIC_VIOLATION, format!("{}: {error}", path.display()))
}
