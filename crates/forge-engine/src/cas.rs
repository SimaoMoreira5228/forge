use std::fs;
use std::path::{Path, PathBuf};

use forge_core::OutputKind;
use forge_diagnostics::{ForgeDiagnostic, codes};

use crate::hasher;

#[derive(Debug, Clone)]
pub struct Cas {
	root: PathBuf,
}

impl Cas {
	pub fn open(out_dir: &Path) -> Self {
		Self {
			root: out_dir.join("cas"),
		}
	}

	fn action_dir(&self, cache_key: &str) -> PathBuf {
		self.root.join("actions").join(cache_key)
	}

	pub fn contains(&self, cache_key: &str) -> bool {
		self.action_dir(cache_key).join("manifest.json").exists()
	}

	pub fn restore(
		&self,
		cache_key: &str,
		outputs: &[(std::path::PathBuf, OutputKind)],
		workspace: &Path,
	) -> Result<(), ForgeDiagnostic> {
		let dir = self.action_dir(cache_key);
		for (rel, _kind) in outputs {
			let src = dir.join(rel);
			let dst = workspace.join(rel);
			copy_file(&src, &dst)?;
		}
		touch_last_accessed(&dir);
		Ok(())
	}

	pub fn store(&self, cache_key: &str, outputs: &[(PathBuf, OutputKind)], sandbox: &Path) -> Result<(), ForgeDiagnostic> {
		let dir = self.action_dir(cache_key);
		fs::create_dir_all(&dir).map_err(|e| io(e, &dir))?;
		for (rel, kind) in outputs {
			let src = sandbox.join(rel);
			let dst = dir.join(rel);
			match kind {
				OutputKind::File => copy_file(&src, &dst)?,
				OutputKind::Directory => copy_tree(&src, &dst)?,
			}
		}
		let manifest = Manifest {
			outputs: outputs.iter().map(|(p, _)| p.to_string_lossy().into_owned()).collect(),
		};
		let bytes = serde_json::to_vec(&manifest)
			.map_err(|e| ForgeDiagnostic::error(codes::hermetic::HERMETIC_VIOLATION, format!("manifest encode: {e}")))?;
		std::fs::write(dir.join("manifest.json"), bytes).map_err(|e| io(e, &dir.join("manifest.json")))?;
		touch_last_accessed(&dir);
		Ok(())
	}

	pub fn gc(&self, max_bytes: u64) -> Result<u64, ForgeDiagnostic> {
		let actions = self.root.join("actions");
		if !actions.exists() {
			return Ok(0);
		}
		let mut entries: Vec<(PathBuf, u64, u64)> = Vec::new();
		for entry in fs::read_dir(&actions).map_err(|e| io(e, &actions))?.flatten() {
			let path = entry.path();
			let size = dir_size(&path)?;
			let accessed = fs::metadata(path.join(".accessed"))
				.and_then(|m| m.modified())
				.ok()
				.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
				.map(|d| d.as_secs())
				.unwrap_or(0);
			entries.push((path, size, accessed));
		}
		entries.sort_by_key(|(_, _, accessed)| *accessed);
		let total: u64 = entries.iter().map(|(_, s, _)| s).sum();
		let mut removed = 0u64;
		let mut running = total;
		for (path, size, _) in entries {
			if running <= max_bytes {
				break;
			}
			if fs::remove_dir_all(&path).is_ok() {
				running -= size;
				removed += size;
			}
		}
		Ok(removed)
	}

	pub fn wipe(&self) -> Result<(), ForgeDiagnostic> {
		let dir = self.root.clone();
		if dir.exists() {
			fs::remove_dir_all(&dir).map_err(|e| io(e, &dir))?;
		}
		Ok(())
	}
}

#[derive(serde::Serialize)]
struct Manifest {
	outputs: Vec<String>,
}

fn copy_file(from: &Path, to: &Path) -> Result<(), ForgeDiagnostic> {
	if !from.exists() {
		return Err(ForgeDiagnostic::error(
			codes::hermetic::HERMETIC_VIOLATION,
			format!("expected output `{}` was not produced", from.display()),
		));
	}
	if let Some(parent) = to.parent() {
		fs::create_dir_all(parent).map_err(|e| io(e, parent))?;
	}
	fs::copy(from, to).map(|_| ()).map_err(|e| io(e, from))
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), ForgeDiagnostic> {
	if !from.is_dir() {
		return Err(ForgeDiagnostic::error(
			codes::hermetic::HERMETIC_VIOLATION,
			format!("expected output directory `{}` was not produced", from.display()),
		));
	}
	for entry in walkdir::WalkDir::new(from) {
		let entry = entry.map_err(|e| io(e.into_io_error().unwrap_or_else(|| std::io::Error::other("walk")), from))?;
		let rel = entry.path().strip_prefix(from).expect("prefix walked");
		let dst = to.join(rel);
		if entry.file_type().is_dir() {
			fs::create_dir_all(&dst).map_err(|e| io(e, &dst))?;
		} else {
			copy_file(entry.path(), &dst)?;
		}
	}
	Ok(())
}

fn touch_last_accessed(dir: &Path) {
	let marker = dir.join(".accessed");
	let _ = fs::File::create(&marker);
}

fn dir_size(dir: &Path) -> Result<u64, ForgeDiagnostic> {
	let mut total = 0;
	for entry in walkdir::WalkDir::new(dir) {
		let entry =
			entry.map_err(|e| ForgeDiagnostic::error(codes::hermetic::HERMETIC_VIOLATION, format!("walk failed: {e}")))?;
		if entry.file_type().is_file() {
			total += entry.metadata().map(|m| m.len()).unwrap_or(0);
		}
	}
	Ok(total)
}

fn io(e: std::io::Error, path: &Path) -> ForgeDiagnostic {
	ForgeDiagnostic::error(codes::hermetic::HERMETIC_VIOLATION, format!("{}: {e}", path.display()))
}

pub fn digest_of(bytes: &[u8]) -> String {
	hasher::hex(blake3::hash(bytes).as_bytes())
}
