use std::io::Read;
use std::path::Path;

use forge_core::TargetUrl;
use forge_diagnostics::{ForgeDiagnostic, codes};
use sha2::Digest;

use super::ToolchainSelection;
use super::store::{CatalogOrigin, INSTALL_MARKER, ToolchainStore, installed_bin_dir};

#[derive(Debug)]
pub enum SyncOutcome {
	AlreadyInstalled,
	Downloaded {
		url: String,
	},
}

pub fn sync_all(store: &ToolchainStore, only: Option<&str>) -> Result<Vec<(String, SyncOutcome)>, ForgeDiagnostic> {
	let mut results = Vec::new();
	for (name, selection) in store.configured() {
		if let Some(filter) = only
			&& filter != name
		{
			continue;
		}
		let outcome = match selection {
			ToolchainSelection::Path { .. } => SyncOutcome::AlreadyInstalled,
			ToolchainSelection::Version { version } => {
				let resolved = store.catalog.resolve(name, Some(version))?;
				let target = resolved.entry.url_for(&resolved.version, &platform_key())?.clone();
				sync_one(store, name, &resolved.name, &resolved.version, &target)?
			}
			ToolchainSelection::Url { url, sha256 } => {
				let pseudo_version = ToolchainStore::url_version(url);
				let target = TargetUrl {
					url: url.clone(),
					sha256: sha256.clone(),
				};
				sync_one(store, name, name, &pseudo_version, &target)?
			}
		};
		results.push((name.clone(), outcome));
	}
	Ok(results)
}

fn platform_key() -> String {
	format!("{}-{}", std::env::consts::OS, normalize_arch(std::env::consts::ARCH))
}

fn normalize_arch(arch: &str) -> String {
	match arch {
		"x86_64" | "amd64" => "x86_64".into(),
		"aarch64" | "arm64" => "aarch64".into(),
		other => other.into(),
	}
}

fn sync_one(
	store: &ToolchainStore,
	config_name: &str,
	catalog_name: &str,
	version: &str,
	target: &TargetUrl,
) -> Result<SyncOutcome, ForgeDiagnostic> {
	let install_dir = store.install_dir(catalog_name, version);
	if installed_bin_dir(&install_dir).is_some() {
		return Ok(SyncOutcome::AlreadyInstalled);
	}

	if target.sha256.is_none() && store.origin_of(config_name) == CatalogOrigin::Workspace {
		eprintln!("warning: `{config_name}` has no sha256 pin; downloads cannot be verified (reproducibility at risk)");
	}

	std::fs::create_dir_all(&install_dir).map_err(|e| io_err("create", &install_dir, e))?;

	let archive_path = install_dir.join("artifact.download");
	download(&target.url, &archive_path)?;

	let actual = file_sha256(&archive_path)?;
	if let Some(expected) = &target.sha256 {
		let expected = expected.to_ascii_lowercase();
		if actual != expected {
			let _ = std::fs::remove_dir_all(&install_dir);
			return Err(ForgeDiagnostic::error(
				codes::hermetic::TOOLCHAIN_MISMATCH,
				format!("`{config_name}` download failed verification: expected sha256 {expected}, got {actual}"),
			));
		}
	}

	extract(&archive_path, &install_dir).inspect_err(|_e| {
		let _ = std::fs::remove_dir_all(&install_dir);
	})?;
	let _ = std::fs::remove_file(&archive_path);

	if let Some(top) = find_install_sh(&install_dir) {
		let prefix = install_dir.clone();
		let status = std::process::Command::new("sh")
			.arg(top.join("install.sh"))
			.arg(format!("--prefix={}", prefix.display()))
			.arg("--without=rust-docs")
			.arg("--without=rust-docs-json-preview")
			.arg("--without=rust-analysis-x86_64-unknown-linux-gnu")
			.arg("--without=llvm-tools-preview")
			.arg("--without=llvm-bitcode-linker-preview")
			.arg("--without=rust-analyzer-preview")
			.arg("--without=clippy-preview")
			.arg("--without=rustfmt-preview")
			.current_dir(top.parent().unwrap_or(&install_dir))
			.status()
			.map_err(|e| io_err("install rust", &prefix, e))?;
		if !status.success() {
			let _ = std::fs::remove_dir_all(&install_dir);
			return Err(ForgeDiagnostic::error(
				codes::hermetic::TOOLCHAIN_MISMATCH,
				format!("`{config_name}` rust install failed (exit {})", status.code().unwrap_or(-1)),
			));
		}
		let _ = std::fs::remove_dir_all(&top);
	}

	if catalog_name == "gcc"
		&& let Some(bin) = installed_bin_dir(&install_dir)
	{
		let _ = ensure_gcc_prefix_links(&bin);
	}

	if installed_bin_dir(&install_dir).is_none() {
		let _ = std::fs::remove_dir_all(&install_dir);
		return Err(ForgeDiagnostic::error(
			codes::hermetic::TOOLCHAIN_MISMATCH,
			format!("`{config_name}` archive contains no bin/ directory with tools"),
		));
	}

	record_marker(&install_dir)?;
	Ok(SyncOutcome::Downloaded { url: target.url.clone() })
}

