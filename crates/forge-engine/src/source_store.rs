use std::path::{Path, PathBuf};

use forge_core::resolver::ForgeLock;
use forge_diagnostics::{ForgeDiagnostic, codes};

use crate::toolchain::sync::{extract, file_sha256};

#[derive(Debug, Clone)]
pub struct SourcePackage {
	pub name: String,
	pub version: String,
	pub url: String,
	pub sha256: String,
}

pub struct SourceStore {
	root: PathBuf,
}

impl SourceStore {
	pub fn new(workspace: &Path) -> Self {
		Self {
			root: workspace.join(".forge/deps"),
		}
	}

	pub fn fetch(&self, package: &SourcePackage) -> Result<PathBuf, ForgeDiagnostic> {
		let destination = self.root.join(&package.name).join(&package.version);
		let marker = destination.join(".forge-source");
		if marker.is_file() {
			return Ok(destination);
		}
		if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
			eprintln!(
				"\x1b[36m  Fetching\x1b[0m {}@{} \x1b[2m{}\x1b[0m",
				package.name, package.version, package.url
			);
		}

		std::fs::create_dir_all(&destination).map_err(|e| io_error("create", &destination, e))?;
		let archive = destination.join("artifact.download");
		if !archive.is_file() {
			crate::toolchain::sync::download(&package.url, &archive)?;
		}
		let actual = file_sha256(&archive)?;
		if actual != package.sha256.to_ascii_lowercase() {
			let _ = std::fs::remove_dir_all(&destination);
			return Err(ForgeDiagnostic::error(
				codes::hermetic::TOOLCHAIN_MISMATCH,
				format!(
					"{}@{} checksum mismatch: expected {}, got {actual}",
					package.name, package.version, package.sha256
				),
			));
		}
		extract(&archive, &destination)?;
		normalize_root(&destination)?;
		std::fs::remove_file(&archive).map_err(|e| io_error("remove", &archive, e))?;
		std::fs::write(&marker, b"1").map_err(|e| io_error("mark", &marker, e))?;
		Ok(destination)
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
				})?;
				Ok((name, path))
			})
			.collect()
	}
}

fn normalize_root(destination: &Path) -> Result<(), ForgeDiagnostic> {
	let mut children = std::fs::read_dir(destination)
		.map_err(|e| io_error("read", destination, e))?
		.flatten()
		.filter(|entry| entry.file_name() != "artifact.download")
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

	fn archive(path: &Path) {
		let file = std::fs::File::create(path).unwrap();
		let encoder = GzEncoder::new(file, flate2::Compression::fast());
		let mut builder = Builder::new(encoder);
		let contents = b"pub fn hello() {}";
		let mut header = tar::Header::new_gnu();
		header.set_size(contents.len() as u64);
		header.set_cksum();
		builder
			.append_data(&mut header, "demo-1.0.0/src/lib.rs", contents.as_slice())
			.unwrap();
		builder.into_inner().unwrap().finish().unwrap();
	}

	#[test]
	fn fetches_verified_archive_and_reuses_marker() {
		let root = std::env::temp_dir().join(format!("forge-source-ok-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let archive_path = root.join("fixture.tar.gz");
		std::fs::create_dir_all(&root).unwrap();
		archive(&archive_path);
		let digest = file_sha256(&archive_path).unwrap();
		let destination = root.join(".forge/deps/demo/1.0.0");
		std::fs::create_dir_all(&destination).unwrap();
		std::fs::copy(&archive_path, destination.join("artifact.download")).unwrap();
		let store = SourceStore::new(&root);
		let package = SourcePackage {
			name: "demo".into(),
			version: "1.0.0".into(),
			url: "file://unused".into(),
			sha256: digest,
		};
		let path = store.fetch(&package).unwrap();
		assert!(path.join("src/lib.rs").is_file());
		assert!(path.join(".forge-source").is_file());
		assert_eq!(store.fetch(&package).unwrap(), path);
		let lock = ForgeLock {
			version: 1,
			packages: vec![forge_core::resolver::LockedPackage {
				name: "demo".into(),
				version: "1.0.0".into(),
				source: Some("file://unused".into()),
				checksum: Some(package.sha256),
				dependencies: Vec::new(),
			}],
		};
		assert_eq!(store.fetch_lock(&lock).unwrap().len(), 1);
		let _ = std::fs::remove_dir_all(root);
	}

	#[test]
	fn rejects_wrong_checksum_before_marking_source() {
		let root = std::env::temp_dir().join(format!("forge-source-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let store = SourceStore::new(&root);
		let package = SourcePackage {
			name: "demo".into(),
			version: "1.0.0".into(),
			url: "file://unused".into(),
			sha256: "deadbeef".into(),
		};
		let _ = std::fs::create_dir_all(root.join(".forge/deps/demo/1.0.0"));
		std::fs::write(root.join(".forge/deps/demo/1.0.0/artifact.download"), b"bad").unwrap();
		assert!(store.fetch(&package).is_err());
		assert!(!root.join(".forge/deps/demo/1.0.0/.forge-source").exists());
		let _ = std::fs::remove_dir_all(root);
	}
}
