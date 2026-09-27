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
	pub fn open() -> Self {
		Self {
			root: crate::store::Store::open().actions(),
		}
	}

	fn action_dir(&self, cache_key: &str) -> PathBuf {
		self.root.join(cache_key)
	}

	pub fn at(root: impl Into<PathBuf>) -> Self {
		Self { root: root.into() }
	}

	pub(crate) fn action_path(&self, cache_key: &str) -> PathBuf {
		self.action_dir(cache_key)
	}

	pub fn contains(&self, cache_key: &str) -> bool {
		self.action_dir(cache_key).join("manifest.json").exists()
	}

	pub fn published_kind(&self, cache_key: &str, path: &Path) -> Option<OutputKind> {
		let target = self.action_dir(cache_key).join(path);
		if target.is_dir() {
			Some(OutputKind::Directory)
		} else if target.is_file() {
			Some(OutputKind::File)
		} else {
			None
		}
	}

	pub fn restore(
		&self,
		cache_key: &str,
		outputs: &[(std::path::PathBuf, OutputKind)],
		workspace: &Path,
	) -> Result<Vec<(String, String)>, ForgeDiagnostic> {
		let dir = self.action_dir(cache_key);
		for (rel, kind) in outputs {
			let src = dir.join(rel);
			let dst = workspace.join(rel);
			match kind {
				OutputKind::File => crate::publish::publish_file(&src, &dst).map_err(|e| ForgeDiagnostic::io(&dst, e))?,
				OutputKind::Directory => {
					crate::publish::publish_tree(&src, &dst).map_err(|e| ForgeDiagnostic::io(&dst, e))?
				}
			}
		}
		touch_last_accessed(&dir);
		self.stored_output_digests(cache_key)
	}

	pub fn stored_output_digests(&self, cache_key: &str) -> Result<Vec<(String, String)>, ForgeDiagnostic> {
		let bytes = fs::read(self.action_dir(cache_key).join("manifest.json"))
			.map_err(|e| ForgeDiagnostic::io(&self.action_dir(cache_key).join("manifest.json"), e))?;
		let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| {
			ForgeDiagnostic::error(
				codes::hermetic::HERMETIC_VIOLATION,
				format!("action {cache_key} has no readable output manifest: {e}"),
			)
		})?;
		Ok(manifest
			.outputs
			.into_iter()
			.map(|output| (output.path, output.hash))
			.collect())
	}

	pub fn store(
		&self,
		cache_key: &str,
		outputs: &[(PathBuf, OutputKind)],
		sandbox: &Path,
	) -> Result<Vec<(String, String)>, ForgeDiagnostic> {
		let dir = self.action_dir(cache_key);
		fs::create_dir_all(&dir).map_err(|e| ForgeDiagnostic::io(&dir, e))?;
		let mut recorded = Vec::with_capacity(outputs.len());
		for (rel, kind) in outputs {
			let src = sandbox.join(rel);
			let dst = dir.join(rel);
			let hash = match kind {
				OutputKind::File => hasher::hex(&copy_file(&src, &dst)?),
				OutputKind::Directory => copy_tree(&src, &dst)?,
			};
			recorded.push(StoredOutput {
				path: rel.to_string_lossy().into_owned(),
				hash,
			});
		}
		let manifest = Manifest { outputs: recorded };
		let bytes = serde_json::to_vec(&manifest)
			.map_err(|e| ForgeDiagnostic::error(codes::hermetic::HERMETIC_VIOLATION, format!("manifest encode: {e}")))?;
		std::fs::write(dir.join("manifest.json"), bytes).map_err(|e| ForgeDiagnostic::io(&dir.join("manifest.json"), e))?;
		touch_last_accessed(&dir);
		Ok(manifest
			.outputs
			.into_iter()
			.map(|output| (output.path, output.hash))
			.collect())
	}

	pub fn gc(&self, max_bytes: u64) -> Result<u64, ForgeDiagnostic> {
		let store = crate::store::Store::open();
		let _lease = store.lock("lease")?;
		let _publication = store.lock("store")?;
		let actions = self.root.clone();
		if !actions.exists() {
			return Ok(0);
		}
		let mut entries: Vec<(PathBuf, u64, u64)> = Vec::new();
		for entry in fs::read_dir(&actions)
			.map_err(|e| ForgeDiagnostic::io(&actions, e))?
			.flatten()
		{
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
			fs::remove_dir_all(&dir).map_err(|e| ForgeDiagnostic::io(&dir, e))?;
		}
		Ok(())
	}
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Manifest {
	outputs: Vec<StoredOutput>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredOutput {
	path: String,
	hash: String,
}

fn copy_file(from: &Path, to: &Path) -> Result<[u8; 32], ForgeDiagnostic> {
	if !from.exists() {
		return Err(ForgeDiagnostic::error(
			codes::hermetic::HERMETIC_VIOLATION,
			format!("expected output `{}` was not produced", from.display()),
		));
	}
	if let Some(parent) = to.parent() {
		fs::create_dir_all(parent).map_err(|e| ForgeDiagnostic::io(parent, e))?;
	}
	let mut reader = fs::File::open(from).map_err(|e| ForgeDiagnostic::io(from, e))?;
	let mut writer = fs::File::create(to).map_err(|e| ForgeDiagnostic::io(to, e))?;
	let mut hasher = blake3::Hasher::new();
	let mut buffer = vec![0u8; 128 * 1024];
	loop {
		let read = std::io::Read::read(&mut reader, &mut buffer).map_err(|e| ForgeDiagnostic::io(from, e))?;
		if read == 0 {
			break;
		}
		hasher.update(&buffer[..read]);
		std::io::Write::write_all(&mut writer, &buffer[..read]).map_err(|e| ForgeDiagnostic::io(to, e))?;
	}
	drop(writer);
	let mode = reader.metadata().map_err(|e| ForgeDiagnostic::io(from, e))?.permissions();
	fs::set_permissions(to, mode).map_err(|e| ForgeDiagnostic::io(to, e))?;
	Ok(*hasher.finalize().as_bytes())
}

fn copy_tree(from: &Path, to: &Path) -> Result<String, ForgeDiagnostic> {
	if !from.is_dir() {
		return Err(ForgeDiagnostic::error(
			codes::hermetic::HERMETIC_VIOLATION,
			format!("expected output directory `{}` was not produced", from.display()),
		));
	}
	fs::create_dir_all(to).map_err(|e| ForgeDiagnostic::io(to, e))?;
	let mut files: Vec<PathBuf> = walkdir::WalkDir::new(from)
		.into_iter()
		.filter_map(Result::ok)
		.filter(|entry| entry.file_type().is_file())
		.map(|entry| entry.path().to_path_buf())
		.collect();
	files.sort();
	let mut hasher = blake3::Hasher::new();
	for file in &files {
		let rel = file.strip_prefix(from).unwrap_or(file);
		hasher.update(rel.to_string_lossy().as_bytes());
		hasher.update(&[0]);
		hasher.update(&copy_file(file, &to.join(rel))?);
	}
	Ok(hasher::hex(hasher.finalize().as_bytes()))
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

pub fn digest_of(bytes: &[u8]) -> String {
	hasher::hex(blake3::hash(bytes).as_bytes())
}
