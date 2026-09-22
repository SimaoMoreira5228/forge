use std::path::{Path, PathBuf};

use forge_core::resolver::ForgeLock;
use forge_diagnostics::{ForgeDiagnostic, codes};

use crate::store::Store;
use crate::toolchain::sync::{extract, fetch_blob};

const MARKER: &str = ".forge-source";

#[derive(Debug, Clone)]
pub struct SourcePackage {
	pub name: String,
	pub version: String,
	pub url: String,
	pub sha256: String,
	pub git_rev: Option<String>,
}

pub struct SourceStore {
	store: Store,
	workspace: PathBuf,
	mirrors: Vec<(String, String)>,
	offline: bool,
}

impl SourceStore {
	pub fn open(workspace: &Path, mirrors: Vec<(String, String)>, offline: bool) -> Self {
		Self::with_store(workspace, Store::open(), mirrors, offline)
	}

	pub fn with_store(workspace: &Path, store: Store, mirrors: Vec<(String, String)>, offline: bool) -> Self {
		Self {
			store,
			workspace: workspace.to_path_buf(),
			mirrors,
			offline,
		}
	}

	fn mirror(&self, url: &str) -> String {
		mirror_url(url, &self.mirrors)
	}

	fn canonical(&self, package: &SourcePackage) -> Result<PathBuf, ForgeDiagnostic> {
		let dir = self.store.sources().join(&package.name).join(&package.version);
		if dir.join(MARKER).is_file() {
			return Ok(dir);
		}
		let url = self.mirror(&package.url);
		if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
			eprintln!(
				"\x1b[36m  Fetching\x1b[0m {}@{} \x1b[2m{}\x1b[0m",
				package.name, package.version, url
			);
		}
		let archive = fetch_blob(&self.store, &url, Some(&package.sha256), self.offline)?;
		let staging = self.store.staging(&format!("src-{}", package.name));
		std::fs::create_dir_all(&staging).map_err(|e| io_error("create", &staging, e))?;
		extract(&archive, &staging).inspect_err(|_e| {
			let _ = std::fs::remove_dir_all(&staging);
		})?;
		normalize_root(&staging)?;
		std::fs::write(staging.join(MARKER), b"1").map_err(|e| io_error("mark", &staging, e))?;
		{
			let _publish = self.store.lock("store")?;
			self.store.publish_dir(&staging, &dir)?;
		}
		Ok(dir)
	}

	pub fn fetch(&self, package: &SourcePackage) -> Result<PathBuf, ForgeDiagnostic> {
		let canonical = match package.git_rev.clone() {
			Some(revision) => self.canonical_git(package, &revision)?,
			None => self.canonical(package)?,
		};
		self.materialize(package, &canonical)
	}

	fn materialize(&self, package: &SourcePackage, canonical: &Path) -> Result<PathBuf, ForgeDiagnostic> {
		let view = self
			.workspace
			.join("forge-out/deps")
			.join(&package.name)
			.join(&package.version);
		let canonical_id = canonical.to_string_lossy();
		if std::fs::read_to_string(view.join(MARKER)).is_ok_and(|marker| marker == canonical_id) {
			return Ok(view);
		}
		let parent = self.workspace.join("forge-out/deps");
		std::fs::create_dir_all(&parent).map_err(|e| io_error("create", &parent, e))?;
		let staging = parent.join(format!(
			".tmp-src-{}-{}-{}",
			package.name,
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.map(|d| d.as_nanos())
				.unwrap_or(0)
		));
		hardlink_tree(canonical, &staging).inspect_err(|_e| {
			let _ = std::fs::remove_dir_all(&staging);
		})?;
		std::fs::write(staging.join(MARKER), canonical_id.as_bytes()).map_err(|e| io_error("mark", &staging, e))?;
		self.store.publish_dir(&staging, &view)?;
		Ok(view)
	}

	fn canonical_git(&self, package: &SourcePackage, revision: &str) -> Result<PathBuf, ForgeDiagnostic> {
		let digest = blake3::hash(format!("{}@{}", package.url, revision).as_bytes())
			.to_hex()
			.to_string();
		let dir = self.store.sources().join("git").join(digest);
		if dir.join(MARKER).is_file() {
			return Ok(dir);
		}
		if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
			eprintln!(
				"\x1b[36m  Fetching\x1b[0m {}@{} \x1b[2m{}#{}\x1b[0m",
				package.name, package.version, package.url, revision
			);
		}
		let staging = self.store.staging(&format!("git-{}", package.name));
		std::fs::create_dir_all(&staging).map_err(|e| io_error("create", &staging, e))?;
		git(&["init", "--quiet"], &staging)?;
		git(&["remote", "add", "origin", &package.url], &staging)?;
		git(&["fetch", "--quiet", "origin", revision], &staging)?;
		git(&["checkout", "--quiet", "FETCH_HEAD"], &staging)?;
		let _ = std::fs::remove_dir_all(staging.join(".git"));
		std::fs::write(staging.join(MARKER), b"1").map_err(|e| io_error("mark", &staging, e))?;
		{
			let _publish = self.store.lock("store")?;
			self.store.publish_dir(&staging, &dir)?;
		}
		Ok(dir)
	}

	pub fn fetch_lock(&self, lock: &ForgeLock) -> Result<Vec<(String, PathBuf)>, ForgeDiagnostic> {
		lock.sources()
			.map_err(|e| ForgeDiagnostic::error(101, e))?
			.into_iter()
			.map(|source| {
				let name = format!("{}@{}", source.name, source.version);
				let path = self.fetch(&SourcePackage {
					name: source.name,
					version: source.version,
					url: source.url,
					sha256: source.checksum,
					git_rev: None,
				})?;
				Ok((name, path))
			})
			.collect()
	}
}

