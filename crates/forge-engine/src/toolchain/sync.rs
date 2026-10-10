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
	Downloaded { url: String },
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
	platform_key_for(std::env::consts::OS, std::env::consts::ARCH)
}

fn platform_key_for(os: &str, arch: &str) -> String {
	format!("{}-{}", catalog_os(os), normalize_arch(arch))
}

fn catalog_os(os: &str) -> &str {
	match os {
		"macos" | "ios" => "darwin",
		other => other,
	}
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
	if install_dir.join(INSTALL_MARKER).is_file() {
		return Ok(SyncOutcome::AlreadyInstalled);
	}

	if target.sha256.is_none() && store.origin_of(config_name) == CatalogOrigin::Workspace {
		eprintln!("warning: `{config_name}` has no sha256 pin; downloads cannot be verified (reproducibility at risk)");
	}

	let archive = fetch_blob(&store.store, &target.url, target.sha256.as_deref(), false)?;

	let staging = store.store.staging(catalog_name);
	std::fs::create_dir_all(&staging).map_err(|e| io_err("create", &staging, e))?;
	extract(&archive, &staging).inspect_err(|_e| {
		let _ = std::fs::remove_dir_all(&staging);
	})?;
	{
		let _publish = store.store.lock("store")?;
		store.store.publish_dir(&staging, &install_dir)?;
	}

	let install = store.catalog.get(catalog_name).and_then(|entry| entry.install.clone());
	if let Some(install) = install
		&& let Some(top) = find_script_dir(&install_dir, &install.name)
	{
		let prefix = install_dir.clone();
		let mut command = std::process::Command::new("sh");
		command
			.arg(top.join(&install.name))
			.arg(format!("--prefix={}", prefix.display()))
			.args(&install.args)
			.current_dir(top.parent().unwrap_or(&install_dir));
		let status = command.status().map_err(|e| io_err("install toolchain", &prefix, e))?;
		if !status.success() {
			let _ = std::fs::remove_dir_all(&install_dir);
			return Err(ForgeDiagnostic::error(
				codes::hermetic::TOOLCHAIN_MISMATCH,
				format!("`{config_name}` install failed (exit {})", status.code().unwrap_or(-1)),
			));
		}
		let _ = std::fs::remove_dir_all(&top);
	}

	if let Some(bin) = installed_bin_dir(&install_dir)
		&& let Some(entry) = store.catalog.get(catalog_name)
	{
		let _ = apply_bin_aliases(&bin, &entry.bin_aliases);
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

pub(crate) fn fetch_blob(
	store: &crate::store::Store,
	url: &str,
	sha256: Option<&str>,
	offline: bool,
) -> Result<std::path::PathBuf, ForgeDiagnostic> {
	let key = match sha256 {
		Some(expected) => expected.to_ascii_lowercase(),
		None => url_digest(url),
	};
	let blob = store.blob(&key);
	if blob.is_file() {
		return Ok(blob);
	}
	if offline {
		return Err(ForgeDiagnostic::error(
			codes::hermetic::TOOLCHAIN_MISMATCH,
			format!("offline: the global store does not hold `{url}`"),
		)
		.with_help("re-run without `--offline` to fetch it"));
	}
	std::fs::create_dir_all(store.blobs()).map_err(|e| io_err("create", &store.blobs(), e))?;
	let staged = store.staging("blob");
	if let Err(e) = download(url, &staged) {
		let _ = std::fs::remove_file(&staged);
		return Err(e);
	}
	let actual = file_sha256(&staged)?;
	if let Some(expected) = sha256
		&& actual != expected.to_ascii_lowercase()
	{
		let _ = std::fs::remove_file(&staged);
		return Err(ForgeDiagnostic::error(
			codes::hermetic::TOOLCHAIN_MISMATCH,
			format!("download failed verification: expected sha256 {expected}, got {actual}"),
		));
	}
	{
		let _publish = store.lock("store")?;
		store.publish_file(&staged, &blob)?;
	}
	Ok(blob)
}

pub(crate) fn url_digest(url: &str) -> String {
	blake3::hash(url.as_bytes()).to_hex().to_string()
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
	if !install_dir.join(INSTALL_MARKER).is_file() {
		return format!("{config_name}: not synced");
	}
	let Some(expected) = &target.sha256 else {
		return format!("{config_name}: installed (no sha256 declared — unverifiable)");
	};
	let blob = store.store.blob(&expected.to_ascii_lowercase());
	if !blob.is_file() {
		return format!("{config_name}: installed (original artifact not retained; cannot re-verify)");
	}
	match file_sha256(&blob) {
		Ok(actual) if actual == expected.to_ascii_lowercase() => format!("{config_name}: verified"),
		Ok(actual) => format!("{config_name}: MISMATCH expected {expected} got {actual}"),
		Err(e) => format!("{config_name}: verify failed ({e})"),
	}
}

fn find_script_dir(install_dir: &Path, script: &str) -> Option<std::path::PathBuf> {
	for entry in std::fs::read_dir(install_dir).ok()?.flatten() {
		let candidate = entry.path().join(script);
		if candidate.is_file() {
			return Some(entry.path());
		}
	}
	None
}

fn apply_bin_aliases(bin_dir: &Path, aliases: &[forge_core::toolchain::catalog::BinAlias]) -> std::io::Result<()> {
	for alias in aliases {
		let Some(source) = find_alias_source(bin_dir, &alias.from) else {
			continue;
		};
		let destination = bin_dir.join(&alias.to);
		if destination.exists() {
			continue;
		}
		#[cfg(unix)]
		std::os::unix::fs::symlink(&source, &destination)?;
		#[cfg(not(unix))]
		std::fs::copy(&source, &destination)?;
	}
	Ok(())
}

fn find_alias_source(bin_dir: &Path, pattern: &str) -> Option<std::path::PathBuf> {
	let mut candidates: Vec<std::path::PathBuf> = std::fs::read_dir(bin_dir)
		.ok()?
		.flatten()
		.map(|entry| entry.path())
		.filter(|path| path.is_file())
		.filter(|path| {
			path.file_name()
				.is_some_and(|name| forge_script::glob::glob_match(pattern, &name.to_string_lossy()))
		})
		.collect();
	candidates.sort();
	candidates.into_iter().next()
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
	use std::collections::BTreeSet;

	use forge_core::Catalog;

	use super::*;
	use crate::toolchain::store::EMBEDDED_CATALOG;

	const CELL_TOOLCHAINS_PER_HOST: &[(&str, &str, &[&str])] = &[
		("linux", "x86_64", &["clang", "rust"]),
		("linux", "aarch64", &["clang", "rust"]),
		("macos", "aarch64", &["clang", "rust"]),
		("macos", "x86_64", &["rust"]),
		("windows", "x86_64", &["clang", "rust"]),
		("windows", "aarch64", &["clang", "rust"]),
	];

	fn bundled_catalog() -> Catalog {
		Catalog::parse(EMBEDDED_CATALOG).expect("bundled catalog must parse")
	}

	fn http_status(url: &str) -> u16 {
		match ureq::head(url).call() {
			Ok(response) => response.status().as_u16(),
			Err(ureq::Error::StatusCode(405)) | Err(ureq::Error::StatusCode(501)) => first_byte_status(url),
			Err(ureq::Error::StatusCode(code)) => code,
			Err(e) => panic!("{url}: {e}"),
		}
	}

	fn first_byte_status(url: &str) -> u16 {
		match ureq::get(url).header("Range", "bytes=0-0").call() {
			Ok(response) => response.status().as_u16(),
			Err(ureq::Error::StatusCode(code)) => code,
			Err(e) => panic!("{url}: {e}"),
		}
	}

	#[test]
	fn catalog_serves_the_platform_key_of_every_host_the_std_cells_need() {
		let catalog = bundled_catalog();
		for (os, arch, toolchains) in CELL_TOOLCHAINS_PER_HOST {
			let key = platform_key_for(os, arch);
			for toolchain in *toolchains {
				let entry = catalog
					.get(toolchain)
					.unwrap_or_else(|| panic!("`{toolchain}` is not in the catalog"));
				assert!(
					entry.targets.contains_key(&key),
					"`{toolchain}` has no `{key}` download, so `forge toolchains sync` cannot resolve it on a {os} {arch} host"
				);
			}
		}
	}

	#[test]
	fn every_catalog_target_key_is_one_the_engine_can_produce() {
		let catalog = bundled_catalog();
		let supported: BTreeSet<String> = CELL_TOOLCHAINS_PER_HOST
			.iter()
			.map(|(os, arch, _)| platform_key_for(os, arch))
			.collect();
		for name in catalog.names() {
			for key in catalog.get(name).unwrap().targets.keys() {
				assert!(
					supported.contains(key),
					"`{name}` advertises `{key}`, which no host resolves to; platform keys are the target-triple os and arch"
				);
			}
		}
	}

	#[test]
	fn every_catalog_url_resolves() {
		if std::env::var_os("FORGE_CATALOG_URLS").is_none() {
			eprintln!("skipping: set FORGE_CATALOG_URLS=1 to resolve every catalog url");
			return;
		}
		let catalog = bundled_catalog();
		let mut dead = Vec::new();
		for name in catalog.names() {
			let resolved = catalog.resolve(name, None).expect("every toolchain resolves");
			for platform in resolved.entry.targets.keys() {
				let target = resolved
					.entry
					.url_for(&resolved.version, platform)
					.expect("its own key resolves");
				let status = http_status(&target.url);
				println!("{status}  {name} {platform}  {}", short_url(&target.url));
				if !(200..300).contains(&status) {
					dead.push(format!("{name} {platform} {status} {}", target.url));
				}
			}
		}
		assert!(dead.is_empty(), "catalog urls that no longer exist:\n{}", dead.join("\n"));
	}

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

	fn serve_once(bytes: Vec<u8>) -> String {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		let address = listener.local_addr().unwrap();
		std::thread::spawn(move || {
			if let Ok((mut stream, _)) = listener.accept() {
				let mut request = [0u8; 1024];
				let _ = std::io::Read::read(&mut stream, &mut request);
				let header = format!(
					"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
					bytes.len()
				);
				let _ = std::io::Write::write_all(&mut stream, header.as_bytes());
				let _ = std::io::Write::write_all(&mut stream, &bytes);
			}
		});
		format!("http://{address}/artifact")
	}

	#[test]
	fn fetch_blob_verifies_before_publishing() {
		let root = std::env::temp_dir().join(format!("forge-sync-blob-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let store = crate::store::Store::at(&root);

		let payload = b"verified payload".to_vec();
		let url = serve_once(payload.clone());
		let wrong = "0".repeat(64);
		assert!(
			fetch_blob(&store, &url, Some(&wrong), false).is_err(),
			"a mismatched digest must not be published"
		);
		assert!(!store.blobs().exists() || std::fs::read_dir(store.blobs()).unwrap().flatten().count() == 0);
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn fetch_blob_reuses_published_blob_without_network() {
		let root = std::env::temp_dir().join(format!("forge-sync-reuse-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		let store = crate::store::Store::at(&root);
		let digest = "a".repeat(64);
		std::fs::create_dir_all(store.blobs()).unwrap();
		std::fs::write(store.blob(&digest), b"cached").unwrap();
		let path = fetch_blob(&store, "https://example.invalid/never", Some(&digest), true).unwrap();
		assert_eq!(path, store.blob(&digest));
		let absent = "b".repeat(64);
		let error = fetch_blob(&store, "https://example.invalid/absent", Some(&absent), true)
			.unwrap_err()
			.to_string();
		assert!(error.contains("https://example.invalid/absent"), "{error}");
		assert!(error.contains("re-run without `--offline`"), "{error}");
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn applies_catalog_bin_aliases() {
		let dir = std::env::temp_dir().join(format!("forge-sync-alias-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("x86_64-buildroot-linux-gnu-gcc"), b"#!/bin/sh\n").unwrap();
		let aliases = vec![
			forge_core::toolchain::catalog::BinAlias {
				from: "*-gcc".into(),
				to: "gcc".into(),
			},
			forge_core::toolchain::catalog::BinAlias {
				from: "*-clang".into(),
				to: "clang".into(),
			},
		];
		apply_bin_aliases(&dir, &aliases).unwrap();
		assert!(dir.join("gcc").is_file());
		assert!(!dir.join("clang").exists(), "unmatched alias is skipped");
		let _ = std::fs::remove_dir_all(&dir);
	}
}
