use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use forge_diagnostics::ForgeDiagnostic;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct FileStat {
	size: u64,
	mtime: i64,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct LedgerFile {
	entries: HashMap<String, HashMap<String, FileStat>>,
}

pub struct Materialized {
	state: parking_lot::Mutex<HashMap<PathBuf, HashMap<String, FileStat>>>,
	path: PathBuf,
}

impl Materialized {
	pub fn load(out_dir: &Path) -> Self {
		let path = out_dir.join("cas/materialized.json");
		let entries: HashMap<String, HashMap<String, FileStat>> = std::fs::read(&path)
			.ok()
			.and_then(|bytes| serde_json::from_slice::<LedgerFile>(&bytes).ok())
			.map(|file| file.entries)
			.unwrap_or_default();
		Self {
			state: parking_lot::Mutex::new(entries.into_iter().map(|(rel, entry)| (PathBuf::from(rel), entry)).collect()),
			path,
		}
	}

	pub fn is_fresh(&self, workspace: &Path, rel: &Path, key: &str) -> bool {
		let state = self.state.lock();
		let Some(entry) = state.get(rel).and_then(|keys| keys.get(key)) else {
			return false;
		};
		let Ok(metadata) = std::fs::metadata(workspace.join(rel)) else {
			return false;
		};
		metadata.len() == entry.size && file_mtime(&metadata) == entry.mtime
	}

	pub fn record(&self, workspace: &Path, rel: &Path, key: &str) {
		let Ok(metadata) = std::fs::metadata(workspace.join(rel)) else {
			return;
		};
		self.state.lock().entry(rel.to_path_buf()).or_default().insert(
			key.to_string(),
			FileStat {
				size: metadata.len(),
				mtime: file_mtime(&metadata),
			},
		);
	}

	pub fn confirm(&self, workspace: &Path, rel: &Path, cas_file: &Path, key: &str) -> bool {
		if self.is_fresh(workspace, rel, key) {
			return true;
		}
		let dst = workspace.join(rel);
		if !same_size(&dst, cas_file) || !bytes_equal(&dst, cas_file) {
			return false;
		}
		self.record(workspace, rel, key);
		true
	}

	pub fn save(&self) {
		let state = self.state.lock();
		let file = LedgerFile {
			entries: state
				.iter()
				.map(|(rel, entry)| (rel.to_string_lossy().into_owned(), entry.clone()))
				.collect(),
		};
		if let Ok(bytes) = serde_json::to_vec(&file)
			&& let Some(parent) = self.path.parent()
			&& std::fs::create_dir_all(parent).is_ok()
		{
			let _ = std::fs::write(&self.path, bytes);
		}
	}

	pub fn sync_dir(&self, cas_src: &Path, workspace: &Path, rel: &Path, key: &str) -> Result<(), ForgeDiagnostic> {
		let src = cas_src.join(rel);
		let dst = workspace.join(rel);
		let mut expected: BTreeSet<PathBuf> = BTreeSet::new();
		let mut files: Vec<PathBuf> = Vec::new();
		let mut dirs: Vec<PathBuf> = Vec::new();
		for entry in walkdir::WalkDir::new(&src).into_iter().filter_map(Result::ok) {
			let path = entry.path().to_path_buf();
			let child = path.strip_prefix(&src).unwrap_or(&path).to_path_buf();
			expected.insert(child.clone());
			if entry.file_type().is_dir() {
				dirs.push(child);
			} else {
				files.push(child);
			}
		}
		for dir in &dirs {
			std::fs::create_dir_all(dst.join(dir)).map_err(|e| ForgeDiagnostic::io(&dst.join(dir), e))?;
		}
		for file in &files {
			let rel_file = rel.join(file);
			if self.confirm(workspace, &rel_file, &src.join(file), key) {
				continue;
			}
			crate::store::publish::publish_file(&src.join(file), &dst.join(file))
				.map_err(|e| ForgeDiagnostic::io(&dst.join(file), e))?;
			self.record(workspace, &rel_file, key);
		}
		if dst.is_dir() {
			let mut stale: Vec<PathBuf> = Vec::new();
			for entry in walkdir::WalkDir::new(&dst).into_iter().filter_map(Result::ok) {
				let path = entry.path().to_path_buf();
				if path == dst {
					continue;
				}
				let child = path.strip_prefix(&dst).unwrap_or(&path).to_path_buf();
				if !expected.contains(&child) {
					stale.push(path);
				}
			}
			stale.sort_by_key(|path| path.components().count());
			for path in stale.iter().rev() {
				if path.is_dir() {
					if std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none()) {
						let _ = std::fs::remove_dir(path);
					}
				} else {
					let _ = std::fs::remove_file(path);
				}
			}
		}
		Ok(())
	}
}