fn git(args: &[&str], cwd: &Path) -> Result<(), ForgeDiagnostic> {
	let output = std::process::Command::new("git")
		.args(args)
		.current_dir(cwd)
		.output()
		.map_err(|e| io_error("run git", cwd, e))?;
	if output.status.success() {
		return Ok(());
	}
	Err(ForgeDiagnostic::error(
		codes::inputs::MISSING_INPUT,
		format!(
			"git {} failed: {}",
			args.join(" "),
			String::from_utf8_lossy(&output.stderr).trim()
		),
	))
}

fn mirror_url(url: &str, mirrors: &[(String, String)]) -> String {
	mirrors
		.iter()
		.find(|(prefix, _)| url.starts_with(prefix.as_str()))
		.map_or_else(
			|| url.to_string(),
			|(prefix, mirror)| format!("{mirror}{}", &url[prefix.len()..]),
		)
}

fn hardlink_tree(from: &Path, to: &Path) -> Result<(), ForgeDiagnostic> {
	std::fs::create_dir_all(to).map_err(|e| io_error("create", to, e))?;
	for entry in walkdir::WalkDir::new(from).min_depth(1) {
		let entry = entry.map_err(|e| {
			ForgeDiagnostic::error(codes::hermetic::HERMETIC_VIOLATION, format!("walk `{}`: {e}", from.display()))
		})?;
		let relative = entry.path().strip_prefix(from).expect("prefix walked");
		let destination = to.join(relative);
		if entry.file_type().is_dir() {
			std::fs::create_dir_all(&destination).map_err(|e| io_error("create", &destination, e))?;
		} else if std::fs::hard_link(entry.path(), &destination).is_err() {
			std::fs::copy(entry.path(), &destination)
				.map(|_| ())
				.map_err(|e| io_error("copy", entry.path(), e))?;
		}
	}
	Ok(())
}

fn normalize_root(destination: &Path) -> Result<(), ForgeDiagnostic> {
	let mut children = std::fs::read_dir(destination)
		.map_err(|e| io_error("read", destination, e))?
		.flatten()
		.collect::<Vec<_>>();
	if children.len() != 1 || !children[0].path().is_dir() {
		return Ok(());
	}
	let root = children.pop().expect("one child").path();
	for entry in std::fs::read_dir(&root).map_err(|e| io_error("read", &root, e))?.flatten() {
		std::fs::rename(entry.path(), destination.join(entry.file_name()))
			.map_err(|e| io_error("normalize", destination, e))?;
	}
	std::fs::remove_dir(&root).map_err(|e| io_error("remove", &root, e))
}