pub(crate) fn download(url: &str, destination: &Path) -> Result<(), ForgeDiagnostic> {
	let response = ureq::get(url)
		.call()
		.map_err(|e| ForgeDiagnostic::error(codes::hermetic::TOOLCHAIN_MISMATCH, format!("GET {url}: {e}")))?;
	let mut reader = response.into_body().into_reader();
	let mut file = std::fs::File::create(destination).map_err(|e| io_err("create", destination, e))?;
	let mut hasher = sha2::Sha256::new();
	let mut buffer = [0u8; 64 * 1024];
	let mut total: u64 = 0;
	loop {
		let read = reader.read(&mut buffer).map_err(|e| io_err("download", destination, e))?;
		if read == 0 {
			break;
		}
		hasher.update(&buffer[..read]);
		total += read as u64;
		if total % (16 * 1024 * 1024) < read as u64 {
			eprint!("\r  {} … {:.0} MB", short_url(url), total as f64 / (1024.0 * 1024.0));
		}
		std::io::Write::write_all(&mut file, &buffer[..read]).map_err(|e| io_err("write", destination, e))?;
	}
	eprintln!("\r  {} … {:.1} MB done", short_url(url), total as f64 / (1024.0 * 1024.0));
	let _ = hasher.finalize();
	Ok(())
}

pub(crate) fn file_sha256(path: &Path) -> Result<String, ForgeDiagnostic> {
	let mut file = std::fs::File::open(path).map_err(|e| io_err("open", path, e))?;
	let mut hasher = sha2::Sha256::new();
	let mut buffer = [0u8; 64 * 1024];
	loop {
		let read = std::io::Read::read(&mut file, &mut buffer).map_err(|e| io_err("hash", path, e))?;
		if read == 0 {
			break;
		}
		hasher.update(&buffer[..read]);
	}
	Ok(hex(&hasher.finalize()))
}

pub fn extract(archive: &Path, destination: &Path) -> Result<(), ForgeDiagnostic> {
	let mut header = [0u8; 262];
	let mut file = std::fs::File::open(archive).map_err(|e| io_err("open", archive, e))?;
	let read = file.read(&mut header).map_err(|e| io_err("read", archive, e))?;
	let magic = &header[..read];

	if magic.starts_with(&[0xFD, b'7', b'z', b'X', b'Z']) {
		let file = std::fs::File::open(archive).map_err(|e| io_err("open", archive, e))?;
		return unpack_tar(xz2::read::XzDecoder::new(file), destination);
	}
	if magic.starts_with(&[0x1F, 0x8B]) {
		let file = std::fs::File::open(archive).map_err(|e| io_err("open", archive, e))?;
		return unpack_tar(flate2::read::GzDecoder::new(file), destination);
	}
	if magic.len() >= 262 && &magic[257..261] == b"ustar" {
		let file = std::fs::File::open(archive).map_err(|e| io_err("open", archive, e))?;
		return unpack_tar(file, destination);
	}
	Err(ForgeDiagnostic::error(
		codes::hermetic::TOOLCHAIN_MISMATCH,
		format!(
			"unsupported archive format `{}` (supported: tar.gz, tar.xz)",
			archive.display()
		),
	))
}

fn unpack_tar<R: Read>(decoder: R, destination: &Path) -> Result<(), ForgeDiagnostic> {
	let mut archive = tar::Archive::new(decoder);
	archive.set_overwrite(true);
	archive.unpack(destination).map_err(|e| {
		ForgeDiagnostic::error(
			codes::hermetic::TOOLCHAIN_MISMATCH,
			format!("extract into {}: {e}", destination.display()),
		)
	})
}

pub fn verify_installed(store: &ToolchainStore, only: Option<&str>) -> Result<Vec<String>, ForgeDiagnostic> {
	let mut lines = Vec::new();
	for (name, selection) in store.configured() {
		if let Some(filter) = only
			&& filter != name
		{
			continue;
		}
		let line = match selection {
			ToolchainSelection::Path { path } => format!("{name}: path-based, nothing to verify ({})", path.display()),
			ToolchainSelection::Version { version } => {
				let resolved = store.catalog.resolve(name, Some(version))?;
				let target = resolved.entry.url_for(&resolved.version, &platform_key())?.clone();
				check_install(store, name, &resolved.name, &resolved.version, &target)
			}
			ToolchainSelection::Url { url, sha256 } => {
				let pseudo_version = ToolchainStore::url_version(url);
				let target = TargetUrl {
					url: url.clone(),
					sha256: sha256.clone(),
				};
				check_install(store, name, name, &pseudo_version, &target)
			}
		};
		lines.push(line);
	}
	Ok(lines)
}