fn file_mtime(metadata: &std::fs::Metadata) -> i64 {
	metadata
		.modified()
		.ok()
		.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
		.map(|duration| duration.as_nanos() as i64)
		.unwrap_or(0)
}

fn same_size(first: &Path, second: &Path) -> bool {
	match (std::fs::metadata(first), std::fs::metadata(second)) {
		(Ok(a), Ok(b)) => a.len() == b.len(),
		_ => false,
	}
}

fn bytes_equal(first: &Path, second: &Path) -> bool {
	use std::io::Read;
	let (Ok(a), Ok(b)) = (std::fs::File::open(first), std::fs::File::open(second)) else {
		return false;
	};
	let mut a = std::io::BufReader::new(a);
	let mut b = std::io::BufReader::new(b);
	let mut a_buf = vec![0u8; 128 * 1024];
	let mut b_buf = vec![0u8; 128 * 1024];
	loop {
		let (Ok(a_read), Ok(b_read)) = (a.read(&mut a_buf), b.read(&mut b_buf)) else {
			return false;
		};
		if a_read != b_read || a_buf[..a_read] != b_buf[..b_read] {
			return false;
		}
		if a_read == 0 {
			return true;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn scratch(name: &str) -> PathBuf {
		let base = std::env::temp_dir().join(format!("forge-materialized-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&base);
		std::fs::create_dir_all(base.join("ws")).unwrap();
		base
	}

	#[test]
	fn an_unrecorded_output_is_never_fresh() {
		let base = scratch("unrecorded");
		let workspace = base.join("ws");
		let materialized = Materialized::load(&base.join("out"));
		std::fs::write(workspace.join("a.rlib"), b"bytes").unwrap();
		assert!(!materialized.is_fresh(&workspace, Path::new("a.rlib"), "key"));
		let _ = std::fs::remove_dir_all(&base);
	}

	#[test]
	fn a_recorded_output_stays_fresh_until_it_changes() {
		let base = scratch("fresh");
		let workspace = base.join("ws");
		let materialized = Materialized::load(&base.join("out"));
		std::fs::write(workspace.join("a.rlib"), b"bytes").unwrap();
		materialized.record(&workspace, Path::new("a.rlib"), "key");
		assert!(materialized.is_fresh(&workspace, Path::new("a.rlib"), "key"));
		assert!(!materialized.is_fresh(&workspace, Path::new("a.rlib"), "other"));
		std::fs::write(workspace.join("a.rlib"), b"changed!").unwrap();
		assert!(!materialized.is_fresh(&workspace, Path::new("a.rlib"), "key"));
		std::fs::remove_file(workspace.join("a.rlib")).unwrap();
		assert!(!materialized.is_fresh(&workspace, Path::new("a.rlib"), "key"));
		let _ = std::fs::remove_dir_all(&base);
	}

	#[test]
	fn the_ledger_survives_a_reload() {
		let base = scratch("reload");
		let workspace = base.join("ws");
		let out_dir = base.join("out");
		std::fs::write(workspace.join("a.rlib"), b"bytes").unwrap();
		let materialized = Materialized::load(&out_dir);
		materialized.record(&workspace, Path::new("a.rlib"), "key");
		materialized.save();
		assert!(Materialized::load(&out_dir).is_fresh(&workspace, Path::new("a.rlib"), "key"));
		let _ = std::fs::remove_dir_all(&base);
	}

	#[test]
	fn a_corrupt_ledger_loads_empty() {
		let base = scratch("corrupt");
		let out_dir = base.join("out");
		std::fs::create_dir_all(out_dir.join("cas")).unwrap();
		std::fs::write(out_dir.join("cas/materialized.json"), b"not json").unwrap();
		let workspace = base.join("ws");
		assert!(!Materialized::load(&out_dir).is_fresh(&workspace, Path::new("a.rlib"), "key"));
		let _ = std::fs::remove_dir_all(&base);
	}

	#[test]
	fn sync_dir_copies_once_then_skips_fresh_files_and_prunes_extras() {
		let base = scratch("syncdir");
		let workspace = base.join("ws");
		let cas = base.join("cas");
		std::fs::create_dir_all(cas.join("out/gen")).unwrap();
		std::fs::write(cas.join("out/gen/a.rs"), b"a").unwrap();
		std::fs::write(cas.join("out/gen/b.rs"), b"b").unwrap();
		let materialized = Materialized::load(&base.join("out"));

		materialized.sync_dir(&cas, &workspace, Path::new("out"), "key").unwrap();
		assert_eq!(std::fs::read(workspace.join("out/gen/a.rs")).unwrap(), b"a");
		let mtime = std::fs::metadata(workspace.join("out/gen/a.rs")).unwrap().modified().unwrap();

		materialized.sync_dir(&cas, &workspace, Path::new("out"), "key").unwrap();
		assert_eq!(
			std::fs::metadata(workspace.join("out/gen/a.rs")).unwrap().modified().unwrap(),
			mtime,
			"a fresh tree must not be recopied"
		);

		std::fs::write(workspace.join("out/gen/stale.rs"), b"stale").unwrap();
		std::fs::write(cas.join("out/gen/a.rs"), b"changed").unwrap();
		materialized.sync_dir(&cas, &workspace, Path::new("out"), "other").unwrap();
		assert_eq!(std::fs::read(workspace.join("out/gen/a.rs")).unwrap(), b"changed");
		assert!(!workspace.join("out/gen/stale.rs").exists(), "extras must be pruned");
		assert_eq!(std::fs::read(workspace.join("out/gen/b.rs")).unwrap(), b"b");
		let _ = std::fs::remove_dir_all(&base);
	}

	#[test]
	fn two_keys_sharing_one_path_converge_without_rewriting() {
		let base = scratch("shared");
		let workspace = base.join("ws");
		let first = base.join("first");
		let second = base.join("second");
		std::fs::create_dir_all(&first).unwrap();
		std::fs::create_dir_all(&second).unwrap();
		std::fs::write(first.join("script"), b"binary").unwrap();
		std::fs::write(second.join("script"), b"binary").unwrap();
		let materialized = Materialized::load(&base.join("out"));
		let rel = Path::new("out/script");

		assert!(!materialized.confirm(&workspace, rel, &first.join("script"), "compile"));
		std::fs::create_dir_all(workspace.join("out")).unwrap();
		std::fs::copy(first.join("script"), workspace.join("out/script")).unwrap();
		assert!(materialized.confirm(&workspace, rel, &first.join("script"), "compile"));
		assert!(materialized.confirm(&workspace, rel, &second.join("script"), "run"));
		let mtime = std::fs::metadata(workspace.join("out/script")).unwrap().modified().unwrap();
		assert!(materialized.confirm(&workspace, rel, &first.join("script"), "compile"));
		assert!(materialized.confirm(&workspace, rel, &second.join("script"), "run"));
		assert_eq!(
			std::fs::metadata(workspace.join("out/script")).unwrap().modified().unwrap(),
			mtime,
			"identical bytes must never be recopied"
		);
		assert!(!materialized.confirm(&workspace, rel, &second.join("missing"), "other"));

		std::fs::write(workspace.join("out/script"), b"BINARX").unwrap();
		assert!(!materialized.confirm(&workspace, rel, &second.join("script"), "run"));
		let _ = std::fs::remove_dir_all(&base);
	}
}