fn io_error(stage: &str, path: &Path, error: std::io::Error) -> ForgeDiagnostic {
	ForgeDiagnostic::error(
		codes::hermetic::HERMETIC_VIOLATION,
		format!("{stage} `{}`: {error}", path.display()),
	)
}

#[cfg(test)]
mod tests {
	use flate2::write::GzEncoder;
	use tar::Builder;

	use super::*;
	use crate::toolchain::sync::file_sha256;

	fn archive(path: &Path, contents: &[u8]) {
		let file = std::fs::File::create(path).unwrap();
		let encoder = GzEncoder::new(file, flate2::Compression::fast());
		let mut builder = Builder::new(encoder);
		let mut header = tar::Header::new_gnu();
		header.set_size(contents.len() as u64);
		header.set_cksum();
		builder.append_data(&mut header, "demo-1.0.0/src/lib.rs", contents).unwrap();
		builder.into_inner().unwrap().finish().unwrap();
	}

	fn fixture(name: &str) -> (PathBuf, PathBuf, PathBuf) {
		let base = std::env::temp_dir().join(format!("forge-source-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&base);
		std::fs::create_dir_all(&base).unwrap();
		(base.join("store"), base.join("ws"), base.join("fixture.tar.gz"))
	}

	#[test]
	fn extracts_verified_archive_then_materializes_into_workspace() {
		let (store_root, ws, archive_path) = fixture("ok");
		archive(&archive_path, b"pub fn hello() {}");
		let digest = file_sha256(&archive_path).unwrap();
		std::fs::create_dir_all(store_root.join("blobs")).unwrap();
		std::fs::copy(&archive_path, store_root.join("blobs").join(&digest)).unwrap();

		let store = SourceStore::with_store(&ws, Store::at(&store_root), Vec::new(), false);
		let package = SourcePackage {
			name: "demo".into(),
			version: "1.0.0".into(),
			url: "https://example.invalid/demo.tar.gz".into(),
			sha256: digest.clone(),
			git_rev: None,
		};
		let path = store.fetch(&package).unwrap();
		assert!(
			path.starts_with(&ws),
			"materialized view must live in the workspace: {}",
			path.display()
		);
		assert!(path.join("src/lib.rs").is_file());
		assert!(path.join(".forge-source").is_file());
		assert!(store_root.join("sources/demo/1.0.0/.forge-source").is_file());
		assert_eq!(store.fetch(&package).unwrap(), path);

		let lock = ForgeLock {
			version: 1,
			packages: vec![forge_core::resolver::LockedPackage {
				name: "demo".into(),
				version: "1.0.0".into(),
				source: Some("https://example.invalid/demo.tar.gz".into()),
				checksum: Some(digest),
				dependencies: Vec::new(),
			}],
		};
		assert_eq!(store.fetch_lock(&lock).unwrap().len(), 1);
	}

	#[test]
	fn corrupt_blob_is_rejected_without_marking_source() {
		let (store_root, ws, _archive_path) = fixture("bad");
		let digest = "0".repeat(64);
		std::fs::create_dir_all(store_root.join("blobs")).unwrap();
		std::fs::write(store_root.join("blobs").join(&digest), b"not an archive").unwrap();

		let store = SourceStore::with_store(&ws, Store::at(&store_root), Vec::new(), false);
		let package = SourcePackage {
			name: "demo".into(),
			version: "1.0.0".into(),
			url: "https://example.invalid/demo.tar.gz".into(),
			sha256: digest,
			git_rev: None,
		};
		assert!(store.fetch(&package).is_err());
		assert!(!store_root.join("sources/demo/1.0.0/.forge-source").exists());
		assert!(!ws.join("forge-out/deps/demo/1.0.0").exists());
	}

	#[test]
	fn mirrors_rewrite_the_longest_matching_prefix() {
		let mirrors = vec![
			("https://crates.io/api/v1".to_string(), "https://mirror.corp/api".to_string()),
			("https://crates.io".to_string(), "https://mirror.corp".to_string()),
		];
		assert_eq!(
			mirror_url("https://crates.io/api/v1/crates/foo/1.0.0/download", &mirrors),
			"https://mirror.corp/api/crates/foo/1.0.0/download"
		);
		assert_eq!(mirror_url("https://elsewhere/x", &mirrors), "https://elsewhere/x");
	}
}