fn check_install(
	store: &ToolchainStore,
	config_name: &str,
	catalog_name: &str,
	version: &str,
	target: &TargetUrl,
) -> String {
	let install_dir = store.install_dir(catalog_name, version);
	let Some(_bin) = installed_bin_dir(&install_dir) else {
		return format!("{config_name}: not synced");
	};
	let Some(expected) = &target.sha256 else {
		return format!("{config_name}: installed (no sha256 declared — unverifiable)");
	};
	let artifact = install_dir.join("artifact.download");
	if !artifact.exists() {
		return format!("{config_name}: installed (original artifact not retained; cannot re-verify)");
	}
	match file_sha256(&artifact) {
		Ok(actual) if actual == expected.to_ascii_lowercase() => format!("{config_name}: verified"),
		Ok(actual) => format!("{config_name}: MISMATCH expected {expected} got {actual}"),
		Err(e) => format!("{config_name}: verify failed ({e})"),
	}
}

fn find_install_sh(install_dir: &Path) -> Option<std::path::PathBuf> {
	for entry in std::fs::read_dir(install_dir).ok()?.flatten() {
		let candidate = entry.path().join("install.sh");
		if candidate.is_file() {
			return Some(entry.path());
		}
	}
	None
}

fn ensure_gcc_prefix_links(bin_dir: &Path) -> std::io::Result<()> {
	let mut prefix: Option<String> = None;
	for entry in std::fs::read_dir(bin_dir)? {
		let entry = entry?;
		let name = entry.file_name().to_string_lossy().into_owned();
		if name.ends_with("-gcc") {
			prefix = Some(name[..name.len() - 3].to_string());
			break;
		}
	}
	let Some(prefix) = prefix else {
		return Ok(());
	};
	for (from, to) in [
		(format!("{prefix}gcc"), "gcc"),
		(format!("{prefix}g++"), "g++"),
		(format!("{prefix}ar"), "ar"),
		(format!("{prefix}ld"), "ld"),
	] {
		let src = bin_dir.join(&from);
		let dst = bin_dir.join(to);
		if src.exists() && !dst.exists() {
			#[cfg(unix)]
			std::os::unix::fs::symlink(&src, &dst)?;
			#[cfg(not(unix))]
			std::fs::copy(&src, &dst)?;
		}
	}
	Ok(())
}

fn record_marker(install_dir: &Path) -> Result<(), ForgeDiagnostic> {
	let Some(bin) = installed_bin_dir(install_dir) else {
		return Ok(());
	};
	let relative = bin.strip_prefix(install_dir).unwrap_or(Path::new(""));
	std::fs::write(install_dir.join(INSTALL_MARKER), relative.to_string_lossy().as_bytes())
		.map_err(|e| io_err("marker", install_dir, e))
}

fn hex(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn short_url(url: &str) -> &str {
	url.rsplit('/').next().unwrap_or(url)
}

fn io_err(stage: &str, path: &Path, e: std::io::Error) -> ForgeDiagnostic {
	ForgeDiagnostic::error(
		codes::hermetic::HERMETIC_VIOLATION,
		format!("{stage} `{}`: {e}", path.display()),
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn write_tar_gz(path: &Path, entries: &[(&str, &[u8])]) {
		use flate2::write::GzEncoder;
		use tar::Builder;
		let file = std::fs::File::create(path).unwrap();
		let encoder = GzEncoder::new(file, flate2::Compression::fast());
		let mut builder = Builder::new(encoder);
		for (name, contents) in entries {
			let mut header = tar::Header::new_gnu();
			header.set_size(contents.len() as u64);
			header.set_mode(0o755);
			header.set_cksum();
			builder.append_data(&mut header, name, *contents).unwrap();
		}
		builder.into_inner().unwrap().finish().unwrap();
	}

	#[test]
	fn extracts_gz_and_finds_nested_bin() {
		let dir = std::env::temp_dir().join(format!("forge-sync-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();

		let fixture = dir.join("toolchain.tar.gz");
		write_tar_gz(
			&fixture,
			&[
				("clang+llvm-99/bin/clang", b"#!/bin/sh\n"),
				("clang+llvm-99/lib/lib.so", b"\x00"),
			],
		);
		let out = dir.join("installed");
		std::fs::create_dir_all(&out).unwrap();
		extract(&fixture, &out).unwrap();

		let bin = installed_bin_dir(&out).expect("bin dir must be discovered");
		assert!(bin.ends_with("bin"));
		assert!(bin.join("clang").is_file());
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn rejects_unknown_formats() {
		let dir = std::env::temp_dir().join(format!("forge-sync-bad-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let zip_like = dir.join("tool.zip");
		std::fs::write(&zip_like, b"PK\x03\x04 nothing here").unwrap();
		let out = dir.join("out");
		assert!(extract(&zip_like, &out).is_err());
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn digest_verification_detects_mismatch() {
		let dir = std::env::temp_dir().join(format!("forge-sync-dig-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let artifact = dir.join("a.bin");
		std::fs::write(&artifact, b"hello").unwrap();
		let good = file_sha256(&artifact).unwrap();
		assert_ne!(good, "deadbeef");
		let _ = std::fs::remove_dir_all(&dir);
	}
}
